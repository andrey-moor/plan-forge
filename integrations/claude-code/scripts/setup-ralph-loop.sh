#!/bin/bash
# Setup Ralph Loop - Initialize state file for iterative development
#
# Usage:
#   Simple mode:  ./setup-ralph-loop.sh "Build the API" --completion-promise "DONE" --max-iterations 20
#   Plan mode:    ./setup-ralph-loop.sh --plan plans/active/my-feature/ --max-iterations 50
#
# State is written to .claude/ralph-loop.local.md

set -euo pipefail

STATE_FILE=".claude/ralph-loop.local.md"

usage() {
    cat <<EOF
Usage: setup-ralph-loop.sh [OPTIONS] [PROMPT]

Initialize a Ralph Loop session for iterative development.

MODES:
  Simple Mode:
    setup-ralph-loop.sh "Your task prompt" [options]

  Plan Mode:
    setup-ralph-loop.sh --plan <path> [options]

OPTIONS:
  --plan <path>           Path to plan directory (enables plan mode)
  --max-iterations <n>    Maximum iterations before auto-exit (default: 20)
  --completion-promise <text>
                          Text that signals completion in simple mode (default: "DONE")
  --help                  Show this help message

EXAMPLES:
  # Simple mode with default promise
  setup-ralph-loop.sh "Build a REST API for user management"

  # Simple mode with custom promise and limit
  setup-ralph-loop.sh "Refactor the auth module" --completion-promise "REFACTOR_COMPLETE" --max-iterations 10

  # Plan mode
  setup-ralph-loop.sh --plan plans/active/add-authentication/ --max-iterations 50

STATE FILE:
  The session state is stored in .claude/ralph-loop.local.md
  To stop the loop manually, delete this file or run cancel-ralph.

EOF
    exit 0
}

# Defaults
MODE="simple"
PROMPT=""
PLAN_PATH=""
PLAN_SLUG=""
MAX_ITERATIONS=20
COMPLETION_PROMISE="DONE"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --help|-h)
            usage
            ;;
        --plan)
            MODE="plan"
            PLAN_PATH="${2:-}"
            shift 2 || { echo "Error: --plan requires a path argument" >&2; exit 1; }
            ;;
        --max-iterations)
            MAX_ITERATIONS="${2:-20}"
            shift 2 || { echo "Error: --max-iterations requires a number" >&2; exit 1; }
            ;;
        --completion-promise)
            COMPLETION_PROMISE="${2:-DONE}"
            shift 2 || { echo "Error: --completion-promise requires text" >&2; exit 1; }
            ;;
        -*)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
        *)
            # Positional argument is the prompt
            PROMPT="$1"
            shift
            ;;
    esac
done

# Validate arguments
if [[ "$MODE" == "simple" && -z "$PROMPT" ]]; then
    echo "Error: Simple mode requires a prompt. Use --plan for plan mode." >&2
    exit 1
fi

if [[ "$MODE" == "plan" ]]; then
    if [[ -z "$PLAN_PATH" ]]; then
        echo "Error: --plan requires a path to the plan directory" >&2
        exit 1
    fi

    # Normalize plan path (remove trailing slash)
    PLAN_PATH="${PLAN_PATH%/}"

    if [[ ! -d "$PLAN_PATH" ]]; then
        echo "Error: Plan directory not found: $PLAN_PATH" >&2
        exit 1
    fi

    # Extract slug from path (last component)
    PLAN_SLUG=$(basename "$PLAN_PATH")

    # Verify plan files exist
    if [[ ! -f "${PLAN_PATH}/${PLAN_SLUG}-plan.md" && ! -f "${PLAN_PATH}/${PLAN_SLUG}-tasks.md" ]]; then
        # Check .plan-forge/ for plan files
        if [[ ! -d ".plan-forge/${PLAN_SLUG}" ]]; then
            echo "Warning: No plan files found in $PLAN_PATH or .plan-forge/${PLAN_SLUG}/" >&2
        fi
    fi
fi

# Get current timestamp
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

# Build state file content
if [[ "$MODE" == "plan" ]]; then
    # Plan mode state
    STATE_CONTENT="---
active: true
iteration: 1
max_iterations: ${MAX_ITERATIONS}
mode: plan
plan_path: ${PLAN_PATH}
plan_slug: ${PLAN_SLUG}
completion_promise: null
started_at: \"${TIMESTAMP}\"
completed_criteria: []
---

# Ralph Loop - Plan Mode

Executing plan from: \`${PLAN_PATH}/\`

## Plan Summary

"
    # Add plan summary if available
    PLAN_MD="${PLAN_PATH}/${PLAN_SLUG}-plan.md"
    if [[ -f "$PLAN_MD" ]]; then
        STATE_CONTENT+=$(head -30 "$PLAN_MD")
    else
        STATE_CONTENT+="Plan files will be read from ${PLAN_PATH}/"
    fi

    STATE_CONTENT+="

## Instructions

Work through the plan tasks until all acceptance criteria pass.
The stop hook will automatically verify testable criteria on each exit attempt.
"
else
    # Simple mode state
    STATE_CONTENT="---
active: true
iteration: 1
max_iterations: ${MAX_ITERATIONS}
mode: simple
plan_path: null
plan_slug: null
completion_promise: \"${COMPLETION_PROMISE}\"
started_at: \"${TIMESTAMP}\"
completed_criteria: []
---

# Ralph Loop - Simple Mode

## Task

${PROMPT}

## Instructions

When the task is complete, output: \`<promise>${COMPLETION_PROMISE}</promise>\`

This will signal completion and allow the session to end.
"
fi

# Write state file
echo "$STATE_CONTENT" > "$STATE_FILE"

# Output confirmation
echo "Ralph Loop initialized!"
echo ""
echo "Mode: $MODE"
if [[ "$MODE" == "plan" ]]; then
    echo "Plan: $PLAN_PATH"
    echo "Slug: $PLAN_SLUG"
else
    echo "Prompt: ${PROMPT:0:50}..."
    echo "Completion Promise: $COMPLETION_PROMISE"
fi
echo "Max Iterations: $MAX_ITERATIONS"
echo ""
echo "State file: $STATE_FILE"
echo ""
echo "To cancel the loop, delete the state file or run: rm $STATE_FILE"
