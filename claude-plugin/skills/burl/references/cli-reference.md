# Burl CLI Reference

Complete documentation for all Burl commands.

## Initialization

### `burl init`

Initialize Burl workflow in a Git repository (idempotent).

```bash
burl init
```

**Actions:**
- Creates workflow worktree at `.burl/` on branch `burl`
- Creates bucket directories: READY/, DOING/, QA/, DONE/, BLOCKED/
- Creates `config.yaml` template
- Creates `agents.yaml` template (agent config)
- Creates `prompts/` (tracked) and `agent-logs/` (untracked) directories
- Creates `.worktrees/` directory at repo root

---

## Task Management

### `burl add <title>`

Create a new task in the READY bucket.

```bash
burl add "Implement player jump" \
  --priority high \
  --affects src/player/jump.rs \
  --affects-globs "src/player/**" \
  --must-not-touch "src/enemy/**" \
  --depends-on TASK-001 \
  --tags feature,player
```

**Arguments:**
| Flag | Description |
|------|-------------|
| `--priority` | Priority level: high, medium (default), low |
| `--affects` | Exact file paths allowed (comma-delimited) |
| `--affects-globs` | Glob patterns for affected paths |
| `--must-not-touch` | Forbidden paths (comma-delimited) |
| `--depends-on` | Task IDs this depends on |
| `--tags` | Tags for categorization |

### `burl status`

Display workflow status summary.

```bash
burl status
```

**Output:**
- Task counts per bucket
- Locked tasks and stale locks
- Tasks with high QA attempts
- Stalled tasks

### `burl show <task-id>`

Display details of a specific task.

```bash
burl show TASK-001
```

---

## Worker Commands

### `burl claim [task-id]`

Claim a task for work (READY → DOING).

```bash
# Claim specific task
burl claim TASK-001

# Claim next available (by priority, then ID)
burl claim
```

**Actions:**
- Acquires per-task lock
- Checks dependencies are satisfied
- Creates Git branch and worktree
- Sets `base_sha` for validation reference
- Moves task to DOING bucket
- Commits workflow state

### `burl submit [task-id]`

Submit task for QA review (DOING → QA).

```bash
# Submit specific task
burl submit TASK-001

# Submit current worktree's task
burl submit
```

**Validation performed:**
1. Scope validation (changed files vs allowed)
2. Stub detection (TODO/unimplemented in added lines)

### `burl worktree <task-id>`

Show the worktree path for a task.

```bash
cd $(burl worktree TASK-001)
```

---

## Agent Commands

### `burl agent list`

List configured agents from `.burl/.workflow/agents.yaml`.

```bash
burl agent list
```

### `burl agent run <task-id> [--agent <id>] [--dry-run]`

Dispatch an agent to work on a claimed task (task must be in DOING).

```bash
# Run the default agent for a task
burl agent run TASK-001

# Override which agent profile to use
burl agent run TASK-001 --agent claude-code

# Preview what would run without executing
burl agent run TASK-001 --dry-run
```

---

## QA Commands

### `burl validate <task-id>`

Run validation checks without changing status.

```bash
burl validate TASK-001
```

**Checks:**
- Scope validation
- Stub pattern detection
- Validation commands (legacy `build_command`, or `validation_profiles` if configured)

### `burl approve <task-id>`

Approve task and merge to main (QA → DONE).

```bash
burl approve TASK-001
```

**Actions:**
1. Fetches origin/main and rebases task branch
2. Re-validates against rebased base
3. Fast-forward merges to main
4. Cleans up worktree and branch
5. Moves task to DONE

### `burl reject <task-id> --reason <reason>`

Reject task and return to work.

```bash
burl reject TASK-001 --reason "Scope violation: touched src/enemy/**"
```

**Actions:**
- Increments `qa_attempts` counter
- Appends rejection reason to task
- Returns to READY (or BLOCKED if max attempts exceeded)
- Preserves branch and worktree

---

## Recovery Commands

### `burl doctor [--repair] [--force]`

Diagnose and repair workflow health issues.

```bash
# Diagnose only
burl doctor

# Repair issues
burl doctor --repair

# Force repairs without confirmation
burl doctor --repair --force
```

**Detects:**
- Stale locks
- Orphan worktrees
- Metadata inconsistencies

### `burl clean [--completed] [--orphans] [--yes]`

Clean up worktrees.

```bash
# Clean completed task worktrees
burl clean --completed

# Clean orphan worktrees
burl clean --orphans

# Skip confirmation
burl clean --completed --yes
```

### `burl lock <action>`

Manage locks.

```bash
# List all locks
burl lock list

# Clear specific lock
burl lock clear TASK-001 --force

# Clear workflow lock
burl lock clear workflow --force
```

---

## Automation Commands

### `burl watch [options]`

Automation loop for claiming and QA processing.

```bash
burl watch --approve --interval-ms 2000
```

**Options:**
| Flag | Description |
|------|-------------|
| `--interval-ms` | Poll interval (default: 2000) |
| `--claim` | Auto-claim READY tasks (default: true) |
| `--qa` | Process QA tasks (default: true) |
| `--approve` | Auto-approve passing tasks |
| `--dispatch` | Auto-dispatch agents for newly-claimed tasks (requires `agents.yaml`) |
| `--once` | Single iteration then exit |

### `burl monitor`

Live dashboard for workflow status.

```bash
burl monitor --interval-ms 1000 --tail 10
```

**Options:**
| Flag | Description |
|------|-------------|
| `--interval-ms` | Refresh interval (default: 1000) |
| `--once` | Run once and exit |
| `--clear` | Clear screen between refreshes (default: true) |
| `--limit` | Tasks shown per bucket (default: 20) |
| `--tail` | Recent events to show (default: 10) |

**Aliases:** `visualizer`, `viz`, `dashboard`
