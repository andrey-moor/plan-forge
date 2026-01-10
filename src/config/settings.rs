use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Main CLI configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliConfig {
    pub planning: PlanningConfig,
    pub review: ReviewConfig,
    pub loop_config: LoopConfig,
    pub output: OutputConfig,
    /// Guardrails configuration for orchestrator mode
    #[serde(default)]
    pub guardrails: GuardrailsConfig,
    /// Orchestrator mode configuration
    #[serde(default)]
    pub orchestrator: OrchestratorConfig,
    /// Use LLM-powered orchestrator instead of deterministic loop controller
    #[serde(default)]
    pub use_orchestrator: bool,
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

/// Configuration for orchestrator guardrails (mandatory human input conditions)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailsConfig {
    /// Maximum iterations before hard stop
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// Maximum total tokens before hard stop (default 500,000)
    #[serde(default = "default_max_total_tokens")]
    pub max_total_tokens: u64,
    /// Maximum tool calls before hard stop
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls: u32,
    /// Execution timeout in seconds
    #[serde(default = "default_execution_timeout_secs")]
    pub execution_timeout_secs: u64,
    /// Soft limit for iterations (triggers human input request)
    #[serde(default = "default_iteration_soft_limit")]
    pub iteration_soft_limit: u32,
    /// Score threshold below which human input is required
    #[serde(default = "default_low_score_threshold")]
    pub low_score_threshold: f32,
    /// Keywords that trigger security-sensitive condition
    #[serde(default = "default_security_keywords")]
    pub security_keywords: Vec<String>,
    /// File patterns that trigger sensitive file condition
    #[serde(default = "default_sensitive_file_patterns")]
    pub sensitive_file_patterns: Vec<String>,
    /// Patterns indicating breaking API changes
    #[serde(default = "default_breaking_api_patterns")]
    pub breaking_api_patterns: Vec<String>,
    /// Patterns indicating data deletion operations
    #[serde(default = "default_data_deletion_patterns")]
    pub data_deletion_patterns: Vec<String>,
}

fn default_max_iterations() -> u32 {
    10
}

fn default_max_total_tokens() -> u64 {
    500_000
}

fn default_max_tool_calls() -> u32 {
    100
}

fn default_execution_timeout_secs() -> u64 {
    600 // 10 minutes
}

fn default_iteration_soft_limit() -> u32 {
    7
}

fn default_low_score_threshold() -> f32 {
    0.5
}

fn default_security_keywords() -> Vec<String> {
    vec![
        "credential".to_string(),
        "auth".to_string(),
        "encrypt".to_string(),
        "secret".to_string(),
        "token".to_string(),
        "password".to_string(),
        "api_key".to_string(),
        "private_key".to_string(),
        "certificate".to_string(),
    ]
}

fn default_sensitive_file_patterns() -> Vec<String> {
    vec![
        "*.env".to_string(),
        "*.env.*".to_string(),
        "*secret*".to_string(),
        "*credential*".to_string(),
        "*.pem".to_string(),
        "*.key".to_string(),
        "**/secrets/**".to_string(),
    ]
}

fn default_breaking_api_patterns() -> Vec<String> {
    vec![
        "pub fn".to_string(),
        "pub struct".to_string(),
        "pub enum".to_string(),
        "pub trait".to_string(),
    ]
}

fn default_data_deletion_patterns() -> Vec<String> {
    vec![
        "DROP TABLE".to_string(),
        "DELETE FROM".to_string(),
        "TRUNCATE".to_string(),
        "rm -rf".to_string(),
        "shutil.rmtree".to_string(),
    ]
}

impl Default for GuardrailsConfig {
    fn default() -> Self {
        Self {
            max_iterations: default_max_iterations(),
            max_total_tokens: default_max_total_tokens(),
            max_tool_calls: default_max_tool_calls(),
            execution_timeout_secs: default_execution_timeout_secs(),
            iteration_soft_limit: default_iteration_soft_limit(),
            low_score_threshold: default_low_score_threshold(),
            security_keywords: default_security_keywords(),
            sensitive_file_patterns: default_sensitive_file_patterns(),
            breaking_api_patterns: default_breaking_api_patterns(),
            data_deletion_patterns: default_data_deletion_patterns(),
        }
    }
}

/// Configuration for orchestrator mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
    /// Path to the orchestrator recipe YAML file
    #[serde(default = "default_orchestrator_recipe")]
    pub recipe: PathBuf,
    /// Override provider for orchestrator
    pub provider_override: Option<String>,
    /// Override model for orchestrator
    pub model_override: Option<String>,
}

fn default_orchestrator_recipe() -> PathBuf {
    PathBuf::from("recipes/orchestrator.yaml")
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            recipe: default_orchestrator_recipe(),
            provider_override: None,
            model_override: None,
        }
    }
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
            guardrails: GuardrailsConfig::default(),
            orchestrator: OrchestratorConfig::default(),
            use_orchestrator: false,
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
    /// - PLAN_FORGE_USE_ORCHESTRATOR: Enable orchestrator mode (true/false)
    /// - PLAN_FORGE_ORCHESTRATOR_PROVIDER: Provider for orchestrator
    /// - PLAN_FORGE_ORCHESTRATOR_MODEL: Model for orchestrator
    /// - PLAN_FORGE_MAX_TOTAL_TOKENS: Maximum total tokens for orchestrator session
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
            if self.orchestrator.recipe.is_relative() {
                self.orchestrator.recipe = recipe_dir.join(&self.orchestrator.recipe);
            }
        }

        // Orchestrator mode
        if let Ok(val) = std::env::var("PLAN_FORGE_USE_ORCHESTRATOR")
            && !val.is_empty()
        {
            self.use_orchestrator = val.parse().unwrap_or(false) || val.eq_ignore_ascii_case("true");
        }

        // Orchestrator provider
        if let Ok(val) = std::env::var("PLAN_FORGE_ORCHESTRATOR_PROVIDER")
            && !val.is_empty()
        {
            self.orchestrator.provider_override = Some(val);
        }

        // Orchestrator model
        if let Ok(val) = std::env::var("PLAN_FORGE_ORCHESTRATOR_MODEL")
            && !val.is_empty()
        {
            self.orchestrator.model_override = Some(val);
        }

        // Max total tokens for orchestrator
        if let Ok(val) = std::env::var("PLAN_FORGE_MAX_TOTAL_TOKENS")
            && let Ok(tokens) = val.parse::<u64>()
        {
            self.guardrails.max_total_tokens = tokens;
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
