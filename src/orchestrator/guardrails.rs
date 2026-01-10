//! Guardrails - Hard limits and mandatory conditions enforced in Rust.
//!
//! This module implements guardrails that cannot be bypassed by the LLM orchestrator.
//! All 6 mandatory human input conditions are checked here, plus hard stops for
//! token budget, iteration limits, and execution timeout.

use glob::Pattern;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use super::orchestration_state::OrchestrationState;

// Re-export from config for convenience
pub use crate::config::GuardrailsConfig;

// ============================================================================
// Mandatory Conditions (require human approval)
// ============================================================================

/// Mandatory conditions that require human approval before proceeding.
/// These are enforced in Rust code and cannot be bypassed by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MandatoryCondition {
    /// Plan involves security-sensitive operations (credentials, auth, encryption)
    SecuritySensitive {
        keywords: Vec<String>,
        locations: Vec<String>,
    },
    /// Plan modifies sensitive files (.env, .pem, .key, secrets)
    SensitiveFilePattern { files: Vec<String> },
    /// Review score below threshold (default 0.5)
    LowScoreThreshold { score: f32, threshold: f32 },
    /// Reached iteration soft limit (default 7)
    IterationSoftLimit { iteration: u32, limit: u32 },
    /// Plan modifies public API signatures
    BreakingApiChanges { locations: Vec<String> },
    /// Plan includes data deletion operations
    DataDeletionOperations { operations: Vec<String> },
}

// ============================================================================
// Hard Stops (non-bypassable limits)
// ============================================================================

/// Hard stops that terminate the session - cannot be approved by human input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum GuardrailHardStop {
    /// Token budget exhausted
    TokenBudgetExhausted { used: u64, limit: u64 },
    /// Maximum iterations exceeded
    MaxIterationsExceeded { iteration: u32, limit: u32 },
    /// Maximum tool calls exceeded
    MaxToolCallsExceeded { calls: u32, limit: u32 },
    /// Execution timeout
    ExecutionTimeout,
    /// Execution error (agent/provider failure)
    ExecutionError { message: String },
}

// ============================================================================
// Guardrails Configuration
// ============================================================================

/// Guardrails struct with all configuration and check methods.
#[derive(Debug, Clone)]
pub struct Guardrails {
    /// Maximum iterations before hard stop
    pub max_iterations: u32,
    /// Maximum tool calls before hard stop
    pub max_tool_calls: u32,
    /// Maximum total tokens before hard stop (default 500,000)
    pub max_total_tokens: u64,
    /// Execution timeout
    pub execution_timeout: Duration,
    /// Iteration soft limit (triggers human input request)
    pub iteration_soft_limit: u32,
    /// Review score threshold for low score condition
    pub low_score_threshold: f32,
    /// Security keywords to detect
    pub security_keywords: Vec<String>,
    /// Sensitive file patterns (glob)
    pub sensitive_file_patterns: Vec<String>,
    /// Breaking API patterns
    pub breaking_api_patterns: Vec<String>,
    /// Data deletion patterns
    pub data_deletion_patterns: Vec<String>,
}

impl Default for Guardrails {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            max_tool_calls: 100,
            max_total_tokens: 500_000,
            execution_timeout: Duration::from_secs(600), // 10 minutes
            iteration_soft_limit: 7,
            low_score_threshold: 0.5,
            security_keywords: vec![
                "credential".to_string(),
                "auth".to_string(),
                "encrypt".to_string(),
                "secret".to_string(),
                "token".to_string(),
                "password".to_string(),
                "api_key".to_string(),
                "private_key".to_string(),
                "certificate".to_string(),
            ],
            sensitive_file_patterns: vec![
                "*.env".to_string(),
                "*.env.*".to_string(),
                "*secret*".to_string(),
                "*credential*".to_string(),
                "*.pem".to_string(),
                "*.key".to_string(),
                "**/secrets/**".to_string(),
            ],
            breaking_api_patterns: vec![
                "pub fn".to_string(),
                "pub struct".to_string(),
                "pub enum".to_string(),
                "pub trait".to_string(),
            ],
            data_deletion_patterns: vec![
                "DROP TABLE".to_string(),
                "DELETE FROM".to_string(),
                "TRUNCATE".to_string(),
                "rm -rf".to_string(),
                "shutil.rmtree".to_string(),
            ],
        }
    }
}

impl Guardrails {
    /// Create guardrails from configuration.
    pub fn from_config(config: &GuardrailsConfig) -> Self {
        Self {
            max_iterations: config.max_iterations,
            max_tool_calls: config.max_tool_calls,
            max_total_tokens: config.max_total_tokens,
            execution_timeout: Duration::from_secs(config.execution_timeout_secs),
            iteration_soft_limit: config.iteration_soft_limit,
            low_score_threshold: config.low_score_threshold,
            security_keywords: config.security_keywords.clone(),
            sensitive_file_patterns: config.sensitive_file_patterns.clone(),
            breaking_api_patterns: config.breaking_api_patterns.clone(),
            data_deletion_patterns: config.data_deletion_patterns.clone(),
        }
    }

    // ========================================================================
    // Hard Stop Checks
    // ========================================================================

    /// Check before any tool call - returns error if hard limit exceeded.
    pub fn check_before_tool_call(&self, state: &OrchestrationState) -> Result<(), GuardrailHardStop> {
        if state.total_tokens >= self.max_total_tokens {
            return Err(GuardrailHardStop::TokenBudgetExhausted {
                used: state.total_tokens,
                limit: self.max_total_tokens,
            });
        }

        if state.iteration >= self.max_iterations {
            return Err(GuardrailHardStop::MaxIterationsExceeded {
                iteration: state.iteration,
                limit: self.max_iterations,
            });
        }

        if state.tool_calls >= self.max_tool_calls {
            return Err(GuardrailHardStop::MaxToolCallsExceeded {
                calls: state.tool_calls,
                limit: self.max_tool_calls,
            });
        }

        Ok(())
    }

    // ========================================================================
    // Mandatory Condition Checks (6 conditions)
    // ========================================================================

    /// Check for security-sensitive content in plan (Condition 1).
    pub fn check_security_sensitive(&self, plan_json: &Value) -> Option<MandatoryCondition> {
        let mut found_keywords = Vec::new();
        let mut locations = Vec::new();

        // Recursively search for security keywords
        self.scan_for_keywords(plan_json, &self.security_keywords, "", &mut found_keywords, &mut locations);

        if !found_keywords.is_empty() {
            Some(MandatoryCondition::SecuritySensitive {
                keywords: found_keywords,
                locations,
            })
        } else {
            None
        }
    }

    /// Check for sensitive file patterns in plan (Condition 2).
    pub fn check_sensitive_files(&self, plan_json: &Value) -> Option<MandatoryCondition> {
        let mut sensitive_files = Vec::new();

        // Extract file_references paths
        if let Some(refs) = plan_json.get("file_references").and_then(|v| v.as_array()) {
            for file_ref in refs {
                if let Some(path) = file_ref.get("path").and_then(|v| v.as_str()) {
                    for pattern_str in &self.sensitive_file_patterns {
                        if let Ok(pattern) = Pattern::new(pattern_str) {
                            if pattern.matches(path) {
                                sensitive_files.push(path.to_string());
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Also check in phases/tasks for file references
        self.scan_for_file_paths(plan_json, &mut sensitive_files);

        if !sensitive_files.is_empty() {
            Some(MandatoryCondition::SensitiveFilePattern {
                files: sensitive_files,
            })
        } else {
            None
        }
    }

    /// Check for low review score (Condition 3).
    pub fn check_low_score(&self, score: f32) -> Option<MandatoryCondition> {
        if score < self.low_score_threshold {
            Some(MandatoryCondition::LowScoreThreshold {
                score,
                threshold: self.low_score_threshold,
            })
        } else {
            None
        }
    }

    /// Check for iteration soft limit (Condition 4).
    pub fn check_iteration_limit(&self, iteration: u32) -> Option<MandatoryCondition> {
        if iteration >= self.iteration_soft_limit {
            Some(MandatoryCondition::IterationSoftLimit {
                iteration,
                limit: self.iteration_soft_limit,
            })
        } else {
            None
        }
    }

    /// Check for breaking API changes in plan (Condition 5).
    pub fn check_breaking_api_changes(&self, plan_json: &Value) -> Option<MandatoryCondition> {
        let mut locations = Vec::new();

        // Look for tasks that modify public APIs
        self.scan_for_patterns(
            plan_json,
            &self.breaking_api_patterns,
            &["modify", "change", "update", "refactor"],
            &mut locations,
        );

        if !locations.is_empty() {
            Some(MandatoryCondition::BreakingApiChanges { locations })
        } else {
            None
        }
    }

    /// Check for data deletion operations in plan (Condition 6).
    pub fn check_data_deletion(&self, plan_json: &Value) -> Option<MandatoryCondition> {
        let mut operations = Vec::new();

        // Scan all string content for deletion patterns
        self.scan_text_for_patterns(plan_json, &self.data_deletion_patterns, &mut operations);

        if !operations.is_empty() {
            Some(MandatoryCondition::DataDeletionOperations { operations })
        } else {
            None
        }
    }

    // ========================================================================
    // Aggregate Checks
    // ========================================================================

    /// Run all 6 mandatory condition checks and return triggered conditions.
    pub fn check_all_conditions(
        &self,
        plan_json: &Value,
        score: f32,
        iteration: u32,
    ) -> Vec<MandatoryCondition> {
        let mut conditions = Vec::new();

        if let Some(c) = self.check_security_sensitive(plan_json) {
            conditions.push(c);
        }
        if let Some(c) = self.check_sensitive_files(plan_json) {
            conditions.push(c);
        }
        if let Some(c) = self.check_low_score(score) {
            conditions.push(c);
        }
        if let Some(c) = self.check_iteration_limit(iteration) {
            conditions.push(c);
        }
        if let Some(c) = self.check_breaking_api_changes(plan_json) {
            conditions.push(c);
        }
        if let Some(c) = self.check_data_deletion(plan_json) {
            conditions.push(c);
        }

        conditions
    }

    /// Check before finalize - ensures all mandatory conditions are approved.
    pub fn check_before_finalize(
        &self,
        plan_json: &Value,
        state: &OrchestrationState,
    ) -> Result<(), MandatoryCondition> {
        let score = state
            .reviews
            .last()
            .and_then(|r| r.get("score").and_then(|v| v.as_f64()))
            .unwrap_or(0.0) as f32;

        let triggered = self.check_all_conditions(plan_json, score, state.iteration);

        // Check each triggered condition against approved human inputs
        for condition in triggered {
            let is_approved = state.human_inputs.iter().any(|input| {
                input.approved
                    && input.condition.as_ref().map(|c| {
                        std::mem::discriminant(c) == std::mem::discriminant(&condition)
                    }).unwrap_or(false)
            });

            if !is_approved {
                return Err(condition);
            }
        }

        Ok(())
    }

    // ========================================================================
    // Helper Methods
    // ========================================================================

    /// Recursively scan JSON for keywords.
    fn scan_for_keywords(
        &self,
        value: &Value,
        keywords: &[String],
        path: &str,
        found: &mut Vec<String>,
        locations: &mut Vec<String>,
    ) {
        match value {
            Value::String(s) => {
                let lower = s.to_lowercase();
                for keyword in keywords {
                    if lower.contains(&keyword.to_lowercase()) {
                        if !found.contains(keyword) {
                            found.push(keyword.clone());
                        }
                        let loc = if path.is_empty() {
                            s.chars().take(50).collect()
                        } else {
                            format!("{}: {}", path, s.chars().take(30).collect::<String>())
                        };
                        if !locations.contains(&loc) {
                            locations.push(loc);
                        }
                    }
                }
            }
            Value::Array(arr) => {
                for (i, item) in arr.iter().enumerate() {
                    let new_path = format!("{}[{}]", path, i);
                    self.scan_for_keywords(item, keywords, &new_path, found, locations);
                }
            }
            Value::Object(obj) => {
                for (key, val) in obj {
                    let new_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{}.{}", path, key)
                    };
                    self.scan_for_keywords(val, keywords, &new_path, found, locations);
                }
            }
            _ => {}
        }
    }

    /// Scan for file paths matching sensitive patterns.
    fn scan_for_file_paths(&self, value: &Value, sensitive_files: &mut Vec<String>) {
        match value {
            Value::String(s) => {
                // Check if this looks like a file path
                if s.contains('/') || s.contains('\\') || s.contains('.') {
                    for pattern_str in &self.sensitive_file_patterns {
                        if let Ok(pattern) = Pattern::new(pattern_str) {
                            if pattern.matches(s) && !sensitive_files.contains(s) {
                                sensitive_files.push(s.clone());
                                break;
                            }
                        }
                    }
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    self.scan_for_file_paths(item, sensitive_files);
                }
            }
            Value::Object(obj) => {
                for val in obj.values() {
                    self.scan_for_file_paths(val, sensitive_files);
                }
            }
            _ => {}
        }
    }

    /// Scan for patterns in combination with action keywords.
    fn scan_for_patterns(
        &self,
        value: &Value,
        patterns: &[String],
        action_keywords: &[&str],
        locations: &mut Vec<String>,
    ) {
        match value {
            Value::String(s) => {
                let lower = s.to_lowercase();
                let has_action = action_keywords.iter().any(|kw| lower.contains(kw));
                if has_action {
                    for pattern in patterns {
                        if lower.contains(&pattern.to_lowercase()) {
                            let loc = s.chars().take(60).collect();
                            if !locations.contains(&loc) {
                                locations.push(loc);
                            }
                        }
                    }
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    self.scan_for_patterns(item, patterns, action_keywords, locations);
                }
            }
            Value::Object(obj) => {
                for val in obj.values() {
                    self.scan_for_patterns(val, patterns, action_keywords, locations);
                }
            }
            _ => {}
        }
    }

    /// Scan all text content for patterns.
    fn scan_text_for_patterns(&self, value: &Value, patterns: &[String], found: &mut Vec<String>) {
        match value {
            Value::String(s) => {
                for pattern in patterns {
                    if s.to_lowercase().contains(&pattern.to_lowercase()) {
                        let snippet: String = s.chars().take(60).collect();
                        if !found.contains(&snippet) {
                            found.push(snippet);
                        }
                    }
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    self.scan_text_for_patterns(item, patterns, found);
                }
            }
            Value::Object(obj) => {
                for val in obj.values() {
                    self.scan_text_for_patterns(val, patterns, found);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_security_sensitive_detection() {
        let guardrails = Guardrails::default();
        let plan = json!({
            "title": "Add authentication",
            "description": "Implement password hashing with bcrypt",
            "phases": [{
                "name": "Auth",
                "tasks": [{
                    "description": "Store user credentials securely"
                }]
            }]
        });

        let result = guardrails.check_security_sensitive(&plan);
        assert!(result.is_some());
        if let Some(MandatoryCondition::SecuritySensitive { keywords, .. }) = result {
            assert!(keywords.contains(&"password".to_string()) || keywords.contains(&"credential".to_string()));
        }
    }

    #[test]
    fn test_sensitive_file_detection() {
        let guardrails = Guardrails::default();
        let plan = json!({
            "file_references": [
                { "path": ".env", "action": "modify" },
                { "path": "src/main.rs", "action": "modify" }
            ]
        });

        let result = guardrails.check_sensitive_files(&plan);
        assert!(result.is_some());
        if let Some(MandatoryCondition::SensitiveFilePattern { files }) = result {
            assert!(files.contains(&".env".to_string()));
        }
    }

    #[test]
    fn test_low_score_detection() {
        let guardrails = Guardrails::default();

        assert!(guardrails.check_low_score(0.3).is_some());
        assert!(guardrails.check_low_score(0.5).is_none());
        assert!(guardrails.check_low_score(0.8).is_none());
    }

    #[test]
    fn test_iteration_limit_detection() {
        let guardrails = Guardrails::default();

        assert!(guardrails.check_iteration_limit(5).is_none());
        assert!(guardrails.check_iteration_limit(7).is_some());
        assert!(guardrails.check_iteration_limit(10).is_some());
    }

    #[test]
    fn test_data_deletion_detection() {
        let guardrails = Guardrails::default();
        let plan = json!({
            "phases": [{
                "tasks": [{
                    "description": "Clean up old data",
                    "implementation_notes": "Run DELETE FROM users WHERE inactive = true"
                }]
            }]
        });

        let result = guardrails.check_data_deletion(&plan);
        assert!(result.is_some());
    }

    #[test]
    fn test_hard_stop_token_budget() {
        let guardrails = Guardrails {
            max_total_tokens: 1000,
            ..Default::default()
        };

        let state = OrchestrationState {
            total_tokens: 1500,
            ..OrchestrationState::new("test".to_string(), "task".to_string(), std::path::PathBuf::new(), "slug".to_string())
        };

        let result = guardrails.check_before_tool_call(&state);
        assert!(matches!(result, Err(GuardrailHardStop::TokenBudgetExhausted { .. })));
    }

    #[test]
    fn test_breaking_api_changes_detection() {
        let guardrails = Guardrails::default();
        let plan = json!({
            "phases": [{
                "tasks": [{
                    "description": "Refactor user module",
                    "implementation_notes": "Modify pub fn get_user() signature to return Result"
                }]
            }]
        });

        let result = guardrails.check_breaking_api_changes(&plan);
        assert!(result.is_some());
        if let Some(MandatoryCondition::BreakingApiChanges { locations }) = result {
            assert!(!locations.is_empty());
        }
    }

    #[test]
    fn test_breaking_api_no_trigger_without_action() {
        let guardrails = Guardrails::default();
        // Plan mentions pub fn but no modify/change/update action
        let plan = json!({
            "description": "Add new pub fn helper()"
        });

        let result = guardrails.check_breaking_api_changes(&plan);
        assert!(result.is_none());
    }

    #[test]
    fn test_check_all_conditions_multiple() {
        let guardrails = Guardrails::default();
        let plan = json!({
            "title": "Security update with data cleanup",
            "description": "Update password handling and clean old records",
            "phases": [{
                "tasks": [{
                    "description": "Store credentials securely",
                    "implementation_notes": "DELETE FROM old_users WHERE expired = true"
                }]
            }],
            "file_references": [
                { "path": ".env.production", "action": "modify" }
            ]
        });

        let conditions = guardrails.check_all_conditions(&plan, 0.3, 8);

        // Should trigger: security sensitive (credentials/password), low score, iteration limit,
        // data deletion (DELETE FROM), sensitive file (.env.production)
        assert!(conditions.len() >= 4);

        let has_security = conditions.iter().any(|c| matches!(c, MandatoryCondition::SecuritySensitive { .. }));
        let has_low_score = conditions.iter().any(|c| matches!(c, MandatoryCondition::LowScoreThreshold { .. }));
        let has_iteration = conditions.iter().any(|c| matches!(c, MandatoryCondition::IterationSoftLimit { .. }));
        let has_deletion = conditions.iter().any(|c| matches!(c, MandatoryCondition::DataDeletionOperations { .. }));

        assert!(has_security, "Should detect security sensitive content");
        assert!(has_low_score, "Should detect low score");
        assert!(has_iteration, "Should detect iteration limit");
        assert!(has_deletion, "Should detect data deletion");
    }

    #[test]
    fn test_check_all_conditions_none_triggered() {
        let guardrails = Guardrails::default();
        let plan = json!({
            "title": "Simple feature",
            "description": "Add a button to the UI",
            "phases": [{
                "tasks": [{
                    "description": "Create button component"
                }]
            }]
        });

        let conditions = guardrails.check_all_conditions(&plan, 0.9, 2);
        assert!(conditions.is_empty());
    }

    #[test]
    fn test_check_before_finalize_blocks_unapproved() {
        let guardrails = Guardrails::default();
        let plan = json!({
            "description": "Handle user credentials"
        });

        let state = OrchestrationState::new(
            "test".to_string(),
            "task".to_string(),
            std::path::PathBuf::new(),
            "slug".to_string(),
        );

        let result = guardrails.check_before_finalize(&plan, &state);
        assert!(result.is_err());
        assert!(matches!(result, Err(MandatoryCondition::SecuritySensitive { .. })));
    }

    #[test]
    fn test_check_before_finalize_allows_approved() {
        use super::super::orchestration_state::HumanInputRecord;

        let guardrails = Guardrails::default();
        let plan = json!({
            "description": "Handle user credentials"
        });

        let mut state = OrchestrationState::new(
            "test".to_string(),
            "task".to_string(),
            std::path::PathBuf::new(),
            "slug".to_string(),
        );

        // Add approved human input for security condition
        state.human_inputs.push(HumanInputRecord {
            question: "Plan involves credentials. Proceed?".to_string(),
            category: "security".to_string(),
            response: Some("Yes, proceed".to_string()),
            condition: Some(MandatoryCondition::SecuritySensitive {
                keywords: vec!["credential".to_string()],
                locations: vec![],
            }),
            iteration: 1,
            timestamp: chrono::Utc::now().to_rfc3339(),
            approved: true,
        });

        // Add a fake review with passing score
        state.reviews.push(json!({ "score": 0.9 }));

        let result = guardrails.check_before_finalize(&plan, &state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_hard_stop_max_iterations() {
        let guardrails = Guardrails {
            max_iterations: 5,
            ..Default::default()
        };

        let mut state = OrchestrationState::new(
            "test".to_string(),
            "task".to_string(),
            std::path::PathBuf::new(),
            "slug".to_string(),
        );
        state.iteration = 6;

        let result = guardrails.check_before_tool_call(&state);
        assert!(matches!(result, Err(GuardrailHardStop::MaxIterationsExceeded { iteration: 6, limit: 5 })));
    }

    #[test]
    fn test_hard_stop_max_tool_calls() {
        let guardrails = Guardrails {
            max_tool_calls: 50,
            ..Default::default()
        };

        let mut state = OrchestrationState::new(
            "test".to_string(),
            "task".to_string(),
            std::path::PathBuf::new(),
            "slug".to_string(),
        );
        state.tool_calls = 51;

        let result = guardrails.check_before_tool_call(&state);
        assert!(matches!(result, Err(GuardrailHardStop::MaxToolCallsExceeded { calls: 51, limit: 50 })));
    }

    #[test]
    fn test_hard_stop_within_limits() {
        let guardrails = Guardrails::default();

        let mut state = OrchestrationState::new(
            "test".to_string(),
            "task".to_string(),
            std::path::PathBuf::new(),
            "slug".to_string(),
        );
        state.iteration = 3;
        state.tool_calls = 10;
        state.total_tokens = 50_000;

        let result = guardrails.check_before_tool_call(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_sensitive_file_nested_in_tasks() {
        let guardrails = Guardrails::default();
        let plan = json!({
            "phases": [{
                "tasks": [{
                    "description": "Update configuration",
                    "files": ["config/secrets/api_keys.json"]
                }]
            }]
        });

        let result = guardrails.check_sensitive_files(&plan);
        assert!(result.is_some());
        if let Some(MandatoryCondition::SensitiveFilePattern { files }) = result {
            assert!(files.iter().any(|f| f.contains("secrets")));
        }
    }

    #[test]
    fn test_security_keywords_case_insensitive() {
        let guardrails = Guardrails::default();
        let plan = json!({
            "description": "Update PASSWORD handling and API_KEY rotation"
        });

        let result = guardrails.check_security_sensitive(&plan);
        assert!(result.is_some());
        if let Some(MandatoryCondition::SecuritySensitive { keywords, .. }) = result {
            assert!(keywords.contains(&"password".to_string()) || keywords.contains(&"api_key".to_string()));
        }
    }

    #[test]
    fn test_data_deletion_patterns_various() {
        let guardrails = Guardrails::default();

        // Test DROP TABLE
        let plan1 = json!({ "task": "Run DROP TABLE temp_data" });
        assert!(guardrails.check_data_deletion(&plan1).is_some());

        // Test TRUNCATE
        let plan2 = json!({ "task": "TRUNCATE logs table" });
        assert!(guardrails.check_data_deletion(&plan2).is_some());

        // Test rm -rf
        let plan3 = json!({ "task": "Clean up with rm -rf /tmp/cache" });
        assert!(guardrails.check_data_deletion(&plan3).is_some());

        // Test shutil.rmtree
        let plan4 = json!({ "task": "Use shutil.rmtree to remove directory" });
        assert!(guardrails.check_data_deletion(&plan4).is_some());

        // Non-matching should return None
        let plan5 = json!({ "task": "Create new table" });
        assert!(guardrails.check_data_deletion(&plan5).is_none());
    }

    // ========================================================================
    // Guardrail Bypass Prevention Tests
    // These tests explicitly prove that guardrails cannot be bypassed
    // ========================================================================

    #[test]
    fn test_cannot_bypass_finalize_with_multiple_conditions() {
        use super::super::orchestration_state::HumanInputRecord;

        let guardrails = Guardrails::default();
        // Plan triggers both security AND data deletion conditions
        let plan = json!({
            "description": "Update password handling and run DELETE FROM users"
        });

        let mut state = OrchestrationState::new(
            "test".to_string(),
            "task".to_string(),
            std::path::PathBuf::new(),
            "slug".to_string(),
        );

        // Only approve security, not data deletion
        state.human_inputs.push(HumanInputRecord {
            question: "Security concern".to_string(),
            category: "security".to_string(),
            response: Some("Yes".to_string()),
            condition: Some(MandatoryCondition::SecuritySensitive {
                keywords: vec!["password".to_string()],
                locations: vec![],
            }),
            iteration: 1,
            timestamp: chrono::Utc::now().to_rfc3339(),
            approved: true,
        });
        state.reviews.push(json!({ "score": 0.9 }));

        // Should still be blocked because DataDeletion is not approved
        let result = guardrails.check_before_finalize(&plan, &state);
        assert!(result.is_err());
        assert!(matches!(result, Err(MandatoryCondition::DataDeletionOperations { .. })));
    }

    #[test]
    fn test_cannot_bypass_security_with_obfuscation() {
        let guardrails = Guardrails::default();

        // Try to hide security keywords in nested JSON
        let plan = json!({
            "phases": [{
                "tasks": [{
                    "subtasks": [{
                        "details": [{
                            "implementation": "Store the api_key in environment variable"
                        }]
                    }]
                }]
            }]
        });

        let result = guardrails.check_security_sensitive(&plan);
        assert!(result.is_some(), "Security keyword should be detected even when deeply nested");
    }

    #[test]
    fn test_cannot_bypass_hard_stop_even_at_boundary() {
        let guardrails = Guardrails {
            max_iterations: 5,
            max_tool_calls: 10,
            max_total_tokens: 1000,
            ..Default::default()
        };

        // Test at exact boundary (should still pass)
        let mut state = OrchestrationState::new(
            "test".to_string(),
            "task".to_string(),
            std::path::PathBuf::new(),
            "slug".to_string(),
        );
        state.iteration = 5;
        state.tool_calls = 10;
        state.total_tokens = 1000;

        let result = guardrails.check_before_tool_call(&state);
        assert!(result.is_err(), "Should fail at exact boundary");

        // Test at boundary - 1 (should pass)
        state.iteration = 4;
        state.tool_calls = 9;
        state.total_tokens = 999;

        let result = guardrails.check_before_tool_call(&state);
        assert!(result.is_ok(), "Should pass just below boundary");
    }

    #[test]
    fn test_approval_requires_matching_condition() {
        use super::super::orchestration_state::HumanInputRecord;

        let guardrails = Guardrails::default();
        let plan = json!({
            "description": "Handle user credentials"
        });

        let mut state = OrchestrationState::new(
            "test".to_string(),
            "task".to_string(),
            std::path::PathBuf::new(),
            "slug".to_string(),
        );

        // Approve a DIFFERENT condition type (iteration limit, not security)
        state.human_inputs.push(HumanInputRecord {
            question: "Continue past iteration limit?".to_string(),
            category: "clarification".to_string(),
            response: Some("Yes".to_string()),
            condition: Some(MandatoryCondition::IterationSoftLimit { iteration: 7, limit: 7 }),
            iteration: 7,
            timestamp: chrono::Utc::now().to_rfc3339(),
            approved: true,
        });
        state.reviews.push(json!({ "score": 0.9 }));

        // Should still block because SecuritySensitive is NOT approved
        let result = guardrails.check_before_finalize(&plan, &state);
        assert!(result.is_err());
        assert!(matches!(result, Err(MandatoryCondition::SecuritySensitive { .. })));
    }

    #[test]
    fn test_unapproved_input_does_not_count() {
        use super::super::orchestration_state::HumanInputRecord;

        let guardrails = Guardrails::default();
        let plan = json!({
            "description": "Handle user credentials"
        });

        let mut state = OrchestrationState::new(
            "test".to_string(),
            "task".to_string(),
            std::path::PathBuf::new(),
            "slug".to_string(),
        );

        // Add human input but with approved = false
        state.human_inputs.push(HumanInputRecord {
            question: "Approve security approach?".to_string(),
            category: "security".to_string(),
            response: Some("No, need changes".to_string()),
            condition: Some(MandatoryCondition::SecuritySensitive {
                keywords: vec!["credential".to_string()],
                locations: vec![],
            }),
            iteration: 1,
            timestamp: chrono::Utc::now().to_rfc3339(),
            approved: false, // NOT approved
        });
        state.reviews.push(json!({ "score": 0.9 }));

        // Should still block because approval was denied
        let result = guardrails.check_before_finalize(&plan, &state);
        assert!(result.is_err());
    }
}
