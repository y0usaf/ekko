use ekko_err::Result;

const DEFAULT_SESSION: &str = "main";

/// Process exit code when the named session doesn't exist (distinct from
/// general errors so scripts can branch on it: `ekko send x hi || [ $? -eq 3 ]`).
pub const EXIT_NOT_FOUND: u8 = 3;

struct CommandSpec {
    name: &'static str,
    about: &'static str,
    usage: &'static str,
    arguments: &'static str,
    options: &'static str,
}
const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "attach",
        about: "Attach to a session, creating it if missing",
        usage: "ekko attach [OPTIONS] [NAME]",
        arguments: "  [NAME]",
        options: "      --force  Kick any other attached clients, becoming the sole viewer",
    },
    CommandSpec {
        name: "new",
        about: "Create and attach a new session.",
        usage: "ekko new [NAME]",
        arguments: "  [NAME]",
        options: "",
    },
    CommandSpec {
        name: "activate",
        about: "Ask one attached ekko client to request focus/attention from its host terminal (e.g. BEL → XDG activation urgency in foot).",
        usage: "ekko activate",
        arguments: "",
        options: "",
    },
    CommandSpec {
        name: "ls",
        about: "List live and resurrectable sessions (tab-separated by default).",
        usage: "ekko ls [OPTIONS]",
        arguments: "",
        options: r#"      --json  Emit one JSON object per line: {"name", "state", "cwd"}. Stable for scripts; the tab format is for humans"#,
    },
    CommandSpec {
        name: "kill",
        about: "Kill a session (exits 3 when the session does not exist).",
        usage: "ekko kill [OPTIONS] <NAME>",
        arguments: "  <NAME>",
        options: "      --force  Escalate to an out-of-band SIGKILL of the daemon (recorded in its manifest) when it doesn't confirm a polite kill. Use when a session is wedged",
    },
    CommandSpec {
        name: "send",
        about: "Inject raw bytes into a session's primary pane, then exit — the scripting verb: `ekko send work \"make test\\n\"`.",
        usage: "ekko send <NAME> <BYTES>",
        arguments: "  <NAME>\n  <BYTES>  The bytes to inject. `\\n`, `\\r`, `\\t`, `\\0`, `\\\\`, and `\\xNN` escapes are decoded; everything else is sent verbatim",
        options: "",
    },
    CommandSpec {
        name: "dump",
        about: "Print a session's scrollback + live screen as plain text to stdout: `ekko dump work | grep error`.",
        usage: "ekko dump <NAME>",
        arguments: "  <NAME>",
        options: "",
    },
];

enum Command {
    Attach { name: Option<String>, force: bool },
    New { name: Option<String> },
    Activate,
    Ls { json: bool },
    Kill { name: String, force: bool },
    Send { name: String, bytes: String },
    Dump { name: String },
}
struct Cli {
    server: Option<String>,
    foreground: bool,
    command: Option<Command>,
}

fn help(spec: Option<&CommandSpec>) -> String {
    if let Some(s) = spec {
        let arguments = if s.arguments.is_empty() {
            String::new()
        } else {
            format!("Arguments:\n{}\n\n", s.arguments)
        };
        let options = if s.options.is_empty() {
            String::new()
        } else {
            format!("{}\n", s.options)
        };
        return format!(
            "{}\n\nUsage: {}\n\n{}Options:\n{}  -h, --help   Print help\n",
            s.about, s.usage, arguments, options
        );
    }
    let mut out =
        String::from("ekko — a terminal multiplexer\n\nUsage: ekko [COMMAND]\n\nCommands:\n");
    for s in COMMANDS {
        out.push_str(&format!("  {:<10}{}\n", s.name, s.about));
    }
    out.push_str("  help      Print this message or the help of the given subcommand(s)\n\nOptions:\n  -h, --help     Print help\n  -V, --version  Print version\n");
    out
}
fn parse(args: &[String]) -> Result<Cli, String> {
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("ekko {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }
    if args.is_empty() {
        return Ok(Cli {
            server: None,
            foreground: false,
            command: None,
        });
    }
    if args[0] == "--help" || args[0] == "-h" {
        print!("{}", help(None));
        std::process::exit(0);
    }
    let mut i = 0;
    let mut server = None;
    let mut foreground = false;
    while i < args.len() && args[i].starts_with('-') {
        match args[i].as_str() {
            "--server" => {
                i += 1;
                server = Some(
                    args.get(i)
                        .ok_or("error: --server requires a value")?
                        .clone(),
                );
            }
            "--foreground" => foreground = true,
            "--help" | "-h" => {
                print!("{}", help(None));
                std::process::exit(0);
            }
            x => return Err(format!("error: unknown option `{x}`")),
        }
        i += 1;
    }
    if i == args.len() {
        return Ok(Cli {
            server,
            foreground,
            command: None,
        });
    }
    let name = &args[i];
    let spec = COMMANDS
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("error: unknown subcommand `{name}`"))?;
    i += 1;
    if args.get(i).is_some_and(|x| x == "--help" || x == "-h") {
        print!("{}", help(Some(spec)));
        std::process::exit(0);
    }
    let rest = &args[i..];
    let flag = |f: &str| rest.iter().position(|x| x == f);
    let positional: Vec<&String> = rest.iter().filter(|x| !x.starts_with('-')).collect();
    let bad = |m: &str| Err(format!("error: {m}"));
    let command = match name.as_str() {
        "attach" => {
            if rest.iter().any(|x| x != "--force" && x.starts_with('-')) {
                return bad("unknown option");
            }
            if positional.len() > 1 {
                return bad(&format!("unexpected argument '{}'", positional[1]));
            }
            Command::Attach {
                name: positional.first().map(|x| (*x).clone()),
                force: flag("--force").is_some(),
            }
        }
        "new" => {
            if rest.iter().any(|x| x.starts_with('-')) {
                return bad("unknown option");
            }
            if positional.len() > 1 {
                return bad(&format!("unexpected argument '{}'", positional[1]));
            }
            Command::New {
                name: positional.first().map(|x| (*x).clone()),
            }
        }
        "activate" => {
            if !rest.is_empty() {
                return bad("unexpected argument");
            }
            Command::Activate
        }
        "ls" => {
            if let Some(extra) = rest.iter().find(|x| *x != "--json") {
                return if extra.starts_with('-') {
                    bad("unknown option")
                } else {
                    bad(&format!("unexpected argument '{}'", extra))
                };
            }
            Command::Ls {
                json: flag("--json").is_some(),
            }
        }
        "kill" => {
            if rest.iter().any(|x| x.starts_with('-') && *x != "--force") {
                return bad("unknown option");
            }
            if positional.len() > 1 {
                return bad(&format!("unexpected argument '{}'", positional[1]));
            }
            if positional.is_empty() {
                return bad("the following required argument was not provided: NAME");
            }
            Command::Kill {
                name: positional[0].clone(),
                force: flag("--force").is_some(),
            }
        }
        "send" => {
            if rest.iter().any(|x| x.starts_with('-')) {
                return bad("unknown option");
            }
            if positional.len() > 2 {
                return bad(&format!("unexpected argument '{}'", positional[2]));
            }
            if positional.len() != 2 {
                return bad("the following required arguments were not provided: NAME BYTES");
            }
            Command::Send {
                name: positional[0].clone(),
                bytes: positional[1].clone(),
            }
        }
        "dump" => {
            if rest.iter().any(|x| x.starts_with('-')) {
                return bad("unknown option");
            }
            if positional.len() > 1 {
                return bad(&format!("unexpected argument '{}'", positional[1]));
            }
            if positional.is_empty() {
                return bad("the following required argument was not provided: NAME");
            }
            Command::Dump {
                name: positional[0].clone(),
            }
        }
        _ => unreachable!(),
    };
    Ok(Cli {
        server,
        foreground,
        command: Some(command),
    })
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match parse(&args) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };

    if let Some(session_name) = cli.server {
        return ekko_server::run(&session_name, !cli.foreground);
    }

    match cli.command {
        // Bare `ekko` starts a fresh session per terminal (like zellij);
        // joining an existing one is an explicit `ekko attach`. `None`
        // defers naming to the registered session namer inside the client.
        None => attach(None, true, false),
        Some(Command::Attach { name, force }) => attach(
            Some(name.unwrap_or_else(|| DEFAULT_SESSION.to_string())),
            true,
            force,
        ),
        Some(Command::New { name }) => attach(name, true, false),
        Some(Command::Activate) => ekko_client::activate(),
        Some(Command::Ls { json }) => {
            for summary in ekko_server::list_sessions()? {
                let state = match (summary.alive, summary.attached) {
                    (true, true) => "attached",
                    (true, false) => "detached",
                    (false, _) => "resurrectable",
                };
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "name": summary.name,
                            "state": state,
                            "cwd": summary.cwd.display().to_string(),
                        })
                    );
                } else {
                    println!("{}\t{}\t{}", summary.name, state, summary.cwd.display());
                }
            }
            Ok(())
        }
        Some(Command::Kill { name, force }) => ctl_exit(ekko_client::kill_session(&name, force)),
        Some(Command::Send { name, bytes }) => {
            let decoded = decode_escapes(&bytes);
            ctl_exit(ekko_client::send(&name, &decoded))
        }
        Some(Command::Dump { name }) => match ekko_client::dump(&name) {
            Ok(text) => {
                print!("{text}");
                Ok(())
            }
            Err(e) => ctl_exit(Err(e)),
        },
    }
}

/// Map a control-verb result onto the process contract: success stays
/// silent on stdout, `NoSuchSession` exits `EXIT_NOT_FOUND`.
fn ctl_exit(result: Result<(), ekko_client::CtlError>) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(e) if ekko_client::is_no_such_session(&e) => {
            eprintln!("{e}");
            std::process::exit(EXIT_NOT_FOUND.into());
        }
        Err(e) => Err(e.into()),
    }
}

/// Decode the `\`-escapes accepted by `ekko send` (`\n`, `\r`, `\t`, `\0`,
/// `\\`, `\xNN`). Unknown escapes pass through untouched, so sending a
/// literal backslash needs `\\` only when followed by one of these letters.
fn decode_escapes(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        match chars.next() {
            Some('n') => out.push(b'\n'),
            Some('r') => out.push(b'\r'),
            Some('t') => out.push(b'\t'),
            Some('0') => out.push(0),
            Some('\\') => out.push(b'\\'),
            Some('x') => {
                let hi = chars.next();
                let lo = chars.next();
                match (
                    hi.and_then(|h| h.to_digit(16)),
                    lo.and_then(|l| l.to_digit(16)),
                ) {
                    (Some(hi), Some(lo)) => out.push((hi * 16 + lo) as u8),
                    _ => {
                        out.push(b'\\');
                        out.push(b'x');
                        out.extend(hi.map(|c| c as u8));
                        out.extend(lo.map(|c| c as u8));
                    }
                }
            }
            Some(other) => {
                out.push(b'\\');
                let mut buf = [0u8; 4];
                out.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
            }
            None => out.push(b'\\'),
        }
    }
    out
}

/// Attach and run until exit; session switches reconnect inside
/// `ekko_client::run` so client-side state (active mode, extension runtime,
/// input threads) survives them.
fn attach(name: Option<String>, create_if_missing: bool, force: bool) -> Result<()> {
    ekko_client::run(ekko_client::ClientOptions {
        session_name: name,
        create_if_missing,
        force,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_documented_escapes() {
        assert_eq!(decode_escapes("ls\\n"), b"ls\n");
        assert_eq!(decode_escapes("a\\rb\\tc\\0d"), b"a\rb\tc\0d");
        assert_eq!(decode_escapes("\\\\"), b"\\");
        assert_eq!(decode_escapes("\\x1b[A"), b"\x1b[A");
        assert_eq!(decode_escapes("plain"), b"plain");
    }

    #[test]
    fn unknown_escapes_pass_through() {
        assert_eq!(decode_escapes("a\\qb"), b"a\\qb");
        assert_eq!(decode_escapes("trailing\\"), b"trailing\\");
        assert_eq!(decode_escapes("\\xzz"), b"\\xzz");
    }
}
