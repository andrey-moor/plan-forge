use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use plan_forge::{
    slugify, CliConfig, FileOutputWriter, GoosePlanner, GooseReviewer, LoopController, Plan,
    PlanForgeServer, ResumeState,
};

// Re-export MCP server types from goose-mcp
use goose_mcp::mcp_server_runner::serve;
use goose_mcp::DeveloperServer;

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
        args: RunArgs,
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

    /// Review pass threshold (0.0-1.0)
    #[arg(long, default_value = "0.8")]
    threshold: f32,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Set up isolated config BEFORE any goose code runs
    setup_isolated_config();

    let cli = Cli::parse();

    match cli.command {
        Some(Command::Mcp { server, config }) => handle_mcp_command(server, config).await,
        Some(Command::Run { args }) => handle_run_command(args).await,
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
        {
            if let Ok(iter) = iter_str.parse::<u32>() {
                if iter > highest_iteration {
                    highest_iteration = iter;
                    latest_plan_path = Some(entry.path());
                }
            }
        }
    }

    let plan_path = latest_plan_path
        .ok_or_else(|| anyhow::anyhow!("No plan files found in {:?}", runs_dir))?;

    let plan_json = std::fs::read_to_string(&plan_path)
        .context(format!("Failed to read plan file: {:?}", plan_path))?;

    let plan: Plan = serde_json::from_str(&plan_json)
        .context(format!("Failed to parse plan JSON from {:?}", plan_path))?;

    info!("Loaded plan from {:?} (iteration {})", plan_path, highest_iteration);
    Ok((plan, highest_iteration))
}

/// Resolve task, slug, and resume state from --path and --task args
fn resolve_input(args: &RunArgs) -> Result<(String, String, Option<ResumeState>)> {
    match &args.path {
        Some(path) if path.is_file() => {
            // Read task from file
            let file_content = std::fs::read_to_string(path)
                .context(format!("Failed to read: {:?}", path))?;

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

            // Find runs directory and load plan from .plan-forge/<slug>/
            let runs_dir = std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".plan-forge")
                .join(&slug);

            let (plan, iteration) = load_latest_plan(&runs_dir)?;

            // --task becomes feedback when resuming
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
    let (task, task_slug, resume_state) = resolve_input(&args)?;

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

    // Set up output directories using task slug
    // Uses .plan-forge/<slug>/ in current working directory for project-local storage
    let runs_dir = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".plan-forge")
        .join(&task_slug);
    config.output.runs_dir = runs_dir.clone();
    config.output.active_dir = args.output;
    config.output.slug = Some(task_slug.clone());

    // Create runs directory if it doesn't exist
    std::fs::create_dir_all(&runs_dir)?;

    // Determine base directory for recipes
    let base_dir = args
        .config
        .as_ref()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    // Create components
    let planner = GoosePlanner::new(config.planning.clone(), base_dir.clone());
    let reviewer = GooseReviewer::new(config.review.clone(), base_dir);
    let output = FileOutputWriter::new(config.output.clone());

    // Create loop controller
    let mut controller = LoopController::new(planner, reviewer, output, config)
        .with_task_slug(task_slug.clone());

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

fn print_result(result: plan_forge::LoopResult) -> Result<()> {
    println!("\n========================================");
    println!("Plan-Review Complete!");
    println!("========================================");
    println!("Total iterations: {}", result.total_iterations);
    println!("Final status: {}", if result.success { "PASSED" } else { "NEEDS REVISION" });
    println!("Final score: {:.2}", result.final_review.llm_review.score);
    println!("Plan title: {}", result.final_plan.title);
    println!("\nReview summary: {}", result.final_review.summary);

    if !result.success {
        println!("\n⚠️  Plan did not pass review after {} iterations", result.total_iterations);
        println!("Review the output files for details on remaining issues.");
        std::process::exit(1);
    }

    Ok(())
}
