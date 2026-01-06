pub mod planner;
pub mod reviewer;
pub mod updater;

pub use planner::*;
pub use reviewer::*;
pub use updater::*;

use anyhow::Result;
use async_trait::async_trait;

use crate::models::{Plan, ReviewResult};
use crate::orchestrator::LoopState;

/// Trait for the planning phase
#[async_trait]
pub trait Planner: Send + Sync {
    /// Generate or update a plan based on current state
    async fn generate_plan(&self, state: &LoopState) -> Result<Plan>;
}

/// Trait for the review phase
#[async_trait]
pub trait Reviewer: Send + Sync {
    /// Review a plan and return findings
    async fn review_plan(&self, plan: &Plan, state: &LoopState) -> Result<ReviewResult>;
}

/// Trait for incorporating feedback into plan updates
#[async_trait]
pub trait Updater: Send + Sync {
    /// Update plan based on review feedback
    async fn update_plan(
        &self,
        plan: &Plan,
        review: &ReviewResult,
        state: &LoopState,
    ) -> Result<Plan>;
}
