//! Deterministic viability checks for plans.
//!
//! These checks run BEFORE LLM-based review to catch structural issues
//! in the plan that can be verified programmatically.
//!
//! # Module Structure
//!
//! - `types`: Core types (ViabilityViolation, ViabilityResult)
//! - `dag`: V-001, V-002 - Cycle detection and dependency validation
//! - `instruction`: V-004, V-005, V-009, V-013, V-014 - Instruction validation
//! - `dataflow`: V-006, V-007, V-008 - Variable references and TDD order
//! - `grounding`: V-003, V-011 - File existence and context ordering
//! - `metrics`: V-010, V-012, DAG analysis - Parallelism and token estimates

mod dag;
mod dataflow;
mod grounding;
mod instruction;
mod metrics;
mod types;

// Re-export all public items
pub use metrics::{DagMetrics, analyze_dag};
pub use types::*;

use crate::models::{FileReference, GroundingSnapshot, Instruction};

// ============================================================================
// Viability Checker
// ============================================================================

/// Performs deterministic viability checks on plans
#[derive(Debug, Clone)]
pub struct ViabilityChecker {
    /// Maximum number of files an EDIT_CODE instruction can touch
    pub max_files_per_edit: usize,
    /// Minimum query length for SEARCH_CODE
    pub min_search_query_length: usize,
}

impl Default for ViabilityChecker {
    fn default() -> Self {
        Self {
            max_files_per_edit: 3,
            min_search_query_length: 3,
        }
    }
}

impl ViabilityChecker {
    /// Create a new viability checker with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Run all viability checks on the given plan data
    pub fn check_all(
        &self,
        instructions: Option<&[Instruction]>,
        grounding: Option<&GroundingSnapshot>,
        file_references: Option<&[FileReference]>,
    ) -> ViabilityResult {
        let mut violations = Vec::new();

        // Run checks on instructions if present
        if let Some(instrs) = instructions {
            // V-014: Check for empty instructions FIRST
            if let Some(v) = self.check_empty_instructions(instrs) {
                violations.push(v);
            }

            // Only run other checks if instructions are non-empty
            if !instrs.is_empty() {
                // V-001: Missing test verification
                if let Some(v) = self.check_missing_test(instrs) {
                    violations.push(v);
                }
                // V-002: Logical flow
                violations.extend(self.check_logical_flow(instrs));
                // V-004: Complexity
                violations.extend(self.check_complexity(instrs));
                // V-005: Params presence
                violations.extend(self.check_params_presence(instrs));
                // V-006: Variable references
                violations.extend(self.check_variable_refs(instrs));
                // V-007: TDD order
                violations.extend(self.check_tdd_order(instrs));
                // V-008: Variable field names
                violations.extend(self.check_variable_field_names(instrs));
                // V-009: Params schema validation
                violations.extend(self.check_params_schema(instrs));
                // V-010: Parallelism check
                violations.extend(self.check_parallelism(instrs));
                // V-011: Grounding order check
                violations.extend(self.check_grounding_order(instrs, grounding));
                // V-012: Token estimates check
                violations.extend(self.check_token_estimates(instrs));
                // V-013: AgentTask params validation
                violations.extend(self.check_agent_task_params(instrs));
            }
        }

        // Run checks on grounding if present
        if let Some(snapshot) = grounding {
            violations.extend(self.check_grounding(snapshot, file_references));
        }

        // Calculate score and pass status
        let critical_count = violations
            .iter()
            .filter(|v| v.severity == ViabilitySeverity::Critical)
            .count();

        let warning_count = violations
            .iter()
            .filter(|v| v.severity == ViabilitySeverity::Warning)
            .count();

        let passed = critical_count == 0;

        // Score: start at 1.0, subtract 0.2 per critical, 0.05 per warning
        let score =
            (1.0 - (critical_count as f32 * 0.2) - (warning_count as f32 * 0.05)).clamp(0.0, 1.0);

        ViabilityResult {
            passed,
            violations,
            score,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{OpCode, VerifiedFile};

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
    fn test_check_all_passing() {
        let checker = ViabilityChecker::new();

        // Create grounding snapshot with files that will be referenced
        let grounding = GroundingSnapshot {
            verified_files: vec![VerifiedFile {
                path: "src/lib.rs".to_string(),
                exists: true,
            }],
            ..Default::default()
        };

        // Create instructions with all required params
        let instructions = vec![
            Instruction {
                id: "step_1".to_string(),
                op: OpCode::SearchCode,
                params: serde_json::json!({
                    "query": "function definition"
                }),
                dependencies: vec![],
                description: "Search for code".to_string(),
                estimated_tokens: Some(500),
                ..Default::default()
            },
            Instruction {
                id: "step_2".to_string(),
                op: OpCode::ReadFiles,
                params: serde_json::json!({
                    "paths": ["src/lib.rs"]
                }),
                dependencies: vec!["step_1".to_string()],
                description: "Read files".to_string(),
                estimated_tokens: Some(1000),
                ..Default::default()
            },
            Instruction {
                id: "step_3".to_string(),
                op: OpCode::GenerateTest,
                params: serde_json::json!({
                    "goal": "Write test for feature",
                    "test_file": "tests/test.rs"
                }),
                dependencies: vec!["step_2".to_string()],
                description: "Generate test".to_string(),
                estimated_tokens: Some(800),
                ..Default::default()
            },
            Instruction {
                id: "step_4".to_string(),
                op: OpCode::RunTest,
                params: serde_json::json!({
                    "command": "cargo test"
                }),
                dependencies: vec!["step_3".to_string()],
                description: "Run test expecting failure".to_string(),
                ..Default::default()
            },
            Instruction {
                id: "step_5".to_string(),
                op: OpCode::EditCode,
                params: serde_json::json!({
                    "goal": "Implement the feature",
                    "files": ["src/lib.rs"]
                }),
                dependencies: vec!["step_4".to_string()],
                description: "Edit code".to_string(),
                estimated_tokens: Some(1200),
                ..Default::default()
            },
            Instruction {
                id: "step_6".to_string(),
                op: OpCode::RunTest,
                params: serde_json::json!({
                    "command": "cargo test"
                }),
                dependencies: vec!["step_5".to_string()],
                description: "Run test expecting pass".to_string(),
                ..Default::default()
            },
        ];

        let result = checker.check_all(Some(&instructions), Some(&grounding), None);

        // Should pass with no critical violations (warnings are OK)
        assert!(
            result.passed,
            "Should pass but got violations: {:?}",
            result.violations
        );
        // Filter for critical violations only - warnings are acceptable
        let critical_violations: Vec<_> = result
            .violations
            .iter()
            .filter(|v| v.severity == ViabilitySeverity::Critical)
            .collect();
        assert!(
            critical_violations.is_empty(),
            "Should have no critical violations but got: {:?}",
            critical_violations
        );
    }

    #[test]
    fn test_check_all_integration() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("step_1", OpCode::SearchCode, vec![]),
            make_instruction("step_2", OpCode::EditCode, vec!["step_1"]),
        ];

        let grounding = GroundingSnapshot {
            verified_files: vec![VerifiedFile {
                path: "nonexistent.rs".to_string(),
                exists: false,
            }],
            ..Default::default()
        };

        let result = checker.check_all(Some(&instructions), Some(&grounding), None);

        // Should have violations: V-001 (missing test) + V-003 (file not found)
        assert!(!result.passed);
        assert!(result.violations.len() >= 2);
    }
}
