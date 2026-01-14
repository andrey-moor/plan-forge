//! Instruction validation checks (V-004, V-005, V-009, V-013, V-014).
//!
//! - V-004: Complexity checks
//! - V-005: Params presence
//! - V-009: Params schema validation
//! - V-013: AgentTask params validation
//! - V-014: Empty instructions check

use crate::models::{Instruction, OpCode, STEP_RESULT_FIELDS};

use super::{ViabilityChecker, ViabilitySeverity, ViabilityViolation};

impl ViabilityChecker {
    /// V-004: Check instruction complexity
    ///
    /// - EDIT_CODE should touch a limited number of files
    /// - SEARCH_CODE should have a specific query (not too short)
    pub fn check_complexity(&self, instructions: &[Instruction]) -> Vec<ViabilityViolation> {
        let mut violations = Vec::new();

        for instr in instructions {
            match instr.op {
                OpCode::EditCode => {
                    // Check if params specifies multiple files
                    if let Some(files) = instr.params.get("files")
                        && let Some(arr) = files.as_array()
                        && arr.len() > self.max_files_per_edit
                    {
                        violations.push(ViabilityViolation {
                            rule_id: "VIABILITY-004".to_string(),
                            instruction_id: Some(instr.id.clone()),
                            severity: ViabilitySeverity::Warning,
                            message: format!(
                                "EDIT_CODE instruction '{}' touches {} files (max {})",
                                instr.id,
                                arr.len(),
                                self.max_files_per_edit
                            ),
                            remediation: "Consider splitting into multiple EDIT_CODE instructions"
                                .to_string(),
                        });
                    }
                }
                OpCode::SearchCode => {
                    // Check query length
                    if let Some(query) = instr.params.get("query")
                        && let Some(q) = query.as_str()
                        && q.len() < self.min_search_query_length
                    {
                        violations.push(ViabilityViolation {
                            rule_id: "VIABILITY-004".to_string(),
                            instruction_id: Some(instr.id.clone()),
                            severity: ViabilitySeverity::Warning,
                            message: format!(
                                "SEARCH_CODE query '{}' is too short (min {} chars)",
                                q, self.min_search_query_length
                            ),
                            remediation: "Use a more specific search query".to_string(),
                        });
                    }
                }
                _ => {}
            }
        }

        violations
    }

    /// V-005: Check that instructions have meaningful params
    ///
    /// Each instruction should have params relevant to its OpCode:
    /// - SEARCH_CODE: must have "query"
    /// - READ_FILES: must have "paths" or use variable reference
    /// - EDIT_CODE: must have "goal" or "files"
    /// - RUN_TEST: must have "target" or variable reference
    pub fn check_params_presence(&self, instructions: &[Instruction]) -> Vec<ViabilityViolation> {
        let mut violations = Vec::new();

        for instr in instructions {
            let missing = match instr.op {
                OpCode::SearchCode | OpCode::SearchSemantic => instr.params.get("query").is_none(),
                OpCode::ReadFiles => {
                    instr.params.get("paths").is_none() && !self.has_variable_ref(&instr.params)
                }
                OpCode::EditCode => {
                    instr.params.get("goal").is_none() && instr.params.get("files").is_none()
                }
                OpCode::RunTest | OpCode::GenerateTest => {
                    instr.params.get("target").is_none()
                        && instr.params.get("behavior").is_none()
                        && !self.has_variable_ref(&instr.params)
                }
                OpCode::RunCommand => instr.params.get("command").is_none(),
                OpCode::VerifyExists => instr.params.get("path").is_none(),
                _ => false,
            };

            if missing {
                violations.push(ViabilityViolation {
                    rule_id: "VIABILITY-005".to_string(),
                    instruction_id: Some(instr.id.clone()),
                    severity: ViabilitySeverity::Warning,
                    message: format!(
                        "Instruction '{}' ({:?}) missing required params",
                        instr.id, instr.op
                    ),
                    remediation: format!("Add appropriate params for {:?} operation", instr.op),
                });
            }
        }

        violations
    }

    /// Check if params contain variable references like ${id.field}
    /// Valid fields: output, stdout, stderr, exit_code, artifacts, metadata
    pub(crate) fn has_variable_ref(&self, params: &serde_json::Value) -> bool {
        let json_str = params.to_string();
        if !json_str.contains("${") {
            return false;
        }
        // Accept any valid StepResult field reference
        STEP_RESULT_FIELDS
            .iter()
            .any(|field| json_str.contains(&format!(".{}", field)))
    }

    /// V-009: Validate params schema for each OpCode
    ///
    /// Check that instruction params have the correct types for their operation.
    pub fn check_params_schema(&self, instructions: &[Instruction]) -> Vec<ViabilityViolation> {
        let mut violations = Vec::new();

        for instr in instructions {
            match instr.op {
                OpCode::SearchCode | OpCode::SearchSemantic => {
                    if let Some(query) = instr.params.get("query")
                        && !query.is_string()
                        && !self.is_variable_ref_value(query)
                    {
                        violations.push(self.schema_violation(&instr.id, "query", "string", query));
                    }
                    if let Some(limit) = instr.params.get("limit")
                        && !limit.is_u64()
                    {
                        violations.push(self.schema_violation(&instr.id, "limit", "number", limit));
                    }
                }
                OpCode::ReadFiles => {
                    if let Some(paths) = instr.params.get("paths")
                        && !paths.is_array()
                        && !paths.is_string()
                        && !self.is_variable_ref_value(paths)
                    {
                        violations.push(self.schema_violation(
                            &instr.id,
                            "paths",
                            "array or string or variable reference",
                            paths,
                        ));
                    }
                }
                OpCode::EditCode => {
                    if let Some(goal) = instr.params.get("goal")
                        && !goal.is_string()
                        && !self.is_variable_ref_value(goal)
                    {
                        violations.push(self.schema_violation(&instr.id, "goal", "string", goal));
                    }
                    if let Some(files) = instr.params.get("files")
                        && !files.is_array()
                        && !self.is_variable_ref_value(files)
                    {
                        violations.push(self.schema_violation(
                            &instr.id,
                            "files",
                            "array or variable reference",
                            files,
                        ));
                    }
                }
                OpCode::RunCommand => {
                    if let Some(command) = instr.params.get("command")
                        && !command.is_string()
                        && !self.is_variable_ref_value(command)
                    {
                        violations
                            .push(self.schema_violation(&instr.id, "command", "string", command));
                    }
                }
                OpCode::RunTest | OpCode::GenerateTest => {
                    if let Some(target) = instr.params.get("target")
                        && !target.is_string()
                        && !self.is_variable_ref_value(target)
                    {
                        violations.push(self.schema_violation(
                            &instr.id,
                            "target",
                            "string or variable reference",
                            target,
                        ));
                    }
                    if let Some(behavior) = instr.params.get("behavior")
                        && !behavior.is_string()
                    {
                        violations
                            .push(self.schema_violation(&instr.id, "behavior", "string", behavior));
                    }
                }
                OpCode::VerifyExists => {
                    if let Some(path) = instr.params.get("path")
                        && !path.is_string()
                        && !self.is_variable_ref_value(path)
                    {
                        violations.push(self.schema_violation(
                            &instr.id,
                            "path",
                            "string or variable reference",
                            path,
                        ));
                    }
                }
                OpCode::GetDependencies => {
                    if let Some(path) = instr.params.get("path")
                        && !path.is_string()
                        && !self.is_variable_ref_value(path)
                    {
                        violations.push(self.schema_violation(
                            &instr.id,
                            "path",
                            "string or variable reference",
                            path,
                        ));
                    }
                }
                OpCode::DefineTask | OpCode::VerifyTask => {
                    // These have flexible schemas - skip strict validation
                }
            }
        }

        violations
    }

    /// Check if a JSON value is a variable reference string like "${id.field}"
    pub(crate) fn is_variable_ref_value(&self, value: &serde_json::Value) -> bool {
        if let Some(s) = value.as_str() {
            s.starts_with("${") && s.contains('.') && s.ends_with('}')
        } else {
            false
        }
    }

    /// Create a schema violation for a param type mismatch
    pub(crate) fn schema_violation(
        &self,
        instr_id: &str,
        param_name: &str,
        expected_type: &str,
        actual_value: &serde_json::Value,
    ) -> ViabilityViolation {
        let actual_type = match actual_value {
            serde_json::Value::Null => "null",
            serde_json::Value::Bool(_) => "boolean",
            serde_json::Value::Number(_) => "number",
            serde_json::Value::String(_) => "string",
            serde_json::Value::Array(_) => "array",
            serde_json::Value::Object(_) => "object",
        };

        ViabilityViolation {
            rule_id: "VIABILITY-009".to_string(),
            instruction_id: Some(instr_id.to_string()),
            severity: ViabilitySeverity::Warning,
            message: format!(
                "Instruction '{}' param '{}' should be {} but got {}",
                instr_id, param_name, expected_type, actual_type
            ),
            remediation: format!("Change '{}' to be a {}", param_name, expected_type),
        }
    }

    /// V-013: EDIT_CODE and GENERATE_TEST must use AgentTask params schema
    ///
    /// Per design doc, AgentTask required fields: role, goal, context_files, constraints
    pub fn check_agent_task_params(&self, instructions: &[Instruction]) -> Vec<ViabilityViolation> {
        let mut violations = Vec::new();

        for instr in instructions {
            if !matches!(instr.op, OpCode::EditCode | OpCode::GenerateTest) {
                continue;
            }

            // Check for required 'goal' or 'task' field (task is an alias for goal)
            let has_goal_or_task = instr
                .params
                .get("goal")
                .or_else(|| instr.params.get("task"))
                .map(|v| v.is_string())
                .unwrap_or(false);

            if !has_goal_or_task {
                violations.push(ViabilityViolation {
                    rule_id: "VIABILITY-013".to_string(),
                    instruction_id: Some(instr.id.clone()),
                    severity: ViabilitySeverity::Critical,
                    message: format!(
                        "Instruction '{}' ({:?}) missing required 'goal' or 'task' param",
                        instr.id, instr.op
                    ),
                    remediation:
                        "Add AgentTask params: goal (required), role, context_files, constraints"
                            .to_string(),
                });
            }

            // Check for other required fields per design doc (Warning level)
            let missing_fields: Vec<&str> = ["role", "context_files", "constraints"]
                .into_iter()
                .filter(|field| instr.params.get(*field).is_none())
                .collect();

            if !missing_fields.is_empty() {
                violations.push(ViabilityViolation {
                    rule_id: "VIABILITY-013".to_string(),
                    instruction_id: Some(instr.id.clone()),
                    severity: ViabilitySeverity::Warning,
                    message: format!(
                        "Instruction '{}' ({:?}) missing AgentTask fields: {:?}",
                        instr.id, instr.op, missing_fields
                    ),
                    remediation: format!(
                        "Add missing fields: {}. Per design doc, AgentTask requires: role, goal, context_files, constraints",
                        missing_fields.join(", ")
                    ),
                });
            }

            // Check for legacy/wrong schema (action, content_description)
            if instr.params.get("action").is_some()
                || instr.params.get("content_description").is_some()
            {
                violations.push(ViabilityViolation {
                    rule_id: "VIABILITY-013".to_string(),
                    instruction_id: Some(instr.id.clone()),
                    severity: ViabilitySeverity::Critical,
                    message: format!(
                        "Instruction '{}' uses legacy params (action/content_description) instead of AgentTask schema",
                        instr.id
                    ),
                    remediation: "Replace with: goal, role, context_files, files, constraints"
                        .to_string(),
                });
            }
        }

        violations
    }

    /// V-014: Check that instructions array is not empty
    ///
    /// Plans MUST have executable instructions.
    pub fn check_empty_instructions(
        &self,
        instructions: &[Instruction],
    ) -> Option<ViabilityViolation> {
        if instructions.is_empty() {
            Some(ViabilityViolation {
                rule_id: "VIABILITY-014".to_string(),
                instruction_id: None,
                severity: ViabilitySeverity::Critical,
                message: "Instructions array is empty - plan has no executable instructions"
                    .to_string(),
                remediation: "Plans MUST have instructions for execution. If viability violations \
                    were reported, FIX the instructions rather than removing them. Generate \
                    instructions following the ISA pattern: SEARCH_CODE → READ_FILES → \
                    GENERATE_TEST → RUN_TEST (expect fail) → EDIT_CODE → RUN_TEST (expect pass)."
                    .to_string(),
            })
        } else {
            None
        }
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

    // V-004: Complexity Tests

    #[test]
    fn test_v004_valid_complexity() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "edit_1".to_string(),
            op: OpCode::EditCode,
            params: serde_json::json!({
                "goal": "Fix bug",
                "files": ["src/lib.rs"]
            }),
            ..Default::default()
        }];

        let violations = checker.check_complexity(&instructions);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v004_too_many_files() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "edit_1".to_string(),
            op: OpCode::EditCode,
            params: serde_json::json!({
                "files": ["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"]
            }),
            ..Default::default()
        }];

        let violations = checker.check_complexity(&instructions);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("touches 5 files"));
    }

    #[test]
    fn test_v004_short_query() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "search_1".to_string(),
            op: OpCode::SearchCode,
            params: serde_json::json!({
                "query": "ab"
            }),
            ..Default::default()
        }];

        let violations = checker.check_complexity(&instructions);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("too short"));
    }

    // V-005: Params Presence Tests

    #[test]
    fn test_v005_missing_query_param() {
        let checker = ViabilityChecker::new();
        let instructions = vec![make_instruction("search_1", OpCode::SearchCode, vec![])];

        let violations = checker.check_params_presence(&instructions);
        assert!(!violations.is_empty());
        assert_eq!(violations[0].rule_id, "VIABILITY-005");
    }

    #[test]
    fn test_v005_with_query_param_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "search_1".to_string(),
            op: OpCode::SearchCode,
            params: serde_json::json!({ "query": "function definition" }),
            ..Default::default()
        }];

        let violations = checker.check_params_presence(&instructions);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v005_edit_code_needs_goal_or_files() {
        let checker = ViabilityChecker::new();
        let instructions = vec![make_instruction("edit_1", OpCode::EditCode, vec![])];

        let violations = checker.check_params_presence(&instructions);
        assert!(!violations.is_empty());
    }

    #[test]
    fn test_v005_edit_code_with_goal_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "edit_1".to_string(),
            op: OpCode::EditCode,
            params: serde_json::json!({ "goal": "Fix the bug" }),
            ..Default::default()
        }];

        let violations = checker.check_params_presence(&instructions);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v005_read_files_with_variable_ref_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "read_1".to_string(),
            op: OpCode::ReadFiles,
            params: serde_json::json!({ "paths": "${search_1.artifacts}" }),
            ..Default::default()
        }];

        let violations = checker.check_params_presence(&instructions);
        assert!(violations.is_empty());
    }

    // V-009: Params Schema Tests

    #[test]
    fn test_v009_edit_code_goal_string_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "edit_1".to_string(),
            op: OpCode::EditCode,
            params: serde_json::json!({ "goal": "Fix bug" }),
            ..Default::default()
        }];

        let violations = checker.check_params_schema(&instructions);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v009_edit_code_goal_wrong_type() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "edit_1".to_string(),
            op: OpCode::EditCode,
            params: serde_json::json!({ "goal": 123 }),
            ..Default::default()
        }];

        let violations = checker.check_params_schema(&instructions);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("should be string"));
    }

    #[test]
    fn test_v009_edit_code_files_array_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "edit_1".to_string(),
            op: OpCode::EditCode,
            params: serde_json::json!({ "files": ["a.rs", "b.rs"] }),
            ..Default::default()
        }];

        let violations = checker.check_params_schema(&instructions);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v009_read_files_paths_array_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "read_1".to_string(),
            op: OpCode::ReadFiles,
            params: serde_json::json!({ "paths": ["src/lib.rs"] }),
            ..Default::default()
        }];

        let violations = checker.check_params_schema(&instructions);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v009_read_files_paths_variable_ref_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "read_1".to_string(),
            op: OpCode::ReadFiles,
            params: serde_json::json!({ "paths": "${search_1.artifacts}" }),
            ..Default::default()
        }];

        let violations = checker.check_params_schema(&instructions);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v009_multiple_violations() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            Instruction {
                id: "search_1".to_string(),
                op: OpCode::SearchCode,
                params: serde_json::json!({ "query": 123, "limit": "ten" }),
                ..Default::default()
            },
            Instruction {
                id: "edit_1".to_string(),
                op: OpCode::EditCode,
                params: serde_json::json!({ "goal": [], "files": "not_array" }),
                ..Default::default()
            },
        ];

        let violations = checker.check_params_schema(&instructions);
        assert!(violations.len() >= 4);
    }

    // V-013: AgentTask Params Tests

    #[test]
    fn test_v013_edit_code_missing_goal() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "edit_1".to_string(),
            op: OpCode::EditCode,
            params: serde_json::json!({ "files": ["src/lib.rs"] }),
            ..Default::default()
        }];

        let violations = checker.check_agent_task_params(&instructions);
        // Should have Critical for missing goal, Warning for missing other fields
        assert!(
            violations
                .iter()
                .any(|v| v.severity == ViabilitySeverity::Critical
                    && v.message.contains("missing required 'goal'"))
        );
    }

    #[test]
    fn test_v013_edit_code_with_goal_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![Instruction {
            id: "edit_1".to_string(),
            op: OpCode::EditCode,
            params: serde_json::json!({
                "goal": "Fix the authentication bug",
                "role": "senior developer",
                "context_files": ["src/auth.rs"],
                "constraints": ["no breaking changes"]
            }),
            ..Default::default()
        }];

        let violations = checker.check_agent_task_params(&instructions);
        // No critical violations (warnings for missing fields are ok if goal present)
        assert!(
            violations
                .iter()
                .all(|v| v.severity != ViabilitySeverity::Critical)
        );
    }

    // V-014: Empty Instructions Tests

    #[test]
    fn test_v014_empty_instructions() {
        let checker = ViabilityChecker::new();
        let instructions: Vec<Instruction> = vec![];

        let violation = checker.check_empty_instructions(&instructions);
        assert!(violation.is_some());
        assert_eq!(violation.unwrap().rule_id, "VIABILITY-014");
    }

    #[test]
    fn test_v014_non_empty_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![make_instruction("step_1", OpCode::SearchCode, vec![])];

        let violation = checker.check_empty_instructions(&instructions);
        assert!(violation.is_none());
    }
}
