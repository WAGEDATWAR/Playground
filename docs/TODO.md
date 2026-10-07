# TODO — rolling working notes

Updated every time work starts, stops or changes direction. Newest status at the top.

## Current focus

**Phase 0 / Milestone 0.4 — World state and spatial: implemented, awaiting CI + user acceptance.** Accepted and tagged so far: `m0.0`..`m0.3`. The dev probe/wander scaffolding (D-009) is still temporary; the real movement system, maps, objects and containment are now in place.

0.4 delivered: `map` (flat tile arrays, chunks, edit versions, move costs), `object`/`containment` (containers, validate_containment, spawn/move/delete with cascade|evict|forbid), `pawn` + `occupancy`, `path` (A* with fixed tie-breaks, destinations, deterministic LRU cache, `BatchExecutor`), `movement` (MovementSystem), dev `commands`, replay v2 (per-table hashes, content refs, `diff_logs`, `bisect`), "did you mean" hints (S-012), `pg-runtime::exec::ScopedThreads`. CLI: `pg map show|path|bench-paths`, `pg replay --diff|--bisect`, `pg sim --dev-map --dev-pawns --object --put --events [prefix] --since --until --threads --content`. Golden replay is now `golden/dev-town-3days.json` (map, 10 pawns, objects, content refs) and CI also checks threaded == serial pathfinding on every OS.

## Blocked / waiting on user

- [ ] Confirm CI for the 0.4 commit (all three OSes: golden replay with pawns and objects, threaded path check) and accept 0.4. No mandatory checkpoint here; next is after 0.6 (saves).

## Next up

1. After acceptance: tag `m0.4`; start 0.5 scheduler and actions skeleton (DaySchedule, reservations, five priorities, conflict/tie rules, plan_day skeleton, commitments protocol, ActionRegistry, ReasonCode with origin) plus S-005 (RNG stream registry), `pg schedule explain`.
2. 0.5 will replace `DevWander` with a real TaskPlanner path (actions `move_to` etc.); remove dev scaffolding as each piece is superseded (D-009).
3. Performance follow-ups logged as S-017 (A* scratch buffers) and S-018 (pawn-aware routing).

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
- 2026-10-06: Accepted 0.3 (`m0.3`). Scheduled all suggestions across milestones/stages and folded them into Blueprint v2.3, Roadmap v4.2 (principle 16, [DX] items), Design Document v3.1 (D-013).
- 2026-10-06: **0.4 world state and spatial**: maps, objects + containment, pawns + occupancy, A* + cache + batch executor, MovementSystem, dev commands, replay v2 with diff/bisect, hints, ScopedThreads, `pg map` tools, golden replay v2. 160+ tests; decisions D-014..D-017; suggestions S-017..S-019.
