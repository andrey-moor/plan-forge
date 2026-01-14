# Cancel Ralph Command

Cancel an active Ralph Loop session.

## Usage

```
/cancel-ralph
```

## Instructions

1. Check if Ralph Loop is active by looking for `.claude/ralph-loop.local.md`

2. If active:
   - Read the current state (mode, iteration, etc.)
   - Delete the state file
   - Confirm cancellation to user

3. If not active:
   - Inform user that no Ralph Loop is running

## Implementation

```bash
if [ -f ".claude/ralph-loop.local.md" ]; then
    rm .claude/ralph-loop.local.md
    echo "Ralph Loop cancelled."
else
    echo "No active Ralph Loop found."
fi
```

## Notes

- This immediately stops the loop - the stop hook will allow exit
- Any progress made is preserved in the codebase
- To restart, use `/ralph-loop` again
