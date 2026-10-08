# Building and testing Playground

This is everything you need to build the project, run the full test suite, and try the developer tools on your own machine. Playground is a Rust workspace; nothing is playable yet (we are in Phase 0), so what you can build and run today is the **test suite** and the **`pg` developer tool**.

## 1. One-time setup (Windows)

1. **Git.** You already have it (Git for Windows).
2. **Rust.** Install from <https://rustup.rs> (accept the defaults). The repository pins the exact compiler in `rust-toolchain.toml` (currently 1.99.0); the first `cargo` command inside the repo downloads that version automatically.
3. **Visual Studio Build Tools** with the **"Desktop development with C++"** workload (needed by Rust on Windows for the linker). If `cargo build` later says `link.exe not found`, this is the missing piece.
4. **Python 3** (any recent version, on your `PATH`). Used only by one small check script, `scripts/check_deps.py`.
5. *Optional:* `cargo install cargo-deny --locked` if you want to run the license and advisory check locally (CI always runs it).

Open a **new** terminal after installing so the new `PATH` is picked up. Check it worked:

```bash
cargo --version
python --version
```

## 2. Get the code

```bash
git clone https://github.com/WAGEDATWAR/Playground.git
cd Playground
```

To build an exact milestone, check out its tag (tags are `m0.0`, `m0.1`, …):

```bash
git checkout m0.7
```

Go back to the latest work with `git checkout main`.

## 3. Run the tests

```bash
cargo test --workspace
```

The first run compiles everything (a few minutes); later runs are fast. You should see `test result: ok.` lines and no `FAILED`. At milestone 0.9 that is about 660 tests (the cookbook end-to-end tests take about 40 seconds in a debug build). `cargo run -p pg-cli -- check` runs every developer check in one go (18 of them). To run only one crate or a few tests:

```bash
cargo test -p pg-core                  # just the simulation core
cargo test -p pg-core movement         # only tests with "movement" in their name
cargo test -p pg-content --test base_pack
```

The same checks CI runs:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
python scripts/check_deps.py           # enforces which crate may depend on which
cargo deny check                       # optional (needs cargo-deny)
```

## 4. Build and run the developer tool (`pg`)

```bash
cargo build --release -p pg-cli        # produces target\release\pg.exe
cargo run -p pg-cli -- <command>       # build (if needed) and run in one step
```

`cargo run` builds a slower debug binary; add `--release` for speed (it matters for `pg map bench-paths` and long simulations). Everything after `--` is passed to `pg`. Run `cargo run -q -p pg-cli -- help` for the full command list. A good tour:

```bash
# Determinism self-test: published and pinned test vectors (must say "0 failed")
cargo run -q -p pg-cli -- selftest

# Look at the shipped content and how a template resolves through its inheritance chain
cargo run -q -p pg-cli -- content lint
cargo run -q -p pg-cli -- content resolve furniture.drawer

# Draw a generated town with walking pawns
cargo run -q -p pg-cli -- map show --seed playground --dev-map 56x30 --dev-pawns 6 --ticks 400 --content data/base

# Pathfinding: one request drawn, then serial vs threaded (results must be identical)
cargo run -q --release -p pg-cli -- map path --from 2,2 --to 37,12 --size 40x14
cargo run -q --release -p pg-cli -- map bench-paths --threads 1,2,4,8

# Simulate three days and write a replay log, then verify it (and a mid-run snapshot)
cargo run -q -p pg-cli -- sim --seed demo --dev-map 40x30 --dev-pawns 8 --days 3 --log demo.json
cargo run -q -p pg-cli -- replay demo.json --snapshot-at 12345

# A pawn's planned day, with the reason for every slot (add --propose to arrange a meeting)
cargo run -q -p pg-cli -- schedule explain pawn_1 --seed town --dev-map 48x36 --dev-pawns 4 --propose 400:pawn_1:pawn_2:20:2:10,10 --at 9000
# The closed action registry
cargo run -q -p pg-cli -- actions
# Watch tasks and commitments happen
cargo run -q -p pg-cli -- sim --seed town --dev-map 48x36 --dev-pawns 4 --days 1 --events task

# Saves: create a world slot, inspect and verify it, damage it on purpose, watch it recover
cargo run -q -p pg-cli -- save create saves --world town --seed demo --dev-map 40x30 --dev-pawns 6 --days 2 --content data/base
cargo run -q -p pg-cli -- save create saves --world town --seed demo --dev-map 40x30 --dev-pawns 6 --days 3 --content data/base
cargo run -q -p pg-cli -- save inspect saves town
cargo run -q -p pg-cli -- save verify saves town --content data/base
#   (now flip a byte in saves/worlds/town/state.2.pgsave with any hex editor and run verify and load again)
cargo run -q -p pg-cli -- save load saves town --ticks 1000
# Export a world to a portable file and import it as a new slot
cargo run -q -p pg-cli -- save export saves town --out town.export.json
cargo run -q -p pg-cli -- save import saves town.export.json --world town-copy
# Bug bundles: cut a replay log at a tick; the bundle replays on any machine
cargo run -q -p pg-cli -- bugbundle create golden/dev-town-3days.json --at 20000 --note "demo" --out demo.pgbundle
cargo run -q -p pg-cli -- bugbundle run demo.pgbundle
# Scenarios: executable acceptance checks
cargo run -q -p pg-cli -- scenario run scenarios/persistence.json
# What changed between two content sets
cargo run -q -p pg-cli -- content diff data/base data/base

# One command that runs the developer checks and prints a single report (run from the repository root)
cargo run -q -p pg-cli -- check
# AI client skeleton (no network needed): providers, the exact request that would be sent, the leak check
cargo run -q -p pg-cli -- ai providers
cargo run -q -p pg-cli -- ai test --provider anthropic --dry-run
cargo run -q -p pg-cli -- ai test --provider player2 --dry-run
cargo run -q -p pg-cli -- ai selfcheck
# With your own key (kept in the OS credential store, never in files): set it from an environment variable,
# then run the real connection test
#   PowerShell:  $env:MY_KEY = "sk-..."; cargo run -q -p pg-cli -- ai key set openai --from-env MY_KEY
cargo run -q -p pg-cli -- ai test --provider openai
# Player2 signs in with a code instead of a pasted key (the registered client id is built in):
#   cargo run -q -p pg-cli -- ai login player2
cargo run -q -p pg-cli -- ai test --provider player2

# The golden replay CI checks on every OS
cargo run -q -p pg-cli -- replay golden/dev-town-3days.json --snapshot-at 12345
```

If you change something and a replay no longer matches, `pg replay --diff a.json b.json` and `pg replay --bisect a.json b.json` tell you where (table, then exact tick) the two runs part ways.

## 5. Where things are

| Path | What |
| --- | --- |
| `crates/pg-core` | The deterministic simulation core |
| `crates/pg-content` | Templates, components, content packs |
| `crates/pg-canon` | Canonical values and the strict JSON parser |
| `crates/pg-runtime` | Threaded path solving (more in 0.8) |
| `tools/pg-cli` | The `pg` developer tool |
| `data/base` | The base game's own content pack |
| `golden/` | Golden replay logs checked in CI |
| `fixtures/saves/` | Pinned save files that every build must keep loading |
| `scenarios/` | Scenario files run by `pg scenario run` (and CI) |
| `docs/reference/` | The binding specification (Blueprint, Design Document, Roadmap) |
| `docs/PLAN.md`, `docs/TODO.md` | The build plan and the rolling working notes |

## 6. Troubleshooting

- **`link.exe not found`** → install the Visual Studio Build Tools C++ workload (step 1.3).
- **`cargo` is not recognized** → open a new terminal, or sign out and in, so `PATH` refreshes.
- **A test fails only on your machine** → that is valuable information, especially anything about *hashes* or *replays* (they must be identical everywhere). Run `cargo test -p pg-core 2>&1` and send me the failing test names and output.
- **Slow first build** → normal; dependencies compile once and are cached in `target/`.

## Packs with scripts (0.9)

```bash
# Check a pack: manifest, data, script lint (API names, capabilities, order-sensitive loops), load in the host.
cargo run -q -p pg-cli -- pack lint packs/cookbook/caffeine

# Run it headless for three days under golden hashes, then again rebuilding every VM at each day boundary.
cargo run -q -p pg-cli -- pack test packs/cookbook/caffeine
# After an intended change: record new golden hashes (packs/golden/<id>.json).
cargo run -q -p pg-cli -- pack test packs/cookbook/caffeine --update

# What scripts cost (use --release for real numbers).
cargo run -q --release -p pg-cli -- pack bench --pawns 200

# The generated API reference and editor type definitions; JSON Schema for pack files.
cargo run -q -p pg-cli -- pack docs
cargo run -q -p pg-cli -- pack docs --luau > pg.d.luau
cargo run -q -p pg-cli -- content schema templates

# Start a new pack.
cargo run -q -p pg-cli -- pack new my_pack packs/my_pack
```

The ScriptVm spike report with every measurement is `docs/spikes/scriptvm.md`.

## Playing the game (0.10)

```bash
# Open the game (needs a desktop; the first build takes a few minutes).
cargo run -p pg-app

# Developer aids.
cargo run -p pg-app -- --demo game      # new, options, ai, saved, game, drawer, focus, pause, overlay: open straight on a screen
cargo run -p pg-app -- --shadow 2       # re-run every keyframe span on 2 threads and compare (overlay, Time tab)
cargo run -p pg-app -- --pack packs/cookbook/caffeine   # also load a pack with scripts (developer use)
cargo run -p pg-app -- --smoke          # scripted session without a window; what CI runs
```

Saves, settings and bug bundles live in the per-user data folder (`%APPDATA%\Playground` on Windows,
`~/Library/Application Support/Playground` on macOS, `~/.local/share/Playground` on Linux); `--data-dir <folder>`
uses another one. Exports are written to `exports/` there and imports are read from `imports/`.

**Keys:** Tab / Shift+Tab and Up / Down move focus, Left / Right change choices and sliders, Enter or Space
press, Escape goes back (in the world it opens the pause menu), Space pauses and resumes, F3 shows the
developer overlay (only after turning on Options, Developer mode). In the world: the controls are in the
bottom-right corner (the speed button opens a drawer); saving is in the pause menu (Escape). Drag to pan the map,
mouse wheel to zoom.

**What to try for the 0.10 checkpoint:** launch; New world (change the name, map size, residents); watch the
town run at 1x, 9x and 27x; pause; Escape and Save and go to main menu; Saved worlds (the picture, Load,
Export, Delete); Continue; Options (scale, window mode, autosave, language `pseudo` to see every string
transformed); Options, AI provider, Player2 (sign-in shows a code and link); close the window while a world
runs and relaunch to see it saved; Options, Developer mode, then F3 for the overlay; switch to another window
and back to see the centred "Welcome back" notice (Options can turn pause-on-focus-loss off).

## Gate scenarios and the soak (0.11)

```bash
# The three Stage 0 gate scenarios (about a minute in a debug build).
cargo run -p pg-cli -- scenario run scenarios/gate-stage0-town.json scenarios/gate-stage0-migrate.json scenarios/gate-stage0-scripts.json

# The 30-day soak with script packs and shadow verification (about 90 seconds in release mode, ten minutes in debug).
cargo run --release -p pg-cli -- scenario run scenarios/soak-30-days.json
```

The gate report with every clause, its evidence and its command is `docs/gates/stage-0.md`.

## Needs, mood and residents (Stage 1)

```bash
# What a seed produces: households, occupations, starting relationships and shared memories.
cargo run -q -p pg-cli -- residents generate --seed demo --count 14

# Run a few residents for days on open ground and show needs, mood and capacities at the end.
cargo run -q -p pg-cli -- sim --seed demo --dev-map 40x30:0 --dev-pawns 4 --days 3 --content data/base --needs

# Watch the needs events (urgent, critical, collapsed, recovered, mood changes).
cargo run -q -p pg-cli -- sim --seed demo --dev-map 40x30:0 --dev-pawns 4 --days 3 --content data/base --events need.
cargo run -q -p pg-cli -- sim --seed demo --dev-map 40x30:0 --dev-pawns 4 --days 3 --content data/base --events mood.

# The inheritance forest of the content, and the actions (now with what each restores).
cargo run -q -p pg-cli -- content tree
cargo run -q -p pg-cli -- actions
```

The numbers (decay, thresholds, mood rules, meal times) are data in `data/base/data/game/`; edit them and re-run.

