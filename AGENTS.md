# Burl (File-Driven eXecution)

This repo is the standalone home for the `burl` tool/spec.

## Key Docs

- PRD / source of truth: `burl.md`
- Implementation task breakdown: `tasks/README.md`

## Development Notes

- Target: cross-platform Rust CLI (`burl`).
- Prefer deterministic, diff-based validation gates (no model self-judging).
- Favor atomic filesystem operations; avoid shell-only assumptions.

