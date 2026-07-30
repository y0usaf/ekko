use anyhow::Result;
use clap::{Parser, Subcommand};

const DEFAULT_SESSION: &str = "main";

/// Process exit code when the named session doesn't exist (distinct from
/// general errors so scripts can branch on it: `ekko send x hi || [ $? -eq 3 ]`).
pub const EXIT_NOT_FOUND: u8 = 3;

#[derive(Parser)]
#[command(name = "ekko", version, about = "ekko — a terminal multiplexer")]
struct Cli {
    /// Run the session daemon for a session (internal; clients spawn this).
    #[arg(long = "server", hide = true, value_name = "SESSION")]
    server: Option<String>,

    /// Stay in the foreground when running as a server (internal/debug).
    #[arg(long, hide = true)]
    foreground: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Attach to a session, creating it if missing.
    Attach {
        name: Option<String>,
        /// Kick any other attached clients, becoming the sole viewer.
        #[arg(long)]
        force: bool,
    },
    /// Create and attach a new session.
    New { name: Option<String> },
    /// Ask one attached ekko client to request focus/attention from its host
    /// terminal (e.g. BEL → XDG activation urgency in foot).
    Activate,
    /// List live and resurrectable sessions (tab-separated by default).
    Ls {
        /// Emit one JSON object per line: {"name", "state", "cwd"}.
        /// Stable for scripts; the tab format is for humans.
        #[arg(long)]
        json: bool,
    },
    /// Kill a session (exits 3 when the session does not exist).
    Kill {
        name: String,
        /// Escalate to an out-of-band SIGKILL of the daemon (recorded in its
        /// manifest) when it doesn't confirm a polite kill. Use when a
        /// session is wedged.
        #[arg(long)]
        force: bool,
    },
    /// Inject raw bytes into a session's primary pane, then exit — the
    /// scripting verb: `ekko send work "make test\n"`.
    Send {
        name: String,
        /// The bytes to inject. `\n`, `\r`, `\t`, `\0`, `\\`, and `\xNN`
        /// escapes are decoded; everything else is sent verbatim.
        bytes: String,
    },
    /// Print a session's scrollback + live screen as plain text to stdout:
    /// `ekko dump work | grep error`.
    Dump { name: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

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
