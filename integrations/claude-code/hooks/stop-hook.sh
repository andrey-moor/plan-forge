#!/bin/bash
# Ralph Loop Stop Hook - Enhanced with Plan-Forge Integration
#
# This hook prevents Claude from exiting until either:
# - Simple mode: completion promise is detected
# - Plan mode: ALL Phase task checkboxes are marked [x] AND acceptance criteria pass
#
# State file: .claude/ralph-loop.local.md (YAML frontmatter + prompt)

set -euo pipefail

# Configuration
STATE_FILE=".claude/ralph-loop.local.md"
DEBUG="${RALPH_LOOP_DEBUG:-false}"

debug() {
    if [[ "$DEBUG" == "true" ]]; then
        echo "[DEBUG] $*" >&2
    fi
}

# Parse YAML frontmatter from state file
parse_yaml() {
    local key="$1"
    local file="$2"

    # Extract YAML between --- markers
    local yaml
    yaml=$(sed -n '/^---$/,/^---$/p' "$file" | sed '1d;$d')

    # Get value for key (simple parsing, handles strings and numbers)
    echo "$yaml" | grep "^${key}:" | sed "s/^${key}:[[:space:]]*//" | tr -d '"'
}

# Check if state file exists and ralph loop is active
check_active() {
    if [[ ! -f "$STATE_FILE" ]]; then
        debug "No state file found at $STATE_FILE"
        echo '{"decision": "approve"}'
        exit 0
    fi

    local active
    active=$(parse_yaml "active" "$STATE_FILE")

    if [[ "$active" != "true" ]]; then
        debug "Ralph loop not active (active=$active)"
        echo '{"decision": "approve"}'
        exit 0
    fi
}

# Get last assistant message from transcript
get_last_assistant_message() {
    local transcript="$1"

    # Extract last assistant message from transcript JSON
    # Transcript is array of messages, find last one with role="assistant"
    echo "$transcript" | jq -r '[.[] | select(.role == "assistant")] | last | .content // ""'
}

# Check for completion promise in simple mode
check_simple_mode() {
    local transcript="$1"
    local completion_promise="$2"

    if [[ -z "$completion_promise" || "$completion_promise" == "null" ]]; then
        debug "No completion promise set"
        return 1
    fi

    local last_message
    last_message=$(get_last_assistant_message "$transcript")

    # Check for promise tag: <promise>TEXT</promise>
    if echo "$last_message" | grep -q "<promise>${completion_promise}</promise>"; then
        debug "Completion promise found: $completion_promise"
        return 0
    fi

    debug "Completion promise NOT found in last message"
    return 1
}

# Check if all Phase task checkboxes are marked complete in plan markdown
check_markdown_tasks() {
    local plan_md="$1"

    if [[ ! -f "$plan_md" ]]; then
        debug "Plan markdown not found: $plan_md"
        return 1
    fi

    # Count Phase checkpoint checkboxes: - [ ] **X.Y ...** or - [x] **X.Y ...**
    # Pattern: checkbox followed by bold text starting with number.number
    local total_tasks
    total_tasks=$(grep -cE '^\s*-\s*\[.\]\s*\*\*[0-9]+\.[0-9]+' "$plan_md" 2>/dev/null | tr -d '[:space:]')
    total_tasks=${total_tasks:-0}

    local done_tasks
    done_tasks=$(grep -ciE '^\s*-\s*\[x\]\s*\*\*[0-9]+\.[0-9]+' "$plan_md" 2>/dev/null | tr -d '[:space:]')
    done_tasks=${done_tasks:-0}

    debug "Task checkboxes: $done_tasks/$total_tasks complete"

    if [[ $total_tasks -eq 0 ]]; then
        debug "No task checkboxes found in plan"
        return 0  # No tasks to track, proceed to acceptance criteria
    fi

    if [[ $done_tasks -lt $total_tasks ]]; then
        # Extract incomplete tasks for feedback
        INCOMPLETE_TASKS=$(grep -E '^\s*-\s*\[ \]\s*\*\*[0-9]+\.[0-9]+' "$plan_md" | head -10)
        TASKS_REMAINING=$((total_tasks - done_tasks))
        export INCOMPLETE_TASKS TASKS_REMAINING
        return 1
    fi

    return 0
}

# Check if all acceptance criteria checkboxes are marked complete in plan markdown
check_acceptance_criteria_checkboxes() {
    local plan_md="$1"

    if [[ ! -f "$plan_md" ]]; then
        debug "Plan markdown not found: $plan_md"
        return 1
    fi

    # Extract the Acceptance Criteria section (between "## Acceptance Criteria" and next "##" or "---")
    local ac_section
    ac_section=$(sed -n '/^## Acceptance Criteria/,/^##\|^---/p' "$plan_md" | grep -v '^##\|^---')

    if [[ -z "$ac_section" ]]; then
        debug "No Acceptance Criteria section found"
        return 0  # No acceptance criteria section, proceed
    fi

    # Count checkboxes in the acceptance criteria section
    local total_ac
    total_ac=$(echo "$ac_section" | grep -cE '^\s*-\s*\[.\]' 2>/dev/null | tr -d '[:space:]')
    total_ac=${total_ac:-0}

    local done_ac
    done_ac=$(echo "$ac_section" | grep -ciE '^\s*-\s*\[x\]' 2>/dev/null | tr -d '[:space:]')
    done_ac=${done_ac:-0}

    debug "Acceptance criteria checkboxes: $done_ac/$total_ac complete"

    if [[ $total_ac -eq 0 ]]; then
        debug "No acceptance criteria checkboxes found"
        return 0  # No checkboxes in AC section, proceed
    fi

    if [[ $done_ac -lt $total_ac ]]; then
        # Extract incomplete acceptance criteria for feedback
        INCOMPLETE_AC=$(echo "$ac_section" | grep -E '^\s*-\s*\[ \]' | head -10)
        AC_REMAINING=$((total_ac - done_ac))
        export INCOMPLETE_AC AC_REMAINING
        return 1
    fi

    return 0
}

# Check plan completion in plan mode
# Verifies both task checkboxes AND acceptance criteria checkboxes are complete
check_plan_mode() {
    local plan_path="$1"
    local plan_slug="$2"

    # Find plan markdown file - try new naming first, then old
    local plan_md="${plan_path}/${plan_slug}-plan.md"
    if [[ ! -f "$plan_md" ]]; then
        plan_md="${plan_path}/${plan_slug}-execution-plan.md"
    fi

    # STEP 1: Check all task checkboxes are complete (Phase tasks with **X.Y format)
    if ! check_markdown_tasks "$plan_md"; then
        debug "Not all tasks complete"
        return 1
    fi

    debug "All tasks complete, checking acceptance criteria checkboxes..."

    # STEP 2: Check all acceptance criteria checkboxes are complete
    if ! check_acceptance_criteria_checkboxes "$plan_md"; then
        debug "Not all acceptance criteria complete"
        return 1
    fi

    debug "All acceptance criteria complete!"
    return 0
}

# Update state file with incremented iteration
increment_iteration() {
    local current_iteration
    current_iteration=$(parse_yaml "iteration" "$STATE_FILE")

    if [[ -z "$current_iteration" || "$current_iteration" == "null" ]]; then
        current_iteration=0
    fi

    local new_iteration=$((current_iteration + 1))

    # Update iteration in state file
    sed -i.bak "s/^iteration:.*$/iteration: $new_iteration/" "$STATE_FILE"
    rm -f "${STATE_FILE}.bak"

    echo "$new_iteration"
}

# Build feedback prompt for plan mode
build_plan_feedback() {
    local plan_path="$1"
    local plan_slug="$2"
    local iteration="$3"

    # Find plan markdown file - try new naming first, then old
    local plan_md="${plan_path}/${plan_slug}-plan.md"
    if [[ ! -f "$plan_md" ]]; then
        plan_md="${plan_path}/${plan_slug}-execution-plan.md"
    fi

    local feedback=""
    feedback+="## Ralph Loop - Iteration $iteration\n\n"

    # Show incomplete tasks first (most actionable)
    if [[ -n "${INCOMPLETE_TASKS:-}" ]]; then
        feedback+="### Incomplete Tasks (${TASKS_REMAINING:-?} remaining)\n\n"
        feedback+="Complete these Phase checkpoints and mark them with \`[x]\`:\n\n"
        feedback+="\`\`\`\n${INCOMPLETE_TASKS}\n\`\`\`\n\n"
        feedback+="**Important**: After completing each task, update the checkbox from \`[ ]\` to \`[x]\` in the plan file.\n\n"
    fi

    # Show incomplete acceptance criteria
    if [[ -n "${INCOMPLETE_AC:-}" ]]; then
        feedback+="### Incomplete Acceptance Criteria (${AC_REMAINING:-?} remaining)\n\n"
        feedback+="Verify these criteria and mark them with \`[x]\` when confirmed:\n\n"
        feedback+="\`\`\`\n${INCOMPLETE_AC}\n\`\`\`\n\n"
    fi

    feedback+="### Your Task\n"
    if [[ -n "${INCOMPLETE_TASKS:-}" ]]; then
        feedback+="Work on the next incomplete task above. When done, mark its checkbox as complete in the plan file.\n\n"
    elif [[ -n "${INCOMPLETE_AC:-}" ]]; then
        feedback+="All tasks are done. Verify the remaining acceptance criteria and mark them as complete.\n\n"
    else
        feedback+="All checkboxes are complete!\n\n"
    fi

    if [[ -f "$plan_md" ]]; then
        feedback+="### Plan File\n"
        feedback+="Location: \`$plan_md\`\n\n"
        feedback+="### Plan Summary\n"
        feedback+="\`\`\`\n$(head -50 "$plan_md")\n\`\`\`\n\n"
    fi

    echo -e "$feedback"
}

# Build feedback prompt for simple mode
build_simple_feedback() {
    local iteration="$1"
    local completion_promise="$2"

    # Get original prompt from state file (everything after YAML frontmatter)
    local original_prompt
    original_prompt=$(sed -n '/^---$/,/^---$/!p' "$STATE_FILE" | tail -n +2)

    local feedback=""
    feedback+="## Ralph Loop - Iteration $iteration\n\n"
    feedback+="The task is not complete yet. Continue working on it.\n\n"
    feedback+="When the task is fully complete, output: \`<promise>${completion_promise}</promise>\`\n\n"
    feedback+="### Original Task\n${original_prompt}\n"

    echo -e "$feedback"
}

# Main hook logic
main() {
    local hook_input
    hook_input=$(cat)

    debug "Hook input received"

    # Check if ralph loop is active
    check_active

    # Parse state
    local mode iteration max_iterations completion_promise plan_path plan_slug
    mode=$(parse_yaml "mode" "$STATE_FILE")
    iteration=$(parse_yaml "iteration" "$STATE_FILE")
    max_iterations=$(parse_yaml "max_iterations" "$STATE_FILE")
    completion_promise=$(parse_yaml "completion_promise" "$STATE_FILE")
    plan_path=$(parse_yaml "plan_path" "$STATE_FILE")
    plan_slug=$(parse_yaml "plan_slug" "$STATE_FILE")

    debug "Mode: $mode, Iteration: $iteration/$max_iterations"

    # Check max iterations
    if [[ -n "$max_iterations" && "$iteration" -ge "$max_iterations" ]]; then
        debug "Max iterations reached ($iteration >= $max_iterations), allowing exit"
        # Clean up state file
        rm -f "$STATE_FILE"
        echo '{"decision": "approve"}'
        exit 0
    fi

    # Extract transcript from hook input
    local transcript
    transcript=$(echo "$hook_input" | jq -r '.transcript // "[]"')

    # Check completion based on mode
    local complete=false

    if [[ "$mode" == "plan" ]]; then
        if check_plan_mode "$plan_path" "$plan_slug"; then
            complete=true
        fi
    else
        # Simple mode (default)
        if check_simple_mode "$transcript" "$completion_promise"; then
            complete=true
        fi
    fi

    if [[ "$complete" == "true" ]]; then
        debug "Task complete! Allowing exit"
        # Clean up state file
        rm -f "$STATE_FILE"
        echo '{"decision": "approve"}'
        exit 0
    fi

    # Not complete - block exit and feed back prompt
    local new_iteration
    new_iteration=$(increment_iteration)

    local feedback
    if [[ "$mode" == "plan" ]]; then
        feedback=$(build_plan_feedback "$plan_path" "$plan_slug" "$new_iteration")
    else
        feedback=$(build_simple_feedback "$new_iteration" "$completion_promise")
    fi

    # Escape feedback for JSON
    local escaped_feedback
    escaped_feedback=$(echo "$feedback" | jq -Rs .)

    debug "Blocking exit, iteration $new_iteration"

    # Return block decision with feedback
    cat <<EOF
{
    "decision": "block",
    "reason": "Ralph Loop iteration $new_iteration - task not complete",
    "message": $escaped_feedback
}
EOF
}

main
