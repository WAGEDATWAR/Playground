//! Single source of truth for the scripting API surface (Blueprint §23.1, §23.7, §23.11, §23.12).
//!
//! Pure data, no VM and no dependencies. Everything that must agree about the API reads it from here:
//! the Luau host installs only what a pack's capabilities allow, the core takes hook clamps and combiners
//! from [`HOOK_POINTS`], `pg pack lint` checks API use and "did you mean" hints, and the generators below
//! produce `pg.d.luau` and the reference documentation. There is no second, hand-maintained surface.

mod hints;

pub use hints::{closest, edit_distance, hint};

/// The API version a build offers. `0.x` is unstable through Stage 10 (Blueprint §23.11).
pub const API_VERSION: (u32, u32) = (0, 1);

/// A capability a pack declares and the player approves (Blueprint §23.2).
pub const CAPABILITIES: [&str; 7] = [
    "read",
    "data",
    "systems",
    "world-write",
    "worldgen",
    "ai",
    "dev",
];

/// When a function may be called.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Only while the entry script runs (registration); afterwards the registries are closed.
    Load,
    /// Any time.
    Any,
}

/// One function of the `pg` namespace.
#[derive(Copy, Clone, Debug)]
pub struct ApiFn {
    /// The module path below `pg`, empty for `pg.<name>` itself (for example `components`).
    pub module: &'static str,
    pub name: &'static str,
    /// The capability needed, or `None` when any pack may call it.
    pub capability: Option<&'static str>,
    pub phase: Phase,
    pub since: &'static str,
    pub deprecated_in: Option<&'static str>,
    /// The Luau function type, as written in `pg.d.luau`.
    pub signature: &'static str,
    pub summary: &'static str,
}

impl ApiFn {
    /// `pg.components.register`.
    pub fn path(&self) -> String {
        if self.module.is_empty() {
            format!("pg.{}", self.name)
        } else {
            format!("pg.{}.{}", self.module, self.name)
        }
    }
}

/// A named Luau type used by signatures.
#[derive(Copy, Clone, Debug)]
pub struct ApiType {
    pub name: &'static str,
    pub definition: &'static str,
}

pub const TYPES: &[ApiType] = &[
    ApiType {
        name: "FieldSpec",
        definition: "{ kind: \"int\", min: number, max: number, default: number }",
    },
    ApiType {
        name: "ComponentSpec",
        definition: "{ name: string, applies_to: { string }, fields: { [string]: FieldSpec }, version: number? }",
    },
    ApiType {
        name: "Query",
        definition: "{ kind: string, with: { string }? }",
    },
    ApiType {
        name: "Command",
        definition: "{ op: string, entity: string, component: string, field: string, value: number }",
    },
    ApiType {
        name: "CommandBuffer",
        definition: "{ set_field: (self: CommandBuffer, entity: string, component: string, field: string, value: number) -> () }",
    },
    ApiType {
        name: "Entity",
        definition: "{ id: string, kind: string, x: number, y: number, needs: { [string]: number }?, mood: string?, outgoing: number?, occupation: string?, get: (self: Entity, component: string) -> { [string]: number }? }",
    },
    ApiType {
        name: "Context",
        definition: "{ tick: number, day: number, minute: number, cmd: CommandBuffer }",
    },
    ApiType {
        name: "SystemSpec",
        definition: "{ id: string, cadence: string, after: string?, query: Query, writes: { string }?, run: (ctx: Context, entity: Entity) -> () }",
    },
];

/// Every function in API 0.1.
pub const FUNCTIONS: &[ApiFn] = &[
    ApiFn {
        module: "pack",
        name: "id",
        capability: Some("read"),
        phase: Phase::Any,
        since: "0.1",
        deprecated_in: None,
        signature: "() -> string",
        summary: "The id of the running pack.",
    },
    ApiFn {
        module: "components",
        name: "register",
        capability: Some("data"),
        phase: Phase::Load,
        since: "0.1",
        deprecated_in: None,
        signature: "(spec: ComponentSpec) -> ()",
        summary: "Declares a component stored as `<pack>.<name>`; its fields are saved, hashed and validated by the core.",
    },
    ApiFn {
        module: "systems",
        name: "register",
        capability: Some("systems"),
        phase: Phase::Load,
        since: "0.1",
        deprecated_in: None,
        signature: "(spec: SystemSpec) -> ()",
        summary: "Registers a system the pipeline runs on the given cadence for every entity matching the query.",
    },
    ApiFn {
        module: "hooks",
        name: "on",
        capability: Some("data"),
        phase: Phase::Load,
        since: "0.1",
        deprecated_in: None,
        signature: "(point: string, fn: (ctx: { [string]: any }) -> number) -> ()",
        summary: "Adds a bounded value hook; the engine combines the packs' answers and clamps the result.",
    },
    ApiFn {
        module: "field",
        name: "int",
        capability: Some("data"),
        phase: Phase::Any,
        since: "0.1",
        deprecated_in: None,
        signature: "(min: number, max: number, default: number) -> FieldSpec",
        summary: "An integer field with a range and a default.",
    },
    ApiFn {
        module: "",
        name: "rand",
        capability: None,
        phase: Phase::Any,
        since: "0.1",
        deprecated_in: None,
        signature: "(stream: string, ...: number | string) -> number",
        summary: "A deterministic 32-bit draw on the pack's stream, a pure function of the world seed, the stream and the keys.",
    },
    ApiFn {
        module: "math",
        name: "idiv",
        capability: None,
        phase: Phase::Any,
        since: "0.1",
        deprecated_in: None,
        signature: "(a: number, b: number) -> number",
        summary: "Integer division rounding toward negative infinity; errors on a zero divisor.",
    },
    ApiFn {
        module: "math",
        name: "muldiv",
        capability: None,
        phase: Phase::Any,
        since: "0.1",
        deprecated_in: None,
        signature: "(a: number, b: number, c: number) -> number",
        summary: "`a * b / c` in exact integer arithmetic, rounding half up.",
    },
    ApiFn {
        module: "math",
        name: "isqrt",
        capability: None,
        phase: Phase::Any,
        since: "0.1",
        deprecated_in: None,
        signature: "(n: number) -> number",
        summary: "The integer square root, rounded down.",
    },
    ApiFn {
        module: "math",
        name: "lerp_permille",
        capability: None,
        phase: Phase::Any,
        since: "0.1",
        deprecated_in: None,
        signature: "(a: number, b: number, t: number) -> number",
        summary: "Interpolates from `a` to `b` by `t` thousandths.",
    },
    ApiFn {
        module: "log",
        name: "info",
        capability: None,
        phase: Phase::Any,
        since: "0.1",
        deprecated_in: None,
        signature: "(message: string) -> ()",
        summary: "Writes to the pack's log (rate-limited).",
    },
    ApiFn {
        module: "log",
        name: "warn",
        capability: None,
        phase: Phase::Any,
        since: "0.1",
        deprecated_in: None,
        signature: "(message: string) -> ()",
        summary: "Writes a warning to the pack's log (rate-limited).",
    },
];

/// How several packs' answers to one hook combine before the engine's clamp (Blueprint §23.9).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Combiner {
    Sum,
    /// Values are permille factors; the result is their product in permille, rounded half up.
    ProductPermille,
    Min,
    Max,
    /// The first pack in load order that answers.
    FirstWins,
}

impl Combiner {
    pub const fn name(self) -> &'static str {
        match self {
            Combiner::Sum => "sum",
            Combiner::ProductPermille => "product-permille",
            Combiner::Min => "min",
            Combiner::Max => "max",
            Combiner::FirstWins => "first-wins",
        }
    }

    /// Combines `values` (in pack load order). With no answers the result is `default`.
    pub fn combine(self, values: &[i64], default: i64) -> i64 {
        let Some(first) = values.first().copied() else {
            return default;
        };
        match self {
            Combiner::Sum => values.iter().fold(0i64, |a, v| a.saturating_add(*v)),
            Combiner::ProductPermille => values.iter().fold(1000i64, |a, v| {
                let p = i128::from(a) * i128::from(*v);
                let r = (p + if p >= 0 { 500 } else { -500 }) / 1000;
                i64::try_from(r).unwrap_or(if r < 0 { i64::MIN } else { i64::MAX })
            }),
            Combiner::Min => values.iter().copied().min().unwrap_or(default),
            Combiner::Max => values.iter().copied().max().unwrap_or(default),
            Combiner::FirstWins => first,
        }
    }
}

/// A place where the engine asks packs for a bounded value (Blueprint §23.1 item 6: bias, never override).
#[derive(Copy, Clone, Debug)]
pub struct HookPoint {
    pub id: &'static str,
    pub combiner: Combiner,
    /// The engine clamps the combined answer to `min..=max`.
    pub min: i64,
    pub max: i64,
    /// What the engine uses when no pack answers.
    pub default: i64,
    pub since: &'static str,
    pub summary: &'static str,
    /// The fields of the context table the hook receives.
    pub context: &'static str,
}

impl HookPoint {
    /// Combines and clamps.
    pub fn resolve(&self, answers: &[i64]) -> i64 {
        self.combiner
            .combine(answers, self.default)
            .clamp(self.min, self.max)
    }
}

/// Every hook point in API 0.1.
pub const HOOK_POINTS: &[HookPoint] = &[
    HookPoint {
    id: "movement.speed_modifier",
    combiner: Combiner::ProductPermille,
    min: 500,
    max: 1500,
    default: 1000,
    since: "0.1",
    summary: "Scales how fast a pawn walks, in permille (1000 is normal, 1500 is one and a half times as fast).",
    context: "pawn: Entity",
},
HookPoint {
    id: "memory.importance_modifier",
    combiner: Combiner::ProductPermille,
    min: 500,
    max: 2000,
    default: 1000,
    since: "0.1",
    summary: "Scales how important a new memory is to the pawn who forms it, in permille (1000 leaves it alone). Important memories fade more slowly and the most important never fade.",
    context: "pawn: Entity",
},
HookPoint {
    id: "relationship.delta_modifier",
    combiner: Combiner::ProductPermille,
    min: 0,
    max: 2000,
    default: 1000,
    since: "0.1",
    summary: "Scales the change a conversation makes to a relationship, in permille (1000 leaves it alone, 0 cancels it). The pair's daily cap still applies afterwards.",
    context: "pawn: Entity (the lower id of the pair)",
},
HookPoint {
    id: "need.decay_modifier",
    combiner: Combiner::ProductPermille,
    min: 500,
    max: 2000,
    default: 1000,
    since: "0.1",
    summary: "Scales how fast all of a pawn's needs fall, in permille (1000 is normal, 700 means the pawn needs to eat, sleep and socialise less often, 1500 more often). Asked once a game minute for each pawn.",
    context: "pawn: Entity",
},
HookPoint {
    id: "mood.comfort_shift",
    combiner: Combiner::Sum,
    min: -300,
    max: 300,
    default: 0,
    since: "0.1",
    summary: "Added to each need level the mood rules look at, so a positive answer makes a pawn feel better off than its needs say and a negative one worse off. Needs and their effects on what a pawn can do are unchanged; only the mood is. Asked once a game minute for each pawn.",
    context: "pawn: Entity",
},
HookPoint {
    id: "conversation.chance_modifier",
    combiner: Combiner::ProductPermille,
    min: 0,
    max: 2000,
    default: 1000,
    since: "0.1",
    summary: "Scales the chance that two residents who are free to talk start a conversation, in permille (0 means never, 2000 twice as likely). Asked for each pair that could talk, once a game minute.",
    context: "pawn: Entity (the lower id of the pair)",
}];

pub fn hook_point(id: &str) -> Option<&'static HookPoint> {
    HOOK_POINTS.iter().find(|h| h.id == id)
}

/// The cadences a system may use.
pub const CADENCES: [&str; 4] = ["tick", "minute", "slot", "day"];

/// The built-in slots a system can be anchored to with `after`, by name.
pub const ANCHORS: [&str; 13] = [
    "NeedsSystem",
    "MoodSystem",
    "DayPlanner",
    "CommitmentSystem",
    "ReservationActivator",
    "TaskPlanner",
    "MovementSystem",
    "ActivitySystem",
    "ConversationSystem",
    "MemorySystem",
    "RelationshipSystem",
    "EventSystem",
    "Maintenance",
];

pub fn find_function(module: &str, name: &str) -> Option<&'static ApiFn> {
    FUNCTIONS
        .iter()
        .find(|f| f.module == module && f.name == name)
}

/// All function paths (`pg.components.register`), for hints.
pub fn function_paths() -> Vec<String> {
    FUNCTIONS.iter().map(ApiFn::path).collect()
}

/// The hint for an unknown `pg` path, such as `pg.component.register`.
pub fn unknown_path_hint(path: &str) -> String {
    let all = function_paths();
    hint(path, all.iter().map(String::as_str))
}

/// The functions a pack with `capabilities` may use.
pub fn allowed_functions<'a>(
    capabilities: impl IntoIterator<Item = &'a str> + Clone,
) -> Vec<&'static ApiFn> {
    FUNCTIONS
        .iter()
        .filter(|f| {
            f.capability
                .is_none_or(|c| capabilities.clone().into_iter().any(|have| have == c))
        })
        .collect()
}

/// `pg.d.luau`: Luau type definitions for editors (Blueprint §23.12), generated from this crate.
pub fn render_luau_defs() -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "--!strict\n-- Generated from pg-api (API {}.{}). Do not edit; regenerate with `pg pack docs --luau`.\n\n",
        API_VERSION.0, API_VERSION.1
    ));
    for t in TYPES {
        out.push_str(&format!("export type {} = {}\n", t.name, t.definition));
    }
    out.push_str("\ndeclare pg: {\n");
    let mut modules: Vec<&str> = FUNCTIONS.iter().map(|f| f.module).collect();
    modules.sort_unstable();
    modules.dedup();
    for m in &modules {
        if m.is_empty() {
            continue;
        }
        out.push_str(&format!("    {m}: {{\n"));
        for f in FUNCTIONS.iter().filter(|f| f.module == *m) {
            out.push_str(&format!("        -- {}\n", f.summary));
            out.push_str(&format!("        {}: {},\n", f.name, f.signature));
        }
        out.push_str("    },\n");
    }
    for f in FUNCTIONS.iter().filter(|f| f.module.is_empty()) {
        out.push_str(&format!("    -- {}\n", f.summary));
        out.push_str(&format!("    {}: {},\n", f.name, f.signature));
    }
    out.push_str("}\n");
    out
}

/// Reference documentation in Markdown, generated from this crate.
pub fn render_docs() -> String {
    let mut out = format!(
        "# The `pg` API {}.{}\n\nGenerated from `pg-api`. The API is unstable (`0.x`) until Stage 11.\n\n## Functions\n\n| Function | Capability | When | Since | Description |\n| --- | --- | --- | --- | --- |\n",
        API_VERSION.0, API_VERSION.1
    );
    for f in FUNCTIONS {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} |\n",
            f.path(),
            f.capability.unwrap_or("none"),
            if f.phase == Phase::Load {
                "load only"
            } else {
                "any time"
            },
            f.since,
            f.summary
        ));
    }
    out.push_str("\n## Hook points\n\n| Point | Combiner | Range | Default | Description |\n| --- | --- | --- | --- | --- |\n");
    for h in HOOK_POINTS {
        out.push_str(&format!(
            "| `{}` | {} | {}..{} | {} | {} Context: `{}`. |\n",
            h.id,
            h.combiner.name(),
            h.min,
            h.max,
            h.default,
            h.summary,
            h.context
        ));
    }
    out.push_str("\n## Capabilities\n\n");
    for c in CAPABILITIES {
        out.push_str(&format!("- `{c}`\n"));
    }
    out
}

#[cfg(test)]
mod tests;
