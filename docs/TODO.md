# TODO — rolling working notes

Updated every time work starts, stops or changes direction. Newest status at the top.

## Current focus

**Phase 0 / Milestone 0.2 — implemented, awaiting CI + checkpoint review.** `time`, `world`, `input`, `pipeline`, `sim`, `replay`, `dev` in `pg-core` (90 tests passing); CLI: `pg sim | replay | time | pipeline`; golden replay `golden/dev-3days.json` checked in CI. Milestones 0.0 and 0.1 are accepted and tagged.

## Blocked / waiting on user

- [ ] **CHECKPOINT REVIEW after 0.2** (PLAN.md): confirm CI result (my tools cannot read check runs) and accept/adjust 0.2 before 0.3.

## Next up

1. After acceptance: tag `m0.2`; start 0.3 content system (templates, resolver, component registry, pack manifest + data-only loader, `pg content lint|resolve`).
2. Remove the `dev` probe scaffolding when real systems arrive (D-009).
3. `Sim` currently holds all state in `WorldState`; entity tables arrive in 0.4. Snapshot = clone for now; real (de)serialization is 0.6.

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
- 2026-10-06: Accepted 0.1; tagged `m0.0` (bootstrap) and `m0.1`; pushed (credentials already stored by Git Credential Manager); CI: actions forced to Node 24.
- 2026-10-06: **0.2 time, tick pipeline, input log, replay**: `time` (SlotMinutes, Clock, TimeFlags), `world` (minimal WorldState + per-table hashes), `input` (SimInput, ordered InputQueue, Canon round-trip), `pipeline` (13 slots, cadence, anchored extensions, trace), `sim` (step, snapshot/restore, per-day hashes with per-table hashes), `replay` (integer-only log, verify, tamper detection), `dev` scaffolding. 90 core tests incl. property tests (split run anywhere == whole run). Dev tools `pg sim|replay|time|pipeline`. Golden replay in CI.
