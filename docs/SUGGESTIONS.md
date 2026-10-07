# Suggestions

Ideas that go beyond the reference specification: system additions and expansions, content, feature variations, tooling and process insights. Nothing here is committed to the plan until you accept it; accepted items move into `PLAN.md` (and `DECISIONS.md` if they change the spec).

Format: **ID — title** · area · status (`proposed` / `accepted` / `rejected` / `done`) · why · cost.

## Process and tooling

- **S-001 — Record/replay bug reports** · tooling · proposed · Every save already stores a seed and content refs, and every run produces a `SimInput` log. A "Save bug bundle" dev action that exports the snapshot + input log + pack hashes makes any odd behavior reproducible by me headlessly. · Low, mostly falls out of 0.2/0.6.
- **S-002 — Hash-diff tool** · tooling · proposed · When golden hashes diverge across OSes, a per-table hash diff (`pg replay --diff a b`) points at the first differing table and tick instead of just "mismatch". · Low.
- **S-003 — Deterministic scenario files** · testing · proposed · Small TOML/JSON "scenario" files (seed, packs, scripted inputs, expected assertions) so acceptance tests for each gate read like the Roadmap's "Done when" lines. · Low–medium.

## Systems

- **S-010 — Reason-code explorer in the dev overlay** · explainability · proposed · Since every decision carries a reason and an origin, a filterable timeline ("why did pawn_1a skip lunch?") would be the main debugging tool for designers and modders. · Medium, builds on 0.5/0.10.

## Content

*(none yet)*

## Modding

- **S-020 — "Mod cookbook" sample packs as CI conformance tests** · modding · proposed · Already implied by Blueprint §23.12; suggest writing the first three samples (new need, new action, new hook) during the 0.9 spike so API ergonomics get tested early. · Low.

## Determinism and testing (added in 0.1)

- **S-004 — Cross-check `Canon` against an independent JSON canonicalizer** · testing · proposed · Property-test that `Canon::to_canonical_string` equals RFC 8785 (JCS) output for integer-only documents using a third-party implementation as a dev-dependency, so a bug in our escaping/ordering cannot hide behind our own vectors. · Low.
- **S-005 — Compile-time registry of RNG stream names** · determinism · proposed · Stream names are strings today; a small registry (const list + test that every `rand`/`Rng::new` call site uses a registered name, mirrored in `pg-api` for pack streams) would catch typos that silently create a "new" stream. · Low–medium.
- **S-006 — Typed draw helpers** · determinism · proposed · Add `weighted_pick`, `normal-ish` (sum of draws, integer) and `dice(n, sides)` helpers once the schedule and conversation systems show which shapes they need; avoids each system hand-rolling its own sampling. · Low, do on demand.
- **S-007 — Make `StateHash` human-diffable in logs** · tooling · proposed · Show per-table short hashes (`pawns:3fa9b2c1 objects:…`) in the 0.2 per-day hash log so a divergence is localized at a glance (pairs with S-002). · Low.
- **S-008 — Replay bisect tool** · tooling · proposed · `pg replay --bisect <a.json> <b.json>` runs two logs in lockstep with a per-tick hash and reports the first tick (and table) where they diverge, turning "the cross-OS golden hash differs" into a one-line answer. Builds on the per-table hashes added in 0.2. · Low.
- **S-009 — Event log filter/search in `pg sim --events`** · tooling · proposed · `--events <kind-prefix>` and `--since/--until` so long runs stay readable once real systems emit hundreds of events per day; the same filter becomes the dev-overlay event viewer. · Low.
- **S-011 — Soak scenario file for `pg sim`** · testing · proposed · A scenario file (seed, packs, scripted inputs, day count, expected invariants such as memory/event bounds) so the Roadmap's "30+ day soak" becomes a one-line CI job. Related to S-003. · Low–medium.

## Authoring and modding experience (added in 0.3)

- **S-012 — "Did you mean…?" hints in validation** · modding DX · proposed · For `unknown_field`, `unknown_component`, `bad_enum` and `missing_parent`, suggest the closest known name (edit distance) in the message. Typos are the most common pack bug and today the error only lists all known names. · Low.
- **S-013 — JSON Schema export for editors** · modding DX · proposed · Generate JSON Schema files from the component registry and template/manifest formats (`pg content schema`), so VS Code gives pack authors inline validation and autocomplete while editing `data/templates/*.json` with zero tooling. Pairs with the generated `pg.d.luau` for scripts (Blueprint §23.12). · Low–medium.
- **S-014 — `pg content diff <old> <new>`** · tooling · proposed · Compare two versions of a pack and report removed/renamed templates, changed component schemas and changed defaults, flagged as save-breaking or safe. Gives the "compatibility report" promised in Blueprint §4.2 a concrete form. · Medium.
- **S-015 — Inheritance tree view** · tooling · proposed · `pg content tree` printing the whole `extends` forest (or Graphviz output) with pack colors, to see at a glance what extends what and spot accidental deep chains. · Low.
- **S-016 — Template "variants" for content breadth** · content · proposed · Beyond single inheritance, allow a data-only `variants` list (e.g. `chair` in wood/metal/plastic with different `value` and `appearance.sprite`) that expands to several templates at load time. Cheap way to grow object variety without hand-writing near-duplicates, and still pure data. · Medium; evaluate once there are 50+ templates.
