use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::fs;
use tracing::info;

use crate::config::OutputConfig;
use crate::models::{Plan, ReviewResult};
use crate::slugify;

use super::OutputWriter;

/// File-based output writer that generates markdown files
pub struct FileOutputWriter {
    config: OutputConfig,
}

impl FileOutputWriter {
    pub fn new(config: OutputConfig) -> Self {
        Self { config }
    }

    async fn ensure_runs_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.config.runs_dir)
            .await
            .context("Failed to create runs directory")
    }

    async fn ensure_active_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.config.active_dir)
            .await
            .context("Failed to create active output directory")
    }

    fn plan_to_markdown(&self, plan: &Plan) -> String {
        let mut md = String::new();

        // Header
        md.push_str(&format!("# {}\n\n", plan.title));
        md.push_str("**Status:** In Progress\n");
        md.push_str(&format!("**Created:** {}\n", plan.metadata.created_at));
        md.push_str(&format!(
            "**Last Updated:** {}\n",
            plan.metadata.last_updated
        ));
        md.push_str(&format!("**Iteration:** {}\n\n", plan.metadata.iteration));

        // Overview
        md.push_str("## Overview\n\n");
        md.push_str(&format!("{}\n\n", plan.description));

        // Context
        if !plan.context.problem_statement.is_empty() {
            md.push_str("## Current State\n\n");
            md.push_str(&format!("{}\n\n", plan.context.problem_statement));

            if !plan.context.constraints.is_empty() {
                md.push_str("**Constraints:**\n");
                for c in &plan.context.constraints {
                    md.push_str(&format!("- {}\n", c));
                }
                md.push('\n');
            }

            if !plan.context.assumptions.is_empty() {
                md.push_str("**Assumptions:**\n");
                for a in &plan.context.assumptions {
                    md.push_str(&format!("- {}\n", a));
                }
                md.push('\n');
            }
        }

        // Phases
        md.push_str("## Phases\n\n");
        for (i, phase) in plan.phases.iter().enumerate() {
            md.push_str(&format!("{}. **{}**: {}\n", i + 1, phase.name, phase.goal));
        }
        md.push('\n');

        // Key Files
        if !plan.file_references.is_empty() {
            md.push_str("## Key Files\n\n");
            for file in &plan.file_references {
                md.push_str(&format!("- `{}` - {}\n", file.path, file.description));
            }
            md.push('\n');
        }

        // Risks
        if !plan.risks.is_empty() {
            md.push_str("## Risks\n\n");
            for risk in &plan.risks {
                md.push_str(&format!(
                    "⚠️ **Risk**: {} - **Mitigation**: {}\n",
                    risk.description, risk.mitigation
                ));
            }
            md.push('\n');
        }

        // Success Criteria
        if !plan.acceptance_criteria.is_empty() {
            md.push_str("## Success Criteria\n\n");
            for criterion in &plan.acceptance_criteria {
                md.push_str(&format!("- {}\n", criterion.description));
            }
        }

        md
    }

    fn plan_to_tasks_markdown(&self, plan: &Plan) -> String {
        let mut md = String::new();

        md.push_str(&format!("# {} - Tasks\n\n", plan.title));

        // Progress summary
        let total_tasks: usize = plan
            .phases
            .iter()
            .flat_map(|p| &p.checkpoints)
            .map(|c| c.tasks.len())
            .sum();

        md.push_str("## Progress\n\n");
        md.push_str(&format!("- **Total**: {}\n", total_tasks));
        md.push_str("- **Completed**: 0 ✅\n");
        md.push_str("- **In Progress**: 0 🔄\n\n");
        md.push_str("---\n\n");

        // Phases and tasks
        for phase in &plan.phases {
            md.push_str(&format!("## {}\n\n", phase.name));

            for checkpoint in &phase.checkpoints {
                for task in &checkpoint.tasks {
                    md.push_str(&format!("- [ ] **{}**\n", task.description));

                    if !task.file_references.is_empty() {
                        md.push_str(&format!(
                            "  - Location: `{}`\n",
                            task.file_references.join("`, `")
                        ));
                    }

                    if let Some(notes) = &task.implementation_notes {
                        md.push_str(&format!("  - Notes: {}\n", notes));
                    }

                    if let Some(validation) = &checkpoint.validation {
                        md.push_str(&format!("  - Validation: {}\n", validation));
                    }

                    md.push('\n');
                }
            }
        }

        md.push_str("---\n");
        md.push_str(&format!("Progress: 0/{} tasks complete\n", total_tasks));

        md
    }

    fn plan_to_context_markdown(&self, plan: &Plan) -> String {
        let mut md = String::new();

        md.push_str(&format!("# {} - Context\n\n", plan.title));

        md.push_str("## Current State\n\n");
        md.push_str("- **Working on**: Phase 1\n");
        md.push_str(&format!(
            "- **Progress**: 0/{} tasks\n",
            plan.phases
                .iter()
                .flat_map(|p| &p.checkpoints)
                .map(|c| c.tasks.len())
                .sum::<usize>()
        ));
        md.push_str("- **Blockers**: None\n");
        md.push_str(&format!(
            "- **Last updated**: {}\n\n",
            plan.metadata.last_updated
        ));

        md.push_str("## Key Decisions\n\n");
        md.push_str("*To be populated during implementation*\n\n");

        md.push_str("## Dependencies\n\n");
        md.push_str("**External:**\n");
        md.push_str("*To be populated during implementation*\n\n");
        md.push_str("**Internal:**\n");
        md.push_str("*To be populated during implementation*\n\n");

        md.push_str("## Discoveries\n\n");
        md.push_str("*To be populated during implementation*\n\n");

        md.push_str("## Active Issues\n\n");
        md.push_str("*None*\n\n");

        md.push_str("## Handoff Notes\n\n");
        if let Some(first_phase) = plan.phases.first() {
            md.push_str(&format!(
                "**Immediate next action**: Start {}\n",
                first_phase.name
            ));
        }

        md
    }
}

#[async_trait]
impl OutputWriter for FileOutputWriter {
    async fn write_intermediate(&self, plan: &Plan, iteration: u32) -> Result<()> {
        self.ensure_runs_dir().await?;

        let filename = format!("plan-iteration-{}.json", iteration);
        let path = self.config.runs_dir.join(&filename);

        let json = serde_json::to_string_pretty(plan)?;
        fs::write(&path, json)
            .await
            .context("Failed to write intermediate plan")?;

        info!("Wrote intermediate plan to {:?}", path);
        Ok(())
    }

    async fn write_review(&self, review: &ReviewResult, iteration: u32) -> Result<()> {
        self.ensure_runs_dir().await?;

        let filename = format!("review-iteration-{}.json", iteration);
        let path = self.config.runs_dir.join(&filename);

        let json = serde_json::to_string_pretty(review)?;
        fs::write(&path, json)
            .await
            .context("Failed to write review")?;

        info!("Wrote review to {:?}", path);
        Ok(())
    }

    async fn write_final(&self, plan: &Plan) -> Result<()> {
        self.ensure_active_dir().await?;
        self.ensure_runs_dir().await?;

        // Use configured slug if available, otherwise derive from plan title
        let task_name = self
            .config
            .slug
            .clone()
            .unwrap_or_else(|| slugify(&plan.title));

        // Create task directory in active_dir for final MD files
        let task_dir = self.config.active_dir.join(&task_name);
        fs::create_dir_all(&task_dir).await?;

        // Always write 3-file structure (plan.md, tasks.md, context.md)
        let plan_path = task_dir.join(format!("{}-plan.md", task_name));
        fs::write(&plan_path, self.plan_to_markdown(plan)).await?;
        info!("Wrote {:?}", plan_path);

        let tasks_path = task_dir.join(format!("{}-tasks.md", task_name));
        fs::write(&tasks_path, self.plan_to_tasks_markdown(plan)).await?;
        info!("Wrote {:?}", tasks_path);

        let context_path = task_dir.join(format!("{}-context.md", task_name));
        fs::write(&context_path, self.plan_to_context_markdown(plan)).await?;
        info!("Wrote {:?}", context_path);

        // Write final JSON to runs_dir (service directory, not committed)
        let json_path = self
            .config
            .runs_dir
            .join(format!("{}-final.json", task_name));
        fs::write(&json_path, serde_json::to_string_pretty(plan)?).await?;
        info!("Wrote {:?}", json_path);

        Ok(())
    }
}
