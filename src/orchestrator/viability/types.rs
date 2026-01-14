//! Core types for viability checking.

use serde::{Deserialize, Serialize};

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
