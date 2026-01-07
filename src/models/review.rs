use serde::{Deserialize, Serialize};

use super::plan::Severity;

/// Results from the review phase
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResult {
    pub passed: bool,
    pub hard_check_results: Vec<HardCheckResult>,
    pub llm_review: LlmReview,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardCheckResult {
    pub check_name: String,
    pub passed: bool,
    pub message: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmReview {
    pub overall_assessment: String,
    pub gaps: Vec<Gap>,
    pub unclear_areas: Vec<UnclearArea>,
    pub suggestions: Vec<Suggestion>,
    pub score: f32,
    /// Set to true if human input is needed before continuing
    #[serde(default)]
    pub requires_human_input: bool,
    /// Explanation of what input is needed from human
    #[serde(default)]
    pub human_input_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gap {
    pub description: String,
    pub location: Option<String>,
    pub severity: Severity,
    pub suggested_fix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnclearArea {
    pub description: String,
    pub questions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub description: String,
    pub rationale: String,
    pub priority: super::plan::Priority,
}

impl ReviewResult {
    /// Calculate if the review passed based on hard checks and LLM score
    pub fn calculate_passed(&mut self, threshold: f32) {
        let hard_failures = self
            .hard_check_results
            .iter()
            .filter(|r| !r.passed && r.severity == Severity::Error)
            .count();

        self.passed = hard_failures == 0 && self.llm_review.score >= threshold;
    }

    /// Extract actionable feedback from the review
    pub fn extract_feedback(&self) -> Vec<String> {
        let mut feedback = Vec::new();

        // Hard check failures
        for check in &self.hard_check_results {
            if !check.passed {
                let prefix = match check.severity {
                    Severity::Error => "[MUST FIX]",
                    Severity::Warning => "[SHOULD FIX]",
                    Severity::Info => "[CONSIDER]",
                };
                feedback.push(format!(
                    "{} {}: {}",
                    prefix, check.check_name, check.message
                ));
            }
        }

        // LLM-identified gaps
        for gap in &self.llm_review.gaps {
            let prefix = match gap.severity {
                Severity::Error => "[CRITICAL]",
                Severity::Warning => "[SHOULD FIX]",
                Severity::Info => "[CONSIDER]",
            };
            feedback.push(format!("{} {}", prefix, gap.description));
            if let Some(fix) = &gap.suggested_fix {
                feedback.push(format!("  Suggested: {}", fix));
            }
        }

        // Unclear areas
        for unclear in &self.llm_review.unclear_areas {
            feedback.push(format!("[CLARIFY] {}", unclear.description));
            for q in &unclear.questions {
                feedback.push(format!("  - {}", q));
            }
        }

        feedback
    }
}

impl Default for LlmReview {
    fn default() -> Self {
        Self {
            overall_assessment: String::new(),
            gaps: Vec::new(),
            unclear_areas: Vec::new(),
            suggestions: Vec::new(),
            score: 0.0,
            requires_human_input: false,
            human_input_reason: None,
        }
    }
}
