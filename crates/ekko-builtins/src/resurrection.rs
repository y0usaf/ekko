//! Resurrection manifests as an extension: the *policy* of when session
//! manifests are written (created on spawn, touched by the heartbeat,
//! status-stamped or deleted on exit) subscribes to the daemon's lifecycle
//! events. The manifest I/O itself is the `ekko-resurrection` library, which
//! `ekko ls` also reads directly (no daemon involved). Disable this extension
//! and no manifest is ever written — `ekko ls` then only shows live sockets.

use std::sync::Arc;

use anyhow::Result;
use ekko_ext::{
    EventHandlerRegistration, EventKind, EventPayload, Extension, ExtensionHost, ExtensionManifest,
    LifecycleEvent, SessionExitReason,
};
use ekko_resurrection::SessionStatus;

pub struct ResurrectionExtension;

fn subscription(
    kind: EventKind,
    label: &str,
    handler: impl Fn(LifecycleEvent) -> Result<()> + Send + Sync + 'static,
) -> EventHandlerRegistration {
    EventHandlerRegistration {
        event: kind,
        label: format!("ekko-builtins.resurrection/{label}"),
        handler: Arc::new(move |event| {
            handler(event)?;
            Ok(None)
        }),
    }
}

impl Extension for ResurrectionExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.resurrection".into(),
            name: "resurrection manifests".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "session manifests for `ekko ls` and detached-session listing".into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        host.subscribe(subscription(EventKind::SessionCreated, "create", |event| {
            if let EventPayload::SessionCreated {
                session_name,
                shell,
                cwd,
            } = &event.payload
            {
                ekko_resurrection::create(session_name, cwd, shell)?;
            }
            Ok(())
        }))?;
        host.subscribe(subscription(EventKind::Heartbeat, "touch", |event| {
            if let EventPayload::Heartbeat { session_name } = &event.payload {
                ekko_resurrection::touch(session_name)?;
            }
            Ok(())
        }))?;
        host.subscribe(subscription(
            EventKind::SessionExited,
            "finalize",
            |event| {
                if let EventPayload::SessionExited {
                    session_name,
                    reason,
                    ..
                } = &event.payload
                {
                    match reason {
                        SessionExitReason::Killed => ekko_resurrection::delete(session_name),
                        SessionExitReason::Crashed => {
                            ekko_resurrection::set_status(session_name, SessionStatus::Crashed)?;
                        }
                        SessionExitReason::ShellExited | SessionExitReason::Shutdown => {
                            ekko_resurrection::set_status(session_name, SessionStatus::Exited)?;
                        }
                    }
                }
                Ok(())
            },
        ))?;
        Ok(())
    }
}
