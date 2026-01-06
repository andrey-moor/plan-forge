use anyhow::Result;
use async_trait::async_trait;

use crate::models::{Plan, ReviewResult};
use crate::orchestrator::LoopState;

use super::{Planner, Updater};

/// Updater that delegates to the planner for updates
/// This exists as a separate trait for clarity, but uses the planner's update logic
pub struct PlannerUpdater<P: Planner> {
    planner: P,
}

impl<P: Planner> PlannerUpdater<P> {
    pub fn new(planner: P) -> Self {
        Self { planner }
    }
}

#[async_trait]
impl<P: Planner + Send + Sync> Updater for PlannerUpdater<P> {
    async fn update_plan(
        &self,
        _plan: &Plan,
        _review: &ReviewResult,
        state: &LoopState,
    ) -> Result<Plan> {
        // The planner already handles updates when state has feedback
        // This method exists for the trait interface, but we delegate to planner
        self.planner.generate_plan(state).await
    }
}
