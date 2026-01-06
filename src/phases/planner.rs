use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, info};

use goose::agents::{Agent, AgentEvent, SessionConfig};
use goose::conversation::message::Message;
use goose::providers::{base::Provider, create_with_named_model};
use goose::recipe::Recipe;
use goose::session::{session_manager::SessionType, SessionManager};

use crate::config::PlanningConfig;
use crate::models::Plan;
use crate::orchestrator::LoopState;

use super::Planner;

/// Planner implementation using goose Agent
pub struct GoosePlanner {
    config: PlanningConfig,
    base_dir: PathBuf,
}

impl GoosePlanner {
    pub fn new(config: PlanningConfig, base_dir: PathBuf) -> Self {
        Self { config, base_dir }
    }

    async fn create_provider(&self, recipe: &Recipe) -> Result<Arc<dyn Provider>> {
        // Use override if provided, otherwise use recipe settings
        let provider_name = self
            .config
            .provider_override
            .as_deref()
            .or(recipe.settings.as_ref().and_then(|s| s.goose_provider.as_deref()))
            .unwrap_or("anthropic");

        let model_name = self
            .config
            .model_override
            .as_deref()
            .or(recipe.settings.as_ref().and_then(|s| s.goose_model.as_deref()))
            .unwrap_or("claude-opus-4-5-20251101");

        info!("Creating provider: {} with model: {}", provider_name, model_name);
        create_with_named_model(provider_name, model_name)
            .await
            .context("Failed to create provider")
    }

    async fn run_agent(&self, prompt: &str, state: &LoopState) -> Result<String> {
        // Load recipe
        let recipe_path = self.base_dir.join(&self.config.recipe);
        let recipe = Recipe::from_file_path(&recipe_path)
            .context(format!("Failed to load recipe from {:?}", recipe_path))?;

        // Create provider
        let provider = self.create_provider(&recipe).await?;

        // Create agent
        let agent = Agent::new();

        // Create session
        let working_dir = state
            .conversation_context
            .working_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let session = SessionManager::create_session(
            working_dir.clone(),
            format!("planner-iteration-{}", state.iteration),
            SessionType::Hidden,
        )
        .await
        .context("Failed to create session")?;

        // Set provider
        agent.update_provider(provider, &session.id).await?;

        // Add extensions from recipe
        if let Some(extensions) = &recipe.extensions {
            for extension in extensions {
                if let Err(e) = agent.add_extension(extension.clone()).await {
                    tracing::warn!("Failed to add extension: {:?}", e);
                }
            }
        }

        // Apply recipe components (sub_recipes, response schema)
        // Enable final_output_tool for schema-validated structured output
        agent
            .apply_recipe_components(
                recipe.sub_recipes.clone(),
                recipe.response.clone(),
                true, // Use final_output_tool for validated JSON output
            )
            .await;

        // Override system prompt with recipe instructions if provided
        if let Some(instructions) = &recipe.instructions {
            agent.override_system_prompt(instructions.clone()).await;
        }

        let session_config = SessionConfig {
            id: session.id,
            schedule_id: None,
            max_turns: Some(100),
            retry_config: None,
        };

        // Create user message with the prompt
        let user_message = Message::user().with_text(prompt);

        // Stream response (cancel_token is None)
        let mut stream = agent
            .reply(user_message, session_config, None)
            .await
            .context("Failed to start agent reply")?;

        let mut last_message = String::new();
        while let Some(event) = stream.next().await {
            match event {
                Ok(AgentEvent::Message(msg)) => {
                    let text = msg.as_concat_text();
                    debug!("Agent message: {}", text);
                    // With final_output_tool, the last message is the validated JSON
                    last_message = text;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Agent error: {:?}", e);
                }
            }
        }

        Ok(last_message)
    }

    fn build_initial_prompt(&self, state: &LoopState) -> String {
        format!(
            r#"Create a comprehensive development plan for the following task:

## Task
{}

## Requirements
- Output your plan as a JSON object
- Include all phases, checkpoints, and tasks
- Identify acceptance criteria, file references, and risks
- Use deterministic language (no "might", "consider", etc.)

## Output Format
Return a JSON object with the following structure:
```json
{{
  "title": "string",
  "description": "string",
  "tier": "quick|standard|strategic",
  "context": {{
    "problem_statement": "string",
    "constraints": ["string"],
    "assumptions": ["string"],
    "existing_patterns": ["string"]
  }},
  "phases": [{{
    "name": "string",
    "goal": "string",
    "tier": "foundation|core|enhancement|polish",
    "checkpoints": [{{
      "id": "string",
      "description": "string",
      "tasks": [{{
        "description": "string",
        "file_references": ["string"],
        "implementation_notes": "string|null"
      }}],
      "validation": "string|null"
    }}],
    "dependencies": ["string"]
  }}],
  "acceptance_criteria": [{{
    "description": "string",
    "testable": true,
    "priority": "required|recommended|optional"
  }}],
  "file_references": [{{
    "path": "string",
    "exists": true|false|null,
    "action": "create|modify|reference|delete",
    "description": "string"
  }}],
  "risks": [{{
    "description": "string",
    "severity": "error|warning|info",
    "mitigation": "string"
  }}],
  "metadata": {{
    "version": 1,
    "created_at": "ISO8601 timestamp",
    "last_updated": "ISO8601 timestamp",
    "iteration": 1
  }}
}}
```
"#,
            state.conversation_context.original_task
        )
    }

    fn build_update_prompt(&self, state: &LoopState, current_plan: &Plan) -> String {
        let feedback = state.conversation_context.pending_feedback.join("\n");
        let plan_json = serde_json::to_string_pretty(current_plan).unwrap_or_default();

        format!(
            r#"Update the following development plan based on review feedback.

## Original Task
{}

## Current Plan
```json
{}
```

## Review Feedback to Address
{}

## Requirements
- Address ALL feedback items marked [MUST FIX] and [CRITICAL]
- Consider items marked [SHOULD FIX] and [CONSIDER]
- Clarify items marked [CLARIFY]
- Output the updated plan as a JSON object with the same structure
- Increment the metadata.version and metadata.iteration
- Update metadata.last_updated

Return ONLY the updated JSON plan.
"#,
            state.conversation_context.original_task, plan_json, feedback
        )
    }
}

#[async_trait]
impl Planner for GoosePlanner {
    async fn generate_plan(&self, state: &LoopState) -> Result<Plan> {
        let prompt = if state.is_first_iteration() {
            info!("Generating initial plan");
            self.build_initial_prompt(state)
        } else {
            info!("Updating plan based on feedback");
            let current_plan = state
                .current_plan
                .as_ref()
                .context("No current plan for update")?;
            self.build_update_prompt(state, current_plan)
        };

        let response = self.run_agent(&prompt, state).await?;
        parse_plan_from_response(&response)
    }
}

/// Parse a Plan from the agent's response
/// With final_output_tool enabled, the response is schema-validated JSON
fn parse_plan_from_response(response: &str) -> Result<Plan> {
    // First try parsing directly (final_output_tool returns clean JSON)
    if let Ok(plan) = serde_json::from_str(response) {
        return Ok(plan);
    }

    // Fallback: try to extract JSON block from response
    if let Some(json_str) = extract_json_block(response) {
        serde_json::from_str(json_str).context("Failed to parse plan JSON")
    } else {
        serde_json::from_str(response).context("Response is not valid JSON")
    }
}

/// Extract JSON block from markdown-formatted response (fallback)
fn extract_json_block(text: &str) -> Option<&str> {
    // Look for ```json ... ``` blocks
    if let Some(start) = text.find("```json") {
        let content_start = start + 7;
        if let Some(end) = text[content_start..].find("```") {
            return Some(text[content_start..content_start + end].trim());
        }
    }

    // Try finding raw JSON object
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            return Some(&text[start..=end]);
        }
    }

    None
}
