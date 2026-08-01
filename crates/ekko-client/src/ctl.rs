//! Out-of-band session control: the Unix-scripting verbs (`ekko send`,
//! `ekko dump`). Each verb is one short-lived socket connection speaking
//! `ekko-proto` directly — no attach, no TTY, no extension runtime — so
//! scripts, cron jobs, and other programs can drive sessions with plain
//! stdin/stdout semantics.

use std::time::Duration;

use anyhow::{Context, Result};
use ekko_proto::{ClientToServer, ServerToClient, socket_path};
use std::os::unix::net::UnixStream;

/// How long a control verb waits for the daemon's reply before concluding
/// it is wedged (same rationale as `kill`'s confirmation budget).
const CTL_REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// Errors a control verb can report. `NoSuchSession` maps to a dedicated
/// process exit code (`EXIT_NOT_FOUND`) so scripts can distinguish "the
/// session isn't there" from "the daemon is broken".
#[derive(Debug)]
pub enum CtlError {
    /// No live socket (or live daemon) answers for the session name.
    NoSuchSession(String),
    /// The daemon is there but failed or refused the request.
    Other(anyhow::Error),
}

impl std::fmt::Display for CtlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchSession(name) => write!(f, "no such session: '{name}'"),
            Self::Other(error) => std::fmt::Display::fmt(error, f),
        }
    }
}

impl std::error::Error for CtlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoSuchSession(_) => None,
            Self::Other(error) => error.source(),
        }
    }
}

impl From<anyhow::Error> for CtlError {
    fn from(error: anyhow::Error) -> Self {
        Self::Other(error)
    }
}

/// Connect to the named session's socket or report `NoSuchSession`.
fn connect(name: &str) -> Result<UnixStream, CtlError> {
    let path = socket_path(name);
    if !path.exists() {
        return Err(CtlError::NoSuchSession(name.to_string()));
    }
    ekko_proto::ipc_connect(&path)
        .with_context(|| format!("connecting to session '{name}'"))
        .map_err(CtlError::Other)
}

/// `ekko send <name> <bytes>`: inject raw bytes into the session's primary
/// pane. Fire-and-forget: the daemon applies the write silently.
pub fn send(name: &str, bytes: &[u8]) -> Result<(), CtlError> {
    let stream = connect(name)?;
    let mut send_half = stream;
    ekko_proto::write_msg(
        &mut send_half,
        &ClientToServer::Inject {
            bytes: bytes.to_vec(),
        },
    )
    .context("sending inject request")
    .map_err(CtlError::Other)
}

/// `ekko dump <name>`: fetch the primary pane's scrollback + live screen as
/// plain text. The text goes to the caller (main prints it to stdout), so
/// the verb composes: `ekko dump work | grep error`.
pub fn dump(name: &str) -> Result<String, CtlError> {
    let stream = connect(name)?;
    let recv = stream
        .try_clone()
        .context("clone control socket")
        .map_err(CtlError::Other)?;
    let mut send_half = stream;
    recv.set_read_timeout(Some(CTL_REPLY_TIMEOUT))
        .context("arming dump reply timeout")
        .map_err(CtlError::Other)?;
    ekko_proto::write_msg(&mut send_half, &ClientToServer::DumpSession)
        .context("sending dump request")
        .map_err(CtlError::Other)?;
    let mut recv = recv;
    loop {
        match ekko_proto::read_msg::<_, ServerToClient>(&mut recv) {
            Ok(Some(ServerToClient::ScrollbackDump { text, .. })) => return Ok(text),
            // Not our reply (e.g. a stray Notice); keep waiting until the
            // timeout fires.
            Ok(Some(_)) => continue,
            Ok(None) => {
                return Err(CtlError::Other(anyhow::anyhow!(
                    "daemon closed the connection before answering the dump"
                )));
            }
            Err(e) => {
                return Err(CtlError::Other(anyhow::anyhow!(
                    "waiting for the dump reply failed: {e}"
                )));
            }
        }
    }
}

/// Whether an error from a control verb means "no such session" (exit code
/// mapping in main).
pub fn is_no_such_session(err: &CtlError) -> bool {
    matches!(err, CtlError::NoSuchSession(_))
}
