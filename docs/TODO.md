# TODO — rolling working notes

Updated every time work starts, stops or changes direction. Newest status at the top.

## Current focus

**Phase 0 / Milestone 0.5 — Scheduler and actions skeleton: implemented, awaiting CI + user acceptance.** Accepted and tagged: `m0.0`..`m0.4`.

0.5 delivered: typed `Stream` enum (S-005) and `Rng::weighted_pick` (S-006); schema fields `Tile`/`EntityId`/`Optional`; `reason` (S-010 reason codes with origin and plain sentences); `schedule` (DaySchedule, five priorities, seeded duty variation, `plan_day`, `replan_from`, `insert_urgent`, `reserve_commitment`, invariants, property tests); `action` (closed `ActionRegistry`: `move_to`, `idle_at`, `meet_at`); `commitment` (state machine + atomic acceptance); `activity` (DayPlanner, ReservationActivator with replanning, TaskPlanner, ActivitySystem, CommitmentSystem, `PlanSource`); `dev::DevPlanSource` replaces DevWander (D-019); commands `DevPropose`/`DevCancelCommitment`; world schema 3 + replay v3 (D-020); Blueprint v2.4 (D-021). CLI: `pg schedule explain <pawn>`, `pg actions`, `pg sim --propose`. Golden replay regenerated; CI also runs `pg actions` and `pg schedule explain`.

Known gaps (by design, Stage 1): needs/occupations feed the planner through `PlanSource`; no preconditions/permissions/effects; relationship hostility not yet used in acceptance; script hooks.

## Blocked / waiting on user

- [ ] Confirm CI for the 0.5 commit (all three OSes: new golden replay with commitments, threaded path check, `pg actions`, `pg schedule explain`) and accept 0.5. No mandatory checkpoint here; the next one is after 0.6 (saves).

## Next up

1. After acceptance: tag `m0.5`; start 0.6 persistence (`pg-persist`: `.pgsave`, atomic write, two generations, compressed/trimmable replay logs S-019, bug bundles S-001, scenario files S-003, S-004, S-014). **Checkpoint: stop for user verification after 0.6.**
2. Prune dev scaffolding as real systems land (D-009): `probe`, `DevProbeSystem`, `DevDaySystem`, `Stream::DevWander`.
3. Performance follow-ups: S-017 (A* scratch buffers), S-018 (pawn-aware routing), S-021 (region labels).

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
- 2026-10-06: **0.5 scheduler and actions skeleton**: see Current focus. Found while testing: two pawns cannot share a meeting tile (added `meet_at` + radius), the activator ends tasks at the boundary tick before the activity system runs (performing ends at `end - 1`), and a slot-length change invalidates schedules and commitments (handled). Logged D-019..D-021, S-021.
