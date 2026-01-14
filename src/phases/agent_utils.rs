//! Shared utilities for agent setup across planner, reviewer, and orchestrator.
//!
//! This module extracts common patterns for provider creation and session setup
//! to reduce code duplication.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::info;

use goose::agents::Agent;
use goose::providers::{base::Provider, create_with_named_model};
use goose::recipe::Recipe;
use goose::session::{Session, SessionManager, session_manager::SessionType};

/// Configuration for creating an LLM provider.
///
/// Abstracts the common provider/model override pattern used by planner, reviewer,
/// and orchestrator.
pub struct ProviderConfig<'a> {
    /// Override provider from config (e.g., "anthropic", "openai", "litellm")
    pub provider_override: Option<&'a str>,
    /// Override model from config
    pub model_override: Option<&'a str>,
    /// Default provider if no override or recipe setting
    pub default_provider: &'static str,
    /// Default model if no override or recipe setting
    pub default_model: &'static str,
    /// Component name for logging (e.g., "planner", "reviewer", "orchestrator")
    pub component_name: &'static str,
}

impl<'a> ProviderConfig<'a> {
    /// Create config for the planner component.
    pub fn for_planner(
        provider_override: Option<&'a str>,
        model_override: Option<&'a str>,
    ) -> Self {
        Self {
            provider_override,
            model_override,
            default_provider: "anthropic",
            default_model: "claude-opus-4-5-20251101",
            component_name: "planner",
        }
    }

    /// Create config for the reviewer component.
    pub fn for_reviewer(
        provider_override: Option<&'a str>,
        model_override: Option<&'a str>,
    ) -> Self {
        Self {
            provider_override,
            model_override,
            default_provider: "anthropic",
            default_model: "claude-opus-4-5-20251101",
            component_name: "reviewer",
        }
    }

    /// Create config for the orchestrator component.
    pub fn for_orchestrator(
        provider_override: Option<&'a str>,
        model_override: Option<&'a str>,
    ) -> Self {
        Self {
            provider_override,
            model_override,
            default_provider: "anthropic",
            default_model: "claude-sonnet-4-20250514",
            component_name: "orchestrator",
        }
    }
}

/// Create an LLM provider from config and recipe settings.
///
/// Priority: config override > recipe setting > default
pub async fn create_provider(
    config: &ProviderConfig<'_>,
    recipe: &Recipe,
) -> Result<Arc<dyn Provider>> {
    let provider_name = config
        .provider_override
        .or(recipe
            .settings
            .as_ref()
            .and_then(|s| s.goose_provider.as_deref()))
        .unwrap_or(config.default_provider);

    let model_name = config
        .model_override
        .or(recipe
            .settings
            .as_ref()
            .and_then(|s| s.goose_model.as_deref()))
        .unwrap_or(config.default_model);

    info!(
        "Creating {} provider: {} with model: {}",
        config.component_name, provider_name, model_name
    );

    create_with_named_model(provider_name, model_name)
        .await
        .with_context(|| format!("Failed to create {} provider", config.component_name))
}

/// Set up an agent session with provider and extensions from recipe.
///
/// Returns the session for further use.
pub async fn setup_agent_session(
    agent: &Agent,
    recipe: &Recipe,
    provider: Arc<dyn Provider>,
    working_dir: &Path,
    session_name: &str,
    component_name: &str,
) -> Result<Session> {
    // Create session
    let session = SessionManager::create_session(
        working_dir.to_path_buf(),
        session_name.to_string(),
        SessionType::Hidden,
    )
    .await
    .with_context(|| format!("Failed to create {} session", component_name))?;

    // Set provider
    agent.update_provider(provider, &session.id).await?;

    // Add extensions from recipe
    if let Some(extensions) = &recipe.extensions {
        for extension in extensions {
            if let Err(e) = agent.add_extension(extension.clone()).await {
                tracing::warn!("Failed to add {} extension: {:?}", component_name, e);
            }
        }
    }

    Ok(session)
}

/// Get working directory, falling back to current directory.
pub fn resolve_working_dir(working_dir: Option<&Path>) -> std::path::PathBuf {
    working_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_config_for_planner() {
        let config = ProviderConfig::for_planner(None, None);
        assert_eq!(config.default_provider, "anthropic");
        assert_eq!(config.default_model, "claude-opus-4-5-20251101");
        assert_eq!(config.component_name, "planner");
    }

    #[test]
    fn test_provider_config_for_reviewer() {
        let config = ProviderConfig::for_reviewer(Some("openai"), Some("gpt-4"));
        assert_eq!(config.provider_override, Some("openai"));
        assert_eq!(config.model_override, Some("gpt-4"));
        assert_eq!(config.component_name, "reviewer");
    }

    #[test]
    fn test_provider_config_for_orchestrator() {
        let config = ProviderConfig::for_orchestrator(None, None);
        assert_eq!(config.default_model, "claude-sonnet-4-20250514");
        assert_eq!(config.component_name, "orchestrator");
    }

    #[test]
    fn test_resolve_working_dir_with_path() {
        let path = std::path::Path::new("/tmp/test");
        let result = resolve_working_dir(Some(path));
        assert_eq!(result, std::path::PathBuf::from("/tmp/test"));
    }

    #[test]
    fn test_resolve_working_dir_without_path() {
        let result = resolve_working_dir(None);
        // Should not panic, returns current dir or empty path
        assert!(result.as_os_str().len() >= 0);
    }
}
