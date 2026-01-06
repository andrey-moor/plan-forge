use serde::{Deserialize, Serialize};

/// A structured development plan with phases and checkpoints
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub title: String,
    pub description: String,
    pub tier: PlanTier,
    pub context: PlanContext,
    pub phases: Vec<PlanPhase>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub file_references: Vec<FileReference>,
    pub risks: Vec<Risk>,
    pub metadata: PlanMetadata,
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
        }
    }

    /// Update the last_updated timestamp and increment version
    pub fn touch(&mut self) {
        self.metadata.last_updated = chrono::Utc::now().to_rfc3339();
        self.metadata.version += 1;
    }
}
