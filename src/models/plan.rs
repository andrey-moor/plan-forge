use serde::{Deserialize, Serialize};

// ============================================================================
// ISA (Instruction Set Architecture) Types
// ============================================================================

/// Valid fields that can be referenced from StepResult using ${instruction_id.field} pattern.
/// These correspond to the StepResult fields defined in the coding-orchestrator spec.
pub const STEP_RESULT_FIELDS: &[&str] = &[
    "output",    // Primary operation result (default)
    "stdout",    // Command stdout (for RUN_COMMAND)
    "stderr",    // Command stderr (for RUN_COMMAND)
    "exit_code", // Process exit code (for RUN_COMMAND)
    "artifacts", // List of created files
    "metadata",  // Additional key-value data
];

/// Operation codes for executable instructions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OpCode {
    // Context Operations
    /// Semantic search (via context7 or similar)
    SearchSemantic,
    /// Code search (ripgrep, grep)
    SearchCode,
    /// Read file contents
    ReadFiles,
    /// Get file imports/references
    GetDependencies,

    // Planning Operations
    /// Define subtask for delegation
    DefineTask,
    /// Verify task against rules
    VerifyTask,

    // Execution Operations
    /// Apply code changes
    EditCode,
    /// Execute shell command
    RunCommand,

    // Testing Operations
    /// Create test file
    GenerateTest,
    /// Execute test target
    RunTest,

    // Verification Operations
    /// Check file/target exists (grounding)
    VerifyExists,
}

/// An executable instruction in the plan DAG
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instruction {
    /// Unique identifier (e.g., "step_1", "locate_files")
    pub id: String,
    /// Operation to perform
    pub op: OpCode,
    /// Parameters for the operation.
    /// Variable references use ${instruction_id.field} pattern
    pub params: serde_json::Value,
    /// IDs of instructions that must complete first
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Human-readable description of what this step does
    pub description: String,
    /// Estimated context token budget for this instruction
    /// Used for planning execution scheduling and context management
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_tokens: Option<u32>,
}

impl Default for Instruction {
    fn default() -> Self {
        Self {
            id: String::new(),
            op: OpCode::VerifyExists,
            params: serde_json::Value::Null,
            dependencies: Vec::new(),
            description: String::new(),
            estimated_tokens: None,
        }
    }
}

// ============================================================================
// Grounding Snapshot Types
// ============================================================================

/// Snapshot of repository state verification before planning
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GroundingSnapshot {
    /// Files verified to exist
    #[serde(default)]
    pub verified_files: Vec<VerifiedFile>,
    /// Build targets verified (cargo targets, etc.)
    #[serde(default)]
    pub verified_targets: Vec<VerifiedTarget>,
    /// Import convention discovered (e.g., "use crate::module::Type")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_convention: Option<String>,
    /// Existing patterns found with file:line references
    #[serde(default)]
    pub existing_patterns: Vec<ExistingPattern>,
}

/// A file that was verified during grounding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedFile {
    pub path: String,
    pub exists: bool,
}

/// A build target that was verified during grounding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedTarget {
    /// Target name (e.g., "cargo test", "cargo build", "bazel test //...")
    pub target: String,
    /// Whether the target resolves/exists
    pub resolves: bool,
}

/// An existing pattern found in the codebase
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExistingPattern {
    /// Pattern description
    pub pattern: String,
    /// File path where pattern was found
    pub file: String,
    /// Line number in the file
    pub line: u32,
}

// ============================================================================
// Plan Types
// ============================================================================

/// A structured development plan with phases and checkpoints
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub title: String,
    pub description: String,
    /// Goal field for spec compliance (optional, falls back to title)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    pub tier: PlanTier,
    pub context: PlanContext,
    pub phases: Vec<PlanPhase>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub file_references: Vec<FileReference>,
    pub risks: Vec<Risk>,
    pub metadata: PlanMetadata,

    // ISA fields (optional for backward compatibility)
    /// Chain-of-thought reasoning before instructions (includes self-verification notes)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,

    /// Operator runbook - how to execute this plan with numbered bash commands
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_runbook: Option<String>,

    /// Phase 0.0 grounding gates with pass criteria and rules
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grounding_gates: Option<Vec<GroundingGate>>,

    /// Grounding snapshot verifying repo state
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grounding_snapshot: Option<GroundingSnapshot>,

    /// Executable instruction DAG
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<Vec<Instruction>>,
}

/// A grounding gate with pass criteria and failure rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundingGate {
    /// Gate identifier (e.g., "0.0.1")
    pub id: String,
    /// What to verify
    pub verification: String,
    /// Explicit success condition
    pub pass_criteria: String,
    /// What to do if verification fails
    pub rule: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanTier {
    Quick,
    Standard,
    Strategic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanContext {
    pub problem_statement: String,
    pub constraints: Vec<String>,
    pub assumptions: Vec<String>,
    pub existing_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanPhase {
    pub name: String,
    pub goal: String,
    pub tier: PhaseTier,
    pub checkpoints: Vec<Checkpoint>,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PhaseTier {
    Foundation,
    Core,
    Enhancement,
    Polish,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub description: String,
    pub tasks: Vec<Task>,
    pub validation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub description: String,
    pub file_references: Vec<String>,
    pub implementation_notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    pub description: String,
    pub testable: bool,
    pub priority: Priority,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Required,
    Recommended,
    Optional,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReference {
    pub path: String,
    pub exists: Option<bool>,
    pub action: FileAction,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileAction {
    Create,
    Modify,
    Reference,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Risk {
    pub description: String,
    pub severity: Severity,
    pub mitigation: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanMetadata {
    pub version: u32,
    pub created_at: String,
    pub last_updated: String,
    pub iteration: u32,
}

impl Plan {
    /// Create a new plan with default metadata
    pub fn new(title: String, description: String, tier: PlanTier) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            title,
            description,
            goal: None, // Will fall back to title via accessor
            tier,
            context: PlanContext {
                problem_statement: String::new(),
                constraints: Vec::new(),
                assumptions: Vec::new(),
                existing_patterns: Vec::new(),
            },
            phases: Vec::new(),
            acceptance_criteria: Vec::new(),
            file_references: Vec::new(),
            risks: Vec::new(),
            metadata: PlanMetadata {
                version: 1,
                created_at: now.clone(),
                last_updated: now,
                iteration: 1,
            },
            // ISA fields (default to None for backward compatibility)
            reasoning: None,
            operator_runbook: None,
            grounding_gates: None,
            grounding_snapshot: None,
            instructions: None,
        }
    }

    /// Update the last_updated timestamp and increment version
    pub fn touch(&mut self) {
        self.metadata.last_updated = chrono::Utc::now().to_rfc3339();
        self.metadata.version += 1;
    }

    /// Get the goal, falling back to title if not set.
    /// This provides spec-compliant access to the plan's objective.
    pub fn goal(&self) -> &str {
        self.goal.as_deref().unwrap_or(&self.title)
    }
}
