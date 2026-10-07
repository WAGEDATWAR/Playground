# Playground — Build Plan

Working plan for building Playground in Rust. The three documents in `docs/reference/` are the **binding specification**:

- `architecture-blueprint-v2.0.md` — how (crates, determinism, scripting, persistence)
- `design-document-v3.0.md` — what and why (pillars, scope, modding boundaries)
- `roadmap-v4.0.md` — when (stages, gates, decisions to confirm)

This plan turns them into an ordered build. It is a working document: status lives in `TODO.md`; ideas that go beyond the spec live in `SUGGESTIONS.md`; deviations from the spec are recorded in `DECISIONS.md`.

## Working agreements

1. **Spec first.** If code and spec disagree, stop and either fix the code or record a decision (and a spec version bump) before continuing.
2. **Tests and dev tools ship with each component.** A milestone is not done until it has unit tests, property tests where the spec calls for them, and a developer-mode feature (CLI command, overlay panel or inspector view) that lets a human see the component working.
3. **Determinism is a feature, tested continuously.** Golden replay hashes grow with the sim and must stay identical across OSes.
4. **Verification gates.** I stop and hand over at the end of every phase (and at marked mid-phase checkpoints) with: what was built, how to run it, test results, deviations, open questions. Work does not continue past a gate until the gate is accepted.
5. **Small commits, tagged milestones.** One logical change per commit; a git tag per accepted milestone (`m0.1`, …) so any step can be rolled back.
6. **Questions go to the user**; guesses go in `DECISIONS.md` only when the choice is cheap to reverse.

## Phase 0 — Reliable foundations (Roadmap Stage 0)

Goal: a deterministic, headless, saved/reloaded sample town; a proven sandboxed script path; cross-OS identical hashes. No gameplay yet.

| Milestone | Scope (crate) | Tests | Dev-mode feature |
| --- | --- | --- | --- |
| **0.0 Bootstrap** | Toolchain pinned, workspace builds, CI (fmt, clippy, test on Windows/Linux/macOS), lint policy, `cargo-deny`, dependency-rule check script, GitHub repo | CI green on all three OSes | `pg --version`, CI badge |
| **0.1 Determinism primitives** ✅ implemented (`pg-core`) | `EntityId`/`Kind`, `Table<T>`, newtypes (`Permille`, `NeedValue`, `Affinity`) with checked/saturating ops, pinned hash + `rand`/streams/`chance`/`shuffle`, canonical serialization, `blake3` state hash | Published test vectors for hash and rand; property tests (stream independence, shuffle is a permutation, canonical form stable); lint-deny checks | `pg rand <stream> <keys>`, `pg hash <file>` |
| **0.2 Time, tick pipeline, replay** ✅ implemented (`pg-core`, `pg-cli`) | `Clock`, tick/minute/slot/day flags, fixed system pipeline with empty systems and extension-point slots, `SimInput` log + ordering, `TickReport`, deterministic replay | Time wrap, boundary flags, input ordering, replay = uninterrupted run, snapshot-mid-run equivalence | `pg sim --seed --days`, `pg replay`, per-day hash log |
| **0.3 Content system** (`pg-content`) | `ObjectTemplate`, resolver (extends, merge, cycles, depth), component registry (built-in components), validators with `ValidationReport`, pack manifest + data-only loader, `data/base` skeleton | Resolver goldens, cycle/depth rejection, schema validation, manifest validation, fuzz targets (manifest) | `pg content lint`, `pg content resolve <id>` |
| **0.4 World state and spatial** (`pg-core`) | `WorldState`, `MapData`, occupancy, containment + `validate_containment`, A\* with fixed tie-breaks, parallel batched pathing with deterministic merge, movement step with lower-id-first rule | Containment invariants (property), A\* optimality vs brute force, parallel == serial at any thread count, blocked-destination cases | `pg map show`, path overlay output, containment checker |
| **0.5 Scheduler and actions skeleton** (`pg-core`) | `DaySchedule`, reservations, five priorities, conflict/tie rules, `plan_day` skeleton, commitments protocol, `ActionRegistry` (closed), `ReasonCode` with origin | Property: no overlap, priority never inverted, commitments reserve both-or-neither; conflict goldens | `pg schedule explain <pawn> <day>` printing reason codes |
| **0.6 Persistence** (`pg-persist`) | `Storage` use, atomic write, `.pgsave` format (header + zstd canonical JSON), manifest + two generations, load/recovery, migrations framework, export/import skeleton, hardened archive reader | Fault injection (truncate, flip byte, missing file), migration fixture tests, archive traversal/bomb tests, save→load→hash equality | `pg save inspect`, `pg save verify` |
| **0.7 Host services and AI client skeleton** (`pg-host`, `pg-host-os`, `pg-ai`) | Traits + test doubles; OS impls (fs, credential store, HTTPS, dialogs); provider adapters (pure), circuit breaker, cooldowns, `redact()`, settings flow | Adapter fixture tests, hostile outputs, **sentinel-key leak test**, breaker state tests | `pg ai test --provider`, redaction self-check |
| **0.8 Runtime and threading** (`pg-runtime`) | Sim thread with fixed-step accumulator, command queue, worker pool, deterministic job-result application, `RenderSnapshot` triple buffer, run states, autosave policy, last-resort `catch_unwind` guard | Same hashes at 1/2/N workers, run-state transitions, autosave triggers, panic-guard test | Tick profiler (`tracing` spans), dev-tool registry skeleton |
| **0.9 ScriptVm spike** (`pg-script`) | `ScriptVm` trait, `mlua`-Luau implementation, sandbox profile (§23.4), deterministic fuel, memory limit, source-only loading, value boundary, one component + system + hook driven by a test pack, quarantine, safe mode | Hostile-pack corpus, VM-reload determinism variant, answers to the 7 spike questions recorded in `docs/spikes/scriptvm.md` | `pg pack lint/test` (minimal), pack inspector output |
| **0.10 Dev overlay shell** (`pg-app`, `pg-render`) | Minimal window (winit + wgpu clear + egui), dev-mode overlay: tick/day, hash, per-system times, reason log, pack status, quarantine list | Snapshot-builder tests; smoke test that the shell starts headless-safe | The overlay itself |
| **0.11 Stage 0 gate** | Cross-OS golden replay in CI, soak test, acceptance scenario "deterministic sample town runs, pauses, saves, reloads, migrates, recovers" | The Roadmap Stage 0 "Done when" checks | Gate report |

Checkpoints for your review: after **0.2** (deterministic core + replay works), after **0.6** (saves), after **0.9** (script spike decision), and at **0.11** (phase gate).

## Later phases (outline; detailed when reached)

Each later phase follows Roadmap §7 stage items. Per stage I will produce a detailed milestone table like the one above before coding.

| Phase | Roadmap stage | Headline |
| --- | --- | --- |
| 1 | Stage 1 | Observation-first town: needs, mood, memory, conversations, worldgen v1, renderer + egui screens, base pack, API 0.1 |
| 2 | Stage 2 | Free-time scheduler, commitments, API 0.2 (systems, actions) |
| 3 | Stage 3 | Generator controls, editor, mutation API, API 0.3 (worldgen) |
| 4 | Stage 4 | Possession and player dialogue |
| 5 | Stage 5 | Custom pawn and roster |
| 6 | Stage 6 | Interiors, jobs, economy, property |
| 7 | Stage 7 | Relationships v2, personality, life cycle |
| 8 | Stage 8 | Crime, injury, services, filter |
| 9 | Stage 9 | Governance |
| 10 | Stage 10 | LLM proposals |
| 11 | Stage 11 | API 1.0, tooling, packaging, accessibility, performance |

## Standing conventions

- Workspace layout, crate roles and the dependency rule are in Blueprint §1–2. `scripts/check-deps` (milestone 0.0) enforces them.
- Lint policy for `pg-core`: `forbid(unsafe_code)`, deny float arithmetic, `unwrap`/`expect`/indexing, disallowed collections and clocks (see `crates/pg-core/clippy.toml`).
- Dev-mode features are developer-only, logged as `SimInput`s when they mutate state, and never compiled into paths that affect release determinism.
- Every new `pg` API function is defined in `pg-api` first (Blueprint §23.1); no hand-written second surface.
