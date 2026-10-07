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
git checkout m0.4
```

Go back to the latest work with `git checkout main`.

## 3. Run the tests

```bash
cargo test --workspace
```

The first run compiles everything (a few minutes); later runs are fast. You should see `test result: ok.` lines and no `FAILED`. At milestone 0.4 that is about 270 tests. To run only one crate or a few tests:

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
| `docs/reference/` | The binding specification (Blueprint, Design Document, Roadmap) |
| `docs/PLAN.md`, `docs/TODO.md` | The build plan and the rolling working notes |

## 6. Troubleshooting

- **`link.exe not found`** → install the Visual Studio Build Tools C++ workload (step 1.3).
- **`cargo` is not recognized** → open a new terminal, or sign out and in, so `PATH` refreshes.
- **A test fails only on your machine** → that is valuable information, especially anything about *hashes* or *replays* (they must be identical everywhere). Run `cargo test -p pg-core 2>&1` and send me the failing test names and output.
- **Slow first build** → normal; dependencies compile once and are cached in `target/`.
