--!strict
-- Cookbook 3: a daily system over several files. Counts how many days each pawn has been in the town.
local limits = require("scripts/limits")

pg.components.register({
  name = "town_age",
  applies_to = { "pawn" },
  fields = { days = pg.field.int(0, limits.MAX_AGE_DAYS, 0) },
})

pg.systems.register({
  id = "age_one_day",
  cadence = "day",
  query = { kind = "pawn", with = { "town_age" } },
  writes = { "town_age" },
  run = function(ctx, pawn)
    local age = pawn:get("town_age").days
    ctx.cmd:set_field(pawn.id, "town_age", "days", limits.next_age(age))
  end,
})
