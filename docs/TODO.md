# TODO — rolling working notes

Updated every time work starts, stops or changes direction. Newest status at the top.

## Current focus

**Phase 0 / Milestone 0.4 — World state and spatial (in progress).** Accepted and tagged: `m0.0`..`m0.3`. Suggestions are scheduled (docs/SUGGESTIONS.md table; specs bumped to Blueprint v2.2 / Roadmap v4.1 / Design Document v3.1).

0.4 design (decided before coding):
- `map` (MapData: flat tile arrays, chunks, edit_version; MoveCosts), `object` + `containment` (ObjectInstance, Location, ContainerState, validate_containment, spawn/move/delete with cascade|evict|forbid), `pawn` (spatial fields + Route), `occupancy` (derived, never hashed), `path` (A* sparse g-scores, (f,h,index) heap, N/E/S/W, cap 20,000, destination resolution, deterministic LRU cache, `BatchExecutor` trait: serial in core, scoped threads in pg-runtime), `movement` (MovementSystem: id order, wait -> sidestep -> repath -> fail).
- WorldState gains `maps`, `objects`, `pawns` (+ movement settings). Derived occupancy and path cache are not hashed.
- Dev commands (logged inputs): create map, spawn pawn/object, move, set tile blocked, put in container. Temporary `DevWander` system keeps pawns walking. D-009 scaffolding stays until real systems arrive.
- Replay format v2: per-table hashes + content refs; `pg replay --diff / --bisect` (S-002/S-007/S-008); `pg sim --events <prefix>` (S-009); "did you mean" hints in content validation (S-012).
- Dev tools: `pg map show | path | bench-paths`.

## Blocked / waiting on user

- [ ] Confirm CI for the 0.3 commit and accept 0.3 (no mandatory checkpoint here; next checkpoint is after 0.6 saves).

## Next up

1. After acceptance: tag `m0.3`; start 0.4 world state and spatial (`WorldState` tables, `MapData`, occupancy, containment + `validate_containment`, A* with fixed tie-breaks, parallel batched pathing with deterministic merge, movement step; `pg map show`).
2. Use `ContentSet` (resolved templates) when objects are instantiated in 0.4; record `ContentSet::refs()` in `WorldState.content_refs`.
3. Remove the `dev` probe scaffolding when real systems arrive (D-009).

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
- 2026-10-06: **0.3 content system**: new `pg-canon` crate (Canon + strict integer-only JSON parser, 19 tests incl. property tests); `pg-content` (ids, report, schema, component registry, templates, resolver, manifest, pack loader, content set; 68 unit tests + 5 base-pack integration tests); `data/base` pack (15 templates incl. the Blueprint example chains); `pg content lint|list|resolve|components`. Blueprint bumped to v2.1 (pg-canon, pack/namespace rules). Decisions D-010..D-012.
- 2026-10-06: Accepted 0.3 (`m0.3`). Scheduled all suggestions across milestones/stages and folded them into Blueprint v2.2, Roadmap v4.1 (principle 16, [DX] items), Design Document v3.1 (D-013).
