# Burl Architecture

> This is a **codebase map + invariants** for contributors (including LLM agents).
> For user-facing semantics and workflow behavior, see `burl.md`.

## TL;DR (mental model)

Burl is a file-based workflow orchestrator that stores durable state as:

- Task markdown files in status buckets (`READY/`, `DOING/`, `QA/`, `DONE/`, `BLOCKED/`)
- Workflow metadata in `.burl/.workflow/` (config, locks, events)
- Git branches + worktrees per task for isolation

Most commands follow the same shape:

1. Resolve `WorkflowContext` (repo root + canonical workflow worktree)
2. Acquire one or more locks (workflow/task/claim)
3. Read/validate config + task file
4. Perform git/worktree operations and/or update task files
5. Commit workflow state changes (when applicable)
6. Append an NDJSON event entry

## Quick lookups

| I want to… | Start here |
| --- | --- |
| Add or change a CLI command | `src/cli/mod.rs`, `src/commands/mod.rs`, `src/commands/<cmd>/` |
| Understand the task file format | `src/task/mod.rs` (model), `src/task/io.rs` (read/write) |
| Understand workflow buckets/indexing | `src/workflow.rs` |
| Understand locks + stale lock behavior | `src/locks/` |
| Understand branch/worktree naming | `src/git_worktree/naming.rs` |
| Understand agent execution | `src/agent/`, `src/commands/agent.rs`, `src/commands/watch.rs` |
| Change validation gates | `src/validate/` |
| Change audit/event logging | `src/events.rs` |
| Change exit code mapping | `src/exit_codes.rs`, `src/error.rs` |

## Entry point

- `src/main.rs` — parses `cli::Cli`, dispatches to `commands::dispatch`, maps errors to exit codes.

## Module map

### Core primitives

- `src/context.rs` — repo/workflow path resolution; exposes `WorkflowContext`.
- `src/workflow.rs` — bucket enumeration + ID/filename helpers; builds `TaskIndex`.
- `src/task/` — task file model (YAML frontmatter + markdown body) + mutation helpers.
- `src/error.rs` — error taxonomy (`BurlError`) and high-level categorization.

### Git & filesystem

- `src/git.rs` — wrapper around `git` invocations with captured stdout/stderr.
- `src/git_worktree/` — branch/worktree operations used by claim/submit/approve/clean.
- `src/task_git.rs` — validates recorded branch/worktree invariants before use in git ops.
- `src/fs/` — atomic writes and cross-platform move helpers.

### Workflow mechanics

- `src/locks/` — workflow/task/claim locks using exclusive file creation; RAII guards.
- `src/events.rs` — append-only NDJSON audit log in `.burl/.workflow/events/`.
- `src/config/` — `.burl/.workflow/config.yaml` parsing with defaults and forward-compatible fields.
- `src/diff/` — `git diff` parsing (changed files + added lines).
- `src/validate/` — deterministic gates:
  - `scope` — enforce `affects`/`affects_globs` and `must_not_touch`
  - `stubs` — detect incomplete code patterns in **added lines only**

### Agent execution (V2)

- `src/agent/config.rs` — parses `.burl/.workflow/agents.yaml` (profiles, defaults, prompt templates).
- `src/agent/prompt/` — task context extraction + prompt generation.
- `src/agent/dispatch/` — subprocess execution with timeout + output capture.

### Commands

- `src/commands/` — one module per command; `src/commands/mod.rs` dispatches from the CLI.
  - Lifecycle: `init`, `claim`, `submit`, `validate_cmd`, `approve`, `reject`
  - Agents: `agent` (manual dispatch), `watch --dispatch` (automation)
  - Ops/UX: `status`, `show`, `worktree`, `lock`, `doctor`, `clean`, `watch`, `monitor`

### Support

- `src/exit_codes.rs` — canonical exit code constants.
- `src/test_support.rs` — helpers for integration tests using temporary git repos.

## Key types

### `WorkflowContext` (`src/context.rs`)

- `repo_root: PathBuf` — main git worktree root
- `workflow_worktree: PathBuf` — workflow worktree path (default `.burl/`)
- `workflow_state_dir: PathBuf` — workflow state directory (default `.burl/.workflow/`)
- `locks_dir: PathBuf` — lock files directory
- `worktrees_dir: PathBuf` — task worktrees parent directory

Common methods: `resolve()`, `ensure_initialized()`, `bucket_path()`, `config_path()`.

### `TaskFile` (`src/task/mod.rs`)

- `frontmatter: TaskFrontmatter` — parsed YAML metadata
- `body: String` — markdown body after frontmatter

Common methods: `parse()`, `serialize()`, `read()`, `write()`, plus mutation helpers.

### `TaskFrontmatter` (`src/task/mod.rs`)

Core fields:

- Identity: `id`, `title`, `priority`, `tags`
- Timestamps: `created`, `started_at`, `submitted_at`, `completed_at`
- Assignment: `assigned_to`, `qa_attempts`, `depends_on`
- Git metadata: `branch`, `worktree`, `base_sha`
- Validation: `affects`, `affects_globs`, `must_not_touch`
- Agent assignment: `agent`
- Forward compatibility: `extra` (unknown fields preserved)

### `TaskIndex` (`src/workflow.rs`)

- `tasks: HashMap<String, TaskInfo>` — map of task ID → info
- `max_number: u32` — highest task number seen

Common methods: `build()`, `find()`, `tasks_in_bucket()`, `next_number()`.

### `LockGuard` (`src/locks/guard.rs`)

RAII guard for workflow/task/claim locks. Releases on drop.

### `Event` (`src/events.rs`)

Append-only NDJSON record with timestamp, action, actor, optional task id and JSON details.

## On-disk workflow layout (canonical)

All workflow state lives under the workflow worktree (default `.burl/`):

- Buckets: `.burl/READY/`, `.burl/DOING/`, `.burl/QA/`, `.burl/DONE/`, `.burl/BLOCKED/`
- Config (committed): `.burl/.workflow/config.yaml`
- Agents config (committed): `.burl/.workflow/agents.yaml`
- Prompts (committed): `.burl/.workflow/prompts/*.md`
- Events (committed): `.burl/.workflow/events/events.ndjson`
- Locks (untracked): `.burl/.workflow/locks/*.lock`
- Agent logs (untracked): `.burl/.workflow/agent-logs/<TASK-ID>/{stdout.log,stderr.log}`
- Task worktrees (untracked): `{repo_root}/.worktrees/task-<NNN>-<slug>/`

## Naming + invariants

- Bucket/task naming conventions are implemented in `src/workflow.rs` and `src/git_worktree/naming.rs`.
- Never trust recorded `branch`/`worktree` blindly: validate via `src/task_git.rs` before use.
- Determinism guardrail: scope/stub checks must remain **diff-based** (no full-file scanning).

## Dependency direction (high level)

```
src/main.rs
  ├─> src/cli/
  ├─> src/commands/
  └─> src/error.rs + src/exit_codes.rs

src/commands/*
  ├─> src/context.rs + src/workflow.rs + src/task/
  ├─> src/locks/ + src/events.rs + src/config/
  ├─> src/validate/ + src/diff/
  ├─> src/git_worktree/ + src/task_git.rs + src/git.rs
  └─> src/fs/
```

Rule of thumb: core modules (`context/workflow/task/...`) should not depend on `commands`.

## Flow example: `burl claim`

1. Parse args in `src/main.rs` via `cli::Cli`
2. Dispatch to `commands::claim::cmd_claim()`
3. Resolve `WorkflowContext` and ensure workflow exists
4. Acquire `claim.lock` (optional global claim serialization)
5. Build `TaskIndex` and select a READY task
6. Acquire `workflow.lock` for state mutation
7. Determine `base_sha`
8. Create task branch + worktree via `src/git_worktree/`
9. Read and mutate the task file (`TaskFile`) frontmatter
10. Move task file READY → DOING
11. Commit workflow state to git (when applicable)
12. Append a `claim` event to the audit log
13. Release locks (RAII drop)

## Validation gates (where enforced)

- Claim: task is READY, deps satisfied, no lock conflicts
- Submit: scope + stub validation (diff-based)
- Validate: scope + stubs + optional build/test commands (config-driven)
- Approve: rebase to main, rerun validation, fast-forward merge, then DONE

## Testing

- Unit tests live alongside modules (`#[cfg(test)]`).
- Integration tests use `src/test_support.rs` to create temporary git repos.
