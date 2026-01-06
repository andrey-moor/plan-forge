# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

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

# Run tests
cargo test

# Check for errors without building
cargo check

# Format code
cargo fmt

# Run linter
cargo clippy
```

### CLI Options

- `--task, -t`: Task description (or feedback when used with --path <dir>)
- `--path, -p`: Path to task file or existing plan directory
  - If file: read task from file (--task becomes additional context)
  - If directory: resume from plan (--task becomes feedback)
- `--working-dir, -w`: Working directory for the planning task
- `--config, -c`: Path to configuration file
- `--planner-model`, `--reviewer-model`: Override LLM models
- `--planner-provider`, `--reviewer-provider`: Override providers (e.g., "anthropic", "openai")
- `--max-iterations`: Maximum iterations before giving up (default: 5)
- `--output, -o`: Output directory for plan files (default: ./dev/active)
- `--threshold`: Review pass threshold 0.0-1.0 (default: 0.8)

### Output Structure

- **Intermediate files**: `~/.config/plan-forge/runs/<task-slug>/` (JSON files, not committed)
- **Final files**: `./dev/active/<task-slug>/` (markdown files, committed to repo)
  - `<task-slug>-plan.md`: Overview and phases
  - `<task-slug>-tasks.md`: Detailed task list
  - `<task-slug>-context.md`: Context for handoff

## Architecture

This is a Rust CLI tool that uses the `goose` crate to run an iterative plan-review feedback loop. It generates development plans using an LLM, reviews them, and refines based on feedback.

### Core Flow

```
Task → Planner (LLM) → Plan → Reviewer (LLM) → ReviewResult
                         ↑                          ↓
                         └── Update with feedback ←─┘
```

The `LoopController` orchestrates this cycle until either:
- The plan passes review (score >= threshold with no hard check failures)
- Maximum iterations reached
- Perfect score achieved (early exit if enabled)
- Human input required (reviewer flags security concern or ambiguity)

### Resume Workflow

When a plan needs user input, or when you want to provide feedback after reviewing a generated plan:

```bash
# Initial run generates plan
cargo run -- run --task "Add user authentication"

# Plan is saved to dev/active/add-user-authentication/
# Review the plan, then resume with feedback
cargo run -- run --path dev/active/add-user-authentication/ \
  --task "Use JWT tokens, not session cookies"
```

The `--path <directory>` feature:
1. Derives task slug from directory name
2. Loads latest plan from `runs/<task-slug>/`
3. Treats `--task` as user feedback
4. Planner updates the plan based on feedback
5. Review loop continues until passing

### Key Components

- **LoopController** (`src/orchestrator/loop_controller.rs`): Manages the plan-review-update loop, tracks state, handles iteration logic
- **Planner trait** (`src/phases/mod.rs`): Interface for plan generation. `GoosePlanner` uses goose Agent with recipes
- **Reviewer trait** (`src/phases/mod.rs`): Interface for plan review. `GooseReviewer` validates plans and produces structured feedback
- **Plan model** (`src/models/plan.rs`): Structured plan with phases, checkpoints, tasks, acceptance criteria, file references, and risks
- **ReviewResult model** (`src/models/review.rs`): Contains hard check results, LLM review (gaps, unclear areas, suggestions, score)

### Recipe System

Recipes in `recipes/` define LLM agent behavior:
- `planner.yaml`: Instructions for generating plans, uses goose extensions for codebase exploration
- `reviewer.yaml`: Instructions for reviewing plans, scoring guidelines (0.9-1.0 excellent, 0.0-0.5 poor)

Recipes specify provider/model defaults (anthropic/claude-opus-4-5-20251101) which can be overridden via CLI or config.

### Configuration

- Default config in `config/default.yaml`
- `CliConfig` in `src/config/settings.rs` defines all configuration options
- Supports provider/model overrides, loop settings, output options

## Dependencies

This project uses the Block goose crates from GitHub:
- `goose` and `goose-mcp` from https://github.com/block/goose (v1.19.0)
- No local checkout required - dependencies are fetched via git
