-- A Lua reimplementation of the scroll-mode builtin
-- (crates/ekko-builtins/src/scroll_mode.rs): keyboard navigation through
-- the session's server-side scrollback. The mechanism (the scroll actions,
-- the wire round-trip) lives in the hosts; a mode is only the key policy,
-- so it fits in a script.
--
-- To replace the builtin wholesale, disable it in config and drop this
-- file in ~/.config/ekko/extensions/:
--
--   [extensions]
--   disabled = ["ekko-builtins.scroll-mode"]
--
-- The mode keeps its name ("scroll"), so every existing binding that
-- enters it keeps working. Without the disable, the duplicate mode name
-- fails the runtime build loudly.

local ext = {
  id = "user.scroll-mode",
  name = "scroll mode (lua)",
  version = "0.1.0",
  description = "keyboard scrollback navigation (j/k, u/d, g/G)",
}

-- Matches an SGR wheel report (ESC [ < 64;...M up / 65;...M down): raw
-- reports reach the mode before the host's mouse routing.
local function wheel(bytes, up)
  local button = up and "64;" or "65;"
  return bytes:sub(1, 3) == "\27[<"
    and bytes:sub(4, 6) == button
    and bytes:sub(-1) == "M"
end

function ext.register(ekko)
  ekko.register_mode({
    name = "scroll",
    on_key = function(state, bytes, snapshot)
      local half_page = math.max(1, math.floor(snapshot.grid_rows / 2))
      local full_page = math.max(1, snapshot.grid_rows)
      if bytes == "k" or bytes == "\27[A" or bytes == "\27OA" then
        return { scroll = 1 }
      elseif bytes == "j" or bytes == "\27[B" or bytes == "\27OB" then
        return { scroll = -1 }
      -- u/d: half page; PageUp/PageDown (CSI 5~/6~): full page.
      elseif bytes == "u" or bytes == "\21" then
        return { scroll = half_page }
      elseif bytes == "d" or bytes == "\4" then
        return { scroll = -half_page }
      elseif bytes == "\27[5~" then
        return { scroll = full_page }
      elseif bytes == "\27[6~" then
        return { scroll = -full_page }
      -- g: jump to the top of history (the server clamps).
      elseif bytes == "g" then
        return { scroll = 2147483647 }
      -- G: back to the live screen, stay in the mode.
      elseif bytes == "G" then
        return { "scroll_to_bottom" }
      elseif wheel(bytes, true) then
        return { scroll = 3 }
      elseif wheel(bytes, false) then
        return { scroll = -3 }
      elseif bytes == "q" or bytes == "\27" or bytes == "\r" or bytes == "\n" then
        return { "exit", "scroll_to_bottom" }
      end
      -- Anything else is swallowed: stay in the mode.
    end,
  })
end

return ext
