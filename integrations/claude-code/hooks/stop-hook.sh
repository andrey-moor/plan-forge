#!/bin/bash
# Ralph Loop Stop Hook - Enhanced with Plan-Forge Integration
#
# This hook prevents Claude from exiting until either:
# - Simple mode: completion promise is detected
# - Plan mode: all required acceptance criteria pass
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

# Check testable acceptance criteria in plan mode
check_plan_mode() {
    local plan_path="$1"
    local plan_slug="$2"

    # Find the plan JSON file
    local plan_json="${plan_path}/${plan_slug}-plan.json"
    if [[ ! -f "$plan_json" ]]; then
        # Try finding iteration files in .plan-forge/
        plan_json=$(find ".plan-forge/${plan_slug}/" -name "plan-iteration-*.json" 2>/dev/null | sort -V | tail -1)
    fi

    if [[ -z "$plan_json" || ! -f "$plan_json" ]]; then
        debug "Plan JSON not found at $plan_path or .plan-forge/$plan_slug/"
        # Can't verify, allow continuation
        return 1
    fi

    debug "Checking acceptance criteria from: $plan_json"

    # Extract acceptance criteria
    local criteria
    criteria=$(jq -r '.acceptance_criteria // []' "$plan_json")

    if [[ "$criteria" == "[]" || -z "$criteria" ]]; then
        debug "No acceptance criteria found in plan"
        return 1
    fi

    # Check each testable criterion
    local total=0
    local passed=0
    local failed_criteria=""

    while IFS= read -r criterion; do
        local description priority testable
        description=$(echo "$criterion" | jq -r '.description // ""')
        priority=$(echo "$criterion" | jq -r '.priority // "Recommended"')
        testable=$(echo "$criterion" | jq -r '.testable // false')

        # Only check testable, required criteria
        if [[ "$testable" != "true" ]]; then
            continue
        fi

        ((total++)) || true

        # Run verification based on description patterns
        local result=1

        case "$description" in
            *"cargo check"*|*"compiles successfully"*)
                cargo check 2>/dev/null && result=0
                ;;
            *"cargo test"*|*"tests pass"*)
                cargo test 2>/dev/null && result=0
                ;;
            *"cargo clippy"*)
                cargo clippy -- -D warnings 2>/dev/null && result=0
                ;;
            *"cargo build"*)
                cargo build 2>/dev/null && result=0
                ;;
            *"file "*" exists"*|*"exists"*)
                # Extract file path from description
                local filepath
                filepath=$(echo "$description" | grep -oE "'[^']+'" | tr -d "'" | head -1)
                if [[ -n "$filepath" && -f "$filepath" ]]; then
                    result=0
                fi
                ;;
            *"file "*" does not exist"*)
                local filepath
                filepath=$(echo "$description" | grep -oE "'[^']+'" | tr -d "'" | head -1)
                if [[ -n "$filepath" && ! -f "$filepath" ]]; then
                    result=0
                fi
                ;;
            *)
                # Unknown pattern, skip
                debug "Unknown criterion pattern: $description"
                ((total--)) || true
                continue
                ;;
        esac

        if [[ $result -eq 0 ]]; then
            ((passed++)) || true
            debug "PASS: $description"
        else
            failed_criteria="${failed_criteria}\n- $description"
            debug "FAIL: $description"
        fi
    done < <(echo "$criteria" | jq -c '.[]')

    debug "Criteria check: $passed/$total passed"

    # All testable criteria must pass
    if [[ $total -gt 0 && $passed -eq $total ]]; then
        return 0
    fi

    # Store failed criteria for feedback
    export FAILED_CRITERIA="$failed_criteria"
    return 1
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

    local plan_md="${plan_path}/${plan_slug}-plan.md"
    local tasks_md="${plan_path}/${plan_slug}-tasks.md"

    local feedback=""
    feedback+="## Ralph Loop - Iteration $iteration\n\n"
    feedback+="The plan-forge acceptance criteria have not all passed yet.\n\n"

    if [[ -n "${FAILED_CRITERIA:-}" ]]; then
        feedback+="### Failed Criteria\n${FAILED_CRITERIA}\n\n"
    fi

    feedback+="### Your Task\n"
    feedback+="Continue implementing the plan. Review the tasks and work on the next incomplete item.\n\n"

    if [[ -f "$plan_md" ]]; then
        feedback+="### Plan Summary\n"
        feedback+="\`\`\`\n$(head -50 "$plan_md")\n\`\`\`\n\n"
    fi

    if [[ -f "$tasks_md" ]]; then
        feedback+="### Tasks\n"
        feedback+="\`\`\`\n$(head -100 "$tasks_md")\n\`\`\`\n\n"
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
