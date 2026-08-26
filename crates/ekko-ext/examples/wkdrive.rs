use ekko_ext::wasm::WasmExtension;
use ekko_ext::{ClientSnapshot, RuntimeBuilder, ThemePalette};
fn snap(mode: &str) -> ClientSnapshot {
    ClientSnapshot {
        session_name: "s1".into(),
        mode: mode.into(),
        cols: 120,
        rows: 30,
        grid_cols: 118,
        grid_rows: 28,
        scrollback: 0,
        panes: vec![],
        focused_pane: None,
        projects: ekko_ext::fallback_group(vec![]),
        status_note: None,
        keybindings: vec![],
        now_ms: 0,
        hidden_surfaces: vec![],
        theme: ThemePalette::fallback(),
    }
}
fn main() {
    let wasm = std::fs::read("/home/y0usaf/.config/ekko/extensions/which-key.wasm").unwrap();
    let ext = WasmExtension::from_bytes(&wasm, "which-key").unwrap();
    let rt = RuntimeBuilder::new()
        .register_extension(ext)
        .build()
        .unwrap();
    for (label, bytes, mode) in [
        ("ctrl+b", vec![0x02u8], "normal"),
        ("ctrl+p", vec![0x10u8], "normal"),
        ("n leader", vec![b'n'], "leader"),
        ("j leader", vec![b'j'], "leader"),
        ("n pane", vec![b'n'], "pane"),
        ("j pane", vec![b'j'], "pane"),
        ("x leader", vec![b'x'], "leader"),
        ("x pane", vec![b'x'], "pane"),
    ] {
        let m_none = rt.match_keybinding(&bytes, None).is_some();
        let m_mode = rt.match_keybinding(&bytes, Some(mode)).is_some();
        let spec = if mode == "normal" {
            rt.match_keybinding(&bytes, None)
        } else {
            rt.match_keybinding(&bytes, Some(mode))
        };
        match spec.map(|s| s.handler.clone()) {
            Some(h) => println!(
                "{label} [{mode}]: matchN={m_none} matchM={m_mode} -> actions={:?}",
                h(&snap(mode))
            ),
            None => println!("{label} [{mode}]: matchN={m_none} matchM={m_mode} -> NO BINDING"),
        }
    }
}
