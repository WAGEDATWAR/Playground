# Decisions log

Short records of decisions made during the build, especially anything that differs from or fills a gap in the reference specification. Newest first. Format: **D-nnn — title** · date · status · context · decision · consequences.

- **D-003 — Edition and toolchain** · 2026-10-06 · accepted · Toolchain pinned to Rust 1.99.0 in `rust-toolchain.toml`; edition stays 2021 for now (2024 migration is optional and separate).
- **D-004 — cargo-deny deferred to 0.1** · 2026-10-06 · accepted · Plan listed it under 0.0, but with no external dependencies yet it cannot be meaningfully tested; it is added with the first dependencies in 0.1.
- **D-002 — License placeholder** · 2026-10-06 · open · Workspace license is `UNLICENSED`, `publish = false`. A license (or staying proprietary) must be chosen before any public release or before accepting outside mod/API contributions.
- **D-001 — Reference documents live in the repo** · 2026-10-06 · accepted · The three specification documents are copied to `docs/reference/` under versioned names and are the binding spec. Changes to the spec are made by editing the copies, bumping the version in the title, and adding an entry here.
