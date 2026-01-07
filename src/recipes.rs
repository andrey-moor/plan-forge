//! Bundled default recipes for plan-forge.
//!
//! Recipes are embedded in the binary using include_str! and can be used as
//! fallbacks when no external recipe file is found.

use anyhow::{Context, Result};
use goose::recipe::Recipe;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Bundled default planner recipe
pub const DEFAULT_PLANNER_RECIPE: &str = include_str!("../recipes/planner.yaml");

/// Bundled default reviewer recipe
pub const DEFAULT_REVIEWER_RECIPE: &str = include_str!("../recipes/reviewer.yaml");

/// Recipe resolution result
pub enum RecipeSource {
    /// Recipe loaded from a file path
    File(PathBuf),
    /// Recipe loaded from bundled default
    Bundled(&'static str),
}

impl RecipeSource {
    /// Get the recipe content
    pub fn content(&self) -> std::io::Result<String> {
        match self {
            RecipeSource::File(path) => std::fs::read_to_string(path),
            RecipeSource::Bundled(content) => Ok(content.to_string()),
        }
    }

    /// Load a Recipe from this source
    pub fn load_recipe(&self) -> Result<Recipe> {
        match self {
            RecipeSource::File(path) => {
                Recipe::from_file_path(path).context(format!("Failed to load recipe from {:?}", path))
            }
            RecipeSource::Bundled(content) => {
                // Recipe implements Deserialize, so we can parse directly from YAML
                serde_yaml::from_str(content).context("Failed to parse bundled recipe")
            }
        }
    }
}

/// Resolve a recipe path, falling back to bundled default if not found.
///
/// Resolution priority:
/// 1. Explicit path if it exists
/// 2. Project-local `.plan-forge/recipes/<name>.yaml`
/// 3. Bundled default
///
/// # Arguments
/// * `recipe_path` - Configured recipe path (may be relative)
/// * `base_dir` - Base directory to resolve relative paths from
/// * `recipe_name` - Recipe name for lookup (e.g., "planner", "reviewer")
pub fn resolve_recipe(
    recipe_path: &Path,
    base_dir: &Path,
    recipe_name: &str,
) -> RecipeSource {
    // 1. Try explicit path
    let explicit_path = if recipe_path.is_absolute() {
        recipe_path.to_path_buf()
    } else {
        base_dir.join(recipe_path)
    };

    if explicit_path.exists() {
        debug!("Using recipe from explicit path: {:?}", explicit_path);
        return RecipeSource::File(explicit_path);
    }

    // 2. Try project-local .plan-forge/recipes/
    let plan_forge_path = base_dir
        .join(".plan-forge/recipes")
        .join(format!("{}.yaml", recipe_name));
    if plan_forge_path.exists() {
        debug!("Using recipe from .plan-forge: {:?}", plan_forge_path);
        return RecipeSource::File(plan_forge_path);
    }

    // 3. Fall back to bundled default
    debug!("Using bundled default recipe for: {}", recipe_name);
    match recipe_name {
        "planner" => RecipeSource::Bundled(DEFAULT_PLANNER_RECIPE),
        "reviewer" => RecipeSource::Bundled(DEFAULT_REVIEWER_RECIPE),
        _ => {
            // Unknown recipe name - try to use the first bundled recipe as fallback
            // This shouldn't happen in normal usage
            RecipeSource::Bundled(DEFAULT_PLANNER_RECIPE)
        }
    }
}

/// Convenience function to resolve and load a recipe.
///
/// This combines resolution and loading in one step.
pub fn load_recipe(
    recipe_path: &Path,
    base_dir: &Path,
    recipe_name: &str,
) -> Result<Recipe> {
    let source = resolve_recipe(recipe_path, base_dir, recipe_name);
    source.load_recipe()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bundled_recipes_not_empty() {
        assert!(!DEFAULT_PLANNER_RECIPE.is_empty());
        assert!(!DEFAULT_REVIEWER_RECIPE.is_empty());
    }

    #[test]
    fn test_bundled_recipes_valid_yaml() {
        // Just check they parse as YAML
        let _: serde_yaml::Value =
            serde_yaml::from_str(DEFAULT_PLANNER_RECIPE).expect("planner recipe should be valid YAML");
        let _: serde_yaml::Value =
            serde_yaml::from_str(DEFAULT_REVIEWER_RECIPE).expect("reviewer recipe should be valid YAML");
    }

    #[test]
    fn test_resolve_recipe_bundled_fallback() {
        // With non-existent path, should fall back to bundled
        let source = resolve_recipe(
            Path::new("nonexistent/path.yaml"),
            Path::new("/tmp"),
            "planner",
        );
        assert!(matches!(source, RecipeSource::Bundled(_)));
    }
}
