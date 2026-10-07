--!nonstrict
-- The trusted prelude (Blueprint §23.4). It is compiled into the host and runs exactly once per VM, before
-- any pack code and before the sandbox is sealed, with access to the raw globals. It
--   * removes every global that is not on the allow-list,
--   * replaces `pairs`, `tostring`, `print`, `setmetatable` and `require` with deterministic or restricted
--     versions,
--   * restricts `math` and `string` in place (the string metatable points at the original table, so
--     replacing the table would leave the originals reachable through `("x"):rep(...)`), and
--   * builds the `invoke` function the host uses to call pack handlers.
-- Arguments: host (functions provided by Rust), cfg (limits).

local host, cfg = ...

-- Originals, captured before anything is replaced.
local type, typeof, next, error, select = type, typeof, next, error, select
local rawget, rawequal = rawget, rawequal
local setmetatable_raw, getmetatable_raw = setmetatable, getmetatable
local tostring_raw = tostring
local table_sort, table_freeze, table_isfrozen = table.sort, table.freeze, table.isfrozen
local G = _G

-- 1. Allow-listed globals ---------------------------------------------------------------------------------
local ALLOWED_GLOBALS = {
    assert = true, error = true, getmetatable = true, ipairs = true, next = true, pairs = true,
    pcall = true, print = true, rawequal = true, rawget = true, rawlen = true, rawset = true,
    require = true, select = true, setmetatable = true, tonumber = true, tostring = true, type = true,
    typeof = true, unpack = true, xpcall = true, _G = true, _VERSION = true,
    math = true, string = true, table = true, utf8 = true, bit32 = true, buffer = true, pg = true,
}
local doomed, nd = {}, 0
for k in next, G do
    if not ALLOWED_GLOBALS[k] then
        nd += 1
        doomed[nd] = k
    end
end
for i = 1, nd do
    G[doomed[i]] = nil
end

-- Keeps only `allowed` fields of a library table, in place.
local function restrict(lib, allowed)
    local gone, n = {}, 0
    for k in next, lib do
        if not allowed[k] then
            n += 1
            gone[n] = k
        end
    end
    for i = 1, n do
        lib[gone[i]] = nil
    end
end

-- 2. math: no ambient randomness, no transcendental functions (they differ between platforms) -------------
restrict(math, { min = true, max = true, abs = true, floor = true, ceil = true, clamp = true, sign = true, huge = true })

-- 3. string: nothing to cap. Luau's pattern matcher calls the interrupt on every step, so fuel bounds it, and
-- allocation (`string.rep`, `..`, `format`) is bounded by the memory limit. Only unneeded members go.
restrict(string, {
    byte = true, char = true, find = true, format = true, gmatch = true, gsub = true, len = true, lower = true,
    match = true, rep = true, reverse = true, sub = true, upper = true, pack = true, packsize = true,
    unpack = true, split = true,
})

-- 4. Deterministic iteration ------------------------------------------------------------------------------
-- `pairs` visits number keys ascending, then string keys in byte order, whatever the insertion history.
-- Sorting each homogeneous group with the built-in comparison costs no script fuel.
local function det_pairs(t)
    if type(t) ~= "table" then
        error("invalid argument #1 to 'pairs' (table expected, got " .. typeof(t) .. ")", 2)
    end
    local nums, strs, nn, ns = {}, {}, 0, 0
    for k in next, t do
        local tk = type(k)
        if tk == "number" then
            nn += 1
            nums[nn] = k
        elseif tk == "string" then
            ns += 1
            strs[ns] = k
        else
            error("pairs: only number and string keys are supported (found a " .. tk .. " key)", 2)
        end
    end
    table_sort(nums)
    table_sort(strs)
    local keys = nums
    for i = 1, ns do
        keys[nn + i] = strs[i]
    end
    local total, i = nn + ns, 0
    return function()
        while i < total do
            i += 1
            local k = keys[i]
            local v = t[k]
            if v ~= nil then
                return k, v
            end
        end
        return nil
    end, t, nil
end
G.pairs = det_pairs

-- 5. tostring / print never reveal addresses ---------------------------------------------------------------
local function det_tostring(v)
    local tv = type(v)
    if tv == "table" then
        local mt = getmetatable_raw(v)
        if type(mt) == "table" and rawget(mt, "__tostring") ~= nil then
            return tostring_raw(v)
        end
        return "table"
    elseif tv == "nil" or tv == "boolean" or tv == "number" or tv == "string" then
        return tostring_raw(v)
    end
    return typeof(v)
end
G.tostring = det_tostring
G.print = function(...)
    local n = select("#", ...)
    local parts = {}
    for i = 1, n do
        parts[i] = det_tostring((select(i, ...)))
    end
    host.log(1, table.concat(parts, "\t"))
end

-- 6. Metatables: no finalizers, no weak tables ------------------------------------------------------------
-- Luau honours `__mode` whenever the collector runs, so the metatable is frozen when it is attached:
-- adding `__mode` afterwards is impossible.
G.setmetatable = function(t, mt)
    if mt ~= nil then
        if type(mt) ~= "table" then
            error("setmetatable: the metatable must be a table or nil", 2)
        end
        if rawget(mt, "__gc") ~= nil then
            error("setmetatable: __gc is not allowed", 2)
        end
        if rawget(mt, "__mode") ~= nil then
            error("setmetatable: __mode (weak tables) is not allowed", 2)
        end
        if not table_isfrozen(mt) then
            table_freeze(mt)
        end
    end
    return setmetatable_raw(t, mt)
end

-- 7. Pack-local require -------------------------------------------------------------------------------------
local loaded, loading = {}, {}
G.require = function(name)
    if type(name) ~= "string" then
        error("require: the module name must be a string", 2)
    end
    local cached = loaded[name]
    if cached ~= nil then
        return cached
    end
    if loading[name] then
        error("require: circular dependency through '" .. name .. "'", 2)
    end
    local chunk = host.module(name)
    loading[name] = true
    local result = chunk()
    loading[name] = nil
    if result == nil then
        result = true
    end
    if type(result) == "table" and not table_isfrozen(result) then
        table_freeze(result)
    end
    loaded[name] = result
    return result
end

-- 8. Handler invocation --------------------------------------------------------------------------------------
-- A call receives its context as plain data and returns the commands it buffered. Entities become
-- read-only views with a `get` method.
local ENTITY = {}
ENTITY.__index = ENTITY
function ENTITY.get(self, component)
    local c = rawget(self, "components")
    if c == nil then
        return nil
    end
    return c[component]
end
table_freeze(ENTITY)

local function view(e)
    if type(e) ~= "table" then
        return e
    end
    return table_freeze(setmetatable_raw(e, ENTITY))
end

local MAX_COMMANDS = 1024
local function new_buffer(list)
    local n = 0
    local buffer = {}
    function buffer.set_field(_, entity, component, field, value)
        if n >= MAX_COMMANDS then
            error("too many commands in one call (limit " .. MAX_COMMANDS .. ")", 2)
        end
        n += 1
        list[n] = { op = "set_field", entity = entity, component = component, field = field, value = value }
    end
    return table_freeze(buffer)
end

local function invoke(fn, args)
    local ctx = args.ctx
    if ctx == nil then
        ctx = {}
    end
    local list = {}
    ctx.cmd = new_buffer(list)
    for k, v in next, ctx do
        if type(v) == "table" and rawget(v, "components") ~= nil then
            ctx[k] = view(v)
        end
    end
    table_freeze(ctx)
    local entity = args.entity
    local result
    if entity ~= nil then
        result = fn(ctx, view(entity))
    else
        result = fn(ctx)
    end
    return result, list
end

return invoke
