# Playground — Native Desktop Product & Build Roadmap

**Version:** 4.9 · **Last updated:** 2026-10-08 **Basis:** Roadmap v4.8 (itself based on v4.0 and v3.0)

## 1. What changed from v3.0

- **v4.9:** the organism questions are answered (D-040). The pressure model is the realistic cardiac-output model; the pawn creator describes a body by **height, weight, sex, age and fitness**, mapped to a physiology profile; harm sources before Stage 8 are illness, starvation, exhaustion and a few accidents, with **weather and fire noted for later** (§12 item 8); the anatomy starts with the original list and gains **eyes and ears, then the neck (a part that holds the throat), then spine segments** as ordered extensions; the profile structure leaves room for **animals and non-human humanoids** (humans only for now); substances are generic classes with accurate names. Suggestions S-040 to S-047 are accepted and scheduled. No gate changed.
- **v4.8:** Stage 0 is accepted (`m0.11`). The Stage 1 plan is settled (procedural sprites for now, the `rfd` crate for native dialogs, emotional-only moods, current map sizes kept and made customisable later). **New: the organism system** (a realism-based pawn health model: body parts, organs, vitals, pain, consciousness, death only from irreversible brain failure) is added as **Stage 2B** after Stage 2, with a forward step in Stage 1 (the capacities interface), fight-or-flight and mental conditions in Stage 7, injury, treatment and services in Stage 8, and the API at 1.0 in Stage 11. Principle 17 is new. Placement and the open questions are in `docs/proposals/organism-system.md` and are confirmed or changed by your answers. No other product decision or gate changed.
- **v4.7:** milestone 0.10 is accepted. Accepted suggestions S-037 to S-039 are scheduled in Stage 1 with the native renderer and UI work: native open and save dialogs replace the exports and imports folders (S-037), AccessKit output for screen readers is switched on (S-038; the full screen-reader pass stays a Stage 11 target) and the main menu gets a living town backdrop (S-039). No product decision or gate changed.
- **v4.6:** milestone 0.9 is accepted. Accepted suggestions S-034 to S-036 are scheduled: the script cost view joins the 0.10 dev overlay (S-036); real batch calls into scripts are built at Stage 1 if the first real system profile shows scripts above a few percent of the tick (S-034); Luau type checking in `pg pack lint` is evaluated with the Stage 11 editor support (S-035). No product decision or gate changed.
- **v4.5:** accepted suggestion S-033 is scheduled as two evaluation items: optional Player2 voice and NPC extras at Stage 4 (possession dialogue) and Stage 11 (audio). No product decision or gate changed.
- **v4.4:** **Player2 is a fifth AI provider** (OpenAI-style web API, sign-in by device code instead of a pasted key, model chosen by the provider; Options flow, Stage 0). Nine accepted suggestions (S-022 to S-030) are scheduled: most of the runtime-facing ones land in milestone 0.8, their UI halves in 0.10 and the soak use in 0.11. A new open decision records the Player2 client id. No product decision or gate changed.
- **v4.3:** Stage 0 now ends with a **launchable desktop app and graphical main menu** (new item and a new clause in "Done when"; gate 1 gains the same clause). The Stage 1 presentation work builds on that shell instead of creating it. No other product decision or gate changed.
- **v4.2:** scheduled three more suggestions: path-search scratch buffers (0.8 profiling pass), pawn-aware routing (Stage 1) and compressed, trimmable replay logs (0.6). No product decision or gate changed.
- **v4.1:** the developer-experience suggestions accepted during Phase 0 are scheduled where they are cheapest and most useful: reproducibility tooling and authoring aids in Stage 0, content-growth tooling in Stages 1 and 6, and polish in Stage 11 (new principle 16, extra items marked **\[DX\]** below). No product decision or gate changed.
- Product decisions, stage order and gates for the *game* are unchanged. Nothing in the simulation design was reversed.
- **Platform:** the platform-agnostic, touch-first handheld direction is replaced by a **native desktop binary**. Handheld, web and mobile are no longer targets. The Platform Abstraction Layer is replaced by a thin **host services** layer that exists for testing, not portability.
- **Technology:** the game is built in **Rust**. Mod and content-pack scripting uses a sandboxed **Luau** VM through one unified API. The renderer is wgpu and the UI is egui (reference choices; Blueprint §1).
- **Modding:** v3.0 prohibited executable mods. v4.0 allows **sandboxed, deterministic, capability-scoped scripts** inside local content packs. Public marketplaces, native-code plugins and unsandboxed mods remain out of scope. Scripting starts as a Stage 0 spike because it shapes the data model and tick pipeline.
- **Performance targets** are raised to desktop levels (about 200 residents on minimum-spec hardware, stretch 500), and the architecture uses a dedicated simulation thread, a worker pool and data-oriented layout.
- **AI access** goes directly from the app to the provider; the hosted gateway is removed. Keys live in the OS credential store.
- Where the original design document disagreed with the roadmap, the roadmap wins; where either disagrees with Design Document v3.1 on platform or modding, v3.0 wins. The reconciliation list is in the Design Document, §17.

## 2. Product direction

**Theme:** Make a small fictional town feel like a place where residents lead their own lives, remember what happens, and respond to player choices, and let players extend that town safely.

**Player promise:** Start by watching a town live. Later, possess a resident, create a pawn and join the town, or shape the town as an editor or elected mayor. Install packs that add new needs, objects, occupations, events and towns.

**Modder promise:** One documented, versioned API with the same power the base game uses; a sandbox that keeps players safe; tools that make authoring pleasant.

**Inspirations:** *Tomodachi Life* and *RimWorld*. The intended experience is a flexible, character-led town sandbox. A deterministic simulation creates consistent outcomes. Optional LLMs add contextual dialogue and, much later, bounded action proposals.

### Player progression

1. **Observer:** watch, pause / speed up, focus and inspect residents. Speech bubbles show dialogue a focused pawn can hear. The player cannot start or join conversations.
2. **Possession:** directly control an existing resident, start or join conversations, return to observation at any time.
3. **Custom pawn:** create a simple pawn and join a town. A full editor follows once playtests show which attributes matter.
4. **Town authoring and leadership:** a town editor creates custom starting towns; unrestricted editing is also available in observation mode. Mayor is an elected in-world occupation (possess the incumbent, or win election as a custom pawn). In-world construction follows town rules and budgets.

## 3. Agreed constraints

- **Reference platform:** Windows 10 / 11 on x86-64 desktop and laptop PCs, played with keyboard and mouse. Linux (x86-64) and macOS (arm64) are built and tested in CI from Stage 0 so the core never drifts, and are supported releases. No handheld, web or mobile targets.
- **Distribution:** direct download of a native binary (portable archive first, installer at Stage 11). No store account and no store release planned.
- **Play scope:** single-player only. The first complete simulation centers on observation.
- **Business model:** free while the core loop is validated; revisit later.
- **Visual style:** top-down pixel art, abstract and non-graphic.
- **First town:** seeded and repeatable; contemporary fictional small town; several districts; hybrid roads (organic main roads, mostly grid-based local blocks); 10–20 adult residents; homes and public gathering places; exterior only.
- **World scale:** one tile ≈ 1 m / 3 ft. Target about 200 residents in a mature town on the minimum-spec desktop (stretch 500), adjusted from profiling.
- **Saving:** multiple local worlds in the OS user-data folder; periodic, focus-loss, close and manual saves; manual world export / import. No accounts or cloud saves in the first release.
- **AI:** bring-your-own-key only (OpenAI, DeepSeek, Anthropic, OpenRouter). The game must be fully playable with no key.
- **Modding:** local packs only; sandboxed Luau scripts through the unified API; capability approval by the player; the base game is a pack.

## 4. Native desktop and modding model

### 4.1 Targets and support tiers

| Tier | Target | Role |
| --- | --- | --- |
| 1 | Windows 10 / 11, x86-64 | Reference platform; performance and acceptance targets |
| 1 | Linux, x86-64 | CI-built and tested from Stage 0 |
| 1 | macOS, arm64 | CI-built and tested from Stage 0 |
| 2 | macOS x86-64, Linux arm64, Windows arm64 | Where CI capacity allows; same golden hashes required |

### 4.2 Host services contract

The game talks to the machine through a small set of traits, each with an in-memory test double so the simulation, persistence and AI client run headless in tests. Rendering and input are owned directly by the application and are not abstracted.

| Service | Needed from | Contract |
| --- | --- | --- |
| Storage | Stage 0 | Named blobs under the user-data folder; atomic replace (temp file, flush, rename); list, delete, free-space query |
| Secret store | Stage 0 | Get / set / delete a provider key in the OS credential store; session-only where none exists; never exported or logged. Keys come from a pasted value or, for Player2, a device-code sign-in whose result goes straight into the store |
| Network (HTTPS) | Stage 0 | Request with timeout and cancel; error classification; allow-listed provider hosts |
| Clock | Stage 0 | Monotonic time for the tick accumulator; wall clock for save metadata only |
| Native dialogs | Stage 0 | Pick a file to read; choose where to save |
| Window lifecycle | Stage 0 | Focus lost, minimized, restored, close requested (delivered by the app, not a trait) |
| Audio | Stage 11 | Play, stop, volume per bus |

### 4.3 Technology baseline

| Concern | Choice | Notes |
| --- | --- | --- |
| Language | Rust (pinned stable toolchain) in a Cargo workspace | Core is `forbid(unsafe_code)`; lints deny floats and nondeterministic constructs |
| Scripting | Luau, one VM per pack, interpreter-only, source-only loading | Behind a private `ScriptVm` boundary; initial implementation `mlua` (Luau backend), confirmed or replaced by the Stage 0 spike (§12) |
| Rendering | wgpu 2D tile and sprite batcher | Own the game loop; no general-purpose engine |
| UI | egui for menus, inspector, editor and dev tools | State machines live in a UI-free model crate |
| Threads | UI thread, simulation thread, worker pool | Deterministic merge of parallel work |
| Persistence | Canonical JSON, zstd-compressed, one file per save generation | Atomic writes; two generations kept |
| Hashing | BLAKE3 for state hashes; a pinned, specified hash for RNG | Test vectors in-repo |

### 4.4 Neutrality and determinism rules

These replace the v3.0 neutrality rules.

1. The simulation core imports nothing from rendering, storage, network, the OS or the scripting VM. Scripts reach it through a trait defined in the core.
2. All machine access goes through host-service traits with test doubles. A CI check on the workspace dependency graph fails the build if the core depends on a forbidden crate.
3. The same seed, content (including scripts) and input log produce the same world on every Tier 1 OS and CPU, at any worker-thread count. CI runs the golden replay suite on every Tier 1 target.
4. No floating point, wall-clock time, ambient randomness, unspecified hashers or hash-map iteration order in authoritative code.

### 4.5 Modding model

- **Packs:** a folder or `.pgpack` file with a manifest, data, optional scripts and assets. The base game is a pack in the same format.
- **API:** one versioned `pg` namespace, described once and used to generate bindings, type definitions, docs, capability checks and lint rules. Extension points: components, systems, actions, events, bounded hooks, content registration, worldgen stages, inspector sections.
- **Containment:** Luau sandbox (no I/O, network, clock, native code or ambient authority), deterministic fuel metering, per-pack memory caps, typed commands validated by the core, per-pack quarantine, safe mode.
- **Capabilities:** `read`, `data`, `systems`, `world-write`, `worldgen`, `ai`, `dev`, approved by the player.
- **Determinism contract:** integers across the boundary, seeded randomness only, no wall-clock, VM at rest between calls.
- **API maturity:** `0.x` unstable through Stage 10; `1.0` freeze at Stage 11. Exposure grows with the systems it extends (see stage items).
- **Honest limit:** the sandbox is a language-level boundary inside the game process. Stage 11 evaluates an optional out-of-process script host for untrusted packs.

## 5. Product decisions and design rules

1. **Simulation first, AI bounded.** Rule-based simulation is authoritative. LLMs write dialogue and, later, propose registered actions at meaningful decision points with cooldowns and deterministic fallbacks.
2. **Bring-your-own AI keys only.** Recommended model per provider plus an optional custom model ID.
3. **Protect keys.** OS credential store only. A key is sent directly to the provider for the request that needs it and is never persisted in logs, saves, exports or crash reports. Provider errors are shown; non-AI fallbacks continue play. Mods cannot reach the network, the keys or the model.
4. **Versioned, data-first content.** Reusable JSON templates and composable components are separate from mutable world state. Validate all imported data. **Scripts supplement data where data is insufficient and run only in the Luau sandbox, as validated requests, never as direct state writes.**
5. **Deterministic schedules.** Start with occupation templates and small variations. The long-term scheduler fills a day from duties, needs, commitments, chores / goals and personality-guided leisure. Mods may bias weights through clamped hooks but cannot change priorities or place reservations directly.
6. **Reservation order:** (1) required work / duties, (2) urgent needs, (3) accepted commitments, (4) personal chores / goals, (5) planned leisure. Truly empty time stays unreserved. Priorities 4–5 may move to a later free slot the same day. A commitment moves only if marked reschedulable; otherwise it fails. Equal priorities resolve by urgency, then earliest commitment, then a stable deterministic tie-break.
7. **Shared commitments:** one resident proposes; the other accepts only if available. Both schedules are reserved only after acceptance. Store whether it can be rescheduled.
8. **Social memory:** initial memories record conversations and shared social events. Important memories persist; minor ones decay.
9. **Psychology in layers:** a few simple moods from needs and events first; later stable traits / values and numeric personality sliders. Represent trauma and mental-health conditions accurately, calmly and never as shorthand for danger.
10. **Configurable tone:** per-world Cozy / Standard / Mature preset (default Standard) changes serious-event likelihood. A separate global graphic-content filter (default on) changes descriptions and visuals. Both bind mods; packs cannot bypass them.
11. **Local-first worlds.** Multiple on-device saves plus manual export / import. A world records the packs and hashes it was made with; a missing or changed pack triggers a compatibility report or safe mode.
12. **Grow systems in dependency order.** Adult-only, no functional businesses at first. Friendship before romance / family. Jobs and economy before tax-funded services and mayoral construction. Births and move-ins have separate per-world switches and a population cap.
13. **Fictional harm stays within game rules.** Crime, injury and death have deterministic in-game consequences and no real-world instruction; substances and treatments are generic classes with accurate names, never brands, doses or procedures. Death is permanent in a world; resurrection is a debug tool.
14. **One API, no back doors.** The base game uses the same pack format and API as player mods. Performance-critical mechanisms stay in Rust and are extended through hooks, not reimplemented in scripts.
15. **Use the machine.** A dedicated simulation thread, a worker pool and data-oriented layout are the default; parallelism is allowed only where golden replays prove identical results at any thread count.
16. **Reproducible, explainable, author-friendly tooling.** Any divergence, failure or odd behavior must be reproducible headlessly (replay diff and bisect, bug bundles, scenario files) and explainable from recorded reasons; content and scripting errors name the likely fix ("did you mean…?"); pack authors get editor support from generated schemas and type definitions, plus runnable cookbook samples that double as conformance tests.
17. **Bodies, not hit points.** Pawn health is a simulated organism: vitals, parts, organs and attributes, with death only from irreversible brain failure. Other systems read derived capacities, never body parts. Pack effects are declarative and clamped, and a pack can never set death or write a vital. Presentation stays abstract and filter-aware, and parts exist regardless of the content rating.

## 6. Effort and foundation guide

**S** contained · **M** several connected pieces · **L** a major feature area · **XL** broad or cross-system. **Foundation: High** means later stages depend on it. These are sequencing aids, not time estimates.

## 7. Stages

### Stage 0 — Reliable foundations

**Effort: XL · Foundation: High · Build first**

- [ ] Cargo workspace with a pinned toolchain; CI on every Tier 1 OS running format, lints (deny floats, unwrap, nondeterministic collections in the core), license and advisory checks, tests and golden replays.
- [ ] Define versioned schemas for worlds, pawns, schedules, reservations, memories, relationships, object definitions / instances, land plots and action proposals.
- [ ] Separate reusable JSON templates and composable components from mutable per-world state. Stable entity IDs with explicit ownership / containment.
- [ ] **Define the host-service traits, their in-memory test doubles and the OS implementations (§4.2), and the UI-free simulation core crate.**
- [ ] Seeded random generation (independent streams per subsystem, pinned hash with test vectors) and deterministic replay from seed + Command log. Validate generated, imported, saved, script-produced and model-produced data at every boundary.
- [ ] Extract time, schedule evaluation, movement and state transitions into testable pure simulation functions.
- [ ] **Threading skeleton:** simulation thread, snapshot hand-off to the UI thread, worker pool with deterministic result application.
- [ ] **Desktop shell with a graphical main menu (milestone 0.10):** the game opens a window to a main menu (Continue, New world, Saved worlds with load / export / import / delete, Options including the AI provider settings, Quit); a world loads into a minimal in-game view (map, pawns, day and time, pause and speed, save, back to the menu); the menu and screen logic live in a UI-free model crate and are tested headless; the dev overlay hangs off the same shell.
- [ ] Normal time: 20–30 minute game day; pause and speed controls; focus-loss pause flow. The world pauses while closed, saves, and asks before resuming.
- [ ] Schedule reservations with the five priorities, same-day rescheduling, commitment acceptance, deterministic conflict and tie rules.
- [ ] Pathfinding (with parallel batched solving) and occupancy rules. Tests for time wrap, schedule boundaries, blocked destinations, reservation conflicts and replay.
- [ ] Options flow for the five AI providers (OpenAI, DeepSeek, Anthropic, OpenRouter, Player2): OS credential-store key storage (or device-code sign-in for Player2), recommended model + optional custom ID (Player2 chooses its own model), connection test (for Player2 a free account check that also shows remaining credits), clear errors, non-AI fallback.
- [ ] Key sent directly to the provider per request; never logged or persisted in saves, exports or crash reports.
- [ ] Versioned local save slots (atomic single-file generations), migrations, corruption recovery, periodic autosave, saves on focus loss / close, manual save, manual JSON world export / import.
- [ ] **Modding spike:** pack manifest and loader; Luau host with sandbox profile, deterministic fuel and memory metering, source-only loading; typed value boundary; one end-to-end extension (a component, a system and a hook) driven by a test pack; the base content loaded through the pack loader; quarantine and safe mode.
- [ ] **\[DX\] Reproducibility and authoring tooling, scheduled across the Stage 0 milestones:** replay diff and bisect with per-table hashes stored in logs, event filter, content-validator hints (0.4); RNG stream registry (0.5); bug bundles, compressed and trimmable replay logs, scenario-file format, canonical-JSON cross-check against RFC 8785, `pg content diff` and the load-time compatibility report (0.6); JSON Schema export for pack data, first three cookbook packs (0.9); reason-code explorer and bug-bundle button in the dev overlay (0.10); soak scenario (0.11); A* scratch-buffer optimization if the 0.8 profiling pass shows pathfinding matters; **runtime and debuggability groundwork in 0.8:** typed event catalog (S-022), row-level state hashes (S-023), save summaries in the manifest (S-024), schema-driven settings registry (S-025), string table with the reason-code sentences moved into it (S-026), automatic redacted bug bundle on a crash or divergence (S-028), keyframe snapshots (S-029) and shadow determinism verification (S-030); **UI halves in 0.10:** generated Options screen, headless UI snapshots with keyboard and accessibility tests (S-027), save thumbnails, event viewer from the catalog, time-scrub control, crash-bundle prompt; shadow verification joins the soak scenario in 0.11.
- [ ] **Sandbox conformance and determinism tests:** hostile-pack corpus, VM-reload determinism variant, cross-OS golden hashes with a script pack loaded.
- [ ] **Define the `ScriptVm` boundary** (Blueprint §23.15) so no other code names the binding crate, and implement it over `mlua` with the Luau backend.
- [ ] **Answer the binding spike questions** (§12 item 1) with recorded measurements, and **decide** whether `mlua` stays, a hybrid is needed or an in-house binding is justified; set initial script budgets from the same measurements.

**Done when:** the app launches to a graphical main menu from which a world can be created, saved, quit and continued; a deterministic sample town can run, pause, save, reload, migrate and recover without losing another save; provider failure is safe; no key appears in logs, exports or crash reports; the core runs headless in tests with no platform code; replay hashes match on Windows, Linux and macOS; a test script pack runs deterministically and a hostile pack is contained without affecting the world.

### Stage 1 — Observation-first living-town slice

**Effort: XL · Foundation: High · First complete simulation target**

- [ ] Replace hard-coded NPCs with validated resident definitions and world instances; generate 10–20 adults from a repeatable seed.
- [ ] Generate households, starting relationships and a few shared memories.
- [ ] Occupation schedule templates with small seeded variations. Occupations are identity / schedule archetypes only, not working jobs.
- [ ] First needs: hunger, energy / sleep, social connection. A few clear mood states driven by needs and events. Moods are **emotional states only**; physical states such as exhaustion and starvation are body conditions (need levels and capacities now, organism conditions from Stage 2B).
- [ ] Record conversations and shared social events as memories. Important persist; minor fade. Explainable rules.
- [ ] One affinity score with readable relationship labels. Resident inspector: current activity, mood, needs, relevant memories, relationship label (and pack-attributed reasons).
- [ ] Watch, pause / speed, focus / inspect. The player cannot initiate or join conversations.
- [ ] Autonomous resident conversations. AI lines when configured, with cooldowns and rule-based fallback. Stylized speech bubbles only when a focused resident can hear.
- [ ] Abstract, non-graphic visuals. No provider key required to play.
- [ ] Generate homes and public gathering places. Exterior-only; no functional jobs, businesses, civic services or interiors.
- [ ] Contemporary small-town theme, several districts, organic main roads + grid local blocks, \~1 m per tile.
- [ ] Generator controls: seed, map size, terrain / water amount, population size. No district-mix controls yet.
- [ ] New worlds default to Standard; Cozy / Standard / Mature per world; global graphic-content filter on by default and affecting both text and visuals.
- [ ] Native renderer and UI: wgpu tile and sprite rendering, egui screens, keyboard and mouse input mapping, window modes, high-DPI handling.
- [ ] **[DX] Native dialogs, accessibility output and menu backdrop (S-037, S-038, S-039):** the operating system's open and save dialogs for import, export and pack installation (replacing the 0.10 exports and imports folders); egui's AccessKit output enabled and tested on the three platforms (the widget tree already carries names, roles and focus order); a small paused seeded town behind the main menu.
- [ ] **Base game shipped as a content pack** using the same format and API as player packs. **Modding API 0.1:** components, events (read), bounded hooks for needs, mood, memory and relationships.
- [ ] Basic Mods screen: list, enable / disable, capability approval, errors, safe-mode launch.
- [ ] **[DX] Script batching (S-034):** profile scripts with the first real systems (`pg pack bench`); if they use more than a few percent of the tick, add a batch entry point so a system's context is built once and Luau is entered once per batch (0.9 measured 12 µs per call and 1.6% of the tick for 200 pawns, so this is conditional).
- [ ] **Pawn-aware routing:** idle pawns act as temporary obstacles (or a crowd cost) in path search, so pawns that stand still while working, sleeping or talking no longer block others; results stay deterministic and cacheable.
- [ ] **Capacities interface (forward step for Stage 2B):** a small set of derived, integer capacities (consciousness, moving, manipulation, talking, eating, breathing) that movement, the scheduler, conversation and actions read instead of reading needs or body parts directly. Stage 1 fills them from needs (a collapsed pawn stops, an exhausted one slows); the organism later becomes their producer and the shortcut is removed.
- [ ] **\[DX\]** `pg content tree` (inheritance forest); evaluate template variants (data-only expansion of one template into several) against the content written so far; add a cookbook sample for hooks.

**Done when:** a seeded 10–20-resident town runs autonomously in the native app, can be observed and inspected, produces overheard conversation, and keeps state and social history after restart; the base content loads through the pack loader; a sample pack adds a component and a hook that visibly change behavior and are attributed in the inspector.

### Stage 2 — Scheduler and social autonomy

**Effort: L · Foundation: High**

- [ ] Upgrade occupation templates into the daily free-time scheduler: duties, needs, commitments, environment, later personality. Leave unreserved time open.
- [ ] Mutual commitments: propose → accept if available → reserve both only after acceptance.
- [ ] Apply the priority order from §5.6; priorities 4–5 move only to a later free slot the same day; priority 3 moves only if flagged reschedulable, else fails.
- [ ] Resolve equal-priority conflicts by urgency, earliest commitment, stable tie-break.
- [ ] Memory / relationship updates for completed or failed shared plans, with importance-based decay.
- [ ] Playtest slot size, activity duration and mood list; settle values in data, not in the data model.
- [ ] **Modding API 0.2:** pack systems with anchored ordering and queries, pack actions in the closed action registry, schedule and commitment hooks, the typed `Effect` set; reason codes attributed to packs.

**Done when:** repeat runs from the same seed and inputs produce the same valid schedules; accepted plans reserve both residents; conflict outcomes are explainable; a sample pack adds an action and a system that run deterministically under replay.

### Stage 2B — Organism core and pawn health

**Effort: XL · Foundation: Medium (high for mods, injury, medicine and psychology that follow)**

A pawn's health is a simulated organism, not hit points. Design, units, model and open questions: `docs/proposals/organism-system.md`; contract: Architecture §8.9.

- [ ] **Vitals and the organism step:** blood volume (from body mass and physiology profile), heart rate, stroke volume and pressure, breathing, blood and organ oxygenation, temperature, pain, adrenaline, nerve, toxicity, consciousness and brain function; integers with fixed units; one fixed-order step per pawn; healthy pawns dormant, acute pawns stepped more often; a world-level clinical time scale.
- [ ] **Body:** parts (hands, arms, chest, stomach, head, legs, feet; mature-rating parts), organs (brain, heart, throat, left and right lung, liver) and skeleton (a single spine for now), each with integrity, oxygenation and efficiency as relevant; attributes with a type, allowed parts, name, description, sub-attributes and type-specific constants (for example `injury_type`); innate attributes for appearance (hair, colour).
- [ ] **Status Effect Registry:** static attributes are plain data; dynamic ones name a registered effect that the organism ticks at a fixed cadence; effects are declarative programs (conditions, bounded vital contributions, transitions, reason keys) run natively, with bounded, batched script hooks only for onset, escalation and clamped modifiers.
- [ ] **Anatomy extensions, in this order, after the base list is working:** eyes and ears (sight and hearing capacities), the neck as a body part that holds the throat, then spine segments (cervical, thoracic, lumbar) for graded paralysis.
- [ ] **Physiology profile:** body mass, height, sex, age and fitness (the pawn creator's language) map to a data-defined profile with the species or race field left open for animals and non-human humanoids later; humans only now.
- [ ] **Blood, oxygen and organs:** transport from lungs to organs; lungs that can fill with fluid; throat patency (breathing and speech); liver toxin clearance; brain regulation of the heart and its autopilot; blood types (ABO and Rh), transfusion compatibility, `acute_hemolytic_reaction`.
- [ ] **Pain and adrenaline:** diminishing aggregation of afflictions, per-pawn threshold, pain shock, temporary extra pain from harmful actions, felt pain = total pain x (1 - adrenaline factor); nerve as the fight-or-flight modifier; **fight-or-flight placeholder** (adrenaline response and a stress input) for systems that need it before Stage 7.
- [ ] **Consciousness, arrest and death:** unconscious, critical, arrest (clinical death with a resuscitation window) and dead (irreversible, from brain failure only); deterministic, explainable, with a non-graphic record; resurrection stays a developer tool.
- [ ] **Capacities** replace the Stage 1 shortcut; movement, scheduling, conversation and actions react (an unconscious pawn replans, a pawn who cannot talk is never dropped from the town).
- [ ] **First conditions and sources of harm:** starvation, exhaustion, dehydration if water becomes a need, a first illness, and a small set of accidents; weather, fire and drowning are parked (§12 item 8); pawns self-treat and help each other with simple care; clinics and surgery come in Stage 8.
- [ ] **Health window:** vitals, a schematic body, attributes with their descriptions, reasons; reachable from inspection and from a button; abstract and filter-aware.
- [ ] **Modding API 0.2b (`pg.organism`):** define attribute types, parts-with-attributes, injury types and effect programs at load time; read vitals and capacities; request interventions as validated commands; bounded hooks; events. A pack can never set death or write a vital; filter variants and an intensity tier are required.
- [ ] **\[DX\]** `pg organism sim` and `pg organism explain`, an Organism tab in the overlay, physiology golden traces, a plausibility lint for pack effects, and a soak with shadow verification.

**Done when:** a pawn that stops eating, or is hurt, goes through readable, explainable physiological changes that match the golden traces on every system; the same seed and inputs give identical bodies after save, reload and replay at any thread count; healthy towns of 200 pawns stay within the organism's cost budget; a sample pack adds an attribute type and an effect that behave and are attributed in the health window; no pack can cause death directly.

### Stage 3 — Town generation and creative editing

**Effort: L–XL · Foundation: High**

- [ ] Complete the seeded generator with map size, terrain / water and population controls; district composition stays internal and balanced.
- [ ] Dedicated town editor in the new-world flow: landscape, road, plot and building tools, with undo / redo and hotkeys for keyboard-and-mouse use.
- [ ] Unrestricted editing in observation mode (creative sandbox, not budget-limited).
- [ ] Compact, walkable generated towns; tune map-size ranges on the reference machine.
- [ ] Validate boundaries, roads, entrances, plots, required locations and resident homes before a town starts.
- [ ] Save and reopen custom town designs.
- [ ] **Modding API 0.3:** generator stages, shippable town designs and the validated mutation API for the `world-write` + `worldgen` capabilities.

**Done when:** players can create or generate a valid town, start it in observation mode, edit it freely and save / reload it without the seed's already-generated starting state changing unexpectedly; a sample pack adds a generator stage and a town design that validate and replay deterministically.

### Stage 4 — Possession and player dialogue

**Effort: M–L · Foundation: Medium**

- [ ] Possess and directly control an existing resident; return to observation without losing that resident's autonomous life.
- [ ] Start and join conversations only while possessing.
- [ ] Optional AI dialogue via the configured provider; actions and relationship outcomes stay rule-based; fallback dialogue works with no key or network.
- [ ] Overheard bubbles for the focused or possessed pawn, subject to hearing range.
- [ ] Graphic-content filter stays global; tone preset stays per world.
- [ ] **[DX] Evaluate Player2 extras (S-033):** optional spoken lines and voice input through Player2's speech endpoints for players who opt in; never part of simulation state, always behind the existing filter and capability settings, and dropped if it cannot be made deterministic-safe and optional.
- [ ] Packs may add tones, topics and fallback dialogue as data; scripts can read the controller but cannot possess or issue player commands.

**Done when:** possession, conversation, return-to-observation and fallback dialogue all work in a saved / reloaded town, and dialogue never changes game state directly.

### Stage 5 — Basic custom pawn and reusable roster

**Effort: M · Foundation: Medium**

- [ ] Deliberately simple pawn creator: name, appearance, occupation, a few traits.
- [ ] Local reusable roster; a created pawn can join a generated or edited town.
- [ ] Basic starting placement and household / social context.
- [ ] Defer full trait sliders and low-level attributes until playtests show which matter.
- [ ] Packs can add occupations, traits and appearance parts as data.

**Done when:** a player can create, save, select and place a custom pawn, then possess it and resume observation.

### Stage 6 — Spatial life, jobs and personal economy

**Effort: XL · Foundation: High for later property and civic systems**

- [ ] Plots, buildings, ownership, home assignments, entrance links; stable IDs through edits and migrations.
- [ ] Reusable hand-authored interior layouts (after the exterior simulation is reliable); persist an interior once created.
- [ ] Functional jobs and workplaces after occupation labels and templates are stable.
- [ ] Personal economy: wages, spending, basic goods, shops / inventory, traceable transactions.
- [ ] Renting and housing costs first; then purchase / sale and ownership changes with explicit affordability and legal rules.
- [ ] Needs connect to goods and services through deterministic rules; get a small job / shop loop working before a full economy.
- [ ] **Modding API:** items, jobs, shops and obligations as data plus validated effects; ledger remains append-only with no direct balance writes.
- [ ] **\[DX\]** Implement template variants once there are about 50 templates, if the Stage 1 evaluation still supports it.

**Done when:** a pawn can work, earn and spend, obtain basic needs, rent or own a home, and keep correct inventory, ownership and balances after reload; a sample pack adds an item and a job that behave correctly under replay.

### Stage 7 — Deeper social life, personality and life cycle

**Effort: XL · Foundation: High for long-term resident stories**

- [ ] Split affinity into affinity, trust, fear and conflict; add relationship archetypes (friend, best friend, lover…).
- [ ] Romance and family only after friendship and relationship persistence work.
- [ ] Stable values / traits and numeric personality sliders (e.g. taciturn ↔ outspoken); editable in the later full pawn editor.
- [ ] High-importance events and relationships influence psychology; minor events never rewrite stable traits.
- [ ] Trauma and mental-health conditions only through research-informed, restrained representations. Never frame diagnosis as a predictor of violence.
- [ ] **Fight-or-flight** in the psychology core (using the organism's adrenaline and nerve), replacing the Stage 2B placeholder; panic and anxiety conditions as organism conditions with mental inputs; brain-damage effects on traits and decisions; blood type inheritance for births; aging effects on physiological baselines.
- [ ] Children and elders; births and move-ins independently toggleable per world with a configurable population cap.
- [ ] One in-game year = one year of aging by default; aging-speed multiplier in settings.
- [ ] Permanent death in a world; resurrection developer-only; non-graphic event record and continuing-world behavior.
- [ ] **Modding API:** trait and condition data with bounded hooks; mods cannot rewrite stable traits from minor events.

**Done when:** social ties, significant experiences, family / life events and aging persist and influence behavior in deterministic, inspectable ways.

### Stage 8 — Crime, safety and civic services

**Effort: XL · Foundation: Medium**

- [ ] Serious crime, injury and legal consequences as fictional, rule-based systems; likelihood responds to each world's preset. Injury is applied to the Stage 2B organism (attributes on parts, vitals, death only from brain failure), with treatment (first aid, resuscitation, transfusion; surgery as scoped) and a clinic and emergency response; abstract visuals only.
- [ ] Generic, non-graphic visuals; the global filter reduces descriptive detail and visual intensity. Validate filtered and unfiltered presentation.
- [ ] Public services in stages, beginning with a clinic and emergency response. Others only once their simulation purpose is defined; school follows child life stages.
- [ ] Deterministic, understandable coverage, response, cost and legal outcomes.
- [ ] **Modding API:** event categories and outcome tables as data; pack-registered event types must supply every filter variant and an intensity tier or the pack is rejected.

**Done when:** the same event gives consistent consequences; presets change likelihood, not authoritative outcomes; filtering changes presentation without corrupting the event record; pack events pass the same filter tests as built-in events.

### Stage 9 — Mayor as an elected occupation

**Effort: L–XL · Foundation: Medium**

- [ ] Periodic elections; votes consider reputation and resident priorities; tune cadence once the social sim is testable.
- [ ] Editor sets an initial mayor; residents may elect another. A player may possess the mayor or run as their custom pawn.
- [ ] Property and business taxes fund construction and services once the economy supports them.
- [ ] In-world construction authority limited by budget, town rules and law. Unrestricted construction stays in editor / observation tools.
- [ ] Link service funding and construction choices to visible outcomes.
- [ ] **Modding API:** permissioned governance actions; authority flows only through `ActionDef` permissions.

**Done when:** elections, office changes, tax revenue, public spending and construction rules persist and are explainable; mayor control is an occupation, not a separate mode.

### Stage 10 — Bounded LLM action proposals

**Effort: L–XL · Foundation: Medium**

- [ ] Only after deterministic schedules, needs, memories and legal action results work.
- [ ] Closed action registry with typed parameters, preconditions, permissions and explicit results.
- [ ] Ask for proposals only at meaningful decision points, with cooldowns, caching, rate limits and deterministic fallback.
- [ ] Send compact context only: needs / mood / task, schedule, selected high-importance memories, relevant values, allowed actions. Never the full save or unrelated residents' private data.
- [ ] Strict-schema JSON proposals validated in simulation code; reject invalid, unreachable, unauthorized or unaffordable actions. The model never writes world state.
- [ ] Developer inspection of proposal context, validation and result with secrets redacted.
- [ ] **Modding API:** pack actions may be marked `ai_proposable` by packs holding the `ai` capability; pack-supplied prompt context is bounded, sanitized, untrusted data.

**Done when:** malformed or impossible output cannot change state; unavailable AI never blocks play; valid proposals produce the same transitions as the registered deterministic actions, including for pack actions.

### Stage 11 — Modding API 1.0, tooling, performance and release readiness

**Effort: XL · Foundation: High for reliable distribution and the modding community**

- [ ] **Freeze Modding API 1.0:** stability policy, `since` and deprecation metadata, compatibility reports at pack load, migration hints in the linter.
- [ ] **\[DX\] Polish and finish:** JSON Schema export for every pack file format, `pg content diff` with full save-compatibility classification, "did you mean…?" for script API names, and the cookbook gallery (every API area has a runnable sample that runs as a CI conformance test).
- [ ] **Authoring tooling:** generated type definitions and reference docs from the single API source; `pg pack new | lint | test | docs | pack`; language-server project files; sample pack gallery that doubles as conformance tests; hot reload and per-pack profiling in developer mode.
- [ ] **Full pack manager:** load order, dependency resolution, capability approvals pinned to content hashes, error and quarantine views, import / export of `.pgpack` files through the hardened archive reader.
- [ ] **Hardening:** fuzz the manifest, archive, value-boundary, import and save paths; finalize script budgets; decide whether to ship an out-of-process, OS-restricted script host.
- [ ] JSON import / export for worlds, pawn rosters, object definitions and custom towns with schema versions, validation reports and safe failure.
- [ ] Developer tools: spawn / remove entities, route inspection, memory inspection / editing, occupation reset, confirmed test injury / healing, scripting tools.
- [ ] Profile on representative desktop machines including a low-end integrated-GPU laptop; set the minimum specification and the resident cap (start at about 200, then raise or lower from measurements).
- [ ] Accessibility and settings: rebindable controls, UI scale and text size, colorblind-safe palettes, window / resolution / vsync options, screen-reader support through AccessKit where feasible, visual / audio settings, onboarding, pause / resume, save recovery.
- [ ] Packaging: portable archive, installers per OS, update instructions, optional opt-in update check, crash reports kept local and redacted.
- [ ] No cloud sync or multiplayer. Monetization stays undecided / free until the core loop is validated.

- [ ] **[DX] Luau type checking (S-035):** evaluate running Luau's analyzer over pack scripts with the generated `pg.d.luau` in `pg pack lint`, alongside the editor-support work.
- [ ] **[DX] Evaluate and, if worthwhile, build Player2 audio extras (S-033):** text-to-speech for dialogue lines through the Audio service, opt-in, with clear credit usage and an off switch.

**Done when:** a representative town runs and saves reliably on the minimum-spec desktop; worlds and packs validate on import; mod fixtures pass on every Tier 1 OS; API 1.0 documentation and tooling let an outside author build and test a pack without the source tree; a build installs and updates without a store account.

## 8. Milestone gates

1. **Reliable core:** a launchable app with a graphical main menu, deterministic clock / scheduler, stable schemas, protected AI settings, local saves and migrations, host services in place, pack loader and Luau sandbox proven by the spike, cross-OS replay hashes identical.
2. **Observation town:** seeded 10–20-adult town in the native app; needs, moods, social memories, one affinity score; watch / pause / speed / focus / overhear; save and reload; base game running as a pack.
3. **Autonomous social sandbox:** algorithmic free-time schedules, accepted commitments, rescheduling, deterministic conflicts.
4. **Create and edit towns:** generator settings, starting-town editor, in-observation edits.
5. **Moddable sandbox:** modding API 0.x covers components, hooks, systems, actions and worldgen stages; sample packs pass golden replay on all Tier 1 OS; a hostile pack is contained.
6. **Possession:** direct control, optional dialogue, return to observation.
7. **Custom pawn:** creator and roster; custom pawn joins and is controlled.
8. **Full town life:** interiors, jobs, personal economy, property, staged public services.
9. **Long-term stories:** expanded relationships, romance / family, personality, life cycle, aging, population growth, configurable serious-event rates.
10. **Mayor and advanced behavior:** elections, tax-funded construction / services, bounded LLM proposals including pack actions.
11. **Release-ready desktop build:** API 1.0 and tooling, pack manager, \~200-resident performance on minimum spec, accessibility, onboarding, installers, reliable updates.

Stage 2B has its own gate check, run with gate 3: the organism's physiology golden traces, its soak with shadow verification, and the pack-cannot-kill hostile cases, on a saved and reloaded world.

A gate is complete only when its acceptance checks pass on a **saved and reloaded** world, not merely when a screen exists.

## 9. Explicit non-goals for the first observation release

- Player-initiated conversation or pawn control.
- Functional jobs, businesses, wages, economy, civic services or elections.
- Enterable interiors.
- Children, elders, births, move-ins or aging.
- Cloud saves, accounts, multiplayer, store distribution or a public mod marketplace.
- Mobile, web or handheld builds.
- An LLM call on every pawn or tick; direct LLM writes to world state.
- Native-code plugins, unsandboxed mods, mod access to the network, files or AI keys, or a stable (1.0) modding API.

## 10. Implementation-time tuning decisions

Tune in the relevant prototype rather than blocking the roadmap:

- Map-size, population, terrain / water ranges; schedule slot and activity durations.
- Initial mood labels and relationship-label thresholds.
- Organism: clinical time scale, step cadence, pain aggregation and thresholds, resuscitation window, blood volume anchors and profile factors, hit tables, first conditions and hazards, and whether mental-health content needs outside review (see the proposal's questions).
- Conversation frequency, memory retention, decay and importance thresholds.
- Cozy / Standard / Mature event rates, election cadence, tax formula, service coverage and cost.
- Minimum desktop specification and performance thresholds (resident target starts at \~200, stretch 500).
- Movement speed versus tile scale (see Architecture §6.3), palette and sprite scope, sound content, and whether mental-health content needs outside subject-matter review.
- Script fuel, memory and quarantine budgets; hook clamp ranges and combiners; the cost of the deterministic iteration replacement; worker-thread defaults.

## 11. Existing prototype

The current app (fixed 24×18 map, five scripted NPCs, hourly routines, API-backed dialogue with fallback lines, in-memory state) is a **reference prototype**, not a base to extend, and none of its TypeScript carries over to the Rust codebase. Reuse its dialogue fallback lines and its provider request and response shapes as inputs (ported to Rust adapters with recorded fixtures). Rebuild the simulation on the Stage 0 core so state, time and schedules are no longer tied to UI effects.

## 12. Decisions to confirm

These are open and should be settled early; none blocks Stage 0 except the first two.

1. **Luau binding approach.** Decided in the Stage 0 spike, behind the `ScriptVm` boundary so the choice is reversible. Start with `mlua` (Luau backend) for its tested handling of the unsafe parts and its ready-made sandbox, memory limit and interrupt support. The spike must answer, with measurements on every Tier 1 target:
   1. Is the fuel count identical across OSes and thread counts, and fine-grained enough to stop runaway loops quickly?
   2. Can Luau be built with floating-point contraction and native code generation disabled, verifiably in CI?
   3. Does the per-pack memory limit abort cleanly without corrupting the VM or other packs?
   4. Is boundary cost acceptable for a 200-pawn per-minute system, per-tick hooks and batched queries?
   5. Can the deterministic `pairs` / `tostring`, restricted libraries, frozen globals and rejected `__gc` / `__mode` be installed, and does the hostile-pack corpus pass?
   6. Is precompiled bytecode impossible to load through any exposed path?
   7. Does the vendored build work on all Tier 1 targets and pass the license check?

   **Triggers to move to a hybrid or in-house binding:** fuel not bit-identical or too coarse; numeric build flags not applicable; boundary overhead blocks the resident target after batching; deterministic behavior needs fixing inside Luau rather than from the host; the bundled Luau version lags a needed fix or a soundness issue goes unfixed. **Fallbacks in order of cost:** fork the helper build crate; keep `mlua` with hot paths on its raw FFI in the same private module; replace the implementation behind `ScriptVm` with an in-house binding over Luau's C API (exact version and patch control, marshalling shaped to our queries, at the price of owning the unsafe code, its fuzzing and every Luau upgrade).
2. **Renderer and UI stack:** wgpu plus egui as the reference choice versus a simpler layer (for example a 2D library such as macroquad). Revisit only if Stage 1 shows friction.
3. **Interpreter-only versus Luau native code generation:** interpreter-only until golden replays prove bit-identical results on every target.
4. **Out-of-process script host:** whether to isolate scripts in a separate, OS-restricted process before distributing untrusted packs widely (Stage 11).
5. **Workshop-style distribution:** whether to ever offer a hosted channel for packs; currently out of scope.
6. **Minimum specification and resident cap:** set from profiling, not assumed.
7. **Organism system:** answered and recorded in D-040 (`docs/proposals/organism-system.md` section 13); only tuning remains (clinical time scale default, pain thresholds, resuscitation window, first illness and accidents).
8. **Weather, seasons and fire (parked):** not scheduled. They would feed the ambient hazards service (S-047) and the organism (cold, heat, smoke, burns) and touch worldgen, the renderer and the economy. Revisit when Stage 3 (generation and editing) and Stage 6 (interiors and buildings) are planned.
7. **Pack signing:** deferred; integrity is by content hash until there is a distribution channel.

8. **Player2 client id (new in v4.4).** Player2's device-code sign-in needs a `client_id` registered with Player2 for this game. Until one exists the build uses a placeholder and the provider will likely refuse it. Needed before the Player2 option can be tried by anyone but a developer; register it by Stage 0's 0.10 checkpoint. Also confirm the API base URL (`https://api.player2.game/v1`, taken from the published OpenAPI document) and the terms at `player2.game/devtos` for a distributed game.
