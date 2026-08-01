use std::process::Command;

fn reject(args: &[&str], message: &str) {
    let output = Command::new(env!("CARGO_BIN_EXE_ekko"))
        .args(args)
        .output()
        .unwrap();
    assert!(!output.status.success(), "unexpected success for {args:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(message),
        "stderr: {:?}",
        output.stderr
    );
}

#[test]
fn unknown_subcommand() {
    reject(&["frobnicate"], "unknown subcommand");
}
#[test]
fn unknown_subcommand_flag() {
    reject(&["ls", "--jason"], "unknown option");
}
#[test]
fn unknown_global_flag() {
    reject(&["--nope"], "unknown option");
}
#[test]
fn wrong_subcommand_flag() {
    reject(&["dump", "--force"], "unknown option");
}
#[test]
fn missing_kill_name() {
    reject(&["kill"], "required argument");
}
#[test]
fn missing_send_bytes() {
    reject(&["send", "work"], "required arguments");
}
#[test]
fn missing_server_value() {
    reject(&["--server"], "--server requires a value");
}
#[test]
fn extra_dump_name() {
    reject(&["dump", "a", "b"], "unexpected argument 'b'");
}
#[test]
fn extra_attach_name() {
    reject(&["attach", "a", "b"], "unexpected argument 'b'");
}
#[test]
fn extra_new_name() {
    reject(&["new", "a", "b"], "unexpected argument 'b'");
}
#[test]
fn extra_activate_argument() {
    reject(&["activate", "a"], "unexpected argument");
}
#[test]
fn extra_ls_argument() {
    reject(&["ls", "a"], "unexpected argument 'a'");
}
#[test]
fn extra_kill_name() {
    reject(&["kill", "a", "b"], "unexpected argument 'b'");
}
#[test]
fn extra_send_bytes() {
    reject(&["send", "a", "b", "c"], "unexpected argument 'c'");
}
