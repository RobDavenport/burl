---
id: TASK-003
title: Task file model (frontmatter) + atomic filesystem helpers
priority: high
depends_on: [TASK-001, TASK-002]
---

## Objective
Implement the task file read/write layer:
1) parse task markdown with YAML frontmatter
2) update frontmatter fields without losing content
3) write updates atomically (cross-platform)

## Context
Source of truth: `burl.md` section “Task file format” and “Atomic filesystem operations”.

## Requirements
### Task parsing/serialization
- Parse YAML frontmatter delimited by a leading `---` block.
- Preserve the markdown body exactly (including headings and whitespace).
- Preserve unknown frontmatter fields (forward-compatible).
- Provide helpers for common mutations:
  - set `assigned_to`, `started_at`, `submitted_at`, `completed_at`
  - set `branch`, `worktree`, `base_sha`
  - increment `qa_attempts`
  - append to “QA Report” section

### Atomic updates
- Never edit task files in-place.
- Must implement: write temp file in same directory → replace original using an atomic strategy.
- Cross-platform requirement:
  - POSIX: atomic replace via rename is acceptable
  - Windows: must use a replace-existing mechanism (do not rely on “delete then rename”)

## Acceptance Criteria
- [ ] Round-trip read → write preserves:
  - all unknown YAML keys
  - markdown body content
- [ ] Atomic replace helper does not leave partial files on crash (best-effort; temp file may remain).
- [ ] Unit tests cover parsing and atomic write behavior.

## Implementation Notes
- Use a “frontmatter + body” struct:
  - `frontmatter: serde_yaml::Value` (or a typed struct + `flatten` map for unknowns)
  - `body: String`
- Appending to “QA Report” should be deterministic:
  - if section exists, append below it
  - if not, add it at end with a heading
- Strong recommendation: implement a single, reusable `atomic_write(path, bytes)` helper and use it everywhere workflow state is mutated (tasks, config, events if rewritten, etc.).

## Test Plan
### Unit
- Parse task file with:
  - required fields present
  - extra unknown keys
  - Windows CRLF line endings
- Serialize and re-parse: equality on unknown keys + body.
- Atomic replace:
  - write file, replace with new content, ensure final content is correct

## Validation
- `cargo test`

