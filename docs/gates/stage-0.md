# Stage 0 gate report

Roadmap v4.7, Stage 0 "Done when", checked clause by clause. Each clause names the evidence that proves it and the
command that reruns it. "Local" is a run on the Windows development machine on 2026-10-08; "CI" is the same
command on Windows, Linux and macOS (`.github/workflows/ci.yml`). The CI column is filled in when CI is green.

**Status: accepted (`m0.11`), CI green on Windows, Linux and macOS.** The first review's changes (in-game layout, drawers, developer mode; D-037) are in. Everything below passes locally; the two things only you can judge are
the feel of the app (clause 1) and whether the evidence is enough to call Stage 0 done.

| # | Clause | Evidence | Rerun | Local | CI |
| --- | --- | --- | --- | --- | --- |
| 1 | The app launches to a graphical main menu from which a world can be created, saved, quit and continued | `pg-app --smoke` runs the whole flow on real files and a real clock (launch, options, new world, play at 27x, overlay, pause menu, save, main menu, continue, close, **relaunch**, continue); `pg-runtime/tests/app_flows.rs` repeats it over in-memory services with failure cases; you click through it by hand | `cargo run -p pg-app -- --smoke`; `cargo run -p pg-app` | pass | |
| 2 | A deterministic sample town can run, pause, save, reload, migrate and recover without losing another save | Scenario `gate-stage0-town` (run, reload identical, snapshot resume identical, export and import, damaged newest save falls back, lost index recovers, **a second world in another slot stays clean**); scenario `gate-stage0-migrate` (the pinned schema 3 save migrates to its recorded hash and then lives on); pause: the run-state machine tests, the pause menu and focus-loss flows in `app_flows.rs`; crash safety: saves failed at every storage operation | `pg scenario run scenarios/gate-stage0-town.json scenarios/gate-stage0-migrate.json` | pass | |
| 3 | Provider failure is safe | AI contract tests (timeouts, offline, auth, rate limit, malformed and hostile replies), circuit breaker, no-key and offline paths in the app flows (sign-in failure is explained and retryable; the game plays without AI) | `cargo test -p pg-ai`; `cargo test -p pg-runtime --test app_flows` | pass | |
| 4 | No key appears in logs, exports or crash reports | Sentinel-key leak test over logs, saves, exports and crash reports (`pg ai selfcheck`, `tools/pg-cli/tests/key_leak.rs`); the controller flow test scans storage, the log, UI snapshots and Debug output for a typed key; keys are a `Secret` type that cannot be printed | `pg ai selfcheck`; `cargo test --workspace` | pass | |
| 5 | The core runs headless in tests with no platform code | `pg-core` depends only on `pg-canon`, `pg-content`, `pg-host` (traits) and `pg-api`, forbids `unsafe`, floats, clocks and `HashMap`; the dependency rule is checked from `cargo metadata`; every core test and every `pg` command runs with no window, clock or network | `python scripts/check_deps.py`; `cargo test -p pg-core` | pass | |
| 6 | Replay hashes match on Windows, Linux and macOS | The golden replay (`golden/dev-town-3days.json`, with snapshot resume) and the pinned final hashes of the three gate scenarios and of the 30-day soak are checked on all three systems in CI; the determinism vectors and the floating-point contraction probe run there too | `pg replay golden/dev-town-3days.json --snapshot-at 12345`; the scenario commands | pass | |
| 7 | A test script pack runs deterministically | Three cookbook packs under golden hashes with the VM-reload variant (`pg pack test`), the end-to-end tests with snapshot, rewind, shadow verification and safe mode, and scenario `gate-stage0-scripts` (packs loaded: reload, fork, 4-day shadow-verified soak, damage and recovery) | `pg pack test packs/cookbook/caffeine` (and the other two); `pg scenario run scenarios/gate-stage0-scripts.json` | pass | |
| 8 | A hostile pack is contained without affecting the world | The 40-test hostile-pack corpus (escapes, runaway loops, memory bombs, hostile values, tampering, bytecode, capability gating) with the VM still usable afterwards; quarantine tests where the rest of the world and other packs carry on | `cargo test -p pg-script` | pass | |
| 9 | The 30+ day soak with shadow verification (gate item, S-011) | `scenarios/soak-30-days.json`: 20 residents, three script packs, 30 game days; every day re-simulated from its snapshot on 3 threads and identical; world state bounded (130,159 to 135,740 bytes), daily script fuel flat (1,421,822 to 1,530,980), population and input log unchanged; final hash pinned. A separate CI job runs it in release mode on all three systems | `cargo run --release -p pg-cli -- scenario run scenarios/soak-30-days.json` (about 90 seconds) | pass | |

## What was run locally

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `python scripts/check_deps.py`,
  `cargo deny check`: clean.
- `cargo test --workspace`: 732 tests pass.
- `pg check`: 29 of 29 checks pass (determinism vectors, content and string lints, golden replay, event catalog,
  settings registry, profile and session runs, pack lint and golden tests, the three gate scenarios, persistence
  scenario, saves, bundles, redaction self-check).
- `pg-app --smoke`: passes, including the relaunch.
- The real window opened and was captured on the main menu, options, AI provider, new world, in-game, pause menu,
  overlay and saved-worlds screens.

## What this gate does not claim

- The window was exercised by scripted sessions and by looking at it; no one has played it for an hour. Your
  click-through (the list in `docs/BUILDING.md`, "Playing the game") is the part only you can do.
- Accessibility output (AccessKit), native file dialogs and a living menu backdrop are Stage 1 items (S-037 to S-039).
- The scripting API is `0.1` and unstable by design; capability approval prompts arrive in Stage 1.
- Linux and macOS have never run the window, only the headless suites and the smoke test.
