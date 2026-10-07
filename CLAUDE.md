# Playground (Rust) — project instructions

Playground is a deterministic 2D pixel-art town life-simulation, built as a **native desktop Rust binary** with a **sandboxed Luau scripting layer** for user content packs.

## Source of truth

The binding specification is in `docs/reference/`:
`architecture-blueprint-v2.6.md` (how), `design-document-v3.1.md` (what/why), `roadmap-v4.3.md` (when).
Read the relevant Blueprint sections before touching a crate. If code and spec disagree, stop and resolve it (fix code, or record a decision in `docs/DECISIONS.md` and update the spec copy with a version bump).

## Working files (keep current)

- `docs/PLAN.md` — phase and milestone plan.
- `docs/TODO.md` — rolling todo and notes. **Update at the start and end of every work session and whenever direction changes.**
- `docs/SUGGESTIONS.md` — ideas beyond the spec; add entries whenever they arise.
- `docs/DECISIONS.md` — decisions and spec deviations.

## Rules that must not be broken

- `pg-core` is headless and deterministic: `forbid(unsafe_code)`; no floats, no `HashMap`/`HashSet`, no clocks, no `rand`, no `unwrap`/`expect`/indexing on data-derived values; it must not depend on the scripting VM.
- Only the core writes `WorldState`. Scripts and AI produce typed, validated requests.
- All Luau access goes through the `ScriptVm` trait inside `pg-script`; nothing else imports the binding crate.
- Keys never reach logs, saves, exports or crash reports; `redact()` on every log path.
- Each component ships with unit tests and a developer-mode feature.

## Process

- Build in phases per `docs/PLAN.md`; stop for user verification at each phase gate and marked checkpoint.
- Commit small and often; tag accepted milestones (`m0.1`, …).
- Ask the user when a decision is theirs; do not guess on anything costly to reverse.
