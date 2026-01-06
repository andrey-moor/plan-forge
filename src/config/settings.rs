use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Main CLI configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliConfig {
    pub planning: PlanningConfig,
    pub review: ReviewConfig,
    pub loop_config: LoopConfig,
    pub output: OutputConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningConfig {
    /// Path to the planner recipe YAML file
    pub recipe: PathBuf,
    /// Override provider from recipe (e.g., "anthropic", "openai")
    pub provider_override: Option<String>,
    /// Override model from recipe (e.g., "claude-opus-4-5-20251101")
    pub model_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewConfig {
    /// Path to the reviewer recipe YAML file
    pub recipe: PathBuf,
    /// Override provider from recipe
    pub provider_override: Option<String>,
    /// Override model from recipe
    pub model_override: Option<String>,
    /// Minimum score (0.0-1.0) to pass review
    pub pass_threshold: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopConfig {
    /// Maximum iterations before giving up
    pub max_iterations: u32,
    /// Exit early if review score is perfect (1.0)
    pub early_exit_on_perfect_score: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    /// Service directory for intermediate JSON files
    /// Defaults to $GOOSE_PATH_ROOT/runs/ or ~/.config/plan-forge/runs/
    pub runs_dir: PathBuf,
    /// Final output directory for committed plan files
    /// Defaults to working_dir/dev/active/
    pub active_dir: PathBuf,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            planning: PlanningConfig {
                recipe: PathBuf::from("recipes/planner.yaml"),
                provider_override: None,
                model_override: None,
            },
            review: ReviewConfig {
                recipe: PathBuf::from("recipes/reviewer.yaml"),
                provider_override: None,
                model_override: None,
                pass_threshold: 0.8,
            },
            loop_config: LoopConfig {
                max_iterations: 5,
                early_exit_on_perfect_score: true,
            },
            output: OutputConfig {
                runs_dir: PathBuf::from("./runs"),
                active_dir: PathBuf::from("./dev/active"),
            },
        }
    }
}

impl CliConfig {
    /// Load configuration from a YAML file
    pub fn from_file(path: &PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: CliConfig = serde_yaml::from_str(&content)?;
        Ok(config)
    }

    /// Load configuration, falling back to defaults if file doesn't exist
    pub fn load_or_default(path: Option<&PathBuf>) -> anyhow::Result<Self> {
        match path {
            Some(p) if p.exists() => Self::from_file(p),
            _ => Ok(Self::default()),
        }
    }
}
