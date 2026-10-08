//! The `ScriptVm` boundary (Blueprint §23.15): the project's own value, registration and error types.
//!
//! Nothing outside the private `luau` module names the binding crate. Everything that crosses the boundary
//! is plain Rust data: integers, bounded strings, lists and string-keyed maps. The "integers only into the
//! core" rule (§23.5) is enforced where Luau values are converted, in one place.

use std::fmt;

/// Limits on values crossing the boundary, so a script cannot hand the core an unbounded structure.
pub const MAX_DEPTH: usize = 8;
pub const MAX_ITEMS: usize = 1024;
pub const MAX_STRING: usize = 4096;
/// The largest integer a script may return: `2^53`, the limit of exact integers in a double.
pub const MAX_INT: i64 = 1 << 53;

/// A value that crosses the boundary.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Nil,
    Bool(bool),
    Int(i64),
    Str(String),
    List(Vec<Val>),
    /// String keys, kept sorted so equal maps compare and hash equal.
    Map(Vec<(String, Val)>),
}

impl Val {
    pub fn map<K: Into<String>>(entries: impl IntoIterator<Item = (K, Val)>) -> Val {
        let mut v: Vec<(String, Val)> = entries.into_iter().map(|(k, v)| (k.into(), v)).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        Val::Map(v)
    }

    pub fn get(&self, key: &str) -> Option<&Val> {
        match self {
            Val::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Val::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Val::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// A budget in deterministic units: Luau interrupt safepoints (function calls and loop back-edges), which
/// depend only on the compiled script, never on the machine (§23.5 rule 5).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Fuel(pub u64);

/// A script source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceChunk {
    /// The path inside the pack, used as the chunk name so errors read `scripts/main.luau:12: ...`.
    pub name: String,
    pub source: String,
}

/// Which registered function a call targets.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HandlerId(pub u32);

/// Everything a VM needs to be created.
#[derive(Clone, Debug)]
pub struct VmConfig {
    pub pack_id: String,
    /// The capabilities the player approved; only the matching parts of `pg` exist.
    pub capabilities: Vec<String>,
    /// The world seed, for `pg.rand`.
    pub seed: u64,
    /// The memory cap for the whole VM, in bytes.
    pub memory_bytes: usize,
}

impl VmConfig {
    pub fn new(pack_id: &str, capabilities: &[&str], seed: u64) -> VmConfig {
        VmConfig {
            pack_id: pack_id.to_owned(),
            capabilities: capabilities.iter().map(|c| (*c).to_owned()).collect(),
            seed,
            memory_bytes: 4 * 1024 * 1024,
        }
    }

    pub fn has(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }
}

/// A field of a script-declared component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    pub name: String,
    pub min: i64,
    pub max: i64,
    pub default: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentSpec {
    /// The short name as written by the pack (`caffeine`); the stored name is `<pack>.<name>`.
    pub name: String,
    pub applies_to: Vec<String>,
    /// Sorted by field name.
    pub fields: Vec<FieldSpec>,
    pub version: u32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Cadence {
    Tick,
    Minute,
    Slot,
    Day,
}

impl Cadence {
    pub fn parse(s: &str) -> Option<Cadence> {
        match s {
            "tick" => Some(Cadence::Tick),
            "minute" => Some(Cadence::Minute),
            "slot" => Some(Cadence::Slot),
            "day" => Some(Cadence::Day),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemSpec {
    pub id: String,
    pub cadence: Cadence,
    /// The built-in system to run after, by name (`NeedsSystem`); `None` means after the last built-in.
    pub after: Option<String>,
    pub kind: String,
    /// Component names (short, as written) an entity must have.
    pub with: Vec<String>,
    pub writes: Vec<String>,
    pub handler: HandlerId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookSpec {
    pub point: String,
    pub handler: HandlerId,
}

/// What the entry script registered during the load phase.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Registrations {
    pub components: Vec<ComponentSpec>,
    pub systems: Vec<SystemSpec>,
    pub hooks: Vec<HookSpec>,
}

/// A command a script asked for. The host validates every one before the core sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptCommand {
    SetField {
        entity: String,
        component: String,
        field: String,
        value: i64,
    },
}

/// A finished call: its return value and the commands it buffered. A failed call has neither.
#[derive(Clone, Debug, PartialEq)]
pub struct CallResult {
    pub value: Val,
    pub commands: Vec<ScriptCommand>,
    /// Fuel the call used.
    pub fuel: u64,
    /// Lines the script logged, as `(level, text)`.
    pub log: Vec<(LogLevel, String)>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
}

/// Why a VM operation failed. The messages name the pack, file and line where Luau knows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VmError {
    /// The source did not compile (or was bytecode).
    Compile(String),
    /// The script raised an error.
    Runtime(String),
    OutOfFuel {
        limit: u64,
    },
    OutOfMemory,
    /// A number that is not an integer, is not finite or is out of range was returned across the boundary.
    NonInteger(String),
    /// A value of the wrong shape was returned or registered.
    BadValue(String),
    /// A registration was refused (duplicate, unknown hook point, capability, closed registry...).
    Registration(String),
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmError::Compile(m) => write!(f, "compile error: {m}"),
            VmError::Runtime(m) => write!(f, "script error: {m}"),
            VmError::OutOfFuel { limit } => write!(f, "script ran out of fuel (limit {limit})"),
            VmError::OutOfMemory => write!(f, "script exceeded its memory limit"),
            VmError::NonInteger(m) => write!(f, "not an integer: {m}"),
            VmError::BadValue(m) => write!(f, "bad value: {m}"),
            VmError::Registration(m) => write!(f, "registration refused: {m}"),
        }
    }
}

impl std::error::Error for VmError {}

/// The boundary. One implementation sits behind it (over `mlua`, in the private `luau` module); a second
/// implementation must pass the same conformance suite (Blueprint §18 items 11 and 12).
///
/// Deviation from the Blueprint's sketch, recorded in D-031: `call` takes its context as plain data inside
/// the arguments and returns the buffered commands, rather than receiving a `&mut CallCtx`. That keeps the
/// VM at rest between calls with no host references retained, makes calls pure functions of their input
/// (so they can be tested, replayed and batched), and lets the VM be `Send` with no lifetimes crossing.
pub trait ScriptVm: Send {
    /// Stages a source file. The entry script runs in [`ScriptVm::run_load_phase`]; other files are
    /// available to `require` by their path without the `.luau` extension.
    fn load(&mut self, chunk: SourceChunk) -> Result<(), VmError>;

    /// Runs the entry script with registration enabled, then closes the registries and freezes globals.
    fn run_load_phase(&mut self, entry: &str, fuel: Fuel) -> Result<Registrations, VmError>;

    /// Lines the pack logged since the last call to this (what a pack printed while loading, say). The
    /// default is none, for VMs that cannot log.
    fn take_log(&mut self) -> Vec<(LogLevel, String)> {
        Vec::new()
    }

    /// Calls a registered function. `args` is a map with `ctx` (the context data) and optionally `entity`.
    fn call(&mut self, handler: HandlerId, args: &Val, fuel: Fuel) -> Result<CallResult, VmError>;

    /// Calls a registered function once per element of `args`, with the same budget for each. Each call is
    /// independent: one failing does not affect the others.
    fn call_batch(
        &mut self,
        handler: HandlerId,
        args: &[Val],
        fuel: Fuel,
    ) -> Vec<Result<CallResult, VmError>>;

    /// Fuel used by every call so far, in deterministic units.
    fn fuel_used(&self) -> u64;

    /// Bytes the VM currently holds.
    fn memory_used(&self) -> usize;

    /// Changes the memory cap.
    fn set_memory_limit(&mut self, bytes: usize);
}
