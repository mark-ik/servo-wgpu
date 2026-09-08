// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
    hash::Hash,
};

use buckram::{
    AlgorithmAvailableSpace, AlgorithmKind, AlgorithmNodeId, AlgorithmSize, AlgorithmTree,
    Baselines, BlockBoxSizing, BlockCornerRadii, BlockCornerRadius, BlockDeferral, BlockDimensions,
    BlockPosition as BuckramBlockPosition, BlockSizeValue, BlockStyle, BoxId, BoxOrigin, ClearSide,
    CollapsedBorderGeometry, ContainingBlock, CssBox, DisplayInside, DisplayOutside,
    FloatContextProvenance, FloatLineConstraints, FloatReferenceBox, FloatSide, FlowAxes,
    FlowLength, FlowLengthAuto, FormattingContextKind, Fragment as TreeFragment, FragmentDraftTree,
    FragmentId, FragmentTree, InternalTableRole, IntrinsicSizeCache, IntrinsicSizeKind,
    IntrinsicSizeQuery, IntrinsicSizes, LayoutResult, LogicalAxis, LogicalRect,
    OverconstrainedInlineAlignment, PhysicalOffset, PhysicalRect, PhysicalSide, PhysicalSides,
    PhysicalSize, PositioningScheme, StaticPosition, StaticPositionSource, TableCell,
    TableCellInput, TableCellLayoutInput, TableCellLayoutOutput, TableCellLayoutPass,
    TableFragmentRole, TableFragments, TableGrid, TableGridInputs, TableGridLines,
    TableRowLayoutError, TableRowSpan, TableTrackInput, TableTrackVisibility,
    resolve_collapsed_border_geometry,
};
use layout_dom_api::{LayoutDom, LocalName, Namespace, NodeKind};
use livery::{
    ComputedValues,
    media::{Device, ViewportSizes},
    stylesheet::ContainerSnapshot,
    values::{
        Alignment as CssAlignment, BorderCollapse, BorderStyle, BorderWidth,
        BoxSizing as CssBoxSizing, CaptionSide, Clear as CssClear, ColumnCount, ColumnFill,
        ColumnWidth, ComputedColor, ContainerType, Direction as CssDirection,
        Display as CssDisplay, FlexBasis as CssFlexBasis, FlexDirection as CssFlexDirection,
        FlexWrap as CssFlexWrap, Float as CssFloat, FontSize, Gap as CssGap,
        GridAutoFlow as CssGridAutoFlow, GridPlacement as CssGridPlacement,
        GridTemplate as CssGridTemplate, GridTrack as CssGridTrack, Inset, Length,
        LengthPercentage as CssLengthPercentage, LineHeight, Margin, Overflow as CssOverflow,
        Position as CssPosition, Radius, RelativeLengthEnvironment,
        ShapeOutside as CssShapeOutside, Size as CssSize, VerticalAlign, WhiteSpaceCollapse,
        WritingMode as CssWritingMode,
    },
};
use taffy::{
    geometry::{Line, Point, Rect, Size},
    prelude::{
        Dimension, LengthPercentage, LengthPercentageAuto, auto, fr, length, line, max_content,
        min_content, percent, span,
    },
    style::{
        AlignContent, AlignContentKeyword, AlignItems, AlignItemsKeyword, BoxSizing,
        Direction as TaffyDirection, Display, FlexBasis as TaffyFlexBasis, FlexDirection, FlexWrap,
        Float as TaffyFloat, GridAutoFlow, GridPlacement, GridTemplateComponent, JustifyContent,
        Overflow, Position, Style,
    },
};

type ImageSources = HashMap<String, Vec<u8>>;

use crate::{
    InteractionStates, LegacyDescendantAlignment, StylePlane, StyleSet, TextSystem,
    box_tree::GeneratedBoxTree,
    style::resolve_styles_with_containers,
    table_block::{
        CellBlockInput, CellFormatter, buckram_table_block, cell_content_block_size,
        commit_table_block, table_block_inputs, verify_table_block,
    },
    table_shadow::{
        DetachedTablePart, LIVE_ROOT_FONT_SIZE, PendingTable, TablePositioningGap,
        TablePositioningGapRecord, TableShadowLedger, buckram_table_columns,
        verify_assigned_columns,
    },
    table_sizing::collapsed_table_borders,
    table_wrapper::{grid_style, wrapper_style},
    text::{InlineLayout, InlineRequest, TextFrame},
};

mod build_block;
mod build_inline;
mod hit_testing;
mod positioned;
mod query;
mod retained;
mod tables;
mod taffy_style;
#[cfg(test)]
mod tests;
mod transaction;

use build_block::*;
use build_inline::*;
use positioned::*;
use retained::*;
use tables::*;
use taffy_style::*;
use transaction::*;
// The transaction and retained-root entries keep their crate and public
// paths: lib.rs, the document and its frame reach them as `layout::<name>`.
pub(crate) use retained::{layout_retained_formatting_root, retained_table_owner};
pub use transaction::{layout, layout_with_text_system, used_value_context};
// These four converters are crate vocabulary: text, paint, table sizing and
// the retained document all call them as `layout::<name>`. Re-exporting keeps
// those call sites exactly as they were.
pub(crate) use taffy_style::{
    border_width_px, length_percentage_px, line_height_px, signed_length_percentage_px,
};
// Crate and public surface the seams carried out with them: paint reads the
// table paint model and stacking order, and lib.rs re-exports both hit tests.
pub use hit_testing::{hit_test, hit_test_with_scroll};
pub(crate) use hit_testing::{order_modified_children, z_index_stacking_level};
pub(crate) use tables::TablePaintModel;

/// Physical geometry used at the DOM compatibility edge and by inline atoms.
pub(crate) type Fragment = PhysicalRect;

/// The static physical rectangle and content-scroll offset of one sticky
/// scrollport. The offset is added in layout space before ordinary scroll
/// painting translates its descendants back toward the viewport.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StickyScrollport {
    pub rect: PhysicalRect,
    pub offset: PhysicalOffset,
}

fn supports_retained_sticky_table_part(role: InternalTableRole) -> bool {
    matches!(
        role,
        InternalTableRole::Wrapper
            | InternalTableRole::Caption
            | InternalTableRole::RowGroup
            | InternalTableRole::HeaderGroup
            | InternalTableRole::FooterGroup
            | InternalTableRole::Row
            | InternalTableRole::Cell
    )
}

/// Table parts whose absolute/fixed fragments and static-position sources are
/// emitted outside K4d track sizing. Row groups, rows, and cells arrive from
/// the post-track zero-anchor formatter rather than from the table grid.
fn supports_shared_positioned_table_part(role: InternalTableRole) -> bool {
    matches!(
        role,
        InternalTableRole::Wrapper
            | InternalTableRole::Caption
            | InternalTableRole::RowGroup
            | InternalTableRole::HeaderGroup
            | InternalTableRole::FooterGroup
            | InternalTableRole::Row
            | InternalTableRole::Cell
    )
}

fn uses_zero_track_static_anchor(role: InternalTableRole) -> bool {
    matches!(
        role,
        InternalTableRole::RowGroup
            | InternalTableRole::HeaderGroup
            | InternalTableRole::FooterGroup
            | InternalTableRole::Row
            | InternalTableRole::Cell
    )
}

#[derive(Clone, Debug)]
struct AtomicSubtree {
    root: BoxId,
    fragments: Vec<AtomicFragment>,
    tables: TableFragmentPlane,
}

#[derive(Clone, Copy, Debug)]
struct AtomicFragment {
    box_id: BoxId,
    fragment: Fragment,
    static_fragment: Fragment,
    containing_block_area: Option<PhysicalRect>,
}

#[derive(Clone, Debug, Default)]
struct AtomicLayoutPlane {
    fragments: HashMap<BoxId, Fragment>,
    // An inline table participates in an ancestor intrinsic query with the
    // table grid's intrinsic pair, not the viewport-sized atomic fragment
    // produced for ordinary placement.
    intrinsic_inline: HashMap<BoxId, IntrinsicSizes>,
    // K4d5 table-grid first baselines, expressed from their inline-table
    // wrapper's margin-box block-start. Only inline-table wrappers populate
    // this map; other atomic boxes retain the existing block-end fallback.
    inline_baselines: HashMap<BoxId, f32>,
    subtrees: Vec<AtomicSubtree>,
    // Accumulated K4c5a shadow ledgers from each atomic root's BuildState.
    table_shadow: TableShadowLedger,
    table_paint: TablePaintPlane,
}

impl AtomicLayoutPlane {
    pub fn get(&self, box_id: BoxId) -> Option<&Fragment> {
        self.fragments.get(&box_id)
    }

    pub fn inline_baseline(&self, box_id: BoxId) -> Option<f32> {
        self.inline_baselines.get(&box_id).copied()
    }
}

impl<Id> crate::text::FragmentLookup<Id> for AtomicLayoutPlane
where
    Id: Copy + Eq + Hash,
{
    fn rect(&self, _id: Id) -> Option<&Fragment> {
        None
    }

    fn atomic_box_rect(&self, box_id: BoxId) -> Option<&Fragment> {
        self.get(box_id)
    }

    fn atomic_box_intrinsic_inline(&self, box_id: BoxId) -> Option<IntrinsicSizes> {
        self.intrinsic_inline.get(&box_id).copied()
    }

    fn atomic_box_baseline(&self, box_id: BoxId) -> Option<f32> {
        self.inline_baseline(box_id)
    }
}

/// Livery's retained wrapper around Buckram's standards-owned layout result.
#[derive(Clone, Debug)]
pub struct LiveryLayout<Id> {
    buckram: LayoutResult<Id>,
    text_frame: Option<TextFrame<Id>>,
    block_algorithms: BlockAlgorithmCounts,
    table_paint: TablePaintPlane,
    table_shadow: TableShadowLedger,
}

/// The outcome of attempting one bounded retained-root formatting pass.
/// `PromoteParent` is deliberately distinct from an unsupported root: only a
/// changed outer size can make the caller widen the retained replacement.
pub(crate) enum RetainedRootFormatting<Id> {
    Formatted(Box<LiveryLayout<Id>>),
    PromoteParent,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BlockAlgorithmCounts {
    pub buckram: usize,
    pub taffy: usize,
    /// Taffy block runs used only for intrinsic/backend scratch sizing.
    pub backend_sizing: usize,
}

impl BlockAlgorithmCounts {
    /// Taffy block runs caused by a CSS-facing deferral rather than scratch
    /// measurement. This is the admission metric for ancestor fallback.
    pub fn css_facing_taffy(self) -> usize {
        self.taffy.saturating_sub(self.backend_sizing)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayoutError(String);

impl fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl LayoutError {
    pub(crate) fn retained_state(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl Error for LayoutError {}

#[derive(Clone, Debug)]
struct TextMeasure {
    min_width: f32,
    max_width: f32,
    height: f32,
}

fn measure_text_algorithm_node(
    known: AlgorithmSize<Option<f32>>,
    available: AlgorithmSize<AlgorithmAvailableSpace>,
    context: Option<&mut TextMeasure>,
) -> AlgorithmSize<f32> {
    let Some(context) = context else {
        return AlgorithmSize::new(0.0, 0.0);
    };
    let available_width = match available.width {
        AlgorithmAvailableSpace::Definite(width) => width,
        AlgorithmAvailableSpace::MinContent => context.min_width,
        AlgorithmAvailableSpace::MaxContent => context.max_width,
    };
    AlgorithmSize::new(
        known
            .width
            .unwrap_or(context.max_width.min(available_width.max(0.0))),
        known.height.unwrap_or(context.height),
    )
}

struct InlineMeasure {
    owner: Option<BoxId>,
    roots: Vec<BoxId>,
    style: ComputedValues,
    width: f32,
    height: f32,
    /// Natural content-box dimensions for a directly measured replaced leaf.
    replaced_size: Option<(f32, f32)>,
    layouts: Vec<InlineLayoutEntry>,
    placement_constraints: Option<FloatLineConstraints>,
}

struct InlineLayoutEntry {
    width: f32,
    constraints: Option<FloatLineConstraints>,
    layout: InlineLayout<BoxId>,
}

#[derive(Clone, Copy)]
struct InlineMeasureGeometry<'a> {
    width: f32,
    intrinsic_kind: Option<IntrinsicSizeKind>,
    line_constraints: Option<&'a FloatLineConstraints>,
}

impl InlineMeasure {
    fn cached_size(
        &self,
        width: f32,
        constraints: Option<&FloatLineConstraints>,
    ) -> Option<(f32, f32)> {
        self.layouts
            .iter()
            .find(|entry| {
                (entry.width - width).abs() <= 0.01 && entry.constraints.as_ref() == constraints
            })
            .map(|entry| entry.layout.size())
    }

    fn remember(
        &mut self,
        width: f32,
        constraints: Option<&FloatLineConstraints>,
        layout: InlineLayout<BoxId>,
    ) -> (f32, f32) {
        let size = layout.size();
        self.layouts.push(InlineLayoutEntry {
            width,
            constraints: constraints.cloned(),
            layout,
        });
        size
    }

    fn layout_for_width(&self, width: f32) -> Option<&InlineLayout<BoxId>> {
        self.layouts
            .iter()
            .filter(|entry| entry.constraints.as_ref() == self.placement_constraints.as_ref())
            .min_by(|left, right| {
                (left.width - width)
                    .abs()
                    .total_cmp(&(right.width - width).abs())
            })
            .map(|entry| &entry.layout)
    }
}

fn measure_inline_context<D>(
    text: &mut TextSystem,
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    boxes: &GeneratedBoxTree<D::NodeId>,
    atomic: &AtomicLayoutPlane,
    context: &mut InlineMeasure,
    geometry: InlineMeasureGeometry<'_>,
) -> (f32, f32)
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    if let Some(size) = context.replaced_size {
        return size;
    }
    let InlineMeasureGeometry {
        width,
        intrinsic_kind,
        line_constraints: constraints,
    } = geometry;
    if let Some(constraints) = constraints {
        context.placement_constraints = Some(constraints.clone());
    }
    context.cached_size(width, constraints).unwrap_or_else(|| {
        let formatted = text.format_inline_group(
            dom,
            styles,
            boxes,
            atomic,
            InlineRequest {
                roots: &context.roots,
                parent_style: &context.style,
                width,
                intrinsic_kind,
                line_constraints: constraints,
            },
        );
        formatted.map_or((context.width, context.height), |layout| {
            context.remember(width, constraints, layout)
        })
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the measured inline context needs its formatter, DOM, style, box, and atomic inputs"
)]
fn measure_inline_algorithm_node<D>(
    text: &mut TextSystem,
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    boxes: &GeneratedBoxTree<D::NodeId>,
    atomic: &AtomicLayoutPlane,
    intrinsic_sizes: &mut IntrinsicSizeCache,
    known: AlgorithmSize<Option<f32>>,
    available: AlgorithmSize<AlgorithmAvailableSpace>,
    context: Option<&mut InlineMeasure>,
    line_constraints: Option<&FloatLineConstraints>,
) -> AlgorithmSize<f32>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let Some(context) = context else {
        return AlgorithmSize::new(0.0, 0.0);
    };
    if let Some((width, height)) = context.replaced_size {
        return AlgorithmSize::new(known.width.unwrap_or(width), known.height.unwrap_or(height));
    }
    let (query_width, definite_cap, intrinsic_kind) = match available.width {
        AlgorithmAvailableSpace::Definite(width) => (width, Some(width), None),
        // A nearly-zero line asks Parley to break at every legal
        // opportunity while retaining each unbreakable item's width.
        AlgorithmAvailableSpace::MinContent => (0.01, None, Some(IntrinsicSizeKind::MinContent)),
        // An infinite line suppresses wrapping and yields max-content.
        AlgorithmAvailableSpace::MaxContent => {
            (f32::INFINITY, None, Some(IntrinsicSizeKind::MaxContent))
        },
    };
    let intrinsic_width = intrinsic_kind.and_then(|kind| {
        let owner = context.owner?;
        let query = IntrinsicSizeQuery::new(owner, LogicalAxis::Inline, kind);
        intrinsic_sizes.get(query).or_else(|| {
            let min_content = measure_inline_context(
                text,
                dom,
                styles,
                boxes,
                atomic,
                context,
                InlineMeasureGeometry {
                    width: 0.01,
                    intrinsic_kind: Some(IntrinsicSizeKind::MinContent),
                    line_constraints: None,
                },
            )
            .0;
            let max_content = measure_inline_context(
                text,
                dom,
                styles,
                boxes,
                atomic,
                context,
                InlineMeasureGeometry {
                    width: f32::INFINITY,
                    intrinsic_kind: Some(IntrinsicSizeKind::MaxContent),
                    line_constraints: None,
                },
            )
            .0;
            let sizes = IntrinsicSizes::new(min_content, max_content)?;
            let result = sizes.get(kind);
            intrinsic_sizes.insert(owner, LogicalAxis::Inline, sizes);
            Some(result)
        })
    });
    let requested_width = known.width.or(intrinsic_width).unwrap_or(query_width);
    let (measured_width, measured_height) = measure_inline_context(
        text,
        dom,
        styles,
        boxes,
        atomic,
        context,
        InlineMeasureGeometry {
            width: requested_width,
            intrinsic_kind,
            line_constraints: intrinsic_kind
                .is_none()
                .then_some(line_constraints)
                .flatten(),
        },
    );
    AlgorithmSize::new(
        known.width.unwrap_or_else(|| {
            intrinsic_width.unwrap_or_else(|| {
                definite_cap.map_or(measured_width, |cap| measured_width.min(cap.max(0.0)))
            })
        }),
        known.height.unwrap_or(measured_height),
    )
}

/// Compare one table's Buckram result against its painted fragments, in both
/// axes.
///
/// Buckram's rectangles are relative to the table grid's own origin, so the
/// block-axis comparison subtracts the grid's painted position. Without that
/// the table's place in the page would be reported as a table-layout
/// disagreement on every cell.
fn verify_one_table<Id>(
    pending: &PendingTable<Id>,
    live_rect_of: &impl Fn(BoxId) -> Option<Fragment>,
    ledger: &mut TableShadowLedger,
) {
    if let Some(assigned) = pending.assigned.as_ref().map(|inline| &inline.column_sizes) {
        let live = pending
            .grid
            .columns
            .iter()
            .enumerate()
            .map(|(index, _)| {
                pending
                    .grid
                    .cells
                    .iter()
                    .find(|cell| cell.column == index && cell.column_span == 1)
                    .and_then(|cell| live_rect_of(cell.source))
                    .map(|rect| rect.width)
            })
            .collect::<Vec<_>>();
        verify_assigned_columns(pending.table, assigned, &live, ledger);
    }
    let (Some(block), Some(grid)) = (pending.block.as_ref(), live_rect_of(pending.table)) else {
        return;
    };
    verify_table_block(
        pending.table,
        block,
        |box_id| live_rect_of(box_id).map(|rect| (rect.y - grid.y, rect.height)),
        &mut ledger.block,
    );
}

/// Format one table cell for Buckram's block pipeline.
///
/// The cell is laid out at exactly the content inline size K4c assigned, with
/// its own inline and block constraints neutralized: Buckram applies those
/// itself, and a floor applied here would come back as measured content. The
/// cell's specified block size reaches Buckram as a row constraint, never as
/// a taller content box, which is why the measurement pass leaves the height
/// automatic.
fn format_table_cell<Context, Source>(
    tree: &mut AlgorithmTree<Style, Context, Source>,
    node: AlgorithmNodeId,
    request: TableCellLayoutInput,
    cell: &CellBlockInput,
    mut measure: impl FnMut(&mut Context, InlineMeasureGeometry<'_>) -> (f32, f32),
) -> TableCellLayoutOutput {
    let offsets = cell.style.offsets;
    let style = tree.style_mut(node);
    let saved = (style.size, style.min_size, style.max_size, style.box_sizing);
    style.box_sizing = BoxSizing::ContentBox;
    style.size.width = Dimension::length(request.content_inline_size);
    style.min_size.width = Dimension::auto();
    style.max_size.width = Dimension::auto();
    style.min_size.height = Dimension::auto();
    style.max_size.height = Dimension::auto();
    style.size.height = match request.pass {
        TableCellLayoutPass::Measure => Dimension::auto(),
        // The percentage pass supplies a used border-box block size; the
        // cell's content box is that minus the offsets Buckram already knows.
        TableCellLayoutPass::ResolvePercentages { cell_block_size } => {
            Dimension::length(cell_content_block_size(cell_block_size, offsets))
        },
    };
    tree.compute_layout_with_measure_excluding_out_of_flow_children(
        node,
        AlgorithmSize::new(
            AlgorithmAvailableSpace::Definite(request.content_inline_size),
            AlgorithmAvailableSpace::MaxContent,
        ),
        |known, available, _, context, _| {
            let Some(context) = context else {
                return AlgorithmSize::new(0.0, 0.0);
            };
            let (width, intrinsic_kind) = match available.width {
                AlgorithmAvailableSpace::Definite(width) => (width, None),
                AlgorithmAvailableSpace::MinContent => (0.01, Some(IntrinsicSizeKind::MinContent)),
                AlgorithmAvailableSpace::MaxContent => {
                    (f32::INFINITY, Some(IntrinsicSizeKind::MaxContent))
                },
            };
            let (measured_width, measured_height) = measure(
                context,
                InlineMeasureGeometry {
                    width: known.width.unwrap_or(width),
                    intrinsic_kind,
                    line_constraints: None,
                },
            );
            AlgorithmSize::new(
                known.width.unwrap_or(measured_width),
                known.height.unwrap_or(measured_height),
            )
        },
    );
    let border_box = tree.unrounded_layout(node).height;
    let baselines = tree.baselines(node);
    let style = tree.style_mut(node);
    (style.size, style.min_size, style.max_size, style.box_sizing) = saved;
    TableCellLayoutOutput {
        content_block_size: cell_content_block_size(border_box, offsets),
        // CSS 2.1 section 10.7 leaves min-height and max-height undefined on
        // a table cell, and the K4d4c matrix measured both engines ignoring
        // them outright, so a cell carries no border-box floor of its own.
        border_box_min_block_size: 0.0,
        // Live descendants stay in the backend tree, which fragment
        // collection already walks. K4d6a's drafts exist for adapters with no
        // such tree, and a zero rectangle unions into the cell's own without
        // changing it.
        baselines,
        overflow: LogicalRect::default(),
        fragments: FragmentDraftTree::default(),
    }
}

/// Feed retained IFC line baselines into Buckram before fragment collection.
/// Parent block, flex, and grid contexts consume these declared outputs
/// through `AlgorithmTree::propagate_declared_baselines`; no post-layout
/// traversal of backend children is involved.
fn populate_inline_baselines(tree: &mut AlgorithmTree<Style, InlineMeasure, Vec<BoxId>>) {
    let direct = tree
        .node_ids()
        .filter_map(|node| {
            let width = tree.layout(node).width;
            tree.context(node)
                .and_then(|context| context.layout_for_width(width))
                .and_then(|layout| layout.baselines())
                .and_then(|(first, last)| Baselines::new(Some(first), Some(last)))
                .map(|baselines| (node, baselines))
        })
        .collect::<Vec<_>>();
    for (node, baselines) in direct {
        tree.set_baselines(node, baselines);
    }
    tree.propagate_declared_baselines();
}

/// Ask the formatter for the admitted intrinsic pair of each positioned
/// root. The map is keyed by Buckram box identity, never a backend node or a
/// completed normal-flow rectangle.
fn positioned_intrinsic_sizes<Context, Source>(
    tree: &mut AlgorithmTree<Style, Context, Source>,
    candidates: &[(BoxId, AlgorithmNodeId)],
    mut measure: impl FnMut(
        AlgorithmSize<Option<f32>>,
        AlgorithmSize<AlgorithmAvailableSpace>,
        AlgorithmNodeId,
        Option<&mut Context>,
        Option<&FloatLineConstraints>,
    ) -> AlgorithmSize<f32>,
) -> HashMap<BoxId, IntrinsicSizes> {
    candidates
        .iter()
        .filter_map(|(box_id, node)| {
            tree.positioned_intrinsic_inline_sizes(
                *node,
                |known, available, node, context, lines| {
                    measure(known, available, node, context, lines)
                },
            )
            .map(|sizes| (*box_id, sizes))
        })
        .collect()
}

type ResolvedLayout<Id> = (StylePlane<Id>, LiveryLayout<Id>);

#[derive(Clone, Copy, Debug, Default)]
struct ContainerBases {
    width: Option<f32>,
    height: Option<f32>,
    inline: Option<f32>,
    block: Option<f32>,
}

/// Resolve deferred container-relative units from the nearest eligible
/// ancestor content boxes. A fallback pass supplies small-viewport values so
/// Taffy can establish those boxes without consuming unresolved units.
pub fn resolve_container_relative_styles<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    viewport: ViewportSizes,
) -> Result<StylePlane<D::NodeId>, LayoutError>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    resolve_container_relative_styles_with_images(dom, styles, viewport, &ImageSources::new())
}

/// Iterate size-query cascade and container-unit resolution until the style
/// plane stabilizes. The pass is bounded so cyclic queries cannot hang a
/// frame; the final bounded state is laid out normally.
pub fn resolve_container_query_styles<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    style_set: &StyleSet,
    device: &Device,
    interactions: &InteractionStates<D::NodeId>,
) -> Result<StylePlane<D::NodeId>, LayoutError>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    resolve_container_query_styles_with_images(
        dom,
        styles,
        style_set,
        device,
        interactions,
        &ImageSources::new(),
    )
}

/// Resolve container-query styles while retaining the caller-owned image
/// ledger for intrinsic-size resolution.
pub fn resolve_container_query_styles_with_images<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    style_set: &StyleSet,
    device: &Device,
    interactions: &InteractionStates<D::NodeId>,
    image_sources: &ImageSources,
) -> Result<StylePlane<D::NodeId>, LayoutError>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    if !style_set.has_container_queries() {
        return resolve_container_relative_styles_with_images(
            dom,
            styles,
            device.viewport_sizes,
            image_sources,
        );
    }
    let mut current = styles.clone();
    for _ in 0..8 {
        let resolved = resolve_container_relative_styles_with_images(
            dom,
            &current,
            device.viewport_sizes,
            image_sources,
        )?;
        let fragments = layout_impl(
            dom,
            &resolved,
            device.viewport_width,
            device.viewport_height,
            image_sources,
        )?;
        let containers = container_snapshots(dom, &resolved, &fragments);
        let next =
            resolve_styles_with_containers(dom, style_set, device, interactions, &containers);
        if next == current {
            return Ok(resolved);
        }
        current = next;
    }
    resolve_container_relative_styles_with_images(
        dom,
        &current,
        device.viewport_sizes,
        image_sources,
    )
}

fn container_snapshots<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
) -> HashMap<D::NodeId, Vec<ContainerSnapshot>>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let mut snapshots = HashMap::new();
    collect_container_snapshots(dom, dom.document(), styles, fragments, &[], &mut snapshots);
    snapshots
}

fn collect_container_snapshots<D>(
    dom: &D,
    id: D::NodeId,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    ancestors: &[ContainerSnapshot],
    snapshots: &mut HashMap<D::NodeId, Vec<ContainerSnapshot>>,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let mut descendants = ancestors.to_vec();
    if dom.kind(id) == NodeKind::Element {
        snapshots.insert(id, ancestors.to_vec());
        if let (Some(style), Some(fragment)) = (styles.get(id), fragments.get(id))
            && style.container_type != ContainerType::Normal
        {
            let (width, height) = content_box_size(style, fragment);
            let (inline_size, block_size) = if style.writing_mode.is_vertical() {
                (height, width)
            } else {
                (width, height)
            };
            descendants.insert(
                0,
                ContainerSnapshot {
                    names: style.container_name.names().to_vec(),
                    container_type: style.container_type,
                    writing_mode: style.writing_mode,
                    width,
                    height,
                    inline_size,
                    block_size,
                },
            );
        }
    }
    for child in dom.flat_children(id) {
        collect_container_snapshots(dom, child, styles, fragments, &descendants, snapshots);
    }
}

fn resolve_container_relative_styles_with_images<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    viewport: ViewportSizes,
    image_sources: &ImageSources,
) -> Result<StylePlane<D::NodeId>, LayoutError>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let mut fallback = styles.clone();
    resolve_relative_subtree(
        dom,
        dom.document(),
        &mut fallback,
        RelativeLengthEnvironment::container_fallback(viewport),
    );
    if fallback == *styles {
        return Ok(styles.clone());
    }
    let fragments = layout_impl(
        dom,
        &fallback,
        viewport.dynamic.width,
        viewport.dynamic.height,
        image_sources,
    )?;

    let mut resolved = styles.clone();
    resolve_container_subtree(
        dom,
        dom.document(),
        &mut resolved,
        &fragments,
        viewport,
        ContainerBases::default(),
    );
    Ok(resolved)
}

fn resolve_relative_subtree<D>(
    dom: &D,
    id: D::NodeId,
    styles: &mut StylePlane<D::NodeId>,
    environment: RelativeLengthEnvironment,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let environment = environment.with_vertical_writing(
        styles
            .get(id)
            .is_some_and(|style| style.writing_mode.is_vertical()),
    );
    styles.resolve_relative_lengths(id, environment);
    for child in dom.flat_children(id) {
        resolve_relative_subtree(dom, child, styles, environment);
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_container_subtree<D>(
    dom: &D,
    id: D::NodeId,
    styles: &mut StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    viewport: ViewportSizes,
    bases: ContainerBases,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let vertical_writing = styles
        .get(id)
        .is_some_and(|style| style.writing_mode.is_vertical());
    styles.resolve_relative_lengths(
        id,
        RelativeLengthEnvironment::container_axes(
            viewport,
            bases.width,
            bases.height,
            bases.inline,
            bases.block,
            vertical_writing,
        ),
    );

    let mut next = bases;
    if let (Some(style), Some(fragment)) = (styles.get(id), fragments.get(id)) {
        let (width, height) = content_box_size(style, fragment);
        let vertical = style.writing_mode.is_vertical();
        let (inline_size, block_size) = if vertical {
            (height, width)
        } else {
            (width, height)
        };
        match style.container_type {
            ContainerType::Normal => {},
            ContainerType::InlineSize => {
                next.inline = Some(inline_size);
                if vertical {
                    next.height = Some(height);
                } else {
                    next.width = Some(width);
                }
            },
            ContainerType::Size => {
                next.width = Some(width);
                next.height = Some(height);
                next.inline = Some(inline_size);
                next.block = Some(block_size);
            },
        }
    }

    for child in dom.flat_children(id) {
        resolve_container_subtree(dom, child, styles, fragments, viewport, next);
    }
}

/// Return a fragment's physical content-box size after its computed padding
/// and borders are removed.
pub fn content_box_size(style: &ComputedValues, fragment: &TreeFragment) -> (f32, f32) {
    let em = match style.font_size {
        FontSize::Value(CssLengthPercentage::Length(Length {
            value,
            unit: livery::values::LengthUnit::Px,
        })) => value,
        _ => 16.0,
    };
    let padding_left = length_percentage_px(style.padding_left.0, em, fragment.width);
    let padding_right = length_percentage_px(style.padding_right.0, em, fragment.width);
    let padding_top = length_percentage_px(style.padding_top.0, em, fragment.width);
    let padding_bottom = length_percentage_px(style.padding_bottom.0, em, fragment.width);
    let border_left = border_width_px(style.border_left_style, style.border_left_width, em);
    let border_right = border_width_px(style.border_right_style, style.border_right_width, em);
    let border_top = border_width_px(style.border_top_style, style.border_top_width, em);
    let border_bottom = border_width_px(style.border_bottom_style, style.border_bottom_width, em);
    (
        (fragment.width - padding_left - padding_right - border_left - border_right).max(0.0),
        (fragment.height - padding_top - padding_bottom - border_top - border_bottom).max(0.0),
    )
}

struct FragmentOutput<'a> {
    fragments: &'a mut FragmentTree,
}

#[derive(Clone, Copy)]
struct FragmentCursor {
    origin: Point<f32>,
    containing: Fragment,
    parent: Option<FragmentId>,
}

fn intrinsic_owner_for_flow_children<Id>(
    boxes: &GeneratedBoxTree<Id>,
    parent: BoxId,
    children: &[BoxId],
) -> Option<BoxId>
where
    Id: Copy + Eq + Hash,
{
    let mut groups = 0;
    let mut inside_group = false;
    for child in children {
        if box_is_inline(boxes, *child) {
            if !inside_group {
                groups += 1;
                inside_group = true;
            }
        } else {
            inside_group = false;
        }
    }
    (groups == 1).then_some(parent)
}

fn box_is_inline<Id>(boxes: &GeneratedBoxTree<Id>, box_id: BoxId) -> bool
where
    Id: Copy + Eq + Hash,
{
    let css_box = &boxes[box_id];
    // K4e4 deleted K4a's wrapper/grid exclusion here: an inline-table's
    // wrapper is inline-level and rides the atomic-inline lane. The grid
    // never reaches this test, because a wrapper's children are built
    // directly rather than through flow-children grouping.
    css_box.display.outside == Some(DisplayOutside::Inline)
        && css_box.float == FloatSide::None
        && matches!(
            css_box.positioning,
            PositioningScheme::Static | PositioningScheme::Relative | PositioningScheme::Sticky
        )
}

/// Return each outermost absolute or fixed descendant of an inline run.
///
/// The run itself remains with the inline formatter, which supplies the line
/// fragment used as the positioned root's static-position source. The root
/// must be built separately, because out-of-flow contents neither occupy that
/// line nor inherit its measured width.
fn positioned_roots_in_inline_group<Id>(boxes: &GeneratedBoxTree<Id>, roots: &[BoxId]) -> Vec<BoxId>
where
    Id: Copy + Eq + Hash,
{
    fn visit<Id>(boxes: &GeneratedBoxTree<Id>, box_id: BoxId, positioned: &mut Vec<BoxId>)
    where
        Id: Copy + Eq + Hash,
    {
        for child in boxes[box_id].children() {
            if matches!(
                boxes[*child].positioning,
                PositioningScheme::Absolute | PositioningScheme::Fixed
            ) {
                positioned.push(*child);
                continue;
            }
            visit(boxes, *child, positioned);
        }
    }

    let mut positioned = Vec::new();
    for root in roots {
        visit(boxes, *root, &mut positioned);
    }
    positioned
}

fn anonymous_block_style<Id>(boxes: &GeneratedBoxTree<Id>, box_id: BoxId) -> BlockStyle
where
    Id: Copy + Eq + Hash,
{
    let flow = boxes[box_id].flow;
    let containing_flow = boxes[box_id]
        .parent()
        .map_or(flow, |parent| boxes[parent].flow);
    BlockStyle::anonymous(flow, containing_flow)
}

fn to_block_style<Id>(
    boxes: &buckram::CssBoxTree<Id>,
    styles: &StylePlane<Id>,
    box_id: BoxId,
    computed: &ComputedValues,
    font_size: f32,
) -> BlockStyle
where
    Id: Copy + Eq + Hash,
{
    let css_box = &boxes[box_id];
    let containing_flow = css_box
        .parent()
        .map_or(FlowAxes::HORIZONTAL_LTR, |parent| boxes[parent].flow);
    let mut size_containment = match computed.container_type {
        ContainerType::Normal => BlockDimensions::new(false, false),
        ContainerType::InlineSize if computed.writing_mode.is_vertical() => {
            BlockDimensions::new(false, true)
        },
        ContainerType::InlineSize => BlockDimensions::new(true, false),
        ContainerType::Size => BlockDimensions::new(true, true),
    };
    if computed.contain.has_size() {
        size_containment = BlockDimensions::new(true, true);
    } else if computed.contain.has_inline_size() {
        if computed.writing_mode.is_vertical() {
            size_containment.height = true;
        } else {
            size_containment.width = true;
        }
    }
    let establishes_bfc = matches!(
        css_box.display.inside,
        Some(DisplayInside::FlowRoot | DisplayInside::Flex | DisplayInside::Grid)
    ) || matches!(
        computed.display,
        CssDisplay::InlineBlock
            | CssDisplay::Table
            | CssDisplay::InlineTable
            | CssDisplay::TableCell
            | CssDisplay::TableCaption
    ) || matches!(
        computed.position,
        CssPosition::Absolute | CssPosition::Fixed
    ) || computed.float != CssFloat::None
        || computed.overflow_x != CssOverflow::Visible
        || computed.overflow_y != CssOverflow::Visible
        || computed.contain.has_layout()
        || computed.contain.has_paint();
    let shape_reference_box = shape_outside_reference_box(computed);
    let nonlinear_shape_radius = shape_outside_has_nonlinear_radius(computed);

    BlockStyle {
        flow: css_box.flow,
        containing_flow,
        size: BlockDimensions::new(
            block_size_value(computed.width, font_size),
            block_size_value(computed.height, font_size),
        ),
        min_size: BlockDimensions::new(
            block_size_value(computed.min_width, font_size),
            block_size_value(computed.min_height, font_size),
        ),
        max_size: BlockDimensions::new(
            block_size_value(computed.max_width, font_size),
            block_size_value(computed.max_height, font_size),
        ),
        margin: PhysicalSides {
            top: block_margin(computed.margin_top, font_size),
            right: block_margin(computed.margin_right, font_size),
            bottom: block_margin(computed.margin_bottom, font_size),
            left: block_margin(computed.margin_left, font_size),
        },
        inset: PhysicalSides {
            top: block_inset(computed.top, font_size),
            right: block_inset(computed.right, font_size),
            bottom: block_inset(computed.bottom, font_size),
            left: block_inset(computed.left, font_size),
        },
        padding: PhysicalSides {
            top: flow_length(computed.padding_top.0, font_size),
            right: flow_length(computed.padding_right.0, font_size),
            bottom: flow_length(computed.padding_bottom.0, font_size),
            left: flow_length(computed.padding_left.0, font_size),
        },
        border: PhysicalSides {
            top: border_width_px(
                computed.border_top_style,
                computed.border_top_width,
                font_size,
            ),
            right: border_width_px(
                computed.border_right_style,
                computed.border_right_width,
                font_size,
            ),
            bottom: border_width_px(
                computed.border_bottom_style,
                computed.border_bottom_width,
                font_size,
            ),
            left: border_width_px(
                computed.border_left_style,
                computed.border_left_width,
                font_size,
            ),
        },
        box_sizing: match computed.box_sizing {
            CssBoxSizing::ContentBox => BlockBoxSizing::ContentBox,
            CssBoxSizing::BorderBox => BlockBoxSizing::BorderBox,
        },
        position: match computed.position {
            CssPosition::Static => BuckramBlockPosition::Static,
            CssPosition::Relative => BuckramBlockPosition::Relative,
            CssPosition::Absolute => BuckramBlockPosition::Absolute,
            CssPosition::Fixed => BuckramBlockPosition::Fixed,
            CssPosition::Sticky => BuckramBlockPosition::Sticky,
        },
        float: match computed.float {
            CssFloat::None => FloatSide::None,
            CssFloat::Left => FloatSide::Left,
            CssFloat::Right => FloatSide::Right,
        },
        float_reference_box: shape_reference_box.unwrap_or(FloatReferenceBox::MarginBox),
        has_shape_outside_box: shape_reference_box.is_some() && !nonlinear_shape_radius,
        corner_radii: BlockCornerRadii {
            top_left: block_corner_radius(computed.border_top_left_radius, font_size),
            top_right: block_corner_radius(computed.border_top_right_radius, font_size),
            bottom_right: block_corner_radius(computed.border_bottom_right_radius, font_size),
            bottom_left: block_corner_radius(computed.border_bottom_left_radius, font_size),
        },
        clear: match computed.clear {
            CssClear::None => ClearSide::None,
            CssClear::Left => ClearSide::Left,
            CssClear::Right => ClearSide::Right,
            CssClear::Both => ClearSide::Both,
        },
        establishes_bfc,
        shrink_to_fit: matches!(computed.width, CssSize::Auto)
            && (computed.display == CssDisplay::InlineBlock || computed.float != CssFloat::None),
        replaced: css_box.replaced,
        aspect_ratio: computed.aspect_ratio.preferred_ratio(),
        size_containment,
        has_nonlinear_lengths: block_style_has_nonlinear_lengths(computed),
        overconstrained_inline_alignment: boxes
            .origin_node(box_id)
            .and_then(|id| styles.legacy_descendant_alignment(id))
            .map(|alignment| match alignment {
                LegacyDescendantAlignment::LineLeft => OverconstrainedInlineAlignment::LineLeft,
                LegacyDescendantAlignment::Center => OverconstrainedInlineAlignment::Center,
                LegacyDescendantAlignment::LineRight => OverconstrainedInlineAlignment::LineRight,
            }),
        is_root_element: css_box.parent().is_none()
            && matches!(css_box.origin, BoxOrigin::Element(_)),
    }
}

fn shape_outside_reference_box(computed: &ComputedValues) -> Option<FloatReferenceBox> {
    match computed.shape_outside {
        CssShapeOutside::None => None,
        CssShapeOutside::MarginBox => Some(FloatReferenceBox::MarginBox),
        CssShapeOutside::BorderBox => Some(FloatReferenceBox::BorderBox),
        CssShapeOutside::PaddingBox => Some(FloatReferenceBox::PaddingBox),
        CssShapeOutside::ContentBox => Some(FloatReferenceBox::ContentBox),
    }
}

fn block_corner_radius(radius: Radius, em: f32) -> BlockCornerRadius {
    let value = flow_length(radius.0, em);
    BlockCornerRadius {
        horizontal: value,
        vertical: value,
    }
}

fn shape_outside_has_nonlinear_radius(computed: &ComputedValues) -> bool {
    shape_outside_reference_box(computed).is_some()
        && [
            computed.border_top_left_radius,
            computed.border_top_right_radius,
            computed.border_bottom_right_radius,
            computed.border_bottom_left_radius,
        ]
        .into_iter()
        .any(|radius| length_has_math(radius.0))
}

fn block_size_value(value: CssSize, em: f32) -> BlockSizeValue {
    match value {
        CssSize::Auto => BlockSizeValue::Auto,
        CssSize::None => BlockSizeValue::None,
        CssSize::MinContent => BlockSizeValue::MinContent,
        CssSize::MaxContent => BlockSizeValue::MaxContent,
        CssSize::FitContent(value) => BlockSizeValue::FitContent(flow_length(value, em)),
        CssSize::Value(value) => BlockSizeValue::Length(flow_length(value, em)),
    }
}

fn block_margin(value: Margin, em: f32) -> FlowLengthAuto {
    match value {
        Margin::Auto => FlowLengthAuto::Auto,
        Margin::Value(value) => FlowLengthAuto::Value(flow_length(value, em)),
    }
}

fn block_inset(value: Inset, em: f32) -> FlowLengthAuto {
    match value {
        Inset::Auto => FlowLengthAuto::Auto,
        Inset::Value(value) => FlowLengthAuto::Value(flow_length(value, em)),
    }
}

fn flow_length(value: CssLengthPercentage, em: f32) -> FlowLength {
    let px = absolute_length_percentage(value, em, 16.0, 0.0);
    let with_unit_basis = absolute_length_percentage(value, em, 16.0, 1.0);
    FlowLength {
        px,
        percentage: with_unit_basis - px,
    }
}

fn block_style_has_nonlinear_lengths(computed: &ComputedValues) -> bool {
    let size_has_math = |size| match size {
        CssSize::FitContent(value) | CssSize::Value(value) => length_has_math(value),
        CssSize::Auto | CssSize::None | CssSize::MinContent | CssSize::MaxContent => false,
    };
    let margin_has_math = |margin| match margin {
        Margin::Value(value) => length_has_math(value),
        Margin::Auto => false,
    };

    [
        computed.width,
        computed.height,
        computed.min_width,
        computed.min_height,
        computed.max_width,
        computed.max_height,
    ]
    .into_iter()
    .any(size_has_math)
        || [
            computed.margin_top,
            computed.margin_right,
            computed.margin_bottom,
            computed.margin_left,
        ]
        .into_iter()
        .any(margin_has_math)
        || [
            computed.padding_top.0,
            computed.padding_right.0,
            computed.padding_bottom.0,
            computed.padding_left.0,
        ]
        .into_iter()
        .any(length_has_math)
}

fn length_has_math(value: CssLengthPercentage) -> bool {
    matches!(value, CssLengthPercentage::Math(_))
}

fn supports_nested_float_state<Id>(
    css_box: &CssBox<Id>,
    block_style: BlockStyle,
    kind: AlgorithmKind,
) -> bool {
    // A relative block remains in normal flow. Livery translates its retained
    // fragment subtree only after Buckram has resolved the shared float state.
    kind == AlgorithmKind::Block
        && css_box.display.outside == Some(DisplayOutside::Block)
        && css_box.display.internal_table.is_none()
        && !block_style.establishes_bfc
        && matches!(
            block_style.position,
            BuckramBlockPosition::Static | BuckramBlockPosition::Relative
        )
        && block_style.float == FloatSide::None
        && !block_style.replaced
        // Buckram mirrors exclusions across horizontal direction changes.
        // Vertical writing modes still need a full axis transform.
        && (block_style.flow == block_style.containing_flow
            || block_style.flow.is_horizontal() && block_style.containing_flow.is_horizontal())
}

fn supports_float_avoidance<Id>(
    css_box: &CssBox<Id>,
    block_style: BlockStyle,
    kind: AlgorithmKind,
) -> bool {
    matches!(
        kind,
        AlgorithmKind::Leaf | AlgorithmKind::Block | AlgorithmKind::Flex | AlgorithmKind::Grid
    ) && (css_box.display.outside == Some(DisplayOutside::Block)
        || (css_box.display.outside == Some(DisplayOutside::Inline) && block_style.shrink_to_fit))
        && matches!(
            css_box.display.inside,
            Some(
                DisplayInside::Flow
                    | DisplayInside::FlowRoot
                    | DisplayInside::Flex
                    | DisplayInside::Grid
            ) | None
        )
        && css_box.display.internal_table.is_none()
        && block_style.establishes_bfc
        && block_style.position == BuckramBlockPosition::Static
        && block_style.float == FloatSide::None
        && !block_style.replaced
        && block_style.flow.is_horizontal()
        && block_style.containing_flow.is_horizontal()
}

fn supports_intrinsic_shrink_to_fit<Id, Context, Source>(
    tree: &AlgorithmTree<Style, Context, Source>,
    node: AlgorithmNodeId,
    boxes: &GeneratedBoxTree<Id>,
    box_id: BoxId,
    computed: &ComputedValues,
    block_style: BlockStyle,
    kind: AlgorithmKind,
) -> bool {
    let css_box = &boxes[box_id];
    let float_root = css_box.display.outside == Some(DisplayOutside::Block)
        && block_style.float != FloatSide::None;
    let atomic_inline_root = css_box.display.outside == Some(DisplayOutside::Inline)
        && block_style.float == FloatSide::None;
    // A flex item is blockified before its contents are formatted. It must
    // not enter the atomic inline shrink-to-fit lane merely because its
    // authored outside display was inline (for example, a canvas flex item).
    let direct_flex_item = css_box
        .parent()
        .is_some_and(|parent| boxes[parent].display.inside == Some(DisplayInside::Flex));
    matches!(
        kind,
        AlgorithmKind::Block | AlgorithmKind::Leaf | AlgorithmKind::Flex | AlgorithmKind::Grid
    ) && matches!(
        css_box.display.inside,
        Some(
            DisplayInside::Flow
                | DisplayInside::FlowRoot
                | DisplayInside::Flex
                | DisplayInside::Grid
        )
    ) && css_box.display.internal_table.is_none()
        && block_style.position == BuckramBlockPosition::Static
        && block_style.shrink_to_fit
        && computed.vertical_align == VerticalAlign::Baseline
        && block_style.flow.is_horizontal()
        && block_style.containing_flow.is_horizontal()
        && (float_root || (atomic_inline_root && !direct_flex_item))
        && tree.supports_intrinsic_shrink_to_fit(node)
}

fn algorithm_kind<Id>(css_box: &CssBox<Id>, leaf: bool) -> AlgorithmKind {
    if leaf {
        return AlgorithmKind::Leaf;
    }
    match (css_box.formatting_context, css_box.display.internal_table) {
        (_, Some(InternalTableRole::Grid)) => AlgorithmKind::Table,
        (Some(FormattingContextKind::Flex), _) => AlgorithmKind::Flex,
        (Some(FormattingContextKind::Grid), _) => AlgorithmKind::Grid,
        _ => AlgorithmKind::Block,
    }
}

/// The grid box a table wrapper splits its computed values with.
fn wrapped_table_grid<Id>(boxes: &GeneratedBoxTree<Id>, wrapper: BoxId) -> Option<BoxId>
where
    Id: Copy + Eq + Hash,
{
    boxes[wrapper]
        .children()
        .iter()
        .copied()
        .find(|child| boxes[*child].display.internal_table == Some(InternalTableRole::Grid))
}

/// Anonymous tables inherit inheritable values from their parent and take
/// initial values for everything else.
fn anonymous_table_style(inherited: Option<&ComputedValues>) -> ComputedValues {
    let mut style = inherited.map(ComputedValues::for_child).unwrap_or_default();
    style.display = CssDisplay::Table;
    style
}

/// The wrapper's width under CSS Tables 3 section 2.2.1: "the width of the
/// table wrapper box is the border-edge width of the table grid box inside
/// it."
///
/// A definite table width makes that computable before layout, which is what
/// stops a wrapper from stretching when it is a flex or grid item - it is
/// never `auto` in the sense stretching means, because the grid decides it.
/// K4e2 resolves the `auto` case from Buckram's own table sizing instead of
/// guessing at it, but that runs after the tree is built, and a flex item's
/// style has to be right before it.
fn wrapper_width_from_grid(grid: &Style) -> Option<Dimension> {
    let width = grid.size.width.into_option()?;
    if grid.box_sizing == BoxSizing::BorderBox {
        return Some(Dimension::length(width));
    }
    let outside = [
        grid.padding.left,
        grid.padding.right,
        grid.border.left,
        grid.border.right,
    ]
    .into_iter()
    .map(|edge| Dimension::from(edge).into_option())
    .sum::<Option<f32>>()?;
    Some(Dimension::length(width + outside))
}

/// The wrapper's children in the order they are laid out.
///
/// CSS 2.1 section 17.4.1 puts a caption above or below the table grid inside
/// the wrapper's margins, according to `caption-side`. Buckram's box tree
/// keeps every caption before the grid, which is the order the source implies;
/// this is the order the page shows. Within a side, source order is kept, so
/// two top captions stack in the order they were written.
fn wrapper_children_in_caption_order<Id>(
    boxes: &GeneratedBoxTree<Id>,
    styles: &StylePlane<Id>,
    wrapper: BoxId,
) -> Vec<BoxId>
where
    Id: Copy + Eq + Hash,
{
    let below = |child: BoxId| {
        boxes[child].display.internal_table == Some(InternalTableRole::Caption)
            && boxes
                .origin_node(child)
                .and_then(|node| styles.get(node))
                .is_some_and(|computed| computed.caption_side == CaptionSide::Bottom)
    };
    let children = boxes[wrapper].children();
    children
        .iter()
        .copied()
        .filter(|child| !below(*child))
        .chain(children.iter().copied().filter(|child| below(*child)))
        .collect()
}

/// Taffy's block algorithm stacks physical top to bottom. A table wrapper is
/// responsible for caption order, so a vertical writing mode must select the
/// matching physical main axis before the generic backend sees its children.
/// The grid remains a distinct child and still owns table tracks; this is not
/// a table-row projection or a fragmentation rule.
fn wrapper_uses_logical_block_axis(style: &mut Style, flow: FlowAxes) -> bool {
    style.flex_direction = match flow.block_start() {
        PhysicalSide::Top => return false,
        PhysicalSide::Bottom => FlexDirection::ColumnReverse,
        PhysicalSide::Left => FlexDirection::Row,
        PhysicalSide::Right => FlexDirection::RowReverse,
    };
    style.display = Display::Flex;
    true
}

/// The horizontal margins a caption carries into its contribution.
///
/// C5 of the K4e1 interop matrix pins that both engines include them: a
/// 176-wide caption with `margin-left: 30px` puts a floor of 206 under the
/// table, not 176.
fn caption_horizontal_margins(computed: &ComputedValues, em: f32, basis: Option<f32>) -> f32 {
    let resolve = |margin: Margin| match margin {
        // An auto margin on a caption resolves against a width the table does
        // not have yet, and neither engine lets it widen the table.
        Margin::Auto => 0.0,
        Margin::Value(value) => {
            absolute_length_percentage(value, em, LIVE_ROOT_FONT_SIZE, basis.unwrap_or(0.0))
        },
    };
    resolve(computed.margin_left) + resolve(computed.margin_right)
}

/// Whether a wrapper needs the `float: left` shrink-to-fit compatibility route.
///
/// Only a block-level in-flow wrapper does. The wrapper of an inline-table is
/// an atomic inline, an absolutely positioned wrapper is shrink-to-fit under
/// CSS 2.1 section 10.3.7, and a flex or grid item is sized by its container -
/// all three already shrink-wrap, and floating them instead would take them
/// out of the formatting context they belong to.
fn wrapper_needs_float_fallback<Id>(
    boxes: &GeneratedBoxTree<Id>,
    wrapper: BoxId,
    style: &Style,
) -> bool
where
    Id: Copy + Eq + Hash,
{
    let laid_out_as_an_item = boxes[wrapper].parent().is_some_and(|parent| {
        matches!(
            boxes[parent].formatting_context,
            Some(FormattingContextKind::Flex | FormattingContextKind::Grid)
        )
    });
    boxes[wrapper].display.outside == Some(DisplayOutside::Block)
        && !laid_out_as_an_item
        && style.position != Position::Absolute
        && style.float == TaffyFloat::None
}

fn anonymous_taffy_style<Id>(css_box: &CssBox<Id>) -> Style {
    let display = match (css_box.formatting_context, css_box.display.internal_table) {
        (Some(FormattingContextKind::Flex), _) => Display::Flex,
        (Some(FormattingContextKind::Grid), _) => Display::Grid,
        _ => Display::Block,
    };
    Style {
        display,
        ..Style::default()
    }
}

fn legacy_origin_node<Id>(boxes: &GeneratedBoxTree<Id>, box_id: BoxId) -> Option<Id>
where
    Id: Copy + Eq + Hash,
{
    match boxes[box_id].origin {
        BoxOrigin::Element(node) if boxes.principal_box(node) == Some(box_id) => Some(node),
        BoxOrigin::Text(node) => Some(node),
        BoxOrigin::Element(_) | BoxOrigin::Pseudo { .. } | BoxOrigin::Anonymous { .. } => None,
    }
}

/// Record flow-relative fragment geometry without changing the physical
/// rectangle consumed by the retained text and paint lanes.
fn fragment_for_box<Id>(
    boxes: &GeneratedBoxTree<Id>,
    box_id: BoxId,
    physical: Fragment,
    relative: Fragment,
    containing: Fragment,
) -> TreeFragment
where
    Id: Copy + Eq + Hash,
{
    let flow = boxes[box_id].flow;
    if flow.is_horizontal() {
        TreeFragment::from_horizontal_physical(box_id, physical)
    } else {
        let logical = flow.logical_rect(
            relative,
            PhysicalSize {
                width: containing.width,
                height: containing.height,
            },
        );
        TreeFragment::from_physical_with_logical(box_id, physical, logical, flow)
    }
}

/// The structural fragments Buckram emitted for each live table, keyed by the
/// grid box that owns them.
type TableFragmentPlane = HashMap<BoxId, TableFragments>;

fn collect_fragments<Id>(
    tree: &AlgorithmTree<Style, TextMeasure, Option<BoxId>>,
    boxes: &GeneratedBoxTree<Id>,
    node: AlgorithmNodeId,
    cursor: FragmentCursor,
    tables: &TableFragmentPlane,
    output: &mut FragmentOutput<'_>,
) -> Result<(), LayoutError>
where
    Id: Copy + Eq + Hash,
{
    let computed = tree.layout(node);
    let static_computed = tree.static_layout(node);
    let origin = Point {
        x: cursor.origin.x + computed.x,
        y: cursor.origin.y + computed.y,
    };
    let rect = Fragment {
        x: origin.x,
        y: origin.y,
        width: computed.width,
        height: computed.height,
    };
    let mut child_parent = cursor.parent;
    {
        let source = *tree.source(node);
        let origin_node = match source {
            Some(box_id) => {
                if let Some(existing) = output
                    .fragments
                    .fragment_ids_for_box(box_id)
                    .last()
                    .copied()
                {
                    child_parent = Some(existing);
                } else {
                    let structural_parent = boxes[box_id].parent().and_then(|parent_box| {
                        output
                            .fragments
                            .fragment_ids_for_box(parent_box)
                            .last()
                            .copied()
                    });
                    let parent = structural_parent.or(cursor.parent);
                    let flow = boxes[box_id].flow;
                    let static_rect = flow.logical_rect(
                        PhysicalRect {
                            x: static_computed.x,
                            y: static_computed.y,
                            width: static_computed.width,
                            height: static_computed.height,
                        },
                        PhysicalSize {
                            width: cursor.containing.width,
                            height: cursor.containing.height,
                        },
                    );
                    let fragment = if flow.is_horizontal() {
                        TreeFragment::from_horizontal_physical(box_id, rect)
                    } else {
                        let logical_rect = flow.logical_rect(
                            PhysicalRect {
                                x: computed.x,
                                y: computed.y,
                                width: computed.width,
                                height: computed.height,
                            },
                            PhysicalSize {
                                width: cursor.containing.width,
                                height: cursor.containing.height,
                            },
                        );
                        TreeFragment::from_physical_with_logical(box_id, rect, logical_rect, flow)
                    };
                    record_static_position(
                        boxes,
                        box_id,
                        parent,
                        static_rect,
                        tree.grid_positioned_area(node),
                        output,
                    );
                    let id = output.fragments.push(
                        fragment
                            .with_baselines(fragment_baselines(tree, boxes, node, box_id, rect)),
                        parent,
                        parent,
                    );
                    child_parent = Some(id);
                    if let Some(emitted) = tables.get(&box_id) {
                        commit_table_structure(emitted, origin, id, boxes, output);
                    }
                }
                legacy_origin_node(boxes, box_id)
            },
            None => None,
        };
        let _ = origin_node;
    }
    for child in tree.children(node) {
        collect_fragments(
            tree,
            boxes,
            *child,
            FragmentCursor {
                origin,
                containing: rect,
                parent: child_parent,
            },
            tables,
            output,
        )?;
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "one recursive walk; six of the eight are context invariant across               the whole walk and the other two are the per-node cursor"
)]
fn collect_inline_fragments<Id>(
    tree: &AlgorithmTree<Style, InlineMeasure, Vec<BoxId>>,
    boxes: &GeneratedBoxTree<Id>,
    node: AlgorithmNodeId,
    cursor: FragmentCursor,
    tables: &TableFragmentPlane,
    output: &mut FragmentOutput<'_>,
    text_frame: &mut TextFrame<Id>,
    styles: &StylePlane<Id>,
) -> Result<(), LayoutError>
where
    Id: Copy + Eq + Hash,
{
    let computed = tree.layout(node);
    let static_computed = tree.static_layout(node);
    let rounded_rect = Fragment {
        x: cursor.origin.x + computed.x,
        y: cursor.origin.y + computed.y,
        width: computed.width,
        height: computed.height,
    };
    // K4d's structural table fragments retain Buckram's subpixel cell
    // positions. Taffy's final rounding is still the ordinary algorithm-tree
    // geometry, but using it as the origin for cell descendants accumulates a
    // pixel of text drift across sibling columns. Descendants of an emitted
    // cell therefore start from the authoritative structural rectangle that
    // already exists in the fragment tree.
    let rect = tree
        .source(node)
        .iter()
        .find(|box_id| boxes[**box_id].display.internal_table == Some(InternalTableRole::Cell))
        .and_then(|box_id| {
            output
                .fragments
                .fragment_ids_for_box(*box_id)
                .last()
                .and_then(|id| output.fragments.get(*id))
                .map(TreeFragment::physical_rect)
        })
        .unwrap_or(rounded_rect);
    let origin = Point {
        x: rect.x,
        y: rect.y,
    };
    let relative_rect = Fragment {
        x: rect.x - cursor.origin.x,
        y: rect.y - cursor.origin.y,
        width: rect.width,
        height: rect.height,
    };
    let placement = if let Some(context) = tree.context(node)
        && let Some(layout) = context.layout_for_width(computed.width)
    {
        Some(layout.place(
            text_frame,
            styles,
            |box_id| boxes.origin_node(box_id),
            (origin.x, origin.y),
            computed.width,
        ))
    } else {
        None
    };
    let mut child_parent = cursor.parent;
    {
        let mut source_ids = tree.source(node).clone();
        if let Some(placement) = &placement {
            source_ids.extend(placement.fragments.keys().copied());
        }
        source_ids.sort_unstable();
        source_ids.dedup();
        for box_id in source_ids {
            if let Some(existing) = output
                .fragments
                .fragment_ids_for_box(box_id)
                .last()
                .copied()
            {
                child_parent.get_or_insert(existing);
                continue;
            }
            let structural_parent = boxes[box_id].parent().and_then(|parent_box| {
                output
                    .fragments
                    .fragment_ids_for_box(parent_box)
                    .last()
                    .copied()
            });
            let parent = structural_parent.or(cursor.parent);
            let line_fragments = placement
                .as_ref()
                .and_then(|placement| placement.fragments.get(&box_id))
                .filter(|fragments| !fragments.is_empty());
            if let Some(line_fragments) = line_fragments {
                for line_fragment in line_fragments {
                    let relative_line = Fragment {
                        x: line_fragment.x - cursor.origin.x,
                        y: line_fragment.y - cursor.origin.y,
                        width: line_fragment.width,
                        height: line_fragment.height,
                    };
                    let flow = boxes[box_id].flow;
                    let static_rect = flow.logical_rect(
                        relative_line,
                        PhysicalSize {
                            width: cursor.containing.width,
                            height: cursor.containing.height,
                        },
                    );
                    record_static_position(boxes, box_id, parent, static_rect, None, output);
                    let fragment_id = output.fragments.push(
                        fragment_for_box(
                            boxes,
                            box_id,
                            *line_fragment,
                            relative_line,
                            cursor.containing,
                        )
                        .with_baselines(fragment_baselines(
                            tree,
                            boxes,
                            node,
                            box_id,
                            *line_fragment,
                        )),
                        parent,
                        parent,
                    );
                    child_parent.get_or_insert(fragment_id);
                }
            } else {
                let flow = boxes[box_id].flow;
                let static_rect = flow.logical_rect(
                    Fragment {
                        x: static_computed.x,
                        y: static_computed.y,
                        width: static_computed.width,
                        height: static_computed.height,
                    },
                    PhysicalSize {
                        width: cursor.containing.width,
                        height: cursor.containing.height,
                    },
                );
                record_static_position(
                    boxes,
                    box_id,
                    parent,
                    static_rect,
                    tree.grid_positioned_area(node),
                    output,
                );
                let fragment_id = output.fragments.push(
                    fragment_for_box(boxes, box_id, rect, relative_rect, cursor.containing)
                        .with_baselines(fragment_baselines(tree, boxes, node, box_id, rect)),
                    parent,
                    parent,
                );
                if let Some(emitted) = tables.get(&box_id) {
                    commit_table_structure(emitted, origin, fragment_id, boxes, output);
                }
                child_parent.get_or_insert(fragment_id);
            }
        }
    }
    for child in tree.children(node) {
        collect_inline_fragments(
            tree,
            boxes,
            *child,
            FragmentCursor {
                origin,
                containing: rect,
                parent: child_parent,
            },
            tables,
            output,
            text_frame,
            styles,
        )?;
    }
    Ok(())
}

/// Normalize HTML table attributes at the Livery boundary, then ask Buckram
/// to derive one topology from the generated CSS boxes. CSS-display tables
/// receive the same model with the default one-by-one spans.
fn build_table_grid<D>(boxes: &GeneratedBoxTree<D::NodeId>, dom: &D, grid: BoxId) -> TableGrid
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    fn visit<D>(
        boxes: &GeneratedBoxTree<D::NodeId>,
        dom: &D,
        box_id: BoxId,
        inputs: &mut TableGridInputs,
    ) where
        D: LayoutDom,
        D::NodeId: Copy + Eq + Hash,
    {
        if let Some(node) = boxes.origin_node(box_id) {
            match boxes[box_id].display.internal_table {
                Some(InternalTableRole::Cell) if html_element(dom, node, &["td", "th"]) => {
                    inputs.set_cell(box_id, html_cell_input(dom, node));
                },
                Some(InternalTableRole::Column) if html_element(dom, node, &["col"]) => {
                    inputs.set_column(
                        box_id,
                        TableTrackInput {
                            span: html_limited_span(html_attribute(dom, node, "span"), 1_000),
                        },
                    );
                },
                Some(InternalTableRole::ColumnGroup) if html_element(dom, node, &["colgroup"]) => {
                    inputs.set_column_group(
                        box_id,
                        TableTrackInput {
                            span: html_limited_span(html_attribute(dom, node, "span"), 1_000),
                        },
                    );
                },
                _ => {},
            }
        }
        for child in boxes[box_id].children() {
            visit(boxes, dom, *child, inputs);
        }
    }

    let mut inputs = TableGridInputs::default();
    visit(boxes, dom, grid, &mut inputs);
    TableGrid::from_box_tree(&**boxes, grid, &inputs)
}

fn html_element<D>(dom: &D, node: D::NodeId, names: &[&str]) -> bool
where
    D: LayoutDom,
{
    dom.element_name(node).is_some_and(|name| {
        name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
            && names
                .iter()
                .any(|candidate| name.local.as_ref().eq_ignore_ascii_case(candidate))
    })
}

fn html_attribute<'dom, D>(dom: &'dom D, node: D::NodeId, local: &str) -> Option<&'dom str>
where
    D: LayoutDom,
{
    dom.attribute(node, &Namespace::from(""), &LocalName::from(local))
}

fn html_limited_span(value: Option<&str>, maximum: usize) -> usize {
    value
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.min(maximum))
        .unwrap_or(1)
}

fn html_cell_input<D>(dom: &D, node: D::NodeId) -> TableCellInput
where
    D: LayoutDom,
{
    let row_span = match html_attribute(dom, node, "rowspan")
        .and_then(|value| value.trim().parse::<usize>().ok())
    {
        Some(0) => TableRowSpan::ToEndOfGroup,
        Some(value) => TableRowSpan::Count(value.min(65_534)),
        None => TableRowSpan::Count(1),
    };
    TableCellInput {
        column_span: html_limited_span(html_attribute(dom, node, "colspan"), 1_000),
        row_span,
    }
}

/// Whether every character is CSS collapsible white space.
///
/// CSS collapsible white space is exactly space, tab, line feed, carriage
/// return, and form feed (css-text-3 section 3). It is deliberately *not*
/// Rust's `char::is_whitespace`, which also matches U+00A0 no-break space
/// and the other Unicode spaces. Those generate content: `&nbsp;` is the
/// standard way a test forces a line box to exist, so trimming it away
/// silently deletes the line.
fn is_collapsible_whitespace(text: &str) -> bool {
    text.chars()
        .all(|ch| matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{c}'))
}

fn is_atomic_inline_box<D>(dom: &D, styles: &StylePlane<D::NodeId>, id: D::NodeId) -> bool
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    styles.get(id).is_some_and(|style| {
        style.display == CssDisplay::InlineBlock
            // K4e4: an inline-table is an atomic inline like an inline-block;
            // its wrapper occupies line space as a unit.
            || style.display == CssDisplay::InlineTable
            || (style.display == CssDisplay::Inline && is_replaced_element(dom, id))
            // CSS 2.1 17.2.1: a replaced element given an internal table
            // display is demoted to inline by box generation, so it is an
            // atomic inline here too. Reading the computed value alone would
            // leave it laid out as an inline container and stretched.
            || (is_replaced_element(dom, id)
                && crate::box_tree::is_internal_table_display(style.display))
    })
}

fn has_atomic_inline_ancestor<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    boxes: &GeneratedBoxTree<D::NodeId>,
    id: D::NodeId,
) -> bool
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let mut ancestor = dom.parent(id);
    while let Some(candidate) = ancestor {
        if boxes
            .principal_box(candidate)
            .is_some_and(|box_id| boxes[box_id].display.outside == Some(DisplayOutside::Inline))
            && is_atomic_inline_box(dom, styles, candidate)
        {
            return true;
        }
        ancestor = dom.parent(candidate);
    }
    false
}

fn is_replaced_element<D>(dom: &D, id: D::NodeId) -> bool
where
    D: LayoutDom,
    D::NodeId: Copy,
{
    dom.kind(id) == NodeKind::Element
        && dom.element_name(id).is_some_and(|name| {
            name.local.as_ref().eq_ignore_ascii_case("img")
                || name.local.as_ref().eq_ignore_ascii_case("canvas")
        })
}

/// Whether the nearest non-anonymous ancestor establishes a flex or grid
/// formatting context, in which `auto` sizing may stretch. Anonymous boxes
/// are skipped: an inline-level image in a grid container sits inside an
/// anonymous grid item, and it is the container's context that governs.
fn stretched_by_ancestor_context<Id>(boxes: &GeneratedBoxTree<Id>, box_id: BoxId) -> bool
where
    Id: Copy + Eq + Hash,
{
    let mut parent = boxes[box_id].parent();
    while let Some(id) = parent {
        let css_box = &boxes[id];
        if matches!(css_box.origin, BoxOrigin::Anonymous { .. }) {
            parent = css_box.parent();
            continue;
        }
        return matches!(
            css_box.formatting_context,
            Some(FormattingContextKind::Flex | FormattingContextKind::Grid)
        );
    }
    false
}

// CSS 2.1 10.4: the used size of a replaced element with an intrinsic size
// and ratio whose width and height are both `auto`, once min/max constraints
// apply. Inputs and outputs are CONTENT-BOX lengths; the caller converts a
// border-box constraint by subtracting the box's edges first and adds them
// back to the result.
//
// The table is the spec's, row for row. Verified against all sixty images of
// css-sizing/box-sizing-replaced-001..003 (border-box with padding,
// border-box with padding and border, content-box), every one resolving to
// the 75x75 content its reference expects, before this was ported.
pub(in crate::layout) fn replaced_min_max(
    natural: (f32, f32),
    min_width: Option<f32>,
    max_width: Option<f32>,
    min_height: Option<f32>,
    max_height: Option<f32>,
) -> (f32, f32) {
    let (w, h) = natural;
    let min_w = min_width.unwrap_or(0.0);
    let min_h = min_height.unwrap_or(0.0);
    // A max below its min is treated as that min.
    let max_w = max_width.unwrap_or(f32::INFINITY).max(min_w);
    let max_h = max_height.unwrap_or(f32::INFINITY).max(min_h);
    if w <= 0.0 || h <= 0.0 {
        return (w.clamp(min_w, max_w), h.clamp(min_h, max_h));
    }
    let ratio = w / h;
    match (w > max_w, w < min_w, h > max_h, h < min_h) {
        (true, _, true, _) => {
            if max_w / w <= max_h / h {
                (max_w, (max_w / ratio).max(min_h))
            } else {
                ((max_h * ratio).max(min_w), max_h)
            }
        },
        (_, true, _, true) => {
            if min_w / w <= min_h / h {
                ((min_h * ratio).min(max_w), min_h)
            } else {
                (min_w, (min_w / ratio).min(max_h))
            }
        },
        (_, true, true, _) => (min_w, max_h),
        (true, _, _, true) => (max_w, min_h),
        (true, _, _, _) => (max_w, (max_w / ratio).max(min_h)),
        (_, true, _, _) => (min_w, (min_w / ratio).min(max_h)),
        (_, _, true, _) => ((max_h * ratio).max(min_w), max_h),
        (_, _, _, true) => ((min_h * ratio).min(max_w), min_h),
        _ => (w, h),
    }
}

fn apply_replaced_intrinsic_style<D>(
    style: &mut Style,
    dom: &D,
    id: D::NodeId,
    computed: &ComputedValues,
    image_sources: &ImageSources,
    font_size: f32,
    block_level_flow: bool,
    containing_width: f32,
) -> Option<(f32, f32)>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let intrinsic = replaced_intrinsic_size(dom, id, image_sources);
    let natural_ratio = intrinsic
        .filter(|(width, height)| *width > 0.0 && *height > 0.0)
        .map(|(width, height)| width / height);

    // Attribute-derived dimensions already reached `computed` through the
    // presentational-hint origin. Layout owns only natural-size resolution.
    let width = definite_size(computed.width, font_size);
    let height = definite_size(computed.height, font_size);
    if computed.aspect_ratio.uses_natural_ratio()
        && natural_ratio.is_some()
        && !(width.is_some() && height.is_some())
    {
        // Taffy's aspect-ratio input participates in sizing even when both
        // axes are definite. CSS's natural ratio does not, so only expose it
        // while at least one axis still needs intrinsic resolution.
        style.aspect_ratio = natural_ratio;
    }

    // CSS 2.1 10.3.4: a block-level replaced element in normal flow resolves
    // `width: auto` to its intrinsic width instead of stretching to its
    // containing block, so the natural size has to reach Taffy as a definite
    // length. Without it a `display: block` canvas fills the
    // container and the natural ratio then stretches its height to match,
    // which is how a 100x100 canvas laid out at 200x200 inside a 200px parent.
    //
    // The rule is narrow on purpose; each exclusion below is a reftest that
    // failed when it was wider.
    //
    // - It is 10.3.4, block-level only. An inline replaced element already
    //   contributes its natural box through the inline atomic-root path
    //   (10.3.2), and both halves of a reftest render through this engine,
    //   so touching inline sizing moved the *references*.
    // - A flex or grid item is the other way round: `auto` there means the
    //   item may stretch or grow, and Taffy already feeds the measured
    //   intrinsic size in as the content size.
    // - An author `aspect-ratio` owns the transfer between the axes.
    // - `min-content`, `max-content` and `fit-content` are keywords, not
    //   `auto`; their replaced contribution is the ratio-transferred size.
    // - Any non-auto height with a natural ratio transfers to the width
    //   (10.3.2), so the intrinsic width only stands when nothing else can
    //   decide it. This is `!height_auto`, not "definite": a percentage
    //   height is indefinite to `definite_size` but Taffy still resolves it
    //   against a definite container, and the width must follow that.
    // - Under `box-sizing: border-box` a replaced element with min/max
    //   constraints needs CSS 2.1 10.4's ratio-preserving clamp run in
    //   border-box space. Taffy's leaf measure already does that; a forced
    //   `size` bypasses it (box-sizing-replaced-001..003), so border-box
    //   stays on the measure path. A block-level border-box replaced element
    //   therefore still stretches; that gap is narrower and is left named.
    let width_auto = matches!(computed.width, CssSize::Auto);
    let height_auto = matches!(computed.height, CssSize::Auto);
    let author_ratio = !computed.aspect_ratio.uses_natural_ratio();
    let content_box = matches!(computed.box_sizing, CssBoxSizing::ContentBox);
    if let Some((natural_width, natural_height)) =
        intrinsic.filter(|_| block_level_flow && !author_ratio && content_box)
    {
        if width_auto && natural_width > 0.0 && !(!height_auto && natural_ratio.is_some()) {
            style.size.width = Dimension::length(natural_width);
        }
        if height_auto && natural_ratio.is_none() && natural_height > 0.0 {
            style.size.height = Dimension::length(natural_height);
        }
    }

    // Under `box-sizing: border-box` the box takes the full CSS 2.1 10.4 route
    // here, with both axes resolved and handed to Taffy as definite border-box
    // lengths. Taffy's leaf path is not 10.4: it clamps, then transfers height
    // from width, and forcing only the width bypassed even that -- which is how
    // box-sizing-replaced-001..003 failed twice. Resolving the whole table in
    // content space and adding the edges back keeps every min/max interaction
    // ratio-preserving. The natural ratio is cleared so leaf.rs cannot re-derive
    // a height over a resolved one. Percentage min/max stay with Taffy, as
    // `definite_size` leaves them indefinite; the three reftests use px only.
    if let Some(natural) = intrinsic.filter(|(w, h)| {
        block_level_flow
            && !author_ratio
            && !content_box
            && width_auto
            && height_auto
            && *w > 0.0
            && *h > 0.0
    }) {
        let px = |value: CssLengthPercentage| {
            absolute_length_percentage(value, font_size, 16.0, containing_width)
        };
        let edge_x = px(computed.padding_left.0)
            + px(computed.padding_right.0)
            + border_width_px(
                computed.border_left_style,
                computed.border_left_width,
                font_size,
            )
            + border_width_px(
                computed.border_right_style,
                computed.border_right_width,
                font_size,
            );
        let edge_y = px(computed.padding_top.0)
            + px(computed.padding_bottom.0)
            + border_width_px(
                computed.border_top_style,
                computed.border_top_width,
                font_size,
            )
            + border_width_px(
                computed.border_bottom_style,
                computed.border_bottom_width,
                font_size,
            );
        let content = |size: CssSize, edge: f32| definite_size(size, font_size).map(|v| v - edge);
        let (used_width, used_height) = replaced_min_max(
            natural,
            content(computed.min_width, edge_x),
            content(computed.max_width, edge_x),
            content(computed.min_height, edge_y),
            content(computed.max_height, edge_y),
        );
        style.size.width = Dimension::length(used_width + edge_x);
        style.size.height = Dimension::length(used_height + edge_y);
        style.aspect_ratio = None;
    }
    intrinsic
}

/// Pass natural replaced dimensions across the browser-facing K5d edge.
/// Attribute-derived dimensions already live in the computed style through
/// presentational hints, so this path does not reread DOM width or height.
fn positioned_replaced_input<D>(
    dom: &D,
    id: D::NodeId,
    image_sources: &ImageSources,
    style: &BlockStyle,
) -> Option<buckram::ReplacedSize>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    if !style.replaced {
        return None;
    }
    let intrinsic_size = replaced_intrinsic_size(dom, id, image_sources).map(|(width, height)| {
        style
            .containing_flow
            .logical_size(PhysicalSize { width, height })
    });
    Some(buckram::ReplacedSize { intrinsic_size })
}

fn replaced_intrinsic_size<D>(
    dom: &D,
    id: D::NodeId,
    image_sources: &ImageSources,
) -> Option<(f32, f32)>
where
    D: LayoutDom,
    D::NodeId: Copy,
{
    if dom.kind(id) != NodeKind::Element {
        return None;
    }
    let local = dom.element_name(id)?.local.as_ref().to_ascii_lowercase();
    if local == "canvas" {
        return Some((
            canvas_intrinsic_dimension(dom, id, "width", 300.0),
            canvas_intrinsic_dimension(dom, id, "height", 150.0),
        ));
    }
    if local != "img" {
        return None;
    }
    let source = dom.attributes(id).find_map(|attribute| {
        (attribute.name.ns.as_ref().is_empty()
            && attribute.name.local.as_ref().eq_ignore_ascii_case("src"))
        .then_some(attribute.value)
    })?;
    let bytes = if let Ok(data_url) = data_url::DataUrl::process(source) {
        data_url.decode_to_vec().ok()?.0
    } else {
        image_sources.get(source)?.clone()
    };
    let image = image::load_from_memory(&bytes).ok()?;
    Some((image.width() as f32, image.height() as f32))
}

fn canvas_intrinsic_dimension<D>(dom: &D, id: D::NodeId, attribute: &str, default: f32) -> f32
where
    D: LayoutDom,
    D::NodeId: Copy,
{
    dom.attributes(id)
        .find_map(|candidate| {
            (candidate.name.ns.as_ref().is_empty()
                && candidate
                    .name
                    .local
                    .as_ref()
                    .eq_ignore_ascii_case(attribute))
            .then_some(candidate.value)
        })
        .and_then(crate::presentational_hints::parse_non_negative_integer_px)
        .unwrap_or(default)
}

fn definite_size(size: CssSize, font_size: f32) -> Option<f32> {
    let CssSize::Value(value) = size else {
        return None;
    };
    match value {
        CssLengthPercentage::Length(length) => Some(absolute_length(length, font_size, 16.0)),
        CssLengthPercentage::Calc(calc) if calc.percentage == 0.0 => {
            Some(calc.px + calc.em * font_size + calc.rem * 16.0)
        },
        _ => None,
    }
}

fn collapsed_word_width(text: &str) -> usize {
    let mut maximum = 0;
    let mut current = 0;
    for character in text.chars() {
        if matches!(
            character,
            '\u{0009}' | '\u{000A}' | '\u{000C}' | '\u{000D}' | ' '
        ) {
            maximum = maximum.max(current);
            current = 0;
        } else {
            current += 1;
        }
    }
    maximum.max(current)
}

fn collapsed_text_width(text: &str) -> usize {
    let mut width = 0;
    let mut pending_space = false;
    for character in text.chars() {
        if matches!(
            character,
            '\u{0009}' | '\u{000A}' | '\u{000C}' | '\u{000D}' | ' '
        ) {
            pending_space = width != 0;
        } else {
            if pending_space {
                width += 1;
                pending_space = false;
            }
            width += 1;
        }
    }
    width
}
