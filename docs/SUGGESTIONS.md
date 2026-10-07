# Suggestions

Ideas that go beyond the reference specification: system additions and expansions, content, feature variations, tooling and process insights. Accepted items are scheduled in `PLAN.md` and folded into the spec documents (Blueprint v2.6, Roadmap v4.3, Design Document v3.1) at the place they belong; anything that changes the spec is also recorded in `DECISIONS.md`.

Format: **ID — title** · area · status (`proposed` / `accepted` / `rejected` / `done`) · why · cost · **target**.

## Where each accepted item lands

Ordering principle: do small, high-leverage tooling when the thing it observes is about to grow; do each item at the milestone where its prerequisite first exists; defer anything that needs systems that don't exist yet.

| ID | Item | Target | Why there |
| --- | --- | --- | --- |
| S-002, S-007, S-008 | Hash diff, per-table hashes in logs, replay bisect | **0.4** | The world is about to gain its first real tables (maps, objects, pawns); divergence localization pays off immediately |
| S-009 | Event log filter | **0.4** | A few lines; movement starts emitting many events |
| S-012 | "Did you mean…?" hints | **0.4** (content), extended **0.9** (scripts) | Pure polish on 0.3's validator; reused by `pg pack lint` |
| S-005 | RNG stream registry | **0.5** | The scheduler adds the `sched.*` streams; registry prevents silent typos before there are many |
| S-006 | Typed draw helpers | on demand, first use **0.5 / Stage 1** | Add only what the scheduler and conversations actually need |
| S-001 | Bug bundle (snapshot + inputs + content refs) | **0.6** | Needs real snapshots and saves |
| S-003 | Scenario files | format **0.6**, used by gate **0.11** and every later gate | Needs save/reload steps; makes "Done when" lines executable |
| S-004 | Canon vs RFC 8785 cross-check | **0.6** | Canonical JSON becomes the save format there |
| S-014 | `pg content diff` + load-time compatibility report | **0.6** | Saves record `content_refs`; mismatch needs a report |
| S-013 | JSON Schema export for pack files | **0.9** (data schemas), polish **Stage 11** | Ships with pack tooling and the `pg.d.luau` generator |
| S-020 | Mod cookbook sample packs as CI conformance | first three **0.9**, grows each stage, gallery **Stage 11** | Tests API ergonomics from the first script spike |
| S-010 | Reason-code explorer | CLI `pg schedule explain` **0.5**, overlay panel **0.10** | Reason codes exist from 0.5; the overlay is 0.10 |
| S-011 | Soak scenario file | **0.11** | Needs scenarios (S-003) and the full Phase 0 loop |
| S-015 | Inheritance tree view | **Stage 1** | Trivial; only useful once content grows past ~15 templates |
| S-016 | Template variants | evaluate end of **Stage 1**, implement when ≥ 50 templates (expected **Stage 6**) | Premature before there is repetitive content to prove the need |
| S-017 | A\* scratch buffers | **0.8** profiling pass (only if paths show up in the profile) | Premature before real profiling; guarded by existing equivalence tests |
| S-018 | Pawn-aware routing | **Stage 1** | Only matters once pawns genuinely stand still (working, sleeping, talking) |
| S-019 | Compressed, trimmable replay logs | **0.6** with S-001 | Persistence brings zstd; bug bundles need small logs |
| S-021 | Map region labels for instant reachability | **0.8** profiling pass, or earlier if failure noise hurts | The dev town already shows `unreachable_or_too_far` from walled pockets; a flood-fill label per map edit version makes "no path" an O(1) answer |

## Process and tooling

- **S-001 — Record/replay bug reports** · tooling · done · Every save already stores a seed and content refs, and every run produces a `SimInput` log. A "Save bug bundle" dev action (`pg bugbundle`, and a dev-overlay button at 0.10) exports the snapshot + input log + pack hashes + tick-hash trail so any odd behavior is reproducible headlessly. · Low, mostly falls out of 0.2/0.6. · **Target: 0.6.** **Done in 0.6** (`pg bugbundle create/run`; a bundle is a trimmed replay log in a compressed container).
- **S-002 — Hash-diff tool** · tooling · accepted · When golden hashes diverge across OSes, a per-table hash diff (`pg replay --diff a b`) points at the first differing day and table instead of just "mismatch". · Low. · **Target: 0.4.**
- **S-003 — Deterministic scenario files** · testing · accepted · Small JSON "scenario" files (seed, packs, scripted inputs, steps such as save/reload, expected assertions) so acceptance tests for each gate read like the Roadmap's "Done when" lines. · Low–medium. · **Target: format at 0.6; used by 0.11 and every later gate.** Format and runner done in 0.6 (`pg scenario run`, `scenarios/persistence.json`); the gate scenarios arrive with 0.11.

## Systems

- **S-010 — Reason-code explorer** · explainability · accepted · Since every decision carries a reason and an origin, a filterable timeline ("why did pawn_1a skip lunch?") is the main debugging tool for designers and modders. · Medium. · **Target: `pg schedule explain` at 0.5; dev-overlay panel at 0.10.**

## Content

- **S-015 — Inheritance tree view** · tooling · accepted · `pg content tree` printing the whole `extends` forest (or Graphviz output) with pack colors, to see what extends what and spot accidental deep chains. · Low. · **Target: Stage 1.**
- **S-016 — Template "variants" for content breadth** · content · accepted (evaluate first) · Beyond single inheritance, allow a data-only `variants` list (e.g. `chair` in wood/metal/plastic with different `value` and `appearance.sprite`) that expands to several templates at load time. Cheap way to grow object variety without near-duplicates, still pure data. · Medium. · **Target: evaluate at end of Stage 1; implement when there are ≥ 50 templates (expected Stage 6).**

## Modding

- **S-020 — "Mod cookbook" sample packs as CI conformance tests** · modding · accepted · Already implied by Blueprint §23.12; write the first three samples (new need, new action, new hook) during the 0.9 spike so API ergonomics get tested early; each later stage adds a sample for what it exposes. · Low. · **Target: 0.9, then each stage; gallery at Stage 11.**
- **S-012 — "Did you mean…?" hints in validation** · modding DX · accepted · For `unknown_field`, `unknown_component`, `bad_enum`, `missing_parent` and similar, suggest the closest known name (edit distance) in the message. Typos are the most common pack bug. · Low. · **Target: content validator at 0.4; scripts/API names at 0.9.**
- **S-013 — JSON Schema export for editors** · modding DX · accepted · Generate JSON Schema files from the component registry and template/manifest formats (`pg content schema`), so VS Code gives pack authors inline validation and autocomplete with zero tooling. Pairs with the generated `pg.d.luau` for scripts (Blueprint §23.12). · Low–medium. · **Target: 0.9 (data schemas); polish at Stage 11.**
- **S-014 — `pg content diff <old> <new>` and a load-time compatibility report** · tooling · done · Compare two versions of a pack and report removed/renamed templates, changed component schemas and changed defaults, flagged as save-breaking or safe. Gives the "compatibility report" in Blueprint §4.2 a concrete form, used when a save's `content_refs` no longer match installed packs. · Medium. · **Target: 0.6.** **Done in 0.6** (`pg content diff`, and the compatibility report returned by every save load and import).

## Determinism and testing

- **S-004 — Cross-check `Canon` against an independent JSON canonicalizer** · testing · done · Property-test that `Canon::to_canonical_string` equals RFC 8785 (JCS) output for integer-only documents using a third-party implementation as a dev-dependency, so a bug in our escaping/ordering cannot hide behind our own vectors. · Low. · **Target: 0.6 (when canonical JSON becomes the save format).** **Done in 0.6** (property test against an independent RFC 8785 serializer; found and pinned the key-order difference, D-022).
- **S-005 — Compile-time registry of RNG stream names** · determinism · accepted · A small registry (const list + test that every `rand`/`Rng::new` call site uses a registered name, mirrored in `pg-api` for pack streams) catches typos that silently create a "new" stream. · Low–medium. · **Target: 0.5.**
- **S-006 — Typed draw helpers** · determinism · accepted (on demand) · `weighted_pick`, integer "normal-ish" (sum of draws) and `dice(n, sides)` helpers once scheduling and conversation show which shapes they need; avoids each system hand-rolling its own sampling. · Low. · **Target: on demand, first use at 0.5 / Stage 1.**
- **S-007 — Make `StateHash` human-diffable in logs** · tooling · accepted · Per-table short hashes (`pawns:3fa9b2c1 objects:…`) in every day-hash line; **already printed by `pg sim` since 0.2**; 0.4 also stores them in replay logs so S-002 and S-008 can use them. · Low. · **Target: done in 0.2; log storage at 0.4.**
- **S-008 — Replay bisect tool** · tooling · accepted · `pg replay --bisect <a.json> <b.json>` runs two logs in lockstep with a per-tick hash and reports the first tick (and table) where they diverge, turning "the cross-OS golden hash differs" into a one-line answer. · Low. · **Target: 0.4.**
- **S-009 — Event log filter/search in `pg sim --events`** · tooling · accepted · `--events <kind-prefix>` and `--since/--until` so long runs stay readable; the same filter becomes the dev-overlay event viewer. · Low. · **Target: 0.4.**
- **S-011 — Soak scenario file** · testing · accepted · A scenario file (seed, packs, scripted inputs, day count, expected invariants such as memory/event bounds) so the Roadmap's "30+ day soak" becomes a one-line CI job. Builds on S-003. · Low–medium. · **Target: 0.11.**

## Performance and simulation (added in 0.4)

- **S-017 — A\* scratch buffers instead of a sparse map** · performance · accepted · Path search keeps its scores in a `BTreeMap` for simplicity (about 4 µs per node expanded; `pg map bench-paths` shows ~3 ms per request on a 160x120 town). A reusable dense `Vec` stamped with a search generation (one per worker thread) removes the allocation and the tree lookups and should be several times faster, with identical results (the Dijkstra property test and the threaded-vs-serial test guard it). Do it when profiling at 0.8 / Stage 11 says paths matter, not before. · Low–medium. · **Target: 0.8 profiling pass.**
- **S-018 — Pawn-aware routing** · simulation · accepted · Today paths ignore other pawns, so a pawn blocked by a stationary pawn waits, sidesteps and finally fails (D-015). Treating *idle* pawns as temporary obstacles in the search (with the cache keyed accordingly), or adding a small "crowd cost" on occupied tiles, would route around standing pawns and doorway clumps. Needs care to keep results deterministic and cacheable. · Medium. · **Target: Stage 1 (when pawns stand still for real: working, sleeping, chatting).**
- **S-019 — Replay log compression and `pg replay --trim`** · tooling · done · Long logs (many days, many pawns) get large because every input is stored as JSON. A zstd-compressed `.pgreplay` form (the persistence crate already brings zstd at 0.6) and a `--trim <tick>` to cut a log to the first N ticks would keep bug bundles (S-001) small. · Low. · **Target: 0.6 with S-001.** **Done in 0.6** (`.pglog` compressed logs, `ReplayLog::trim`, bundles).
- **S-021 — Map region labels for instant reachability** · pathfinding · accepted · Label connected passable regions per map (flood fill, recomputed when `edit_version` changes, derived state like the path cache). "Is A reachable from B?" becomes one comparison, so a planner or the dev plan source can skip unreachable destinations, `blocked_destination` and `unreachable_or_too_far` become exact, and A\* is never run for hopeless requests (today it exhausts the expansion cap). Seen in 0.5: the dev town regularly produces unreachable pockets between buildings. · Low-medium. · **Target: 0.8 profiling pass (earlier if the failure noise becomes a problem).**

## Proposed in 0.7 (awaiting your decision)

An inspection pass over the code as it stands, looking for the cheapest places to buy leverage before the UI and runtime are built on top. Each has a recommended target; none is scheduled until you accept it.

| ID | Item | Recommended target | Cost | Why now |
| --- | --- | --- | --- | --- |
| S-022 | Typed event catalog | 0.8 (debug validation), 0.10 (viewer) | Low-medium | Events are `(kind: text, detail)`; the overlay's event viewer, filters and pack events need a list of known kinds with field schemas |
| S-023 | Row-level state hashes | 0.8 | Low-medium | Bisect names the table; a Merkle layer names the entity (`pawns: pawn_3`), which is what a person debugging actually needs |
| S-024 | Save summaries in the manifest | 0.10 | Low | The Saved Worlds screen needs name, day, population, play time and an optional thumbnail without loading each world |
| S-025 | Schema-driven settings | 0.10 (registry and generated screen; 0.7 shipped the AI settings by hand) | Medium | One typed registry gives validation, defaults, `device.json`, `pg settings` and the Options screen; the AI flow in 0.7 is the first user |
| S-026 | String table (localisation-ready UI text) | 0.10 | Low-medium | The menus are the first player-facing text; keys from the start are cheap, retrofitting is not; reason-code sentences move into the same table |
| S-027 | Headless UI snapshots and an accessibility tree | 0.10 | Medium | Menu models emit a widget tree that tests snapshot and keyboard-navigate; egui's accessibility support then comes almost free |
| S-028 | Automatic bug bundle on panic or divergence | 0.8 (runtime guard), 0.10 (button) | Low | A crash leaves a replayable `.pgbundle` (redacted) instead of a description |
| S-029 | Keyframe snapshots and time scrub | 0.8 (ring), 0.10 (overlay control) | Medium | A bounded ring of snapshots lets the dev overlay step back and re-run forward deterministically; it also feeds S-028 |
| S-030 | Shadow determinism verification | 0.8, soak in 0.11 | Medium | A worker re-simulates from a keyframe with a different thread count and compares day hashes, catching ordering bugs that unit tests miss |
| S-031 | Reason-code coverage lint | **done in 0.7** | Tiny | Every `ReasonCode::builtin("...")` literal in the source must have a sentence template and vice versa; a ten-line test |
| S-032 | `pg check`: one-command verification report | **done in 0.7** | Low | Runs selftest, content lint, golden replay, scenarios and a save/verify cycle and prints one report; makes each checkpoint a single paste-able command and mirrors CI |

Details:

- **S-022 Typed event catalog** · tooling · proposed · Declare each event kind once (kind, category, field schema, default visibility) using the same `ParamSchema` as components. Debug builds validate every `emit`; `pg events list` and the overlay show the catalog; packs register `<pack>.<kind>`. Replay and hashing are unaffected (events are not state).
- **S-023 Row-level state hashes** · tooling · proposed · Hash each table row (entity) and combine rows into the table hash. Memory and CPU cost are about the same as hashing the table whole. `pg replay --bisect` can then say which entity differs; day hashes in logs stay per-table, with an optional per-row dump on mismatch.
- **S-024 Save summaries** · UX · proposed · Additive manifest fields: world name, day, population, play ticks, content packs, last-saved time, optional small thumbnail (rendered by the app). No world load is needed to draw the Saved Worlds list, and damaged slots still list.
- **S-025 Schema-driven settings** · architecture · proposed · A registry of typed settings (id, type and range, default, label key, scope device or world, restart-required) built on `ParamSchema`. Generates the Options screen, `pg settings get/set/list`, validation and migration of `settings/device.json`, and documentation. Keys never appear in it (they stay in `SecretStore`).
- **S-026 String table** · UX · proposed · All player-visible text comes from keyed tables (`strings/en.json` in the base pack, pack-namespaced keys for packs). Reason-code sentences, error messages and menu labels use it; a lint reports missing and unused keys; a pseudo-locale (accented, 40% longer) exposes layout problems early.
- **S-027 Headless UI snapshots and accessibility** · testing · proposed · Screens emit a small widget tree (labels, buttons, fields, focus order). Tests snapshot the tree as text and assert keyboard-only reachability; the egui layer maps the same tree to its accessibility output. Accessibility is a Stage 11 release item in the Roadmap; this makes it incremental instead of a rewrite.
- **S-028 Automatic bug bundle** · reliability · proposed · The runtime's last-resort guard and the divergence detector write a redacted `.pgbundle` (latest keyframe plus inputs since) to a `crash/` folder; the overlay and the next launch offer to open it.
- **S-029 Keyframe snapshots and time scrub** · tooling · proposed · A memory-bounded ring of snapshots (for example one per simulated hour, last 24). The dev overlay can rewind to one and replay forward; a "what if" input injected after a rewind runs deterministically.
- **S-030 Shadow determinism verification** · reliability · proposed · In dev and soak runs a worker re-runs the span between two keyframes on a different thread count and compares per-table hashes at day boundaries; a mismatch writes a bundle (S-028) and flags the system that ran last.
- **S-031 Reason-code coverage lint** · testing · done · Implemented in 0.7 (`pg-core` test scanning the source tree).
- **S-032 `pg check`** · tooling · done (0.7) · One command: determinism vectors, content lint, golden replay with snapshot check, persistence scenario, a save/verify cycle and a bug-bundle round trip, with a final PASS/FAIL table. Exit code for CI; the same output is what you paste back at a checkpoint.
