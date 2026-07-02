//! Spatial damage tracking (Priority 2 of the renderer optimization plan).
//! Drawing primitives mark a `DirtyRegion`; the diff loop can then skip clean
//! regions entirely, dropping complexity from O(Cols * Rows) to O(DirtyCells).

/// A rectangular region of the cell grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = (self.x + self.width).min(other.x + other.width);
        let y2 = (self.y + self.height).min(other.y + other.height);

        if x2 > x1 && y2 > y1 {
            Some(Rect {
                x: x1,
                y: y1,
                width: x2 - x1,
                height: y2 - y1,
            })
        } else {
            None
        }
    }
}

/// A collection of dirty rectangles.
#[derive(Debug, Default, Clone)]
pub struct DirtyRegion {
    rects: Vec<Rect>,
}

impl DirtyRegion {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn clear(&mut self) {
        self.rects.clear();
    }

    pub fn add(&mut self, rect: Rect) {
        if rect.width <= 0 || rect.height <= 0 {
            return;
        }
        // Simple, fast union: coalesce with the last rect if they overlap or
        // are directly adjacent on the same row band. This keeps the rect
        // count bounded without a full sweep-line merge.
        if let Some(last) = self.rects.last_mut() {
            // Same single row, horizontally adjacent/overlapping → extend.
            if last.y == rect.y
                && last.height == rect.height
                && rect.x <= last.x + last.width
                && last.x <= rect.x + rect.width
            {
                let new_right = (last.x + last.width).max(rect.x + rect.width);
                last.x = last.x.min(rect.x);
                last.width = new_right - last.x;
                return;
            }
            // Same columns, vertically adjacent/overlapping → extend.
            if last.x == rect.x
                && last.width == rect.width
                && rect.y <= last.y + last.height
                && last.y <= rect.y + rect.height
            {
                let new_bottom = (last.y + last.height).max(rect.y + rect.height);
                last.y = last.y.min(rect.y);
                last.height = new_bottom - last.y;
                return;
            }
        }
        self.rects.push(rect);
    }

    /// Mark the entire region dirty by pushing a full-screen rect.
    pub fn add_full(&mut self, cols: i32, rows: i32) {
        self.rects.clear();
        self.rects.push(Rect::new(0, 0, cols, rows));
    }

    pub fn add_rect(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.add(Rect::new(x, y, width, height));
    }

    /// Iterate flat row-major cell indices covered by the dirty rects,
    /// clamped to `(cols, rows)`.
    pub fn iter_indices(&self, cols: i32, rows: i32) -> impl Iterator<Item = usize> + '_ {
        let cols_us = cols as usize;
        let bounds = Rect::new(0, 0, cols, rows);
        self.rects
            .iter()
            .filter_map(move |r| r.intersection(&bounds))
            .flat_map(move |r| {
                let start_x = r.x as usize;
                let start_y = r.y as usize;
                let end_x = (r.x + r.width) as usize;
                let end_y = (r.y + r.height) as usize;
                (start_y..end_y).flat_map(move |y| (start_x..end_x).map(move |x| y * cols_us + x))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_coalesces_horizontal_strips() {
        let mut d = DirtyRegion::new();
        d.add_rect(0, 0, 1, 1);
        d.add_rect(1, 0, 1, 1);
        d.add_rect(2, 0, 1, 1);
        assert_eq!(d.rects.len(), 1);
        assert_eq!(d.rects[0], Rect::new(0, 0, 3, 1));
    }

    #[test]
    fn iter_indices_covers_dirty_cells() {
        let mut d = DirtyRegion::new();
        d.add_rect(1, 1, 2, 1);
        let idx: Vec<usize> = d.iter_indices(4, 3).collect();
        // row 1, cols 1 and 2 → 1*4+1=5, 1*4+2=6
        assert_eq!(idx, vec![5, 6]);
    }

    #[test]
    #[allow(unused)]
    fn iter_indices_clamps_to_bounds() {
        let mut d = DirtyRegion::new();
        d.add_rect(-2, -2, 10, 10);
        let idx: Vec<usize> = d.iter_indices(3, 2).collect();
        // clamped to (0,0)-(3,2) → 0..6
        assert_eq!(idx, vec![0, 1, 2, 3, 4, 5]);
    }
}
