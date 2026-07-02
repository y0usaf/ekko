//! Buffered draw ops: the guard between Lua and the render hot path.
//!
//! Native extensions draw straight into the host's `DrawContext` (trusted,
//! unguarded — the documented hot-path exception). Lua draw callbacks
//! instead receive a `ctx` table whose functions append data-only ops to a
//! buffer while the instruction budget is live; the host replays the buffer
//! into the real context only after the Lua call returns cleanly. A script
//! that errors or overruns its budget draws nothing at all.
//!
//! Convention: `ctx` functions are dot-called (`ctx.put_text(...)`, not
//! `ctx:put_text(...)`). Colors are role names from the active theme
//! (`"text"`, `"accent"`, ...) or `"#rrggbb"` hex.

use std::sync::{Arc, Mutex};

use mlua::{Lua, Table};
use ekko_ext::{Color, DrawContext, Rect, ThemePalette};

use crate::convert::resolve_color;

pub enum DrawOp {
    FillRect {
        rect: Rect,
        fg: Color,
        bg: Color,
    },
    SetCell {
        col: i32,
        row: i32,
        fg: Color,
        bg: Color,
        text: String,
        underline: bool,
    },
    PutText {
        col: i32,
        row: i32,
        max_cols: i32,
        fg: Color,
        bg: Color,
        value: String,
        reverse: bool,
        bold: bool,
    },
    DrawBox {
        rect: Rect,
        fill_fg: Color,
        bg: Color,
        border: Color,
    },
}

pub fn replay(ops: &[DrawOp], ctx: &mut dyn DrawContext) {
    for op in ops {
        match op {
            DrawOp::FillRect { rect, fg, bg } => ctx.fill_rect(*rect, *fg, *bg),
            DrawOp::SetCell {
                col,
                row,
                fg,
                bg,
                text,
                underline,
            } => ctx.set_cell(*col, *row, *fg, *bg, text, *underline),
            DrawOp::PutText {
                col,
                row,
                max_cols,
                fg,
                bg,
                value,
                reverse,
                bold,
            } => ctx.put_text_styled(*col, *row, *max_cols, *fg, *bg, value, *reverse, *bold),
            DrawOp::DrawBox {
                rect,
                fill_fg,
                bg,
                border,
            } => ctx.draw_box(*rect, *fill_fg, *bg, *border),
        }
    }
}

/// Build the `ctx` table handed to a Lua draw callback: `size()`,
/// `fill_rect`, `set_cell`, `put_text`, `put_text_bold`, `put_text_styled`,
/// and `draw_box`, all appending to `ops`.
pub fn ops_context_table(
    lua: &Lua,
    ops: Arc<Mutex<Vec<DrawOp>>>,
    size: (i32, i32),
    palette: ThemePalette,
) -> mlua::Result<Table> {
    let ctx = lua.create_table()?;

    ctx.set(
        "size",
        lua.create_function(move |_, ()| Ok((size.0, size.1)))?,
    )?;

    let buf = ops.clone();
    ctx.set(
        "fill_rect",
        lua.create_function(
            move |_, (col, row, cols, rows, fg, bg): (i32, i32, i32, i32, String, String)| {
                buf.lock().unwrap().push(DrawOp::FillRect {
                    rect: Rect::new(col, row, cols, rows),
                    fg: color(&fg, &palette)?,
                    bg: color(&bg, &palette)?,
                });
                Ok(())
            },
        )?,
    )?;

    let buf = ops.clone();
    ctx.set(
        "set_cell",
        lua.create_function(
            move |_,
                  (col, row, fg, bg, text, underline): (
                i32,
                i32,
                String,
                String,
                String,
                Option<bool>,
            )| {
                buf.lock().unwrap().push(DrawOp::SetCell {
                    col,
                    row,
                    fg: color(&fg, &palette)?,
                    bg: color(&bg, &palette)?,
                    text,
                    underline: underline.unwrap_or(false),
                });
                Ok(())
            },
        )?,
    )?;

    for (name, bold_default, reverse_default) in
        [("put_text", false, false), ("put_text_bold", true, false)]
    {
        let buf = ops.clone();
        ctx.set(
            name,
            lua.create_function(
                move |_,
                      (col, row, max_cols, fg, bg, value): (
                    i32,
                    i32,
                    i32,
                    String,
                    String,
                    String,
                )| {
                    buf.lock().unwrap().push(DrawOp::PutText {
                        col,
                        row,
                        max_cols,
                        fg: color(&fg, &palette)?,
                        bg: color(&bg, &palette)?,
                        value,
                        reverse: reverse_default,
                        bold: bold_default,
                    });
                    Ok(())
                },
            )?,
        )?;
    }

    let buf = ops.clone();
    ctx.set(
        "put_text_styled",
        lua.create_function(
            move |_,
                  (col, row, max_cols, fg, bg, value, reverse, bold): (
                i32,
                i32,
                i32,
                String,
                String,
                String,
                Option<bool>,
                Option<bool>,
            )| {
                buf.lock().unwrap().push(DrawOp::PutText {
                    col,
                    row,
                    max_cols,
                    fg: color(&fg, &palette)?,
                    bg: color(&bg, &palette)?,
                    value,
                    reverse: reverse.unwrap_or(false),
                    bold: bold.unwrap_or(false),
                });
                Ok(())
            },
        )?,
    )?;

    let buf = ops;
    ctx.set(
        "draw_box",
        lua.create_function(
            move |_,
                  (col, row, cols, rows, fill_fg, bg, border): (
                i32,
                i32,
                i32,
                i32,
                String,
                String,
                String,
            )| {
                buf.lock().unwrap().push(DrawOp::DrawBox {
                    rect: Rect::new(col, row, cols, rows),
                    fill_fg: color(&fill_fg, &palette)?,
                    bg: color(&bg, &palette)?,
                    border: color(&border, &palette)?,
                });
                Ok(())
            },
        )?,
    )?;

    Ok(ctx)
}

fn color(word: &str, palette: &ThemePalette) -> mlua::Result<Color> {
    resolve_color(word, palette).map_err(|e| mlua::Error::RuntimeError(e.to_string()))
}
