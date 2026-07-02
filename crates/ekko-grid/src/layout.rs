//! Cell rectangle primitive used by the compositor and the diff renderer.
//! Frame *layout* (which chrome regions exist and how they dock) is decided
//! by the extension system — see `ekko_ext::resolve_layout`.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellRect {
    pub col: i32,
    pub row: i32,
    pub cols: i32,
    pub rows: i32,
}

impl CellRect {
    pub fn new(col: i32, row: i32, cols: i32, rows: i32) -> Self {
        Self {
            col,
            row,
            cols,
            rows,
        }
    }

    pub fn inset_edges(self, left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            col: self.col + left,
            row: self.row + top,
            cols: (self.cols - left - right).max(0),
            rows: (self.rows - top - bottom).max(0),
        }
    }

    pub fn contains_cell(self, col: i32, row: i32) -> bool {
        col >= self.col
            && row >= self.row
            && col < self.col + self.cols
            && row < self.row + self.rows
    }
}
