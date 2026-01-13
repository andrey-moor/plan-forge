//! Deterministic viability checks for plans.
//!
//! These checks run BEFORE LLM-based review to catch structural issues
//! in the plan that can be verified programmatically.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::models::{
    FileAction, FileReference, GroundingSnapshot, Instruction, OpCode, STEP_RESULT_FIELDS,
};

// ============================================================================
// Viability Types
// ============================================================================

/// Severity level of a viability violation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ViabilitySeverity {
    /// Blocks approval - must be fixed
    Critical,
    /// Should be addressed but doesn't block
    Warning,
}

/// A violation found during viability checking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViabilityViolation {
    /// Rule identifier (e.g., "VIABILITY-001")
    pub rule_id: String,
    /// ID of the instruction that caused the violation (if applicable)
    pub instruction_id: Option<String>,
    /// Severity level
    pub severity: ViabilitySeverity,
    /// Human-readable description of the violation
    pub message: String,
    /// Suggested fix
    pub remediation: String,
}

/// Result of running viability checks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViabilityResult {
    /// Whether all critical checks passed
    pub passed: bool,
    /// List of violations found
    pub violations: Vec<ViabilityViolation>,
    /// Overall viability score (0.0 - 1.0)
    pub score: f32,
}

impl Default for ViabilityResult {
    fn default() -> Self {
        Self {
            passed: true,
            violations: Vec::new(),
            score: 1.0,
        }
    }
}

// ============================================================================
// DAG Metrics
// ============================================================================

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
            // (no point checking structure of an empty array)
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
                violations.extend(self.check_grounding_order(instrs));
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
        let score = (1.0 - (critical_count as f32 * 0.2) - (warning_count as f32 * 0.05))
            .max(0.0)
            .min(1.0);

        ViabilityResult {
            passed,
            violations,
            score,
        }
    }

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
                    if let Some(files) = instr.params.get("files") {
                        if let Some(arr) = files.as_array() {
                            if arr.len() > self.max_files_per_edit {
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
                                    remediation:
                                        "Consider splitting into multiple EDIT_CODE instructions"
                                            .to_string(),
                                });
                            }
                        }
                    }
                }
                OpCode::SearchCode => {
                    // Check query length
                    if let Some(query) = instr.params.get("query") {
                        if let Some(q) = query.as_str() {
                            if q.len() < self.min_search_query_length {
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
                OpCode::SearchCode | OpCode::SearchSemantic => {
                    instr.params.get("query").is_none()
                }
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
    fn has_variable_ref(&self, params: &serde_json::Value) -> bool {
        let json_str = params.to_string();
        if !json_str.contains("${") {
            return false;
        }
        // Accept any valid StepResult field reference
        STEP_RESULT_FIELDS
            .iter()
            .any(|field| json_str.contains(&format!(".{}", field)))
    }

    /// V-006: Check that variable references have corresponding dependencies
    ///
    /// If instruction B uses ${A.output}, B must list A in its dependencies.
    /// Dependencies are for sequencing, variable refs are for data flow.
    /// This checks: data flow requires proper sequencing (refs → deps).
    pub fn check_variable_refs(&self, instructions: &[Instruction]) -> Vec<ViabilityViolation> {
        let mut violations = Vec::new();

        // Collect all valid instruction IDs for reference validation
        let valid_ids: HashSet<&str> = instructions.iter().map(|i| i.id.as_str()).collect();

        // Regex to find ${instruction_id.field} patterns
        let var_ref_pattern =
            regex::Regex::new(r"\$\{([a-zA-Z0-9_-]+)\.([a-zA-Z0-9_]+)\}").unwrap();

        for instr in instructions {
            let params_str = instr.params.to_string();

            // Find all variable references in params
            for cap in var_ref_pattern.captures_iter(&params_str) {
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

        // Regex to match ${instruction_id.field} patterns
        let var_ref_pattern = regex::Regex::new(r"\$\{(\w+)\.(\w+)\}").unwrap();

        for instr in instructions {
            let params_str = instr.params.to_string();

            for cap in var_ref_pattern.captures_iter(&params_str) {
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

    /// V-009: Validate params schema for each OpCode
    ///
    /// Check that instruction params have the correct types for their operation:
    /// - SEARCH_CODE/SEARCH_SEMANTIC: query must be a string
    /// - READ_FILES: paths must be an array or variable reference
    /// - EDIT_CODE: goal/files must be string/array if present
    /// - RUN_COMMAND: command must be a string
    /// - RUN_TEST/GENERATE_TEST: target/behavior must be strings if present
    /// - VERIFY_EXISTS: path must be a string
    pub fn check_params_schema(&self, instructions: &[Instruction]) -> Vec<ViabilityViolation> {
        let mut violations = Vec::new();

        for instr in instructions {
            match instr.op {
                OpCode::SearchCode | OpCode::SearchSemantic => {
                    if let Some(query) = instr.params.get("query") {
                        if !query.is_string() && !self.is_variable_ref_value(query) {
                            violations.push(self.schema_violation(
                                &instr.id,
                                "query",
                                "string",
                                query,
                            ));
                        }
                    }
                    if let Some(limit) = instr.params.get("limit") {
                        if !limit.is_u64() {
                            violations.push(self.schema_violation(
                                &instr.id,
                                "limit",
                                "number",
                                limit,
                            ));
                        }
                    }
                }
                OpCode::ReadFiles => {
                    if let Some(paths) = instr.params.get("paths") {
                        if !paths.is_array()
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
                }
                OpCode::EditCode => {
                    if let Some(goal) = instr.params.get("goal") {
                        if !goal.is_string() && !self.is_variable_ref_value(goal) {
                            violations.push(self.schema_violation(
                                &instr.id,
                                "goal",
                                "string",
                                goal,
                            ));
                        }
                    }
                    if let Some(files) = instr.params.get("files") {
                        if !files.is_array() && !self.is_variable_ref_value(files) {
                            violations.push(self.schema_violation(
                                &instr.id,
                                "files",
                                "array or variable reference",
                                files,
                            ));
                        }
                    }
                }
                OpCode::RunCommand => {
                    if let Some(command) = instr.params.get("command") {
                        if !command.is_string() && !self.is_variable_ref_value(command) {
                            violations.push(self.schema_violation(
                                &instr.id,
                                "command",
                                "string",
                                command,
                            ));
                        }
                    }
                }
                OpCode::RunTest | OpCode::GenerateTest => {
                    if let Some(target) = instr.params.get("target") {
                        if !target.is_string() && !self.is_variable_ref_value(target) {
                            violations.push(self.schema_violation(
                                &instr.id,
                                "target",
                                "string or variable reference",
                                target,
                            ));
                        }
                    }
                    if let Some(behavior) = instr.params.get("behavior") {
                        if !behavior.is_string() {
                            violations.push(self.schema_violation(
                                &instr.id,
                                "behavior",
                                "string",
                                behavior,
                            ));
                        }
                    }
                }
                OpCode::VerifyExists => {
                    if let Some(path) = instr.params.get("path") {
                        if !path.is_string() && !self.is_variable_ref_value(path) {
                            violations.push(self.schema_violation(
                                &instr.id,
                                "path",
                                "string or variable reference",
                                path,
                            ));
                        }
                    }
                }
                OpCode::GetDependencies => {
                    if let Some(path) = instr.params.get("path") {
                        if !path.is_string() && !self.is_variable_ref_value(path) {
                            violations.push(self.schema_violation(
                                &instr.id,
                                "path",
                                "string or variable reference",
                                path,
                            ));
                        }
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
    fn is_variable_ref_value(&self, value: &serde_json::Value) -> bool {
        if let Some(s) = value.as_str() {
            s.starts_with("${") && s.contains('.') && s.ends_with('}')
        } else {
            false
        }
    }

    /// Create a schema violation for a param type mismatch
    fn schema_violation(
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

    // ========================================================================
    // DAG Analysis
    // ========================================================================

    /// Compute DAG parallelization metrics
    pub fn analyze_dag(&self, instructions: &[Instruction]) -> DagMetrics {
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
        let all_deps: HashSet<&str> = instructions
            .iter()
            .flat_map(|i| i.dependencies.iter().map(|d| d.as_str()))
            .collect();
        let leaf_nodes = instructions
            .iter()
            .filter(|i| !all_deps.contains(i.id.as_str()))
            .count();

        // Compute topological levels for critical path and max width
        let levels = self.compute_topological_levels(instructions);
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
        let unnecessary_deps = self.find_unnecessary_deps(instructions);

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
    fn compute_topological_levels(&self, instructions: &[Instruction]) -> HashMap<String, usize> {
        let mut levels: HashMap<String, usize> = HashMap::new();

        // Build dependency map
        let instr_map: HashMap<&str, &Instruction> =
            instructions.iter().map(|i| (i.id.as_str(), i)).collect();

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

        // Silence unused variable warning
        let _ = instr_map;

        levels
    }

    /// Find dependencies where the dependent doesn't reference ${dep.*}
    fn find_unnecessary_deps(&self, instructions: &[Instruction]) -> Vec<String> {
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

    /// V-010: Informational - Dependencies without variable references
    ///
    /// Dependencies without variable refs are VALID for sequencing (per design doc).
    /// This is now INFO-level only, not a violation. Sequencing dependencies ensure
    /// correct execution order without requiring data flow between instructions.
    pub fn check_parallelism(&self, instructions: &[Instruction]) -> Vec<ViabilityViolation> {
        // NOTE: Sequencing dependencies (deps without ${ref}) are valid per design doc.
        // Design doc explicitly separates:
        // - dependencies: "IDs of instructions that must complete before this one" (ordering)
        // - variable references: "${id.field}" in params (data flow)
        //
        // This check is now informational only. No violations are produced.
        // Future: Could add Info-level hints for potential parallelism opportunities,
        // but only if the dependency is truly unnecessary (not sequencing-required).

        let _ = instructions; // Silence unused warning
        Vec::new()
    }

    /// V-011: Context operations should precede execution operations
    ///
    /// SEARCH_CODE, SEARCH_SEMANTIC, READ_FILES, GET_DEPENDENCIES should come
    /// before EDIT_CODE, RUN_COMMAND in the DAG to ensure proper grounding.
    pub fn check_grounding_order(&self, instructions: &[Instruction]) -> Vec<ViabilityViolation> {
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

    /// V-013: EDIT_CODE and GENERATE_TEST must use AgentTask params schema
    ///
    /// Per design doc, AgentTask required fields: role, goal, context_files, constraints
    /// - goal/task: CRITICAL if missing
    /// - role, context_files, constraints: WARNING if missing (design doc marks required)
    /// Rejects legacy params: action, content_description
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
                .or_else(|| instr.params.get("task")) // Accept "task" as alias for "goal"
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
            // Design doc schema: "required": ["role", "goal", "context_files", "constraints"]
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
    /// Plans MUST have executable instructions. An empty instructions array
    /// indicates the planner removed instructions to avoid violations rather
    /// than fixing them.
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

// ============================================================================
// Tests
// ============================================================================

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
            make_instruction("step_a", OpCode::SearchCode, vec!["step_c"]),
            make_instruction("step_b", OpCode::ReadFiles, vec!["step_a"]),
            make_instruction("step_c", OpCode::EditCode, vec!["step_b"]),
        ];

        let violations = checker.check_logical_flow(&instructions);
        assert!(
            violations.iter().any(|v| v.message.contains("Circular")),
            "Should detect circular dependency"
        );
    }

    #[test]
    fn test_v002_valid_dependencies() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("step_1", OpCode::SearchCode, vec![]),
            make_instruction("step_2", OpCode::ReadFiles, vec!["step_1"]),
            make_instruction("step_3", OpCode::EditCode, vec!["step_2"]),
            make_instruction("step_4", OpCode::RunTest, vec!["step_3"]),
        ];

        let violations = checker.check_logical_flow(&instructions);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v003_missing_file() {
        let checker = ViabilityChecker::new();
        let snapshot = GroundingSnapshot {
            verified_files: vec![
                crate::models::VerifiedFile {
                    path: "src/existing.rs".to_string(),
                    exists: true,
                },
                crate::models::VerifiedFile {
                    path: "src/missing.rs".to_string(),
                    exists: false,
                },
            ],
            ..Default::default()
        };

        let violations = checker.check_grounding(&snapshot, None);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id, "VIABILITY-003");
        assert!(violations[0].message.contains("missing.rs"));
    }

    #[test]
    fn test_v003_all_files_exist() {
        let checker = ViabilityChecker::new();
        let snapshot = GroundingSnapshot {
            verified_files: vec![
                crate::models::VerifiedFile {
                    path: "src/lib.rs".to_string(),
                    exists: true,
                },
                crate::models::VerifiedFile {
                    path: "src/main.rs".to_string(),
                    exists: true,
                },
            ],
            ..Default::default()
        };

        let violations = checker.check_grounding(&snapshot, None);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v003_file_being_created_ok() {
        let checker = ViabilityChecker::new();
        let snapshot = GroundingSnapshot {
            verified_files: vec![crate::models::VerifiedFile {
                path: "src/utils.rs".to_string(),
                exists: false,
            }],
            ..Default::default()
        };
        let file_refs = vec![FileReference {
            path: "src/utils.rs".to_string(),
            exists: Some(false),
            action: FileAction::Create,
            description: "New utils module".to_string(),
        }];

        let violations = checker.check_grounding(&snapshot, Some(&file_refs));
        assert!(
            violations.is_empty(),
            "Should not flag files being created"
        );
    }

    #[test]
    fn test_v003_truly_missing_file() {
        let checker = ViabilityChecker::new();
        let snapshot = GroundingSnapshot {
            verified_files: vec![crate::models::VerifiedFile {
                path: "src/missing.rs".to_string(),
                exists: false,
            }],
            ..Default::default()
        };
        // No file_reference with action=create for this file
        let file_refs: Vec<FileReference> = vec![];

        let violations = checker.check_grounding(&snapshot, Some(&file_refs));
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id, "VIABILITY-003");
    }

    #[test]
    fn test_v003_mixed_create_and_missing() {
        let checker = ViabilityChecker::new();
        let snapshot = GroundingSnapshot {
            verified_files: vec![
                crate::models::VerifiedFile {
                    path: "src/new_file.rs".to_string(),
                    exists: false,
                },
                crate::models::VerifiedFile {
                    path: "src/missing.rs".to_string(),
                    exists: false,
                },
            ],
            ..Default::default()
        };
        // Only src/new_file.rs is being created, src/missing.rs is truly missing
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
    fn test_v004_too_many_files() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("edit", OpCode::EditCode, vec![]);
        instr.params = serde_json::json!({
            "files": ["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"]
        });

        let violations = checker.check_complexity(&[instr]);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id, "VIABILITY-004");
        assert!(violations[0].message.contains("5 files"));
    }

    #[test]
    fn test_v004_short_query() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("search", OpCode::SearchCode, vec![]);
        instr.params = serde_json::json!({ "query": "fn" });

        let violations = checker.check_complexity(&[instr]);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id, "VIABILITY-004");
        assert!(violations[0].message.contains("too short"));
    }

    #[test]
    fn test_v004_valid_complexity() {
        let checker = ViabilityChecker::new();
        let mut edit = make_instruction("edit", OpCode::EditCode, vec![]);
        edit.params = serde_json::json!({ "files": ["a.rs", "b.rs"] });

        let mut search = make_instruction("search", OpCode::SearchCode, vec![]);
        search.params = serde_json::json!({ "query": "impl Default" });

        let violations = checker.check_complexity(&[edit, search]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_check_all_integration() {
        let checker = ViabilityChecker::new();

        // Plan with issues: edit without test, missing file
        let instructions = vec![
            make_instruction("step_1", OpCode::SearchCode, vec![]),
            make_instruction("step_2", OpCode::EditCode, vec!["step_1"]),
        ];

        let snapshot = GroundingSnapshot {
            verified_files: vec![crate::models::VerifiedFile {
                path: "src/missing.rs".to_string(),
                exists: false,
            }],
            ..Default::default()
        };

        let result = checker.check_all(Some(&instructions), Some(&snapshot), None);

        assert!(!result.passed);
        assert!(result.score < 1.0);
        assert!(result.violations.len() >= 2); // V-001 + V-003
    }

    #[test]
    fn test_check_all_passing() {
        let checker = ViabilityChecker::new();

        // Create instructions with all required params including V-013 AgentTask fields
        // V-012 requires estimated_tokens on context-heavy ops
        let mut search = make_instruction("step_1", OpCode::SearchCode, vec![]);
        search.params = serde_json::json!({ "query": "impl Handler" });
        search.estimated_tokens = Some(200);

        let mut gen_test = make_instruction("step_2", OpCode::GenerateTest, vec!["step_1"]);
        gen_test.params = serde_json::json!({
            "role": "TESTER",
            "goal": "Test handler behavior",
            "behavior": "Handler works",
            "context_files": ["${step_1.output}"],
            "constraints": ["Cover edge cases"]
        });
        gen_test.estimated_tokens = Some(1500);

        let mut run_fail = make_instruction("step_3", OpCode::RunTest, vec!["step_2"]);
        run_fail.params = serde_json::json!({ "target": "${step_2.output}", "expected_result": "failure" });

        // edit depends on step_3 (TDD pattern)
        let mut edit = make_instruction("step_4", OpCode::EditCode, vec!["step_3"]);
        edit.params = serde_json::json!({
            "role": "ENGINEER",
            "goal": "Implement handler",
            "context_files": ["${step_3.output}"],
            "constraints": ["Follow existing patterns"]
        });
        edit.estimated_tokens = Some(2000);

        // run_pass depends on step_4 (the edit)
        let mut run_pass = make_instruction("step_5", OpCode::RunTest, vec!["step_4"]);
        run_pass.params = serde_json::json!({
            "target": "${step_4.artifacts}",
            "expected_result": "success"
        });

        let instructions = vec![search, gen_test, run_fail, edit, run_pass];

        let snapshot = GroundingSnapshot {
            verified_files: vec![crate::models::VerifiedFile {
                path: "src/lib.rs".to_string(),
                exists: true,
            }],
            ..Default::default()
        };

        let result = checker.check_all(Some(&instructions), Some(&snapshot), None);

        assert!(result.passed);
        assert_eq!(result.score, 1.0);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn test_viability_result_default() {
        let result = ViabilityResult::default();
        assert!(result.passed);
        assert_eq!(result.score, 1.0);
        assert!(result.violations.is_empty());
    }

    // ========================================================================
    // V-005: Params Presence Tests
    // ========================================================================

    #[test]
    fn test_v005_missing_query_param() {
        let checker = ViabilityChecker::new();
        let instructions = vec![make_instruction("search", OpCode::SearchCode, vec![])];

        let violations = checker.check_params_presence(&instructions);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id, "VIABILITY-005");
        assert!(violations[0].message.contains("missing required params"));
    }

    #[test]
    fn test_v005_with_query_param_ok() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("search", OpCode::SearchCode, vec![]);
        instr.params = serde_json::json!({ "query": "impl Handler" });

        let violations = checker.check_params_presence(&[instr]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v005_read_files_with_variable_ref_ok() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("read", OpCode::ReadFiles, vec!["search"]);
        instr.params = serde_json::json!({ "paths": "${search.output}" });

        let violations = checker.check_params_presence(&[instr]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v005_edit_code_needs_goal_or_files() {
        let checker = ViabilityChecker::new();
        let instr = make_instruction("edit", OpCode::EditCode, vec![]);

        let violations = checker.check_params_presence(&[instr]);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("EditCode"));
    }

    #[test]
    fn test_v005_edit_code_with_goal_ok() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("edit", OpCode::EditCode, vec![]);
        instr.params = serde_json::json!({ "goal": "Add validation" });

        let violations = checker.check_params_presence(&[instr]);
        assert!(violations.is_empty());
    }

    // ========================================================================
    // V-006: Variable Reference Tests (refs → deps direction)
    // ========================================================================

    #[test]
    fn test_v006_ref_without_dependency() {
        let checker = ViabilityChecker::new();
        // Instruction references ${search.output} but doesn't depend on "search"
        let search = make_instruction("search", OpCode::SearchCode, vec![]);
        let mut read = make_instruction("read", OpCode::ReadFiles, vec![]); // No dependency!
        read.params = serde_json::json!({ "paths": "${search.output}" });

        let violations = checker.check_variable_refs(&[search, read]);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id, "VIABILITY-006");
        assert!(violations[0].message.contains("doesn't depend on 'search'"));
    }

    #[test]
    fn test_v006_ref_with_dependency_ok() {
        let checker = ViabilityChecker::new();
        // Instruction references ${search.output} AND depends on "search"
        let search = make_instruction("search", OpCode::SearchCode, vec![]);
        let mut read = make_instruction("read", OpCode::ReadFiles, vec!["search"]);
        read.params = serde_json::json!({ "paths": "${search.output}" });

        let violations = checker.check_variable_refs(&[search, read]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v006_no_refs_ok() {
        let checker = ViabilityChecker::new();
        // No variable references = no V-006 violations (deps without refs is valid for sequencing)
        let search = make_instruction("search", OpCode::SearchCode, vec![]);
        let mut edit = make_instruction("edit", OpCode::EditCode, vec!["search"]);
        edit.params = serde_json::json!({ "goal": "some work" });

        let violations = checker.check_variable_refs(&[search, edit]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v006_ref_to_nonexistent_instruction_ok() {
        let checker = ViabilityChecker::new();
        // Reference to non-existent instruction is handled by V-002, not V-006
        let mut read = make_instruction("read", OpCode::ReadFiles, vec![]);
        read.params = serde_json::json!({ "paths": "${nonexistent.output}" });

        let violations = checker.check_variable_refs(&[read]);
        assert!(violations.is_empty()); // V-006 skips unknown refs
    }

    // ========================================================================
    // V-007: TDD Order Tests
    // ========================================================================

    #[test]
    fn test_v007_edit_without_test_before() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("search", OpCode::SearchCode, vec![]),
            make_instruction("edit", OpCode::EditCode, vec!["search"]),
            make_instruction("test", OpCode::RunTest, vec!["edit"]),
        ];

        let violations = checker.check_tdd_order(&instructions);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id, "VIABILITY-007");
        assert!(violations[0].message.contains("TDD Red-Green pattern"));
    }

    #[test]
    fn test_v007_tdd_pattern_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![
            make_instruction("search", OpCode::SearchCode, vec![]),
            make_instruction("gen_test", OpCode::GenerateTest, vec!["search"]),
            make_instruction("run_fail", OpCode::RunTest, vec!["gen_test"]),
            make_instruction("edit", OpCode::EditCode, vec!["run_fail"]),
            make_instruction("run_pass", OpCode::RunTest, vec!["edit"]),
        ];

        let violations = checker.check_tdd_order(&instructions);
        assert!(violations.is_empty());
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

    // ========================================================================
    // V-008: Variable Field Names Tests
    // ========================================================================

    #[test]
    fn test_v008_valid_output_field() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("read", OpCode::ReadFiles, vec!["search"]);
        instr.params = serde_json::json!({ "paths": "${search.output}" });

        let violations = checker.check_variable_field_names(&[instr]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v008_valid_stdout_field() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("process", OpCode::EditCode, vec!["cmd"]);
        instr.params = serde_json::json!({ "data": "${cmd.stdout}" });

        let violations = checker.check_variable_field_names(&[instr]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v008_valid_artifacts_field() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("test", OpCode::RunTest, vec!["edit"]);
        instr.params = serde_json::json!({ "target": "${edit.artifacts}" });

        let violations = checker.check_variable_field_names(&[instr]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v008_invalid_field_name() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("read", OpCode::ReadFiles, vec!["search"]);
        instr.params = serde_json::json!({ "paths": "${search.result}" }); // "result" is invalid

        let violations = checker.check_variable_field_names(&[instr]);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id, "VIABILITY-008");
        assert!(violations[0].message.contains("result"));
    }

    #[test]
    fn test_v008_multiple_invalid_fields() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("read", OpCode::ReadFiles, vec!["a", "b"]);
        instr.params = serde_json::json!({
            "paths": "${a.foo}",  // invalid
            "data": "${b.bar}"   // invalid
        });

        let violations = checker.check_variable_field_names(&[instr]);
        assert_eq!(violations.len(), 2);
    }

    #[test]
    fn test_v008_no_variable_refs_ok() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("search", OpCode::SearchCode, vec![]);
        instr.params = serde_json::json!({ "query": "impl Handler" });

        let violations = checker.check_variable_field_names(&[instr]);
        assert!(violations.is_empty());
    }

    // ========================================================================
    // V-009: Params Schema Validation Tests
    // ========================================================================

    #[test]
    fn test_v009_search_code_query_string_ok() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("search", OpCode::SearchCode, vec![]);
        instr.params = serde_json::json!({ "query": "impl Handler" });

        let violations = checker.check_params_schema(&[instr]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v009_search_code_query_wrong_type() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("search", OpCode::SearchCode, vec![]);
        instr.params = serde_json::json!({ "query": 123 }); // Number instead of string

        let violations = checker.check_params_schema(&[instr]);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id, "VIABILITY-009");
        assert!(violations[0].message.contains("query"));
        assert!(violations[0].message.contains("string"));
        assert!(violations[0].message.contains("number"));
    }

    #[test]
    fn test_v009_search_code_limit_ok() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("search", OpCode::SearchCode, vec![]);
        instr.params = serde_json::json!({ "query": "test", "limit": 10 });

        let violations = checker.check_params_schema(&[instr]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v009_search_code_limit_wrong_type() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("search", OpCode::SearchCode, vec![]);
        instr.params = serde_json::json!({ "query": "test", "limit": "ten" }); // String instead of number

        let violations = checker.check_params_schema(&[instr]);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("limit"));
    }

    #[test]
    fn test_v009_read_files_paths_array_ok() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("read", OpCode::ReadFiles, vec![]);
        instr.params = serde_json::json!({ "paths": ["src/lib.rs", "src/main.rs"] });

        let violations = checker.check_params_schema(&[instr]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v009_read_files_paths_variable_ref_ok() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("read", OpCode::ReadFiles, vec!["search"]);
        instr.params = serde_json::json!({ "paths": "${search.output}" });

        let violations = checker.check_params_schema(&[instr]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v009_read_files_paths_wrong_type() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("read", OpCode::ReadFiles, vec![]);
        instr.params = serde_json::json!({ "paths": 42 }); // Number instead of array

        let violations = checker.check_params_schema(&[instr]);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("paths"));
    }

    #[test]
    fn test_v009_edit_code_goal_string_ok() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("edit", OpCode::EditCode, vec![]);
        instr.params = serde_json::json!({ "goal": "Add validation" });

        let violations = checker.check_params_schema(&[instr]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v009_edit_code_files_array_ok() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("edit", OpCode::EditCode, vec![]);
        instr.params = serde_json::json!({ "files": ["src/lib.rs"] });

        let violations = checker.check_params_schema(&[instr]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v009_edit_code_goal_wrong_type() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("edit", OpCode::EditCode, vec![]);
        instr.params = serde_json::json!({ "goal": ["not", "a", "string"] });

        let violations = checker.check_params_schema(&[instr]);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("goal"));
        assert!(violations[0].message.contains("array"));
    }

    #[test]
    fn test_v009_run_command_command_string_ok() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("cmd", OpCode::RunCommand, vec![]);
        instr.params = serde_json::json!({ "command": "cargo test" });

        let violations = checker.check_params_schema(&[instr]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v009_run_command_command_wrong_type() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("cmd", OpCode::RunCommand, vec![]);
        instr.params = serde_json::json!({ "command": {"shell": "bash"} }); // Object instead of string

        let violations = checker.check_params_schema(&[instr]);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("command"));
        assert!(violations[0].message.contains("object"));
    }

    #[test]
    fn test_v009_verify_exists_path_string_ok() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("verify", OpCode::VerifyExists, vec![]);
        instr.params = serde_json::json!({ "path": "src/lib.rs" });

        let violations = checker.check_params_schema(&[instr]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v009_verify_exists_path_variable_ref_ok() {
        let checker = ViabilityChecker::new();
        let mut instr = make_instruction("verify", OpCode::VerifyExists, vec!["search"]);
        instr.params = serde_json::json!({ "path": "${search.output}" });

        let violations = checker.check_params_schema(&[instr]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v009_multiple_violations() {
        let checker = ViabilityChecker::new();
        let mut search = make_instruction("search", OpCode::SearchCode, vec![]);
        search.params = serde_json::json!({ "query": 123, "limit": "ten" }); // Both wrong

        let violations = checker.check_params_schema(&[search]);
        assert_eq!(violations.len(), 2);
    }

    // ========================================================================
    // V-010: Parallelism Check Tests (now informational-only, no violations)
    // ========================================================================

    #[test]
    fn test_v010_sequencing_deps_valid() {
        let checker = ViabilityChecker::new();
        let mut search = make_instruction("search", OpCode::SearchCode, vec![]);
        search.params = serde_json::json!({ "query": "impl Handler" });

        // Dependencies without variable refs are VALID for sequencing per design doc
        let mut edit = make_instruction("edit", OpCode::EditCode, vec!["search"]);
        edit.params = serde_json::json!({ "goal": "Add handler" }); // No ${search.*} reference

        let violations = checker.check_parallelism(&[search, edit]);
        assert!(violations.is_empty()); // No violations - sequencing deps are valid
    }

    #[test]
    fn test_v010_data_flow_deps_ok() {
        let checker = ViabilityChecker::new();
        let mut search = make_instruction("search", OpCode::SearchCode, vec![]);
        search.params = serde_json::json!({ "query": "impl Handler" });

        let mut read = make_instruction("read", OpCode::ReadFiles, vec!["search"]);
        read.params = serde_json::json!({ "paths": "${search.output}" }); // Uses reference

        let violations = checker.check_parallelism(&[search, read]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v010_no_dependencies_ok() {
        let checker = ViabilityChecker::new();
        let mut search = make_instruction("search", OpCode::SearchCode, vec![]);
        search.params = serde_json::json!({ "query": "impl Handler" });

        let violations = checker.check_parallelism(&[search]);
        assert!(violations.is_empty());
    }

    // ========================================================================
    // DAG Metrics Tests
    // ========================================================================

    #[test]
    fn test_analyze_dag_empty() {
        let checker = ViabilityChecker::new();
        let metrics = checker.analyze_dag(&[]);
        assert_eq!(metrics.total_nodes, 0);
        assert_eq!(metrics.total_edges, 0);
    }

    #[test]
    fn test_analyze_dag_single_node() {
        let checker = ViabilityChecker::new();
        let mut search = make_instruction("search", OpCode::SearchCode, vec![]);
        search.params = serde_json::json!({ "query": "test" });

        let metrics = checker.analyze_dag(&[search]);
        assert_eq!(metrics.total_nodes, 1);
        assert_eq!(metrics.total_edges, 0);
        assert_eq!(metrics.root_nodes, 1);
        assert_eq!(metrics.leaf_nodes, 1);
        assert_eq!(metrics.critical_path_length, 1);
        assert_eq!(metrics.max_width, 1);
    }

    #[test]
    fn test_analyze_dag_linear_chain() {
        let checker = ViabilityChecker::new();
        let mut step1 = make_instruction("step1", OpCode::SearchCode, vec![]);
        step1.params = serde_json::json!({ "query": "test" });

        let mut step2 = make_instruction("step2", OpCode::ReadFiles, vec!["step1"]);
        step2.params = serde_json::json!({ "paths": "${step1.output}" });

        let mut step3 = make_instruction("step3", OpCode::EditCode, vec!["step2"]);
        step3.params = serde_json::json!({ "goal": "edit", "files": "${step2.output}" });

        let metrics = checker.analyze_dag(&[step1, step2, step3]);
        assert_eq!(metrics.total_nodes, 3);
        assert_eq!(metrics.total_edges, 2);
        assert_eq!(metrics.root_nodes, 1);
        assert_eq!(metrics.leaf_nodes, 1);
        assert_eq!(metrics.critical_path_length, 3);
        assert_eq!(metrics.max_width, 1);
        // Linear chain = ratio of 1/3
        assert!((metrics.parallelization_ratio - 0.333).abs() < 0.01);
    }

    #[test]
    fn test_analyze_dag_parallel_branches() {
        let checker = ViabilityChecker::new();
        // Setup -> 3 parallel branches
        let mut setup = make_instruction("setup", OpCode::SearchCode, vec![]);
        setup.params = serde_json::json!({ "query": "setup" });

        let mut branch1 = make_instruction("branch1", OpCode::EditCode, vec!["setup"]);
        branch1.params = serde_json::json!({ "goal": "b1", "context": "${setup.output}" });

        let mut branch2 = make_instruction("branch2", OpCode::EditCode, vec!["setup"]);
        branch2.params = serde_json::json!({ "goal": "b2", "context": "${setup.output}" });

        let mut branch3 = make_instruction("branch3", OpCode::EditCode, vec!["setup"]);
        branch3.params = serde_json::json!({ "goal": "b3", "context": "${setup.output}" });

        let metrics = checker.analyze_dag(&[setup, branch1, branch2, branch3]);
        assert_eq!(metrics.total_nodes, 4);
        assert_eq!(metrics.total_edges, 3);
        assert_eq!(metrics.root_nodes, 1);
        assert_eq!(metrics.leaf_nodes, 3);
        assert_eq!(metrics.critical_path_length, 2);
        assert_eq!(metrics.max_width, 3);
        // Parallel branches = ratio of 3/2 = 1.5
        assert!((metrics.parallelization_ratio - 1.5).abs() < 0.01);
    }

    #[test]
    fn test_analyze_dag_finds_unnecessary_deps() {
        let checker = ViabilityChecker::new();
        let mut step1 = make_instruction("step1", OpCode::SearchCode, vec![]);
        step1.params = serde_json::json!({ "query": "test" });

        // This depends on step1 but doesn't reference it
        let mut step2 = make_instruction("step2", OpCode::EditCode, vec!["step1"]);
        step2.params = serde_json::json!({ "goal": "edit" }); // No ${step1.*}

        let metrics = checker.analyze_dag(&[step1, step2]);
        assert_eq!(metrics.unnecessary_deps.len(), 1);
        assert_eq!(metrics.unnecessary_deps[0], "step1->step2");
    }

    // ========================================================================
    // V-011: Grounding Order Tests
    // ========================================================================

    #[test]
    fn test_v011_edit_without_context_dependency() {
        let checker = ViabilityChecker::new();

        // EDIT_CODE with no dependencies - missing grounding
        let edit = make_instruction("edit", OpCode::EditCode, vec![]);

        let violations = checker.check_grounding_order(&[edit]);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id, "VIABILITY-011");
        assert!(violations[0].message.contains("no dependencies"));
    }

    #[test]
    fn test_v011_edit_with_context_dependency_ok() {
        let checker = ViabilityChecker::new();

        // Proper grounding: SEARCH_CODE -> READ_FILES -> EDIT_CODE
        let mut search = make_instruction("search", OpCode::SearchCode, vec![]);
        search.params = serde_json::json!({ "query": "handler" });

        let mut read = make_instruction("read", OpCode::ReadFiles, vec!["search"]);
        read.params = serde_json::json!({ "paths": "${search.output}" });

        let mut edit = make_instruction("edit", OpCode::EditCode, vec!["read"]);
        edit.params = serde_json::json!({ "goal": "implement", "context_files": ["${read.output}"] });

        let violations = checker.check_grounding_order(&[search, read, edit]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v011_edit_with_non_context_dependency() {
        let checker = ViabilityChecker::new();

        // EDIT_CODE depends on RUN_COMMAND (not a context op)
        let cmd = make_instruction("cmd", OpCode::RunCommand, vec![]);
        let edit = make_instruction("edit", OpCode::EditCode, vec!["cmd"]);

        let violations = checker.check_grounding_order(&[cmd, edit]);
        // Both should have violations: cmd has no deps, edit depends on non-context
        assert!(violations.len() >= 1);
        assert!(violations.iter().any(|v| v.instruction_id == Some("edit".to_string())));
    }

    #[test]
    fn test_v011_run_command_setup_ok() {
        let checker = ViabilityChecker::new();

        // RUN_COMMAND for setup (mkdir) with no deps is OK if followed by context ops
        let mkdir = make_instruction("mkdir", OpCode::RunCommand, vec![]);
        let mut search = make_instruction("search", OpCode::SearchCode, vec!["mkdir"]);
        search.params = serde_json::json!({ "query": "test" });

        // mkdir has no context dep, so it gets flagged
        let violations = checker.check_grounding_order(&[mkdir, search]);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].instruction_id, Some("mkdir".to_string()));
    }

    // ========================================================================
    // V-012: Token Estimates Tests
    // ========================================================================

    #[test]
    fn test_v012_missing_token_estimate() {
        let checker = ViabilityChecker::new();

        // EDIT_CODE without estimated_tokens
        let edit = make_instruction("edit", OpCode::EditCode, vec![]);

        let violations = checker.check_token_estimates(&[edit]);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id, "VIABILITY-012");
        assert!(violations[0].message.contains("missing estimated_tokens"));
    }

    #[test]
    fn test_v012_with_token_estimate_ok() {
        let checker = ViabilityChecker::new();

        // EDIT_CODE with estimated_tokens
        let mut edit = make_instruction("edit", OpCode::EditCode, vec![]);
        edit.estimated_tokens = Some(2000);

        let violations = checker.check_token_estimates(&[edit]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v012_run_test_no_estimate_ok() {
        let checker = ViabilityChecker::new();

        // RUN_TEST doesn't require estimated_tokens
        let test = make_instruction("test", OpCode::RunTest, vec![]);

        let violations = checker.check_token_estimates(&[test]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v012_multiple_missing_estimates() {
        let checker = ViabilityChecker::new();

        let search = make_instruction("search", OpCode::SearchCode, vec![]);
        let read = make_instruction("read", OpCode::ReadFiles, vec!["search"]);
        let edit = make_instruction("edit", OpCode::EditCode, vec!["read"]);
        let test = make_instruction("test", OpCode::RunTest, vec!["edit"]);

        let violations = checker.check_token_estimates(&[search, read, edit, test]);
        // search, read, edit should be flagged, test should not
        assert_eq!(violations.len(), 3);
        assert!(violations.iter().all(|v| v.rule_id == "VIABILITY-012"));
    }

    // ========================================================================
    // V-013: AgentTask Params Tests
    // ========================================================================

    #[test]
    fn test_v013_edit_code_missing_goal() {
        let checker = ViabilityChecker::new();
        let mut edit = make_instruction("edit1", OpCode::EditCode, vec![]);
        edit.params = serde_json::json!({
            "action": "create",
            "file": "src/foo.rs"
        });

        let violations = checker.check_agent_task_params(&[edit]);
        // Missing goal (Critical), missing fields warning (Warning), legacy params (Critical)
        assert_eq!(violations.len(), 3);
        assert!(violations.iter().all(|v| v.rule_id == "VIABILITY-013"));
        assert_eq!(
            violations.iter().filter(|v| v.severity == ViabilitySeverity::Critical).count(),
            2
        );
    }

    #[test]
    fn test_v013_edit_code_with_all_fields_ok() {
        let checker = ViabilityChecker::new();
        let mut edit = make_instruction("edit1", OpCode::EditCode, vec![]);
        edit.params = serde_json::json!({
            "role": "ENGINEER",
            "goal": "Implement rate limiting",
            "files": ["src/api.rs"],
            "context_files": ["${read_api.output}"],
            "constraints": ["Must use token bucket algorithm"]
        });

        let violations = checker.check_agent_task_params(&[edit]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v013_edit_code_missing_optional_fields_warning() {
        let checker = ViabilityChecker::new();
        let mut edit = make_instruction("edit1", OpCode::EditCode, vec![]);
        edit.params = serde_json::json!({
            "goal": "Implement rate limiting",  // Has goal but missing role, context_files, constraints
            "files": ["src/api.rs"]
        });

        let violations = checker.check_agent_task_params(&[edit]);
        // Should have 1 Warning for missing fields
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].severity, ViabilitySeverity::Warning);
        assert!(violations[0].message.contains("missing AgentTask fields"));
    }

    #[test]
    fn test_v013_generate_test_missing_goal() {
        let checker = ViabilityChecker::new();
        let mut test = make_instruction("test1", OpCode::GenerateTest, vec![]);
        test.params = serde_json::json!({
            "test_file": "tests/test_foo.rs"
        });

        let violations = checker.check_agent_task_params(&[test]);
        // Missing goal (Critical) + missing fields (Warning)
        assert_eq!(violations.len(), 2);
        assert!(violations.iter().any(|v| v.severity == ViabilitySeverity::Critical));
    }

    #[test]
    fn test_v013_generate_test_with_all_fields_ok() {
        let checker = ViabilityChecker::new();
        let mut test = make_instruction("test1", OpCode::GenerateTest, vec![]);
        test.params = serde_json::json!({
            "role": "TESTER",
            "goal": "Create unit tests for RateLimiter",
            "behavior": "Rate limiter blocks requests after threshold",
            "target_file": "tests/test_rate_limiter.rs",
            "context_files": ["src/rate_limiter.rs"],
            "constraints": ["Cover edge cases"]
        });

        let violations = checker.check_agent_task_params(&[test]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_v013_edit_code_with_task_alias_warns_missing_fields() {
        let checker = ViabilityChecker::new();
        let mut edit = make_instruction("edit1", OpCode::EditCode, vec![]);
        edit.params = serde_json::json!({
            "task": "Implement rate limiting",  // Using "task" alias instead of "goal"
            "files": ["src/api.rs"]
        });

        let violations = checker.check_agent_task_params(&[edit]);
        // Should pass goal check with "task", but warn about missing role/context_files/constraints
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].severity, ViabilitySeverity::Warning);
    }

    #[test]
    fn test_v013_generate_test_with_task_alias_warns_missing_fields() {
        let checker = ViabilityChecker::new();
        let mut test = make_instruction("test1", OpCode::GenerateTest, vec![]);
        test.params = serde_json::json!({
            "role": "TESTER",
            "task": "Create unit tests for RateLimiter",  // Using "task" alias
            "behavior": "Rate limiter blocks requests after threshold"
        });

        let violations = checker.check_agent_task_params(&[test]);
        // Has role but missing context_files and constraints
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].severity, ViabilitySeverity::Warning);
    }

    #[test]
    fn test_v013_legacy_params_rejected() {
        let checker = ViabilityChecker::new();
        let mut edit = make_instruction("edit1", OpCode::EditCode, vec![]);
        edit.params = serde_json::json!({
            "goal": "Some valid goal",
            "role": "ENGINEER",
            "context_files": ["src/lib.rs"],
            "constraints": ["Be careful"],
            "action": "create",  // Legacy param - should be rejected
            "content_description": "Some description"  // Legacy param
        });

        let violations = checker.check_agent_task_params(&[edit]);
        // Should have 1 violation for legacy params (all required fields present)
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("legacy params"));
    }

    #[test]
    fn test_v013_other_ops_ignored() {
        let checker = ViabilityChecker::new();
        // SEARCH_CODE and READ_FILES don't require AgentTask params
        let search = make_instruction("search", OpCode::SearchCode, vec![]);
        let read = make_instruction("read", OpCode::ReadFiles, vec!["search"]);

        let violations = checker.check_agent_task_params(&[search, read]);
        assert!(violations.is_empty());
    }

    // Note: V-010 escalation tests removed - V-010 is now informational only
    // per design doc: dependencies are for sequencing, variable refs are for data flow.
    // Sequencing deps without variable refs are valid and no longer flagged as violations.

    // ========================================================================
    // V-014: Empty Instructions Tests
    // ========================================================================

    #[test]
    fn test_v014_empty_instructions() {
        let checker = ViabilityChecker::new();
        let instructions: Vec<Instruction> = vec![];

        let violation = checker.check_empty_instructions(&instructions);
        assert!(violation.is_some());
        let v = violation.unwrap();
        assert_eq!(v.rule_id, "VIABILITY-014");
        assert_eq!(v.severity, ViabilitySeverity::Critical);
        assert!(v.message.contains("empty"));
    }

    #[test]
    fn test_v014_non_empty_instructions_ok() {
        let checker = ViabilityChecker::new();
        let instructions = vec![make_instruction("step_1", OpCode::SearchCode, vec![])];

        let violation = checker.check_empty_instructions(&instructions);
        assert!(violation.is_none());
    }

    #[test]
    fn test_v014_in_check_all() {
        let checker = ViabilityChecker::new();
        let empty_instructions: Vec<Instruction> = vec![];

        let result = checker.check_all(Some(&empty_instructions), None, None);

        assert!(!result.passed);
        assert!(result
            .violations
            .iter()
            .any(|v| v.rule_id == "VIABILITY-014"));
    }

    #[test]
    fn test_v014_empty_skips_other_checks() {
        let checker = ViabilityChecker::new();
        let empty_instructions: Vec<Instruction> = vec![];

        let result = checker.check_all(Some(&empty_instructions), None, None);

        // Should only have V-014, not other violations (since other checks are skipped)
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].rule_id, "VIABILITY-014");
    }
}
