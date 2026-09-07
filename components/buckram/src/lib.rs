// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Genet's standards-owned CSS layout model.
//!
//! Buckram owns CSS box identity, provenance, and layout fragments. Style
//! engines and layout algorithms integrate through these types without
//! defining their shape.

#![forbid(unsafe_code)]

mod block;
mod box_tree;
mod flow;
mod fragment_tree;
mod fragmentation;
mod intrinsic;
mod positioning;
mod sticky;
mod table;
mod taffy_adapter;

pub use block::{
    BlockBoxSizing, BlockContainingBlock, BlockCornerRadii, BlockCornerRadius, BlockDeferral,
    BlockDimensions, BlockFormattingContext, BlockMarginCollapse, BlockMarginState, BlockPlacement,
    BlockPosition, BlockSizeValue, BlockStyle, ClearSide, CollapsedMargin, FloatAvailableSpace,
    FloatAvoidingPlacement, FloatLineConstraints, FloatReferenceBox, FloatSide, FlowLength,
    FlowLengthAuto, OverconstrainedInlineAlignment, UsedInlineSize, solve_float_inline_size,
    solve_in_flow_inline_size, solve_in_flow_inline_size_for_available,
    solve_shrink_to_fit_inline_size,
};
pub use box_tree::{
    AnonymousBoxKind, BoxGeneration, BoxId, BoxOrigin, BoxTreeInput, ContainingBlock,
    ContainingBlockEstablishment, CssBox, CssBoxTree, DisplayInside, DisplayOutside, DisplayRole,
    FloatContextProvenance, FormattingContextKind, InternalTableRole, PositioningScheme,
    PseudoElement, generate_box_tree,
};
pub use flow::{
    Direction, FlowAxes, LogicalAxis, LogicalOffset, LogicalRect, LogicalSides, LogicalSize,
    PhysicalOffset, PhysicalRect, PhysicalSide, PhysicalSides, PhysicalSize, WritingMode,
};
pub use fragment_tree::{
    Baselines, Fragment, FragmentId, FragmentTree, LayoutIdentityMap, LayoutResult, StaticPosition,
    StaticPositionSource,
};
pub use fragmentation::{
    BlockBreakToken, BreakToken, ClearanceState, DeferredBreakKind, DeferredBreakToken,
    FloatExclusionState, Fragmentainer, FragmentainerId, FragmentainerKind, FragmentationContext,
    FragmentationContextId, InlineResumeState, MulticolFill, SequentialMulticolInput,
};
pub use intrinsic::{
    IntrinsicQueryError, IntrinsicQueryState, IntrinsicSizeCache, IntrinsicSizeKind,
    IntrinsicSizeQuery, IntrinsicSizes, block_intrinsic_sizes_for_definite_inline,
};
pub use positioning::{
    PositionedBoxGeometry, PositionedBoxInput, ReplacedSize, solve_positioned_box,
};
pub use sticky::{StickyAxisInput, solve_sticky_axis};
pub use table::{
    AffineLengthPercentage, CaptionMinContribution, CellBlockOffsets, CellCollapsedBorderMetrics,
    CellInlineOffsets, CollapsedBorderGeometry, CollapsedBorderGeometryError,
    CollapsedBorderInteropDeferral, CollapsedBorderMetricError, CollapsedBorderMetrics,
    CollapsedBorderPaintSegment, CollapsedBorderProjection, CollapsedBorderSegmentMetric,
    CollapsedBorderSideMetrics, FragmentDraft, FragmentDraftTree, GridEdgeOrientation,
    InlineSizeConstraint, ResolvedTableBorder, ResolvedTableBorderGrid, TableAlignment,
    TableAutomaticColumnGroupInput, TableAutomaticColumnInput, TableAutomaticColumnMeasureInput,
    TableAutomaticColumnMeasures, TableAutomaticInlineSizingIndefinite,
    TableAutomaticInlineSizingInput, TableAutomaticInlineSizingOutcome, TableBlockBorderMetrics,
    TableBlockConstraint, TableBlockDeferral, TableBlockLayout, TableBlockSizingInput,
    TableBorderCandidate, TableBorderCandidates, TableBorderDisposition, TableBorderError,
    TableBorderLedgerEntry, TableBorderOrderKey, TableBorderOrigin, TableBorderPrecedence,
    TableBorderResolutionError, TableBorderSide, TableBorderSides, TableBorderSource,
    TableBorderSources, TableBorderStyle, TableBoxSizing, TableCell, TableCellAlignment,
    TableCellBlockStyle, TableCellFormatter, TableCellInlineMeasure, TableCellInlineStyle,
    TableCellInput, TableCellLayoutInput, TableCellLayoutOutput, TableCellLayoutPass,
    TableCellPlacement, TableCollapsedBlockMetrics, TableCollapsedBorderMetrics,
    TableColumnMeasure, TableDeferral, TableFixedColumnGroupInput, TableFixedColumnInput,
    TableFixedInlineSizingInput, TableFixedInlineSizingOutcome, TableFixedLayoutFallback,
    TableFragment, TableFragmentRole, TableFragments, TableGrid, TableGridEdge, TableGridError,
    TableGridInputs, TableGridLines, TableInlineBorderMetrics, TableInlineConstraints,
    TableInlineProperty, TableInlineSizingError, TableInlineSizingInput, TableInlineSizingResult,
    TableIntrinsicMeasureProvider, TablePercentagePass, TableRowBaseline, TableRowLayoutError,
    TableRowMeasure, TableRowSizing, TableRowSpan, TableSeparatedBlockMetrics,
    TableSeparatedBorderMetrics, TableSlot, TableSpanMeasureDistribution, TableTrack,
    TableTrackGroup, TableTrackGroupKind, TableTrackInput, TableTrackVisibility,
    TableTrackVisibilityState, align_table_cells, apply_baseline_row_minima,
    cache_automatic_table_grid_intrinsic_sizes, collect_table_border_candidates,
    collect_table_cell_inline_measures, compare_table_border_candidates, emit_table_fragments,
    format_table_cells, layout_table_block, measure_automatic_columns, measure_single_span_rows,
    project_collapsed_border_metrics, query_table_cell_inline_sizes,
    resolve_collapsed_border_geometry, resolve_percentage_block_sizes,
    resolve_table_border_candidates, size_automatic_table_inline, size_fixed_table_inline,
    size_table_rows, spanned_cell_content_inline_size,
};
pub use taffy_adapter::{
    AlgorithmAvailableSpace, AlgorithmKind, AlgorithmLayout, AlgorithmNodeId, AlgorithmSize,
    AlgorithmStyle, AlgorithmTree, BlockAlgorithm, FragmentationDispatch, FragmentationOutput,
};
