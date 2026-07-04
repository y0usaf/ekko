-- A server-side spawn hook (host = "server"): this script runs inside the
-- session daemon, not the client. It shows the two server-side hook shapes:
--
--  * `before_pty_spawn` is a gate — returning { spawn_override = ... }
--    rewrites the spawn before it happens. `shell` and `cwd` replace the
--    host-resolved values; `env` entries are appended to the child
--    environment. Here every shell gets EKKO_SPAWN_HOOK stamped with the
--    session name.
--  * `session_created` / `client_attached` are notifications. A notice
--    returned from `session_created` has nowhere to go — it fires before
--    any client is attached — so the idiom is to stash the payload and
--    surface it on `client_attached`.
--
-- Drop in ~/.config/ekko/extensions/. The daemon evaluates scripts once at
-- session start: edits take effect on the next session (`ekko kill` +
-- restart is the reload path).

local ext = {
  id = "user.spawn-hook",
  name = "spawn hook",
  version = "0.1.0",
  description = "stamp spawned shells and report the spawn on attach",
  host = "server",
}

local created -- session_created payload, held for the first attach notice

function ext.register(ekko)
  ekko.subscribe("before_pty_spawn", function(payload)
    return {
      spawn_override = {
        env = { EKKO_SPAWN_HOOK = "lua:" .. payload.session_name },
      },
    }
  end)

  ekko.subscribe("session_created", function(payload)
    created = payload
  end)

  ekko.subscribe("client_attached", function(payload)
    if not created then
      return
    end
    local msg = string.format(
      "session '%s' spawned %s in %s",
      created.session_name, created.shell, created.cwd)
    created = nil
    return { notice = { level = "info", message = msg } }
  end)
end

return ext
