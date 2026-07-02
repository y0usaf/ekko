-- Example user leader map. Drop this file in ~/.config/ekko/extensions/ and
-- every entry appears in the which-key panel automatically (press the
-- leader chord, ctrl+space by default, to see it).
--
-- A leader entry is just a keybinding registered with `mode = "leader"`.
-- The host keeps the leader mode active after an entry fires, so an entry
-- that runs a plain action returns "exit_mode" ahead of it; leave it out on
-- purpose to make an entry repeatable ("sticky").

local ext = {
  id = "user.leader-map",
  name = "leader map",
  version = "0.1.0",
  description = "example which-key leader entries",
}

function ext.register(ekko)
  -- Leaf entry: leader, then w -> jump to a named session.
  ekko.register_keybinding({
    mode = "leader",
    chord = "w",
    description = "work session",
    handler = function()
      return { "exit_mode", { switch_session = "work" } }
    end,
  })

  -- Leaf entry dispatching through the command registry, so the leader map
  -- stays data: key -> description -> command line.
  ekko.register_keybinding({
    mode = "leader",
    chord = "N",
    description = "named session",
    handler = function()
      return { "exit_mode", { invoke_command = "new scratch" } }
    end,
  })

  -- Sticky entry: no exit_mode, so the leader stays active and the key can
  -- be pressed repeatedly (here: page through sessions from the snapshot).
  ekko.register_keybinding({
    mode = "leader",
    chord = "j",
    description = "next session (sticky)",
    handler = function(snapshot)
      local sessions = {}
      for _, project in ipairs(snapshot.projects) do
        for _, session in ipairs(project.sessions) do
          sessions[#sessions + 1] = session.name
        end
      end
      for i, name in ipairs(sessions) do
        if name == snapshot.session_name then
          local target = sessions[(i % #sessions) + 1]
          if target ~= name then
            return { { switch_session = target } }
          end
        end
      end
      return { { set_status_note = { text = "no other session", kind = "info" } } }
    end,
  })
end

return ext
