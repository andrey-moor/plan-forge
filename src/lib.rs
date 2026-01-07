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
pub use orchestrator::{LoopController, LoopResult, LoopState, ResumeState};
pub use output::{FileOutputWriter, OutputWriter};
pub use phases::{GoosePlanner, GooseReviewer, Planner, Reviewer};

// Re-export MCP server
pub use mcp::{PlanForgeServer, SessionStatus};

// Re-export slug utilities
pub use slug::{generate_slug, slugify, slugify_truncate};
