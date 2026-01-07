//! Plan-Forge MCP Server implementation.
//!
//! Exposes planning tools to AI assistants via MCP protocol.

use rmcp::{
    handler::server::router::tool::ToolRouter,
    model::{CallToolResult, Content, ErrorCode, ErrorData, Implementation, Role, ServerCapabilities, ServerInfo},
    schemars::JsonSchema,
    tool, tool_handler, tool_router, ServerHandler,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use crate::{
    generate_slug, slugify, CliConfig, FileOutputWriter, GoosePlanner, GooseReviewer,
    LoopController, OutputConfig, OutputWriter, Plan, ResumeState,
};

use super::status::{derive_status, list_sessions, SessionInfo, SessionStatus};

// ============================================================================
// Session Metadata
// ============================================================================

/// Metadata stored for each planning session.
///
/// Saved to `.plan-forge/<slug>/session-meta.json` and used to maintain
/// consistent slugs across session operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    /// The slug used for this session (directory name)
    pub slug: String,
    /// The original task description
    pub task: String,
    /// When the session was created
    pub created_at: String,
}

impl SessionMeta {
    pub fn new(slug: String, task: String) -> Self {
        Self {
            slug,
            task,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

// ============================================================================
// Tool Parameters
// ============================================================================

/// Parameters for the plan_status tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PlanStatusParams {
    /// Session ID to check. If not provided, uses current session.
    pub session_id: Option<String>,
}

/// Parameters for the plan_list tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PlanListParams {
    /// Maximum number of sessions to return (default: 10)
    pub limit: Option<u32>,
}

/// Parameters for the plan_get tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PlanGetParams {
    /// Which file to read: "plan", "tasks", or "context"
    pub file: String,
    /// Session ID. If not provided, uses current session.
    pub session_id: Option<String>,
}

/// Parameters for the plan_run tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PlanRunParams {
    /// Task description (for new session) or feedback/answer (for resume)
    pub task: String,
    /// Session ID to resume. If not provided, creates new session or uses current.
    pub session_id: Option<String>,
    /// Reset turns counter when resuming (default: false)
    #[serde(default)]
    pub reset_turns: bool,
}

/// Parameters for the plan_approve tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PlanApproveParams {
    /// Session ID to approve. If not provided, uses current session.
    pub session_id: Option<String>,
}

// ============================================================================
// Server Implementation
// ============================================================================

/// Plan-Forge MCP Server
///
/// Exposes planning tools for AI assistants to create, review, and manage
/// development plans.
#[derive(Clone)]
pub struct PlanForgeServer {
    tool_router: ToolRouter<Self>,
    /// Current session ID (tracked in memory)
    current_session: Arc<RwLock<Option<String>>>,
    /// Configuration
    config: Arc<CliConfig>,
    /// Base directory (where .plan-forge/ lives)
    base_dir: PathBuf,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for PlanForgeServer {
    fn get_info(&self) -> ServerInfo {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());

        let instructions = format!(
            r#"Plan-Forge: AI-powered development planning tool.

This server provides tools for creating, reviewing, and managing development plans.
Plans are stored in .plan-forge/<session>/ and output to ./dev/active/<session>/.

Available tools:
- plan_run: Create a new planning session or resume an existing one
- plan_status: Check the status of a planning session
- plan_list: List all planning sessions
- plan_get: Read plan, tasks, or context markdown files
- plan_approve: Force approve a plan (write to dev/active/ even if review failed)

Current directory: {cwd}
"#
        );

        ServerInfo {
            server_info: Implementation {
                name: "plan-forge".to_string(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                title: Some("Plan-Forge".to_string()),
                icons: None,
                website_url: None,
            },
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(instructions),
            ..Default::default()
        }
    }
}

#[tool_router(router = tool_router)]
impl PlanForgeServer {
    /// Create a new server with auto-detected config and env overrides.
    ///
    /// Config resolution:
    /// 1. Try ./config/default.yaml
    /// 2. Try ./plan-forge.yaml
    /// 3. Use defaults
    /// 4. Apply PLAN_FORGE_* environment variable overrides
    pub fn new() -> Self {
        let base_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        // Try to auto-detect config file
        let config_path = Self::auto_detect_config(&base_dir);
        let config = CliConfig::load_with_env(config_path.as_ref())
            .unwrap_or_else(|_| CliConfig::default().apply_env_overrides());

        Self {
            tool_router: Self::tool_router(),
            current_session: Arc::new(RwLock::new(None)),
            config: Arc::new(config),
            base_dir,
        }
    }

    /// Create with custom configuration (with env overrides applied)
    pub fn with_config(config: CliConfig) -> Self {
        let base_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let config = config.apply_env_overrides();

        Self {
            tool_router: Self::tool_router(),
            current_session: Arc::new(RwLock::new(None)),
            config: Arc::new(config),
            base_dir,
        }
    }

    /// Auto-detect config file in the current directory
    fn auto_detect_config(base_dir: &Path) -> Option<PathBuf> {
        // Check common config file locations (in priority order)
        let candidates = [
            base_dir.join(".plan-forge/config.yaml"),
            base_dir.join("plan-forge.yaml"),
            base_dir.join(".plan-forge.yaml"),
            base_dir.join("config/default.yaml"),
        ];

        candidates.into_iter().find(|path| path.exists())
    }

    /// Get the .plan-forge directory path
    fn plan_forge_dir(&self) -> PathBuf {
        self.base_dir.join(".plan-forge")
    }

    /// Get session directory for a given session ID
    fn session_dir(&self, session_id: &str) -> PathBuf {
        self.plan_forge_dir().join(session_id)
    }

    /// Resolve session ID - use provided, current, or error
    fn resolve_session(&self, session_id: Option<String>) -> Result<String, ErrorData> {
        if let Some(id) = session_id {
            return Ok(id);
        }

        let current = self.current_session.read().map_err(|_| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "Failed to read current session".to_string(),
                None,
            )
        })?;

        current.clone().ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                "No session specified and no current session active".to_string(),
                None,
            )
        })
    }

    /// Set current session
    fn set_current_session(&self, session_id: String) {
        if let Ok(mut current) = self.current_session.write() {
            *current = Some(session_id);
        }
    }

    // ========================================================================
    // Read-Only Tools
    // ========================================================================

    /// Get the status of a planning session.
    ///
    /// Returns session status, iteration count, latest score, and other metadata.
    /// Status can be: ready, in_progress, needs_input, approved, or max_turns.
    #[tool(
        name = "plan_status",
        description = "Get the status of a planning session. Returns status (ready/in_progress/needs_input/approved/max_turns), iteration count, score, and other metadata."
    )]
    pub async fn plan_status(
        &self,
        params: rmcp::handler::server::wrapper::Parameters<PlanStatusParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let session_id = self.resolve_session(params.0.session_id)?;
        let session_dir = self.session_dir(&session_id);

        let info = derive_status(
            &session_dir,
            self.config.review.pass_threshold,
            self.config.loop_config.max_iterations,
        )
        .map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to derive status: {}", e),
                None,
            )
        })?;

        let response = serde_json::to_string_pretty(&info).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to serialize status: {}", e),
                None,
            )
        })?;

        Ok(CallToolResult::success(vec![Content::text(response)
            .with_audience(vec![Role::Assistant])]))
    }

    /// List all planning sessions.
    ///
    /// Returns a list of session IDs with their status, sorted by most recent first.
    #[tool(
        name = "plan_list",
        description = "List all planning sessions in .plan-forge/. Returns session IDs with status, sorted by most recent first."
    )]
    pub async fn plan_list(
        &self,
        params: rmcp::handler::server::wrapper::Parameters<PlanListParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let limit = params.0.limit.unwrap_or(10) as usize;
        let plan_forge_dir = self.plan_forge_dir();

        let sessions = list_sessions(&plan_forge_dir).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to list sessions: {}", e),
                None,
            )
        })?;

        let mut session_infos: Vec<SessionInfo> = Vec::new();

        for session_id in sessions.into_iter().take(limit) {
            let session_dir = self.session_dir(&session_id);
            if let Ok(info) = derive_status(
                &session_dir,
                self.config.review.pass_threshold,
                self.config.loop_config.max_iterations,
            ) {
                session_infos.push(info);
            }
        }

        let response = serde_json::to_string_pretty(&session_infos).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to serialize sessions: {}", e),
                None,
            )
        })?;

        Ok(CallToolResult::success(vec![Content::text(response)
            .with_audience(vec![Role::Assistant])]))
    }

    /// Read plan content from dev/active/ directory.
    ///
    /// Reads one of the plan markdown files: plan, tasks, or context.
    #[tool(
        name = "plan_get",
        description = "Read plan content. Specify file='plan', 'tasks', or 'context' to read the corresponding markdown file from dev/active/<session>/."
    )]
    pub async fn plan_get(
        &self,
        params: rmcp::handler::server::wrapper::Parameters<PlanGetParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let session_id = self.resolve_session(params.0.session_id)?;
        let file_type = params.0.file.to_lowercase();
        let session_dir = self.session_dir(&session_id);

        // Try to use session metadata slug, fall back to plan title slug for backwards compat
        let slug = self
            .load_session_meta(&session_dir)
            .map(|m| m.slug)
            .or_else(|_| {
                self.load_latest_plan(&session_dir)
                    .map(|p| slugify(&p.title))
            })?;

        let filename = match file_type.as_str() {
            "plan" => format!("{}-plan.md", slug),
            "tasks" => format!("{}-tasks.md", slug),
            "context" => format!("{}-context.md", slug),
            _ => {
                return Err(ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    format!(
                        "Invalid file type '{}'. Use 'plan', 'tasks', or 'context'.",
                        file_type
                    ),
                    None,
                ));
            }
        };

        // Files are in dev/active/<slug>/<slug>-<type>.md
        let file_path = self
            .base_dir
            .join("dev/active")
            .join(&slug)
            .join(&filename);

        let content = std::fs::read_to_string(&file_path).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to read {}: {}", file_path.display(), e),
                None,
            )
        })?;

        Ok(CallToolResult::success(vec![Content::text(content)
            .with_audience(vec![Role::Assistant])]))
    }

    // ========================================================================
    // Execution Tools
    // ========================================================================

    /// Create or resume a planning session.
    ///
    /// - For new sessions: task is the task description
    /// - For resume: task is feedback or answer to continue planning
    ///
    /// This tool runs the plan-review loop until it reaches a pause point
    /// (approved, needs_input, or max_turns).
    #[tool(
        name = "plan_run",
        description = "Create a new planning session or resume an existing one. For new sessions, 'task' is the task description. For resume, 'task' is feedback to incorporate. Runs the plan-review loop until approved, needs_input, or max_turns."
    )]
    pub async fn plan_run(
        &self,
        params: rmcp::handler::server::wrapper::Parameters<PlanRunParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let task = params.0.task.clone();
        let session_id = params.0.session_id.clone();
        let reset_turns = params.0.reset_turns;

        // Determine if this is a new session or resume
        let (task_slug, resume_state, is_new_session) = if let Some(sid) = session_id {
            // Resume existing session
            let session_dir = self.session_dir(&sid);

            if !session_dir.exists() {
                return Err(ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    format!("Session '{}' not found", sid),
                    None,
                ));
            }

            // Load latest plan for resume
            let resume = self.load_resume_state(&session_dir, &task, reset_turns)?;
            (sid, Some(resume), false)
        } else {
            // Check if we have a current session to resume
            let current = self.current_session.read().ok().and_then(|c| c.clone());

            if let Some(sid) = current {
                let session_dir = self.session_dir(&sid);
                let info = derive_status(
                    &session_dir,
                    self.config.review.pass_threshold,
                    self.config.loop_config.max_iterations,
                )
                .ok();

                // If current session is in a resumable state, resume it
                if let Some(info) = info
                    && matches!(
                        info.status,
                        SessionStatus::NeedsInput | SessionStatus::MaxTurns | SessionStatus::InProgress
                    )
                {
                    let resume = self.load_resume_state(&session_dir, &task, reset_turns)?;
                    return self.run_loop(sid, task, Some(resume), false).await;
                }
            }

            // New session - generate slug using LLM
            let (provider, model) = self.get_slug_provider_model();
            let slug = generate_slug(&task, &provider, &model).await;
            (slug, None, true)
        };

        self.run_loop(task_slug, task, resume_state, is_new_session).await
    }

    /// Force approve a session and write to dev/active/.
    ///
    /// Use this to approve a plan even if it didn't pass review.
    #[tool(
        name = "plan_approve",
        description = "Force approve a planning session and write output to dev/active/. Use when you want to proceed with a plan even if it didn't pass automated review."
    )]
    pub async fn plan_approve(
        &self,
        params: rmcp::handler::server::wrapper::Parameters<PlanApproveParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let session_id = self.resolve_session(params.0.session_id)?;
        let session_dir = self.session_dir(&session_id);

        // Load latest plan
        let plan = self.load_latest_plan(&session_dir)?;

        // Try to use session metadata slug, fall back to plan title slug for backwards compat
        let slug = self
            .load_session_meta(&session_dir)
            .map(|m| m.slug)
            .unwrap_or_else(|_| slugify(&plan.title));

        // Write to dev/active/ with the session slug
        let output = FileOutputWriter::new(OutputConfig {
            runs_dir: session_dir.clone(),
            active_dir: self.base_dir.join("dev/active"),
            slug: Some(slug.clone()),
        });

        output.write_final(&plan).await.map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to write plan: {}", e),
                None,
            )
        })?;

        let response = format!(
            "Plan '{}' approved and written to dev/active/{}/",
            plan.title, slug
        );

        Ok(CallToolResult::success(vec![Content::text(response)
            .with_audience(vec![Role::Assistant])]))
    }

    // ========================================================================
    // Helper Methods
    // ========================================================================

    /// Load the latest plan from a session directory
    fn load_latest_plan(&self, session_dir: &PathBuf) -> Result<Plan, ErrorData> {
        let mut highest_iteration = 0u32;
        let mut latest_plan_path = None;

        let entries = std::fs::read_dir(session_dir).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to read session directory: {}", e),
                None,
            )
        })?;

        for entry in entries.flatten() {
            let filename = entry.file_name();
            let filename_str = filename.to_string_lossy();

            if let Some(iter_str) = filename_str
                .strip_prefix("plan-iteration-")
                .and_then(|s| s.strip_suffix(".json"))
            {
                if let Ok(iter) = iter_str.parse::<u32>() {
                    if iter > highest_iteration {
                        highest_iteration = iter;
                        latest_plan_path = Some(entry.path());
                    }
                }
            }
        }

        let plan_path = latest_plan_path.ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "No plan files found in session".to_string(),
                None,
            )
        })?;

        let plan_json = std::fs::read_to_string(&plan_path).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to read plan: {}", e),
                None,
            )
        })?;

        serde_json::from_str(&plan_json).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to parse plan: {}", e),
                None,
            )
        })
    }

    /// Load resume state from session directory
    fn load_resume_state(
        &self,
        session_dir: &PathBuf,
        feedback: &str,
        reset_turns: bool,
    ) -> Result<ResumeState, ErrorData> {
        let plan = self.load_latest_plan(session_dir)?;

        // Find highest iteration
        let mut highest_iteration = 0u32;
        if let Ok(entries) = std::fs::read_dir(session_dir) {
            for entry in entries.flatten() {
                let filename = entry.file_name();
                let filename_str = filename.to_string_lossy();

                if let Some(iter_str) = filename_str
                    .strip_prefix("plan-iteration-")
                    .and_then(|s| s.strip_suffix(".json"))
                {
                    if let Ok(iter) = iter_str.parse::<u32>() {
                        highest_iteration = highest_iteration.max(iter);
                    }
                }
            }
        }

        let start_iteration = if reset_turns { 1 } else { highest_iteration + 1 };

        let feedback_items = if feedback.is_empty() {
            Vec::new()
        } else {
            vec![format!("[USER FEEDBACK] {}", feedback)]
        };

        Ok(ResumeState {
            plan,
            feedback: feedback_items,
            start_iteration,
        })
    }

    /// Save session metadata to session directory
    fn save_session_meta(&self, session_dir: &PathBuf, meta: &SessionMeta) -> Result<(), ErrorData> {
        let meta_path = session_dir.join("session-meta.json");
        let json = serde_json::to_string_pretty(meta).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to serialize session metadata: {}", e),
                None,
            )
        })?;
        std::fs::write(&meta_path, json).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to write session metadata: {}", e),
                None,
            )
        })?;
        Ok(())
    }

    /// Load session metadata from session directory
    fn load_session_meta(&self, session_dir: &PathBuf) -> Result<SessionMeta, ErrorData> {
        let meta_path = session_dir.join("session-meta.json");
        let json = std::fs::read_to_string(&meta_path).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to read session metadata: {}", e),
                None,
            )
        })?;
        serde_json::from_str(&json).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to parse session metadata: {}", e),
                None,
            )
        })
    }

    /// Get provider and model for slug generation (uses planner config)
    fn get_slug_provider_model(&self) -> (String, String) {
        let provider = self
            .config
            .planning
            .provider_override
            .clone()
            .unwrap_or_else(|| "anthropic".to_string());
        let model = self
            .config
            .planning
            .model_override
            .clone()
            .unwrap_or_else(|| "claude-opus-4-5-20251101".to_string());
        (provider, model)
    }

    /// Run the planning loop
    async fn run_loop(
        &self,
        task_slug: String,
        task: String,
        resume_state: Option<ResumeState>,
        is_new_session: bool,
    ) -> Result<CallToolResult, ErrorData> {
        // Set as current session
        self.set_current_session(task_slug.clone());

        // Create session directory
        let session_dir = self.session_dir(&task_slug);
        std::fs::create_dir_all(&session_dir).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to create session directory: {}", e),
                None,
            )
        })?;

        // Save session metadata for new sessions
        if is_new_session {
            let meta = SessionMeta::new(task_slug.clone(), task.clone());
            self.save_session_meta(&session_dir, &meta)?;
        }

        // Set up config with session paths
        let mut config = (*self.config).clone();
        config.output.runs_dir = session_dir.clone();
        config.output.active_dir = self.base_dir.join("dev/active");
        // Set the slug so output files use the same directory name
        config.output.slug = Some(task_slug.clone());

        // Determine base directory for recipes
        let base_dir = self.base_dir.clone();

        // Create components
        let planner = GoosePlanner::new(config.planning.clone(), base_dir.clone());
        let reviewer = GooseReviewer::new(config.review.clone(), base_dir);
        let output = FileOutputWriter::new(config.output.clone());

        // Create loop controller
        let mut controller =
            LoopController::new(planner, reviewer, output, config).with_task_slug(task_slug.clone());

        // Apply resume state if present
        if let Some(resume) = resume_state {
            controller = controller.with_resume(resume);
        }

        // Run the loop
        let result = controller.run(task, None).await;

        match result {
            Ok(result) => {
                let response = serde_json::json!({
                    "session_id": task_slug,
                    "status": if result.success { "approved" } else { "max_turns" },
                    "iterations": result.total_iterations,
                    "score": result.final_review.llm_review.score,
                    "title": result.final_plan.title,
                    "summary": result.final_review.summary,
                });

                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_audience(vec![Role::Assistant])]))
            }
            Err(e) => {
                let error_msg = e.to_string();

                // Check if this is a human input required error
                if error_msg.contains("Human input required") {
                    let info = derive_status(
                        &session_dir,
                        self.config.review.pass_threshold,
                        self.config.loop_config.max_iterations,
                    )
                    .ok();

                    let response = serde_json::json!({
                        "session_id": task_slug,
                        "status": "needs_input",
                        "reason": info.as_ref().and_then(|i| i.input_reason.clone()),
                        "iteration": info.as_ref().map(|i| i.iteration),
                        "score": info.as_ref().and_then(|i| i.latest_score),
                    });

                    return Ok(CallToolResult::success(vec![Content::text(
                        serde_json::to_string_pretty(&response).unwrap_or_default(),
                    )
                    .with_audience(vec![Role::Assistant])]));
                }

                Err(ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Planning failed: {}", error_msg),
                    None,
                ))
            }
        }
    }
}

impl Default for PlanForgeServer {
    fn default() -> Self {
        Self::new()
    }
}
