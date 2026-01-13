//! OrchestratorClient - In-process MCP extension for orchestrating plan generation and review.
//!
//! This module implements an MCP client that provides tools for the orchestrator agent
//! to coordinate plan generation, review, and human input handling. Tools are registered
//! via ExtensionManager::add_client() and prefixed as 'plan-forge-orchestrator__<tool_name>'.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tracing::info;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, GetPromptResult, Implementation, InitializeResult, JsonObject,
    ListPromptsResult, ListResourcesResult, ListToolsResult, ProtocolVersion,
    ReadResourceResult, ServerCapabilities, ServerNotification, Tool, ToolsCapability,
};
use rmcp::serde_json::{self, Value};
use rmcp::ServiceError;

// Type alias matching goose mcp_client.rs
type Error = ServiceError;
use schemars::schema_for;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use goose::agents::extension::ExtensionConfig;
use goose::agents::extension_manager::ExtensionManager;

use super::guardrails::Guardrails;
use super::orchestration_state::{
    HumanInputRecord, IterationOutcome, IterationRecord, OrchestrationState, OrchestrationStatus,
};
use super::viability::{ViabilityChecker, ViabilitySeverity};
use crate::models::Plan;
use crate::phases::{GoosePlanner, GooseReviewer};

/// Extension name used for tool prefixing (tools become plan-forge-orchestrator__<name>)
pub const EXTENSION_NAME: &str = "plan-forge-orchestrator";

// ============================================================================
// Token Usage Tracking
// ============================================================================

/// Token usage from an agent run.
/// Uses Option<i32> to match goose Session fields (session_manager.rs:88-90).
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
}

impl TokenUsage {
    pub fn new(input: Option<i32>, output: Option<i32>) -> Self {
        Self {
            input_tokens: input,
            output_tokens: output,
        }
    }
}

// ============================================================================
// Session Registry for Concurrent Session Management
// ============================================================================

/// Registry for managing concurrent orchestration sessions.
/// Uses RwLock for the registry (many reads, few writes) and Mutex for individual session state.
pub struct SessionRegistry {
    sessions: RwLock<HashMap<String, Arc<Mutex<OrchestrationState>>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    /// Get or create a session state, returning a reference to the session's state.
    pub async fn get_or_create(
        &self,
        session_id: &str,
        initial_state: OrchestrationState,
    ) -> Arc<Mutex<OrchestrationState>> {
        // First, try to get existing session with read lock
        {
            let sessions = self.sessions.read().await;
            if let Some(state) = sessions.get(session_id) {
                return Arc::clone(state);
            }
        }

        // Not found, acquire write lock and create
        let mut sessions = self.sessions.write().await;
        // Double-check in case another task created it
        if let Some(state) = sessions.get(session_id) {
            return Arc::clone(state);
        }

        let state = Arc::new(Mutex::new(initial_state));
        sessions.insert(session_id.to_string(), Arc::clone(&state));
        state
    }

    /// Get an existing session state.
    pub async fn get(&self, session_id: &str) -> Option<Arc<Mutex<OrchestrationState>>> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).map(Arc::clone)
    }

    /// Remove a session from the registry.
    pub async fn remove(&self, session_id: &str) -> Option<Arc<Mutex<OrchestrationState>>> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(session_id)
    }
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tool Input Schemas
// ============================================================================

/// Input for generate_plan tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GeneratePlanInput {
    /// The task description for plan generation
    pub task: String,
    /// Optional feedback from previous review to incorporate
    #[serde(default)]
    pub feedback: Option<Vec<String>>,
}

/// Input for review_plan tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviewPlanInput {
    /// The plan JSON to review
    pub plan_json: Value,
}

/// Input for request_human_input tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RequestHumanInputInput {
    /// The question to ask the human
    pub question: String,
    /// Category of input needed
    pub category: String,
    /// Additional context for the human
    #[serde(default)]
    pub context: Option<String>,
}

/// Input for finalize tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FinalizeInput {
    /// The final plan JSON to commit
    pub plan_json: Value,
}

// ============================================================================
// OrchestratorClient
// ============================================================================

/// MCP client for orchestrating plan generation and review.
/// Implements McpClientTrait and is registered via ExtensionManager::add_client().
///
/// Each OrchestratorClient instance is tied to a single session via session-scoped
/// Arc references. Multiple clients can exist for different sessions.
pub struct OrchestratorClient {
    /// Session identifier
    pub session_id: String,
    /// Session directory for state persistence
    session_dir: PathBuf,
    /// Session-scoped state (shared with GooseOrchestrator)
    state: Arc<Mutex<OrchestrationState>>,
    /// Guardrails for enforcing hard limits
    guardrails: Arc<Guardrails>,
    /// Planner for generating plans
    planner: Arc<GoosePlanner>,
    /// Reviewer for reviewing plans
    reviewer: Arc<GooseReviewer>,
    /// MCP initialization info
    info: InitializeResult,
}

impl OrchestratorClient {
    /// Create a new OrchestratorClient for a specific session.
    pub fn new(
        session_id: String,
        session_dir: PathBuf,
        state: Arc<Mutex<OrchestrationState>>,
        guardrails: Arc<Guardrails>,
        planner: Arc<GoosePlanner>,
        reviewer: Arc<GooseReviewer>,
    ) -> Self {
        let info = InitializeResult {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {
                    list_changed: Some(false),
                }),
                resources: None,
                prompts: None,
                completions: None,
                experimental: None,
                logging: None,
            },
            server_info: Implementation {
                name: EXTENSION_NAME.to_string(),
                title: Some("Plan Forge Orchestrator".to_string()),
                version: env!("CARGO_PKG_VERSION").to_string(),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Orchestrator tools for plan generation and review coordination.".to_string(),
            ),
        };

        Self {
            session_id,
            session_dir,
            state,
            guardrails,
            planner,
            reviewer,
            info,
        }
    }

    /// Save current state to disk. Called after every tool operation.
    async fn persist_state(&self) {
        let state = self.state.lock().await;
        if let Err(e) = state.save(&self.session_dir) {
            tracing::warn!("Failed to persist state: {}", e);
        }
    }

    /// Get ServerInfo for add_client() registration.
    /// Converts internal InitializeResult to ServerInfo format.
    pub fn get_server_info(&self) -> Option<rmcp::model::ServerInfo> {
        Some(rmcp::model::ServerInfo {
            protocol_version: self.info.protocol_version.clone(),
            capabilities: self.info.capabilities.clone(),
            server_info: self.info.server_info.clone(),
            instructions: self.info.instructions.clone(),
        })
    }

    /// Get tool definitions for this extension.
    fn get_tools() -> Vec<Tool> {
        // Helper function to create schema from JsonSchema type
        fn get_schema<T: JsonSchema>() -> JsonObject {
            let schema = schema_for!(T);
            let schema_value = serde_json::to_value(schema).expect("Failed to serialize schema");
            schema_value.as_object().unwrap().clone()
        }

        vec![
            Tool::new(
                "generate_plan".to_string(),
                "Generate or update a development plan. Call with task description and optional feedback.".to_string(),
                get_schema::<GeneratePlanInput>(),
            ),
            Tool::new(
                "review_plan".to_string(),
                "Review a plan for completeness and quality. Returns review results with guardrail checks.".to_string(),
                get_schema::<ReviewPlanInput>(),
            ),
            Tool::new(
                "request_human_input".to_string(),
                "Pause for human approval or input. Use when review indicates mandatory condition triggered.".to_string(),
                get_schema::<RequestHumanInputInput>(),
            ),
            Tool::new(
                "finalize".to_string(),
                "Complete the planning session with the final approved plan. Only call when review passed.".to_string(),
                get_schema::<FinalizeInput>(),
            ),
            Tool::new(
                "check_limits".to_string(),
                "Check current iteration count, tool calls, and token budget status.".to_string(),
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        ]
    }

    // ========================================================================
    // Tool Handlers
    // ========================================================================

    /// Handle generate_plan tool call.
    /// Uses short-lived lock pattern to prevent deadlocks.
    async fn handle_generate_plan(&self, arguments: Option<JsonObject>) -> CallToolResult {
        // 1. Check limits BEFORE any work (short lock)
        {
            let mut state = self.state.lock().await;
            if let Err(hard_stop) = self.guardrails.check_before_tool_call(&state) {
                // Update state to HardStopped before returning error
                state.status = OrchestrationStatus::HardStopped {
                    reason: hard_stop.clone(),
                };
                return CallToolResult::error(vec![Content::text(format!(
                    "Hard stop: {:?}",
                    hard_stop
                ))]);
            }
        } // lock released

        // 2. Extract parameters from arguments (no lock needed)
        let input: GeneratePlanInput = match arguments {
            Some(args) => match serde_json::from_value(Value::Object(args)) {
                Ok(v) => v,
                Err(e) => {
                    return CallToolResult::error(vec![Content::text(format!(
                        "Invalid arguments: {}",
                        e
                    ))])
                }
            },
            None => {
                return CallToolResult::error(vec![Content::text("Missing required arguments")])
            }
        };

        // 3. Read current state for context (short lock)
        let (iteration, working_dir, task, current_plan) = {
            let state = self.state.lock().await;
            (
                state.iteration,
                state.working_dir.clone(),
                state.task.clone(),
                state.current_plan.clone(),
            )
        }; // lock released

        // 4. Call planner (async, NO lock held)
        let task_str = if input.task.is_empty() {
            task
        } else {
            input.task
        };

        let (plan_json, token_usage) = match self
            .planner
            .generate_plan_json(
                &task_str,
                input.feedback.as_deref(),
                current_plan.as_ref(),
                working_dir.to_str(),
            )
            .await
        {
            Ok(v) => v,
            Err(e) => {
                return CallToolResult::error(vec![Content::text(format!(
                    "Planner failed: {}",
                    e
                ))])
            }
        };

        // 5. Update state with results (short lock)
        {
            let mut state = self.state.lock().await;
            state.current_plan = Some(plan_json.clone());
            state.iteration = iteration + 1;
            state.tool_calls += 1;
            state.add_tokens(token_usage.input_tokens, token_usage.output_tokens);

            // Mark that this new plan needs to be reviewed
            state.needs_review = true;

            // Track planner tokens in breakdown
            let input = token_usage.input_tokens.map(|t| t.max(0) as u64).unwrap_or(0);
            let output = token_usage.output_tokens.map(|t| t.max(0) as u64).unwrap_or(0);
            state.token_breakdown.add_planner(input, output);

            // Regenerate context summary after each iteration for efficient context passing
            state.context_summary = state.generate_context_summary();
        } // lock released

        // Persist state after plan generation
        self.persist_state().await;

        // Return plan without validation - all checks happen in review_plan
        let response = serde_json::json!({
            "plan": plan_json,
            "tokens_used": token_usage.input_tokens.unwrap_or(0) + token_usage.output_tokens.unwrap_or(0)
        });

        CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap_or_else(|_| response.to_string()),
        )])
    }

    /// Handle review_plan tool call.
    /// Runs V-* viability checks first (deterministic), then Q-* quality checks (LLM).
    /// If V-* fails, skips expensive LLM review.
    async fn handle_review_plan(&self, arguments: Option<JsonObject>) -> CallToolResult {
        // 1. Check limits (short lock)
        {
            let mut state = self.state.lock().await;
            if let Err(hard_stop) = self.guardrails.check_before_tool_call(&state) {
                // Update state to HardStopped before returning error
                state.status = OrchestrationStatus::HardStopped {
                    reason: hard_stop.clone(),
                };
                return CallToolResult::error(vec![Content::text(format!(
                    "Hard stop: {:?}",
                    hard_stop
                ))]);
            }
        }

        // 2. Extract plan from arguments
        let input: ReviewPlanInput = match arguments {
            Some(args) => match serde_json::from_value(Value::Object(args)) {
                Ok(v) => v,
                Err(e) => {
                    return CallToolResult::error(vec![Content::text(format!(
                        "Invalid arguments: {}",
                        e
                    ))])
                }
            },
            None => {
                return CallToolResult::error(vec![Content::text("Missing required arguments")])
            }
        };

        // 3. Read iteration and capture starting metrics (short lock)
        let (iteration, starting_tool_calls, starting_tokens) = {
            let state = self.state.lock().await;
            (state.iteration, state.tool_calls, state.total_tokens)
        };

        // 4. Run V-* viability checks FIRST (deterministic, cheap)
        let viability_result = match serde_json::from_value::<Plan>(input.plan_json.clone()) {
            Ok(plan) => {
                let checker = ViabilityChecker::new();
                let result = checker.check_all(
                    plan.instructions.as_deref(),
                    plan.grounding_snapshot.as_ref(),
                    Some(&plan.file_references),
                );
                let metrics = plan.instructions.as_ref().map(|i| checker.analyze_dag(i));

                // Determine if validation passed (no Critical-severity violations)
                let passed = result
                    .violations
                    .iter()
                    .all(|v| v.severity != ViabilitySeverity::Critical);

                Some((result, metrics, passed))
            }
            Err(_) => None, // Plan doesn't parse - skip viability checks
        };

        // 5. If V-* critical failures, skip expensive LLM review
        if let Some((ref viability, ref metrics, passed)) = viability_result {
            if !passed {
                // Count violations
                let total_violations = viability.violations.len() as u32;
                let critical_violations = viability
                    .violations
                    .iter()
                    .filter(|v| v.severity == ViabilitySeverity::Critical)
                    .count() as u32;

                // Update state with viability failure (short lock)
                {
                    let mut state = self.state.lock().await;
                    state.tool_calls += 1;
                    // Reset validation flags - viability failed means review not passed
                    state.requires_human_input_pending = false;
                    state.last_review_passed = false;
                    // Mark that the plan has been reviewed (even though it failed viability)
                    state.needs_review = false;
                    state.context_summary = state.generate_context_summary();

                    // Record iteration with viability failure
                    let record = IterationRecord {
                        iteration,
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        viability_violations: total_violations,
                        viability_critical: critical_violations,
                        viability_passed: false,
                        review_score: None,
                        review_passed: None,
                        tool_calls_this_iteration: state.tool_calls - starting_tool_calls,
                        tokens_this_iteration: state.total_tokens - starting_tokens,
                        outcome: IterationOutcome::ViabilityFailed,
                    };
                    state.iteration_history.push(record);
                }
                self.persist_state().await;

                let response = serde_json::json!({
                    "viability": {
                        "violations": viability.violations,
                        "metrics": metrics,
                        "passed": false
                    },
                    "llm_review": null,  // Skipped - plan not viable
                    "passed": false,
                    "requires_human_input": false,
                    "summary": "Plan failed viability checks. Fix violations before review."
                });

                return CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&response).unwrap_or_else(|_| response.to_string()),
                )]);
            }
        }

        // 6. Run LLM review (expensive) only if plan is viable
        let (review_json, token_usage) = match self
            .reviewer
            .review_plan_json(&input.plan_json)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                return CallToolResult::error(vec![Content::text(format!(
                    "Reviewer failed: {}",
                    e
                ))])
            }
        };

        // 7. Extract score and check if passed DETERMINISTICALLY
        let score = review_json
            .get("score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32;

        // CRITICAL: Use deterministic score threshold check, not LLM's "passed" field
        // The LLM reviewer's "passed" field is informational only - we enforce the threshold
        let score_passed = self.guardrails.score_passes(score);

        // 8. Build response with viability + LLM review
        // Human input requirement only comes from reviewer LLM (security, ambiguity, etc.)
        let requires_human_input = review_json
            .get("requires_human_input")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Log if LLM's passed field disagrees with deterministic check
        let llm_passed = review_json.get("passed").and_then(|v| v.as_bool()).unwrap_or(false);
        if llm_passed != score_passed {
            tracing::warn!(
                "LLM reviewer passed={} but score {} {} threshold {} (using deterministic check)",
                llm_passed,
                score,
                if score_passed { ">=" } else { "<" },
                self.guardrails.score_threshold
            );
        }

        let response = serde_json::json!({
            "viability": viability_result.as_ref().map(|(v, m, p)| serde_json::json!({
                "violations": v.violations,
                "metrics": m,
                "passed": p
            })),
            "llm_review": review_json,
            "passed": score_passed,  // Deterministic: score >= threshold
            "score": score,
            "threshold": self.guardrails.score_threshold,
            "requires_human_input": requires_human_input,
            "summary": review_json.get("summary").and_then(|v| v.as_str()).unwrap_or("Review complete"),
        });

        // 9. Update state (short lock)
        {
            let mut state = self.state.lock().await;
            state.reviews.push(review_json);
            state.tool_calls += 1;
            state.add_tokens(token_usage.input_tokens, token_usage.output_tokens);

            // Set validation flags based on DETERMINISTIC score check (not LLM's passed field)
            state.last_review_passed = score_passed;
            state.requires_human_input_pending = requires_human_input;

            // Mark that the current plan has been reviewed
            state.needs_review = false;

            // Track reviewer tokens in breakdown
            let input = token_usage.input_tokens.map(|t| t.max(0) as u64).unwrap_or(0);
            let output = token_usage.output_tokens.map(|t| t.max(0) as u64).unwrap_or(0);
            state.token_breakdown.add_reviewer(input, output);

            // Track best plan (highest score seen)
            if score > state.best_score {
                state.best_score = score;
                state.best_plan = state.current_plan.clone();
                info!("New best plan with score {:.2}", score);
            }

            // Get viability stats for iteration record
            let (viability_violations, viability_critical) = viability_result
                .as_ref()
                .map(|(v, _, _)| {
                    let total = v.violations.len() as u32;
                    let critical = v
                        .violations
                        .iter()
                        .filter(|vv| vv.severity == ViabilitySeverity::Critical)
                        .count() as u32;
                    (total, critical)
                })
                .unwrap_or((0, 0));

            // Record iteration with review result (using deterministic score check)
            let record = IterationRecord {
                iteration,
                timestamp: chrono::Utc::now().to_rfc3339(),
                viability_violations,
                viability_critical,
                viability_passed: true, // We got here, so viability passed
                review_score: Some(score),
                review_passed: Some(score_passed),  // Deterministic check
                tool_calls_this_iteration: state.tool_calls - starting_tool_calls,
                tokens_this_iteration: state.total_tokens - starting_tokens,
                outcome: if requires_human_input {
                    IterationOutcome::HumanInputRequested
                } else if score_passed {
                    IterationOutcome::ReviewPassed
                } else {
                    IterationOutcome::ReviewFailed
                },
            };
            state.iteration_history.push(record);

            // Regenerate context summary with latest review data
            state.context_summary = state.generate_context_summary();
        }

        // Persist state after review
        self.persist_state().await;

        CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap_or_else(|_| response.to_string()),
        )])
    }

    /// Handle request_human_input tool call.
    /// Validates that the reviewer authorized this pause via requires_human_input=true.
    async fn handle_request_human_input(&self, arguments: Option<JsonObject>) -> CallToolResult {
        let input: RequestHumanInputInput = match arguments {
            Some(args) => match serde_json::from_value(Value::Object(args)) {
                Ok(v) => v,
                Err(e) => {
                    return CallToolResult::error(vec![Content::text(format!(
                        "Invalid arguments: {}",
                        e
                    ))])
                }
            },
            None => {
                return CallToolResult::error(vec![Content::text("Missing required arguments")])
            }
        };

        // 1. Check if this request is authorized (reviewer set requires_human_input=true)
        let (iteration, is_authorized) = {
            let state = self.state.lock().await;
            (state.iteration, state.requires_human_input_pending)
        };

        // Reject unauthorized pause attempts
        if !is_authorized {
            tracing::warn!(
                "Orchestrator called request_human_input without reviewer approval at iteration {}. \
                Category: {}, Question: {}",
                iteration,
                input.category,
                input.question.chars().take(100).collect::<String>()
            );

            let response = serde_json::json!({
                "status": "rejected",
                "error": "UNAUTHORIZED_PAUSE",
                "message": "The reviewer did not set requires_human_input=true. You cannot pause on your own judgment.",
                "required_action": "Call generate_plan with feedback from the last review to continue iterating.",
                "iteration": iteration
            });

            return CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&response).unwrap_or_else(|_| response.to_string()),
            )]);
        }

        // Build reason from category and question
        let reason = format!("{}: {}", input.category, input.question);

        // Create human input record
        let record = HumanInputRecord {
            question: input.question.clone(),
            category: input.category.clone(),
            response: None,
            reason: Some(reason.clone()),
            iteration,
            timestamp: chrono::Utc::now().to_rfc3339(),
            approved: false,
        };

        // Update state with pending request (short lock)
        {
            let mut state = self.state.lock().await;
            state.pending_human_input = Some(record.clone());
            state.status = OrchestrationStatus::Paused { reason: reason.clone() };
            state.tool_calls += 1;
            // Clear the authorization flag - it's been used
            state.requires_human_input_pending = false;
        }

        let response = serde_json::json!({
            "status": "paused",
            "reason": reason,
            "question": input.question,
            "category": input.category,
        });

        // Persist state after requesting human input
        self.persist_state().await;

        CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap_or_else(|_| response.to_string()),
        )])
    }

    /// Handle finalize tool call.
    /// Validates that the plan has passed review before allowing finalization.
    async fn handle_finalize(&self, arguments: Option<JsonObject>) -> CallToolResult {
        let input: FinalizeInput = match arguments {
            Some(args) => match serde_json::from_value(Value::Object(args)) {
                Ok(v) => v,
                Err(e) => {
                    return CallToolResult::error(vec![Content::text(format!(
                        "Invalid arguments: {}",
                        e
                    ))])
                }
            },
            None => {
                return CallToolResult::error(vec![Content::text("Missing required arguments")])
            }
        };

        // Check guardrails before finalizing (short lock to read state)
        let state_snapshot = {
            let state = self.state.lock().await;
            state.clone()
        };

        // 1. Validate that the plan has actually passed review
        if !state_snapshot.last_review_passed {
            tracing::warn!(
                "Orchestrator called finalize but plan hasn't passed review at iteration {}",
                state_snapshot.iteration
            );

            let response = serde_json::json!({
                "status": "rejected",
                "error": "PLAN_NOT_APPROVED",
                "message": "Cannot finalize - the plan has not passed review.",
                "required_action": "Call generate_plan with feedback to improve the plan, then call review_plan.",
                "iteration": state_snapshot.iteration
            });

            return CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&response).unwrap_or_else(|_| response.to_string()),
            )]);
        }

        // Update state to completed (short lock)
        {
            let mut state = self.state.lock().await;
            state.current_plan = Some(input.plan_json.clone());
            state.status = OrchestrationStatus::Completed;
            state.tool_calls += 1;
        }

        // Persist final state
        self.persist_state().await;

        let response = serde_json::json!({
            "success": true,
            "message": "Plan finalized successfully",
        });

        CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap_or_else(|_| response.to_string()),
        )])
    }

    /// Handle check_limits tool call.
    ///
    /// Returns current iteration state including the task for context recovery
    /// in case of conversation compaction.
    async fn handle_check_limits(&self) -> CallToolResult {
        let (iterations, tool_calls, total_tokens, task) = {
            let state = self.state.lock().await;
            (
                state.iteration,
                state.tool_calls,
                state.total_tokens,
                state.task.clone(),
            )
        };

        let max_iterations = self.guardrails.max_iterations;
        let max_tool_calls = self.guardrails.max_tool_calls;
        let max_total_tokens = self.guardrails.max_total_tokens;

        let exceeded = iterations >= max_iterations
            || tool_calls >= max_tool_calls
            || total_tokens >= max_total_tokens;

        let exceeded_reason = if iterations >= max_iterations {
            Some(format!(
                "Max iterations exceeded: {} >= {}",
                iterations, max_iterations
            ))
        } else if tool_calls >= max_tool_calls {
            Some(format!(
                "Max tool calls exceeded: {} >= {}",
                tool_calls, max_tool_calls
            ))
        } else if total_tokens >= max_total_tokens {
            Some(format!(
                "Token budget exhausted: {} >= {}",
                total_tokens, max_total_tokens
            ))
        } else {
            None
        };

        let response = serde_json::json!({
            "task": task,  // Include task for context recovery
            "iterations": iterations,
            "tool_calls": tool_calls,
            "total_tokens": total_tokens,
            "limits": {
                "max_iterations": max_iterations,
                "max_tool_calls": max_tool_calls,
                "max_total_tokens": max_total_tokens,
            },
            "exceeded": exceeded,
            "exceeded_reason": exceeded_reason,
            "token_budget_remaining": max_total_tokens.saturating_sub(total_tokens),
        });

        CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap_or_else(|_| response.to_string()),
        )])
    }
}

// ============================================================================
// McpClientTrait Implementation
// ============================================================================

#[async_trait]
impl goose::agents::mcp_client::McpClientTrait for OrchestratorClient {
    async fn list_resources(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListResourcesResult, Error> {
        // Resources not supported - return error following TodoClient pattern
        Err(Error::TransportClosed)
    }

    async fn read_resource(
        &self,
        _uri: &str,
        _cancel_token: CancellationToken,
    ) -> Result<ReadResourceResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn list_tools(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        // Note: ExtensionManager strips the prefix before calling,
        // so we match on unprefixed tool names
        let result = match name {
            "generate_plan" => self.handle_generate_plan(arguments).await,
            "review_plan" => self.handle_review_plan(arguments).await,
            "request_human_input" => self.handle_request_human_input(arguments).await,
            "finalize" => self.handle_finalize(arguments).await,
            "check_limits" => self.handle_check_limits().await,
            _ => CallToolResult::error(vec![Content::text(format!("Unknown tool: {}", name))]),
        };

        Ok(result)
    }

    async fn list_prompts(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListPromptsResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn get_prompt(
        &self,
        _name: &str,
        _arguments: Value,
        _cancel_token: CancellationToken,
    ) -> Result<GetPromptResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        // Following TodoClient pattern at todo_extension.rs:237-239
        mpsc::channel(1).1
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}

// ============================================================================
// Registration Helper
// ============================================================================

/// Register OrchestratorClient with ExtensionManager.
/// Must be called BEFORE agent.reply() for tools to be visible.
///
/// Wraps client correctly as Arc<Mutex<Box<dyn McpClientTrait>>> per extension_manager.rs:51.
pub async fn register_orchestrator_extension(
    manager: &ExtensionManager,
    client: OrchestratorClient,
) {
    // Extract server_info BEFORE moving client into Box
    let server_info = client.get_server_info();

    // Wrap client for McpClientBox type
    let boxed: Box<dyn goose::agents::mcp_client::McpClientTrait> = Box::new(client);
    let mcp_client_box = Arc::new(tokio::sync::Mutex::new(boxed));

    // Create config for metadata (used for display, not client creation)
    let config = ExtensionConfig::Builtin {
        name: EXTENSION_NAME.to_string(),
        description: "Orchestrator tools for plan generation and review".to_string(),
        display_name: Some("Plan Forge Orchestrator".to_string()),
        timeout: Some(300),
        bundled: Some(false),
        available_tools: vec![
            "generate_plan".to_string(),
            "review_plan".to_string(),
            "request_human_input".to_string(),
            "finalize".to_string(),
            "check_limits".to_string(),
        ],
    };

    manager
        .add_client(
            EXTENSION_NAME.to_string(),
            config,
            mcp_client_box,
            server_info,
            None,
        )
        .await;
}

/// Factory function for creating session-scoped OrchestratorClient.
pub fn create_orchestrator_client(
    session_id: String,
    session_dir: PathBuf,
    state: Arc<Mutex<OrchestrationState>>,
    guardrails: Arc<Guardrails>,
    planner: Arc<GoosePlanner>,
    reviewer: Arc<GooseReviewer>,
) -> OrchestratorClient {
    OrchestratorClient::new(session_id, session_dir, state, guardrails, planner, reviewer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_session_registry_creation() {
        let registry = SessionRegistry::new();
        assert!(registry.sessions.try_read().is_ok());
    }

    #[test]
    fn test_token_usage_default() {
        let usage = TokenUsage::default();
        assert!(usage.input_tokens.is_none());
        assert!(usage.output_tokens.is_none());
    }

    #[test]
    fn test_token_usage_new() {
        let usage = TokenUsage::new(Some(100), Some(200));
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(200));
    }

    #[tokio::test]
    async fn test_session_registry_get_or_create() {
        let registry = SessionRegistry::new();
        let initial_state = OrchestrationState::new(
            "session1".to_string(),
            "test task".to_string(),
            PathBuf::from("/test"),
            "test-slug".to_string(),
        );

        // First call creates the session
        let state1 = registry.get_or_create("session1", initial_state.clone()).await;
        {
            let s = state1.lock().await;
            assert_eq!(s.session_id, "session1");
        }

        // Second call returns existing session
        let initial_state2 = OrchestrationState::new(
            "session1".to_string(),
            "different task".to_string(),
            PathBuf::from("/other"),
            "other-slug".to_string(),
        );
        let state2 = registry.get_or_create("session1", initial_state2).await;
        {
            let s = state2.lock().await;
            // Should still have the original task, not the new one
            assert_eq!(s.task, "test task");
        }
    }

    #[tokio::test]
    async fn test_session_registry_get() {
        let registry = SessionRegistry::new();

        // Get non-existent session returns None
        assert!(registry.get("nonexistent").await.is_none());

        // Create a session
        let initial_state = OrchestrationState::new(
            "session1".to_string(),
            "test task".to_string(),
            PathBuf::from("/test"),
            "test-slug".to_string(),
        );
        registry.get_or_create("session1", initial_state).await;

        // Now get returns Some
        let state = registry.get("session1").await;
        assert!(state.is_some());
    }

    #[tokio::test]
    async fn test_session_registry_remove() {
        let registry = SessionRegistry::new();
        let initial_state = OrchestrationState::new(
            "session1".to_string(),
            "test task".to_string(),
            PathBuf::from("/test"),
            "test-slug".to_string(),
        );

        registry.get_or_create("session1", initial_state).await;
        assert!(registry.get("session1").await.is_some());

        // Remove the session
        let removed = registry.remove("session1").await;
        assert!(removed.is_some());

        // Session no longer exists
        assert!(registry.get("session1").await.is_none());
    }

    #[tokio::test]
    async fn test_session_registry_multiple_sessions() {
        let registry = SessionRegistry::new();

        // Create multiple sessions
        for i in 0..5 {
            let initial_state = OrchestrationState::new(
                format!("session{}", i),
                format!("task{}", i),
                PathBuf::from("/test"),
                format!("slug{}", i),
            );
            let state = registry.get_or_create(&format!("session{}", i), initial_state).await;
            {
                let mut s = state.lock().await;
                s.iteration = i as u32;
            }
        }

        // Verify each session has independent state
        for i in 0..5 {
            let state = registry.get(&format!("session{}", i)).await.unwrap();
            let s = state.lock().await;
            assert_eq!(s.iteration, i as u32);
            assert_eq!(s.task, format!("task{}", i));
        }
    }

    #[tokio::test]
    async fn test_session_registry_concurrent_access() {
        use std::sync::Arc;

        let registry = Arc::new(SessionRegistry::new());
        let mut handles = vec![];

        // Spawn multiple tasks that access the registry concurrently
        for i in 0..10 {
            let reg = Arc::clone(&registry);
            let handle = tokio::spawn(async move {
                let initial_state = OrchestrationState::new(
                    format!("concurrent{}", i),
                    format!("task{}", i),
                    PathBuf::from("/test"),
                    format!("slug{}", i),
                );
                let state = reg.get_or_create(&format!("concurrent{}", i), initial_state).await;

                // Modify state
                {
                    let mut s = state.lock().await;
                    s.tool_calls += 1;
                }

                // Read state
                {
                    let s = state.lock().await;
                    s.tool_calls
                }
            });
            handles.push(handle);
        }

        // Wait for all tasks to complete
        let results: Vec<_> = futures::future::join_all(handles).await;
        for result in results {
            assert_eq!(result.unwrap(), 1); // Each session should have tool_calls = 1
        }
    }

    #[test]
    fn test_tool_definitions() {
        let tools = OrchestratorClient::get_tools();
        assert_eq!(tools.len(), 5);

        let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(tool_names.contains(&"generate_plan"));
        assert!(tool_names.contains(&"review_plan"));
        assert!(tool_names.contains(&"request_human_input"));
        assert!(tool_names.contains(&"finalize"));
        assert!(tool_names.contains(&"check_limits"));
    }

    #[test]
    fn test_generate_plan_input_schema() {
        // Verify the schema can be generated without panic
        let schema = schemars::schema_for!(GeneratePlanInput);
        let schema_json = serde_json::to_value(schema).unwrap();
        assert!(schema_json.get("properties").is_some());
    }

    #[test]
    fn test_review_plan_input_schema() {
        let schema = schemars::schema_for!(ReviewPlanInput);
        let schema_json = serde_json::to_value(schema).unwrap();
        assert!(schema_json.get("properties").is_some());
    }

    #[test]
    fn test_request_human_input_schema() {
        let schema = schemars::schema_for!(RequestHumanInputInput);
        let schema_json = serde_json::to_value(schema).unwrap();
        let properties = schema_json.get("properties").unwrap();
        assert!(properties.get("question").is_some());
        assert!(properties.get("category").is_some());
    }

    #[test]
    fn test_finalize_input_schema() {
        let schema = schemars::schema_for!(FinalizeInput);
        let schema_json = serde_json::to_value(schema).unwrap();
        let properties = schema_json.get("properties").unwrap();
        assert!(properties.get("plan_json").is_some());
    }

    #[test]
    fn test_generate_plan_input_deserialize() {
        let json = serde_json::json!({
            "task": "Create a new feature",
            "feedback": ["Add error handling", "Improve documentation"]
        });

        let input: GeneratePlanInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.task, "Create a new feature");
        assert_eq!(input.feedback.unwrap().len(), 2);
    }

    #[test]
    fn test_generate_plan_input_deserialize_minimal() {
        let json = serde_json::json!({
            "task": "Create a new feature"
        });

        let input: GeneratePlanInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.task, "Create a new feature");
        assert!(input.feedback.is_none());
    }
}
