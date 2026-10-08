//! The Luau implementation of [`ScriptVm`], over `mlua` (Blueprint §23.4, §23.15).
//!
//! This is the **only** file in the workspace that names the binding crate. The sandbox profile is built in
//! three layers: the standard libraries that are opened at all (no `io`, `os`, `debug`, `package`,
//! `coroutine`, `vector`), the trusted `prelude.luau` that trims and replaces what remains, and Luau's own
//! sandbox mode that seals the library tables read-only. The `pg` table is built here from `pg-api`, so a
//! pack sees exactly the functions its capabilities allow and nothing else.

use crate::vm::{
    Cadence, CallResult, ComponentSpec, FieldSpec, Fuel, HandlerId, HookSpec, LogLevel,
    Registrations, ScriptCommand, ScriptVm, SourceChunk, SystemSpec, Val, VmConfig, VmError,
    MAX_DEPTH, MAX_INT, MAX_ITEMS, MAX_STRING,
};
use mlua::chunk::{ChunkMode, Compiler};
use mlua::{Error, Function, Lua, LuaOptions, RegistryKey, StdLib, Table, Value, VmState};
use pg_api::{Phase, ANCHORS, CADENCES};
use pg_core::rng::{Key, Rng, Seed, Stream};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const PRELUDE: &str = include_str!("prelude.luau");
/// Longest source file a pack may stage.
pub const MAX_SOURCE_BYTES: usize = 512 * 1024;
const MAX_LOG_LINES_PER_CALL: usize = 16;
const MAX_LOG_LINE: usize = 256;
const MAX_COMMANDS: usize = 1024;
/// Globals the prelude replaces: the compiler must not treat them as the builtins.
const MUTABLE_GLOBALS: [&str; 5] = ["pairs", "setmetatable", "tostring", "print", "require"];
/// Builtin fast-calls switched off: everything the prelude replaces or removes.
const DISABLED_BUILTINS: [&str; 33] = [
    "pairs",
    "setmetatable",
    "tostring",
    "print",
    "require",
    "math.acos",
    "math.asin",
    "math.atan2",
    "math.atan",
    "math.cosh",
    "math.cos",
    "math.deg",
    "math.exp",
    "math.fmod",
    "math.frexp",
    "math.ldexp",
    "math.log10",
    "math.log",
    "math.modf",
    "math.pow",
    "math.rad",
    "math.sinh",
    "math.sin",
    "math.sqrt",
    "math.tanh",
    "math.tan",
    "math.round",
    "math.lerp",
    "math.map",
    "math.noise",
    "math.random",
    "math.randomseed",
    "math.pi",
];
const COMPONENT_KINDS: [&str; 3] = ["pawn", "object", "map"];

/// What the host and the interrupt callback share about fuel.
struct FuelState {
    used: AtomicU64,
    /// The absolute `used` value at which the current call runs out.
    limit: AtomicU64,
    /// The budget the current call was given, for error messages.
    budget: AtomicU64,
    exhausted: AtomicBool,
}

#[derive(Default)]
struct Shared {
    /// `true` only while the entry script runs; registration functions refuse otherwise.
    loading: bool,
    reg: Registrations,
    handlers: Vec<RegistryKey>,
    /// Sources by path (`scripts/main.luau`) and by module name (`scripts/main`).
    sources: BTreeMap<String, String>,
    log: Vec<(LogLevel, String)>,
}

pub struct LuauVm {
    lua: Lua,
    cfg: VmConfig,
    shared: Arc<Mutex<Shared>>,
    fuel: Arc<FuelState>,
    invoke: Option<Function>,
}

fn lock(s: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

fn rt(msg: impl Into<String>) -> Error {
    Error::runtime(msg.into())
}

fn root_cause(e: &Error) -> &Error {
    match e {
        Error::CallbackError { cause, .. } => root_cause(cause),
        _ => e,
    }
}

/// Luau appends a traceback to runtime errors; the first lines (`file:line: message`) are what authors
/// need, so the traceback is dropped from the message.
fn trim_traceback(m: &str) -> String {
    m.split("\nstack traceback:")
        .next()
        .unwrap_or(m)
        .trim()
        .to_owned()
}

// ---- value conversion (the integers-only boundary lives here) ------------------------------------------------

fn int_in_range(i: i64) -> Result<i64, VmError> {
    if i.unsigned_abs() > MAX_INT as u64 {
        return Err(VmError::NonInteger(format!("{i} is out of range")));
    }
    Ok(i)
}

fn number_to_int(n: f64) -> Result<i64, VmError> {
    if !n.is_finite() {
        return Err(VmError::NonInteger(format!("{n}")));
    }
    if n.fract() != 0.0 {
        return Err(VmError::NonInteger(format!("{n}")));
    }
    if n.abs() > MAX_INT as f64 {
        return Err(VmError::NonInteger(format!("{n} is out of range")));
    }
    Ok(n as i64)
}

fn lua_to_val(v: &Value, depth: usize, budget: &mut usize) -> Result<Val, VmError> {
    if depth > MAX_DEPTH {
        return Err(VmError::BadValue(format!(
            "nested more than {MAX_DEPTH} levels deep"
        )));
    }
    match v {
        Value::Nil => Ok(Val::Nil),
        Value::Boolean(b) => Ok(Val::Bool(*b)),
        Value::Integer(i) => Ok(Val::Int(int_in_range(*i)?)),
        Value::Number(n) => Ok(Val::Int(number_to_int(*n)?)),
        Value::String(s) => {
            let text = s
                .to_str()
                .map_err(|_| VmError::BadValue("text that is not valid UTF-8".into()))?;
            if text.len() > MAX_STRING {
                return Err(VmError::BadValue(format!(
                    "text longer than {MAX_STRING} bytes"
                )));
            }
            Ok(Val::Str(text.to_string()))
        }
        Value::Table(t) => {
            let mut arr: Vec<(i64, Val)> = Vec::new();
            let mut map: Vec<(String, Val)> = Vec::new();
            for pair in t.pairs::<Value, Value>() {
                let (k, val) = pair.map_err(|e| VmError::BadValue(e.to_string()))?;
                if *budget == 0 {
                    return Err(VmError::BadValue(format!(
                        "more than {MAX_ITEMS} items in one value"
                    )));
                }
                *budget -= 1;
                let inner = lua_to_val(&val, depth + 1, budget)?;
                match k {
                    Value::Integer(i) => arr.push((int_in_range(i)?, inner)),
                    Value::Number(n) => arr.push((number_to_int(n)?, inner)),
                    Value::String(s) => {
                        let key = s.to_str().map_err(|_| {
                            VmError::BadValue("a key that is not valid UTF-8".into())
                        })?;
                        map.push((key.to_string(), inner));
                    }
                    other => {
                        return Err(VmError::BadValue(format!(
                            "a {} used as a key",
                            other.type_name()
                        )))
                    }
                }
            }
            match (arr.is_empty(), map.is_empty()) {
                (true, true) => Ok(Val::List(Vec::new())),
                (false, true) => {
                    arr.sort_by_key(|(i, _)| *i);
                    for (n, (i, _)) in arr.iter().enumerate() {
                        if *i != n as i64 + 1 {
                            return Err(VmError::BadValue(
                                "a list with holes or non-positive indexes".into(),
                            ));
                        }
                    }
                    Ok(Val::List(arr.into_iter().map(|(_, v)| v).collect()))
                }
                (true, false) => {
                    map.sort_by(|a, b| a.0.cmp(&b.0));
                    Ok(Val::Map(map))
                }
                (false, false) => Err(VmError::BadValue(
                    "a table that mixes list and map entries".into(),
                )),
            }
        }
        other => Err(VmError::BadValue(format!(
            "a {} cannot cross the script boundary",
            other.type_name()
        ))),
    }
}

fn to_val(v: &Value) -> Result<Val, VmError> {
    let mut budget = MAX_ITEMS;
    lua_to_val(v, 0, &mut budget)
}

fn val_to_lua(lua: &Lua, v: &Val) -> mlua::Result<Value> {
    Ok(match v {
        Val::Nil => Value::Nil,
        Val::Bool(b) => Value::Boolean(*b),
        Val::Int(i) => Value::Number(*i as f64),
        Val::Str(s) => Value::String(lua.create_string(s)?),
        Val::List(items) => {
            let t = lua.create_table()?;
            for (i, item) in items.iter().enumerate() {
                t.raw_set(i + 1, val_to_lua(lua, item)?)?;
            }
            Value::Table(t)
        }
        Val::Map(entries) => {
            let t = lua.create_table()?;
            for (k, item) in entries {
                t.raw_set(k.as_str(), val_to_lua(lua, item)?)?;
            }
            Value::Table(t)
        }
    })
}

// ---- strict parsing of registrations ---------------------------------------------------------------------------

fn is_name(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The entries of a map value, refusing anything outside `known` (with a hint for the closest name).
fn entries<'a>(v: &'a Val, what: &str, known: &[&str]) -> Result<Vec<(&'a str, &'a Val)>, Error> {
    let items: Vec<(&str, &Val)> = match v {
        Val::Map(m) => m.iter().map(|(k, v)| (k.as_str(), v)).collect(),
        Val::List(l) if l.is_empty() => Vec::new(),
        _ => return Err(rt(format!("{what} must be a table"))),
    };
    for (k, _) in &items {
        if !known.contains(k) {
            return Err(rt(format!(
                "{what} has no field '{k}'{} (known: {})",
                pg_api::hint(k, known.iter().copied()),
                known.join(", ")
            )));
        }
    }
    Ok(items)
}

fn field<'a>(items: &[(&'a str, &'a Val)], name: &str) -> Option<&'a Val> {
    items.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
}

fn string_list(v: Option<&Val>, what: &str) -> Result<Vec<String>, Error> {
    match v {
        None | Some(Val::Nil) => Ok(Vec::new()),
        Some(Val::List(l)) => l
            .iter()
            .map(|i| {
                i.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| rt(format!("{what} must be a list of text")))
            })
            .collect(),
        Some(_) => Err(rt(format!("{what} must be a list of text"))),
    }
}

fn parse_component(v: &Val) -> Result<ComponentSpec, Error> {
    let items = entries(
        v,
        "a component",
        &["name", "applies_to", "fields", "version"],
    )?;
    let name = field(&items, "name")
        .and_then(Val::as_str)
        .ok_or_else(|| rt("a component needs a text `name`"))?;
    if !is_name(name, 32) {
        return Err(rt(format!(
            "component name '{name}' must be lower-case letters, digits and underscores, starting with a letter (at most 32)"
        )));
    }
    let applies_to = string_list(field(&items, "applies_to"), "`applies_to`")?;
    if applies_to.is_empty() {
        return Err(rt(
            "`applies_to` must name at least one kind (pawn, object, map)",
        ));
    }
    for k in &applies_to {
        if !COMPONENT_KINDS.contains(&k.as_str()) {
            return Err(rt(format!(
                "unknown kind '{k}' in `applies_to`{} (known: {})",
                pg_api::hint(k, COMPONENT_KINDS),
                COMPONENT_KINDS.join(", ")
            )));
        }
    }
    let fields_v = field(&items, "fields").ok_or_else(|| rt("a component needs `fields`"))?;
    let mut fields = Vec::new();
    match fields_v {
        Val::Map(m) => {
            for (fname, spec) in m {
                if !is_name(fname, 32) {
                    return Err(rt(format!("field name '{fname}' is not a valid name")));
                }
                let f = entries(
                    spec,
                    &format!("field '{fname}'"),
                    &["kind", "min", "max", "default"],
                )?;
                if field(&f, "kind").and_then(Val::as_str) != Some("int") {
                    return Err(rt(format!(
                        "field '{fname}' must be made with pg.field.int(min, max, default)"
                    )));
                }
                let get = |n: &str| {
                    field(&f, n)
                        .and_then(Val::as_int)
                        .ok_or_else(|| rt(format!("field '{fname}' needs an integer `{n}`")))
                };
                let (min, max, default) = (get("min")?, get("max")?, get("default")?);
                if min > max || default < min || default > max {
                    return Err(rt(format!(
                        "field '{fname}' needs min <= default <= max (got {min}, {default}, {max})"
                    )));
                }
                fields.push(FieldSpec {
                    name: fname.clone(),
                    min,
                    max,
                    default,
                });
            }
        }
        Val::List(l) if l.is_empty() => {}
        _ => return Err(rt("`fields` must be a table of field specs")),
    }
    if fields.is_empty() || fields.len() > 32 {
        return Err(rt("a component needs between 1 and 32 fields"));
    }
    let version = match field(&items, "version") {
        None | Some(Val::Nil) => 1,
        Some(Val::Int(i)) if (1..=1_000_000).contains(i) => *i as u32,
        Some(_) => return Err(rt("`version` must be a positive integer")),
    };
    Ok(ComponentSpec {
        name: name.to_owned(),
        applies_to,
        fields,
        version,
    })
}

fn parse_system(v: &Val, handler: HandlerId) -> Result<SystemSpec, Error> {
    let items = entries(
        v,
        "a system",
        &["id", "cadence", "after", "query", "writes", "reads", "run"],
    )?;
    let id = field(&items, "id")
        .and_then(Val::as_str)
        .ok_or_else(|| rt("a system needs a text `id`"))?;
    if !is_name(id, 48) {
        return Err(rt(format!(
            "system id '{id}' must be lower-case letters, digits and underscores, starting with a letter"
        )));
    }
    let cadence_text = field(&items, "cadence")
        .and_then(Val::as_str)
        .ok_or_else(|| rt("a system needs a `cadence` (tick, minute, slot or day)"))?;
    let cadence = Cadence::parse(cadence_text).ok_or_else(|| {
        rt(format!(
            "unknown cadence '{cadence_text}'{} (known: {})",
            pg_api::hint(cadence_text, CADENCES),
            CADENCES.join(", ")
        ))
    })?;
    let after = match field(&items, "after") {
        None | Some(Val::Nil) => None,
        Some(Val::Str(a)) => {
            if !ANCHORS.contains(&a.as_str()) {
                return Err(rt(format!(
                    "unknown system '{a}' in `after`{} (known: {})",
                    pg_api::hint(a, ANCHORS),
                    ANCHORS.join(", ")
                )));
            }
            Some(a.clone())
        }
        Some(_) => return Err(rt("`after` must be text")),
    };
    let query = field(&items, "query").ok_or_else(|| rt("a system needs a `query`"))?;
    let q = entries(query, "`query`", &["kind", "with"])?;
    let kind = field(&q, "kind")
        .and_then(Val::as_str)
        .ok_or_else(|| rt("`query` needs a text `kind`"))?;
    if !COMPONENT_KINDS.contains(&kind) {
        return Err(rt(format!(
            "unknown kind '{kind}' in `query`{} (known: {})",
            pg_api::hint(kind, COMPONENT_KINDS),
            COMPONENT_KINDS.join(", ")
        )));
    }
    let with = string_list(field(&q, "with"), "`with`")?;
    let writes = string_list(field(&items, "writes"), "`writes`")?;
    Ok(SystemSpec {
        id: id.to_owned(),
        cadence,
        after,
        kind: kind.to_owned(),
        with,
        writes,
        handler,
    })
}

fn parse_commands(v: &Value) -> Result<Vec<ScriptCommand>, VmError> {
    let val = to_val(v)?;
    let Val::List(list) = val else {
        return Err(VmError::BadValue("the command list is not a list".into()));
    };
    if list.len() > MAX_COMMANDS {
        return Err(VmError::BadValue(format!(
            "more than {MAX_COMMANDS} commands in one call"
        )));
    }
    let mut out = Vec::with_capacity(list.len());
    for c in &list {
        let text = |k: &str| {
            c.get(k)
                .and_then(Val::as_str)
                .map(str::to_owned)
                .ok_or_else(|| VmError::BadValue(format!("a command needs a text `{k}`")))
        };
        match c.get("op").and_then(Val::as_str) {
            Some("set_field") => {
                let value = match c.get("value") {
                    Some(Val::Int(i)) => *i,
                    _ => {
                        return Err(VmError::NonInteger(
                            "set_field needs an integer `value`".into(),
                        ))
                    }
                };
                out.push(ScriptCommand::SetField {
                    entity: text("entity")?,
                    component: text("component")?,
                    field: text("field")?,
                    value,
                });
            }
            other => return Err(VmError::BadValue(format!("unknown command {other:?}"))),
        }
    }
    Ok(out)
}

// ---- integer helpers for pg.math ----------------------------------------------------------------------------------

fn int_arg(v: &Value, what: &str) -> Result<i64, Error> {
    match v {
        Value::Integer(i) => int_in_range(*i).map_err(|e| rt(format!("{what}: {e}"))),
        Value::Number(n) => number_to_int(*n).map_err(|e| rt(format!("{what}: {e}"))),
        other => Err(rt(format!(
            "{what}: expected a number, got {}",
            other.type_name()
        ))),
    }
}

fn int_result(v: i128, what: &str) -> Result<Value, Error> {
    if v.abs() > i128::from(MAX_INT) {
        return Err(rt(format!("{what}: the result is out of range")));
    }
    Ok(Value::Number(v as f64))
}

fn floor_div(a: i128, b: i128) -> i128 {
    let q = a / b;
    if (a % b != 0) && ((a < 0) != (b < 0)) {
        q - 1
    } else {
        q
    }
}

fn isqrt(n: i128) -> i128 {
    if n < 2 {
        return n;
    }
    let mut x = (n as f64).sqrt() as i128;
    while x * x > n {
        x -= 1;
    }
    while (x + 1) * (x + 1) <= n {
        x += 1;
    }
    x
}

// ---- the VM ---------------------------------------------------------------------------------------------------------

impl LuauVm {
    pub fn new(cfg: VmConfig) -> Result<LuauVm, VmError> {
        let compile = |e: Error| VmError::Compile(trim_traceback(&e.to_string()));
        // Only the libraries a pack may see are opened at all.
        let libs = StdLib::TABLE
            | StdLib::STRING
            | StdLib::UTF8
            | StdLib::BIT
            | StdLib::MATH
            | StdLib::BUFFER;
        let lua = Lua::new_with(libs, LuaOptions::default()).map_err(compile)?;
        // One fixed, explicit compiler configuration: the same bytecode, and so the same fuel, everywhere.
        // Luau compiles direct calls to builtins into fast paths that never look at the global, and `for k, v in
        // pairs(t)` into a direct `next` loop. Both would bypass the replaced and removed functions, so they are
        // switched off for every name the prelude touches (found by the hostile-pack corpus, spike question 5).
        lua.set_compiler(
            Compiler::new()
                .set_optimization_level(1)
                .set_debug_level(1)
                .set_mutable_globals(MUTABLE_GLOBALS)
                .set_disabled_builtins(DISABLED_BUILTINS),
        );
        let shared = Arc::new(Mutex::new(Shared::default()));
        let fuel = Arc::new(FuelState {
            used: AtomicU64::new(0),
            limit: AtomicU64::new(u64::MAX),
            budget: AtomicU64::new(0),
            exhausted: AtomicBool::new(false),
        });

        // Host functions for the prelude.
        let host = lua.create_table().map_err(compile)?;
        {
            let sh = Arc::clone(&shared);
            host.set(
                "module",
                lua.create_function(move |lua, name: String| {
                    let src = lock(&sh).sources.get(&name).cloned();
                    let src =
                        src.ok_or_else(|| rt(format!("require: no module '{name}' in this pack")))?;
                    lua.load(src)
                        .set_name(name)
                        .set_mode(ChunkMode::Text)
                        .into_function()
                })
                .map_err(compile)?,
            )
            .map_err(compile)?;
            let sh = Arc::clone(&shared);
            host.set(
                "log",
                lua.create_function(move |_, (level, text): (i64, String)| {
                    push_log(&sh, level, &text);
                    Ok(())
                })
                .map_err(compile)?,
            )
            .map_err(compile)?;
        }
        let limits = lua.create_table().map_err(compile)?;
        let prelude: Function = lua
            .load(PRELUDE)
            .set_name("=prelude")
            .set_mode(ChunkMode::Text)
            .into_function()
            .map_err(compile)?;
        let invoke: Function = prelude.call((host, limits)).map_err(compile)?;

        let mut vm = LuauVm {
            lua,
            cfg,
            shared,
            fuel,
            invoke: Some(invoke),
        };
        vm.install_pg().map_err(compile)?;
        // Seal the library tables, then give scripts their own global environment.
        vm.lua.sandbox(true).map_err(compile)?;
        // Fuel: counted at every Luau safepoint (function calls and loop back-edges).
        let f = Arc::clone(&vm.fuel);
        vm.lua.set_interrupt(move |_| {
            let n = f.used.fetch_add(1, Ordering::Relaxed) + 1;
            if n > f.limit.load(Ordering::Relaxed) {
                f.exhausted.store(true, Ordering::Relaxed);
                Err(Error::runtime("the script ran out of fuel"))
            } else {
                Ok(VmState::Continue)
            }
        });
        let used = vm.lua.used_memory();
        let cap = vm.cfg.memory_bytes.max(used + 64 * 1024);
        vm.lua.set_memory_limit(cap).map_err(compile)?;
        Ok(vm)
    }

    /// Builds the `pg` table from `pg-api`: exactly the functions the pack's capabilities allow.
    fn install_pg(&mut self) -> mlua::Result<()> {
        let lua = &self.lua;
        let pg = lua.create_table()?;
        let caps: Vec<&str> = self.cfg.capabilities.iter().map(String::as_str).collect();
        for f in pg_api::allowed_functions(caps.iter().copied()) {
            let func = self.implementation(f.module, f.name, f.phase)?;
            let target = if f.module.is_empty() {
                pg.clone()
            } else {
                match pg.get::<Value>(f.module)? {
                    Value::Table(t) => t,
                    _ => {
                        let t = lua.create_table()?;
                        pg.set(f.module, t.clone())?;
                        t
                    }
                }
            };
            target.set(f.name, func)?;
        }
        // Freeze `pg` and its modules: packs cannot add to or replace the API.
        for pair in pg.pairs::<String, Value>() {
            if let (_, Value::Table(t)) = pair? {
                t.set_readonly(true);
            }
        }
        pg.set_readonly(true);
        lua.globals().set("pg", pg)?;
        Ok(())
    }

    fn implementation(&self, module: &str, name: &str, phase: Phase) -> mlua::Result<Function> {
        let lua = &self.lua;
        let shared = Arc::clone(&self.shared);
        let pack = self.cfg.pack_id.clone();
        let seed = self.cfg.seed;
        let gate = move |what: &str| -> Result<(), Error> {
            if phase == Phase::Load && !lock(&shared).loading {
                return Err(rt(format!(
                    "{what} can only be called while the pack loads: the registries are closed"
                )));
            }
            Ok(())
        };
        let shared = Arc::clone(&self.shared);
        match (module, name) {
            ("pack", "id") => lua.create_function(move |_, ()| Ok(pack.clone())),
            ("components", "register") => lua.create_function(move |_, t: Table| {
                gate("pg.components.register")?;
                let spec =
                    parse_component(&to_val(&Value::Table(t)).map_err(|e| rt(e.to_string()))?)?;
                let mut sh = lock(&shared);
                if sh.reg.components.iter().any(|c| c.name == spec.name) {
                    return Err(rt(format!("component '{}' is registered twice", spec.name)));
                }
                sh.reg.components.push(spec);
                Ok(())
            }),
            ("systems", "register") => lua.create_function(move |lua, t: Table| {
                gate("pg.systems.register")?;
                let run: Function = t
                    .get("run")
                    .map_err(|_| rt("a system needs a `run` function"))?;
                // The function cannot cross the boundary as data: take it out of the table first.
                let copy = lua.create_table()?;
                for pair in t.pairs::<String, Value>() {
                    let (k, v) = pair?;
                    if k != "run" {
                        copy.set(k, v)?;
                    }
                }
                let val = to_val(&Value::Table(copy)).map_err(|e| rt(e.to_string()))?;
                let key = lua.create_registry_value(run)?;
                let mut sh = lock(&shared);
                let handler = HandlerId(sh.handlers.len() as u32);
                let spec = parse_system(&val, handler)?;
                if sh.reg.systems.iter().any(|s| s.id == spec.id) {
                    return Err(rt(format!("system '{}' is registered twice", spec.id)));
                }
                sh.handlers.push(key);
                sh.reg.systems.push(spec);
                Ok(())
            }),
            ("hooks", "on") => lua.create_function(move |lua, (point, f): (String, Function)| {
                gate("pg.hooks.on")?;
                if pg_api::hook_point(&point).is_none() {
                    let known: Vec<&str> = pg_api::HOOK_POINTS.iter().map(|h| h.id).collect();
                    return Err(rt(format!(
                        "unknown hook point '{point}'{} (known: {})",
                        pg_api::hint(&point, known.iter().copied()),
                        known.join(", ")
                    )));
                }
                let key = lua.create_registry_value(f)?;
                let mut sh = lock(&shared);
                let handler = HandlerId(sh.handlers.len() as u32);
                sh.handlers.push(key);
                sh.reg.hooks.push(HookSpec { point, handler });
                Ok(())
            }),
            ("field", "int") => {
                lua.create_function(move |lua, (min, max, default): (Value, Value, Value)| {
                    let t = lua.create_table()?;
                    t.set("kind", "int")?;
                    t.set("min", int_arg(&min, "pg.field.int min")?)?;
                    t.set("max", int_arg(&max, "pg.field.int max")?)?;
                    t.set("default", int_arg(&default, "pg.field.int default")?)?;
                    t.set_readonly(true);
                    Ok(t)
                })
            }
            ("", "rand") => lua.create_function(move |_, args: mlua::Variadic<Value>| {
                let mut it = args.into_iter();
                let stream = match it.next() {
                    Some(Value::String(s)) => s.to_str().map(|s| s.to_string()).unwrap_or_default(),
                    _ => return Err(rt("pg.rand needs a stream name as its first argument")),
                };
                if !is_name(&stream, 32) {
                    return Err(rt(format!("'{stream}' is not a valid stream name")));
                }
                let mut ints: Vec<i64> = Vec::new();
                let mut strs: Vec<String> = Vec::new();
                let mut order: Vec<bool> = Vec::new(); // true = int
                for v in it {
                    match v {
                        Value::Integer(i) => {
                            ints.push(int_in_range(i).map_err(|e| rt(e.to_string()))?);
                            order.push(true);
                        }
                        Value::Number(n) => {
                            ints.push(number_to_int(n).map_err(|e| rt(e.to_string()))?);
                            order.push(true);
                        }
                        Value::String(s) => {
                            let s = s.to_str().map_err(|_| rt("keys must be valid UTF-8"))?;
                            if s.len() > 64 {
                                return Err(rt("a string key is longer than 64 bytes"));
                            }
                            strs.push(s.to_string());
                            order.push(false);
                        }
                        other => {
                            return Err(rt(format!(
                                "pg.rand keys must be integers or text, not {}",
                                other.type_name()
                            )))
                        }
                    }
                }
                if order.len() > 6 {
                    return Err(rt("pg.rand takes at most 6 keys"));
                }
                let (mut ii, mut si) = (0, 0);
                let keys: Vec<Key<'_>> = order
                    .iter()
                    .map(|is_int| {
                        if *is_int {
                            ii += 1;
                            Key::Int(ints[ii - 1])
                        } else {
                            si += 1;
                            Key::Str(&strs[si - 1])
                        }
                    })
                    .collect();
                let rng = Rng::new(
                    Seed(seed),
                    Stream::Pack {
                        pack: &pack,
                        name: &stream,
                    },
                    &keys,
                );
                Ok(f64::from(rng.draw(0)))
            }),
            ("math", "idiv") => lua.create_function(|_, (a, b): (Value, Value)| {
                let (a, b) = (int_arg(&a, "pg.math.idiv")?, int_arg(&b, "pg.math.idiv")?);
                if b == 0 {
                    return Err(rt("pg.math.idiv: division by zero"));
                }
                int_result(floor_div(i128::from(a), i128::from(b)), "pg.math.idiv")
            }),
            ("math", "muldiv") => lua.create_function(|_, (a, b, c): (Value, Value, Value)| {
                let a = i128::from(int_arg(&a, "pg.math.muldiv")?);
                let b = i128::from(int_arg(&b, "pg.math.muldiv")?);
                let c = i128::from(int_arg(&c, "pg.math.muldiv")?);
                if c == 0 {
                    return Err(rt("pg.math.muldiv: division by zero"));
                }
                // floor((2ab + c) / 2c), with the divisor made positive: rounds half up.
                let (num, den) = if c > 0 {
                    (2 * a * b + c, 2 * c)
                } else {
                    (-(2 * a * b) - c, -2 * c)
                };
                int_result(floor_div(num, den), "pg.math.muldiv")
            }),
            ("math", "isqrt") => lua.create_function(|_, n: Value| {
                let n = int_arg(&n, "pg.math.isqrt")?;
                if n < 0 {
                    return Err(rt("pg.math.isqrt: the argument must not be negative"));
                }
                int_result(isqrt(i128::from(n)), "pg.math.isqrt")
            }),
            ("math", "lerp_permille") => {
                lua.create_function(|_, (a, b, t): (Value, Value, Value)| {
                    let a = i128::from(int_arg(&a, "pg.math.lerp_permille")?);
                    let b = i128::from(int_arg(&b, "pg.math.lerp_permille")?);
                    let t = i128::from(int_arg(&t, "pg.math.lerp_permille")?);
                    let num = 2 * (b - a) * t + 1000;
                    int_result(a + floor_div(num, 2000), "pg.math.lerp_permille")
                })
            }
            ("log", "info") => {
                let sh = Arc::clone(&self.shared);
                lua.create_function(move |_, text: String| {
                    push_log(&sh, 1, &text);
                    Ok(())
                })
            }
            ("log", "warn") => {
                let sh = Arc::clone(&self.shared);
                lua.create_function(move |_, text: String| {
                    push_log(&sh, 2, &text);
                    Ok(())
                })
            }
            _ => Err(rt(format!(
                "pg-api lists pg.{module}.{name} but the Luau host has no implementation"
            ))),
        }
    }

    fn compile_source(&self, name: &str, source: &str) -> Result<Function, VmError> {
        if source.as_bytes().first() == Some(&0x1b) {
            return Err(VmError::Compile(format!(
                "{name}: precompiled bytecode is not accepted; packs are loaded from source only"
            )));
        }
        self.lua
            .load(source)
            .set_name(name)
            .set_mode(ChunkMode::Text)
            .into_function()
            .map_err(|e| VmError::Compile(trim_traceback(&e.to_string())))
    }

    fn classify(&self, e: &Error) -> VmError {
        if self.fuel.exhausted.load(Ordering::Relaxed) {
            return VmError::OutOfFuel {
                limit: self.fuel.budget.load(Ordering::Relaxed),
            };
        }
        match root_cause(e) {
            Error::MemoryError(_) => VmError::OutOfMemory,
            Error::SyntaxError { message, .. } => VmError::Compile(trim_traceback(message)),
            Error::RuntimeError(m) if m.contains("not enough memory") => VmError::OutOfMemory,
            Error::RuntimeError(m) => VmError::Runtime(trim_traceback(m)),
            other => VmError::Runtime(trim_traceback(&other.to_string())),
        }
    }

    fn arm_fuel(&self, fuel: Fuel) -> u64 {
        let start = self.fuel.used.load(Ordering::Relaxed);
        self.fuel.exhausted.store(false, Ordering::Relaxed);
        self.fuel.budget.store(fuel.0, Ordering::Relaxed);
        self.fuel
            .limit
            .store(start.saturating_add(fuel.0), Ordering::Relaxed);
        start
    }

    fn disarm_fuel(&self) {
        self.fuel.limit.store(u64::MAX, Ordering::Relaxed);
    }

    /// Lines the pack logged since the last call to this.
    pub fn take_log_lines(&mut self) -> Vec<(LogLevel, String)> {
        std::mem::take(&mut lock(&self.shared).log)
    }
}

fn push_log(shared: &Mutex<Shared>, level: i64, text: &str) {
    let mut sh = lock(shared);
    if sh.log.len() >= MAX_LOG_LINES_PER_CALL {
        return;
    }
    let mut line = text.to_owned();
    if line.len() > MAX_LOG_LINE {
        let mut cut = MAX_LOG_LINE;
        while !line.is_char_boundary(cut) {
            cut -= 1;
        }
        line.truncate(cut);
    }
    sh.log.push((
        if level == 2 {
            LogLevel::Warn
        } else {
            LogLevel::Info
        },
        line,
    ));
}

impl ScriptVm for LuauVm {
    fn load(&mut self, chunk: SourceChunk) -> Result<(), VmError> {
        if chunk.source.len() > MAX_SOURCE_BYTES {
            return Err(VmError::Compile(format!(
                "{}: the file is larger than {MAX_SOURCE_BYTES} bytes",
                chunk.name
            )));
        }
        // Compile now so a syntax error is reported at load, with the file name.
        self.compile_source(&chunk.name, &chunk.source)?;
        let mut sh = lock(&self.shared);
        let module = chunk
            .name
            .strip_suffix(".luau")
            .unwrap_or(&chunk.name)
            .to_owned();
        sh.sources.insert(module, chunk.source.clone());
        sh.sources.insert(chunk.name, chunk.source);
        Ok(())
    }

    fn take_log(&mut self) -> Vec<(LogLevel, String)> {
        self.take_log_lines()
    }

    fn run_load_phase(&mut self, entry: &str, fuel: Fuel) -> Result<Registrations, VmError> {
        let source = lock(&self.shared)
            .sources
            .get(entry)
            .cloned()
            .ok_or_else(|| {
                VmError::Compile(format!("the entry script '{entry}' was not loaded"))
            })?;
        let func = self.compile_source(entry, &source)?;
        lock(&self.shared).loading = true;
        self.arm_fuel(fuel);
        let result = func.call::<()>(());
        self.disarm_fuel();
        lock(&self.shared).loading = false;
        if let Err(e) = result {
            return Err(self.classify(&e));
        }
        // The registries are closed: user globals are frozen too.
        self.lua.globals().set_readonly(true);
        Ok(lock(&self.shared).reg.clone())
    }

    fn call(&mut self, handler: HandlerId, args: &Val, fuel: Fuel) -> Result<CallResult, VmError> {
        let invoke = self
            .invoke
            .clone()
            .ok_or_else(|| VmError::Runtime("the VM has no invoke function".into()))?;
        let func: Function = {
            let sh = lock(&self.shared);
            let key = sh
                .handlers
                .get(handler.0 as usize)
                .ok_or_else(|| VmError::BadValue(format!("no handler {}", handler.0)))?;
            self.lua
                .registry_value(key)
                .map_err(|e| VmError::Runtime(e.to_string()))?
        };
        let args_value = val_to_lua(&self.lua, args).map_err(|e| self.classify(&e))?;
        lock(&self.shared).log.clear();
        let start = self.arm_fuel(fuel);
        let outcome = invoke.call::<(Value, Value)>((func, args_value));
        let used = self.fuel.used.load(Ordering::Relaxed) - start;
        let exhausted = self.fuel.exhausted.load(Ordering::Relaxed);
        self.disarm_fuel();
        let (ret, commands) = match outcome {
            Ok(pair) => pair,
            Err(e) => return Err(self.classify(&e)),
        };
        if exhausted {
            // The script caught the out-of-fuel error and carried on; the call still fails.
            return Err(VmError::OutOfFuel { limit: fuel.0 });
        }
        let value = to_val(&ret)?;
        let commands = parse_commands(&commands)?;
        let log = std::mem::take(&mut lock(&self.shared).log);
        Ok(CallResult {
            value,
            commands,
            fuel: used,
            log,
        })
    }

    fn call_batch(
        &mut self,
        handler: HandlerId,
        args: &[Val],
        fuel: Fuel,
    ) -> Vec<Result<CallResult, VmError>> {
        args.iter().map(|a| self.call(handler, a, fuel)).collect()
    }

    fn fuel_used(&self) -> u64 {
        self.fuel.used.load(Ordering::Relaxed)
    }

    fn memory_used(&self) -> usize {
        self.lua.used_memory()
    }

    fn set_memory_limit(&mut self, bytes: usize) {
        let floor = self.lua.used_memory() + 64 * 1024;
        let _ = self.lua.set_memory_limit(bytes.max(floor));
    }
}

/// Test support: real Luau bytecode must be refused when it is offered as source text.
#[cfg(test)]
pub(crate) fn bytecode_is_rejected_as_source() -> bool {
    let Ok(bytes) = Compiler::new().compile("return 1") else {
        return false;
    };
    let Ok(lua) = Lua::new_with(StdLib::NONE, LuaOptions::default()) else {
        return false;
    };
    lua.load(bytes)
        .set_mode(ChunkMode::Text)
        .into_function()
        .is_err()
}
