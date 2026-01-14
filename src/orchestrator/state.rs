use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::{Plan, ReviewResult};

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
    /// Best review score achieved (orchestrator mode only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_score: Option<f32>,
    /// Total tokens consumed (orchestrator mode only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

impl LoopResult {
    /// Create a LoopResult from orchestration data
    pub fn from_orchestration(
        plan: Plan,
        plan_json: Option<Value>,
        iterations: u32,
        review: ReviewResult,
        best_score: Option<f32>,
        total_tokens: Option<u64>,
    ) -> Self {
        Self {
            final_plan: plan,
            final_plan_json: plan_json,
            total_iterations: iterations,
            success: review.passed,
            final_review: review,
            best_score,
            total_tokens,
        }
    }
}
