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
| `plan_run` | Create or resume a planning session |
| `plan_status` | Get session status (ready/in_progress/needs_input/approved/max_turns) |
| `plan_list` | List all planning sessions |
| `plan_get` | Read plan, tasks, or context markdown files |
| `plan_approve` | Force approve a plan and write to dev/active/ |

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
        Plan saved to: dev/active/add-user-authentication-with-oauth/

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

./dev/active/<slug>/        # Final output (commit to repo)
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
| `max_turns` | Max iterations reached without approval |

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
  pass_threshold: 0.8

loop_config:
  max_iterations: 5
  early_exit_on_perfect_score: true

output:
  runs_dir: ./.plan-forge
  active_dir: ./dev/active
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

### Environment Variables

Environment variables override config file values:

| Variable | Description | Default |
|----------|-------------|---------|
| `PLAN_FORGE_THRESHOLD` | Review pass threshold (0.0-1.0) | 0.8 |
| `PLAN_FORGE_MAX_ITERATIONS` | Maximum planning iterations | 5 |
| `PLAN_FORGE_PLANNER_PROVIDER` | Override planner provider | - |
| `PLAN_FORGE_PLANNER_MODEL` | Override planner model | - |
| `PLAN_FORGE_REVIEWER_PROVIDER` | Override reviewer provider | - |
| `PLAN_FORGE_REVIEWER_MODEL` | Override reviewer model | - |
| `PLAN_FORGE_RECIPE_DIR` | Directory to search for recipes | - |

### Bundled Recipes

Plan-forge includes default planner and reviewer recipes bundled in the binary. Recipe resolution:

1. Explicit path in config (if it exists)
2. Project-local `.plan-forge/recipes/planner.yaml` or `.plan-forge/recipes/reviewer.yaml`
3. Bundled defaults (no external files required)
