//! Sandbox conformance, hostile-pack corpus and determinism tests for the Luau `ScriptVm`
//! (Blueprint §18 items 11 and 12, §23.4 to §23.6). A second implementation behind the trait must pass
//! these same cases.

use super::*;

const ENTRY: &str = "scripts/main.luau";
const CALL_FUEL: u64 = 200_000;

fn config(caps: &[&str]) -> VmConfig {
    VmConfig::new("test", caps, 7)
}

fn all_caps() -> Vec<&'static str> {
    vec!["read", "data", "systems"]
}

fn make(caps: &[&str], source: &str) -> Result<(LuauVm, Registrations), VmError> {
    let mut vm = LuauVm::new(config(caps))?;
    vm.load(SourceChunk {
        name: ENTRY.into(),
        source: source.into(),
    })?;
    let reg = vm.run_load_phase(ENTRY, Fuel(CALL_FUEL))?;
    Ok((vm, reg))
}

/// Registers system `t` (the body under test) and system `ok` (a sanity probe) and calls `t`.
fn run_body(body: &str) -> (LuauVm, Result<CallResult, VmError>, Registrations) {
    let source = format!(
        r#"
        pg.systems.register({{ id = "t", cadence = "tick", query = {{ kind = "pawn" }},
            run = function(ctx, e) {body} end }})
        pg.systems.register({{ id = "ok", cadence = "tick", query = {{ kind = "pawn" }},
            run = function(ctx, e) return 41 + 1 end }})
        "#
    );
    let (mut vm, reg) = make(&all_caps(), &source).unwrap_or_else(|e| panic!("load: {e}"));
    let r = vm.call(reg.systems[0].handler, &entity_args(), Fuel(CALL_FUEL));
    (vm, r, reg)
}

fn entity_args() -> Val {
    Val::map([
        (
            "ctx",
            Val::map([("tick", Val::Int(5)), ("day", Val::Int(0))]),
        ),
        (
            "entity",
            Val::map([
                ("id", Val::Str("pawn_1".into())),
                ("kind", Val::Str("pawn".into())),
                (
                    "components",
                    Val::map([("caffeine", Val::map([("level", Val::Int(50))]))]),
                ),
            ]),
        ),
    ])
}

/// After any failure the VM must still answer a trivial call correctly.
fn assert_still_usable(vm: &mut LuauVm, reg: &Registrations) {
    let r = vm
        .call(reg.systems[1].handler, &entity_args(), Fuel(CALL_FUEL))
        .expect("the VM must stay usable after a contained failure");
    assert_eq!(r.value, Val::Int(42));
}

fn value_of(body: &str) -> Val {
    let (_, r, _) = run_body(body);
    r.unwrap_or_else(|e| panic!("`{body}` failed: {e}")).value
}

fn error_of(body: &str) -> VmError {
    let (mut vm, r, reg) = run_body(body);
    let e = r.expect_err(&format!("`{body}` should have failed"));
    assert_still_usable(&mut vm, &reg);
    e
}

fn runtime_error_containing(body: &str, needle: &str) {
    match error_of(body) {
        VmError::Runtime(m) => assert!(m.contains(needle), "`{body}`: {m}"),
        other => panic!("`{body}`: expected a runtime error containing '{needle}', got {other:?}"),
    }
}

// ---- the surface a pack sees ---------------------------------------------------------------------------------

#[test]
fn only_allow_listed_globals_exist() {
    let names = match value_of(
        r#"local out = {} for k in next, _G do out[#out + 1] = k end table.sort(out) return table.concat(out, ",")"#,
    ) {
        Val::Str(s) => s,
        other => panic!("{other:?}"),
    };
    // `_G` here is the sandbox's global table; its names come from the sealed original.
    let allowed = [
        "_G",
        "_VERSION",
        "assert",
        "bit32",
        "buffer",
        "error",
        "getmetatable",
        "ipairs",
        "math",
        "next",
        "pairs",
        "pcall",
        "pg",
        "print",
        "rawequal",
        "rawget",
        "rawlen",
        "rawset",
        "require",
        "select",
        "setmetatable",
        "string",
        "table",
        "tonumber",
        "tostring",
        "type",
        "typeof",
        "unpack",
        "utf8",
        "xpcall",
    ];
    for n in names.split(',') {
        assert!(allowed.contains(&n), "unexpected global '{n}' in: {names}");
    }
    for n in [
        "pairs",
        "print",
        "tostring",
        "setmetatable",
        "require",
        "pg",
        "math",
        "string",
    ] {
        assert!(names.split(',').any(|x| x == n), "missing global {n}");
    }
}

#[test]
fn dangerous_globals_are_absent() {
    let v = value_of(
        r#"return os == nil and io == nil and debug == nil and package == nil and coroutine == nil
            and loadstring == nil and load == nil and getfenv == nil and setfenv == nil and newproxy == nil
            and collectgarbage == nil and gcinfo == nil and dofile == nil and loadfile == nil
            and dostring == nil and vector == nil and string.dump == nil"#,
    );
    assert_eq!(v, Val::Bool(true));
}

#[test]
fn math_and_string_are_restricted_to_the_profile() {
    let math = value_of(
        r#"local o = {} for k in next, math do o[#o+1] = k end table.sort(o) return table.concat(o, ",")"#,
    );
    assert_eq!(
        math,
        Val::Str("abs,ceil,clamp,floor,huge,max,min,sign".into())
    );
    for gone in [
        "random",
        "randomseed",
        "sin",
        "cos",
        "tan",
        "sqrt",
        "exp",
        "log",
        "pow",
        "noise",
        "pi",
        "fmod",
        "round",
    ] {
        assert_eq!(
            value_of(&format!("return math.{gone} == nil")),
            Val::Bool(true),
            "{gone}"
        );
    }
    assert_eq!(
        value_of("return math.floor(7.9) + math.max(1, 5) + math.abs(-3)"),
        Val::Int(15)
    );
    assert_eq!(value_of("return math.clamp(15, 0, 10)"), Val::Int(10));
    assert_eq!(
        value_of(r#"return string.format("%d-%s", 5, "x")"#),
        Val::Str("5-x".into())
    );
    assert_eq!(
        value_of(r#"return ("abc"):upper()"#),
        Val::Str("ABC".into())
    );
}

#[test]
fn the_pg_table_is_exactly_what_the_capabilities_allow() {
    for caps in [
        vec![],
        vec!["read"],
        vec!["data"],
        vec!["systems"],
        vec!["read", "data", "systems"],
        vec!["ai", "dev", "worldgen", "world-write"],
    ] {
        let mut vm = LuauVm::new(config(&caps)).unwrap();
        vm.load(SourceChunk {
            name: ENTRY.into(),
            source: r#"
                local out = {}
                for module, v in pairs(pg) do
                    if type(v) == "function" then out[#out + 1] = "pg." .. module
                    else for name in pairs(v) do out[#out + 1] = "pg." .. module .. "." .. name end end
                end
                error("LIST:" .. table.concat(out, ","))
            "#
            .into(),
        })
        .unwrap();
        let listed = match vm.run_load_phase(ENTRY, Fuel(CALL_FUEL)) {
            Err(VmError::Runtime(m)) => m,
            other => panic!("{other:?}"),
        };
        let listed = listed
            .split("LIST:")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_owned();
        let mut got: Vec<&str> = listed.split(',').filter(|s| !s.is_empty()).collect();
        got.sort_unstable();
        let mut want: Vec<String> = pg_api::allowed_functions(caps.iter().copied())
            .iter()
            .map(|f| f.path())
            .collect();
        want.sort();
        assert_eq!(got, want, "capabilities {caps:?}");
    }
}

#[test]
fn every_api_function_has_an_implementation() {
    // Installing with every capability would fail with "no implementation" for any function pg-api lists
    // that the host does not provide.
    let caps: Vec<&str> = pg_api::CAPABILITIES.to_vec();
    LuauVm::new(config(&caps)).expect("every pg-api function must be implemented");
}

// ---- hostile-pack corpus: containment --------------------------------------------------------------------

#[test]
fn escaping_through_require_is_impossible() {
    for path in [
        "../../etc/passwd",
        "os",
        "io",
        "/abs/path",
        "scripts/../../x",
        "debug",
        "",
    ] {
        runtime_error_containing(&format!(r#"return require("{path}")"#), "no module");
    }
    runtime_error_containing("return require(5)", "module name must be a string");
}

#[test]
fn runaway_loops_stop_on_fuel_and_the_vm_survives() {
    for body in [
        "while true do end",
        "repeat until false",
        "for i = 1, 1e12 do end",
        "local function f() return f() end return f()",
        "while true do pcall(function() while true do end end) end",
        "pcall(function() while true do end end) return 1",
        "xpcall(error, function() while true do end end) return 1",
        "local t = {} while true do t[#t + 1] = 1 end",
    ] {
        let e = error_of(body);
        assert!(
            matches!(e, VmError::OutOfFuel { .. } | VmError::OutOfMemory)
                || matches!(&e, VmError::Runtime(m) if m.contains("stack overflow")),
            "`{body}`: {e:?}"
        );
    }
    assert!(matches!(
        error_of("while true do end"),
        VmError::OutOfFuel { limit: CALL_FUEL }
    ));
}

#[test]
fn deep_recursion_is_an_error_not_a_crash() {
    let e = error_of("local function f(n) return 1 + f(n + 1) end return f(1)");
    assert!(
        matches!(&e, VmError::Runtime(m) if m.contains("stack overflow"))
            || matches!(e, VmError::OutOfFuel { .. }),
        "{e:?}"
    );
}

#[test]
fn a_loop_is_cut_off_close_to_its_budget() {
    let (mut vm, reg) = make(&all_caps(), r#"pg.systems.register({ id = "t", cadence = "tick", query = { kind = "pawn" }, run = function() while true do end end })"#).unwrap();
    let before = vm.fuel_used();
    let r = vm.call(reg.systems[0].handler, &entity_args(), Fuel(1000));
    assert!(matches!(r, Err(VmError::OutOfFuel { limit: 1000 })));
    let used = vm.fuel_used() - before;
    assert!(
        (1000..=1010).contains(&used),
        "stopped after {used} units for a budget of 1000"
    );
}

#[test]
fn memory_bombs_abort_cleanly_and_leave_the_vm_usable() {
    for body in [
        r#"local t = {} for i = 1, 1e9 do t[i] = string.rep("x", 4000) end"#,
        "local t = table.create(1e8, 0) return #t",
        "local b = buffer.create(2^30) return buffer.len(b)",
        r#"local s = "x" for i = 1, 60 do s = s .. s end return #s"#,
    ] {
        let (mut vm, r, reg) = run_body(body);
        let e = r.expect_err(body);
        assert!(
            matches!(e, VmError::OutOfMemory | VmError::OutOfFuel { .. })
                || matches!(&e, VmError::Runtime(m) if m.contains("memory") || m.contains("too long") || m.contains("overflow")),
            "`{body}`: {e:?}"
        );
        assert_still_usable(&mut vm, &reg);
        assert!(
            vm.memory_used() < 8 * 1024 * 1024,
            "memory came back down after the abort"
        );
    }
}

#[test]
fn one_packs_memory_exhaustion_does_not_touch_another() {
    let (mut a, reg_a) = {
        let (vm, reg, ..) = make(&all_caps(), r#"
            pg.systems.register({ id = "t", cadence = "tick", query = { kind = "pawn" },
                run = function() local t = {} for i = 1, 1e9 do t[i] = string.rep("y", 4000) end end })
            pg.systems.register({ id = "ok", cadence = "tick", query = { kind = "pawn" }, run = function() return 42 end })
        "#).unwrap();
        (vm, reg)
    };
    let (mut b, reg_b) = make(&all_caps(), r#"
        pg.systems.register({ id = "t", cadence = "tick", query = { kind = "pawn" }, run = function() return 1 end })
        pg.systems.register({ id = "ok", cadence = "tick", query = { kind = "pawn" }, run = function() return 42 end })
    "#).unwrap();
    let before = b.memory_used();
    assert!(matches!(
        a.call(reg_a.systems[0].handler, &entity_args(), Fuel(CALL_FUEL)),
        Err(VmError::OutOfMemory)
    ));
    assert_eq!(
        b.call(reg_b.systems[1].handler, &entity_args(), Fuel(CALL_FUEL))
            .unwrap()
            .value,
        Val::Int(42)
    );
    assert!(b.memory_used().abs_diff(before) < 4096);
    assert_eq!(
        a.call(reg_a.systems[1].handler, &entity_args(), Fuel(CALL_FUEL))
            .unwrap()
            .value,
        Val::Int(42)
    );
}

#[test]
fn natively_long_functions_are_bounded_by_fuel_or_memory() {
    use std::time::{Duration, Instant};
    // Catastrophic patterns: Luau's matcher calls the interrupt on every step, so fuel stops them.
    for body in [
        r#"return (("a"):rep(400)):find(".-.-.-.-.-b")"#,
        r#"return (("a"):rep(2000)):match("^(.-)a*(.-)a*(.-)a*b$")"#,
    ] {
        let started = Instant::now();
        let e = error_of(body);
        assert!(
            matches!(e, VmError::OutOfFuel { .. } | VmError::OutOfMemory),
            "`{body}`: {e:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "`{body}` took {:?}",
            started.elapsed()
        );
    }
    // Allocation is bounded by the memory limit, whatever the size asked for.
    for body in [
        r#"return string.rep("x", 1e9)"#,
        r#"return ("x"):rep(1e9)"#,
        r#"return string.rep("abc", 1e6, ",")"#,
        r#"return string.format("%s%s", ("x"):rep(2000000), ("y"):rep(2000000)):upper()"#,
    ] {
        let started = Instant::now();
        let e = error_of(body);
        assert!(
            matches!(e, VmError::OutOfMemory | VmError::BadValue(_)),
            "`{body}`: {e:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "`{body}` took {:?}",
            started.elapsed()
        );
    }
    // Large but legal work inside the limit finishes quickly.
    for body in [
        r#"local t = {} for i = 1, 100000 do t[i] = (i * 7919) % 100003 end table.sort(t) return t[1]"#,
        r#"return #(("x"):rep(1000000):reverse():upper():lower())"#,
        r#"local s = (("a"):rep(30000)):gsub("a", "bb") return #s"#,
        r#"local t = {} for i = 1, 50000 do t[i] = "ab" end return #table.concat(t, ",")"#,
    ] {
        let started = Instant::now();
        let _ = run_body(body).1;
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "`{body}` took {:?}",
            started.elapsed()
        );
    }
    // Ordinary pattern use works.
    assert_eq!(
        value_of(r#"local a, b = ("key=42"):match("(%w+)=(%d+)") return tonumber(b)"#),
        Val::Int(42)
    );
    assert_eq!(
        value_of(r#"return (("a,b,c"):gsub(",", ";"))"#),
        Val::Str("a;b;c".into())
    );
    assert_eq!(
        value_of(r#"local n = 0 for w in ("x y z"):gmatch("%a") do n += 1 end return n"#),
        Val::Int(3)
    );
    assert_eq!(
        value_of(r#"return ("a.b"):find(".", 1, true)"#),
        Val::Int(2)
    );
}

#[test]
fn metatables_cannot_bring_in_finalizers_or_weak_tables() {
    runtime_error_containing("setmetatable({}, { __gc = function() end })", "__gc");
    runtime_error_containing(r#"setmetatable({}, { __mode = "k" })"#, "__mode");
    // The late form: attach first, add `__mode` afterwards. The metatable is frozen on attachment.
    runtime_error_containing(
        r#"local mt = {} setmetatable({}, mt) mt.__mode = "k""#,
        "readonly",
    );
    runtime_error_containing("setmetatable({}, 5)", "must be a table");
    // `__metatable` is honoured by the engine.
    assert_eq!(
        value_of(
            r#"local t = setmetatable({}, { __metatable = "locked" }) return getmetatable(t)"#
        ),
        Val::Str("locked".into())
    );
    runtime_error_containing(
        r#"local t = setmetatable({}, { __metatable = "locked" }) setmetatable(t, {})"#,
        "protected",
    );
    // Ordinary class patterns work.
    assert_eq!(
        value_of(
            r#"
            local C = {} C.__index = C
            function C.get(self) return self.v end
            local o = setmetatable({ v = 9 }, C)
            return o:get()"#
        ),
        Val::Int(9)
    );
}

#[test]
fn builtins_cannot_be_tampered_with() {
    for body in [
        "string.rep = nil",
        "math.floor = function() return 0 end",
        "table.insert = nil",
        r#"getmetatable("").__index = {}"#,
        r#"getmetatable("").__index.rep = nil"#,
        "_G.x = 1",
        "setmetatable(_G, {})",
        "pg.rand = nil",
        "pg.hacked = 1",
        "pg.math.idiv = nil",
        "rawset(_G, 'x', 1)",
        "rawset(string, 'rep', nil)",
        "utf8.char = nil",
        "bit32.band = nil",
    ] {
        runtime_error_containing(body, "readonly");
    }
}

#[test]
fn removed_and_replaced_builtins_cannot_be_reached_through_compiler_fast_paths() {
    // `math.sin(x)` is compiled to a fast call that never reads the global, so removing the global is not
    // enough: the compiler must be told not to fast-call it.
    for call in [
        "math.sin(1)",
        "math.cos(1)",
        "math.tan(1)",
        "math.sqrt(4)",
        "math.exp(1)",
        "math.log(2)",
        "math.pow(2, 2)",
        "math.random()",
        "math.random(1, 5)",
        "math.randomseed(1)",
        "math.fmod(5, 3)",
        "math.round(1.5)",
        "math.atan2(1, 1)",
        "math.noise(1)",
        "math.lerp(0, 1, 0.5)",
        "math.modf(1.5)",
    ] {
        runtime_error_containing(&format!("return {call}"), "attempt to");
    }
    // Replaced functions are the restricted ones even when called directly.
    runtime_error_containing("setmetatable({}, { __gc = function() end })", "__gc");
    assert_eq!(value_of("return tostring({})"), Val::Str("table".into()));
    assert_eq!(
        value_of(
            r#"local o = {} for k in pairs({ z = 1, a = 2 }) do o[#o + 1] = k end return table.concat(o)"#
        ),
        Val::Str("az".into())
    );
}

#[test]
fn globals_are_frozen_after_load_so_state_cannot_leak_between_calls() {
    runtime_error_containing("counter = (counter or 0) + 1 return counter", "readonly");
    runtime_error_containing("evil = 1", "readonly");
}

#[test]
fn registration_is_closed_after_load() {
    runtime_error_containing(
        r#"pg.components.register({ name = "late", applies_to = {"pawn"}, fields = { a = pg.field.int(0, 1, 0) } })"#,
        "registries are closed",
    );
    runtime_error_containing(
        r#"pg.hooks.on("movement.speed_modifier", function() return 1000 end)"#,
        "registries are closed",
    );
}

#[test]
fn bytecode_cannot_be_loaded() {
    let mut vm = LuauVm::new(config(&all_caps())).unwrap();
    for src in [
        "\u{1b}Lua\u{5}\u{0}",
        "\u{1b}",
        "\u{3}\u{1}\u{0}\u{0}",
        "\u{6}\u{3}",
    ] {
        let r = vm.load(SourceChunk {
            name: ENTRY.into(),
            source: src.into(),
        });
        assert!(matches!(r, Err(VmError::Compile(_))), "{src:?}: {r:?}");
    }
    // No script-visible path compiles or loads anything.
    assert_eq!(
        value_of("return load == nil and loadstring == nil and dofile == nil and loadfile == nil and string.dump == nil"),
        Val::Bool(true)
    );
    assert!(luau_bytecode_is_never_accepted_as_text());
}

#[test]
fn syntax_and_runtime_errors_name_file_and_line() {
    let mut vm = LuauVm::new(config(&all_caps())).unwrap();
    let e = vm
        .load(SourceChunk {
            name: ENTRY.into(),
            source: "local x = = 1".into(),
        })
        .unwrap_err();
    assert!(
        matches!(&e, VmError::Compile(m) if m.contains("scripts/main.luau") && m.contains(":1")),
        "{e:?}"
    );
    match error_of("error('boom')") {
        VmError::Runtime(m) => {
            assert!(m.contains("scripts/main.luau") && m.contains("boom"), "{m}")
        }
        other => panic!("{other:?}"),
    }
    match error_of("local t = nil return t.x") {
        VmError::Runtime(m) => assert!(m.contains("scripts/main.luau"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        error_of("error({ code = 7 })"),
        VmError::Runtime(_)
    ));
}

// ---- determinism of the VM surface ---------------------------------------------------------------------------

#[test]
fn tostring_and_print_never_show_addresses() {
    assert_eq!(
        value_of(
            r#"return tostring({}) == "table" and tostring(print) == "function" and tostring(function() end) == "function" and tostring(5) == "5" and tostring(nil) == "nil""#
        ),
        Val::Bool(true)
    );
    assert_eq!(
        value_of(
            r#"return tostring(setmetatable({}, { __tostring = function() return "me" end }))"#
        ),
        Val::Str("me".into())
    );
    let (_, r, _) = run_body(r#"print({}, print, 1, "a") for i = 1, 40 do print("line", i) end"#);
    let r = r.unwrap();
    assert_eq!(r.log.len(), 16, "log lines are rate-limited per call");
    assert_eq!(
        r.log[0],
        (LogLevel::Info, "table\tfunction\t1\ta".to_owned())
    );
}

#[test]
fn pairs_order_does_not_depend_on_insertion_history() {
    let a = value_of(
        r#"
        local t = {}
        t.zeta = 1 t.alpha = 2 t[10] = 3 t.mid = 4 t[2] = 5 t[1] = 6
        local o = {} for k in pairs(t) do o[#o + 1] = tostring(k) end return table.concat(o, ",")"#,
    );
    let b = value_of(
        r#"
        local t = {}
        t[1] = 6 t[2] = 5 t.mid = 4 t[10] = 3 t.alpha = 2 t.zeta = 1
        for i = 1, 50 do t["filler" .. i] = i end
        for i = 1, 50 do t["filler" .. i] = nil end
        local o = {} for k in pairs(t) do o[#o + 1] = tostring(k) end return table.concat(o, ",")"#,
    );
    assert_eq!(a, Val::Str("1,2,10,alpha,mid,zeta".into()));
    assert_eq!(a, b);
    runtime_error_containing(
        "for k in pairs({ [true] = 1 }) do end",
        "only number and string keys",
    );
    runtime_error_containing("for k in pairs(5) do end", "table expected");
    assert_eq!(
        value_of("local n = 0 for _, v in ipairs({ 5, 6, 7 }) do n += v end return n"),
        Val::Int(18)
    );
}

#[test]
fn pairs_survives_removal_during_iteration() {
    assert_eq!(
        value_of("local t = { a = 1, b = 2, c = 3 } local n = 0 for k in pairs(t) do t.b = nil n += 1 end return n"),
        Val::Int(2)
    );
}

#[test]
fn numbers_cross_the_boundary_only_as_integers() {
    assert_eq!(value_of("return 5"), Val::Int(5));
    assert_eq!(value_of("return -7"), Val::Int(-7));
    assert_eq!(value_of("return 3.0"), Val::Int(3));
    assert_eq!(value_of("return 2^53"), Val::Int(1 << 53));
    assert_eq!(value_of("return nil"), Val::Nil);
    assert_eq!(value_of("return true"), Val::Bool(true));
    for body in [
        "return 1.5",
        "return 0/0",
        "return 2^60",
        "return -2^60",
        "return math.huge",
        "return -math.huge",
        "return 1/3",
    ] {
        assert!(matches!(error_of(body), VmError::NonInteger(_)), "{body}");
    }
    assert!(matches!(
        error_of(r#"ctx.cmd:set_field("pawn_1", "c", "f", 1.5)"#),
        VmError::NonInteger(_)
    ));
}

#[test]
fn values_crossing_the_boundary_are_bounded_and_typed() {
    assert_eq!(
        value_of("return { 1, 2, 3 }"),
        Val::List(vec![Val::Int(1), Val::Int(2), Val::Int(3)])
    );
    assert_eq!(
        value_of(r#"return { b = 2, a = 1 }"#),
        Val::map([("a", Val::Int(1)), ("b", Val::Int(2))])
    );
    for body in [
        "return function() end",
        "return print",
        "local t = {} t.t = t return t",
        r#"return { 1, 2, nil, 4 }"#,
        r#"return { 1, a = 2 }"#,
        r#"return { [true] = 1 }"#,
        r#"return "\xff""#,
        r#"return string.rep("x", 4000) .. string.rep("y", 200)"#,
        "local t = {} for i = 1, 2000 do t[i] = i end return t",
        "return { { { { { { { { { 1 } } } } } } } } }",
        "return buffer.create(4)",
    ] {
        assert!(matches!(error_of(body), VmError::BadValue(_)), "{body}");
    }
}

#[test]
fn commands_are_buffered_validated_and_capped() {
    let (_, r, _) = run_body(
        r#"ctx.cmd:set_field(e.id, "caffeine", "level", e:get("caffeine").level - 5) return 1"#,
    );
    let r = r.unwrap();
    assert_eq!(
        r.commands,
        vec![ScriptCommand::SetField {
            entity: "pawn_1".into(),
            component: "caffeine".into(),
            field: "level".into(),
            value: 45
        }]
    );
    runtime_error_containing(
        r#"for i = 1, 2000 do ctx.cmd:set_field("pawn_1", "c", "f", i) end"#,
        "too many commands",
    );
    // The context is read-only.
    runtime_error_containing("ctx.tick = 99", "readonly");
    runtime_error_containing("e.id = 'pawn_2'", "readonly");
    assert_eq!(value_of("return ctx.tick + ctx.day"), Val::Int(5));
}

#[test]
fn a_failing_call_discards_its_commands() {
    let (mut vm, r, reg) =
        run_body(r#"ctx.cmd:set_field("pawn_1", "c", "f", 1) error("late failure")"#);
    assert!(matches!(r, Err(VmError::Runtime(_))));
    // A later successful call carries only its own commands.
    let again = vm
        .call(reg.systems[1].handler, &entity_args(), Fuel(CALL_FUEL))
        .unwrap();
    assert!(again.commands.is_empty());
}

#[test]
fn modules_load_from_the_pack_and_are_frozen() {
    let mut vm = LuauVm::new(config(&all_caps())).unwrap();
    vm.load(SourceChunk {
        name: "scripts/util.luau".into(),
        source: "local M = {} function M.double(x) return x * 2 end return M".into(),
    })
    .unwrap();
    vm.load(SourceChunk {
        name: ENTRY.into(),
        source: r#"
        local util = require("scripts/util")
        local again = require("scripts/util")
        assert(util == again, "modules are cached")
        pg.systems.register({ id = "t", cadence = "tick", query = { kind = "pawn" },
            run = function() util.double = nil return util.double(21) end })
    "#
        .into(),
    })
    .unwrap();
    let reg = vm.run_load_phase(ENTRY, Fuel(CALL_FUEL)).unwrap();
    let e = vm
        .call(reg.systems[0].handler, &entity_args(), Fuel(CALL_FUEL))
        .unwrap_err();
    assert!(
        matches!(&e, VmError::Runtime(m) if m.contains("readonly")),
        "modules are frozen: {e:?}"
    );
}

#[test]
fn circular_requires_are_an_error() {
    let mut vm = LuauVm::new(config(&all_caps())).unwrap();
    vm.load(SourceChunk {
        name: "scripts/a.luau".into(),
        source: r#"return require("scripts/b")"#.into(),
    })
    .unwrap();
    vm.load(SourceChunk {
        name: "scripts/b.luau".into(),
        source: r#"return require("scripts/a")"#.into(),
    })
    .unwrap();
    vm.load(SourceChunk {
        name: ENTRY.into(),
        source: r#"require("scripts/a")"#.into(),
    })
    .unwrap();
    let e = vm.run_load_phase(ENTRY, Fuel(CALL_FUEL)).unwrap_err();
    assert!(
        matches!(&e, VmError::Runtime(m) if m.contains("circular")),
        "{e:?}"
    );
}

// ---- registration validation -------------------------------------------------------------------------------------

fn load_err(source: &str) -> String {
    match make(&all_caps(), source) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected a load failure for: {source}"),
    }
}

#[test]
fn registration_mistakes_get_precise_messages_with_hints() {
    let m =
        load_err(r#"pg.components.register({ name = "x", applys_to = {"pawn"}, fields = {} })"#);
    assert!(
        m.contains("no field 'applys_to'") && m.contains("did you mean 'applies_to'"),
        "{m}"
    );
    let m = load_err(r#"pg.hooks.on("movement.speed_modifer", function() return 1000 end)"#);
    assert!(m.contains("did you mean 'movement.speed_modifier'"), "{m}");
    let m = load_err(
        r#"pg.components.register({ name = "Bad Name", applies_to = {"pawn"}, fields = { a = pg.field.int(0, 1, 0) } })"#,
    );
    assert!(m.contains("lower-case"), "{m}");
    let m = load_err(
        r#"pg.components.register({ name = "x", applies_to = {"pwan"}, fields = { a = pg.field.int(0, 1, 0) } })"#,
    );
    assert!(m.contains("did you mean 'pawn'"), "{m}");
    let m = load_err(
        r#"pg.components.register({ name = "x", applies_to = {"pawn"}, fields = { a = pg.field.int(5, 1, 3) } })"#,
    );
    assert!(m.contains("min <= default <= max"), "{m}");
    let m = load_err(
        r#"pg.systems.register({ id = "s", cadence = "minut", query = { kind = "pawn" }, run = function() end })"#,
    );
    assert!(m.contains("did you mean 'minute'"), "{m}");
    let m = load_err(
        r#"pg.systems.register({ id = "s", cadence = "tick", after = "NeedSystem", query = { kind = "pawn" }, run = function() end })"#,
    );
    assert!(m.contains("did you mean 'NeedsSystem'"), "{m}");
    let m = load_err(
        r#"pg.systems.register({ id = "s", cadence = "tick", query = { kind = "pawn" } })"#,
    );
    assert!(m.contains("`run` function"), "{m}");
    let m = load_err(
        r#"local c = { name = "x", applies_to = {"pawn"}, fields = { a = pg.field.int(0, 1, 0) } }
           pg.components.register(c) pg.components.register(c)"#,
    );
    assert!(m.contains("registered twice"), "{m}");
    let m = load_err(r#"pg.componets.register({})"#);
    assert!(m.contains("componets") || m.contains("nil"), "{m}");
}

#[test]
fn capabilities_gate_the_api_structurally() {
    // With no capabilities a pack still has the pure helpers, but not the registries.
    let mut vm = LuauVm::new(config(&[])).unwrap();
    vm.load(SourceChunk { name: ENTRY.into(), source: "assert(pg.components == nil and pg.systems == nil and pg.hooks == nil and pg.pack == nil) assert(pg.rand ~= nil and pg.math.idiv ~= nil and pg.log.info ~= nil)".into() }).unwrap();
    vm.run_load_phase(ENTRY, Fuel(CALL_FUEL)).unwrap();
    let mut vm = LuauVm::new(config(&["data"])).unwrap();
    vm.load(SourceChunk {
        name: ENTRY.into(),
        source: r#"pg.systems.register({})"#.into(),
    })
    .unwrap();
    let e = vm.run_load_phase(ENTRY, Fuel(CALL_FUEL)).unwrap_err();
    assert!(matches!(e, VmError::Runtime(_)));
}

#[test]
fn load_phase_has_a_fuel_budget_too() {
    let e = make(&all_caps(), "while true do end").err().unwrap();
    assert!(matches!(e, VmError::OutOfFuel { .. }));
}

// ---- pg helpers ------------------------------------------------------------------------------------------------------

#[test]
fn integer_helpers_are_exact() {
    assert_eq!(value_of("return pg.math.idiv(7, 2)"), Val::Int(3));
    assert_eq!(value_of("return pg.math.idiv(-7, 2)"), Val::Int(-4));
    assert_eq!(value_of("return pg.math.idiv(7, -2)"), Val::Int(-4));
    assert_eq!(value_of("return pg.math.idiv(-7, -2)"), Val::Int(3));
    runtime_error_containing("return pg.math.idiv(1, 0)", "division by zero");
    assert_eq!(value_of("return pg.math.muldiv(10, 3, 4)"), Val::Int(8)); // 7.5 -> 8
    assert_eq!(value_of("return pg.math.muldiv(-10, 3, 4)"), Val::Int(-7)); // -7.5 -> -7 (half up)
    assert_eq!(value_of("return pg.math.muldiv(5, 5, -2)"), Val::Int(-12)); // -12.5 -> -12
    assert_eq!(
        value_of("return pg.math.muldiv(4000000000, 4000000000, 4000000000)"),
        Val::Int(4_000_000_000)
    );
    assert_eq!(value_of("return pg.math.isqrt(0)"), Val::Int(0));
    assert_eq!(value_of("return pg.math.isqrt(99)"), Val::Int(9));
    assert_eq!(value_of("return pg.math.isqrt(100)"), Val::Int(10));
    assert_eq!(value_of("return pg.math.isqrt(2^52)"), Val::Int(67_108_864));
    runtime_error_containing("return pg.math.isqrt(-1)", "negative");
    assert_eq!(
        value_of("return pg.math.lerp_permille(100, 200, 500)"),
        Val::Int(150)
    );
    assert_eq!(
        value_of("return pg.math.lerp_permille(0, 10, 250)"),
        Val::Int(3)
    ); // 2.5 -> 3
    runtime_error_containing("return pg.math.idiv(1.5, 2)", "not an integer");
    runtime_error_containing("return pg.math.muldiv(2^52, 2^52, 1)", "out of range");
}

#[test]
fn pg_rand_is_a_pure_function_of_seed_stream_and_keys() {
    let a = value_of(r#"return pg.rand("coffee", 5, "pawn_1")"#);
    assert_eq!(a, value_of(r#"return pg.rand("coffee", 5, "pawn_1")"#));
    assert_ne!(a, value_of(r#"return pg.rand("coffee", 6, "pawn_1")"#));
    assert_ne!(a, value_of(r#"return pg.rand("tea", 5, "pawn_1")"#));
    // A different world seed gives different numbers; the same seed gives the same ones in a new VM.
    let mut other = LuauVm::new(VmConfig::new("test", &["systems"], 8)).unwrap();
    other.load(SourceChunk { name: ENTRY.into(), source: r#"pg.systems.register({ id = "t", cadence = "tick", query = { kind = "pawn" }, run = function() return pg.rand("coffee", 5, "pawn_1") end })"#.into() }).unwrap();
    let reg = other.run_load_phase(ENTRY, Fuel(CALL_FUEL)).unwrap();
    let b = other
        .call(reg.systems[0].handler, &entity_args(), Fuel(CALL_FUEL))
        .unwrap()
        .value;
    assert_ne!(a, b);
    runtime_error_containing(r#"return pg.rand("Bad Stream")"#, "valid stream name");
    runtime_error_containing(
        r#"return pg.rand("s", 1, 2, 3, 4, 5, 6, 7)"#,
        "at most 6 keys",
    );
    runtime_error_containing(r#"return pg.rand("s", {})"#, "integers or text");
    runtime_error_containing("return pg.rand(5)", "stream name");
    // Matches the core's own stream machinery.
    let direct = pg_core::rng::Rng::new(
        pg_core::rng::Seed(7),
        pg_core::rng::Stream::Pack {
            pack: "test",
            name: "coffee",
        },
        &[pg_core::rng::Key::Int(5), pg_core::rng::Key::Str("pawn_1")],
    )
    .draw(0);
    assert_eq!(a, Val::Int(i64::from(direct)));
}

// ---- fuel determinism -------------------------------------------------------------------------------------------------

const FUEL_SCRIPT: &str = r#"
    pg.systems.register({ id = "t", cadence = "tick", query = { kind = "pawn" },
        run = function(ctx, e)
            local total = 0
            for i = 1, 100 do
                total += i % 7
                if i % 10 == 0 then total += pg.math.idiv(total, 3) end
            end
            local keys = {}
            for k in pairs({ b = 1, a = 2, c = 3 }) do keys[#keys + 1] = k end
            return total + #table.concat(keys)
        end })
"#;

#[test]
fn fuel_is_deterministic_and_pinned() {
    let measure = || {
        let (mut vm, reg) = make(&all_caps(), FUEL_SCRIPT).unwrap();
        let r = vm
            .call(reg.systems[0].handler, &entity_args(), Fuel(CALL_FUEL))
            .unwrap();
        (r.value, r.fuel)
    };
    let first = measure();
    assert_eq!(
        first,
        measure(),
        "same script, same input, fresh VM: same fuel"
    );
    // The pinned figure is the cross-platform check (spike question 1): CI runs this on Windows, Linux and
    // macOS, so any difference in bytecode generation or safepoints between targets fails here.
    assert_eq!(
        first,
        (Val::Int(PINNED_VALUE), PINNED_FUEL),
        "if the compiler or prelude changed on purpose, update both figures and record why"
    );
}

const PINNED_VALUE: i64 = 1987;
const PINNED_FUEL: u64 = 145;

#[test]
fn fuel_does_not_depend_on_call_order_or_history() {
    let (mut vm, reg) = make(&all_caps(), FUEL_SCRIPT).unwrap();
    let first = vm
        .call(reg.systems[0].handler, &entity_args(), Fuel(CALL_FUEL))
        .unwrap()
        .fuel;
    for _ in 0..5 {
        let r = vm
            .call(reg.systems[0].handler, &entity_args(), Fuel(CALL_FUEL))
            .unwrap();
        assert_eq!(r.fuel, first, "later calls cost the same as the first");
    }
    // A call that fails part-way does not change what the next one costs.
    let _ = vm.call(reg.systems[0].handler, &entity_args(), Fuel(10));
    assert_eq!(
        vm.call(reg.systems[0].handler, &entity_args(), Fuel(CALL_FUEL))
            .unwrap()
            .fuel,
        first
    );
}

// ---- the VM-reload variant (Blueprint §18 item 12) --------------------------------------------------------------------

#[test]
fn a_hidden_closure_state_is_caught_by_rebuilding_the_vm() {
    // The one way a script can keep state between calls despite frozen globals: a captured local.
    let leaky = r#"
        local calls = 0
        pg.systems.register({ id = "t", cadence = "tick", query = { kind = "pawn" },
            run = function() calls += 1 return calls end })
    "#;
    let clean = r#"pg.systems.register({ id = "t", cadence = "tick", query = { kind = "pawn" }, run = function() return 7 end })"#;
    let persistent = |src: &str| {
        let (mut vm, reg) = make(&all_caps(), src).unwrap();
        (0..3)
            .map(|_| {
                vm.call(reg.systems[0].handler, &entity_args(), Fuel(CALL_FUEL))
                    .unwrap()
                    .value
            })
            .collect::<Vec<_>>()
    };
    let reloaded = |src: &str| {
        (0..3)
            .map(|_| {
                let (mut vm, reg) = make(&all_caps(), src).unwrap();
                vm.call(reg.systems[0].handler, &entity_args(), Fuel(CALL_FUEL))
                    .unwrap()
                    .value
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        persistent(clean),
        reloaded(clean),
        "a clean pack is identical either way"
    );
    assert_ne!(
        persistent(leaky),
        reloaded(leaky),
        "hidden state shows up as a difference when the VM is rebuilt"
    );
}

#[test]
fn identical_calls_give_identical_results_across_vms() {
    let run = || {
        let (mut vm, reg) = make(&all_caps(), FUEL_SCRIPT).unwrap();
        (0..4)
            .map(|_| {
                vm.call(reg.systems[0].handler, &entity_args(), Fuel(CALL_FUEL))
                    .unwrap()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}

// ---- batching and the boundary cost -----------------------------------------------------------------------------------

#[test]
fn a_batch_runs_every_call_independently() {
    let (mut vm, reg) = make(&all_caps(), r#"
        pg.systems.register({ id = "t", cadence = "tick", query = { kind = "pawn" },
            run = function(ctx, e) if e.id == "pawn_2" then error("only pawn_2 fails") end return ctx.tick end })
    "#).unwrap();
    let args: Vec<Val> = (1..=3)
        .map(|i| {
            Val::map([
                ("ctx", Val::map([("tick", Val::Int(i))])),
                ("entity", Val::map([("id", Val::Str(format!("pawn_{i}")))])),
            ])
        })
        .collect();
    let out = vm.call_batch(reg.systems[0].handler, &args, Fuel(CALL_FUEL));
    assert_eq!(out[0].as_ref().unwrap().value, Val::Int(1));
    assert!(out[1].is_err());
    assert_eq!(out[2].as_ref().unwrap().value, Val::Int(3));
}

// ---- a pack that uses everything --------------------------------------------------------------------------------------

#[test]
fn the_blueprint_example_pack_registers_everything() {
    let (_, reg) = make(&all_caps(), r#"
        --!strict
        pg.components.register({ name = "caffeine", applies_to = { "pawn" }, fields = { level = pg.field.int(0, 1000, 0) } })
        pg.systems.register({ id = "caffeine_decay", cadence = "minute", after = "NeedsSystem",
            query = { kind = "pawn", with = { "caffeine" } }, writes = { "caffeine" },
            run = function(ctx, pawn)
                local level = pawn:get("caffeine").level
                if level > 0 then ctx.cmd:set_field(pawn.id, "caffeine", "level", math.max(0, level - 5)) end
            end })
        pg.hooks.on("movement.speed_modifier", function(ctx)
            local c = ctx.pawn:get("caffeine")
            if c and c.level > 200 then return 1200 end
            return 1000
        end)
    "#).unwrap();
    assert_eq!(reg.components.len(), 1);
    assert_eq!(reg.systems[0].after.as_deref(), Some("NeedsSystem"));
    assert_eq!(reg.systems[0].with, vec!["caffeine".to_string()]);
    assert_eq!(reg.systems[0].writes, vec!["caffeine".to_string()]);
    assert_eq!(reg.hooks[0].point, "movement.speed_modifier");
}

// Bytecode is produced here only to prove it is refused; no other code names the binding.
fn luau_bytecode_is_never_accepted_as_text() -> bool {
    crate::luau::bytecode_is_rejected_as_source()
}
