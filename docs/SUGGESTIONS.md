# Suggestions

Ideas that go beyond the reference specification: system additions and expansions, content, feature variations, tooling and process insights. Accepted items are scheduled in `PLAN.md` and folded into the spec documents (Blueprint v2.2, Roadmap v4.1, Design Document v3.1) at the place they belong; anything that changes the spec is also recorded in `DECISIONS.md`.

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

## Process and tooling

- **S-001 — Record/replay bug reports** · tooling · accepted · Every save already stores a seed and content refs, and every run produces a `SimInput` log. A "Save bug bundle" dev action (`pg bugbundle`, and a dev-overlay button at 0.10) exports the snapshot + input log + pack hashes + tick-hash trail so any odd behavior is reproducible headlessly. · Low, mostly falls out of 0.2/0.6. · **Target: 0.6.**
- **S-002 — Hash-diff tool** · tooling · accepted · When golden hashes diverge across OSes, a per-table hash diff (`pg replay --diff a b`) points at the first differing day and table instead of just "mismatch". · Low. · **Target: 0.4.**
- **S-003 — Deterministic scenario files** · testing · accepted · Small JSON "scenario" files (seed, packs, scripted inputs, steps such as save/reload, expected assertions) so acceptance tests for each gate read like the Roadmap's "Done when" lines. · Low–medium. · **Target: format at 0.6; used by 0.11 and every later gate.**

## Systems

- **S-010 — Reason-code explorer** · explainability · accepted · Since every decision carries a reason and an origin, a filterable timeline ("why did pawn_1a skip lunch?") is the main debugging tool for designers and modders. · Medium. · **Target: `pg schedule explain` at 0.5; dev-overlay panel at 0.10.**

## Content

- **S-015 — Inheritance tree view** · tooling · accepted · `pg content tree` printing the whole `extends` forest (or Graphviz output) with pack colors, to see what extends what and spot accidental deep chains. · Low. · **Target: Stage 1.**
- **S-016 — Template "variants" for content breadth** · content · accepted (evaluate first) · Beyond single inheritance, allow a data-only `variants` list (e.g. `chair` in wood/metal/plastic with different `value` and `appearance.sprite`) that expands to several templates at load time. Cheap way to grow object variety without near-duplicates, still pure data. · Medium. · **Target: evaluate at end of Stage 1; implement when there are ≥ 50 templates (expected Stage 6).**

## Modding

- **S-020 — "Mod cookbook" sample packs as CI conformance tests** · modding · accepted · Already implied by Blueprint §23.12; write the first three samples (new need, new action, new hook) during the 0.9 spike so API ergonomics get tested early; each later stage adds a sample for what it exposes. · Low. · **Target: 0.9, then each stage; gallery at Stage 11.**
- **S-012 — "Did you mean…?" hints in validation** · modding DX · accepted · For `unknown_field`, `unknown_component`, `bad_enum`, `missing_parent` and similar, suggest the closest known name (edit distance) in the message. Typos are the most common pack bug. · Low. · **Target: content validator at 0.4; scripts/API names at 0.9.**
- **S-013 — JSON Schema export for editors** · modding DX · accepted · Generate JSON Schema files from the component registry and template/manifest formats (`pg content schema`), so VS Code gives pack authors inline validation and autocomplete with zero tooling. Pairs with the generated `pg.d.luau` for scripts (Blueprint §23.12). · Low–medium. · **Target: 0.9 (data schemas); polish at Stage 11.**
- **S-014 — `pg content diff <old> <new>` and a load-time compatibility report** · tooling · accepted · Compare two versions of a pack and report removed/renamed templates, changed component schemas and changed defaults, flagged as save-breaking or safe. Gives the "compatibility report" in Blueprint §4.2 a concrete form, used when a save's `content_refs` no longer match installed packs. · Medium. · **Target: 0.6.**

## Determinism and testing

- **S-004 — Cross-check `Canon` against an independent JSON canonicalizer** · testing · accepted · Property-test that `Canon::to_canonical_string` equals RFC 8785 (JCS) output for integer-only documents using a third-party implementation as a dev-dependency, so a bug in our escaping/ordering cannot hide behind our own vectors. · Low. · **Target: 0.6 (when canonical JSON becomes the save format).**
- **S-005 — Compile-time registry of RNG stream names** · determinism · accepted · A small registry (const list + test that every `rand`/`Rng::new` call site uses a registered name, mirrored in `pg-api` for pack streams) catches typos that silently create a "new" stream. · Low–medium. · **Target: 0.5.**
- **S-006 — Typed draw helpers** · determinism · accepted (on demand) · `weighted_pick`, integer "normal-ish" (sum of draws) and `dice(n, sides)` helpers once scheduling and conversation show which shapes they need; avoids each system hand-rolling its own sampling. · Low. · **Target: on demand, first use at 0.5 / Stage 1.**
- **S-007 — Make `StateHash` human-diffable in logs** · tooling · accepted · Per-table short hashes (`pawns:3fa9b2c1 objects:…`) in every day-hash line; **already printed by `pg sim` since 0.2**; 0.4 also stores them in replay logs so S-002 and S-008 can use them. · Low. · **Target: done in 0.2; log storage at 0.4.**
- **S-008 — Replay bisect tool** · tooling · accepted · `pg replay --bisect <a.json> <b.json>` runs two logs in lockstep with a per-tick hash and reports the first tick (and table) where they diverge, turning "the cross-OS golden hash differs" into a one-line answer. · Low. · **Target: 0.4.**
- **S-009 — Event log filter/search in `pg sim --events`** · tooling · accepted · `--events <kind-prefix>` and `--since/--until` so long runs stay readable; the same filter becomes the dev-overlay event viewer. · Low. · **Target: 0.4.**
- **S-011 — Soak scenario file** · testing · accepted · A scenario file (seed, packs, scripted inputs, day count, expected invariants such as memory/event bounds) so the Roadmap's "30+ day soak" becomes a one-line CI job. Builds on S-003. · Low–medium. · **Target: 0.11.**
