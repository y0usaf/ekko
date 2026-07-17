-- Example user pane map: the whole stock pane surface reimplemented as a
-- script. Disable the builtin in init.lua and this file replaces it
-- wholesale — same command names, same leader keys, no privileged path:
--
--   return { extensions = { disabled = { "ekko-builtins.panes" } } }
--
-- Commands and leader entries are ordinary registrations, so help and the
-- which-key panel pick them up from the live registries exactly like the
-- builtin's.

local ext = {
  id = "user.pane-keys",
  name = "pane keys",
  version = "0.1.0",
  description = "pane commands and leader keys from Lua",
}

function ext.register(ekko)
  ekko.register_command({
    name = "split",
    args_hint = "right|down",
    description = "split the focused pane",
    handler = function(args)
      if args == "right" then
        return "split_right"
      elseif args == "down" then
        return "split_down"
      else
        return { { set_status_note = { text = "usage: split right|down", kind = "error" } } }
      end
    end,
  })

  ekko.register_command({
    name = "pane-focus",
    args_hint = "left|right|up|down",
    description = "focus the neighboring pane in a direction",
    handler = function(args)
      return { focus_direction = args }
    end,
  })

  ekko.register_command({
    name = "pane-close",
    description = "close the focused pane",
    handler = function()
      return "close_focused_pane"
    end,
  })

  local leader = {
    { chord = "|", description = "split right", action = "split_right" },
    { chord = "-", description = "split down", action = "split_down" },
    { chord = "h", description = "focus left", action = { focus_direction = "left" } },
    { chord = "j", description = "focus down", action = { focus_direction = "down" } },
    { chord = "k", description = "focus up", action = { focus_direction = "up" } },
    { chord = "l", description = "focus right", action = { focus_direction = "right" } },
    { chord = "x", description = "close pane", action = "close_focused_pane" },
  }
  for _, entry in ipairs(leader) do
    ekko.register_keybinding({
      mode = "leader",
      chord = entry.chord,
      description = entry.description,
      handler = function()
        return { "exit_mode", entry.action }
      end,
    })
  end
end

return ext
