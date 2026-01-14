# Burl Architecture

This document describes the current codebase structure, key types, and module dependencies.

## Entry Point

- `main.rs` - CLI entry point. Parses arguments via `cli::Cli`, dispatches to command handlers, and maps errors to exit codes.

## Top-Level Modules

### Core Primitives

- **`context`** - Repository and workflow context resolution. Finds the git repo root from any working directory, resolves canonical workflow worktree paths, and provides `WorkflowContext` with all workflow file locations.

- **`workflow`** - Task index and bucket operations. Enumerates task files across buckets, validates task IDs, generates task filenames, and provides `TaskIndex` for querying tasks by ID or bucket.

- **`task`** - Task file model (YAML frontmatter + markdown body). Parses and serializes task files with forward-compatible field preservation. Provides `TaskFile` and `TaskFrontmatter` types.

- **`error`** - Error types and exit code mapping. Uses `thiserror` for error definitions. Maps `BurlError` variants to exit codes (1=user error, 2=validation failure, 3=git failure, 4=lock failure).

### Git & Filesystem

- **`git`** - Git command runner. Safe wrapper around git commands with captured stdout/stderr. Returns `GitOutput` on success or `BurlError::GitError` on failure.

- **`git_worktree`** - Git worktree and task branch operations. Creates/removes worktrees, manages task branches, fetches from remote, determines base_sha. Used by claim, approve, clean commands.

- **`task_git`** - Task git/worktree invariant validation. Validates recorded branch names and worktree paths follow conventions before use in git operations.

- **`fs`** - Filesystem utilities. Provides atomic writes for workflow state integrity and cross-platform file moves.

### Workflow Mechanics

- **`locks`** - Locking subsystem. Implements workflow.lock (global), task locks (per-task), and claim.lock (optional). Uses RAII guards for automatic cleanup. Lock files are JSON with owner, pid, timestamp, and action metadata.

- **`events`** - Event logging (append-only audit log). Writes NDJSON events to `.burl/.workflow/events/events.ndjson` for workflow actions (init, claim, submit, approve, etc.). Events logged while holding workflow.lock.

- **`config`** - Configuration model. Represents `.burl/.workflow/config.yaml` with forward-compatible parsing, defaults, and validation. Defines `Config`, `MergeStrategy`, `ConflictPolicy` types.

- **`diff`** - Diff parsing primitives. Parses `git diff` output to extract changed files and added lines for scope validation and stub detection. Handles new files, renames, and hunk headers.

- **`validate`** - Validation checks. Two submodules:
  - `scope` - Enforces that changes are within allowed paths (affects/affects_globs) and not in forbidden paths (must_not_touch)
  - `stubs` - Detects incomplete code patterns in added lines only (deterministic, diff-based)

### Commands

- **`commands`** - Command dispatcher and implementations. Routes CLI commands to handlers. Each command is in a submodule with tests.

- **`cli`** - CLI argument parsing. Uses clap derive macros. Defines command structure and arguments.

### Support

- **`exit_codes`** - Exit code constants (0=success, 1=user error, 2=validation failure, 3=git failure, 4=lock failure).

- **`test_support`** - Test utilities (creates temporary git repos for testing).

## Key Types

### `WorkflowContext` (context.rs)
- `repo_root: PathBuf` - Main git worktree root
- `workflow_worktree: PathBuf` - Workflow worktree path (`.burl/`)
- `workflow_state_dir: PathBuf` - Workflow state directory (`.burl/.workflow/`)
- `locks_dir: PathBuf` - Lock files directory
- `worktrees_dir: PathBuf` - Task worktrees parent directory

Methods: `resolve()`, `ensure_initialized()`, `bucket_path()`, `config_path()`, etc.

### `TaskFile` (task/mod.rs)
- `frontmatter: TaskFrontmatter` - Parsed YAML metadata
- `body: String` - Markdown content after frontmatter

Methods: `parse()`, `serialize()`, `read()`, `write()`, mutation helpers.

### `TaskFrontmatter` (task/mod.rs)
Core fields: `id`, `title`, `priority`, `created`, `assigned_to`, `qa_attempts`, `started_at`, `submitted_at`, `completed_at`, `worktree`, `branch`, `base_sha`, `affects`, `affects_globs`, `must_not_touch`, `depends_on`, `tags`, `extra` (forward compatibility).

### `TaskIndex` (workflow.rs)
- `tasks: HashMap<String, TaskInfo>` - Map of task ID to info
- `max_number: u32` - Highest task number seen

Methods: `build()`, `find()`, `tasks_in_bucket()`, `next_number()`.

### `LockGuard` (locks/guard.rs)
RAII guard for workflow, task, and claim locks. Automatically releases lock on drop. Prevents double-release via Drop flag.

### `Event` (events.rs)
- `ts: DateTime<Utc>` - Event timestamp
- `action: EventAction` - What happened (init, claim, submit, etc.)
- `actor: String` - Who performed the action
- `task: Option<String>` - Task ID if task-specific
- `details: Option<Value>` - Freeform JSON details

### `BurlError` (error.rs)
Variants: `NotImplemented`, `UserError`, `ValidationError`, `GitError`, `LockError`. Each maps to a specific exit code.

## Module Dependencies

```
main.rs
  ├─> cli (argument parsing)
  ├─> commands (dispatcher)
  └─> error, exit_codes

commands/
  ├─> context (workflow resolution)
  ├─> workflow (task index)
  ├─> task (task file model)
  ├─> locks (locking)
  ├─> events (audit logging)
  ├─> config (configuration)
  ├─> validate (scope, stubs)
  ├─> diff (git diff parsing)
  ├─> git_worktree (worktree operations)
  ├─> task_git (invariant validation)
  ├─> git (git runner)
  └─> fs (atomic writes)

context
  ├─> git (repo root detection)
  └─> error

workflow
  ├─> context
  └─> error

task
  ├─> error
  └─> fs (atomic writes)

locks
  ├─> context
  ├─> config
  └─> error

validate
  ├─> diff (changed files, added lines)
  ├─> config (stub patterns)
  └─> error

git_worktree
  ├─> context
  ├─> git
  └─> error
```

## Command Flow Example: `burl claim`

1. `main.rs` parses args via `cli::Cli`
2. `commands::dispatch()` routes to `claim::cmd_claim()`
3. Claim resolves `WorkflowContext` and validates workflow exists
4. Acquires `claim.lock` (global claim serialization)
5. Builds `TaskIndex` to find task in READY bucket
6. Acquires `workflow.lock` for state mutation
7. Fetches from remote, determines `base_sha`
8. Creates task branch and worktree via `git_worktree`
9. Reads task file via `TaskFile::read()`
10. Updates frontmatter (assigned_to, started_at, branch, worktree, base_sha)
11. Moves task file from READY to DOING bucket
12. Commits workflow state to git
13. Appends claim event to audit log
14. Releases locks (RAII drop)
15. Returns success

## State Storage

All workflow state is stored in the canonical workflow worktree (`.burl/.workflow/`):

- **Buckets**: `READY/`, `DOING/`, `QA/`, `DONE/`, `BLOCKED/` - contain task markdown files
- **Config**: `config.yaml` - workflow configuration (committed)
- **Locks**: `locks/*.lock` - lock files (untracked)
- **Events**: `events/events.ndjson` - audit log (committed)
- **Task worktrees**: `{repo_root}/.worktrees/task-NNN-slug/` - isolated work areas (untracked)

## Validation Gates

- **Claim**: Task must be in READY, not locked, dependencies satisfied
- **Submit**: Scope validation (affects/must_not_touch), stub detection
- **Validate**: Same as submit, plus optional build/test commands
- **Approve**: Re-runs validation after rebase to main, fast-forward merge

## Locking Strategy

- **workflow.lock**: Global lock for any workflow state mutation (moving tasks, updating metadata). Acquired during commit phase.
- **task locks**: Per-task lock to prevent concurrent work on same task. Acquired at start of claim/submit/approve/reject.
- **claim.lock**: Optional global lock to serialize claims (prevents race when multiple agents claim simultaneously).

All locks use exclusive file creation (create_new) and contain JSON metadata for debugging stale locks.

## Testing

Most modules have `#[cfg(test)] mod tests` with unit tests. Integration tests use `test_support::create_test_repo()` to create temporary git repos. Commands validate behavior with mock git repos.
