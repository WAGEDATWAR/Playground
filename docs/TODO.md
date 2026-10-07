# TODO — rolling working notes

Updated every time work starts, stops or changes direction. Newest status at the top.

## Current focus

**Phase 0 / Milestone 0.6 — Persistence: implemented, CHECKPOINT: waiting for your verification.** Accepted and tagged: `m0.0`..`m0.5`.

0.6 delivered: typed `Reader` + `WorldState::from_canon` (round trips at every stage of a run, corruption never panics); `pg-host` `Storage` trait with a fault-injecting `MemStorage`, `pg-host-os` `FsStorage` (temp file, fsync, rename); `pg-persist`: `.pgsave` container (header, BLAKE3, zstd), slot store (manifest with two generations, atomic save, recovery, damaged marking, per-generation verify), migrations framework, content compatibility report (S-014), world export/import pipeline, hardened archive reader, compressed logs and bug bundles (S-019, S-001), scenario runner (S-003); `ReplayLog::trim`; RFC 8785 cross-check (S-004, D-022); `pg content diff`. CLI: `pg save create/list/inspect/verify/load/export/import`, `pg bugbundle create/run`, `pg scenario run`, `pg content diff`. CI runs a save/verify cycle, the persistence scenario and a bundle round trip. Blueprint v2.5; decisions D-022..D-024.

Crash safety is tested by failing storage at every operation of a save; a load afterwards always returns the old or the new world.

## Blocked / waiting on user

- [ ] **Checkpoint after 0.6:** confirm CI (three OSes) and try the persistence commands (see `docs/BUILDING.md` section 4). Then say "accept 0.6" so I tag `m0.6` and start 0.7.

## Next up

1. After acceptance: tag `m0.6`; start 0.7 host services and AI client skeleton (`pg-host` SecretStore/Net/Clock/Dialogs doubles, `pg-host-os` implementations, `pg-ai` adapters, circuit breaker, `redact()`, sentinel-key leak test).
2. Prune dev scaffolding as real systems land (D-009): `probe`, `DevProbeSystem`, `DevDaySystem`, `Stream::DevWander`.
3. Performance follow-ups: S-017, S-018, S-021.
4. Possible later improvement: zstd levels above "fastest" if save size ever matters (needs a different encoder).

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
- 2026-10-07: **0.6 persistence**: see Current focus. Findings: `Canon` key order vs RFC 8785 (D-022); only the fastest zstd level exists in the pure-Rust encoder; a replay log is tiny, so the useful compression and trimming target is the world snapshot in bundles.
