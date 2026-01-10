use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::{Plan, ReviewResult};
use super::guardrails::MandatoryCondition;

/// State for resuming from an existing plan
#[derive(Debug, Clone)]
pub struct ResumeState {
    /// The existing plan to resume from
    pub plan: Plan,
    /// User feedback to incorporate
    pub feedback: Vec<String>,
    /// The iteration to start from
    pub start_iteration: u32,
}

/// State maintained across the feedback loop
#[derive(Debug, Clone)]
pub struct LoopState {
    /// Current iteration number (starts at 0, incremented before each iteration)
    pub iteration: u32,
    /// Maximum allowed iterations
    pub max_iterations: u32,
    /// The current plan (None before first generation)
    pub current_plan: Option<Plan>,
    /// History of all reviews performed
    pub review_history: Vec<ReviewResult>,
    /// Context preserved across iterations
    pub conversation_context: ConversationContext,
}

/// Context that needs to be preserved and passed between iterations
#[derive(Debug, Clone)]
pub struct ConversationContext {
    /// Key insights discovered during planning
    pub preserved_context: Vec<String>,
    /// Review feedback to incorporate in next iteration
    pub pending_feedback: Vec<String>,
    /// The original task description
    pub original_task: String,
    /// Working directory for the planning task
    pub working_dir: Option<String>,
}

impl LoopState {
    /// Create a new loop state for a given task
    pub fn new(task: String, max_iterations: u32, working_dir: Option<String>) -> Self {
        Self {
            iteration: 0,
            max_iterations,
            current_plan: None,
            review_history: Vec::new(),
            conversation_context: ConversationContext {
                preserved_context: Vec::new(),
                pending_feedback: Vec::new(),
                original_task: task,
                working_dir,
            },
        }
    }

    /// Check if the loop should continue
    pub fn should_continue(&self) -> bool {
        // Stop if we've reached max iterations
        if self.iteration >= self.max_iterations {
            return false;
        }

        // Continue if no review yet (first iteration)
        if self.review_history.is_empty() {
            return true;
        }

        // Continue if the latest review failed
        self.review_history
            .last()
            .map(|r| !r.passed)
            .unwrap_or(true)
    }

    /// Increment iteration counter
    pub fn next_iteration(&mut self) {
        self.iteration += 1;
    }

    /// Update pending feedback from the latest review
    #[allow(deprecated)]
    pub fn update_feedback_from_review(&mut self, review: &ReviewResult) {
        self.conversation_context.pending_feedback = review.extract_feedback();
    }

    /// Get the latest review result
    pub fn latest_review(&self) -> Option<&ReviewResult> {
        self.review_history.last()
    }

    /// Check if this is the first iteration
    pub fn is_first_iteration(&self) -> bool {
        self.iteration == 1
    }
}

/// Result of the entire loop execution
#[derive(Debug, Serialize, Deserialize)]
pub struct LoopResult {
    /// The final plan (parsed from JSON)
    pub final_plan: Plan,
    /// The final plan as raw JSON (for orchestrator mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_plan_json: Option<Value>,
    /// Total iterations completed
    pub total_iterations: u32,
    /// The final review result
    pub final_review: ReviewResult,
    /// Whether the session was successful
    pub success: bool,
    /// Triggered mandatory conditions (orchestrator mode only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triggered_conditions: Option<Vec<MandatoryCondition>>,
    /// Total tokens consumed (orchestrator mode only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

impl LoopResult {
    pub fn new(state: &LoopState) -> Option<Self> {
        let plan = state.current_plan.clone()?;
        let review = state.review_history.last()?.clone();

        Some(Self {
            final_plan: plan,
            final_plan_json: None,
            total_iterations: state.iteration,
            final_review: review.clone(),
            success: review.passed,
            triggered_conditions: None,
            total_tokens: None,
        })
    }
}
