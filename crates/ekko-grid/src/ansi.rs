//! Diff renderer (port of pi-harness `tui::ansi`): emits only the cells that
//! changed since the previous frame, wrapped in synchronized-update markers so
//! the terminal applies each frame atomically.
//!
//! Implements the renderer optimization plan:
//! - Priority 1 (interning / packed cells): diffs over flat `PackedCell`
//!   arrays with contiguous comparisons.
//! - Priority 2 (damage tracking): uses `CellSurface::take_dirty_indices`
//!   to skip clean regions.
//! - Priority 3 (patch optimization): builds a `Vec<Patch>` IR, optimizes
//!   it, then runs `AnsiCodegen` with relative cursor moves and style
//!   diffing.
//! - Priority 4 (scrollback modeling): hashes rows to detect uniform shifts
//!   and emit `\x1b[NS` scroll-up sequences instead of repainting moved cells.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{self, Write};

use crate::cell_surface::CellSurface;
use crate::color::Color;
use crate::layout::CellRect;
use crate::packed::{PackedCell, PackedStyle, StringInterner};

pub const DEFAULT_FG: Color = Color::rgba(0, 0, 0, 0);
pub const DEFAULT_BG: Color = Color::rgba(0, 0, 0, 0);

/// Where to park the real terminal cursor after a frame (1-based ANSI is
/// handled internally; fields are 0-based cells).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HardwareCursor {
    pub col: i32,
    pub row: i32,
}

// ---------------------------------------------------------------------------
// Style IR
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct AnsiStyle {
    fg: Color,
    bg: Color,
    bold: bool,
    italic: bool,
    underline: bool,
    reverse: bool,
}

impl AnsiStyle {
    fn from_packed(style: &PackedStyle) -> Self {
        Self {
            fg: style.fg(),
            bg: style.bg(),
            bold: style.bold(),
            italic: style.italic(),
            underline: style.underline(),
            reverse: style.reverse(),
        }
    }
}

// ---------------------------------------------------------------------------
// Patch IR (Priority 3)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum Patch {
    Move { row: u16, col: u16 },
    Text { string: String, width: u16 },
    Style(AnsiStyle),
    Newline,
}

struct PatchOptimizer {
    patches: Vec<Patch>,
}

impl PatchOptimizer {
    fn optimize(mut self) -> Vec<Patch> {
        self.pass_merge_adjacent_text();
        self.pass_skip_redundant_styles();
        self.patches
    }

    /// Merge adjacent `Text` patches that have no intervening `Move`/`Style`.
    fn pass_merge_adjacent_text(&mut self) {
        let mut out = Vec::with_capacity(self.patches.len());
        for patch in self.patches.drain(..) {
            match (patch, out.last_mut()) {
                (
                    Patch::Text { string, width },
                    Some(Patch::Text {
                        string: prev_str,
                        width: prev_w,
                    }),
                ) => {
                    prev_str.push_str(&string);
                    *prev_w += width;
                }
                (p, _) => out.push(p),
            }
        }
        self.patches = out;
    }

    /// Drop `Style` patches that would set the same style as the current
    /// running style.
    fn pass_skip_redundant_styles(&mut self) {
        let mut out = Vec::with_capacity(self.patches.len());
        let mut cur_style = AnsiStyle::default();
        for patch in self.patches.drain(..) {
            if let Patch::Style(target) = &patch {
                if *target == cur_style {
                    continue;
                }
                cur_style = *target;
            }
            out.push(patch);
        }
        self.patches = out;
    }
}

// ---------------------------------------------------------------------------
// Codegen (Priority 3)
// ---------------------------------------------------------------------------

struct AnsiCodegen {
    current_style: AnsiStyle,
    cur_row: u16,
    cur_col: u16,
    cols: u16,
    buf: String,
}

impl AnsiCodegen {
    fn new(cols: u16) -> Self {
        Self {
            current_style: AnsiStyle::default(),
            cur_row: 0,
            cur_col: 0,
            cols,
            buf: String::new(),
        }
    }

    /// Emit only the differential style attributes relative to
    /// `current_style`. Uses a safe `\x1b[0m` reset when complex attributes
    /// (bold/reverse/underline) are turning off, then re-applies what's left.
    fn apply_style(&mut self, target: AnsiStyle) {
        let cur = self.current_style;
        if target == cur {
            return;
        }

        // If any boolean attribute is turning off, a full reset is the
        // simplest correct path; we then re-emit fg/bg/any remaining booleans.
        let cur_complex = cur.bold || cur.italic || cur.reverse || cur.underline;
        let target_complex = target.bold || target.italic || target.reverse || target.underline;
        if cur_complex && !target_complex {
            self.buf.push_str("\x1b[0m");
            self.current_style = AnsiStyle::default();
        }

        // Now diff the colors.
        let after_reset = self.current_style;
        if target.fg != after_reset.fg {
            self.write_fg(target.fg);
            self.current_style.fg = target.fg;
        }
        if target.bg != after_reset.bg {
            self.write_bg(target.bg);
            self.current_style.bg = target.bg;
        }
        // Diff booleans (only when we didn't just reset — after a reset all
        // booleans are off, so we simply set the target ones).
        if self.current_style != AnsiStyle::default() || cur_complex == target_complex {
            if target.bold != self.current_style.bold {
                if target.bold {
                    self.buf.push_str("\x1b[1m");
                } else {
                    self.buf.push_str("\x1b[22m");
                }
                self.current_style.bold = target.bold;
            }
            if target.italic != self.current_style.italic {
                if target.italic {
                    self.buf.push_str("\x1b[3m");
                } else {
                    self.buf.push_str("\x1b[23m");
                }
                self.current_style.italic = target.italic;
            }
            if target.underline != self.current_style.underline {
                if target.underline {
                    self.buf.push_str("\x1b[4m");
                } else {
                    self.buf.push_str("\x1b[24m");
                }
                self.current_style.underline = target.underline;
            }
            if target.reverse != self.current_style.reverse {
                if target.reverse {
                    self.buf.push_str("\x1b[7m");
                } else {
                    self.buf.push_str("\x1b[27m");
                }
                self.current_style.reverse = target.reverse;
            }
        } else {
            // After a reset: just turn on whatever the target wants.
            if target.bold {
                self.buf.push_str("\x1b[1m");
            }
            if target.italic {
                self.buf.push_str("\x1b[3m");
            }
            if target.underline {
                self.buf.push_str("\x1b[4m");
            }
            if target.reverse {
                self.buf.push_str("\x1b[7m");
            }
            self.current_style.bold = target.bold;
            self.current_style.italic = target.italic;
            self.current_style.underline = target.underline;
            self.current_style.reverse = target.reverse;
        }
    }

    fn write_fg(&mut self, color: Color) {
        use std::fmt::Write;
        let _ = write!(self.buf, "{}", SgrColor { color, bg: false });
    }

    fn write_bg(&mut self, color: Color) {
        use std::fmt::Write;
        let _ = write!(self.buf, "{}", SgrColor { color, bg: true });
    }

    /// Emit a cursor move. Uses a relative forward move (`\x1b[NC`) only when
    /// on the same row and the target is ahead *and* won't wrap past the
    /// column bound; otherwise an absolute `\x1b[row;colH`.
    fn emit_move(&mut self, row: u16, col: u16) {
        if self.cur_row == row && col > self.cur_col {
            let diff = col - self.cur_col;
            // Guard against relative moves that would exceed the column
            // bound and potentially wrap to the next line in some terminals.
            if self.cur_col + diff <= self.cols {
                use std::fmt::Write;
                let _ = write!(self.buf, "\x1b[{}C", diff);
                self.cur_col = col;
                return;
            }
        }
        use std::fmt::Write;
        let _ = write!(self.buf, "\x1b[{};{}H", row, col);
        self.cur_row = row;
        self.cur_col = col;
    }

    fn emit_text(&mut self, string: &str, width: u16) {
        self.buf.push_str(string);
        // Advance the cursor model by the text width (in cells).
        self.cur_col = self.cur_col.saturating_add(width);
        // Clamp at column bound; a newline in the IR resets to col 1.
        if self.cur_col > self.cols {
            self.cur_col = self.cols;
        }
    }

    fn finish(mut self) -> String {
        // Always end with a reset so the next frame starts clean.
        self.buf.push_str("\x1b[0m");
        self.buf
    }
}

// ---------------------------------------------------------------------------
// Scrollback modeling (Priority 4)
// ---------------------------------------------------------------------------

/// Hash one row of packed cells directly from the flat array, resolving text
/// through the interner without allocating a temporary `String`/`Vec<u8>` per
/// cell (the plan's "do not allocate temporary String objects" guidance).
fn hash_row(cells: &[PackedCell], interner: &StringInterner) -> u64 {
    let mut hasher = DefaultHasher::new();
    for cell in cells {
        hasher.write(interner.resolve(cell.text).as_bytes());
        cell.style.fg.hash(&mut hasher);
        cell.style.bg.hash(&mut hasher);
        cell.style.flags.hash(&mut hasher);
    }
    hasher.finish()
}

/// A detected uniform viewport shift within the terminal pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrollDirective {
    None,
    Up(u32),
    Down(u32),
}

/// Detect a uniform shift by comparing row hashes. `Up(n)` means the pane
/// scrolled up by `n` (new lines exposed at the bottom); `Down(n)` means it
/// scrolled down by `n` (new lines exposed at the top, e.g. scrollback nav).
/// Search depth is capped at 20 to bound the O(N^2) worst case, and the
/// shifted slice must cover the entire remaining pane (plan's risk guidance).
fn detect_scroll(prev_hashes: &[u64], curr_hashes: &[u64], rows: usize) -> ScrollDirective {
    for n in 1..rows.min(20) {
        // Scroll up: curr[0..rows-n] == prev[n..rows]
        if curr_hashes[..rows - n] == prev_hashes[n..] {
            return ScrollDirective::Up(n as u32);
        }
        // Scroll down: curr[n..rows] == prev[0..rows-n]
        if curr_hashes[n..] == prev_hashes[..rows - n] {
            return ScrollDirective::Down(n as u32);
        }
    }
    ScrollDirective::None
}

// ---------------------------------------------------------------------------
// Renderer
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct AnsiRenderer {
    prev_row_hashes: Option<Vec<u64>>,
}

impl AnsiRenderer {
    /// Drop scroll frame history so the next render can't mistake unrelated
    /// content (after a resize, alt-screen switch, or session focus change)
    /// for a scroll.
    pub fn invalidate(&mut self) {
        self.prev_row_hashes = None;
    }

    pub fn render<W: Write>(
        &mut self,
        out: &mut W,
        surface: &mut CellSurface,
        term_pane: CellRect,
        hardware_cursor: Option<HardwareCursor>,
    ) -> io::Result<()> {
        write!(out, "\x1b[?2026h\x1b[?25l")?;

        if surface.dirty_all {
            // Full repaint: first frame, resize, or alt-screen switch.
            // Consume the dirty state so steady state resumes.
            render_full(out, surface)?;
            surface.dirty_all = false;
            surface.dirty_region.clear();
        } else {
            // Steady state: the surface's own damage tracking is sufficient
            // for correctness. `prev_row_hashes` (possibly absent or stale)
            // only gates the scroll-detection optimization inside.
            render_diff(out, surface, term_pane, &mut self.prev_row_hashes)?;
        }

        render_hardware_cursor(out, hardware_cursor)?;
        write!(out, "\x1b[?2026l")?;

        // Cache the terminal pane's row hashes so the next frame can detect a
        // uniform shift. Scoped to the pane (not the whole surface): the
        // sidebar/statusbar don't shift with terminal scrolls.
        //
        // When the pane doesn't span the full width (sidebar present), scroll
        // detection is bypassed, so computing hashes would be dead work — and
        // keeping the old ones would let a later full-width frame diff against
        // stale history, so they are dropped instead.
        let pane_full_width = term_pane.col == 0 && term_pane.col + term_pane.cols == surface.cols;
        self.prev_row_hashes = if pane_full_width {
            Some(pane_row_hashes(surface, term_pane))
        } else {
            None
        };

        out.flush()?;
        Ok(())
    }
}

/// Compute per-row hashes for the terminal pane's row band.
fn pane_row_hashes(surface: &CellSurface, term_pane: CellRect) -> Vec<u64> {
    let cols = surface.cols as usize;
    let top = term_pane.row as usize;
    let height = term_pane.rows as usize;
    let left = term_pane.col as usize;
    let pane_cols = term_pane.cols as usize;
    (0..height)
        .map(|r| {
            let start = (top + r) * cols + left;
            let end = start + pane_cols;
            hash_row(&surface.cells[start..end], &surface.interner)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Full repaint
// ---------------------------------------------------------------------------

fn render_full<W: Write>(out: &mut W, surface: &CellSurface) -> io::Result<()> {
    write!(out, "\x1b[H")?;
    for row in 0..surface.rows {
        write!(out, "\x1b[{};1H", row + 1)?;
        let mut current_style: Option<AnsiStyle> = None;
        let row_start = (row * surface.cols) as usize;
        for col in 0..surface.cols {
            let cell = &surface.cells[row_start + col as usize];
            if cell.style.continuation() {
                continue;
            }
            let style = AnsiStyle::from_packed(&cell.style);
            if current_style != Some(style) {
                write_style_full(out, style)?;
                current_style = Some(style);
            }
            out.write_all(surface.interner.resolve(cell.text).as_bytes())?;
        }
        write!(out, "\x1b[0m")?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Diff repaint
// ---------------------------------------------------------------------------

fn render_diff<W: Write>(
    out: &mut W,
    surface: &mut CellSurface,
    term_pane: CellRect,
    prev_row_hashes: &mut Option<Vec<u64>>,
) -> io::Result<()> {
    let cols = surface.cols as usize;

    // Consume the dirty region up front: it tells us what changed this frame
    // (relative to the surface's own prior content, i.e. the previous frame,
    // since the surface is persistent). `dirty_all` is handled by the caller.
    let dirty = surface.take_dirty_indices();
    // `take_dirty_indices` only returns `None` on `dirty_all`, which the caller
    // already routed to `render_full`; here it is always `Some`.
    let indices = dirty.unwrap_or_default();

    // Nothing changed this frame — emit no cell updates.
    if indices.is_empty() {
        return Ok(());
    }

    // -- Priority 4: scroll detection (terminal-pane-scoped) --------------
    // Only attempt a scroll escape when the pane spans the full width: DECSTBM
    // scroll regions are full-width row bands, so with a sidebar present a
    // scroll would also move the sidebar. When a scroll is detected but unsafe
    // to emit (sidebar up), we fall through to the dirty diff below, which
    // repaints the shifted cells correctly.
    let pane_full_width = term_pane.col == 0 && term_pane.col + term_pane.cols == surface.cols;

    let scroll = if pane_full_width
        && let Some(prev_hashes) = prev_row_hashes.as_ref()
        && prev_hashes.len() == term_pane.rows as usize
    {
        let curr_hashes = pane_row_hashes(surface, term_pane);
        detect_scroll(prev_hashes, &curr_hashes, term_pane.rows as usize)
    } else {
        ScrollDirective::None
    };

    if pane_full_width && scroll != ScrollDirective::None {
        // The shifted rows are handled by the scroll escape; only the
        // newly-exposed rows are repainted inside `emit_scroll`. Dirty cells
        // OUTSIDE the pane's row band (statusbar, top-docked chrome) changed
        // independently of the scroll and still need their diff.
        emit_scroll(out, surface, term_pane, scroll, cols)?;
        let pane_rows =
            term_pane.row as usize * cols..(term_pane.row + term_pane.rows) as usize * cols;
        let outside: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|index| !pane_rows.contains(index))
            .collect();
        if !outside.is_empty() {
            let patches = collect_dirty_patches(surface, &outside, cols);
            emit_patches(out, surface, patches, cols as u16)?;
        }
        return Ok(());
    }

    // -- Priority 2 + 3: dirty-index diff with patch IR -------------------
    let patches = collect_dirty_patches(surface, &indices, cols);
    emit_patches(out, surface, patches, cols as u16)
}

/// Emit a DECSTBM-scoped scroll for the terminal pane (full-width only), then
/// repaint only the newly-exposed rows.
fn emit_scroll<W: Write>(
    out: &mut W,
    surface: &CellSurface,
    term_pane: CellRect,
    directive: ScrollDirective,
    cols: usize,
) -> io::Result<()> {
    let top = term_pane.row + 1; // 1-based region top
    let bottom = term_pane.row + term_pane.rows; // 1-based region bottom
    let (amount, scroll_down) = match directive {
        ScrollDirective::Up(n) => (n, false),
        ScrollDirective::Down(n) => (n, true),
        ScrollDirective::None => return Ok(()),
    };

    // Scope scrolling to the pane's row band, then reset to full screen so the
    // following absolute cursor moves are unambiguous.
    write!(out, "\x1b[{top};{bottom}r")?;
    // Move to the bottom (scroll up) or top (scroll down) of the region; the
    // column is irrelevant for SU/SD.
    if scroll_down {
        write!(out, "\x1b[{top};1H\x1b[{amount}T")?;
    } else {
        write!(out, "\x1b[{bottom};1H\x1b[{amount}S")?;
    }
    write!(out, "\x1b[r")?;

    // Repaint only the newly-exposed rows: the bottom `amount` (scroll up) or
    // the top `amount` (scroll down) of the pane.
    let amt = amount as usize;
    let height = term_pane.rows as usize;
    let left = term_pane.col as usize;
    let pane_cols = term_pane.cols as usize;
    let repaint_rows: Box<dyn Iterator<Item = usize>> = if scroll_down {
        Box::new((0..amt).map(|r| term_pane.row as usize + r))
    } else {
        let start = term_pane.row as usize + height - amt;
        Box::new((0..amt).map(move |r| start + r))
    };

    let mut patches = Vec::new();
    for row in repaint_rows {
        collect_full_row_patches(surface, row, left, pane_cols, cols, &mut patches);
    }
    emit_patches(out, surface, patches, cols as u16)
}

/// Group dirty cell indices into Move/Style/Text run patches. Cells are
/// assumed to be row-major flat indices into `surface.cells`.
fn collect_dirty_patches(surface: &CellSurface, indices: &[usize], cols: usize) -> Vec<Patch> {
    let mut patches = Vec::new();
    let mut idx_iter = indices.iter().copied().peekable();
    while let Some(start_idx) = idx_iter.next() {
        let start_row = start_idx / cols;
        let start_col = start_idx % cols;
        // Skip a leading continuation cell (its wide glyph is emitted by the
        // main cell's run).
        let cell = &surface.cells[start_idx];
        if cell.style.continuation() {
            continue;
        }
        let style = AnsiStyle::from_packed(&cell.style);
        let mut text_buf = String::new();
        let mut width: u16 = 0;
        text_buf.push_str(surface.interner.resolve(cell.text));
        width += cell_text_width(surface, cell);

        let mut run_end_col = start_col;
        // Extend the run across immediately-following dirty cells on the same
        // row with the same style.
        while let Some(&next_idx) = idx_iter.peek() {
            let next_row = next_idx / cols;
            let next_col = next_idx % cols;
            if next_row != start_row || next_col != run_end_col + 1 {
                break;
            }
            let next_cell = &surface.cells[next_idx];
            if next_cell.style.continuation() {
                run_end_col = next_col;
                idx_iter.next();
                continue;
            }
            if AnsiStyle::from_packed(&next_cell.style) != style {
                break;
            }
            text_buf.push_str(surface.interner.resolve(next_cell.text));
            width += cell_text_width(surface, next_cell);
            run_end_col = next_col;
            idx_iter.next();
        }
        patches.push(Patch::Move {
            row: (start_row + 1) as u16,
            col: (start_col + 1) as u16,
        });
        patches.push(Patch::Style(style));
        patches.push(Patch::Text {
            string: text_buf,
            width,
        });
    }
    patches
}

/// Emit every non-continuation cell in `row` over columns `[col0, col0+width)`
/// (1-based row in the emitted `Move`), grouped into style runs. Used to
/// repaint the freshly-exposed rows after a scroll, where there is no
/// "previous" set to diff against.
fn collect_full_row_patches(
    surface: &CellSurface,
    row: usize,
    col0: usize,
    width: usize,
    cols: usize,
    patches: &mut Vec<Patch>,
) {
    let row_start = row * cols;
    let end_col = col0 + width;
    let mut col = col0;
    while col < end_col {
        let index = row_start + col;
        let cell = &surface.cells[index];
        if cell.style.continuation() {
            col += 1;
            continue;
        }
        let style = AnsiStyle::from_packed(&cell.style);
        let start_col = col;
        let mut text_buf = String::new();
        let mut text_width: u16 = 0;
        while col < end_col {
            let idx = row_start + col;
            let c = &surface.cells[idx];
            if c.style.continuation() {
                col += 1;
                continue;
            }
            if AnsiStyle::from_packed(&c.style) != style {
                break;
            }
            text_buf.push_str(surface.interner.resolve(c.text));
            text_width += cell_text_width(surface, c);
            col += 1;
            // Skip following continuation cells of this wide glyph.
            while col < end_col && surface.cells[row_start + col].style.continuation() {
                col += 1;
            }
        }
        if !text_buf.is_empty() {
            patches.push(Patch::Move {
                row: (row + 1) as u16,
                col: (start_col + 1) as u16,
            });
            patches.push(Patch::Style(style));
            patches.push(Patch::Text {
                string: text_buf,
                width: text_width,
            });
        }
    }
}

fn cell_text_width(surface: &CellSurface, cell: &PackedCell) -> u16 {
    let text = surface.interner.resolve(cell.text);
    text.chars()
        .map(|c| {
            unicode_width::UnicodeWidthChar::width(c)
                .unwrap_or(1)
                .max(if cell.style.continuation() { 0 } else { 1 }) as u16
        })
        .sum::<u16>()
        .max(1)
}

fn emit_patches<W: Write>(
    out: &mut W,
    _surface: &CellSurface,
    patches: Vec<Patch>,
    cols: u16,
) -> io::Result<()> {
    let optimized = PatchOptimizer { patches }.optimize();
    let mut codegen = AnsiCodegen::new(cols);
    for patch in &optimized {
        match patch {
            Patch::Move { row, col } => codegen.emit_move(*row, *col),
            Patch::Style(style) => codegen.apply_style(*style),
            Patch::Text { string, width } => codegen.emit_text(string, *width),
            Patch::Newline => {
                codegen.buf.push('\n');
                codegen.cur_row += 1;
                codegen.cur_col = 1;
            }
        }
    }
    let buf = codegen.finish();
    out.write_all(buf.as_bytes())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn render_hardware_cursor<W: Write>(
    out: &mut W,
    hardware_cursor: Option<HardwareCursor>,
) -> io::Result<()> {
    if let Some(cursor) = hardware_cursor {
        write!(out, "\x1b[{};{}H\x1b[?25h", cursor.row + 1, cursor.col + 1)
    } else {
        write!(out, "\x1b[?25l")
    }
}

/// Full style write (used by `render_full`): reset + fg/bg + booleans.
fn write_style_full<W: Write>(out: &mut W, style: AnsiStyle) -> io::Result<()> {
    write!(out, "\x1b[0m")?;
    write_fg(out, style.fg)?;
    write_bg(out, style.bg)?;
    if style.bold {
        write!(out, "\x1b[1m")?;
    }
    if style.italic {
        write!(out, "\x1b[3m")?;
    }
    if style.underline {
        write!(out, "\x1b[4m")?;
    }
    if style.reverse {
        write!(out, "\x1b[7m")?;
    }
    Ok(())
}

fn write_fg<W: Write>(out: &mut W, color: Color) -> io::Result<()> {
    write!(out, "{}", SgrColor { color, bg: false })
}

fn write_bg<W: Write>(out: &mut W, color: Color) -> io::Result<()> {
    write!(out, "{}", SgrColor { color, bg: true })
}

/// One SGR color sequence, shared by the diff codegen (`String` buffer) and
/// `render_full` (`io::Write`). Every foreground parameter has a background
/// twin at +10 (default 39→49, base 30→40, bright 90→100, extended 38→48),
/// so a single implementation covers both layers.
struct SgrColor {
    color: Color,
    bg: bool,
}

impl std::fmt::Display for SgrColor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let off: u16 = if self.bg { 10 } else { 0 };
        let default = if self.bg { DEFAULT_BG } else { DEFAULT_FG };
        if self.color == default {
            return write!(f, "\x1b[{}m", 39 + off);
        }
        if let Some(index) = self.color.ansi_index_value() {
            return match index {
                0..=7 => write!(f, "\x1b[{}m", 30 + off + u16::from(index)),
                8..=15 => write!(f, "\x1b[{}m", 90 + off + u16::from(index) - 8),
                _ => write!(f, "\x1b[{};5;{index}m", 38 + off),
            };
        }
        let (r, g, b) = self.color.rgb_components();
        if ekko_tui::has_truecolor() {
            write!(f, "\x1b[{};2;{r};{g};{b}m", 38 + off)
        } else {
            write!(
                f,
                "\x1b[{};5;{}m",
                38 + off,
                ekko_tui::rgb_to_xterm256(r, g, b)
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render one frame of a persistent surface (terminal pane = whole grid).
    fn frame(renderer: &mut AnsiRenderer, surface: &mut CellSurface) -> String {
        let pane = CellRect::new(0, 0, surface.cols, surface.rows);
        let mut out = Vec::new();
        renderer.render(&mut out, surface, pane, None).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn second_identical_frame_emits_no_cells() {
        let mut renderer = AnsiRenderer::default();
        let mut surface = CellSurface::new(4, 2, DEFAULT_FG, DEFAULT_BG);
        surface.put_text(0, 0, 4, DEFAULT_FG, DEFAULT_BG, "hi");
        let first = frame(&mut renderer, &mut surface);
        assert!(first.contains("hi"));
        assert!(first.starts_with("\x1b[?2026h"));
        assert!(first.ends_with("\x1b[?2026l"));
        // No writes between frames -> dirty region empty -> no cell updates.
        let second = frame(&mut renderer, &mut surface);
        assert!(!second.contains("hi"));
    }

    #[test]
    fn changed_cell_is_rewritten_in_place() {
        let mut renderer = AnsiRenderer::default();
        let mut surface = CellSurface::new(4, 2, DEFAULT_FG, DEFAULT_BG);
        surface.put_text(0, 0, 4, DEFAULT_FG, DEFAULT_BG, "ab");
        frame(&mut renderer, &mut surface);
        surface.put_text(1, 0, 4, DEFAULT_FG, DEFAULT_BG, "X");
        let diff = frame(&mut renderer, &mut surface);
        assert!(diff.contains("\x1b[1;2H"));
        assert!(diff.contains('X'));
        assert!(!diff.contains('a'));
    }

    #[test]
    fn shrinking_line_rewrites_blank_tail_cells() {
        let mut renderer = AnsiRenderer::default();
        let mut surface = CellSurface::new(6, 1, DEFAULT_FG, DEFAULT_BG);
        surface.put_text(0, 0, 6, DEFAULT_FG, DEFAULT_BG, "abcdef");
        frame(&mut renderer, &mut surface);

        // Shorten the line: write "ab", then blank the now-unused tail cells.
        // (A real compositor clears trailing cells when content shrinks; with
        // mark-on-change those blanking writes are what dirty the tail.)
        surface.put_text(0, 0, 6, DEFAULT_FG, DEFAULT_BG, "ab");
        surface.fill_rect(CellRect::new(2, 0, 4, 1), DEFAULT_FG, DEFAULT_BG);
        let diff = frame(&mut renderer, &mut surface);

        assert!(diff.contains("\x1b[1;3H"), "diff={:?}", diff);
        assert!(diff.contains("    "), "diff={:?}", diff);
    }

    #[test]
    fn recolors_reversed_cells_when_only_colors_change() {
        ekko_tui::color_cap::force_color_capability(
            ekko_tui::color_cap::ColorCapability::TrueColor,
        );
        let mut renderer = AnsiRenderer::default();
        let mut surface = CellSurface::new(2, 1, DEFAULT_FG, DEFAULT_BG);
        surface.put_text_styled(
            0,
            0,
            2,
            Color::rgb(255, 0, 0),
            Color::rgb(0, 0, 0),
            "X",
            true,
            false,
        );
        frame(&mut renderer, &mut surface);

        // Same text, same reverse, different fg -> only the color changes.
        surface.put_text_styled(
            0,
            0,
            2,
            Color::rgb(0, 255, 0),
            Color::rgb(0, 0, 0),
            "X",
            true,
            false,
        );
        let diff = frame(&mut renderer, &mut surface);

        assert!(diff.contains("\x1b[1;1H"), "diff={:?}", diff);
        assert!(diff.contains("\x1b[38;2;0;255;0m"), "diff={:?}", diff);
        assert!(diff.contains("\x1b[7m"), "diff={:?}", diff);
        assert!(diff.contains('X'), "diff={:?}", diff);
    }

    #[test]
    fn uniform_scroll_up_emits_decstbm_and_repaints_only_bottom_row() {
        // 4-wide, 3-tall grid; terminal pane = whole grid (no sidebar), so the
        // DECSTBM scroll escape is safe to emit.
        let mut renderer = AnsiRenderer::default();
        let mut surface = CellSurface::new(4, 3, DEFAULT_FG, DEFAULT_BG);
        surface.put_text(0, 0, 4, DEFAULT_FG, DEFAULT_BG, "row");
        surface.put_text(0, 1, 4, DEFAULT_FG, DEFAULT_BG, "row");
        surface.put_text(0, 2, 4, DEFAULT_FG, DEFAULT_BG, "row");
        // Frame 1: full repaint, caches pane row hashes.
        frame(&mut renderer, &mut surface);

        // Scroll up by 1: rows 1,2 keep "row", row 0 scrolls off, bottom row
        // becomes "new". Each shifted row is re-written (content moved), so
        // mark-on-change dirties all three rows; scroll detection then turns
        // that into a DECSTBM scroll-up + bottom-row repaint.
        surface.put_text(0, 0, 4, DEFAULT_FG, DEFAULT_BG, "row");
        surface.put_text(0, 1, 4, DEFAULT_FG, DEFAULT_BG, "row");
        surface.put_text(0, 2, 4, DEFAULT_FG, DEFAULT_BG, "new");
        let diff = frame(&mut renderer, &mut surface);

        // DECSTBM: set region rows 1..3, scroll up 1, reset region.
        assert!(diff.contains("\x1b[1;3r"), "diff={:?}", diff);
        assert!(diff.contains("\x1b[3;1H\x1b[1S"), "diff={:?}", diff);
        assert!(diff.contains("\x1b[r"), "diff={:?}", diff);
        // Only the newly-exposed bottom row is repainted.
        assert!(diff.contains("new"), "diff={:?}", diff);
        // The shifted rows are NOT re-emitted as text runs (no "row" beyond
        // what the bottom repaint needs).
        let row_count = diff.matches("row").count();
        assert_eq!(
            row_count, 0,
            "shifted rows should be scrolled, not repainted; diff={:?}",
            diff
        );
    }

    #[test]
    fn scroll_repaints_dirty_chrome_outside_the_pane() {
        // Pane = rows 0..2 of a 3-row grid; row 2 is a statusbar. A scroll in
        // the pane plus a statusbar change in the same frame must emit both
        // the scroll escape AND the statusbar diff (regression: the scroll
        // path used to drop all non-pane dirty cells).
        let mut renderer = AnsiRenderer::default();
        let mut surface = CellSurface::new(4, 3, DEFAULT_FG, DEFAULT_BG);
        let pane = CellRect::new(0, 0, 4, 2);
        surface.put_text(0, 0, 4, DEFAULT_FG, DEFAULT_BG, "aa");
        surface.put_text(0, 1, 4, DEFAULT_FG, DEFAULT_BG, "bb");
        surface.put_text(0, 2, 4, DEFAULT_FG, DEFAULT_BG, "S1");
        let mut out = Vec::new();
        renderer.render(&mut out, &mut surface, pane, None).unwrap();

        // Scroll the pane up by one and change the statusbar.
        surface.put_text(0, 0, 4, DEFAULT_FG, DEFAULT_BG, "bb");
        surface.put_text(0, 1, 4, DEFAULT_FG, DEFAULT_BG, "cc");
        surface.put_text(0, 2, 4, DEFAULT_FG, DEFAULT_BG, "S2");
        let mut out = Vec::new();
        renderer.render(&mut out, &mut surface, pane, None).unwrap();
        let diff = String::from_utf8(out).unwrap();

        assert!(diff.contains("\x1b[1S"), "scroll escape; diff={diff:?}");
        assert!(diff.contains("cc"), "exposed row repainted; diff={diff:?}");
        // Only the changed statusbar cell ('1' -> '2' at row 3, col 2) is
        // re-emitted — but it must not be dropped.
        assert!(
            diff.contains("\x1b[3;2H"),
            "statusbar diff kept; diff={diff:?}"
        );
        assert!(diff.contains('2'), "statusbar diff kept; diff={diff:?}");
    }

    #[test]
    fn italic_emits_sgr_3() {
        let mut renderer = AnsiRenderer::default();
        let mut surface = CellSurface::new(4, 1, DEFAULT_FG, DEFAULT_BG);
        frame(&mut renderer, &mut surface);
        let style =
            crate::packed::PackedStyle::new(DEFAULT_FG, DEFAULT_BG, false, false, false, false)
                .with_italic(true);
        surface.put_span_packed(0, 0, 1, "x", style);
        let diff = frame(&mut renderer, &mut surface);
        assert!(diff.contains("\x1b[3m"), "diff={diff:?}");
    }

    #[test]
    fn scroll_detection_skipped_when_sidebar_shares_row_band() {
        // 6-wide, 3-tall grid; terminal pane is cols 2..6 (a sidebar occupies
        // cols 0..2 in the same rows). DECSTBM is full-width so a scroll escape
        // would smear the sidebar -> we must fall through to the dirty diff.
        let mut renderer = AnsiRenderer::default();
        let mut surface = CellSurface::new(6, 3, DEFAULT_FG, DEFAULT_BG);
        // Paint the sidebar column distinctly so we can assert it is preserved.
        for r in 0..3 {
            surface.set_cell(0, r, DEFAULT_FG, DEFAULT_BG, "S", false);
        }
        let pane = CellRect::new(2, 0, 4, 3);
        let mut out = Vec::new();
        renderer.render(&mut out, &mut surface, pane, None).unwrap();
        String::from_utf8(out).unwrap();

        // Scroll the terminal pane up by 1 (rows 1,2 keep "T", bottom -> "N").
        for r in 0..3 {
            surface.set_cell(2, r, DEFAULT_FG, DEFAULT_BG, "T", false);
        }
        let mut out = Vec::new();
        renderer.render(&mut out, &mut surface, pane, None).unwrap();
        let diff = String::from_utf8(out).unwrap();

        // No scroll escape (would smear the sidebar); the dirty diff repaints
        // the changed pane cells instead.
        assert!(
            !diff.contains("\x1b[1S"),
            "no scroll escape with sidebar; diff={:?}",
            diff
        );
        assert!(
            diff.contains("T"),
            "changed pane cells repainted; diff={:?}",
            diff
        );
        // Regression: with a sidebar (pane not full width) steady-state frames
        // must stay on the diff path — the unchanged sidebar cells must NOT be
        // re-emitted by a full repaint.
        assert!(
            !diff.contains('S'),
            "unchanged sidebar cells must not be repainted; diff={:?}",
            diff
        );
    }

    #[test]
    fn narrow_pane_steady_state_emits_only_changed_cells() {
        // Pane narrower than the surface (sidebar layout): the renderer never
        // caches row hashes, which previously forced a full repaint every
        // frame. Steady state must still diff.
        let mut renderer = AnsiRenderer::default();
        let mut surface = CellSurface::new(8, 2, DEFAULT_FG, DEFAULT_BG);
        surface.put_text(0, 0, 8, DEFAULT_FG, DEFAULT_BG, "sidebar!");
        let pane = CellRect::new(4, 0, 4, 2);
        let mut out = Vec::new();
        renderer.render(&mut out, &mut surface, pane, None).unwrap();

        // No writes at all -> second frame emits no cell content.
        let mut out = Vec::new();
        renderer.render(&mut out, &mut surface, pane, None).unwrap();
        let second = String::from_utf8(out).unwrap();
        assert!(
            !second.contains("sidebar"),
            "identical frame repainted; out={:?}",
            second
        );

        // A single-cell change repaints only that cell.
        surface.put_text(5, 1, 1, DEFAULT_FG, DEFAULT_BG, "X");
        let mut out = Vec::new();
        renderer.render(&mut out, &mut surface, pane, None).unwrap();
        let third = String::from_utf8(out).unwrap();
        assert!(third.contains('X'), "changed cell missing; out={:?}", third);
        assert!(
            !third.contains("sidebar"),
            "unchanged cells repainted; out={:?}",
            third
        );
    }
}
