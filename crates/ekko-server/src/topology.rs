//! Pure canonical tiled-pane topology and deterministic geometry.
//!
//! P2 intentionally has no wire caller; the production mutation surface is
//! consumed by P3. Keep the complete server-internal mechanism compiled now.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::HashSet;

use ekko_proto::PaneBorderStyle;

use crate::terminal_pane::PaneId;

/// Smallest terminal parser/PTY geometry accepted for a new tiled child.
pub(crate) const MIN_PANE_COLS: u16 = 2;
pub(crate) const MIN_PANE_ROWS: u16 = 1;
/// Terminal cells are approximately twice as tall as they are wide.
const CELL_ASPECT: u32 = 2;
/// Integer cuts can make areas differ by at most one row/column strip.
pub(crate) const EQUAL_AREA_ROUNDING_BOUND: u32 = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
}

impl Rect {
    fn right(self) -> u32 {
        u32::from(self.x) + u32::from(self.cols)
    }

    fn bottom(self) -> u32 {
        u32::from(self.y) + u32::from(self.rows)
    }

    fn center_x2(self) -> u32 {
        u32::from(self.x) * 2 + u32::from(self.cols)
    }

    fn center_y2(self) -> u32 {
        u32::from(self.y) * 2 + u32::from(self.rows)
    }

    fn overlaps_x(self, other: Self) -> bool {
        u32::from(self.x) < other.right() && u32::from(other.x) < self.right()
    }

    fn overlaps_y(self, other: Self) -> bool {
        u32::from(self.y) < other.bottom() && u32::from(other.y) < self.bottom()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SplitAxis {
    /// Place the new child to the right of the existing leaf.
    Horizontal,
    /// Place the new child below the existing leaf.
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Exact first-child share of a split. Keeping numerator/denominator in the
/// tree avoids geometry drift when the same topology is resolved repeatedly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SplitRatio {
    first: u16,
    total: u16,
}

impl SplitRatio {
    pub(crate) const HALF: Self = Self { first: 1, total: 2 };

    pub(crate) fn new(first: u16, total: u16) -> Option<Self> {
        (total > 0 && first > 0 && first < total).then_some(Self { first, total })
    }

    fn first_extent(self, extent: u16) -> u16 {
        ((u32::from(extent) * u32::from(self.first)) / u32::from(self.total)) as u16
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Leaf(PaneId),
    Split {
        axis: SplitAxis,
        ratio: SplitRatio,
        first: Box<Node>,
        second: Box<Node>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PaneTopology {
    root: Option<Node>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TopologyError {
    MissingLeaf,
    DuplicateLeaf,
    ZeroCanvas,
    ChildTooSmall,
}

impl PaneTopology {
    pub(crate) fn new(initial: PaneId) -> Self {
        Self {
            root: Some(Node::Leaf(initial)),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.leaves().len()
    }

    pub(crate) fn contains(&self, pane: PaneId) -> bool {
        self.root.as_ref().is_some_and(|root| contains(root, pane))
    }

    /// Canonical depth-first leaf order (first/left/top before second/right/
    /// bottom). This is the deterministic fallback for focus repair.
    pub(crate) fn leaves(&self) -> Vec<PaneId> {
        let mut leaves = Vec::new();
        if let Some(root) = &self.root {
            collect_leaves(root, &mut leaves);
        }
        leaves
    }

    pub(crate) fn first_leaf(&self) -> Option<PaneId> {
        self.root.as_ref().map(first_leaf)
    }

    /// Return a proposed split without mutating this tree. The caller can
    /// resolve/validate it and spawn the child before committing the clone.
    pub(crate) fn with_append(&self, child: PaneId) -> Result<Self, TopologyError> {
        if self.contains(child) {
            return Err(TopologyError::DuplicateLeaf);
        }
        let Some(root) = &self.root else {
            return Err(TopologyError::MissingLeaf);
        };
        Ok(Self {
            root: Some(Node::Split {
                axis: SplitAxis::Horizontal,
                ratio: SplitRatio::HALF,
                first: Box::new(root.clone()),
                second: Box::new(Node::Leaf(child)),
            }),
        })
    }

    pub(crate) fn with_split(
        &self,
        target: PaneId,
        child: PaneId,
        axis: SplitAxis,
        ratio: SplitRatio,
    ) -> Result<Self, TopologyError> {
        if self.contains(child) {
            return Err(TopologyError::DuplicateLeaf);
        }
        let Some(root) = &self.root else {
            return Err(TopologyError::MissingLeaf);
        };
        let mut proposed = root.clone();
        if !split_leaf(&mut proposed, target, child, axis, ratio) {
            return Err(TopologyError::MissingLeaf);
        }
        Ok(Self {
            root: Some(proposed),
        })
    }

    /// Remove one leaf and promote its sibling into the parent's position.
    /// The node representation cannot retain unary splits.
    pub(crate) fn remove(&mut self, pane: PaneId) -> bool {
        let Some(root) = self.root.take() else {
            return false;
        };
        let (root, removed) = remove_leaf(root, pane);
        self.root = root;
        removed
    }

    /// Resolve every leaf exactly once in canonical traversal order. Each
    /// leaf's rect is its *content* area: `style` reserves separator cells
    /// between sibling subtrees (its gap) and around the whole canvas (its
    /// margin), so reserved cells belong to no pane.
    pub(crate) fn resolve(
        &self,
        canvas: Rect,
        style: PaneBorderStyle,
    ) -> Result<Vec<(PaneId, Rect)>, TopologyError> {
        if canvas.cols == 0 || canvas.rows == 0 {
            return Err(TopologyError::ZeroCanvas);
        }
        let margin = style.margin();
        // A canvas the margin fully consumes is too small, not zero.
        let Some(inner_cols) = canvas.cols.checked_sub(margin * 2) else {
            return Err(TopologyError::ChildTooSmall);
        };
        let Some(inner_rows) = canvas.rows.checked_sub(margin * 2) else {
            return Err(TopologyError::ChildTooSmall);
        };
        if inner_cols == 0 || inner_rows == 0 {
            return Err(if margin == 0 {
                TopologyError::ZeroCanvas
            } else {
                TopologyError::ChildTooSmall
            });
        }
        let inner = Rect {
            x: canvas.x + margin,
            y: canvas.y + margin,
            cols: inner_cols,
            rows: inner_rows,
        };
        let mut resolved = Vec::with_capacity(self.len());
        if let Some(root) = &self.root {
            resolve_node(root, inner, style.gap(), &mut resolved)?;
        }
        debug_assert_eq!(
            resolved
                .iter()
                .map(|(id, _)| *id)
                .collect::<HashSet<_>>()
                .len(),
            resolved.len()
        );
        Ok(resolved)
    }

    /// Resolve leaves by recursive proportional halving, independent of the
    /// historical BSP split directions. Separator cells are removed at every
    /// cut exactly as in `resolve_node`.
    pub(crate) fn resolve_equal(
        &self,
        canvas: Rect,
        style: PaneBorderStyle,
    ) -> Result<Vec<(PaneId, Rect)>, TopologyError> {
        if canvas.cols == 0 || canvas.rows == 0 {
            return Err(TopologyError::ZeroCanvas);
        }
        let margin = style.margin();
        let inner = Rect {
            x: canvas.x + margin,
            y: canvas.y + margin,
            cols: canvas
                .cols
                .checked_sub(margin * 2)
                .ok_or(TopologyError::ChildTooSmall)?,
            rows: canvas
                .rows
                .checked_sub(margin * 2)
                .ok_or(TopologyError::ChildTooSmall)?,
        };
        if inner.cols == 0 || inner.rows == 0 {
            return Err(TopologyError::ChildTooSmall);
        }
        let ids = self.leaves();
        let mut out = Vec::with_capacity(ids.len());
        resolve_equal_nodes(&ids, inner, style.gap(), &mut out)?;
        Ok(out)
    }

    pub(crate) fn resolve_viable(
        &self,
        canvas: Rect,
        style: PaneBorderStyle,
    ) -> Result<Vec<(PaneId, Rect)>, TopologyError> {
        let resolved = self.resolve(canvas, style)?;
        if resolved
            .iter()
            .any(|(_, rect)| rect.cols < MIN_PANE_COLS || rect.rows < MIN_PANE_ROWS)
        {
            return Err(TopologyError::ChildTooSmall);
        }
        Ok(resolved)
    }

    pub(crate) fn neighbor(
        &self,
        pane: PaneId,
        direction: Direction,
        canvas: Rect,
        style: PaneBorderStyle,
    ) -> Option<PaneId> {
        let resolved = self.resolve(canvas, style).ok()?;
        neighbor_in(&resolved, pane, direction)
    }

    #[cfg(test)]
    fn split_count(&self) -> usize {
        fn count(node: &Node) -> usize {
            match node {
                Node::Leaf(_) => 0,
                Node::Split { first, second, .. } => 1 + count(first) + count(second),
            }
        }
        self.root.as_ref().map_or(0, count)
    }
}

fn contains(node: &Node, pane: PaneId) -> bool {
    match node {
        Node::Leaf(id) => *id == pane,
        Node::Split { first, second, .. } => contains(first, pane) || contains(second, pane),
    }
}

fn collect_leaves(node: &Node, leaves: &mut Vec<PaneId>) {
    match node {
        Node::Leaf(id) => leaves.push(*id),
        Node::Split { first, second, .. } => {
            collect_leaves(first, leaves);
            collect_leaves(second, leaves);
        }
    }
}

fn first_leaf(node: &Node) -> PaneId {
    match node {
        Node::Leaf(id) => *id,
        Node::Split { first, .. } => first_leaf(first),
    }
}

fn split_leaf(
    node: &mut Node,
    target: PaneId,
    child: PaneId,
    axis: SplitAxis,
    ratio: SplitRatio,
) -> bool {
    match node {
        Node::Leaf(id) if *id == target => {
            *node = Node::Split {
                axis,
                ratio,
                first: Box::new(Node::Leaf(target)),
                second: Box::new(Node::Leaf(child)),
            };
            true
        }
        Node::Leaf(_) => false,
        Node::Split { first, second, .. } => {
            split_leaf(first, target, child, axis, ratio)
                || split_leaf(second, target, child, axis, ratio)
        }
    }
}

fn remove_leaf(node: Node, pane: PaneId) -> (Option<Node>, bool) {
    match node {
        Node::Leaf(id) => {
            if id == pane {
                (None, true)
            } else {
                (Some(Node::Leaf(id)), false)
            }
        }
        Node::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let (new_first, removed) = remove_leaf(*first, pane);
            if removed {
                return (
                    match new_first {
                        Some(first) => Some(Node::Split {
                            axis,
                            ratio,
                            first: Box::new(first),
                            second,
                        }),
                        None => Some(*second),
                    },
                    true,
                );
            }
            let first = new_first.expect("unchanged subtree remains present");
            let (new_second, removed) = remove_leaf(*second, pane);
            (
                match new_second {
                    Some(second) => Some(Node::Split {
                        axis,
                        ratio,
                        first: Box::new(first),
                        second: Box::new(second),
                    }),
                    None => Some(first),
                },
                removed,
            )
        }
    }
}

fn resolve_equal_nodes(
    ids: &[PaneId],
    rect: Rect,
    gap: u16,
    out: &mut Vec<(PaneId, Rect)>,
) -> Result<(), TopologyError> {
    let n = ids.len();
    if n == 1 {
        out.push((ids[0], rect));
        return Ok(());
    }
    let prime_large = n >= 5 && (2..n).all(|d| !n.is_multiple_of(d));
    if !prime_large {
        let mut pairs: Vec<(usize, usize)> = (1..=n)
            .filter(|c| n.is_multiple_of(*c))
            .map(|c| (c, n / c))
            .collect();
        pairs.sort_by(|&(c1, r1), &(c2, r2)| {
            let w1 = u32::from(rect.cols) * r1 as u32;
            let h1 = CELL_ASPECT * u32::from(rect.rows) * c1 as u32;
            let w2 = u32::from(rect.cols) * r2 as u32;
            let h2 = CELL_ASPECT * u32::from(rect.rows) * c2 as u32;
            let (max1, min1) = (w1.max(h1), w1.min(h1));
            let (max2, min2) = (w2.max(h2), w2.min(h2));
            (max1 * min2).cmp(&(max2 * min1)).then_with(|| c1.cmp(&c2))
        });
        for (c, r) in pairs {
            let wc = match rect.cols.checked_sub(gap * (c - 1) as u16) {
                Some(value) => value,
                None => continue,
            };
            let wr = match rect.rows.checked_sub(gap * (r - 1) as u16) {
                Some(value) => value,
                None => continue,
            };
            if wc / (c as u16) < MIN_PANE_COLS || wr / (r as u16) < MIN_PANE_ROWS {
                continue;
            }
            for i in 0..c {
                for j in 0..r {
                    let x0 = u32::from(wc) * i as u32 / c as u32;
                    let x1 = u32::from(wc) * (i + 1) as u32 / c as u32;
                    let y0 = u32::from(wr) * j as u32 / r as u32;
                    let y1 = u32::from(wr) * (j + 1) as u32 / r as u32;
                    out.push((
                        ids[i * r + j],
                        Rect {
                            x: rect.x + x0 as u16 + gap * i as u16,
                            y: rect.y + y0 as u16 + gap * j as u16,
                            cols: (x1 - x0) as u16,
                            rows: (y1 - y0) as u16,
                        },
                    ));
                }
            }
            return Ok(());
        }
        // No divisor pair fits this canvas; fall through to the recursive
        // halving path below, which handles arbitrary n and small canvases.
    }
    let a = n.div_ceil(2);
    let extent = if u32::from(rect.cols) > u32::from(rect.rows) * CELL_ASPECT {
        rect.cols
    } else {
        rect.rows
    };
    let working = extent
        .checked_sub(gap)
        .ok_or(TopologyError::ChildTooSmall)?;
    let first_extent = ((u32::from(working) * a as u32) / n as u32) as u16;
    let second_extent = working - first_extent;
    let horizontal = extent == rect.cols;
    if (horizontal && (first_extent < MIN_PANE_COLS || second_extent < MIN_PANE_COLS))
        || (!horizontal && (first_extent < MIN_PANE_ROWS || second_extent < MIN_PANE_ROWS))
    {
        return Err(TopologyError::ChildTooSmall);
    }
    let (first, second) = if horizontal {
        (
            Rect {
                cols: first_extent,
                ..rect
            },
            Rect {
                x: rect.x + first_extent + gap,
                cols: second_extent,
                ..rect
            },
        )
    } else {
        (
            Rect {
                rows: first_extent,
                ..rect
            },
            Rect {
                y: rect.y + first_extent + gap,
                rows: second_extent,
                ..rect
            },
        )
    };
    resolve_equal_nodes(&ids[..a], first, gap, out)?;
    resolve_equal_nodes(&ids[a..], second, gap, out)
}

fn resolve_node(
    node: &Node,
    rect: Rect,
    gap: u16,
    resolved: &mut Vec<(PaneId, Rect)>,
) -> Result<(), TopologyError> {
    match node {
        Node::Leaf(id) => resolved.push((*id, rect)),
        Node::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let extent = match axis {
                SplitAxis::Horizontal => rect.cols,
                SplitAxis::Vertical => rect.rows,
            };
            // The gap cells belong to the separator, not to either child.
            let working = extent
                .checked_sub(gap)
                .ok_or(TopologyError::ChildTooSmall)?;
            let first_extent = ratio.first_extent(working);
            let second_extent = working - first_extent;
            if first_extent == 0 || second_extent == 0 {
                return Err(TopologyError::ChildTooSmall);
            }
            let (first_rect, second_rect) = match axis {
                SplitAxis::Horizontal => (
                    Rect {
                        cols: first_extent,
                        ..rect
                    },
                    Rect {
                        x: rect.x + first_extent + gap,
                        cols: second_extent,
                        ..rect
                    },
                ),
                SplitAxis::Vertical => (
                    Rect {
                        rows: first_extent,
                        ..rect
                    },
                    Rect {
                        y: rect.y + first_extent + gap,
                        rows: second_extent,
                        ..rect
                    },
                ),
            };
            resolve_node(first, first_rect, gap, resolved)?;
            resolve_node(second, second_rect, gap, resolved)?;
        }
    }
    Ok(())
}

/// `(overlap penalty, primary gap, perpendicular center distance)`.
pub(crate) fn neighbor_in(
    resolved: &[(PaneId, Rect)],
    pane: PaneId,
    direction: Direction,
) -> Option<PaneId> {
    let source = resolved.iter().find(|(id, _)| *id == pane)?.1;
    resolved
        .iter()
        .filter(|(id, _)| *id != pane)
        .filter_map(|(id, rect)| neighbor_rank(source, *rect, direction).map(|rank| (rank, *id)))
        .min_by_key(|(rank, id)| (*rank, id.0))
        .map(|(_, id)| id)
}

fn neighbor_rank(source: Rect, candidate: Rect, direction: Direction) -> Option<(u8, u32, u32)> {
    match direction {
        Direction::Left if candidate.right() <= u32::from(source.x) => Some((
            u8::from(!source.overlaps_y(candidate)),
            u32::from(source.x) - candidate.right(),
            source.center_y2().abs_diff(candidate.center_y2()),
        )),
        Direction::Right if u32::from(candidate.x) >= source.right() => Some((
            u8::from(!source.overlaps_y(candidate)),
            u32::from(candidate.x) - source.right(),
            source.center_y2().abs_diff(candidate.center_y2()),
        )),
        Direction::Up if candidate.bottom() <= u32::from(source.y) => Some((
            u8::from(!source.overlaps_x(candidate)),
            u32::from(source.y) - candidate.bottom(),
            source.center_x2().abs_diff(candidate.center_x2()),
        )),
        Direction::Down if u32::from(candidate.y) >= source.bottom() => Some((
            u8::from(!source.overlaps_x(candidate)),
            u32::from(candidate.y) - source.bottom(),
            source.center_x2().abs_diff(candidate.center_x2()),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: u64) -> PaneId {
        PaneId(value)
    }

    fn canvas(cols: u16, rows: u16) -> Rect {
        Rect {
            x: 0,
            y: 0,
            cols,
            rows,
        }
    }

    fn assert_geometry(topology: &PaneTopology, bounds: Rect) {
        let first = topology
            .resolve_equal(bounds, PaneBorderStyle::None)
            .unwrap();
        let second = topology
            .resolve_equal(bounds, PaneBorderStyle::None)
            .unwrap();
        assert_eq!(first, second, "resolution must be deterministic");
        assert_eq!(first.len(), topology.len());
        assert_eq!(
            first
                .iter()
                .map(|(pane, _)| *pane)
                .collect::<HashSet<_>>()
                .len(),
            first.len(),
            "every leaf appears exactly once"
        );
        for (index, (_, rect)) in first.iter().enumerate() {
            assert!(rect.cols >= MIN_PANE_COLS && rect.rows >= MIN_PANE_ROWS);
            assert!(rect.right() <= bounds.right() && rect.bottom() <= bounds.bottom());
            for (_, other) in &first[index + 1..] {
                assert!(
                    !rect.overlaps_x(*other) || !rect.overlaps_y(*other),
                    "leaf rectangles overlap: {rect:?} and {other:?}"
                );
            }
        }
    }

    #[test]
    fn exhaustive_small_split_trees_are_deterministic_bounded_and_disjoint() {
        for cols in 4..=18 {
            for rows in 2..=10 {
                for first_axis in [SplitAxis::Horizontal, SplitAxis::Vertical] {
                    for second_axis in [SplitAxis::Horizontal, SplitAxis::Vertical] {
                        for ratio in [
                            SplitRatio::HALF,
                            SplitRatio::new(1, 3).unwrap(),
                            SplitRatio::new(2, 3).unwrap(),
                        ] {
                            let topology = PaneTopology::new(id(1));
                            let Ok(topology) = topology
                                .with_split(id(1), id(2), first_axis, ratio)
                                .and_then(|tree| tree.with_split(id(1), id(3), second_axis, ratio))
                            else {
                                unreachable!();
                            };
                            if topology
                                .resolve_viable(canvas(cols, rows), PaneBorderStyle::None)
                                .is_ok()
                            {
                                assert_geometry(&topology, canvas(cols, rows));
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn equal_layout_worked_examples_and_tile_invariants() {
        for n in [1usize, 2, 3, 4, 5, 6, 7, 8, 9, 12] {
            let mut topology = PaneTopology::new(id(1));
            for value in 2..=n as u64 {
                topology = topology.with_append(id(value)).unwrap();
            }
            let geometry = topology
                .resolve_equal(canvas(80, 24), PaneBorderStyle::None)
                .unwrap();
            assert_eq!(geometry.len(), n);
            let areas: Vec<u32> = geometry
                .iter()
                .map(|(_, r)| u32::from(r.cols) * u32::from(r.rows))
                .collect();
            let min = *areas.iter().min().unwrap();
            let max = *areas.iter().max().unwrap();
            assert!(max - min <= EQUAL_AREA_ROUNDING_BOUND);
            assert_geometry(&topology, canvas(80, 24));
            for (i, (_, a)) in geometry.iter().enumerate() {
                for (_, b) in &geometry[i + 1..] {
                    assert!(!a.overlaps_x(*b) || !a.overlaps_y(*b));
                }
            }
            if n == 3 {
                assert_eq!(
                    geometry
                        .iter()
                        .map(|(_, r)| (r.x, r.y, r.cols, r.rows))
                        .collect::<Vec<_>>(),
                    vec![(0, 0, 26, 24), (26, 0, 27, 24), (53, 0, 27, 24)]
                );
            }
            if n == 4 {
                assert_eq!(
                    geometry
                        .iter()
                        .map(|(_, r)| (r.x, r.y, r.cols, r.rows))
                        .collect::<Vec<_>>(),
                    vec![
                        (0, 0, 40, 12),
                        (0, 12, 40, 12),
                        (40, 0, 40, 12),
                        (40, 12, 40, 12)
                    ]
                );
            }
        }
    }

    #[test]
    fn equal_layout_directional_neighbors_use_geometry() {
        let mut topology = PaneTopology::new(id(1));
        for value in 2..=4 {
            topology = topology.with_append(id(value)).unwrap();
        }
        let geometry = topology
            .resolve_equal(canvas(80, 24), PaneBorderStyle::None)
            .unwrap();
        assert_eq!(neighbor_in(&geometry, id(1), Direction::Right), Some(id(3)));
        assert_eq!(neighbor_in(&geometry, id(1), Direction::Down), Some(id(2)));
        assert_eq!(neighbor_in(&geometry, id(2), Direction::Left), None);
        assert_eq!(geometry.len(), 4);
    }

    #[test]
    fn split_ratios_and_right_down_order_are_explicit() {
        let topology = PaneTopology::new(id(1))
            .with_split(
                id(1),
                id(2),
                SplitAxis::Horizontal,
                SplitRatio::new(1, 3).unwrap(),
            )
            .unwrap()
            .with_split(id(2), id(3), SplitAxis::Vertical, SplitRatio::HALF)
            .unwrap();
        assert_eq!(
            topology
                .resolve(canvas(12, 8), PaneBorderStyle::None)
                .unwrap(),
            vec![
                (
                    id(1),
                    Rect {
                        x: 0,
                        y: 0,
                        cols: 4,
                        rows: 8
                    }
                ),
                (
                    id(2),
                    Rect {
                        x: 4,
                        y: 0,
                        cols: 8,
                        rows: 4
                    }
                ),
                (
                    id(3),
                    Rect {
                        x: 4,
                        y: 4,
                        cols: 8,
                        rows: 4
                    }
                ),
            ]
        );
    }

    #[test]
    fn invalid_split_is_a_pure_rejection() {
        let topology = PaneTopology::new(id(1));
        let proposed = topology
            .with_split(id(1), id(2), SplitAxis::Horizontal, SplitRatio::HALF)
            .unwrap();
        assert_eq!(
            proposed.resolve_viable(canvas(3, 10), PaneBorderStyle::None),
            Err(TopologyError::ChildTooSmall)
        );
        assert_eq!(topology.leaves(), vec![id(1)]);
        assert_eq!(
            topology
                .resolve(canvas(3, 10), PaneBorderStyle::None)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn removing_every_leaf_position_promotes_siblings_without_unary_nodes() {
        let original = PaneTopology::new(id(1))
            .with_split(id(1), id(2), SplitAxis::Horizontal, SplitRatio::HALF)
            .unwrap()
            .with_split(id(1), id(3), SplitAxis::Vertical, SplitRatio::HALF)
            .unwrap()
            .with_split(id(2), id(4), SplitAxis::Vertical, SplitRatio::HALF)
            .unwrap();
        for removed in original.leaves() {
            let mut topology = original.clone();
            assert!(topology.remove(removed));
            assert!(!topology.contains(removed));
            assert_eq!(topology.len(), 3);
            assert_eq!(topology.split_count(), topology.len() - 1);
            assert_geometry(&topology, canvas(80, 24));
        }
    }

    #[test]
    fn directional_neighbors_follow_resolved_geometry() {
        let topology = PaneTopology::new(id(1))
            .with_split(id(1), id(2), SplitAxis::Horizontal, SplitRatio::HALF)
            .unwrap()
            .with_split(id(1), id(3), SplitAxis::Vertical, SplitRatio::HALF)
            .unwrap();
        let bounds = canvas(80, 24);
        assert_eq!(
            topology.neighbor(id(1), Direction::Right, bounds, PaneBorderStyle::None),
            Some(id(2))
        );
        assert_eq!(
            topology.neighbor(id(1), Direction::Down, bounds, PaneBorderStyle::None),
            Some(id(3))
        );
        assert_eq!(
            topology.neighbor(id(2), Direction::Left, bounds, PaneBorderStyle::None),
            Some(id(1))
        );
        assert_eq!(
            topology.neighbor(id(3), Direction::Up, bounds, PaneBorderStyle::None),
            Some(id(1))
        );
        assert_eq!(
            topology.neighbor(id(2), Direction::Right, bounds, PaneBorderStyle::None),
            None
        );
    }

    #[test]
    fn compact_style_reserves_one_separator_cell_per_split() {
        let topology = PaneTopology::new(id(1))
            .with_split(id(1), id(2), SplitAxis::Horizontal, SplitRatio::HALF)
            .unwrap()
            .with_split(id(2), id(3), SplitAxis::Vertical, SplitRatio::HALF)
            .unwrap();
        let resolved = topology
            .resolve(canvas(12, 8), PaneBorderStyle::Compact)
            .unwrap();
        assert_eq!(
            resolved,
            vec![
                (
                    id(1),
                    Rect {
                        x: 0,
                        y: 0,
                        cols: 5,
                        rows: 8
                    }
                ),
                (
                    id(2),
                    Rect {
                        x: 6,
                        y: 0,
                        cols: 6,
                        rows: 3
                    }
                ),
                (
                    id(3),
                    Rect {
                        x: 6,
                        y: 4,
                        cols: 6,
                        rows: 4
                    }
                ),
            ]
        );
        // Separator cells (col 5; row 3 right of the split) belong to no pane.
        for (_, rect) in &resolved {
            assert!(!rect.overlaps_x(Rect {
                x: 5,
                y: 0,
                cols: 1,
                rows: 8
            }));
        }
        for (_, rect) in &resolved[1..] {
            assert!(!rect.overlaps_y(Rect {
                x: 6,
                y: 3,
                cols: 6,
                rows: 1
            }));
        }
    }

    #[test]
    fn frame_style_reserves_a_margin_and_two_cell_gaps() {
        let topology = PaneTopology::new(id(1))
            .with_split(id(1), id(2), SplitAxis::Horizontal, SplitRatio::HALF)
            .unwrap();
        let resolved = topology
            .resolve(canvas(12, 8), PaneBorderStyle::Frame)
            .unwrap();
        // Canvas inset by the 1-cell margin; the 2-cell inter-pane gap is
        // each pane's facing frame column.
        assert_eq!(
            resolved,
            vec![
                (
                    id(1),
                    Rect {
                        x: 1,
                        y: 1,
                        cols: 4,
                        rows: 6
                    }
                ),
                (
                    id(2),
                    Rect {
                        x: 7,
                        y: 1,
                        cols: 4,
                        rows: 6
                    }
                ),
            ]
        );
        // A lone pane still pays only the margin.
        let single = PaneTopology::new(id(1))
            .resolve(canvas(12, 8), PaneBorderStyle::Frame)
            .unwrap();
        assert_eq!(
            single,
            vec![(
                id(1),
                Rect {
                    x: 1,
                    y: 1,
                    cols: 10,
                    rows: 6
                }
            )]
        );
    }

    #[test]
    fn separator_cells_shrink_viability() {
        let topology = PaneTopology::new(id(1))
            .with_split(id(1), id(2), SplitAxis::Horizontal, SplitRatio::HALF)
            .unwrap();
        // 5 cols edge-to-edge fits 2+2; compact needs 2+1+2 = 5 too,
        // but frame needs 2+2+2 plus margins = 8.
        assert!(
            topology
                .resolve_viable(canvas(5, 4), PaneBorderStyle::Compact)
                .is_ok()
        );
        assert_eq!(
            topology.resolve_viable(canvas(5, 4), PaneBorderStyle::Frame),
            Err(TopologyError::ChildTooSmall)
        );
        assert!(
            topology
                .resolve_viable(canvas(8, 4), PaneBorderStyle::Frame)
                .is_ok()
        );
        // Even a lone framed pane needs the margin plus its minimum width.
        let single = PaneTopology::new(id(1));
        assert_eq!(
            single.resolve_viable(canvas(2, 4), PaneBorderStyle::Frame),
            Err(TopologyError::ChildTooSmall)
        );
    }

    #[test]
    fn ratios_reject_zero_and_edge_shares() {
        assert_eq!(SplitRatio::new(0, 2), None);
        assert_eq!(SplitRatio::new(1, 0), None);
        assert_eq!(SplitRatio::new(2, 2), None);
        assert!(SplitRatio::new(1, 2).is_some());
    }
}
