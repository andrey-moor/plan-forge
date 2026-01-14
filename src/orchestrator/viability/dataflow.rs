//! Data flow validation checks (V-006, V-007, V-008).
//!
//! - V-006: Variable references must have dependencies
//! - V-007: TDD order compliance
//! - V-008: Variable field name validation

use std::collections::HashSet;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::models::{Instruction, OpCode, STEP_RESULT_FIELDS};

use super::{ViabilityChecker, ViabilitySeverity, ViabilityViolation};

/// Pattern to match ${instruction_id.field} variable references (V-006)
static VAR_REF_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\$\{([a-zA-Z0-9_-]+)\.([a-zA-Z0-9_]+)\}").expect("invalid VAR_REF_PATTERN regex")
});

/// Pattern to match ${instruction_id.field} for field validation (V-008)
static VAR_FIELD_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\$\{(\w+)\.(\w+)\}").expect("invalid VAR_FIELD_PATTERN regex")
});

impl ViabilityChecker {
    /// V-006: Check that variable references have corresponding dependencies
    ///
    /// If instruction B uses ${A.output}, B must list A in its dependencies.
    pub fn check_variable_refs(&self, instructions: &[Instruction]) -> Vec<ViabilityViolation> {
        let mut violations = Vec::new();

        // Collect all valid instruction IDs for reference validation
        let valid_ids: HashSet<&str> = instructions.iter().map(|i| i.id.as_str()).collect();

        for instr in instructions {
            let params_str = instr.params.to_string();

            // Find all variable references in params
            for cap in VAR_REF_PATTERN.captures_iter(&params_str) {
                let referenced_id = cap.get(1).unwrap().as_str();

                // Skip if referencing non-existent instruction (V-002 handles this)
                if !valid_ids.contains(referenced_id) {
                    continue;
                }

                // Check if the referenced instruction is in dependencies
                if !instr.dependencies.iter().any(|d| d == referenced_id) {
                    violations.push(ViabilityViolation {
                        rule_id: "VIABILITY-006".to_string(),
                        instruction_id: Some(instr.id.clone()),
                        severity: ViabilitySeverity::Critical,
                        message: format!(
                            "Instruction '{}' references ${{{}.*}} but doesn't depend on '{}'",
                            instr.id, referenced_id, referenced_id
                        ),
                        remediation: format!(
                            "Add '{}' to dependencies array to ensure proper execution order",
                            referenced_id
                        ),
                    });
                }
            }
        }

        violations
    }

    /// V-007: Check TDD pattern compliance
    ///
    /// For EDIT_CODE instructions, verify TDD order:
    /// 1. GENERATE_TEST should come before EDIT_CODE
    /// 2. RUN_TEST (expecting failure) should be between GENERATE_TEST and EDIT_CODE
    /// 3. RUN_TEST (expecting success) should follow EDIT_CODE
    pub fn check_tdd_order(&self, instructions: &[Instruction]) -> Vec<ViabilityViolation> {
        let mut violations = Vec::new();

        // Find EDIT_CODE instructions
        let edit_indices: Vec<usize> = instructions
            .iter()
            .enumerate()
            .filter(|(_, i)| i.op == OpCode::EditCode)
            .map(|(idx, _)| idx)
            .collect();

        for edit_idx in edit_indices {
            // Check if there's a GENERATE_TEST before this EDIT_CODE
            let has_test_before = instructions[..edit_idx]
                .iter()
                .any(|i| i.op == OpCode::GenerateTest);

            // Check if there's a RUN_TEST after this EDIT_CODE
            let has_test_after = instructions[edit_idx..]
                .iter()
                .any(|i| i.op == OpCode::RunTest);

            if !has_test_before && has_test_after {
                violations.push(ViabilityViolation {
                    rule_id: "VIABILITY-007".to_string(),
                    instruction_id: Some(instructions[edit_idx].id.clone()),
                    severity: ViabilitySeverity::Warning,
                    message: format!(
                        "EDIT_CODE '{}' has tests after but not before (violates TDD Red-Green pattern)",
                        instructions[edit_idx].id
                    ),
                    remediation:
                        "Add GENERATE_TEST and initial RUN_TEST (expect failure) before EDIT_CODE"
                            .to_string(),
                });
            }
        }

        violations
    }

    /// V-008: Validate variable reference field names
    ///
    /// Check that all ${instruction_id.field} references use valid field names.
    /// Valid fields: output, stdout, stderr, exit_code, artifacts, metadata
    pub fn check_variable_field_names(
        &self,
        instructions: &[Instruction],
    ) -> Vec<ViabilityViolation> {
        let mut violations = Vec::new();

        for instr in instructions {
            let params_str = instr.params.to_string();

            for cap in VAR_FIELD_PATTERN.captures_iter(&params_str) {
                let field = &cap[2];
                if !STEP_RESULT_FIELDS.contains(&field) {
                    violations.push(ViabilityViolation {
                        rule_id: "VIABILITY-008".to_string(),
                        instruction_id: Some(instr.id.clone()),
                        severity: ViabilitySeverity::Warning,
                        message: format!(
                            "Instruction '{}' uses invalid variable field '{}' - valid fields: {:?}",
                            instr.id, field, STEP_RESULT_FIELDS
                        ),
                        remediation: format!(
                            "Use a valid StepResult field: {}",
                            STEP_RESULT_FIELDS.join(", ")
                        ),
                    });
                }
            }
        }

        violations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_instruction(id: &str, op: OpCode, deps: Vec<&str>) -> Instruction {
        Instruction {
            id: id.to_string(),
            op,
            params: serde_json::json!({}),
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            description: format!("Test instruction {}", id),
            ..Default::default()
        }
    }

    // V-006: Variable Reference Tests

    #[test]
    fn test_v006_no_refs_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("step_1", OpCode::SearchCode, vec![]),
            make_instruction("step_2", OpCode::ReadFiles, vec!["step_1"]),
        ];

        let violations = checker.check_variable_refs(&instructions);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v006_ref_with_dependency_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("step_1", OpCode::SearchCode, vec![]),
            Instruction {
                id: "step_2".to_string(),
                op: OpCode::ReadFiles,
                params: serde_json::json!({ "paths": "${step_1.artifacts}" }),
                dependencies: vec!["step_1".to_string()],
                ..Default::default()
            },
        ];

        let violations = checker.check_variable_refs(&instructions);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v006_ref_to_nonexistent_instruction_ok() {
        let checker = ViabilityChecker::new();
        // Reference to non-existent instruction is handled by V-002, not V-006
        let instructions = vec![Instruction {
            id: "step_1".to_string(),
            op: OpCode::ReadFiles,
            params: serde_json::json!({ "paths": "${nonexistent.artifacts}" }),
            dependencies: vec![],
            ..Default::default()
        }];

        let violations = checker.check_variable_refs(&instructions);
        assert!(violations.is_empty()); // V-006 skips unknown refs
    }

    // V-007: TDD Order Tests

    #[test]
    fn test_v007_tdd_pattern_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("test_gen", OpCode::GenerateTest, vec![]),
            make_instruction("test_run_1", OpCode::RunTest, vec!["test_gen"]),
            make_instruction("edit", OpCode::EditCode, vec!["test_run_1"]),
            make_instruction("test_run_2", OpCode::RunTest, vec!["edit"]),
        ];

        let violations = checker.check_tdd_order(&instructions);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v007_edit_without_test_before() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("search", OpCode::SearchCode, vec![]),
            make_instruction("edit", OpCode::EditCode, vec!["search"]),
            make_instruction("test_run", OpCode::RunTest, vec!["edit"]),
        ];

        let violations = checker.check_tdd_order(&instructions);
        assert!(!violations.is_empty());
        assert_eq!(violations[0].rule_id, "VIABILITY-007");
    }

    #[test]
    fn test_v007_edit_without_any_test_ok() {
        let checker = ViabilityChecker::new();
        // This is caught by V-001, not V-007
        let instructions = vec![
            make_instruction("search", OpCode::SearchCode, vec![]),
            make_instruction("edit", OpCode::EditCode, vec!["search"]),
        ];

        let violations = checker.check_tdd_order(&instructions);
        assert!(violations.is_empty()); // V-007 only triggers if there's a test AFTER
    }

    // V-008: Variable Field Names Tests

    #[test]
    fn test_v008_no_variable_refs_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![make_instruction("step_1", OpCode::SearchCode, vec![])];

        let violations = checker.check_variable_field_names(&instructions);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v008_valid_output_field() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "step_1".to_string(),
            op: OpCode::ReadFiles,
            params: serde_json::json!({ "paths": "${search.output}" }),
            ..Default::default()
        }];

        let violations = checker.check_variable_field_names(&instructions);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v008_valid_artifacts_field() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "step_1".to_string(),
            op: OpCode::ReadFiles,
            params: serde_json::json!({ "paths": "${search.artifacts}" }),
            ..Default::default()
        }];

        let violations = checker.check_variable_field_names(&instructions);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v008_valid_stdout_field() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "step_1".to_string(),
            op: OpCode::ReadFiles,
            params: serde_json::json!({ "paths": "${cmd.stdout}" }),
            ..Default::default()
        }];

        let violations = checker.check_variable_field_names(&instructions);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v008_invalid_field_name() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "step_1".to_string(),
            op: OpCode::ReadFiles,
            params: serde_json::json!({ "paths": "${search.invalid_field}" }),
            ..Default::default()
        }];

        let violations = checker.check_variable_field_names(&instructions);
        assert!(!violations.is_empty());
        assert_eq!(violations[0].rule_id, "VIABILITY-008");
        assert!(violations[0].message.contains("invalid_field"));
    }

    #[test]
    fn test_v008_multiple_invalid_fields() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "step_1".to_string(),
            op: OpCode::EditCode,
            params: serde_json::json!({
                "goal": "${a.bad_field}",
                "files": ["${b.wrong}"]
            }),
            ..Default::default()
        }];

        let violations = checker.check_variable_field_names(&instructions);
        assert_eq!(violations.len(), 2);
    }
}
