# Deferrals and stubs

Everything we have chosen **not** to build yet, and everything in the code that is a stand-in for something real.
Deferral discipline: nothing is postponed in conversation only. If it is not built, it is on this page.

## Rules

1. **Same-day entry.** The day something is deferred, stubbed, parked or left as a known limit, it gets an entry here
   (ID `DF-nnn`, never reused) and, if it is a stub in code, a marker comment at the stub: `// DEFERRED: DF-nnn`.
   `scripts/check_deferrals.py` (part of `pg milestone`) fails if a marker names an ID that is not on this page, or
   if an open stub entry has no marker in the code.
2. **Every entry says six things:** what it is, why it is not built, where it lives (code and docs), the target
   (a milestone or stage, or `unscheduled` with a date by which a target must be chosen), what "done" means, and
   the source (decision and suggestion IDs).
3. **Gate review.** At every milestone gate and phase gate, walk this page. Each entry whose target is at or before
   that gate is built, rescheduled (with a `docs/DECISIONS.md` entry and the new target written here) or dropped
   (with a decision). `unscheduled` entries past their date are forced to one of those three.
4. **Closing.** A finished entry moves to **Closed**, with the date and the commit. Closed entries are never deleted.
5. **Kinds.** *Stub* is code standing in for the real thing. *Deferred* is a feature we decided to build later.
   *Limit* is a known weakness of something that exists. *Process* is a check or habit we do not have yet.
   *Parked* is an idea set aside on purpose with no target.

Last reviewed: 2026-10-29, at the start of 1.9 (this page was created then; DF-004 and DF-006 verified in step 1).

## A. Stubs and scaffolding in the code

| ID | Kind | What | Why not done | Where | Target | Done when | Source |
| --- | --- | --- | --- | --- | --- | --- | --- |
| DF-002 | Stub | **Scaffolding town fallback**: when content has no game data, creating a world builds a map with `DevCreateMap` and `DevSpawnPawn` instead of the generator | Stripped-down test packs have no generator data | `crates/pg-runtime/src/app.rs` (`create_world`) | With DF-001 | Every world is made by the generator (tests carry their own fixture content) | D-009 |
| DF-003 | Stub | **Developer commands** (`DevCreateMap`, `DevSpawnPawn`, `DevNudge`, ...) are the only way some tools and tests change the world | They are logged as inputs and developer-only; real gameplay and editor commands replace them over time | `crates/pg-core/src/commands.rs` | Stage 3 (editor and mutation API) and Stage 4 (possession); the dev ones stay dev-only | No release path needs a `Dev*` command | roadmap Stage 3, 4 |
| DF-004 | Stub | **Empty pipeline slots**: Relationship, Event and Maintenance run a no-op system in the built-in pipeline. Verified 2026-10-29 (1.9 step 1): relationship change lives inside the conversation system, and the three slots are the attachment points `pg-script` uses for pack systems, so they stay. What is missing is a built-in system for Event and Maintenance | A built-in Event or Maintenance system has not been needed yet | `crates/pg-core/src/pipeline.rs` (`builtin_system`), `crates/pg-script/src/host.rs` | Stage 2 for a built-in Event system (scheduler events); Maintenance when a system needs it. The slots themselves are kept | Each slot either has a real built-in system or is documented in Blueprint 6 as an extension-only anchor | Blueprint 6, D-080 |
| DF-005 | Stub | **Procedural sprites** stand in for art | No art assets yet; the atlas format is ready for real art | `crates/pg-render/src/sprite.rs`, `atlas.rs` | When you have assets | Real art replaces the generated sprites with no code change outside `pg-render` | D-038 |
| DF-006 | Stub | **Action system pieces, remainder after 2.0**: preconditions and effects are built (D-086). Still missing: **permissions** (who may run an action; needs roles and offices), **costs beyond time** (money), the effects `SetFlag`, `SetField`, `MoveItem`, `SpawnObject` (need flags, ext writes, items, spawn rules), relationship hostility in plan acceptance (2.1) and script weight hooks (2.6) | Nothing in Stages 1 to 2 needs them | `crates/pg-core/src/action.rs`, `effects.rs` | Hostility 2.1; weight hooks 2.6; permissions Stage 9; money costs Stage 6; the other effects with the stage that adds their storage | Each piece is built where listed, or dropped with a decision | D-021, D-080, D-086 |
| DF-007 | Limit | **Developer fixtures stay on purpose** (decided in 1.9, D-084): the dev probe system and the `WorldState.probe` table, the dev plan source, the dev generators, the `dev.*` RNG streams and the `dev.day_started` event. Game worlds (content with game data) run none of them. They stay because the determinism, divergence and keyframe tests use the probe table as their stand-in for drifted state, and the developer commands and benchmarks (`pg sim --dev-map`, `pg profile`) use the dev map and plan source | Removing them means giving those tests another divergence marker, which buys nothing while game worlds are clean | `crates/pg-core/src/dev.rs`, `world.rs` (`probe`), `rng.rs`, `life.rs`; `pg-cli`; `pg-runtime` tests | Reconsider at Stage 11 (packaging), or sooner if they get in the way; the probe table must not appear in a game save before the schema ships | `probe` is gone from the saved state, or the entry is dropped with a decision | D-009, D-045, D-084 |

## B. Deferred features

| ID | Kind | What | Why not done | Where | Target | Done when | Source |
| --- | --- | --- | --- | --- | --- | --- | --- |
| DF-010 | Deferred | **Accessibility output (AccessKit)**: screen-reader names, roles and focus for every screen; keyboard handling of permission checkboxes; focus handling in drawers | The widget tree already carries names, roles and focus order; the work is the egui integration and a full pass | `crates/pg-ui-model/src/widget.rs`, `pg-app` | Stage 11 | A screen reader can use every screen; the keyboard reaches every control | D-033, D-061, D-063, S-038 |
| DF-011 | Deferred | **Audio service** (sound, music, spoken lines) | Trait and null double exist only | `crates/pg-host` | Stage 11 | Real audio behind the trait; optional text-to-speech (S-033) evaluated | D-026, S-033 |
| DF-012 | Deferred | **Real per-world pack choice**: choosing which packs a world uses when it is made | The content set is built once at launch; a world uses what is loaded. New world lists the packs and Manage packs opens Mods | `crates/pg-runtime/src/mods.rs`, `app.rs`; New world screen | **Start of Stage 3** (the editor and generator controls rework world creation; D-085) | A new world can be made with a different pack set than the one loaded at launch, and a saved world records and restores its own set | D-060, D-077, D-078 |
| DF-013 | Deferred | **Pack settings page**: editing the per-world settings a manifest declares | Per-world pack setting storage does not exist | `crates/pg-content/src/manifest.rs` (`PackSetting`), Mods screen | With DF-012 | Declared settings appear in the pack's pane and New world and are saved with the world | D-078 |
| DF-014 | Deferred | **Mods: reorder load order by dragging**, with the order saved | Needs a drag widget and a saved order that the loader honours | Mods screen, `mods.rs` | **Start of Stage 3**, with DF-012 (D-085) | The player can change the order, it is saved, and the load report shows it | D-078 |
| DF-015 | Deferred | **Mods: pack icons and screenshots** | Needs a manifest field and image loading | manifest, Mods screen | Stage 11 (packaging) | A pack may name an icon file that the list and pane show | D-078 |
| DF-016 | Deferred | **Mods: sort options** beyond the fixed order (base, then by name, failures last) | Not needed until lists are long | Mods screen | `unscheduled` | Sort choices exist | D-078 |
| DF-017 | Deferred | **Events (read) for scripts**: scripts cannot subscribe to the event stream; they react through systems and hooks | Kept the API 0.1 surface small | `crates/pg-api` | API 0.2 (Stage 2) | A capability-gated read of the event stream with attribution | D-062 |
| DF-018 | Deferred | **Batch entry point for hooks** (`S-034`) | Script cost was 3.1 percent of the tick at 200 pawns, under the threshold that justified it | `crates/pg-script` | Only if script cost passes a few percent | Measured benefit and a decision | D-062, S-034 |
| DF-019 | Deferred | **Wall-clock watchdog** for scripts and `PackQuarantined` as a recorded input | Deterministic fuel already bounds scripts; the watchdog guards hangs outside fuel | `crates/pg-script` | **2.6** (new script surface: actions; D-085) | A hang outside fuel is caught, the pack is quarantined, and the quarantine is replayable | D-031 |
| DF-020 | Deferred | **Renderer extras**: building roles drawn differently, day and night tint, animated water | Procedural art first | `crates/pg-render` | `unscheduled` (with real art, DF-005) | Each is drawn | D-052 |
| DF-021 | Deferred | **UI polish**: motion (hover and press easing, windows fading), a custom pixel-friendly UI font, a second look at AI options' Provider box (title and label repeat) | 1.8b was about structure and the base look | `crates/pg-app/src/theme.rs`, `ui.rs`, `docs/UI-DESIGN.md` | Stage 11, or earlier if you ask | Motion and font in the theme; the repeated label fixed | D-070, UI-DESIGN |
| DF-022 | Deferred | **Customisable map sizes**; New world offers three sizes | Stage 3 generator controls | New world, `MapSize` | Stage 3 | The player can set the size | D-038 |
| DF-023 | Deferred | **Player mode** (a world-creation toggle that locks the player to one pawn, blocking observation and possession) | Observation mode, which includes possession, is all the product does today | `crates/pg-ui-model` | Stage 4 | The toggle exists and the modes behave as D-048 describes | D-048 |
| DF-025 | Deferred | **Gossip and second-hand memories** | Needs the memory and relationship layers to settle | `crates/pg-core/src/memory.rs` | Stage 3 | A memory can be passed on with its source and a trust effect | S-053 |
| DF-026 | Deferred | **Template variants** (`S-016`): evaluated against 15 templates, not needed yet | Not enough templates | `crates/pg-content` | Stage 6, once there are 50 or more templates | Variants exist or the decision to drop them is recorded | D-062, S-016 |
| DF-027 | Deferred | **Organism system** (vitals, parts, status effects, pain, consciousness, conditions) | Accepted as Stage 2B | `docs/proposals/organism-system.md` | Stage 2B | The Stage 2B "Done when" | D-039, D-040, S-040 to S-047 |
| DF-028 | Deferred | **Luau type check in `pg pack lint`** | Needs the analyzer built or bundled | `tools/pg-cli` | Stage 11 | `--!strict` errors show before load | S-035 |
| DF-029 | Deferred | **Player2 extras**: server-side NPC memory, game-data store, speech | Not needed for text generation | `crates/pg-ai` | Stage 4 and 11 (evaluate) | Each evaluated and built or dropped | S-033 |
| DF-030 | Deferred | **Editor, mutation API, worldgen API** | Stage 3 | roadmap | Stage 3 | The Stage 3 "Done when" | roadmap |
| DF-031 | Deferred | **Dormancy and acute stepping, declarative effect programs, and the other organism suggestions** | Part of Stage 2B | `docs/SUGGESTIONS.md` | Stage 2B | Their own checks | S-041 to S-047 |

## C. Known limits and tuning debts

| ID | Kind | What | Where | Target | Done when | Source |
| --- | --- | --- | --- | --- | --- | --- |
| DF-040 | Limit | Personality changes how often residents talk by a modest amount; line sets by topic and tone are small | conversation data | Tune with the social report when content grows | A clear, measured difference | D-055 |
| DF-041 | Limit | The map view draws zoomed-out maps at a fractional zoom (tile edges are whole pixels, widths 15 or 16) | `pg-render` | Accepted unless it looks wrong | A decision to keep or snap | D-070 |
| DF-042 | Limit | A* keeps its scores in a `BTreeMap` and there are no region labels for instant reachability. Measured 2026-10-29 (1.9 step 2, D-081): a 50-resident town (the most a town makes) costs 0.057 ms per tick, 0.06 percent of the tick budget; movement is 82 percent of that and the worst single tick is 47 ms (residents leaving for work together) | `pg-core` pathfinding | **Trigger, not a date:** build S-017 and S-021 when any single tick passes 50 ms at 1x or the simulation passes 5 percent of the tick budget at an allowed town size (for example if `MAX_RESIDENTS` is raised, or interiors arrive in Stage 6) | The trigger fires and they are built, or Stage 6 closes without it firing and the entry is dropped | S-017, S-021, D-081 |
| DF-043 | Limit | The Saved worlds pane now warns when a world's packs differ from the ones loaded now; what loading does about it is the existing content compatibility check on load (Blueprint 13). Making differences resolvable, not just reported, needs DF-012 | `pg-runtime`, Saved worlds screen | With DF-012 | A world with a different pack set can be opened with its own set | D-078 |
| DF-044 | Limit | The preview exe shares your normal Playground data folder for saves and settings | `pg preview` | Your choice | A decision | D-071 |
| DF-047 | Stub | **Chores and personal goals (priority 4) have no data source**: the planner places them, nothing supplies them. The Roadmap's Stage 2 list does not name them; they belong with jobs and property. Also not built: leisure weighted by *who will be at the place* (plan time cannot know; 2.3's proposals cover meeting friends) and time-of-day windows for duties | Not in the Stage 2 "Done when" | `crates/pg-core/src/schedule.rs` (`Chore`), `life.rs` | Chores Stage 6; company-aware leisure 2.3 | Occupations or the world can supply chores and the planner places them, with reasons | D-088 |

## D. Process and tooling gaps

| ID | Kind | What | Target | Done when | Source |
| --- | --- | --- | --- | --- | --- |
| DF-051 | Process | Local Linux check (S-060) | Opportunistic; only when easy | A one-command run on Linux | S-060 |
| DF-052 | Process | Cross-OS determinism relies on CI; there is no local macOS or Linux run | Standing | CI stays green | D-048 |
| DF-053 | Process | Real folder and file pickers are tested through a scripted double; the Windows and Linux dialogs have been confirmed by you only for export and import | You confirm Install a pack on a real folder | Confirmed | D-059, D-060 |
| DF-054 | Process | `cargo-fuzz` targets (parser, manifest, archive reader, import, save loader); proptest stands in | Stage 11 | Fuzz targets run in CI | D-011 |
| DF-055 | Process | Edition 2024 migration (optional) | `unscheduled` | Decision | D-003 |
| DF-056 | Process | Branch ruleset requires no status checks; `LICENSE` copyright line to confirm | You | Decision | TODO |
| DF-057 | Process | Required before the Stage 1 gate: S-061 conversation and path scale check, S-064 save-size watch, S-070 story digests with their golden in `pg check --full` | **1.9** | Each done and recorded in `docs/gates/stage-1.md` | S-061, S-064, S-070 |

## E. Parked on purpose

| ID | What | Why | Source |
| --- | --- | --- | --- |
| DF-070 | Weather, seasons and fire | Roadmap item parked | S-048, D-040 |
| DF-071 | Animals and non-human humanoids | The physiology profile leaves room | D-040 |
| DF-072 | Anatomy extensions after Stage 2B's base (eyes and ears, neck, spine segments) | After the base | D-040 |

## Closed

| ID | What | Closed | Commit |
| --- | --- | --- | --- |
| DF-045 | Social conflict in the base content: the 30-day, 20-resident digest now shows 5 rifts against 98 warmings and a mixed mood spread (`pg residents digest`, `report`); `pg check` keeps it so (a month with no rift, or more rifts than warmings, fails) | 2026-10-30 | 2.5 |
| DF-024 | Friends seek each other out: residents with a friend and free time ask them to meet (S-055). In the 30-day, 20-resident digest 73 plans are asked, 55 kept, 17 broken, and 7 of 24 friendships of 550 and up cross households (`pg residents report`) | 2026-10-29 | 2.3 |
| DF-046 | Kept and broken plans seen in the base town, the pair cooldown (tested), and bounds on live plans and on the commitments table (tested; finished plans pruned after `keep_days`) | 2026-10-29 | 2.3 |
| DF-050 | `cargo deny` was not part of the local gate; it only failed in CI. `pg milestone` now runs it (skipped with a note when cargo-deny is not installed) | 2026-10-29 | 1.9 step 8 |
| DF-001 | Dev scaffolding in the game pipeline: game worlds ran the scaffolding day counter (changed saved state, emitted a developer event). Taken out of every world with game data; the rest is kept as developer fixtures (DF-007) | 2026-10-29 | 1.9 step 7 (D-084) |
