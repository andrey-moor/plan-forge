//! OrchestrationState - Persistent state for orchestrator sessions.
//!
//! This module defines the state structure that persists across orchestration
//! iterations and can be resumed after human input or system restart.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::guardrails::GuardrailHardStop;

/// Current schema version for state files.
/// Bump when adding/removing/modifying fields.
pub const SCHEMA_VERSION: u32 = 2;

// ============================================================================
// Token Breakdown
// ============================================================================

/// Breakdown of token usage by component for diagnostics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenBreakdown {
    /// Orchestrator agent input tokens
    pub orchestrator_input: u64,
    /// Orchestrator agent output tokens
    pub orchestrator_output: u64,
    /// Planner agent input tokens (via tool calls)
    pub planner_input: u64,
    /// Planner agent output tokens (via tool calls)
    pub planner_output: u64,
    /// Reviewer agent input tokens (via tool calls)
    pub reviewer_input: u64,
    /// Reviewer agent output tokens (via tool calls)
    pub reviewer_output: u64,
    /// Total tokens consumed
    pub total: u64,
    /// Whether any tokens were estimated (provider didn't report)
    pub estimated: bool,
}

impl TokenBreakdown {
    /// Calculate overhead ratio (orchestrator tokens / total tokens).
    /// Returns 0.0 if total is 0.
    pub fn overhead_ratio(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        let orchestrator_total = self.orchestrator_input + self.orchestrator_output;
        orchestrator_total as f64 / self.total as f64
    }

    /// Add orchestrator tokens.
    pub fn add_orchestrator(&mut self, input: u64, output: u64) {
        self.orchestrator_input = self.orchestrator_input.saturating_add(input);
        self.orchestrator_output = self.orchestrator_output.saturating_add(output);
        self.total = self.total.saturating_add(input).saturating_add(output);
    }

    /// Add planner tokens.
    pub fn add_planner(&mut self, input: u64, output: u64) {
        self.planner_input = self.planner_input.saturating_add(input);
        self.planner_output = self.planner_output.saturating_add(output);
        self.total = self.total.saturating_add(input).saturating_add(output);
    }

    /// Add reviewer tokens.
    pub fn add_reviewer(&mut self, input: u64, output: u64) {
        self.reviewer_input = self.reviewer_input.saturating_add(input);
        self.reviewer_output = self.reviewer_output.saturating_add(output);
        self.total = self.total.saturating_add(input).saturating_add(output);
    }
}

// ============================================================================
// Orchestration Status
// ============================================================================

/// Status of an orchestration session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OrchestrationStatus {
    /// Session is actively running
    Running,
    /// Session completed successfully (review passed)
    Completed,
    /// Session completed with best effort (review did not pass threshold)
    CompletedBestEffort,
    /// Session paused for human input (reviewer flagged requires_human_input)
    Paused {
        /// Reason for pause (from reviewer's human_input_reason)
        reason: String,
    },
    /// Session failed with error
    Failed { error: String },
    /// Session hit a hard stop (cannot be resumed)
    HardStopped { reason: GuardrailHardStop },
}

// ============================================================================
// Human Input Record
// ============================================================================

/// Record of a human input request and response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanInputRecord {
    /// The question asked
    pub question: String,
    /// Category of input (security, architecture, clarification, etc.)
    pub category: String,
    /// Human's response (None if pending)
    pub response: Option<String>,
    /// Reason for the request (from reviewer)
    pub reason: Option<String>,
    /// Iteration when request was made
    pub iteration: u32,
    /// Timestamp in ISO8601 format
    pub timestamp: String,
    /// Whether the human approved continuing
    pub approved: bool,
}

// ============================================================================
// Iteration Tracking
// ============================================================================

/// Outcome of a single planning iteration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum IterationOutcome {
    /// Viability checks failed (structural issues)
    ViabilityFailed,
    /// Viability passed but LLM review failed
    ReviewFailed,
    /// Both viability and review passed
    ReviewPassed,
    /// Human input was requested
    HumanInputRequested,
    /// Orchestrator responded with text instead of tool call
    TextResponseDetected,
}

/// Record of a single planning iteration for analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationRecord {
    /// Which iteration this was (1-indexed)
    pub iteration: u32,
    /// When this iteration completed
    pub timestamp: String,
    /// Number of viability violations
    pub viability_violations: u32,
    /// Number of critical viability violations
    pub viability_critical: u32,
    /// Whether viability checks passed
    pub viability_passed: bool,
    /// LLM review score (if viability passed)
    pub review_score: Option<f32>,
    /// Whether LLM review passed (if viability passed)
    pub review_passed: Option<bool>,
    /// Tool calls made this iteration
    pub tool_calls_this_iteration: u32,
    /// Tokens used this iteration
    pub tokens_this_iteration: u64,
    /// What happened in this iteration
    pub outcome: IterationOutcome,
}

// ============================================================================
// Orchestration State
// ============================================================================

/// Persistent state for an orchestration session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationState {
    /// Schema version for migration support
    pub schema_version: u32,
    /// Session identifier
    pub session_id: String,
    /// Original task description
    pub task: String,
    /// Working directory for planning
    pub working_dir: PathBuf,
    /// URL-friendly slug for output
    pub task_slug: String,
    /// Current iteration number
    pub iteration: u32,
    /// Total tool calls made
    pub tool_calls: u32,
    /// Total tokens consumed (u64 for headroom)
    pub total_tokens: u64,
    /// Session start time in ISO8601 format
    pub start_time_iso: String,
    /// Current session status
    pub status: OrchestrationStatus,
    /// Current plan JSON (if generated)
    pub current_plan: Option<Value>,
    /// History of review results
    pub reviews: Vec<Value>,
    /// History of human input requests/responses
    pub human_inputs: Vec<HumanInputRecord>,
    /// Summary of session context (for LLM)
    pub context_summary: String,
    /// Pending human input request (not yet answered)
    pub pending_human_input: Option<HumanInputRecord>,
    /// Token usage breakdown by component
    #[serde(default)]
    pub token_breakdown: TokenBreakdown,
    /// Per-iteration history for progress analysis
    #[serde(default)]
    pub iteration_history: Vec<IterationRecord>,
    /// Set to true by review_plan when reviewer returns requires_human_input=true.
    /// Reset to false after legitimate request_human_input call or at start of new review.
    /// Used to validate that request_human_input is only called when authorized.
    #[serde(default)]
    pub requires_human_input_pending: bool,
    /// Set by review_plan to track whether the last review passed.
    /// Used by finalize to validate the call is legitimate.
    #[serde(default)]
    pub last_review_passed: bool,
    /// Best plan seen so far (highest scoring)
    #[serde(default)]
    pub best_plan: Option<Value>,
    /// Best review score seen so far
    #[serde(default)]
    pub best_score: f32,
    /// Set to true by generate_plan when a new plan is created.
    /// Set to false by review_plan after the plan has been reviewed.
    /// Used to ensure the orchestrator calls review_plan after generate_plan.
    #[serde(default)]
    pub needs_review: bool,
}

impl OrchestrationState {
    /// Create a new orchestration state for a task.
    pub fn new(session_id: String, task: String, working_dir: PathBuf, task_slug: String) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            session_id,
            task: task.clone(),
            working_dir,
            task_slug,
            iteration: 0,
            tool_calls: 0,
            total_tokens: 0,
            start_time_iso: chrono::Utc::now().to_rfc3339(),
            status: OrchestrationStatus::Running,
            current_plan: None,
            reviews: Vec::new(),
            human_inputs: Vec::new(),
            context_summary: format!("Task: {}", task.chars().take(200).collect::<String>()),
            pending_human_input: None,
            token_breakdown: TokenBreakdown::default(),
            iteration_history: Vec::new(),
            requires_human_input_pending: false,
            last_review_passed: false,
            best_plan: None,
            best_score: 0.0,
            needs_review: false,
        }
    }

    /// Add tokens to the total, handling Option<i32> input safely.
    /// Treats None and negative values as 0, uses saturating_add for overflow.
    pub fn add_tokens(&mut self, input_tokens: Option<i32>, output_tokens: Option<i32>) {
        let input = input_tokens.map(|t| t.max(0) as u64).unwrap_or(0);
        let output = output_tokens.map(|t| t.max(0) as u64).unwrap_or(0);
        self.total_tokens = self
            .total_tokens
            .saturating_add(input)
            .saturating_add(output);
    }

    /// Save state to a JSON file using atomic write pattern.
    pub fn save(&self, session_dir: &Path) -> Result<()> {
        fs::create_dir_all(session_dir).context("Failed to create session directory")?;

        let state_file = session_dir.join("orchestration-state.json");
        let temp_file = session_dir.join(".orchestration-state.json.tmp");

        let json = serde_json::to_string_pretty(self).context("Failed to serialize state")?;

        // Write to temp file first
        fs::write(&temp_file, &json).context("Failed to write temp state file")?;

        // Atomic rename
        fs::rename(&temp_file, &state_file).context("Failed to rename state file")?;

        Ok(())
    }

    /// Load state from a session directory.
    /// Returns None if file doesn't exist or migration fails.
    pub fn load(session_dir: &Path) -> Result<Option<Self>> {
        let state_file = session_dir.join("orchestration-state.json");

        if !state_file.exists() {
            return Ok(None);
        }

        let json = fs::read_to_string(&state_file).context("Failed to read state file")?;
        let state: Self = serde_json::from_str(&json).context("Failed to parse state file")?;

        // Check schema version and migrate if needed
        if state.schema_version != SCHEMA_VERSION {
            tracing::warn!(
                "State schema version mismatch: {} vs {}. Attempting migration.",
                state.schema_version,
                SCHEMA_VERSION
            );
            return Self::migrate(state);
        }

        Ok(Some(state))
    }

    /// Attempt to migrate state from older schema version.
    fn migrate(state: Self) -> Result<Option<Self>> {
        // Currently only version 1 exists, so no migrations needed yet
        // Future migrations would go here:
        // if state.schema_version == 0 {
        //     // migrate from v0 to v1
        //     state.schema_version = 1;
        // }

        if state.schema_version != SCHEMA_VERSION {
            tracing::warn!(
                "Cannot migrate state from version {} to {}. Starting fresh.",
                state.schema_version,
                SCHEMA_VERSION
            );
            // Preserve human_inputs for audit trail even when starting fresh
            tracing::info!(
                "Preserved {} human input records from old state",
                state.human_inputs.len()
            );
            return Ok(None);
        }

        Ok(Some(state))
    }

    /// Calculate elapsed duration since session start.
    pub fn elapsed_duration(&self) -> chrono::Duration {
        let start = chrono::DateTime::parse_from_rfc3339(&self.start_time_iso)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());
        chrono::Utc::now() - start
    }

    /// Check if the session can be resumed (not in hard-stopped state).
    pub fn can_resume(&self) -> bool {
        !matches!(self.status, OrchestrationStatus::HardStopped { .. })
    }

    /// Generate a context summary for the LLM.
    pub fn generate_context_summary(&self) -> String {
        let mut summary = Vec::new();

        // Task (truncated)
        summary.push(format!(
            "Task: {}",
            self.task.chars().take(200).collect::<String>()
        ));

        // Progress
        summary.push(format!("Iteration: {}", self.iteration));
        summary.push(format!("Total tokens: {}", self.total_tokens));

        // Last review summary
        if let Some(last_review) = self.reviews.last()
            && let Some(review_summary) = last_review.get("summary").and_then(|v| v.as_str())
        {
            summary.push(format!(
                "Last review: {}",
                review_summary.chars().take(200).collect::<String>()
            ));
        }

        // Human inputs
        for input in &self.human_inputs {
            if input.approved {
                summary.push(format!(
                    "Human approved {:?}: {}",
                    input.reason.as_deref().unwrap_or("(unknown reason)"),
                    input.response.as_deref().unwrap_or("(no response)")
                ));
            }
        }

        // Cap total length
        let full_summary = summary.join("\n");
        if full_summary.len() > 2000 {
            format!("{}...", full_summary.chars().take(1997).collect::<String>())
        } else {
            full_summary
        }
    }
}

// ============================================================================
// Human Response (for resume)
// ============================================================================

/// Human response provided when resuming a paused session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanResponse {
    /// The response text
    pub response: String,
    /// Whether the human approves continuing
    pub approved: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_state_creation() {
        let state = OrchestrationState::new(
            "test-session".to_string(),
            "Build a web app".to_string(),
            PathBuf::from("/tmp"),
            "build-web-app".to_string(),
        );

        assert_eq!(state.session_id, "test-session");
        assert_eq!(state.iteration, 0);
        assert_eq!(state.total_tokens, 0);
        assert_eq!(state.schema_version, SCHEMA_VERSION);
        assert!(matches!(state.status, OrchestrationStatus::Running));
    }

    #[test]
    fn test_add_tokens() {
        let mut state = OrchestrationState::new(
            "test".to_string(),
            "task".to_string(),
            PathBuf::new(),
            "slug".to_string(),
        );

        state.add_tokens(Some(100), Some(200));
        assert_eq!(state.total_tokens, 300);

        state.add_tokens(None, Some(50));
        assert_eq!(state.total_tokens, 350);

        state.add_tokens(Some(-10), Some(10)); // Negative treated as 0
        assert_eq!(state.total_tokens, 360);
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempdir().unwrap();
        let session_dir = dir.path().join("test-session");

        let mut state = OrchestrationState::new(
            "test-session".to_string(),
            "Build something".to_string(),
            PathBuf::from("/tmp"),
            "build-something".to_string(),
        );
        state.iteration = 3;
        state.total_tokens = 5000;

        state.save(&session_dir).unwrap();

        let loaded = OrchestrationState::load(&session_dir).unwrap().unwrap();
        assert_eq!(loaded.session_id, "test-session");
        assert_eq!(loaded.iteration, 3);
        assert_eq!(loaded.total_tokens, 5000);
    }

    #[test]
    fn test_can_resume() {
        let mut state = OrchestrationState::new(
            "test".to_string(),
            "task".to_string(),
            PathBuf::new(),
            "slug".to_string(),
        );

        assert!(state.can_resume());

        state.status = OrchestrationStatus::Paused {
            reason: "test".to_string(),
        };
        assert!(state.can_resume());

        state.status = OrchestrationStatus::HardStopped {
            reason: super::super::guardrails::GuardrailHardStop::ExecutionTimeout,
        };
        assert!(!state.can_resume());
    }
}
