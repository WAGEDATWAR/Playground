//! The script host: runs every pack's VM against the world (Blueprint §23.3, §23.6, §23.9).
//!
//! One VM per pack, all driven from the simulation thread. The host
//! * loads each pack in load order (a failure quarantines that pack and nothing else),
//! * turns registrations into core pieces: declared components ([`ExtSchemas`]), pipeline systems and
//!   hook answers,
//! * builds read-only views of the world as plain data, calls handlers, and applies the commands they
//!   buffered through the core's validated write path ([`apply_set_field`]),
//! * meters fuel per call and per tick, records errors in the world (so quarantine survives snapshots,
//!   keyframes and saves) and reports them as events.
//!
//! The host itself holds no simulation state: everything that influences a later tick lives in the
//! world. Rebuilding the host (and so every VM) at any tick must give identical results; the VM-reload
//! tests check that.

use crate::vm::{
    Cadence as VmCadence, ComponentSpec, Fuel, HandlerId, ScriptCommand, ScriptVm, SourceChunk,
    Val, VmConfig, VmError,
};
use crate::LuauVm;
use pg_api::HookPoint;
use pg_core::canon::Canon;
use pg_core::ext::{apply_set_field, ComponentDef, ExtSchemas, FieldDef};
use pg_core::hooks::HookHost;
use pg_core::id::{EntityId, Kind};
use pg_core::pipeline::{Cadence, Pipeline, Placement, System, SystemSlot, TickCtx};
use pg_core::world::WorldState;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// A pack's scripts, as the host needs them.
#[derive(Clone, Debug)]
pub struct ScriptPackInput {
    pub id: String,
    pub version: String,
    pub hash: String,
    pub capabilities: Vec<String>,
    pub entry: String,
    pub sources: BTreeMap<String, String>,
}

impl ScriptPackInput {
    pub fn from_content(p: &pg_content::ScriptPack<'_>) -> ScriptPackInput {
        ScriptPackInput {
            id: p.id.to_owned(),
            version: p.version.clone(),
            hash: p.hash.to_owned(),
            capabilities: p.capabilities.iter().map(|c| (*c).to_owned()).collect(),
            entry: p.entry.to_owned(),
            sources: p.sources.clone(),
        }
    }
}

/// Budgets and the quarantine policy (Blueprint §23.6). Fuel is in deterministic units.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptLimits {
    pub fuel_per_call: u64,
    pub fuel_per_tick: u64,
    pub load_fuel: u64,
    pub memory_bytes: usize,
    /// Errors inside `error_window_ticks` that quarantine a pack.
    pub max_errors: usize,
    pub error_window_ticks: u64,
}

impl Default for ScriptLimits {
    fn default() -> ScriptLimits {
        ScriptLimits {
            fuel_per_call: 50_000,
            fuel_per_tick: 1_000_000,
            load_fuel: 2_000_000,
            memory_bytes: 8 * 1024 * 1024,
            max_errors: 3,
            error_window_ticks: 14_400,
        }
    }
}

/// Creates a VM; the Luau implementation by default, a test double in tests.
pub type VmFactory = dyn Fn(VmConfig) -> Result<Box<dyn ScriptVm>, VmError> + Send + Sync;

fn luau_factory(cfg: VmConfig) -> Result<Box<dyn ScriptVm>, VmError> {
    Ok(Box::new(LuauVm::new(cfg)?))
}

struct Loaded {
    id: String,
    vm: Box<dyn ScriptVm>,
    reg: crate::vm::Registrations,
}

/// A pack the host could not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadFailure {
    pub pack: String,
    pub message: String,
}

/// A script error found while the world was read-only (a hook), reported by the maintenance system.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingError {
    pack: String,
    point: String,
    message: String,
}

/// What one handler has cost so far (suggestion S-036): calls, fuel and errors, for the overlay.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MeterRow {
    pub calls: u64,
    pub fuel: u64,
    pub errors: u32,
}

/// Collects [`MeterRow`]s per `(pack, point)`. Shared between every host a factory builds, so the numbers
/// keep accumulating when the VMs are rebuilt. Development statistics only: nothing here is world state.
#[derive(Default)]
pub struct ScriptMeter {
    rows: Mutex<BTreeMap<(String, String), MeterRow>>,
}

impl ScriptMeter {
    pub fn new() -> ScriptMeter {
        ScriptMeter::default()
    }

    fn with_row(&self, pack: &str, point: &str, f: impl FnOnce(&mut MeterRow)) {
        let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
        f(rows.entry((pack.to_owned(), point.to_owned())).or_default());
    }

    pub fn record_call(&self, pack: &str, point: &str, fuel: u64) {
        self.with_row(pack, point, |r| {
            r.calls += 1;
            r.fuel += fuel;
        });
    }

    pub fn record_error(&self, pack: &str, point: &str) {
        self.with_row(pack, point, |r| r.errors += 1);
    }

    /// `(pack, point, row)` in a stable order.
    pub fn rows(&self) -> Vec<(String, String, MeterRow)> {
        self.rows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|((p, q), r)| (p.clone(), q.clone(), r.clone()))
            .collect()
    }

    pub fn reset(&self) {
        self.rows.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

pub struct ScriptHost {
    meter: Option<Arc<ScriptMeter>>,
    packs: Vec<Loaded>,
    schemas: ExtSchemas,
    failures: Vec<LoadFailure>,
    failures_reported: bool,
    pending: Vec<PendingError>,
    /// Packs whose hook failed this tick: not asked again until the maintenance system has recorded it.
    failed_now: Vec<String>,
    limits: ScriptLimits,
    /// Fuel spent by systems in the current tick.
    tick_fuel: u64,
    tick_fuel_for: u64,
}

fn lock(h: &Mutex<ScriptHost>) -> std::sync::MutexGuard<'_, ScriptHost> {
    h.lock().unwrap_or_else(|e| e.into_inner())
}

fn kind_of(name: &str) -> Option<Kind> {
    match name {
        "pawn" => Some(Kind::Pawn),
        "object" => Some(Kind::Object),
        "map" => Some(Kind::Map),
        _ => None,
    }
}

fn slot_of(name: &str) -> Option<SystemSlot> {
    Some(match name {
        "NeedsSystem" => SystemSlot::Needs,
        "MoodSystem" => SystemSlot::Mood,
        "DayPlanner" => SystemSlot::DayPlanner,
        "CommitmentSystem" => SystemSlot::Commitment,
        "ReservationActivator" => SystemSlot::ReservationActivator,
        "TaskPlanner" => SystemSlot::TaskPlanner,
        "MovementSystem" => SystemSlot::Movement,
        "ActivitySystem" => SystemSlot::Activity,
        "ConversationSystem" => SystemSlot::Conversation,
        "MemorySystem" => SystemSlot::Memory,
        "RelationshipSystem" => SystemSlot::Relationship,
        "EventSystem" => SystemSlot::Event,
        "Maintenance" => SystemSlot::Maintenance,
        _ => return None,
    })
}

fn component_def(pack: &str, c: &ComponentSpec) -> Result<ComponentDef, String> {
    let mut applies_to = Vec::new();
    for k in &c.applies_to {
        applies_to.push(kind_of(k).ok_or_else(|| format!("unknown kind '{k}'"))?);
    }
    Ok(ComponentDef {
        name: format!("{pack}.{}", c.name),
        pack: pack.to_owned(),
        applies_to,
        fields: c
            .fields
            .iter()
            .map(|f| FieldDef {
                name: f.name.clone(),
                min: f.min,
                max: f.max,
                default: f.default,
            })
            .collect(),
        version: c.version,
    })
}

fn cap(text: &str, max: usize) -> String {
    let mut s = text.to_owned();
    if s.len() > max {
        let mut cut = max;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
    s
}

impl ScriptHost {
    /// Starts every pack with the Luau VM.
    pub fn new(packs: &[ScriptPackInput], seed: u64, limits: ScriptLimits) -> ScriptHost {
        ScriptHost::with_factory(packs, seed, limits, &luau_factory)
    }

    pub fn with_factory(
        packs: &[ScriptPackInput],
        seed: u64,
        limits: ScriptLimits,
        factory: &VmFactory,
    ) -> ScriptHost {
        let mut host = ScriptHost {
            meter: None,
            packs: Vec::new(),
            schemas: ExtSchemas::new(),
            failures: Vec::new(),
            failures_reported: false,
            pending: Vec::new(),
            failed_now: Vec::new(),
            limits,
            tick_fuel: 0,
            tick_fuel_for: u64::MAX,
        };
        for p in packs {
            if let Err(message) = host.start_pack(p, seed, factory) {
                host.failures.push(LoadFailure {
                    pack: p.id.clone(),
                    message,
                });
            }
        }
        host
    }

    fn start_pack(
        &mut self,
        p: &ScriptPackInput,
        seed: u64,
        factory: &VmFactory,
    ) -> Result<(), String> {
        let caps: Vec<&str> = p.capabilities.iter().map(String::as_str).collect();
        let mut cfg = VmConfig::new(&p.id, &caps, seed);
        cfg.memory_bytes = self.limits.memory_bytes;
        let mut vm = factory(cfg).map_err(|e| e.to_string())?;
        for (name, source) in &p.sources {
            vm.load(SourceChunk {
                name: name.clone(),
                source: source.clone(),
            })
            .map_err(|e| e.to_string())?;
        }
        let reg = vm
            .run_load_phase(&p.entry, Fuel(self.limits.load_fuel))
            .map_err(|e| e.to_string())?;
        // Cross-validate before anything is declared, so a bad pack leaves no trace.
        let mut defs = Vec::new();
        for c in &reg.components {
            defs.push(component_def(&p.id, c)?);
        }
        let own: Vec<&str> = reg.components.iter().map(|c| c.name.as_str()).collect();
        for s in &reg.systems {
            for name in s.with.iter().chain(&s.writes) {
                let known = own.contains(&name.as_str())
                    || self.schemas.get(name).is_some()
                    || defs.iter().any(|d| &d.name == name);
                if !known {
                    return Err(format!(
                        "system '{}' names component '{name}', which no loaded pack declares{}",
                        s.id,
                        pg_api::hint(name, own.iter().copied())
                    ));
                }
            }
            for w in &s.writes {
                if !own.contains(&w.as_str()) {
                    return Err(format!(
                        "system '{}' writes '{w}', but a pack may write only its own components",
                        s.id
                    ));
                }
            }
        }
        for d in defs {
            self.schemas.register(d)?;
        }
        self.packs.push(Loaded {
            id: p.id.clone(),
            vm,
            reg,
        });
        Ok(())
    }

    /// Records what every handler costs into `meter` from now on.
    pub fn set_meter(&mut self, meter: Option<Arc<ScriptMeter>>) {
        self.meter = meter;
    }

    pub fn schemas(&self) -> &ExtSchemas {
        &self.schemas
    }

    pub fn failures(&self) -> &[LoadFailure] {
        &self.failures
    }

    /// Ids of the packs that started, in load order.
    pub fn active_packs(&self) -> Vec<&str> {
        self.packs.iter().map(|p| p.id.as_str()).collect()
    }

    /// Total fuel every VM has used so far.
    pub fn fuel_used(&self) -> u64 {
        self.packs.iter().map(|p| p.vm.fuel_used()).sum()
    }

    pub fn memory_used(&self) -> usize {
        self.packs.iter().map(|p| p.vm.memory_used()).sum()
    }

    /// A component's fields for `entity`, keyed as `pack_id` sees them: its own components by short name,
    /// everything else by full name.
    fn components_view(&self, world: &WorldState, pack_id: &str, entity: EntityId) -> Val {
        let mut comps = Vec::new();
        for def in self.schemas.iter() {
            if !def.applies(entity.kind()) {
                continue;
            }
            let key = if def.pack == pack_id {
                def.name
                    .strip_prefix(&format!("{pack_id}."))
                    .unwrap_or(&def.name)
                    .to_owned()
            } else {
                def.name.clone()
            };
            let fields = world.ext.fields_of(def, entity);
            comps.push((
                key,
                Val::map(fields.into_iter().map(|(k, v)| (k, Val::Int(v)))),
            ));
        }
        Val::map(comps)
    }

    fn entity_view(&self, world: &WorldState, pack_id: &str, entity: EntityId) -> Val {
        let mut e = vec![
            ("id".to_owned(), Val::Str(entity.to_string())),
            (
                "kind".to_owned(),
                Val::Str(entity.kind().prefix().to_owned()),
            ),
            (
                "components".to_owned(),
                self.components_view(world, pack_id, entity),
            ),
        ];
        if let Some(p) = world.pawns.get(entity) {
            e.push(("x".to_owned(), Val::Int(i64::from(p.position.tile.x))));
            e.push(("y".to_owned(), Val::Int(i64::from(p.position.tile.y))));
        }
        Val::map(e)
    }

    fn entities_of(world: &WorldState, kind: Kind) -> Vec<EntityId> {
        match kind {
            Kind::Pawn => world.pawns.iter().map(|(id, _)| id).collect(),
            Kind::Object => world.objects.iter().map(|(id, _)| id).collect(),
            Kind::Map => world.maps.iter().map(|(id, _)| id).collect(),
            _ => Vec::new(),
        }
    }

    /// The full stored name for a component a script named: its own short name or a full name.
    fn full_name(pack: &str, name: &str) -> String {
        if name.contains('.') {
            name.to_owned()
        } else {
            format!("{pack}.{name}")
        }
    }

    fn report_error(
        &self,
        ctx: &mut TickCtx<'_>,
        pack: &str,
        point: &str,
        message: &str,
        immediate: bool,
    ) {
        if let Some(m) = &self.meter {
            m.record_error(pack, point);
        }
        let tick = ctx.flags.tick;
        ctx.emit(
            "script.error",
            Canon::map([
                ("pack", Canon::str(pack)),
                ("point", Canon::str(cap(point, 96))),
                ("message", Canon::str(cap(message, 400))),
            ]),
        );
        let newly = ctx.world.ext.record_error(
            pack,
            tick,
            self.limits.max_errors,
            self.limits.error_window_ticks,
            immediate,
        );
        if newly {
            ctx.emit(
                "script.quarantined",
                Canon::map([
                    ("pack", Canon::str(pack)),
                    ("reason", Canon::str(cap(message, 200))),
                ]),
            );
        }
    }

    /// Runs one registered system over the entities it queries.
    fn run_system(&mut self, pack_index: usize, system_index: usize, ctx: &mut TickCtx<'_>) {
        let tick = ctx.flags.tick;
        if self.tick_fuel_for != tick {
            self.tick_fuel_for = tick;
            self.tick_fuel = 0;
        }
        let Some(pack) = self.packs.get(pack_index) else {
            return;
        };
        let pack_id = pack.id.clone();
        if ctx.world.ext.is_quarantined(&pack_id) {
            return;
        }
        let Some(spec) = pack.reg.systems.get(system_index).cloned() else {
            return;
        };
        let Some(kind) = kind_of(&spec.kind) else {
            return;
        };
        let point = format!("system {}.{}", pack_id, spec.id);
        let ctx_data = Val::map([
            ("tick", Val::Int(i64::try_from(tick).unwrap_or(i64::MAX))),
            (
                "day",
                Val::Int(i64::try_from(ctx.flags.day).unwrap_or(i64::MAX)),
            ),
            ("minute", Val::Int(i64::from(ctx.flags.minute_of_day))),
        ]);
        // All views are built before any call, so no handler sees another's effects (§23.5 rule 6).
        let entities = Self::entities_of(ctx.world, kind);
        let args: Vec<(EntityId, Val)> = entities
            .into_iter()
            .map(|e| {
                (
                    e,
                    Val::map([
                        ("ctx", ctx_data.clone()),
                        ("entity", self.entity_view(ctx.world, &pack_id, e)),
                    ]),
                )
            })
            .collect();
        let mut buffered: Vec<(EntityId, Vec<ScriptCommand>)> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        let mut printed: Vec<(crate::vm::LogLevel, String)> = Vec::new();
        let per_call = self.limits.fuel_per_call;
        let per_tick = self.limits.fuel_per_tick;
        let Some(pack) = self.packs.get_mut(pack_index) else {
            return;
        };
        for (entity, a) in &args {
            if self.tick_fuel >= per_tick {
                errors.push(format!(
                    "the tick's script fuel budget ({per_tick}) was used up; {entity} and later entities were skipped"
                ));
                break;
            }
            let budget = per_call.min(per_tick - self.tick_fuel);
            match pack.vm.call(spec.handler, a, Fuel(budget)) {
                Ok(r) => {
                    if let Some(m) = &self.meter {
                        m.record_call(&pack_id, &point, r.fuel);
                    }
                    self.tick_fuel += r.fuel;
                    printed.extend(r.log);
                    buffered.push((*entity, r.commands));
                }
                Err(e) => {
                    self.tick_fuel += budget.min(per_call);
                    errors.push(format!("{entity}: {e}"));
                }
            }
        }
        // What the pack printed with pg.log reaches the developer console as `script.log` events.
        for (level, text) in printed {
            ctx.emit(
                "script.log",
                Canon::map([
                    ("pack", Canon::str(pack_id.clone())),
                    (
                        "level",
                        Canon::str(match level {
                            crate::vm::LogLevel::Info => "info",
                            crate::vm::LogLevel::Warn => "warn",
                        }),
                    ),
                    ("text", Canon::str(cap(&text, 400))),
                ]),
            );
        }
        // Apply every successful call's commands in entity order, through the core's validation.
        for (entity, commands) in buffered {
            for c in commands {
                let ScriptCommand::SetField {
                    entity: target,
                    component,
                    field,
                    value,
                } = c;
                let result = target
                    .parse::<EntityId>()
                    .map_err(|_| format!("'{target}' is not an entity id"))
                    .and_then(|id| {
                        apply_set_field(
                            ctx.world,
                            &self.schemas,
                            &pack_id,
                            id,
                            &Self::full_name(&pack_id, &component),
                            &field,
                            value,
                        )
                        .map_err(|e| e.to_string())
                    });
                if let Err(reason) = result {
                    ctx.emit(
                        "input_rejected",
                        Canon::map([
                            ("command", Canon::str("script.set_field")),
                            (
                                "reason",
                                Canon::str(cap(
                                    &format!("{pack_id}: {reason} (from {entity})"),
                                    400,
                                )),
                            ),
                        ]),
                    );
                }
            }
        }
        for e in errors {
            self.report_error(ctx, &pack_id, &point, &e, false);
        }
    }

    /// Records load failures once, then errors hooks found while the world was read-only.
    fn maintain(&mut self, ctx: &mut TickCtx<'_>) {
        if !self.failures_reported {
            self.failures_reported = true;
            for f in self.failures.clone() {
                if !ctx.world.ext.is_quarantined(&f.pack) {
                    self.report_error(ctx, &f.pack, "load", &f.message, true);
                }
            }
        }
        for p in std::mem::take(&mut self.pending) {
            self.report_error(ctx, &p.pack, &p.point, &p.message, false);
        }
        self.failed_now.clear();
    }

    fn answer_hook(
        &mut self,
        point: &'static HookPoint,
        world: &WorldState,
        subject: EntityId,
    ) -> Vec<i64> {
        let mut answers = Vec::new();
        let tick = world.clock.tick();
        let per_call = self.limits.fuel_per_call;
        for i in 0..self.packs.len() {
            let Some(pack) = self.packs.get(i) else {
                continue;
            };
            if world.ext.is_quarantined(&pack.id) || self.failed_now.contains(&pack.id) {
                continue;
            }
            let handlers: Vec<HandlerId> = pack
                .reg
                .hooks
                .iter()
                .filter(|h| h.point == point.id)
                .map(|h| h.handler)
                .collect();
            if handlers.is_empty() {
                continue;
            }
            let pack_id = pack.id.clone();
            let args = Val::map([(
                "ctx",
                Val::map([
                    ("tick", Val::Int(i64::try_from(tick).unwrap_or(i64::MAX))),
                    ("pawn", self.entity_view(world, &pack_id, subject)),
                ]),
            )]);
            let Some(pack) = self.packs.get_mut(i) else {
                continue;
            };
            for h in handlers {
                match pack.vm.call(h, &args, Fuel(per_call)) {
                    Ok(r) => {
                        if let Some(m) = &self.meter {
                            m.record_call(&pack_id, &format!("hook {}", point.id), r.fuel);
                        }
                        match r.value {
                            Val::Int(v) => answers.push(v),
                            other => {
                                self.pending.push(PendingError {
                                    pack: pack_id.clone(),
                                    point: format!("hook {}", point.id),
                                    message: format!(
                                        "a hook must return an integer, not {other:?}"
                                    ),
                                });
                                self.failed_now.push(pack_id.clone());
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        self.pending.push(PendingError {
                            pack: pack_id.clone(),
                            point: format!("hook {}", point.id),
                            message: e.to_string(),
                        });
                        self.failed_now.push(pack_id.clone());
                        break;
                    }
                }
            }
        }
        answers
    }
}

struct ScriptSystem {
    id: String,
    host: Arc<Mutex<ScriptHost>>,
    pack_index: usize,
    system_index: usize,
}

impl System for ScriptSystem {
    fn id(&self) -> &str {
        &self.id
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        lock(&self.host).run_system(self.pack_index, self.system_index, ctx);
    }
}

struct Maintenance {
    host: Arc<Mutex<ScriptHost>>,
}

impl System for Maintenance {
    fn id(&self) -> &str {
        "script.maintenance"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        lock(&self.host).maintain(ctx);
    }
}

/// The hook host the sim installs.
pub struct HostHooks(pub Arc<Mutex<ScriptHost>>);

impl HookHost for HostHooks {
    fn ask(
        &mut self,
        point: &'static HookPoint,
        world: &WorldState,
        subject: EntityId,
    ) -> Vec<i64> {
        lock(&self.0).answer_hook(point, world, subject)
    }
}

/// Adds the host's systems to `pipeline` (in pack load order, then registration order) and returns the hook
/// host to give the sim.
pub fn install(
    host: &Arc<Mutex<ScriptHost>>,
    pipeline: &mut Pipeline,
) -> Result<HostHooks, String> {
    let layout: Vec<(usize, usize, String, VmCadence, Option<String>)> = {
        let h = lock(host);
        h.packs
            .iter()
            .enumerate()
            .flat_map(|(pi, p)| {
                p.reg.systems.iter().enumerate().map(move |(si, s)| {
                    (
                        pi,
                        si,
                        format!("{}.{}", p.id, s.id),
                        s.cadence,
                        s.after.clone(),
                    )
                })
            })
            .collect()
    };
    for (pack_index, system_index, id, cadence, after) in layout {
        let slot = after
            .as_deref()
            .map_or(Some(SystemSlot::Maintenance), slot_of)
            .ok_or_else(|| format!("system {id}: unknown anchor"))?;
        let cadence = match cadence {
            VmCadence::Tick => Cadence::Tick,
            VmCadence::Minute => Cadence::Minute,
            VmCadence::Slot => Cadence::Slot,
            VmCadence::Day => Cadence::Day,
        };
        pipeline
            .add_extension(
                slot,
                Placement::After,
                cadence,
                Box::new(ScriptSystem {
                    id,
                    host: Arc::clone(host),
                    pack_index,
                    system_index,
                }),
            )
            .map_err(|e| e.to_string())?;
    }
    pipeline
        .add_extension(
            SystemSlot::Maintenance,
            Placement::After,
            Cadence::Tick,
            Box::new(Maintenance {
                host: Arc::clone(host),
            }),
        )
        .map_err(|e| e.to_string())?;
    Ok(HostHooks(Arc::clone(host)))
}

/// Starts a host for `packs` and installs it into `pipeline`: the one call the runtime makes when it builds
/// a pipeline (also when it rebuilds one for a restored or shadow sim, which is what makes "rebuild every VM
/// at any tick" the normal path rather than a special case).
pub fn attach(
    packs: &[ScriptPackInput],
    seed: u64,
    limits: ScriptLimits,
    pipeline: &mut Pipeline,
) -> Result<(Arc<Mutex<ScriptHost>>, HostHooks), String> {
    attach_with_meter(packs, seed, limits, None, pipeline)
}

/// Like [`attach`], also recording handler costs into `meter` (suggestion S-036).
pub fn attach_with_meter(
    packs: &[ScriptPackInput],
    seed: u64,
    limits: ScriptLimits,
    meter: Option<Arc<ScriptMeter>>,
    pipeline: &mut Pipeline,
) -> Result<(Arc<Mutex<ScriptHost>>, HostHooks), String> {
    let mut h = ScriptHost::new(packs, seed, limits);
    h.set_meter(meter);
    let host = Arc::new(Mutex::new(h));
    let hooks = install(&host, pipeline)?;
    Ok((host, hooks))
}
