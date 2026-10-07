# Playground — Design Document v3.1

**Status (v3.2):** adds **Player2 as a fifth AI provider** (§ providers) and commits the project to localisation-ready text and keyboard- and screen-reader-friendly menus from the first graphical build (§12.6, §14.4); no change to scope or pillars. **Status (v3.1):** adds the developer-experience commitments accepted during Phase 0 (§12.6, §14.4); no change to scope or pillars. Updated from v2.0 to define Playground as a **native desktop game written in Rust, with a sandboxed Luau scripting layer for user-created content packs**. Where this document and earlier versions conflict, v3.0 decides (see §17). **Companion docs:** Native Desktop Roadmap v4.2 (when), Architecture Blueprint v2.3 (how).

## 1. Premise

Playground is a 2D pixel-art life-simulation sandbox for desktop computers. A small fictional town is full of residents (**pawns**) who follow routines, form relationships, remember what happens and react to each other and to you. You begin as an observer, then can possess a resident, create your own pawn, edit the town, and eventually hold office as mayor. Players can extend the game with local **content packs** that add data and sandboxed scripts through the same API the base game uses.

It takes inspiration from *Tomodachi Life* (charming, character-led social play) and *RimWorld* (systemic simulation that produces stories, supported by a strong mod community). Its defining ideas: **a deterministic game simulation does nearly all the work; an optional LLM is reserved for the narrow places where it shines**, namely natural dialogue now and bounded action proposals much later; and **the game is built to be extended safely**, with every rule that matters validated by the simulation no matter who wrote the script that asked for it.

## 2. Design pillars

1. **Living town.** Residents lead their own lives whether or not you watch. Every outcome is explainable from visible state.
2. **Simulation is authoritative.** Rules decide what happens. LLM output and mod scripts are never trusted to change state directly; both can only request changes that the simulation validates.
3. **Works without AI.** No key, no network: the game is still complete. AI adds flavor, not function.
4. **Reactive depth, simple surfaces.** Each system is mechanically distinct, simple to read, and feeds the others (needs → schedule → conversation → memory → relationship).
5. **Player-shaped.** Configurable tone, creatable towns and pawns, data-driven content.
6. **Local-first and native.** One native desktop application. Saves and mods live on the player's machine; worlds export as JSON. No accounts, no required network, no installer-service dependencies. The design takes full advantage of the desktop: a larger simulation, multithreading, fast saves, keyboard and mouse.
7. **Moddable through one sandboxed API.** The base game is built from the same content-pack format and scripting API that players use. Mods can add and modify engine-level components and behaviors, but run contained in a sandbox, deterministic, capability-scoped and quarantinable.
8. **Grow in dependency order.** Each system ships only after the ones it depends on are reliable.

## 3. Scope

### 3.1 First release: Observation Town (Stages 0–2)

- Native desktop build (Windows 10/11 first; Linux and macOS built and tested alongside).
- Seeded, repeatable exterior-only small town with 10–20 adult residents.
- Needs (hunger, energy, social), simple moods, one affinity score with labels.
- Occupation-based schedules evolving into the free-time scheduler with commitments.
- Conversations with speech bubbles for the focused resident; AI or rule-based text.
- Watch, pause, speed, focus, inspect. Multiple local saves, export / import.
- Provider options for five AI services (bring your own key, or sign in with Player2).
- Base game delivered as a content pack; the pack loader and Luau sandbox are working (API 0.x, developer-facing). Local packs can add data, components and hooks.

### 3.2 Planned expansion (in order)

| Phase | Adds |
| --- | --- |
| Stage 3 | Generator controls, town editor, unrestricted editing in observation mode; modding API for worldgen stages and town designs |
| Stage 4 | Possession and player dialogue |
| Stage 5 | Simple custom pawn and local roster |
| Stage 6 | Plots, buildings, interiors, jobs, wages, shops, rent, property |
| Stage 7 | Richer relationships, romance / family, personality, life cycle, aging, death |
| Stage 8 | Crime, injury, legal consequences, clinic and emergency services |
| Stage 9 | Elections and mayor as occupation, taxes, in-world construction |
| Stage 10 | LLM action proposals through a closed action registry |
| Stage 11 | Modding API 1.0, pack manager and tooling, profiling, accessibility, onboarding, installer, distribution |

Modding capabilities grow with the systems they extend: components and hooks first, then systems and actions, then worldgen, then economy, psychology, events and governance as those systems appear (Blueprint §23.8).

### 3.3 Out of scope

Multiplayer, accounts, cloud saves, store distribution, a public mod marketplace or in-game mod browser, native-code or unsandboxed mods, scripts that bypass simulation validation, mod access to the network, the file system or AI keys, an LLM call per tick or per pawn, LLM writes to world state. A stable (1.0) modding API before Stage 11. Mobile, web and handheld targets. Monetization is undecided; the game stays free for now.

## 4. Player modes

| Mode | Player can | Player cannot |
| --- | --- | --- |
| Observer | Pause, speed up, focus, inspect, overhear nearby chat; later, edit the town freely | Start or join conversations, control a pawn |
| Possession | Move a resident, start or join conversations, release back to observation | Alter another pawn's psychology directly |
| Custom pawn | Everything in possession, with a pawn you made | — |
| Editor | Landscape, roads, plots, buildings, objects (creative, budget-free) | — |
| Mayor (occupation) | Act within office rules and budget | Bypass town rules (that is editor power) |

Mayor is **not** a separate mode. It is an elected occupation that a possessed or custom pawn can hold.

## 5. The world

### 5.1 Town structure

- **Worldspaces:** the town is an overworld map; each enterable building has its own interior map linked by entrances. First release is exterior-only.
- **Scale:** one tile ≈ 1 m. A mature town targets about 200 residents on a minimum-spec desktop (stretch goal 500), settled by profiling; the first town has 10–20.
- **Districts and roads:** several districts; organic main roads with mostly grid-based local blocks. District mix is generated and balanced internally.
- **Time:** a normal game day lasts 20–30 real minutes. Pause and speed controls. The world pauses whenever the app is closed or loses window focus, and asks before resuming (the focus-loss pause can be turned off in settings).
- **Generation:** controlled by seed, map size, terrain / water amount and population size. Packs can add generator stages and ship ready-made towns.

### 5.2 Property (Stage 6)

- **Land plot:** id, map position, bounds (l × w in feet), area, zoning (residential, commercial, special), owner, market status (owned, leased, foreclosed, for sale).
- **Building:** home, business, public space, civic building or other. Zoning limits what may be built. Sprite on the overworld; entering opens its interior map.
- **City-owned plots:** town hall, library, clinic / hospital, school, police / fire. Not purchasable.
- **Interiors:** reusable **hand-authored layouts** chosen per building type and size, instantiated on first entry and then persisted. (This replaces the original idea of procedural interiors.)
- **Ownership:** pawns, businesses and the player may own plots, with explicit affordability and legal rules. Renting comes before buying.

### 5.3 Objects and content

All in-world things are **objects** defined as JSON templates that inherit from one another and are composed from reusable components.

- Inheritance example: `BaseObject > BaseItem > BaseFurniture > Drawer`; `BaseObject > BaseCreature > BaseSapient > BaseHumanoid > Human`.
- Components describe physical descriptors, interaction contexts, containers, durability, value, damage and so on. Packs can register new components with declarative, bounded schemas.
- **Containers** have slots that accept compatible object types. One object can hold several containers (for example, a room has furniture and decoration containers).
- Templates are versioned and kept strictly apart from mutable per-world state. Imported content is validated. Data packs never execute code; script packs run only inside the Luau sandbox (§12).

## 6. Residents (pawns)

### 6.1 Core state

| Component | Contents |
| --- | --- |
| Identity | Name, appearance, occupation archetype, traits (later values and sliders) |
| Needs | Hunger, energy / sleep, social connection (more later; packs may add needs within engine bounds) |
| Mood | A few readable states derived from needs and events |
| Task | Current activity and any step directions (go here / do this) |
| Reservation | The schedule slot currently being honored |
| Goal | Long-term plans (buy a house, get promoted, marry) — later stages |
| Memories | Memory bank (§6.3) |
| Relationships | Per-pawn affinity and label (later richer dimensions) |
| Inventory | Items held (Stage 6+) |
| Pack components | Extra data attached by content packs, saved and inspectable like any other |

### 6.2 Needs and mood

Needs decay deterministically and are restored by activities. Mood is a function of current needs plus recent memory impact. Moods begin as a short, clear list tuned in play. Personality sliders and stable traits join later and bias, but never override, rules. Mods may adjust decay rates and add mood rules only within engine-clamped ranges.

### 6.3 Memory

Each memory has: **type**, **participants**, **severity** (1–5 base per type), **impact** (signed, how it affects the pawn), **importance** (derived from severity and impact), and **decay rate**.

- First types: conversation and shared social event; more are added with later systems.
- Important memories persist; minor ones fade by explainable rules.
- High-importance memories and relationships may shape later psychology. Minor events never rewrite stable traits.

### 6.4 Relationships

First: one **affinity** score with labels (stranger, acquaintance, friend…) at thresholds tuned in play. Later: separate affinity, trust, fear and conflict, plus archetypes (friend, best friend, lover). Romance and family follow only after friendship persists reliably.

### 6.5 Occupations

First: an occupation is an **identity and schedule archetype only** (for example barista, teacher, retiree) that determines a daily template with small seeded variations. Later (Stage 6): occupations become functional jobs with workplaces, hours, wages and promotions. Mayor is added in Stage 9. Packs can add occupations as data.

### 6.6 Schedules and reservations

- A day is divided into slots (default 30 game minutes; tunable). Slots are filled by **reservations**.
- **Priority order:** 1 required work / duties → 2 urgent needs → 3 accepted commitments → 4 personal chores / goals → 5 planned leisure. Unreserved time stays open.
- **Moving rules:** priorities 4–5 may shift to a later free slot the same day. A priority-3 commitment moves only if marked reschedulable; otherwise it fails.
- **Ties:** urgency, then earliest commitment, then a stable deterministic tie-break.
- **Shared commitments:** one resident proposes; the other accepts only if available; both are reserved only after acceptance.
- Completed or failed shared plans create memories and relationship changes.
- Mods can weight leisure choices and propose plans, but cannot place reservations or change priorities directly.

### 6.7 Life cycle (Stage 7)

Adults only at first. Later: children, elders, births, move-ins (each a separate per-world switch) with a population cap. One game year equals one year of aging by default, with an aging-speed multiplier. Death is permanent in a world; resurrection is a developer tool only.

## 7. Social and dialogue

- Residents start conversations autonomously when social need, proximity, schedule and cooldowns allow.
- **The outcome of a conversation is decided by rules** (topic, tone, relationship, mood). The text is presentation. Dialogue never changes game state directly.
- Text comes from the configured LLM when available, else a rule-based fallback built from topic, mood and relationship. Packs can add topics, tones and fallback lines.
- A speech bubble appears only when a focused or possessed pawn is within hearing range.
- Possession (Stage 4) lets the player speak; options and outcomes are still rule-based.

## 8. AI usage policy

| Use | When | Constraint |
| --- | --- | --- |
| Dialogue lines | Stage 1 onward | Optional; cooldowns; fallback always present; text only |
| Action proposals | Stage 10 | Closed registry; strict JSON; validated by simulation; cooldowns and caching |

Providers: OpenAI, DeepSeek, Anthropic, OpenRouter and Player2. Players pay providers directly (Player2 players use their own Player2 account and credits and sign in with a short code in their browser; no key is pasted, and Player2 chooses the model). A recommended model per provider plus an optional custom model ID. Keys are held in the operating system's credential store (session-only where none exists), sent directly to the provider only for the request that needs them, and never written to logs, saves, exports or crash reports. Only compact, relevant context is ever sent; never the full save or unrelated residents' private data. **Mods have no access to the network, to provider keys or to the model**; a pack with the `ai` capability can only mark its own actions as proposable and contribute bounded, sanitized context data.

## 9. Content tone and filtering

- **Tone preset (per world):** Cozy, Standard (default), Mature. Presets change the *likelihood* of serious events, never the authoritative outcome of an event.
- **Graphic-content filter (global, default on):** changes descriptive text detail and visual intensity only. The underlying event record is identical either way.
- Visuals are always abstract and non-graphic.
- Crime, injury and death are fictional, rule-bound and carry in-game consequences only. Mental health and trauma are represented accurately and calmly, never as shorthand for danger.
- **These rules bind mods.** Event types a pack registers must supply every filter variant and an intensity tier, or the pack is rejected. Packs cannot disable or bypass the filter or the tone preset.

## 10. Economy, services and governance (Stages 6, 8, 9)

- **Personal economy:** wages, spending, goods, shops and inventory, traceable transactions, rent, then property transactions. A small job / shop loop comes before a full economy.
- **Needs and goods:** deterministic rules connect needs to goods and services.
- **Services:** clinic and emergency response first; others only when their simulation purpose is defined; school follows child life stages. Coverage, response, cost and legal outcomes are deterministic and explainable.
- **Mayor:** periodic elections; votes weigh reputation and resident priorities; property and business taxes fund services and construction; in-world construction is limited by budget, town rules and law.

## 11. Persistence and data

- Multiple local world slots in the operating system's user-data folder; autosave, focus-loss, close and manual saves; corruption recovery that never destroys another save. Each save keeps the previous generation as a fallback.
- Versioned schemas with migrations. Entities have stable IDs.
- Manual JSON export / import for worlds, pawn rosters, object definitions and custom towns, with validation reports.
- A world records which packs (and exact versions and content hashes) it was made with. If a pack is missing or changed, the game explains what differs and offers to continue with a compatibility report or open the world in **safe mode** (scripts off). A pack's data is never deleted from a save just because the pack is absent.
- Local content-pack manager (Stage 1 basics, Stage 11 full).

## 12. Modding and content packs

### 12.1 Goals

Playground should be easy to extend and hard to break. Players and creators should be able to add new things to the town, change how systems behave within sensible limits, and share the result as a pack, without ever being able to compromise another player's computer, save or keys.

### 12.2 What a pack can be

- **Data pack:** JSON templates, tables, towns, layouts, dialogue tables, localization and assets. Needs no scripting.
- **Script pack:** a data pack plus Luau scripts that use the **modding API**. Examples: a new need or status (caffeine, boredom), a new object with a custom interaction, a new occupation with its own routine rules, a random-event category, a generator stage that places a new kind of district, a hook that makes certain residents more talkative, an inspector panel that shows a pack's custom data.
- The **base game is itself a pack**, built with the same format and API, so what modders see is what the game uses.

### 12.3 The modding API in one paragraph

One versioned, documented API (the `pg` namespace) exposes the engine's extension points: components, systems, actions, events, bounded value hooks, content registration, generator stages and read-only world queries. It is described once in a single source of truth that also produces the type definitions, reference docs, capability checks and lint rules. Scripts read a consistent view of the world and request changes as typed commands and effects; the simulation validates every one exactly as it validates built-in behavior (Blueprint §23).

### 12.4 Boundaries

| Mods can | Mods cannot |
| --- | --- |
| Add components, needs, objects, occupations, actions, events, topics, towns, generator stages | Read or write files, use the network, run native code, or see provider keys |
| Adjust weights and rates through bounded hooks | Override hard rules (priorities, containment, ledger sums, permissions, filter, tone) |
| Request changes to the world through validated effects | Write world state directly or bypass preconditions and limits |
| Read world state they have the capability to read | Read other packs' private state or the player's save outside the API |
| Add inspector sections as data | Draw to the screen, capture input or open windows |

### 12.5 Player trust and safety

- Packs declare **capabilities** (`read`, `data`, `systems`, `world-write`, `worldgen`, `ai`, `dev`); the player approves them when installing or updating a pack.
- Scripts run in a **sandbox** with deterministic CPU and memory limits. A script that errors, loops or runs out of memory is stopped; repeated failure **quarantines** the pack while the world keeps running.
- **Safe mode** opens any world with scripts off.
- Scripts are deterministic by contract (integers across the API, seeded randomness, no wall-clock), so seeded towns reproduce identically with the same packs and replays stay valid.
- **Honest limits:** the sandbox is a strong containment layer inside the game process but not a guarantee against every possible bug. For wider distribution of untrusted packs, an optional out-of-process script host is evaluated at Stage 11.

### 12.6 Explainability and quality

- Any decision a pack influences is attributed to that pack in the resident inspector.
- Errors name the pack, file, line and extension point, and appear in the Mods screen.
- Tools: type definitions and a linter, a headless test and replay runner, hot reload in developer mode, sample packs.
- Friendly errors: validation messages suggest the closest known name ("did you mean…?"); pack authors get JSON Schema for data files and type definitions for scripts, so editors validate and autocomplete as they type.
- Cookbook: a gallery of small runnable sample packs covers every part of the API and doubles as the compatibility test suite.
- Compatibility: when a pack changes, a diff tool and a load-time report say which changes are safe for existing saves and which are save-breaking.

**Text and accessibility foundations (v3.2).** All player-visible text, including the plain-language reasons behind pawn decisions, comes from keyed string tables rather than being written into code, so translation and pack-supplied wording are possible without rewrites, and a stretched pseudo-locale is used to catch layout problems early. Menus are built as a structure of labelled controls with a defined keyboard order, which is both what automated tests check and what screen readers read. Full accessibility options remain a Stage 11 deliverable; this removes the expensive retrofit.

### 12.7 API maturity

The API is `0.x` and may change through Stage 10 (documented, with lint migration hints). It freezes at `1.0` in Stage 11, with a deprecation policy. Early packs declare the API range they target; a pack built for an incompatible range is refused with a clear message.

### 12.8 Distribution

Local packs only: a folder or `.pgpack` file placed in the mods folder or installed through the Mods screen. No in-game marketplace or auto-download is planned. A workshop-style channel is an open decision for later.

## 13. Menus and flows

| Menu | Contents | Available from |
| --- | --- | --- |
| Continue | Load the most recent save | Stage 0 |
| New Game | New seeded world; Load Town (a saved custom design); pack selection and per-pack settings | Stage 1 / 3 |
| Saved Worlds | Slots, export, import, delete | Stage 0 |
| Town Creation | Town builder / editor | Stage 3 |
| Pawn Creation | Pawn creator and roster | Stage 5 |
| Mods | Installed packs, enable / disable, load order, capability approvals, errors, safe-mode launch (local only; no Browse Mods marketplace) | Stage 1 (basic) / 11 (full) |
| Options | General, Graphics (window mode, resolution, vsync, UI scale), Audio, Controls (rebindable), LLM Configuration | Stage 0 (LLM, General); rest by Stage 11 |
| Quit | Exit with a final save | Stage 0 |

## 14. Tools

### 14.1 Town builder (Stage 3)

Graphical editor: toolbar on top, viewport in the center, selected-tool parameters on the right. Tools: **Generate** (click to generate a town from current parameters), Landscape, Road, Plot, Building / object placer, Copy and Paste selection. Validation runs before a town may start. Designed for keyboard and mouse, with undo / redo and hotkeys.

### 14.2 Pawn creator

First version: name, appearance, occupation, a few traits. The full editor (sliders, low-level attributes) waits for playtest evidence.

### 14.3 Resident inspector (Stage 1)

Shows current activity, mood, needs, relevant memories and relationship label, with pack-contributed sections. It doubles as the primary explainability tool.

### 14.4 Developer / debug tools (Stage 11, hooks earlier)

World: spawn and destroy objects, spawn pawn, toggle pathfinding display. Pawns: full heal, resurrect, injure (random or by part), add / remove items, view / add / remove memories, reset occupation. Scripting: pack inspector (registrations, per-pack time and memory, errors), script console, hot reload, per-pack log viewer. Destructive actions need confirmation. Reproducibility and explanation: replay diff and bisect to locate any divergence, a **bug bundle** (snapshot, inputs, content versions) that lets any odd behavior be replayed exactly, scenario files that make acceptance checks executable, and a reason-code explorer that answers "why did this resident do that?".

**Added in v3.2.** The event viewer lists events from a declared catalog (kind, fields, visibility); the overlay can step back to an earlier keyframe and replay forward deterministically; an unexpected crash or a detected divergence leaves a redacted, replayable bug bundle that the next launch offers to open; and in soak runs a second worker re-simulates stretches with a different thread count to prove the results match.

## 15. Component map

| Component | Responsibility | Stage |
| --- | --- | --- |
| Simulation core | Time, state, rules, determinism | 0 |
| Content system and pack loader | Templates, components, objects, packs, validation | 0 |
| Script host and API | Luau sandbox, metering, unified `pg` API, capabilities | 0 (spike) / grows |
| Persistence | Slots, migrations, import / export | 0 |
| Host services | Files, credential store, HTTPS, native dialogs, audio | 0 |
| AI client | Provider access, key handling, fallback | 0 |
| Spatial system | Maps, pathfinding, occupancy | 0 |
| Needs / mood | Drives behavior | 1 |
| Memory / relationships | Social history | 1 |
| Scheduler | Reservations, commitments | 0–2 |
| Conversation system | Rules + dialogue text | 1 |
| Town generator / editor | Seeded towns, creative editing | 1 / 3 |
| Presentation | Rendering, UI, camera, bubbles | 1 |
| Control system | Possession, player commands | 4 |
| Economy / property / interiors | Jobs, money, buildings | 6 |
| Life cycle / psychology | Aging, traits, trauma | 7 |
| Events / services | Crime, injury, civic services | 8 |
| Governance | Elections, taxes, construction | 9 |
| Action registry / proposals | Bounded LLM actions | 10 |
| Pack manager, API 1.0 and tooling, dev tools | Distribution, authoring and debugging | 11 |

## 16. Success criteria

- A seeded town reproduces identically from the same seed and inputs, with the same packs, on Windows, Linux and macOS.
- Any visible behavior can be explained by inspecting state, including behavior influenced by a pack.
- The game is fully playable with no AI key and no network.
- No key appears in any log, save, export or crash report.
- Every milestone passes its acceptance checks on a **saved and reloaded** world.
- A pack that errors, loops forever or exhausts memory cannot crash the game or corrupt a save; a world always opens in safe mode.
- The base game runs on the same pack format and API that players use.
- The simulation meets its performance targets on the minimum-spec desktop (resident target set from profiling).

## 17. Reconciliation: original design doc vs. roadmap vs. v3.0

The roadmap refined the original design document; v3.0 supersedes both where it changes direction (platform and modding).

| Topic | Original design doc | Roadmap v2 / v3 | Decision (v3.0) |
| --- | --- | --- | --- |
| LLM role | LLM infers each pawn's next action | Rule-based simulation decides; LLM writes dialogue, and in Stage 10 proposes validated actions | Unchanged |
| Interiors | Procedurally generated on entry | Reusable hand-authored layouts, persisted; exterior-only first | Unchanged |
| Occupations | Employed / minor / retired / unemployed with hours | Archetypes first; functional jobs only after templates stabilize (Stage 6) | Unchanged |
| Schedule slots | Fixed 30-minute segments | 30 minutes is the starting default; tuned in playtest | Unchanged |
| Priority order | Five tiers, 1 highest, undefined contents | Defined: work → urgent needs → commitments → chores / goals → leisure, with move and tie rules | Unchanged; mods cannot alter priorities |
| Mods | Browse and manage mods | Local data packs only; no executable code or marketplace | **Sandboxed Luau scripts in local packs through one API; no marketplace; base game is a pack** |
| Menu | Includes Browse Mods, Controls, etc. | Phased in | Phased in as in §13; Mods screen is local-only |
| Memory model | Types, severity, impact, importance, decay | Kept as the basis | Unchanged |
| Player freedom | Serial killer, monopoly, etc. from the start | Staged: crime in Stage 8, economy in Stage 6, with presets and filtering | Unchanged |
| Debug tools | Listed | Delivered in Stage 11; resurrection is developer-only | Unchanged; adds scripting tools |
| Tick model | Constant tick rate tied to town time | Kept; time paces 20–30 min days with pause / speed | Unchanged |
| Platform | Not stated | First target sideloaded handheld; platform-agnostic design with web and desktop later | **Native desktop binary (Rust). Windows first; Linux and macOS in CI from Stage 0. No handheld, web or mobile targets** |
| Technology | Not stated | TypeScript prototype; framework undecided | **Rust workspace; Luau for mod scripting; wgpu renderer; egui UI** |
| AI access | Provider options | Hosted or in-process gateway | **Direct from the desktop app to the provider; no gateway; OS credential store** |
| Resident scale | Not stated | About 50 on the handheld reference device | **About 200 on minimum-spec desktop, stretch 500; from profiling** |
| Property | City-owned civic plots, purchasable others | Kept; applied from Stage 6 | Unchanged |
| Players | Not stated | Single-player only | Unchanged |

## 18. Open tuning items

Slot size and activity durations; mood list and label thresholds; conversation frequency; memory retention and decay; Cozy / Standard / Mature event rates; election cadence; tax formula; service coverage and cost; resident cap and minimum desktop specification; art palette and sound scope; whether mental-health content needs outside review; script fuel and memory budgets and quarantine thresholds; hook clamp ranges and combiners; the Luau binding approach (initially `mlua` behind a replaceable boundary, settled by the Stage 0 spike); whether to isolate the script host in a separate process; whether to add a workshop-style distribution channel later.

## 19. Glossary

- **Pawn / resident:** a simulated person.
- **Reservation:** a claim on a schedule slot for a specific purpose.
- **Commitment:** a reservation shared between two pawns, created by proposal and acceptance.
- **Observation mode:** the default mode in which the player does not control any pawn.
- **Possession:** direct player control of an existing pawn.
- **Preset:** Cozy / Standard / Mature, set per world.
- **Graphic-content filter:** global presentation setting.
- **Content pack:** a validated bundle of data (templates, towns, rosters, assets), optionally with scripts. The base game is one.
- **Script pack:** a content pack that includes Luau scripts using the modding API.
- **Luau:** the sandboxed scripting language used for mods.
- **Modding API (`pg`):** the single versioned interface through which scripts extend the engine.
- **Capability:** a declared permission a pack needs (for example `systems`, `world-write`), approved by the player.
- **Hook:** a bounded extension point where a pack can nudge a value, which the engine clamps.
- **Quarantine:** disabling a misbehaving pack without harming the world.
- **Safe mode:** opening a world with all scripts off.
- **Host services:** the thin layer that gives the game access to files, the credential store, the network and native dialogs.
