# Playground — Architecture Blueprint v3.14

**Purpose:** a granular technical blueprint for building Playground as a **native desktop binary written in Rust, with a sandboxed Luau scripting layer for user-created content packs**, as defined by the Design Document v3.2 and Roadmap v4.9. Roadmap decisions are binding; this document says *how*. **Notation:** interfaces are written in Rust-style pseudocode (structs, enums, traits). Where a part is built, the text describes it **as built** and the code is the authority on names and signatures; where it is not built yet, the sketch is a contract whose invariants may not change. Stage tags like **\[S4\]** show when a part is first built. **Reading order:** §0–6 are the foundation (Stage 0), and §23 (scripting and mods) is also Stage 0 foundation because it shapes the data model and tick pipeline. §7–15 are the simulation systems and the presentation layer. §16–22 cover later modules, quality and the build map. §24 covers the native build and distribution.

**Status (v3.14):** the Mods screen as unrollable tiles in a bounded scrolling list; the `Scroll` widget; decision D-061. **Status (v3.13):** the Mods screen and how packs are chosen at launch (§23.2); decision D-060. **Status (v3.12):** native file dialogs (§3); decision D-059. **Status (v3.11):** the town journal is part of the saved world (§14.9); decision D-058. **Status (v3.10):** the town journal (§14.9); decision D-056. **Status (v3.9):** the social refinements (§9): a personality number, per-turn tone, relationship and mood bands in the fallback lines, talks that look back on shared memories; the content hash no longer includes the string tables (§4); decision D-055. **Status (v3.8):** the sprite renderer as built (§14.4): procedural sprites, atlas, draw list, interpolation, the wgpu layer through an egui paint callback, click to focus; decision D-052. **Status (v3.7):** conversation lines are recorded into the history (§8.3, §9.4): the `RecordDialogue` command, `Talk.lines`, `Memory.talk`, recall; AI lines (§10) as built; decisions D-050, D-051. **Status (v3.6):** the as-built text now covers milestone 1.4: conversations, memories and relationships (§8.3, §8.4, §9), the two new hook points (§23.9), new events, `pg residents inspect`; decisions D-046. **Status (v3.5):** adds the developer console (§14.8, §20). **Status (v3.4):** the as-built text now covers Stage 1 through milestone 1.2: schema 4 (residents, households, relationships, memories, the town), game data tables (§4.7), needs, mood and capacities (§8.1, §8.2), the town generator (§12.1) and pawn-aware routing (§7.3); decisions D-041 to D-043. **Status (v3.3):** organism decisions folded in (D-040): cardiac-output pressure model, physiology profile inputs (height, weight, sex, age, fitness; species left open), ordered anatomy extensions, generic substances. **Status (v3.2):** adds the organism system as a contract (§8.9, Stage 2B), emotional-only moods (§8.2), the capacities interface (§8.1) and the injury model (§16.5) built on the organism; nothing already built changes. **Status (v3.1):** the document describes the system **as built through milestone 0.10** (the end of Stage 0 apart from the gate, 0.11). Notes that earlier versions kept per milestone have been folded into the sections they belong to, text that disagreed with the code was corrected (listed in `docs/DECISIONS.md` D-035), and the presentation layer (§14) was rewritten around the three-layer UI that was built.

### Revision history

| Version | Settled | Where |
| --- | --- | --- |
| 1.0 | First blueprint (web and mobile oriented) | — |
| 2.0 | Native Rust binary, Luau scripting layer, host-services layer, simulation thread and worker pool, §23 modding API | whole document |
| 2.1 | `pg-canon` crate: integer-only canonical values and strict JSON, split out of `pg-core` (D-010, D-011) | §1, §2, §5.4 |
| 2.2 | Reproducibility and authoring tooling: replay diff and bisect, bug bundles, scenario files, RNG stream registry, compatibility report | §5.1, §13, §18, §20, §23.12 |
| 2.3 | Path scratch buffers (conditional), pawn-aware routing (Stage 1), compressed and trimmable replay logs | §7.3, §19, §20 |
| 2.4 | Scheduler, commitments and action skeleton made precise (milestone 0.5) | §8.5 to §8.7 |
| 2.5 | Persistence made precise: containers, both generations in the manifest, pure-Rust zstd, crash-safe saves (milestone 0.6, D-024) | §13, §20 |
| 2.6 | The app shell and graphical main menu became a Stage 0 milestone (D-025) | §14 |
| 2.7 | Host services and redaction, five providers including Player2 and device-code sign-in (0.7); typed event catalog, row-level hashes, save summaries, settings registry, string tables, bug-bundle-on-crash, keyframes, shadow verification (0.8, S-022 to S-030) | §3, §5.4, §6, §10, §13, §14 |
| 2.8 | ScriptVm spike outcome (0.9): compiler configuration, no string caps, extension data semantics, quarantine as world state, the `ScriptVm` trait as built (D-031) | §23 |
| 2.9 | App shell and main menu (0.10): the three-layer UI (D-033) | §14 |
| 3.0 | Consolidation: as-built description through 0.10, notes folded into their sections, corrections (D-035) | whole document |
| 3.14 | Mods screen redesign: tiles, `Widget::Scroll`, pack details (D-061) | §14.3, §23.2 |
| 3.13 | Installed packs, the Mods screen, capability approval and safe mode (D-060) | §23.2 |
| 3.12 | Native open and save dialogs through `rfd` behind the `Dialogs` trait (D-059) | §3 |
| 3.11 | The journal moved into the world state: written by the core, saved, hashed, replayed (D-058) | §14.9 |
| 3.10 | The town journal: events to entries, the window, the J key and HUD button, the mode rule (D-056) | §14.9 |
| 3.9 | Personality (`Pawn.outgoing`), per-turn tones, line bands by relationship and mood, looking-back talks with rehearsal; the content hash excludes string tables (D-055) | §4.7, §9 |
| 3.8 | The sprite renderer (milestone 1.6): `pg-render` sprites, atlas, scene and interpolation, the GPU layer, the view's input and the renderer stats (D-052) | §14.4 |
| 3.7 | AI lines for conversations someone can hear (milestone 1.5) and recorded conversation history (`RecordDialogue`, lines kept in the memories, recall) (D-050, D-051) | §8.3, §9.4, §10.3, §10.4 |
| 3.6 | Conversations, memories and relationships as built (milestone 1.4): the conversation table, talks as pawn state, memory bounds and daily fading, relationship caps and label hysteresis, fallback dialogue, hooks `memory.importance_modifier` and `relationship.delta_modifier`, `pg residents inspect` (D-046) | §4.7, §8.3, §8.4, §9, §23.9 |
| 3.5 | The developer console: severities, the shared log, event severities, the model and window, `pg sim --console` (D-044) | §14.8, §20 |
| 3.4 | Stage 1 as built through 1.2: schema 4, game data tables, needs, mood and capacities, population and town generation, pawn-aware routing, new tools and events (D-041 to D-043) | §4.5 to §4.7, §7.3, §8.1, §8.2, §12.1, §13.5, §14.3, §20, App. B and C |
| 3.3 | Organism questions answered: profile inputs, anatomy extension order, species left open, generic substances (D-040) | §8.9 |
| 3.2 | Organism system (pawn health) as a Stage 2B contract; emotional-only moods; capacities interface in Stage 1 (D-038, D-039) | §8.1, §8.2, §8.9, §16.5, §22, §23.8, App. B |
| 3.1 | Review of the Stage 0 build (0.11): in-game layout, drawers and their placement rules, developer mode, saving only in the pause menu (D-037) | §14, App. C |

## 0. Architectural principles

1. **Pure headless core.** The simulation core has no I/O, no rendering, no clock, no randomness source of its own, and does not depend on the scripting VM. It runs in a unit test or a command-line tool.
2. **Single writer.** Only the core mutates `WorldState`, and only through validated `Command`s and `SimInput`s applied at a tick boundary. Scripts never write state; they *request* changes that the core validates (§23.9).
3. **Determinism.** The same content (including scripts), seed and input log always produce the same state hash on every supported OS and CPU architecture.
4. **Data over code, then scripts, then Rust.** Behavior varies by data templates and tables first; Luau scripts extend behavior where data is insufficient; Rust implements generic mechanisms and performance-critical systems.
5. **Validate at every boundary.** Generated, imported, loaded, user-typed, model-produced and script-produced data are all untrusted until validated.
6. **AI is an adapter outside the core.** Its results re-enter only as recorded, validated inputs.
7. **Presentation is derived.** Rendering and UI read snapshots; they never hold authoritative state.
8. **Explainability.** Every decision (task choice, schedule placement, relationship change, script hook result) records a short machine-readable reason, attributed to its source (engine or pack).
9. **Budgeted work.** Expensive work (planning, pathfinding, generation, script execution) is bounded by deterministic budgets and runs through the worker pool or a time-sliced job queue, never unbounded inside a tick or frame.
10. **Contained extensibility.** All user code runs in the Luau sandbox, through one versioned API generated from a single source of truth, under declared capabilities. A faulty or hostile pack can be quarantined without harming the world, the save or the host.
11. **Native by design.** Use the machine: a dedicated simulation thread, a worker pool, data-oriented memory layout, and direct OS services. No portability abstraction is paid for beyond what testing needs.

## 1. System overview

```
+---------------------------- playground (one native binary) ------------------------------+
|  UI thread (main)                                     Worker pool (jobs)                  |
|  window | input | UI (egui, wgpu) | app controller   autosave encode | shadow verification  |
|     ^ RenderSnapshot (Arc swap)       | Control (channel)  sign-in | connection tests       |
|     |                                 v                                                   |
|  +--+---------------------------------------------------------------------------+       |
|  | Sim thread: fixed-step runtime: command queue, job results, session, autosave |       |
|  |   +------------------+   extension calls   +---------------------------+     |       |
|  |   | Simulation core  | <-----------------> | Script host (pg-script)   |     |       |
|  |   | (pure, headless) |  hooks / systems /  | Luau VM per pack, sandbox,|     |       |
|  |   +------------------+  actions / events   | fuel + memory metering    |     |       |
|  |                                            +---------------------------+     |       |
|  +--+--------------+----------------+---------------+------------------------+         |
|     |              |                |               |                                    |
|  Content        Persistence      AI client       Worldgen                                |
|  (templates,    (slots,          (adapters,      (pipeline,                              |
|   packs,         migrations,      fallback,       editor commands,                       |
|   validation)    export)          prompts)        validators)                            |
|  +----------------------------------------------------------------------------------+   |
|  | Host services (traits + in-memory test doubles + OS implementations):            |   |
|  | Storage, Secrets, Net, Clock, Dialogs, Audio                                     |   |
|  +----------------------------------------------------------------------------------+   |
+-------------------------------------------+-----------------------------------------------+
                                            | HTTPS direct from the machine (key per request)
                                      Model providers
```

**Threads.**

- **UI thread (main).** Window, input, the egui frame and (later) audio. It owns the UI model and the app controller, never touches `WorldState`, and draws from the latest `RenderSnapshot`.
- **Sim thread (`pg-sim`).** Owns `WorldState`, the core and every Luau VM (Luau states are single-threaded). Runs the fixed-step loop on its own accumulator, decoupled from display refresh (§6.1).
- **Worker pool.** Owned (`'static`) jobs only: autosave encode and compress, shadow verification re-simulation, connection tests, and later conversation, AI and worldgen work. Results come back through job handles and are applied by the sim thread. Path batches borrow map data and so use scoped threads behind the `BatchExecutor` trait instead (§6.5, §7.3). The sign-in flow runs on its own short-lived thread with a cancel token.

**Hand-offs.** The sim thread publishes an immutable `RenderSnapshot` through a `SnapshotPublisher` (one `Arc` swapped under a short lock; readers clone the `Arc` and then work without a lock; unchanged maps are shared between snapshots). The UI side sends `Control` messages (run-state events, commands, save, rewind, bundle) over a channel; the loop applies commands at the next tick and the runtime records every applied input with its tick (§5.3). Loop events (saved, save failed, crashed, diverged, rewound, bundle written) come back over a second channel.

**Dependency rule (enforced by `scripts/check_deps.py` in CI from `cargo metadata`):**

| Crate | May depend on (workspace) |
| --- | --- |
| `pg-host`, `pg-canon`, `pg-api` | nothing |
| `pg-host-os` | `pg-host` |
| `pg-content` | `pg-canon`, `pg-api` |
| `pg-core` | `pg-canon`, `pg-content`, `pg-host`, `pg-api` |
| `pg-script` | `pg-core`, `pg-api`, `pg-content` |
| `pg-persist`, `pg-ai` | `pg-core`, `pg-content`, `pg-host` |
| `pg-worldgen` | `pg-core`, `pg-content` |
| `pg-ui-model` | `pg-core`, `pg-host` |
| `pg-runtime` | `pg-core`, `pg-content`, `pg-script`, `pg-persist`, `pg-ai`, `pg-worldgen`, `pg-ui-model`, `pg-host` |
| `pg-render` | nothing (plain geometry) |
| `pg-app` | `pg-runtime`, `pg-render`, `pg-host-os`, `pg-ui-model`, `pg-host`, `pg-core`, `pg-content`, `pg-ai` |
| `pg-cli` | `pg-runtime`, `pg-host-os`, `pg-core`, `pg-content`, `pg-persist`, `pg-host`, `pg-ai`, `pg-script`, `pg-api` |

External crates are restricted too: `mlua` only in `pg-script`, `wgpu` and `winit` only in `pg-render` and `pg-app`, `egui` only in `pg-app`, `rand` only in `pg-cli`; the build also fails if `mlua` is built with `luau-jit` (§23.4). `pg-core` never links the VM: it defines the `HookHost` trait (§23.9) that the script host implements (dependency inversion). Nothing imports `pg-app`. `pg-core` is `#![forbid(unsafe_code)]` with the determinism lint set (§5.2).

## 2. Workspace layout

```
playground/
  Cargo.toml  rust-toolchain.toml  deny.toml  .cargo/config.toml      pinned toolchain; license policy; numeric build flags (§24)
  crates/
    pg-canon/     integer-only canonical values (`Canon`), canonical serialization, strict JSON parser; no dependencies
    pg-api/       the scripting API as data: function and type specs, capabilities, hook points with
                  combiners and clamps, since/deprecated versions; generators for pg.d.luau and the
                  reference docs; the shared "did you mean" hint code. No VM
    pg-content/   schemas, template resolver, validators, pack manifest and loader (data, strings,
                  scripts), content set with load order and content refs, settings registry, string
                  tables and pseudo-locale, JSON Schema export
    pg-core/      time, rng, ids, tables, world state, spatial (maps, occupancy, pathfinding, movement),
                  pawns, schedule and reservations, commitments, actions, tick pipeline, events and the
                  event catalog, input queue, replay and state hashing, reason codes, extension data
                  (`ext`) and hook points (`hooks`), dev scaffolding.  [later] needs, mood, memory,
                  social, conversation, economy, property, lifecycle, psychology, governance, proposals
    pg-script/    the `ScriptVm` boundary (`vm`), the Luau implementation (`luau`, the only module that
                  imports `mlua`, with its sandbox `prelude.luau`), and the script host (`host`: VMs per
                  pack, systems, hooks, fuel, quarantine, cost meter)
    pg-persist/   containers (`.pgsave`, `.pglog`, `.pgbundle`), the slot store (generations, manifest and
                  summary, recovery, delete), migrations, compatibility report, export/import, hardened
                  archive reader, crash reports
    pg-ai/        provider adapters (five), client with cache, rate cap, retry and circuit breaker,
                  settings and key manager, device-code sign-in, self-check
    pg-worldgen/  [Stage 1] generator pipeline, editor commands, validators (a placeholder until then)
    pg-runtime/   the sim loop and session thread, run states and accumulator, autosave, keyframes and
                  rewind, shadow verification, panic guard, worker pool, snapshot publisher, profiler,
                  device settings, developer-tool registry, thumbnails, and the app controller
    pg-ui-model/  the widget tree, event and effect vocabulary, the screen state machines, the developer
                  overlay model, the map palette. No drawing, no I/O
    pg-host/      host-service traits, in-memory doubles, `Secret`, `redact()`, logging
    pg-host-os/   OS implementations: filesystem, credential store, HTTPS, clock
    pg-render/    camera and tile rectangles (plain geometry); the tile and sprite renderer with atlases is Stage 1
    pg-app/       the binary: winit window, egui frame, map view, wiring, `--smoke`
  tools/
    pg-cli/       the `pg` command-line tools (§20)
  data/base/      the base game's own content pack (same format as user packs; §23.3)
  packs/          `cookbook/` sample packs that double as conformance tests; `golden/` their recorded hashes
  golden/  fixtures/  scenarios/    golden replay logs, pinned save fixtures, scenario files
  scripts/        `check_deps.py` (dependency and feature rules)
  docs/           reference/ (the three specifications), spikes/, PLAN, TODO, SUGGESTIONS, DECISIONS, BUILDING
```

## 3. Host services layer \[S0\]

A small set of traits that core-adjacent crates use for anything touching the machine. Every trait has an in-memory test double so the sim, persistence and AI client run in tests and in the headless CLI. `pg-host-os` provides the real implementations. **Rendering surface and input are not abstracted**: `pg-app` and `pg-render` own the window, GPU and input events directly, and map them to `Command`s.

```rust
trait Storage {                            // atomic named blobs under the user-data directory
    fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>>;
    fn write_atomic(&self, name: &str, data: &[u8]) -> io::Result<()>;   // temp file + fsync + rename
    fn delete(&self, name: &str) -> io::Result<()>;
    fn list(&self, prefix: &str) -> io::Result<Vec<BlobInfo>>;           // name, size, modified
    fn free_space(&self) -> Option<u64>;
}
trait SecretStore {                        // provider keys only; OS credential store
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError>;
    fn set(&self, name: &str, value: &Secret) -> Result<(), SecretError>;
    fn delete(&self, name: &str) -> Result<(), SecretError>;
    fn is_persistent(&self) -> bool;       // false: keys last for the session only (shown to the player)
}
trait Net {                                // blocking; always called from a worker thread
    fn request(&self, req: HttpRequest, cancel: &CancelToken) -> NetResult;
}
enum NetResult { Ok { status: u16, body: String },
                 Err(NetError /* Timeout | Offline | Cancelled | Tls | Other */) }
trait Clock { fn now_monotonic(&self) -> Duration; fn wall_clock_iso(&self) -> String; } // wall clock: save metadata only
trait Dialogs {                            // native file dialogs
    fn pick_file_to_read(&self, extensions: &[&str]) -> Option<PathBuf>;
    fn pick_file_to_write(&self, suggested: &str) -> Option<PathBuf>;
}
trait Audio { fn play(&self, id: &str, bus: Bus); fn set_volume(&self, bus: Bus, v: f32); fn stop(&self, bus: Bus); }
```

**Lifecycle** is not a trait: `pg-app` translates window events (focus lost, minimized, close requested, restored) into runtime events (§6.4).

**Rules:** no wall clock inside `pg-core`; secrets never pass through `Storage`; exports are built by `pg-persist`, which strips secrets; on Linux where no credential service is available, the key is kept for the session only and is never written to disk in plaintext.

**As built.** Beyond the traits above, `pg-host` has `LogSink` (with `RedactingLog`, `MemLog`, `StderrLog`), `Secret` and `redact()`, `AllowListNet`, `CancelToken`, and an in-memory double for every trait (`MemStorage` with fault injection, `MemSecretStore`, `ScriptedNet` that records requests, `FixedClock`, `ScriptedDialogs`, `NullAudio`). `pg-host-os` provides `FsStorage`, `KeyringSecretStore` (Windows Credential Manager, macOS Keychain, Linux Secret Service, with the session-only fallback), `UreqNet` (rustls, `https_only`, no redirects) and `SystemClock`. `NativeDialogs` (S-037, D-059) implements `Dialogs` with the `rfd` crate: the system's own pickers on Windows and macOS, and the desktop portal on Linux (so it works under X11 and Wayland without GTK). `AppServices` carries them as an optional service: with them, Export asks where to save and Import asks which file to open (cancelling does nothing, and a file the player picked is left alone); without them (tests, headless runs) the `imports/` and `exports/` folders of the data directory are used as before. Pack installation will use the same dialogs when the Mods screen arrives. Opening a link in the browser and showing a file in the file manager are two small closures the app hands to the controller (`AppServices`, §14.2), not host traits.

## 4. Data model

### 4.1 Identifiers \[S0\]

```rust
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
struct EntityId { kind: Kind /* u8: pawn, obj, plot, ... */, n: u32 }   // displays as "pawn_1a", "obj_3f2"
```

- **EntityId** counters are per world and per kind, stored in `WorldState.id_counters`, incremented only by the core, and **never reused** (including after deletion or death).
- **TemplateId** = dotted lower-case slug, for example `furniture.drawer`, `occupation.barista`. Pack-defined templates are namespaced by the pack id unless they intentionally extend or override a base template through a declared mechanism (§23.3).
- **Import remap:** imported entities get fresh EntityIds; a remap table rewrites every reference; unresolved references fail validation.
- References between entities are always by id, never by pointer or `Rc`.

### 4.2 Templates versus state \[S0\]

|  | Templates (content) | State (world) |
| --- | --- | --- |
| Mutability | Immutable at runtime | Mutated by the core only |
| Storage | Content packs (including the base pack) | World save |
| Identity | TemplateId + version | EntityId |
| Examples | Object defs, occupations, schedule templates, names, layouts, fallback dialogue, scripts | Pawns, objects, maps, memories, reservations, script-defined component values |

A world save records `content_refs: Vec<ContentRef { pack_id, version, hash }>`. The hash covers data **and script source**. Loading fails safely with a clear message if a referenced pack is missing or its hash differs; the player may choose to continue with a compatibility report or start in **safe mode** (§23.10). The compatibility report is the same one `pg content diff <old> <new>` produces offline: removed or renamed templates, changed component schemas and defaults, each flagged *save-breaking* or *safe* \[S0, milestone 0.6\].

### 4.3 Object templates and resolution \[S0\]

```rust
struct ObjectTemplate {
    id: TemplateId,
    schema: u32,                                   // template schema version
    extends: Option<TemplateId>,                   // single inheritance chain, root is "base.object"
    tags: BTreeSet<Tag>,                           // "furniture", "weapon", "humanoid"
    components: BTreeMap<ComponentName, Option<ComponentParams>>,  // None = delete inherited component
    containers: Vec<ContainerDef>,                 // sugar for the "container" component
}
```

**Resolution algorithm** (`resolve_template(id)`), memoized per content load:

1. Walk `extends` to the root; reject cycles and depth > 16.
2. From root to leaf, merge `components` by name: objects deep-merge, arrays replace, `None` deletes the inherited component.
3. Union `tags`. Validate each component against its registered component schema.
4. Output a frozen `ResolvedTemplate` (`Arc`-shared, immutable).

**Pack files and namespaces \[S0\]:** a pack's templates live in `data/templates/**/*.json` (each file one template or a list). Only the `base` pack may define ids outside its own namespace; every other pack's template ids must start with `<pack_id>.`. A template may extend only templates from its own pack or from a pack it (transitively) depends on. Every chain must end at `base.object`.

Example chains: `base.object > base.item > base.furniture > furniture.drawer`; `base.object > base.creature > base.sapient > base.humanoid > creature.human`.

**Component registry:** each component has a name, a params schema, an initial-state factory, and optional systems that read / write its state. Built-in components (`physical`, `interaction`, `container`, `durability`, `value`, `damage`, `needs_restore`, `appearance`) are registered in Rust. **Pack components** are registered through the script API using the declarative schema subset (§23.4, §23.10): bounded integer, enum, boolean, entity-reference and bounded-list fields. This lets the validator, serializer, state hasher, inspector and migrations handle them generically with no per-component Rust. *Using* any component in a template is data-only.

### 4.4 Containers and containment \[S0\]

```rust
struct ContainerDef { id: String, accepts: AcceptRule, capacity: u32, ordered: bool }
struct ContainerState { slots: Vec<EntityId> }                       // instance state
struct Parent { container_owner: EntityId, container_id: String }    // on child
```

**Invariants** (checked by `validate_containment`, run on every mutation in debug builds and on load):

- Every entity has at most one `Parent`.
- No containment cycles.
- A child satisfies its container's `accepts` and capacity.
- Parent and child agree (child lists parent; parent lists child).
- Deleting a container owner either re-parents or deletes children per a declared policy (`Cascade | Evict | Forbid`).

### 4.5 World state shape \[S0\]

```rust
struct WorldState {
    schema: u32,
    meta: WorldMeta,                      // world_id, name, seed, created_iso
    settings: WorldSettings,              // preset, aging multiplier, population switches, cap, mod_settings
    clock: Clock,                         // { tick: u64 } the only time source in core
    id_counters: BTreeMap<Kind, u32>,
    rng_counters: BTreeMap<StreamId, u32>,    // per stream (see §5)
    maps: Table<MapData>,                 // overworld + interiors [interiors S6]
    objects: Table<ObjectInstance>,
    pawns: Table<Pawn>,
    households: Table<Household>,
    relationships: Table<Relationship>,   // keyed pair_key(a,b), a < b
    commitments: Table<Commitment>,
    events: EventLog,                     // bounded ring + summary archive
    plots: Table<Plot>, buildings: Table<Building>,        // [S6]
    ledger: Table<Transaction>, accounts: Table<Account>,  // [S6]
    ext: ExtState,                        // pack component values, keyed by (EntityId, "pack.component"); orphans (§23.10)
    content_refs: Vec<ContentRef>,
    starting_snapshot_hash: Hash,         // see §12.4
}
struct Table<T> { rows: BTreeMap<EntityId, T> }   // iteration is always in ascending id order
```

`BTreeMap` is the default because sorted iteration is a determinism requirement. A dense sorted `Vec` plus an index is an allowed optimization if profiling demands it, provided iteration order and hashes are unchanged (golden replays prove it).

### 4.6 Core entity schemas

```rust
struct Pawn {                                      // [S1]
    id: EntityId, name: String, appearance: AppearanceRef,
    occupation: OccupationRef { template: TemplateId, variation: i32 },  // archetype first [S1], job later [S6]
    controller: Controller /* Sim | Player */,     // [S4]
    needs: Needs { hunger: i32, energy: i32, social: i32 },              // 0..=1000 fixed-point
    mood: MoodId,
    position: Position { map: EntityId, x: i32, y: i32 },
    facing: Dir4,
    state: PawnState /* Idle | Moving | Performing | Conversing | Sleeping | Incapacitated | Dead */,
    task: Option<Task>,
    schedule: DaySchedule,                         // today's slots (§8.5)
    memories: Vec<Memory>,                         // bounded, see §8.3
    household_id: Option<EntityId>, home_ref: Option<EntityId>,
    traits: Vec<TraitId>,                          // few at first; values / sliders added [S7]
    birth_day: i64,                                // game-day index [S7]
    inventory_ref: Option<EntityId>,               // container object [S6]
    flags: BTreeMap<FlagId, i32>,
}
struct Task { id: TaskId, action_id: ActionId, params: ActionParams, reservation_id: Option<ReservationId>,
              step_index: u32, started_tick: u64, reason: ReasonCode }
struct Memory { id: MemoryId, ty: MemoryType, tick: u64, participants: Vec<EntityId>,
                severity: u8 /* 1..=5 */, impact: i32 /* signed */, importance: i32, decay_per_day: i32,
                retention: i32, topic: Option<TopicId>, tone: Option<ToneId>, summary_key: Option<String> }
struct Relationship { a: EntityId, b: EntityId, affinity: i32 /* -1000..=1000 */, label: LabelId,
                      last_interaction_tick: u64 /* [S7] trust, fear, conflict, archetype */ }
struct Reservation { id: ReservationId, pawn_id: EntityId, start_slot: u32, slots: u32,
                     priority: u8 /* 1..=5 */, kind: ReservationKind, action_id: ActionId,
                     params: ActionParams, urgency: i32, commitment_id: Option<CommitmentId>,
                     movable: bool, reason: ReasonCode }
struct Commitment { id: CommitmentId, proposer: EntityId, invitee: EntityId, day: i64,
                    start_slot: u32, slots: u32, activity: ActivityId, place: PlaceRef, reschedulable: bool,
                    state: CommitmentState /* Proposed | Accepted | Declined | Expired | Active | Completed | Failed | Cancelled */,
                    created_tick: u64, expires_tick: u64 }
```

All authoritative numbers are integers (`i32`, or `i64`/`u64` for ticks and day indexes) or fixed-point newtypes (§5.2). Domain values are newtypes (`Permille`, `NeedValue`, `Affinity`) with checked or saturating constructors so out-of-range values cannot be represented.

### 4.7 Game data tables \[S1\]

Numbers and lists the systems read are data in a pack's `data/game/` folder (milestone 1.0): `needs.json`, `mood.json`, `memory.json`, `relationships.json`, `conversation.json`, `occupations.json`, `names.json`, `residents.json` and `worldgen.json`. Everything is integers. Each file is validated with the path of every problem and a "did you mean" hint, entries are identified by `id`, a later pack's entry with an existing id replaces it (and the report says so), and the merged whole is cross-checked (a mood rule names a real mood and need, a resident a real occupation, relationship labels cover -1000 to 1000, duties fit in a day and do not overlap, roles and district kinds are known); the core adds the check that every action an occupation or need uses exists in the action registry. String keys the data refers to must exist in the string table (a warning otherwise). The data is hashed with the rest of the pack, so a world records which tuning it was made with.

**Needs** (`needs.json`) carry their decay per game hour and an active multiplier, urgent and critical thresholds, the capacities they lower at each threshold, a passive recovery for a pawn that cannot act, and a `restore` block: the action, the place kind (`home`, `workplace`, `gathering`, `anywhere`), how long an unplanned restoring activity lasts, and the day's **routine** (start window and duration of meals, bedtime and company). **Mood** is a list of moods and an ordered rule table (`need_critical`, `any_need_urgent`, `all_needs_above`, `memory` with a tone, window and importance, `idle_hours`; the first rule whose conditions all hold decides). **Memory** parameters, **relationship** labels with hysteresis and the daily cap, **occupations** (duties and leisure with seeded variation bounds), **names** and authored **residents** complete the people; `worldgen.json` holds the town generator's sizes and role weights.

## 5. Determinism \[S0\]

### 5.1 Randomness

Use **counter-based, stateless** random draws so results do not depend on call order across systems:

```rust
fn rand(world_seed: &Seed, stream: StreamId, keys: &[Key], counter: u32) -> u32
// = mix64(hash(world_seed), hash(stream), hash(keys...), counter) truncated to 32 bits
```

- **Hash:** a fixed, specified integer hash (for example an `xxh3-64` or splitmix64 construction), pinned by exact crate version or implemented in-repo, with published test vectors. Never `std::hash::DefaultHasher` or any hasher whose algorithm is unspecified.
- Derived helpers: `rand_int(min, max)`, `chance(permille)`, `pick(list)`, `shuffle(list)` (Fisher–Yates over a keyed stream).
- **Stream registry \[S0, from milestone 0.5\]:** every stream name is declared once in a registry (`pg_core::rng::streams`, mirrored in `pg-api` for `mod.<pack>.<name>` streams) and a test fails if any draw site uses an unregistered name, so a typo can never silently create a "new" stream.
- **Streams** are named: `worldgen.terrain`, `worldgen.roads`, `worldgen.plots`, `worldgen.people`, `sched.variation`, `sched.tiebreak`, `social.topic`, `social.outcome`, `event.<category>`, `path.tiebreak`. **Pack streams** are namespaced `mod.<pack_id>.<name>` and are the only streams scripts can draw from (§23.5). Each draw site passes stable keys (pawn id, day index), so adding a new system or pack never perturbs existing streams.
- Where a draw must be consumed sequentially, a counter in `WorldState.rng_counters[stream]` is incremented and saved.

### 5.2 Numbers and ordering

- No floating point in authoritative state or rules. `pg-core` enables `#![deny(clippy::float_arithmetic, clippy::float_cmp)]`. Use integers; for fractional values use fixed-point with a documented scale (needs 0–1000; rates in permille; positions in tiles as integers, sub-tile offsets only in presentation). Floats are permitted in `pg-render`, camera and audio, and inside the Luau VM, but **never cross into authoritative state** (the script boundary rejects non-integer numbers, §23.5).
- Forbidden in `pg-core` (enforced with `clippy.toml` `disallowed-types` / `disallowed-methods` and CI): `std::collections::HashMap` / `HashSet` (use `BTreeMap` / `BTreeSet`; a fixed-seed hasher is allowed only for derived lookups that are never iterated), `Instant`, `SystemTime`, `rand` and `thread_rng`, locale-dependent sorting, `f32` / `f64`.
- All iteration is over id-sorted tables. All sorts are stable with an explicit total-order comparator ending in the entity id (`sort_by`, never an unspecified order; `sort_unstable` only when the comparator is already a total order).
- Division and rounding rules are specified: integer division truncates toward zero (Rust `/`); `%` follows the dividend's sign; a named `round_half_up` helper is used for rounding. Arithmetic uses named checked or saturating helpers where overflow is possible, and `overflow-checks = true` is kept on in release builds for `pg-core`, so a bug is a loud error rather than silent wraparound.
- No `unwrap` / `expect` / indexing panics on data-derived values in `pg-core` (`clippy::unwrap_used`, `clippy::indexing_slicing` denied outside tests).

### 5.3 Input log and replay

All external influence on the world is a `SimInput` stamped with the tick at which it applies:

```rust
enum SimInput {                                                   // as built
    SettingChange(SettingChange),                                 // world settings (slot length)
    Command { actor: Option<EntityId>, cmd: Command },            // player actions, time control, dev commands
    // arrive with their stages:
    // Dialogue { conversation_id, lines, source /* Ai | Fallback */ }   [S1]
    // Proposal { pawn_id, proposal }                                    [S10]
}
struct StampedInput { tick: u64, seq: u64, input: SimInput }      // the queue stamps the tick and a sequence number
```

Inputs apply at the start of the tick they are stamped with, ordered by (tick, kind order, sequence). AI text is nondeterministic, so its **result is recorded as an input** and replay uses the recording. Script behavior is deterministic by contract (§23.5), so scripts need no recording beyond their content hash in `content_refs`; a pack's quarantine is **world state** (§23.6), not an input, and developer hot reload is a tool, not part of a replay. Therefore `(initial snapshot | seed + content refs) + SimInput log` always reproduces the same state. Replay logs record every applied input, per-table hashes per day and the content refs, and are versioned (format 4); a log may carry a start state so it can be trimmed (§20).

### 5.4 State hashing

`hash_state(world)` = `blake3` over the canonical serialization (sorted keys, no whitespace, integers only), computed per table and combined. This includes pack component tables (`ext`). The runtime computes and logs a hash at every day boundary in debug and test builds, and stores the latest in each save for integrity checks.

**Known difference from RFC 8785 (0.6, D-022):** `Canon` writes object members in Unicode code point order (UTF-8 byte order); RFC 8785 sorts by UTF-16 code units. They agree except when keys mix characters from U+E000-U+FFFF with supplementary-plane characters. Engine keys are ASCII, so hashes and saves are unaffected and remain deterministic; the difference is pinned by a test and matters only if canonical JSON is ever handed to an external RFC 8785 verifier with such keys.

**Row-level hashes.** Each table's hash is the combination of per-row hashes (row id and the row's canonical form), so a divergence can be localised below the table: `pg replay --bisect` and the divergence detector name the entity (`pawns: pawn_3`). Day hashes recorded in logs stay per-table; a per-row dump is produced on mismatch. Memory and CPU cost are about those of hashing the table whole. Pack extension data (`ext`) is a table of its own that is left out of the hash while empty, so worlds without packs hash as they always did (§23.10).

## 6. Simulation loop and time \[S0\]

### 6.1 Time units

- **Tick** = smallest simulation step. `TICKS_PER_GAME_MINUTE = 10` (constant). A game day = 1,440 minutes = 14,400 ticks.
- **Slot** = schedule slot, default 30 game minutes = 300 ticks (data setting `slot_minutes`).
- **Speed.** Four speeds: **1x, 3x, 9x and 27x** are 10, 30, 90 and 270 ticks per real second. At 1x a tick is 100 ms and a game day lasts 24 real minutes (inside the 20–30 minute target); there is no separate day-length setting. Headless and developer fast-forward run unthrottled.

The core knows only ticks. The sim thread converts real time to a tick count with an accumulator that keeps the fractional remainder:

```
ticks = accumulator.advance(elapsed, speed)          // at most MAX_TICKS_PER_FRAME (50) per frame
for _ in 0..ticks { step() }
```

If the machine cannot keep up, the world runs slower (the excess is dropped); it never skips ticks and never spirals.

### 6.2 Tick pipeline (fixed order)

1. **Apply inputs** scheduled for this tick, sorted by (kind order, pawn id, sequence).
2. **Advance clock**; compute boundary flags (minute, slot, day).
3. **Systems** run in this order; each declares reads, writes, cadence and **extension points** (§23.7). Pack systems run at declared anchor points (`before` / `after` a built-in system) in a total, deterministic order (anchor, then pack load order, then system id).
4. **Emit** a `TickReport` (changed entity ids, events, reasons) to the runtime for snapshots and logging.

| # | System | Cadence | Reads → writes | Extension points |
| --- | --- | --- | --- | --- |
| 1 | NeedsSystem | minute | needs, activity effects → needs | hook `needs.decay_modifier` |
| 2 | MoodSystem | minute | needs, recent memories → mood | hook `mood.rules` (appended rules) |
| 3 | DayPlanner | day / on invalidation | occupation, needs, commitments → schedule | hooks `schedule.duty_variation`, `schedule.leisure_weight` |
| 4 | CommitmentSystem | slot | commitments, schedules → commitments, reservations | hook `commitment.accept_modifier` |
| 5 | ReservationActivator | slot | schedule → pawn intent | — |
| 6 | TaskPlanner | tick (only pawns needing a task) | intent, world → task | action registry; hook `task.free_time_candidates` |
| 7 | MovementSystem | tick | tasks, occupancy → positions | — (costs are data) |
| 8 | ActivitySystem | tick | tasks → task progress, effects | action `effects` |
| 9 | ConversationSystem | tick | proximity, social need → conversations | hooks `conversation.topic_weight`, `conversation.outcome_delta` |
| 10 | MemorySystem | event / day | events → memories; day: decay | hook `memory.importance_modifier` |
| 11 | RelationshipSystem | event | memories / outcomes → relationships | hook `relationship.delta_modifier` |
| 12 | EventSystem | slot / day | presets, rolls → events \[S8\] | event categories (data), event handlers |
| 13 | Maintenance | tick | indexes, cleanup | — |

Systems whose per-pawn work is independent may run data-parallel on the worker pool (for example needs and mood updates), provided results are merged in ascending id order and golden replays stay identical. This is an optimization gated on measurement (§19), not a requirement.

**Event catalog (S-022).** Events stay `(kind, detail)` values that are not part of hashed state, but each kind is declared once in a catalog (kind, category, field schema using `ParamSchema`, default visibility). Debug builds validate every emitted event against it; packs register `<pack>.<kind>`; `pg events list` and the overlay's event viewer read the same catalog (Appendix B).

### 6.3 Movement speed versus tile scale

One tile is about 1 m, but a game day lasts about 25 real minutes, so physically accurate walking would cross the town in a blink. **Movement speed is a tuned abstraction**, stored as `move_ticks_per_tile` (a data value of at least 1 tick per tile, tuned so typical in-town trips take roughly 5–20 game minutes; if that needs finer speed control, raise `TICKS_PER_GAME_MINUTE`). It is a setting, not a derived value, and is listed in the roadmap's tuning decisions.

### 6.4 Run states and the runtime loop

```
Paused <-> Running(speed)                  the player pauses, resumes and changes speed
Running -> Suspended                       the window lost focus or was minimised (if the setting is on); a save starts
Suspended -> Running | Paused              only when the player resumes: the world never resumes by itself
any -> Closing -> Stopped                  close requested: final save, then the loop ends
any -> Crashed                             a tick panicked: the world is frozen, never continued
```

`RunState`, `RunEvent` and the transition function are pure (`pg-runtime::control`) and table-tested. A loaded world starts **Paused**. The pause menu pauses a running world and resumes it when closed (a UI rule, §14.3). The accumulator, the autosave timer (every N minutes of running time; a setting) and the 50-tick frame cap live in the same module. `SimLoop` owns the sim, the input log, the keyframe ring, the run state and the save and shadow jobs; `Session` runs it on the `pg-sim` thread with a frame interval of a few milliseconds and exposes `send`, `snapshot`, `events` and `shutdown`.

**Safety nets.** (a) *Panic guard:* every tick runs under `catch_unwind` with a process-wide hook that captures the message, location and backtrace on the panicking thread; a panic freezes the world (`Crashed`), writes a redacted crash report and an automatic `.pgbundle` to `crash/`, and keeps the UI alive; the next launch offers the bundle (§14.3). (b) *Keyframes (S-029):* a memory-bounded ring of full snapshots (default one per simulated hour, the last 24), each with its state hash and the input-log position; they are what the overlay's time scrub restores, what the crash path starts a bundle from, and what shadow verification replays. (c) *Shadow verification (S-030):* optionally (developer and soak runs) a worker re-simulates each span between keyframes in a fresh sim with a different executor thread count and a freshly built pipeline (so every script VM is rebuilt) and compares state hashes; a mismatch names the differing tables, pauses the world and writes a bundle. (d) *Rewind:* the newest keyframe at or before a target tick is restored and the logged inputs replayed forward; the future is discarded.

### 6.5 Jobs and the worker pool

`WorkerPool` is a fixed set of threads running **owned** jobs (`'static` closures). `submit` returns a `JobHandle` (`join`, `try_join`); `map_ordered` applies a function to a list and returns the results **in input order**, so using the pool can never reorder anything the simulation sees; a job that panics yields a `JobPanic` to its caller and the worker keeps running. Used today for save encoding, shadow re-simulation and connection tests; conversation, AI and generation jobs follow.

**Pure parallel work inside a tick** (a batch of path requests) borrows the maps, which a persistent thread cannot hold without `unsafe`, so it goes through the core's `BatchExecutor` trait: a serial implementation in the core and a scoped-thread implementation in the runtime. Because each path request is a pure function of its inputs, results are identical for any executor and thread count (checked in unit tests, by `pg map bench-paths` and in CI). **Asynchronous work** returns a result that the sim thread applies at a tick boundary, and anything that changes the world is a recorded input, so determinism holds even though jobs finish at different real times.

## 7. Spatial systems

### 7.1 Map data \[S0\]

```rust
struct MapData {
    id: EntityId, kind: MapKind /* Overworld | Interior */, w: i32, h: i32,
    terrain: Vec<u8>,          // terrain type id per tile (grass, water, sand, ...)
    surface: Vec<u8>,          // road, sidewalk, floor, none
    blocked: Vec<u8>,          // 1 = static blocker (building footprint, water, wall)
    zone_id: Vec<u16>,         // district / room id per tile
    portals: Vec<Portal>,      // entrances linking maps [interiors S6]
    chunk_size: u8,            // 16
}
struct Portal { id: PortalId, tile: Tile, to_map: EntityId, to_tile: Tile, building_id: Option<EntityId> }
```

Tile arrays are flat, row-major, indexed `y*w+x`. Editing a tile marks its chunk dirty for the render cache and for path invalidation.

### 7.2 Occupancy \[S0\]

- `occupant_at: BTreeMap<MapId, Vec<i32>>` holds the id index of the pawn standing on each tile (or -1). It is **derived** state, rebuilt on load and maintained by MovementSystem.
- One pawn per tile for walking. Interaction slots (a bed, a bench seat) are defined by object components (`interaction.slots`) and reserved by task, not by tile.

### 7.3 Pathfinding \[S0\]

- **Algorithm:** A\* on a 4-connected grid (8-connected optional later). Cost = terrain/surface cost (road cheaper than grass; water and blockers impassable). Heuristic = Manhattan distance times minimum step cost.
- **Determinism:** the open set is a binary heap ordered by (f, h, tile index). Neighbor expansion order is fixed (N, E, S, W). No randomness.
- **Limits:** node-expansion cap per request (default 20,000); on exceeding, return `UnreachableOrTooFar`.
- **Goal handling:** a destination is a `PlaceRef` (tile, object interaction slot, room, or building entrance). The resolver picks the nearest free *interaction tile* by (distance, tile index). If none is free, the request returns `BlockedDestination`.
- **Parallel batches:** each tick, path requests are collected, solved in parallel on the worker pool (a pure function of map, endpoints and `map_edit_version`), and applied in ascending pawn id order. Because requests are pure, results are identical regardless of thread count.
- **Caching:** key `(map_id, from, to, map_edit_version)`; bounded LRU. Cache is derived and never saved.
- **Scaling \[S11 if profiling needs it\]:** hierarchical pathfinding over 16×16 chunks with portal edges; the abstract route is computed first, then refined per chunk. A golden test compares its path cost against flat A\*.
- **Pawn-aware routing \[S1, built in 1.2\]:** the first solve of a route ignores other pawns and is cached as above. When a pawn has been blocked (it waited, sidestepped and asks to re-solve), the re-solve treats the tiles of pawns that are standing still (no route) as obstacles, other than the goal, and walks around them (`find_path_avoiding`, uncached because the obstacle set changes every tick, deterministic because it depends only on the world at that tick); if no way around exists it falls back to the plain path and waits as before. This replaced a jam of residents at gathering places and doors, and turned most `path_blocked` failures into short detours.
- **Cross-map \[S6\]:** route over the portal graph (Dijkstra on portals, cost = in-map path length), then path inside each map.

### 7.4 Movement step \[S0\]

Each tick, for a pawn with a path and `move_ticks_per_tile` elapsed since its last step:

1. Look at the next tile. If static-blocked (map edited), request a repath.
2. If occupied by another pawn: the pawn with the **lower id** moves; the other waits. After `max_wait_ticks`, try a one-tile sidestep to a free neighbor; after `max_repaths`, fail the step with reason `PathBlocked`.
3. Otherwise move, update occupancy and facing.

Presentation interpolates between the previous and current tile; interpolation never feeds back into state.

### 7.5 Interaction points \[S1\]

Objects and map features expose named interaction points (for example `sit`, `sleep`, `eat`, `talk_here`). Actions refer to points by name and map to a concrete tile at planning time. Public gathering places (parks, plazas) are objects with `gathering` and `interaction` components, and drive social proximity.

## 8. Pawn behavior systems

### 8.1 Needs \[S1\]

Values are integers 0–1000 (1000 = fully satisfied). Parameters live in data (`needs.json` in the base pack). Packs may add needs through the script API (§23.7) within engine-defined bounds.

| Need | Decay | Restored by | Urgent below | Critical below |
| --- | --- | --- | --- | --- |
| hunger | per game minute | eating activities | 300 | 100 |
| energy | per game minute (faster when active) | sleeping, resting | 250 | 80 |
| social | per game minute (slower when recently social) | conversations, shared events | 350 | 150 |

Decay rates are permille per minute and tunable. Activities declare `needs_restore` effects as per-minute deltas. At *critical*, a current reservation of priority 3 or lower-importance is interrupted and a priority-2 urgent-need reservation is inserted. Consequences of ignoring needs are limited in S1 (mood penalty, slowdown); stronger ones come with later systems. The `needs.decay_modifier` hook may scale decay within an engine-clamped range (500–1500 permille).

**Capacities (interface built in Stage 1).** Consequences of needs are not hard-coded into movement or scheduling. The core derives a small record of integer permille capacities per pawn each minute (`consciousness`, `moving`, `manipulation`, `talking`, `eating`, `breathing`; sight and hearing later) and movement, the scheduler, conversation and actions read only that record (for example `movement.speed_modifier` scales by `moving`; an action declares which capacities it needs). In Stage 1 the record is filled from needs; from Stage 2B the organism (§8.9) produces it and the needs shortcut is removed. Hunger, energy and social stay as the player-readable needs.

**As built (1.1).** `NeedsSystem` runs every game minute in the Needs slot (and `MoodSystem` in the Mood slot); both are installed when content with game data is given to a `Sim`, replacing the scaffolding probe. A need decays by `per_minute(minute, rate)`, the difference of two whole-minute totals on the global minute counter, so 55 points an hour is exactly 55 over any 60 minutes with nothing to carry, save or hash; walking raises the rate by the need's active multiplier. Restoring is a property of **actions**: `ActionDef.restores` lists the needs an action restores per hour while it is being performed (and whether another pawn must be within five tiles); the built-in actions `eat`, `sleep`, `rest` and `socialise` do this. A pawn that cannot act recovers at the need's passive rate. Crossing below urgent or critical raises `need.urgent` or `need.critical` and asks for a replan. The day's rhythm comes from each need's `restore.routine`: the day planner places a restoring activity for each window (a new `UrgentNeed.earliest` bound with the window's last start as its deadline), and a need that is already urgent adds an unplanned one at once. Capacities are derived from the needs each minute (the lowest cap per capacity); a pawn whose consciousness falls below 500 collapses where it stands (task and route cleared), recovers slowly, and stays down until its energy is back at the urgent level, so it never flickers at a threshold. `Pawn.capacities` is saved and hashed; movement speed scales with `moving` and a pawn that cannot act neither walks nor starts tasks.

### 8.2 Mood \[S1\]

Mood is a small enum derived each minute by an ordered rule table (first match wins), for example:

1. social need critical → `lonely`
2. recent high-importance negative memory (last 6 game hours) → `upset` or `sad` (by the memory's tone)
3. any need urgent → `uneasy`
4. recent positive memory or all needs above 600 → `content` / `cheerful`
5. nothing scheduled and nothing happening for a long stretch → `bored`
6. otherwise → `neutral`

Moods are **emotional states only**. Physical states such as exhaustion and starvation are body conditions (capacities now, organism conditions from Stage 2B), not moods. From Stage 2B the rule table may also read pain and conditions as inputs.

**As built (1.1).** The rule table is `mood.json`, evaluated for every pawn each game minute (the first rule whose conditions all hold decides); a change raises `mood.changed` with the rule's id. Memories made before the world began (tick 0) do not move anyone's mood. `idle_since` on the pawn feeds the `bored` rule (nothing planned and nothing being done for the rule's hours).

The list and thresholds are data and are tuned in play. Packs may append rules at declared priorities through `mood.rules`; engine rules at priority 1 (critical needs) cannot be pre-empted. Mood influences conversation tone and activity choice weights, never hard rules.

### 8.3 Memory \[S1\]

**Creation:** MemorySystem converts qualifying events (conversation ended, shared event) into one memory per participant.

```
severity    = base table by type (1..=5)
impact      = signed from outcome: positive/negative tone x relationship change magnitude
importance  = severity * (10 + |impact|)          // integer, higher = more important
decay_per_day = max(1, K / severity)              // low-severity memories fade faster
retention   = 1000 at creation
```

**Daily decay:** `retention -= decay_per_day`. A memory is **persistent** if `importance >= persist_threshold` (never removed); otherwise it is deleted when `retention <= 0`.

**Bounds:** each pawn keeps at most `max_memories` (default 200). When over, remove the lowest `importance * retention` non-persistent memory (ties by older tick, then id). Removed minor memories roll into a small per-relationship summary (`last_topic`, `count`) so relationships do not forget everything.

**Retrieval:** `select_relevant_memories(pawn, context, n)` scores by (importance, recency, participant overlap, topic match). It feeds the inspector and, later, AI context builders, and returns the reason fields so the inspector can explain each pick.

**As built (1.4).** `pg-core::memory` holds `remember` (add, then forget the lowest `importance * retention` ordinary memory while over `max_memories`; ties by older tick, then id), `age_memories` (the daily fade, run by `MemorySystem` at each new day boundary) and `select_relevant_memories` (importance, plus up to 300 for recency over two days, 200 for involving the person in question, 150 for the same topic; each pick returns its reasons). Persistent memories (importance at or above `persist_threshold`) are never forgotten, so the bound applies to ordinary memories only. What is forgotten is rolled up into the relationship it concerned (`last_topic`, `forgotten`) and raises `memory.expired` (`faded` or `crowded_out`). Conversations make one memory per participant (`memory.created`) with the topic's severity, the applied affinity change as impact, and the summary key `memory.conversation.<topic>`; the hook `memory.importance_modifier` scales its importance. The sizes are steady rather than growing: low-severity memories last about 17 days, so a town's saved state levels off after a few weeks (the 30-day soak allows 3x growth from day 5 for this reason).

### 8.4 Relationships \[S1; expanded S7\]

- Stored once per pair (`a < b`). `affinity` −1000..=1000.
- After each conversation, `affinity += outcome_delta(topic, tone, moods, traits)`, clamped, with a small per-pair daily cap for diminishing returns. Pack changes go through the same function and the same cap (`relationship.delta_modifier` hook, or the `AdjustRelationship` effect), so they cannot bypass diminishing returns.
- **Labels** (as built in 1.4: `label_with_hysteresis`; a label moves up only when affinity is the margin past the next label's threshold and down only when the margin below its own) map from affinity thresholds (data): stranger, acquaintance, friendly, friend, close friend; negative side: wary, disliked, enemy. Hysteresis (a margin) prevents flickering labels.
- **As built (1.4).** A relationship is created at the pair's first conversation (starting affinity from the data) and every change goes through `Relationship::apply_delta`: the net change since the start of the game day is held within `+-daily_cap`, affinity is clamped to -1000..1000, the label is updated with hysteresis and `relationship.label_changed` is raised. The pair also keeps `last_topic` and `forgotten`. The `relationship.delta_modifier` hook scales the requested change before the cap (so the cap cannot be bypassed).
- **\[S7\]** adds `trust`, `fear`, `conflict` and archetypes with their own update rules; romance and family states are added only after friendship persistence passes its acceptance checks.

### 8.5 Scheduler and reservations \[S0 core, S1 templates, S2 full\]

```rust
struct DaySchedule { day: i64, slot_minutes: u32, slots: Vec<Option<ReservationId>>,
                     reservations: BTreeMap<ReservationId, Reservation> }
```

**Inputs to planning:** occupation template (duties and hours, with seeded variation), current needs, accepted commitments for the day, personal chores / goals, leisure candidates (weighted by environment and later personality).

**Planning algorithm** (`plan_day(pawn, day)`), deterministic:

```
slots = empty array of N slots
1. place PRIORITY 1 duties: from the occupation template, apply seeded variation
   (shift/shorten within template-declared bounds) via rand(sched.variation, [pawn, day]).
2. place PRIORITY 2 urgent needs: for each need predicted to hit 'urgent' during the day
   (projection from current value and decay), insert a restore activity at the earliest
   free slot before the predicted time; if none is free, it may displace a priority 4-5
   reservation (never 1 or 3).
3. place PRIORITY 3 accepted commitments at their agreed slots.
   If a slot conflicts with priority 1: duties win; the commitment follows its
   reschedulable flag (move to nearest free slot same day, else fail with reason).
4. place PRIORITY 4 chores/goals: sort by (urgency desc, earliest deadline, id); first-fit into free slots.
5. place PRIORITY 5 leisure: candidates weighted by (mood, needs, environment, [S7] personality,
   clamped schedule.leisure_weight hook results); choose with rand(sched.tiebreak, [pawn, day, slot]);
   leave remaining slots UNRESERVED.
```

**Conflict rules (all deterministic):**

- Priority 1 beats 5; a lower number never yields to a higher number.
- Equal priority: higher `urgency`, then earlier `created_tick` / earliest commitment, then lower `reservation.id`.
- Priorities 4–5 that lose may move to a *later* free slot the same day; if none, they are dropped with reason `NoFreeSlot`.
- A priority-3 commitment loses only to priority 1–2 and then follows its `reschedulable` flag.

Script hooks and script-requested reservations enter planning only as weights and *requests*; they cannot place a reservation directly or change a priority.

**Replanning triggers:** day start; need becomes critical; commitment accepted / cancelled; possession ends \[S4\]; map edit invalidating a destination; interruption (event, injury). `replan_from(slot)` re-runs steps 2–5 for remaining slots only, never touching completed slots.

**As built:** (a) Planning runs in strict priority order, so a full `plan_day` never has a lower-priority reservation to displace; "may displace a priority 4-5 reservation" is therefore implemented by `insert_urgent`, the mid-day path used when a need becomes critical (displaced reservations return unchanged if their slots are still free, otherwise move to the first later free run, otherwise are dropped with `no_free_slot`). (b) Replans are requested by setting `Pawn.replan` and carried out at the next slot boundary (inside ReservationActivator, before activation), starting at that boundary, so a slot already under way is never rewritten; a task failure therefore replans from the following slot. (c) A started reservation is never displaced, including by a commitment. (d) Reservation ids are per-schedule counters and survive being lifted out and put back. (e) A change of slot length makes every schedule stale (it is replanned at the next boundary) and cancels live commitments, because both are expressed in slots.

**Activation:** at each slot boundary ReservationActivator sets the pawn's intent to the reservation covering that slot. Unreserved slots set intent `Free`; the TaskPlanner may then pick small self-directed behavior from a data table (wander, sit), keeping unreserved time genuinely open.

### 8.6 Commitments protocol \[S2\]

```
proposer decides -> create Commitment{state: Proposed, expires_tick}
  -> invitee evaluates at next slot boundary (deterministic order: created_tick, then id):
       available(invitee, slots) && not blocked by priority 1-2 && relationship not hostile
         -> Accepted; BOTH schedules reserve at priority 3 (atomic: both or neither)
         -> else Declined (reason code)
  -> expiry with no answer -> Expired
  -> at start: Active -> both pawns head to PlaceRef
  -> end: Completed (memory + relationship + social restore) or Failed (no-show, blocked path)
```

Reservations are created **only after acceptance**, and acceptance is evaluated on copies of both schedules and committed only if both succeed (atomic). A commitment may displace priority 4-5 reservations that have not started; it never displaces a duty, urgent need, another commitment or anything under way. Two pawns cannot share a tile, so commitments use the built-in `meet_at` action: a pawn has arrived once it is within `MEET_RADIUS` (2) tiles of the place, and attendance at the end is judged by the same radius. Relationship hostility is not yet consulted (relationships arrive in Stage 1). A failure records a memory for both (negative impact for the pawn who was stood up) and updates affinity under the same diminishing-returns rules.

### 8.7 Action registry, planner and executor \[S0 skeleton, S1 actions, S10 proposals\]

All behavior flows through a **closed action registry**: closed means no registrations after the load phase completes. The same registry later validates LLM proposals. Built-in actions are registered in Rust or by the base pack; pack actions are registered through the script API during the load phase (§23.7).

```rust
struct ActionDef {
    id: ActionId,                        // "move_to", "eat", "sleep", "talk_to", or "<pack>.<name>"
    origin: Origin,                      // Builtin | Pack(PackId)
    params: ParamSchema,                 // typed, with ranges and entity-kind constraints
    preconditions: Vec<Precondition>,    // pure predicates over WorldState (declarative, or a script predicate)
    permissions: Vec<Permission>,        // who may run it (pawn, controller, role, [S9] office)
    steps: Vec<StepTemplate>,            // ordered: MoveTo(point) -> PerformFor(minutes) -> Apply(effects)
    effects: EffectSource,               // declarative Vec<Effect>, or a script fn returning Vec<Effect>
    interruptible: bool,
    cost: Option<CostSpec>,              // time, money [S6]
    ai_proposable: bool,                 // [S10] only if the pack also holds the `ai` capability
}
enum Effect {                            // closed set: scripts can only produce these
    AddNeed { pawn: EntityId, need: NeedId, delta: i32 },
    SetFlag { pawn: EntityId, flag: FlagId, value: i32 },
    SetField { target: EntityId, component: ComponentName, field: FieldId, value: i32 },
    AddMemory { pawn: EntityId, memory: MemorySpec },
    AdjustRelationship { a: EntityId, b: EntityId, delta: i32, reason: ReasonCode },
    MoveItem { item: EntityId, to: ContainerRef },
    SpawnObject { template: TemplateId, at: PlaceRef },
    // ... extended per stage, always typed and validated
}
```

**TaskPlanner** takes the pawn's current intent (a reservation or `Free`), selects the `ActionDef`, resolves params, checks preconditions, and instantiates a `Task` with steps. **ActivitySystem** executes the current step each tick: `MoveTo` delegates to MovementSystem; `PerformFor` counts ticks and applies per-minute effects; `Apply` runs effects atomically through the core's validation (preconditions re-checked, ranges clamped, containment enforced).

**Skeleton status:** the registry, `ActionDef` (typed params, steps, `interruptible`, `ai_proposable`), `TaskPlanner` and `ActivitySystem` exist with the built-ins `move_to`, `idle_at` and `meet_at` and the step kinds `MoveTo { within }` and `PerformUntilSlotEnd`. Preconditions, permissions, costs and the `Effect` set are added with the systems that need them (Needs, Stage 1). A pack registers actions only while the registry is open; namespaced `<pack>.<name>`; a built-in may not contain a dot.

**Failure handling:** any step failure ends the Task with a `ReasonCode` (`BlockedDestination`, `PathBlocked`, `PreconditionFailed`, `Interrupted`, `Unaffordable`, `Unauthorized`, `ScriptError`), writes an event, and triggers `replan_from(current_slot)`.

### 8.8 Reason codes \[S0\]

Every planning, failure and acceptance decision stores a `ReasonCode` plus a small parameter bag (for example `{slot: 14, displaced_by: "res_a1"}`) **and its origin** (`Builtin` or `Pack(id)`). The inspector renders them as short plain sentences and attributes pack-influenced decisions to the pack. They are saved with the current day's schedule and dropped at day rollover, except those attached to memories or events.

### 8.9 Organism system (pawn health) \[S2B\]

A pawn's body is a simulated organism: integer vitals, parts, organs and attributes, stepped by the core. Full rationale, model and open questions: `docs/proposals/organism-system.md`. This section is the contract.

**State.** Per pawn: a `vitals` row (blood volume ml, blood oxygen saturation, heart rate, stroke volume, mean pressure mmHg, breath rate, core temperature in tenths of a degree, total and felt pain, adrenaline, nerve, toxicity, consciousness, brain function, anoxic exposure, run state alive / unconscious / critical / arrest / dead) and sparse `attributes` rows (part, attribute type, severity and type-specific integers, start tick, effect id). Part and organ integrity, oxygenation and efficiency are small fixed arrays per physiology profile. Everything is integer (permille, ml, per-minute rates), canonical, hashed per row (§5.4) and migrated like any table; a pawn with no attributes stores no attribute rows.

**Body and attributes.** A **physiology profile** (data) lists the parts, organs and skeleton, hit weights, and supplies baseline factors derived from the inputs the pawn creator offers (**height, weight, sex, age and fitness**); it carries a species or race field so animals and non-human humanoids can have their own profiles later (humans only now). The base anatomy is hands, arms, chest, stomach, head, legs and feet (plus the mature-rating parts), brain, heart, throat, left and right lung and liver, and a single spine; ordered extensions are eyes and ears, the neck as a part holding the throat, then spine segments; blood volume comes from interpolating anchor points through mass, then the profile factor. An **attribute type** declares its kind (`innate`, `injury`, `condition`, `status`, ...), the parts it may attach to (empty means the whole body), a name and description as string keys with arguments (each with a variant for every filter level and an intensity tier), optional sub-attribute types, and required typed constants through the parameter schema (an injury requires `injury_type`). Static attributes are inert data. Dynamic attributes name a registered **effect**.

**Status Effect Registry (SER).** The registry is built from loaded packs (base first) and then closed. An effect is a declarative program: a cadence in organism steps, an optional condition over vitals and attributes, bounded contributions to named vitals or organs (additive or multiplicative, rates per organism minute), transitions (attach, replace or end attributes at severity thresholds, with `injury_type` branching), and reason keys. The organism ticks every active effect in registry order at its cadence, in one place; attributes never tick themselves.

**The step** (per pawn, fixed order): effects due; haemodynamics (heart rate set by the brain, adrenaline and pressure feedback; stroke volume from blood volume and heart efficiency; pressure from output and vessel tone); respiration (airway patency, lung function and fluid, ambient air); oxygen delivery and organ oxygenation (each organ with its own time constant, brain demand high); organ efficiency, liver clearance, heart autopilot when the brain cannot regulate; temperature; pain (diminishing aggregation, threshold, felt pain = total x (1 - adrenaline factor), temporary pain from harmful actions); consciousness; capacities; run-state transitions. Death is `brain_function` at zero for the irreversible condition (anoxic exposure beyond the limit, which the cold extends, or catastrophic cranial damage); arrest is the resuscitation window before it. Death sets `state = Dead` through the life-cycle path of §16.4. Substances in effects are generic classes with accurate names (antibiotic, opioid analgesic, anticoagulant, antiseptic), never brands or real doses.

**Cost control.** A pawn with no active effect and all vitals inside their homeostatic band is **dormant** (no step; a slow daily baseline only). Pawns with active effects step at the base cadence (one game minute); acute pawns (a vital outside its critical band) step every 2 ticks. Per-pawn work is independent, so steps run on the worker pool and results are applied in pawn-id order (like path batches), identical at any thread count. Budget: under 5 percent of the tick at 200 pawns, measured by the profiler's system probe.

**Time scale.** Rates are authored per organism minute and divided by the world's **clinical time scale** (a per-world setting; see the proposal), so acute events can be made to last game hours instead of real seconds.

**Reasons.** Each vital change records the effect or attribute and the dominant input as a reason code (§8.8), so the health window and `pg organism explain` can say why.

**API (`pg.organism`, API 0.2b).** Capabilities `organism.read`, `organism.define` (load-phase registration of attribute types, injury types and effects; closed after load) and `organism.apply` (request add or remove an attribute, or a treatment, as a validated command). Hooks `organism.onset`, `organism.escalate` and `organism.vital_modifier` run in batches at low cadence with clamped results. A pack can never set death or write a vital; contributions clamp to engine bounds; descriptions need every filter variant and an intensity tier or the pack is rejected; all cost is metered.

**Fight-or-flight.** The psychology core (§16.4, Stage 7) owns the response; the organism provides adrenaline, nerve and a stress input. Before Stage 7 a placeholder reads them (adrenaline dampens pain, a threat raises adrenaline) so systems that need a response have one.

**Tools.** `pg organism sim` (vitals over time with reasons), `pg organism explain <pawn>`, an overlay Organism tab (stepped pawns, cost, SER buckets), developer injure and heal (confirmed, logged as inputs), physiology golden traces in CI and a plausibility lint for pack effects.

## 9. Conversation system \[S1; player side S4\]

### 9.0 As built (1.4)

`ConversationSystem` (slot 9) runs each game minute. **Eligibility:** a resident is free when it is not already in a talk, can act, has `talking` capacity of at least 500, is standing still and either has no task or is performing `socialise` or `idle_at`; a pair may talk when both are free, they are within `talk_range` (Manhattan, same map), at least one wants company (social below `social_below`, or is out socialising) and the pair's cooldown has passed. Pairs are formed greedily in id order, each resident with the nearest other (ties to the lower id), and each pair draws against `chance_permille + affinity / 5` from `social.topic` keyed on (pair, day, minute). **Decided at the start:** the topic (weighted pick among topics whose affinity range and mood conditions hold), the tone (the first tone by priority whose conditions hold: hostile, curt, warm, cheerful, friendly in the base data) and the number of turns. One tone per conversation (not per turn) in Stage 1. The talk is stored on both pawns (`Pawn.talk`, the lower id is the leader), so it is in the saved state and the hash; both stand where they are for `turns * turn_minutes` game minutes. **Closed** by the leader: the outcome is the topic's affinity change scaled by the tone, with a small seeded wobble (`social.outcome`), then the hook, the daily cap and the clamp; both residents gain the data's `social_restore` scaled by the tone; each gets a memory; events `conversation.closed`, `memory.created` and possibly `relationship.label_changed`. **Called off** (`conversation.cancelled`, no outcome) if the two end up further apart than `max_apart`, on different maps, or one cannot act. Fallback lines (`conversation::dialogue`) are chosen from `conversation.json` by topic and tone (the most specific set; hostile and curt have their own), seeded by (pair, start tick), alternating speakers; they are presentation and nothing reads them back. The 8x8-tile spatial hash planned for conversation checks is not used yet: the pair search is a scan over free residents, which is cheap at the sizes in Stage 1.

**As built (1.7, social refinements).** (1) **Personality:** each resident has `outgoing`, -500 to 500 (the average of two seeded draws, so most are near the middle), set when the town is made. It scales the chance a pair starts talking, shortens or lengthens the pair's cooldown (reserved pairs wait up to twice as long, outgoing pairs as little as two thirds), and how much a talk lifts the social need. The inspector shows it as outgoing, balanced or reserved. In practice conversation counts are limited more by who is near whom than by the chance, so the effect is modest. (2) **Per-turn tone:** the first turn has the pair's tone; each later turn reads the tone rules again with the affinity moved by up to `tone_drift`, so a pair near a boundary can warm up or cool off. The outcome follows the mean of the turns' delta and restore factors. (3) **Bands:** a line set may name relationship labels and speaker moods as well as a topic and tone; the best fit wins (tone, then topic, then the bands; the first in the file breaks a tie). The talk keeps the tones, the relationship label and the speakers' moods it was read with (`Talk` and `TalkRecord`), so remembered conversations rebuild exactly the lines that were shown. (4) **Looking back:** a topic marked `needs_shared_memory` can only be chosen when the two remember a conversation together; the line may refer to it with `{memory}` (a short phrase per topic, `memory.phrase.<topic>`), and when the talk closes the remembered conversation regains `rehearse` points of retention for both. (5) **Content hash:** `content_hash` leaves out `data/strings/` (text is presentation), so rewording a label no longer changes the content refs in saves and replays or the golden replay's pin.

### 9.1 Trigger

At each slot boundary, and when two non-busy pawns are within `talk_range` tiles (default 2), the ConversationSystem considers a conversation if all hold: both `Idle` or in a conversation-friendly activity, `social` below a threshold or leisure intent, pair cooldown elapsed, neither in a priority 1–2 reservation. Candidate pairs are sorted deterministically; each is accepted by a `chance(permille)` draw keyed on (pair, day, slot) with weights from affinity and mood (and clamped `conversation.topic_weight` hook results).

### 9.2 States

`approach → greet → exchange (1–N turns) → close`. Participants stand still and face each other. A conversation has a rule-decided `topic`, per-turn `tone`, and a number of turns drawn from a data range.

### 9.3 Outcome versus text

- **Outcome (authoritative):** computed by rules when the conversation closes: `affinity_delta`, `social_restore`, memory records, optional commitment proposal. Inputs: topic, tone, moods, relationship and (later) traits; `conversation.outcome_delta` may adjust within a clamp.
- **Text (presentation):** `Vec<DialogueLine>` for display only. The text never feeds state.

### 9.4 Lazy line generation

Line text is generated **only when someone could hear it** (a focused or possessed pawn within `hear_range`, default 8 tiles). Otherwise the conversation resolves from structured data and stores `summary_key` + topic in memories, with no model call.

1. An audible conversation starts → `DialogueRequest { conversation_id, topic, tones, participants (names, relationship label, moods), context }` goes to the AI client.
2. The client returns lines or falls back. The result is recorded as a `Dialogue` SimInput with its application tick (§5.3).
3. The UI shows bubbles paced by a data-driven reading speed. If lines arrive late, turns already elapsed use fallback lines.

**As built (1.5, 1.6).** `pg-ai::dialogue` builds the prompt (trusted instructions in the system part; names, moods and up to three memory summaries only as cleaned, bounded data in the user part) and checks the reply (exactly one line per turn; refused for a link, over 140 characters, an empty or out-of-character line, or the content filter in cozy worlds and with the device filter on). `pg-runtime::dialogue::DialogueService` acts only when the focused resident is within `hear_range` of a speaker: it prepares the fallback lines at once and, with AI on, asks the client on the worker pool (one request per conversation, a cooldown per resident, at most two in flight, plus the client's rate cap, cache and breaker). A reply replaces only the turns not yet spoken. **History:** the lines that were actually shown (generated text for the turns after the reply arrived, fallback before) are submitted as a logged `RecordDialogue { a, b, started, lines }` input; the core checks it (1 to 8 lines, the right number for the talk, 140 characters, no control characters) and stores the lines on both residents' talk while it is under way, or in both residents' memory of it (`Memory.talk`: start tick, tone, turns, lines) once it has closed. Because it is a logged input the words are part of the saved and replayed state. Fallback lines are not stored: a memory with no recorded words rebuilds them from the data (`conversation::recall`), which is exact because they are a pure function of the talk. Recall returns each line as written text or a string key to fill in. Generated lines never change outcomes: with AI on, off or failing, everything but the words is identical (tested by hash).

### 9.5 Fallback dialogue \[S1\]

A content table keyed by `(topic, tone, relation_label_band, mood_band)` yields line templates with slots (`{name}`, `{place}`, `{time_of_day}`). Selection is seeded (`rand(social.topic, ...)`), so the same state yields the same lines. This keeps the game fully playable without a key. Packs may add topics, tones and fallback tables as data.

### 9.6 Player participation \[S4\]

While possessing, the player starts or joins a conversation through a Command. Player choices map to a **rule-level tone / intent** (friendly, curious, blunt…) that decides the outcome. Free text, if offered, is presentation only and is classified into one of the enumerated tones by deterministic rules (with an optional AI classifier whose result is clamped to the enumerated tones).

## 10. AI integration \[S0 settings, S1 dialogue, S10 proposals\]

### 10.1 Components

```rust
trait AiClient {                         // runs on worker threads; results re-enter as SimInputs
    fn test_connection(&self, cfg: &AiConfig) -> AiResult<ConnectionInfo>;   // credits/tier when the provider reports them
    fn generate_dialogue(&self, req: &DialogueRequest, cfg: &AiConfig) -> AiResult<Vec<DialogueLine>>;
    fn propose_action(&self, req: &ProposalRequest, cfg: &AiConfig) -> AiResult<ActionProposal>;   // [S10]
}
struct AiConfig { provider: Provider /* OpenAi | DeepSeek | Anthropic | OpenRouter | Player2 */, model: String /* key is NOT here; empty for providers that choose the model */ }
type AiResult<T> = Result<AiOk<T>, AiError>;
enum AiError { Auth, RateLimit { retry_after: Option<Duration> }, Quota, Timeout, Offline, BadOutput, Provider(String), Disabled }

trait ProviderAdapter {
    fn id(&self) -> Provider;
    fn recommended_model(&self) -> &str;
    fn build_request(&self, task: &AiTask, cfg: &AiConfig, key: &str) -> HttpRequest;
    fn parse_response(&self, task: &AiTask, body: &str) -> AiResult<serde_json::Value>;
    fn classify_error(&self, status: u16, body: &str) -> AiError;
}
```

Adapters are pure request builders and parsers, testable with recorded fixtures and no network.

**As built.** (a) Adapters are pure and fixture-tested; provider replies are untrusted: size-capped (1 MiB), parsed with serde_json (real providers send floats, which the core's strict parser would refuse; this is the only place outside persistence edges that uses it), reduced to one text field, stripped of control and direction-override characters and length-capped; error messages are redacted and capped. (b) `ProviderAdapter` also has an optional `connection_check` (a free account request) with `parse_connection_check -> ConnectionInfo { credits, tier }`; providers without one test with a tiny generation. (c) The client adds a bounded response cache, a per-minute request cap, one retry on transient failures and a circuit breaker that counts only provider-health failures (timeouts, offline, 5xx, unusable replies), not bad keys, quota, rate limits or cancels. (d) `Provider::chooses_own_model()` providers send no model id and reject a custom model in settings.

**Player2.** Taken from Player2's published OpenAPI document (`https://api.player2.game/v1/openapi.json`): base `https://api.player2.game/v1`; `POST /chat/completions` is OpenAI-style (`messages`, `max_tokens`, `temperature`, `stream`; no `model` field) with `Authorization: Bearer <p2Key>`; `GET /account/joules` returns `{ joules, patron_tier, user_id }` and is the free connection check; `GET /health` is a liveness probe; 401, 402 (insufficient credits), 429 are used for auth, quota and rate limiting. Keys come from the **device-code flow** (`POST /login/device/new { client_id }` returns `deviceCode`, `userCode`, `verificationUri[Complete]`, `expiresIn`, `interval`; `POST /login/device/token { client_id, device_code, grant_type: "urn:ietf:params:oauth:grant-type:device_code" }` returns `{ p2Key }`), or a pasted key. `pg_ai::login` implements the protocol as pure request builders, parsers and a polling schedule (`DeviceLoginSession`) driven by a caller-supplied clock, so it never sleeps; the verification link must be https on `player2.game` or a subdomain, the user code is cleaned and capped, timings are clamped, and the device code is a `Secret`. The Player2 NPC (`/npcs/*`), game-data, text-to-speech and speech-to-text endpoints are **not used**: dialogue stays rule-driven with optional generated lines through `/chat/completions` (suggestion S-033 records the possible later uses). The `client_id` is an open decision (Roadmap §12 item 8).

### 10.2 Key handling

1. The key lives in `SecretStore` (Windows Credential Manager, macOS Keychain, Linux Secret Service) only. Settings and saves hold provider and model id, never the key. Where no credential service exists, the key is session-only and never written to disk in plaintext.
2. The AI client reads the key at call time and calls the provider **directly over HTTPS** (rustls) through the `Net` trait on a worker thread. There is no gateway and no proxy; a desktop binary has no origin restrictions, so the hosted-gateway design from v1.0 is removed. Requests go only to an allow-listed set of provider hosts.
3. A single `redact()` utility scrubs keys from any string before logging. A CI test runs the game with a sentinel key and greps logs, saves, exports and crash reports for it.
4. **Packs never reach the network or the key.** Scripts have no I/O (§23.4). Pack-supplied strings that reach prompts are length-capped, sanitized, treated as untrusted data in the *user* payload, and never placed in the system instruction.

**As built.** `Secret` wraps every key: no `Display`, `Debug` prints `Secret(***)`, best-effort wipe on drop. `HttpRequest`'s `Debug` hides credential headers and query values. `redact()` removes secrets it is told about and anything shaped like a credential (`sk-…`, `AIza…`, `Bearer …`, `x-api-key: …`, `key=…`, `"api_key":"…"`); every log path goes through `RedactingLog`. `AllowListNet` refuses anything but plain `https` to an allow-listed host; the real network layer never follows redirects. Where no OS credential service exists the store is session-only and says so (`is_persistent`). The sentinel-key test scans errors, logs, `Debug` output, storage, saves, exports, replay logs, bug bundles and crash reports, raw and decompressed; `pg ai selfcheck` runs the AI-side scan and `pg check` runs everything.

### 10.3 Reliability

- Per-request timeout (default 8 s), one retry on transient errors, and a **circuit breaker** (opens after N consecutive failures, half-open probe after a delay) so a broken provider never stalls play.
- Cooldowns per conversation and per pawn, a global requests-per-minute cap, and a bounded response cache keyed by a hash of the compact request.
- Any failure returns an `AiError` and the caller uses fallback. Errors show as a non-blocking status (for example 'AI unavailable: invalid key'), never a modal that halts the sim.
- Output validation: dialogue must parse to the expected shape, respect length caps and pass a check keyed on the current filter level before display. Bad output counts as `BadOutput`.

### 10.4 Prompt construction

`build_dialogue_prompt(req)` is a pure function producing a system instruction (setting, tone preset, filter level, output schema) and a compact user payload (names, relationship label, moods, topic, up to 3 selected memories as short summaries, time of day, place). It never includes the full save, other residents' private data, or keys. Tone preset and filter level are given as instructions *and* enforced by output checks.

### 10.5 Proposals \[S10\]

```rust
struct ProposalRequest { pawn_id: EntityId, decision_point: DecisionPointKind,
    compact_context: CompactContext /* needs, mood, task, schedule_window, memories, values */,
    allowed_actions: Vec<AllowedAction { id: ActionId, param_schema: ParamSchema }> }
struct ActionProposal { action_id: ActionId, params: serde_json::Value, rationale: Option<String> /* display only */ }
```

**Validation pipeline** (in core, before any state change): schema-check → action exists in registry → params typed and in range → target entities exist and are reachable → preconditions pass → permissions pass → affordable → not on cooldown. Any failure rejects the proposal with a reason, logged to the developer inspector, and the deterministic planner continues. A valid proposal becomes the same Task the planner would have built. Decision points are explicit enum values (`FreeSlot`, `Conflict`, `SocialOpportunity`…), never 'every tick'. Pack actions appear in `allowed_actions` only if marked `ai_proposable` by a pack holding the `ai` capability.

## 11. Content tone and filtering \[S1 base, S8 full\]

### 11.1 Two independent controls

| Control | Scope | Affects | Never affects |
| --- | --- | --- | --- |
| Tone preset: Cozy / Standard / Mature | Per world (`settings.tone_preset`) | Probability tables for event categories | Outcomes of an event once rolled |
| Graphic-content filter | Global (device setting, default on) | Text detail level and visual intensity | The event record, rules, state |

### 11.2 Mechanism

- **Event rolls:** `EventSystem` uses `chance(permille(category, preset))` from a data table, with the keyed stream `event.<category>`. Changing the preset mid-world affects only future rolls and is itself logged.
- **Event record:** each event stores a canonical, abstract record (`type`, `severity`, `participants`, `outcome_code`). Presentation chooses a **description variant** per filter level (`Full`, `Reduced`, `Minimal`) and a **visual intensity tier**. Variants live in content tables keyed by `outcome_code`, so filtering can never change what happened.
- **Validation:** tests render every event type at each filter level and assert (a) all variants exist and (b) the event record hash is identical regardless of filter. **Pack-registered event types must supply all three variants and an intensity tier, or the pack fails validation.** Packs cannot disable or bypass the filter.
- Visuals are abstract by design (icons, color shifts, stylized effects); there is no graphic asset to unlock.

## 12. World generation and editing \[S1 generator, S3 editor\]

### 12.1 Generator pipeline

```rust
struct GenParams { seed: String, map_size: SizeClass /* or {w,h} */, terrain: Terrain { water: u8, relief: u8 },
                   population: u32, tone_preset: TonePreset }
```

Stages are pure functions `fn(GenContext) -> GenContext` with named RNG streams, each producing a validated intermediate. Packs may register additional stages (§23.7) at declared positions:

1. **Terrain:** integer value-noise heightmap from `worldgen.terrain`; thresholds for water, shore, land; guarantee a connected main landmass (flood fill; regenerate with a sub-seed on failure).
2. **Districts:** place N district seeds (minimum spacing); assign types (residential, commercial core, civic, park, mixed) by balanced quotas, so the mix is internal and not user-exposed.
3. **Main roads:** connect district centers with a spanning set plus a few loops, routed by A\* on a cost grid with jitter from `worldgen.roads`, producing organic curves.
4. **Local blocks:** inside each district, lay a grid aligned to the nearest main road with block sizes from a data range; clip to land; add sidewalks.
5. **Plots:** subdivide blocks into lots by zone (lot width / depth ranges in tiles, \~1 m each).
6. **Buildings:** place footprints and entrance tiles (facing a road); assign roles (home, public). In S1 these are exterior shells only.
7. **Public gathering places:** park, plaza, benches, fountain, placed centrally in districts and along main roads.
8. **Population:** create households from data, assign homes, generate residents (names, appearance, adult ages, occupation archetypes, a few traits).
9. **Social seeding:** starting relationships (household, neighbors, occupation archetype overlap) and a small number of shared memories to give context.
10. **Validation:** see §12.3. On failure, retry with `attempt+1` mixed into stream keys (up to a cap), then report an error.

The generator is deterministic: `(params, content refs)` always yields an identical `WorldState`. Independent regions may be generated in parallel when each uses keyed streams and results are merged in a fixed order. Generation runs as a worker-pool job with a progress report.

**As built (1.2).** `pg-core::worldgen` implements the pipeline as pure functions of `(seed, parameters, game data)` with an attempt number mixed into every stream; the order is terrain, districts, main roads, local streets, buildings, plazas, people, validation, retried up to eight times. Differences from the sketch above: terrain is integer value noise with a quantile water threshold (the requested share of the map, limited by `max_water_percent`), a sandy shore and only the largest landmass kept; districts are zoned by nearest centre (the zone number is stored in the map); main roads join the districts by a spanning tree plus a few loops, routed by A\* over jittered costs with sidewalks beside them; each non-park district gets a street grid; **buildings stand along the streets** (a sidewalk, a yard tile that is the entrance, then a footprint of blocked tiles with a one-tile gap to its neighbours) instead of being cut from lots, with roles drawn from the district's weights, which gives the same density without a separate plot step (plots return with the property system in Stage 6); plazas are paved floor at the centre of the commercial and park districts and are the **gathering places**. Residents come from `plan_population` (§4.7): each household gets a home building and each resident a tile of their own in its yard (open ground only, so a resident asleep never blocks a one-tile passage), and residents whose occupation has duties get a workplace tile in the yard of a non-home building. `Town` (districts, buildings with their entrances, gathering places and the starting hash) is hashed state. Validation (`validate_world`, also run after load and edits) checks that a gathering place exists, buildings are inside the map and do not overlap, and that every entrance, home, workplace and gathering place is reachable from the main plaza over open ground. Generation is the command `GenerateTown { w, h, water, residents, tone }`, so it is in the input log and replays, and it stores the world's **starting hash** (§12.4). The tone preset (`cozy`, `standard`, `mature`) is chosen here and kept in the world settings.

### 12.2 Editor \[S3\]

`TownDesign` is a template-level document (maps, plots, buildings, object placements, optional resident definitions, initial mayor \[S9\]). Editing is a stack of reversible `EditCommand`s (paint terrain, draw road, define plot, place building, place object, copy / paste, delete, generate-region), giving undo / redo. The editor reuses worldgen stage functions for 'generate here'. The editor UI is built with egui over the same `ui-model` state.

Two editing contexts:

- **New-world editor:** edits a `TownDesign`; the result is instantiated into a world on start. Town designs can be shipped in packs.
- **Live observation edits:** the same commands applied to a running world through a validated **mutation API** that (a) preserves stable ids, (b) evicts or relocates occupants of changed tiles, (c) invalidates path caches and replans affected pawns, (d) enforces containment invariants, and (e) logs each edit as a `SimInput`. These are unrestricted creative edits, not budget-limited. Scripts reach this API only with the `world-write` capability and only through validated commands (§23.8).

### 12.3 Town validation

Run before starting a world and after live edits: map bounds; all roads connected; every building has a reachable entrance; plots inside bounds and non-overlapping; required locations exist (a gathering place, housing for every household); every resident has a home; containment invariants hold; no pawn on a blocked tile. Output is a structured `ValidationReport { errors, warnings }`; errors block start.

### 12.4 Starting snapshot

On world creation the core stores `starting_snapshot_hash` (hash of the initial state). Re-generating from the same seed must reproduce it. Later edits and play never alter the stored hash; the editor flags when a world has diverged. This supports the roadmap rule that saving and reloading must not change an already-generated starting state.

## 13. Persistence \[S0\]

### 13.1 Storage layout (through `Storage`, under the OS user-data directory)

```
worlds/<world_id>/manifest.json        small: name, seed, schema, content_refs, both retained generations (state hash,
                                       play ticks, day, saved time) and a summary (population, maps, objects, thumbnail)
worlds/<world_id>/state.<gen>.pgsave   one file per generation: header (magic, schema, uncompressed length, blake3)
                                       + zstd-compressed canonical JSON of WorldState
worlds/<world_id>/thumb.<gen>.png      a small picture of the town, written with the generation
exports/<world>-<time>.pgworld.json    exports (import reads imports/*.json and consumes them)
crash/<time>.pgbundle                  automatic bug bundles; crash/dismissed/ marks the ones already shown
designs/<design_id>.json               saved custom TownDesigns
roster/<pawn_id>.json                  reusable custom pawns [S5]
settings/device.json                   device settings (filter level, audio, window, provider + model; NO keys)
settings/pack_approvals.json           per-pack capability grants keyed by pack id + content hash
mods/<pack_id>/ or mods/<pack_id>.pgpack   installed content packs (the shipped base pack lives beside the executable, read-only)
```

User-data directory: `%APPDATA%\Playground` (Windows), `~/Library/Application Support/Playground` (macOS), `$XDG_DATA_HOME/Playground` or `~/.local/share/Playground` (Linux), resolved from the environment; `--data-dir` overrides it. `<gen>` is a monotonically increasing generation number. The manifest names the current generation; the previous generation is kept as a fallback. A single file per generation replaces v1.0's multi-part snapshots, since a desktop has the memory and disk to write it whole.

**Summaries and settings.** The manifest carries an additive summary (`population`, `maps`, `objects` and the thumbnail blob's name) beside the generation list, so the Saved Worlds screen lists worlds without loading them; older manifests without it still load, and damaged slots still list. A thumbnail is a PNG (stored deflate blocks, no dependency) drawn with the in-game palette and removed with its generation. `SlotStore::delete_world` removes every file under a world. **Settings registry (S-025):** device settings are described by a typed registry built on `ParamSchema` (id, type and range, default, label key `settings.<id>`, scope device or world, restart-required) with cross-setting rules; it validates `settings/device.json` leniently (a bad entry keeps its default and is reported, unknown sections written by a newer build are preserved), drives `pg settings get/set/list` and generates the Options screen (§14.3). Secrets are never settings.

### 13.2 Save procedure (atomic, never destroys the last good save)

1. Snapshot the state in memory (structural clone at a tick boundary on the sim thread; the sim resumes while a worker encodes). If profiling shows the clone is too slow at large sizes, switch to copy-on-write table chunks.
2. Encode to canonical JSON, compress with zstd, compute the checksum.
3. Write generation `g+1` to a temp file in the same directory, `fsync`, then rename into place.
4. Write the manifest the same way, pointing to `g+1` (this is the commit point). On Unix, `fsync` the directory.
5. Delete generation `g-1` only after the manifest commit succeeds. Keep exactly two generations.

A failure at any step leaves the previous manifest and generation intact.

**As built:** (a) The manifest lists **both** retained generations, newest first, each with its own state hash, tick, day and time, so a fallback load can report how much play was lost and can verify the older file against its own hash. (b) Each `.pgsave` is `magic "PGSAVE\0\1"` + schema (u32) + uncompressed length (u64) + BLAKE3 (32 bytes) + one zstd frame; `.pglog` and `.pgbundle` use the same layout with their own magic, so one kind can never be opened as another. Decoding checks the header, caps the declared length, stops decompressing at that length and verifies the checksum last. (c) Compression uses `ruzstd` (pure Rust, no C build); its encoder implements one level ("fastest", roughly zstd level 1), which is enough for a town save; any standard zstd decoder reads the result. (d) A save is the world state at a tick boundary. Inputs queued for **future** ticks are not part of it: the runtime applies player commands at the next tick, so the queue is empty at a save. (e) Crash safety is tested by failing the storage at every operation of a save and requiring that a load afterwards returns the old or the new world, never a third state and never "damaged".

### 13.3 Save triggers

| Trigger | Behavior |
| --- | --- |
| Periodic autosave | Every N real minutes of running time (data setting) |
| Window focus lost / minimized | Immediate save before suspension |
| Close requested | Save, then allow exit |
| Manual save | UI command; shows result |
| Before migration | A copy of the pre-migration generation is kept until the migrated save loads cleanly |
| Before changing the active pack set | Save first; pack changes apply only between worlds |

### 13.4 Load and recovery

1. Read the manifest; if missing or invalid, scan for the newest valid generation.
2. Verify the checksum; if it fails, load the previous generation and tell the player (with the time difference).
3. Validate against the schema; verify `content_refs` against installed packs; run migrations (including pack data migrations, §23.10); run `validate_containment` and the town validator.
3a. Every generation is also checked against the hash the manifest recorded for it, so a file swapped or restored from the wrong place is rejected even when its checksum is valid.
4. If everything fails, mark the slot **damaged**, keep its files, offer export-for-support, and never touch other slots.

### 13.5 Migrations

- `schema` is an integer. Each migration is a pure function `migrate_vN_to_vN+1(raw) -> raw` in an ordered list; running from any older version applies them in sequence.
- Every migration ships with a fixture (old save) and a test asserting the migrated save validates and its derived hash equals a recorded golden.
- Migrations are **additive-first**: add fields with defaults, then deprecate. IDs are never changed by a migration.
- Saves from a **newer** schema than the app knows are refused with a clear message, never partially loaded.
- **Shipped chain:** 3 to 4 (Stage 1): pawns gain occupation, needs, mood, household, memories, capacities, home and workplace tiles and the idle marker; the world gains households, relationships and the town; the settings gain the tone preset. `fixtures/saves/world-v3.json` never changes and its recorded hash is the hash of the migrated world; `world-v4.json` is a generated town that has lived for a day and a half and loads directly. Schema 4 may still grow during Stage 1 (it has not shipped), and the fixtures are refreshed with a decision entry when it does.
- **Pack component data** migrates through pack-supplied pure migration functions (§23.10) run in the sandbox; a failing pack migration refuses the load (offering safe mode), never partially applies.

### 13.6 Export and import \[S0 world, S11 packs\]

```rust
struct ExportBundle { format: String /* "playground-export" */, kind: BundleKind /* World | Roster | Design | Pack */,
                      schema: u32, app_version: String, created_iso: String,
                      content_refs: Vec<ContentRef>, payload: serde_json::Value, hash: Hash }
```

- Export builds the bundle from validated data and strips anything not in the schema. Secrets are impossible to include because they are never in the stored model.
- Import pipeline: size check → parse → schema detect → migrate → validate (structure, references, containment, town rules) → produce a `ValidationReport` and a plan → write as a new slot. Failure leaves existing data untouched. Entity ids keep their values on import (ids only have meaning inside one world), so no remap is needed; the id of the new slot is made unique from the world's name. The app reads export files from the `imports/` folder and deletes each one after a successful import; it writes exports to `exports/`.
- Worlds import as new slots; they do not overwrite unless the player explicitly chooses.
- **Pack archives (`.pgpack`, zip)** are extracted by a hardened reader: reject path traversal (`..`, absolute paths, drive letters, symlinks), cap entry count, per-file and total decompressed size, and compression ratio. Packs are installed only after the manifest validates and the player approves capabilities (§23.2).

## 14. Presentation layer \[S0 shell and menus, S1 world view\]

### 14.1 Three layers

```
 pg-app (thin shell)        window, GPU surface, egui frame, map painting, OS glue
   │  UiEvent in / widget Tree out                  AppEffect out / answers in
 pg-ui-model (pure)         screens as state machines; widget tree; overlay model     <── no I/O, no window
   │  AppEffect                                     UiEvent
 pg-runtime::app (controller)   performs effects against the host traits and the running world
   │  Control / events / RenderSnapshot
 sim thread (pg-sim)        the world
```

- **`pg-ui-model`** contains every decision about what is on screen and what a click, key or window event does. It never draws, reads a file or starts a thread, so every flow is a unit test.
- **`AppController`** (`pg-runtime::app`) turns the model's effects into actions on the real services (storage, credential store, network, clock) and the running world, and answers with events. Because it sits behind the host traits it is tested end to end over in-memory services: create, play, save, quit, relaunch, continue; settings; keys; sign-in; export and import; damaged saves; crash prompt; overlay data.
- **`pg-app`** owns the window and GPU and nothing else: winit for the window and events, `egui-wgpu`'s winit `Painter` for the surface, `egui-winit` for input. It draws the model's widget tree and the map.

The world view consumes `RenderSnapshot`s and the UI owns no simulation state (principle 7).

### 14.2 Events, effects and the controller

```rust
enum UiEvent {                       // into the model
    Key(Key), Click(WidgetId), Text(WidgetId, String), Toggle(..), Choose(..), Slide(..),   // input
    CloseRequested, FocusLost, FocusGained,                                                  // window
    Booted(BootInfo), WorldsListed(..), WorldOpened { name }, Failed(String), Hud(HudInfo),  // answers
    SettingsLoaded(..), AiLoaded(..), LoginPrompt { code, url }, LoginFinished(..),
    ConnectionResult(..), Notice(String), Imported(..), Exported(..),
}
enum AppEffect {                     // out of the model
    ListWorlds, CreateWorld { name, seed, size, residents }, LoadWorld(id), DeleteWorld(id),
    ExportWorld(id), ImportWorld, SaveNow, SetRunning(bool), SetSpeed(String), LeaveWorld,
    WindowFocus(bool), Rewind { tick }, CutBundle,
    SetSetting { id, value }, SelectProvider(id), SetModel { .. }, SetKey { provider, key: Secret },
    ClearKey(id), TestConnection(id), StartLogin(id), CancelLogin,
    OpenUrl(url), RevealPath(name), DismissCrash, Quit,
}
```

Both print redacted: a typed key shows only as a length in `UiEvent` and as `<redacted>` in `AppEffect::SetKey`, whose payload is a `Secret`. Gameplay `Command`s (possess, move, edit, and so on) stay with the core (§5.3); the UI model's effects are application-level requests, and the controller turns the ones that change the world (`SetRunning`, `SetSpeed`, window focus, rewind, save) into run-state events and loop controls for the sim thread.

`AppController::new(AppServices, Option<Arc<ContentSet>>)`. `AppServices` bundles the storage, clock, secret store, the (allow-listed) network, the log, the link opener and file revealer, an injectable sleep for the sign-in worker, the data directory, the app version and the Player2 client id. The controller's surface: `boot()`, `perform(effect) -> Vec<UiEvent>`, `poll() -> Vec<UiEvent>` (background results, the running world's events, and the HUD when it changed), `snapshot()`, `overlay_data()`, `text(key, args)` (the string table in the active locale), `thumbnail_image(name)`, `shutdown()` and developer options such as shadow verification. What it does:

- **Worlds.** Lists worlds from manifests (name, day, residents, play time, packs, thumbnail, damaged flag, newest first). Creates a world (dev town of the chosen size with N residents, saved immediately with a thumbnail), loads one (with the content compatibility check and a notice when a fallback generation was used), opens it as a `Session` on the sim thread paused, closes it with a final save, deletes it, exports it and imports from the `imports/` folder.
- **Settings.** Builds the Options items from the device settings registry, validates and saves each change, and applies the ones that matter live (autosave interval, pause on focus loss).
- **AI.** Provider list with key status (`KeyManager`), model choice (refused for providers that choose their own), key save and clear, a connection test on a worker (credits shown for Player2), and the **Player2 sign-in**: a worker thread starts the device flow, reports the code and link, polls on the provider's schedule until approval, stores the key and reports the result; cancelling stops it at the next poll. Links are opened only if they pass the sign-in code's safety check (https, the provider's domain).
- **Safety.** A bug bundle left by a crash is offered once at launch and then marked dismissed; keys never reach the log, storage (apart from the credential store), snapshots or the model's state.

### 14.3 The model, the widget tree and the screens

**Screens** (`pg-ui-model::app`): `Boot`, `CrashPrompt`, `MainMenu`, `NewWorld`, `SavedWorlds`, `Options`, `AiOptions`, `InGame`, `Pause`; later screens (inspector, town creation, mods, editor) extend the same machine. The model keeps a stack, so Options opened from the main menu or from the pause menu both return where they came from.

- **Main menu:** Continue (the newest world that is not damaged; disabled when there is none), New world, Saved worlds, Options, Quit.
- **New world:** name, seed (empty means random), map size (small 48x36, medium 64x48, large 96x72), residents (0 to 50, default 14), water (percent of the map, 0 to 50, default 18) and the content tone (Cozy, Standard by default, Mature), with validation. Creating a world generates the town (§12.1) and shows why if it cannot.
- **Saved worlds:** one row per world (name, day, residents, a damaged marker); the selected row shows its picture and details; Load, Export, Delete (with a confirmation that Escape cancels) and Import.
- **Options:** **generated** from the settings registry, grouped by the first part of the setting id: a toggle for a boolean, a slider for a range, a choice for an enumeration, a text field for text, a note on restart-required settings. AI settings have their own screen.
- **AI options:** provider choice; for pasted-key providers a secret field with save and remove; for Player2 the sign-in flow (explanation, code and link, open in browser, waiting, cancel, failure with retry, signed in with sign out); the model field or a note that the provider chooses; a connection test with its result. The game states that it plays fully without AI.
- **In game:** the screen is laid out in three places (`HudParts`: `top`, `controls`, `dialog`). A thin **top bar** shows world name, day and time, residents and "Paused" when stopped. The **simulation controls** sit in the **bottom-right corner**: Pause or Play, one **speed button** that opens a drawer listing 1x, 3x, 9x and 27x (the button shows the current speed), and Menu. There is **no save button in the game view**: saving is in the pause menu only (Save now, and the two Save-and-leave buttons; autosave and the focus-loss save continue as before). When the world suspended itself because the window lost focus, a **notice window centred on the screen** says so and offers Resume; it takes keyboard focus when it appears, so Enter resumes. Space pauses and resumes (unless a drawer is open, where it picks the focused item); Escape closes an open drawer first and otherwise opens the **pause menu** (Resume, Save now, Options, Save and go to main menu, Save and quit), which pauses a running world and resumes it when closed; the controls stay visible but inert behind it.
- **Window events.** Closing saves first. Losing focus (if the Options toggle "Pause and save when the window loses focus", setting `time.pause_on_focus_loss`, is on; it is on by default) suspends the world and saves; regaining it changes nothing until the player resumes.
- **Developer mode** (Options, group Developer, setting `dev.enabled`, off by default): F3 and the overlay (§14.5) exist only while it is on, and turning it off closes the overlay. The Developer group is always listed last in Options.
- **Notices** (failures, confirmations) appear above the current screen until the next input and never change the screen.

**The widget tree** (`pg-ui-model::widget`): `Heading`, `Label`, `Note`, `Spacer`, `Button`, `TextField` (with a secret flag), `Toggle`, `Choice`, `Slider`, `Progress`, `Thumbnail`, `Row`, `Group` and `Drawer`. Every interactive widget has a stable id (`main.new`, `ai.key`, `setting.ui.scale_percent`) that is never translated. **Drawers** are a button that opens a panel of choices, either a **list** (one scrollable column, as wide as its longest entry, at most `max_rows` visible) or a **grid** (`GridSpec`: `max_cols`, `max_rows`, cell size, gap; it is as many columns as there are items up to the maximum and as many rows as needed up to the maximum, and scrolls beyond that), with an `Align` (start, centre or end) across the unroll direction. The model keeps which drawer is open (at most one; leaving a screen closes it) and its items join the focus order after the button, so opening puts focus on the selected item, arrows move through the items (Up and Down jump a row in a grid) and stop at the ends, Enter or Space picks, and Escape closes and returns focus to the button. **Placement is pure geometry in `pg-ui-model::layout`** (tested without a window): `unroll_dir` opens the panel **opposite the nearest screen border** (ties go to the vertical axis, so a corner button opens upward), and `place` puts the panel `gap` from the button, aligned as asked, kept `margin` inside every border and **cut down to the room that exists**, which makes it scroll. All sizes are in egui points, i.e. after the interface scale is applied, so at a larger scale the same window holds fewer points and a drawer that no longer fits scrolls instead of leaving the screen. `pg-app::ui::draw_drawer` measures the longest entry with the real font, asks `place` for the rectangle, draws the panel in a foreground area with a scroll area, and reports a press outside the button and panel as a toggle, which closes it.

A `Tree` can list its **focus order** and render a **text snapshot** (secrets as dots), which is what snapshot tests compare and what a screen-reader layer will map (S-038).

**Keyboard-first navigation** lives in the model, not the view: Tab, Shift+Tab, Up and Down move focus through the focus order (wrapping), Left and Right change the focused choice or slider, Enter or Space activate the focused widget, Escape goes back (and in the world opens the pause menu), F3 toggles the overlay. Focus always lands on a real widget. A test explores **every reachable screen state** (activating each widget in turn, answering effects as the app would) and asserts that each has focusable widgets with unique ids, that Tab visits every one, and that the keyboard alone leads back to the main menu.

**String tables (S-026).** All player-visible text is a key into `strings/<locale>.json` in the base pack (pack strings are namespaced `<pack>.<key>`); screens receive a translator and never contain literal text, setting labels are `settings.<id>` and enumeration options `settings.<id>.<value>`. The reason-code sentences live in the same table. Lints and tests report missing and unused keys, a virtual `pseudo` locale (accented, 40 percent longer) is always available (Options, language), and a test visits every screen asserting that each key it asks for exists.

### 14.4 The in-game view

The view paints the first map of the latest snapshot into the central area through egui's painter. `pg-render` provides the geometry as plain, tested arithmetic: a `Camera` (centre in tiles and pixels per tile; fit, pan, zoom around the cursor, clamp) and `tile_rects`, which turns the visible tiles into rectangles and **merges runs of equal colour along a row** so open grass costs a handful of shapes. Colours come from `pg_ui_model::palette` (terrain, surface, blocked tiles, a colour per pawn id), the same palette the save thumbnails use. Pawns are circles with names when zoomed in and a tooltip with the current activity on hover. Drag pans, the wheel zooms, and until the player moves the camera it re-fits the map as the window changes. The wgpu surface is managed by `egui-wgpu`; the dedicated tile and sprite renderer with atlases, chunk caches, interpolation between ticks and integer-scaled pixel art is **Stage 1** work, as are bubbles, focus selection and the inspector.

**As built (1.6).** `pg-render` holds four pure parts, all unit tested without a window, and one thin GPU part. (1) `sprite`: every picture is drawn in code (16 by 16 pixels): terrain looks per tile kind (grass, water, sand, road, sidewalk, floor, brick wall with and without a window) chosen per tile position, and residents with a look derived from the id (shirt, skin, hair), four facings and two walking frames, plus a shadow and a selection ring; real assets can later replace them behind the same `SpriteKey`s. (2) `atlas`: a shelf packer with padding, one 512-wide texture for everything. (3) `scene`: `build_scene` turns a camera, a `Terrain` and the residents into a list of textured quads (visible range only, ground then residents sorted back to front, positions rounded to whole pixels at whole zoom so tiles meet exactly); the zoom snaps to whole multiples of the art size above 16 pixels a tile (integer-scaled pixel art); `pick_pawn` finds the resident under a click; a golden hash of a fixed draw list is pinned in a test. (4) `interp`: a resident that steps to a new tile glides there over the step time, from the snapshot's tick plus the observed ticks per second (never more than a few ticks ahead, standing still when paused). (5) `gpu`: `GpuScene` uploads the atlas and draws all quads in one instanced draw call with nearest-neighbour sampling; the app registers it in egui-wgpu's callback resources and the map view adds a paint callback for its rectangle, so the map is drawn inside the egui frame (names and hover notes are egui text on top). The earlier painter-based drawing remains for the headless smoke test and as a fallback when no GPU layer is registered. Clicking a resident selects it and tells the dialogue service (`set_dialogue_focus`), which is what makes conversations near it audible; the developer-mode HUD line shows quads, tiles in view and draw calls.

### 14.5 The developer overlay (F3)

Available only in developer mode (§14.3). A floating window over any screen with tabs: **Time** (tick, day, run state, the latest keyframe's state hash, shadow-verification status, the keyframe ring as buttons that **rewind** the world, a **bug bundle** button), **Systems** (per-system calls, average microseconds and share, from the profiler probe), **Events** (the recent events with the S-009 kind-prefix filter), **Reasons** (each pawn's latest planning failure explained from the string table), **Packs** (loaded packs and whether they run scripts) and **Scripts** (calls, fuel and errors per pack and handler: the script cost view, S-036). The model owns the tab and filter; the controller fills the data from the snapshot, the profiler and the script meter once per frame while it is visible. Developer aids on the binary: `--smoke`, `--demo <screen>` (new, options, ai, saved, game, drawer, focus, pause, overlay) and `--shadow <n>` (§20).

### 14.6 The window shell

`pg-app` creates a 1280x800 window (minimum 800x560), a wgpu surface through `egui-wgpu`'s `Painter` (vsync on unless `--no-vsync`) and an `egui_winit::State`. Each redraw: take egui input, run the app's frame, tessellate and paint. The app's frame polls the controller, turns key presses into model keys (arrows, Space and Enter are left to a focused text field), asks the model for the current tree, draws it, and sends back the events the player produced. UI scale (a setting) sets egui's zoom factor and the window mode (windowed, borderless, fullscreen) is applied live. Closing the window dispatches `CloseRequested`, waits for the save, and exits.

A **secret text field** keeps the real text only in egui's temporary memory and reports it to the model, which holds it as a `Secret`; the tree the model produces contains dots. The same code is exercised headless: `pg-app --smoke` runs a scripted session (launch, options, new world, play at 27x, overlay, pause menu, save, back to the menu, continue, close) on real files and a real clock under a headless egui context, tessellating every frame; unit tests click buttons and type into fields through egui's input events. CI runs the smoke test on all three systems; the window itself is not opened there.

### 14.7 Input mapping and accessibility

Raw events become model events in two places: key presses in `pg-app::ui::collect_keys` and pointer interaction through the widgets; all bindings for the menus are fixed in 0.10, and rebindable controls, high-DPI handling and the world-view hotkeys are Stage 1 work. Every action in the menus is reachable by keyboard (tested). UI scale and window mode are settings. **Screen-reader output** (egui's AccessKit integration, which the widget tree already supports with names, roles and focus order) is scheduled for Stage 1 enabling and testing (S-038), with the full pass in Stage 11.

### 14.8 The developer console \[S1, milestone 1.3a\]

A window of log lines for developers, opened with the backtick key (`` ` ``) **only while developer mode is on** (Options, Developer), on any screen; Escape or the Close button closes it, and turning developer mode off closes it too. `pg-host::console` holds the pieces: `Severity` (**Debug** for routine detail that is useful to filter out, **Info** for what the game and script packs print on purpose, **Warn** for something that went wrong and was recovered from, **Error** for a failed operation, **Fatal** for a world that cannot carry on, such as a panicked tick), `Entry` (sequence number, severity, source, tick, text), a bounded shared `Console` (the newest 5,000 entries; every line is redacted and clipped on the way in) and `ConsoleLog`, a `LogSink` wrapper so everything the app and the simulation loop already log also appears. Debug entries are only produced while developer mode is on, so nothing pays for noise nobody can see.

**Coverage comes from the event catalog.** Every built-in event kind has a console severity listed in `pg-core::events::SEVERITIES`; a test fails until a new kind is listed and until no stale kind remains, so a change to the simulation cannot add an event the console does not show or shows at an unconsidered level. The sim thread sends each tick's events through `EventCatalog::console_line` (kind and fields, with an explanation in place of a machine-readable reason), the controller adds app messages (worlds opened, saves, settings changes, every failure the player is told about as an Error) and the crash path writes a Fatal. **Script packs** print with `pg.log.info` and `pg.log.warn`; each printed line becomes a `script.log` event (pack, level, text) and shows as Info or Warn; prints made while a pack loads are not captured yet. Packs' own event kinds show as Info.

**The window** (`pg-ui-model::console`, drawn by `pg-app`): a text box at the top keeps only entries whose displayed line (prefix included, case ignored) contains the text; one button per severity switches that type on or off, **all on except Debug, and put back to that every time a world is loaded**; Clear empties the log; the lines follow the newest. Each line is prefixed with its type (`[Error]: text`) and coloured: Debug blue, Info white, Warn yellow, Error red, Fatal black on a red background. The model is pure state, fed new entries by sequence number, so a different front end (a terminal, the future inspector) can reuse it. The command-line twin is `pg sim --console [LEVEL] [--filter TEXT]`, which prints the same lines for a headless run.

**Extending it.** A new system reports through its events (declare the kind, give it a severity in `SEVERITIES`); the application logs through the usual `LogSink`; a new front end reads `Console::since(seq)`.

### 14.9 The town journal \[S1, milestone 1.7\]

A short list of what happened in the town that is worth noticing, newest first, so a town can be followed without watching one resident. `pg-core::journal` writes the journal as the simulation's events are emitted (`TickCtx::emit` calls `journal::record`), so it is **part of the world state** (a `journal` table: saved, hashed, replayed identically, 120 entries at most): a relationship changing label, a resident collapsing or recovering, a need becoming critical, and a conversation that moved a pair by at least +50 or -40. An entry holds the tick, a kind, a weight (1 routine, 2 notable, 3 serious) and arguments (names are taken when the event happens); the sentence is the string `journal.<kind>` filled in when the window is drawn, and an argument named `x_key` holds a string key that is looked up first. The loop publishes the world's journal in each snapshot. The window (`pg-ui-model::journal::tree`) opens from the HUD's Journal button or the J key and closes with its Close button. Entries hold names, not ids, so they stay readable when a resident is gone. **Modes:** it belongs to observation mode (which includes possession) and not to player mode, which does not exist yet; `journal::available(mode)` is the one place the rule lives, and the model's `mode` is always observation until player mode is built (Stage 4).

## 15. Possession and control \[S4\]

- `pawn.controller` is `Sim` or `Player`. Possessing sets `Player`; releasing sets `Sim`. Exactly zero or one pawn is player-controlled.
- **While possessed:** the pawn's needs, memories and relationships still run. The pawn's own schedule is **suspended** (no task activation); the player issues `Move` / `StartConversation` commands that become Tasks through the same ActionRegistry (so preconditions apply).
- **On release:** `replan_from(now)` rebuilds the remaining day. Accepted commitments that fall in the possessed span and were not honored resolve as `Failed` under normal rules (with their memory and relationship effects), so autonomous life continues consistently and no special-case exists.
- Other pawns treat a possessed pawn exactly like any other pawn; they cannot tell they are interacting with the player.
- Scripts can read `controller` but cannot change it or issue player commands.
- **Custom pawn \[S5\]:** `PawnDef` is validated, instantiated through the same entity factory as generated residents, and can be placed (start tile, household / social context). Roster entries are templates; placing creates a new entity with a fresh id. Packs may supply occupations, traits and appearance parts as data.

## 16. Later modules (Stages 6–9)

Built on the same patterns: data templates, tables in `WorldState`, systems in the tick pipeline, actions in the registry, deterministic rules, reason codes. Each module defines its script extension points when it is designed (§23.8 lists the planned exposure by stage).

### 16.1 Interiors and buildings \[S6\]

- `InteriorLayout` templates are hand-authored (rooms, zones, object placements, entrances) and selected by building type and size. An interior `MapData` is instantiated from the layout on first entry, then persisted. Cross-map pathing uses portals (§7.3). Packs may ship layouts as data.
- Zones group rooms (a home: house, garage, backyard, basement), with bedroom count derived from building size.

### 16.2 Property \[S6\]

```rust
struct Plot { id: EntityId, map: EntityId, x: i32, y: i32, w: i32, h: i32, area_ft2: i32,
              zoning: Zoning /* Residential | Commercial | Special */, owner: OwnerRef /* includes City */,
              market: Market /* Owned | Leased | Foreclosed | ForSale */, building_id: Option<EntityId> }
struct Building { id: EntityId, plot_id: EntityId, kind: BuildingKind /* Home | Business | Public | Civic | Other */,
                  template_id: TemplateId, interior_map_id: Option<EntityId>, entrance: Portal }
```

City-owned plots (town hall, library, clinic, school, police / fire) are flagged non-purchasable. Ownership changes are single atomic transactions (title + payment) in the ledger.

### 16.3 Economy \[S6\]

- **Ledger:** append-only `Transaction { id, tick, from, to, amount, reason, refs }` with integer minor currency units; balances are `Account` rows that always equal the sum of the ledger (checked in tests and on load). Scripts can request transactions only through validated `Effect`s; they can never edit balances directly.
- **Jobs:** an occupation template gains `workplace`, `shift_rules`, `wage`, `skills`. `work_shift` is an ActionDef (priority 1); wages post at shift end.
- **Goods and shops:** items are objects in container inventories; shops hold stock containers with prices; `buy` / `sell` actions move items and money atomically. Needs connect to goods through `needs_restore` components.
- **Rent / housing:** recurring `Obligation` rows (due tick, amount, payee); unpaid obligations trigger deterministic consequences from a data table.
- Start with one job, one shop and rent; extend after that loop is stable.

### 16.4 Social, psychology, life cycle \[S7\]

- **Relationships:** extend `Relationship` with trust, fear, conflict and an archetype state machine (friend → close friend → best friend; romance and family states unlock after friendship tests pass).
- **Personality:** `traits` plus numeric sliders (for example taciturn ↔ outspoken) stored as integers; they bias weights (conversation chance, tone choice, leisure picks) and never override rules.
- **Significant experiences:** only memories with importance above a threshold may adjust stable traits, by small bounded steps with a per-year cap. Minor events never do.
- **Conditions:** modeled as data-defined `Condition` records (onset triggers, severity, effects on needs/mood weights, recovery paths, care actions) authored with research-informed abstractions. Conditions are never inputs to violence probabilities. Content review may precede implementation (see roadmap tuning list).
- **Life cycle:** `birth_day` and an aging-speed multiplier convert game days to age; stages (child, adult, elder) change schedules and permissions; births and move-ins are separate per-world switches under a population cap; death sets `state = Dead`, releases reservations, logs a non-graphic record, and the pawn becomes a historical entity (id never reused).

### 16.5 Events, crime, injury and services \[S8\]

- **EventSystem** rolls categories using the tone preset (§11). Crime and injury are resolved by deterministic rule tables (location, opportunity, relationship, conditions, nearby services) with a `rand` draw per decision.
- **Injury model:** injuries are attributes on the organism's parts (§8.9) with an `injury_type`; the event system chooses a cause, a part (by hit weights) and a severity from deterministic tables, and the organism produces the consequences. Presentation is abstract and filter-aware; no gore data exists in content, and the record is identical at every filter level. Treatment (first aid, resuscitation, transfusion, surgery as scoped) is a set of actions that request attribute changes through `organism.apply`.
- **Law and consequences:** an `Offense` record yields a deterministic outcome table (warning, fine, detention, restitution) from service coverage and presence.
- **Services:** `Service` entities (clinic, emergency response, later others) with staffing, coverage radius over the road graph, response time, and cost. Response is computed on the road graph deterministically.

### 16.6 Governance \[S9\]

- **Election:** every `election_interval_days` (tuned), candidates are residents who opt in by data rules. Each resident's vote is a deterministic function of reputation (aggregated relationship and memory signals) and priorities, with seeded tie-breaks; results and rationales are recorded.
- **Office:** `mayor` is an occupation with a role permission set. The mayor's authority comes through **permissions on ActionDefs** (for example `approve_construction`), never through a separate game mode.
- **Budget:** property and business taxes accrue to a town account; construction and services spend from it. A construction request is an action that validates budget, zoning, rules and legal constraints, then spawns a build job. Editor / observation edits bypass this by design.

## 17. Validation and error handling

| Boundary | Check | On failure |
| --- | --- | --- |
| Content / pack load | Manifest, API version, capabilities, schema, inheritance cycles, references, component params, script compile, registrations | Refuse the pack; show report; built-ins are tested in CI |
| Script runtime | Fuel, memory, value-boundary conversion, command validation, panics-as-errors | Discard that invocation's commands; record a reason; quarantine the pack after repeated failures (§23.6) |
| Generation | Town validator (§12.3) | Retry sub-seed, then error |
| Save load | Checksum, schema, `content_refs`, migrations, invariants | Fallback generation, else damaged slot |
| Import | Size, parse, schema, references, remap, archive safety | Reject with report; nothing written |
| Command | Preconditions, permissions | Reject; UI notice; no state change |
| AI output | Shape, length, filter-level check | Treat as failure; fallback |
| Proposal | Registry validation pipeline | Reject; deterministic planner continues |
| Core invariants | Containment, ledger sums, occupancy, id uniqueness | Debug builds assert; release builds log, quarantine the offending entity and continue where safe |

**Error policy:** the sim must not crash on bad data. A pawn or object that violates invariants is quarantined (removed from systems, flagged in the inspector) and the world continues. Core code returns `Result` rather than panicking on data-derived conditions. As a last resort, the sim thread wraps each tick in `catch_unwind`: a panic aborts the tick, stops the sim, reports a stable error code, and offers to restore the in-memory snapshot taken at the last day boundary or the last autosave; a half-applied tick is never continued. User-facing messages are plain language with a stable error code for support.

## 18. Testing strategy

Tooling: `cargo nextest`, `proptest` (property tests), `insta` (snapshot tests), `criterion` (benchmarks), `cargo-fuzz` (parsers and boundaries), `cargo clippy` with the project's deny lists, `cargo deny` (licenses and advisories).

1. **Unit tests** for pure functions: rand, hashing test vectors, fixed-point math, template resolution, containment, A\*, needs, mood, memory scoring, scheduling steps, reservation conflicts, label hysteresis.
2. **Property tests** (generated inputs): schedules never overlap; a lower-priority number never loses to a higher number; commitments are reserved for both or neither; containment invariants hold after random edit sequences; ledger always balances.
3. **Golden replays:** for each fixture seed, run N game days headless and compare per-day state hashes with recorded goldens. Variants: **every supported OS and CPU architecture in the CI matrix** (Windows x64, Linux x64, macOS arm64 at minimum) must produce identical hashes; different worker-thread counts must produce identical hashes; replay from a saved snapshot mid-run must match an uninterrupted run.
4. **Save / migration tests:** every historical save fixture migrates and validates; corruption injection (truncate, flip a byte, missing file) must recover or mark damaged without touching other slots.
5. **Generator tests:** a seed matrix (sizes × water × population) all validate; connectivity and entrance reachability asserted; invalid parameters rejected.
6. **AI contract tests:** recorded provider fixtures per adapter; malformed, partial, oversized and hostile outputs; timeout / offline / auth / rate-limit paths; sentinel-key leakage test across logs, saves, exports and crash reports.
7. **Presentation model tests:** UI state machines, snapshot builder output for known states, bubble hearing-range logic.
8. **Filter tests:** every event type (including pack-registered ones) at every filter level; event-record hash independent of filter.
9. **Soak tests:** headless run of 30+ game days with the resident target count; assert no growth beyond bounds (memories, events, caches, VM memory), stable tick time.
10. **Acceptance tests per gate:** each gate's 'Done when' is a scripted scenario that runs on a saved-and-reloaded world.
11. **Sandbox conformance suite (scripts):** a corpus of hostile packs that attempt escape (`getfenv`, `loadstring`, `require` traversal, `debug`, metatable and builtin tampering), resource abuse (infinite loop, deep recursion, memory bomb, huge `string.rep`, pathological patterns), nondeterminism (`pairs` order, `tostring` of tables, floats at the boundary, weak tables) and invalid output. Each must be contained or rejected, with the world and other packs unaffected.
12. **Script determinism tests:** fixture packs under golden replay; a **VM-reload variant** that rebuilds all VMs at each day boundary and must produce identical hashes (this catches hidden mutable closure state, §23.5); fuel accounting identical across OS and architecture.
13. **Fuzzing:** pack manifest parser, archive reader, Luau-value-to-command marshalling, import pipeline, save loader.
14. **Independent canonicalization check \[S0, milestone 0.6\]:** a property test that `Canon::to_canonical_string` equals RFC 8785 (JCS) output, from a third-party implementation used as a dev-dependency, for integer-only documents, so a bug in our own escaping or ordering cannot hide behind our own vectors.
15. **Scenario files \[S0, format at 0.6, gate scenarios at 0.11\]:** each gate's 'Done when' (and the 30+ day soak with its invariants: bounded state and daily script cost, no growth in population or input log, shadow-verified days) is a JSON scenario (seed, packs, scripted inputs, steps such as save and reload, assertions) run by `pg scenario run`, so acceptance tests read like the roadmap. The Stage 0 gate report is `docs/gates/stage-0.md`.

**Suites as built (through 0.10).** Unit and property tests in every crate; pinned determinism vectors (`pg selftest`); golden replays across the OS matrix and with different thread counts; mid-run snapshot and resume equivalence; save fault injection at every storage operation; the AI contract suite and a **sentinel-key leak test** across logs, saves, exports, crash reports, UI snapshots and Debug output (`pg ai selfcheck`, `tools/pg-cli/tests`, the controller flow tests); the **hostile-pack corpus** and the capability-surface checks (`pg-script/src/tests.rs`); the cookbook end-to-end tests with the VM-reload variant and safe mode (`pg-script/tests`, `pg-runtime/tests/scripts.rs`, `pg pack test`); UI model flow, snapshot and keyboard-reachability tests (`pg-ui-model`); whole-session app flows over in-memory services (`pg-runtime/tests/app_flows.rs`); the egui interaction tests and `pg-app --smoke`; and `pg check`, which runs the developer checks in one report. `cargo-fuzz` targets are deferred to Stage 11 hardening (proptest covers the parsers meanwhile; D-011).

## 19. Performance plan

- **Targets (initial, refined by profiling on the minimum-spec machine):** simulation step at 200 residents averages under 4 ms and never exceeds 12 ms on the reference machine (at 50 residents, well under 1 ms); script execution is capped at a fixed share of the tick budget (tuned); render frame budget 6.9 ms (144 Hz) to 16.6 ms (60 Hz) with a stable frame time; autosave encode never blocks the sim or render threads; cold load of a mature save in about a second or two on SSD.
- **Threads and layout:** the sim thread and UI thread never share mutable state. Hot per-pawn data uses a data-oriented layout (structure-of-arrays where profiling shows benefit); typed `Vec`s for tiles; no allocation in per-tick hot paths (scratch arenas such as bump allocators reset each tick); spatial hash of pawns by 8×8 tile cells for conversation checks.
- **Algorithmic safeguards:** per-minute systems iterate pawns in id order but skip pawns whose inputs did not change (dirty flags); the TaskPlanner runs only for pawns needing a task; path requests are cached, capped and batched in parallel; deterministic data-parallel maps (§6.2, §7.3) are used only where golden replays confirm identical hashes at any thread count.
- **Rendering:** wgpu instanced sprite batches and cached tile-chunk textures; a sprite atlas; only the viewport plus margin is submitted; the renderer runs from the latest snapshot and never waits on the sim.
- **Scripts:** one VM per pack, interpreter-only (no Luau native code generation until it is proven bit-identical under golden replays); per-system queries are batched so the Rust↔Luau boundary is crossed per system or per batch rather than per field access; compiled bytecode is cached by source hash in memory (never loaded from a pack).
- **Memory:** bounded memories, events (ring buffer + daily summary archive) and caches; per-pack VM memory caps.
- **Path search scratch buffers (conditional):** search keeps scores in a sparse `BTreeMap` (about 4 µs per node expanded). The 0.8 profiling pass showed paths are not a cost at 200 pawns, so this stays as is; if a later profile shows paths matter, replace it with a reusable dense array stamped with a search generation, one per worker thread. Results must stay bit-identical (the Dijkstra property test and the threaded-vs-serial checks guard this).
- **Optional unfocused fast-path:** for pawns far from the camera, per-tick stepping may be replaced by computing arrival ticks from path length, applied only if golden replays prove identical outcomes (including occupancy conflicts); otherwise keep full stepping and lower the resident cap.
- **Profiling:** the core stays clockless, so timing lives outside it: a `SystemProbe` trait in the pipeline is implemented by the runtime's `Profiler` (per-system calls, total and maximum time), and a `ScriptMeter` records calls, fuel and errors per pack and handler. `pg profile` prints the table headless, `pg pack bench` measures script cost, and the overlay shows both live. Measured in 0.8: 200 pawns on a 96x72 town cost about 0.10 ms per tick (movement 70 percent); scripts with the three cookbook packs add about 1.6 ms per tick at 200 pawns and 12 µs per handler call (0.9). Acceptance thresholds are set from measurements on representative machines (including a low-end integrated-GPU laptop).

## 20. Developer tools \[S0 from 0.4, completed S11\]

Developer tools are registered once in `pg-runtime::devtools` (id, title, category, command-line form, whether the overlay has a panel), so a tool cannot exist in the overlay without a command-line twin; `pg tools` prints the registry and the overlay (§14.5) is built on it. Destructive tools require developer mode and confirmation, and anything that changes the world is a logged input so replays stay faithful.

| Group | Tools | Where |
| --- | --- | --- |
| Reproducibility | Replay verify, **diff** (first differing day and table) and **bisect** (first differing tick, naming the differing rows); **bug bundles** (snapshot, input log, content refs, hash trail; replayable headlessly; written automatically on a crash or divergence and on demand); **scenario runner**; trimmable, compressed replay logs; the **VM-reload** and **shadow verification** variants | `pg replay`, `pg bugbundle`, `pg scenario`, `pg run --shadow`, overlay Time |
| Time | Keyframe ring and **time scrub**; run, scrub and autosave from a headless session | `pg run [--scrub]`, overlay Time |
| Explainability | Event viewer with kind-prefix and tick-range filters; the typed **event catalog**; **reason-code explorer** ("why did pawn_1a skip lunch?") | `pg sim --events`, `pg events`, `pg schedule explain`, overlay Events and Reasons |
| Performance | Per-system tick profiler; script cost per pack and handler; path benchmarks | `pg profile`, `pg pack bench`, `pg map bench-paths`, overlay Systems and Scripts |
| Persistence | Save inspector, verify, load, export, import; compatibility report | `pg save ...`, `pg content diff` |
| Configuration | The settings registry (list, get, set, reset), string tables (lint, show, pseudo) | `pg settings`, `pg strings` |
| Content and packs | Content lint, list, resolve, components, hash, JSON Schema export; pack lint (API names, capabilities, order-sensitive loops, module-level state), pack test under golden hashes, generated API reference and `pg.d.luau`, scaffolding; "did you mean" hints everywhere | `pg content ...`, `pg pack lint\|test\|docs\|new\|bench` |
| AI | Provider list, key status, settings, a dry run that prints the exact request (with the credential hidden), a leak self-check | `pg ai ...` |
| Residents and towns | The population a seed produces (households, occupations, relationships, memories); a generated town drawn as text with its statistics and a pinned starting hash; needs, mood and capacities at the end of a run; the content inheritance tree; the actions with what each restores; re-recording a golden replay | `pg residents generate`, `pg worldgen preview`, `pg sim --needs` and `--town`, `pg content tree`, `pg actions`, `pg replay --rerecord` |
| Console | The developer console: every message at a severity with a text filter and a switch per type | `pg sim --console [LEVEL] [--filter TEXT]`, overlay: backtick |
| One report | The checks above that need no setup, in one table; CI runs the same | `pg check` |
| App | Scripted headless session; open on a screen; shadow verification | `pg-app --smoke`, `--demo <screen>`, `--shadow <n>` |

Later (Stage 11): spawn and destroy, pawn editing, schedule viewer, route inspector, proposal inspector with secrets redacted \[S10\], a script console under the `dev` capability, and hot reload of a pack at a tick boundary (recorded as a `ScriptReload` input).

**Formats.** A replay log (`playground-replay`, version 4) holds every applied input, per-table and per-row day hashes and the content refs, and may carry a **start state** (the world as of tick *T* plus the inputs still queued then): `ReplayLog::trim(T)` makes one, replaying it reaches the same hashes as the full log, and untrimmed logs are unchanged. A bug bundle (`.pgbundle`) is a trimmed log plus a note, the app version and the time in the compressed container; `pg bugbundle run` replays it anywhere. Scenario files (`playground-scenario`, version 1) drive build, run, save, reload, damage, recover, snapshot-fork, export and import, migrate (a pinned old save through the migration chain), soak (N days with every day re-simulated on another thread count, bounded state and script cost, unchanged population and input log) and assert steps (counts, tick, day and a pinned state-hash prefix that holds across operating systems); the world section may name content packs. The runner takes a `ScenarioEnv` for what a step needs from the machine, and the runtime supplies one with content, scripts and thread counts. CI runs `scenarios/persistence.json`, the three `gate-stage0-*` scenarios and, in its own job in release mode, `scenarios/soak-30-days.json`.

## 21. Security and privacy

- **Secrets:** only in `SecretStore`; never in settings, saves, exports, crash reports or logs; `redact()` on every outbound log path.
- **Data minimization:** AI payloads are compact and per-request; no save-wide context; no unrelated residents' private data; no device identifiers.
- **Untrusted data:** all imports and packs are size-limited and schema-validated; archives are extracted by the hardened reader (§13.6); **no native code is ever loaded from a pack**, and packs cannot initiate network or filesystem access.
- **Untrusted scripts:** pack scripts run only in the Luau sandbox (§23). Defense in depth: no ambient authority in the VM (no `io`, `os`, `debug`, `require` beyond the pack's own files, no network, no FFI); capability-gated API; deterministic fuel and per-pack memory caps; Luau **source only** (precompiled bytecode is rejected, because bytecode is not guaranteed safe to load from untrusted sources); all effects pass the core's validation; user approval of capabilities at install; per-pack quarantine; safe mode.
- **Honest limits:** the sandbox is a language-level boundary inside the game process. A bug in the Luau VM or its Rust binding could in principle escape it. For distribution beyond a trusted audience, Stage 11 evaluates running the script host in a separate, OS-restricted process (Windows job object and restricted token; Linux seccomp / namespaces; macOS sandbox profile) communicating over a bounded channel. This is recorded as a decision to confirm in the roadmap, not a v1 commitment.
- **Supply chain:** pinned toolchain and `Cargo.lock`; `cargo deny` for licenses and advisories; release artifacts are built reproducibly in CI. Pack integrity is by content hash; pack signing is deferred.
- **Local-first:** no accounts, telemetry off by default; any future diagnostics are opt-in and exclude world content. Crash reports are local files with secrets redacted.

## 22. Build map: stages to modules

| Stage | New or extended modules | Key deliverables | Acceptance anchor |
| --- | --- | --- | --- |
| 0 | workspace + CI, pg-host (+ OS impls), pg-core (time, rng, ids, tables, store, schedule skeleton, spatial, actions skeleton, ext points), pg-api, pg-content + pack loader, pg-script (Luau host, sandbox, metering), pg-persist, ai client + settings, pg-runtime (sim thread, snapshots) | Deterministic sample town runs, saves, migrates, recovers; key safety; a test pack runs under replay | Replay hashes identical on Windows / Linux / macOS; save fault injection; sentinel-key test; sandbox conformance suite |
| 1 | needs, mood, memory, social (affinity), conversation, worldgen v1, ui-model, pg-render, pg-app screens, inspector; base pack; script API 0.1: components, hooks, events (read) | Seeded 10–20 resident observation town on a native build | Autonomous run; overheard bubbles; restart retains state; base content loads via the pack loader |
| 2 | scheduler full, commitments; script API 0.2: systems, actions, schedule and commitment hooks | Free-time scheduling, accepted plans | Same seed, same schedules; explainable conflicts; sample pack adds an action and system |
| 2B | organism: vitals and step, parts and attributes, SER and effect programs, pain, consciousness, capacities, first conditions, health window; script API 0.2b (`pg.organism`) | Pawn health as a simulated body | Golden physiology traces on all systems; 200-pawn cost budget; pack cannot kill; sample pack adds an attribute type and effect |
| 3 | worldgen controls, editor, mutation API, town validator; script API 0.3: worldgen stages, town designs | Create / generate / edit towns | Start edited town; no starting-state drift; sample worldgen pack |
| 4 | control (possession), player conversation | Possess and talk | Possession round trip on reloaded save |
| 5 | pawn creator, roster; API: traits, occupations, appearance parts | Custom pawn placement | Create, save, place, possess |
| 6 | interiors, property, economy, jobs; API: items, jobs, shops, obligations | Work / earn / spend / rent | Balances and ownership persist |
| 7 | relationships v2, personality, conditions, life cycle; API: personality and condition data + hooks | Long-term stories | Deterministic, inspectable influence |
| 8 | events, crime, injury, services, filter; API: event categories, offenses | Consequences and services | Preset changes likelihood only; pack events satisfy filter rules |
| 9 | governance; API: office permissions, governance actions | Elections, taxes, construction | Explainable office changes |
| 10 | proposals; API: `ai_proposable` pack actions | Bounded LLM actions | Bad output cannot change state |
| 11 | API 1.0 freeze and docs, `pg pack` tooling, pack manager, dev tools, perf, accessibility, installer, release; script-host hardening decision | Release-ready desktop build with a stable modding API | Profiling on minimum-spec hardware; mod fixtures pass on all OS; installer and update flow |

## 23. Scripting and mod runtime \[S0 spike, grows by stage\]

### 23.1 Modding API objectives

1. **One unified API.** Every extension point (components, systems, actions, events, hooks, content, worldgen stages, dev and inspector views) is reached through a single `pg` namespace, versioned together, and described once in `pg-api`. From that one description the project generates the Luau bindings, type definitions (`pg.d.luau`), reference documentation, capability checks, lint metadata and compatibility reports. There is no second, hand-maintained surface.
2. **Expose engine-level components, do not fork them.** Mods add and modify behavior by registering against the same registries the engine and base game use. The base game is itself a content pack in the same format (§23.3), so the API is proven by shipping it.
3. **Contained execution.** Scripts run only in the Luau sandbox; they have no I/O, network, clock, filesystem, native code or access to other packs' private data. Containment is structural (nothing is exposed) rather than blocklist-based.
4. **Deterministic by contract.** Given the same state, inputs and seed, a script produces the same output on every supported OS and architecture, so replay, golden tests and save/reload equivalence keep holding with mods loaded (§23.5).
5. **The core stays the single writer.** Scripts read a consistent view and *request* changes as typed commands and effects. The core validates and applies them with the same preconditions, ranges, containment checks and diminishing-returns rules as built-in code (§23.9).
6. **Bias, never override.** Hooks return bounded values that the engine clamps, in line with the rule that personality, mood and mods influence weights but never replace hard rules. Hard rules (priority order, containment, ledger sums, permissions, filter) are not hookable.
7. **Explainable.** Every script-influenced decision records a reason code attributed to its pack and hook, visible in the inspector. Script errors name the pack, file, line and extension point.
8. **Safe failure.** A script that errors, loops, exhausts memory or returns invalid data affects only its own invocation; repeated failure quarantines the pack; the world, the save and other packs continue. Any world can be opened in safe mode with scripts disabled.
9. **Capability-scoped.** Packs declare the capabilities they need; the player approves them; the API enforces them at registration and at call time.
10. **Good for authors.** Typed API definitions and editor support, a linter, a headless test and replay harness, hot reload in developer mode, documented samples, and clear errors. A data-only pack must never need to write a line of script.
11. **Stable and evolvable.** Versioned API (`0.x` unstable through Stage 10, `1.0` frozen at Stage 11), per-function `since` and deprecation metadata, and compatibility reports at pack load.

### 23.2 Trust model and containment layers

| Layer | What it does | Stage |
| --- | --- | --- |
| Language sandbox | Luau sandbox mode: read-only builtins, no `io` / `os` / `debug` / `require` beyond pack files / `loadstring`, no coroutines, controlled `pairs` and `tostring` | S0 |
| Capability gating | A pack can register and call only what its approved capabilities allow | S0 |
| Deterministic metering | Instruction-count fuel per call and per tick; per-pack memory cap; caps on built-in string and table functions | S0 |
| Command validation | Scripts emit typed commands; the core validates every one | S0 |
| Quarantine and safe mode | Misbehaving packs are disabled without harming the world; worlds can load with scripts off | S0 |
| Source-only loading | Compile from source; reject precompiled bytecode | S0 |
| Install-time consent | Capability prompt, content-hash pinning of approvals | S1 |
| Process isolation (optional) | Script host in a separate OS-restricted process | S11 decision |

Capabilities (declared in the manifest, approved by the player):

| Capability | Allows |
| --- | --- |
| `read` | Query the world view; register inspector sections and read-only dev views |
| `data` | Register components, events, topics, tones, templates; own-component writes; bounded hooks |
| `systems` | Register systems and actions; emit effects on pack-owned and permitted components |
| `world-write` | Request changes to core components through validated effects (needs, memories, relationships, items, edits) |
| `worldgen` | Register generator stages and town designs |
| `ai` | Mark pack actions as proposable by the LLM; contribute bounded prompt-context data |
| `dev` | Developer-mode console and tools (disabled in normal play) |

**As built (1.7, the Mods screen).** Installed packs live in `packs/<id>/` under the data folder; what the player chose is `mods.json` in storage (enabled pack ids, the capabilities approved per pack, a one-shot safe-mode request). `pg-runtime::mods` finds the packs (a folder that is not a valid pack is listed with the loader's report instead of disappearing), decides what to load, and installs a pack from a folder after validating it. At launch the base game is always loaded, then developer packs given with `--pack`, then each enabled pack whose capabilities beyond `read` and `data` have all been approved; a pack that is enabled but missing approval is left out with a note saying which capability it needs. If the chosen packs cannot be built together, the game starts with the base game alone and says why, so a bad pack can never stop it from starting. Safe mode (the `--safe-mode` flag, or the Mods screen's switch for the next launch) loads only the base game. Changes take effect at the next launch, and the screen says so whenever what is chosen differs from what is running. The screen (`pg-ui-model`, reachable from the main menu) is a bounded, scrolling list (the `Scroll` widget) of one tile per pack, then the screen's own controls. A rolled-up tile shows the pack's name, a mark for an error or for missing approval, what it affects (its capabilities) and an On/Off button; clicking the name unrolls it to show the version, id, file count, size and content hash, its state (running now, will load at the next start, will be left out, off), its dependencies, one checkbox per capability that needs approval (and a line for those that do not), and at the bottom Remove, Enable or Disable, and Open Package (the pack's folder in the file manager). An invalid pack gets a tile with the loader's report. Below the list: Launch in Safe Mode (a switch for the next launch), Install a pack (a system folder dialog), Show the packs folder and Back. This is a first-stage layout with room to grow. `--demo mods` and `--demo mods-open` open it.

### 23.3 Pack format and lifecycle

A pack is a directory or `.pgpack` zip:

```
my_pack/
  pack.json            manifest
  data/                JSON templates, tables, towns, layouts, dialogue tables
  scripts/             .luau sources (entry listed in the manifest)
  assets/              sprites, audio (validated formats and sizes)
  locale/              string tables
```

```json
{
  "id": "coffee_shop",
  "name": "Coffee Shop",
  "version": "0.3.0",
  "api": ">=0.2 <0.4",
  "engine": ">=0.5",
  "depends": [{ "id": "base", "version": ">=0.5" }],
  "capabilities": ["data", "systems"],
  "entry": "scripts/main.luau",
  "settings": [{ "id": "caffeine_strength", "type": "int", "min": 0, "max": 100, "default": 50 }]
}
```

**Lifecycle (all before any world starts):**

1. **Discover** installed packs and the shipped base pack.
2. **Validate manifest** (schema, id, version, size limits); resolve dependencies and `api` / `engine` ranges; compute a deterministic load order (topological by `depends`, then pack id).
3. **Approve capabilities** (new or changed hash → prompt).
4. **Load data** (templates, tables), validate cross-references.
5. **Compile scripts** from source in the pack's own VM; run the entry script in the **load phase**, where `pg.*.register*` calls are available. Load runs under a fuel budget and records every registration.
6. **Freeze registries.** After load, registration functions throw; globals and module tables are frozen. The registries are closed.
7. **Cross-validate** registrations (component names, action ids, hook clamps, reference targets) and produce a `PackReport`.
8. **Ready.** Pack set and hashes are recorded in `content_refs` when a world starts or loads. Pack changes apply only between worlds.

Packs may *extend* base templates (new components on an existing template, through a declared `patch` mechanism that is validated and ordered) but cannot silently replace them; replacement requires an explicit `override` that is shown in the compatibility report.

### 23.4 VM and sandbox profile

One Luau state per pack, created and run only on the sim thread. Luau is embedded through the `ScriptVm` boundary (§23.15), initially implemented over the `mlua` crate with its Luau backend, compiled with floating-point contraction disabled and without native code generation. The sandbox profile below is a property of the VM implementation, not of the binding: whichever implementation sits behind `ScriptVm` must pass the same conformance suite (§18 item 11).

| Area | Profile |
| --- | --- |
| Allowed libraries | `math` (restricted subset), `string` (restricted), `table`, `utf8`, `bit32`, `buffer`; pack-local `require` for its own modules |
| Removed | `io`, `os`, `debug`, `package`, `loadstring` / `load`, `getfenv` / `setfenv`, `collectgarbage`, `coroutine`, `newproxy`, `gcinfo`, the Luau `vector` type |
| `math` | Keep `min`, `max`, `abs`, `floor`, `ceil`, `clamp`, `sign`; remove `random`, `randomseed`, `sin`, `cos`, `tan`, `sqrt`, `exp`, `log`, `pow` family and other transcendental functions; provide `pg.math` integer and fixed-point helpers (`idiv`, `muldiv`, `isqrt`, `lerp_permille`) |
| `string` | Keep formatting and the pattern functions; remove `string.dump` and anything not on the allow-list. No length caps are needed: Luau's pattern matcher calls the interrupt on every step, so fuel stops a catastrophic pattern, and allocation (`rep`, `..`, `format`) is bounded by the memory limit (spike findings, D-031) |
| `pairs` / `next` | Replaced with a deterministic iterator (array part in order, then keys sorted by type and value; only string and number keys); `ipairs` unchanged |
| `tostring` / `print` | `tostring` of tables, functions and userdata returns a type tag, never an address; `print` routes to the per-pack logger |
| Metatables | `setmetatable` rejects `__gc` and `__mode` (no finalizers or weak tables); builtin metatables are frozen; `__metatable` is enforced |
| Globals | Sandbox mode makes standard-library tables read-only; the only added global is `pg`; user globals are frozen after load |
| Types | `--!strict` supported; type definitions in generated `pg.d.luau` |

**Build and compiler configuration (found by the 0.9 spike, D-031).** Luau compiles direct calls to builtins (`math.sin(x)`, `setmetatable(...)`) into fast-call instructions that never read the global, and `for k, v in pairs(t)` into a direct `next` loop. Removing or replacing a global therefore does not remove the capability. The host sets the compiler's `mutable_globals` and `disabled_builtins` for every name the prelude replaces or removes, and the hostile-pack corpus calls each removed function directly. Metatables are frozen when attached (`__mode` is honoured by the collector whenever it runs, so rejecting it at attach time is not enough). Bare `for k, v in t do` and `next(t)` cannot be intercepted; they are an order-sensitive pattern that `pg pack lint` reports and the VM-reload tests catch. The vendored Luau is built without native code generation (no `luau-jit` feature, checked in CI) and with floating-point contraction off through per-target flags in `.cargo/config.toml`, verified at run time by a fused-multiply-add probe on every target.

### 23.5 Determinism contract

Rules every pack must satisfy and the host enforces or tests:

1. **Integers only across the boundary.** Numbers returned to the core, written to components or placed in effects must be integers in range; a non-integer, `NaN` or out-of-range value rejects that invocation with `ScriptError::NonInteger`. Scripts may compute with Luau's doubles internally; the supported way to do fractional math is the `pg.math` fixed-point helpers. Basic IEEE-754 double operations (`+ - * /`) are deterministic across the supported OS and CPU combinations when the VM is built without fused multiply-add contraction, which the build enforces; transcendental functions are removed because they are not.
2. **No time and no ambient randomness.** The only time source is `pg.world.tick()` and the derived day, slot and minute; the only randomness is `pg.rand(stream, keys...)`, drawn from the pack's namespaced streams via the core's counter-based RNG.
3. **No iteration-order dependence.** Because `pairs` is deterministic by construction, the iteration order is stable; linting still flags order-sensitive patterns.
4. **VM at rest between calls.** No coroutines, no finalizers, no weak tables. State that must persist lives in components (saved and hashed) or in the pack's `settings`. Module-level mutable state is forbidden by lint and by freezing; the remaining gap (a closure mutating a captured local) is detected by the VM-reload test variant (§18) and warned about in `pg pack lint`.
5. **Fuel, not wall-clock.** Script limits are counted in deterministic units (Luau interrupt safepoints), so a slow machine and a fast machine cut a runaway script off at the same point.
6. **Pure over snapshots.** Handlers see a read-only view of the state as of the start of the system's invocation; they cannot observe other handlers' pending commands.
7. **Deterministic call order.** Hooks, systems and event handlers run in the total order defined in §6.2 and §23.9 (anchor, pack load order, id; entities in ascending id).

### 23.6 Metering and failure handling

- **Fuel:** `script_fuel_per_call` and `script_fuel_per_tick` (tuned in the S0 spike). Exhaustion aborts the call; its pending commands are discarded; the error is recorded with pack, extension point and entity; the world continues.
- **Memory:** `script_memory_bytes_per_pack` via the allocator limit. Exhaustion aborts the call the same way.
- **C-function caps:** built-in functions that run to completion without safepoints are length-capped (see §23.4) so they cannot stall the sim thread.
- **Watchdog (backstop only):** a wall-clock watchdog protects the sim thread against a pathological hang the fuel counter cannot see. If it fires, the runtime stops the pack, records `PackQuarantined` as a `SimInput` with its tick (so replay reproduces the quarantine rather than diverging), and surfaces a notice. Normal limits are always the deterministic ones.
- **Quarantine is world state.** Failures that fuel and memory limits produce are deterministic, so they are counted per pack (error ticks, and the quarantine tick) in `WorldState.ext`, which survives snapshots, keyframes, rewind and saves, and is hashed; a pack that fails to load is quarantined at once. Recording quarantine as a `SimInput` is needed only for the wall-clock watchdog, which is deferred to Stage 1 (D-031). The defaults are 3 errors within one game day, 50,000 fuel per call, 1,000,000 per tick, 2,000,000 for loading and 8 MiB per pack (`docs/spikes/scriptvm.md`).
- **Quarantine policy:** after `script_max_errors` failures within a window, or on any load-phase failure, a pack is quarantined for the session and flagged in the world's metadata. Its components remain in the save as inert **orphan data** (§23.10) so re-enabling the pack restores them.
- **Reporting:** errors carry pack id, script path, line, extension point, entity id and reason code, and appear in the Mods screen, the inspector and the developer log.

### 23.7 The unified API (`pg` namespace)

| Module | Purpose | Capability | Stage |
| --- | --- | --- | --- |
| `pg.pack` | Manifest info, per-world pack settings, API version | `read` | S0 |
| `pg.world` | Read-only world view: ticks, time, `entity(id)`, `find{kind, with, tag}` (ids in ascending order), `rel(a,b)`, `tile(map,x,y)`, `template(id)` | `read` | S0 |
| `pg.components` | `register{name, applies_to, fields, migrate}`; read and (own-component) write through the command buffer | `data` | S1 |
| `pg.events` | `register{type, severity, variants}` (abstract record + all filter variants); `on(type, fn)`; `emit(type, record)` | `data` | S1 |
| `pg.hooks` | `on(point, fn)`: bounded, engine-clamped value hooks listed in §6.2 | `data` | S1 |
| `pg.actions` | `register{id, params, preconditions, steps, effects, permissions, interruptible, ai_proposable}` | `systems` | S2 |
| `pg.systems` | `register{id, cadence, after / before, query, reads, writes, run}` | `systems` | S2 |
| `pg.effects` / `ctx.cmd` | Construct typed `Effect`s and commands (§8.7) | `systems`, `world-write` | S2 |
| `pg.content` | Register topics, tones, fallback tables, templates (at load) | `data` | S1 |
| `pg.worldgen` | `register_stage{id, after / before, run}` over `GenContext`; town designs | `worldgen` | S3 |
| `pg.rand`, `pg.math` | Seeded draws on pack streams; integer and fixed-point helpers | none | S0 |
| `pg.log` | Per-pack logging (rate-limited) | none | S0 |
| `pg.ui` | Inspector sections and HUD badges as view-model data (no drawing) | `read` | S11 |
| `pg.dev` | Console and tool registration | `dev` | S11 |

Example pack script (`--!strict` Luau):

```lua
--!strict
-- Adds a caffeine component, a decay system, an energy hook and a drink_coffee action.

pg.components.register({
  name = "caffeine",                       -- stored as ext.coffee_shop.caffeine
  applies_to = { "pawn" },
  fields = { level = pg.field.int(0, 1000, 0) },
})

pg.systems.register({
  id = "caffeine_decay",
  cadence = "minute",
  after = "NeedsSystem",
  query = { kind = "pawn", with = { "caffeine" } },
  writes = { "caffeine" },
  run = function(ctx, pawn)
    local level = pawn:get("caffeine").level
    if level > 0 then
      ctx.cmd:set_field(pawn.id, "caffeine", "level", math.max(0, level - 5))
    end
  end,
})

pg.hooks.on("needs.decay_modifier", function(ctx)    -- permille; engine clamps to 500..1500
  if ctx.need == "energy" and ctx.pawn:get("caffeine").level > 200 then
    return 700
  end
  return 1000
end)

pg.actions.register({
  id = "coffee_shop.drink_coffee",
  params = { source = pg.param.entity("object", { tag = "coffee_machine" }) },
  preconditions = function(ctx) return ctx.pawn:is_near(ctx.params.source, 1) end,
  steps = { pg.step.move_to("source"), pg.step.perform_for(5) },
  effects = function(ctx)
    return {
      pg.effects.add_need(ctx.pawn.id, "energy", 120),
      pg.effects.set_field(ctx.pawn.id, "caffeine", "level", 600),
    }
  end,
})
```

### 23.8 Engine exposure matrix

| Engine area | Read | Extend / write through | Constraints |
| --- | --- | --- | --- |
| Components and templates | All resolved templates | Register components; add templates; `patch` / `override` with report | Declarative schemas; bounded fields; validated like built-ins |
| Needs and mood | Values, labels | `needs.decay_modifier`, `mood.rules`; `AddNeed` effects; new needs (S1+) with bounds | Clamped; critical-need rules not pre-emptable |
| Memory | Retrieved memories | `AddMemory` effect; `memory.importance_modifier` | Bounded counts; same decay and cap rules |
| Relationships | Affinity, label | `AdjustRelationship` effect; `relationship.delta_modifier` | Same daily cap and clamping as core |
| Schedule and commitments | Schedules, reservations (read) | `schedule.*` and `commitment.accept_modifier` hooks; `propose_plan` as an action | Cannot place reservations or change priorities directly |
| Actions and tasks | Registry, current task | Register actions; `task.free_time_candidates` | Closed after load; effects limited to the `Effect` enum |
| Events | Event log, abstract records | Register types (with all filter variants); handlers; `emit` | Filter and tone rules apply; no record mutation after emit |
| Conversation and dialogue | Conversations, topics | `conversation.*` hooks; topics, tones, fallback tables | Text is presentation; outcomes via clamped hooks |
| World generation | Params, `GenContext` | `register_stage`; town designs | Named RNG streams; validated before start |
| Spatial | Map, tiles, occupancy (read) | Editor mutation API with `world-write` + `worldgen` | Same mutation validator as the editor |
| Economy and property \[S6\] | Accounts, prices, ownership | Items, jobs, shops, obligations as data; actions | Ledger is append-only; no direct balance writes |
| Organism and health \[S2B\] | Vitals, capacities, attributes, events | Define attribute types, injury types and effect programs (data); request interventions; bounded batched hooks | No death or direct vital writes; clamped contributions; filter variants required |
| Psychology and life cycle \[S7\] | Traits, conditions | Trait and condition data; bounded hooks | Cannot rewrite stable traits from minor events |
| Services and crime \[S8\] | Services, offenses | Event categories, outcome tables (data) | Tone preset changes likelihood only |
| Governance \[S9\] | Office, budget | New permissioned actions | Authority only through ActionDef permissions |
| AI \[S10\] | — | `ai_proposable` actions; bounded context data | No network or key access; proposals validated like any other |
| UI | Snapshot (read) | Inspector sections as view-model data | No drawing or input capture from scripts |

### 23.9 Execution model

- **Views and commands:** each invocation receives a `ctx` with a read-only view of the state as of the start of the invocation (`ctx.world`, entity views) and a command buffer (`ctx.cmd`). Reading your own earlier commands in the same invocation is not supported.
- **Flush points:** after each system invocation batch, each hook call that returns effects, and each action `effects` call, the host validates and queues the buffered commands; the core applies them at the next defined flush point in the pipeline. Validation uses the same checks as built-in commands (preconditions, ranges, permissions, containment) and rejects with a reason code; one rejected command does not discard the rest of a batch unless the batch is declared atomic.
- **Order:** systems and handlers run in the order defined in §6.2 and §23.5(7). Commands from different packs apply in that same order, so conflicts resolve deterministically.
- **Queries:** `pg.world.find` returns ids in ascending order; `query` declarations on systems let the host iterate matching entities and call `run` per entity (or per batch), minimizing boundary crossings.
- **Hooks:** a hook receives a small, typed context and returns a bounded integer; multiple packs' hooks combine by the hook's declared combiner (sum, product-permille, min, max, or first-wins) before the engine's clamp. The core asks through its `HookHost` trait (`ask(point, world, subject) -> answers in pack load order`); the hook points, combiners and clamps are defined once in `pg-api`. Hook points so far: `movement.speed_modifier` (product of permille factors, clamped to 500..1500), asked only for pawns that are walking; `memory.importance_modifier` (product, 500..2000), asked for the pawn who forms a conversation memory; and `relationship.delta_modifier` (product, 0..2000), asked once per closed conversation with the pair's lower id as subject.

### 23.10 Data model integration

- **Component storage:** pack components live in `WorldState.ext`, keyed by component name then entity, with typed integer fields from the declarative schema. A component **applies to every entity of its declared kinds**: an entity with nothing stored has the field defaults, and only written values are stored. The store is included in saves, in the canonical form, in the state hash and in the inspector, and is validated on every write by the core (declared component, existing entity of an applicable kind, known field, value in range, requesting pack owns the component). It is **left out of the canonical form and the hash while empty**, so worlds without packs keep their hashes. Per-pack error history and quarantine live in the same store. Explicit attach and detach of components arrives with templates in Stage 1.
- **Pack settings:** declared in the manifest, chosen at world creation, stored in `WorldSettings.mod_settings`, readable via `pg.pack.settings`.
- **Migrations:** a component declares a `version` and a pure `migrate(from, old_fields) -> new_fields` function. The host runs the chain at load inside the sandbox; any failure refuses the load and offers safe mode.
- **Orphans and safe mode:** if a pack is missing, disabled, quarantined or fails validation, its component values are preserved untouched in `ext` as inert orphan data and are re-attached if the pack returns. Safe mode loads the world with all scripts off, so a save is never lost to a bad pack. Because orphan data is hashed with a marker, safe-mode play is flagged as divergent for replay purposes.
- **Content refs:** `content_refs` records each active pack's id, version and content hash (data + script source). A hash mismatch triggers the compatibility report.

### 23.11 API versioning and stability

- The API follows semantic versioning: `0.x` is explicitly unstable through Stage 10 (breaking changes allowed with a changelog and automated `pg pack lint` migration hints); `1.0` freezes at Stage 11.
- Each `pg-api` entry carries `since`, optional `deprecated_in` and `removed_in`, and a capability. Deprecated functions keep working for at least two minor versions after 1.0, with lint warnings.
- A pack declares an `api` range. At load the host reports exactly which functions the pack uses, which are deprecated, and which require capabilities not approved.
- The base pack must build against the current API with no private back doors.

### 23.12 Tooling

- **Generated artifacts** from `pg-api`: `pg.d.luau` type definitions, reference docs, a machine-readable manifest of functions and hooks, and the lint rule set.
- **`pg pack` CLI:** `new` (scaffold from a template), `lint` (manifest, API use, determinism patterns, undeclared globals, upvalue mutation heuristics), `test` (run the pack in a headless world under golden replay, including the VM-reload variant), `docs`, `pack` (build `.pgpack`).
- **Editor support:** project files for Luau language-server integration using the generated definitions.
- **Developer mode:** pack inspector, script console, per-system profiling and hot reload (recorded as `ScriptReload`).
- **Authoring aids \[S0 data files at 0.9; scripts at 0.9; polish Stage 11\]:** validation errors suggest the closest known name ("did you mean…?", edit distance) for unknown fields, components, enum values, parents and API names; `pg content schema` exports JSON Schema for templates, manifests and component params so editors give inline validation with no tooling; the generated `pg.d.luau` covers scripts. `pg content tree` (Stage 1) prints the `extends` forest; template *variants* (data-only expansion of one template into several) are evaluated at the end of Stage 1 and implemented when there are about 50 templates.
- **Samples:** a small gallery of reference packs (a need, an action, a hook, a worldgen stage, a town design) that double as API conformance tests in CI.

### 23.13 Performance notes

- VM per pack bounds blast radius and memory, at the cost of a small per-pack baseline; cross-pack interaction is through events and component data, not shared Lua state.
- Prefer `query`-driven systems and batched hooks; budget scripts per system and per pack and surface the numbers in the profiler.
- Interpreter-only by default. Luau native code generation may be evaluated later and enabled only if golden replays prove bit-identical results on every supported target.
- Heavy or hot mechanisms (pathfinding, scheduling, conversation selection) remain Rust and are extended through hooks rather than reimplemented in scripts.

### 23.14 Out of scope and open decisions

- **Out of scope:** native-code plugins, scripted drawing and shaders, network or file access, direct cross-pack function calls (initially), scripted UI screens, and any marketplace or auto-download of packs.
- **Decisions to confirm:** the Luau binding (initial choice `mlua`, with explicit spike criteria and upgrade triggers in §23.15); the cost and exact semantics of the deterministic `pairs` replacement; interpreter-only versus later native codegen; whether to isolate the script host in a separate process at Stage 11; whether to offer a workshop-style distribution later.

### 23.15 ScriptVm boundary and binding decision \[S0, decided in 0.9\]

**Boundary.** `pg-script` talks to Luau only through one trait, and only the private `luau` module names the binding crate. The trait speaks in the project's own value, registration and error types, so replacing the implementation changes one module.

```rust
trait ScriptVm: Send {
    fn load(&mut self, chunk: SourceChunk) -> Result<(), VmError>;                      // compile from source; bytecode is refused
    fn run_load_phase(&mut self, entry: &str, fuel: Fuel) -> Result<Registrations, VmError>; // registration on; then registries close, globals freeze
    fn call(&mut self, handler: HandlerId, args: &Val, fuel: Fuel) -> Result<CallResult, VmError>;
    fn call_batch(&mut self, handler: HandlerId, args: &[Val], fuel: Fuel) -> Vec<Result<CallResult, VmError>>;
    fn fuel_used(&self) -> u64;           // deterministic units (§23.5)
    fn memory_used(&self) -> usize;
    fn set_memory_limit(&mut self, bytes: usize);
}
```

- `Val` (nil, bool, integer, bounded text, list, string-keyed map) crosses the boundary. Integers are limited to ±2^53 and every conversion is range-checked in one place; floats, `NaN`, cycles, functions, deep or huge tables and invalid UTF-8 are errors.
- A call's **context is data in its arguments** and the **buffered commands come back in the result** (rather than a `&mut CallCtx` and a registrar callback): the VM keeps no reference to the host between calls, calls are pure functions of their input (testable, batchable, replayable) and the VM is `Send`.
- `Registrations` (components, systems, hooks) are returned as plain data and cross-validated by the host before anything is declared.
- `call_batch` is part of the trait from the start; today it loops over `call` (S-034 covers a real batch entry point if profiling ever asks for one).
- A second implementation (a test double, later possibly an in-house binding) must pass the same sandbox conformance, determinism and fuel tests (§18 items 11 and 12).

**Decision.** `mlua` with its Luau backend stays; none of the upgrade triggers was hit. Why it was chosen: years of use around the unsafe parts (stack discipline, error crossing, value rooting), a ready-made sandbox mode, memory limit, interrupt callback and compiler options. The vendored Luau (0.740) builds on all three Tier 1 targets and passes `cargo deny`.

**Spike outcome** (the full report with measurements is `docs/spikes/scriptvm.md`):

1. *Fuel determinism:* interrupt-based fuel is identical across Windows, Linux and macOS and any thread count (pinned figures run in CI) and stops a runaway loop within about ten units of its budget. Luau's pattern matcher also calls the interrupt, so catastrophic patterns are stopped by fuel and no string caps are needed.
2. *Numeric build control:* no native code generation (`luau-jit` is never built; CI fails if it appears) and floating-point contraction off through per-target flags in `.cargo/config.toml`, verified at run time by a fused-multiply-add probe on every target.
3. *Memory limit:* exceeding it aborts the call cleanly, the VM stays usable and other packs' VMs are untouched.
4. *Boundary cost:* about 12 µs per handler call; scripts add about 1.6 percent of a 100 ms tick at 200 pawns (§19).
5. *Sandbox:* the profile of §23.4 installs and holds, including the compiler configuration that stops builtin fast-calls and the `pairs` lowering from bypassing removed or replaced functions; the hostile-pack corpus passes. Known gap: bare `for k, v in t do` and `next(t)` cannot be intercepted (`pg pack lint` reports them; the VM-reload test is the backstop).
6. *Source-only loading:* no path accepts bytecode.
7. *Build and licensing:* the vendored build is about 35 s on Windows and needs only a C++ compiler.

**Upgrade triggers** (move to a hybrid or an in-house binding if any hold): fuel not bit-identical across targets or too coarse; required numeric flags cannot be applied through the binding's build; boundary overhead blocking the resident target even after batching; deterministic behaviour fixable only inside Luau; the bundled Luau lagging a needed fix. **Fallbacks, in order of cost:** (1) fork the helper build crate to pin the Luau version and flags; (2) keep `mlua` but write hot paths against its raw FFI inside the same private module; (3) replace the implementation behind `ScriptVm` with an in-house binding over Luau's C API, at the price of owning the unsafe code, its fuzzing and every Luau upgrade.

## 24. Native build, packaging and distribution \[S0 CI, S11 release\]

- **Toolchain:** pinned stable Rust in `rust-toolchain.toml`; `Cargo.lock` committed; reproducible release builds in CI.
- **Targets:**

| Tier | Target | Role |
| --- | --- | --- |
| 1 | Windows 10 / 11, x86-64 (MSVC) | Reference platform; performance and acceptance targets |
| 1 | Linux, x86-64 (glibc, Vulkan) | Built and tested in CI from Stage 0 |
| 1 | macOS, arm64 (Metal) | Built and tested in CI from Stage 0 |
| 2 | macOS x86-64, Linux arm64, Windows arm64 | Built where CI capacity allows; same golden hashes required |

- **Determinism across targets:** the golden replay suite runs on every Tier 1 target in CI and must produce identical hashes (§18). Compiler flags that could change numeric results are explicitly controlled: `.cargo/config.toml` sets per-target `CXXFLAGS` (`-ffp-contract=off` for GCC and Clang targets, `/fp:precise` for MSVC) for the vendored Luau, a run-time probe checks the result on every target, and `scripts/check_deps.py` rejects `luau-jit`.
- **Continuous integration:** on Windows, Linux and macOS: `cargo fmt --check`, the dependency and feature rules, `clippy -D warnings`, all tests, the determinism vectors, content and string lints, the golden replay with snapshot resume, scenarios, save and bug-bundle round trips, the AI self-check, the cookbook pack tests, `pg-app --smoke`, `pg check` and the parallel pathfinding check; plus a `cargo deny` job. The window itself is not opened in CI.
- **Release profile:** LTO, `codegen-units = 1`, `panic = "unwind"` (needed for the tick-level safety net), `overflow-checks = true` for `pg-core`, symbols stripped from the shipped binary with separate debug symbols kept.
- **Packaging:** a portable zip is the baseline; an installer (MSI or NSIS on Windows, a notarized disk image on macOS, a tarball or AppImage on Linux) follows at Stage 11. The base pack and the API definitions ship beside the executable. No store distribution is planned.
- **Updates:** no auto-updater at first. An opt-in, manual "check for updates" against a release feed may follow; nothing is downloaded or installed without the player's action, and no telemetry is sent.
- **User data and mods:** see §13.1. Mods are installed by dropping a folder or `.pgpack` into the mods directory or through the in-game Mods screen (Stage 1); until capability approval exists, packs with scripts load only when named on the command line (`pg-app --pack <dir>`).
- **Crash handling:** a panic hook and last-resort tick guard (§17) write a local crash report (redacted) and offer recovery from the latest autosave.
- **Licensing hygiene:** `cargo deny` enforces allowed licenses for all dependencies, including Luau and the chosen binding.

## Appendix A. Starter action registry

| Action id | Params | Precondition (summary) | Effect (summary) | Stage |
| --- | --- | --- | --- | --- |
| `move_to` | place: PlaceRef | Destination resolvable, reachable | Position changes along path | 0 |
| `move_to_room` | room_id | Room exists; interior reachable | Pawn's current room updated | 6 |
| `idle_at` | place, minutes | At place | None / small energy regain | 1 |
| `eat` | source (object / venue) | Food source available; near it | hunger up | 1 |
| `sleep` | bed (object) | Bed free; tired or scheduled | energy up; sleeping state | 1 |
| `rest` | seat (object) | Seat free | energy small up | 1 |
| `socialize` | place | Another pawn present or arriving | social up if conversation occurs | 1 |
| `talk_to` | target_pawn, topic, tone | Target within range and available | Starts conversation | 1 / 4 |
| `propose_plan` | invitee, slots, activity, place | Invitee valid; slots free for proposer | Creates Commitment | 2 |
| `respond_plan` | commitment_id, accept | Commitment pending for this pawn | Accept (reserve both) or decline | 2 |
| `work_shift` | workplace | Employed; at / en route | Wage posted; duty progress | 6 |
| `buy` / `sell` | item, venue, qty | Stock; funds; capacity | Item and money transfer | 6 |
| `pay_rent` | obligation_id | Funds available | Ledger transaction | 6 |
| `vote` | candidate_id | Election open; eligible | Ballot recorded | 9 |
| `approve_construction` | request_id | Office permission; budget; zoning | Build job created | 9 |

The original design document's enum table (`0 | move_to_room | room_id | current_room`) maps onto this registry: ENUM = `ActionId`, FUNC = the action's effect handler, TARGET = `params`, OUTPUT = the field(s) written by `effects`. Pack actions use ids of the form `<pack_id>.<name>` and appear in this registry once loaded.

## Appendix B. Event types

The **catalog** (`pg events list`, as built through 0.10, 28 kinds): `input_rejected`, `setting_changed`; world: `map.created`, `pawn.spawned`, `object.spawned`, `object.contained`, `world_edited`; movement: `move.requested`, `move.arrived`, `move.failed`, `move.sidestep`; schedule: `schedule.planned`, `schedule.replanned`; tasks: `task.started`, `task.done`, `task.failed`; commitments: `commitment.proposed`, `.accepted`, `.declined`, `.expired`, `.cancelled`, `.failed`, `.active`, `.completed`; pack diagnostics: `script.error`, `script.quarantined`; developer scaffolding: `dev.nudged`, `dev.day_started`. Each kind has a category, a field schema and a default visibility; debug builds validate every emitted event against the catalog.

Planned: `conversation_closed`, `need_critical`, `mood_changed`, `memory_created`, `memory_expired`, `relationship_label_changed`, `possessed` / `released` \[S4\], later `transaction`, `birth`, `death`, `offense`, `injury`, `election_result`, `construction_started`. Packs register additional types as `<pack_id>.<type>` with all required filter variants.

Life events (Stage 1): `need.urgent`, `need.critical`, `pawn.collapsed`, `pawn.recovered`, `mood.changed` (category `life`) and `town.generated` (category `world`).

Organism events \[S2B\]: `organism.attribute_added`, `organism.attribute_removed`, `organism.state_changed` (alive, unconscious, critical, arrest, dead), `organism.unconscious`, `organism.regained_consciousness`, `organism.resuscitated`, `death`; each has an intensity tier and filter variants for its text.

## Appendix C. Key settings (data-driven, tunable)

| Key | Default | Scope |
| --- | --- | --- |
| speeds | 1x, 3x, 9x, 27x (10, 30, 90, 270 ticks per second) | fixed |
| `slot_minutes` | 30 | world |
| `move_ticks_per_tile` | tuned | world |
| `talk_range`, `hear_range` | 2, 8 tiles | world |
| `max_memories`, `persist_threshold` | 200, tuned | world |
| `tone_preset` | standard | world |
| `content.graphic_filter` | on | device |
| `water` (new-world control, percent of the map) | 18 (0 to 50) | world creation |
| `aging_multiplier` | 1 | world \[S7\] |
| `births_enabled`, `move_ins_enabled`, `population_cap` | off, off, tuned | world \[S7\] |
| `ai_timeout_ms`, `ai_rpm_cap`, `ai_cooldowns` | 8000, tuned, tuned | device |
| `time.autosave_minutes` | 5 (1 to 60) | device |
| `time.pause_on_focus_loss` | on | device |
| `dev.enabled` | off | device |
| `ui.window_mode`, `ui.scale_percent`, `ui.vsync`, `ui.language` | windowed, 100, on, en | device |
| `sim_worker_threads` | auto (cores − 2, min 1) | device |
| `script_fuel_per_call`, `script_fuel_per_tick`, load fuel | 50,000, 1,000,000, 2,000,000 | device / world |
| `script_memory_bytes_per_pack` | 8 MiB | device |
| `script_max_errors`, `script_error_window_ticks` | 3, 14,400 | device |
| `script_watchdog_ms` | tuned (backstop only) | device |
| `scripts_enabled` (safe mode off / on) | on | world (override at load) |
