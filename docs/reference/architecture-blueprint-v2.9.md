# Playground — Architecture Blueprint v2.9

**Purpose:** a granular technical blueprint for building Playground as a **native desktop binary written in Rust, with a sandboxed Luau scripting layer for user-created content packs**, as defined by the Design Document v3.1 and Roadmap v4.2. Roadmap decisions are binding; this document says *how*. **Notation:** interfaces are written in Rust-style pseudocode (structs, enums, traits). It is a spec, not final source; names and signatures may shift during implementation, but the contracts and invariants may not. Stage tags like **\[S4\]** show when a part is first built. **Reading order:** §0–6 are the foundation (Stage 0), and §23 (scripting and mods) is also Stage 0 foundation because it shapes the data model and tick pipeline. §7–15 are the simulation systems. §16–22 cover later modules, quality and the build map. §24 covers the native build and distribution.

**What changed from v1.0:** the platform-agnostic PAL, web/mobile shells, hosted AI gateway and touch input are removed. The project is one native binary with a thin host-services layer, a dedicated simulation thread, a worker pool, and an embedded Luau VM per content pack. The TypeScript reference notation is replaced by Rust. A new §23 defines the modding API objectives and implementation; extension points are cross-referenced from §4, §6, §8, §12, §13, §17, §18, §20 and §21.

**v2.1 (during Phase 0):** added the `pg-canon` crate (§1, §2). The canonical value type and serialization moved out of `pg-core` into a dependency-free crate because `pg-content` needs them and `pg-core` depends on `pg-content`. It also holds a strict, integer-only JSON parser (rejects floats, duplicate keys, lone surrogates and over-deep nesting) used for content packs and replay logs. See `docs/DECISIONS.md` D-010, D-011.

**v2.2 (during Phase 0):** scheduled the accepted developer-experience suggestions (`docs/SUGGESTIONS.md`): reproducibility tooling (§20, replay diff and bisect, bug bundles, scenario files), the RNG stream registry (§5.1), canonical-JSON cross-checking (§18), the content compatibility report and `pg content diff` (§4.2, §13.4), pack-authoring aids (§23.12), and the reason-code and event viewers (§20). Stage tags show when each lands.

**v2.3 (during Phase 0):** scheduled three performance and simulation suggestions: path-search scratch buffers (§19), pawn-aware routing (§7.3, Stage 1) and compressed, trimmable replay logs (§20, milestone 0.6).

**v2.4 (during Phase 0):** milestone 0.5 made the scheduler, commitment and action sections precise where the first implementation had to choose (§8.5 replan timing and displacement scope, §8.6 gathering radius and slot-length changes, §8.7 which parts of the action skeleton exist). No behaviour was removed.

**v2.5 (during Phase 0):** milestone 0.6 made persistence precise where the first implementation had to choose: the manifest lists both generations (§13.1, §13.4), the `.pgsave`/`.pglog`/`.pgbundle` container layout, the pure-Rust zstd encoder and its single level (§13.1), what a save contains and what it deliberately does not (§13.2), replay-log trimming and bug bundles (§20), and a known difference between `Canon` and RFC 8785 key ordering (§5.4).

**v2.6 (during Phase 0):** the Stage 0 app shell and graphical main menu are specified as a milestone of their own (§14.3 "Stage 0 scope"), and Roadmap v4.3 makes them part of the Stage 0 gate.

**v2.7 (during Phase 0):** milestone 0.7 specifics (§3 services and redaction, §10 provider set, Player2 and the device-code sign-in, key and log hygiene) and the architecture additions accepted as S-022 to S-030: typed event catalog (§6.2, §20), row-level state hashes (§5.4), save summaries (§13.1), schema-driven settings (§13.1, §14.3), string tables (§14.3), automatic bug bundles, keyframe snapshots and shadow verification (§6.4, §17, §18), headless UI snapshots (§14.3).

**v2.8 (during Phase 0):** milestone 0.9 (the ScriptVm spike) settled what the first implementation had to choose. §23.4: the compiler is configured so removed and replaced builtins cannot be reached through Luau's fast-call and `pairs` lowering, and string functions need no length caps because Luau's pattern matcher is interruptible by fuel. §23.5 and §23.10: pack components apply to every entity of their kinds, are stored only when written, and are hashed only when present. §23.6: quarantine from deterministic failures is world state. §23.15: `call` takes its context as data and returns buffered commands; registrations are returned rather than pushed to a registrar. See `docs/spikes/scriptvm.md` and D-031.

**v2.9 (during Phase 0):** milestone 0.10 (app shell and graphical main menu), §14: the UI is three layers. `pg-ui-model` holds pure screen state machines from events to state and effects, plus the widget tree (with text snapshots, focus order and keyboard navigation); `pg-runtime::app::AppController` performs effects against the host traits and the running world; `pg-app` is a thin winit, wgpu and egui shell that draws the widget tree and the map through egui's painter. Import and export use folders until native dialogs (S-037), the pause menu pauses a running world, AccessKit is deferred (S-038). See D-033.

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
|  window | input | renderer (wgpu) | UI (egui) | audio   path batches | autosave encode    |
|     ^ RenderSnapshot (triple buffer)  | Commands (SPSC)    worldgen | validation | HTTPS  |
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

- **UI thread (main).** Window, input, renderer, UI and audio. Never touches `WorldState`.
- **Sim thread.** Owns `WorldState`, the core and every Luau VM (Luau states are single-threaded). Runs the fixed-step loop on its own accumulator, decoupled from display refresh.
- **Worker pool.** Pure jobs only: batched pathfinding, autosave encode and compress, worldgen, pack and content validation, HTTPS requests. Results return to the sim thread and are applied at tick boundaries in a deterministic order (§6.5).

**Hand-offs.** The sim thread publishes `RenderSnapshot`s into a lock-free triple buffer (the last two ticks, for interpolation). The UI thread sends `Command`s over a single-producer queue; the runtime stamps each with its application tick and records it (§5.3).

**Dependency rule (enforced by CI via `cargo` metadata checks):**
`pg-app → pg-runtime → {pg-core, pg-content, pg-script, pg-persist, pg-ai, pg-worldgen, pg-ui-model} → pg-host`. `pg-script → {pg-core, pg-api}`. `pg-canon` (dependency-free) sits below everything: `pg-content → pg-canon` and `pg-core → pg-canon`. `pg-core` may import only `pg-content` types and `pg-host` *types* it never calls, and it defines the `ScriptHost` trait that `pg-script` implements (dependency inversion), so the core never links the VM. Nothing imports `pg-app`. `pg-core` is `#![forbid(unsafe_code)]`.

## 2. Workspace layout

```
playground/
  Cargo.toml  rust-toolchain.toml  clippy.toml  deny.toml      pinned toolchain, lint and license policy
  crates/
    pg-core/      time, rng, ids, tables, world, spatial, needs, mood, memory,
                  social, schedule, commitments, actions, events, conversation, ext (extension points),
                  [later] economy, property, lifecycle, psychology, services, governance, proposals
    pg-canon/     integer-only canonical values, canonical serialization, strict JSON parser;
                  no dependencies; shared by pg-content and pg-core
    pg-api/       single source of truth for the scripting API: function and type specs, capabilities,
                  hook clamps, since/deprecated versions, docs (pure data; no VM)
    pg-content/   schemas, template resolver, validators, pack manifest + loader, built-in templates
    pg-script/    Luau host (VM per pack), sandbox profile, value marshalling, fuel and memory metering,
                  bindings generated from pg-api, script-defined component support.
                  All Luau access goes through the `ScriptVm` boundary (§23.15); only one private
                  module inside this crate imports the binding crate (mlua initially)
    pg-persist/   codecs, migrations, slot manager, export/import, archive safety
    pg-ai/        provider adapters, prompt builders, fallback dialogue, client
    pg-worldgen/  generator pipeline, editor commands, town validators
    pg-runtime/   sim loop, command queue, job queue, session, autosave policy, snapshot publisher
    pg-ui-model/  view-model builders, UI state machines (no drawing)
    pg-host/      host-service traits + in-memory test doubles
    pg-host-os/   OS implementations: filesystem, credential store, HTTPS, native dialogs
    pg-render/    wgpu 2D tile/sprite renderer, camera, sprite atlases, bitmap text
    pg-app/       the binary: window loop, egui screens, audio, wiring
  tools/
    pg-cli/       headless sim, replay runner, content linter, save inspector, bench,
                  `pg pack` (lint, test, docs, pack, new)
  data/
    base/         the base game's own content pack (same format as user packs; §23.3)
  fixtures/  golden/  docs/ (generated API reference)
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
    fn get(&self, key: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, key: &str, value: &str) -> Result<(), SecretError>;
    fn delete(&self, key: &str) -> Result<(), SecretError>;
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

**Implementation notes (0.7).** Beyond the traits above, `pg-host` has `LogSink` (with `RedactingLog`, `MemLog`, `StderrLog`), `Secret` and `redact()`, `AllowListNet`, and doubles for every trait (`MemSecretStore`, `ScriptedNet` that records requests, `FixedClock`, `ScriptedDialogs`, `NullAudio`, fault-injecting `MemStorage`). `pg-host-os` provides `FsStorage`, `KeyringSecretStore` (Windows Credential Manager, macOS Keychain, Linux Secret Service, with the session-only fallback), `UreqNet` (rustls, `https_only`, no redirects) and `SystemClock`. The native-dialogs implementation arrives with the app shell (0.10).

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
enum SimInput {
    Command    { tick: u64, cmd: Command },                                  // player actions, time control
    Dialogue   { tick: u64, conversation_id: ConversationId, lines: Vec<DialogueLine>, source: DialogueSource /* Ai | Fallback */ },
    Proposal   { tick: u64, pawn_id: EntityId, proposal: ActionProposal },   // [S10]
    SettingChange { tick: u64, key: SettingKey, value: SettingValue },
    PackQuarantined { tick: u64, pack_id: PackId, reason: QuarantineReason },  // recorded so replay reproduces it (§23.6)
    ScriptReload { tick: u64, pack_id: PackId, content_hash: Hash },           // dev builds only (§20)
}
```

AI text is nondeterministic, so its **result is recorded as an input** and replay uses the recording. Script behavior is deterministic by contract (§23.5), so scripts need no recording beyond their content hash in `content_refs`. Therefore `(initial snapshot | seed + content refs) + SimInput log` always reproduces the same state.

### 5.4 State hashing

`hash_state(world)` = `blake3` over the canonical serialization (sorted keys, no whitespace, integers only), computed per table and combined. This includes pack component tables (`ext`). The runtime computes and logs a hash at every day boundary in debug and test builds, and stores the latest in each save for integrity checks.

**Known difference from RFC 8785 (0.6, D-022):** `Canon` writes object members in Unicode code point order (UTF-8 byte order); RFC 8785 sorts by UTF-16 code units. They agree except when keys mix characters from U+E000-U+FFFF with supplementary-plane characters. Engine keys are ASCII, so hashes and saves are unaffected and remain deterministic; the difference is pinned by a test and matters only if canonical JSON is ever handed to an external RFC 8785 verifier with such keys.

**Row-level hashes (S-023, milestone 0.8).** Each table's hash becomes the combination of per-row hashes (row id and the row's canonical form), so a divergence can be localised below the table: `pg replay --bisect` and the divergence detector name the entity (`pawns: pawn_3`). Day hashes recorded in logs stay per-table; a per-row dump is produced on mismatch. Memory and CPU cost are about those of hashing the table whole.

## 6. Simulation loop and time \[S0\]

### 6.1 Time units

- **Tick** = smallest simulation step. `TICKS_PER_GAME_MINUTE = 10` (constant). A game day = 1,440 minutes = 14,400 ticks.
- **Slot** = schedule slot, default 30 game minutes = 300 ticks (data setting `slot_minutes`).
- **Day length setting** `day_real_seconds` (1,200–1,800; default 1,500) and **speed** (1×, 2×, 4×, 8×). At 1× a tick is about 100 ms of real time, so the sim has a large per-tick budget on a desktop CPU. Headless and developer fast-forward run unthrottled.

The core knows only ticks. The sim thread converts real time to a tick count:

```
ticks_per_real_second = 14400 / day_real_seconds * speed
accumulator += real_delta_seconds * ticks_per_real_second      // fractional kept in the runtime
while accumulator >= 1 && ticks_run_this_frame < MAX_TICKS_PER_FRAME { step(); accumulator -= 1 }
```

If the machine cannot keep up, the world runs slower (excess accumulator is dropped); it never skips ticks.

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

**Event catalog (S-022, milestone 0.8).** Events stay `(kind, detail)` values that are not part of hashed state, but each kind is declared once in a catalog (kind, category, field schema using `ParamSchema`, default visibility). Debug builds validate every emitted event against it; packs register `<pack>.<kind>`; `pg events list` and the overlay's event viewer read the same catalog.

### 6.3 Movement speed versus tile scale

One tile is about 1 m, but a game day lasts about 25 real minutes, so physically accurate walking would cross the town in a blink. **Movement speed is a tuned abstraction**, stored as `move_ticks_per_tile` (a data value of at least 1 tick per tile, tuned so typical in-town trips take roughly 5–20 game minutes; if that needs finer speed control, raise `TICKS_PER_GAME_MINUTE`). It is a setting, not a derived value, and is listed in the roadmap's tuning decisions.

### 6.4 Run states

```
Running <-> PausedByUser
Running -> Suspended (window focus lost or minimized; autosave fires)
Suspended -> AwaitingResume (window focused; show a prompt) -> Running | PausedByUser
Running -> Exiting (close requested; autosave; then quit)
```

The world never advances while Suspended or AwaitingResume. The "pause on focus loss" behavior is a device setting, default on.

**Runtime safety nets (S-028, S-029, S-030, milestone 0.8).** (a) *Keyframes:* the sim thread keeps a memory-bounded ring of snapshots (default one per simulated hour, the last 24), each with the inputs applied since; they are what the overlay's time-scrub restores, what the crash path starts a bundle from, and what shadow verification replays. (b) *Automatic bug bundle:* the last-resort tick guard and the divergence detector write a redacted `.pgbundle` (latest keyframe plus the inputs since, content refs, note) to `crash/`; the next launch offers to open it. (c) *Shadow verification:* in dev and soak runs a worker re-simulates the span between two keyframes with a different worker count and compares per-table hashes at day boundaries; a mismatch writes a bundle and flags the last system that ran.

### 6.5 Jobs and the worker pool

The runtime runs non-urgent work on a worker pool: path computation, day planning, autosave encode and compress, generation, content and pack validation, AI requests. Each job has a priority, a time budget and is cancellable. **Pure parallel work inside a tick** (for example a batch of path requests) uses a scoped parallel map and merges results in ascending id order before the tick proceeds. **Asynchronous work** (autosave, AI, generation) returns a result that is applied as a `SimInput` at the next tick boundary, so determinism holds even though jobs finish at different real times (the **tick of application is recorded**).

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
- **Pawn-aware routing \[S1\]:** search ignores other pawns today (a pawn behind a stationary pawn waits, sidesteps, and finally fails with `path_blocked`). From Stage 1, when pawns genuinely stand still (working, sleeping, talking), idle pawns count as temporary obstacles, or tiles they occupy carry a small crowd cost, so pawns route around them. The cache key then includes an occupancy version, and results stay a pure function of their inputs, so parallel solving remains identical to serial.
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

### 8.2 Mood \[S1\]

Mood is a small enum derived each minute by an ordered rule table (first match wins), for example:

1. any need critical → the matching state (`exhausted`, `starving`, `lonely`)
2. recent high-importance negative memory (last 6 game hours) → `upset`
3. any need urgent → `uneasy`
4. recent positive memory or all needs above 600 → `content` / `cheerful`
5. otherwise → `neutral`

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

### 8.4 Relationships \[S1; expanded S7\]

- Stored once per pair (`a < b`). `affinity` −1000..=1000.
- After each conversation, `affinity += outcome_delta(topic, tone, moods, traits)`, clamped, with a small per-pair daily cap for diminishing returns. Pack changes go through the same function and the same cap (`relationship.delta_modifier` hook, or the `AdjustRelationship` effect), so they cannot bypass diminishing returns.
- **Labels** map from affinity thresholds (data): stranger, acquaintance, friendly, friend, close friend; negative side: wary, disliked, enemy. Hysteresis (a margin) prevents flickering labels.
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

**Implementation notes (0.5):** (a) Planning runs in strict priority order, so a full `plan_day` never has a lower-priority reservation to displace; "may displace a priority 4-5 reservation" is therefore implemented by `insert_urgent`, the mid-day path used when a need becomes critical (displaced reservations return unchanged if their slots are still free, otherwise move to the first later free run, otherwise are dropped with `no_free_slot`). (b) Replans are requested by setting `Pawn.replan` and carried out at the next slot boundary (inside ReservationActivator, before activation), starting at that boundary, so a slot already under way is never rewritten; a task failure therefore replans from the following slot. (c) A started reservation is never displaced, including by a commitment. (d) Reservation ids are per-schedule counters and survive being lifted out and put back. (e) A change of slot length makes every schedule stale (it is replanned at the next boundary) and cancels live commitments, because both are expressed in slots.

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

**Skeleton status (0.5):** the registry, `ActionDef` (typed params, steps, `interruptible`, `ai_proposable`), `TaskPlanner` and `ActivitySystem` exist with the built-ins `move_to`, `idle_at` and `meet_at` and the step kinds `MoveTo { within }` and `PerformUntilSlotEnd`. Preconditions, permissions, costs and the `Effect` set are added with the systems that need them (Needs, Stage 1). A pack registers actions only while the registry is open; namespaced `<pack>.<name>`; a built-in may not contain a dot.

**Failure handling:** any step failure ends the Task with a `ReasonCode` (`BlockedDestination`, `PathBlocked`, `PreconditionFailed`, `Interrupted`, `Unaffordable`, `Unauthorized`, `ScriptError`), writes an event, and triggers `replan_from(current_slot)`.

### 8.8 Reason codes \[S0\]

Every planning, failure and acceptance decision stores a `ReasonCode` plus a small parameter bag (for example `{slot: 14, displaced_by: "res_a1"}`) **and its origin** (`Builtin` or `Pack(id)`). The inspector renders them as short plain sentences and attributes pack-influenced decisions to the pack. They are saved with the current day's schedule and dropped at day rollover, except those attached to memories or events.

## 9. Conversation system \[S1; player side S4\]

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

**Implementation notes (0.7).** (a) Adapters are pure and fixture-tested; provider replies are untrusted: size-capped (1 MiB), parsed with serde_json (real providers send floats, which the core's strict parser would refuse; this is the only place outside persistence edges that uses it), reduced to one text field, stripped of control and direction-override characters and length-capped; error messages are redacted and capped. (b) `ProviderAdapter` also has an optional `connection_check` (a free account request) with `parse_connection_check -> ConnectionInfo { credits, tier }`; providers without one test with a tiny generation. (c) The client adds a bounded response cache, a per-minute request cap, one retry on transient failures and a circuit breaker that counts only provider-health failures (timeouts, offline, 5xx, unusable replies), not bad keys, quota, rate limits or cancels. (d) `Provider::chooses_own_model()` providers send no model id and reject a custom model in settings.

**Player2 (added in 0.7 groundwork).** Taken from Player2's published OpenAPI document (`https://api.player2.game/v1/openapi.json`): base `https://api.player2.game/v1`; `POST /chat/completions` is OpenAI-style (`messages`, `max_tokens`, `temperature`, `stream`; no `model` field) with `Authorization: Bearer <p2Key>`; `GET /account/joules` returns `{ joules, patron_tier, user_id }` and is the free connection check; `GET /health` is a liveness probe; 401, 402 (insufficient credits), 429 are used for auth, quota and rate limiting. Keys come from the **device-code flow** (`POST /login/device/new { client_id }` returns `deviceCode`, `userCode`, `verificationUri[Complete]`, `expiresIn`, `interval`; `POST /login/device/token { client_id, device_code, grant_type: "urn:ietf:params:oauth:grant-type:device_code" }` returns `{ p2Key }`), or a pasted key. `pg_ai::login` implements the protocol as pure request builders, parsers and a polling schedule (`DeviceLoginSession`) driven by a caller-supplied clock, so it never sleeps; the verification link must be https on `player2.game` or a subdomain, the user code is cleaned and capped, timings are clamped, and the device code is a `Secret`. The Player2 NPC (`/npcs/*`), game-data, text-to-speech and speech-to-text endpoints are **not used**: dialogue stays rule-driven with optional generated lines through `/chat/completions` (suggestion S-033 records the possible later uses). The `client_id` is an open decision (Roadmap §12 item 8).

### 10.2 Key handling

1. The key lives in `SecretStore` (Windows Credential Manager, macOS Keychain, Linux Secret Service) only. Settings and saves hold provider and model id, never the key. Where no credential service exists, the key is session-only and never written to disk in plaintext.
2. The AI client reads the key at call time and calls the provider **directly over HTTPS** (rustls) through the `Net` trait on a worker thread. There is no gateway and no proxy; a desktop binary has no origin restrictions, so the hosted-gateway design from v1.0 is removed. Requests go only to an allow-listed set of provider hosts.
3. A single `redact()` utility scrubs keys from any string before logging. A CI test runs the game with a sentinel key and greps logs, saves, exports and crash reports for it.
4. **Packs never reach the network or the key.** Scripts have no I/O (§23.4). Pack-supplied strings that reach prompts are length-capped, sanitized, treated as untrusted data in the *user* payload, and never placed in the system instruction.

**Implementation notes (0.7).** `Secret` wraps every key: no `Display`, `Debug` prints `Secret(***)`, best-effort wipe on drop. `HttpRequest`'s `Debug` hides credential headers and query values. `redact()` removes secrets it is told about and anything shaped like a credential (`sk-…`, `AIza…`, `Bearer …`, `x-api-key: …`, `key=…`, `"api_key":"…"`); every log path goes through `RedactingLog`. `AllowListNet` refuses anything but plain `https` to an allow-listed host; the real network layer never follows redirects. Where no OS credential service exists the store is session-only and says so (`is_persistent`). The sentinel-key test scans errors, logs, `Debug` output, storage, saves, exports, replay logs, bug bundles and crash reports, raw and decompressed; `pg ai selfcheck` runs the AI-side scan and `pg check` runs everything.

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
worlds/<world_id>/manifest.json        small: name, seed, schema, saved_iso, play_ticks, day, content_refs, state_hash, thumbnail ref
worlds/<world_id>/state.<gen>.pgsave   one file per generation: header (magic, schema, uncompressed length, blake3)
                                       + zstd-compressed canonical JSON of WorldState
designs/<design_id>.json               saved custom TownDesigns
roster/<pawn_id>.json                  reusable custom pawns [S5]
settings/device.json                   device settings (filter level, audio, window, provider + model; NO keys)
settings/pack_approvals.json           per-pack capability grants keyed by pack id + content hash
mods/<pack_id>/ or mods/<pack_id>.pgpack   installed content packs (the shipped base pack lives beside the executable, read-only)
```

User-data directory: `%APPDATA%\Playground` (Windows), `~/Library/Application Support/Playground` (macOS), `$XDG_DATA_HOME/playground` (Linux), resolved through a directories crate. `<gen>` is a monotonically increasing generation number. The manifest names the current generation; the previous generation is kept as a fallback. A single file per generation replaces v1.0's multi-part snapshots, since a desktop has the memory and disk to write it whole.

**Save summaries (S-024, milestone 0.8).** The manifest gains additive summary fields (world name, day, population, play ticks, content packs, last-saved time and, from the app, an optional small thumbnail blob `thumb.<gen>.png` referenced by name) so the Saved Worlds screen lists worlds without loading them, and damaged slots still list. **Settings registry (S-025, milestone 0.8):** device settings are described by a typed registry built on `ParamSchema` (id, type and range, default, label key, scope device or world, restart-required); it validates `settings/device.json`, migrates it (same rules as §13.5), drives `pg settings get/set/list` and, in 0.10, generates the Options screen. Secrets are never settings.

### 13.2 Save procedure (atomic, never destroys the last good save)

1. Snapshot the state in memory (structural clone at a tick boundary on the sim thread; the sim resumes while a worker encodes). If profiling shows the clone is too slow at large sizes, switch to copy-on-write table chunks.
2. Encode to canonical JSON, compress with zstd, compute the checksum.
3. Write generation `g+1` to a temp file in the same directory, `fsync`, then rename into place.
4. Write the manifest the same way, pointing to `g+1` (this is the commit point). On Unix, `fsync` the directory.
5. Delete generation `g-1` only after the manifest commit succeeds. Keep exactly two generations.

A failure at any step leaves the previous manifest and generation intact.

**Implementation notes (0.6):** (a) The manifest lists **both** retained generations, newest first, each with its own state hash, tick, day and time, so a fallback load can report how much play was lost and can verify the older file against its own hash. (b) Each `.pgsave` is `magic "PGSAVE\0\1"` + schema (u32) + uncompressed length (u64) + BLAKE3 (32 bytes) + one zstd frame; `.pglog` and `.pgbundle` use the same layout with their own magic, so one kind can never be opened as another. Decoding checks the header, caps the declared length, stops decompressing at that length and verifies the checksum last. (c) Compression uses `ruzstd` (pure Rust, no C build); its encoder implements one level ("fastest", roughly zstd level 1), which is enough for a town save; any standard zstd decoder reads the result. (d) A save is the world state at a tick boundary. Inputs queued for **future** ticks are not part of it: the runtime applies player commands at the next tick, so the queue is empty at a save. (e) Crash safety is tested by failing the storage at every operation of a save and requiring that a load afterwards returns the old or the new world, never a third state and never "damaged".

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
- **Pack component data** migrates through pack-supplied pure migration functions (§23.10) run in the sandbox; a failing pack migration refuses the load (offering safe mode), never partially applies.

### 13.6 Export and import \[S0 world, S11 packs\]

```rust
struct ExportBundle { format: String /* "playground-export" */, kind: BundleKind /* World | Roster | Design | Pack */,
                      schema: u32, app_version: String, created_iso: String,
                      content_refs: Vec<ContentRef>, payload: serde_json::Value, hash: Hash }
```

- Export builds the bundle from validated data and strips anything not in the schema. Secrets are impossible to include because they are never in the stored model.
- Import pipeline: size check → parse → schema detect → migrate → validate (structure, references, containment, town rules) → id remap → produce a `ValidationReport` → player confirms → write as a new slot. Failure leaves existing data untouched.
- Worlds import as new slots; they do not overwrite unless the player explicitly chooses.
- **Pack archives (`.pgpack`, zip)** are extracted by a hardened reader: reject path traversal (`..`, absolute paths, drive letters, symlinks), cap entry count, per-file and total decompressed size, and compression ratio. Packs are installed only after the manifest validates and the player approves capabilities (§23.2).

## 14. Presentation layer \[S1\]

### 14.1 Contract

The presentation layer consumes `RenderSnapshot` from the runtime and emits `Command`s. It owns no simulation state.

```rust
struct RenderSnapshot {
    tick: u64, day: i64, minute_of_day: u32, speed: u8, run_state: RunState,
    map: MapView { id: EntityId, w: i32, h: i32, dirty_chunks: Vec<ChunkId> },
    pawns: Vec<PawnView { id, name, x, y, prev_x, prev_y, facing, sprite: SpriteRef, state_tag, mood_icon }>,
    objects: Vec<ObjectView { id, sprite: SpriteRef, x, y }>,          // visible area only
    bubbles: Vec<Bubble { id, pawn_id, text, expires_tick, style }>,   // audible to focus only
    focus: Focus { pawn_id: Option<EntityId>, inspector: Option<InspectorModel> },
    notices: Vec<UiNotice>,                                            // AI status, save status, errors, pack errors
}
```

The runtime builds the snapshot once per sim step (or per presented frame when interpolating) from the last two ticks, and only for the **camera viewport plus margin**. Snapshots are published through the triple buffer so the UI thread never blocks the sim thread.

### 14.2 Commands (UI to core)

```rust
enum Command {
    SetSpeed(u8 /* 0|1|2|4|8 */), Pause, Resume,
    FocusPawn(Option<EntityId>),                                       // UI-only, not logged in sim
    Possess(EntityId), Release,                                        // [S4]
    Move { to: Tile }, StartConversation { with: EntityId, tone: ToneId },   // [S4]
    EditWorld(EditCommand),                                            // [S3]
    CreatePawn(PawnDef), PlacePawn { id: EntityId, at: Tile },         // [S5]
    Save, DevTool { tool: DevToolId, args: serde_json::Value },        // [S11]
}
```

Commands that change simulation state become `SimInput`s; view-only commands (focus, camera) stay in the UI model and are not part of replay.

### 14.3 UI structure

Screens are state machines in `pg-ui-model`; `pg-app` draws them with egui (menus, inspector, editor panels, dev tools) while the world view is drawn by `pg-render`.

`Boot → MainMenu → {Continue | NewWorld | SavedWorlds | TownCreation | PawnCreation | Mods | Options} → InGame → {PauseMenu | Inspector | Editor}`

**Stage 0 scope (milestone 0.10).** The first graphical build ships `Boot`, `MainMenu` (Continue, New world, Saved worlds, Options, Quit), a dev-only `NewWorld` (generates the dev town from a seed and size), `SavedWorlds` (list from the slot store with load, export, import, delete and a damaged-slot indicator), `Options` (device settings and the AI provider flow from §10), `InGame` (map and pawns, day and time, pause and speed, save, menu button) and `PauseMenu` (resume, save, options, back to main menu, quit). Every screen is a pure state machine in `pg-ui-model`: it consumes `UiEvent`s and returns the next state plus `AppEffect` requests (load slot, save, spawn world, quit) that `pg-app` carries out through the runtime and host services, so the whole menu flow is unit- and scenario-tested without a window. `pg-app` accepts `--smoke` (create the window or a headless surface, run the boot sequence and a few frames, exit 0) for CI. The Stage 1 screens (inspector, town creation, mods and so on) extend the same state machines.

- **InGame HUD:** time and day, speed control, pause, focus name, notices, menu button.
- **Inspector panel:** activity, mood, needs bars, top relevant memories with reasons, relationship labels, pack-contributed sections (view-model data only; §23.7).
- **Options:** General, Graphics (window mode, resolution, vsync, UI scale), Audio, Controls (rebindable), LLM Configuration (provider, model, key entry, test, status); others as their features arrive.
- **Mods:** installed packs, enable / disable, load order, capability approvals, errors and quarantine status, safe-mode launch.

**String tables (S-026, milestone 0.8 core, 0.10 menus).** All player-visible text is a key into `strings/<locale>.json` tables in the base pack (pack strings are namespaced `<pack>.<key>`); the reason-code sentence table moves into them and `ReasonCode::explain` renders from the active table. A lint reports missing and unused keys and a pseudo-locale (accented, 40 percent longer) is available in dev builds. **Headless UI snapshots (S-027, milestone 0.10):** each screen model can emit a UI-agnostic widget tree (labels, buttons, fields, focus order); tests snapshot it as text and assert that every action is reachable by keyboard; the egui layer maps the same tree to its accessibility output.

### 14.4 Camera, focus and bubbles

- Camera: pan, zoom steps, follow-focus; clamped to map bounds; chunked tile rendering with a cache (dirty chunks redrawn). Pixel-art rendering uses integer scale factors, nearest sampling and a fixed internal resolution option, with correct handling of high-DPI displays and multiple monitors.
- Focus selects one pawn. Bubbles display only for conversations where the focused or possessed pawn is within `hear_range` of a speaker.
- Text is pixel-font bitmap with fixed wrapping rules; bubble stacking resolves overlaps deterministically by pawn id.

### 14.5 Input mapping

Raw mouse and keyboard events become semantic intents (click-select, drag or middle-button pan, wheel zoom, click-to-move, hotkeys) in `pg-app`'s mapper, then Commands. All bindings are rebindable and every action is reachable by keyboard. UI scale, text size and colorblind-safe palettes are settings; screen-reader support through AccessKit is a Stage 11 target.

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
- **Injury model:** abstract severity tiers and a small set of body-region tags for health effects; no gore data exists in content.
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
15. **Scenario files \[S0, format at 0.6\]:** each gate's 'Done when' (and the 30+ day soak with its invariants: bounded memories, events, caches, VM memory) is a JSON scenario (seed, packs, scripted inputs, steps such as save and reload, assertions) run by `pg scenario run`, so acceptance tests read like the roadmap.

**Added in v2.7.** Sentinel-key leak test across every output surface (milestone 0.7, in `tools/pg-cli/tests`); crash-at-every-storage-operation fault injection for saves (0.6); shadow determinism verification in soak runs (0.8, 0.11); menu-flow scenarios against in-memory storage and the scripted AI client (0.10, 0.11); headless UI snapshot and keyboard-reachability tests (0.10).

## 19. Performance plan

- **Targets (initial, refined by profiling on the minimum-spec machine):** simulation step at 200 residents averages under 4 ms and never exceeds 12 ms on the reference machine (at 50 residents, well under 1 ms); script execution is capped at a fixed share of the tick budget (tuned); render frame budget 6.9 ms (144 Hz) to 16.6 ms (60 Hz) with a stable frame time; autosave encode never blocks the sim or render threads; cold load of a mature save in about a second or two on SSD.
- **Threads and layout:** the sim thread and UI thread never share mutable state. Hot per-pawn data uses a data-oriented layout (structure-of-arrays where profiling shows benefit); typed `Vec`s for tiles; no allocation in per-tick hot paths (scratch arenas such as bump allocators reset each tick); spatial hash of pawns by 8×8 tile cells for conversation checks.
- **Algorithmic safeguards:** per-minute systems iterate pawns in id order but skip pawns whose inputs did not change (dirty flags); the TaskPlanner runs only for pawns needing a task; path requests are cached, capped and batched in parallel; deterministic data-parallel maps (§6.2, §7.3) are used only where golden replays confirm identical hashes at any thread count.
- **Rendering:** wgpu instanced sprite batches and cached tile-chunk textures; a sprite atlas; only the viewport plus margin is submitted; the renderer runs from the latest snapshot and never waits on the sim.
- **Scripts:** one VM per pack, interpreter-only (no Luau native code generation until it is proven bit-identical under golden replays); per-system queries are batched so the Rust↔Luau boundary is crossed per system or per batch rather than per field access; compiled bytecode is cached by source hash in memory (never loaded from a pack).
- **Memory:** bounded memories, events (ring buffer + daily summary archive) and caches; per-pack VM memory caps.
- **Path search scratch buffers \[S0, 0.8 profiling pass\]:** search currently keeps scores in a sparse `BTreeMap` (simple and allocation-light for small searches, about 4 µs per node expanded). If profiling shows paths matter, replace it with a reusable dense array stamped with a search generation, one per worker thread. Results must stay bit-identical (the Dijkstra property test and the threaded-vs-serial checks guard this).
- **Optional unfocused fast-path:** for pawns far from the camera, per-tick stepping may be replaced by computing arrival ticks from path length, applied only if golden replays prove identical outcomes (including occupancy conflicts); otherwise keep full stepping and lower the resident cap.
- **Profiling:** `tracing` spans with a Tracy or puffin integration (dev builds) record per-system and per-pack time; `pg bench` benchmarks systems headless with `criterion`. Acceptance thresholds are set from measurements on representative machines (including a low-end integrated-GPU laptop).

## 20. Developer tools \[S11, with hooks earlier\]

A `DevToolRegistry` exposes tools as Commands marked developer-only, requiring a developer-mode toggle, confirmation for destructive actions, and logging as `SimInput`s (so replays remain faithful).

| Group | Tools |
| --- | --- |
| World | Spawn object, destroy object, spawn pawn, show pathfinding overlay (view-only), time jump |
| Pawn | Full heal, resurrect (dev only), injure random / by part, add / remove item, view / add / remove memory, reset occupation |
| Inspection | Schedule viewer with reason codes, route inspector, state hash viewer, proposal inspector with secrets redacted \[S10\], event log |
| Content | Template resolver viewer, pack validator, save inspector |
| Scripting | Pack inspector (registered components, systems, hooks, actions; per-system fuel and time; errors and quarantine status), script console (runs in the sandbox with the `dev` capability, logged as a `SimInput`), hot reload of a pack at a tick boundary (recorded as `ScriptReload`), `print` / log viewer per pack |
| Reproducibility | Replay **diff** (`pg replay --diff a b`: first differing day and table, from per-table hashes stored in replay logs) and **bisect** (`--bisect`: first differing tick) \[S0, 0.4\]; **bug bundle** (`pg bugbundle`, overlay button): snapshot + input log + content refs + tick-hash trail, replayable headlessly \[S0, 0.6\]; **scenario runner** (`pg scenario run`); replay logs compress to `.pgreplay` (zstd) and `pg replay --trim <tick>` cuts a log to its first N ticks, keeping bug bundles small \[S0, 0.6\] |

**Implementation notes (0.6):** a replay log may carry an optional `start` state (the world as of tick *T* plus the inputs still queued then); `ReplayLog::trim(T)` produces one, replaying it reaches the same hashes as the full log, and untrimmed logs are byte-identical to before. A bug bundle (`.pgbundle`) is a trimmed log plus a note, version and time in the compressed container; `pg bugbundle run` replays it anywhere. Scenario files (`playground-scenario`, version 1) drive build/run/save/reload/damage/recover/assert steps; `pg scenario run` executes them and CI runs `scenarios/persistence.json`.
| Explainability | Event viewer with kind-prefix and tick-range filters (`pg sim --events <prefix> --since --until`) \[S0, 0.4\]; **reason-code explorer**: filterable timeline of decisions with their origin ("why did pawn_1a skip lunch?"): `pg schedule explain` at 0.5, overlay panel at 0.10 |
| Content authoring | `pg content tree` (inheritance forest) \[Stage 1\]; `pg content diff` and the load-time compatibility report \[0.6\]; `pg content schema` (JSON Schema export) \[0.9\]; "did you mean…?" hints in every validation message \[0.4, extended 0.9\] |

`pg-cli` mirrors the content and scripting tools for headless use (reproducibility and authoring aids are listed in the table above): `pg pack lint | test | docs | pack | new`, `pg sim`, `pg replay`, `pg bench`, `pg save inspect`.

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
- **As built (0.9):** failures that fuel and memory limits produce are deterministic, so they are counted and quarantined in `WorldState.ext` (per-pack error ticks and quarantine tick), which survives snapshots, keyframes, rewind and saves, and is hashed. Recording quarantine as a `SimInput` is needed only for the wall-clock watchdog and is deferred with it to Stage 1 (D-031).
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
- **Hooks:** a hook receives a small, typed context and returns a bounded integer or small struct; multiple packs' hooks combine by the hook's declared combiner (sum, product-permille, min, max, or first-wins) before the engine's clamp.

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

### 23.15 ScriptVm boundary and binding decision \[S0 spike\]

**Boundary.** `pg-script` talks to Luau only through one narrow, crate-private trait. Nothing else in the workspace, including the rest of `pg-script`, names the binding crate's types. The trait speaks in the project's own value and command types, so replacing the implementation changes one module.

```rust
trait ScriptVm {
    fn new(cfg: VmConfig) -> Result<Self, VmError> where Self: Sized;   // sandboxed state: libs, globals, limits
    fn load(&mut self, chunk: SourceChunk) -> Result<(), VmError>;      // compile from source; bytecode is rejected
    fn run_load_phase(&mut self, entry: &str, fuel: Fuel) -> Result<Registrations, VmError>; // closes registries, freezes globals
    fn call(&mut self, handler: HandlerId, args: &Val, fuel: Fuel) -> Result<CallResult, VmError>;
        // `args` carries the context as plain data; the result carries the return value and the buffered commands
    fn call_batch(&mut self, handler: HandlerId, args: &[Val], fuel: Fuel) -> Vec<Result<CallResult, VmError>>;
    fn fuel_used(&self) -> u64;                                         // deterministic units (§23.5)
    fn memory_used(&self) -> usize;
    fn set_limits(&mut self, memory_bytes: usize);
    fn freeze_globals(&mut self);
}
```

- **As built (0.9, D-031):** `Val` (nil, bool, integer, bounded text, list, string-keyed map) replaces `ScriptArgs`/`ScriptRet`. The context is data in the arguments and commands come back in the result, instead of a `&mut CallCtx` and a `Registrar` callback: the VM keeps no reference to the host between calls, calls are pure functions of their input (testable, batchable, replayable) and the VM is `Send`. Integer values are limited to ±2^53 and every conversion is range-checked in one place.
- `ScriptArgs` and `ScriptRet` are plain Rust enums and structs (integers, bounded strings, ids, small tables of the same). Conversion to and from Luau values lives entirely behind the trait, so the boundary rule "integers only into the core" (§23.5) is enforced in one place.
- Batched calls (`call_batch` over a slice of entity views) are part of the trait from the start, so a faster marshalling path can be added without touching callers.
- A second implementation (a test double and later possibly an in-house binding) must pass the same sandbox conformance, determinism and fuel-accounting tests (§18 items 11–12).

**Initial implementation: `mlua` with its Luau backend.** It is chosen to reach a working, tested sandbox fast, not because the choice is final. Rationale: years of use around the unsafe parts (stack discipline, error and exception crossing, value rooting, userdata lifetimes); ready-made sandbox mode, memory limit, interrupt callback, compiler options and library selection; and a raw FFI escape hatch for hot paths.

**Spike questions (Stage 0 must answer all with measurements, recorded in the repo):**

1. **Fuel determinism:** is the interrupt-based fuel count identical across Windows, Linux and macOS (and at any worker-thread count) for the same script and inputs, and is it fine-grained enough to stop a runaway loop quickly?
2. **Numeric build control:** can Luau be built with floating-point contraction disabled and without native code generation, and can CI verify those flags on every Tier 1 target?
3. **Memory limit:** does exceeding the per-pack limit abort the call cleanly, leave the VM usable or safely discardable, and never corrupt other packs' VMs?
4. **Boundary cost:** is a per-minute system over 200 pawns, a per-tick hook and a query-driven batch within the script share of the tick budget? Benchmark with `criterion` against the rewritten-in-Rust equivalent to see the overhead, and compare per-entity calls against `call_batch`.
5. **Sandbox profile:** can the deterministic `pairs`, `tostring`, restricted `math` / `string`, removed libraries, frozen globals and rejected `__gc` / `__mode` be installed and enforced cleanly after VM creation, and does the whole hostile-pack corpus (§18 item 11) pass?
6. **Source-only loading:** is it impossible to feed precompiled bytecode through any exposed path?
7. **Build and licensing:** does the vendored build work reliably on all Tier 1 targets and pass `cargo deny`?

**Upgrade triggers (move to a hybrid or an in-house binding if any hold):**

- Fuel counting is not bit-identical across targets, or is too coarse to bound a hostile loop.
- The required numeric build flags cannot be applied through the binding's build, and forking the helper crate is not enough.
- Boundary overhead blocks the resident target even after batching.
- Deterministic behavior (for example `pairs`) is too slow or too fragile when patched from the host and needs to be fixed inside Luau.
- The binding's bundled Luau version lags a needed fix, or an unfixed soundness issue appears in the code we depend on.

**Fallbacks, in order of cost:** (1) fork the helper build crate to pin the Luau version and flags; (2) keep `mlua` but write the hot paths against its raw FFI inside the same private module; (3) replace the implementation behind `ScriptVm` with an in-house binding over Luau's C API, which gives exact version and patch control and marshalling shaped to our queries, at the price of owning the unsafe code, its fuzzing and every Luau upgrade. Because the rest of the engine only sees `ScriptVm`, each step is bounded in blast radius.

## 24. Native build, packaging and distribution \[S0 CI, S11 release\]

- **Toolchain:** pinned stable Rust in `rust-toolchain.toml`; `Cargo.lock` committed; reproducible release builds in CI.
- **Targets:**

| Tier | Target | Role |
| --- | --- | --- |
| 1 | Windows 10 / 11, x86-64 (MSVC) | Reference platform; performance and acceptance targets |
| 1 | Linux, x86-64 (glibc, Vulkan) | Built and tested in CI from Stage 0 |
| 1 | macOS, arm64 (Metal) | Built and tested in CI from Stage 0 |
| 2 | macOS x86-64, Linux arm64, Windows arm64 | Built where CI capacity allows; same golden hashes required |

- **Determinism across targets:** the golden replay suite runs on every Tier 1 target in CI and must produce identical hashes (§18). Compiler flags that could change numeric results (fast-math, FMA contraction in the Luau build) are explicitly disabled.
- **Release profile:** LTO, `codegen-units = 1`, `panic = "unwind"` (needed for the tick-level safety net), `overflow-checks = true` for `pg-core`, symbols stripped from the shipped binary with separate debug symbols kept.
- **Packaging:** a portable zip is the baseline; an installer (MSI or NSIS on Windows, a notarized disk image on macOS, a tarball or AppImage on Linux) follows at Stage 11. The base pack and the API definitions ship beside the executable. No store distribution is planned.
- **Updates:** no auto-updater at first. An opt-in, manual "check for updates" against a release feed may follow; nothing is downloaded or installed without the player's action, and no telemetry is sent.
- **User data and mods:** see §13.1. Mods are installed by dropping a folder or `.pgpack` into the mods directory or through the in-game Mods screen.
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

## Appendix B. Event types (initial)

`conversation_closed`, `commitment_{proposed|accepted|declined|failed|completed}`, `task_failed`, `need_critical`, `mood_changed`, `memory_created`, `memory_expired`, `relationship_label_changed`, `world_edited` \[S3\], `possessed` / `released` \[S4\], `pack_quarantined` (diagnostic), later `transaction`, `birth`, `death`, `offense`, `injury`, `election_result`, `construction_started`. Packs register additional types as `<pack_id>.<type>` with all required filter variants.

## Appendix C. Key settings (data-driven, tunable)

| Key | Default | Scope |
| --- | --- | --- |
| `day_real_seconds` | 1500 | device / world |
| `speed_steps` | 1, 2, 4, 8 | device |
| `slot_minutes` | 30 | world |
| `move_ticks_per_tile` | tuned | world |
| `talk_range`, `hear_range` | 2, 8 tiles | world |
| `max_memories`, `persist_threshold` | 200, tuned | world |
| `tone_preset` | standard | world |
| `graphic_filter` | on | device |
| `aging_multiplier` | 1 | world \[S7\] |
| `births_enabled`, `move_ins_enabled`, `population_cap` | off, off, tuned | world \[S7\] |
| `ai_timeout_ms`, `ai_rpm_cap`, `ai_cooldowns` | 8000, tuned, tuned | device |
| `autosave_minutes` | tuned | device |
| `pause_on_focus_loss` | on | device |
| `window_mode`, `ui_scale`, `vsync` | windowed, auto, on | device |
| `sim_worker_threads` | auto (cores − 2, min 1) | device |
| `script_fuel_per_call`, `script_fuel_per_tick` | tuned (S0 spike) | device / world |
| `script_memory_bytes_per_pack` | tuned | device |
| `script_max_errors`, `script_error_window_ticks` | tuned | device |
| `script_watchdog_ms` | tuned (backstop only) | device |
| `scripts_enabled` (safe mode off / on) | on | world (override at load) |
