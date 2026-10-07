# Playground — Build Plan

Working plan for building Playground in Rust. The three documents in `docs/reference/` are the **binding specification**:

- `architecture-blueprint-v2.2.md` — how (crates, determinism, scripting, persistence)
- `design-document-v3.1.md` — what and why (pillars, scope, modding boundaries)
- `roadmap-v4.1.md` — when (stages, gates, decisions to confirm)

This plan turns them into an ordered build. It is a working document: status lives in `TODO.md`; ideas that go beyond the spec live in `SUGGESTIONS.md` (accepted ones are scheduled below and folded into the specs); deviations from the spec are recorded in `DECISIONS.md`.

## Working agreements

1. **Spec first.** If code and spec disagree, stop and either fix the code or record a decision (and a spec version bump) before continuing.
2. **Tests and dev tools ship with each component.** A milestone is not done until it has unit tests, property tests where the spec calls for them, and a developer-mode feature (CLI command, overlay panel or inspector view) that lets a human see the component working.
3. **Determinism is a feature, tested continuously.** Golden replay hashes grow with the sim and must stay identical across OSes.
4. **Verification gates.** I stop and hand over at the end of every phase (and at marked mid-phase checkpoints) with: what was built, how to run it, test results, deviations, open questions. Work does not continue past a gate until the gate is accepted.
5. **Small commits, tagged milestones.** One logical change per commit; a git tag per accepted milestone (`m0.1`, …) so any step can be rolled back.
6. **Questions go to the user**; guesses go in `DECISIONS.md` only when the choice is cheap to reverse.
7. **Temporary scaffolding is labelled and scheduled for removal** (D-009: the dev probe and `dev.*` systems are replaced by real systems as they land).

## Phase 0 — Reliable foundations (Roadmap Stage 0)

Goal: a deterministic, headless, saved/reloaded sample town; a proven sandboxed script path; cross-OS identical hashes. No gameplay yet.

Done and accepted: **0.0 Bootstrap** (`m0.0`), **0.1 Determinism primitives** (`m0.1`), **0.2 Time, tick pipeline, replay** (`m0.2`), **0.3 Content system** (`m0.3`).

| Milestone | Scope (crate) | Tests | Dev-mode feature |
| --- | --- | --- | --- |
| **0.4 World state and spatial** ✅ implemented (`pg-core`, `pg-runtime`, `pg-cli`) | Real world tables: `maps`, `objects`, `pawns` (spatial fields only), `MapData` + chunks + edit versions, derived occupancy, containment (`ObjectInstance`, `ContainerState`, `Parent`, `validate_containment`, delete policies), A\* with fixed tie-breaks and bounded LRU cache, destination resolution (`PlaceRef`: tile / object interaction point), batched path solving through a `BatchExecutor` (serial in core, scoped threads in `pg-runtime`), `MovementSystem` (lower-id-first, wait → sidestep → repath → fail), dev commands to create maps/pawns/edit tiles, a temporary `DevWander` system. **Suggestions folded in:** S-002 hash diff, S-007 per-table hashes stored in replay logs, S-008 replay bisect, S-009 event filter, S-012 "did you mean" hints for the content validator | Containment invariants under random edit sequences (property); A\* optimality vs brute force; **parallel == serial at 1/2/4/8 threads**; blocked-destination, head-on swap, repath and failure cases; map-edit invalidates cache; replay with a map + wandering pawns reproduces; hash-diff/bisect locate a seeded divergence | `pg map show` (ASCII map, pawns, routes), `pg map path`, `pg map bench-paths` (serial vs threaded, verifies identical output), `pg replay --diff / --bisect`, `pg sim --events <prefix>`, containment check output |
| **0.5 Scheduler and actions skeleton** (`pg-core`) | `DaySchedule`, reservations, five priorities, conflict/tie rules, `plan_day` skeleton, commitments protocol, closed `ActionRegistry`, `ReasonCode` with origin. **Folded in:** S-005 RNG stream registry, S-006 draw helpers as first needed | Property: no overlap, priority never inverted, commitments reserve both-or-neither; conflict goldens; stream names all registered | `pg schedule explain <pawn> <day>` printing reason codes (S-010, CLI part) |
| **0.6 Persistence** (`pg-persist`) | `Storage` use, atomic write, `.pgsave` format (header + zstd canonical JSON), manifest + two generations, load/recovery, migrations framework, export/import skeleton, hardened archive reader, **content compatibility report on load**. **Folded in:** S-001 bug bundle, S-003 scenario file format, S-004 canonical JSON cross-check against RFC 8785, S-014 `pg content diff` | Fault injection (truncate, flip byte, missing file), migration fixture tests, archive traversal/bomb tests, save→load→hash equality, JCS cross-check property test, content-diff cases | `pg save inspect / verify`, `pg bugbundle`, `pg content diff`, `pg scenario run` |
| **0.7 Host services and AI client skeleton** (`pg-host`, `pg-host-os`, `pg-ai`) | Traits + test doubles; OS impls (fs, credential store, HTTPS, dialogs); provider adapters (pure), circuit breaker, cooldowns, `redact()`, settings flow | Adapter fixture tests, hostile outputs, **sentinel-key leak test**, breaker state tests | `pg ai test --provider`, redaction self-check |
| **0.8 Runtime and threading** (`pg-runtime`) | Sim thread with fixed-step accumulator, command queue, worker pool (generalizing 0.4's executor), deterministic job-result application, `RenderSnapshot` triple buffer, run states, autosave policy, last-resort `catch_unwind` guard | Same hashes at 1/2/N workers, run-state transitions, autosave triggers, panic-guard test | Tick profiler (`tracing` spans), dev-tool registry skeleton |
| **0.9 ScriptVm spike** (`pg-script`, `pg-api`) | `ScriptVm` trait, `mlua`-Luau implementation, sandbox profile (§23.4), deterministic fuel, memory limit, source-only loading, value boundary, one component + system + hook driven by a test pack, quarantine, safe mode. **Folded in:** S-020 first three cookbook packs, S-013 JSON Schema export for data files, S-012 hints extended to API names | Hostile-pack corpus, VM-reload determinism variant, answers to the 7 spike questions in `docs/spikes/scriptvm.md`, cookbook packs run under golden replay | `pg pack lint/test`, `pg content schema`, pack inspector output |
| **0.10 Dev overlay shell** (`pg-app`, `pg-render`) | Minimal window (winit + wgpu clear + egui), dev-mode overlay: tick/day, hash, per-system times, event viewer with the S-009 filter, **reason-code explorer (S-010)**, pack status, quarantine list, bug-bundle button (S-001) | Snapshot-builder tests; smoke test that the shell starts headless-safe | The overlay itself |
| **0.11 Stage 0 gate** | Cross-OS golden replay in CI, **soak scenario (S-011)** run from a scenario file, acceptance scenario "deterministic sample town runs, pauses, saves, reloads, migrates, recovers" | The Roadmap Stage 0 "Done when" checks, written as scenarios (S-003) | Gate report |

Checkpoints for your review: after **0.2** (done), after **0.6** (saves), after **0.9** (script spike decision), and at **0.11** (phase gate).

## Later phases (outline; detailed when reached)

Each later phase follows Roadmap §7 stage items. Per stage I will produce a detailed milestone table like the one above before coding. Accepted suggestions that land in later stages:

| Phase | Roadmap stage | Headline | Suggestions scheduled here |
| --- | --- | --- | --- |
| 1 | Stage 1 | Observation-first town: needs, mood, memory, conversations, worldgen v1, renderer + egui screens, base pack, API 0.1 | S-015 inheritance tree view; evaluate S-016 template variants at the end; remove dev probe scaffolding (D-009); cookbook sample for hooks (S-020) |
| 2 | Stage 2 | Free-time scheduler, commitments, API 0.2 (systems, actions) | cookbook sample for systems/actions (S-020) |
| 3 | Stage 3 | Generator controls, editor, mutation API, API 0.3 (worldgen) | cookbook sample for worldgen (S-020) |
| 4 | Stage 4 | Possession and player dialogue | — |
| 5 | Stage 5 | Custom pawn and roster | — |
| 6 | Stage 6 | Interiors, jobs, economy, property | implement S-016 template variants once there are ≥ 50 templates |
| 7 | Stage 7 | Relationships v2, personality, life cycle | — |
| 8 | Stage 8 | Crime, injury, services, filter | — |
| 9 | Stage 9 | Governance | — |
| 10 | Stage 10 | LLM proposals | — |
| 11 | Stage 11 | API 1.0, tooling, packaging, accessibility, performance | polish S-013 JSON Schema export, S-014 content diff, S-012 hints for scripts; S-020 cookbook gallery |

## Standing conventions

- Workspace layout, crate roles and the dependency rule are in Blueprint §1–2. `scripts/check_deps.py` enforces them.
- Lint policy for `pg-core`: `forbid(unsafe_code)`, deny float arithmetic, `unwrap`/`expect`/indexing, disallowed collections and clocks (see `crates/pg-core/clippy.toml`).
- Dev-mode features are developer-only, logged as `SimInput`s when they mutate state, and never compiled into paths that affect release determinism.
- Every new `pg` API function is defined in `pg-api` first (Blueprint §23.1); no hand-written second surface.
- Every new RNG stream name is registered (S-005, from 0.5).
