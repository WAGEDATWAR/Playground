# TODO — rolling working notes

Updated every time work starts, stops or changes direction. Newest status at the top.

## Current focus

**Phase 0 / Milestone 0.0 — Bootstrap.** Planning and skeleton done; waiting on two answers from the user (see Blocked).

## Blocked / waiting on user

- [ ] **Rust toolchain not installed** on this machine (no `cargo`, `rustc`, `rustup`; MSVC Build Tools not detected). Need permission to install, or the user installs it. Nothing can be compiled or tested until then.
- [ ] **GitHub:** `gh` CLI is not installed and not authenticated; git has no remote. Need the repository name and visibility (default proposal: `Playground`, private), and either the user installs/authenticates `gh` or creates an empty repo and gives me its URL.

## Next up

1. Commit the skeleton locally (done once docs are written).
2. After toolchain install: pin the exact toolchain in `rust-toolchain.toml`, run `cargo build` / `cargo clippy` / `cargo fmt` on the skeleton, fix anything the unverified skeleton got wrong.
3. Add CI (`.github/workflows/ci.yml`), `cargo-deny` config, `scripts/check-deps`.
4. Push to GitHub, protect nothing yet, tag `m0.0` when CI is green on all three OSes.
5. Start 0.1 determinism primitives (see `PLAN.md`).

## Notes (non-obvious things to remember)

- The skeleton was written **without compiling** (no toolchain). Treat it as unverified until the first build.
- `Cargo.toml` license is a placeholder (`UNLICENSED`); needs a decision before any public release (`DECISIONS.md`).
- `rust-toolchain.toml` uses `stable` for now; pin to an exact version at 0.0 for reproducible builds (Blueprint §24).
- Edition 2021 chosen for safety; revisit 2024 once the toolchain version is known.
- The three reference docs still carry stale file names/versions in Downloads; the copies here are named by version.
- Earlier correction: the old TypeScript prototype is **not** a porting oracle; the Rust core is built fresh from the Blueprint (Roadmap §11).

## Done log

- 2026-10-06: Read the three reference documents; created repo skeleton (14 crates, workspace manifest, lint policy for `pg-core`); saved reference docs under `docs/reference/`; wrote `PLAN.md`, `TODO.md`, `SUGGESTIONS.md`, `DECISIONS.md`, `README.md`, `CLAUDE.md`.
