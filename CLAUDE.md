# CLAUDE.md

Quick reference for Claude Code when working with this repository. For detailed documentation, see [README.md](README.md).

## Build and Development Commands

```bash
# Build the project
cargo build

# Run with task string
cargo run -- run --task "your task description"

# Run with task from file
cargo run -- run --path requirements.md

# Task from file with additional context
cargo run -- run --path requirements.md --task "Focus on security"

# Resume from existing plan directory
cargo run -- run --path dev/active/my-task-slug/

# Resume with feedback
cargo run -- run --path dev/active/my-task-slug/ --task "also add error handling"

# Run with verbose logging
cargo run -- run --task "your task" --verbose

# Run with LLM-powered orchestrator (experimental)
cargo run -- run --task "your task" --use-orchestrator

# Orchestrator with token budget limit
cargo run -- run --task "your task" --use-orchestrator --max-total-tokens 100000

# Run tests
cargo test

# Check for errors without building
cargo check

# Format code
cargo fmt

# Run linter
cargo clippy
```

See [README.md#cli-options](README.md#cli-options) for full CLI reference. Key paths:
- Session files: `.plan-forge/<task-slug>/`
- Output files: `./dev/active/<task-slug>/`

### MCP Server

```bash
# Run plan-forge MCP server
plan-forge mcp plan-forge

# Run developer tools server (from goose)
plan-forge mcp developer
```

Available tools via MCP: `plan_run`, `plan_status`, `plan_list`, `plan_get`, `plan_approve`

**Using LiteLLM with MCP server:**

```bash
export LITELLM_HOST=http://localhost:4000
export LITELLM_API_KEY=sk-xxx
export PLAN_FORGE_PLANNER_PROVIDER=litellm
export PLAN_FORGE_PLANNER_MODEL=claude-opus-4.5
export PLAN_FORGE_REVIEWER_PROVIDER=litellm
export PLAN_FORGE_REVIEWER_MODEL=claude-opus-4.5

plan-forge mcp plan-forge
```

## Architecture

This is a Rust CLI tool that uses the `goose` crate to run an iterative plan-review feedback loop. It generates development plans using an LLM, reviews them, and refines based on feedback.

### Core Flow

```text
Task → Planner (LLM) → Plan → Reviewer (LLM) → ReviewResult
                         ↑                          ↓
                         └── Update with feedback ←─┘
```

The `LoopController` (legacy) or `GooseOrchestrator` (with `--use-orchestrator`) orchestrates this cycle until either:
- The plan passes review (score >= threshold with no hard check failures)
- Maximum iterations reached
- Perfect score achieved (early exit if enabled)
- Human input required (reviewer flags security concern or ambiguity)
- Hard stop triggered (token budget, timeout)

### Key Components

- **GooseOrchestrator** (`src/phases/orchestrator.rs`): LLM-powered orchestrator with guardrails. Uses goose Agent with in-process MCP extension pattern for dynamic workflow decisions.
- **LoopController** (`src/orchestrator/loop_controller.rs`): [Deprecated] Legacy deterministic loop controller
- **OrchestratorClient** (`src/orchestrator/client.rs`): MCP client implementing tool handlers for plan generation, review, and human input. Registered via ExtensionManager.
- **Guardrails** (`src/orchestrator/guardrails.rs`): Enforces 6 mandatory conditions requiring human approval plus hard stops for token budget/iterations/timeout
- **Planner trait** (`src/phases/mod.rs`): Interface for plan generation. `GoosePlanner` uses goose Agent with recipes
- **Reviewer trait** (`src/phases/mod.rs`): Interface for plan review. `GooseReviewer` validates plans and produces structured feedback
- **Plan model** (`src/models/plan.rs`): Structured plan with phases, checkpoints, tasks, acceptance criteria, file references, and risks
- **ReviewResult model** (`src/models/review.rs`): Contains hard check results, LLM review (gaps, unclear areas, suggestions, score)

### Recipe System

Recipes (`planner.yaml`, `reviewer.yaml`) define LLM behavior. See [README.md#recipe-customization](README.md#recipe-customization).

### Configuration

- Default config in `config/default.yaml`
- `CliConfig` in `src/config/settings.rs` defines all configuration options
- Supports provider/model overrides, loop settings, output options

## Dependencies

This project uses the Block goose crates from GitHub:
- `goose` and `goose-mcp` from https://github.com/block/goose (v1.19.0)
- No local checkout required - dependencies are fetched via git
