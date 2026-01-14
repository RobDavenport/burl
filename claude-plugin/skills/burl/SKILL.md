---
description: |
  Complete guide to Burl, a file-based workflow orchestrator for agentic AI development. Use this skill when working with Burl workflows, creating tasks, understanding scope constraints, or running Burl CLI commands.

  Trigger phrases: "burl", "burl workflow", "burl task", "burl cli", "task states", "scope constraints", "affects_globs", "must_not_touch", "claim", "submit", "validate", "approve", "reject", "READY", "DOING", "QA", "DONE", "BLOCKED"

  **Load references for detailed information:**
  - Full CLI documentation: `references/cli-reference.md`
  - Task file format specification: `references/task-format.md`
  - Validation rules (scope/stubs): `references/validation.md`
---

# Burl Workflow Orchestrator

Burl is a minimal, file-based workflow orchestrator for agentic coding pipelines. It uses folders as status buckets, Git worktrees for isolation, and deterministic validation gates.

## Core Principles

1. **Filesystem is truth**: Task status = folder location (READY/DOING/QA/DONE/BLOCKED)
2. **Git worktrees isolate work**: Each task gets its own worktree at `.worktrees/task-NNN-slug/`
3. **Deterministic validation**: Diff-based scope/stub checks - no LLM judging LLM
4. **Race-safe**: Per-task locks prevent double-claims
5. **Portable**: Workflow state is Git-committed, can pause/resume across machines

## Directory Structure

```
.burl/                      # Workflow worktree (branch: burl)
  .workflow/
    READY/                  # Claimable tasks
    DOING/                  # Work in progress
    QA/                     # Submitted, awaiting validation
    DONE/                   # Approved and merged
    BLOCKED/                # Dependencies unmet or max attempts
    config.yaml             # Workflow configuration
    events/events.ndjson    # Audit log
    locks/                  # Machine-local (untracked)

.worktrees/
  task-001-slug/            # Isolated worktree per task
```

## Task States & Transitions

```
         claim
READY ─────────► DOING ───────► QA ───────► DONE
   ▲              │ submit       │ approve
   │              │              │
   └──── reject ◄─┘              └── reject ─► READY (or BLOCKED)
```

## Task File Quick Reference

Filename: `TASK-NNN-slug.md` with YAML frontmatter:

| Field | Purpose | Example |
|-------|---------|---------|
| `id` | Unique identifier | `TASK-001` |
| `title` | Task description | `Implement player jump` |
| `priority` | Ordering (high/medium/low) | `high` |
| `affects` | Exact paths allowed to change | `[src/player.rs]` |
| `affects_globs` | Patterns for new/existing files | `[src/player/**]` |
| `must_not_touch` | Forbidden paths (hard fail) | `[src/enemy/**]` |
| `depends_on` | Task dependencies | `[TASK-001]` |
| `base_sha` | Diff reference (set on claim) | `abc123...` |

## CLI Quick Reference

| Command | Purpose |
|---------|---------|
| `burl init` | Initialize workflow in repo |
| `burl add "Title" --affects-globs "src/**"` | Create task in READY |
| `burl status` | Show workflow summary |
| `burl show TASK-ID` | Display task details |
| `burl claim [TASK-ID]` | Claim task (READY → DOING) |
| `burl submit [TASK-ID]` | Submit for QA (DOING → QA) |
| `burl validate TASK-ID` | Run validation checks |
| `burl approve TASK-ID` | Approve and merge (QA → DONE) |
| `burl reject TASK-ID --reason "..."` | Reject with reason |
| `burl doctor [--repair]` | Diagnose/repair issues |
| `burl clean --completed` | Remove completed worktrees |

## Typical Workflow

```bash
# Initialize (once)
burl init

# Create task with scope
burl add "Implement feature" \
  --priority high \
  --affects-globs "src/feature/**" \
  --must-not-touch "src/core/**"

# Claim and work
burl claim TASK-001
cd $(burl worktree TASK-001)
# ... make changes, commit ...

# Submit for validation
burl submit TASK-001

# Approve (if validation passes)
burl approve TASK-001
```

## Scope Constraints

Scope constraints define what files a task is allowed to modify:

- **`affects`**: Explicit file paths that can be changed
- **`affects_globs`**: Glob patterns for files (supports new files)
- **`must_not_touch`**: Forbidden patterns - touching these fails validation

**Validation rules:**
1. Files matching `must_not_touch` → **FAIL** (S1)
2. Files not in `affects` or `affects_globs` → **FAIL** (S2)

## Stub Detection

Validation fails if added lines contain incomplete code patterns:
- `TODO`, `FIXME`, `XXX`, `HACK`
- `unimplemented!()`, `todo!()`
- `panic!("not implemented")`
- `NotImplementedError`, `raise NotImplemented`
- `pass` (Python placeholder), `...` (ellipsis)

**Important:** Only **added lines** in the diff are checked, not pre-existing code.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | User error (bad args, invalid state) |
| 2 | Validation failure (scope/stubs) |
| 3 | Git operation failure |
| 4 | Lock acquisition failure |

## Common Issues

**"task is locked"**: Another process holds the lock. Wait or force clear:
```bash
burl lock clear TASK-001 --force
```

**"scope violation"**: Changed files outside allowed scope. Either revert changes or update task scope.

**"stub patterns found"**: Added lines contain TODO/unimplemented. Remove before submit.
