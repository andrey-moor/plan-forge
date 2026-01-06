use crate::models::{HardCheckResult, Plan, Severity};

/// A hard validation check that runs as Rust code
pub struct CheckDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub validator: fn(&Plan) -> HardCheckResult,
}

/// Collection of hard validation checks
pub struct HardChecklist {
    pub checks: Vec<CheckDefinition>,
}

impl Default for HardChecklist {
    fn default() -> Self {
        Self {
            checks: vec![
                CheckDefinition {
                    name: "has_acceptance_criteria",
                    description: "Plan must have at least one acceptance criterion",
                    validator: check_has_acceptance_criteria,
                },
                CheckDefinition {
                    name: "has_phases",
                    description: "Plan must have at least one phase",
                    validator: check_has_phases,
                },
                CheckDefinition {
                    name: "phases_have_tasks",
                    description: "Each phase must have at least one checkpoint with tasks",
                    validator: check_phases_have_tasks,
                },
                CheckDefinition {
                    name: "file_references_valid",
                    description: "File references must have valid paths",
                    validator: check_file_references_valid,
                },
                CheckDefinition {
                    name: "no_empty_descriptions",
                    description: "Tasks must have non-empty descriptions",
                    validator: check_no_empty_descriptions,
                },
                CheckDefinition {
                    name: "has_risks",
                    description: "Plan should identify potential risks",
                    validator: check_has_risks,
                },
            ],
        }
    }
}

impl HardChecklist {
    /// Run all checks against a plan
    pub fn run_all(&self, plan: &Plan) -> Vec<HardCheckResult> {
        self.checks.iter().map(|check| (check.validator)(plan)).collect()
    }
}

fn check_has_acceptance_criteria(plan: &Plan) -> HardCheckResult {
    let passed = !plan.acceptance_criteria.is_empty();
    HardCheckResult {
        check_name: "has_acceptance_criteria".to_string(),
        passed,
        message: if passed {
            format!("Found {} acceptance criteria", plan.acceptance_criteria.len())
        } else {
            "Plan has no acceptance criteria defined".to_string()
        },
        severity: Severity::Error,
    }
}

fn check_has_phases(plan: &Plan) -> HardCheckResult {
    let passed = !plan.phases.is_empty();
    HardCheckResult {
        check_name: "has_phases".to_string(),
        passed,
        message: if passed {
            format!("Found {} phases", plan.phases.len())
        } else {
            "Plan has no phases defined".to_string()
        },
        severity: Severity::Error,
    }
}

fn check_phases_have_tasks(plan: &Plan) -> HardCheckResult {
    let empty_phases: Vec<_> = plan
        .phases
        .iter()
        .filter(|p| p.checkpoints.is_empty() || p.checkpoints.iter().all(|c| c.tasks.is_empty()))
        .map(|p| p.name.clone())
        .collect();

    let passed = empty_phases.is_empty();
    HardCheckResult {
        check_name: "phases_have_tasks".to_string(),
        passed,
        message: if passed {
            "All phases have tasks".to_string()
        } else {
            format!("Phases without tasks: {}", empty_phases.join(", "))
        },
        severity: Severity::Error,
    }
}

fn check_file_references_valid(plan: &Plan) -> HardCheckResult {
    let invalid: Vec<_> = plan
        .file_references
        .iter()
        .filter(|f| f.path.is_empty() || f.path.contains(".."))
        .map(|f| f.path.clone())
        .collect();

    let passed = invalid.is_empty();
    HardCheckResult {
        check_name: "file_references_valid".to_string(),
        passed,
        message: if passed {
            format!("All {} file references are valid", plan.file_references.len())
        } else {
            format!("Invalid file references: {}", invalid.join(", "))
        },
        severity: Severity::Error,
    }
}

fn check_no_empty_descriptions(plan: &Plan) -> HardCheckResult {
    let mut empty_count = 0;

    for phase in &plan.phases {
        for checkpoint in &phase.checkpoints {
            for task in &checkpoint.tasks {
                if task.description.trim().is_empty() {
                    empty_count += 1;
                }
            }
        }
    }

    let passed = empty_count == 0;
    HardCheckResult {
        check_name: "no_empty_descriptions".to_string(),
        passed,
        message: if passed {
            "All tasks have descriptions".to_string()
        } else {
            format!("{} tasks have empty descriptions", empty_count)
        },
        severity: Severity::Warning,
    }
}

fn check_has_risks(plan: &Plan) -> HardCheckResult {
    let passed = !plan.risks.is_empty();
    HardCheckResult {
        check_name: "has_risks".to_string(),
        passed,
        message: if passed {
            format!("Identified {} risks", plan.risks.len())
        } else {
            "No risks identified - consider potential issues".to_string()
        },
        severity: Severity::Warning,
    }
}
