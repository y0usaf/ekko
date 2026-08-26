fn challenge(_desc: &str) {}
fn main() {
    let wasm = std::fs::read("/home/y0usaf/.config/ekko/extensions/which-key.wasm").unwrap();
    let mut ctx = cordis::Context::new();
    let id = ctx.mount(&wasm).unwrap();
    for mode in ["normal", "leader", "pane"] {
        // keybindings listing reflecting the mode scope, as ClientSnapshot would
        let kbs = match mode {
            "normal" => {
                r#"[{"chord_text":"ctrl+b","mode":null,"description":"leader"},{"chord_text":"ctrl+p","mode":null,"description":"pane"}]"#
            }
            "leader" => {
                r#"[{"chord_text":"ctrl+b","mode":"leader","description":"close panel"},{"chord_text":"n","mode":"leader","description":"new session"},{"chord_text":"j","mode":"leader","description":"next session"},{"chord_text":"x","mode":"leader","description":"kill session"},{"chord_text":"d","mode":"leader","description":"detach"},{"chord_text":"s","mode":"leader","description":"scroll"}]"#
            }
            "pane" => {
                r#"[{"chord_text":"n","mode":"pane","description":"new pane"},{"chord_text":"j","mode":"pane","description":"focus down"},{"chord_text":"x","mode":"pane","description":"close pane"},{"chord_text":"q","mode":"pane","description":"exit pane"}]"#
            }
            _ => "[]",
        };
        let payload = format!(
            r#"{{"name":"user.which-key:bottom","snapshot":{{"session_name":"s1","mode":"{mode}","cols":120,"rows":30,"keybindings":{kbs}}}}}"#
        );
        ctx.call(id, "on_surface_draw", &payload).unwrap();
        let ops = ctx.take_ops(id).unwrap();
        println!("BOTTOM mode={mode}: {} draw ops", ops.len());
        for (kind, args) in &ops {
            if kind == "put_text" {
                println!("   text(draw) -> {:?}", args.get(2));
            }
        }
    }
}
