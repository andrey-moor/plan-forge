use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[allow(deprecated)]
use plan_forge::{
    generate_slug, slugify_truncate,
    CliConfig, FileOutputWriter, GooseOrchestrator, GoosePlanner, GooseReviewer, HumanResponse,
    LoopController, OrchestrationState, Plan, PlanForgeServer, ResumeState, SessionRegistry, slugify,
};

// Re-export MCP server types from goose-mcp
use goose_mcp::DeveloperServer;
use goose_mcp::mcp_server_runner::serve;

/// MCP server variants available in plan-forge
#[derive(Clone, Debug)]
pub enum McpServer {
    /// Developer tools server (from goose)
    Developer,
    /// Plan-forge planning server
    PlanForge,
}

impl std::str::FromStr for McpServer {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace([' ', '-'], "").as_str() {
            "developer" => Ok(McpServer::Developer),
            "planforge" => Ok(McpServer::PlanForge),
            _ => Err(format!(
                "Invalid MCP server: '{}'. Available: developer, plan-forge",
                s
            )),
        }
    }
}

/// Set up isolated configuration directory for plan-forge.
/// This prevents interference from global goose configuration.
/// Uses .plan-forge/.goose/ in the current working directory.
fn setup_isolated_config() {
    // Only set if not already set (allows override for testing)
    if std::env::var("GOOSE_PATH_ROOT").is_err() {
        // Use .plan-forge/.goose/ in current working directory for project-local isolation
        let goose_base = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".plan-forge/.goose");

        // Create directories if they don't exist (including subdirs goose expects)
        let _ = std::fs::create_dir_all(goose_base.join("config"));
        let _ = std::fs::create_dir_all(goose_base.join("data/sessions"));
        let _ = std::fs::create_dir_all(goose_base.join("state/logs"));

        // Create minimal goose config to suppress "extensions not found" warning
        let config_file = goose_base.join("config/config.yaml");
        if !config_file.exists() {
            let _ = std::fs::write(&config_file, "extensions: {}\n");
        }

        // SAFETY: This is called early in main() before spawning any threads,
        // and before any goose code runs, so it's safe to modify env vars.
        unsafe {
            std::env::set_var("GOOSE_PATH_ROOT", &goose_base);
        }
    }
}

/// Plan-Forge CLI: Iterative plan generation with automated review
#[derive(Parser, Debug)]
#[command(name = "plan-forge")]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the plan-review loop for a task
    #[command(name = "run")]
    Run {
        #[command(flatten)]
        args: Box<RunArgs>,
    },

    /// Run an MCP server (developer or plan-forge)
    #[command(name = "mcp")]
    Mcp {
        #[arg(value_parser = clap::value_parser!(McpServer))]
        server: McpServer,

        /// Path to configuration file
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
}

#[derive(Parser, Debug)]
struct RunArgs {
    /// Task description (or feedback/context when used with --path <dir>)
    #[arg(short, long)]
    task: Option<String>,

    /// Path to task file or existing plan directory
    /// - File: read task description from file
    /// - Directory: resume from plan (--task becomes feedback)
    #[arg(short = 'p', long)]
    path: Option<PathBuf>,

    /// Working directory for the planning task
    #[arg(short, long)]
    working_dir: Option<PathBuf>,

    /// Path to configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Override planner model (e.g., "claude-opus-4-5-20251101")
    #[arg(long)]
    planner_model: Option<String>,

    /// Override reviewer model (e.g., "gpt-4o")
    #[arg(long)]
    reviewer_model: Option<String>,

    /// Override planner provider (e.g., "anthropic", "openai")
    #[arg(long)]
    planner_provider: Option<String>,

    /// Override reviewer provider
    #[arg(long)]
    reviewer_provider: Option<String>,

    /// Maximum iterations before giving up
    #[arg(long, default_value = "5")]
    max_iterations: u32,

    /// Output directory for final plan files
    #[arg(short, long, default_value = "./dev/active")]
    output: PathBuf,

    /// Explicit slug for directory/session naming (skips LLM generation)
    #[arg(long)]
    slug: Option<String>,

    /// Review pass threshold (0.0-1.0)
    #[arg(long, default_value = "0.8")]
    threshold: f32,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    // ============ Orchestrator Mode Flags ============

    /// Use deprecated LoopController instead of LLM-powered orchestrator
    #[arg(long)]
    use_legacy_loop: bool,

    /// Override orchestrator model (e.g., "claude-sonnet-4-20250514")
    #[arg(long)]
    orchestrator_model: Option<String>,

    /// Override orchestrator provider (e.g., "anthropic", "openai")
    #[arg(long)]
    orchestrator_provider: Option<String>,

    /// Maximum total tokens for orchestrator session (default: 500000, -1 for unlimited)
    #[arg(long, allow_negative_numbers = true)]
    max_total_tokens: Option<i64>,

    /// Session ID to resume (alternative to --path <dir> for orchestrator sessions)
    #[arg(long, value_name = "ID")]
    session_id: Option<String>,

    /// Feedback for resuming paused orchestrator sessions. Use natural language:
    /// - Answer questions: "Use JWT with 24h expiry"
    /// - Approve: "Looks good, proceed"
    /// - Request changes: "Please revise to use PostgreSQL"
    #[arg(long, value_name = "TEXT")]
    feedback: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Set up isolated config BEFORE any goose code runs
    setup_isolated_config();

    let cli = Cli::parse();

    match cli.command {
        Some(Command::Mcp { server, config }) => handle_mcp_command(server, config).await,
        Some(Command::Run { args }) => handle_run_command(*args).await,
        None => {
            // Default behavior: show help
            eprintln!("No command specified. Use --help for usage information.");
            eprintln!("Example: plan-forge run --task \"implement feature X\"");
            std::process::exit(1);
        }
    }
}

async fn handle_mcp_command(server: McpServer, config_path: Option<PathBuf>) -> Result<()> {
    // MCP servers should run with minimal logging to stderr
    // since they communicate via stdio
    match server {
        McpServer::Developer => serve(DeveloperServer::new()).await?,
        McpServer::PlanForge => {
            // Load config with env overrides if path provided
            let plan_forge_server = if let Some(path) = config_path {
                let config = CliConfig::load_with_env(Some(&path))?;
                PlanForgeServer::with_config(config)
            } else {
                // Auto-detect config and apply env overrides
                PlanForgeServer::new()
            };
            serve(plan_forge_server).await?
        }
    }
    Ok(())
}

/// Load the latest plan from a runs directory
fn load_latest_plan(runs_dir: &PathBuf) -> Result<(Plan, u32)> {
    // Find highest plan-iteration-N.json
    let mut highest_iteration = 0u32;
    let mut latest_plan_path = None;

    for entry in std::fs::read_dir(runs_dir)
        .context(format!("Failed to read runs directory: {:?}", runs_dir))?
    {
        let entry = entry?;
        let filename = entry.file_name();
        let filename_str = filename.to_string_lossy();

        if let Some(iter_str) = filename_str
            .strip_prefix("plan-iteration-")
            .and_then(|s| s.strip_suffix(".json"))
            && let Ok(iter) = iter_str.parse::<u32>()
            && iter > highest_iteration
        {
            highest_iteration = iter;
            latest_plan_path = Some(entry.path());
        }
    }

    let plan_path =
        latest_plan_path.ok_or_else(|| anyhow::anyhow!("No plan files found in {:?}", runs_dir))?;

    let plan_json = std::fs::read_to_string(&plan_path)
        .context(format!("Failed to read plan file: {:?}", plan_path))?;

    let plan: Plan = serde_json::from_str(&plan_json)
        .context(format!("Failed to parse plan JSON from {:?}", plan_path))?;

    info!(
        "Loaded plan from {:?} (iteration {})",
        plan_path, highest_iteration
    );
    Ok((plan, highest_iteration))
}

/// Resolve task, slug, and resume state from --path and --task args
fn resolve_input(args: &RunArgs) -> Result<(String, String, Option<ResumeState>)> {
    match &args.path {
        Some(path) if path.is_file() => {
            // Read task from file
            let file_content =
                std::fs::read_to_string(path).context(format!("Failed to read: {:?}", path))?;

            // Combine with --task if provided (as additional context)
            let task = match &args.task {
                Some(extra) => format!("{}\n\nAdditional context: {}", file_content.trim(), extra),
                None => file_content.trim().to_string(),
            };

            // Use first line for slug (usually the title)
            let slug = slugify(task.lines().next().unwrap_or(&task));
            Ok((task, slug, None))
        }

        Some(path) if path.is_dir() => {
            // Resume from plan directory
            let slug = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
                .ok_or_else(|| anyhow::anyhow!("Invalid directory name: {:?}", path))?;

            // Find session directory in .plan-forge/<slug>/
            let session_dir = std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".plan-forge")
                .join(&slug);

            // Check for orchestrator state first (new orchestrator mode)
            let state_file = session_dir.join("orchestration-state.json");
            if state_file.exists() {
                // Orchestrator session - load state to get original task
                let state = OrchestrationState::load(&session_dir)?
                    .ok_or_else(|| anyhow::anyhow!("Failed to load orchestration state from {:?}", session_dir))?;

                info!("Resuming orchestrator session from iteration {}", state.iteration);

                // Return original task - orchestrator handles resume internally
                // The --task or --feedback arg will be used as human response in orchestrator mode
                return Ok((state.task.clone(), slug, None));
            }

            // Fall back to legacy plan-iteration-N.json lookup
            let (plan, iteration) = load_latest_plan(&session_dir)?;

            // --task becomes feedback when resuming (legacy mode)
            let feedback = args
                .task
                .as_ref()
                .map(|f| vec![format!("[USER FEEDBACK] {}", f)])
                .unwrap_or_default();

            let task = args.task.clone().unwrap_or_else(|| plan.title.clone());

            let resume = ResumeState {
                plan,
                feedback,
                start_iteration: iteration + 1,
            };

            Ok((task, slug, Some(resume)))
        }

        Some(path) => {
            anyhow::bail!("Path does not exist: {:?}", path)
        }

        None => {
            // Original: require --task
            let task = args
                .task
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Either --task or --path is required"))?;
            let slug = slugify(&task);
            Ok((task, slug, None))
        }
    }
}

#[allow(deprecated)] // Legacy LoopController used in non-orchestrator path
async fn handle_run_command(args: RunArgs) -> Result<()> {
    // Set up logging
    let filter = if args.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();

    info!("Plan-Review CLI starting");

    // Resolve task, slug, and resume state from --path and --task
    let (task, resolved_slug, resume_state) = resolve_input(&args)?;

    // Use explicit --slug if provided, otherwise use resolved slug
    let task_slug = if let Some(explicit_slug) = &args.slug {
        info!("Using explicit slug: {}", explicit_slug);
        slugify(explicit_slug)
    } else {
        resolved_slug
    };

    info!("Task: {}", task);
    info!("Task slug: {}", task_slug);

    // Load configuration
    let mut config = CliConfig::load_with_env(args.config.as_ref())?;

    // Apply CLI overrides
    if let Some(model) = args.planner_model {
        config.planning.model_override = Some(model);
    }
    if let Some(provider) = args.planner_provider {
        config.planning.provider_override = Some(provider);
    }
    if let Some(model) = args.reviewer_model {
        config.review.model_override = Some(model);
    }
    if let Some(provider) = args.reviewer_provider {
        config.review.provider_override = Some(provider);
    }
    config.loop_config.max_iterations = args.max_iterations;
    config.review.pass_threshold = args.threshold;

    // Set up output directories using task slug (for legacy mode)
    // Uses .plan-forge/<slug>/ in current working directory for project-local storage
    // NOTE: Orchestrator mode generates its own slug and creates its directory later
    let runs_dir = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".plan-forge")
        .join(&task_slug);
    config.output.runs_dir = runs_dir.clone();
    config.output.active_dir = args.output.clone();
    config.output.slug = Some(task_slug.clone());

    // Determine base directory for recipes
    let base_dir = args
        .config
        .as_ref()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    // Branch based on orchestrator mode (default) vs legacy loop controller
    if !args.use_legacy_loop && config.use_orchestrator {
        // ============ Orchestrator Mode (default) ============
        info!("Using LLM-powered orchestrator mode");

        // Apply orchestrator-specific config overrides FIRST (before slug generation)
        if let Some(model) = args.orchestrator_model {
            config.orchestrator.model_override = Some(model);
        }
        if let Some(provider) = args.orchestrator_provider {
            config.orchestrator.provider_override = Some(provider);
        }
        if let Some(tokens) = args.max_total_tokens {
            // -1 (or any negative) means unlimited
            config.guardrails.max_total_tokens = if tokens < 0 {
                u64::MAX
            } else {
                tokens as u64
            };
        }

        // Use slug from resolve_input (handles resume correctly), or generate if needed
        let is_resuming = args.path.as_ref().map(|p| p.is_dir()).unwrap_or(false);
        let task_slug = if is_resuming {
            // When resuming, use the slug from the session directory (already resolved above)
            info!("Using resumed session slug: {}", task_slug);
            task_slug.clone()
        } else if let Some(explicit_slug) = &args.slug {
            info!("Using explicit slug: {}", explicit_slug);
            slugify(explicit_slug)
        } else if let Some((provider, model)) = config.slug_provider_model() {
            let slug = generate_slug(&task, &provider, &model).await;
            info!("LLM generated slug: {}", slug);
            slug
        } else {
            let slug = slugify_truncate(&task);
            info!("Fallback slug: {}", slug);
            slug
        };

        // Update runs_dir with the new slug
        let runs_dir = args
            .working_dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().expect("Failed to get current dir"))
            .join(".plan-forge")
            .join(&task_slug);
        std::fs::create_dir_all(&runs_dir)?;
        config.output.slug = Some(task_slug.clone());

        // Build human response from --feedback flag, or --task when resuming from directory
        // Natural language in feedback handles approve/revise/answer semantics
        let is_dir_resume = args.path.as_ref().map(|p| p.is_dir()).unwrap_or(false);
        let human_response = args.feedback
            .or_else(|| {
                // When resuming from a directory, --task can serve as feedback
                if is_dir_resume {
                    args.task.clone()
                } else {
                    None
                }
            })
            .map(|text| HumanResponse {
                response: text,
                approved: true, // Natural language in feedback handles intent
            });

        // Use explicit session_id if provided, otherwise derive from path
        let session_id = args.session_id.clone().or_else(|| {
            args.path.as_ref().and_then(|p| {
                if p.is_dir() {
                    p.file_name().and_then(|n| n.to_str()).map(String::from)
                } else {
                    None
                }
            })
        });

        // Create session registry for concurrent session management
        let session_registry = std::sync::Arc::new(SessionRegistry::new());

        // Create orchestrator
        let orchestrator = GooseOrchestrator::new(
            config.orchestrator.clone(),
            config.guardrails.clone(),
            base_dir.clone(),
            runs_dir.clone(),
            session_registry,
        );

        // Run orchestrator
        let working_dir_path = args.working_dir.clone();
        let result = orchestrator
            .run(task, working_dir_path, human_response, session_id)
            .await?;

        // Convert to LoopResult for consistent output
        let loop_result: plan_forge::LoopResult = result.into();
        print_result(loop_result)
    } else {
        // ============ Legacy Loop Controller Mode (DEPRECATED) ============
        tracing::warn!(
            "Using deprecated LoopController. Consider using orchestrator mode (default). \
             The --use-legacy-loop flag will be removed in a future version."
        );

        // Create runs directory for legacy mode (orchestrator mode creates its own above)
        std::fs::create_dir_all(&runs_dir)?;

        // Create components
        let planner = GoosePlanner::new(config.planning.clone(), base_dir.clone());
        let reviewer = GooseReviewer::new(config.review.clone(), base_dir);
        let output = FileOutputWriter::new(config.output.clone());

        // Create loop controller
        let mut controller =
            LoopController::new(planner, reviewer, output, config).with_task_slug(task_slug.clone());

        // Apply resume state if present
        if let Some(resume) = resume_state {
            if !resume.feedback.is_empty() {
                info!("Resuming with user feedback");
            } else {
                info!("Resuming without feedback (re-running review)");
            }
            controller = controller.with_resume(resume);
        }

        let working_dir = args.working_dir.map(|p| p.to_string_lossy().to_string());
        let result = controller.run(task, working_dir).await?;

        print_result(result)
    }
}

fn print_result(result: plan_forge::LoopResult) -> Result<()> {
    println!("\n========================================");
    println!("Plan-Review Complete!");
    println!("========================================");
    println!("Total iterations: {}", result.total_iterations);
    println!(
        "Final status: {}",
        if result.success {
            "PASSED"
        } else {
            "NEEDS REVISION"
        }
    );
    println!("Final score: {:.2}", result.final_review.llm_review.score);
    println!("Plan title: {}", result.final_plan.title);
    println!("\nReview summary: {}", result.final_review.summary);

    if !result.success {
        println!(
            "\n⚠️  Plan did not pass review after {} iterations",
            result.total_iterations
        );
        println!("Review the output files for details on remaining issues.");
        std::process::exit(1);
    }

    Ok(())
}
