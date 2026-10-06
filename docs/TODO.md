# TODO — rolling working notes

Updated every time work starts, stops or changes direction. Newest status at the top.

## Current focus

**Phase 0 / Milestone 0.1 — Determinism primitives: implemented, awaiting review.** All code, tests and dev tools are done and passing locally (Windows). Remaining: confirm CI (incl. the new cross-OS `pg selftest` step and `cargo-deny`) is green, tag `m0.0` (CI on the bootstrap push) and `m0.1`, then wait for user acceptance before starting 0.2.

## Blocked / waiting on user

- [ ] **Confirm CI result** on GitHub Actions for the pushed `main` (my GitHub tools cannot read check runs). I need: green/red per OS, and the failing log if red. Then I tag `m0.0`.
- [ ] **Review/accept 0.1** (see the milestone report in chat), then I start 0.2 (time, tick pipeline, replay).
- [ ] **First push of the 0.1 commit(s)** (`git push`), same as before: local git has no stored credentials.

## Next up

1. Tag `m0.0` once CI for the bootstrap commit is confirmed green; tag `m0.1` after acceptance.
2. 0.2: `Clock`/tick flags, system pipeline with extension-point slots, `SimInput` log + ordering, `TickReport`, replay; `pg sim` / `pg replay` CLI. Checkpoint review after 0.2.
3. Run `cargo deny check` locally once the background install finishes; fix `deny.toml` if its schema or license list needs adjusting.

## Shell note

Tool shells start without the Rust PATH. Use PowerShell and prefix commands with
`$env:Path = [Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User')`.
Python is available for scripts (`scripts/check_deps.py`). Avoid bash heredocs containing many quotes; write files with the Write tool.

## Notes (non-obvious things to remember)

- The skeleton now compiles and passes local checks on Windows (verified 2026-10-06 with Rust 1.99.0). CI on Linux/macOS is still unverified.
- `Cargo.toml` license is a placeholder (`UNLICENSED`); needs a decision before any public release (`DECISIONS.md`).
- `rust-toolchain.toml` pins `1.99.0`. CI uses `dtolnay/rust-toolchain@stable`; it should honor the pin via rust-toolchain.toml (verify on first CI run).
- Edition 2021 kept for now; toolchain 1.99.0 supports 2024, so migrate deliberately (`cargo fix --edition`) as a small separate commit if wanted.
- The three reference docs still carry stale file names/versions in Downloads; the copies here are named by version.
- Earlier correction: the old TypeScript prototype is **not** a porting oracle; the Rust core is built fresh from the Blueprint (Roadmap §11).

## Done log

- 2026-10-06: Read the three reference documents; created repo skeleton (14 crates, workspace manifest, lint policy for `pg-core`); saved reference docs under `docs/reference/`; wrote `PLAN.md`, `TODO.md`, `SUGGESTIONS.md`, `DECISIONS.md`, `README.md`, `CLAUDE.md`.

- 2026-10-06: Verified toolchain; pinned Rust 1.99.0; `cargo fmt`, build, clippy, test all clean; added `scripts/check_deps.py` (dependency rule from Blueprint §1, restricts `mlua`/`wgpu`/`winit`/`egui`/`rand` to allowed crates) and wired it into CI.
- 2026-10-06: **0.1 determinism primitives** in `pg-core`: `id` (EntityId/Kind/IdCounters, canonical base36 text), `table` (sorted `Table<T>`), `num` (Permille/NeedValue/Affinity, round_half_up_div, muldiv, scale_permille), `rng` (FNV-1a+SplitMix64 construction, counter-based `Rng` with range/int_in/chance/pick/shuffle, stream names), `canon` (integer-only canonical serialization), `hash` (BLAKE3 state hash + per-table combine), `vectors` (published + pinned test vectors). 48 unit/property tests. Dev tools: `pg selftest | rand | hash | id`. Added `deny.toml`, CI selftest + cargo-deny jobs.
