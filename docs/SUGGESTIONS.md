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
| S-022 | Typed event catalog | catalog and debug validation **0.8**, event viewer **0.10** | The overlay's event viewer, filters and pack events need a list of known kinds; validation is cheapest before many event kinds exist |
| S-023 | Row-level state hashes | **0.8** | Names the diverging entity, not just the table; cost is the same as hashing tables whole |
| S-024 | Save summaries in the manifest | summary fields **0.8**, thumbnails and list UI **0.10** | The Saved Worlds screen needs them; additive manifest change is trivial now, awkward after more saves exist |
| S-025 | Schema-driven settings | registry **0.8**, generated screen **0.10** | The AI settings are the first hand-written settings; moving them onto a registry before more are added avoids a rewrite |
| S-026 | String table (localisation-ready text) | core **0.8**, menu strings **0.10** | Reason-code sentences exist already; moving them first proves the mechanism before UI text multiplies |
| S-027 | Headless UI snapshots and accessibility tree | **0.10** | Needs the screen models; builds keyboard-first menus from the start |
| S-028 | Automatic bug bundle on panic or divergence | guard **0.8**, prompt **0.10** | The panic guard and divergence detector land in 0.8; bundles and keyframes already exist |
| S-029 | Keyframe snapshots and time scrub | ring **0.8**, scrub control **0.10** | Snapshots and restore exist; the ring is small and feeds S-028 and S-030 |
| S-030 | Shadow determinism verification | **0.8**, soak use **0.11** | Needs the worker pool; catches ordering bugs before gameplay systems multiply |
| S-031 | Reason-code coverage lint | **done in 0.7** | Tiny guard test |
| S-032 | `pg check` | **done in 0.7** | One-command verification report |
| S-034 | Real batch calls into scripts | **Stage 1**, only if the first real system profile warrants it | 0.9 measured 12 µs per call, 1.6% of the tick at 200 pawns |
| S-035 | Luau type check in `pg pack lint` | evaluate with **Stage 11** editor support | Needs Luau's analyzer built or bundled |
| S-036 | Script cost view in the overlay | **0.10** | The numbers exist already (fuel per call, profiler probe) |
| S-037 | Native file dialogs | **Stage 1** (with the Mods screen and pack installation) | Replaces the 0.10 exports and imports folders |
| S-038 | Accessibility output (AccessKit) | **Stage 1** enable and test; full screen-reader pass **Stage 11** | The widget tree already has names, roles and focus order |
| S-039 | Living main-menu backdrop | **Stage 1** (presentation) | One small paused sim and the existing map view |
| S-033 | Player2 extras (voice, NPC and game-data endpoints) | evaluate **Stage 4** (possession dialogue) and **Stage 11** (audio) | Not needed for text generation; recorded so the options are not forgotten |
| S-049 | Public-repository hygiene | **done now** (licence, README, SECURITY.md, history scan); settings left to the owner | D-047 |
| S-050, S-051, S-052, S-059, S-065, S-066, S-068 | CI cost, one command for pins, plateau soak, pin triage, social property tests, schema fingerprint, `pg milestone` | **1.4b** (a small tooling milestone before 1.5) | They make every later milestone cheaper and safer |
| S-060 | Local Linux check | opportunistic | Very low priority; only when easy |
| S-054, S-057, S-062 | Remember-when callbacks, per-turn tone and line bands, dialogue lint | **1.5** | They share one structure with the AI prompt and the fallback lines |
| S-069, S-071 | Social report; benchmark suite | **1.4b** (suite wrapping what exists), **1.6** (social report before tuning, render targets) | |
| S-056, S-058, S-067 | Personality slice, town journal (observation and possession modes only), console extras | **1.7** | Journal needs the 1.7 UI; see D-048 for the modes |
| S-061, S-064, S-070 | Conversation scale check, save size watch, story digests | **1.8** (profile pass), Stage 1 gate report, **1.9** (digest golden in `pg check --full`) | |
| S-055 | Friends seek each other out | **Stage 2** | Uses commitments |
| S-053 | Gossip and second-hand memories | **Stage 3** | |

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

## Accepted in 0.7

An inspection pass over the code as it stood after 0.6, looking for the cheapest places to buy leverage before the UI and runtime are built on top. S-022 to S-030 were accepted and scheduled (placement table above, Roadmap v4.4, Blueprint v2.7); S-031 and S-032 were done in 0.7.

Placement, targets and reasons for each are in the table at the top of this file.

Details:

- **S-022 Typed event catalog** · tooling · accepted · Declare each event kind once (kind, category, field schema, default visibility) using the same `ParamSchema` as components. Debug builds validate every `emit`; `pg events list` and the overlay show the catalog; packs register `<pack>.<kind>`. Replay and hashing are unaffected (events are not state).
- **S-023 Row-level state hashes** · tooling · accepted · Hash each table row (entity) and combine rows into the table hash. Memory and CPU cost are about the same as hashing the table whole. `pg replay --bisect` can then say which entity differs; day hashes in logs stay per-table, with an optional per-row dump on mismatch.
- **S-024 Save summaries** · UX · accepted · Additive manifest fields: world name, day, population, play ticks, content packs, last-saved time, optional small thumbnail (rendered by the app). No world load is needed to draw the Saved Worlds list, and damaged slots still list.
- **S-025 Schema-driven settings** · architecture · accepted · A registry of typed settings (id, type and range, default, label key, scope device or world, restart-required) built on `ParamSchema`. Generates the Options screen, `pg settings get/set/list`, validation and migration of `settings/device.json`, and documentation. Keys never appear in it (they stay in `SecretStore`).
- **S-026 String table** · UX · accepted · All player-visible text comes from keyed tables (`strings/en.json` in the base pack, pack-namespaced keys for packs). Reason-code sentences, error messages and menu labels use it; a lint reports missing and unused keys; a pseudo-locale (accented, 40% longer) exposes layout problems early.
- **S-027 Headless UI snapshots and accessibility** · testing · accepted · Screens emit a small widget tree (labels, buttons, fields, focus order). Tests snapshot the tree as text and assert keyboard-only reachability; the egui layer maps the same tree to its accessibility output. Accessibility is a Stage 11 release item in the Roadmap; this makes it incremental instead of a rewrite.
- **S-028 Automatic bug bundle** · reliability · accepted · The runtime's last-resort guard and the divergence detector write a redacted `.pgbundle` (latest keyframe plus inputs since) to a `crash/` folder; the overlay and the next launch offer to open it.
- **S-029 Keyframe snapshots and time scrub** · tooling · accepted · A memory-bounded ring of snapshots (for example one per simulated hour, last 24). The dev overlay can rewind to one and replay forward; a "what if" input injected after a rewind runs deterministically.
- **S-030 Shadow determinism verification** · reliability · accepted · In dev and soak runs a worker re-runs the span between two keyframes on a different thread count and compares per-table hashes at day boundaries; a mismatch writes a bundle (S-028) and flags the system that ran last.
- **S-031 Reason-code coverage lint** · testing · done · Implemented in 0.7 (`pg-core` test scanning the source tree).
- **S-032 `pg check`** · tooling · done (0.7) · One command: determinism vectors, content lint, golden replay with snapshot check, persistence scenario, a save/verify cycle and a bug-bundle round trip, with a final PASS/FAIL table. Exit code for CI; the same output is what you paste back at a checkpoint.
- **S-033 — Player2 extras** · integration · accepted · Player2's API also offers NPC conversation endpoints with server-side memory (`/npcs/*`), a key-value game-data store (`/game-data/batch`), text-to-speech (`/tts/*`) and speech-to-text (`/stt/*`). None is needed for text generation, which uses `/chat/completions` like the other providers. Possible later uses: spoken dialogue lines for players who opt in (Stage 11 audio), voice input for possession dialogue (Stage 4), and nothing for the simulation itself (which must stay deterministic and local). Cost: medium per feature; **evaluate at Stage 4 and Stage 11**, and only behind the existing capability and filter settings.
- **S-034 Real batch calls** · performance · accepted · `call_batch` currently loops over `call` (12.5 µs per entity). A batch entry point in the prelude could build the context once and enter Lua once per batch; worth doing only if a profile shows scripts above a few percent of the tick (0.9 spike measured 1.6% for 200 pawns).
- **S-035 Luau type check in `pg pack lint`** · tooling · accepted · Run Luau's analyzer over scripts with the generated `pg.d.luau` so `--!strict` errors show up before load. Needs the analyzer built or bundled; evaluate with the Stage 11 editor-support work.
- **S-036 Script cost view in the overlay** · tooling · accepted · Show fuel and time per pack, per system and per hook next to the tick profiler, so authors see which of their handlers is expensive (the numbers already exist: `fuel_used` and the profiler probe).
- **S-037 Native file dialogs for import and export** · UX · accepted · Replace the exports and imports folders with the operating system's open and save dialogs (the `Dialogs` host trait already exists). Needs a dialog crate whose Linux backend does not require a GTK build; evaluate `rfd` with its portal backend.
- **S-038 Accessibility output** · accessibility · accepted · Turn on egui's `accesskit` feature so screen readers see the widget tree (names, roles, focus order, the secret-field role). The tree already has everything needed; the work is enabling it and testing on the three platforms.
- **S-039 A living main-menu backdrop** · polish · accepted · Run a tiny seeded town behind the main menu (paused after a few days, slowly panning) so the first screen already shows the game. Uses the same view; costs one small sim at launch.

## Accepted with the organism system (2026-10-08)

All eight belong to the organism proposal (`docs/proposals/organism-system.md`) and are scheduled with it in Roadmap v4.8; all accepted (D-040).

- **S-040 Capacities layer in Stage 1** · architecture · accepted · Movement, scheduling, conversation and actions read a small record of derived capacities (consciousness, moving, manipulation, talking, eating, breathing) instead of needs or body parts. Stage 1 fills it from needs; the organism takes over in Stage 2B. Keeps the body model from leaking into every system and gives mods a stable contract from API 0.1. · Low. · **Target: Stage 1 (1.1).**
- **S-041 Declarative effect programs** · modding, performance · accepted · Status effects are data (cadence, condition, bounded vital contributions, transitions, reason keys) executed natively by the organism; Luau only at bounded, batched hooks. Avoids a script call per effect per pawn per tick (12 microseconds each) and keeps clamps enforceable. · Medium. · **Target: Stage 2B.**
- **S-042 Dormancy and acute stepping with a cost budget** · performance · accepted · Healthy pawns do no organism work; acute pawns step more often; per-pawn steps run on the worker pool with ordered application; a profiler budget (under 5 percent of the tick at 200 pawns) is a CI check. · Medium. · **Target: Stage 2B.**
- **S-043 Clinical time scale** · design · accepted · A per-world setting that divides authored per-minute physiology rates, so a bleed-out can last game hours instead of seconds of real time, without changing the model. · Low. · **Target: Stage 2B.**
- **S-044 Physiology goldens and a plausibility lint** · testing, modding DX · accepted · Golden vitals traces (healthy, bleed with and without care, drowning, starvation, fever) identical on all systems, and a lint that flags pack effects with implausible rates, unbounded loops or missing description variants. · Medium. · **Target: Stage 2B, polish Stage 11.**
- **S-045 Parts exist regardless of rating** · design · accepted · The body model includes parts per physiology profile; the content rating and the graphic filter only decide what the health window lists and how it words it, so a save never depends on a presentation setting. · Low. · **Target: Stage 2B.**
- **S-046 Blood type inheritance** · simulation · accepted · Born pawns inherit ABO and Rh from their parents by simple genetics instead of rolling randomly; generated adults still roll. · Low. · **Target: Stage 7.**
- **S-047 Ambient hazards service** · simulation · accepted · One place that answers "what is the air, temperature and water like here and now" for the organism (indoors, outdoors, smoke, cold water), with a stub per district and season until weather exists. Needed for drowning, hypothermia and low-oxygen cases. · Medium. · **Target: Stage 2B stub, grows with Stage 3 and 6.**
- **S-048 Weather, seasons and fire** · simulation · parked · Noted to come back to. They would feed the ambient hazards service (S-047) and the organism (cold, heat, smoke, burns), and touch worldgen, rendering and the economy. · Large. · **Revisit when Stage 3 and Stage 6 are planned (Roadmap section 12 item 8).**

## Accepted in the pass after 1.4 (2026-10-08)

All accepted on 2026-10-08 (D-047, D-048), with the notes marked **Decision** below. **Recommended first, in this order:** S-049 (the repository is public now), S-051 and S-059 (pins cost real time every milestone), S-066 (catches a missed schema change), S-050 (keeps the free CI minutes going). The rest are ranked by when their prerequisite exists.

### Repository and process

- **S-049 Public-repository hygiene** · process · accepted · The repository is public now, so: (1) run a secret scan over the whole history once (gitleaks or trufflehog) and add it to CI; the code is designed to keep keys out (`redact()`), but history is what a scan checks, and the Player2 `client_id` is public by design but worth confirming; (2) decide the licence (D-002 is still open, and "UNLICENSED" in a public repository means all rights reserved, which is a legitimate choice but should be stated in the README and a `LICENSE` file); (3) a short README status section (what works, what does not, how to build, that contributions are not open yet), `SECURITY.md` (how to report a problem privately) and branch protection on `main`. · Low, mostly settings. · **Target: now. Done 2026-10-08: MIT licence, README, SECURITY.md and a history scan (D-047). Left for the owner: branch protection, private vulnerability reporting and secret scanning in the repository settings.**
- **S-050 Make CI cheap** · process · accepted · Add `concurrency` with cancel-in-progress, path filters (docs-only changes skip the build), a nightly or manual-only schedule for the 30-day soak and the three-OS determinism matrix, and keep one fast Linux job on every push. `rust-cache` is already used. · Low. · **Target: this week.**
- **S-051 One command for every pin** · tooling · accepted · Re-recording after a state change touches the golden replay, three pack goldens, four scenario hashes, the soak, the town starting hash and two fixtures, by hand, in six different ways. `pg pins check` would list every pin with expected and found values; `pg pins update` would rewrite all of them (after printing the diff for review). `pg check --full` would call it. · Medium; saves 10 to 20 minutes per milestone. · **Target: before 1.6.**
- **S-059 Pin-failure triage** · tooling · accepted · When a pin fails, say *which table changed* and a one-line guess (per-table hashes already exist in replays): "pawns table differs; first differing row pawn_3". Turns "state hash mismatch" into a pointer. · Low to medium. · **Target: with S-051.**
- **S-066 Schema fingerprint check** · testing · accepted · A `pg check` entry that hashes the *shape* of the saved world (field names and kinds written by `to_canon`) and compares it with a recorded fingerprint; changing the shape without bumping the schema or adding a migration fails with a clear message. Today a missed migration is found only when a fixture happens to trip. · Medium. · **Target: before the Stage 1 gate (schema 4 ships then).**
- **S-068 `pg milestone`** · tooling · accepted · Runs the whole milestone checklist (format, clippy, tests, `check --full`, smoke test), prints a one-paragraph status in the form used for gate reviews, and optionally commits, tags and pushes. · Low. · **Target: with S-051.**
- **S-060 Local Linux check** · process · accepted · A script that runs `pg check` inside a Linux container or WSL, so a cross-OS determinism difference can be found on this machine; useful whenever CI is unavailable. · Low. · **Decision: very low priority; only when it is easy to do.** **Target: opportunistic.**

### Conversation, memory and relationships (built on 1.4)

- **S-053 Gossip and second-hand memories** · simulation · accepted · A talk about a third person can pass a memory on as a weaker, less reliable copy ("heard from Tess that Omar was unkind"), tagged with its source and a reliability that drops with each retelling. Gives the town a rumour mechanic from data already there (participants, topic, importance) and a reason for opinions to spread. Needs one memory type and a `gossip` topic. · Medium. · **Target: Stage 3.**
- **S-054 Remember-when callbacks** · simulation, presentation · accepted · A topic that picks one of the pair's shared memories (`select_relevant_memories` already scores them), shows a line naming it and strengthens it a little (rehearsal). Makes long friendships read differently from new ones, and gives the AI prompt in 1.5 real context. · Low to medium. · **Target: with or after 1.5.**
- **S-055 Friends seek each other out** · simulation · accepted · Residents with a friendly or better label propose meet-ups through the existing commitment system, weighted by who they have not seen lately. Turns relationships into movement through the town instead of chance meetings at the plaza. · Medium. · **Target: Stage 2.**
- **S-056 Early personality slice** · simulation · accepted · One signed number ("outgoing to reserved") that scales the chance to start a talk and the social need's decay, with trait tags later. The Blueprint reserves traits for later; one number now makes residents less uniform and costs one field and two multipliers. · Low. · **Target: 1.7 or the Stage 1 gate.**
- **S-057 Per-turn tone and richer line selection** · fidelity · accepted · Tone per turn (a talk can start friendly and sour), and line sets keyed by relationship band and mood band as Blueprint section 9.5 describes. Mostly data plus a small change to the draw. · Medium. · **Target: with 1.5, so AI and fallback lines share one structure.**
- **S-062 Dialogue lint** · testing, modding DX · accepted · `pg content lint` checks every topic and tone has a line set (or a stated fallback), every `{placeholder}` is one the engine fills, line length fits a bubble, and no tone is unreachable. · Low. · **Target: with 1.5.**
- **S-065 Property tests for the social rules** · testing · accepted · `proptest` over random sequences of conversations: affinity stays in range, the daily cap holds across day changes, labels never flicker within the margin, ordinary memories never exceed the bound, persistent ones are never lost. The example tests cover only the cases we thought of. · Low. · **Target: soon.**

### Observability

- **S-069 Social report** · tooling · accepted · `pg sim --social-report`: loneliness histogram, conversations per day, label counts, strongest and weakest pairs, and an ASCII or DOT relationship graph. Tells you in ten seconds whether tuning (cooldown, chance, topic weights) did what you meant, and feeds the inspector's town view in 1.7. · Low. · **Target: before tuning in 1.6.**
- **S-058 Town journal** · UX · accepted · A player-facing feed of notable events built from the event catalog's severity table (new friendships and rifts, collapses, big memories). The data exists; 1.7's UI can render it, and it is the best way to notice a town when no single resident is in focus. **Decision: available in observation mode and possession mode only, never in player mode (D-048).** · Medium. · **Target: 1.7.**
- **S-070 Story digests** · testing · accepted · A per-seed readable digest of notable events (first friendship, first rift, first collapse, busiest talker) recorded as a golden. A diff of prose is easier to review than a hash and shows when a change altered behaviour in a way you can feel. **Decision: the digest golden is part of `pg check --full`** (and of `pg pins`, S-051). · Medium. · **Target: Stage 1 gate.**

### Performance and robustness

- **S-061 Scale check for conversations** · performance · accepted · The pair search is a scan, fine at 14 to 30 residents. Add a 150-resident profile run to `pg check` (budget reported, not failed) and build the 8x8 spatial hash only if the number is bad. · Low. · **Target: 1.8 profile pass.**
- **S-052 Soak checks plateau, not ratio** · testing · accepted · The soak's "state may grow 3x from day 5" is blunt now that memories fade over weeks. Compare the last ten days with the ten before instead (small growth means a steady state) and fail on a rising trend. · Low. · **Target: soon.**
- **S-064 Save size watch** · performance · accepted · Saved state is about 590 KB for 20 residents after 70 days; `pg save create` could report size by table so memory growth is visible. Interning repeated strings is possible later and not needed yet. · Low. · **Target: Stage 1 gate report.**

### Console and tooling

- **S-067 Console extras** · tooling · accepted · Capture pack load-time prints (known gap from 1.2a), a copy-visible-lines button, a save-log-to-file button (redacted) and remembered filter presets. · Low. · **Target: with 1.7.**

### Benchmarking

- **S-071 Central benchmark suite** · tooling, performance · accepted · One tool, `pg bench`, that knows every measurable target (tick profile at a chosen resident count, path search serial and parallel, town generation, conversation scale, save encode and load, replay speed, script cost per pack and hook, soak timing, and later rendering) and runs any selected number of them: `pg bench --list`, `pg bench --targets ticks,paths,worldgen`, `pg bench --all`. Each target declares its parameters, how many runs it makes, and an optional budget. The run writes a physical report file (JSON for tools and a Markdown table for people) under `bench/reports/` named for the date and commit, recording the machine (OS, CPU, threads), build profile, version, the targets with their parameters, median and worst run times, and pass or fail against budgets; `--baseline <report>` adds a comparison column. Timings never touch simulation state or hashes. The existing `pg profile`, `pg map bench-paths` and the `pg check` profile entry become targets of it; the in-game overlay shows the latest report in developer mode. · Medium. · **Target: first version with 1.4b (wrapping what exists), grows with each stage; renderer targets in 1.6; profile pass in 1.8.**
