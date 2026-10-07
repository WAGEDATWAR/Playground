# ScriptVm spike (milestone 0.9)

Answers to the seven questions in Blueprint §23.15 and Roadmap §12 item 1, with measurements. Recorded
2026-10-07 on Windows 11 (MSVC, Rust 1.99.0, `mlua` 0.12.2 with vendored Luau 0.740). **Linux and macOS
figures come from CI** and are marked *(CI)* until a green run is recorded; CI was green on all three systems at m0.9, so the pinned fuel value (145) and the contraction probe agree on Windows, Linux and macOS.

**Decision: `mlua` with the Luau backend stays behind the `ScriptVm` boundary.** None of the upgrade
triggers in §23.15 has been hit. The hostile-pack corpus found four places where the profile in the
Blueprint was not enough by itself; all four were fixed inside the one private module (see "Findings").

## 1. Fuel determinism

Fuel is counted at Luau's interrupt safepoints (function calls and loop back-edges), configured with one
fixed compiler setting (optimisation level 1, debug level 1) so every target compiles the same bytecode.

* The same script costs the same on every call, in a fresh VM, in a VM that has run other calls, and after
  a call that was cut off part-way (`fuel_does_not_depend_on_call_order_or_history`).
* The pinned test script costs **145** units and returns 1987 (`fuel_is_deterministic_and_pinned`). The
  test runs in CI on Windows, Linux and macOS, so any difference between targets fails there. *(CI)*
* Granularity: a runaway `while true do end` is cut off within 10 units of its budget
  (`a_loop_is_cut_off_close_to_its_budget`), which is microseconds. A loop inside `pcall`, an infinite
  tail call and an error handler that loops are all stopped; a script that catches the out-of-fuel error and
  carries on still fails the call.
* The interrupt is also called inside the pattern matcher, so catastrophic patterns such as
  `(("a"):rep(400)):find(".-.-.-.-.-b")` are stopped by fuel in under 50 ms. The Blueprint assumed built-in
  C functions could not be interrupted and planned length caps; none are needed and none are built.

## 2. Numeric build control

* **Native code generation:** the `luau-jit` feature is not enabled, so the code generator is not
  compiled. `scripts/check_deps.py` fails if `luau-jit` or `luau-vector4` ever appears in the feature tree.
* **Floating-point contraction:** `.cargo/config.toml` sets `-ffp-contract=off` for the GCC/Clang targets
  and `/fp:precise` for MSVC through the `cc` crate's per-target `CXXFLAGS_<target>`. The test
  `floating_point_contraction_is_off` computes `(2^27+1)^2 - (2^54+2^28)`, which is 0 without fused
  multiply-add and 1 with it, and runs on every target in CI. *(CI: aarch64 is the target that matters;
  GitHub's macOS runners are Apple silicon.)*
* The vendored build passes `-fno-math-errno` for `sqrt`; `math.sqrt` is removed from the profile anyway.

## 3. Memory limit

* The cap is set after setup (the VM holds about **650 KiB** at rest, with the prelude and `pg`); default
  8 MiB per pack.
* Allocation bombs (`string.rep`, doubling a string, `table.create(1e8)`, `buffer.create(2^30)`, filling a
  table) abort the call with `OutOfMemory`, the VM answers the next call correctly, and its memory returns
  to normal (`memory_bombs_abort_cleanly_and_leave_the_vm_usable`).
* One pack running out of memory leaves another pack's VM untouched
  (`one_packs_memory_exhaustion_does_not_touch_another`): each pack has its own Luau state and allocator.

## 4. Boundary cost

`pg pack bench` (release build, 200 pawns on 96x72, the three cookbook packs, one simulated day):

| | per tick |
| --- | --- |
| without scripts | 0.10 ms |
| with scripts (one hook asked per walking pawn per tick, one minute system, one slot system, one day system) | 1.7 ms |
| scripts add | 1.6 ms (1.6% of the 100 ms tick at 1x) |

* One handler call with an entity view costs **about 12 µs** (marshalling the view in, running ~18 fuel,
  marshalling the command out). A per-minute system over 200 pawns is about **2.4 ms once a minute**.
* `call_batch` costs the same per element (12.5 µs) because it is a loop over `call`; the saving a batch
  could give is in building the context once and entering Lua once per batch, which needs a batch entry
  point in the prelude. Recorded as S-034; not needed at these numbers.
* At 27x (270 ticks per second) scripts with these packs use about 0.45 s of CPU per second on the
  simulation thread, so heavy packs at top speed will need budgets or batching. The profiler (`pg profile`,
  `pg pack bench`) shows this per pack.
* A Rust rewrite of the same systems was not benchmarked: the question it answers (is the overhead
  tolerable) is answered by the absolute numbers above.

## 5. Sandbox profile and the hostile-pack corpus

All installed after VM creation and enforced: deterministic `pairs`, `tostring` and `print` without
addresses, `math` and `string` restricted in place, removed libraries and globals, sealed library tables,
frozen user globals after load, `__gc` and `__mode` rejected. The corpus (`crates/pg-script/src/tests.rs`,
39 tests) covers: escapes through `require`, direct and indirect access to removed functions, tampering with
builtins and the string metatable, runaway loops (including inside `pcall`), deep recursion, memory bombs,
catastrophic patterns, hostile return values (floats, `NaN`, infinities, cycles, deep and huge tables,
invalid UTF-8, functions), command floods, late registration, bytecode, circular and traversing requires,
and capability gating (`pg` contains exactly the functions the capabilities allow, checked against `pg-api`).

### Findings (all fixed in the host; none needs an upgrade trigger)

1. **Compiler fast paths bypass removed and replaced globals.** `math.sin(1)` still worked after
   `math.sin = nil` because Luau compiles it to a fast call that never reads the global; a replaced
   `setmetatable` and `pairs` were bypassed the same way (the compiler lowers `for k, v in pairs(t)` to a
   direct `next` loop). Fixed with the compiler's `mutable_globals` and `disabled_builtins`; the corpus now
   calls every removed function directly.
2. **`mlua` integers are `i64`.** Numbers with an exact integer value arrive as integers, so `2^60` passed
   the "is it a float" check; every conversion now range-checks to ±2^53.
3. **String caps are unnecessary** (see question 1).
4. **Late `__mode`.** Attaching a metatable and adding `__mode` afterwards would give a weak table;
   metatables are frozen when attached.

### Known gap

Bare `for k, v in t do` and `next(t)` cannot be intercepted, so their order follows Luau's table layout.
For string and number keys it is stable for a given insertion history, which is why scripts are
deterministic in practice, but it depends on history rather than content, and tables with function or
table keys hash by address. Mitigations: `pg pack lint` reports bare iteration, `pairs` is deterministic by
construction, and the VM-reload test (rebuilding every VM at each day boundary) is the backstop. If this
proves fragile the fix belongs inside Luau, which is one of the upgrade triggers.

## 6. Source-only loading

`ChunkMode::Text` is forced for every chunk, so real Luau bytecode offered as source is a syntax error
(`bytecode_cannot_be_loaded`); sources starting with the ESC byte are refused before compiling and any other
bytes are compiled as text, which fails as a syntax error; `load`, `loadstring`, `dofile`, `loadfile` and `string.dump` do not exist; and
`require` can only return a source the host staged from the pack. Pack files must be valid UTF-8 text.

## 7. Build and licensing

* The vendored build (`luau0-src`) compiles in about 35 s on Windows with MSVC and needs only a C++
  compiler, which the Rust toolchain already requires on Windows. `cargo deny check` passes with no new
  licence entries (MIT). *(CI for Linux and macOS.)*

## Budgets set from the measurements

| Budget | Value | Why |
| --- | --- | --- |
| `fuel_per_call` | 50,000 | typical handlers use 20 to 200 units; this allows loops of thousands of iterations |
| `fuel_per_tick` | 1,000,000 | about 15 ms of script work worst case; a tick that uses it up stops the system and records an error |
| `load_fuel` | 2,000,000 | registration is cheap (the cookbook packs use 2 to 11) |
| `memory_bytes` | 8 MiB | 12 times the VM baseline |
| quarantine | 3 errors within 14,400 ticks (one day), or any load failure | enough to ignore a one-off, quick to stop a loop |

## What is built and what is deferred

Built: the `ScriptVm` boundary and Luau implementation, `pg` API from `pg-api` gated by capabilities, one
component + system + hook driven by real packs through the pack loader, quarantine kept in the world,
safe mode, VM-reload variant, golden hashes for three cookbook packs, `pg pack lint|test|docs|new|bench`,
`pg content schema`, hints for API names.

Deferred (recorded in D-031): the wall-clock watchdog and `PackQuarantined` as a recorded input, explicit
component attach and migration functions, capability approval prompts (Stage 1), effects beyond own-field
writes, per-pack settings, `pg.world` queries beyond the views passed in, real batching (S-034), Luau type
checking in `pg pack lint` (S-035), and the Mods screen.
