// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Caller-owned scratch tree for Taffy's low-level layout algorithms.
//!
//! Taffy supplies block, flex, and grid algorithms. Buckram owns the tree,
//! source identity, contexts, and returned placements. The public surface in
//! this module deliberately uses Buckram types rather than Taffy types.

use std::{marker::PhantomData, slice};

use taffy::{
    BlockContext, Cache, CacheTree, DetailedGridInfo, Layout, LayoutBlockContainer,
    LayoutFlexboxContainer, LayoutGridContainer, LayoutInput, LayoutOutput, LayoutPartialTree,
    NodeId, RoundTree, RunMode, SizingMode, Style, TraversePartialTree, TraverseTree,
    compute_block_layout, compute_cached_layout, compute_flexbox_layout, compute_grid_layout,
    compute_hidden_layout, compute_leaf_layout, compute_root_layout, round_layout,
};

use crate::block::FloatContextState;
use crate::block::solve_in_flow_inline_size_for_border_box;
use crate::fragmentation::SequentialMulticolInput;
use crate::{
    Baselines, BlockBoxSizing, BlockContainingBlock, BlockDeferral, BlockFormattingContext,
    BlockMarginState, BlockSizeValue, BlockStyle, ClearSide, CollapsedMargin, FloatLineConstraints,
    FloatSide, FlowAxes, FlowLength, FlowLengthAuto, IntrinsicSizeKind, IntrinsicSizes,
    LogicalRect, LogicalSides, LogicalSize, PhysicalRect, PhysicalSides, PhysicalSize,
    solve_float_inline_size, solve_in_flow_inline_size, solve_shrink_to_fit_inline_size,
};

mod run;
#[cfg(test)]
mod tests;

use run::*;

/// Formatting role selected by Buckram before entering a backend algorithm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlgorithmKind {
    Hidden,
    Leaf,
    Block,
    Flex,
    Grid,
    /// A Buckram table formatting context. K4h selects it for every table
    /// grid before layout; K4d6 supplies final geometry when the table
    /// pipeline accepts its sizing input.
    Table,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockAlgorithm {
    Buckram,
    Taffy,
}

/// Dispatch state for a completed algorithm run. A sequential input currently
/// records an explicit unsupported dispatch until the formatter can publish
/// continuation geometry; it must never look like ordinary continuous flow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FragmentationDispatch {
    Continuous,
    SequentialMulticolUnsupported,
}

impl Default for FragmentationDispatch {
    fn default() -> Self {
        Self::Continuous
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FragmentationOutput {
    pub dispatch: FragmentationDispatch,
    pub sequential_inputs: Vec<(AlgorithmNodeId, SequentialMulticolInput)>,
}

/// Stable identity within one scratch algorithm tree.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AlgorithmNodeId(u32);

impl AlgorithmNodeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    fn from_taffy(id: NodeId) -> Self {
        Self(
            usize::from(id)
                .try_into()
                .expect("a Taffy node id exceeded u32::MAX"),
        )
    }

    fn into_taffy(self) -> NodeId {
        NodeId::from(self.index())
    }
}

/// Width and height pair at the algorithm boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AlgorithmSize<T> {
    pub width: T,
    pub height: T,
}

impl<T> AlgorithmSize<T> {
    pub const fn new(width: T, height: T) -> Self {
        Self { width, height }
    }
}

/// Available-space constraint without exposing the backend enum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AlgorithmAvailableSpace {
    Definite(f32),
    MinContent,
    MaxContent,
}

/// Parent-relative placement returned by the algorithm backend.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AlgorithmLayout {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

mod sealed {
    pub trait AlgorithmStyle {
        fn as_taffy_style(&self) -> &taffy::Style;
        fn as_taffy_style_mut(&mut self) -> &mut taffy::Style;
    }

    impl AlgorithmStyle for taffy::Style {
        fn as_taffy_style(&self) -> &taffy::Style {
            self
        }

        fn as_taffy_style_mut(&mut self) -> &mut taffy::Style {
            self
        }
    }
}

/// A style accepted by the private Taffy algorithm adapter.
///
/// This sealed trait keeps Taffy out of Buckram's method signatures while the
/// K1 caller still constructs the backend style privately. K3 replaces that
/// caller-side lowering with Buckram-owned logical and intrinsic inputs.
pub trait AlgorithmStyle: sealed::AlgorithmStyle {}

impl AlgorithmStyle for Style {}

struct AlgorithmNode<S, Context, Source> {
    kind: AlgorithmKind,
    block_style: BlockStyle,
    block_algorithm: Option<BlockAlgorithm>,
    block_deferral: Option<BlockDeferral>,
    block_margins: Option<BlockMarginState>,
    exported_float_state: Option<FloatContextState>,
    nested_float_state_enabled: bool,
    inline_context_float: bool,
    float_line_constraints_enabled: bool,
    float_avoidance_enabled: bool,
    intrinsic_shrink_to_fit_enabled: bool,
    flex_content_base_query: bool,
    flex_content_base_probe_active: bool,
    intrinsic_inline_sizes: Option<IntrinsicSizes>,
    table_wrapper_inline_size: Option<f32>,
    style: S,
    context: Option<Context>,
    source: Source,
    parent: Option<AlgorithmNodeId>,
    children: Vec<AlgorithmNodeId>,
    cache: Cache,
    unrounded_layout: Layout,
    final_layout: Layout,
    grid_info: Option<DetailedGridInfo>,
    /// CSS Grid §9.2: a direct absolute or fixed child's static position is
    /// aligned in the grid container's content box unless that grid also
    /// generates the child's containing block, in which case the §9.1 grid
    /// area applies. The renderer sets this from the K5a box graph; the
    /// adapter never derives it from backend positioning.
    grid_static_position_uses_grid_area: bool,
    baselines: Baselines,
    sequential_multicol: Option<SequentialMulticolInput>,
}

/// A caller-owned arena used only while running layout algorithms.
///
/// Source identity lives on each node, so callers need no parallel
/// `NodeId -> source` map. The style parameter is generic and the public
/// methods expose only Buckram identifiers and geometry.
pub struct AlgorithmTree<S, Context, Source> {
    nodes: Vec<AlgorithmNode<S, Context, Source>>,
    fragmentation_output: FragmentationOutput,
    /// Resolves a Taffy calc() pointer against a percentage basis. The tree
    /// owner stores calc values with stable addresses, tags them into
    /// `Dimension::calc`, and installs the matching interpreter here; without
    /// one, a calc dimension resolves to zero (Taffy's own fallback).
    calc_resolver: Option<fn(*const (), f32) -> f32>,
}

impl<S, Context, Source> Default for AlgorithmTree<S, Context, Source> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S, Context, Source> AlgorithmTree<S, Context, Source> {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            fragmentation_output: FragmentationOutput::default(),
            calc_resolver: None,
        }
    }

    /// Install the interpreter for calc()-tagged dimensions in this tree's
    /// styles. The caller guarantees every tagged pointer outlives the tree.
    pub fn set_calc_resolver(&mut self, resolver: fn(*const (), f32) -> f32) {
        self.calc_resolver = Some(resolver);
    }

    pub fn new_with_children(
        &mut self,
        kind: AlgorithmKind,
        style: S,
        children: &[AlgorithmNodeId],
        source: Source,
    ) -> AlgorithmNodeId {
        self.new_with_children_and_block_style(kind, BlockStyle::default(), style, children, source)
    }

    pub fn new_with_children_and_block_style(
        &mut self,
        kind: AlgorithmKind,
        block_style: BlockStyle,
        style: S,
        children: &[AlgorithmNodeId],
        source: Source,
    ) -> AlgorithmNodeId {
        self.push(kind, block_style, style, children, None, source)
    }

    pub fn new_leaf_with_context(
        &mut self,
        style: S,
        context: Context,
        source: Source,
    ) -> AlgorithmNodeId {
        self.new_leaf_with_context_and_block_style(BlockStyle::default(), style, context, source)
    }

    pub fn new_leaf_with_context_and_block_style(
        &mut self,
        block_style: BlockStyle,
        style: S,
        context: Context,
        source: Source,
    ) -> AlgorithmNodeId {
        self.push(
            AlgorithmKind::Leaf,
            block_style,
            style,
            &[],
            Some(context),
            source,
        )
    }

    fn push(
        &mut self,
        kind: AlgorithmKind,
        block_style: BlockStyle,
        style: S,
        children: &[AlgorithmNodeId],
        context: Option<Context>,
        source: Source,
    ) -> AlgorithmNodeId {
        let id = AlgorithmNodeId(
            self.nodes
                .len()
                .try_into()
                .expect("an algorithm tree exceeded u32::MAX nodes"),
        );
        self.nodes.push(AlgorithmNode {
            kind,
            block_style,
            block_algorithm: None,
            block_deferral: None,
            block_margins: None,
            exported_float_state: None,
            nested_float_state_enabled: false,
            inline_context_float: false,
            float_line_constraints_enabled: false,
            float_avoidance_enabled: false,
            intrinsic_shrink_to_fit_enabled: false,
            flex_content_base_query: false,
            flex_content_base_probe_active: false,
            intrinsic_inline_sizes: None,
            table_wrapper_inline_size: None,
            style,
            context,
            source,
            parent: None,
            children: children.to_vec(),
            cache: Cache::new(),
            unrounded_layout: Layout::new(),
            final_layout: Layout::new(),
            grid_info: None,
            grid_static_position_uses_grid_area: false,
            baselines: Baselines::default(),
            sequential_multicol: None,
        });
        for child in children {
            let previous = self.nodes[child.index()].parent.replace(id);
            assert!(
                previous.is_none(),
                "an algorithm scratch node cannot have two parents"
            );
        }
        id
    }

    pub fn source(&self, id: AlgorithmNodeId) -> &Source {
        &self.nodes[id.index()].source
    }

    pub fn kind(&self, id: AlgorithmNodeId) -> AlgorithmKind {
        self.nodes[id.index()].kind
    }

    pub fn style(&self, id: AlgorithmNodeId) -> &S {
        &self.nodes[id.index()].style
    }

    pub fn block_style(&self, id: AlgorithmNodeId) -> BlockStyle {
        self.nodes[id.index()].block_style
    }

    pub fn block_algorithm(&self, id: AlgorithmNodeId) -> Option<BlockAlgorithm> {
        self.nodes[id.index()].block_algorithm
    }

    /// The named Buckram boundary that selected Taffy's block algorithm.
    ///
    /// `None` means this node either used Buckram's block algorithm or is not
    /// a block-algorithm node. Taffy descendants reached while an ancestor's
    /// fallback is already active report `BackendSizingMode`; their ancestor
    /// retains the CSS-facing reason.
    pub fn block_deferral(&self, id: AlgorithmNodeId) -> Option<BlockDeferral> {
        self.nodes[id.index()].block_deferral
    }

    pub fn block_margins(&self, id: AlgorithmNodeId) -> Option<BlockMarginState> {
        self.nodes[id.index()].block_margins
    }

    /// Supply style-owned sequential multicol parameters from the CSS owner.
    /// Used container geometry remains a run-side responsibility. This is a
    /// dormant formatter contract until the run emits fragmentainer and
    /// continuation records.
    pub fn set_sequential_multicol(&mut self, id: AlgorithmNodeId, input: SequentialMulticolInput) {
        self.nodes[id.index()].sequential_multicol = Some(input);
        self.nodes[id.index()].cache.clear();
    }

    /// Remove a previously lowered sequential input before the next run.
    ///
    /// The formatter owns admission lifetime. Clearing the input also clears
    /// the run's backend cache so a later continuous run cannot reuse the
    /// unsupported input's measurements.
    pub fn clear_sequential_multicol(&mut self, id: AlgorithmNodeId) {
        self.nodes[id.index()].sequential_multicol = None;
        self.nodes[id.index()].cache.clear();
    }

    pub fn sequential_multicol(&self, id: AlgorithmNodeId) -> Option<SequentialMulticolInput> {
        self.nodes[id.index()].sequential_multicol
    }

    pub fn fragmentation_output(&self) -> &FragmentationOutput {
        &self.fragmentation_output
    }

    pub fn block_algorithm_counts(&self) -> (usize, usize) {
        self.nodes.iter().fold((0, 0), |(buckram, taffy), node| {
            match node.block_algorithm {
                Some(BlockAlgorithm::Buckram) => (buckram + 1, taffy),
                Some(BlockAlgorithm::Taffy) => (buckram, taffy + 1),
                None => (buckram, taffy),
            }
        })
    }

    /// Count Taffy block runs by their named Buckram deferral.
    ///
    /// This keeps CSS-facing fallback distinct from scratch measurement in
    /// [`BlockDeferral::BackendSizingMode`]. A table cell's intrinsic pass may
    /// use the latter without causing any block ancestor to abandon Buckram.
    pub fn block_deferral_count(&self, deferral: BlockDeferral) -> usize {
        self.nodes
            .iter()
            .filter(|node| {
                node.block_algorithm == Some(BlockAlgorithm::Taffy)
                    && node.block_deferral == Some(deferral)
            })
            .count()
    }

    pub fn style_mut(&mut self, id: AlgorithmNodeId) -> &mut S {
        &mut self.nodes[id.index()].style
    }

    /// Publish a table algorithm's used grid width to its generated wrapper.
    ///
    /// Backend root setup may offer the containing width as a provisional
    /// known dimension. The table grid is the standards-owned authority for
    /// its wrapper's border-edge width, so this explicit carrier outranks that
    /// provisional value when Buckram admits the wrapper as a block child.
    pub fn set_table_wrapper_inline_size(&mut self, id: AlgorithmNodeId, size: f32) {
        assert!(
            size.is_finite() && size >= 0.0,
            "a table wrapper inline size must be finite and non-negative"
        );
        let node = &mut self.nodes[id.index()];
        node.table_wrapper_inline_size = Some(size);
        let size = BlockSizeValue::Length(FlowLength::px(size));
        if node.block_style.flow.is_horizontal() {
            node.block_style.size.width = size;
        } else {
            node.block_style.size.height = size;
        }
        node.cache.clear();
    }

    /// Admit this direct flex/grid child to the renderer's static-position
    /// provider. Buckram selects the narrow participation boundary and keeps
    /// the resulting rectangle as the K5b formatter output; callers never
    /// lower a CSS positioning role into the backend style themselves.
    ///
    /// The provider is deliberately unavailable outside a flex or grid
    /// parent. Ordinary block, inline, and table-internal out-of-flow routes
    /// have separate Buckram formatting boundaries.
    pub fn enable_flex_grid_static_position_provider(&mut self, id: AlgorithmNodeId)
    where
        S: AlgorithmStyle,
    {
        let parent = self.nodes[id.index()]
            .parent
            .expect("a flex/grid static-position provider requires an attached direct child");
        assert!(
            matches!(
                self.nodes[parent.index()].kind,
                AlgorithmKind::Flex | AlgorithmKind::Grid
            ),
            "the static-position provider belongs only to a flex or grid parent"
        );
        assert!(
            matches!(
                self.nodes[id.index()].block_style.position,
                crate::BlockPosition::Absolute | crate::BlockPosition::Fixed
            ),
            "the static-position provider requires an absolute or fixed child"
        );
        sealed::AlgorithmStyle::as_taffy_style_mut(&mut self.nodes[id.index()].style).position =
            taffy::Position::Absolute;
    }

    /// Select this direct grid child's finalized grid area as its
    /// static-position alignment container (CSS Grid §9.2, second sentence).
    /// Callers make this selection only when the K5a containing-block graph
    /// chose this same grid as the child's containing block; otherwise the
    /// provider keeps the grid's content box. With `auto` grid lines the area
    /// is bounded by the grid's padding edges, as §9.1 requires.
    pub fn use_grid_area_for_static_position(&mut self, id: AlgorithmNodeId) {
        let parent = self.nodes[id.index()]
            .parent
            .expect("a grid static-position area requires an attached direct child");
        assert_eq!(
            self.nodes[parent.index()].kind,
            AlgorithmKind::Grid,
            "only a direct grid child can use a grid-area static-position rectangle"
        );
        assert!(
            matches!(
                self.nodes[id.index()].block_style.position,
                crate::BlockPosition::Absolute | crate::BlockPosition::Fixed
            ),
            "the grid-area static-position rectangle requires an absolute or fixed child"
        );
        self.nodes[id.index()].grid_static_position_uses_grid_area = true;
    }

    /// Supply the resolved CSS inline size to a detached absolute/fixed
    /// formatting root before its second formatting pass. The positioned
    /// solver owns the value; this scratch-tree setter merely gives the local
    /// formatter the same constraint without asking Taffy to select a
    /// containing block or out-of-flow participation.
    pub fn set_positioned_inline_size(&mut self, id: AlgorithmNodeId, size: f32) {
        assert!(
            size.is_finite() && size >= 0.0,
            "a positioned formatting inline size must be finite and non-negative"
        );
        let style = &mut self.nodes[id.index()].block_style;
        let size = BlockSizeValue::Length(FlowLength::px(size));
        if style.flow.is_horizontal() {
            style.size.width = size;
        } else {
            style.size.height = size;
        }
    }

    /// Supply a standards-resolved block size to a detached absolute/fixed
    /// formatting root before its constrained second formatting pass.
    pub fn set_positioned_block_size(&mut self, id: AlgorithmNodeId, size: f32) {
        assert!(
            size.is_finite() && size >= 0.0,
            "a positioned formatting block size must be finite and non-negative"
        );
        let style = &mut self.nodes[id.index()].block_style;
        let size = BlockSizeValue::Length(FlowLength::px(size));
        if style.flow.is_horizontal() {
            style.size.height = size;
        } else {
            style.size.width = size;
        }
    }

    /// Clear formatter state after a standards-owned used size changes a
    /// detached formatting root. The next tree walk must visit that root and
    /// its ancestors rather than reuse the pre-positioning layout cache.
    pub fn clear_layout_cache(&mut self) {
        for node in &mut self.nodes {
            node.cache.clear();
            node.intrinsic_inline_sizes = None;
        }
    }

    /// Admit this direct measured leaf to Buckram's float-aware line lane.
    ///
    /// Measured contexts opt in explicitly because a generic callback is not
    /// necessarily an inline formatter and may ignore float constraints.
    pub fn enable_float_line_constraints(&mut self, id: AlgorithmNodeId) {
        assert!(
            self.nodes[id.index()].context.is_some(),
            "only a measured leaf can consume float line constraints"
        );
        self.nodes[id.index()].float_line_constraints_enabled = true;
    }

    /// Admit this ordinary block as a continuation of its parent's BFC.
    ///
    /// Callers decide from the generated CSS box role, keeping backend display
    /// lookalikes and unresolved formatting-context boundaries outside this
    /// lane.
    pub fn enable_nested_float_state(&mut self, id: AlgorithmNodeId) {
        let node = &self.nodes[id.index()];
        assert_eq!(
            node.kind,
            AlgorithmKind::Block,
            "nested float state requires an ordinary block algorithm node"
        );
        assert!(
            !node.block_style.establishes_bfc,
            "an explicit BFC must not inherit its parent's float state"
        );
        self.nodes[id.index()].nested_float_state_enabled = true;
    }

    /// Preserve the generated-box fact that this blockified float originated
    /// inside an inline formatting context.
    pub fn mark_inline_context_float(&mut self, id: AlgorithmNodeId) {
        assert_ne!(
            self.nodes[id.index()].block_style.float,
            FloatSide::None,
            "only a floated box can carry inline-context float provenance"
        );
        self.nodes[id.index()].inline_context_float = true;
    }

    /// Admit this block-level independent formatting context to Buckram's
    /// float-avoidance policy.
    ///
    /// The caller opts in because a generic flex/grid backend or atomic inline
    /// box can establish a BFC while still requiring sizing and baseline work
    /// outside this lane.
    pub fn enable_float_avoidance(&mut self, id: AlgorithmNodeId) {
        let node = &mut self.nodes[id.index()];
        assert!(
            matches!(
                node.kind,
                AlgorithmKind::Leaf
                    | AlgorithmKind::Block
                    | AlgorithmKind::Flex
                    | AlgorithmKind::Grid
            ),
            "float avoidance accepts block, leaf, flex, and grid algorithms"
        );
        assert!(
            node.block_style.establishes_bfc && node.block_style.float == FloatSide::None,
            "float avoidance requires an in-flow BFC"
        );
        node.float_avoidance_enabled = true;
    }

    /// Whether this auto-width box has a complete intrinsic inline provider.
    ///
    /// The provider keeps block, inline, flex, and grid roles distinct. It
    /// rejects percentage and cyclic shapes instead of recovering a width from
    /// a completed backend layout.
    pub fn supports_intrinsic_shrink_to_fit(&self, id: AlgorithmNodeId) -> bool {
        let node = &self.nodes[id.index()];
        matches!(
            node.kind,
            AlgorithmKind::Block | AlgorithmKind::Leaf | AlgorithmKind::Flex | AlgorithmKind::Grid
        ) && node.block_style.shrink_to_fit
            && self.intrinsic_inline_subtree_is_admitted(id, true)
    }

    /// Admit an auto-width float or atomic inline box to Buckram's intrinsic
    /// shrink-to-fit lane.
    pub fn enable_intrinsic_shrink_to_fit(&mut self, id: AlgorithmNodeId) {
        let node = &self.nodes[id.index()];
        assert!(
            matches!(
                node.kind,
                AlgorithmKind::Block
                    | AlgorithmKind::Leaf
                    | AlgorithmKind::Flex
                    | AlgorithmKind::Grid
            ),
            "intrinsic shrink-to-fit requires an admitted formatting context"
        );
        assert!(
            node.block_style.shrink_to_fit,
            "intrinsic shrink-to-fit requires an auto-width shrink-to-fit box"
        );
        assert!(
            self.supports_intrinsic_shrink_to_fit(id),
            "intrinsic shrink-to-fit requires an admitted in-flow subtree"
        );
        self.nodes[id.index()].intrinsic_shrink_to_fit_enabled = true;
    }

    /// Whether this node is using Buckram's admitted intrinsic query lane.
    pub fn uses_intrinsic_shrink_to_fit(&self, id: AlgorithmNodeId) -> bool {
        self.nodes[id.index()].intrinsic_shrink_to_fit_enabled
    }

    /// Query admitted min-content and max-content widths for an out-of-flow
    /// formatting root.
    ///
    /// The caller remains responsible for selecting the containing block and
    /// resolving the positioned inset equation. This temporarily removes the
    /// position deferral only for the intrinsic query, so a completed
    /// normal-flow rectangle never becomes the browser-facing auto width.
    pub fn positioned_intrinsic_inline_sizes<Measure>(
        &mut self,
        id: AlgorithmNodeId,
        mut measure: Measure,
    ) -> Option<IntrinsicSizes>
    where
        S: AlgorithmStyle,
        Measure: FnMut(
            AlgorithmSize<Option<f32>>,
            AlgorithmSize<AlgorithmAvailableSpace>,
            AlgorithmNodeId,
            Option<&mut Context>,
            Option<&FloatLineConstraints>,
        ) -> AlgorithmSize<f32>,
    {
        if !matches!(
            self.nodes[id.index()].kind,
            AlgorithmKind::Block | AlgorithmKind::Leaf
        ) || !matches!(
            self.nodes[id.index()].block_style.position,
            crate::BlockPosition::Absolute | crate::BlockPosition::Fixed
        ) {
            return None;
        }

        let original_style = self.nodes[id.index()].block_style;
        {
            let root_style = &mut self.nodes[id.index()].block_style;
            root_style.position = crate::BlockPosition::Static;
            // The intrinsic query measures the root's contents. K5d applies
            // the root's preferred ratio to those contributions afterwards.
            root_style.aspect_ratio = None;
        }
        if !self.intrinsic_inline_subtree_is_admitted(id, true) {
            self.nodes[id.index()].block_style = original_style;
            return None;
        }

        let mut run = AlgorithmRun {
            tree: self,
            measure: &mut measure,
            line_constraints: None,
            nested_float_state: None,
            resolved_shrink_to_fit: None,
            fixed_leaf_intrinsics_enabled: false,
            marker: PhantomData,
        };
        let result = run.measure_intrinsic_inline_subtree(id, None).ok();
        run.tree.nodes[id.index()].block_style = original_style;
        result
    }

    fn intrinsic_inline_subtree_is_admitted(&self, id: AlgorithmNodeId, is_root: bool) -> bool {
        let node = &self.nodes[id.index()];
        let measured_replaced_leaf =
            node.kind == AlgorithmKind::Leaf && node.context.is_some() && node.block_style.replaced;
        if (node.block_style.replaced || node.block_style.aspect_ratio.is_some())
            && !measured_replaced_leaf
        {
            return false;
        }
        if !intrinsic_inline_style_is_admitted(node.block_style, is_root) {
            return false;
        }
        // The generic leaf measurement callback and the flex/grid intrinsic
        // query are still physical-width routes. Same-flow vertical block
        // trees without measured leaves have complete logical inline inputs,
        // so admit that narrower route without treating text or algorithms as
        // vertically intrinsic-capable.
        if !node.block_style.flow.is_horizontal()
            && (matches!(node.kind, AlgorithmKind::Flex | AlgorithmKind::Grid)
                || (matches!(node.kind, AlgorithmKind::Leaf) && node.context.is_some()))
        {
            return false;
        }
        let admitted = match node.kind {
            AlgorithmKind::Hidden => true,
            // A context-free leaf represents an empty formatting root. Its
            // min-content and max-content contributions are both zero, so it
            // is safe to admit without asking a formatter callback to infer a
            // normal-flow fallback rectangle.
            AlgorithmKind::Leaf => true,
            AlgorithmKind::Block | AlgorithmKind::Flex | AlgorithmKind::Grid => node
                .children
                .iter()
                .copied()
                .all(|child| self.intrinsic_inline_subtree_is_admitted(child, false)),
            // K4d owns table intrinsic admission when live dispatch lands.
            AlgorithmKind::Table => false,
        };
        admitted
    }

    pub fn children(&self, id: AlgorithmNodeId) -> &[AlgorithmNodeId] {
        &self.nodes[id.index()].children
    }

    pub fn context(&self, id: AlgorithmNodeId) -> Option<&Context> {
        self.nodes[id.index()].context.as_ref()
    }

    pub fn layout(&self, id: AlgorithmNodeId) -> AlgorithmLayout {
        let layout = self.nodes[id.index()].final_layout;
        AlgorithmLayout {
            x: layout.location.x,
            y: layout.location.y,
            width: layout.size.width,
            height: layout.size.height,
        }
    }

    /// The formatting coordinate assigned before CSS positioning applies
    /// insets. This carries a formatter output into Buckram's K5 positioning
    /// pass; it does not expose backend node identity or parent selection.
    pub fn static_layout(&self, id: AlgorithmNodeId) -> AlgorithmLayout {
        let layout = self.nodes[id.index()].final_layout;
        AlgorithmLayout {
            x: layout.static_location.x,
            y: layout.static_location.y,
            width: layout.size.width,
            height: layout.size.height,
        }
    }

    /// The grid area Taffy finalized for this direct absolutely positioned
    /// child before it applied the child's insets and self-alignment. Taffy
    /// reports its grid-column and grid-row axes as horizontal X/Y; project
    /// those track coordinates through the container flow before exposing the
    /// physical rectangle that the K5 route consumes.
    pub fn grid_positioned_area(&self, id: AlgorithmNodeId) -> Option<PhysicalRect> {
        let parent = self.nodes[id.index()].parent?;
        if self.nodes[parent.index()].kind != AlgorithmKind::Grid {
            return None;
        }
        let area = self.nodes[parent.index()]
            .grid_info
            .as_ref()?
            .positioned_items
            .iter()
            .find(|item| item.node == id.into_taffy())?
            .grid_area;
        let track_area = LogicalRect::from_horizontal_physical(PhysicalRect {
            x: area.left,
            y: area.top,
            width: (area.right - area.left).max(0.0),
            height: (area.bottom - area.top).max(0.0),
        });
        let container = &self.nodes[parent.index()];
        Some(container.block_style.flow.physical_rect(
            track_area,
            PhysicalSize {
                width: container.final_layout.size.width,
                height: container.final_layout.size.height,
            },
        ))
    }

    /// The rectangle an algorithm wrote, before the backend's rounding pass.
    ///
    /// An owned formatting context that adjusts its own subtree after writing
    /// it must read back the same store it wrote, or it would fold one
    /// rounding into the unrounded values and round twice.
    pub fn unrounded_layout(&self, id: AlgorithmNodeId) -> AlgorithmLayout {
        let layout = self.nodes[id.index()].unrounded_layout;
        AlgorithmLayout {
            x: layout.location.x,
            y: layout.location.y,
            width: layout.size.width,
            height: layout.size.height,
        }
    }

    /// Hand one node's dispatch to a Buckram algorithm after the table's
    /// block pipeline has written its final geometry. Table grids begin on
    /// the Buckram dispatcher too, so a named sizing deferral never revives a
    /// backend table route.
    pub fn set_kind(&mut self, id: AlgorithmNodeId, kind: AlgorithmKind) {
        self.nodes[id.index()].kind = kind;
    }

    /// Write a rectangle a Buckram algorithm already decided.
    ///
    /// A formatting context Buckram owns outright computes its own geometry
    /// and its children's before the backend walk begins; its node then
    /// reports that size and does not recurse, so nothing overwrites what was
    /// written here. Positions are relative to the node's parent, matching
    /// [`AlgorithmTree::layout`].
    ///
    /// The rectangle is written unrounded. The backend's rounding pass walks
    /// the whole tree from the root and derives every final layout from the
    /// unrounded one, so writing only the final layout would be discarded,
    /// and writing only a pre-rounded value would round this subtree on a
    /// different grid from its siblings.
    pub fn set_layout(&mut self, id: AlgorithmNodeId, layout: AlgorithmLayout) {
        assert!(
            [layout.x, layout.y, layout.width, layout.height]
                .into_iter()
                .all(f32::is_finite),
            "an owned formatting context must write a finite rectangle"
        );
        let node = &mut self.nodes[id.index()];
        node.unrounded_layout.location.x = layout.x;
        node.unrounded_layout.location.y = layout.y;
        node.unrounded_layout.size.width = layout.width;
        node.unrounded_layout.size.height = layout.height;
        node.final_layout = node.unrounded_layout;
    }

    /// First and last baseline outputs produced by this formatting context.
    /// They are logical offsets from the node's block-start edge.
    pub fn baselines(&self, id: AlgorithmNodeId) -> Baselines {
        self.nodes[id.index()].baselines
    }

    /// Replace a direct formatting-context baseline result before parent
    /// contexts consume it. The adapter stores the modeled output and never
    /// asks Taffy to rediscover a descendant baseline later.
    pub fn set_baselines(&mut self, id: AlgorithmNodeId, baselines: Baselines) {
        assert!(
            Baselines::new(baselines.first, baselines.last).is_some(),
            "formatting-context baselines must be finite logical offsets"
        );
        self.nodes[id.index()].baselines = baselines;
    }

    /// Propagate child formatting-context baseline outputs through the
    /// standards-owned scratch tree. This consumes each child's declared
    /// output and parent-relative placement, never Taffy's child traversal.
    pub fn propagate_baselines(&mut self) {
        for node in &mut self.nodes {
            node.baselines = Baselines::synthesized_from_block_end(node.final_layout.size.height);
        }
        self.propagate_declared_baselines();
    }

    /// Re-run parent baseline selection after a caller has supplied direct
    /// line-formatting baselines for admitted measured contexts.
    pub fn propagate_declared_baselines(&mut self) {
        for index in (0..self.nodes.len()).rev() {
            let children = self.nodes[index].children.clone();
            let first = children.iter().copied().find_map(|child| {
                let child_node = &self.nodes[child.index()];
                (child_node.block_style.float == FloatSide::None
                    && !child_node.block_style.is_out_of_flow())
                .then(|| {
                    child_node
                        .baselines
                        .first
                        .map(|baseline| child_node.final_layout.location.y + baseline)
                })
                .flatten()
            });
            let last = children.iter().rev().copied().find_map(|child| {
                let child_node = &self.nodes[child.index()];
                (child_node.block_style.float == FloatSide::None
                    && !child_node.block_style.is_out_of_flow())
                .then(|| {
                    child_node
                        .baselines
                        .last
                        .map(|baseline| child_node.final_layout.location.y + baseline)
                })
                .flatten()
            });
            if first.is_some() || last.is_some() {
                self.nodes[index].baselines = Baselines::new(first, last)
                    .expect("child baseline outputs remain finite logical offsets");
            }
        }
    }

    pub fn node_ids(&self) -> impl Iterator<Item = AlgorithmNodeId> + '_ {
        (0..self.nodes.len()).map(|index| {
            AlgorithmNodeId(
                index
                    .try_into()
                    .expect("an algorithm scratch tree exceeded u32::MAX nodes"),
            )
        })
    }

    fn begin_fragmentation_run(&mut self) {
        self.fragmentation_output = FragmentationOutput::default();
        for (index, node) in self.nodes.iter().enumerate() {
            if let Some(input) = node.sequential_multicol {
                self.fragmentation_output.dispatch =
                    FragmentationDispatch::SequentialMulticolUnsupported;
                self.fragmentation_output
                    .sequential_inputs
                    .push((AlgorithmNodeId(index as u32), input));
            }
        }
    }
}

impl<S, Context, Source> AlgorithmTree<S, Context, Source>
where
    S: AlgorithmStyle,
{
    /// Run scratch layout with the root's renderer-owned positioned children
    /// presented to Taffy as out of flow.
    ///
    /// Buckram retains their CSS position and final geometry separately. This
    /// adapter role only prevents a direct absolute or fixed child from
    /// contributing to an intrinsic or table-cell measurement while preserving
    /// the backend node for later formatting and fragment collection.
    pub fn compute_layout_with_measure_excluding_out_of_flow_children<Measure>(
        &mut self,
        root: AlgorithmNodeId,
        available: AlgorithmSize<AlgorithmAvailableSpace>,
        measure: Measure,
    ) where
        Measure: FnMut(
            AlgorithmSize<Option<f32>>,
            AlgorithmSize<AlgorithmAvailableSpace>,
            AlgorithmNodeId,
            Option<&mut Context>,
            Option<&FloatLineConstraints>,
        ) -> AlgorithmSize<f32>,
    {
        let children = self.nodes[root.index()].children.clone();
        let mut flipped = Vec::new();
        for child in children {
            if !self.nodes[child.index()].block_style.is_out_of_flow() {
                continue;
            }
            let style =
                sealed::AlgorithmStyle::as_taffy_style_mut(&mut self.nodes[child.index()].style);
            flipped.push((child, style.position));
            style.position = taffy::Position::Absolute;
        }
        self.clear_layout_cache();
        self.compute_layout_with_measure(root, available, measure);
        for (child, previous) in flipped {
            sealed::AlgorithmStyle::as_taffy_style_mut(&mut self.nodes[child.index()].style)
                .position = previous;
        }
        // The next ordinary run must not reuse a cache entry made while those
        // private backend roles were temporarily different.
        self.clear_layout_cache();
    }

    pub fn compute_layout_with_measure<Measure>(
        &mut self,
        root: AlgorithmNodeId,
        available: AlgorithmSize<AlgorithmAvailableSpace>,
        mut measure: Measure,
    ) where
        Measure: FnMut(
            AlgorithmSize<Option<f32>>,
            AlgorithmSize<AlgorithmAvailableSpace>,
            AlgorithmNodeId,
            Option<&mut Context>,
            Option<&FloatLineConstraints>,
        ) -> AlgorithmSize<f32>,
    {
        self.begin_fragmentation_run();
        let available = taffy::Size {
            width: to_taffy_available(available.width),
            height: to_taffy_available(available.height),
        };
        let mut run = AlgorithmRun {
            tree: self,
            measure: &mut measure,
            line_constraints: None,
            nested_float_state: None,
            resolved_shrink_to_fit: None,
            fixed_leaf_intrinsics_enabled: false,
            marker: PhantomData,
        };
        compute_root_layout(&mut run, root.into_taffy(), available);
        round_layout(&mut run, root.into_taffy());
        run.tree.propagate_baselines();
    }
}

fn to_taffy_available(value: AlgorithmAvailableSpace) -> taffy::AvailableSpace {
    match value {
        AlgorithmAvailableSpace::Definite(value) => taffy::AvailableSpace::Definite(value),
        AlgorithmAvailableSpace::MinContent => taffy::AvailableSpace::MinContent,
        AlgorithmAvailableSpace::MaxContent => taffy::AvailableSpace::MaxContent,
    }
}

fn from_taffy_available(value: taffy::AvailableSpace) -> AlgorithmAvailableSpace {
    match value {
        taffy::AvailableSpace::Definite(value) => AlgorithmAvailableSpace::Definite(value),
        taffy::AvailableSpace::MinContent => AlgorithmAvailableSpace::MinContent,
        taffy::AvailableSpace::MaxContent => AlgorithmAvailableSpace::MaxContent,
    }
}

#[derive(Clone, Copy)]
struct PhysicalOptionalSize {
    width: Option<f32>,
    height: Option<f32>,
}

impl PhysicalOptionalSize {
    fn from_taffy(size: taffy::Size<Option<f32>>) -> Self {
        Self {
            width: size.width,
            height: size.height,
        }
    }

    fn from_available(size: taffy::Size<taffy::AvailableSpace>) -> Self {
        Self {
            width: available_option(size.width),
            height: available_option(size.height),
        }
    }

    fn into_taffy(self) -> taffy::Size<Option<f32>> {
        taffy::Size {
            width: self.width,
            height: self.height,
        }
    }
}

fn available_option(value: taffy::AvailableSpace) -> Option<f32> {
    match value {
        taffy::AvailableSpace::Definite(value) => Some(value),
        taffy::AvailableSpace::MinContent | taffy::AvailableSpace::MaxContent => None,
    }
}

fn logical_optional_size(axes: FlowAxes, physical: PhysicalOptionalSize) -> LogicalOptionalSize {
    if axes.is_horizontal() {
        LogicalOptionalSize {
            inline: physical.width,
            block: physical.height,
        }
    } else {
        LogicalOptionalSize {
            inline: physical.height,
            block: physical.width,
        }
    }
}

fn physical_optional_size(axes: FlowAxes, logical: LogicalOptionalSize) -> PhysicalOptionalSize {
    if axes.is_horizontal() {
        PhysicalOptionalSize {
            width: logical.inline,
            height: logical.block,
        }
    } else {
        PhysicalOptionalSize {
            width: logical.block,
            height: logical.inline,
        }
    }
}

fn physical_available_size(
    axes: FlowAxes,
    logical: AlgorithmSize<AlgorithmAvailableSpace>,
) -> taffy::Size<taffy::AvailableSpace> {
    if axes.is_horizontal() {
        taffy::Size {
            width: to_taffy_available(logical.width),
            height: to_taffy_available(logical.height),
        }
    } else {
        taffy::Size {
            width: to_taffy_available(logical.height),
            height: to_taffy_available(logical.width),
        }
    }
}

fn logical_available_size(
    axes: FlowAxes,
    physical: taffy::Size<taffy::AvailableSpace>,
) -> AlgorithmSize<AlgorithmAvailableSpace> {
    if axes.is_horizontal() {
        AlgorithmSize::new(
            from_taffy_available(physical.width),
            from_taffy_available(physical.height),
        )
    } else {
        AlgorithmSize::new(
            from_taffy_available(physical.height),
            from_taffy_available(physical.width),
        )
    }
}

#[derive(Clone, Copy)]
struct LogicalOptionalSize {
    inline: Option<f32>,
    block: Option<f32>,
}

fn resolve_outer_dimension(
    preferred: BlockSizeValue,
    minimum: BlockSizeValue,
    maximum: BlockSizeValue,
    containing_size: Option<f32>,
    padding_border: f32,
    box_sizing: BlockBoxSizing,
) -> Option<f32> {
    preferred
        .resolve_definite(containing_size)
        .map(|preferred| specified_outer_size(preferred, padding_border, box_sizing))
        .map(|preferred| {
            clamp_outer_dimension(
                preferred,
                minimum,
                maximum,
                containing_size,
                padding_border,
                box_sizing,
            )
        })
}

fn clamp_outer_dimension(
    value: f32,
    minimum: BlockSizeValue,
    maximum: BlockSizeValue,
    containing_size: Option<f32>,
    padding_border: f32,
    box_sizing: BlockBoxSizing,
) -> f32 {
    let minimum = minimum
        .resolve_definite(containing_size)
        .map(|minimum| specified_outer_size(minimum, padding_border, box_sizing))
        .unwrap_or(padding_border);
    let maximum = maximum
        .resolve_definite(containing_size)
        .map(|maximum| specified_outer_size(maximum, padding_border, box_sizing));
    let value = value.max(minimum);
    maximum.map_or(value, |maximum| value.min(maximum.max(padding_border)))
}

fn specified_outer_size(specified: f32, padding_border: f32, box_sizing: BlockBoxSizing) -> f32 {
    match box_sizing {
        BlockBoxSizing::ContentBox => specified.max(0.0) + padding_border,
        BlockBoxSizing::BorderBox => specified.max(padding_border),
    }
}

fn to_taffy_rect(sides: PhysicalSides<f32>) -> taffy::Rect<f32> {
    taffy::Rect {
        left: sides.left,
        right: sides.right,
        top: sides.top,
        bottom: sides.bottom,
    }
}

/// Whether a box is opaque to the block formatting context it participates
/// in: it establishes an independent formatting context and is not a float,
/// so CSS 2.1 section 9.4.1 gives it its own algorithm and nothing inside it
/// interacts with the outside. Floats keep their existing walk because their
/// placement is the parent's decision (section 9.5).
fn is_opaque_formatting_root(style: BlockStyle) -> bool {
    style.establishes_bfc && style.float == FloatSide::None
}

/// The narrow orthogonal flex shape whose automatic block size can be left to
/// Taffy while its horizontal block parent keeps Buckram's normal-flow walk.
///
/// The child's definite logical inline size gives Taffy a complete flex main
/// axis. Its logical block axis is the physical width here, so it must remain
/// unknown rather than inheriting the horizontal parent's definite inline
/// size. Livery lowers only CSS `row` and `row-reverse` in this vertical flow
/// to Taffy's physical `Column` directions.
fn admits_orthogonal_auto_block_flex_child(
    parent_style: BlockStyle,
    child_kind: AlgorithmKind,
    child_style: BlockStyle,
    child_flex_direction: taffy::FlexDirection,
) -> bool {
    let child_block_size = if child_style.flow.is_horizontal() {
        child_style.size.height
    } else {
        child_style.size.width
    };
    parent_style.flow.is_horizontal()
        && child_kind == AlgorithmKind::Flex
        && matches!(
            child_style.position,
            crate::BlockPosition::Static | crate::BlockPosition::Relative
        )
        && child_style.float == FloatSide::None
        && child_style.clear == ClearSide::None
        && !child_style.shrink_to_fit
        && !child_style.replaced
        && child_style.aspect_ratio.is_none()
        && !child_style.size_containment.width
        && !child_style.size_containment.height
        && !child_style.has_nonlinear_lengths
        && child_style.containing_flow == parent_style.flow
        && child_style.flow.is_horizontal() != parent_style.flow.is_horizontal()
        && child_style.min_size
            == crate::BlockDimensions::new(BlockSizeValue::Auto, BlockSizeValue::Auto)
        && child_style.max_size
            == crate::BlockDimensions::new(BlockSizeValue::None, BlockSizeValue::None)
        && matches!(
            intrinsic_absolute_size(intrinsic_inline_dimension(
                child_style.size,
                child_style.flow
            )),
            Some(_)
        )
        && matches!(child_block_size, BlockSizeValue::Auto)
        && matches!(
            child_flex_direction,
            taffy::FlexDirection::Column | taffy::FlexDirection::ColumnReverse
        )
}

/// Whether an intrinsic query ignores authored preferred sizes.
fn is_content_sizing_mode(sizing_mode: SizingMode) -> bool {
    matches!(
        sizing_mode,
        SizingMode::ContentSize | SizingMode::ContentSizeForAutomaticMinimum
    )
}

struct ChildIter<'a>(slice::Iter<'a, AlgorithmNodeId>);

impl Iterator for ChildIter<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().copied().map(AlgorithmNodeId::into_taffy)
    }
}
