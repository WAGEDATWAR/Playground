# TODO — rolling working notes

Updated every time work starts, stops or changes direction. Newest status at the top.

## Current focus

**Phase 0 / Milestone 0.0 — Bootstrap.** Toolchain verified (Rust 1.99.0 pinned, MSVC Build Tools present). Skeleton builds, passes `clippy -D warnings`, tests, `fmt --check` and `scripts/check_deps.py` locally on Windows. Remaining for 0.0: push to GitHub and get CI green on all 3 OSes, then tag `m0.0`.

## Blocked / waiting on user

- [ ] **GitHub repo:** the GitHub connector failed to connect in this session (HTTP 400, "Authorization header is badly formatted") and `gh` CLI is not installed, so I could not create the repo. User will create an empty **private** repo `Playground` and send the URL; then `git remote add origin <url>`, rename branch to `main`, push, watch CI, tag `m0.0`.

## Next up

1. After push: confirm CI is green on Windows / Linux / macOS; fix any OS-specific issue; tag `m0.0`.
2. `cargo-deny` config (`deny.toml`) is deferred to milestone 0.1, when the first external dependencies (blake3, serde…) arrive; adding it with no deps would be untestable.
3. Start 0.1 determinism primitives (see `PLAN.md`).

## Shell note

Tool shells start without the Rust PATH. Use PowerShell and prefix commands with
`$env:Path = [Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User')`.
Python is available for scripts (`scripts/check_deps.py`).

## Notes (non-obvious things to remember)

- The skeleton now compiles and passes local checks on Windows (verified 2026-10-06 with Rust 1.99.0). CI on Linux/macOS is still unverified.
- `Cargo.toml` license is a placeholder (`UNLICENSED`); needs a decision before any public release (`DECISIONS.md`).
- `rust-toolchain.toml` pins `1.99.0`. CI uses `dtolnay/rust-toolchain@stable`; it should honor the pin via rust-toolchain.toml (verify on first CI run).
- Edition 2021 kept for now; toolchain 1.99.0 supports 2024, so migrate deliberately (`cargo fix --edition`) as a small separate commit if wanted.
- The three reference docs still carry stale file names/versions in Downloads; the copies here are named by version.
- Earlier correction: the old TypeScript prototype is **not** a porting oracle; the Rust core is built fresh from the Blueprint (Roadmap §11).

## Done log

- 2026-10-06: Read the three reference documents; created repo skeleton (14 crates, workspace manifest, lint policy for `pg-core`); saved reference docs under `docs/reference/`; wrote `PLAN.md`, `TODO.md`, `SUGGESTIONS.md`, `DECISIONS.md`, `README.md`, `CLAUDE.md`.

- 2026-10-06: Verified toolchain; pinned Rust 1.99.0; `cargo fmt`, build, clippy, test all clean; added `scripts/check_deps.py` (dependency rule from Blueprint §1, restricts `mlua`/`wgpu`/`winit`/`egui`/`rand` to allowed crates) and wired it into CI.
