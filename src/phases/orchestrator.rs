//! GooseOrchestrator - LLM-powered orchestrator for plan generation and review.
//!
//! This module implements an orchestrator agent that uses goose's Agent with
//! in-process MCP extensions to coordinate the plan-review workflow.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, info, warn};

use goose::agents::{Agent, AgentEvent, SessionConfig};
use goose::conversation::message::{Message, MessageContent};
use goose::providers::{base::Provider, create_with_named_model};
use goose::recipe::Recipe;
use goose::session::{session_manager::SessionType, SessionManager};

use crate::config::{GuardrailsConfig, OrchestratorConfig, OutputConfig};
use crate::models::Plan;
use crate::output::FileOutputWriter;
use crate::orchestrator::{
    create_orchestrator_client, register_orchestrator_extension, GuardrailHardStop, Guardrails,
    HumanResponse, IterationOutcome, IterationRecord, OrchestrationState,
    OrchestrationStatus, SessionRegistry, TokenBreakdown,
};
use crate::phases::{GoosePlanner, GooseReviewer};
use crate::recipes::load_recipe;
// ============================================================================
// OrchestrationResult
// ============================================================================

/// Result of an orchestration session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationResult {
    /// Final plan JSON (if generated)
    pub final_plan: Option<Value>,
    /// Best plan seen during session (highest score)
    pub best_plan: Option<Value>,
    /// Best review score achieved
    pub best_score: f32,
    /// Final status of the session
    pub status: OrchestrationStatus,
    /// Number of iterations completed
    pub iterations: u32,
    /// Total tool calls made
    pub tool_calls: u32,
    /// Total tokens consumed
    pub total_tokens: u64,
    /// Session ID for resume
    pub session_id: String,
    /// Token usage breakdown by component
    pub token_breakdown: TokenBreakdown,
}

// ============================================================================
// GooseOrchestrator
// ============================================================================

/// LLM-powered orchestrator that coordinates plan generation and review.
///
/// Unlike the deterministic LoopController, the GooseOrchestrator uses an LLM
/// agent to make decisions about when to generate, review, pause for human input,
/// and finalize plans.
pub struct GooseOrchestrator {
    /// Orchestrator configuration
    config: OrchestratorConfig,
    /// Guardrails configuration
    guardrails_config: GuardrailsConfig,
    /// Base directory for the project
    base_dir: PathBuf,
    /// Session directory for orchestrator state
    session_dir: PathBuf,
    /// Session registry for concurrent session management
    session_registry: Arc<SessionRegistry>,
}

impl GooseOrchestrator {
    /// Create a new orchestrator.
    pub fn new(
        config: OrchestratorConfig,
        guardrails_config: GuardrailsConfig,
        base_dir: PathBuf,
        runs_dir: PathBuf,
        session_registry: Arc<SessionRegistry>,
    ) -> Self {
        Self {
            config,
            guardrails_config,
            base_dir,
            session_dir: runs_dir,
            session_registry,
        }
    }

    /// Create the LLM provider for the orchestrator agent.
    async fn create_provider(&self, recipe: &Recipe) -> Result<Arc<dyn Provider>> {
        let provider_name = self
            .config
            .provider_override
            .as_deref()
            .or(recipe
                .settings
                .as_ref()
                .and_then(|s| s.goose_provider.as_deref()))
            .unwrap_or("anthropic");

        let model_name = self
            .config
            .model_override
            .as_deref()
            .or(recipe
                .settings
                .as_ref()
                .and_then(|s| s.goose_model.as_deref()))
            .unwrap_or("claude-sonnet-4-20250514");

        info!(
            "Creating orchestrator provider: {} with model: {}",
            provider_name, model_name
        );
        create_with_named_model(provider_name, model_name)
            .await
            .context("Failed to create orchestrator provider")
    }

    /// Run the orchestrator for a task.
    ///
    /// # Arguments
    /// * `task` - The task description
    /// * `working_dir` - Working directory for plan generation
    /// * `human_response` - Optional human response for resuming paused sessions
    /// * `session_id` - Optional session ID for resuming (generates new if None)
    pub async fn run(
        &self,
        task: String,
        working_dir: Option<PathBuf>,
        human_response: Option<HumanResponse>,
        session_id: Option<String>,
    ) -> Result<OrchestrationResult> {
        // Generate or use provided session ID
        let session_id = session_id.unwrap_or_else(|| {
            format!(
                "orchestrator-{}",
                chrono::Utc::now().format("%Y%m%d-%H%M%S")
            )
        });

        // Use session_dir directly (already includes slug from MCP server)
        let session_dir = self.session_dir.clone();
        // Extract task_slug from directory name for OrchestrationState
        let task_slug = self
            .session_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        info!("Starting orchestrator session: {}", session_id);

        // Create shared components
        let guardrails = Arc::new(Guardrails::from_config(&self.guardrails_config));

        // Get planner/reviewer provider/model from environment variables
        // This allows orchestrator mode to use the same configuration as CLI
        let planner_provider = std::env::var("PLAN_FORGE_PLANNER_PROVIDER").ok();
        let planner_model = std::env::var("PLAN_FORGE_PLANNER_MODEL").ok();
        let reviewer_provider = std::env::var("PLAN_FORGE_REVIEWER_PROVIDER").ok();
        let reviewer_model = std::env::var("PLAN_FORGE_REVIEWER_MODEL").ok();
        let pass_threshold: f32 = std::env::var("PLAN_FORGE_THRESHOLD")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.8);

        let planner = Arc::new(GoosePlanner::new(
            crate::config::PlanningConfig {
                recipe: PathBuf::from("recipes/planner.yaml"),
                provider_override: planner_provider,
                model_override: planner_model,
            },
            self.base_dir.clone(),
        ));

        let reviewer = Arc::new(GooseReviewer::new(
            crate::config::ReviewConfig {
                recipe: PathBuf::from("recipes/reviewer.yaml"),
                provider_override: reviewer_provider,
                model_override: reviewer_model,
                pass_threshold,
            },
            self.base_dir.clone(),
        ));

        // Check for existing state (resume scenario)
        let existing_state = OrchestrationState::load(&session_dir)?;

        // Get or create session state
        let working_dir_path = working_dir.unwrap_or_else(|| self.base_dir.clone());

        let initial_state = if let Some(state) = existing_state {
            // Check if we can resume
            if !state.can_resume() {
                return Err(anyhow::anyhow!(
                    "Cannot resume session in {:?} state",
                    state.status
                ));
            }
            info!("Resuming orchestrator session from iteration {}", state.iteration);
            state
        } else {
            OrchestrationState::new(
                session_id.clone(),
                task.clone(),
                working_dir_path.clone(),
                task_slug.clone(),
            )
        };

        // Get or create session state in registry
        let session_state = self
            .session_registry
            .get_or_create(&session_id, initial_state)
            .await;

        // Handle human response if provided
        if let Some(hr) = human_response {
            let mut state = session_state.lock().await;
            if let Some(pending) = state.pending_human_input.take() {
                let completed = crate::orchestrator::HumanInputRecord {
                    question: pending.question,
                    category: pending.category,
                    response: Some(hr.response.clone()),
                    reason: pending.reason.clone(),
                    iteration: state.iteration,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    approved: hr.approved,
                };
                state.human_inputs.push(completed);
                state.status = OrchestrationStatus::Running;

                info!(
                    "Human response received: approved={}, response={}",
                    hr.approved,
                    hr.response.chars().take(100).collect::<String>()
                );
            } else {
                warn!("Human response provided but no pending input request");
            }
        }

        // Load orchestrator recipe and create provider (shared across iterations)
        let recipe = load_recipe(&self.config.recipe, &self.base_dir, "orchestrator")?;
        let provider = self.create_provider(&recipe).await?;

        // Run agent with timeout
        let timeout_duration =
            Duration::from_secs(self.guardrails_config.execution_timeout_secs);
        let max_iterations = self.guardrails_config.max_iterations;

        // Track sessions created for token accounting
        let mut iteration_sessions: Vec<String> = Vec::new();

        // Stateless iteration loop: create fresh agent per iteration
        loop {
            // 1. Check iteration limit BEFORE creating agent
            let current_iteration = {
                let state = session_state.lock().await;
                state.iteration
            };

            if current_iteration >= max_iterations {
                let mut state = session_state.lock().await;
                warn!(
                    "Max iterations ({}) reached without completion",
                    max_iterations
                );
                state.status = OrchestrationStatus::HardStopped {
                    reason: GuardrailHardStop::MaxIterationsExceeded {
                        iteration: current_iteration,
                        limit: max_iterations,
                    },
                };
                break;
            }

            // 2. Create FRESH agent for this iteration
            let agent = Agent::new();
            let iteration_session = SessionManager::create_session(
                working_dir_path.clone(),
                format!("orchestrator-iter-{}", current_iteration),
                SessionType::Hidden,
            )
            .await
            .context("Failed to create orchestrator iteration session")?;

            let iteration_session_id = iteration_session.id.clone();
            iteration_sessions.push(iteration_session_id.clone());

            // Apply provider and recipe instructions
            agent
                .update_provider(provider.clone(), &iteration_session_id)
                .await?;
            if let Some(instructions) = &recipe.instructions {
                agent.override_system_prompt(instructions.clone()).await;
            }

            // Register orchestrator extension for this fresh agent
            let orchestrator_client = create_orchestrator_client(
                session_id.clone(),
                session_dir.clone(),
                session_state.clone(),
                guardrails.clone(),
                planner.clone(),
                reviewer.clone(),
            );
            register_orchestrator_extension(&agent.extension_manager, orchestrator_client).await;

            info!(
                "Iteration {}: Created fresh agent with session {}",
                current_iteration, iteration_session_id
            );

            // 3. Build EXPLICIT context message (no conversation history dependency)
            let context_message = self
                .build_iteration_context(&task, &session_state, max_iterations)
                .await;
            let user_message = Message::user().with_text(&context_message);

            let session_config = SessionConfig {
                id: iteration_session_id.clone(),
                schedule_id: None,
                max_turns: Some(50), // Fewer turns per iteration (single decision)
                retry_config: None,
            };

            // 4. Run agent ONCE for this iteration
            let stream_result = tokio::time::timeout(
                timeout_duration,
                agent.reply(user_message, session_config, None),
            )
            .await;

            let stream = match stream_result {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    // Agent error - save state and return
                    let mut state = session_state.lock().await;
                    state.status = OrchestrationStatus::Failed {
                        error: format!("Agent error: {}", e),
                    };
                    state.save(&session_dir)?;
                    return Err(e.into());
                }
                Err(_) => {
                    // Timeout - save state with hard stop
                    let mut state = session_state.lock().await;
                    state.status = OrchestrationStatus::HardStopped {
                        reason: GuardrailHardStop::ExecutionTimeout,
                    };
                    state.save(&session_dir)?;
                    return Err(anyhow::anyhow!("Orchestrator execution timeout"));
                }
            };

            // 5. Process stream (tools update shared state via OrchestratorClient)
            let got_tool_call = self.process_agent_stream(stream).await?;

            // Track tokens from this iteration's session
            if let Ok(sess) = SessionManager::get_session(&iteration_session_id, false).await {
                let mut state = session_state.lock().await;
                state.add_tokens(
                    sess.accumulated_input_tokens,
                    sess.accumulated_output_tokens,
                );

                let input = sess
                    .accumulated_input_tokens
                    .map(|t| t.max(0) as u64)
                    .unwrap_or(0);
                let output = sess
                    .accumulated_output_tokens
                    .map(|t| t.max(0) as u64)
                    .unwrap_or(0);
                state.token_breakdown.add_orchestrator(input, output);
            }

            // 6. DETERMINISTIC FINALIZATION: Force completion when review passed
            // This is critical because the LLM may not always call finalize even when instructed to.
            // We enforce this deterministically to prevent wasted iterations after a passing review.
            {
                let mut state = session_state.lock().await;
                if state.last_review_passed && !state.requires_human_input_pending {
                    info!(
                        "Review passed (score passed threshold) - forcing completion (deterministic finalization)"
                    );
                    state.status = OrchestrationStatus::Completed;
                }
            }

            // 7. Check terminal state
            let is_terminal = {
                let state = session_state.lock().await;
                matches!(
                    state.status,
                    OrchestrationStatus::Completed
                        | OrchestrationStatus::Paused { .. }
                        | OrchestrationStatus::HardStopped { .. }
                        | OrchestrationStatus::Failed { .. }
                )
            };

            if is_terminal {
                info!("Orchestrator reached terminal state");
                break;
            }

            // Handle text-only response (no tool calls) - record and continue
            // With stateless iteration, we simply try again with fresh context
            if !got_tool_call {
                let mut state = session_state.lock().await;
                warn!(
                    "Iteration {}: No tool calls made, will retry with fresh context",
                    current_iteration
                );
                let record = IterationRecord {
                    iteration: state.iteration,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    viability_violations: 0,
                    viability_critical: 0,
                    viability_passed: false,
                    review_score: None,
                    review_passed: None,
                    tool_calls_this_iteration: 0,
                    tokens_this_iteration: 0,
                    outcome: IterationOutcome::TextResponseDetected,
                };
                state.iteration_history.push(record);
                state.iteration += 1; // Increment to avoid infinite loop
            }

            // No continuation prompt needed - next iteration creates fresh agent with full context
        }

        // Token tracking now happens per-iteration inside the loop
        // Check if we got any token data (for warning about estimation)
        {
            let state = session_state.lock().await;
            if state.token_breakdown.estimated {
                warn!(
                    "Token tracking not available from provider - budget enforcement may be inaccurate"
                );
            }
        }

        // Read final state and save
        let final_state = {
            let state = session_state.lock().await;
            state.clone()
        };
        final_state.save(&session_dir)?;

        // Write plan to dev/active/ for completed, best-effort, and paused states
        // - Completed: Final approved plan (passed review)
        // - CompletedBestEffort: Best plan seen (did not pass review threshold)
        // - Paused: Draft plan for user review before providing feedback
        let should_write_plan = matches!(
            final_state.status,
            OrchestrationStatus::Completed
                | OrchestrationStatus::CompletedBestEffort
                | OrchestrationStatus::Paused { .. }
        );

        if should_write_plan {
            // Use best_plan for CompletedBestEffort, otherwise current_plan
            let plan_to_write = if matches!(final_state.status, OrchestrationStatus::CompletedBestEffort) {
                final_state.best_plan.as_ref().or(final_state.current_plan.as_ref())
            } else {
                final_state.current_plan.as_ref()
            };

            if let Some(plan_json) = plan_to_write {
                match serde_json::from_value::<Plan>(plan_json.clone()) {
                    Ok(plan) => {
                        let output_config = OutputConfig {
                            runs_dir: session_dir.clone(),
                            active_dir: self.base_dir.join("dev/active"),
                            slug: Some(final_state.task_slug.clone()),
                        };
                        let output = FileOutputWriter::new(output_config);

                        // Determine status for output
                        let output_status = match &final_state.status {
                            OrchestrationStatus::Completed => crate::output::PlanStatus::Approved,
                            OrchestrationStatus::CompletedBestEffort => crate::output::PlanStatus::BestEffort {
                                score: final_state.best_score,
                            },
                            OrchestrationStatus::Paused { .. } => crate::output::PlanStatus::Draft,
                            _ => crate::output::PlanStatus::Draft,
                        };

                        if let Err(e) = output.write_final_with_plan_status(&plan, output_status).await {
                            warn!("Failed to write plan to dev/active/: {}", e);
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse plan JSON for output: {}", e);
                    }
                }
            }
        }

        info!(
            "Orchestrator session complete: status={:?}, iterations={}, tokens={}",
            final_state.status, final_state.iteration, final_state.total_tokens
        );

        // Log warning if orchestrator overhead is high
        let overhead = final_state.token_breakdown.overhead_ratio();
        if overhead > 0.2 {
            warn!(
                "High orchestrator overhead: {:.1}% of total tokens used by orchestrator agent",
                overhead * 100.0
            );
        }

        Ok(OrchestrationResult {
            final_plan: final_state.current_plan,
            best_plan: final_state.best_plan,
            best_score: final_state.best_score,
            status: final_state.status,
            iterations: final_state.iteration,
            tool_calls: final_state.tool_calls,
            total_tokens: final_state.total_tokens,
            session_id,
            token_breakdown: final_state.token_breakdown,
        })
    }

    /// Build comprehensive context message for a single orchestrator iteration.
    ///
    /// This function creates an explicit context message that includes all information
    /// the orchestrator needs to make a decision, without relying on conversation history.
    /// This enables stateless per-iteration operation where each iteration runs with a
    /// fresh agent.
    async fn build_iteration_context(
        &self,
        task: &str,
        session_state: &tokio::sync::Mutex<OrchestrationState>,
        max_iterations: u32,
    ) -> String {
        let state = session_state.lock().await;

        // Build iteration history summary (last 5 iterations)
        let iteration_history = if state.iteration_history.is_empty() {
            "(no previous iterations)".to_string()
        } else {
            state
                .iteration_history
                .iter()
                .rev()
                .take(5)
                .map(|r| {
                    format!(
                        "- Iter {}: score={}, viability={}, outcome={:?}",
                        r.iteration,
                        r.review_score
                            .map(|s| format!("{:.2}", s))
                            .unwrap_or_else(|| "-".to_string()),
                        if r.viability_passed { "pass" } else { "fail" },
                        r.outcome
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        // Get last review summary
        let last_review_summary = state
            .reviews
            .last()
            .map(|r| {
                // Extract key fields for a concise summary
                let passed = r.get("passed").and_then(|v| v.as_bool()).unwrap_or(false);
                let requires_human = r
                    .get("requires_human_input")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let summary = r
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(no summary)");

                format!(
                    "Passed: {}\nRequires human input: {}\nSummary: {}",
                    passed, requires_human, summary
                )
            })
            .unwrap_or_else(|| "(no previous review)".to_string());

        // Check if resuming from human input
        let human_input_context = if let Some(last_input) = state.human_inputs.last() {
            format!(
                "\n## Human Input Received\nQuestion: {}\nResponse: {}\nApproved: {}",
                last_input.question,
                last_input.response.as_deref().unwrap_or("(none)"),
                last_input.approved
            )
        } else {
            String::new()
        };

        // Check for pending human input
        let pending_context = if state.pending_human_input.is_some() {
            "\n## Note\nThere is a pending human input request. If you just received human input, proceed based on their response."
        } else {
            ""
        };

        format!(
            r#"## Task
{task}

## Current State
Iteration: {iteration}/{max_iterations}
Tokens used: {tokens}
Tool calls: {tool_calls}
Status: {status:?}
Needs review: {needs_review}
{human_input_context}{pending_context}

## Iteration History (recent)
{iteration_history}

## Last Review Result
{last_review_summary}

## Instructions
Based on the above context, take the appropriate action (check IN ORDER):
1. If iteration is 0 and no plan exists: call `plan-forge-orchestrator__generate_plan` with the task
2. If needs_review=true: call `plan-forge-orchestrator__review_plan` with the current plan (REQUIRED after every generate_plan)
3. If last review passed=true and requires_human_input=false: call `plan-forge-orchestrator__finalize`
4. If requires_human_input=true: call `plan-forge-orchestrator__request_human_input`
5. If last review passed=false: call `plan-forge-orchestrator__generate_plan` with feedback from the review

IMPORTANT: Respond ONLY with tool calls. Make your decision now."#,
            task = task,
            iteration = state.iteration,
            max_iterations = max_iterations,
            tokens = state.total_tokens,
            tool_calls = state.tool_calls,
            status = state.status,
            needs_review = state.needs_review,
            human_input_context = human_input_context,
            pending_context = pending_context,
            iteration_history = iteration_history,
            last_review_summary = last_review_summary,
        )
    }

    /// Process the agent event stream for a single iteration.
    ///
    /// Returns whether any tool calls were made during this iteration.
    /// Tool responses are handled by the OrchestratorClient extension.
    async fn process_agent_stream<S, E>(&self, mut stream: S) -> Result<bool>
    where
        S: futures::Stream<Item = Result<AgentEvent, E>> + Unpin,
        E: std::fmt::Debug,
    {
        let mut got_tool_call = false;

        while let Some(event) = stream.next().await {
            match event {
                Ok(AgentEvent::Message(msg)) => {
                    for content in &msg.content {
                        match content {
                            MessageContent::ToolResponse(_) | MessageContent::ToolRequest(_) => {
                                got_tool_call = true;
                            }
                            MessageContent::Text(text) => {
                                debug!(
                                    "Orchestrator: {}",
                                    text.text.chars().take(200).collect::<String>()
                                );
                            }
                            _ => {}
                        }
                    }
                }
                Ok(_) => {
                    // Other events (McpNotification, ModelChange, HistoryReplaced)
                }
                Err(e) => {
                    warn!("Stream error: {:?}", e);
                }
            }
        }

        Ok(got_tool_call)
    }
}

// ============================================================================
// LoopResult Conversion
// ============================================================================

use crate::models::{
    LlmReview, PlanContext, PlanMetadata, PlanTier, ReviewResult,
};
use crate::orchestrator::LoopResult;

impl From<OrchestrationResult> for LoopResult {
    fn from(result: OrchestrationResult) -> Self {
        // Try to parse Plan from the JSON value
        let final_plan = result
            .final_plan
            .as_ref()
            .and_then(|v| serde_json::from_value::<Plan>(v.clone()).ok())
            .unwrap_or_else(|| {
                // Fallback minimal Plan if JSON doesn't match schema
                let now = chrono::Utc::now().to_rfc3339();
                Plan {
                    title: result
                        .final_plan
                        .as_ref()
                        .and_then(|v| v.get("title"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("Orchestrated Plan")
                        .to_string(),
                    description: result
                        .final_plan
                        .as_ref()
                        .and_then(|v| v.get("description"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("Plan generated via orchestrator")
                        .to_string(),
                    goal: result
                        .final_plan
                        .as_ref()
                        .and_then(|v| v.get("goal"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    tier: PlanTier::Standard,
                    context: PlanContext {
                        problem_statement: String::new(),
                        constraints: vec![],
                        assumptions: vec![],
                        existing_patterns: vec![],
                    },
                    phases: vec![],
                    acceptance_criteria: vec![],
                    file_references: vec![],
                    risks: vec![],
                    metadata: PlanMetadata {
                        version: 1,
                        created_at: now.clone(),
                        last_updated: now,
                        iteration: result.iterations,
                    },
                    // ISA fields (optional)
                    reasoning: None,
                    operator_runbook: None,
                    grounding_gates: None,
                    grounding_snapshot: None,
                    instructions: None,
                }
            });

        // Create ReviewResult from status
        let final_review = ReviewResult {
            passed: matches!(result.status, OrchestrationStatus::Completed),
            summary: format!("Orchestrator completed with status: {:?}", result.status),
            hard_check_results: vec![],
            llm_review: LlmReview {
                overall_assessment: format!("Status: {:?}", result.status),
                gaps: vec![],
                unclear_areas: vec![],
                suggestions: vec![],
                score: if matches!(result.status, OrchestrationStatus::Completed) {
                    1.0
                } else {
                    0.0
                },
                requires_human_input: false,
                human_input_reason: None,
            },
        };

        LoopResult {
            final_plan,
            final_plan_json: result.final_plan,
            total_iterations: result.iterations,
            final_review,
            success: matches!(result.status, OrchestrationStatus::Completed),
            best_score: Some(result.best_score),
            total_tokens: Some(result.total_tokens),
        }
    }
}
