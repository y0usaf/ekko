//! The help overlay, rendered from a [`HelpPayload`] this extension builds
//! (via `OverlaySpec::build_payload`) from the live command/keybinding
//! registries at open time — so help never drifts from what is actually
//! registered.

use std::sync::Arc;

use anyhow::Result;
use ekko_ext::{
    ClientSnapshot, DrawContext, Extension, ExtensionHost, ExtensionManifest, OVERLAY_HELP,
    OverlayOutcome, OverlayPayload, OverlaySpec, OverlayState, Rect, RegistryView,
};

pub struct HelpOverlayExtension;

/// One line of the help overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelpEntry {
    Heading(String),
    Item { keys: String, description: String },
    Blank,
}

/// The keybinding and command reference, built fresh from the live
/// registries every time the overlay opens.
pub struct HelpPayload {
    pub entries: Vec<HelpEntry>,
}

struct HelpState {
    entries: Vec<HelpEntry>,
}

impl Extension for HelpOverlayExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.help".into(),
            name: "help overlay".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "keybinding and command reference".into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        host.register_overlay(OverlaySpec {
            name: OVERLAY_HELP.into(),
            description: "keybinding and command reference".into(),
            init_state: Arc::new(|payload| {
                let entries = payload
                    .and_then(|p| p.downcast::<HelpPayload>().ok())
                    .map(|p| p.entries)
                    .unwrap_or_default();
                Box::new(HelpState { entries }) as OverlayState
            }),
            render: Arc::new(render_help),
            handle_key: Arc::new(|_state, bytes| {
                if bytes == b"\x1b" || bytes == b"q" {
                    OverlayOutcome::Close
                } else {
                    OverlayOutcome::None
                }
            }),
            build_payload: Some(Arc::new(|registries| {
                Box::new(build_help_payload(registries)) as OverlayPayload
            })),
        })
    }
}

fn build_help_payload(registries: &RegistryView) -> HelpPayload {
    let mut entries = Vec::new();
    for binding in registries.keybindings.iter().filter(|b| b.mode.is_none()) {
        entries.push(HelpEntry::Item {
            keys: binding.chord_text.clone(),
            description: binding.description.clone(),
        });
    }
    // Mode-scoped bindings, grouped under their mode's heading.
    let mut modes: Vec<&str> = Vec::new();
    for mode in registries
        .keybindings
        .iter()
        .filter_map(|b| b.mode.as_deref())
    {
        if !modes.contains(&mode) {
            modes.push(mode);
        }
    }
    for mode in modes {
        entries.push(HelpEntry::Blank);
        entries.push(HelpEntry::Heading(format!("{mode} mode")));
        for binding in &registries.keybindings {
            if binding.mode.as_deref() == Some(mode) {
                entries.push(HelpEntry::Item {
                    keys: binding.chord_text.clone(),
                    description: binding.description.clone(),
                });
            }
        }
    }
    if !registries.commands.is_empty() {
        entries.push(HelpEntry::Blank);
        for command in &registries.commands {
            let mut keys = format!(":{}", command.name);
            if !command.args_hint.is_empty() {
                keys.push(' ');
                keys.push_str(&command.args_hint);
            }
            entries.push(HelpEntry::Item {
                keys,
                description: command.description.clone(),
            });
        }
    }
    HelpPayload { entries }
}

fn render_help(ctx: &mut dyn DrawContext, state: &mut OverlayState, snapshot: &ClientSnapshot) {
    let Some(state) = state.downcast_ref::<HelpState>() else {
        return;
    };
    let theme = &snapshot.theme;
    let (cols, rows) = ctx.size();
    let lines: Vec<String> = state
        .entries
        .iter()
        .map(|entry| match entry {
            HelpEntry::Heading(text) => text.clone(),
            HelpEntry::Item { keys, description } => format!("{keys}  {description}"),
            HelpEntry::Blank => String::new(),
        })
        .chain(std::iter::once("esc / q  close this help".to_string()))
        .collect();

    let width = (cols - 4).clamp(10, 60).min(cols.max(1));
    let height = (lines.len() as i32 + 2).min(rows.max(3));
    let col = (cols - width) / 2;
    let row = (rows - height) / 2;
    let rect = Rect::new(col.max(0), row.max(0), width, height);
    ctx.draw_box(rect, theme.text, theme.surface, theme.accent);
    ctx.put_text_bold(
        rect.col + 2,
        rect.row,
        rect.cols - 4,
        theme.heading,
        theme.surface,
        "help",
    );
    for (i, line) in lines.iter().enumerate() {
        let line_row = rect.row + 1 + i as i32;
        if line_row >= rect.row + rect.rows - 1 {
            break;
        }
        ctx.put_text(
            rect.col + 2,
            line_row,
            rect.cols - 4,
            theme.text,
            theme.surface,
            line,
        );
    }
}
