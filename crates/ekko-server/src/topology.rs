//! Pure canonical tiled-pane topology and deterministic geometry.
//!
//! P2 intentionally has no wire caller; the production mutation surface is
//! consumed by P3. Keep the complete server-internal mechanism compiled now.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::HashSet;

use crate::terminal_pane::PaneId;

/// Smallest terminal parser/PTY geometry accepted for a new tiled child.
pub(crate) const MIN_PANE_COLS: u16 = 2;
pub(crate) const MIN_PANE_ROWS: u16 = 1;

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

    /// Resolve every leaf exactly once in canonical traversal order.
    pub(crate) fn resolve(&self, canvas: Rect) -> Result<Vec<(PaneId, Rect)>, TopologyError> {
        if canvas.cols == 0 || canvas.rows == 0 {
            return Err(TopologyError::ZeroCanvas);
        }
        let mut resolved = Vec::with_capacity(self.len());
        if let Some(root) = &self.root {
            resolve_node(root, canvas, &mut resolved)?;
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

    pub(crate) fn resolve_viable(
        &self,
        canvas: Rect,
    ) -> Result<Vec<(PaneId, Rect)>, TopologyError> {
        let resolved = self.resolve(canvas)?;
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
    ) -> Option<PaneId> {
        let resolved = self.resolve(canvas).ok()?;
        let source = resolved.iter().find(|(id, _)| *id == pane)?.1;
        resolved
            .into_iter()
            .filter(|(id, _)| *id != pane)
            .filter_map(|(id, rect)| neighbor_rank(source, rect, direction).map(|rank| (rank, id)))
            .min_by_key(|(rank, id)| (*rank, id.0))
            .map(|(_, id)| id)
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

fn resolve_node(
    node: &Node,
    rect: Rect,
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
            let first_extent = ratio.first_extent(match axis {
                SplitAxis::Horizontal => rect.cols,
                SplitAxis::Vertical => rect.rows,
            });
            let second_extent = match axis {
                SplitAxis::Horizontal => rect.cols - first_extent,
                SplitAxis::Vertical => rect.rows - first_extent,
            };
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
                        x: rect.x + first_extent,
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
                        y: rect.y + first_extent,
                        rows: second_extent,
                        ..rect
                    },
                ),
            };
            resolve_node(first, first_rect, resolved)?;
            resolve_node(second, second_rect, resolved)?;
        }
    }
    Ok(())
}

/// `(overlap penalty, primary gap, perpendicular center distance)`.
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
        let first = topology.resolve_viable(bounds).unwrap();
        let second = topology.resolve_viable(bounds).unwrap();
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
                            if topology.resolve_viable(canvas(cols, rows)).is_ok() {
                                assert_geometry(&topology, canvas(cols, rows));
                            }
                        }
                    }
                }
            }
        }
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
            topology.resolve(canvas(12, 8)).unwrap(),
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
            proposed.resolve_viable(canvas(3, 10)),
            Err(TopologyError::ChildTooSmall)
        );
        assert_eq!(topology.leaves(), vec![id(1)]);
        assert_eq!(topology.resolve(canvas(3, 10)).unwrap().len(), 1);
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
            topology.neighbor(id(1), Direction::Right, bounds),
            Some(id(2))
        );
        assert_eq!(
            topology.neighbor(id(1), Direction::Down, bounds),
            Some(id(3))
        );
        assert_eq!(
            topology.neighbor(id(2), Direction::Left, bounds),
            Some(id(1))
        );
        assert_eq!(topology.neighbor(id(3), Direction::Up, bounds), Some(id(1)));
        assert_eq!(topology.neighbor(id(2), Direction::Right, bounds), None);
    }

    #[test]
    fn ratios_reject_zero_and_edge_shares() {
        assert_eq!(SplitRatio::new(0, 2), None);
        assert_eq!(SplitRatio::new(1, 0), None);
        assert_eq!(SplitRatio::new(2, 2), None);
        assert!(SplitRatio::new(1, 2).is_some());
    }
}
