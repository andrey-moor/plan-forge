mod agent_utils;
mod json_utils;
pub mod orchestrator;
pub mod planner;
pub mod reviewer;

pub use agent_utils::{create_provider, resolve_working_dir, setup_agent_session, ProviderConfig};
pub use json_utils::extract_json_block;
pub use orchestrator::*;
pub use planner::*;
pub use reviewer::*;

use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;

use crate::models::{Plan, ReviewResult};

/// Context for plan generation
#[derive(Debug, Clone)]
pub struct PlanningContext {
    /// The task to accomplish
    pub task: String,
    /// Current iteration number
    pub iteration: u32,
    /// Working directory for file operations
    pub working_dir: Option<PathBuf>,
    /// Feedback from previous reviews to incorporate
    pub pending_feedback: Vec<String>,
    /// Current plan (if updating)
    pub current_plan: Option<Plan>,
}

impl PlanningContext {
    /// Create a new planning context for initial plan generation
    pub fn new(task: String, working_dir: Option<PathBuf>) -> Self {
        Self {
            task,
            iteration: 1,
            working_dir,
            pending_feedback: Vec::new(),
            current_plan: None,
        }
    }

    /// Create a context for plan update with feedback
    pub fn with_feedback(
        task: String,
        iteration: u32,
        working_dir: Option<PathBuf>,
        pending_feedback: Vec<String>,
        current_plan: Option<Plan>,
    ) -> Self {
        Self {
            task,
            iteration,
            working_dir,
            pending_feedback,
            current_plan,
        }
    }
}

/// Context for plan review
#[derive(Debug, Clone)]
pub struct ReviewContext {
    /// Current iteration number
    pub iteration: u32,
    /// Working directory for file operations
    pub working_dir: Option<PathBuf>,
}

impl ReviewContext {
    /// Create a new review context
    pub fn new(iteration: u32, working_dir: Option<PathBuf>) -> Self {
        Self {
            iteration,
            working_dir,
        }
    }
}

/// Trait for the planning phase
#[async_trait]
pub trait Planner: Send + Sync {
    /// Generate or update a plan based on current context
    async fn generate_plan(&self, ctx: &PlanningContext) -> Result<Plan>;
}

/// Trait for the review phase
#[async_trait]
pub trait Reviewer: Send + Sync {
    /// Review a plan and return findings
    async fn review_plan(&self, plan: &Plan, ctx: &ReviewContext) -> Result<ReviewResult>;
}
