--!strict
-- A module: loaded with require, cached and frozen. Constants and pure helpers only; module-level
-- mutable state is not allowed (and would be caught by the VM-reload test).
local M = {}
M.MAX_AGE_DAYS = 36500
function M.next_age(age: number): number
  return math.min(M.MAX_AGE_DAYS, age + 1)
end
return M
