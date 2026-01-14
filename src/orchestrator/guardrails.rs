//! Guardrails - Hard limits enforced in Rust.
//!
//! This module implements guardrails that cannot be bypassed by the LLM orchestrator.
//!
//! ## Design Philosophy
//!
//! Guardrails enforce **hard stops** - non-bypassable limits:
//! - Token budget exhaustion
//! - Maximum iterations reached
//! - Maximum tool calls exceeded
//! - Execution timeout
//!
//! Human input requirements are handled separately by the LLM reviewer through
//! the `requires_human_input` field, which has full context awareness for:
//! - Security concerns
//! - Ambiguous requirements
//! - Clarification needs
//!
//! This approach avoids the previous complexity of "soft limits" that would pause
//! for human approval based on iteration counts or score thresholds.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::orchestration_state::OrchestrationState;

// Re-export from config for convenience
pub use crate::config::GuardrailsConfig;

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

/// Guardrails struct with hard limit configuration and check methods.
///
/// This struct contains only numeric/deterministic limits. Human input
/// requirements are delegated to the LLM reviewer's `requires_human_input` field.
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
    /// Review score threshold for determining pass/fail (default 0.8)
    pub score_threshold: f32,
}

impl Default for Guardrails {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            max_tool_calls: 100,
            max_total_tokens: 500_000,
            execution_timeout: Duration::from_secs(600), // 10 minutes
            score_threshold: 0.8,
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
            score_threshold: config.score_threshold,
        }
    }

    // ========================================================================
    // Hard Stop Checks
    // ========================================================================

    /// Check before any tool call - returns error if hard limit exceeded.
    pub fn check_before_tool_call(
        &self,
        state: &OrchestrationState,
    ) -> Result<(), GuardrailHardStop> {
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
    // Score Threshold Check
    // ========================================================================

    /// Check if a review score meets the passing threshold.
    pub fn score_passes(&self, score: f32) -> bool {
        score >= self.score_threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_passes() {
        let guardrails = Guardrails::default(); // threshold = 0.8

        assert!(!guardrails.score_passes(0.3));
        assert!(!guardrails.score_passes(0.79));
        assert!(guardrails.score_passes(0.8));
        assert!(guardrails.score_passes(0.9));
    }

    #[test]
    fn test_hard_stop_token_budget() {
        let guardrails = Guardrails {
            max_total_tokens: 1000,
            ..Default::default()
        };

        let state = OrchestrationState {
            total_tokens: 1500,
            ..OrchestrationState::new(
                "test".to_string(),
                "task".to_string(),
                std::path::PathBuf::new(),
                "slug".to_string(),
            )
        };

        let result = guardrails.check_before_tool_call(&state);
        assert!(matches!(
            result,
            Err(GuardrailHardStop::TokenBudgetExhausted { .. })
        ));
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
        assert!(matches!(
            result,
            Err(GuardrailHardStop::MaxIterationsExceeded {
                iteration: 6,
                limit: 5
            })
        ));
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
        assert!(matches!(
            result,
            Err(GuardrailHardStop::MaxToolCallsExceeded {
                calls: 51,
                limit: 50
            })
        ));
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
    fn test_cannot_bypass_hard_stop_even_at_boundary() {
        let guardrails = Guardrails {
            max_iterations: 5,
            max_tool_calls: 10,
            max_total_tokens: 1000,
            ..Default::default()
        };

        // Test at exact boundary (should fail)
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
}
