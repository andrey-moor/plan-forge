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
use crate::output::{FileOutputWriter, OutputWriter};
use crate::orchestrator::{
    create_orchestrator_client, register_orchestrator_extension, GuardrailHardStop, Guardrails,
    HumanResponse, MandatoryCondition, OrchestrationState, OrchestrationStatus, SessionRegistry,
    TokenBreakdown,
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
    /// Final status of the session
    pub status: OrchestrationStatus,
    /// Number of iterations completed
    pub iterations: u32,
    /// Total tool calls made
    pub tool_calls: u32,
    /// Total tokens consumed
    pub total_tokens: u64,
    /// Triggered mandatory conditions
    pub triggered_conditions: Vec<MandatoryCondition>,
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

        let planner = Arc::new(GoosePlanner::new(
            crate::config::PlanningConfig {
                recipe: PathBuf::from("recipes/planner.yaml"),
                provider_override: None,
                model_override: None,
            },
            self.base_dir.clone(),
        ));

        let reviewer = Arc::new(GooseReviewer::new(
            crate::config::ReviewConfig {
                recipe: PathBuf::from("recipes/reviewer.yaml"),
                provider_override: None,
                model_override: None,
                pass_threshold: 0.8,
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
                    condition: pending.condition.clone(),
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

        // Load orchestrator recipe
        let recipe = load_recipe(&self.config.recipe, &self.base_dir, "orchestrator")?;

        // Create provider and agent
        let provider = self.create_provider(&recipe).await?;
        let agent = Agent::new();

        // Create session for the orchestrator agent
        let orchestrator_session = SessionManager::create_session(
            working_dir_path.clone(),
            format!("orchestrator-{}", chrono::Utc::now().timestamp()),
            SessionType::Hidden,
        )
        .await
        .context("Failed to create orchestrator session")?;

        let orchestrator_session_id = orchestrator_session.id.clone();
        agent.update_provider(provider, &orchestrator_session_id).await?;

        // Apply recipe instructions
        if let Some(instructions) = &recipe.instructions {
            agent.override_system_prompt(instructions.clone()).await;
        }

        // CRITICAL: Create and register the orchestrator extension BEFORE agent.reply()
        let orchestrator_client = create_orchestrator_client(
            session_id.clone(),
            session_state.clone(),
            guardrails.clone(),
            planner,
            reviewer,
        );

        register_orchestrator_extension(&agent.extension_manager, orchestrator_client).await;

        info!("Orchestrator extension registered with agent");

        // Build the initial prompt
        let prompt = self.build_prompt(&task, &session_state).await;

        let session_config = SessionConfig {
            id: orchestrator_session_id.clone(),
            schedule_id: None,
            max_turns: Some(200), // Allow many turns for orchestrator
            retry_config: None,
        };

        let user_message = Message::user().with_text(&prompt);

        // Run agent with timeout
        let timeout_duration =
            Duration::from_secs(self.guardrails_config.execution_timeout_secs);

        let stream_result = tokio::time::timeout(
            timeout_duration,
            agent.reply(user_message, session_config, None),
        )
        .await;

        let mut stream = match stream_result {
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

        // Process the agent stream, parsing tool responses for status detection
        while let Some(event) = stream.next().await {
            match event {
                Ok(AgentEvent::Message(msg)) => {
                    // Parse tool responses for terminal state detection
                    for content in &msg.content {
                        match content {
                            MessageContent::ToolResponse(response) => {
                                // Try to parse the tool result to detect terminal states
                                if let Ok(result) = &response.tool_result {
                                    if let Some(content) = result.content.first() {
                                        if let Some(text) = content.raw.as_text() {
                                            // Parse JSON to detect status
                                            if let Ok(json) = serde_json::from_str::<Value>(&text.text) {
                                                // Detect request_human_input response
                                                if json.get("status").and_then(|s| s.as_str()) == Some("paused") {
                                                    debug!("Detected paused status from tool response");
                                                }
                                                // Detect finalize response
                                                if json.get("success").and_then(|s| s.as_bool()) == Some(true) {
                                                    debug!("Detected successful finalization");
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            MessageContent::Text(text) => {
                                debug!("Orchestrator: {}", text.text.chars().take(200).collect::<String>());
                            }
                            _ => {}
                        }
                    }
                }
                Ok(_) => {
                    // Other events (McpNotification, ModelChange, HistoryReplaced)
                }
                Err(e) => {
                    warn!("Orchestrator stream error: {:?}", e);
                }
            }
        }

        // Get token usage from orchestrator session and track in breakdown
        if let Ok(sess) = SessionManager::get_session(&orchestrator_session_id, false).await {
            let mut state = session_state.lock().await;

            // Check if token tracking is available
            let estimated = sess.accumulated_input_tokens.is_none() && sess.accumulated_output_tokens.is_none();
            if estimated {
                warn!(
                    "Token tracking not available from provider - budget enforcement may be inaccurate"
                );
                state.token_breakdown.estimated = true;
            }

            // Add to total tokens
            state.add_tokens(sess.accumulated_input_tokens, sess.accumulated_output_tokens);

            // Track orchestrator tokens in breakdown
            let input = sess.accumulated_input_tokens.map(|t| t.max(0) as u64).unwrap_or(0);
            let output = sess.accumulated_output_tokens.map(|t| t.max(0) as u64).unwrap_or(0);
            state.token_breakdown.add_orchestrator(input, output);
        }

        // Read final state and save
        let final_state = {
            let state = session_state.lock().await;
            state.clone()
        };
        final_state.save(&session_dir)?;

        // Write final output to dev/active/ if plan was completed
        if matches!(final_state.status, OrchestrationStatus::Completed) {
            if let Some(plan_json) = &final_state.current_plan {
                match serde_json::from_value::<Plan>(plan_json.clone()) {
                    Ok(plan) => {
                        let output_config = OutputConfig {
                            runs_dir: session_dir.clone(),
                            active_dir: self.base_dir.join("dev/active"),
                            slug: Some(final_state.task_slug.clone()),
                        };
                        let output = FileOutputWriter::new(output_config);

                        if let Err(e) = output.write_final(&plan).await {
                            warn!("Failed to write final output to dev/active/: {}", e);
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
            status: final_state.status,
            iterations: final_state.iteration,
            tool_calls: final_state.tool_calls,
            total_tokens: final_state.total_tokens,
            triggered_conditions: final_state.triggered_conditions,
            session_id,
            token_breakdown: final_state.token_breakdown,
        })
    }

    /// Build the initial prompt for the orchestrator agent.
    async fn build_prompt(
        &self,
        task: &str,
        session_state: &tokio::sync::Mutex<OrchestrationState>,
    ) -> String {
        let state = session_state.lock().await;

        if matches!(state.status, OrchestrationStatus::Paused { .. }) {
            // Resume prompt
            let last_input = state.human_inputs.last();
            format!(
                r#"Resuming orchestration session.

## Original Task
{}

## Session Context
{}

## Human Input Received
{}

Please continue the planning workflow from where you left off.
If the human approved, you may proceed to finalize if the plan passes review.
If not approved, incorporate the feedback and regenerate the plan."#,
                task,
                state.context_summary,
                last_input
                    .map(|i| {
                        format!(
                            "Question: {}\nResponse: {}\nApproved: {}",
                            i.question,
                            i.response.as_deref().unwrap_or("(none)"),
                            i.approved
                        )
                    })
                    .unwrap_or_else(|| "(no prior input)".to_string())
            )
        } else {
            // Initial prompt
            format!(
                r#"You are the Plan-Forge orchestrator. Generate a comprehensive development plan for the following task.

## Task
{}

## Instructions
1. First, call plan-forge-orchestrator__check_limits to verify your token budget.
2. Call plan-forge-orchestrator__generate_plan with the task to create an initial plan.
3. Call plan-forge-orchestrator__review_plan to evaluate the plan.
4. Based on the review:
   - If passed=true and requires_human_input=false, call plan-forge-orchestrator__finalize
   - If requires_human_input=true, call plan-forge-orchestrator__request_human_input
   - If passed=false, incorporate feedback and regenerate
5. Repeat until finalized or a stopping condition is reached.

Begin by checking your limits, then generate the initial plan."#,
                task
            )
        }
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
            triggered_conditions: Some(result.triggered_conditions),
            total_tokens: Some(result.total_tokens),
        }
    }
}
