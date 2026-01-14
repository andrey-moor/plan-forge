//! Slug generation utilities.
//!
//! Provides both LLM-based intelligent slug generation and fallback truncation.

use anyhow::Result;
use goose::conversation::message::Message;
use goose::providers::create_with_named_model;
use tracing::{debug, warn};

/// Maximum length for generated slugs (directory names)
/// Target: 20 chars (IDE-friendly), hard limit: 30 chars
const MAX_SLUG_LENGTH: usize = 30;

/// Generate a short, meaningful slug using LLM.
///
/// Uses the provided provider/model configuration to generate a 1-3 word slug
/// (max 20 chars target) that captures the essence of the task. Falls back to
/// truncated slugify if LLM generation fails.
///
/// # Arguments
/// * `task` - The full task description
/// * `provider` - Provider name (e.g., "anthropic", "openai")
/// * `model` - Model name (e.g., "claude-3-5-haiku-latest")
pub async fn generate_slug(task: &str, provider: &str, model: &str) -> String {
    match generate_slug_llm(task, provider, model).await {
        Ok(slug) => {
            debug!("LLM generated slug: {}", slug);
            slug
        }
        Err(e) => {
            warn!("LLM slug generation failed, using fallback: {}", e);
            slugify_truncate(task)
        }
    }
}

/// Generate slug using LLM (internal implementation).
async fn generate_slug_llm(task: &str, provider_name: &str, model_name: &str) -> Result<String> {
    let provider = create_with_named_model(provider_name, model_name).await?;

    let system = r#"Generate a very short slug for the task.

IMPORTANT: These slugs appear as directory names in IDE sidebars - brevity is critical.

Rules:
- 1-3 words maximum (2 words ideal)
- Max 20 characters total
- Lowercase, hyphen-separated
- Capture the core noun/action only
- Skip ALL modifiers: "comprehensive", "new", "better", "based on", etc.
- Use common abbreviations: auth, config, api, db, impl, fix, refactor

Examples:
- "Add user authentication to the Express app" → "user-auth"
- "Create comprehensive execution plan for orchestrator" → "orchestrator"
- "Refactor database connection pool" → "db-pool"
- "Fix bug in payment processing" → "payment-fix"
- "Implement new caching layer for API" → "api-cache"
- "Update logging configuration" → "log-config"

Respond with ONLY the slug, no quotes."#;

    let messages = vec![Message::user().with_text(task)];

    let (response, _usage) = provider.complete(system, &messages, &[]).await?;

    let raw_slug = response.as_concat_text().trim().to_string();

    // Validate and clean the LLM response
    validate_slug(&raw_slug)
}

/// Validate LLM-generated slug and clean it.
fn validate_slug(raw: &str) -> Result<String> {
    // Remove any quotes the LLM might have added
    let cleaned = raw.trim_matches('"').trim_matches('\'').trim();

    // Convert to lowercase and normalize characters (spaces and invalid chars become hyphens)
    let slug: String = cleaned
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();

    // Collapse consecutive dashes
    let mut result = String::new();
    let mut prev_dash = true;
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

    // Trim trailing dash
    if result.ends_with('-') {
        result.pop();
    }

    // Validate length
    if result.is_empty() {
        anyhow::bail!("LLM returned empty slug");
    }
    if result.len() > MAX_SLUG_LENGTH {
        anyhow::bail!("LLM slug too long: {}", result.len());
    }

    Ok(result)
}

/// Convert a string to a URL-friendly slug (basic conversion).
///
/// Does NOT truncate - use `slugify_truncate` for length-limited slugs.
pub fn slugify(title: &str) -> String {
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();

    // Collapse consecutive dashes and trim leading/trailing dashes
    let mut result = String::new();
    let mut prev_dash = true;
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

    if result.ends_with('-') {
        result.pop();
    }

    result
}

/// Convert a string to a URL-friendly slug with truncation.
///
/// Truncates at word boundaries to stay under MAX_SLUG_LENGTH.
/// This is used as a fallback when LLM slug generation fails.
pub fn slugify_truncate(task: &str) -> String {
    let mut result = slugify(task);

    // Truncate if too long, preferring to break at word boundaries
    if result.len() > MAX_SLUG_LENGTH {
        if let Some(pos) = result[..MAX_SLUG_LENGTH].rfind('-') {
            result.truncate(pos);
        } else {
            result.truncate(MAX_SLUG_LENGTH);
        }
    }

    // Ensure no trailing dash after truncation
    if result.ends_with('-') {
        result.pop();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify_basic() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("Test 123!"), "test-123");
        assert_eq!(slugify("  Multiple   Spaces  "), "multiple-spaces");
    }

    #[test]
    fn test_slugify_truncate_long_input() {
        let long_task = "Add MCP server configuration support with bundled recipes environment variables and CLI config flag for the plan-forge tool";
        let slug = slugify_truncate(long_task);
        assert!(slug.len() <= MAX_SLUG_LENGTH);
        assert!(!slug.ends_with('-'));
        assert!(!slug.is_empty());
    }

    #[test]
    fn test_slugify_truncate_short_input() {
        let short_task = "Add auth";
        let slug = slugify_truncate(short_task);
        assert_eq!(slug, "add-auth");
    }

    #[test]
    fn test_validate_slug_clean() {
        assert_eq!(validate_slug("my-slug").unwrap(), "my-slug");
        assert_eq!(validate_slug("\"my-slug\"").unwrap(), "my-slug");
        assert_eq!(validate_slug("'my-slug'").unwrap(), "my-slug");
        assert_eq!(validate_slug("My Slug!").unwrap(), "my-slug");
    }

    #[test]
    fn test_validate_slug_empty() {
        assert!(validate_slug("").is_err());
        assert!(validate_slug("   ").is_err());
        assert!(validate_slug("---").is_err());
    }
}
