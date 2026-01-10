//! OrchestratorClient - In-process MCP extension for orchestrating plan generation and review.
//!
//! This module implements an MCP client that provides tools for the orchestrator agent
//! to coordinate plan generation, review, and human input handling. Tools are registered
//! via ExtensionManager::add_client() and prefixed as 'plan-forge-orchestrator__<tool_name>'.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
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
use super::orchestration_state::{HumanInputRecord, OrchestrationState, OrchestrationStatus};
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
            state,
            guardrails,
            planner,
            reviewer,
            info,
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
            let state = self.state.lock().await;
            if let Err(hard_stop) = self.guardrails.check_before_tool_call(&state) {
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
        let (iteration, working_dir, task) = {
            let state = self.state.lock().await;
            (
                state.iteration,
                state.working_dir.clone(),
                state.task.clone(),
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
            .generate_plan_json(&task_str, input.feedback.as_deref(), working_dir.to_str())
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

            // Track planner tokens in breakdown
            let input = token_usage.input_tokens.map(|t| t.max(0) as u64).unwrap_or(0);
            let output = token_usage.output_tokens.map(|t| t.max(0) as u64).unwrap_or(0);
            state.token_breakdown.add_planner(input, output);

            // Regenerate context summary after each iteration for efficient context passing
            state.context_summary = state.generate_context_summary();
        } // lock released

        CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&plan_json).unwrap_or_else(|_| plan_json.to_string()),
        )])
    }

    /// Handle review_plan tool call.
    async fn handle_review_plan(&self, arguments: Option<JsonObject>) -> CallToolResult {
        // 1. Check limits (short lock)
        {
            let state = self.state.lock().await;
            if let Err(hard_stop) = self.guardrails.check_before_tool_call(&state) {
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

        // 3. Read iteration from state (short lock)
        let iteration = {
            let state = self.state.lock().await;
            state.iteration
        };

        // 4. Call reviewer (async, no lock)
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

        // 5. Run guardrail checks (no lock needed - guardrails is Arc)
        let score = review_json
            .get("score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32;

        let triggered_conditions =
            self.guardrails
                .check_all_conditions(&input.plan_json, score, iteration);

        // 6. Build response with guardrail info
        let requires_human_input = !triggered_conditions.is_empty();
        let response = serde_json::json!({
            "llm_review": review_json,
            "guardrail_checks": triggered_conditions.iter().map(|c| format!("{:?}", c)).collect::<Vec<_>>(),
            "passed": review_json.get("passed").and_then(|v| v.as_bool()).unwrap_or(false),
            "requires_human_input": requires_human_input,
            "mandatory_condition": triggered_conditions.first().map(|c| format!("{:?}", c)),
            "summary": review_json.get("summary").and_then(|v| v.as_str()).unwrap_or("Review complete"),
        });

        // 7. Update state (short lock)
        {
            let mut state = self.state.lock().await;
            state.reviews.push(review_json);
            state.tool_calls += 1;
            state.add_tokens(token_usage.input_tokens, token_usage.output_tokens);

            // Track reviewer tokens in breakdown
            let input = token_usage.input_tokens.map(|t| t.max(0) as u64).unwrap_or(0);
            let output = token_usage.output_tokens.map(|t| t.max(0) as u64).unwrap_or(0);
            state.token_breakdown.add_reviewer(input, output);

            // Track triggered conditions
            for condition in &triggered_conditions {
                state.triggered_conditions.push(condition.clone());
            }

            // Regenerate context summary with latest review data
            state.context_summary = state.generate_context_summary();
        }

        CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap_or_else(|_| response.to_string()),
        )])
    }

    /// Handle request_human_input tool call.
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

        // Get the most recent triggered condition
        let (iteration, triggered_condition) = {
            let state = self.state.lock().await;
            (state.iteration, state.triggered_conditions.last().cloned())
        };

        // Create human input record
        let record = HumanInputRecord {
            question: input.question.clone(),
            category: input.category.clone(),
            response: None,
            condition: triggered_condition.clone(),
            iteration,
            timestamp: chrono::Utc::now().to_rfc3339(),
            approved: false,
        };

        // Update state with pending request (short lock)
        {
            let mut state = self.state.lock().await;
            state.pending_human_input = Some(record.clone());
            state.status = OrchestrationStatus::Paused {
                condition: triggered_condition.clone(),
            };
            state.tool_calls += 1;
        }

        let response = serde_json::json!({
            "status": "paused",
            "reason": "human_input_required",
            "question": input.question,
            "category": input.category,
            "condition": triggered_condition.map(|c| format!("{:?}", c)),
        });

        CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap_or_else(|_| response.to_string()),
        )])
    }

    /// Handle finalize tool call.
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

        // Check if any mandatory conditions are blocking
        if let Err(blocking_condition) = self
            .guardrails
            .check_before_finalize(&input.plan_json, &state_snapshot)
        {
            return CallToolResult::error(vec![Content::text(format!(
                "Cannot finalize: unapproved mandatory condition {:?}",
                blocking_condition
            ))]);
        }

        // Update state to completed (short lock)
        {
            let mut state = self.state.lock().await;
            state.current_plan = Some(input.plan_json.clone());
            state.status = OrchestrationStatus::Completed;
            state.tool_calls += 1;
        }

        let response = serde_json::json!({
            "success": true,
            "message": "Plan finalized successfully",
        });

        CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap_or_else(|_| response.to_string()),
        )])
    }

    /// Handle check_limits tool call.
    async fn handle_check_limits(&self) -> CallToolResult {
        let (iterations, tool_calls, total_tokens) = {
            let state = self.state.lock().await;
            (state.iteration, state.tool_calls, state.total_tokens)
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
    state: Arc<Mutex<OrchestrationState>>,
    guardrails: Arc<Guardrails>,
    planner: Arc<GoosePlanner>,
    reviewer: Arc<GooseReviewer>,
) -> OrchestratorClient {
    OrchestratorClient::new(session_id, state, guardrails, planner, reviewer)
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
