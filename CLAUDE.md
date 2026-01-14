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

# Orchestrator with token budget limit
cargo run -- run --task "your task" --max-total-tokens 100000

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

**Using Microsoft Foundry with MCP server:**

```bash
export MICROSOFT_FOUNDRY_RESOURCE=foundry-myresource
export MICROSOFT_FOUNDRY_API_KEY=your-key
export PLAN_FORGE_ORCHESTRATOR_PROVIDER=microsoft_foundry
export PLAN_FORGE_ORCHESTRATOR_MODEL=claude-opus-4-5
export PLAN_FORGE_PLANNER_PROVIDER=microsoft_foundry
export PLAN_FORGE_PLANNER_MODEL=claude-opus-4-5
export PLAN_FORGE_REVIEWER_PROVIDER=microsoft_foundry
export PLAN_FORGE_REVIEWER_MODEL=claude-opus-4-5

plan-forge mcp plan-forge
```

## Architecture

This is a Rust CLI tool that uses the `goose` crate to run an iterative plan-review feedback loop. It generates development plans using an LLM, reviews them, and refines based on feedback.

### Component Responsibilities

| Component | Responsibility | LLM? |
|-----------|---------------|------|
| **GooseOrchestrator** | Coordination via tool calls | Yes (agent) |
| **GoosePlanner** | Generate development plans | Yes (agent) |
| **GooseReviewer** | Review plans (Q-* quality checks) | Yes (agent) |
| **ViabilityChecker** | Structural validation (V-* checks) | No |
| **Guardrails** | Numeric limits (tokens, iterations) | No |
| **OrchestratorClient** | MCP tools for orchestrator agent | No |

### Data Flow

```text
Orchestrator Agent
    │
    ├─► generate_plan tool
    │       └─► GoosePlanner → plan JSON
    │
    └─► review_plan tool
            ├─► ViabilityChecker → V-* violations (deterministic)
            │       (if V-* fails, skip LLM review)
            ├─► GooseReviewer → Q-* quality, score (LLM-based)
            └─► Guardrails → hard stops (tokens, iterations, timeout)
```

### Design Principles

1. **Planner = Pure Generation**: No policy checks in planner
2. **Reviewer = All Policy**: V-* (structural) + Q-* (semantic) checks
3. **Orchestrator = Coordination**: Calls tools, makes decisions, no checks
4. **Fail-Fast in Review**: V-* checks run before expensive LLM review

The `GooseOrchestrator` (default) orchestrates this cycle until either:
- The plan passes review (viability + quality checks pass)
- Maximum iterations reached
- Human input required (reviewer flags security concern or ambiguity)
- Hard stop triggered (token budget, timeout)

### Key Components

- **GooseOrchestrator** (`src/phases/orchestrator.rs`): LLM-powered orchestrator with guardrails. Uses goose Agent with in-process MCP extension pattern for dynamic workflow decisions.
- **OrchestratorClient** (`src/orchestrator/client.rs`): MCP client implementing tool handlers for plan generation, review, and human input. Registered via ExtensionManager.
- **ViabilityChecker** (`src/orchestrator/viability/`): Deterministic V-* checks (V-001 to V-014) for instruction structure validation. Split into focused modules:
  - `mod.rs`: Main checker and `check_all` entry point
  - `types.rs`: Core types (ViabilityViolation, ViabilityResult)
  - `dag.rs`: V-001, V-002 (dependency/cycle validation)
  - `instruction.rs`: V-004, V-005, V-009, V-013, V-014 (instruction params)
  - `dataflow.rs`: V-006, V-007, V-008 (variable refs, TDD order)
  - `grounding.rs`: V-003, V-011 (file existence, context order)
  - `metrics.rs`: V-010, V-012 (DAG analysis, token estimates)
- **Guardrails** (`src/orchestrator/guardrails.rs`): Enforces hard stops for token budget, max iterations, and execution timeout. Score threshold checked deterministically.
- **Planner trait** (`src/phases/mod.rs`): Interface for plan generation. `GoosePlanner` uses goose Agent with recipes
- **Reviewer trait** (`src/phases/mod.rs`): Interface for plan review. `GooseReviewer` validates plans and produces structured feedback
- **Plan model** (`src/models/plan.rs`): Structured plan with phases, checkpoints, tasks, acceptance criteria, file references, and risks
- **ReviewResult model** (`src/models/review.rs`): Contains hard check results, LLM review (gaps, unclear areas, suggestions, score)

### Recipe System

Recipes (`planner.yaml`, `reviewer.yaml`) define LLM behavior. See [README.md#recipe-customization](README.md#recipe-customization).

### Context Engineering for LLM Prompts

When writing or modifying LLM prompts/recipes, follow these Anthropic best practices:

**1. Use XML Tags for Structure**
```xml
<critical-constraints>
Most important rules that MUST be followed
</critical-constraints>

<examples>
Concrete examples showing correct patterns
</examples>

<final-checklist>
Reminder of key requirements at the end
</final-checklist>
```

**2. Front-Load Critical Constraints (Primacy)**
Place the most important requirements in the FIRST 50 lines. Claude has strong primacy effects - information at the start is retained better.

**3. Repeat Critical Info at End (Recency)**
Add a `<final-checklist>` or `<reminder>` section at the end repeating the most violated constraints.

**4. Use Positive Instructions**
- **DO**: "Add `goal` param to every EDIT_CODE"
- **DON'T**: "WRONG: `{ "action": "create" }`"

Lead with correct patterns. Minimize "WRONG" examples.

**5. Examples Immediately After Schema**
When defining a schema or format, provide a complete example within 10 lines.

**6. Concise Reference Tables**
Replace verbose explanations with scannable tables:

| Check | Severity | Requirement |
|-------|----------|-------------|
| V-013 | Critical | EDIT_CODE/GENERATE_TEST need `goal` |

### Configuration

- Default config in `config/default.yaml`
- `CliConfig` in `src/config/settings.rs` defines all configuration options
- Supports provider/model overrides, loop settings, output options

## Dependencies

This project uses a forked goose with Microsoft Foundry provider support:
- `goose` and `goose-mcp` from https://github.com/andrey-moor/goose (branch: `feat/microsoft-foundry-provider`)
- Based on Block's goose (https://github.com/block/goose)
- No local checkout required - dependencies are fetched via git
