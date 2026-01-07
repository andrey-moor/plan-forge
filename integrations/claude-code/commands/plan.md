---
description: Create and manage AI-driven development plans using plan-forge
---

# /plan Command

Create and manage development plans using the plan-forge MCP server.

## Usage

```
/plan <task description>     - Start a new planning session in background
/plan status [session_id]    - Check status of a planning session
/plan list                   - List all planning sessions
/plan get <file> [session]   - Read plan content (plan|tasks|context)
/plan approve [session_id]   - Force approve current plan
/plan resume <id> <feedback> - Resume session with feedback
```

## Command Handling

When the user runs `/plan`, parse the arguments and execute accordingly:

### `/plan <task>` (New Plan)
1. Spawn the `plan-forge` agent with the Task tool using `run_in_background: true`
2. Pass the task description to the agent
3. Immediately respond: "Started planning session for: <task>. Use `/plan status` to check progress."

### `/plan status [session_id]`
1. Call `plan_status(session_id)` via MCP
2. Format and display the result:
   - Session ID
   - Status (ready/in_progress/needs_input/approved/max_turns)
   - Current iteration
   - Latest score (if available)
   - Input reason (if needs_input)

### `/plan list`
1. Call `plan_list()` via MCP
2. Display all sessions in a table format with status

### `/plan get <file> [session_id]`
1. Validate file is one of: plan, tasks, context
2. Call `plan_get(file, session_id)` via MCP
3. Display the markdown content

### `/plan approve [session_id]`
1. Call `plan_approve(session_id)` via MCP
2. Confirm: "Plan approved and written to dev/active/<slug>/"

### `/plan resume <session_id> <feedback>`
1. Spawn the `plan-forge` agent with the Task tool using `run_in_background: true`
2. Pass the feedback and session_id to resume the planning session
3. Immediately respond: "Resuming planning session: <session_id>. Use `/plan status` to check progress."

## Examples

```
User: /plan Add user authentication with OAuth

Claude: Started planning session for: "Add user authentication with OAuth"
        Session ID: add-user-authentication-with-oauth
        Use `/plan status` to check progress.

User: /plan status

Claude: Session: add-user-authentication-with-oauth
        Status: approved
        Iterations: 2
        Score: 0.85
        Plan saved to: dev/active/add-user-authentication-with-oauth/

User: /plan get plan

Claude: [displays plan content]
```

## Background Execution

For new plans (`/plan <task>`), always use background execution:
- Spawn the plan-forge agent asynchronously
- Return immediately so user can continue working
- User checks progress with `/plan status`

This prevents blocking the conversation during the planning process which can take 30 seconds to several minutes.
