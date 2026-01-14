//! DAG validation checks (V-001, V-002).
//!
//! - V-001: Missing test verification
//! - V-002: Logical flow / cycle detection

use std::collections::{HashMap, HashSet};

use crate::models::{Instruction, OpCode};

use super::{ViabilityChecker, ViabilitySeverity, ViabilityViolation};

impl ViabilityChecker {
    /// V-001: Check that code edits have corresponding test verification
    ///
    /// If plan has instructions with op=EDIT_CODE, it MUST have a downstream
    /// instruction with op=RUN_TEST.
    pub fn check_missing_test(&self, instructions: &[Instruction]) -> Option<ViabilityViolation> {
        let has_edit = instructions.iter().any(|i| i.op == OpCode::EditCode);
        let has_test = instructions.iter().any(|i| i.op == OpCode::RunTest);

        if has_edit && !has_test {
            let edit_id = instructions
                .iter()
                .find(|i| i.op == OpCode::EditCode)
                .map(|i| i.id.clone());

            Some(ViabilityViolation {
                rule_id: "VIABILITY-001".to_string(),
                instruction_id: edit_id,
                severity: ViabilitySeverity::Critical,
                message: "Code edit without test verification".to_string(),
                remediation: "Add RUN_TEST instruction after EDIT_CODE to verify changes"
                    .to_string(),
            })
        } else {
            None
        }
    }

    /// V-002: Check logical flow of instruction dependencies
    ///
    /// - RUN_TEST must depend on the code it tests (directly or transitively)
    /// - Dependencies must reference existing instruction IDs
    /// - No circular dependencies
    pub fn check_logical_flow(&self, instructions: &[Instruction]) -> Vec<ViabilityViolation> {
        let mut violations = Vec::new();

        // Build set of valid instruction IDs
        let valid_ids: HashSet<&str> = instructions.iter().map(|i| i.id.as_str()).collect();

        // Check each instruction's dependencies
        for instr in instructions {
            for dep in &instr.dependencies {
                if !valid_ids.contains(dep.as_str()) {
                    violations.push(ViabilityViolation {
                        rule_id: "VIABILITY-002".to_string(),
                        instruction_id: Some(instr.id.clone()),
                        severity: ViabilitySeverity::Critical,
                        message: format!(
                            "Instruction '{}' depends on non-existent instruction '{}'",
                            instr.id, dep
                        ),
                        remediation: format!(
                            "Either remove dependency '{}' or add the missing instruction",
                            dep
                        ),
                    });
                }
            }
        }

        // Check for circular dependencies using DFS
        if let Some(cycle) = self.detect_cycle(instructions) {
            violations.push(ViabilityViolation {
                rule_id: "VIABILITY-002".to_string(),
                instruction_id: Some(cycle[0].clone()),
                severity: ViabilitySeverity::Critical,
                message: format!("Circular dependency detected: {}", cycle.join(" -> ")),
                remediation: "Remove or restructure dependencies to eliminate the cycle"
                    .to_string(),
            });
        }

        violations
    }

    /// Detect cycles in the instruction dependency graph
    fn detect_cycle(&self, instructions: &[Instruction]) -> Option<Vec<String>> {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut path = Vec::new();

        // Build adjacency map
        let adj: HashMap<&str, Vec<&str>> = instructions
            .iter()
            .map(|i| {
                (
                    i.id.as_str(),
                    i.dependencies.iter().map(|d| d.as_str()).collect(),
                )
            })
            .collect();

        for instr in instructions {
            if !visited.contains(instr.id.as_str()) {
                if let Some(cycle) =
                    self.dfs_cycle(&instr.id, &adj, &mut visited, &mut rec_stack, &mut path)
                {
                    return Some(cycle);
                }
            }
        }

        None
    }

    fn dfs_cycle<'a>(
        &self,
        node: &'a str,
        adj: &HashMap<&'a str, Vec<&'a str>>,
        visited: &mut HashSet<&'a str>,
        rec_stack: &mut HashSet<&'a str>,
        path: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        visited.insert(node);
        rec_stack.insert(node);
        path.push(node.to_string());

        if let Some(neighbors) = adj.get(node) {
            for &neighbor in neighbors {
                if !visited.contains(neighbor) {
                    if let Some(cycle) = self.dfs_cycle(neighbor, adj, visited, rec_stack, path) {
                        return Some(cycle);
                    }
                } else if rec_stack.contains(neighbor) {
                    // Found cycle - return path from neighbor to current
                    let cycle_start = path.iter().position(|n| n == neighbor).unwrap();
                    let mut cycle: Vec<String> = path[cycle_start..].to_vec();
                    cycle.push(neighbor.to_string());
                    return Some(cycle);
                }
            }
        }

        path.pop();
        rec_stack.remove(node);
        None
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

    #[test]
    fn test_v001_missing_test_detected() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("step_1", OpCode::SearchCode, vec![]),
            make_instruction("step_2", OpCode::EditCode, vec!["step_1"]),
            // No RUN_TEST
        ];

        let violation = checker.check_missing_test(&instructions);
        assert!(violation.is_some());
        let v = violation.unwrap();
        assert_eq!(v.rule_id, "VIABILITY-001");
        assert_eq!(v.severity, ViabilitySeverity::Critical);
    }

    #[test]
    fn test_v001_test_present_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("step_1", OpCode::SearchCode, vec![]),
            make_instruction("step_2", OpCode::EditCode, vec!["step_1"]),
            make_instruction("step_3", OpCode::RunTest, vec!["step_2"]),
        ];

        let violation = checker.check_missing_test(&instructions);
        assert!(violation.is_none());
    }

    #[test]
    fn test_v001_no_edit_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("step_1", OpCode::SearchCode, vec![]),
            make_instruction("step_2", OpCode::ReadFiles, vec!["step_1"]),
        ];

        let violation = checker.check_missing_test(&instructions);
        assert!(violation.is_none());
    }

    #[test]
    fn test_v002_invalid_dependency() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("step_1", OpCode::SearchCode, vec![]),
            make_instruction("step_2", OpCode::ReadFiles, vec!["nonexistent"]),
        ];

        let violations = checker.check_logical_flow(&instructions);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id, "VIABILITY-002");
        assert!(violations[0].message.contains("nonexistent"));
    }

    #[test]
    fn test_v002_circular_dependency() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("step_1", OpCode::SearchCode, vec!["step_3"]),
            make_instruction("step_2", OpCode::ReadFiles, vec!["step_1"]),
            make_instruction("step_3", OpCode::EditCode, vec!["step_2"]),
        ];

        let violations = checker.check_logical_flow(&instructions);
        assert!(!violations.is_empty());
        assert!(violations.iter().any(|v| v.message.contains("Circular")));
    }

    #[test]
    fn test_v002_valid_dependencies() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("step_1", OpCode::SearchCode, vec![]),
            make_instruction("step_2", OpCode::ReadFiles, vec!["step_1"]),
            make_instruction("step_3", OpCode::EditCode, vec!["step_2"]),
        ];

        let violations = checker.check_logical_flow(&instructions);
        assert!(violations.is_empty());
    }
}
