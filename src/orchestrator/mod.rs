pub mod client;
pub mod guardrails;
pub mod loop_controller;
pub mod orchestration_state;
pub mod state;

// Legacy exports (deprecated, use orchestrator mode)
#[allow(deprecated)]
pub use loop_controller::{HumanInputRequired, LoopController};
pub use state::{LoopResult, LoopState, ResumeState};

// New orchestrator exports
pub use client::{
    create_orchestrator_client, register_orchestrator_extension, OrchestratorClient,
    SessionRegistry, TokenUsage, EXTENSION_NAME,
};
pub use guardrails::{GuardrailHardStop, Guardrails, GuardrailsConfig, MandatoryCondition};
pub use orchestration_state::{HumanInputRecord, HumanResponse, OrchestrationState, OrchestrationStatus, TokenBreakdown};
