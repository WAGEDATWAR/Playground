# Playground

A deterministic 2D pixel-art life-simulation sandbox for desktop. A small fictional town of residents who follow routines, form relationships and remember what happens; extendable with sandboxed Luau content packs.

**Status:** Phase 0 (foundations) — pre-alpha, nothing playable yet.

- Specification: [`docs/reference/`](docs/reference/)
- Build plan: [`docs/PLAN.md`](docs/PLAN.md) · rolling todo: [`docs/TODO.md`](docs/TODO.md)
- Ideas beyond the spec: [`docs/SUGGESTIONS.md`](docs/SUGGESTIONS.md) · decisions: [`docs/DECISIONS.md`](docs/DECISIONS.md)

## Layout

`crates/` holds the workspace crates (`pg-core`, `pg-content`, `pg-script`, …); `tools/pg-cli` is the headless developer tool; `data/base` will hold the base game's own content pack. See Blueprint §2.
