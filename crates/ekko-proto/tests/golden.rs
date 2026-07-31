use ekko_proto::*;
use std::path::PathBuf;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn client_values() -> Vec<(&'static str, ClientToServer)> {
    vec![
        (
            "Attach",
            ClientToServer::Attach {
                wire_version: 11,
                cols: 80,
                rows: 24,
                cwd: PathBuf::from("/tmp"),
                shell: Some(PathBuf::from("/bin/sh")),
                force: false,
                terminal_colors: None,
            },
        ),
        ("Detach", ClientToServer::Detach),
        ("Resize", ClientToServer::Resize { cols: 80, rows: 24 }),
        ("Key", ClientToServer::Key(vec![0x1b, b'[', b'A'])),
        ("Paste", ClientToServer::Paste(b"hi".to_vec())),
        (
            "SplitPane::Right",
            ClientToServer::SplitPane {
                direction: SplitDirection::Right,
            },
        ),
        (
            "FocusDirection::Left",
            ClientToServer::FocusDirection {
                direction: Direction::Left,
            },
        ),
        ("FocusPane", ClientToServer::FocusPane { pane: 7 }),
        ("CloseFocusedPane", ClientToServer::CloseFocusedPane),
        ("Scroll", ClientToServer::Scroll { delta: -3 }),
        ("ScrollReset", ClientToServer::ScrollReset),
        (
            "SearchScrollback",
            ClientToServer::SearchScrollback {
                query: "needle".into(),
            },
        ),
        ("DumpScrollback", ClientToServer::DumpScrollback),
        ("KillCurrentSession", ClientToServer::KillCurrentSession),
        ("KillSession", ClientToServer::KillSession("main".into())),
        ("Ping", ClientToServer::Ping),
        ("Activate", ClientToServer::Activate),
        (
            "Inject",
            ClientToServer::Inject {
                bytes: b"x".to_vec(),
            },
        ),
        ("DumpSession", ClientToServer::DumpSession),
    ]
}
fn server_values() -> Vec<(&'static str, ServerToClient)> {
    vec![
        (
            "Attached",
            ServerToClient::Attached {
                session_name: "main".into(),
                wire_version: 11,
            },
        ),
        (
            "AttachRejected::WrongWireVersion",
            ServerToClient::AttachRejected(AttachRejectReason::WrongWireVersion),
        ),
        (
            "Workspace",
            ServerToClient::Workspace(WorkspaceUpdate {
                epoch: 1,
                panes: vec![],
                focused: 0,
                grids: vec![],
                border_style: PaneBorderStyle::None,
            }),
        ),
        ("Bell", ServerToClient::Bell),
        ("Exit::Normal", ServerToClient::Exit(ExitReason::Normal)),
        ("Pong", ServerToClient::Pong),
        (
            "Notice",
            ServerToClient::Notice(ServerNotice {
                source: "test".into(),
                level: NoticeLevel::Info,
                message: "hello".into(),
            }),
        ),
        ("Title", ServerToClient::Title("title".into())),
        (
            "ClipboardCopy",
            ServerToClient::ClipboardCopy(b"abc".to_vec()),
        ),
        ("Activate", ServerToClient::Activate),
        (
            "ActivateResult",
            ServerToClient::ActivateResult { delivered: true },
        ),
        (
            "SearchResults",
            ServerToClient::SearchResults {
                pane: 2,
                query: "q".into(),
                matches: vec![SearchMatch {
                    row: 3,
                    col: 4,
                    len: 1,
                }],
            },
        ),
        (
            "ScrollbackDump",
            ServerToClient::ScrollbackDump {
                pane: 2,
                text: "text".into(),
            },
        ),
    ]
}

#[test]
fn golden_wire_bytes() {
    // Expected payload bytes (excluding frame length prefix). Update these only
    // alongside a WIRE_VERSION bump when the encoding legitimately changes.
    let expected: &[(&str, &str)] = &[
        (
            "ClientToServer::Attach",
            "000000000b0000005000180004000000000000002f746d700107000000000000002f62696e2f73680000",
        ),
        ("ClientToServer::Detach", "01000000"),
        ("ClientToServer::Resize", "0200000050001800"),
        ("ClientToServer::Key", "0300000003000000000000001b5b41"),
        ("ClientToServer::Paste", "0400000002000000000000006869"),
        ("ClientToServer::SplitPane::Right", "0500000000000000"),
        ("ClientToServer::FocusDirection::Left", "0600000000000000"),
        ("ClientToServer::FocusPane", "070000000700000000000000"),
        ("ClientToServer::CloseFocusedPane", "08000000"),
        ("ClientToServer::Scroll", "09000000fdffffff"),
        ("ClientToServer::ScrollReset", "0a000000"),
        (
            "ClientToServer::SearchScrollback",
            "0b00000006000000000000006e6565646c65",
        ),
        ("ClientToServer::DumpScrollback", "0c000000"),
        ("ClientToServer::KillCurrentSession", "0d000000"),
        (
            "ClientToServer::KillSession",
            "0e00000004000000000000006d61696e",
        ),
        ("ClientToServer::Ping", "0f000000"),
        ("ClientToServer::Activate", "10000000"),
        ("ClientToServer::Inject", "11000000010000000000000078"),
        ("ClientToServer::DumpSession", "12000000"),
        (
            "ServerToClient::Attached",
            "0000000004000000000000006d61696e0b000000",
        ),
        (
            "ServerToClient::AttachRejected::WrongWireVersion",
            "0100000000000000",
        ),
        (
            "ServerToClient::Workspace",
            "02000000010000000000000000000000000000000000000000000000000000000000000000000000",
        ),
        ("ServerToClient::Bell", "03000000"),
        ("ServerToClient::Exit::Normal", "0400000000000000"),
        ("ServerToClient::Pong", "05000000"),
        (
            "ServerToClient::Notice",
            "0600000004000000000000007465737400000000050000000000000068656c6c6f",
        ),
        (
            "ServerToClient::Title",
            "0700000005000000000000007469746c65",
        ),
        (
            "ServerToClient::ClipboardCopy",
            "080000000300000000000000616263",
        ),
        ("ServerToClient::Activate", "09000000"),
        ("ServerToClient::ActivateResult", "0a00000001"),
        (
            "ServerToClient::SearchResults",
            "0b000000020000000000000001000000000000007101000000000000000300000004000100",
        ),
        (
            "ServerToClient::ScrollbackDump",
            "0c0000000200000000000000040000000000000074657874",
        ),
    ];
    let mut actual = Vec::new();
    for (n, value) in client_values() {
        let name = format!("ClientToServer::{n}");
        actual.push((name, bincode::serialize(&value).unwrap()));
    }
    for (n, value) in server_values() {
        let name = format!("ServerToClient::{n}");
        actual.push((name, bincode::serialize(&value).unwrap()));
    }
    if expected.is_empty() {
        for (name, bytes) in &actual {
            eprintln!("({name:?}, {:?}),", hex(bytes));
        }
        panic!("golden vectors need filling");
    }
    assert_eq!(
        expected.len(),
        actual.len(),
        "golden table must cover every variant"
    );
    for ((name, want), (actual_name, got)) in expected.iter().zip(actual.iter()) {
        assert_eq!(name, actual_name, "golden message ordering changed");
        assert_eq!(
            *want,
            hex(got),
            "wire encoding changed for {name}; bump WIRE_VERSION and update this golden vector"
        );
    }
}
