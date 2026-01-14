use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::fs;
use tracing::info;

use crate::config::OutputConfig;
use crate::models::{GroundingGate, GroundingSnapshot, Instruction, Plan, ReviewResult};
use crate::orchestrator::viability::{DagMetrics, analyze_dag};
use crate::slugify;

use super::OutputWriter;

/// Status indicator for plan output
#[derive(Debug, Clone)]
pub enum PlanStatus {
    /// Plan passed review and is approved
    Approved,
    /// Plan did not pass review threshold - best effort result
    BestEffort { score: f32 },
    /// Plan is a draft awaiting human input
    Draft,
}

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

    fn plan_to_markdown_with_status(&self, plan: &Plan, is_draft: bool) -> String {
        let status = if is_draft {
            PlanStatus::Draft
        } else {
            PlanStatus::Approved
        };
        self.plan_to_markdown_with_plan_status(plan, status)
    }

    fn plan_to_markdown_with_plan_status(&self, plan: &Plan, status: PlanStatus) -> String {
        let mut md = String::new();

        // =========================================================================
        // Header (clean, minimal - matches reference plan style)
        // =========================================================================
        md.push_str(&format!("# {}\n\n", plan.title));
        match status {
            PlanStatus::Approved => md.push_str("**Status**: Approved\n"),
            PlanStatus::BestEffort { score } => md.push_str(&format!(
                "**Status**: Best Effort (score {:.2} - did not pass 0.80 threshold)\n",
                score
            )),
            PlanStatus::Draft => md.push_str("**Status**: DRAFT - Awaiting Human Input\n"),
        }
        md.push_str(&format!("**Objective**: {}\n\n", plan.description));

        // =========================================================================
        // Operator Runbook (FIRST actionable section)
        // =========================================================================
        if let Some(runbook) = &plan.operator_runbook {
            md.push_str("## How to execute this plan (operator runbook)\n\n");
            md.push_str(runbook);
            md.push_str("\n\n---\n\n");
        }

        // =========================================================================
        // Phase 0.0: Grounding Gate (verify before any coding)
        // =========================================================================
        if let Some(gates) = &plan.grounding_gates
            && !gates.is_empty()
        {
            md.push_str(&self.render_grounding_gates(gates));
        }

        // =========================================================================
        // Implementation Phases (main content - full task details)
        // =========================================================================
        for (i, phase) in plan.phases.iter().enumerate() {
            // Strip "Phase N:" prefix if present to avoid duplication (e.g., "Phase 0: Phase 0: Foundation")
            let clean_name = if phase.name.starts_with("Phase ") {
                phase
                    .name
                    .split_once(": ")
                    .map(|(_, rest)| rest)
                    .unwrap_or(&phase.name)
            } else {
                &phase.name
            };
            md.push_str(&format!("## Phase {}: {}\n\n", i, clean_name));
            md.push_str(&format!("**Goal**: {}\n\n", phase.goal));

            for checkpoint in &phase.checkpoints {
                // Checkpoint as checkbox header
                md.push_str(&format!(
                    "- [ ] **{} {}**\n",
                    checkpoint.id, checkpoint.description
                ));

                // Tasks with file references inline
                for task in &checkpoint.tasks {
                    md.push_str(&format!("  - {}\n", task.description));

                    // File references as markdown links
                    for file_ref in &task.file_references {
                        if file_ref.contains("](") {
                            // Already formatted as markdown link
                            md.push_str(&format!("    - {}\n", file_ref));
                        } else {
                            // Convert to markdown link format
                            md.push_str(&format!("    - [`{}`]({}:1)\n", file_ref, file_ref));
                        }
                    }

                    if let Some(notes) = &task.implementation_notes {
                        md.push_str(&format!("  - **Note**: {}\n", notes));
                    }
                }

                // Validation criteria for checkpoint
                if let Some(validation) = &checkpoint.validation {
                    md.push_str(&format!("  - **Validation**: {}\n", validation));
                }
                md.push('\n');
            }
        }

        md.push_str("---\n\n");

        // =========================================================================
        // Acceptance Criteria (testable success conditions)
        // =========================================================================
        if !plan.acceptance_criteria.is_empty() {
            md.push_str("## Acceptance Criteria\n\n");
            for criterion in &plan.acceptance_criteria {
                let priority = match criterion.priority {
                    crate::models::Priority::Required => "required",
                    crate::models::Priority::Recommended => "recommended",
                    crate::models::Priority::Optional => "optional",
                };
                let testable = if criterion.testable {
                    "testable"
                } else {
                    "manual"
                };
                md.push_str(&format!(
                    "- [ ] {} ({}, {})\n",
                    criterion.description, priority, testable
                ));
            }
            md.push('\n');
        }

        // =========================================================================
        // Risks & Mitigations
        // =========================================================================
        if !plan.risks.is_empty() {
            md.push_str("## Risks\n\n");
            for risk in &plan.risks {
                let severity = match risk.severity {
                    crate::models::Severity::Error => "HIGH",
                    crate::models::Severity::Warning => "MEDIUM",
                    crate::models::Severity::Info => "LOW",
                };
                md.push_str(&format!("- **[{}]** {}\n", severity, risk.description));
                md.push_str(&format!("  - **Mitigation**: {}\n\n", risk.mitigation));
            }
        }

        // =========================================================================
        // Appendix: Technical Details (for reference/automation)
        // =========================================================================
        let has_appendix = plan.reasoning.is_some()
            || plan.grounding_snapshot.is_some()
            || (plan.instructions.is_some()
                && plan.instructions.as_ref().is_some_and(|i| !i.is_empty()))
            || !plan.context.problem_statement.is_empty()
            || !plan.context.constraints.is_empty()
            || !plan.context.existing_patterns.is_empty();

        if has_appendix {
            md.push_str("---\n\n");
            md.push_str("## Appendix\n\n");

            // Context (constraints, assumptions, existing patterns)
            if !plan.context.problem_statement.is_empty()
                || !plan.context.constraints.is_empty()
                || !plan.context.assumptions.is_empty()
                || !plan.context.existing_patterns.is_empty()
            {
                md.push_str("### Context\n\n");

                if !plan.context.problem_statement.is_empty() {
                    md.push_str(&format!("{}\n\n", plan.context.problem_statement));
                }

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

                if !plan.context.existing_patterns.is_empty() {
                    md.push_str("**Existing Patterns:**\n");
                    for p in &plan.context.existing_patterns {
                        md.push_str(&format!("- {}\n", p));
                    }
                    md.push('\n');
                }
            }

            // Planning Reasoning (self-verification notes)
            if let Some(reasoning) = &plan.reasoning {
                md.push_str("### Planning Reasoning\n\n");
                md.push_str(reasoning);
                md.push_str("\n\n");
            }

            // Grounding Evidence (verification data)
            if let Some(grounding) = &plan.grounding_snapshot {
                md.push_str(&self.render_grounding_snapshot(grounding));
            }

            // Execution Instructions (ISA DAG for automation)
            if let Some(instructions) = &plan.instructions
                && !instructions.is_empty()
            {
                md.push_str(&self.render_instructions(instructions));
            }
        }

        md
    }

    /// Render grounding gates (Phase 0.0) to markdown
    fn render_grounding_gates(&self, gates: &[GroundingGate]) -> String {
        let mut md = String::new();

        md.push_str("## Phase 0.0: Grounding Gate\n\n");
        md.push_str("**Goal**: Verify repo reality before any implementation work begins.\n\n");

        for gate in gates {
            md.push_str(&format!("- [ ] **{} {}**\n", gate.id, gate.verification));
            md.push_str(&format!("  - **Pass criteria**: {}\n", gate.pass_criteria));
            md.push_str(&format!("  - **Rule**: {}\n", gate.rule));
            md.push('\n');
        }

        md.push_str("---\n\n");
        md
    }

    /// Render grounding snapshot to markdown (as appendix subsection)
    fn render_grounding_snapshot(&self, snapshot: &GroundingSnapshot) -> String {
        let mut md = String::new();

        md.push_str("### Grounding Evidence\n\n");

        // Verified files
        if !snapshot.verified_files.is_empty() {
            md.push_str("**Verified Files:**\n\n");
            md.push_str("| Path | Exists |\n");
            md.push_str("|------|--------|\n");
            for file in &snapshot.verified_files {
                let status = if file.exists { "✅" } else { "❌" };
                md.push_str(&format!("| `{}` | {} |\n", file.path, status));
            }
            md.push('\n');
        }

        // Verified targets
        if !snapshot.verified_targets.is_empty() {
            md.push_str("**Verified Targets:**\n\n");
            md.push_str("| Target | Resolves |\n");
            md.push_str("|--------|----------|\n");
            for target in &snapshot.verified_targets {
                let status = if target.resolves { "✅" } else { "❌" };
                md.push_str(&format!("| `{}` | {} |\n", target.target, status));
            }
            md.push('\n');
        }

        // Import convention
        if let Some(convention) = &snapshot.import_convention {
            md.push_str("**Import Convention:**\n\n");
            md.push_str(&format!("```\n{}\n```\n\n", convention));
        }

        // Existing patterns
        if !snapshot.existing_patterns.is_empty() {
            md.push_str("**Existing Patterns:**\n\n");
            for pattern in &snapshot.existing_patterns {
                md.push_str(&format!(
                    "- **{}** at [`{}:{}`]({}:{})\n",
                    pattern.pattern, pattern.file, pattern.line, pattern.file, pattern.line
                ));
            }
            md.push('\n');
        }

        md
    }

    /// Escape special characters for mermaid diagram labels
    fn escape_mermaid(&self, text: &str) -> String {
        text.replace('"', "'")
            .replace('[', "&#91;")
            .replace(']', "&#93;")
            .replace('(', "&#40;")
            .replace(')', "&#41;")
            .replace('{', "&#123;")
            .replace('}', "&#125;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('|', "&#124;")
    }

    /// Render instruction DAG to markdown with mermaid diagram (as appendix subsection)
    fn render_instructions(&self, instructions: &[Instruction]) -> String {
        let mut md = String::new();

        md.push_str("### Execution Instructions (ISA)\n\n");

        // Compute and show DAG parallelization metrics
        let metrics = analyze_dag(instructions);
        self.render_dag_metrics(&mut md, &metrics);

        // Mermaid DAG diagram
        md.push_str("```mermaid\ngraph TD\n");
        for instr in instructions {
            // Node with description (use serde for SCREAMING_SNAKE_CASE OpCodes)
            let label = serde_json::to_string(&instr.op)
                .unwrap_or_else(|_| format!("{:?}", instr.op))
                .trim_matches('"')
                .to_string();
            let desc = self.escape_mermaid(&instr.description);
            // Use <br/> for line breaks (works in all mermaid versions)
            md.push_str(&format!("    {}[\"{}<br/>{}\"]\n", instr.id, label, desc));

            // Edges from dependencies
            for dep in &instr.dependencies {
                md.push_str(&format!("    {} --> {}\n", dep, instr.id));
            }
        }
        md.push_str("```\n\n");

        // Detailed instruction list
        md.push_str("**Instruction Details:**\n\n");
        for (i, instr) in instructions.iter().enumerate() {
            md.push_str(&format!("{}. **{}** (`{:?}`)\n", i + 1, instr.id, instr.op));
            md.push_str(&format!("   - {}\n", instr.description));
            if !instr.dependencies.is_empty() {
                md.push_str(&format!(
                    "   - Depends on: {}\n",
                    instr.dependencies.join(", ")
                ));
            }
            // Show params if non-empty and non-null
            if !instr.params.is_null()
                && instr.params != serde_json::json!({})
                && !instr.params.as_object().is_some_and(|o| o.is_empty())
            {
                // Format params compactly for readability
                if let Ok(params_str) = serde_json::to_string(&instr.params) {
                    if params_str.len() <= 100 {
                        md.push_str(&format!("   - Params: `{}`\n", params_str));
                    } else {
                        // For longer params, use multi-line format
                        if let Ok(pretty) = serde_json::to_string_pretty(&instr.params) {
                            md.push_str("   - Params:\n");
                            md.push_str("     ```json\n");
                            for line in pretty.lines() {
                                md.push_str(&format!("     {}\n", line));
                            }
                            md.push_str("     ```\n");
                        }
                    }
                }
            }
            // Show estimated tokens if present
            if let Some(tokens) = instr.estimated_tokens {
                md.push_str(&format!("   - Estimated tokens: {}\n", tokens));
            }
        }
        md.push('\n');

        md
    }

    /// Render DAG parallelization metrics as a markdown table
    fn render_dag_metrics(&self, md: &mut String, metrics: &DagMetrics) {
        if metrics.total_nodes == 0 {
            return;
        }

        md.push_str("**DAG Parallelization Metrics:**\n\n");
        md.push_str("| Metric | Value |\n");
        md.push_str("|--------|-------|\n");
        md.push_str(&format!(
            "| Total Instructions | {} |\n",
            metrics.total_nodes
        ));
        md.push_str(&format!(
            "| Root Nodes (parallel start) | {} |\n",
            metrics.root_nodes
        ));
        md.push_str(&format!(
            "| Critical Path Length | {} |\n",
            metrics.critical_path_length
        ));
        md.push_str(&format!(
            "| Max Concurrent Operations | {} |\n",
            metrics.max_width
        ));
        md.push_str(&format!(
            "| Parallelization Ratio | {:.2} |\n",
            metrics.parallelization_ratio
        ));

        if !metrics.unnecessary_deps.is_empty() {
            md.push_str(&format!(
                "| Unnecessary Dependencies | {} |\n",
                metrics.unnecessary_deps.len()
            ));
        }
        md.push('\n');

        // Show warning for low parallelization
        if metrics.total_nodes > 10 && metrics.parallelization_ratio < 1.0 {
            md.push_str("> **Note**: Parallelization ratio is low. Consider restructuring to allow more parallel execution.\n\n");
        }
    }

    /// Write final plan with optional draft status indicator
    pub async fn write_final_with_status(&self, plan: &Plan, is_draft: bool) -> Result<()> {
        self.ensure_active_dir().await?;
        self.ensure_runs_dir().await?;

        // Use configured slug if available, otherwise derive from plan title
        let task_name = self
            .config
            .slug
            .clone()
            .unwrap_or_else(|| slugify(&plan.title));

        // Create task directory in active_dir for final output
        let task_dir = self.config.active_dir.join(&task_name);
        fs::create_dir_all(&task_dir).await?;

        // Write single consolidated execution plan (matches reference plan format)
        let plan_path = task_dir.join(format!("{}-plan.md", task_name));
        fs::write(
            &plan_path,
            self.plan_to_markdown_with_status(plan, is_draft),
        )
        .await?;
        info!("Wrote {:?}", plan_path);

        // Write execution DAG JSON to active_dir (for automation/execution)
        // This is the machine-readable ISA DAG that downstream tools can consume
        if let Some(instructions) = &plan.instructions
            && !instructions.is_empty()
        {
            let dag_path = task_dir.join(format!("{}-dag.json", task_name));
            let dag_content = serde_json::json!({
                "goal": plan.goal(), // Use accessor that falls back to title
                "reasoning": plan.reasoning.clone(),
                "instructions": instructions
            });
            fs::write(&dag_path, serde_json::to_string_pretty(&dag_content)?).await?;
            info!("Wrote {:?}", dag_path);
        }

        // Write final JSON to runs_dir (for machine processing, not committed)
        let json_path = self
            .config
            .runs_dir
            .join(format!("{}-final.json", task_name));
        fs::write(&json_path, serde_json::to_string_pretty(plan)?).await?;
        info!("Wrote {:?}", json_path);

        Ok(())
    }

    /// Write final plan with explicit status (approved, best-effort, draft)
    pub async fn write_final_with_plan_status(
        &self,
        plan: &Plan,
        status: PlanStatus,
    ) -> Result<()> {
        self.ensure_active_dir().await?;
        self.ensure_runs_dir().await?;

        // Use configured slug if available, otherwise derive from plan title
        let task_name = self
            .config
            .slug
            .clone()
            .unwrap_or_else(|| slugify(&plan.title));

        // Create task directory in active_dir for final output
        let task_dir = self.config.active_dir.join(&task_name);
        fs::create_dir_all(&task_dir).await?;

        // Write single consolidated execution plan (matches reference plan format)
        let plan_path = task_dir.join(format!("{}-plan.md", task_name));
        fs::write(
            &plan_path,
            self.plan_to_markdown_with_plan_status(plan, status),
        )
        .await?;
        info!("Wrote {:?}", plan_path);

        // Write execution DAG JSON to active_dir (for automation/execution)
        // This is the machine-readable ISA DAG that downstream tools can consume
        if let Some(instructions) = &plan.instructions
            && !instructions.is_empty()
        {
            let dag_path = task_dir.join(format!("{}-dag.json", task_name));
            let dag_content = serde_json::json!({
                "goal": plan.goal(), // Use accessor that falls back to title
                "reasoning": plan.reasoning.clone(),
                "instructions": instructions
            });
            fs::write(&dag_path, serde_json::to_string_pretty(&dag_content)?).await?;
            info!("Wrote {:?}", dag_path);
        }

        // Write final JSON to runs_dir (for machine processing, not committed)
        let json_path = self
            .config
            .runs_dir
            .join(format!("{}-final.json", task_name));
        fs::write(&json_path, serde_json::to_string_pretty(plan)?).await?;
        info!("Wrote {:?}", json_path);

        Ok(())
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
        self.write_final_with_status(plan, false).await
    }
}
