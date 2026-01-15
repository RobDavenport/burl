# Burl Validation Rules

Detailed specification for scope and stub validation gates.

## Validation Triggers

Validation runs at two points:

1. **On submit** (`burl submit`): Validates diff from `base_sha..HEAD`
2. **On approve** (`burl approve`): Validates diff from `origin/main..HEAD` (after rebase)

## Scope Validation

Scope validation ensures changes stay within declared boundaries.

### Rule S1: Forbidden Paths

**Check:** No changed file matches any `must_not_touch` pattern.

```yaml
must_not_touch:
  - src/enemy/**
  - src/networking/**
```

**Result if violated:**
```
FAIL (exit code 2)
Scope violation: src/enemy/ai.rs matches must_not_touch: src/enemy/**
```

### Rule S2: Allowed Paths

**Check:** Every changed file must match either:
- An explicit path in `affects`, OR
- A pattern in `affects_globs`

```yaml
affects:
  - src/player/jump.rs
affects_globs:
  - src/player/**
```

**Result if violated:**
```
FAIL (exit code 2)
Out of scope: src/utils/helper.rs (not in affects or affects_globs)
```

### Evaluation Order

1. Check S1 (forbidden) first
2. If S1 passes, check S2 (allowed)

### Glob Pattern Syntax

Uses standard glob patterns:
- `*` matches any characters except `/`
- `**` matches any characters including `/`
- `?` matches single character

Examples:
| Pattern | Matches | Does Not Match |
|---------|---------|----------------|
| `src/*.rs` | `src/lib.rs` | `src/player/mod.rs` |
| `src/**/*.rs` | `src/player/mod.rs` | `tests/test.rs` |
| `src/player/**` | `src/player/jump.rs`, `src/player/state/mod.rs` | `src/enemy/ai.rs` |

### New File Handling

- **`affects`**: Only matches existing files at task creation time
- **`affects_globs`**: Matches new files created during implementation

Use `affects_globs` when the task may add new files.

## Stub Detection

Stub detection prevents incomplete code from passing QA.

### Checked Patterns

Default patterns (configurable in `config.yaml`):

```yaml
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
```

### Critical: Added Lines Only

**Stub patterns are checked ONLY on added lines (`+` lines in diff), NOT on the entire file.**

This prevents false rejections due to pre-existing TODOs in untouched code.

### Checked File Extensions

Default extensions:
```yaml
stub_check_extensions: [rs, py, ts, js, tsx, jsx]
```

### Example Failures

```
FAIL (exit code 2)
Stub patterns found in added lines:
  src/player/jump.rs:67  + unimplemented!()
  src/player/jump.rs:45  + // TODO: implement cooldown
```

## Validation Commands

### Legacy: `build_command`

Optional single build/test command runs in the task worktree:

```yaml
# config.yaml
build_command: "cargo test"
```

- Empty string disables build validation
- Non-zero exit code = validation failure

### `validation_profiles`

You can configure an ordered, multi-step validation pipeline and optionally run steps only when relevant files change:

```yaml
# config.yaml
default_validation_profile: rust

validation_profiles:
  rust:
    steps:
      - name: fmt
        command: cargo fmt --all -- --check
        run_if_changed_extensions: [rs]

      - name: test
        command: cargo test
        run_if_changed_extensions: [rs]
```

Selection order:
1. Task frontmatter `validation_profile` (if set)
2. Config `default_validation_profile` (if set)
3. Otherwise, falls back to legacy `build_command`

Per-step conditions:
- `run_if_changed_extensions`: run if any changed file has one of these extensions
- `run_if_changed_globs`: run if any changed file matches one of these globs

## Validation Report

After validation, results are appended to the task's QA Report section:

```markdown
## QA Report

### Validation Run: 2026-01-13T15:30:00Z
- Scope: PASS
- Stubs: PASS
- Build: PASS (cargo test)

Ready for approval.
```

Or on failure:

```markdown
## QA Report

### Validation Run: 2026-01-13T15:30:00Z
- Scope: FAIL
  - src/enemy/ai.rs matches must_not_touch: src/enemy/**
- Stubs: FAIL
  - src/player/jump.rs:67 + unimplemented!()

Rejected. See above for fixes needed.
```

## Rebase Validation

On `burl approve`:

1. Task branch is rebased onto `origin/main`
2. Validation runs against the **rebased** diff (`origin/main..HEAD`)
3. This catches scope violations introduced by upstream changes

Example: If someone else added a file to `must_not_touch` while the task was in QA, rebase validation will catch the conflict.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | All validation passed |
| 2 | Validation failed (scope, stubs, or build) |

## Configuration Reference

Full validation config in `.burl/.workflow/config.yaml`:

```yaml
# Stub patterns (regex, applied to added lines)
stub_patterns:
  - "TODO"
  - "FIXME"
  # ... add custom patterns

# File extensions to check for stubs
stub_check_extensions: [rs, py, ts, js, tsx, jsx]

# Optional build command
build_command: "cargo test"

# Max QA attempts before BLOCKED
qa_max_attempts: 3

# Conflict policy for overlapping scopes
conflict_policy: fail  # fail | warn | ignore
```
