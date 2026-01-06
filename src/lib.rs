pub mod config;
pub mod models;
pub mod orchestrator;
pub mod output;
pub mod phases;

// Re-export main types
pub use config::{CliConfig, HardChecklist};
pub use models::{Plan, ReviewResult};
pub use orchestrator::{LoopController, LoopResult, LoopState, ResumeState};
pub use output::{FileOutputWriter, OutputWriter};
pub use phases::{GoosePlanner, GooseReviewer, Planner, Reviewer};

/// Convert a string to a URL-friendly slug
/// Used for task directories in runs/ and dev/active/
pub fn slugify(title: &str) -> String {
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();

    // Collapse consecutive dashes and trim leading/trailing dashes
    let mut result = String::new();
    let mut prev_dash = true; // Start true to skip leading dashes
    for c in slug.chars() {
        if c == '-' {
            if !prev_dash {
                result.push(c);
            }
            prev_dash = true;
        } else {
            result.push(c);
            prev_dash = false;
        }
    }
    // Remove trailing dash if present
    if result.ends_with('-') {
        result.pop();
    }
    result
}
