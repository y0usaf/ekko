use ekko_ext::RuntimeBuilder;
use ekko_ext::wasm::WasmExtension;
fn main() {
    let wasm = std::fs::read("/home/y0usaf/dev/ekko/guests/which-key/target/wasm32-unknown-unknown/release/which_key_wasm.wasm").unwrap();
    let ext = WasmExtension::from_bytes(&wasm, "which-key").unwrap();
    let rt = RuntimeBuilder::new()
        .register_extension(ext)
        .build()
        .unwrap();
    for n in ["leader", "pane"] {
        println!("mode({n}) registered: {}", rt.mode(n).is_some());
    }
    println!(
        "ctrl+b matchN={} ctrl+p matchN={}",
        rt.match_keybinding(&[0x02], None).is_some(),
        rt.match_keybinding(&[0x10], None).is_some()
    );
}
