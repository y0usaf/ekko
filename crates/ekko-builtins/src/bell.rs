//! Bell notice policy: rate-limit terminal bells into an `EmitNotice`
//! return, which the hub translates into a wire `Notice` shown in the
//! client's statusbar. The raw wire `Bell` (the audible \x07) is core
//! mechanism and is sent by the hub unconditionally; this extension only
//! owns the "visual bell" policy.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use ekko_ext::{
    EventHandlerRegistration, EventKind, EventReturn, Extension, ExtensionHost, ExtensionManifest,
    NoticeLevel,
};

const BELL_NOTICE_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct BellThrottleExtension {
    last_notice: Arc<Mutex<Option<Instant>>>,
}

impl Extension for BellThrottleExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.bell".into(),
            name: "bell notices".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "rate-limited visual bell notices".into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        let last_notice = self.last_notice.clone();
        host.subscribe(EventHandlerRegistration {
            event: EventKind::Bell,
            label: "ekko-builtins.bell/notice".into(),
            handler: Arc::new(move |_event| {
                let mut last = last_notice.lock().expect("bell throttle poisoned");
                let now = Instant::now();
                if last.is_some_and(|at| now.duration_since(at) < BELL_NOTICE_INTERVAL) {
                    return Ok(None);
                }
                *last = Some(now);
                Ok(Some(EventReturn::EmitNotice {
                    level: NoticeLevel::Info,
                    message: "bell".into(),
                }))
            }),
        })
    }
}
