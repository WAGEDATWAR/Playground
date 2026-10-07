--!strict
-- Cookbook 1: a component, two systems and a hook.
--   * `caffeine.level` is stored per pawn (0 means none; saved and hashed by the core).
--   * `brew` (every 30-minute slot) gives some pawns a coffee, chosen with the pack's own random stream.
--   * `decay` (every minute) wears the caffeine off.
--   * the `movement.speed_modifier` hook makes caffeinated pawns walk faster.
-- Scripts only *request* changes through `ctx.cmd`; the core validates and applies them.

pg.components.register({
  name = "caffeine",
  applies_to = { "pawn" },
  fields = { level = pg.field.int(0, 1000, 0) },
})

pg.systems.register({
  id = "brew",
  cadence = "slot",
  query = { kind = "pawn", with = { "caffeine" } },
  writes = { "caffeine" },
  run = function(ctx, pawn)
    -- A pure function of (world seed, stream, keys): the same on every machine and in every replay.
    if pg.rand("brew", ctx.tick, pawn.id) % 4 == 0 then
      ctx.cmd:set_field(pawn.id, "caffeine", "level", 600)
    end
  end,
})

pg.systems.register({
  id = "decay",
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

-- Combined with other packs' answers by multiplying permille factors, then clamped by the engine (500..1500).
pg.hooks.on("movement.speed_modifier", function(ctx)
  local c = ctx.pawn:get("caffeine")
  if c ~= nil and c.level > 200 then
    return 1300
  end
  return 1000
end)
