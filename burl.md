# PRD — Burl: Minimal File‑Driven eXecution Orchestrator (`burl`)

**Document status:** Active (V1 implemented; spec is source of truth)  
**Last updated:** 2026-01-13  
**Owner:** (you)  
**Target platform:** Local developer machines (Windows/macOS/Linux), Git repository required

Quick links:
- `README.md` — quick start + repo overview
- `ROADMAP.md` — current vs future features

---

## 1. Summary

`burl` (File-Driven eXecution) is a **minimal, file-based workflow orchestrator** for agentic coding and review pipelines. The workflow is expressed entirely as files and folders inside a repo:

- **Folders are status buckets** (READY/DOING/QA/DONE/BLOCKED)
- **Task files are durable state committed to Git**
- **A canonical “workflow worktree” is the single source of truth** for `.workflow/` (so multiple task worktrees never diverge)
- **Git worktrees isolate work**
- **Deterministic, diff-based validation gates** decide pass/fail—no “LLM judging LLM”

The system is intentionally “SQLite-like”: portable, inspectable, no server, no DB.

---

## 2. Problem Statement

When multiple AI agents (or humans) work concurrently:
- tasks get **double-claimed** (race conditions)
- branches/worktrees get messy
- QA becomes subjective (“looks done”)
- agents can leave partial work (TODOs/stubs)
- merges become risky (conflicts, hidden scope creep)

We need a **simple** workflow that:
- is race-safe
- isolates work
- enforces scope
- blocks stubs deterministically
- merges conservatively

---

## 3. Goals (V1)

### G1 — File/Folder workflow is the source of truth
- A repo clone + fetch + creating the workflow worktree (via `burl init` or `git worktree add .burl burl`) is sufficient to understand pipeline state via `ls .burl/.workflow/...` (default layout).
- Workflow state is stored on a dedicated Git branch (default `burl`) so it can be paused and resumed on another machine.

### G2 — Race-safe claiming and transitions
- No double-claim, no partial transition states.
- Operations are transactional with rollback where practical.

### G3 — Deterministic, reproducible validation
- Validation runs on **diff hunks** and **declared scope**, using a stored `base_sha`.

### G4 — Safe Git integration with worktrees
- Work is performed on per-task branches + isolated worktrees.
- Merges are conservative by default (rebase + `--ff-only`).

### G5 — Human-friendly, tool-agnostic
- Tasks are markdown with YAML frontmatter.
- Any agent tool can participate by reading files and running shell commands.

### G6 — Portable pause/resume
- You can wind down all workers, push workflow state + task branches, and resume on another machine without losing status/history.

---

## 4. Non‑Goals (V1)

- No long-running daemon/service.
- No distributed lock service.
- No networked agent management or infra.
- No fancy UI requirements (TUI/“watch mode” is **V2**).
- No PR creation automation (can be added later).

---

## 5. Personas & Use Cases

### Persona A — Solo developer using agents
- Wants to dispatch multiple “worker” agents safely in parallel.
- Wants deterministic QA gates so they don’t babysit.

### Persona B — Human QA / Reviewer
- Needs to validate quickly and reject with actionable reasons.

### Persona C — Automation runner
- Later wants `burl watch` (V2) to auto-claim/validate.

---

## 6. Success Criteria

V1 is “done” when:

1. `burl init` creates a working structure (workflow branch + `.burl/` worktree + `.worktrees/`)  
2. `burl add` creates a valid task file  
3. `burl claim` is **race-safe** and creates worktree + branch  
4. `burl submit` validates scope + stubs (diff-based) and moves to QA  
5. `burl validate` runs all configured checks (scope/stubs/build/tests)  
6. `burl approve` rebases + merges `--ff-only`, cleans up worktree/branch, moves to DONE  
7. `burl reject` returns task with reasons, increments attempts, preserves branch/worktree  
8. Multiple concurrent claim attempts never double-claim a task  
9. Failures mid-transition do not corrupt workflow state (rollback or safe leftovers)  
10. Everything is inspectable without the tool (folders + markdown)

---

## 7. High‑Level Architecture

### 7.1 Directory structure

```
.burl/                       # canonical workflow worktree (Git branch: burl)
  .workflow/               # workflow state (tracked, durable)
    READY/
    DOING/
    QA/
    DONE/
    BLOCKED/
    locks/                 # untracked, machine-local
    events/                # tracked (NDJSON), append-only
    config.yaml
    agents.yaml

.worktrees/
  task-001-<slug>/
  task-002-<slug>/
```

- `.burl/` is the only location `burl` reads/writes workflow state. Commands may be invoked from any directory/worktree; the tool resolves the repo root and then targets `.burl/.workflow/`.
- `locks/` is intentionally **not** committed (locks are machine-local and would cause “phantom” locks when moving machines).
- `events/` is committed (audit/recovery across machines). See “Logging & Observability”.

### 7.2 “Filesystem is truth” principle

- The definitive status is the **folder** containing the task file inside the canonical workflow worktree (`.burl/.workflow/...` by default).
- Frontmatter fields mirror and support folder status, but folder wins if inconsistent.
- `burl doctor` (optional) can detect and repair inconsistencies.

---

## 8. Data Model

### 8.1 Task file format

Filename: `TASK-{id}-{slug}.md`

```markdown
---
id: TASK-001
title: Implement player jump
priority: high
created: 2026-01-13T10:00:00Z

# Ownership/attempts
assigned_to: null
qa_attempts: 0

# Lifecycle timestamps
started_at: null
submitted_at: null
completed_at: null

# Git/worktree state (populated on claim)
worktree: null
branch: null
base_sha: null   # REQUIRED for diff-based validation (set on claim)

# Scope control
affects:
  - src/player/jump.rs
  - src/player/mod.rs
affects_globs:
  - src/player/**        # optional; recommended for tasks that may add files
must_not_touch:
  - src/enemy/**
  - src/networking/**

# Dependency control
depends_on: []

# Freeform
tags: [feature, player]
---

## Objective
Single sentence describing what "done" looks like.

## Acceptance Criteria
- [ ] Criterion 1 (specific, verifiable)
- [ ] Criterion 2
- [ ] Criterion 3

## Context
Relevant notes/links, constraints, file references.

## Implementation Notes
<!-- Worker fills -->

## QA Report
<!-- Validator fills (tool can append) -->
```

**Notes:**
- `base_sha` is mandatory after `claim` and is used for diff-based checks at `submit`/`validate`. `approve` rebases onto the current upstream and must validate against the rebased base (see “Deterministic Validation”).
- `affects` is a list of explicit paths.
- `affects_globs` allows controlled expansion (directories/globs) and supports new files.
- Scope checks treat **allowed paths** as: `affects` ∪ `affects_globs`.
- `worktree` is a best-effort local path. On a different machine, `burl` may recreate a task worktree at the configured worktree root and update/override the recorded path.

### 8.2 Configuration files

#### `.burl/.workflow/config.yaml` (V1; default layout)

```yaml
max_parallel: 3

# Workflow state (durable, in Git)
workflow_branch: burl
workflow_worktree: .burl
workflow_auto_commit: true        # commit workflow state changes after transitions
workflow_auto_push: false         # enable for “always resumable elsewhere”

# Git behavior
main_branch: main
remote: origin
merge_strategy: rebase_ff_only   # rebase_ff_only | ff_only | manual
push_main_on_approve: false
push_task_branch_on_submit: false # enable if you want DOING/QA work resumable elsewhere by default

# Concurrency/locks
lock_stale_minutes: 120          # stale lock recovery threshold
use_global_claim_lock: true      # optional; redundant if workflow lock is required

# QA policy
qa_max_attempts: 3
auto_priority_boost_on_retry: true

# Validation hooks
build_command: "cargo test"      # empty string disables build/test validation

# Stub patterns are applied to ADDED lines in diff hunks (not whole files)
stub_patterns:
  - "TODO"
  - "FIXME"
  - "XXX"
  - "HACK"
  - "unimplemented!"
  - "todo!"
  - 'panic!\s*\(\s*"not implemented'
  - "NotImplementedError"
  - "raise NotImplemented"
  - '^\s*pass\s*$'
  - '^\s*\.\.\.\s*$'

stub_check_extensions: [rs, py, ts, js, tsx, jsx]

# Conflict policy when declared scopes overlap
conflict_policy: fail            # fail | warn | ignore
```

**Notes:**
- `workflow_branch` / `workflow_worktree` are bootstrap parameters. Since this config lives on the workflow branch, changing these requires explicit init/migration behavior (V1 may treat `burl` + `.burl` as fixed defaults).

#### `.burl/.workflow/agents.yaml` (V2; optional in V1; default layout)

May exist in V1 but execution is not required for the core tool. Keep it stable for future automation.

---

## 9. State Machine & Transitions

### 9.1 Buckets

- **READY**: claimable tasks
- **DOING**: claimed tasks with worktrees/branches
- **QA**: submitted tasks awaiting validation
- **DONE**: approved and merged
- **BLOCKED**: dependencies unmet or external constraints

### 9.2 Transition diagram

```
         claim
READY ─────────► DOING ───────► QA ───────► DONE
   ▲              │ submit       │ approve
   │              │              │
   └──── reject ◄─┘              └── reject ─► READY
            (from QA)
```

### 9.3 Transition rules (high-level)

- `claim`: READY → DOING (creates branch + worktree, sets `base_sha`)
- `submit`: DOING → QA (runs scope+stub gates, records `submitted_at`)
- `validate`: QA (runs deterministic checks; no transition)
- `approve`: QA → DONE (rebase + `--ff-only` merge; cleanup)
- `reject`: QA → READY (increments `qa_attempts`, appends reason, preserves branch/worktree)

Optional:
- `block`: READY/DOING/QA → BLOCKED (requires reason)
- `unblock`: BLOCKED → READY (if deps satisfied)

---

## 10. Concurrency & Atomicity (Critical Requirements)

This section defines **the safety model**. Without these guarantees, the workflow will be unreliable.

### 10.1 Atomic filesystem operations

**Requirement A1 — Prefer atomic renames (same filesystem)**
- Folder transitions and atomic writes rely on `rename()` semantics when possible.
- The canonical workflow state directory (default: `.burl/.workflow/`) should live on a single filesystem/volume for atomicity.
- Task moves MUST attempt `fs::rename(src, dst)` first (atomic on a single filesystem).
- If `rename` fails with `EXDEV` (“Invalid cross-device link”), fall back to a copy+delete move (non-atomic) and treat it as degraded safety: a crash mid-move can temporarily duplicate bucket state; `burl doctor --repair` can reconcile it.

**Requirement A2 — Atomic task file updates**
- Update task files via: write to temp file in same directory → `rename()` over original.
- Never write frontmatter in-place.
- Implementation must be cross-platform safe:
  - POSIX: `rename` replaces atomically
  - Windows: use a “replace file” strategy (write temp, then replace existing)

### 10.2 Locking model

**Goal:** prevent double-claim and partial transitions.

**Lock files live in**: the canonical workflow worktree under `.workflow/locks/` (default: `.burl/.workflow/locks/`).

#### 10.2.0 Workflow lock (required)
Because workflow state is committed to Git (workflow branch), `burl` must prevent concurrent commands from:
- racing on workflow file moves/writes
- racing on workflow-branch commits (Git index/HEAD mutations)

To do this, any command that **mutates workflow state** MUST acquire a global workflow lock:
- Create `.burl/.workflow/locks/workflow.lock` (default layout) using **create_new** semantics.
- If lock exists → operation fails with “workflow locked”.

This lock is intended to be held for the **critical section** that edits `.burl/.workflow/**` and commits the workflow branch (not for long-running builds/tests).

#### 10.2.1 Per-task lock (required)
- Before any transition involving a task:
  - Create `.burl/.workflow/locks/TASK-001.lock` (default layout) using **create_new** semantics.
  - If lock exists → operation fails with “task locked”.
- Lock file body includes metadata:
  - `owner` (string; e.g., hostname/user/agent name)
  - `pid` (optional)
  - `created_at` timestamp
  - `action` (claim/submit/approve/etc.)

#### 10.2.2 Global claim lock (optional but recommended for “claim next”)
When `burl claim` is called without a task ID (auto-pick):
- Acquire `.burl/.workflow/locks/claim.lock` (default layout) to serialize selection from READY.
- This prevents two claimers selecting the same “next task”.

**Config:** `use_global_claim_lock: true`

### 10.3 Stale lock recovery

Locks can persist if a process crashes.

- A lock is considered stale if `now - created_at > lock_stale_minutes`.
- Provide recovery command(s):
  - `burl lock list`
  - `burl lock clear TASK-001` (requires `--force`)
  - `burl doctor --repair` (optional; can clear stale locks)

**Policy:** Never auto-clear locks without explicit user action.

### 10.4 Transactional transitions (rollback)

Each command that mutates state is a transaction:

- Acquire lock(s)
- Validate preconditions (including that the workflow worktree has no unexpected tracked modifications)
- Perform actions in safe order
- If any step fails, rollback or leave safe artifacts
- Release lock(s)

**Safe ordering principle:** Update metadata first (atomically), then move file bucket, then append logs, then commit workflow state—so folder status and committed state match the latest metadata.


### 10.5 Correctness guarantees (what “safe” means)

`burl` can be made **race-safe** and **fail-safe** under clear assumptions:

**Assumptions**
- The canonical workflow worktree exists (default path `.burl/`) and all `burl` commands resolve and operate on its `.workflow/` directory.
- Workflow state (`.burl/.workflow/**`) and local task worktrees (`.worktrees/**`) ideally live on the **same filesystem/volume** (so rename/replace is atomic, and worktree paths are stable).
- All workflow state mutations happen through `burl` commands (no manual folder shuffling mid-operation).
- Locks are implemented with **create_new** semantics and respected by all actors (agents/humans).

**Guarantees (under the assumptions, and when atomic renames succeed)**
- **At-most-one mutator per task:** per-task lock files ensure only one process can claim/submit/validate/approve/reject a given task at a time.
- **No split-brain workflow state:** all workflow reads/writes target the canonical workflow worktree (`.burl/.workflow/**` by default), so multiple task worktrees never diverge on bucket state.
- **No partial task-file writes:** task metadata updates use atomic replace (temp + rename), so frontmatter is never half-written.
- **Bucket state is never duplicated:** bucket moves use atomic rename, so a task cannot exist in two buckets simultaneously (except in degraded EXDEV fallback scenarios or manual edits).
- **Atomic dispatch (claim) is deterministic:** either a task ends up in DOING with recorded `base_sha` + branch/worktree, or it remains in READY; double-claim is prevented.
- **Merges are conservative:** default `rebase_ff_only` + `git merge --ff-only` prevents accidental merge commits or “best-effort” merges.

**Non-guarantees (by design)**
- Git conflicts can still happen at rebase time if two tasks touch the same code; `burl` ensures conflicts are surfaced and block merging safely.
- `burl` cannot prevent a user/agent from running arbitrary `git` commands in a worktree; it can only make the **happy-path commands** safe and auditable.
- Build/test commands can be nondeterministic (flaky tests); `burl` reports failures but cannot “guarantee green”.

### 10.6 Failure modes and recovery (it should fail *safe*)

Even with strong atomicity, processes can crash. The design goal is: **no silent corruption, and all failure states are recoverable**.

Common recoverable failures:
- **Crash while holding a lock:** leaves `.burl/.workflow/locks/TASK-xxx.lock` (or workflow lock).
  - Recovery: `burl lock list` → `burl lock clear TASK-xxx --force` (after verifying staleness).
- **Crash after creating branch/worktree but before bucket move:** may leave an orphan branch/worktree.
  - Recovery: `burl doctor` identifies orphan artifacts; `burl clean` or targeted cleanup removes them.
- **Crash after metadata write but before bucket move:** task remains in the old bucket with new metadata.
  - Recovery: `burl doctor --repair` can reconcile folder status with metadata (policy-driven, no destructive changes without flags).
- **Cross-filesystem rename/replace failure:** atomic rename may not work if directories are on different volumes (or on some mounts).
  - Recovery: move `.burl/` / `.worktrees/` onto the same filesystem for full atomicity. If you ran with degraded copy+delete moves, run `burl doctor --repair` to reconcile any duplicated bucket state.

Recommended recovery command (V1):
- `burl doctor` (read-only): report inconsistencies, stale locks, orphan worktrees/branches, missing `base_sha`, bucket/metadata mismatches.
- `burl doctor --repair --force`: apply **safe** repairs (e.g., clear stale locks, fix bucket placement) but never delete branches/worktrees without explicit cleanup flags.


---

## 11. Git Worktree & Branch Model

### 11.1 Workflow branch + canonical workflow worktree (source of truth)

Workflow state is tracked in Git on a dedicated branch and edited from a dedicated worktree so that:
- multiple task worktrees never diverge on workflow state
- workflow state can be pushed and resumed on another machine

**Defaults (configurable):**
- Workflow branch: `burl` (config `workflow_branch`)
- Workflow worktree path: `.burl/` (config `workflow_worktree`)

**Rules:**
- `.burl/.workflow/**` is the only authoritative workflow state.
- Any command that mutates workflow state edits files under `.burl/.workflow/**`, commits those changes to the workflow branch (config `workflow_auto_commit`), and may optionally push (config `workflow_auto_push`).
- The workflow branch is intended to be linear/fast-forward (no manual rebases/merges); `burl` assumes it can append commits safely.
- Task worktrees MUST NOT be treated as a source of truth for `.workflow/**` (they may not even have it).
- `.burl/.workflow/locks/**` is intentionally untracked/machine-local.
- The workflow branch/worktree is for workflow state only; product code changes must happen on task branches/worktrees.

### 11.2 Task naming conventions

- Branch: `task-001-player-jump` (configurable template)
- Worktree path: `.worktrees/task-001-player-jump/`

### 11.3 Base SHA

On `claim`, store:

- `base_sha = <remote>/<main_branch> HEAD` at the time of claiming

`base_sha` is the reference for pre-merge diff checks (`submit`/`validate`). On `approve`, the branch is rebased onto the current upstream and diff checks must be performed relative to the rebased base (see “Deterministic Validation”).

### 11.4 Diff commands

- Changed files: `git diff --name-only {base_sha}..HEAD`
- Changed hunks: `git diff -U0 {base_sha}..HEAD` (U0 = no context for precise “added lines” scanning)

### 11.5 Task worktree lifecycle

**On claim:**
1. Create branch at `{base_sha}`
2. Create worktree at `.worktrees/<task>/`

**On approve (default strategy `rebase_ff_only`):**
1. In task worktree: fetch remote main
2. Rebase task branch onto `origin/main`
   - if conflicts → reject (or move to BLOCKED) with reason
3. In repo root: `git checkout main`
4. `git merge --ff-only <task-branch>`
   - if fails → reject with reason
5. Optional: push main (if configured via `push_main_on_approve`)
6. Remove worktree and delete branch (configurable cleanup)

### 11.6 Reject behavior & reuse

On `reject`:
- Task returns to READY **but keeps branch/worktree paths in metadata**.
- Next `claim TASK-001` reuses the existing worktree if present and valid.
- If missing, recreate branch/worktree from `base_sha` or latest main depending on policy.

**Policy (V1 default):**
- Reuse existing branch/worktree if present.
- Do not silently change `base_sha` on reuse.

### 11.7 Pause/resume on another machine (portable workflow)

To wind down and resume elsewhere:
1. Ensure each task worktree is in a known state (recommended: commit WIP; no uncommitted changes).
2. Push task branches you want to resume elsewhere (configurable automation: `push_task_branch_on_submit`).
3. Push the workflow branch (configurable automation: `workflow_auto_push`) so READY/DOING/QA/DONE state and QA reports are available.
4. On the new machine: clone/fetch, run `burl init` (idempotent) to recreate the canonical workflow worktree, then recreate any missing task worktrees from their branches as needed.

---

## 12. Deterministic Validation

### 12.0 Diff base selection (important)

Diff-based checks must be computed against the correct base:
- For `burl submit` and `burl validate`: use the task’s stored `base_sha` (`{base_sha}..HEAD`).
- For `burl approve`: **rebase first**, then validate against the rebased base (`{remote}/{main_branch}..HEAD`, typically `origin/main..HEAD`).

### 12.1 Scope enforcement

**Allowed paths** = `affects` + `affects_globs`  
**Forbidden paths** = `must_not_touch` globs

**Rule S1 — No forbidden touch**
- If any changed file matches `must_not_touch` → fail.

**Rule S2 — Only allowed touch**
- Every changed file must match at least one allowed path or glob → fail otherwise.

**New files:** are allowed only if they match allowed globs or explicit allowed directories.

### 12.2 Stub detection (diff-based)

**Critical improvement:** stub patterns must be checked on **added lines only**, not whole files.

Algorithm:
1. Compute diff hunks from `{diff_base}..HEAD` (see “Diff base selection”).
2. For each added line (`+...`) in files with extensions in `stub_check_extensions`:
   - test against compiled regexes in `stub_patterns`
3. Any match → fail with exact file + line + matched content.

**Rationale:** prevents rejecting tasks due to pre-existing TODOs elsewhere in the file.

### 12.3 Build/Test validation

If `build_command` is non-empty:
- run in the worktree directory
- non-zero exit code → fail
- capture stdout/stderr summary into QA Report

**V1 recommendation:** run on `burl validate` and `burl approve`, optional on `burl submit`.

---

## 13. CLI Requirements (V1)

### 13.1 Commands

#### Setup
- `burl init`
  - create or attach the canonical workflow worktree (default: `.burl/` on branch `burl`)
  - create `.burl/.workflow/` buckets, `locks/`, `events/`, default config templates
  - ensure `.burl/.workflow/locks/` is untracked (gitignored) and exists locally
  - write `.burl/.workflow/.gitignore` with `locks/` so locks never get committed
  - create `.worktrees/` directory (local, untracked)
  - commit initial workflow scaffolding to the workflow branch (if enabled)
  - (recommended) add `.burl/` and `.worktrees/` to `.git/info/exclude` so `git status` stays clean without touching `main`

#### Task management
- `burl add "title" [--priority] [--affects ...] [--must-not-touch ...] [--depends-on ...] [--tags ...]`
  - creates task in `.burl/.workflow/READY/` and commits workflow state (if enabled)

- `burl status`
  - counts per bucket + highlights locked/stalled tasks

- `burl show TASK-001`
  - render task markdown and key metadata

#### Worker operations
- `burl claim [TASK-ID]`
  - if TASK-ID omitted: select next claimable task (global claim lock optional; workflow lock required for mutations)
  - creates/attaches task worktree + branch, writes `base_sha`, moves task READY → DOING in `.burl/.workflow/`
  - prints worktree path

- `burl submit [TASK-ID]`
  - runs scope+stub checks (diff-based) and requires at least one commit
  - writes `submitted_at`, moves DOING → QA in `.burl/.workflow/`

- `burl worktree [TASK-ID]`
  - prints recorded worktree path

#### QA operations
- `burl validate TASK-ID`
  - runs: scope + stub + build/test
  - appends structured results to “QA Report” section and/or writes `.burl/.workflow/events/...`

- `burl approve TASK-ID`
  - requires `burl validate` to pass (or runs validate internally)
  - rebases + merges (strategy-based)
  - cleans up worktree, moves to DONE, sets `completed_at`

- `burl reject TASK-ID --reason "..."`
  - increments attempts, appends reason, moves to READY

#### Locks & recovery
- `burl lock list`
- `burl lock clear TASK-ID --force`
- `burl doctor`                         # report stale locks, mismatches, orphan artifacts
- `burl doctor --repair --force`        # apply safe repairs (policy-driven)
- `burl clean`                          # remove completed worktrees, orphan worktrees (with confirmation flags)

### 13.2 Return codes
- `0`: success
- `1`: user error (bad args, invalid state)
- `2`: validation failure (scope/stubs/build)
- `3`: git operation failure
- `4`: lock acquisition failure

---

## 14. Transition Semantics (Detailed, Atomic)

### 14.1 `burl claim` (transaction)

**Locks:**
- if TASK-ID omitted: acquire `claim.lock` (optional, config) to pick “next” from READY
- always acquire `TASK-001.lock`
- acquire `workflow.lock` for the critical section that mutates `.burl/.workflow/**` and commits workflow state

**Steps:**
1. Verify task file exists in READY (in the workflow worktree) and parses.
2. Verify dependencies are DONE; else move to BLOCKED (optional) and fail.
3. Check conflicts with DOING tasks (declared overlap):
   - if `conflict_policy=fail` → fail
   - if warn → print warning, allow
4. Determine `base_sha = origin/main HEAD` (fetch first).
5. Create branch at `base_sha` if not existing (or reuse if already exists and allowed).
6. Create/attach worktree (if exists, validate it points to branch).
7. Atomically update task frontmatter:
   - set `assigned_to`, `started_at`, `branch`, `worktree`, `base_sha`
8. Atomically move task file: READY → DOING.
9. Append event log entry.
10. Commit workflow branch (if enabled) so the claim is durable across machines.
11. Release locks.

**Rollback rules:**
- If branch created but worktree creation fails → delete branch (if created in this transaction).
- If metadata updated but move fails → revert metadata (or leave but report; folder still READY).
- If move succeeds but later step fails → treat as DOING and require `burl doctor` (should be rare; move is last).

### 14.2 `burl submit`

**Locks:**
- acquire `TASK.lock`
- acquire `workflow.lock` for the critical section that mutates `.burl/.workflow/**` and commits workflow state

**Steps:**
1. Verify task is in DOING and worktree exists.
2. Verify git branch matches task branch.
3. Verify `base_sha` exists.
4. Verify there is at least one commit vs `base_sha`.
5. Run scope + stub diff checks (fail → stay in DOING).
6. Atomically update task: set `submitted_at`.
7. Move DOING → QA.
8. Append event log entry.
9. Commit workflow branch (if enabled).
10. Release lock.

### 14.3 `burl validate`

- Acquire lock.
- Run validations (scope/stubs/build/test).
- Acquire `workflow.lock`, append structured report to QA Report section and log event, then commit workflow branch (if enabled).
- No bucket move.
- Release lock.

### 14.4 `burl approve`

**Locks:**
- acquire `TASK.lock`
- acquire `workflow.lock` for the critical section that mutates `.burl/.workflow/**` and commits workflow state

**Steps:**
1. Verify task is in QA.
2. Fetch `origin/main`.
3. Rebase task branch onto `origin/main` in worktree.
   - conflict → reject with “rebase conflict” (or move to BLOCKED)
4. Run `validate` against the rebased base (`origin/main..HEAD`) and fail fast on scope/stubs/build/test.
5. Merge `--ff-only` into local `main`.
   - fail → reject with “non-FF merge required”
6. Optional push.
7. Cleanup worktree + delete branch.
8. Atomically set `completed_at`.
9. Move QA → DONE.
10. Append event log entry.
11. Commit workflow branch (if enabled).
12. Release lock.

### 14.5 `burl reject`

- Acquire lock.
- Verify task is in QA.
- Increment `qa_attempts`; append reason to QA Report; optionally boost priority.
- Move QA → READY.
- Preserve branch/worktree paths (no cleanup by default).
- Append event log entry.
- Commit workflow branch (if enabled).
- Release lock.

**If `qa_attempts >= qa_max_attempts`:**
- default: move to BLOCKED with reason “max QA attempts reached” (configurable).

---

## 15. Logging & Observability

All logs live under the workflow state directory (`.burl/.workflow/` by default) and are committed to the workflow branch so they travel with the workflow when moving machines.

### 15.1 Event log format (NDJSON)

Append-only file (default layout): `.burl/.workflow/events/events.ndjson`  
Optional per-task logs: `.burl/.workflow/events/TASK-001.ndjson`

Example record:
```json
{"ts":"2026-01-13T10:35:11Z","task":"TASK-001","action":"claim","actor":"robert@HOST","details":{"branch":"task-001-player-jump","worktree":".worktrees/task-001-player-jump","base_sha":"abc123"}}
```

### 15.2 What must be logged (V1)
- init
- add
- claim
- submit
- validate (pass/fail + summary)
- approve
- reject
- lock clear
- clean

---

## 16. Error Handling (User‑actionable)

Errors must be:
- specific
- show the exact violating files/lines
- prescribe the fix

Examples:

**Lock error**
```
❌ Cannot claim TASK-001

Reason: task is locked by another process
Lock: .burl/.workflow/locks/TASK-001.lock (created 34m ago by worker@HOST)
```

**Scope violation**
```
❌ Scope violation

TASK-001 touched files outside allowed scope:
  ✗ src/networking/client.rs  (matches must_not_touch: src/networking/**)
  ✗ src/enemy/ai.rs           (not in affects/affects_globs)
Fix: revert these changes or widen scope in the task file.
```

**Stub violation (diff-based)**
```
❌ Stub patterns found in added lines

src/player/jump.rs:67  + unimplemented!()
src/player/jump.rs:45  + // TODO: implement cooldown
```

---

## 17. Security & Safety Considerations

- Path traversal: task IDs and filenames must be sanitized; never allow `../` paths.
- Command execution: if/when agent commands exist (V2), arguments must be templated safely; avoid shell injection by using argv arrays where possible.
- Never run destructive git commands without explicit flags (`--force`, `--prune`, etc.).

---

## 18. Testing Plan

### 18.1 Unit tests
- Task parsing/serialization preserves unknown fields (forward compat).
- Atomic write helper replaces file safely.
- Scope matching (globs + explicit paths).
- Stub matching (regex compilation + diff parsing).

### 18.2 Integration tests (real git repo in temp dir)
- init → add → claim → submit → validate → approve happy path
- concurrent claim attempts (spawn 2 processes) → exactly one succeeds
- crash simulation: create lock then exit → stale lock recovery workflow
- reject path preserves worktree and allows re-claim
- approve conflict path produces deterministic failure
- workflow durability: workflow branch contains committed state; recreate `.burl/` worktree and continue

### 18.3 Failure injection
- simulate `git worktree add` failure
- simulate rename failure (cross-device)
- simulate build command failure

---

## 19. Delivery Plan

### V1 Milestones (recommended)
1. **Workflow bootstrap**: workflow branch + `.burl/` worktree  
2. **Core file model**: init/add/show/status  
3. **Locking + atomic write utilities**  
4. **claim (transactional)** with branch/worktree + base_sha  
5. **diff-based validation**: scope + stubs  
6. **submit → QA**  
7. **validate** (add build/test hook)  
8. **approve** (rebase + ff-only) + cleanup  
9. **reject** + attempt policy  
10. **events log** + `clean` + lock tools  

### V2 (post-V1)
- `burl watch` automation loop
- `burl monitor` TUI dashboard
- `agents.yaml` execution + prompt generation
- more advanced conflict detection (actual diffs between tasks, not just declared overlaps)
- PR integration (GitHub/GitLab)

---

## 20. Open Decisions (with default recommendations)

1. **Reject destination after QA fail**
   - Default: QA → READY (preserve worktree/branch)
   - Alternative: QA → DOING (unassigned) to prevent immediate re-claim by another worker

2. **Cleanup on reject**
   - Default: keep worktree (fast iteration)
   - Alternative: remove worktree to reduce disk usage (require recreate)

3. **Conflict policy**
   - Default: `fail` when overlaps detected (safer)
   - Alternative: `warn` for small repos or wide globs

---

## Appendix A — Minimal Rust Module Layout (suggested)

```
burl/
  src/
    main.rs
    cli/
    core/
      task.rs
      config.rs
      workflow.rs
      lock.rs
      fs_atomic.rs
      git.rs
    validation/
      scope.rs
      stubs_diff.rs
      build.rs
    util/
      log.rs
```

---

## Appendix B — Hard Requirements Checklist

- [ ] Workflow state lives on a dedicated Git branch (default: `burl`)  
- [ ] Canonical workflow worktree exists (default: `.burl/`) and is the only workflow source of truth  
- [ ] Workflow state mutations are committed (and optionally pushed) after transitions  
- [ ] Global workflow lock serializes workflow mutations/commits  
- [ ] Per-task lock uses create_new semantics  
- [ ] Optional global claim lock for “claim next”  
- [ ] Task file writes are atomic (temp + replace)  
- [ ] Bucket moves are atomic rename  
- [ ] `base_sha` stored on claim and used for submit/validate diffs  
- [ ] Approve validates against rebased base (`origin/main..HEAD`)  
- [ ] Stub detection is diff-based (added lines only)  
- [ ] Scope enforcement supports globs and new files under allowed globs  
- [ ] Approve strategy is conservative (rebase + ff-only)  
- [ ] Append-only event log exists and is updated on all transitions  
