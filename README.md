# plan-forge

A CLI tool for iterative AI-driven development planning. Uses LLM agents to generate comprehensive development plans, review them for quality, and refine based on feedback.

[![CI](https://github.com/andrey-moor/plan-forge/actions/workflows/ci.yml/badge.svg)](https://github.com/andrey-moor/plan-forge/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

## Features

- **Iterative Plan-Review Loop**: Generates plans, reviews them for gaps and clarity, and refines until quality threshold is met
- **Multiple LLM Providers**: Supports Anthropic, OpenAI, LiteLLM, and other providers via [goose](https://github.com/block/goose)
- **Customizable Recipes**: Configure prompts, models, and MCP server extensions via YAML files
- **Structured Output**: Plans are validated against JSON schemas and exported as markdown
- **Resume Workflow**: Pick up where you left off with feedback-driven refinement

## Quick Start

### Prerequisites

- Rust 1.91+ (uses edition 2024)
- An API key for your chosen LLM provider

### Installation

```bash
# Clone the repository
git clone https://github.com/andrey-moor/plan-forge.git
cd plan-forge

# Build
cargo build --release

# Or install directly
cargo install --path .
```

### First Run

```bash
# Set your API key
export ANTHROPIC_API_KEY="your-key-here"

# Generate a plan
cargo run -- run --task "Add user authentication to the web app"
```

## Usage

### Basic Commands

```bash
# Generate a plan from a task description
cargo run -- run --task "your task description"

# Read task from a file
cargo run -- run --path requirements.md

# Task from file with additional context
cargo run -- run --path requirements.md --task "Focus on security aspects"

# Resume from an existing plan with feedback
cargo run -- run --path dev/active/my-task/ --task "Use JWT instead of sessions"

# Verbose logging
cargo run -- run --task "your task" --verbose
```

### CLI Options

| Option | Short | Description |
|--------|-------|-------------|
| `--task` | `-t` | Task description (or feedback when resuming) |
| `--path` | `-p` | Path to task file or existing plan directory |
| `--working-dir` | `-w` | Working directory for the planning task |
| `--config` | `-c` | Path to configuration file |
| `--planner-model` | | Override LLM model for planning |
| `--reviewer-model` | | Override LLM model for review |
| `--planner-provider` | | Override provider (anthropic, openai, litellm) |
| `--reviewer-provider` | | Override provider for review |
| `--max-iterations` | | Maximum iterations before giving up (default: 5) |
| `--output` | `-o` | Output directory for plan files (default: ./dev/active) |
| `--threshold` | | Review pass threshold 0.0-1.0 (default: 0.8) |
| `--verbose` | `-v` | Enable debug logging |

## Configuration

### Environment Variables

| Variable | Provider | Required |
|----------|----------|----------|
| `ANTHROPIC_API_KEY` | Anthropic | Yes (if using Anthropic) |
| `OPENAI_API_KEY` | OpenAI | Yes (if using OpenAI) |
| `LITELLM_HOST` | LiteLLM | Yes (if using LiteLLM) |
| `LITELLM_API_KEY` | LiteLLM | Yes (if using LiteLLM) |

### Config File

Create a `config.yaml` or use the default at `config/default.yaml`:

```yaml
planning:
  recipe: recipes/planner.yaml
  provider_override: null    # Override provider from recipe
  model_override: null       # Override model from recipe

review:
  recipe: recipes/reviewer.yaml
  provider_override: null
  model_override: null
  pass_threshold: 0.8        # Score needed to pass review (0.0-1.0)

loop_config:
  max_iterations: 5          # Max plan-review cycles
  early_exit_on_perfect_score: true

output:
  runs_dir: ./runs           # Intermediate JSON files
  active_dir: ./dev/active   # Final markdown output
```

### Recipe Customization

Recipes define LLM agent behavior. Located in `recipes/`:

```yaml
version: "1.0.0"
title: "Strategic Planner"
description: "Generates comprehensive development plans"

# System prompt for the LLM
instructions: |
  You are an elite strategic planning specialist...

  ## Planning Process
  1. Understand the Task
  2. Explore the Codebase
  3. Identify Patterns
  ...

# MCP server extensions
extensions:
  - name: developer
    type: builtin
    description: "Developer tools for file operations"
    timeout: 300
  - name: context7
    type: stdio
    cmd: npx
    args: ["-y", "@upstash/context7-mcp@latest"]
    description: "Up-to-date documentation for libraries"
    timeout: 60

# Default provider and model
settings:
  goose_provider: anthropic
  goose_model: claude-opus-4-5-20251101

# Output schema (JSON Schema)
response:
  json_schema:
    type: object
    properties:
      title: { type: string }
      phases: { type: array, ... }
      ...
```

## Providers

### Anthropic (Default)

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
cargo run -- run --task "your task"
```

Or override in recipe:
```yaml
settings:
  goose_provider: anthropic
  goose_model: claude-opus-4-5-20251101
```

### OpenAI

```bash
export OPENAI_API_KEY="sk-..."
cargo run -- run --task "your task" --planner-provider openai --planner-model gpt-4o
```

### LiteLLM

LiteLLM allows you to use various model providers through a unified proxy. This is useful for accessing models via GitHub Copilot, Azure, or other backends.

**Setup with Docker:**

```bash
# Run litellm proxy
docker run -d \
  -p 4000:4000 \
  -v /path/to/config.yaml:/app/config.yaml:ro \
  ghcr.io/berriai/litellm:main-latest

# Example litellm config.yaml for GitHub Copilot
general_settings:
  master_key: sk-

model_list:
  - model_name: '*'
    litellm_params:
      model: github_copilot/*
      extra_headers:
        Editor-Version: vscode/1.100.0
```

**Environment variables:**

```bash
export LITELLM_HOST=http://localhost:4000
export LITELLM_API_KEY=sk-
```

**Recipe configuration:**

```yaml
settings:
  goose_provider: litellm
  goose_model: claude-opus-4.5  # Note: dot notation, not dashes
```

**Available models via GitHub Copilot:**
- `claude-opus-4.5`, `claude-sonnet-4.5`, `claude-sonnet-4`
- `gpt-4o`, `gpt-4-turbo`
- And other models supported by your Copilot subscription

## MCP Extensions

[Model Context Protocol (MCP)](https://modelcontextprotocol.io/) servers provide tools to the LLM agent.

### Built-in Extensions

```yaml
extensions:
  - name: developer
    type: builtin
    description: "File operations, shell commands, codebase exploration"
    timeout: 300
```

### External Extensions (via stdio)

```yaml
extensions:
  # Documentation lookup
  - name: context7
    type: stdio
    cmd: npx
    args: ["-y", "@upstash/context7-mcp@latest"]
    timeout: 60

  # Custom MCP server
  - name: my-tools
    type: stdio
    cmd: /path/to/mcp-server
    args: ["--config", "server.json"]
    timeout: 120
```

### Popular MCP Servers

- **context7**: Up-to-date library documentation (`npx -y @upstash/context7-mcp@latest`)
- **filesystem**: File system operations
- **github**: GitHub API integration
- **postgres/sqlite**: Database access

See [MCP Servers](https://github.com/modelcontextprotocol/servers) for more options.

## Architecture

```
Task Description
       │
       ▼
┌─────────────────┐
│    Planner      │  ← Recipe: planner.yaml
│  (LLM Agent)    │  ← Extensions: developer, context7
└────────┬────────┘
         │ Plan (JSON)
         ▼
┌─────────────────┐
│    Reviewer     │  ← Recipe: reviewer.yaml
│  (LLM Agent)    │  ← Hard checks + LLM review
└────────┬────────┘
         │ ReviewResult
         ▼
    ┌────┴────┐
    │ Passed? │
    └────┬────┘
    No   │   Yes
    │    │    │
    ▼    │    ▼
 Update  │  Output
  Plan   │  Markdown
    │    │
    └────┘
```

### Core Flow

1. **Planner** generates a structured plan using the LLM with codebase exploration tools
2. **Hard Checks** validate plan structure (has phases, tasks, acceptance criteria)
3. **Reviewer** evaluates gaps, clarity, and feasibility via LLM
4. **Loop Controller** decides: refine (score < threshold) or accept (score >= threshold)
5. **Output** writes final markdown files to `./dev/active/<task-slug>/`

### Output Structure

**Intermediate files** (JSON, not committed):
```
~/.config/plan-forge/runs/<task-slug>/
├── plan-iteration-1.json
├── plan-iteration-2.json
├── review-iteration-1.json
└── review-iteration-2.json
```

**Final output** (Markdown, committed):
```
./dev/active/<task-slug>/
├── <task-slug>-plan.md      # Overview, phases, risks
├── <task-slug>-tasks.md     # Detailed task breakdown
└── <task-slug>-context.md   # Context for handoff
```

## Development

```bash
# Build
cargo build

# Run tests
cargo test

# Check for errors
cargo check

# Format code
cargo fmt

# Run linter
cargo clippy
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on:
- Development setup
- Code style
- Pull request process
- Issue reporting

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

Built on [goose](https://github.com/block/goose) by Block, Inc.
