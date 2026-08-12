//! Zero-dependency error type and helpers, used in place of `anyhow`.
//!
//! Reproduces just the `anyhow` API surface ekko actually exercises:
//! `Result`, `Error`, `.context`, `.with_context`, and the `err!`/`bail!`
//! macros. There is no downcasting or cause-chain walking anywhere in the
//! two bins, so a single message-formatting error is a faithful replacement.

use std::error::Error as StdError;
use std::fmt;

/// A dependency-free error carrying a formatted message.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Error {
    msg: String,
}

impl Error {
    /// Build an error from a formatted message.
    pub fn msg(msg: impl Into<String>) -> Self {
        Self { msg: msg.into() }
    }

    /// Return a copy with `context` prepended in front of the current
    /// message: `"context: inner"`.
    pub fn context(self, context: impl fmt::Display) -> Self {
        let msg = format!("{context}: {}", self.msg);
        Self { msg }
    }

    pub fn with_context<C, F>(self, f: F) -> Self
    where
        C: fmt::Display,
        F: FnOnce() -> C,
    {
        self.context(f())
    }

    pub fn msg_str(&self) -> &str {
        &self.msg
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Error").field(&self.msg).finish()
    }
}

// Convert any [`std::error::Error`] into [`Error`], for a bare `?` on a
// foreign operation (io, mlua, serde_json, ...). This blanket is what makes
// `?` on arbitrary std errors work without an explosive per-type enum.
//
// To keep the blanket coherent, [`Error`] deliberately does NOT implement
// `std::error::Error`: if it did, this `impl<E> From<E> for Error` would
// overlap `core`'s reflexive `impl<T> From<T> for T`. anyhow makes the same
// choice at the public `Error` level. Nothing in ekko needs `Error` itself to
// be a `std::error::Error`; the one place that looked like it (passing an
// error into `mlua::Error::external`) converts via the message string instead.
impl<E> From<E> for Error
where
    E: StdError + Send + Sync + 'static,
{
    fn from(e: E) -> Self {
        Self { msg: e.to_string() }
    }
}

/// The common result alias ekko's functions use.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Trait exposing `.context` / `.with_context` on `Result` and `Option`,
/// matching `anyhow::Context`.
pub trait Context<T> {
    fn context<C>(self, context: C) -> Result<T>
    where
        C: fmt::Display;
    fn with_context<C, F>(self, f: F) -> Result<T>
    where
        C: fmt::Display,
        F: FnOnce() -> C;
}

impl<T, E> Context<T> for std::result::Result<T, E>
where
    E: Into<Error>,
{
    fn context<C>(self, context: C) -> Result<T>
    where
        C: fmt::Display,
    {
        self.map_err(|e| Error::msg(format!("{context}: {}", e.into())))
    }
    fn with_context<C, F>(self, f: F) -> Result<T>
    where
        C: fmt::Display,
        F: FnOnce() -> C,
    {
        self.context(f())
    }
}

impl<T> Context<T> for Option<T> {
    fn context<C>(self, context: C) -> Result<T>
    where
        C: fmt::Display,
    {
        self.ok_or_else(|| Error::msg(context.to_string()))
    }
    fn with_context<C, F>(self, f: F) -> Result<T>
    where
        C: fmt::Display,
        F: FnOnce() -> C,
    {
        self.context(f())
    }
}

/// Construct an [`Error`] from a format string: `err!("bad {}", x)`.
#[macro_export]
macro_rules! err {
    ($($arg:tt)*) => {
        $crate::Error::msg(format!($($arg)*))
    };
}

/// Return early with an [`Error`] built from a format string.
#[macro_export]
macro_rules! bail {
    ($($arg:tt)*) => {
        return Err($crate::err!($($arg)*));
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    fn maybe() -> Result<i32> {
        Err(io::Error::other("io failure").into())
    }

    #[test]
    fn converts_io_error_via_question_mark_result() {
        let r: Result<i32> = maybe().context("loading");
        let msg = r.unwrap_err().to_string();
        assert_eq!(msg, "loading: io failure");
    }

    #[test]
    fn option_context_unwraps_inner() {
        let r: Result<i32> = None.with_context(|| "no value".to_string());
        assert_eq!(r.unwrap_err().to_string(), "no value");
        let r: Result<i32> = Some(7).context("some");
        assert_eq!(r.unwrap(), 7);
    }

    #[test]
    fn macros_work() {
        let e = err!("boom {}", 1);
        assert_eq!(e.to_string(), "boom 1");
        fn f() -> Result<()> {
            bail!("early {}", "stop");
        }
        assert!(f().is_err());
    }
}
