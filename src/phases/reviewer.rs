use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use std::path::PathBuf;
use tracing::{debug, info};

use goose::agents::{Agent, AgentEvent, SessionConfig};
use goose::conversation::message::Message;
use goose::session::SessionManager;

use crate::config::{HardChecklist, ReviewConfig};
use crate::models::{LlmReview, Plan, ReviewResult};
use crate::orchestrator::TokenUsage;
use crate::recipes::load_recipe;

use super::{
    create_provider, resolve_working_dir, setup_agent_session, extract_json_block,
    ProviderConfig, ReviewContext, Reviewer,
};

/// Reviewer implementation using goose Agent with hard checklist
pub struct GooseReviewer {
    config: ReviewConfig,
    checklist: HardChecklist,
    base_dir: PathBuf,
    /// Score threshold for passing review (from guardrails.score_threshold)
    score_threshold: f32,
}

impl GooseReviewer {
    pub fn new(config: ReviewConfig, base_dir: PathBuf, score_threshold: f32) -> Self {
        Self {
            config,
            checklist: HardChecklist::default(),
            base_dir,
            score_threshold,
        }
    }

    async fn run_llm_review(&self, plan: &Plan, ctx: &ReviewContext) -> Result<LlmReview> {
        // Load recipe (with fallback to bundled default)
        let recipe = load_recipe(&self.config.recipe, &self.base_dir, "reviewer")?;

        // Create provider using shared utility
        let provider_config = ProviderConfig::for_reviewer(
            self.config.provider_override.as_deref(),
            self.config.model_override.as_deref(),
        );
        let provider = create_provider(&provider_config, &recipe).await?;

        // Create agent and session using shared utility
        let agent = Agent::new();
        let working_dir = resolve_working_dir(ctx.working_dir.as_deref());
        let session_name = format!("reviewer-iteration-{}", ctx.iteration);
        let session = setup_agent_session(
            &agent,
            &recipe,
            provider,
            &working_dir,
            &session_name,
            "reviewer",
        )
        .await?;

        // Apply recipe components
        // Enable final_output_tool for schema-validated structured output
        agent
            .apply_recipe_components(
                recipe.sub_recipes.clone(),
                recipe.response.clone(),
                true, // Use final_output_tool for validated JSON output
            )
            .await;

        // Override system prompt with recipe instructions
        if let Some(instructions) = &recipe.instructions {
            agent.override_system_prompt(instructions.clone()).await;
        }

        let session_config = SessionConfig {
            id: session.id,
            schedule_id: None,
            max_turns: Some(50),
            retry_config: None,
        };

        // Build review prompt
        let prompt = self.build_review_prompt(plan);
        let user_message = Message::user().with_text(&prompt);

        // Stream response
        let mut stream = agent
            .reply(user_message, session_config, None)
            .await
            .context("Failed to start reviewer agent")?;

        let mut last_message = String::new();
        while let Some(event) = stream.next().await {
            match event {
                Ok(AgentEvent::Message(msg)) => {
                    let text = msg.as_concat_text();
                    debug!("Reviewer message: {}", text);
                    // With final_output_tool, the last message is the validated JSON
                    last_message = text;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Reviewer error: {:?}", e);
                }
            }
        }

        parse_llm_review(&last_message)
    }

    fn build_review_prompt(&self, plan: &Plan) -> String {
        let plan_json = serde_json::to_string_pretty(plan).unwrap_or_default();
        self.build_review_prompt_from_json(&plan_json)
    }

    fn build_review_prompt_from_json(&self, plan_json: &str) -> String {
        format!(
            r#"Review the following development plan and identify any gaps, unclear areas, or issues.

## Plan to Review
```json
{}
```

## Review Criteria
1. **Completeness**: Are all aspects of the task covered?
2. **Clarity**: Are descriptions specific and actionable?
3. **Feasibility**: Are tasks realistic and well-scoped?
4. **Dependencies**: Are phase dependencies correct?
5. **Acceptance Criteria**: Are they testable and measurable?
6. **Risks**: Are potential issues identified with mitigations?
7. **File References**: Do the referenced files make sense for the task?

## Instructions
- Use available tools to VERIFY ALL claims about codebase structure
- Any claim about file existence or structure MUST be backed by tool use
- Do not assume codebase structure - search and verify with tools
- Check if referenced files exist in the codebase
- Validate that code patterns mentioned actually exist
- Assess if acceptance criteria are actually testable

## Output Format
Return a JSON object with your review:
```json
{{
  "overall_assessment": "Brief summary of the plan quality",
  "gaps": [
    {{
      "description": "What's missing or incomplete",
      "location": "Where in the plan (e.g., 'Phase 2, Checkpoint 1')",
      "severity": "error|warning|info",
      "suggested_fix": "How to address this gap"
    }}
  ],
  "unclear_areas": [
    {{
      "description": "What needs clarification",
      "questions": ["Specific questions to answer"]
    }}
  ],
  "suggestions": [
    {{
      "description": "Improvement suggestion",
      "rationale": "Why this would help",
      "priority": "required|recommended|optional"
    }}
  ],
  "score": 0.0-1.0
}}
```

Score guidelines:
- 0.9-1.0: Excellent, minor or no issues
- 0.7-0.9: Good, some improvements needed
- 0.5-0.7: Fair, significant gaps to address
- 0.0-0.5: Poor, major revision needed
"#,
            plan_json
        )
    }

    /// Review a plan JSON and return raw JSON value with token usage.
    /// Used by the orchestrator for schema-flexible review.
    pub async fn review_plan_json(&self, plan_json: &Value) -> Result<(Value, TokenUsage)> {
        info!("Running plan review JSON for orchestrator");

        let plan_str = serde_json::to_string_pretty(plan_json).unwrap_or_default();
        let prompt = self.build_review_prompt_from_json(&plan_str);

        // Load recipe
        let recipe = load_recipe(&self.config.recipe, &self.base_dir, "reviewer")?;

        // Create provider using shared utility
        let provider_config = ProviderConfig::for_reviewer(
            self.config.provider_override.as_deref(),
            self.config.model_override.as_deref(),
        );
        let provider = create_provider(&provider_config, &recipe).await?;

        // Create agent and session using shared utility
        let agent = Agent::new();
        let working_dir = resolve_working_dir(None);
        let session_name = format!("orchestrator-reviewer-{}", chrono::Utc::now().timestamp());
        let session = setup_agent_session(
            &agent,
            &recipe,
            provider,
            &working_dir,
            &session_name,
            "reviewer",
        )
        .await?;

        let session_id = session.id.clone();

        agent
            .apply_recipe_components(recipe.sub_recipes.clone(), recipe.response.clone(), true)
            .await;

        if let Some(instructions) = &recipe.instructions {
            agent.override_system_prompt(instructions.clone()).await;
        }

        let session_config = SessionConfig {
            id: session_id.clone(),
            schedule_id: None,
            max_turns: Some(50),
            retry_config: None,
        };

        let user_message = Message::user().with_text(&prompt);
        let mut stream = agent
            .reply(user_message, session_config, None)
            .await
            .context("Failed to start reviewer agent")?;

        let mut last_message = String::new();
        while let Some(event) = stream.next().await {
            match event {
                Ok(AgentEvent::Message(msg)) => {
                    last_message = msg.as_concat_text();
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Reviewer error: {:?}", e);
                }
            }
        }

        // Get token usage from session
        let token_usage = if let Ok(sess) =
            SessionManager::get_session(&session_id, false).await
        {
            TokenUsage::new(sess.accumulated_input_tokens, sess.accumulated_output_tokens)
        } else {
            TokenUsage::default()
        };

        // Parse response as JSON Value (flexible schema)
        let review_json: Value = if let Ok(json) = serde_json::from_str(&last_message) {
            json
        } else if let Some(json_str) = extract_json_block(&last_message) {
            serde_json::from_str(json_str).unwrap_or_else(|_| {
                serde_json::json!({
                    "overall_assessment": "Failed to parse review",
                    "score": 0.5,
                    "passed": false,
                    "summary": "Review parsing failed"
                })
            })
        } else {
            serde_json::json!({
                "overall_assessment": "Failed to parse review response",
                "score": 0.5,
                "passed": false,
                "summary": "Review parsing failed - response was not valid JSON"
            })
        };

        // Add passed field based on score and threshold
        let score = review_json.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let passed = score >= self.score_threshold;

        let mut result = review_json;
        if let Some(obj) = result.as_object_mut() {
            obj.insert("passed".to_string(), Value::Bool(passed));
            obj.insert(
                "summary".to_string(),
                Value::String(format!(
                    "Review score: {:.2} (threshold: {:.2}) - {}",
                    score,
                    self.score_threshold,
                    if passed { "PASSED" } else { "NEEDS REVISION" }
                )),
            );
        }

        Ok((result, token_usage))
    }
}

#[async_trait]
impl Reviewer for GooseReviewer {
    async fn review_plan(&self, plan: &Plan, ctx: &ReviewContext) -> Result<ReviewResult> {
        info!("Running review for iteration {}", ctx.iteration);

        // Step 1: Run hard checks (fast, deterministic)
        info!("Running hard validation checks...");
        let hard_results = self.checklist.run_all(plan);

        let hard_failures: Vec<_> = hard_results
            .iter()
            .filter(|r| !r.passed && r.severity == crate::models::Severity::Error)
            .collect();
        info!(
            "Hard checks: {} passed, {} failed",
            hard_results.len() - hard_failures.len(),
            hard_failures.len()
        );

        // Step 2: Run LLM qualitative review
        info!("Running LLM review...");
        let llm_review = self.run_llm_review(plan, ctx).await?;
        info!("LLM review score: {:.2}", llm_review.score);

        // Calculate if passed
        let passed = hard_failures.is_empty() && llm_review.score >= self.score_threshold;

        let summary = if passed {
            format!(
                "Plan PASSED review with score {:.2} (threshold: {:.2})",
                llm_review.score, self.score_threshold
            )
        } else {
            format!(
                "Plan NEEDS REVISION: {} hard check failures, {} gaps, score {:.2} (threshold: {:.2})",
                hard_failures.len(),
                llm_review.gaps.len(),
                llm_review.score,
                self.score_threshold
            )
        };

        Ok(ReviewResult {
            passed,
            hard_check_results: hard_results,
            llm_review,
            summary,
        })
    }
}

/// Parse LLM review from response
/// With final_output_tool enabled, the response is schema-validated JSON
fn parse_llm_review(response: &str) -> Result<LlmReview> {
    // First try parsing directly (final_output_tool returns clean JSON)
    if let Ok(review) = serde_json::from_str(response) {
        return Ok(review);
    }

    // Fallback: try to extract JSON block
    if let Some(json_str) = extract_json_block(response) {
        serde_json::from_str(json_str).context("Failed to parse LLM review JSON")
    } else {
        // Return default with low score if parsing fails
        tracing::warn!("Could not parse LLM review, using default");
        Ok(LlmReview {
            overall_assessment: "Failed to parse review response".to_string(),
            score: 0.5,
            ..Default::default()
        })
    }
}
