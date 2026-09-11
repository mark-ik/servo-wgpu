// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The block-level builder: projects the box tree's block, flex, grid and
//! table subtrees into Buckram's algorithm tree.

use super::*;

/// Lower only fully definite horizontal sequential column parameters.
/// Unsupported balancing and cyclic/indefinite inputs remain absent from
/// Buckram's input plane and therefore retain the ordinary fallback.
pub(in crate::layout) fn lower_sequential_multicol(
    computed: &ComputedValues,
    block_style: BlockStyle,
) -> Option<buckram::SequentialMulticolInput> {
    if computed.column_fill != ColumnFill::Auto
        || !block_style.flow.is_horizontal()
        || block_style.position != BuckramBlockPosition::Static
        || block_style.float != FloatSide::None
    {
        return None;
    }
    let absolute = |value: CssLengthPercentage| match value {
        CssLengthPercentage::Zero => Some(0.0),
        CssLengthPercentage::Length(length)
            if length.unit == livery::values::LengthUnit::Px && length.value >= 0.0 =>
        {
            Some(length.value)
        },
        _ => None,
    };
    // Keep admission narrow until the run-side used-size seam resolves the
    // actual content box. These values are validation only and never enter
    // the style-owned input.
    let CssSize::Value(width_value) = computed.width else {
        return None;
    };
    let CssSize::Value(height_value) = computed.height else {
        return None;
    };
    absolute(width_value)?;
    absolute(height_value)?;
    let ColumnCount::Count(column_count) = computed.column_count else {
        return None;
    };
    let gap = absolute(computed.column_gap.0)?;
    let column_width = match computed.column_width {
        ColumnWidth::Auto => None,
        ColumnWidth::Length(value) => Some(absolute(value)?),
    };
    buckram::SequentialMulticolInput::new(
        column_count as usize,
        column_width,
        gap,
        buckram::MulticolFill::Auto,
    )
}

pub(in crate::layout) fn lower_sequential_multicol_if_allowed(
    computed: &ComputedValues,
    block_style: BlockStyle,
    nested: bool,
) -> Option<buckram::SequentialMulticolInput> {
    if nested {
        None
    } else {
        lower_sequential_multicol(computed, block_style)
    }
}

pub(in crate::layout) struct BuildState<'a, D: LayoutDom> {
    pub(in crate::layout) dom: &'a D,
    pub(in crate::layout) styles: &'a StylePlane<D::NodeId>,
    pub(in crate::layout) boxes: &'a GeneratedBoxTree<D::NodeId>,
    pub(in crate::layout) tree: AlgorithmTree<Style, TextMeasure, Option<BoxId>>,
    pub(in crate::layout) image_sources: &'a ImageSources,
    pub(in crate::layout) text: Option<&'a mut TextSystem>,
    pub(in crate::layout) table_shadow: TableShadowLedger,
    pub(in crate::layout) pending_tables: Vec<PendingTable<D::NodeId>>,
}

impl<D> BuildState<'_, D>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    fn has_multicol_ancestor(&self, box_id: BoxId) -> bool {
        let mut ancestor = self.boxes[box_id].parent();
        while let Some(box_id) = ancestor {
            if let Some(node) = self.boxes.origin_node(box_id)
                && self
                    .styles
                    .get(node)
                    .is_some_and(|computed| matches!(computed.column_count, ColumnCount::Count(_)))
            {
                return true;
            }
            ancestor = self.boxes[box_id].parent();
        }
        false
    }

    /// One intrinsic inline query through the same measure contract the main
    /// layout uses. Only sound once painted fragments have been collected,
    /// because it recomputes the subtree's scratch layout.
    pub(in crate::layout) fn measure_intrinsic_width(
        &mut self,
        node: AlgorithmNodeId,
        available: AlgorithmAvailableSpace,
    ) -> f32 {
        self.tree
            .compute_layout_with_measure_excluding_out_of_flow_children(
                node,
                AlgorithmSize::new(available, AlgorithmAvailableSpace::MaxContent),
                |known, available, _, context, _| {
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
                },
            );
        self.tree.layout(node).width
    }

    /// Measure one cell's border-box intrinsic pair through the live measure
    /// contract. The cell's own width sizing is neutralized for the query,
    /// because Buckram applies those constraints itself, and restored
    /// afterwards so the main layout pass sees the real style.
    pub(in crate::layout) fn measure_cell_intrinsics(
        &mut self,
        cell_node: AlgorithmNodeId,
    ) -> Option<IntrinsicSizes> {
        let style = self.tree.style_mut(cell_node);
        let saved = (style.size.width, style.min_size.width, style.max_size.width);
        style.size.width = Dimension::auto();
        style.min_size.width = Dimension::auto();
        style.max_size.width = Dimension::auto();
        let direct_child_width = |tree: &AlgorithmTree<Style, TextMeasure, Option<BoxId>>| {
            tree.children(cell_node)
                .iter()
                .filter(|child| !tree.block_style(**child).is_out_of_flow())
                .map(|child| tree.unrounded_layout(*child).width)
                .reduce(f32::max)
                .unwrap_or_else(|| tree.unrounded_layout(cell_node).width)
        };
        self.measure_intrinsic_width(cell_node, AlgorithmAvailableSpace::MinContent);
        let min = direct_child_width(&self.tree);
        self.measure_intrinsic_width(cell_node, AlgorithmAvailableSpace::MaxContent);
        let max = direct_child_width(&self.tree);
        let style = self.tree.style_mut(cell_node);
        (style.size.width, style.min_size.width, style.max_size.width) = saved;
        IntrinsicSizes::new(min, max.max(min))
    }

    /// The floor a caption puts under the table's inline size.
    ///
    /// Its own min-content width plus its horizontal margins, which is what
    /// C5 and C6 of the K4e1 interop matrix pin. Unlike a cell measurement
    /// this does *not* neutralize the caption's own `width`: C7 shows a
    /// specified caption width participating like any other box, so a
    /// `width: 300px` caption puts a floor of 300 under the table. Several
    /// captions each put their own floor down and the widest one wins.
    pub(in crate::layout) fn measure_caption_min(
        &mut self,
        captions: &[(AlgorithmNodeId, f32)],
    ) -> Option<f32> {
        captions
            .iter()
            .map(|(caption, margins)| {
                self.measure_intrinsic_width(*caption, AlgorithmAvailableSpace::MinContent)
                    + margins
            })
            .reduce(f32::max)
            .filter(|minimum| minimum.is_finite() && *minimum >= 0.0)
    }

    pub(in crate::layout) fn build_anonymous_table_grid(
        &mut self,
        box_id: BoxId,
        inherited: Option<&ComputedValues>,
        parent_font_size: f32,
        containing_size: (Option<f32>, Option<f32>),
    ) -> Result<Option<AlgorithmNodeId>, LayoutError> {
        let table_style = anonymous_table_style(inherited);
        let computed = grid_style(&table_style, containing_size);
        let font_size = font_size_px(&computed.font_size, parent_font_size);
        let child_containing_size =
            resolved_child_containing_size(&computed, font_size, containing_size);
        let table = build_table_grid(self.boxes, self.dom, box_id);
        let mut cell_nodes = Vec::with_capacity(table.cells.len());
        let mut children = Vec::with_capacity(table.cells.len());
        for cell in &table.cells {
            let built = self.build_box(
                cell.source,
                Some(&computed),
                font_size,
                child_containing_size,
            )?;
            cell_nodes.push(built);
            if let Some(node) = built {
                children.push(node);
            }
        }
        let mut out_of_flow_parts = Vec::with_capacity(table.out_of_flow_parts.len());
        for part in &table.out_of_flow_parts {
            let Some(node) =
                self.build_box(*part, Some(&computed), font_size, child_containing_size)?
            else {
                continue;
            };
            out_of_flow_parts.push(DetachedTablePart {
                box_id: *part,
                node,
            });
        }
        let taffy_style = to_taffy_style(&computed, font_size);
        let block_style = to_block_style(self.boxes, self.styles, box_id, &computed, font_size);
        let kind = algorithm_kind(&self.boxes[box_id], children.is_empty());
        let node = self.tree.new_with_children_and_block_style(
            kind,
            block_style,
            taffy_style,
            &children,
            Some(box_id),
        );
        enable_flex_grid_static_position_provider(
            &mut self.tree,
            self.styles,
            self.boxes,
            box_id,
            node,
        );
        self.pending_tables.push(PendingTable {
            table: box_id,
            node: None,
            table_style,
            table_node: node,
            wrapper: None,
            captions: Vec::new(),
            grid: table,
            collapsed_borders: None,
            collapsed_border_metrics: None,
            cell_nodes,
            out_of_flow_parts,
            font_size,
            containing_width: containing_size.0,
            containing_height: containing_size.1,
            assigned: None,
            block: None,
        });
        Ok(Some(node))
    }

    /// K4c5b and K4d6b: compute Buckram's columns for every noted table and
    /// pin them as explicit grid tracks, then lay out the block axis. Runs
    /// after the tree is built and before the main layout pass; the queries
    /// only scribble on scratch layout state that the main pass recomputes.
    pub(in crate::layout) fn apply_buckram_table_layout(&mut self) {
        let mut pendings = std::mem::take(&mut self.pending_tables);
        let mut aggregate = std::mem::take(&mut self.table_shadow);
        for pending in &mut pendings {
            self.table_shadow = TableShadowLedger::default();
            {
                let computed = pending.table_style.clone();
                pending.collapsed_border_metrics = None;
                pending.collapsed_borders = if computed.border_collapse == BorderCollapse::Collapse
                {
                    match collapsed_table_borders(
                        self.boxes,
                        self.styles,
                        &pending.grid,
                        pending.table,
                        &computed,
                        pending.font_size,
                    ) {
                        Ok(borders) => {
                            pending.collapsed_border_metrics = Some(borders.metrics);
                            self.table_shadow.collapsed_metrics += 1;
                            Some(borders.winners)
                        },
                        Err(error) => {
                            self.table_shadow.skip(
                                pending.table,
                                crate::table_shadow::TableShadowSkip::CollapsedBorder(error),
                            );
                            None
                        },
                    }
                } else {
                    None
                };
                let intrinsics = pending
                    .cell_nodes
                    .clone()
                    .into_iter()
                    .map(|cell_node| cell_node.and_then(|node| self.measure_cell_intrinsics(node)))
                    .collect::<Vec<_>>();
                let caption_min = self.measure_caption_min(&pending.captions.clone());
                let columns = buckram_table_columns(
                    self.boxes,
                    self.styles,
                    &pending.grid,
                    pending.table,
                    &computed,
                    pending.collapsed_border_metrics.as_ref(),
                    pending.font_size,
                    pending.containing_width,
                    caption_min,
                    &intrinsics,
                    &mut self.table_shadow,
                );
                pending.assigned = columns;
                self.size_wrapper_from_grid(pending);
            }
            self.apply_buckram_table_rows(std::slice::from_mut(pending));
            aggregate.record_table(pending.table, std::mem::take(&mut self.table_shadow));
        }
        self.table_shadow = aggregate;
        self.pending_tables = pendings;
    }

    /// Give the wrapper the grid's border-edge width, which is CSS Tables 3
    /// section 2.2.1: "the width of the table wrapper box is the border-edge
    /// width of the table grid box inside it."
    ///
    /// Buckram's table inline sizing has just produced that width, so the rule
    /// is an assignment rather than a measurement, and an `auto` table width is
    /// no harder than a specified one - the shrink-wrapping already happened,
    /// inside the table algorithm that owns it.
    ///
    /// A table Buckram deferred has no such width. Its wrapper falls back to
    /// the `float: left` shrink-to-fit that stood in for this rule before
    /// K4e2, whose domain is now exactly the deferral set.
    pub(in crate::layout) fn size_wrapper_from_grid(&mut self, pending: &PendingTable<D::NodeId>) {
        let (Some(wrapper), Some(inline)) = (pending.wrapper, pending.assigned.as_ref()) else {
            return;
        };
        // The fallback float was applied when the tree was built, before this
        // width existed. Retire it here rather than leaving both in play - but
        // only where it was this route that put it there, never where the
        // author wrote `float` on the table and K4e1 migrated it.
        let authored_float = pending
            .node
            .and_then(|node| self.styles.get(node))
            .is_some_and(|computed| computed.float != CssFloat::None);
        let style = self.tree.style_mut(wrapper);
        style.size.width = Dimension::length(inline.used_grid_inline_size);
        if !authored_float {
            style.float = TaffyFloat::None;
        }
        self.tree
            .set_table_wrapper_inline_size(wrapper, inline.used_grid_inline_size);
    }

    /// Run Buckram's block pipeline for every table whose columns it assigned.
    pub(in crate::layout) fn apply_buckram_table_rows(
        &mut self,
        pendings: &mut [PendingTable<D::NodeId>],
    ) {
        let mut ledger = std::mem::take(&mut self.table_shadow.block);
        let Self {
            tree,
            styles,
            boxes,
            table_shadow,
            ..
        } = self;
        for pending in pendings {
            let Some(inline) = pending.assigned.as_ref() else {
                continue;
            };
            let computed = &pending.table_style;
            let Some(inputs) = table_block_inputs(
                boxes,
                styles,
                &pending.grid,
                pending.table,
                computed,
                pending.collapsed_border_metrics.as_ref(),
                pending.font_size,
                pending.containing_height,
                &mut ledger,
            ) else {
                continue;
            };
            let mut formatter = CellFormatter(|request: TableCellLayoutInput| {
                let index = pending
                    .grid
                    .cells
                    .iter()
                    .position(|cell| cell.source == request.box_id)
                    .ok_or(TableRowLayoutError::InvalidCellOutput {
                        box_id: request.box_id,
                    })?;
                let node =
                    pending.cell_nodes[index].ok_or(TableRowLayoutError::InvalidCellOutput {
                        box_id: request.box_id,
                    })?;
                Ok(format_table_cell(
                    tree,
                    node,
                    request,
                    &inputs.cells[index],
                    |context: &mut TextMeasure, geometry| {
                        (
                            context.max_width.min(geometry.width.max(0.0)),
                            context.height,
                        )
                    },
                ))
            });
            pending.block = buckram_table_block(
                &pending.grid,
                pending.table,
                inline,
                &inputs,
                pending.containing_height,
                &mut formatter,
                &mut ledger,
            );
            if let Some(block) = &mut pending.block {
                apply_relative_table_part_offsets(
                    block,
                    pending.table,
                    boxes,
                    styles,
                    pending.font_size,
                    inline.used_grid_inline_size,
                    &mut table_shadow.positioning_gaps,
                );
                commit_table_block(tree, pending.table_node, block, inline, |box_id| {
                    pending
                        .grid
                        .cells
                        .iter()
                        .position(|cell| cell.source == box_id)
                        .and_then(|index| pending.cell_nodes[index])
                });
            }
        }
        table_shadow.block = ledger;
    }

    /// The retained structural paint model for every table Buckram laid out.
    pub(in crate::layout) fn table_paint_plane(&self) -> TablePaintPlane {
        table_paint_plane(&self.pending_tables, self.boxes, self.styles)
    }

    /// Assert the painted fragments honored every assigned column vector, and
    /// record how far the painted cells sit from Buckram's block rectangles.
    /// Runs after fragment collection.
    pub(in crate::layout) fn verify_table_layout(
        &mut self,
        live_rect_of: impl Fn(BoxId) -> Option<Fragment>,
    ) {
        let pendings = std::mem::take(&mut self.pending_tables);
        for pending in pendings {
            let mut ledger = self.table_shadow.take_table(pending.table);
            verify_one_table(&pending, &live_rect_of, &mut ledger);
            self.table_shadow.record_table(pending.table, ledger);
        }
    }

    /// Format each detached table part only after K4d has emitted its
    /// in-flow structural parent. The parent fragment is the zero-track
    /// static-position source; the local root itself never joins the table
    /// algorithm tree or changes a row/column measurement.
    pub(in crate::layout) fn collect_out_of_flow_table_parts(
        &mut self,
        fragments: &mut FragmentTree,
        tables: &TableFragmentPlane,
    ) -> Result<(), LayoutError> {
        let parts = self
            .pending_tables
            .iter()
            .flat_map(|table| table.out_of_flow_parts.iter().copied())
            .collect::<Vec<_>>();
        for part in parts {
            let Some(parent_box) = self.boxes[part.box_id].parent() else {
                continue;
            };
            let Some(parent) = fragments.fragment_ids_for_box(parent_box).last().copied() else {
                continue;
            };
            let Some(containing) = fragments.get(parent).map(TreeFragment::physical_rect) else {
                continue;
            };
            self.tree.compute_layout_with_measure(
                part.node,
                AlgorithmSize::new(
                    AlgorithmAvailableSpace::Definite(containing.width),
                    AlgorithmAvailableSpace::Definite(containing.height),
                ),
                |known, available, _, context, _| {
                    measure_text_algorithm_node(known, available, context)
                },
            );
            let mut output = FragmentOutput { fragments };
            collect_fragments(
                &self.tree,
                self.boxes,
                part.node,
                FragmentCursor {
                    origin: Point {
                        x: containing.x,
                        y: containing.y,
                    },
                    containing,
                    parent: Some(parent),
                },
                tables,
                &mut output,
            )?;
        }
        Ok(())
    }

    /// The single entry every nesting level of the block descent passes
    /// through, so it is where the descent buys stack. `build_box` ->
    /// `build_children` -> `build_flow_children` -> `build_box` is the cycle;
    /// the other two are unreachable except through this one, so guarding here
    /// covers the whole loop with one check per level.
    pub(in crate::layout) fn build_box(
        &mut self,
        box_id: BoxId,
        inherited: Option<&ComputedValues>,
        parent_font_size: f32,
        containing_size: (Option<f32>, Option<f32>),
    ) -> Result<Option<AlgorithmNodeId>, LayoutError> {
        crate::with_recursion_stack(move || {
            self.build_box_on_this_stack(box_id, inherited, parent_font_size, containing_size)
        })
    }

    fn build_box_on_this_stack(
        &mut self,
        box_id: BoxId,
        inherited: Option<&ComputedValues>,
        parent_font_size: f32,
        containing_size: (Option<f32>, Option<f32>),
    ) -> Result<Option<AlgorithmNodeId>, LayoutError> {
        match self.boxes[box_id].origin {
            BoxOrigin::Element(node) => {
                let computed = self.styles.get(node).cloned().unwrap_or_default();
                // K4e1: the wrapper above this grid took the properties
                // CSS 2.1 section 17.4 assigns to it; the grid sees them unset.
                let (computed, table_style) =
                    if self.boxes[box_id].display.internal_table == Some(InternalTableRole::Grid) {
                        (grid_style(&computed, containing_size), Some(computed))
                    } else {
                        (computed, None)
                    };
                let font_size = font_size_px(&computed.font_size, parent_font_size);
                let mut child_containing_size =
                    resolved_child_containing_size(&computed, font_size, containing_size);
                if self
                    .dom
                    .parent(node)
                    .is_some_and(|parent| self.dom.kind(parent) == NodeKind::Document)
                {
                    // The root element's containing block is the initial
                    // containing block. Preserve its definite block size for
                    // percentage-height descendants even when the root's own
                    // height is auto.
                    child_containing_size.1 = child_containing_size.1.or(containing_size.1);
                }
                // A `display: table` box takes its flattened cells directly,
                // so the row-group and row boxes never enter the tree.
                let table = (matches!(
                    computed.display,
                    CssDisplay::Table | CssDisplay::InlineTable
                ))
                .then(|| build_table_grid(self.boxes, self.dom, box_id));
                let mut table_cell_nodes = Vec::new();
                let mut table_out_of_flow_parts = Vec::new();
                let children = if let Some(table) = table.as_ref() {
                    let mut children = Vec::with_capacity(table.cells.len());
                    for cell in &table.cells {
                        let built = self.build_box(
                            cell.source,
                            Some(&computed),
                            font_size,
                            child_containing_size,
                        )?;
                        table_cell_nodes.push(built);
                        let Some(taffy_node) = built else {
                            continue;
                        };
                        children.push(taffy_node);
                    }
                    for part in &table.out_of_flow_parts {
                        let Some(node) = self.build_box(
                            *part,
                            Some(&computed),
                            font_size,
                            child_containing_size,
                        )?
                        else {
                            continue;
                        };
                        table_out_of_flow_parts.push(DetachedTablePart {
                            box_id: *part,
                            node,
                        });
                    }
                    children
                } else {
                    self.boxes[box_id]
                        .children()
                        .iter()
                        .filter_map(|child| {
                            self.build_box(
                                *child,
                                Some(&computed),
                                font_size,
                                child_containing_size,
                            )
                            .transpose()
                        })
                        .collect::<Result<Vec<_>, _>>()?
                };
                let mut taffy_style = to_taffy_style(&computed, font_size);
                taffy_style.size.width =
                    dimension_with_basis(computed.width, font_size, containing_size.0);
                taffy_style.size.height =
                    dimension_with_basis(computed.height, font_size, containing_size.1);
                taffy_style.min_size.width =
                    dimension_with_basis(computed.min_width, font_size, containing_size.0);
                taffy_style.min_size.height =
                    dimension_with_basis(computed.min_height, font_size, containing_size.1);
                taffy_style.max_size.width =
                    dimension_with_basis(computed.max_width, font_size, containing_size.0);
                taffy_style.max_size.height =
                    dimension_with_basis(computed.max_height, font_size, containing_size.1);
                let replaced_size = apply_replaced_intrinsic_style(
                    &mut taffy_style,
                    self.dom,
                    node,
                    &computed,
                    self.image_sources,
                    font_size,
                    matches!(
                        self.boxes[box_id].display.outside,
                        Some(buckram::DisplayOutside::Block)
                    ) && !stretched_by_ancestor_context(self.boxes, box_id),
                    // Percentage padding against an indefinite basis is zero.
                    containing_size.0.unwrap_or(0.0),
                );
                // Taffy exempts a compressible replaced element from block
                // stretch-sizing (CSS 2.1 10.3.4) and from grid `normal`
                // stretching (css-grid-1 6.2). Two conditions narrow it.
                //
                // It is armed only for a box that actually becomes a measured
                // leaf: a `<canvas>` with fallback content is a block container,
                // and arming it there would let Taffy shrink-wrap the fallback
                // instead of laying it out.
                //
                // And only under `content-box`. Arming it for a border-box
                // replaced element changes which path applies CSS 2.1 10.4's
                // ratio-preserving min/max clamp, and box-sizing-replaced-001,
                // -002 and -003 fail when it does. The cost is named: a
                // border-box replaced element still stretches, in a block
                // container and as a grid item alike.
                // Since taffy's block path stopped reading this flag, arming it
                // reaches only the grid `normal` exemption, so border-box leaves
                // are safe to include: a border-box replaced grid item no longer
                // stretches either.
                taffy_style.item_is_replaced = replaced_size.is_some() && children.is_empty();
                let block_style =
                    to_block_style(self.boxes, self.styles, box_id, &computed, font_size);
                let kind = algorithm_kind(&self.boxes[box_id], children.is_empty());
                let dom_node = node;
                let node =
                    if let Some((width, height)) = replaced_size.filter(|_| children.is_empty()) {
                        self.tree.new_leaf_with_context_and_block_style(
                            block_style,
                            taffy_style,
                            TextMeasure {
                                min_width: width,
                                max_width: width,
                                height,
                            },
                            Some(box_id),
                        )
                    } else {
                        self.tree.new_with_children_and_block_style(
                            kind,
                            block_style,
                            taffy_style,
                            &children,
                            Some(box_id),
                        )
                    };
                if let Some(input) = lower_sequential_multicol_if_allowed(
                    &computed,
                    block_style,
                    self.has_multicol_ancestor(box_id),
                ) {
                    self.tree.set_sequential_multicol(node, input);
                }
                enable_flex_grid_static_position_provider(
                    &mut self.tree,
                    self.styles,
                    self.boxes,
                    box_id,
                    node,
                );
                // K4c5b: Buckram owns this table's columns. They are computed
                // before the main layout pass, once the whole tree exists and
                // intrinsic queries can run, and pinned as explicit tracks.
                if let Some(grid) = table {
                    self.pending_tables.push(PendingTable {
                        table: box_id,
                        node: Some(dom_node),
                        table_style: table_style.unwrap_or_default(),
                        table_node: node,
                        wrapper: None,
                        captions: Vec::new(),
                        grid,
                        collapsed_borders: None,
                        collapsed_border_metrics: None,
                        cell_nodes: std::mem::take(&mut table_cell_nodes),
                        out_of_flow_parts: std::mem::take(&mut table_out_of_flow_parts),
                        font_size,
                        containing_width: containing_size.0,
                        containing_height: containing_size.1,
                        assigned: None,
                        block: None,
                    });
                }
                if supports_nested_float_state(&self.boxes[box_id], block_style, kind) {
                    self.tree.enable_nested_float_state(node);
                }
                if block_style.float != FloatSide::None
                    && self.boxes[box_id].float_context == FloatContextProvenance::Inline
                {
                    self.tree.mark_inline_context_float(node);
                }
                if supports_float_avoidance(&self.boxes[box_id], block_style, kind) {
                    self.tree.enable_float_avoidance(node);
                }
                if supports_intrinsic_shrink_to_fit(
                    &self.tree,
                    node,
                    self.boxes,
                    box_id,
                    &computed,
                    block_style,
                    kind,
                ) {
                    self.tree.enable_intrinsic_shrink_to_fit(node);
                }
                Ok(Some(node))
            },
            BoxOrigin::Text(node) => {
                let text = self.dom.text(node).unwrap_or("");
                let preserves_whitespace = inherited.is_some_and(|style| {
                    matches!(
                        style.white_space_collapse,
                        WhiteSpaceCollapse::Preserve | WhiteSpaceCollapse::BreakSpaces
                    )
                });
                if text.is_empty() || (!preserves_whitespace && is_collapsible_whitespace(text)) {
                    return Ok(None);
                }
                let font_size = parent_font_size;
                let line_height = inherited
                    .map(|style| line_height_px(&style.line_height, font_size))
                    .unwrap_or(font_size * 1.2);
                let mut min_width = if preserves_whitespace {
                    text.lines()
                        .map(|line| line.chars().count())
                        .max()
                        .unwrap_or(0)
                } else {
                    collapsed_word_width(text)
                } as f32
                    * font_size
                    * 0.6;
                let mut max_width = if preserves_whitespace {
                    min_width
                } else {
                    collapsed_text_width(text) as f32 * font_size * 0.6
                };
                let line_count = if preserves_whitespace {
                    text.lines().count().max(1)
                } else {
                    1
                };
                let mut height = line_count as f32 * line_height;
                if let Some(text_system) = self.text.as_deref_mut()
                    && let Some(parent_style) = inherited
                {
                    let fragments = AtomicLayoutPlane::default();
                    let roots = [box_id];
                    let minimum = text_system
                        .format_inline_group(
                            self.dom,
                            self.styles,
                            self.boxes,
                            &fragments,
                            InlineRequest {
                                roots: &roots,
                                parent_style,
                                width: 0.01,
                                intrinsic_kind: Some(IntrinsicSizeKind::MinContent),
                                line_constraints: None,
                            },
                        )
                        .map(|layout| layout.size());
                    let maximum = text_system
                        .format_inline_group(
                            self.dom,
                            self.styles,
                            self.boxes,
                            &fragments,
                            InlineRequest {
                                roots: &roots,
                                parent_style,
                                width: f32::INFINITY,
                                intrinsic_kind: Some(IntrinsicSizeKind::MaxContent),
                                line_constraints: None,
                            },
                        )
                        .map(|layout| layout.size());
                    if let Some((minimum, _)) = minimum {
                        min_width = minimum;
                    }
                    if let Some((maximum, maximum_height)) = maximum {
                        max_width = maximum.max(min_width);
                        height = maximum_height;
                    }
                }
                let node = self.tree.new_leaf_with_context_and_block_style(
                    anonymous_block_style(self.boxes, box_id),
                    Style {
                        display: Display::Block,
                        ..Style::default()
                    },
                    TextMeasure {
                        min_width,
                        max_width,
                        height,
                    },
                    Some(box_id),
                );
                Ok(Some(node))
            },
            BoxOrigin::Pseudo {
                owner,
                pseudo: buckram::PseudoElement::Marker,
            } => {
                let marker_only_inline_run = self.boxes[box_id].parent().is_some_and(|parent| {
                    matches!(self.boxes[parent].origin, BoxOrigin::Anonymous { .. })
                        && self.boxes[parent].formatting_context
                            == Some(FormattingContextKind::Inline)
                        && self.boxes[parent].children() == [box_id]
                });
                if !marker_only_inline_run {
                    return Ok(None);
                }
                let Some(style) = self.styles.get(owner) else {
                    return Ok(None);
                };
                let Some(marker) =
                    crate::text::inside_marker_text(self.dom, self.styles, owner, style)
                else {
                    return Ok(None);
                };
                let font_size = font_size_px(&style.font_size, parent_font_size);
                let line_height = line_height_px(&style.line_height, font_size);
                // Marker strings preserve repeated authored spaces.
                let marker_width = marker.chars().count() as f32 * font_size * 0.6;
                let min_width = marker_width;
                let max_width = marker_width;
                let node = self.tree.new_leaf_with_context_and_block_style(
                    anonymous_block_style(self.boxes, box_id),
                    Style {
                        display: Display::Block,
                        ..Style::default()
                    },
                    TextMeasure {
                        min_width,
                        max_width: max_width.max(min_width),
                        height: line_height,
                    },
                    Some(box_id),
                );
                Ok(Some(node))
            },
            BoxOrigin::Pseudo { .. } | BoxOrigin::Anonymous { .. } => {
                if let Some(grid) = (self.boxes[box_id].display.internal_table
                    == Some(InternalTableRole::Wrapper))
                .then(|| wrapped_table_grid(self.boxes, box_id))
                .flatten()
                {
                    // See InlineBuildState's corresponding K4e1 wrapper.
                    let table = match legacy_origin_node(self.boxes, grid) {
                        Some(element) => self.styles.get(element).cloned().unwrap_or_default(),
                        None => anonymous_table_style(inherited),
                    };
                    let computed = wrapper_style(&table);
                    let font_size = font_size_px(&computed.font_size, parent_font_size);
                    let mut caption_nodes = Vec::new();
                    let mut children = Vec::new();
                    for child in wrapper_children_in_caption_order(self.boxes, self.styles, box_id)
                    {
                        let Some(child_node) =
                            self.build_box(child, inherited, parent_font_size, containing_size)?
                        else {
                            continue;
                        };
                        if self.boxes[child].display.internal_table
                            == Some(InternalTableRole::Caption)
                            && matches!(
                                self.boxes[child].positioning,
                                PositioningScheme::Static
                                    | PositioningScheme::Relative
                                    | PositioningScheme::Sticky
                            )
                        {
                            let caption = self
                                .boxes
                                .origin_node(child)
                                .and_then(|node| self.styles.get(node))
                                .cloned()
                                .unwrap_or_default();
                            let em = font_size_px(&caption.font_size, font_size);
                            caption_nodes.push((
                                child_node,
                                caption_horizontal_margins(&caption, em, containing_size.0),
                            ));
                        }
                        children.push(child_node);
                    }
                    let mut taffy_style = to_taffy_style(&computed, font_size);
                    let logical_wrapper =
                        wrapper_uses_logical_block_axis(&mut taffy_style, self.boxes[box_id].flow);
                    if wrapper_needs_float_fallback(self.boxes, box_id, &taffy_style) {
                        taffy_style.float = TaffyFloat::Left;
                    }
                    let wrapper_grid_width = wrapper_width_from_grid(&to_taffy_style(
                        &grid_style(&table, containing_size),
                        font_size,
                    ));
                    if let Some(width) = wrapper_grid_width {
                        taffy_style.size.width = width;
                    }
                    let block_style =
                        to_block_style(self.boxes, self.styles, box_id, &computed, font_size);
                    let kind = if logical_wrapper {
                        AlgorithmKind::Flex
                    } else {
                        algorithm_kind(&self.boxes[box_id], children.is_empty())
                    };
                    let node = self.tree.new_with_children_and_block_style(
                        kind,
                        block_style,
                        taffy_style,
                        &children,
                        Some(box_id),
                    );
                    if let Some(width) = wrapper_grid_width.and_then(Dimension::into_option) {
                        self.tree.set_table_wrapper_inline_size(node, width);
                    }
                    enable_flex_grid_static_position_provider(
                        &mut self.tree,
                        self.styles,
                        self.boxes,
                        box_id,
                        node,
                    );
                    if let Some(pending) = self
                        .pending_tables
                        .iter_mut()
                        .find(|pending| pending.table == grid)
                    {
                        pending.wrapper = Some(node);
                        pending.captions = caption_nodes;
                    }
                    return Ok(Some(node));
                }
                if self.boxes[box_id].display.internal_table == Some(InternalTableRole::Grid) {
                    return self.build_anonymous_table_grid(
                        box_id,
                        inherited,
                        parent_font_size,
                        containing_size,
                    );
                }
                let computed = inherited.cloned().unwrap_or_default();
                let children = self.boxes[box_id]
                    .children()
                    .iter()
                    .filter_map(|child| {
                        self.build_box(*child, Some(&computed), parent_font_size, containing_size)
                            .transpose()
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let block_style = anonymous_block_style(self.boxes, box_id);
                let kind = algorithm_kind(&self.boxes[box_id], children.is_empty());
                let node = self.tree.new_with_children_and_block_style(
                    kind,
                    block_style,
                    anonymous_taffy_style(&self.boxes[box_id]),
                    &children,
                    Some(box_id),
                );
                enable_flex_grid_static_position_provider(
                    &mut self.tree,
                    self.styles,
                    self.boxes,
                    box_id,
                    node,
                );
                if supports_nested_float_state(&self.boxes[box_id], block_style, kind) {
                    self.tree.enable_nested_float_state(node);
                }
                Ok(Some(node))
            },
        }
    }
}
