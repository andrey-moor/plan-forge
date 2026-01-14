---
description: Update plan documentation before context compaction
argument-hint: Optional - specific context or focus area (leave empty for comprehensive update)
---

Context limits approaching. Update plan documentation in `dev/active/` to ensure seamless continuation after context reset.

**Key principle**: Context files maintain CURRENT STATE, not history. Prune aggressively.

## Flags

- `--with-parent`: When completing a sub-plan, also update the parent plan's status

## Plan Completion Assessment (EXECUTE FIRST)

Before updating, assess the current state of each plan:

| State | Action | Criteria |
|-------|--------|----------|
| **Continue** | Update progress, capture context | More tasks remain, progress being made |
| **Graduate** | Move to `dev/completed/` | All success criteria met |
| **Pause** | Add pause note with resume instructions | Blocked or deprioritized |
| **Abandon** | Archive with explanation | No longer relevant |

### Graduation Checklist

Before moving a plan to `dev/completed/`:
- [ ] All tasks in tasks file marked ✅
- [ ] Success criteria in plan file verified
- [ ] No active blockers

**To graduate a plan:**
```bash
mv dev/active/[task-name]/ dev/completed/
```

### Sub-Plan Graduation (with --with-parent flag)

When completing a sub-plan that has a parent:

1. Graduate the sub-plan as normal
2. Find parent plan (from `Parent:` field in sub-plan header)
3. Update parent's Sub-Plans section:
   ```markdown
   - ✅ [Sub-plan name] → `../completed/[sub-plan]/`
   ```
4. Update parent's progress counter

**Parent identification:**
- **Primary**: Read `Parent:` field from sub-plan header
- **Fallback**: Match by naming prefix (`antipattern-training` → `antipattern-detector`)
- **Validation**: Warn if explicit reference doesn't match prefix

---

## Size Limits (ENFORCE STRICTLY)

| File | Target | Warning | Action if exceeded |
|------|--------|---------|-------------------|
| `context.md` | <300 lines | >400 lines | Prune or split plan |
| `plan.md` | <150 lines | >200 lines | Remove task details |
| `tasks.md` | Variable | >500 lines | Split into sub-plans |

**If context.md exceeds 400 lines**: Stop and consider splitting the plan into sub-plans.

---

## Update Strategy

**Focus**: Capture information that would be **hard to rediscover** from code alone.

**Capture (hard to rediscover):**
- Complex problems solved and how
- Architectural decisions and rationale
- Tricky bugs found and workarounds
- Integration discoveries
- Non-obvious dependencies

**DO NOT capture (easily found):**
- Information obvious from code structure
- Standard operations (file creation, imports)
- Things easily found via Prism/Grep
- General best practices
- Historical session-by-session progress (keep current only)

---

## Required Updates

### 1. Update Task Progress (`tasks.md`)

**Update task status:**
- ✅ = Completed
- 🔄 = In progress: `🔄 [brief state]`
- ⛔ = Blocked: `⛔ [blocker]`
- [ ] = Pending

**Update progress counter:**
```markdown
Progress: X/Y tasks complete
```

---

### 2. Update Context (`context.md`) - CURRENT STATE ONLY

**IMPORTANT**: Replace existing sections, do NOT append new session sections.

The context file should always reflect CURRENT state, not historical accumulation.

#### Template (target ~200 lines):

```markdown
# [Task Name] - Context

## Current State
- **Working on**: [Exact current task]
- **Progress**: X/Y tasks complete
- **Blockers**: [List or "None"]
- **Last updated**: YYYY-MM-DD

## Key Decisions (max 10)

Keep only decisions relevant to REMAINING work. Prune completed/obsolete decisions.

- **[Decision]**: [One-line rationale] (YYYY-MM-DD)
- **[Decision]**: [One-line rationale] (YYYY-MM-DD)

## Discoveries (max 15)

Keep only insights needed for REMAINING work. Prune obsolete discoveries.

- [Non-obvious insight still relevant]
- [Integration point affecting remaining tasks]

## Active Issues

Only list UNRESOLVED issues. Move resolved to Resolution Log.

- **[Issue]**: [Workaround if any]

## Resolution Log (one-liners only)

Compressed record of solved problems. One line each, oldest can be pruned.

- [Issue] → [Solution] (YYYY-MM-DD)
- [Issue] → [Solution] (YYYY-MM-DD)

## Handoff Notes

Always keep current. Replace entirely on each update.

**Immediate next action**: [Exact first step]
**Current file**: `path/to/file.ts` (lines X-Y)
**Uncommitted changes**: [List or "None"]
**Verification command**: `[command to verify state]`
```

#### Pruning Rules (APPLY ON EVERY UPDATE)

| Section | Max Entries | Prune Strategy |
|---------|-------------|----------------|
| Key Decisions | 10 | Remove decisions for completed phases |
| Discoveries | 15 | Remove insights no longer relevant |
| Active Issues | 5 | Move resolved → Resolution Log |
| Resolution Log | 10 | Remove oldest when full |

**When adding new entries:**
1. Check current count
2. If at max, remove oldest/least relevant entry
3. Then add new entry

---

### 3. Update Plan (`plan.md`) - ONLY IF SIGNIFICANT CHANGES

**Update ONLY if:**
- Approach fundamentally changed
- New risks identified
- Success criteria changed
- Scope significantly expanded/reduced

**DO NOT update for:**
- Minor task adjustments
- Implementation details
- Time variations

---

## Quick/Standard Tier Updates

For single-file plans (`-quick.md` or `-plan.md`):

1. Update task checkboxes
2. Add brief notes inline if needed
3. Update progress counter
4. Keep file under 100 lines

---

## Parent Plan Updates

When a plan has sub-plans (check for `## Sub-Plans` section):

**Parent tasks.md format:**
```markdown
## Sub-Plan Status
- ✅ [Feature 1] → `../completed/[name]/`
- 🔄 [Feature 2] → `[name]/`
- [ ] [Feature 3] → `[name]/`

Progress: 1/3 sub-plans complete
```

**Parent context.md**: Keep minimal - only shared decisions affecting multiple sub-plans.

---

## Update Process

1. **List active plans**: `ls dev/active/`

2. **For each plan, update in order**:
   - `tasks.md` - Update statuses, progress counter
   - `context.md` - Replace Current State, prune sections
   - `plan.md` - Only if significant changes

3. **Check size limits**:
   - If context.md > 400 lines → prune harder or split plan
   - If tasks.md > 500 lines → split into sub-plans

4. **If graduating with parent** (`--with-parent`):
   - Move sub-plan to `dev/completed/`
   - Update parent's Sub-Plans section
   - Update parent's progress counter

---

## Output

After updating:

```
✓ Updated: dev/active/[task-name]/

Files:
- tasks.md: X/Y complete, Z in-progress, N blocked
- context.md: [lines] lines (target <300)
- plan.md: [Updated/No changes]

Current state:
- Working on: [task]
- Next action: [step]
- Blockers: [count]

[If --with-parent used:]
- Parent [parent-name] updated: sub-plan marked complete
```

---

## Examples

### ✅ Good Context (Concise)

```markdown
## Key Decisions (max 10)
- **Use DirectMCP over agent-based fetching**: 5x faster, 100% reliable (2025-12-07)
- **F1 threshold 0.6 for lexical routing**: Data-driven from evaluation (2025-12-07)

## Discoveries (max 15)
- MCP SDK provides `stdio_client` for direct JSON-RPC without agent overhead
- Pattern routing query takes ~10ms for 562 patterns - no caching needed

## Resolution Log (one-liners only)
- Agent 0 findings → Wrong Foundry model name, fixed to `claude-sonnet-4-5` (2025-12-07)
- JSON output not created → Format inference from extension added (2025-12-07)
```

### ❌ Bad Context (Bloated)

```markdown
## Session Progress (2025-12-07)
[20 lines of session details...]

## Previous Session (2025-12-06)
[30 lines of old session details...]

## Session Before That (2025-12-05)
[40 lines of even older details...]
```

**Fix**: Delete all session sections, keep only Current State.

---

## Additional Context: $ARGUMENTS

[Process any specific context or focus areas provided by user]
