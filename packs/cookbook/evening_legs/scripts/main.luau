--!strict
-- Cookbook 2: a hook-only pack, no component and no system. Pawns walk slower after 20:00.
-- There is no clock in scripts: the only time source is the tick in the context (10 ticks per minute).

local TICKS_PER_MINUTE = 10
local MINUTES_PER_DAY = 1440
local EVENING = 20 * 60

pg.hooks.on("movement.speed_modifier", function(ctx)
  local minute_of_day = pg.math.idiv(ctx.tick, TICKS_PER_MINUTE) % MINUTES_PER_DAY
  if minute_of_day >= EVENING then
    return 800
  end
  return 1000
end)
