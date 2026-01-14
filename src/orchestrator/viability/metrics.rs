//! DAG metrics and analysis (V-010, V-012).
//!
//! - V-010: Parallelism check (informational)
//! - V-012: Token estimates check
//! - DAG analysis functions

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::models::{Instruction, OpCode};

use super::{ViabilityChecker, ViabilitySeverity, ViabilityViolation};

/// Metrics describing DAG parallelization characteristics
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DagMetrics {
    /// Total number of instructions in the DAG
    pub total_nodes: usize,
    /// Total number of dependency edges
    pub total_edges: usize,
    /// Root nodes (no dependencies) - can start immediately
    pub root_nodes: usize,
    /// Leaf nodes (no dependents) - final results
    pub leaf_nodes: usize,
    /// Longest dependency chain length
    pub critical_path_length: usize,
    /// Maximum concurrent operations at any topological level
    pub max_width: usize,
    /// Parallelization ratio (max_width / critical_path_length)
    pub parallelization_ratio: f32,
    /// Edges where dependent doesn't reference ${dep.*}
    pub unnecessary_deps: Vec<String>,
}

/// Compute DAG parallelization metrics
pub fn analyze_dag(instructions: &[Instruction]) -> DagMetrics {
    if instructions.is_empty() {
        return DagMetrics::default();
    }

    let total_nodes = instructions.len();
    let total_edges: usize = instructions.iter().map(|i| i.dependencies.len()).sum();

    // Root nodes: no dependencies
    let root_nodes = instructions
        .iter()
        .filter(|i| i.dependencies.is_empty())
        .count();

    // Leaf nodes: no other instruction depends on them
    let all_deps: std::collections::HashSet<&str> = instructions
        .iter()
        .flat_map(|i| i.dependencies.iter().map(|d| d.as_str()))
        .collect();
    let leaf_nodes = instructions
        .iter()
        .filter(|i| !all_deps.contains(i.id.as_str()))
        .count();

    // Compute topological levels for critical path and max width
    let levels = compute_topological_levels(instructions);
    let critical_path_length = levels.values().copied().max().unwrap_or(0) + 1;

    // Max width: count instructions at each level
    let mut level_counts: HashMap<usize, usize> = HashMap::new();
    for level in levels.values() {
        *level_counts.entry(*level).or_insert(0) += 1;
    }
    let max_width = level_counts.values().copied().max().unwrap_or(1);

    // Parallelization ratio
    let parallelization_ratio = if critical_path_length > 0 {
        max_width as f32 / critical_path_length as f32
    } else {
        1.0
    };

    // Find unnecessary dependencies
    let unnecessary_deps = find_unnecessary_deps(instructions);

    DagMetrics {
        total_nodes,
        total_edges,
        root_nodes,
        leaf_nodes,
        critical_path_length,
        max_width,
        parallelization_ratio,
        unnecessary_deps,
    }
}

/// Compute topological level for each instruction
/// Level 0 = no dependencies, Level N = max(dep levels) + 1
fn compute_topological_levels(instructions: &[Instruction]) -> HashMap<String, usize> {
    let mut levels: HashMap<String, usize> = HashMap::new();

    // Initialize root nodes at level 0
    for instr in instructions {
        if instr.dependencies.is_empty() {
            levels.insert(instr.id.clone(), 0);
        }
    }

    // Iteratively compute levels until stable
    let mut changed = true;
    while changed {
        changed = false;
        for instr in instructions {
            if levels.contains_key(&instr.id) {
                continue;
            }

            // Check if all dependencies have levels
            let all_deps_have_levels = instr
                .dependencies
                .iter()
                .all(|d| levels.contains_key(d.as_str()));

            if all_deps_have_levels {
                let max_dep_level = instr
                    .dependencies
                    .iter()
                    .filter_map(|d| levels.get(d.as_str()))
                    .copied()
                    .max()
                    .unwrap_or(0);
                levels.insert(instr.id.clone(), max_dep_level + 1);
                changed = true;
            }
        }
    }

    // Handle any remaining unprocessed nodes (cycles or missing deps)
    for instr in instructions {
        levels.entry(instr.id.clone()).or_insert(0);
    }

    levels
}

/// Find dependencies where the dependent doesn't reference ${dep.*}
fn find_unnecessary_deps(instructions: &[Instruction]) -> Vec<String> {
    let mut unnecessary = Vec::new();

    for instr in instructions {
        let params_str = instr.params.to_string();
        for dep in &instr.dependencies {
            // Check if this dependency is actually referenced in params
            if !params_str.contains(&format!("${{{}", dep)) {
                unnecessary.push(format!("{}->{}", dep, instr.id));
            }
        }
    }

    unnecessary
}

impl ViabilityChecker {
    /// V-010: Informational - Dependencies without variable references
    ///
    /// Dependencies without variable refs are VALID for sequencing (per design doc).
    /// This is now INFO-level only, not a violation.
    pub fn check_parallelism(&self, instructions: &[Instruction]) -> Vec<ViabilityViolation> {
        // NOTE: Sequencing dependencies (deps without ${ref}) are valid per design doc.
        // This check is now informational only. No violations are produced.
        let _ = instructions; // Silence unused warning
        Vec::new()
    }

    /// V-012: Check that context-heavy instructions have token estimates
    ///
    /// EDIT_CODE, READ_FILES, SEARCH_CODE, SEARCH_SEMANTIC should have
    /// estimated_tokens for budget planning.
    pub fn check_token_estimates(&self, instructions: &[Instruction]) -> Vec<ViabilityViolation> {
        let token_required_ops = [
            OpCode::EditCode,
            OpCode::ReadFiles,
            OpCode::SearchCode,
            OpCode::SearchSemantic,
            OpCode::GenerateTest,
        ];

        instructions
            .iter()
            .filter(|i| token_required_ops.contains(&i.op))
            .filter(|i| i.estimated_tokens.is_none())
            .map(|i| ViabilityViolation {
                rule_id: "VIABILITY-012".to_string(),
                instruction_id: Some(i.id.clone()),
                severity: ViabilitySeverity::Warning,
                message: format!(
                    "Instruction '{}' ({:?}) missing estimated_tokens",
                    i.id, i.op
                ),
                remediation: "Add estimated_tokens field for context budget planning".to_string(),
            })
            .collect()
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

    // DAG Analysis Tests

    #[test]
    fn test_analyze_dag_empty() {
        let instructions: Vec<Instruction> = vec![];
        let metrics = analyze_dag(&instructions);

        assert_eq!(metrics.total_nodes, 0);
        assert_eq!(metrics.total_edges, 0);
    }

    #[test]
    fn test_analyze_dag_single_node() {
        let instructions = vec![make_instruction("step_1", OpCode::SearchCode, vec![])];
        let metrics = analyze_dag(&instructions);

        assert_eq!(metrics.total_nodes, 1);
        assert_eq!(metrics.total_edges, 0);
        assert_eq!(metrics.root_nodes, 1);
        assert_eq!(metrics.leaf_nodes, 1);
        assert_eq!(metrics.critical_path_length, 1);
    }

    #[test]
    fn test_analyze_dag_linear_chain() {
        let instructions = vec![
            make_instruction("step_1", OpCode::SearchCode, vec![]),
            make_instruction("step_2", OpCode::ReadFiles, vec!["step_1"]),
            make_instruction("step_3", OpCode::EditCode, vec!["step_2"]),
        ];
        let metrics = analyze_dag(&instructions);

        assert_eq!(metrics.total_nodes, 3);
        assert_eq!(metrics.total_edges, 2);
        assert_eq!(metrics.root_nodes, 1);
        assert_eq!(metrics.leaf_nodes, 1);
        assert_eq!(metrics.critical_path_length, 3);
        assert_eq!(metrics.max_width, 1);
    }

    #[test]
    fn test_analyze_dag_parallel_branches() {
        let instructions = vec![
            make_instruction("root", OpCode::SearchCode, vec![]),
            make_instruction("branch_a", OpCode::ReadFiles, vec!["root"]),
            make_instruction("branch_b", OpCode::ReadFiles, vec!["root"]),
            make_instruction("merge", OpCode::EditCode, vec!["branch_a", "branch_b"]),
        ];
        let metrics = analyze_dag(&instructions);

        assert_eq!(metrics.total_nodes, 4);
        assert_eq!(metrics.total_edges, 4);
        assert_eq!(metrics.root_nodes, 1);
        assert_eq!(metrics.leaf_nodes, 1);
        assert_eq!(metrics.critical_path_length, 3);
        assert_eq!(metrics.max_width, 2); // branch_a and branch_b are parallel
    }

    #[test]
    fn test_analyze_dag_finds_unnecessary_deps() {
        let instructions = vec![
            make_instruction("step_1", OpCode::SearchCode, vec![]),
            // step_2 depends on step_1 but doesn't use ${step_1.*}
            make_instruction("step_2", OpCode::ReadFiles, vec!["step_1"]),
        ];
        let metrics = analyze_dag(&instructions);

        assert!(!metrics.unnecessary_deps.is_empty());
        assert!(metrics.unnecessary_deps[0].contains("step_1->step_2"));
    }

    // V-010: Parallelism Check Tests (now informational-only, no violations)

    #[test]
    fn test_v010_no_violations() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("step_1", OpCode::SearchCode, vec![]),
            make_instruction("step_2", OpCode::ReadFiles, vec!["step_1"]),
        ];

        let violations = checker.check_parallelism(&instructions);
        assert!(violations.is_empty());
    }

    // V-012: Token Estimates Tests

    #[test]
    fn test_v012_missing_token_estimate() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("search", OpCode::SearchCode, vec![]),
            make_instruction("read", OpCode::ReadFiles, vec!["search"]),
            make_instruction("edit", OpCode::EditCode, vec!["read"]),
        ];

        let violations = checker.check_token_estimates(&instructions);
        assert_eq!(violations.len(), 3);
        assert!(violations.iter().all(|v| v.rule_id == "VIABILITY-012"));
    }

    #[test]
    fn test_v012_with_token_estimates_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            Instruction {
                id: "search".to_string(),
                op: OpCode::SearchCode,
                estimated_tokens: Some(500),
                ..Default::default()
            },
            Instruction {
                id: "edit".to_string(),
                op: OpCode::EditCode,
                estimated_tokens: Some(1000),
                ..Default::default()
            },
        ];

        let violations = checker.check_token_estimates(&instructions);
        assert!(violations.is_empty());
    }
}
