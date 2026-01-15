# Burl (File-Driven eXecution)

`burl` is a minimal, file-based workflow orchestrator for agentic coding/review pipelines.

## What it does

- Stores workflow state as **folders + markdown task files** in a dedicated Git worktree (default `.burl/` on branch `burl`).
- Creates one Git worktree per task under `.worktrees/` to isolate work.
- Enforces deterministic gates (scope + stubs; optional build/test command) using **diffs against a stored `base_sha`**.

## Install (from source)

```bash
cargo install --path .
```

## Get up and running

### Existing project (most common)

Prereqs:
- You’re in a Git repo (not a submodule) with at least one commit.
- If your default branch isn’t `main`, set `main_branch` in `.burl/.workflow/config.yaml` after init.

From the root of a Git repository you want to use `burl` in:

```bash
# first time (idempotent)
burl init

# create a task (goes to READY/)
burl add "Implement player jump" --priority high --affects-globs "src/player/**"

# claim work (moves READY -> DOING and creates a task worktree)
burl claim TASK-001
# (cd into the printed worktree path to do the work)

# in the task worktree: make commits, then submit for QA
burl submit TASK-001

# run validation (scope/stubs + optional build/test)
burl validate TASK-001

# accept/reject
burl approve TASK-001
# or
burl reject TASK-001 --reason "Scope exceeded; touched src/net/**" # may move to BLOCKED if max attempts reached
```

### New project

```bash
mkdir my-project && cd my-project
git init -b main
echo "# my-project" > README.md
git add README.md
git commit -m "Initial commit"

burl init
```

### Working across machines (or a team repo)

The durable workflow state lives on the `burl` branch (the `.burl/` worktree is just a checkout of it).

- On machine A: push workflow state + any in-progress task branches:
  - `git push <remote> burl`
  - `git push <remote> <task-branch>`
- On machine B: fetch the workflow branch locally, then init to attach the worktree:
  - `git fetch <remote> burl:burl`
  - `burl init`

## Agent dispatch

```bash
# list configured agents
burl agent list

# dispatch an agent to work on a claimed task
burl agent run TASK-001

# use a specific agent (overrides task/default assignment)
burl agent run TASK-001 --agent claude-code

# preview what would run without executing
burl agent run TASK-001 --dry-run
```

Requires `.burl/.workflow/agents.yaml` and a task in the `DOING` bucket (claimed with `burl claim`).

## Live dashboard + automation

```bash
# lightweight TUI-style dashboard (alias: `visualizer`)
burl monitor

# automation loop: claim READY tasks up to max_parallel, validate QA tasks
burl watch

# also auto-approve QA tasks (runs validations via approve)
burl watch --approve

# fully automated: claim, dispatch agents, and validate/approve
burl watch --dispatch --approve
```

## Files and folders

Default layout:

```
.burl/                 # canonical workflow worktree (branch: burl)
  .workflow/
    READY/ DOING/ QA/ DONE/ BLOCKED/
    config.yaml
    agents.yaml        # agent configuration
    prompts/           # generated agent prompts
    agent-logs/        # agent stdout/stderr (untracked)
    events/events.ndjson
    locks/             # untracked, machine-local

.worktrees/
  task-001-.../        # per-task worktrees
```

## Configuration

Workflow config lives at `.burl/.workflow/config.yaml`.

Common knobs:
- `main_branch`, `remote`
- `build_command` (legacy single-step hook; empty string disables build/test validation)
- `validation_profiles`, `default_validation_profile` (multi-step pipeline; optional)
- `stub_patterns`, `stub_check_extensions`
- `merge_strategy`, `conflict_detection`, `conflict_policy`
- `workflow_auto_commit`, `workflow_auto_push`

## Agent configuration

Edit `.burl/.workflow/agents.yaml` (scaffolded by `burl init`) to configure agent profiles:

```yaml
agents:
  claude-code:
    name: "Claude Code CLI"
    command: "claude -p \"{prompt_file}\""
    timeout_seconds: 600
    default: true

  crush:
    name: "Crush"
    command: "crush run --task \"{task_file}\" --prompt \"{prompt_file}\""
    timeout_seconds: 1800
    capabilities: [multi-file, refactoring]

defaults:
  timeout_seconds: 600
  prompt_template: default

prompt_templates:
  default: |
    # Task: {title}
    ## Objective
    {objective}
    ## Acceptance Criteria
    {acceptance_criteria}
```

Note: commands are split using shell-style quoting. Quote placeholders like `"{prompt_file}"` if paths may contain spaces.

Template variables available in commands and prompts:
- Identity: `{task_id}`, `{title}`, `{priority}`
- Paths: `{prompt_file}`, `{task_file}`, `{worktree}`
- Scope: `{affects}`, `{affects_globs}`, `{must_not_touch}`
- Git/worktree: `{branch}`, `{base_sha}`
- Task metadata: `{tags}`, `{depends_on}`
- Task body sections: `{objective}`, `{acceptance_criteria}`, `{context}`, `{implementation_notes}`, `{test_plan}`, `{body}`

## Development

```bash
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
```

If `cargo check`/`cargo clippy` fails with `Invalid cross-device link (os error 18)`, treat it as an environment/toolchain issue and use `cargo build` + `cargo test` as the validation gate.

## Docs

- Spec / PRD: `burl.md`
- Roadmap: `ROADMAP.md`
