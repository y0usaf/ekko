;; config.wasm for ekko — the WASM settings module (cordis set 1).
;;
;; Replaces the former init.lua. On mount it writes the `config` key (the
;; whole settings JSON) via the cordis `ctx_set` host function; the ekko
;; config bridge parses that JSON into the Config schema. This reproduces the
;; settings the old init.lua carried:
;;   - extensions.disabled: the builtin leader/statusbar/sidebar/panes/
;;     keybindings are off (which-key owns those surfaces)
;;   - ui.pane_borders = "frame", border_glyphs = ASCII, pane_layout = "equal",
;;     animation_interval_ms = 33
(module
  (import "host" "ctx_set" (func $ctx_set (param i32 i32 i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "config{\"extensions\":{\"disabled\":[\"ekko-builtins.leader\",\"ekko-builtins.statusbar\",\"ekko-builtins.sidebar\",\"ekko-builtins.panes\",\"ekko-builtins.keybindings\"]},\"ui\":{\"pane_borders\":\"frame\",\"border_glyphs\":{\"horizontal\":\"-\",\"vertical\":\"|\",\"junction\":\"+\"},\"pane_layout\":\"equal\",\"animation_interval_ms\":33}}")

  (func (export "scratch") (result i32 i32)
    i32.const 256 i32.const 256)

  (func (export "mount")
    ;; ctx_set(key="config"@0 len 6, value=JSON@6 len 296)
    i32.const 0  i32.const 6  i32.const 6  i32.const 296  call $ctx_set)

  (func (export "on_change") (param i32 i32))
)
