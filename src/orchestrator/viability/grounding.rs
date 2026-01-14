//! Grounding validation checks (V-003, V-011).
//!
//! - V-003: File existence validation
//! - V-011: Context ordering validation

use std::collections::{HashMap, HashSet};

use crate::models::{FileAction, FileReference, GroundingSnapshot, Instruction, OpCode};

use super::{ViabilityChecker, ViabilitySeverity, ViabilityViolation};

impl ViabilityChecker {
    /// V-003: Check grounding snapshot for non-existent files
    ///
    /// All verified_files must have exists=true for files the plan will modify,
    /// UNLESS the file is being created (action=Create in file_references).
    pub fn check_grounding(
        &self,
        snapshot: &GroundingSnapshot,
        file_references: Option<&[FileReference]>,
    ) -> Vec<ViabilityViolation> {
        // Build set of files being created (these are allowed to not exist)
        let files_being_created: HashSet<&str> = file_references
            .map(|refs| {
                refs.iter()
                    .filter(|r| matches!(r.action, FileAction::Create))
                    .map(|r| r.path.as_str())
                    .collect()
            })
            .unwrap_or_default();

        snapshot
            .verified_files
            .iter()
            .filter(|f| !f.exists)
            .filter(|f| !files_being_created.contains(f.path.as_str()))
            .map(|f| ViabilityViolation {
                rule_id: "VIABILITY-003".to_string(),
                instruction_id: None,
                severity: ViabilitySeverity::Critical,
                message: format!("Plan references non-existent file: {}", f.path),
                remediation:
                    "Verify file path is correct or add file_reference with action=create"
                        .to_string(),
            })
            .collect()
    }

    /// V-011: Context operations should precede execution operations
    ///
    /// SEARCH_CODE, SEARCH_SEMANTIC, READ_FILES, GET_DEPENDENCIES should come
    /// before EDIT_CODE, RUN_COMMAND in the DAG to ensure proper grounding.
    pub fn check_grounding_order(
        &self,
        instructions: &[Instruction],
        _grounding: Option<&GroundingSnapshot>,
    ) -> Vec<ViabilityViolation> {
        let mut violations = Vec::new();

        let context_ops = [
            OpCode::SearchCode,
            OpCode::SearchSemantic,
            OpCode::ReadFiles,
            OpCode::GetDependencies,
        ];
        let execution_ops = [OpCode::EditCode, OpCode::RunCommand];

        // Build dependency graph to check if context ops properly flow to execution ops
        let id_to_idx: HashMap<&str, usize> = instructions
            .iter()
            .enumerate()
            .map(|(idx, i)| (i.id.as_str(), idx))
            .collect();

        // Find execution ops that don't have any context op in their dependency chain
        for (idx, instr) in instructions.iter().enumerate() {
            if !execution_ops.contains(&instr.op) {
                continue;
            }

            // Check if this execution op has any context op as a transitive dependency
            let mut visited = HashSet::new();

            fn find_context_dep(
                instructions: &[Instruction],
                id_to_idx: &HashMap<&str, usize>,
                current_idx: usize,
                visited: &mut HashSet<usize>,
                context_ops: &[OpCode],
            ) -> bool {
                if visited.contains(&current_idx) {
                    return false;
                }
                visited.insert(current_idx);

                let instr = &instructions[current_idx];
                if context_ops.contains(&instr.op) {
                    return true;
                }

                for dep_id in &instr.dependencies {
                    if let Some(&dep_idx) = id_to_idx.get(dep_id.as_str()) {
                        if find_context_dep(instructions, id_to_idx, dep_idx, visited, context_ops) {
                            return true;
                        }
                    }
                }
                false
            }

            let has_context_dep =
                find_context_dep(instructions, &id_to_idx, idx, &mut visited, &context_ops);

            if !has_context_dep && !instr.dependencies.is_empty() {
                // Has dependencies but none lead to context ops - might be missing grounding
                violations.push(ViabilityViolation {
                    rule_id: "VIABILITY-011".to_string(),
                    instruction_id: Some(instr.id.clone()),
                    severity: ViabilitySeverity::Warning,
                    message: format!(
                        "Execution op '{}' ({:?}) has no context-gathering ops in its dependency chain",
                        instr.id, instr.op
                    ),
                    remediation: "Add SEARCH_CODE or READ_FILES instructions before execution ops to establish context".to_string(),
                });
            } else if instr.dependencies.is_empty() {
                // Execution op with no dependencies at all - definitely missing grounding
                violations.push(ViabilityViolation {
                    rule_id: "VIABILITY-011".to_string(),
                    instruction_id: Some(instr.id.clone()),
                    severity: ViabilitySeverity::Warning,
                    message: format!(
                        "Execution op '{}' ({:?}) has no dependencies - missing grounding phase",
                        instr.id, instr.op
                    ),
                    remediation: "Add SEARCH_CODE or READ_FILES before this instruction to gather context".to_string(),
                });
            }
        }

        violations
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::VerifiedFile;

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

    // V-003: Grounding Tests

    #[test]
    fn test_v003_all_files_exist() {
        let checker = ViabilityChecker::new();
        let snapshot = GroundingSnapshot {
            verified_files: vec![VerifiedFile {
                path: "src/lib.rs".to_string(),
                exists: true,
            }],
            ..Default::default()
        };

        let violations = checker.check_grounding(&snapshot, None);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v003_missing_file() {
        let checker = ViabilityChecker::new();
        let snapshot = GroundingSnapshot {
            verified_files: vec![VerifiedFile {
                path: "src/missing.rs".to_string(),
                exists: false,
            }],
            ..Default::default()
        };

        let violations = checker.check_grounding(&snapshot, None);
        assert!(!violations.is_empty());
        assert_eq!(violations[0].rule_id, "VIABILITY-003");
    }

    #[test]
    fn test_v003_file_being_created_ok() {
        let checker = ViabilityChecker::new();
        let snapshot = GroundingSnapshot {
            verified_files: vec![VerifiedFile {
                path: "src/new_file.rs".to_string(),
                exists: false,
            }],
            ..Default::default()
        };
        let file_refs = vec![FileReference {
            path: "src/new_file.rs".to_string(),
            exists: Some(false),
            action: FileAction::Create,
            description: "New file".to_string(),
        }];

        let violations = checker.check_grounding(&snapshot, Some(&file_refs));
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v003_mixed_create_and_missing() {
        let checker = ViabilityChecker::new();
        let snapshot = GroundingSnapshot {
            verified_files: vec![
                VerifiedFile {
                    path: "src/new_file.rs".to_string(),
                    exists: false,
                },
                VerifiedFile {
                    path: "src/missing.rs".to_string(),
                    exists: false,
                },
            ],
            ..Default::default()
        };
        let file_refs = vec![FileReference {
            path: "src/new_file.rs".to_string(),
            exists: Some(false),
            action: FileAction::Create,
            description: "New file".to_string(),
        }];

        let violations = checker.check_grounding(&snapshot, Some(&file_refs));
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("missing.rs"));
    }

    #[test]
    fn test_v003_truly_missing_file() {
        let checker = ViabilityChecker::new();
        let snapshot = GroundingSnapshot {
            verified_files: vec![VerifiedFile {
                path: "src/missing.rs".to_string(),
                exists: false,
            }],
            ..Default::default()
        };

        let violations = checker.check_grounding(&snapshot, None);
        assert!(!violations.is_empty());
    }

    // V-011: Grounding Order Tests

    #[test]
    fn test_v011_proper_grounding_order() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("search", OpCode::SearchCode, vec![]),
            make_instruction("read", OpCode::ReadFiles, vec!["search"]),
            make_instruction("edit", OpCode::EditCode, vec!["read"]),
        ];

        let violations = checker.check_grounding_order(&instructions, None);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v011_edit_without_context() {
        let checker = ViabilityChecker::new();
        let instructions = vec![make_instruction("edit", OpCode::EditCode, vec![])];

        let violations = checker.check_grounding_order(&instructions, None);
        assert!(!violations.is_empty());
        assert_eq!(violations[0].rule_id, "VIABILITY-011");
    }

    #[test]
    fn test_v011_edit_with_only_test_dep() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("test", OpCode::RunTest, vec![]),
            make_instruction("edit", OpCode::EditCode, vec!["test"]),
        ];

        let violations = checker.check_grounding_order(&instructions, None);
        // RunTest is not a context op, so edit should flag as missing grounding
        assert!(!violations.is_empty());
    }
}
