# Decisions log

Short records of decisions made during the build, especially anything that differs from or fills a gap in the reference specification. Newest first. Format: **D-nnn — title** · date · status · context · decision · consequences.

- **D-003 — Edition and toolchain** · 2026-10-06 · provisional · Skeleton uses Rust edition 2021 and `channel = "stable"` because no toolchain is installed to check against. Pin an exact version and reconsider edition 2024 at milestone 0.0.
- **D-002 — License placeholder** · 2026-10-06 · open · Workspace license is `UNLICENSED`, `publish = false`. A license (or staying proprietary) must be chosen before any public release or before accepting outside mod/API contributions.
- **D-001 — Reference documents live in the repo** · 2026-10-06 · accepted · The three specification documents are copied to `docs/reference/` under versioned names and are the binding spec. Changes to the spec are made by editing the copies, bumping the version in the title, and adding an entry here.
