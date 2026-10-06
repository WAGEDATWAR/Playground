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
