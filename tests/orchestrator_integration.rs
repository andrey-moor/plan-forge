//! Integration tests for the orchestrator agent architecture.
//!
//! These tests verify the orchestrator flows including:
//! - Full flow to completion
//! - Hard stop enforcement (iterations, tokens, tool calls)
//! - Resume with human response
//! - Token budget enforcement
//! - Concurrent session handling
//! - ISA types (OpCode, Instruction, GroundingSnapshot)

use std::path::PathBuf;
use std::sync::Arc;

use plan_forge::models::{
    ExistingPattern, GroundingSnapshot, Instruction, OpCode, VerifiedFile, VerifiedTarget,
};
use plan_forge::orchestrator::{
    GuardrailHardStop, Guardrails, GuardrailsConfig, HumanInputRecord,
    OrchestrationState, OrchestrationStatus, SessionRegistry, TokenBreakdown,
};

// ============================================================================
// Guardrails Tests
// ============================================================================

#[test]
fn test_guardrails_score_passes() {
    let config = GuardrailsConfig::default();
    let guardrails = Guardrails::from_config(&config);

    // Default threshold is 0.8
    assert!(!guardrails.score_passes(0.3));
    assert!(!guardrails.score_passes(0.79));
    assert!(guardrails.score_passes(0.8));
    assert!(guardrails.score_passes(0.9));
}

#[test]
fn test_guardrails_custom_score_threshold() {
    let config = GuardrailsConfig {
        score_threshold: 0.5,
        ..Default::default()
    };
    let guardrails = Guardrails::from_config(&config);

    assert!(!guardrails.score_passes(0.3));
    assert!(!guardrails.score_passes(0.49));
    assert!(guardrails.score_passes(0.5));
    assert!(guardrails.score_passes(0.9));
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

#[test]
fn test_guardrails_hard_stop_max_tool_calls() {
    let config = GuardrailsConfig {
        max_tool_calls: 50,
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
    state.tool_calls = 49;
    assert!(guardrails.check_before_tool_call(&state).is_ok());

    // At limit
    state.tool_calls = 50;
    let result = guardrails.check_before_tool_call(&state);
    assert!(matches!(
        result,
        Err(GuardrailHardStop::MaxToolCallsExceeded { .. })
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
    state.status = OrchestrationStatus::Paused { reason: "test".to_string() };
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
        question: "Security concern: Should we proceed?".to_string(),
        category: "security".to_string(),
        response: Some("Yes, approved".to_string()),
        reason: Some("Potential security issue in plan".to_string()),
        iteration: 2,
        timestamp: "2024-01-01T00:00:00Z".to_string(),
        approved: true,
    };

    assert_eq!(record.question, "Security concern: Should we proceed?");
    assert_eq!(record.category, "security");
    assert!(record.response.is_some());
    assert!(record.reason.is_some());
    assert!(record.approved);
}

#[test]
fn test_human_input_record_with_reason() {
    let record = HumanInputRecord {
        question: "Requirements unclear. Continue?".to_string(),
        category: "clarification".to_string(),
        response: Some("Yes, continue with assumption X".to_string()),
        reason: Some("clarification: Requirements unclear".to_string()),
        iteration: 3,
        timestamp: "2024-01-01T00:00:00Z".to_string(),
        approved: true,
    };

    assert_eq!(record.category, "clarification");
    assert_eq!(record.reason.as_deref(), Some("clarification: Requirements unclear"));
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

    // Transition to Paused (with reason from reviewer)
    state.status = OrchestrationStatus::Paused {
        reason: "security: Potential credential exposure".to_string(),
    };
    assert!(matches!(state.status, OrchestrationStatus::Paused { .. }));
    assert!(state.can_resume());

    // Transition to Completed
    state.status = OrchestrationStatus::Completed;
    assert!(matches!(state.status, OrchestrationStatus::Completed));

    // Transition to CompletedBestEffort
    state.status = OrchestrationStatus::CompletedBestEffort;
    assert!(matches!(state.status, OrchestrationStatus::CompletedBestEffort));

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

#[test]
fn test_orchestration_status_paused_with_reason() {
    let mut state = OrchestrationState::new(
        "test".to_string(),
        "task".to_string(),
        PathBuf::from("/tmp"),
        "slug".to_string(),
    );

    // Pause due to reviewer flagging human input needed
    state.status = OrchestrationStatus::Paused {
        reason: "security: Plan includes credential handling".to_string(),
    };

    assert!(matches!(state.status, OrchestrationStatus::Paused { .. }));
    assert!(state.can_resume());

    if let OrchestrationStatus::Paused { reason } = &state.status {
        assert!(reason.contains("security"));
    } else {
        panic!("Expected Paused status with reason");
    }
}

// ============================================================================
// ISA (Instruction Set Architecture) Tests
// ============================================================================

#[test]
fn test_opcode_serialization() {
    // Test that OpCodes serialize to SCREAMING_SNAKE_CASE
    let ops = vec![
        (OpCode::SearchSemantic, "\"SEARCH_SEMANTIC\""),
        (OpCode::SearchCode, "\"SEARCH_CODE\""),
        (OpCode::ReadFiles, "\"READ_FILES\""),
        (OpCode::GetDependencies, "\"GET_DEPENDENCIES\""),
        (OpCode::DefineTask, "\"DEFINE_TASK\""),
        (OpCode::VerifyTask, "\"VERIFY_TASK\""),
        (OpCode::EditCode, "\"EDIT_CODE\""),
        (OpCode::RunCommand, "\"RUN_COMMAND\""),
        (OpCode::GenerateTest, "\"GENERATE_TEST\""),
        (OpCode::RunTest, "\"RUN_TEST\""),
        (OpCode::VerifyExists, "\"VERIFY_EXISTS\""),
    ];

    for (op, expected) in ops {
        let serialized = serde_json::to_string(&op).unwrap();
        assert_eq!(serialized, expected, "OpCode {:?} should serialize to {}", op, expected);

        // Verify roundtrip
        let deserialized: OpCode = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, op, "OpCode roundtrip failed for {:?}", op);
    }
}

#[test]
fn test_instruction_model() {
    let instruction = Instruction {
        id: "step_1".to_string(),
        op: OpCode::SearchCode,
        params: serde_json::json!({
            "query": "fn main",
            "path": "src/"
        }),
        dependencies: vec![],
        description: "Search for main function".to_string(),
        ..Default::default()
    };

    // Test serialization
    let json = serde_json::to_string(&instruction).unwrap();
    assert!(json.contains("\"id\":\"step_1\""));
    assert!(json.contains("\"op\":\"SEARCH_CODE\""));
    assert!(json.contains("\"query\":\"fn main\""));

    // Test deserialization
    let parsed: Instruction = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.id, "step_1");
    assert_eq!(parsed.op, OpCode::SearchCode);
    assert_eq!(parsed.description, "Search for main function");
    assert!(parsed.dependencies.is_empty());
}

#[test]
fn test_instruction_with_dependencies() {
    let instr1 = Instruction {
        id: "locate_files".to_string(),
        op: OpCode::SearchCode,
        params: serde_json::json!({"query": "struct Plan"}),
        dependencies: vec![],
        description: "Find Plan struct".to_string(),
        ..Default::default()
    };

    let instr2 = Instruction {
        id: "read_context".to_string(),
        op: OpCode::ReadFiles,
        params: serde_json::json!({"paths": "${locate_files.output}"}),
        dependencies: vec!["locate_files".to_string()],
        description: "Read found files".to_string(),
        ..Default::default()
    };

    // Verify dependency chain
    assert!(instr1.dependencies.is_empty());
    assert_eq!(instr2.dependencies, vec!["locate_files"]);

    // Verify variable reference in params
    let params_str = serde_json::to_string(&instr2.params).unwrap();
    assert!(params_str.contains("${locate_files.output}"));
}

#[test]
fn test_grounding_snapshot_default() {
    let snapshot = GroundingSnapshot::default();

    assert!(snapshot.verified_files.is_empty());
    assert!(snapshot.verified_targets.is_empty());
    assert!(snapshot.import_convention.is_none());
    assert!(snapshot.existing_patterns.is_empty());
}

#[test]
fn test_grounding_snapshot_with_data() {
    let snapshot = GroundingSnapshot {
        verified_files: vec![
            VerifiedFile {
                path: "src/lib.rs".to_string(),
                exists: true,
            },
            VerifiedFile {
                path: "src/missing.rs".to_string(),
                exists: false,
            },
        ],
        verified_targets: vec![VerifiedTarget {
            target: "cargo test".to_string(),
            resolves: true,
        }],
        import_convention: Some("use crate::module::Type".to_string()),
        existing_patterns: vec![ExistingPattern {
            pattern: "impl Default for".to_string(),
            file: "src/models/plan.rs".to_string(),
            line: 61,
        }],
    };

    // Test serialization roundtrip
    let json = serde_json::to_string(&snapshot).unwrap();
    let parsed: GroundingSnapshot = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.verified_files.len(), 2);
    assert!(parsed.verified_files[0].exists);
    assert!(!parsed.verified_files[1].exists);
    assert_eq!(parsed.verified_targets.len(), 1);
    assert!(parsed.verified_targets[0].resolves);
    assert_eq!(
        parsed.import_convention,
        Some("use crate::module::Type".to_string())
    );
    assert_eq!(parsed.existing_patterns.len(), 1);
    assert_eq!(parsed.existing_patterns[0].line, 61);
}

#[test]
fn test_verified_file_serialization() {
    let file = VerifiedFile {
        path: "Cargo.toml".to_string(),
        exists: true,
    };

    let json = serde_json::to_string(&file).unwrap();
    assert!(json.contains("\"path\":\"Cargo.toml\""));
    assert!(json.contains("\"exists\":true"));

    let parsed: VerifiedFile = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.path, "Cargo.toml");
    assert!(parsed.exists);
}

#[test]
fn test_verified_target_serialization() {
    let target = VerifiedTarget {
        target: "cargo build --release".to_string(),
        resolves: true,
    };

    let json = serde_json::to_string(&target).unwrap();
    let parsed: VerifiedTarget = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.target, "cargo build --release");
    assert!(parsed.resolves);
}

#[test]
fn test_existing_pattern_serialization() {
    let pattern = ExistingPattern {
        pattern: "pub fn new(".to_string(),
        file: "src/config/settings.rs".to_string(),
        line: 42,
    };

    let json = serde_json::to_string(&pattern).unwrap();
    let parsed: ExistingPattern = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.pattern, "pub fn new(");
    assert_eq!(parsed.file, "src/config/settings.rs");
    assert_eq!(parsed.line, 42);
}
