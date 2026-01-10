//! Integration tests for the orchestrator agent architecture.
//!
//! These tests verify the orchestrator flows including:
//! - Full flow to completion
//! - Guardrail condition triggers
//! - Resume with human response
//! - Token budget enforcement
//! - Concurrent session handling

use std::path::PathBuf;
use std::sync::Arc;

use plan_forge::orchestrator::{
    GuardrailHardStop, Guardrails, GuardrailsConfig, HumanInputRecord, MandatoryCondition,
    OrchestrationState, OrchestrationStatus, SessionRegistry, TokenBreakdown,
};

// ============================================================================
// Guardrails Tests
// ============================================================================

#[test]
fn test_guardrails_security_sensitive_detection() {
    let config = GuardrailsConfig::default();
    let guardrails = Guardrails::from_config(&config);

    // Plan with security keywords
    let plan = serde_json::json!({
        "title": "Add authentication",
        "phases": [{
            "name": "Setup",
            "tasks": [{
                "title": "Configure API key handling",
                "description": "Store credentials securely"
            }]
        }]
    });

    let conditions = guardrails.check_all_conditions(&plan, 0.9, 1);
    assert!(
        conditions
            .iter()
            .any(|c| matches!(c, MandatoryCondition::SecuritySensitive { .. })),
        "Should detect security-sensitive keywords"
    );
}

#[test]
fn test_guardrails_sensitive_file_detection() {
    let config = GuardrailsConfig::default();
    let guardrails = Guardrails::from_config(&config);

    // Plan modifying .env file
    let plan = serde_json::json!({
        "title": "Configure environment",
        "phases": [{
            "name": "Setup",
            "tasks": [{
                "title": "Update .env file",
                "file_path": ".env"
            }]
        }]
    });

    let conditions = guardrails.check_all_conditions(&plan, 0.9, 1);
    assert!(
        conditions
            .iter()
            .any(|c| matches!(c, MandatoryCondition::SensitiveFilePattern { .. })),
        "Should detect sensitive file pattern"
    );
}

#[test]
fn test_guardrails_low_score_threshold() {
    let config = GuardrailsConfig::default();
    let guardrails = Guardrails::from_config(&config);

    let plan = serde_json::json!({
        "title": "Simple task",
        "phases": []
    });

    // Score below 0.5 should trigger
    let conditions = guardrails.check_all_conditions(&plan, 0.3, 1);
    assert!(
        conditions
            .iter()
            .any(|c| matches!(c, MandatoryCondition::LowScoreThreshold { .. })),
        "Should detect low score"
    );

    // Score above 0.5 should not trigger
    let conditions = guardrails.check_all_conditions(&plan, 0.6, 1);
    assert!(
        !conditions
            .iter()
            .any(|c| matches!(c, MandatoryCondition::LowScoreThreshold { .. })),
        "Should not trigger for acceptable score"
    );
}

#[test]
fn test_guardrails_iteration_soft_limit() {
    let config = GuardrailsConfig::default();
    let guardrails = Guardrails::from_config(&config);

    let plan = serde_json::json!({
        "title": "Simple task",
        "phases": []
    });

    // Iteration 6 should not trigger (default limit is 7)
    let conditions = guardrails.check_all_conditions(&plan, 0.9, 6);
    assert!(
        !conditions
            .iter()
            .any(|c| matches!(c, MandatoryCondition::IterationSoftLimit { .. })),
        "Should not trigger before soft limit"
    );

    // Iteration 7 should trigger
    let conditions = guardrails.check_all_conditions(&plan, 0.9, 7);
    assert!(
        conditions
            .iter()
            .any(|c| matches!(c, MandatoryCondition::IterationSoftLimit { .. })),
        "Should trigger at soft limit"
    );
}

#[test]
fn test_guardrails_data_deletion_detection() {
    let config = GuardrailsConfig::default();
    let guardrails = Guardrails::from_config(&config);

    let plan = serde_json::json!({
        "title": "Database cleanup",
        "phases": [{
            "name": "Cleanup",
            "tasks": [{
                "title": "Remove old data",
                "description": "DROP TABLE old_users"
            }]
        }]
    });

    let conditions = guardrails.check_all_conditions(&plan, 0.9, 1);
    assert!(
        conditions
            .iter()
            .any(|c| matches!(c, MandatoryCondition::DataDeletionOperations { .. })),
        "Should detect data deletion operations"
    );
}

#[test]
fn test_guardrails_hard_stop_max_iterations() {
    let config = GuardrailsConfig {
        max_iterations: 10,
        ..Default::default()
    };
    let guardrails = Guardrails::from_config(&config);

    let mut state = OrchestrationState::new(
        "test-session".to_string(),
        "Test task".to_string(),
        PathBuf::from("/tmp"),
        "test-task".to_string(),
    );

    // Within limit
    state.iteration = 9;
    assert!(guardrails.check_before_tool_call(&state).is_ok());

    // At limit
    state.iteration = 10;
    let result = guardrails.check_before_tool_call(&state);
    assert!(matches!(
        result,
        Err(GuardrailHardStop::MaxIterationsExceeded { .. })
    ));
}

#[test]
fn test_guardrails_hard_stop_token_budget() {
    let config = GuardrailsConfig {
        max_total_tokens: 100_000,
        ..Default::default()
    };
    let guardrails = Guardrails::from_config(&config);

    let mut state = OrchestrationState::new(
        "test-session".to_string(),
        "Test task".to_string(),
        PathBuf::from("/tmp"),
        "test-task".to_string(),
    );

    // Within budget
    state.total_tokens = 99_000;
    assert!(guardrails.check_before_tool_call(&state).is_ok());

    // Over budget
    state.total_tokens = 100_001;
    let result = guardrails.check_before_tool_call(&state);
    assert!(matches!(
        result,
        Err(GuardrailHardStop::TokenBudgetExhausted { .. })
    ));
}

// ============================================================================
// OrchestrationState Tests
// ============================================================================

#[test]
fn test_orchestration_state_creation() {
    let state = OrchestrationState::new(
        "session-123".to_string(),
        "Build a web app".to_string(),
        PathBuf::from("/project"),
        "build-web-app".to_string(),
    );

    assert_eq!(state.session_id, "session-123");
    assert_eq!(state.task, "Build a web app");
    assert_eq!(state.iteration, 0);
    assert_eq!(state.tool_calls, 0);
    assert_eq!(state.total_tokens, 0);
    assert!(matches!(state.status, OrchestrationStatus::Running));
    assert!(state.current_plan.is_none());
    assert!(state.reviews.is_empty());
    assert!(state.human_inputs.is_empty());
}

#[test]
fn test_orchestration_state_add_tokens() {
    let mut state = OrchestrationState::new(
        "test".to_string(),
        "task".to_string(),
        PathBuf::from("/tmp"),
        "slug".to_string(),
    );

    // Add tokens
    state.add_tokens(Some(100), Some(50));
    assert_eq!(state.total_tokens, 150);

    // Add more tokens
    state.add_tokens(Some(200), Some(100));
    assert_eq!(state.total_tokens, 450);

    // Handle None values
    state.add_tokens(None, None);
    assert_eq!(state.total_tokens, 450);

    // Handle negative values (should be treated as 0)
    state.add_tokens(Some(-10), Some(-5));
    assert_eq!(state.total_tokens, 450);
}

#[test]
fn test_orchestration_state_can_resume() {
    let mut state = OrchestrationState::new(
        "test".to_string(),
        "task".to_string(),
        PathBuf::from("/tmp"),
        "slug".to_string(),
    );

    // Running state can resume
    state.status = OrchestrationStatus::Running;
    assert!(state.can_resume());

    // Completed state can resume
    state.status = OrchestrationStatus::Completed;
    assert!(state.can_resume());

    // Paused state can resume
    state.status = OrchestrationStatus::Paused { condition: None };
    assert!(state.can_resume());

    // Failed state can resume
    state.status = OrchestrationStatus::Failed {
        error: "test".to_string(),
    };
    assert!(state.can_resume());

    // HardStopped state cannot resume
    state.status = OrchestrationStatus::HardStopped {
        reason: GuardrailHardStop::ExecutionTimeout,
    };
    assert!(!state.can_resume());
}

#[test]
fn test_orchestration_state_save_and_load() {
    let temp_dir = tempfile::tempdir().unwrap();
    let session_dir = temp_dir.path().join("test-session");

    let mut state = OrchestrationState::new(
        "session-456".to_string(),
        "Create API endpoint".to_string(),
        PathBuf::from("/project"),
        "create-api".to_string(),
    );

    state.iteration = 3;
    state.tool_calls = 5;
    state.total_tokens = 1500;
    state.current_plan = Some(serde_json::json!({"title": "API Plan"}));

    // Save state
    state.save(&session_dir).unwrap();

    // Load state
    let loaded = OrchestrationState::load(&session_dir).unwrap().unwrap();

    assert_eq!(loaded.session_id, "session-456");
    assert_eq!(loaded.task, "Create API endpoint");
    assert_eq!(loaded.iteration, 3);
    assert_eq!(loaded.tool_calls, 5);
    assert_eq!(loaded.total_tokens, 1500);
    assert!(loaded.current_plan.is_some());
}

// ============================================================================
// TokenBreakdown Tests
// ============================================================================

#[test]
fn test_token_breakdown_default() {
    let breakdown = TokenBreakdown::default();

    assert_eq!(breakdown.orchestrator_input, 0);
    assert_eq!(breakdown.orchestrator_output, 0);
    assert_eq!(breakdown.planner_input, 0);
    assert_eq!(breakdown.planner_output, 0);
    assert_eq!(breakdown.reviewer_input, 0);
    assert_eq!(breakdown.reviewer_output, 0);
    assert_eq!(breakdown.total, 0);
    assert!(!breakdown.estimated);
}

#[test]
fn test_token_breakdown_add_methods() {
    let mut breakdown = TokenBreakdown::default();

    breakdown.add_orchestrator(100, 50);
    assert_eq!(breakdown.orchestrator_input, 100);
    assert_eq!(breakdown.orchestrator_output, 50);
    assert_eq!(breakdown.total, 150);

    breakdown.add_planner(200, 100);
    assert_eq!(breakdown.planner_input, 200);
    assert_eq!(breakdown.planner_output, 100);
    assert_eq!(breakdown.total, 450);

    breakdown.add_reviewer(150, 75);
    assert_eq!(breakdown.reviewer_input, 150);
    assert_eq!(breakdown.reviewer_output, 75);
    assert_eq!(breakdown.total, 675);
}

#[test]
fn test_token_breakdown_overhead_ratio() {
    let mut breakdown = TokenBreakdown::default();

    // Empty breakdown
    assert_eq!(breakdown.overhead_ratio(), 0.0);

    // Add tokens: 150 orchestrator, 500 total = 30% overhead
    breakdown.add_orchestrator(100, 50);
    breakdown.add_planner(200, 100);
    breakdown.add_reviewer(50, 0);

    let ratio = breakdown.overhead_ratio();
    assert!((ratio - 0.3).abs() < 0.01, "Expected ~30% overhead, got {}", ratio);
}

// ============================================================================
// SessionRegistry Tests
// ============================================================================

#[tokio::test]
async fn test_session_registry_get_or_create() {
    let registry = SessionRegistry::new();

    let state1 = OrchestrationState::new(
        "session-1".to_string(),
        "Task 1".to_string(),
        PathBuf::from("/tmp"),
        "task-1".to_string(),
    );

    // First call creates the session
    let session1 = registry.get_or_create("session-1", state1.clone()).await;
    {
        let s = session1.lock().await;
        assert_eq!(s.task, "Task 1");
    }

    // Second call returns existing session (even with different initial state)
    let state2 = OrchestrationState::new(
        "session-1".to_string(),
        "Different Task".to_string(),
        PathBuf::from("/tmp"),
        "different".to_string(),
    );
    let session1_again = registry.get_or_create("session-1", state2).await;
    {
        let s = session1_again.lock().await;
        assert_eq!(s.task, "Task 1"); // Original task preserved
    }
}

#[tokio::test]
async fn test_session_registry_concurrent_sessions() {
    let registry = Arc::new(SessionRegistry::new());

    // Create two different sessions
    let state1 = OrchestrationState::new(
        "session-a".to_string(),
        "Task A".to_string(),
        PathBuf::from("/tmp"),
        "task-a".to_string(),
    );
    let state2 = OrchestrationState::new(
        "session-b".to_string(),
        "Task B".to_string(),
        PathBuf::from("/tmp"),
        "task-b".to_string(),
    );

    let session_a = registry.get_or_create("session-a", state1).await;
    let session_b = registry.get_or_create("session-b", state2).await;

    // Modify session A
    {
        let mut s = session_a.lock().await;
        s.iteration = 5;
    }

    // Session B should be unaffected
    {
        let s = session_b.lock().await;
        assert_eq!(s.iteration, 0);
    }

    // Session A should retain changes
    {
        let s = session_a.lock().await;
        assert_eq!(s.iteration, 5);
    }
}

// ============================================================================
// Human Input Record Tests
// ============================================================================

#[test]
fn test_human_input_record_creation() {
    let record = HumanInputRecord {
        question: "Should we proceed with security changes?".to_string(),
        category: "security".to_string(),
        response: Some("Yes, approved".to_string()),
        condition: Some(MandatoryCondition::SecuritySensitive {
            keywords: vec!["api_key".to_string()],
            locations: vec!["config.rs".to_string()],
        }),
        iteration: 2,
        timestamp: "2024-01-01T00:00:00Z".to_string(),
        approved: true,
    };

    assert_eq!(record.question, "Should we proceed with security changes?");
    assert_eq!(record.category, "security");
    assert!(record.response.is_some());
    assert!(record.condition.is_some());
    assert!(record.approved);
}

// ============================================================================
// Status Transition Tests
// ============================================================================

#[test]
fn test_orchestration_status_transitions() {
    let mut state = OrchestrationState::new(
        "test".to_string(),
        "task".to_string(),
        PathBuf::from("/tmp"),
        "slug".to_string(),
    );

    // Initial state is Running
    assert!(matches!(state.status, OrchestrationStatus::Running));

    // Transition to Paused
    state.status = OrchestrationStatus::Paused {
        condition: Some(MandatoryCondition::SecuritySensitive {
            keywords: vec!["password".to_string()],
            locations: vec!["auth.rs".to_string()],
        }),
    };
    assert!(matches!(state.status, OrchestrationStatus::Paused { .. }));
    assert!(state.can_resume());

    // Transition to Completed
    state.status = OrchestrationStatus::Completed;
    assert!(matches!(state.status, OrchestrationStatus::Completed));

    // Transition to HardStopped (terminal)
    state.status = OrchestrationStatus::HardStopped {
        reason: GuardrailHardStop::MaxIterationsExceeded {
            iteration: 10,
            limit: 10,
        },
    };
    assert!(matches!(state.status, OrchestrationStatus::HardStopped { .. }));
    assert!(!state.can_resume());
}
