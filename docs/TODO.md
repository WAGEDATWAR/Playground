# TODO — rolling working notes

Updated every time work starts, stops or changes direction. Newest status at the top.

## Current focus

**Stage 1, milestone 1.4 is implemented (accepted: 1.2 `m1.2`, 1.3 `m1.3`).** Residents now talk: nearby free residents chat about topics from `conversation.json` with a tone chosen by rules, which moves their relationship (daily cap, label hysteresis), restores social and leaves a memory each; memories fade daily and are bounded; D-046, Blueprint v3.6. Look with `cargo run -q -p pg-cli -- sim --seed demo --town 64x48 --residents 14 --content data/base --days 3 --events conversation` and `pg residents inspect pawn_1 --seed demo --town 64x48 --residents 14 --content data/base --days 2` (needs, mood, memories with reasons, relationships). Next: 1.5 AI lines, then 1.6 (renderer, the next checkpoint for you).

**Known, scheduled:** the map view still draws generated towns with the painter-based view (the real renderer is 1.6); building roles are not yet drawn differently; conversation bubbles and the inspector panel come with 1.6/1.7 (the data and `pg residents inspect` exist now).

**Parked (come back later):** weather, seasons and fire (S-048, Roadmap section 12 item 8); animals and non-human humanoids (the physiology profile leaves room); anatomy extensions after Stage 2B's base (eyes and ears, neck, spine segments); customisable map sizes (Stage 3 generator controls); real art assets to replace procedural sprites.

## CI status

The repository is public (D-047), so GitHub Actions minutes are free again. Locally, `cargo run --release -p pg-cli -- check --full` stays the gate before a push; S-050 will make CI cheaper.

## Blocked / waiting on user

- [x] (done 2026-10-07, id in `pg_ai::login::CLIENT_ID`; terms at `player2.game/devtos` still worth a read before release) **Register a Player2 `client_id`** for this game (Roadmap §12 item 8) and confirm the API base URL and terms (`player2.game/devtos`). Until then `pg ai login player2` uses a placeholder the provider will likely refuse. Not blocking 0.8.

## Next up

1. ~~Owner repository settings~~ done 2026-10-08: branch ruleset on `main` (no deletion, no force push, admin bypass), private vulnerability reporting, secret scanning. Optional: require status checks once S-050 makes CI cheap and stable. Still to confirm: the copyright line in `LICENSE` (currently "WAGEDATWAR and the Playground contributors").
2. ~~1.4b Process and safety net~~ done (D-049): `pg pins`, `pg schema`, `pg milestone`, `pg bench`, cheaper CI, property tests, plateau soak.
3. ~~1.5 AI lines~~ done (D-050); S-054 and S-057 moved to 1.6/1.7. **Next:** 1.6 native renderer (checkpoint for you): S-069 social report first; the renderer must call `set_dialogue_focus` and draw bubbles from `DialogueService::lines`.
4. 1.6 Native renderer (checkpoint): S-069 social report first; render benchmark targets.
5. 1.7 (S-056, S-058 observation and possession modes only, S-067), 1.8 (S-061, bench suite coverage, S-064), 1.9 gate (S-070 digest golden in `pg check --full`).
6. Later: S-055 (Stage 2), S-053 (Stage 3), S-060 only when easy.

Modes (D-048): observation mode includes possession and is all the product does today; player mode (a world-creation toggle locking the player to one pawn, blocking observation and possession) comes with possession in Stage 4.

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
- 2026-10-07: **0.7 host services and AI client skeleton**: see Current focus. Findings: real providers return floats, so the AI boundary uses `serde_json` instead of the core parser; the keyring ecosystem changed shape (v4, per-platform store crates behind a `v1` feature); `webpki-roots` needs a data-license entry in `deny.toml`.
- 2026-10-07: Accepted 0.7. Added Player2 as a provider (D-027): adapter, provider-chosen model, free credits check, device-code sign-in protocol (`pg_ai::login`), `pg ai login`; 506 tests. Accepted S-022..S-030 and placed them (D-028); specs bumped to Blueprint v2.7, Roadmap v4.4, Design Document v3.2.
- 2026-10-07: **0.8 runtime and debuggability groundwork**: see Current focus. Findings: a persistent pool cannot host borrowed path jobs without `unsafe` (D-030); 200 pawns cost 0.10 ms per tick, so S-017 is not needed yet; the GUI stack needed three license/advisory entries in `deny.toml`.
- 2026-10-07: **0.9 ScriptVm spike**: see Current focus. Findings: Luau fast-calls bypass removed globals (compiler options needed), the pattern matcher is fuel-interruptible, `mlua` integers are `i64`, metatables must be frozen on attach; scripts cost 12 us per handler call and 1.6% of the tick for 200 pawns. Player2 client id registered and built in.
