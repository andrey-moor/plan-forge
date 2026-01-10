pub mod config;
pub mod mcp;
pub mod models;
pub mod orchestrator;
pub mod output;
pub mod phases;
pub mod recipes;
pub mod slug;

// Re-export main types
pub use config::{CliConfig, HardChecklist, OutputConfig};
pub use models::{Plan, ReviewResult};
#[allow(deprecated)]
pub use orchestrator::{
    HumanResponse, LoopController, LoopResult, LoopState, OrchestrationState, OrchestrationStatus,
    ResumeState, SessionRegistry,
};
pub use output::{FileOutputWriter, OutputWriter};
pub use phases::{GooseOrchestrator, GoosePlanner, GooseReviewer, Planner, Reviewer};

// Re-export MCP server
pub use mcp::{PlanForgeServer, SessionStatus};

// Re-export slug utilities
pub use slug::{generate_slug, slugify, slugify_truncate};
