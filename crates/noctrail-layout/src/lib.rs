//! Workspace and pane layout boundary for Noctrail.

use std::collections::HashMap;

use noctrail_runtime::PaneId;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkspaceId(pub u8);

impl WorkspaceId {
    pub const MIN: u8 = 1;
    pub const MAX: u8 = 9;

    pub const fn new(raw: u8) -> Self {
        Self(raw)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkspaceError {
    #[error("workspace id {0} is outside the supported 1..9 range")]
    InvalidWorkspaceId(u8),
}

impl TryFrom<u8> for WorkspaceId {
    type Error = WorkspaceError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if (Self::MIN..=Self::MAX).contains(&value) {
            Ok(Self(value))
        } else {
            Err(WorkspaceError::InvalidWorkspaceId(value))
        }
    }
}

pub const DEFAULT_WORKSPACE_ID: WorkspaceId = WorkspaceId(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDirection {
    Left,
    Right,
    Up,
    Down,
}

impl SplitAxis {
    fn from_rect(rect: LayoutRect) -> Self {
        if rect.width >= rect.height {
            Self::Vertical
        } else {
            Self::Horizontal
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl LayoutRect {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn area(self) -> u32 {
        u32::from(self.width) * u32::from(self.height)
    }

    fn x_end(self) -> u16 {
        self.x.saturating_add(self.width)
    }

    fn y_end(self) -> u16 {
        self.y.saturating_add(self.height)
    }

    fn center_x(self) -> i32 {
        i32::from(self.x) + i32::from(self.width) / 2
    }

    fn center_y(self) -> i32 {
        i32::from(self.y) + i32::from(self.height) / 2
    }

    fn split_vertical(self, left_width: u16) -> (Self, Self) {
        let left_width = clamp_split_dimension(self.width, left_width);
        let right_width = self.width.saturating_sub(left_width);

        (
            Self::new(self.x, self.y, left_width, self.height),
            Self::new(
                self.x.saturating_add(left_width),
                self.y,
                right_width,
                self.height,
            ),
        )
    }

    fn split_horizontal(self, top_height: u16) -> (Self, Self) {
        let top_height = clamp_split_dimension(self.height, top_height);
        let bottom_height = self.height.saturating_sub(top_height);

        (
            Self::new(self.x, self.y, self.width, top_height),
            Self::new(
                self.x,
                self.y.saturating_add(top_height),
                self.width,
                bottom_height,
            ),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneLayout {
    pub pane_id: PaneId,
    pub rect: LayoutRect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutNode {
    Leaf {
        pane_id: PaneId,
    },
    Split {
        axis: SplitAxis,
        ratio: u16,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

impl LayoutNode {
    fn leaf(pane_id: PaneId) -> Self {
        Self::Leaf { pane_id }
    }

    fn contains(&self, pane_id: PaneId) -> bool {
        match self {
            Self::Leaf { pane_id: current } => *current == pane_id,
            Self::Split { first, second, .. } => {
                first.contains(pane_id) || second.contains(pane_id)
            }
        }
    }

    fn pane_count(&self) -> usize {
        match self {
            Self::Leaf { .. } => 1,
            Self::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }

    fn first_leaf(&self) -> PaneId {
        match self {
            Self::Leaf { pane_id } => *pane_id,
            Self::Split { first, .. } => first.first_leaf(),
        }
    }

    fn find_rect(&self, target: PaneId, rect: LayoutRect) -> Option<LayoutRect> {
        match self {
            Self::Leaf { pane_id } if *pane_id == target => Some(rect),
            Self::Leaf { .. } => None,
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (first_rect, second_rect) = match axis {
                    SplitAxis::Vertical => {
                        let split = split_dimension(rect.width, *ratio);
                        rect.split_vertical(split)
                    }
                    SplitAxis::Horizontal => {
                        let split = split_dimension(rect.height, *ratio);
                        rect.split_horizontal(split)
                    }
                };

                first
                    .find_rect(target, first_rect)
                    .or_else(|| second.find_rect(target, second_rect))
            }
        }
    }

    fn arrange(&self, rect: LayoutRect, out: &mut Vec<PaneLayout>) {
        match self {
            Self::Leaf { pane_id } => out.push(PaneLayout {
                pane_id: *pane_id,
                rect,
            }),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (first_rect, second_rect) = match axis {
                    SplitAxis::Vertical => {
                        let split = split_dimension(rect.width, *ratio);
                        rect.split_vertical(split)
                    }
                    SplitAxis::Horizontal => {
                        let split = split_dimension(rect.height, *ratio);
                        rect.split_horizontal(split)
                    }
                };

                first.arrange(first_rect, out);
                second.arrange(second_rect, out);
            }
        }
    }

    fn split_leaf(self, target: PaneId, new_pane: PaneId, insert_axis: SplitAxis) -> (Self, bool) {
        match self {
            Self::Leaf { pane_id } if pane_id == target => (
                Self::Split {
                    axis: insert_axis,
                    ratio: 50,
                    first: Box::new(Self::leaf(pane_id)),
                    second: Box::new(Self::leaf(new_pane)),
                },
                true,
            ),
            Self::Leaf { pane_id } => (Self::Leaf { pane_id }, false),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (first, inserted) = first.split_leaf(target, new_pane, insert_axis);
                if inserted {
                    (
                        Self::Split {
                            axis,
                            ratio,
                            first: Box::new(first),
                            second,
                        },
                        true,
                    )
                } else {
                    let (second, inserted) = second.split_leaf(target, new_pane, insert_axis);
                    (
                        Self::Split {
                            axis,
                            ratio,
                            first: Box::new(first),
                            second: Box::new(second),
                        },
                        inserted,
                    )
                }
            }
        }
    }

    fn remove_leaf(self, target: PaneId) -> (Option<Self>, bool) {
        match self {
            Self::Leaf { pane_id } if pane_id == target => (None, true),
            Self::Leaf { pane_id } => (Some(Self::Leaf { pane_id }), false),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (first, removed) = first.remove_leaf(target);
                if removed {
                    return match first {
                        Some(first) => (
                            Some(Self::Split {
                                axis,
                                ratio,
                                first: Box::new(first),
                                second,
                            }),
                            true,
                        ),
                        None => (Some(*second), true),
                    };
                }

                let first = match first {
                    Some(first) => first,
                    None => return (Some(*second), false),
                };

                let (second, removed) = second.remove_leaf(target);
                if removed {
                    return match second {
                        Some(second) => (
                            Some(Self::Split {
                                axis,
                                ratio,
                                first: Box::new(first),
                                second: Box::new(second),
                            }),
                            true,
                        ),
                        None => (Some(first), true),
                    };
                }

                match second {
                    Some(second) => (
                        Some(Self::Split {
                            axis,
                            ratio,
                            first: Box::new(first),
                            second: Box::new(second),
                        }),
                        false,
                    ),
                    None => (Some(first), false),
                }
            }
        }
    }

    fn swap_leaves(&mut self, left: PaneId, right: PaneId) -> usize {
        match self {
            Self::Leaf { pane_id } if *pane_id == left => {
                *pane_id = right;
                1
            }
            Self::Leaf { pane_id } if *pane_id == right => {
                *pane_id = left;
                1
            }
            Self::Leaf { .. } => 0,
            Self::Split { first, second, .. } => {
                first.swap_leaves(left, right) + second.swap_leaves(left, right)
            }
        }
    }

    fn resize_for_pane(
        &mut self,
        target: PaneId,
        direction: FocusDirection,
        delta: u16,
        rect: LayoutRect,
    ) -> bool {
        match self {
            Self::Leaf { .. } => false,
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (first_rect, second_rect) = match axis {
                    SplitAxis::Vertical => {
                        let split = split_dimension(rect.width, *ratio);
                        rect.split_vertical(split)
                    }
                    SplitAxis::Horizontal => {
                        let split = split_dimension(rect.height, *ratio);
                        rect.split_horizontal(split)
                    }
                };

                let first_contains = first.contains(target);
                let second_contains = second.contains(target);
                let ratio_delta = delta.min(98);

                let adjusted = match (*axis, direction, first_contains, second_contains) {
                    (SplitAxis::Vertical, FocusDirection::Left, true, false)
                    | (SplitAxis::Vertical, FocusDirection::Right, false, true)
                    | (SplitAxis::Horizontal, FocusDirection::Up, true, false)
                    | (SplitAxis::Horizontal, FocusDirection::Down, false, true) => {
                        *ratio = ratio.saturating_add(ratio_delta).min(99);
                        true
                    }
                    (SplitAxis::Vertical, FocusDirection::Right, true, false)
                    | (SplitAxis::Vertical, FocusDirection::Left, false, true)
                    | (SplitAxis::Horizontal, FocusDirection::Down, true, false)
                    | (SplitAxis::Horizontal, FocusDirection::Up, false, true) => {
                        *ratio = ratio.saturating_sub(ratio_delta).max(1);
                        true
                    }
                    _ => false,
                };

                if adjusted {
                    return true;
                }

                if first.resize_for_pane(target, direction, delta, first_rect) {
                    return true;
                }

                second.resize_for_pane(target, direction, delta, second_rect)
            }
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LayoutError {
    #[error("layout tree is empty")]
    Empty,
    #[error("layout tree already has a root")]
    RootAlreadyPresent,
    #[error("pane {0:?} was not found in the layout tree")]
    PaneNotFound(PaneId),
    #[error("pane {0:?} already exists in the layout tree")]
    DuplicatePane(PaneId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutTree {
    root: Option<LayoutNode>,
    active: Option<PaneId>,
}

impl LayoutTree {
    pub fn empty() -> Self {
        Self {
            root: None,
            active: None,
        }
    }

    pub fn new(root_pane: PaneId) -> Self {
        Self {
            root: Some(LayoutNode::leaf(root_pane)),
            active: Some(root_pane),
        }
    }

    pub fn root(&self) -> Option<&LayoutNode> {
        self.root.as_ref()
    }

    pub fn active_pane(&self) -> Option<PaneId> {
        self.active
    }

    pub fn pane_count(&self) -> usize {
        self.root.as_ref().map_or(0, LayoutNode::pane_count)
    }

    pub fn contains(&self, pane_id: PaneId) -> bool {
        self.root
            .as_ref()
            .is_some_and(|root| root.contains(pane_id))
    }

    pub fn insert_root(&mut self, pane_id: PaneId) -> Result<(), LayoutError> {
        if self.root.is_some() {
            return Err(LayoutError::RootAlreadyPresent);
        }

        self.root = Some(LayoutNode::leaf(pane_id));
        self.active = Some(pane_id);
        Ok(())
    }

    pub fn set_active_pane(&mut self, pane_id: PaneId) -> Result<(), LayoutError> {
        if !self.contains(pane_id) {
            return Err(LayoutError::PaneNotFound(pane_id));
        }

        self.active = Some(pane_id);
        Ok(())
    }

    pub fn focus_direction(
        &mut self,
        direction: FocusDirection,
        surface: LayoutRect,
    ) -> Result<PaneId, LayoutError> {
        let active = self.active.ok_or(LayoutError::Empty)?;
        let next = self.neighbor_in_direction(active, direction, surface)?;

        self.active = Some(next);
        Ok(next)
    }

    pub fn swap_active(
        &mut self,
        direction: FocusDirection,
        surface: LayoutRect,
    ) -> Result<PaneId, LayoutError> {
        let active = self.active.ok_or(LayoutError::Empty)?;
        let neighbor = self.neighbor_in_direction(active, direction, surface)?;
        if neighbor == active {
            return Ok(active);
        }

        let root = self.root.as_mut().ok_or(LayoutError::Empty)?;
        let swapped = root.swap_leaves(active, neighbor);
        if swapped != 2 {
            return Err(LayoutError::PaneNotFound(neighbor));
        }

        self.active = Some(active);
        Ok(active)
    }

    pub fn resize_active(
        &mut self,
        direction: FocusDirection,
        delta: u16,
        surface: LayoutRect,
    ) -> Result<(), LayoutError> {
        let active = self.active.ok_or(LayoutError::Empty)?;
        let root = self.root.as_mut().ok_or(LayoutError::Empty)?;
        if root.resize_for_pane(active, direction, delta, surface) {
            Ok(())
        } else {
            Err(LayoutError::PaneNotFound(active))
        }
    }

    pub fn arrange(&self, surface: LayoutRect) -> Vec<PaneLayout> {
        let mut out = Vec::new();
        if let Some(root) = &self.root {
            root.arrange(surface, &mut out);
        }
        out
    }

    pub fn layout_map(&self, surface: LayoutRect) -> HashMap<PaneId, LayoutRect> {
        self.arrange(surface)
            .into_iter()
            .map(|layout| (layout.pane_id, layout.rect))
            .collect()
    }

    pub fn split_active(
        &mut self,
        new_pane: PaneId,
        surface: LayoutRect,
    ) -> Result<PaneId, LayoutError> {
        let active = self.active.ok_or(LayoutError::Empty)?;
        self.split(active, new_pane, surface)
    }

    pub fn split_active_with_axis(
        &mut self,
        new_pane: PaneId,
        axis: SplitAxis,
    ) -> Result<PaneId, LayoutError> {
        let active = self.active.ok_or(LayoutError::Empty)?;
        self.split_with_axis(active, new_pane, axis)
    }

    pub fn split(
        &mut self,
        target: PaneId,
        new_pane: PaneId,
        surface: LayoutRect,
    ) -> Result<PaneId, LayoutError> {
        if self.contains(new_pane) {
            return Err(LayoutError::DuplicatePane(new_pane));
        }

        let root = self.root.take().ok_or(LayoutError::Empty)?;
        let target_rect = root
            .find_rect(target, surface)
            .ok_or(LayoutError::PaneNotFound(target))?;
        let axis = SplitAxis::from_rect(target_rect);
        self.split_node_with_axis(root, target, new_pane, axis)
    }

    pub fn split_with_axis(
        &mut self,
        target: PaneId,
        new_pane: PaneId,
        axis: SplitAxis,
    ) -> Result<PaneId, LayoutError> {
        if self.contains(new_pane) {
            return Err(LayoutError::DuplicatePane(new_pane));
        }

        let root = self.root.take().ok_or(LayoutError::Empty)?;
        self.split_node_with_axis(root, target, new_pane, axis)
    }

    fn split_node_with_axis(
        &mut self,
        root: LayoutNode,
        target: PaneId,
        new_pane: PaneId,
        axis: SplitAxis,
    ) -> Result<PaneId, LayoutError> {
        let (root, inserted) = root.split_leaf(target, new_pane, axis);

        if !inserted {
            self.root = Some(root);
            return Err(LayoutError::PaneNotFound(target));
        }

        self.root = Some(root);
        self.active = Some(new_pane);
        Ok(new_pane)
    }

    pub fn close(&mut self, pane_id: PaneId) -> Result<Option<PaneId>, LayoutError> {
        let root = self.root.take().ok_or(LayoutError::Empty)?;
        let (root, removed) = root.remove_leaf(pane_id);

        if !removed {
            self.root = root;
            return Err(LayoutError::PaneNotFound(pane_id));
        }

        self.root = root;
        if self.root.is_none() {
            self.active = None;
            return Ok(None);
        }

        if self.active == Some(pane_id) {
            self.active = self.root.as_ref().map(LayoutNode::first_leaf);
        }

        Ok(self.active)
    }

    fn neighbor_in_direction(
        &self,
        active: PaneId,
        direction: FocusDirection,
        surface: LayoutRect,
    ) -> Result<PaneId, LayoutError> {
        let layouts = self.arrange(surface);
        let current = layouts
            .iter()
            .find(|layout| layout.pane_id == active)
            .ok_or(LayoutError::PaneNotFound(active))?;

        Ok(layouts
            .iter()
            .filter(|layout| layout.pane_id != active)
            .filter_map(|candidate| {
                focus_candidate_score(current.rect, candidate.rect, direction)
                    .map(|score| (score, candidate.pane_id))
            })
            .min_by_key(|(score, _)| *score)
            .map(|(_, pane_id)| pane_id)
            .unwrap_or(active))
    }
}

impl Default for LayoutTree {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSet {
    active: WorkspaceId,
    layouts: HashMap<WorkspaceId, LayoutTree>,
}

impl WorkspaceSet {
    pub fn new(root_pane: PaneId) -> Self {
        let mut layouts = HashMap::new();
        layouts.insert(DEFAULT_WORKSPACE_ID, LayoutTree::new(root_pane));
        Self {
            active: DEFAULT_WORKSPACE_ID,
            layouts,
        }
    }

    pub fn active_workspace(&self) -> WorkspaceId {
        self.active
    }

    pub fn contains_workspace(&self, workspace_id: WorkspaceId) -> bool {
        self.layouts.contains_key(&workspace_id)
    }

    pub fn workspace_ids(&self) -> Vec<WorkspaceId> {
        let mut ids = self.layouts.keys().copied().collect::<Vec<_>>();
        ids.sort_by_key(|id| id.0);
        ids
    }

    pub fn layout(&self, workspace_id: WorkspaceId) -> Option<&LayoutTree> {
        self.layouts.get(&workspace_id)
    }

    pub fn layout_mut(&mut self, workspace_id: WorkspaceId) -> Option<&mut LayoutTree> {
        self.layouts.get_mut(&workspace_id)
    }

    pub fn active_layout(&self) -> &LayoutTree {
        self.layouts
            .get(&self.active)
            .expect("active workspace should always have a layout entry")
    }

    pub fn active_layout_mut(&mut self) -> &mut LayoutTree {
        self.layouts
            .get_mut(&self.active)
            .expect("active workspace should always have a layout entry")
    }

    pub fn switch_to(&mut self, workspace_id: WorkspaceId) -> bool {
        let existed = self.layouts.contains_key(&workspace_id);
        self.layouts
            .entry(workspace_id)
            .or_insert_with(LayoutTree::empty);
        self.active = workspace_id;
        existed
    }
}

fn clamp_split_dimension(total: u16, preferred: u16) -> u16 {
    if total <= 1 {
        return total;
    }

    preferred.clamp(1, total - 1)
}

fn split_dimension(total: u16, ratio: u16) -> u16 {
    if total <= 1 {
        return total;
    }

    let ratio = ratio.clamp(1, 99);
    let preferred = (u32::from(total) * u32::from(ratio) / 100) as u16;
    clamp_split_dimension(total, preferred)
}

fn overlap_1d(start_a: u16, end_a: u16, start_b: u16, end_b: u16) -> u16 {
    end_a.min(end_b).saturating_sub(start_a.max(start_b))
}

fn focus_candidate_score(
    current: LayoutRect,
    candidate: LayoutRect,
    direction: FocusDirection,
) -> Option<(u16, u16, u16)> {
    match direction {
        FocusDirection::Left => {
            if candidate.x_end() > current.x {
                return None;
            }
            Some((
                axis_gap(candidate.x_end(), current.x),
                orthogonal_gap(current.y, current.y_end(), candidate.y, candidate.y_end()),
                center_distance(current.center_y(), candidate.center_y()),
            ))
        }
        FocusDirection::Right => {
            if candidate.x < current.x_end() {
                return None;
            }
            Some((
                axis_gap(current.x_end(), candidate.x),
                orthogonal_gap(current.y, current.y_end(), candidate.y, candidate.y_end()),
                center_distance(current.center_y(), candidate.center_y()),
            ))
        }
        FocusDirection::Up => {
            if candidate.y_end() > current.y {
                return None;
            }
            Some((
                axis_gap(candidate.y_end(), current.y),
                orthogonal_gap(current.x, current.x_end(), candidate.x, candidate.x_end()),
                center_distance(current.center_x(), candidate.center_x()),
            ))
        }
        FocusDirection::Down => {
            if candidate.y < current.y_end() {
                return None;
            }
            Some((
                axis_gap(current.y_end(), candidate.y),
                orthogonal_gap(current.x, current.x_end(), candidate.x, candidate.x_end()),
                center_distance(current.center_x(), candidate.center_x()),
            ))
        }
    }
}

fn axis_gap(near_edge: u16, far_edge: u16) -> u16 {
    far_edge.saturating_sub(near_edge)
}

fn orthogonal_gap(start_a: u16, end_a: u16, start_b: u16, end_b: u16) -> u16 {
    if overlap_1d(start_a, end_a, start_b, end_b) > 0 {
        0
    } else if end_a <= start_b {
        start_b.saturating_sub(end_a)
    } else {
        start_a.saturating_sub(end_b)
    }
}

fn center_distance(center_a: i32, center_b: i32) -> u16 {
    center_a.abs_diff(center_b).min(u16::MAX as u32) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_layouts(layouts: Vec<PaneLayout>) -> HashMap<PaneId, LayoutRect> {
        layouts
            .into_iter()
            .map(|layout| (layout.pane_id, layout.rect))
            .collect()
    }

    #[test]
    fn single_pane_fills_the_surface() {
        let tree = LayoutTree::new(PaneId::new(1));
        let layouts = map_layouts(tree.arrange(LayoutRect::new(0, 0, 80, 24)));

        assert_eq!(
            layouts,
            HashMap::from([(PaneId::new(1), LayoutRect::new(0, 0, 80, 24))])
        );
    }

    #[test]
    fn wide_surface_splits_vertically() {
        let mut tree = LayoutTree::new(PaneId::new(1));
        let active = tree
            .split_active(PaneId::new(2), LayoutRect::new(0, 0, 120, 40))
            .expect("split should succeed");

        assert_eq!(active, PaneId::new(2));
        assert_eq!(tree.active_pane(), Some(PaneId::new(2)));
        assert_eq!(tree.pane_count(), 2);

        let layouts = map_layouts(tree.arrange(LayoutRect::new(0, 0, 120, 40)));
        assert_eq!(layouts[&PaneId::new(1)], LayoutRect::new(0, 0, 60, 40));
        assert_eq!(layouts[&PaneId::new(2)], LayoutRect::new(60, 0, 60, 40));
    }

    #[test]
    fn tall_surface_splits_horizontally() {
        let mut tree = LayoutTree::new(PaneId::new(10));
        tree.split_active(PaneId::new(11), LayoutRect::new(0, 0, 40, 120))
            .expect("split should succeed");

        let layouts = map_layouts(tree.arrange(LayoutRect::new(0, 0, 40, 120)));
        assert_eq!(layouts[&PaneId::new(10)], LayoutRect::new(0, 0, 40, 60));
        assert_eq!(layouts[&PaneId::new(11)], LayoutRect::new(0, 60, 40, 60));
    }

    #[test]
    fn explicit_split_axis_overrides_surface_heuristic() {
        let mut tree = LayoutTree::new(PaneId::new(1));
        tree.split_active_with_axis(PaneId::new(2), SplitAxis::Horizontal)
            .expect("split should succeed");

        let layouts = map_layouts(tree.arrange(LayoutRect::new(0, 0, 120, 40)));
        assert_eq!(layouts[&PaneId::new(1)], LayoutRect::new(0, 0, 120, 20));
        assert_eq!(layouts[&PaneId::new(2)], LayoutRect::new(0, 20, 120, 20));
    }

    #[test]
    fn nested_splits_stay_stable() {
        let mut tree = LayoutTree::new(PaneId::new(1));
        let surface = LayoutRect::new(0, 0, 160, 90);

        for next in 2..=8 {
            tree.split_active(PaneId::new(next), surface)
                .expect("split should succeed");
        }

        let layouts = tree.arrange(surface);
        assert_eq!(tree.pane_count(), 8);
        assert_eq!(layouts.len(), 8);
        assert!(layouts.iter().all(|layout| layout.rect.width > 0));
        assert!(layouts.iter().all(|layout| layout.rect.height > 0));

        let total_area: u32 = layouts.iter().map(|layout| layout.rect.area()).sum();
        assert_eq!(total_area, surface.area());
    }

    #[test]
    fn closing_active_pane_collapses_to_survivor() {
        let mut tree = LayoutTree::new(PaneId::new(1));
        let surface = LayoutRect::new(0, 0, 120, 40);
        tree.split_active(PaneId::new(2), surface)
            .expect("split should succeed");

        assert_eq!(
            tree.close(PaneId::new(2)).expect("close should succeed"),
            Some(PaneId::new(1))
        );
        assert_eq!(tree.active_pane(), Some(PaneId::new(1)));
        assert_eq!(tree.pane_count(), 1);

        let layouts = map_layouts(tree.arrange(surface));
        assert_eq!(
            layouts,
            HashMap::from([(PaneId::new(1), LayoutRect::new(0, 0, 120, 40))])
        );
    }

    #[test]
    fn closing_nested_non_active_pane_preserves_focus() {
        let mut tree = LayoutTree::new(PaneId::new(1));
        let surface = LayoutRect::new(0, 0, 120, 80);
        tree.split_active(PaneId::new(2), surface)
            .expect("split should succeed");
        tree.split_active(PaneId::new(3), surface)
            .expect("split should succeed");

        assert_eq!(tree.active_pane(), Some(PaneId::new(3)));
        assert_eq!(
            tree.close(PaneId::new(2)).expect("close should succeed"),
            Some(PaneId::new(3))
        );
        assert_eq!(tree.active_pane(), Some(PaneId::new(3)));
        assert_eq!(tree.pane_count(), 2);
    }

    #[test]
    fn directional_focus_finds_the_adjacent_pane() {
        let mut tree = LayoutTree::new(PaneId::new(1));
        let surface = LayoutRect::new(0, 0, 120, 80);
        tree.split_active(PaneId::new(2), surface)
            .expect("first split should succeed");
        tree.split_active(PaneId::new(3), surface)
            .expect("second split should succeed");

        assert_eq!(tree.active_pane(), Some(PaneId::new(3)));
        assert_eq!(
            tree.focus_direction(FocusDirection::Left, surface)
                .expect("focus should succeed"),
            PaneId::new(1)
        );
        assert_eq!(
            tree.focus_direction(FocusDirection::Right, surface)
                .expect("focus should succeed"),
            PaneId::new(2)
        );
        assert_eq!(
            tree.focus_direction(FocusDirection::Down, surface)
                .expect("focus should succeed"),
            PaneId::new(3)
        );
        assert_eq!(
            tree.focus_direction(FocusDirection::Up, surface)
                .expect("focus should succeed"),
            PaneId::new(2)
        );
    }

    #[test]
    fn directional_focus_stays_on_current_pane_at_the_edge() {
        let mut tree = LayoutTree::new(PaneId::new(1));
        let surface = LayoutRect::new(0, 0, 120, 40);
        tree.split_active(PaneId::new(2), surface)
            .expect("split should succeed");
        tree.set_active_pane(PaneId::new(1))
            .expect("active pane should exist");

        assert_eq!(
            tree.focus_direction(FocusDirection::Left, surface)
                .expect("focus should succeed"),
            PaneId::new(1)
        );
        assert_eq!(tree.active_pane(), Some(PaneId::new(1)));
    }

    #[test]
    fn resizing_active_split_changes_neighbor_rects() {
        let mut tree = LayoutTree::new(PaneId::new(1));
        let surface = LayoutRect::new(0, 0, 120, 40);
        tree.split_active(PaneId::new(2), surface)
            .expect("split should succeed");

        tree.resize_active(FocusDirection::Left, 10, surface)
            .expect("resize should succeed");

        let layouts = map_layouts(tree.arrange(surface));
        assert_eq!(layouts[&PaneId::new(1)], LayoutRect::new(0, 0, 48, 40));
        assert_eq!(layouts[&PaneId::new(2)], LayoutRect::new(48, 0, 72, 40));
    }

    #[test]
    fn swapping_active_pane_changes_leaf_positions() {
        let mut tree = LayoutTree::new(PaneId::new(1));
        let surface = LayoutRect::new(0, 0, 120, 40);
        tree.split_active(PaneId::new(2), surface)
            .expect("split should succeed");

        tree.swap_active(FocusDirection::Left, surface)
            .expect("swap should succeed");

        let layouts = map_layouts(tree.arrange(surface));
        assert_eq!(tree.active_pane(), Some(PaneId::new(2)));
        assert_eq!(layouts[&PaneId::new(2)], LayoutRect::new(0, 0, 60, 40));
        assert_eq!(layouts[&PaneId::new(1)], LayoutRect::new(60, 0, 60, 40));
    }

    #[test]
    fn empty_tree_rejects_operations() {
        let mut tree = LayoutTree::empty();
        let surface = LayoutRect::new(0, 0, 80, 24);

        assert!(matches!(
            tree.split_active(PaneId::new(2), surface),
            Err(LayoutError::Empty)
        ));
        assert!(matches!(
            tree.close(PaneId::new(1)),
            Err(LayoutError::Empty)
        ));

        tree.insert_root(PaneId::new(1))
            .expect("root insert should succeed");
        assert!(matches!(
            tree.insert_root(PaneId::new(2)),
            Err(LayoutError::RootAlreadyPresent)
        ));
    }

    #[test]
    fn workspace_switch_keeps_independent_layout_trees() {
        let mut workspaces = WorkspaceSet::new(PaneId::new(1));
        let surface = LayoutRect::new(0, 0, 120, 40);
        workspaces
            .active_layout_mut()
            .split_active(PaneId::new(2), surface)
            .expect("split should succeed");

        assert_eq!(workspaces.active_workspace(), WorkspaceId::new(1));
        assert_eq!(workspaces.active_layout().pane_count(), 2);

        let existed = workspaces.switch_to(WorkspaceId::new(2));
        assert!(!existed);
        assert_eq!(workspaces.active_layout().pane_count(), 0);

        workspaces
            .active_layout_mut()
            .insert_root(PaneId::new(10))
            .expect("root insert should succeed");
        assert_eq!(workspaces.active_layout().pane_count(), 1);

        let existed = workspaces.switch_to(WorkspaceId::new(1));
        assert!(existed);
        assert_eq!(workspaces.active_layout().pane_count(), 2);
        assert_eq!(
            workspaces.active_layout().active_pane(),
            Some(PaneId::new(2))
        );
    }

    #[test]
    fn workspace_ids_are_limited_to_one_through_nine() {
        assert!(WorkspaceId::try_from(1).is_ok());
        assert!(WorkspaceId::try_from(9).is_ok());
        assert!(matches!(
            WorkspaceId::try_from(0),
            Err(WorkspaceError::InvalidWorkspaceId(0))
        ));
        assert!(matches!(
            WorkspaceId::try_from(10),
            Err(WorkspaceError::InvalidWorkspaceId(10))
        ));
    }
}
