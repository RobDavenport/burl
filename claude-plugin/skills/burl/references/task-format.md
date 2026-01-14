# Burl Task File Format

Complete specification for task files.

## File Naming

Pattern: `TASK-{id}-{slug}.md`

- **id**: Three or more digits, zero-padded (e.g., `001`, `042`, `123`)
- **slug**: Lowercase, hyphens for spaces, max 50 chars

Examples:
- `TASK-001-implement-player-jump.md`
- `TASK-042-fix-auth-bug.md`

## Frontmatter Fields

### Required Fields

```yaml
---
id: TASK-001
title: Implement player jump
---
```

### Priority & Categorization

```yaml
priority: high          # high | medium | low (default: medium)
created: 2026-01-13T10:00:00Z
tags: [feature, player]
```

### Ownership & Attempts

```yaml
assigned_to: null       # Set on claim
qa_attempts: 0          # Incremented on reject
```

### Lifecycle Timestamps

```yaml
started_at: null        # Set on claim
submitted_at: null      # Set on submit
completed_at: null      # Set on approve
```

### Git/Worktree State

```yaml
worktree: null          # Path to task worktree (set on claim)
branch: null            # Task branch name (set on claim)
base_sha: null          # CRITICAL: Diff reference for validation
```

**Important:** `base_sha` is set when the task is claimed and is used for all diff-based validation. It represents the state of `origin/main` at claim time.

### Scope Control

```yaml
affects:
  - src/player/jump.rs
  - src/player/mod.rs
affects_globs:
  - src/player/**
must_not_touch:
  - src/enemy/**
  - src/networking/**
```

**Field semantics:**

| Field | Purpose | Supports New Files |
|-------|---------|-------------------|
| `affects` | Exact paths allowed | No (files must exist) |
| `affects_globs` | Pattern matching | Yes |
| `must_not_touch` | Forbidden patterns | N/A |

### Dependencies

```yaml
depends_on:
  - TASK-001
  - TASK-002
```

Tasks with unmet dependencies cannot be claimed (stay in READY).

## Body Structure

```markdown
## Objective
Single sentence describing what "done" looks like.

## Acceptance Criteria
- [ ] Criterion 1 (specific, verifiable)
- [ ] Criterion 2 (measurable outcome)
- [ ] Criterion 3

## Context
Background information, constraints, related files, links.

## Implementation Notes
<!-- Worker fills during implementation -->

## QA Report
<!-- Validator fills with validation results -->
```

## Complete Example

```markdown
---
id: TASK-001
title: Implement player jump mechanic
priority: high
created: 2026-01-13T10:00:00Z

assigned_to: null
qa_attempts: 0

started_at: null
submitted_at: null
completed_at: null

worktree: null
branch: null
base_sha: null

affects:
  - src/player/jump.rs
  - src/player/mod.rs
affects_globs:
  - src/player/**
  - tests/player/**
must_not_touch:
  - src/enemy/**
  - src/networking/**

depends_on: []
tags: [feature, player, physics]
---

## Objective
Player can jump when grounded, with configurable height and gravity.

## Acceptance Criteria
- [ ] Jump triggered on spacebar when `is_grounded` is true
- [ ] Jump height configurable via `JUMP_HEIGHT` constant
- [ ] Gravity applied correctly during jump arc
- [ ] Cannot double-jump (grounded check enforced)
- [ ] Unit tests cover edge cases

## Context
- Physics system in `src/physics/` - use existing gravity constant
- Player state machine in `src/player/state.rs`
- See design doc: docs/player-movement.md

## Implementation Notes

## QA Report
```

## Forward Compatibility

Unknown YAML fields are preserved when reading/writing task files. This allows adding custom fields without breaking the tool.

## Atomic Operations

Task files are written atomically:
1. Write to temporary file
2. Rename to target (atomic on most filesystems)

This prevents partial writes on crash/interrupt.
