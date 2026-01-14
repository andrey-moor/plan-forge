# Plan-Forge Claude Code Integration

This directory contains integration files for using plan-forge with Claude Code.

## Quick Setup

### 1. Install plan-forge

```bash
# Clone and build
git clone https://github.com/andrey-moor/plan-forge
cd plan-forge
cargo build --release

# Add to PATH (or copy binary to a location in PATH)
export PATH="$PATH:$(pwd)/target/release"
```

### 2. Configure MCP Server

Add the following to your project's `.mcp.json` file (create it in your project root):

```json
{
  "mcpServers": {
    "plan-forge": {
      "command": "plan-forge",
      "args": ["mcp", "plan-forge"]
    }
  }
}
```

Or add to your global Claude Code MCP configuration.

### 3. Install Agents, Commands, and Skills (Optional)

Copy the integration files to your Claude Code configuration:

```bash
# Create directories
mkdir -p ~/.claude/agents ~/.claude/commands ~/.claude/skills

# Copy agents
cp integrations/claude-code/agents/*.md ~/.claude/agents/

# Copy slash commands
cp integrations/claude-code/commands/*.md ~/.claude/commands/

# Copy skills
cp -r integrations/claude-code/skills/* ~/.claude/skills/
```

Or for project-specific use, copy to `.claude/` in your project root.

### 4. Enable Background Agent Permissions (Recommended)

For the `/plan` command and skill to work without manual approval prompts, add the MCP tools to your allowed permissions in `.claude/settings.local.json`:

```json
{
  "permissions": {
    "allow": [
      "mcp__plan-forge__plan_run",
      "mcp__plan-forge__plan_status",
      "mcp__plan-forge__plan_list",
      "mcp__plan-forge__plan_get",
      "mcp__plan-forge__plan_approve"
    ]
  }
}
```

This enables background agents to call plan-forge tools autonomously, which is required for the async planning workflow.

## Available Tools

The plan-forge MCP server exposes 5 tools:

| Tool | Description |
|------|-------------|
| `plan_run` | Create or resume a planning session. Use `feedback` param when resuming paused sessions. |
| `plan_status` | Get session status (ready/in_progress/needs_input/approved/max_turns) |
| `plan_list` | List all planning sessions |
| `plan_get` | Read plan, tasks, or context markdown files |
| `plan_approve` | Force approve a plan and write to plans/active/ |

### plan_run Parameters

| Parameter | Required | Description |
|-----------|----------|-------------|
| `task` | Yes | Task description (for new) or feedback text (for resume) |
| `session_id` | No | Session ID to resume |
| `feedback` | No | Natural language feedback for resuming paused sessions |

**Feedback examples:**
- Answer questions: `"Use JWT with 24h expiry"`
- Approve: `"Looks good, proceed"`
- Request changes: `"Please revise to use PostgreSQL"`

## Slash Command: /plan

The `/plan` command provides explicit control over planning sessions:

```bash
/plan <task>              # Start new background planning session
/plan status [session]    # Check planning progress
/plan list                # List all sessions
/plan get <file>          # Read plan|tasks|context file
/plan approve             # Force approve current plan
/plan resume <id> <msg>   # Resume with feedback
```

### Example Workflow

```
User: /plan Add user authentication with OAuth

Claude: Started planning session for: "Add user authentication with OAuth"
        Session ID: add-user-authentication-with-oauth
        Use `/plan status` to check progress.

[planning runs in background while you continue working]

User: /plan status

Claude: Session: add-user-authentication-with-oauth
        Status: approved
        Iterations: 2
        Score: 0.85
        Plan saved to: plans/active/add-user-authentication-with-oauth/

User: /plan get plan

Claude: [displays the generated plan]
```

## Auto-Detect Skill

The plan skill automatically detects planning requests:

- "plan <task> in the background"
- "create a development plan for <task>"
- "I need a plan for <task>"

When triggered, it spawns a background agent and returns immediately.

## Background Agent Pattern

Both the `/plan` command and skill use Claude Code's background agent feature:

```
User: "Plan adding OAuth support"

Claude:
1. Spawns plan-forge agent with run_in_background=true
2. Returns immediately with session info
3. User can continue conversation
4. Check progress with /plan status
```

This prevents blocking the conversation during planning (30 seconds to several minutes).

## File Structure

Plan-forge creates files in two locations:

```
.plan-forge/
  <session-slug>/           # Session data (intermediate files)
    plan-iteration-1.json
    review-iteration-1.json
    ...
  .goose/                   # Goose isolation (gitignore this)

./plans/active/<slug>/        # Final output (commit to repo)
  <slug>-plan.md
  <slug>-tasks.md
  <slug>-context.md
```

## Session Status Values

| Status | Meaning |
|--------|---------|
| `ready` | Session created but no planning started |
| `in_progress` | Planning loop is running |
| `needs_input` | Reviewer flagged need for human input |
| `approved` | Plan passed review (score >= threshold) |
| `best_effort` | Completed but did not pass threshold |
| `max_turns` | Max iterations reached without approval |
| `hard_stopped` | Hit a hard limit (tokens, timeout, etc.) |

## Configuration

The MCP server can be configured via config file, environment variables, or MCP command arguments.

### Config File

Config files are auto-detected in priority order:
1. `.plan-forge/config.yaml` (recommended)
2. `plan-forge.yaml`
3. `.plan-forge.yaml`
4. `config/default.yaml`

Or specify explicitly via `--config`:

```json
{
  "mcpServers": {
    "plan-forge": {
      "command": "plan-forge",
      "args": ["mcp", "plan-forge", "--config", "./my-config.yaml"]
    }
  }
}
```

Example config file (`.plan-forge/config.yaml`):

```yaml
planning:
  recipe: recipes/planner.yaml
  provider_override: null
  model_override: null

review:
  recipe: recipes/reviewer.yaml
  provider_override: null
  model_override: null

orchestrator:
  recipe: recipes/orchestrator.yaml
  provider_override: null
  model_override: null

guardrails:
  max_iterations: 10
  score_threshold: 0.8
  max_total_tokens: 500000

output:
  runs_dir: ./.plan-forge
  active_dir: ./plans/active
```

**Using LiteLLM proxy:**

```yaml
planning:
  provider_override: litellm
  model_override: claude-opus-4.5

review:
  provider_override: litellm
  model_override: claude-sonnet-4
  pass_threshold: 0.8
```

With environment variables:
```bash
export LITELLM_HOST=http://localhost:4000
export LITELLM_API_KEY=sk-your-key
```

**Claude Code MCP integration with LiteLLM:**

When configuring plan-forge as an MCP server in Claude Code, pass LiteLLM credentials via environment variables in `.mcp.json`:

```json
{
  "mcpServers": {
    "plan-forge": {
      "command": "plan-forge",
      "args": ["mcp", "plan-forge"],
      "env": {
        "LITELLM_HOST": "http://localhost:4000",
        "LITELLM_API_KEY": "sk-xxx",
        "PLAN_FORGE_PLANNER_PROVIDER": "litellm",
        "PLAN_FORGE_PLANNER_MODEL": "claude-opus-4.5",
        "PLAN_FORGE_REVIEWER_PROVIDER": "litellm",
        "PLAN_FORGE_REVIEWER_MODEL": "claude-opus-4.5"
      }
    }
  }
}
```

This passes the LiteLLM configuration to the MCP server process.

### Environment Variables

Environment variables override config file values:

| Variable | Description | Default |
|----------|-------------|---------|
| `PLAN_FORGE_THRESHOLD` | Review pass threshold (0.0-1.0) | 0.8 |
| `PLAN_FORGE_MAX_ITERATIONS` | Maximum planning iterations | 10 |
| `PLAN_FORGE_MAX_TOTAL_TOKENS` | Maximum total tokens for session | 500000 |
| `PLAN_FORGE_PLANNER_PROVIDER` | Override planner provider | - |
| `PLAN_FORGE_PLANNER_MODEL` | Override planner model | - |
| `PLAN_FORGE_REVIEWER_PROVIDER` | Override reviewer provider | - |
| `PLAN_FORGE_REVIEWER_MODEL` | Override reviewer model | - |
| `PLAN_FORGE_ORCHESTRATOR_PROVIDER` | Override orchestrator provider | - |
| `PLAN_FORGE_ORCHESTRATOR_MODEL` | Override orchestrator model | - |
| `PLAN_FORGE_RECIPE_DIR` | Directory to search for recipes | - |
| `PLAN_FORGE_PLAN_DIR` | Output directory for plan files | plans/active |

### Bundled Recipes

Plan-forge includes default planner and reviewer recipes bundled in the binary. Recipe resolution:

1. Explicit path in config (if it exists)
2. Project-local `.plan-forge/recipes/planner.yaml` or `.plan-forge/recipes/reviewer.yaml`
3. Bundled defaults (no external files required)

## Ralph Loop Integration

Ralph Loop is an iterative development loop that prevents Claude from exiting until a task is complete. It integrates with plan-forge plans to use acceptance criteria as completion conditions.

### Setup Ralph Loop

Copy the Ralph Loop files to your `.claude/` directory:

```bash
# Create directories
mkdir -p .claude/hooks .claude/scripts .claude/commands .claude/skills

# Copy hooks
cp integrations/claude-code/hooks/stop-hook.sh .claude/hooks/stop.sh

# Copy scripts
cp integrations/claude-code/scripts/setup-ralph-loop.sh .claude/scripts/

# Copy commands
cp integrations/claude-code/commands/ralph-loop.md .claude/commands/
cp integrations/claude-code/commands/cancel-ralph.md .claude/commands/

# Copy skills
cp -r integrations/claude-code/skills/ralph-loop .claude/skills/
cp -r integrations/claude-code/skills/cancel-ralph .claude/skills/

# Make scripts executable
chmod +x .claude/hooks/stop.sh .claude/scripts/setup-ralph-loop.sh
```

### Configure Hook Permission

Add the stop hook to your `.claude/settings.local.json`:

```json
{
  "permissions": {
    "allow": [
      "Bash(.claude/scripts/setup-ralph-loop.sh:*)",
      "Bash(rm .claude/ralph-loop.local.md)"
    ]
  },
  "hooks": {
    "Stop": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": ".claude/hooks/stop.sh"
          }
        ]
      }
    ]
  }
}
```

### Usage

**Simple Mode** - Text prompt with completion promise:

```bash
/ralph-loop "Build a REST API for user authentication"
/ralph-loop "Refactor the database layer" --max-iterations 30
```

When complete, output `<promise>DONE</promise>` to signal completion.

**Plan Mode** - Use plan-forge plan with acceptance criteria:

```bash
/ralph-loop --plan plans/active/my-feature/ --max-iterations 50
```

The stop hook automatically verifies testable acceptance criteria on each exit attempt.

**Cancel Loop**:

```bash
/cancel-ralph
```

### How It Works

1. **Initialization**: `/ralph-loop` creates state file at `.claude/ralph-loop.local.md`
2. **Stop Hook**: On each exit attempt, `.claude/hooks/stop.sh` runs:
   - Simple mode: Checks for `<promise>TEXT</promise>` in last message
   - Plan mode: Runs testable acceptance criteria (cargo test, file exists, etc.)
3. **Iteration**: If not complete, increments iteration and feeds back prompt
4. **Completion**: When criteria pass, removes state file and allows exit

### Testable Acceptance Criteria

The stop hook recognizes these patterns from plan acceptance criteria:

| Pattern | Verification |
|---------|-------------|
| `"cargo check completes successfully"` | Runs `cargo check` |
| `"cargo test passes"` | Runs `cargo test` |
| `"cargo clippy passes"` | Runs `cargo clippy -- -D warnings` |
| `"file 'X' exists"` | Tests file existence |
| `"file 'X' does not exist"` | Tests file absence |

### State File Format

```yaml
---
active: true
iteration: 1
max_iterations: 50
mode: plan                          # 'simple' or 'plan'
plan_path: plans/active/my-feature/ # For plan mode
plan_slug: my-feature               # For plan mode
completion_promise: "DONE"          # For simple mode
started_at: "2024-01-13T..."
completed_criteria: []
---

[Original prompt or plan summary]
```
