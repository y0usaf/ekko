use anyhow::Result;
use clap::{Parser, Subcommand};

const DEFAULT_SESSION: &str = "main";

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
    /// List live and resurrectable sessions.
    Ls,
    /// Kill a session.
    Kill { name: String },
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
        Some(Command::Ls) => {
            for summary in ekko_server::list_sessions()? {
                let state = match (summary.alive, summary.attached) {
                    (true, true) => "attached",
                    (true, false) => "detached",
                    (false, _) => "resurrectable",
                };
                println!("{}\t{}\t{}", summary.name, state, summary.cwd.display());
            }
            Ok(())
        }
        Some(Command::Kill { name }) => ekko_client::kill_session(&name),
    }
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
