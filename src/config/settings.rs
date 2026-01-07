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
    /// Session directory for intermediate JSON files
    /// Defaults to .plan-forge/ (slug appended by CLI)
    pub runs_dir: PathBuf,
    /// Final output directory for committed plan files
    /// Defaults to ./dev/active/
    pub active_dir: PathBuf,
    /// Session slug (used for output directory name)
    /// If None, derived from plan title
    #[serde(skip)]
    pub slug: Option<String>,
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
                runs_dir: PathBuf::from("./.plan-forge"),
                active_dir: PathBuf::from("./dev/active"),
                slug: None,
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

    /// Apply environment variable overrides.
    ///
    /// Environment variables (PLAN_FORGE_*) override config file values
    /// but are themselves overridden by CLI arguments.
    ///
    /// Supported environment variables:
    /// - PLAN_FORGE_THRESHOLD: Review pass threshold (0.0-1.0)
    /// - PLAN_FORGE_MAX_ITERATIONS: Maximum planning iterations
    /// - PLAN_FORGE_PLANNER_PROVIDER: Provider for planner (e.g., "anthropic")
    /// - PLAN_FORGE_PLANNER_MODEL: Model for planner
    /// - PLAN_FORGE_REVIEWER_PROVIDER: Provider for reviewer
    /// - PLAN_FORGE_REVIEWER_MODEL: Model for reviewer
    /// - PLAN_FORGE_RECIPE_DIR: Directory to search for recipes
    pub fn apply_env_overrides(mut self) -> Self {
        // Threshold
        if let Ok(val) = std::env::var("PLAN_FORGE_THRESHOLD")
            && let Ok(threshold) = val.parse::<f32>()
        {
            self.review.pass_threshold = threshold.clamp(0.0, 1.0);
        }

        // Max iterations
        if let Ok(val) = std::env::var("PLAN_FORGE_MAX_ITERATIONS")
            && let Ok(max) = val.parse::<u32>()
        {
            self.loop_config.max_iterations = max;
        }

        // Planner provider
        if let Ok(val) = std::env::var("PLAN_FORGE_PLANNER_PROVIDER")
            && !val.is_empty()
        {
            self.planning.provider_override = Some(val);
        }

        // Planner model
        if let Ok(val) = std::env::var("PLAN_FORGE_PLANNER_MODEL")
            && !val.is_empty()
        {
            self.planning.model_override = Some(val);
        }

        // Reviewer provider
        if let Ok(val) = std::env::var("PLAN_FORGE_REVIEWER_PROVIDER")
            && !val.is_empty()
        {
            self.review.provider_override = Some(val);
        }

        // Reviewer model
        if let Ok(val) = std::env::var("PLAN_FORGE_REVIEWER_MODEL")
            && !val.is_empty()
        {
            self.review.model_override = Some(val);
        }

        // Recipe directory (prepended to recipe paths if they're relative)
        if let Ok(val) = std::env::var("PLAN_FORGE_RECIPE_DIR")
            && !val.is_empty()
        {
            let recipe_dir = PathBuf::from(&val);
            // Only modify if the current recipe paths are relative
            if self.planning.recipe.is_relative() {
                self.planning.recipe = recipe_dir.join(&self.planning.recipe);
            }
            if self.review.recipe.is_relative() {
                self.review.recipe = recipe_dir.join(&self.review.recipe);
            }
        }

        self
    }

    /// Load configuration with environment variable overrides applied.
    ///
    /// Priority: Config file > Env vars > Defaults
    /// (CLI args override everything, applied separately in main.rs)
    pub fn load_with_env(path: Option<&PathBuf>) -> anyhow::Result<Self> {
        Self::load_or_default(path).map(|c| c.apply_env_overrides())
    }
}
