# Stage 1 gate report

Roadmap v4.9, Stage 1 "Done when", checked clause by clause, then the gate items from `docs/PLAN.md` (1.9). Each
clause names the evidence that proves it and the command that reruns it. "Local" is a run on the Windows development
machine; "CI" is the same commands on Windows, Linux and macOS (`.github/workflows/ci.yml`). The CI column is
filled in when CI is green on the commit that carries this report.

**Status: awaiting your review.** Everything below passes locally. What only you can judge is whether the town is
worth watching (clause 1 and 2 in the window), and whether the evidence is enough to call Stage 1 done.

## The Stage 1 "Done when", clause by clause

| # | Clause | Evidence | Rerun | Local | CI |
| --- | --- | --- | --- | --- | --- |
| 1 | A seeded 10 to 20 resident town **runs autonomously in the native app** | Scenario `gate-stage1-town` (14 residents, 64x48, 14 game days, base game only; every day re-simulated on three thread counts and identical); `pg-app --smoke` (the whole window flow on real files and a real clock, including a world run in real time); the app-flow tests in `crates/pg-runtime/tests/app_flows.rs` (creation, play, pause, save, quit, relaunch, Continue) | `pg scenario run scenarios/gate-stage1-town.json`; `cargo run -p pg-app -- --smoke` | pass | |
| 2 | The town **can be observed and inspected** | The sprite renderer (procedural art, interpolation, click to focus); the resident inspector (state, memories with reasons, relationships, conversations, and in developer mode what packs are doing); the town journal (observation and possession modes); the developer overlay and console; frame-buffer captures of every screen (`pg-app --demo <screen> --capture`, `docs/BUILDING.md`); `pg residents inspect` and `pg residents report` | `cargo run -p pg-app -- --demo inspector`; `pg residents inspect <pawn> --seed S --town 48x36` | pass | |
| 3 | The town **produces overheard conversation** | 501 conversations in a 14-resident, 10-day town and 1,552 in a 20-resident, 30-day town (the story digests); lines are chosen by topic, tone, mood and relationship, with per-turn tone and memory callbacks; a conversation within hearing of the watched resident gets its lines prepared, and with an AI provider on, generated (cooldowns, in-flight cap, late replies, offline and hostile replies handled); lines are recorded into the history and recalled from memories | `pg residents digest ...`; the dialogue tests in `crates/pg-runtime/src/dialogue.rs`; `pg ai dialogue-test` | pass | |
| 4 | The town **keeps state and social history after restart** | `gate-stage1-town` requires relationships, memories, remembered conversations and journal entries to exist, then a save and reload that gives the identical state hash (so the whole social history is intact), a snapshot resume that agrees, and a fall-back to the older save after damage with the history still there; the journal is part of the saved and hashed world (schema 4); the smoke test relaunches and continues | `pg scenario run scenarios/gate-stage1-town.json`; the smoke test | pass | |
| 5 | The **base content loads through the pack loader** | The base game is a pack (`data/base`) loaded by the same loader and format as player packs; `base_pack` tests (loads with no warnings, stable content hash, inheritance as designed, dialogue lint); every scenario above lists `data/base` as its only content | `pg content lint`; `cargo test -p pg-content` | pass | |
| 6 | **A sample pack adds a component and a hook that visibly change behaviour and are attributed in the inspector** | The hardy cookbook pack (a component and three hooks: needs decay, mood comfort, conversation chance) under a golden hash with the VM-reload variant; `the_hardy_pack_makes_gritty_residents_need_less_often_and_the_inspector_can_say_why` and `asking_the_hooks_to_explain_changes_nothing` (`crates/pg-script/tests/cookbook.rs`); the inspector's "What content packs are doing" section in developer mode; scenario `gate-stage1-packs` (base plus hardy plus caffeine, a week, shadow-verified, reload and resume identical) | `pg pack test packs/cookbook/hardy`; `pg scenario run scenarios/gate-stage1-packs.json` | pass | |

## The other 1.9 gate items

| Item | Evidence | Local | CI |
| --- | --- | --- | --- |
| **Longer soak with bounds** (30 days, 20 residents, three script packs, every day shadow-verified on three thread counts) | `scenarios/soak-30-days.json`: state size bounded (plateau check on the last ten days), daily script fuel bounded, **at most 169 memories in any one head (cap 200)**, **journal at its cap of 120 entries**, population and input log unchanged, **save 139 KB against a 250 KB ceiling**. Events are not saved state; the path cache is capacity-bound by construction | pass | |
| **Cross-OS pinned hashes** | The golden replay, four pack goldens, seven scenarios, the town generator's starting hash, the world-v3 and world-v4 fixtures, the saved-world shape and two story digests are all in `pg pins`; CI runs them on Windows, Linux and macOS | pass | |
| **Menu-flow smoke on the new renderer** | `pg-app --smoke` drives the real window code (a headless frame renders every screen: 389 shapes in the last run) and relaunches | pass | |
| **S-061 conversation scale** (D-081) | `pg profile --town` and the `scale` bench target. A 50-resident town (the most the generator makes) costs 0.057 ms a tick, 0.06 percent of the tick; conversations 1.55 microseconds a tick (2.8 percent); scaled quadratically to 200 residents about 25 microseconds. No spatial hash needed | pass | |
| **S-064 save-size watch** (D-082) | Compressed save, 96x72: 14 residents 31 KB (day 1), 107 KB (day 30), 123 KB (day 200); 50 residents 76 KB, 536 KB, 608 KB (day 100). Growth levels off after about day 30 | pass | |
| **S-070 story digests** (D-083) | `golden/story-digest-small.txt` and `-month.txt` in `pg pins` and `pg check`; a change to behaviour shows as the lines that differ | pass | |
| **Deferral review** (`docs/DEFERRALS.md`, D-079, D-080, D-084) | The register was created at the start of 1.9 with 44 entries; DF-004 and DF-006 verified and rescheduled, DF-001 closed (game worlds run no scaffolding; the rest kept on purpose as DF-007), DF-042 measured and given a trigger, DF-050 closed; every open stub has a marker in the code and the register is checked by `pg milestone` and CI | pass | |

## What was run locally

- `pg milestone` (8 steps, all pass): formatting, the dependency rule, the deferral register, `cargo deny`, clippy
  (warnings are errors), the whole test suite, `pg check --full` (every check, every pinned hash, the 30-day soak,
  final hash 81861cfa) and the app smoke test.
- The real window opened and was captured on every screen (main menu, New world with its sample town, Saved worlds,
  Mods, Options, AI options, pause, the game with the HUD, inspector, journal, console, overlay).

## Findings this gate produced

- **Game worlds ran a piece of developer scaffolding** (a day counter that changed saved state and emitted a developer
  event). Taken out; the only table that changed was the scaffolding's own (D-084). This was only free to do because
  the saved-state format has not shipped.
- **The base content has little social conflict**: no relationship soured in 30 days; fifteen residents collapsed from
  exhaustion (DF-045, Stage 2 tuning).
- **Action preconditions, costs and effects were promised for Stage 1 (D-021) and never built**; rescheduled to Stage 2
  (DF-006, D-080).
- **Pathfinding is the cost that grows with the town** (82 percent of a 50-resident tick; the worst single tick is
  47 ms when residents leave for work together). It has a measurable trigger (DF-042, D-081), not a date.

## What this gate does not claim

- The window was exercised by scripted sessions, captures and looking at it; no one has played it for an hour. Your
  click-through (`docs/BUILDING.md`, "Playing the game", and the preview build) is the part only you can do.
- Linux and macOS have run the headless suites and the smoke test in CI, never the window.
- AccessKit, audio, per-world pack choice and the other items in `docs/DEFERRALS.md` are deferred on purpose and
  tracked there with targets.
- The scripting API is 0.1 and unstable by design.
