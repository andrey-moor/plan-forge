---
name: auto-plan
description: Auto-detect and handle requests to create development plans in the background using plan-forge
---

# Plan Skill

Automatically detect when users want to create development plans and handle them using the plan-forge MCP server.

## Trigger Patterns

This skill should be invoked when the user's request matches patterns like:

- "plan <task> in the background"
- "create a development plan for <task>"
- "start planning <task>"
- "plan out <task>"
- "make a plan for <task>"
- "I need a plan for <task>"
- "generate a plan for <task>"

## Behavior

When triggered:

1. **Extract the task** from the user's request
2. **Spawn background agent** using the Task tool with `plan-forge` agent and `run_in_background: true`
3. **Acknowledge immediately** - don't wait for planning to complete
4. **Provide status check instructions**

## Response Template

```
I've started a background planning session for: "<task>"

The plan-forge agent is now:
1. Analyzing the codebase
2. Generating a comprehensive development plan
3. Reviewing and refining until quality threshold is met

This typically takes 1-5 iterations (30 seconds to a few minutes).

**Check progress:** `/plan status`
**View results:** `/plan get plan`

I'll continue to be available while planning runs in the background.
```

## Example Interactions

### Example 1: Explicit background request
```
User: Plan adding OAuth authentication in the background

Claude: I've started a background planning session for: "adding OAuth authentication"
        [spawns plan-forge agent with run_in_background: true]
        Check progress with `/plan status`
```

### Example 2: Implicit planning request
```
User: I need a development plan for implementing user roles

Claude: I've started a background planning session for: "implementing user roles"
        [spawns plan-forge agent with run_in_background: true]
        Check progress with `/plan status`
```

### Example 3: Status check follow-up
```
User: How's that plan coming along?

Claude: [calls plan_status via MCP]
        Session: implementing-user-roles
        Status: in_progress
        Iteration: 2 of 5
        Still working on it...
```

## Integration with /plan Command

This skill works alongside the `/plan` slash command:
- Skill: Auto-detects planning intent in natural conversation
- Command: Explicit invocation with `/plan <task>`

Both use the same underlying plan-forge MCP tools and background agent.

## Handling needs_input Response

When you check on a background agent (via TaskOutput) and it returns with `status="needs_input"`:

1. **Draft plan is visible** - The plan is automatically written to `dev/active/<slug>/` with "DRAFT - Awaiting Human Input" status so users can review it
2. **Parse the reviewer's questions** from the `reason` field
3. **Present questions interactively** using the AskUserQuestion tool when possible:

   Example - if reason contains architectural decision questions:
   ```
   AskUserQuestion:
     questions:
       - question: "Which storage mechanism should be used?"
         header: "Storage"
         options:
           - label: "Redis"
             description: "In-memory cache, good for sessions"
           - label: "PostgreSQL"
             description: "Persistent, good for user data"
           - label: "File-based"
             description: "Simple, no external dependencies"
         multiSelect: false
   ```

4. **When user responds**, resume using `feedback` parameter:
   ```
   plan_run(
     task="",
     session_id="<session-slug>",
     feedback="Use Redis for session storage"
   )
   ```

**When to use AskUserQuestion vs free text:**
- Use options when reviewer lists specific alternatives (storage, auth, etc.)
- Use free text when questions are open-ended or need custom answers
- Always include "Other" option via the tool's default behavior

## Notes

- Always use background execution for new plans
- User can continue conversation while planning runs
- **Draft plans are visible**: When paused, plans are written to `dev/active/<slug>/` with "DRAFT" status
- If plan needs human input, agent stops and returns with `needs_input` status
- Use the `feedback` parameter in `plan_run` to continue with user's answers
