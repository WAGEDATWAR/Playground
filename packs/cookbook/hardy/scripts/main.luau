--!strict
-- Cookbook 4: hooks that visibly change how residents live.
--   * `hardy.grit` (0..1000) is stored per pawn; 0 means ordinary.
--   * `toughen` (every 30-minute slot) gives a third of the pawns some grit, once, from the pack's own stream.
--   * `need.decay_modifier` makes gritty pawns need to eat, sleep and socialise less often.
--   * `mood.comfort_shift` makes them feel a little better off than their needs say.
--   * `conversation.chance_modifier` makes them a bit chattier.
-- Hooks read the pawn's components, needs and mood (API 0.1) and answer with a small integer; the engine
-- combines the packs' answers and clamps them, so a hook can bias a decision but never break a rule.
-- Open a gritty resident in the inspector to see these effects listed against this pack.

pg.components.register({
  name = "hardy",
  applies_to = { "pawn" },
  fields = { grit = pg.field.int(0, 1000, 0) },
})

pg.systems.register({
  id = "toughen",
  cadence = "slot",
  query = { kind = "pawn", with = { "hardy" } },
  writes = { "hardy" },
  run = function(ctx, pawn)
    if pawn:get("hardy").grit == 0 and pg.rand("toughen", pawn.id) % 3 == 0 then
      -- 300 to 900, a pure function of the world seed and the pawn.
      ctx.cmd:set_field(pawn.id, "hardy", "grit", 300 + pg.rand("grit", pawn.id) % 601)
    end
  end,
})

pg.hooks.on("need.decay_modifier", function(ctx)
  local h = ctx.pawn:get("hardy")
  if h == nil or h.grit == 0 then
    return 1000
  end
  -- Up to 40 percent slower at full grit.
  return 1000 - pg.math.idiv(h.grit * 400, 1000)
end)

pg.hooks.on("mood.comfort_shift", function(ctx)
  local h = ctx.pawn:get("hardy")
  if h == nil then
    return 0
  end
  return pg.math.idiv(h.grit, 10)
end)

pg.hooks.on("conversation.chance_modifier", function(ctx)
  local h = ctx.pawn:get("hardy")
  if h ~= nil and h.grit > 0 then
    return 1200
  end
  return 1000
end)
