//! The stock sidebar, registered as a docked surface through the public API.
//! Owns its own scroll state (captured `Arc<Mutex<..>>` — extensions never
//! reach into host state) and routes clicks/wheel through `on_mouse`.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use ekko_ext::{
    ClientSnapshot, Color, DockEdge, DrawContext, Extension, ExtensionHost, ExtensionManifest,
    MouseKind, Rect, SurfaceMouseEvent, SurfaceSize, SurfaceSpec, UiAction, fade_toward,
};
use ekko_tui::{display_cell_width, truncate_to_cells};

use crate::rows::{self, SidebarRowKind};

const SIDEBAR_SELECTOR: &str = "> ";
const SCROLLBAR_TRACK: &str = "\u{2502}";
const SCROLLBAR_THUMB: &str = "\u{2503}";

pub struct SidebarExtension {
    /// Configured preferred width (`[ui] sidebar_width`).
    width: u16,
    scroll: Arc<Mutex<usize>>,
}

impl SidebarExtension {
    pub fn new(width: u16) -> Self {
        Self {
            width,
            scroll: Arc::new(Mutex::new(0)),
        }
    }
}

impl Extension for SidebarExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.sidebar".into(),
            name: "sidebar".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "project/session sidebar".into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        let draw_scroll = self.scroll.clone();
        let mouse_scroll = self.scroll.clone();
        host.register_surface(SurfaceSpec {
            name: "sidebar".into(),
            dock: DockEdge::Left,
            priority: 0,
            size: SurfaceSize::Scaled {
                preferred: i32::from(self.width),
                min: 24,
                max_fraction_denom: 4,
                min_remaining: 32,
            },
            hide_below: Some((64, 4)),
            visible: None,
            draw: Arc::new(move |ctx, snapshot| {
                draw_sidebar(ctx, snapshot, &draw_scroll);
            }),
            on_mouse: Some(Arc::new(move |event, snapshot| {
                handle_mouse(event, snapshot, &mouse_scroll)
            })),
            // No tick: alive sessions use a static glyph. A permanent
            // animation tick would keep the whole client render loop hot.
            wants_tick: None,
        })
    }
}

fn draw_sidebar(ctx: &mut dyn DrawContext, snapshot: &ClientSnapshot, scroll: &Mutex<usize>) {
    let (cols, rows) = ctx.size();
    let theme = &snapshot.theme;
    ctx.fill_rect(Rect::new(0, 0, cols, rows), theme.text, theme.sidebar_bg);
    if rows <= 0 {
        return;
    }

    let row_model = rows::build_rows(&snapshot.projects, &snapshot.session_name);
    let visible_rows = rows.max(0) as usize;
    let scroll_pos = {
        let mut guard = scroll.lock().expect("sidebar scroll poisoned");
        *guard = (*guard).min(row_model.len().saturating_sub(visible_rows));
        *guard
    };

    let mut content_cols = cols;
    if cols > 1 {
        content_cols = cols - 1;
        ctx.render_scrollbar(
            cols - 1,
            0,
            rows,
            visible_rows,
            row_model.len(),
            scroll_pos,
            theme.border,
            theme.sidebar_bg,
            SCROLLBAR_TRACK,
            theme.accent_2,
            SCROLLBAR_THUMB,
        );
    }

    for (offset, row) in row_model
        .iter()
        .skip(scroll_pos)
        .take(visible_rows)
        .enumerate()
    {
        let rect = Rect::new(0, offset as i32, content_cols, 1);
        match row.kind {
            SidebarRowKind::Project(_) => render_project_rule(ctx, rect, &row.text, snapshot),
            SidebarRowKind::Session { .. } => render_session_row(ctx, rect, row, snapshot),
        }
    }
}

fn render_project_rule(
    ctx: &mut dyn DrawContext,
    rect: Rect,
    label: &str,
    snapshot: &ClientSnapshot,
) {
    if rect.cols <= 0 {
        return;
    }
    let theme = &snapshot.theme;
    let value = truncate_to_cells(&format!(" {label} "), rect.cols.max(0) as usize);
    let value_cols = display_cell_width(&value) as i32;
    let label_col = rect.col + (rect.cols.saturating_sub(value_cols)) / 2;

    for col in rect.col..(rect.col + rect.cols) {
        if col >= label_col && col < label_col + value_cols {
            continue;
        }
        ctx.set_cell(
            col,
            rect.row,
            theme.border,
            theme.sidebar_bg,
            "\u{2501}",
            false,
        );
    }
    ctx.put_text_bold(
        label_col,
        rect.row,
        value_cols,
        theme.heading,
        theme.sidebar_bg,
        &value,
    );
}

fn render_session_row(
    ctx: &mut dyn DrawContext,
    rect: Rect,
    row: &rows::SidebarRow,
    snapshot: &ClientSnapshot,
) {
    if rect.cols <= 0 {
        return;
    }
    let theme = &snapshot.theme;
    let branch = if row.current { SIDEBAR_SELECTOR } else { "  " };
    let indent_cols = rect.cols.min(display_cell_width(branch) as i32);
    let branch_fg = if row.current {
        theme.accent
    } else {
        theme.border
    };
    ctx.put_text(
        rect.col,
        rect.row,
        indent_cols,
        branch_fg,
        theme.sidebar_bg,
        branch,
    );

    let glyph = if row.alive { "\u{25cf}" } else { "\u{00b7}" };
    let glyph_cols = display_cell_width(glyph) as i32;
    let text_cols = rect
        .cols
        .saturating_sub(indent_cols)
        .saturating_sub(glyph_cols + 1);
    let value = truncate_to_cells(&row.text, text_cols.max(0) as usize);

    let title_fg = if row.current { theme.text } else { theme.muted };
    let title_col = rect.col + indent_cols;
    if row.current && row.alive {
        put_gradient_text(
            ctx,
            Rect::new(title_col, rect.row, text_cols, 1),
            &value,
            theme.sidebar_bg,
            theme.accent,
            theme.accent_2,
        );
    } else {
        ctx.put_text(
            title_col,
            rect.row,
            text_cols,
            title_fg,
            theme.sidebar_bg,
            &value,
        );
    }

    let glyph_color = if row.alive {
        theme.running
    } else {
        theme.muted
    };
    let glyph_col = rect.col + rect.cols - glyph_cols;
    ctx.put_text(
        glyph_col,
        rect.row,
        glyph_cols,
        glyph_color,
        theme.sidebar_bg,
        glyph,
    );
}

/// Per-cell fg gradient from `from` to `to` across `value`'s cells.
fn put_gradient_text(
    ctx: &mut dyn DrawContext,
    rect: Rect,
    value: &str,
    bg: Color,
    from: Color,
    to: Color,
) {
    if rect.cols <= 0 || value.is_empty() {
        return;
    }
    let value_cols = display_cell_width(value) as i32;
    let denom = value_cols.saturating_sub(1).max(1) as u16;
    let mut cursor = rect.col;
    let mut buf = [0u8; 4];
    for ch in value.chars() {
        if cursor >= rect.col + rect.cols {
            break;
        }
        let encoded = ch.encode_utf8(&mut buf);
        let width = (display_cell_width(encoded) as i32).max(1);
        if cursor + width > rect.col + rect.cols {
            break;
        }
        let mix = (((cursor - rect.col) as u16 * 255) / denom) as u8;
        let fg = fade_toward(from, to, mix);
        ctx.set_cell(cursor, rect.row, fg, bg, encoded, false);
        cursor += width;
    }
}

fn handle_mouse(
    event: &SurfaceMouseEvent,
    snapshot: &ClientSnapshot,
    scroll: &Mutex<usize>,
) -> Vec<UiAction> {
    let row_model = rows::build_rows(&snapshot.projects, &snapshot.session_name);
    let visible_rows = event.region_rows.max(0) as usize;
    let mut guard = scroll.lock().expect("sidebar scroll poisoned");
    match event.kind {
        MouseKind::LeftPress => {
            let row_index = *guard + event.row.max(0) as usize;
            if let Some(row) = row_model.get(row_index)
                && matches!(row.kind, SidebarRowKind::Session { .. })
                && row.text != snapshot.session_name
            {
                return vec![UiAction::SwitchSession {
                    name: row.text.clone(),
                }];
            }
            Vec::new()
        }
        MouseKind::WheelUp => {
            let target = guard.saturating_sub(1);
            *guard = rows::ensure_visible(target, target, visible_rows, row_model.len());
            Vec::new()
        }
        MouseKind::WheelDown => {
            *guard = (*guard + 1).min(row_model.len().saturating_sub(visible_rows));
            Vec::new()
        }
        MouseKind::LeftDrag | MouseKind::LeftRelease | MouseKind::Other => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::test_group;
    use ekko_ext::ThemePalette;

    fn snapshot() -> ClientSnapshot {
        ClientSnapshot {
            session_name: "s1".into(),
            mode: ClientSnapshot::NORMAL_MODE.into(),
            cols: 80,
            rows: 24,
            grid_cols: 80,
            grid_rows: 23,
            scrollback: 0,
            projects: vec![test_group("a", &["s1", "s2"])],
            status_note: None,
            keybindings: vec![],
            now_ms: 0,
            theme: ThemePalette::fallback(),
        }
    }

    #[test]
    fn click_on_session_row_switches() {
        let ext = SidebarExtension::new(36);
        let event = SurfaceMouseEvent {
            kind: MouseKind::LeftPress,
            col: 2,
            row: 2, // row 0 = project rule, row 1 = s1, row 2 = s2
            region_cols: 30,
            region_rows: 20,
        };
        let actions = handle_mouse(&event, &snapshot(), &ext.scroll);
        assert_eq!(actions, vec![UiAction::SwitchSession { name: "s2".into() }]);
    }

    #[test]
    fn click_on_project_rule_does_nothing() {
        let ext = SidebarExtension::new(36);
        let event = SurfaceMouseEvent {
            kind: MouseKind::LeftPress,
            col: 2,
            row: 0,
            region_cols: 30,
            region_rows: 20,
        };
        assert!(handle_mouse(&event, &snapshot(), &ext.scroll).is_empty());
    }

    #[test]
    fn wheel_scrolls_and_clamps() {
        let ext = SidebarExtension::new(36);
        let wheel = |kind| SurfaceMouseEvent {
            kind,
            col: 0,
            row: 0,
            region_cols: 30,
            region_rows: 2, // 3 rows total, 2 visible -> max scroll 1
        };
        assert!(handle_mouse(&wheel(MouseKind::WheelDown), &snapshot(), &ext.scroll).is_empty());
        assert_eq!(*ext.scroll.lock().unwrap(), 1);
        handle_mouse(&wheel(MouseKind::WheelDown), &snapshot(), &ext.scroll);
        assert_eq!(*ext.scroll.lock().unwrap(), 1);
        handle_mouse(&wheel(MouseKind::WheelUp), &snapshot(), &ext.scroll);
        assert_eq!(*ext.scroll.lock().unwrap(), 0);
    }
}
