use anyhow::Result;
use tracing::{info, warn};

use crate::config::CliConfig;
use crate::models::Plan;
use crate::output::OutputWriter;
use crate::phases::{Planner, Reviewer};

use super::state::{LoopResult, LoopState, ResumeState};

/// Result type for human input required pause
#[derive(Debug)]
pub struct HumanInputRequired {
    pub reason: String,
    pub task_slug: String,
}

/// Controls the plan-review-update feedback loop
#[deprecated(
    since = "0.2.0",
    note = "Use GooseOrchestrator with --use-orchestrator flag for LLM-powered orchestration"
)]
pub struct LoopController<P, R, O>
where
    P: Planner,
    R: Reviewer,
    O: OutputWriter,
{
    planner: P,
    reviewer: R,
    output: O,
    config: CliConfig,
    resume_state: Option<ResumeState>,
    task_slug: Option<String>,
}

#[allow(deprecated)]
impl<P, R, O> LoopController<P, R, O>
where
    P: Planner,
    R: Reviewer,
    O: OutputWriter,
{
    pub fn new(planner: P, reviewer: R, output: O, config: CliConfig) -> Self {
        Self {
            planner,
            reviewer,
            output,
            config,
            resume_state: None,
            task_slug: None,
        }
    }

    /// Set resume state to continue from an existing plan
    pub fn with_resume(mut self, state: ResumeState) -> Self {
        self.resume_state = Some(state);
        self
    }

    /// Set task slug for human input pause messages
    pub fn with_task_slug(mut self, slug: String) -> Self {
        self.task_slug = Some(slug);
        self
    }

    /// Run the complete feedback loop
    pub async fn run(&self, task: String, working_dir: Option<String>) -> Result<LoopResult> {
        let mut state = LoopState::new(task, self.config.loop_config.max_iterations, working_dir);

        // Handle resume state if provided
        if let Some(resume) = &self.resume_state {
            info!(
                "Resuming from existing plan (iteration {})",
                resume.start_iteration
            );
            state.iteration = resume.start_iteration.saturating_sub(1); // Will be incremented
            state.current_plan = Some(resume.plan.clone());

            // Add user feedback to pending feedback
            if !resume.feedback.is_empty() {
                state.conversation_context.pending_feedback = resume.feedback.clone();
                info!("User feedback to incorporate: {:?}", resume.feedback);
            }
        }

        info!("Starting plan-review-update loop");
        info!("Max iterations: {}", state.max_iterations);

        while state.should_continue() {
            state.next_iteration();
            info!(
                "=== Iteration {} of {} ===",
                state.iteration, state.max_iterations
            );

            // Phase 1: Generate or Update Plan
            // Skip planning if resuming with existing plan and no feedback
            let plan = if self.resume_state.is_some()
                && state.iteration == self.resume_state.as_ref().unwrap().start_iteration
                && self.resume_state.as_ref().unwrap().feedback.is_empty()
            {
                // Just re-run review on existing plan
                info!("Phase 1: Using existing plan (resume without feedback)");
                state.current_plan.clone().unwrap()
            } else {
                let plan = self.run_planning_phase(&state).await?;
                state.current_plan = Some(plan.clone());
                plan
            };

            // Write intermediate plan to runs directory
            self.output
                .write_intermediate(&plan, state.iteration)
                .await?;

            // Phase 2: Review Plan
            let review = self.run_review_phase(&plan, &state).await?;

            // Store review in history
            state.review_history.push(review.clone());

            // Write review output
            self.output.write_review(&review, state.iteration).await?;

            info!("Review summary: {}", review.summary);

            // Check if human input is required
            if review.llm_review.requires_human_input {
                let task_slug = self
                    .task_slug
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());
                let reason = review
                    .llm_review
                    .human_input_reason
                    .clone()
                    .unwrap_or_else(|| "Human verification required".to_string());

                warn!("Review requires human input: {}", reason);

                // Write to both runs/ (for resume) and dev/active/ (for user review)
                self.output.write_final(&plan).await?;

                return Err(anyhow::anyhow!(
                    "Human input required: {}\n\nReview the plan at: dev/active/{}/\nResume with: plan-forge run --path dev/active/{} --task \"your response\"",
                    reason,
                    task_slug,
                    task_slug
                ));
            }

            if review.passed {
                info!("Plan PASSED review!");

                // Check for early exit on perfect score
                if self.config.loop_config.early_exit_on_perfect_score
                    && review.llm_review.score >= 0.95
                {
                    info!("Perfect score achieved, exiting early");
                    break;
                }
                break;
            } else {
                warn!(
                    "Review found issues. Gaps: {}, Unclear areas: {}",
                    review.llm_review.gaps.len(),
                    review.llm_review.unclear_areas.len()
                );

                // Extract feedback for next iteration
                state.update_feedback_from_review(&review);
                info!(
                    "Extracted {} feedback items for next iteration",
                    state.conversation_context.pending_feedback.len()
                );
            }
        }

        // Write final output
        if let Some(plan) = &state.current_plan {
            self.output.write_final(plan).await?;
        }

        // Build result
        let result = LoopResult::new(&state)
            .ok_or_else(|| anyhow::anyhow!("No plan or review generated"))?;

        info!(
            "Loop completed after {} iterations. Success: {}",
            result.total_iterations, result.success
        );

        Ok(result)
    }

    async fn run_planning_phase(&self, state: &LoopState) -> Result<Plan> {
        if state.is_first_iteration() {
            info!("Phase 1: Generating initial plan...");
        } else {
            info!("Phase 1: Updating plan based on feedback...");
        }

        self.planner.generate_plan(state).await
    }

    async fn run_review_phase(
        &self,
        plan: &Plan,
        state: &LoopState,
    ) -> Result<crate::models::ReviewResult> {
        info!("Phase 2: Reviewing plan...");
        self.reviewer.review_plan(plan, state).await
    }
}
