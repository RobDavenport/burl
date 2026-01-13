# FDX V1 — Build Task Breakdown

These task specs break `fdx.md` into **bite-sized, sequential, implementable** units you can hand to agents. Each task includes acceptance criteria plus testing/validation expectations.

**Workflow:** complete tasks in order; do not start a task until its dependencies are DONE and tests are green.

## Task Order (V1)

1. `tasks/TASK-001-fdx-cli-scaffold.md`
2. `tasks/TASK-002-repo-discovery-git-config.md`
3. `tasks/TASK-003-task-model-frontmatter-atomic-fs.md`
4. `tasks/TASK-004-locking-subsystem.md`
5. `tasks/TASK-005-init-workflow-scaffolding.md`
6. `tasks/TASK-006-events-log-ndjson.md`
7. `tasks/TASK-007-task-management-commands.md`
8. `tasks/TASK-008-git-worktree-branch-helpers.md`
9. `tasks/TASK-009-claim-command-transaction.md`
10. `tasks/TASK-010-diff-parsing-primitives.md`
11. `tasks/TASK-011-scope-validation.md`
12. `tasks/TASK-012-stub-detection-validation.md`
13. `tasks/TASK-013-submit-command.md`
14. `tasks/TASK-014-validate-command.md`
15. `tasks/TASK-015-approve-command.md`
16. `tasks/TASK-016-reject-command.md`
17. `tasks/TASK-017-doctor-command.md`
18. `tasks/TASK-018-clean-command.md`

## Conventions

- **Exit codes:** follow `fdx.md` section “Return codes” (0/1/2/3/4).
- **Determinism:** validation gates must be **diff-based** using the stored `base_sha` (and the rebased base on approve).
- **No LLM self-judging:** validations must be purely mechanical/deterministic (diff + config + command exit codes).
- **Cross-platform:** Windows/macOS/Linux support is required for filesystem atomics and subprocess execution.

## Suggested Code Architecture (Maintainable V1)

- `main.rs`: CLI parsing + top-level exit-code mapping only (no business logic).
- `context.rs`: resolves repo root + canonical workflow paths + actor id + loads config.
- `errors.rs`: a small error type that carries `{ exit_code, message, details }` (avoid panics).
- `fs/atomic.rs`: `atomic_write` + `atomic_rename` helpers used everywhere.
- `locks.rs`: lock acquire/list/clear (RAII guards).
- `task.rs`: task frontmatter parse/serialize + “QA Report” append helper.
- `workflow.rs`: bucket enumeration + atomic move helpers (folder = truth).
- `git.rs`: git runner + worktree/branch operations (all `git` calls funneled here).
- `diff.rs`: diff parsing utilities (changed files + added lines with line numbers).
- `validate/`: `scope.rs`, `stubs.rs`, `build.rs` producing structured results.
- `commands/`: one module per CLI command, each taking `&Context` and returning `Result<(), FdxError>`.

Design notes:
- Centralize “mutating workflow state” operations in a helper that:
  1) checks workflow worktree cleanliness
  2) acquires `workflow.lock`
  3) performs writes/moves/logging
  4) commits/pushes workflow branch per config

## Configurability Principles (V1)

- Treat `.fdx/` + branch `fdx` as fixed layout in V1 (avoid config bootstrap/migration complexity).
- Keep config forward-compatible:
  - ignore/round-trip unknown keys
  - validate known enums and numeric ranges
- Make policy toggles config-driven (already in PRD): `merge_strategy`, `conflict_policy`, `qa_max_attempts`, `stub_patterns`, `build_command`, `*_auto_push` flags.
