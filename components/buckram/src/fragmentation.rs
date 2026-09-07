// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Retained fragmentation inputs and the ordinary block resume kernel.
//!
//! This is deliberately a model layer. It records the state a formatter must
//! carry across a fragmentainer boundary without introducing a second box
//! tree or making a fragmentainer a backend-owned rectangle cache.

#![allow(dead_code)]

use crate::{
    BlockMarginState, BoxId, FlowAxes, FragmentId, FragmentTree, LogicalRect, LogicalSize,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FragmentationContextId(u32);

impl FragmentationContextId {
    /// The unfragmented root context used by the existing continuous lane.
    pub const INITIAL: Self = Self(0);

    pub(crate) const fn from_index(index: u32) -> Self {
        Self(index)
    }

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FragmentainerId(u32);

impl FragmentainerId {
    pub(crate) const fn from_index(index: u32) -> Self {
        Self(index)
    }

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FragmentainerKind {
    Column,
}

/// The only multicol input admitted by the first live formatter seam.
///
/// `Auto` is represented explicitly so balancing cannot be mistaken for this
/// sequential lane. The formatter still owns used-size resolution and
/// producing fragmentainers; this value carries only style-owned parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MulticolFill {
    Auto,
    Balance,
    BalanceAll,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SequentialMulticolInput {
    column_count: usize,
    column_width: Option<f32>,
    column_gap: f32,
    fill: MulticolFill,
}

impl SequentialMulticolInput {
    pub fn new(
        column_count: usize,
        column_width: Option<f32>,
        column_gap: f32,
        fill: MulticolFill,
    ) -> Option<Self> {
        (matches!(fill, MulticolFill::Auto)
            && column_count > 0
            && column_width.is_none_or(|width| width.is_finite() && width > 0.0)
            && column_gap.is_finite()
            && column_gap >= 0.0)
            .then_some(Self {
                column_count,
                column_width,
                column_gap,
                fill,
            })
    }

    pub fn column_count(self) -> usize {
        self.column_count
    }

    pub fn column_width(self) -> Option<f32> {
        self.column_width
    }

    pub fn column_gap(self) -> f32 {
        self.column_gap
    }

    pub fn fill(self) -> MulticolFill {
        self.fill
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FragmentationContext {
    pub(crate) id: FragmentationContextId,
    pub(crate) parent: Option<FragmentationContextId>,
    pub(crate) flow: FlowAxes,
    pub(crate) fragmentainers: Vec<FragmentainerId>,
}

impl FragmentationContext {
    pub fn id(&self) -> FragmentationContextId {
        self.id
    }

    pub fn parent(&self) -> Option<FragmentationContextId> {
        self.parent
    }

    pub fn flow(&self) -> FlowAxes {
        self.flow
    }

    pub fn fragmentainers(&self) -> &[FragmentainerId] {
        &self.fragmentainers
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fragmentainer {
    pub(crate) id: FragmentainerId,
    pub(crate) context: FragmentationContextId,
    pub(crate) parent_context: Option<FragmentationContextId>,
    pub(crate) sequence: usize,
    pub(crate) kind: FragmentainerKind,
    pub(crate) flow: FlowAxes,
    pub(crate) logical_rect: LogicalRect,
}

impl Fragmentainer {
    pub fn id(&self) -> FragmentainerId {
        self.id
    }

    pub fn context(&self) -> FragmentationContextId {
        self.context
    }

    pub fn parent_context(&self) -> Option<FragmentationContextId> {
        self.parent_context
    }

    pub fn sequence(&self) -> usize {
        self.sequence
    }

    pub fn kind(&self) -> FragmentainerKind {
        self.kind
    }

    pub fn flow(&self) -> FlowAxes {
        self.flow
    }

    pub fn logical_rect(&self) -> LogicalRect {
        self.logical_rect
    }

    pub fn logical_size(&self) -> LogicalSize {
        LogicalSize {
            inline: self.logical_rect.inline_size,
            block: self.logical_rect.block_size,
        }
    }
}

/// The live block formatter's exclusion snapshot is still crate-private and
/// includes rounded shape geometry. K6b retains an explicit typed deferral
/// rather than inventing a lossy public copy of that state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FloatExclusionState {
    Deferred,
}

impl Default for FloatExclusionState {
    fn default() -> Self {
        Self::Deferred
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ClearanceState {
    /// Clearance ownership remains in the ordinary block formatter until the
    /// continuation path consumes the live float state.
    #[default]
    Deferred,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InlineResumeState {
    #[default]
    Deferred,
}

/// The state required to resume an ordinary block without replaying accepted
/// children. Nested and inline state remain typed even when a given block has
/// no nested child or inline content.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockBreakToken {
    pub box_id: BoxId,
    pub next_child: usize,
    pub nested_child: Option<Box<BreakToken>>,
    /// Provisional model state. The live margin-collapse owner has not yet
    /// been threaded through this synthetic kernel.
    pub carried_margin: BlockMarginState,
    pub float_exclusions: FloatExclusionState,
    pub clearance: ClearanceState,
    pub inline: InlineResumeState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeferredBreakKind {
    Inline,
    Table,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeferredBreakToken {
    pub box_id: BoxId,
    pub kind: DeferredBreakKind,
}

/// Algorithm-owned continuation variants. Consumers must match the variant
/// rather than interpreting a numeric offset with formatter-specific meaning.
#[derive(Clone, Debug, PartialEq)]
pub enum BreakToken {
    Block(BlockBreakToken),
    Deferred(DeferredBreakToken),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BlockFragmentChild {
    pub box_id: BoxId,
    pub block_size: f32,
}

impl BlockFragmentChild {
    pub(crate) fn new(box_id: BoxId, block_size: f32) -> Self {
        assert!(block_size.is_finite() && block_size >= 0.0);
        Self { box_id, block_size }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BlockFragmentationState {
    pub carried_margin: BlockMarginState,
    pub float_exclusions: FloatExclusionState,
    pub clearance: ClearanceState,
    pub inline: InlineResumeState,
    pub nested_child: Option<Box<BreakToken>>,
}

impl Default for BlockFragmentationState {
    fn default() -> Self {
        Self {
            carried_margin: BlockMarginState::default(),
            float_exclusions: FloatExclusionState::default(),
            clearance: ClearanceState::default(),
            inline: InlineResumeState::default(),
            nested_child: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BlockFragmentStep {
    pub fragment: Option<FragmentId>,
    pub next: Option<BlockBreakToken>,
    pub next_child: usize,
    pub outcome: BlockFragmentOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BlockFragmentOutcome {
    Completed,
    Continued,
    Monolithic(BoxId),
}

/// Emit one ordinary block fragment into a fixed fragmentainer.
///
/// The caller supplies the retained box identity and child list. The next
/// call consumes the returned token and appends to the same [`FragmentTree`].
/// This intentionally handles breaks between ordinary children only; forced
/// break, widows/orphans, and monolithic-child policy belong to the next
/// formatter gate.
pub(crate) fn start_block_fragment(
    fragments: &mut FragmentTree,
    fragmentainer: FragmentainerId,
    box_id: BoxId,
    children: &[BlockFragmentChild],
    state: &BlockFragmentationState,
) -> BlockFragmentStep {
    emit_block_fragment(fragments, fragmentainer, box_id, children, state, None)
}

/// Resume an ordinary block from the token produced by the prior step. The
/// token is the sole source of carried state on this path.
pub(crate) fn resume_block_fragment(
    fragments: &mut FragmentTree,
    fragmentainer: FragmentainerId,
    box_id: BoxId,
    children: &[BlockFragmentChild],
    token: &BlockBreakToken,
) -> BlockFragmentStep {
    let state = BlockFragmentationState {
        carried_margin: token.carried_margin,
        float_exclusions: token.float_exclusions,
        clearance: token.clearance,
        inline: token.inline,
        nested_child: token.nested_child.clone(),
    };
    emit_block_fragment(
        fragments,
        fragmentainer,
        box_id,
        children,
        &state,
        Some(token),
    )
}

fn emit_block_fragment(
    fragments: &mut FragmentTree,
    fragmentainer: FragmentainerId,
    box_id: BoxId,
    children: &[BlockFragmentChild],
    state: &BlockFragmentationState,
    resume: Option<&BlockBreakToken>,
) -> BlockFragmentStep {
    let start = resume.map_or(0, |token| {
        assert_eq!(token.box_id, box_id, "a block token belongs to one CSS box");
        token.next_child
    });
    assert!(
        start <= children.len(),
        "a block token cannot skip past children"
    );
    let record = fragments
        .fragmentainer(fragmentainer)
        .expect("the block must resume in a live fragmentainer")
        .clone();
    let capacity = record.logical_rect().block_size;
    assert!(capacity.is_finite() && capacity >= 0.0);

    let mut cursor = 0.0;
    let mut monolithic = None;
    let mut next_child = start;
    while next_child < children.len() {
        let child = children[next_child];
        if cursor == 0.0 && child.block_size > capacity {
            // Monolithic children are reported as a continuation boundary and
            // remain whole. The caller can route them to a later policy gate.
            monolithic = Some(child.box_id);
            break;
        }
        if cursor > 0.0 && cursor + child.block_size > capacity {
            break;
        }
        let child_rect = LogicalRect {
            inline_start: record.logical_rect().inline_start,
            block_start: cursor,
            inline_size: record.logical_rect().inline_size,
            block_size: child.block_size,
        };
        let child_fragment = crate::Fragment::from_logical(
            child.box_id,
            child_rect,
            crate::PhysicalSize {
                width: record.logical_rect().inline_size,
                height: record.logical_rect().block_size,
            },
            record.flow(),
        );
        // The parent is inserted first below, so child materialization is
        // deferred until the parent id exists.
        let _ = child_fragment;
        cursor += child.block_size;
        next_child += 1;
    }

    let has_next = next_child < children.len() && monolithic.is_none();
    if let Some(box_id) = monolithic {
        return BlockFragmentStep {
            fragment: None,
            next: None,
            next_child: start,
            outcome: BlockFragmentOutcome::Monolithic(box_id),
        };
    }
    let token = has_next.then(|| BlockBreakToken {
        box_id,
        next_child,
        nested_child: state.nested_child.clone(),
        carried_margin: state.carried_margin,
        float_exclusions: state.float_exclusions,
        clearance: state.clearance,
        inline: state.inline,
    });
    let mut parent = crate::Fragment::from_logical(
        box_id,
        record.logical_rect(),
        crate::PhysicalSize {
            width: record.logical_rect().inline_size,
            height: record.logical_rect().block_size,
        },
        record.flow(),
    );
    parent.continuation = token.clone().map(BreakToken::Block);
    let parent_id = fragments.push_in_fragmentainer(parent, None, None, fragmentainer);

    let mut cursor = 0.0;
    for child in children
        .iter()
        .copied()
        .skip(start)
        .take(next_child - start)
    {
        let child_rect = LogicalRect {
            inline_start: record.logical_rect().inline_start,
            block_start: cursor,
            inline_size: record.logical_rect().inline_size,
            block_size: child.block_size,
        };
        let child_fragment = crate::Fragment::from_logical(
            child.box_id,
            child_rect,
            crate::PhysicalSize {
                width: record.logical_rect().inline_size,
                height: record.logical_rect().block_size,
            },
            record.flow(),
        );
        fragments.push_in_fragmentainer(
            child_fragment,
            Some(parent_id),
            Some(parent_id),
            fragmentainer,
        );
        cursor += child.block_size;
    }

    BlockFragmentStep {
        fragment: Some(parent_id),
        next: token,
        next_child,
        outcome: if has_next {
            BlockFragmentOutcome::Continued
        } else {
            BlockFragmentOutcome::Completed
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Baselines, BoxOrigin, ContainingBlock, CssBox, CssBoxTree, DisplayRole, PositioningScheme,
    };

    #[test]
    fn fixed_columns_resume_one_box_with_typed_state() {
        let mut boxes = CssBoxTree::default();
        let root = boxes.push(
            CssBox::new(
                BoxOrigin::Element(1u8),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Static,
                false,
                None,
                ContainingBlock::Initial,
            ),
            None,
            true,
        );
        let children = (2u8..=5)
            .map(|node| {
                boxes.push(
                    CssBox::new(
                        BoxOrigin::Element(node),
                        DisplayRole::BLOCK_FLOW,
                        FlowAxes::HORIZONTAL_LTR,
                        PositioningScheme::Static,
                        false,
                        None,
                        ContainingBlock::Box(root),
                    ),
                    Some(root),
                    true,
                )
            })
            .collect::<Vec<_>>();
        let children = children
            .into_iter()
            .map(|box_id| BlockFragmentChild::new(box_id, 50.0))
            .collect::<Vec<_>>();

        let mut fragments = FragmentTree::default();
        let context = fragments.create_fragmentation_context(
            Some(FragmentationContextId::INITIAL),
            FlowAxes::HORIZONTAL_LTR,
        );
        let first_column = fragments.create_fragmentainer(
            context,
            LogicalRect {
                inline_start: 0.0,
                block_start: 0.0,
                inline_size: 100.0,
                block_size: 100.0,
            },
            FragmentainerKind::Column,
        );
        let second_column = fragments.create_fragmentainer(
            context,
            LogicalRect {
                inline_start: 120.0,
                block_start: 0.0,
                inline_size: 100.0,
                block_size: 100.0,
            },
            FragmentainerKind::Column,
        );
        let carried_margin = BlockMarginState::from_box(
            crate::BlockStyle::default(),
            100.0,
            crate::CollapsedMargin::from_margin(7.0),
            crate::CollapsedMargin::from_margin(-2.0),
            crate::BlockMarginCollapse::NONE,
            false,
        );
        let state = BlockFragmentationState {
            carried_margin,
            float_exclusions: FloatExclusionState::Deferred,
            clearance: ClearanceState::Deferred,
            inline: InlineResumeState::Deferred,
            nested_child: Some(Box::new(BreakToken::Deferred(DeferredBreakToken {
                box_id: children[2].box_id,
                kind: DeferredBreakKind::Table,
            }))),
        };

        let first = start_block_fragment(&mut fragments, first_column, root, &children, &state);
        assert_eq!(first.outcome, BlockFragmentOutcome::Continued);
        assert_eq!(first.next_child, 2);
        let first_id = first.fragment.expect("ordinary content emits a fragment");
        let token = first.next.as_ref().expect("the first column resumes");
        assert_eq!(token.carried_margin, carried_margin);
        assert_eq!(token.clearance, ClearanceState::Deferred);
        assert!(matches!(
            token.float_exclusions,
            FloatExclusionState::Deferred
        ));
        assert!(matches!(token.inline, InlineResumeState::Deferred));
        match token.nested_child.as_deref() {
            Some(BreakToken::Deferred(deferred)) => {
                assert_eq!(deferred.box_id, children[2].box_id);
                assert_eq!(deferred.kind, DeferredBreakKind::Table);
            },
            other => panic!("unexpected nested continuation: {other:?}"),
        }

        // A resumed formatter takes state from its token.
        let second = resume_block_fragment(&mut fragments, second_column, root, &children, token);
        assert_eq!(second.outcome, BlockFragmentOutcome::Completed);
        let second_id = second.fragment.expect("the continuation emits a fragment");
        assert_eq!(
            fragments.fragmentation_context(context).unwrap().parent(),
            Some(FragmentationContextId::INITIAL)
        );
        assert_eq!(
            fragments
                .fragmentation_context(context)
                .unwrap()
                .fragmentainers(),
            &[first_column, second_column]
        );
        assert_eq!(fragments.fragment_ids_for_box(root).len(), 2);
        assert_eq!(
            fragments.get(first_id).unwrap().fragmentainer(),
            Some(first_column)
        );
        assert_eq!(
            fragments.get(second_id).unwrap().fragmentainer(),
            Some(second_column)
        );
        assert_eq!(
            fragments.get(first_id).unwrap().flow(),
            fragments.fragmentainer(first_column).unwrap().flow()
        );
        assert_eq!(
            fragments.get(second_id).unwrap().flow(),
            fragments.fragmentainer(second_column).unwrap().flow()
        );
        assert_eq!(
            fragments.get(first_id).unwrap().fragmentation_context(),
            context
        );
        assert_eq!(fragments.fragment_ids_for_box(children[0].box_id).len(), 1);
        assert_eq!(fragments.fragment_ids_for_box(children[2].box_id).len(), 1);
        let first_child = fragments.fragment_ids_for_box(children[0].box_id)[0];
        let second_child = fragments.fragment_ids_for_box(children[2].box_id)[0];
        assert_eq!(fragments.get(first_child).unwrap().parent(), Some(first_id));
        assert_eq!(
            fragments.get(first_child).unwrap().containing_fragment(),
            Some(first_id)
        );
        assert_eq!(
            fragments.get(second_child).unwrap().parent(),
            Some(second_id)
        );
        assert_eq!(
            fragments.get(second_child).unwrap().containing_fragment(),
            Some(second_id)
        );
        assert_eq!(
            fragments
                .get(first_child)
                .unwrap()
                .logical_rect
                .inline_start,
            0.0
        );
        assert_eq!(
            fragments.get(first_child).unwrap().logical_rect.block_start,
            0.0
        );
        assert_eq!(
            fragments
                .get(second_child)
                .unwrap()
                .logical_rect
                .inline_start,
            120.0
        );
        assert_eq!(
            fragments
                .get(second_child)
                .unwrap()
                .logical_rect
                .block_start,
            0.0
        );
        fragments.recompute_overflow();
        assert_eq!(
            fragments.get(first_id).unwrap().overflow,
            fragments.get(first_id).unwrap().logical_rect
        );
        assert_eq!(
            fragments.get(second_id).unwrap().overflow,
            fragments.get(second_id).unwrap().logical_rect
        );
        assert_eq!(boxes.len(), 5, "the retained box tree was not rebuilt");
        assert_eq!(fragments.fragmentainer(first_column).unwrap().sequence(), 0);
        assert_eq!(
            fragments.fragmentainer(second_column).unwrap().sequence(),
            1
        );
        // Baselines are not synthesized by this kernel; the next inline/block
        // formatter gate supplies real baseline outputs.
        assert_eq!(
            fragments.get(first_id).unwrap().baselines,
            Baselines::default()
        );

        let third_column = fragments.create_fragmentainer(
            context,
            LogicalRect {
                inline_start: 240.0,
                block_start: 0.0,
                inline_size: 100.0,
                block_size: 100.0,
            },
            FragmentainerKind::Column,
        );
        let before_monolithic_len = fragments.len();
        let before_monolithic_roots = fragments.roots().len();
        let monolithic = start_block_fragment(
            &mut fragments,
            third_column,
            root,
            &[BlockFragmentChild::new(children[0].box_id, 101.0)],
            &BlockFragmentationState::default(),
        );
        assert_eq!(
            monolithic.outcome,
            BlockFragmentOutcome::Monolithic(children[0].box_id)
        );
        assert!(monolithic.fragment.is_none());
        assert!(monolithic.next.is_none(), "monolithic content cannot loop");
        assert_eq!(fragments.len(), before_monolithic_len);
        assert_eq!(fragments.roots().len(), before_monolithic_roots);
    }
}
