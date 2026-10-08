# Playground

A deterministic 2D pixel-art life-simulation sandbox for desktop. A small fictional town of residents who follow routines, form relationships and remember what happens; extendable with sandboxed Luau content packs.

**Status:** pre-alpha. Stage 0 (foundations) is done and Stage 1 (a living town) is under way: a desktop app opens, generates a town and runs it with residents who have needs and moods, follow occupation schedules, talk, and form memories and relationships. There is no finished game yet and the rendering is a placeholder; saves and APIs may change without notice.

**Contributions:** not open yet. Issues and discussion are welcome; please hold pull requests until a contributing guide exists. Security reports: see [`SECURITY.md`](SECURITY.md).

**Licence:** [MIT](LICENSE).

- Specification: [`docs/reference/`](docs/reference/)
- **Build and test it yourself:** [`docs/BUILDING.md`](docs/BUILDING.md)
- Build plan: [`docs/PLAN.md`](docs/PLAN.md) · rolling todo: [`docs/TODO.md`](docs/TODO.md)
- Ideas beyond the spec: [`docs/SUGGESTIONS.md`](docs/SUGGESTIONS.md) · decisions: [`docs/DECISIONS.md`](docs/DECISIONS.md)

## Layout

`crates/` holds the workspace crates (`pg-core`, `pg-content`, `pg-script`, …); `tools/pg-cli` is the headless developer tool; `data/base` will hold the base game's own content pack. See Blueprint §2.
