# Ralph Loop Command

Start an iterative development loop that prevents Claude from exiting until the task is complete.

## Usage

```
/ralph-loop <prompt>                         # Simple mode with default settings
/ralph-loop <prompt> --max-iterations 30     # Custom iteration limit
/ralph-loop --plan plans/active/my-feature/  # Plan mode with plan-forge
```

## Arguments

$ARGUMENTS will contain the user's input after `/ralph-loop`.

## Instructions

Parse the arguments and initialize the Ralph Loop:

1. **Determine Mode**:
   - If `--plan` flag is present: Plan mode
   - Otherwise: Simple mode with the provided prompt

2. **Run Setup Script**:
   ```bash
   .claude/scripts/setup-ralph-loop.sh $ARGUMENTS
   ```

3. **Confirm Initialization**:
   - Display the mode (simple or plan)
   - Show max iterations
   - For plan mode: show plan path and summary
   - For simple mode: show prompt and completion promise

4. **Start Working**:
   - In simple mode: Begin working on the task
   - In plan mode: Read the plan files and start on the first incomplete task

## Examples

### Simple Mode
```
/ralph-loop "Build a REST API for user authentication"
```
Response: Initialize loop, start working on the API.

### Plan Mode
```
/ralph-loop --plan plans/active/add-oauth-support/ --max-iterations 50
```
Response: Initialize loop, read plan files, start implementing.

## Important Notes

- The stop hook (`.claude/hooks/stop.sh`) will prevent exit until completion
- In simple mode, output `<promise>DONE</promise>` when complete
- In plan mode, testable acceptance criteria are checked automatically
- Use `/cancel-ralph` to stop the loop early
- State is stored in `.claude/ralph-loop.local.md`
