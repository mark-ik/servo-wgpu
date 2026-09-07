// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    Device, InteractionStates, StyleSet, emit_paint_list_with_text_system, resolve_styles,
};
use genet_static_dom::StaticDocument;
use paint_list_api::DeviceIntSize;

fn node_by_id(
    dom: &StaticDocument,
    node: <StaticDocument as LayoutDom>::NodeId,
    id: &str,
) -> Option<<StaticDocument as LayoutDom>::NodeId> {
    if dom.attribute(node, &Namespace::from(""), &LocalName::from("id")) == Some(id) {
        return Some(node);
    }
    dom.dom_children(node)
        .find_map(|child| node_by_id(dom, child, id))
}

#[test]
fn livery_lowers_only_definite_auto_multicol_inputs() {
    let dom = StaticDocument::parse(
        "<div id=columns style='width:264px;height:120px;column-count:3;column-gap:12px;column-fill:auto'></div>",
    );
    let node = node_by_id(&dom, dom.document(), "columns").expect("columns");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let input = super::build_block::lower_sequential_multicol(
        styles.get(node).expect("computed style"),
        BlockStyle::default(),
    )
    .expect("definite auto columns are lowered");
    assert_eq!(input.column_count(), 3);
    assert_eq!(input.column_width(), None);
    assert_eq!(input.column_gap(), 12.0);

    let balance = StaticDocument::parse(
        "<div id=columns style='width:264px;height:120px;column-count:3;column-gap:12px;column-fill:balance'></div>",
    );
    let balance_node = node_by_id(&balance, balance.document(), "columns").expect("columns");
    let balance_styles = resolve_styles(
        &balance,
        &StyleSet::cambium(&[]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    assert!(
        super::build_block::lower_sequential_multicol(
            balance_styles.get(balance_node).expect("computed style"),
            BlockStyle::default(),
        )
        .is_none()
    );

    let explicit_width = StaticDocument::parse(
        "<div id=columns style='width:264px;height:120px;column-count:3;column-width:80px;column-gap:12px;column-fill:auto'></div>",
    );
    let explicit_node =
        node_by_id(&explicit_width, explicit_width.document(), "columns").expect("columns");
    let explicit_styles = resolve_styles(
        &explicit_width,
        &StyleSet::cambium(&[]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let explicit = super::build_block::lower_sequential_multicol(
        explicit_styles.get(explicit_node).expect("computed style"),
        BlockStyle::default(),
    )
    .expect("explicit column width is lowered as style input");
    assert_eq!(explicit.column_width(), Some(80.0));

    let padded = super::build_block::lower_sequential_multicol(
        styles.get(node).expect("computed style"),
        BlockStyle {
            padding: PhysicalSides::splat(FlowLength::px(8.0)),
            border: PhysicalSides::splat(2.0),
            ..BlockStyle::default()
        },
    )
    .expect("padding and border do not become fake used geometry");
    assert_eq!(padded, input);

    let indefinite = StaticDocument::parse(
        "<div id=columns style='width:264px;column-count:3;column-gap:12px;column-fill:auto'></div>",
    );
    let indefinite_node =
        node_by_id(&indefinite, indefinite.document(), "columns").expect("columns");
    let indefinite_styles = resolve_styles(
        &indefinite,
        &StyleSet::cambium(&[]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    assert!(
        super::build_block::lower_sequential_multicol(
            indefinite_styles
                .get(indefinite_node)
                .expect("computed style"),
            BlockStyle::default(),
        )
        .is_none()
    );

    assert!(
        super::build_block::lower_sequential_multicol(
            styles.get(node).expect("computed style"),
            BlockStyle {
                flow: FlowAxes::new(
                    buckram::WritingMode::VerticalRl,
                    buckram::Direction::Ltr,
                ),
                ..BlockStyle::default()
            },
        )
        .is_none()
    );
}

#[test]
fn nested_multicol_build_admits_only_the_outer_algorithm_node() {
    let dom = StaticDocument::parse(
        "<div id=outer style='width:264px;height:120px;column-count:3;column-gap:12px;column-fill:auto'><div id=inner style='width:264px;height:120px;column-count:3;column-gap:12px;column-fill:auto'></div></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let outer = node_by_id(&dom, dom.document(), "outer").expect("outer");
    let inner = node_by_id(&dom, dom.document(), "inner").expect("inner");
    let boxes = GeneratedBoxTree::from_dom(&dom, &styles);
    let outer_box = boxes.principal_box(outer).expect("outer box");
    let inner_box = boxes.principal_box(inner).expect("inner box");
    let image_sources = HashMap::new();
    let mut state = BuildState {
        dom: &dom,
        styles: &styles,
        boxes: &boxes,
        tree: AlgorithmTree::new(),
        image_sources: &image_sources,
        text: None,
        table_shadow: TableShadowLedger::default(),
        pending_tables: Vec::new(),
    };
    let outer_node = state
        .build_box(outer_box, None, 16.0, (Some(320.0), Some(240.0)))
        .expect("outer build")
        .expect("outer algorithm node");
    let inner_node = state
        .tree
        .node_ids()
        .find(|id| state.tree.source(*id) == &Some(inner_box))
        .expect("inner algorithm node");
    assert!(state.tree.sequential_multicol(outer_node).is_some());
    assert!(state.tree.sequential_multicol(inner_node).is_none());
}

#[test]
fn flex_child_align_self_projects_parent_cross_axis_and_subject_flow() {
    let dom = StaticDocument::parse("<div id=flex><div id=item></div></div>");
    let project = |parent_css: &str, child_css: &str| {
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&[&format!(
                "#flex {{ display: flex; {parent_css} }} #item {{ {child_css} }}"
            )]),
            &Device::screen(320.0, 240.0),
            &InteractionStates::default(),
        );
        let parent_node = node_by_id(&dom, dom.document(), "flex").expect("flex");
        let item_node = node_by_id(&dom, dom.document(), "item").expect("item");
        let parent = styles.get(parent_node).expect("parent style");
        let item = styles.get(item_node).expect("item style");
        let mut style = to_taffy_style(item, 16.0);
        map_flex_child_self_alignment(&mut style, parent, flex_flow_axes(item), item);
        style.align_self.map(|alignment| alignment.keyword)
    };

    for (description, parent_css, child_css, expected) in [
        (
            "vertical-rl rtl column nowrap start",
            "writing-mode: vertical-rl; direction: rtl; flex-flow: column nowrap;",
            "align-self: start;",
            AlignItemsKeyword::End,
        ),
        (
            "vertical-rl rtl column nowrap flex-start",
            "writing-mode: vertical-rl; direction: rtl; flex-flow: column nowrap;",
            "align-self: flex-start;",
            AlignItemsKeyword::FlexEnd,
        ),
        (
            "vertical-rl rtl column wrap-reverse flex-start",
            "writing-mode: vertical-rl; direction: rtl; flex-flow: column wrap-reverse;",
            "align-self: flex-start;",
            AlignItemsKeyword::FlexStart,
        ),
        (
            "vertical-lr rtl column nowrap end",
            "writing-mode: vertical-lr; direction: rtl; flex-flow: column nowrap;",
            "align-self: end;",
            AlignItemsKeyword::Start,
        ),
        (
            "physical column right cross start, horizontal subject self-start",
            "writing-mode: vertical-rl; flex-flow: row nowrap;",
            "writing-mode: horizontal-tb; align-self: self-start;",
            AlignItemsKeyword::End,
        ),
        (
            "physical column right cross start, vertical-rl subject self-start",
            "writing-mode: vertical-rl; flex-flow: row nowrap;",
            "writing-mode: vertical-rl; align-self: self-start;",
            AlignItemsKeyword::Start,
        ),
        (
            "physical column right cross start, horizontal subject self-end",
            "writing-mode: vertical-rl; flex-flow: row nowrap;",
            "writing-mode: horizontal-tb; align-self: self-end;",
            AlignItemsKeyword::Start,
        ),
        (
            "physical column right cross start, vertical-rl subject self-end",
            "writing-mode: vertical-rl; flex-flow: row nowrap;",
            "writing-mode: vertical-rl; align-self: self-end;",
            AlignItemsKeyword::End,
        ),
        (
            "physical row content fallback reads child height",
            "writing-mode: vertical-rl; direction: rtl; flex-flow: column wrap-reverse;",
            "height: max-content; align-self: auto;",
            AlignItemsKeyword::FlexStart,
        ),
        (
            "physical column content fallback reads child width",
            "writing-mode: vertical-rl; flex-flow: row nowrap;",
            "width: max-content; align-self: auto;",
            AlignItemsKeyword::FlexStart,
        ),
        (
            "auto content width inherits center instead of stretch fallback",
            "writing-mode: vertical-rl; flex-flow: row nowrap; align-items: center;",
            "width: max-content; align-self: auto;",
            AlignItemsKeyword::Center,
        ),
        (
            "explicit stretch content width uses the flex-start fallback",
            "writing-mode: vertical-rl; flex-flow: row nowrap;",
            "width: max-content; align-self: stretch;",
            AlignItemsKeyword::FlexStart,
        ),
        (
            "explicit center content width does not use the stretch fallback",
            "writing-mode: vertical-rl; flex-flow: row nowrap;",
            "width: max-content; align-self: center;",
            AlignItemsKeyword::Center,
        ),
        (
            "auto content width inherits end instead of stretch fallback",
            "writing-mode: vertical-rl; flex-flow: row nowrap; align-items: end;",
            "width: max-content; align-self: auto;",
            AlignItemsKeyword::End,
        ),
    ] {
        assert_eq!(
            project(parent_css, child_css),
            Some(expected),
            "{description}"
        );
    }

    assert_eq!(
        project(
            "writing-mode: vertical-rl; flex-flow: row nowrap;",
            "align-self: auto;",
        ),
        None,
        "ordinary auto still inherits the parent's align-items",
    );
    assert_eq!(
        project(
            "writing-mode: vertical-rl; flex-flow: row nowrap;",
            "height: max-content; align-self: auto;",
        ),
        None,
        "a physical column ignores the child's main-axis content height",
    );
    assert_eq!(
        project(
            "writing-mode: vertical-rl; flex-flow: column nowrap;",
            "width: max-content; align-self: auto;",
        ),
        None,
        "a physical row ignores the child's main-axis content width",
    );
}

#[test]
fn auto_align_self_resolves_parent_self_edges_against_the_subject_flow() {
    let mut parent = ComputedValues::default();
    parent.display = CssDisplay::Flex;
    parent.writing_mode = CssWritingMode::VerticalRl;
    parent.flex_direction = CssFlexDirection::Row;
    let mut horizontal = ComputedValues::default();
    horizontal.align_self = CssAlignment::Auto;
    let mut vertical = horizontal.clone();
    vertical.writing_mode = CssWritingMode::VerticalRl;

    let project = |parent: &ComputedValues, subject: &ComputedValues| {
        let mut style = to_taffy_style(subject, 16.0);
        map_flex_child_self_alignment(&mut style, parent, flex_flow_axes(subject), subject);
        style.align_self.map(|alignment| alignment.keyword)
    };

    parent.align_items = CssAlignment::SelfStart;
    assert_eq!(
        project(&parent, &horizontal),
        Some(AlignItemsKeyword::End),
        "horizontal subject self-start is its physical left edge",
    );
    assert_eq!(
        project(&parent, &vertical),
        Some(AlignItemsKeyword::Start),
        "vertical-rl subject self-start is its physical right edge",
    );

    parent.align_items = CssAlignment::SelfEnd;
    assert_eq!(
        project(&parent, &horizontal),
        Some(AlignItemsKeyword::Start),
        "horizontal subject self-end is its physical right edge",
    );
    assert_eq!(
        project(&parent, &vertical),
        Some(AlignItemsKeyword::End),
        "vertical-rl subject self-end is its physical left edge",
    );
}

#[test]
fn flex_child_style_projection_excludes_non_element_provenance() {
    assert_eq!(element_origin_node(BoxOrigin::Element(1u8)), Some(1));
    assert_eq!(element_origin_node(BoxOrigin::Text(1u8)), None);
    assert_eq!(
        element_origin_node(BoxOrigin::Pseudo {
            owner: 1u8,
            pseudo: buckram::PseudoElement::Before,
        }),
        None,
    );
    assert_eq!(
        element_origin_node(BoxOrigin::Anonymous {
            owner: Some(1u8),
            kind: buckram::AnonymousBoxKind::Block,
        }),
        None,
    );
}

#[test]
fn non_flex_vertical_style_does_not_receive_flex_direction_lowering() {
    let dom = StaticDocument::parse("<div id=block></div>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "#block { writing-mode: vertical-rl; direction: rtl; display: block; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let block = styles
        .get(node_by_id(&dom, dom.document(), "block").expect("block"))
        .expect("block style");
    assert_eq!(to_taffy_style(block, 16.0).direction, TaffyDirection::Ltr);
}

#[test]
fn html_table_spans_are_normalized_before_buckram_receives_them() {
    let dom = StaticDocument::parse(
        "<table id=table><tbody><tr><td id=first colspan=9001 rowspan=0></td></tr><tr><td id=second></td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "table { display: table; } tbody { display: table-row-group; } tr { display: table-row; } td { display: table-cell; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let boxes = GeneratedBoxTree::from_dom(&dom, &styles);
    let table = node_by_id(&dom, dom.document(), "table").expect("table");
    let grid = boxes.principal_box(table).expect("table grid");
    let model = build_table_grid(&boxes, &dom, grid);

    assert_eq!(model.cells[0].column_span, 1_000);
    assert_eq!(model.cells[0].row_span, 2);
    assert_eq!(model.cells[1].column, 1_000);
}

#[test]
fn css_display_tables_do_not_consume_html_span_attributes() {
    let dom = StaticDocument::parse(
        "<div id=table><div id=row><div id=cell colspan=9 rowspan=0></div></div></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "#table { display: table; } #row { display: table-row; } #cell { display: table-cell; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let boxes = GeneratedBoxTree::from_dom(&dom, &styles);
    let table = node_by_id(&dom, dom.document(), "table").expect("table");
    let grid = boxes.principal_box(table).expect("table grid");
    let model = build_table_grid(&boxes, &dom, grid);

    assert_eq!(model.cells[0].column_span, 1);
    assert_eq!(model.cells[0].row_span, 1);
}

#[test]
fn tables_dispatch_through_buckram_without_a_grid_bridge() {
    let dom =
        StaticDocument::parse("<table><tbody><tr><td>one</td><td>two</td></tr></tbody></table>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "table { display: table; } tbody { display: table-row-group; } tr { display: table-row; } td { display: table-cell; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let ledger = layout.table_shadow_ledger();
    assert_eq!(
        ledger.assigned, 1,
        "Buckram must assign the table: {ledger:?}"
    );
    assert_eq!(
        ledger.honored, 1,
        "the committed table must honor Buckram columns: {ledger:?}"
    );
    assert_eq!(
        ledger.block.laid_out, 1,
        "Buckram must commit the table block axis: {ledger:?}"
    );
    assert!(
        ledger.skipped.is_empty() && ledger.block.skipped.is_empty(),
        "the basic table may not fall back to a backend route: {ledger:?}"
    );
}

#[test]
fn absolute_static_position_keeps_its_formatting_source_and_k5a_containing_block() {
    let dom = StaticDocument::parse(
        "<div id=containing><div id=source><div id=positioned>item</div></div></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "#containing { position: relative; width: 200px; } #source { width: 120px; } \
             #positioned { position: absolute; left: 36px; top: 11px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let by_id = |id| node_by_id(&dom, dom.document(), id).expect("node");
    let source = layout
        .boxes()
        .principal_box(by_id("source"))
        .expect("source box");
    let containing = layout
        .boxes()
        .principal_box(by_id("containing"))
        .expect("containing box");
    let positioned = layout
        .boxes()
        .principal_box(by_id("positioned"))
        .expect("positioned box");
    let source_fragment = layout
        .fragments()
        .fragment_ids_for_box(source)
        .first()
        .copied()
        .expect("source fragment");
    let static_position = layout
        .fragments()
        .static_position_for_box(positioned)
        .expect("static-position record");

    assert_eq!(
        static_position.source,
        StaticPositionSource::Fragment(source_fragment),
        "the record must keep the source formatting fragment",
    );
    assert_eq!(
        static_position.containing_block,
        buckram::ContainingBlock::Box(containing),
        "the absolute containing block comes from the K5a graph, not the source parent",
    );
    assert_eq!(
        (
            static_position.logical_rect.inline_start,
            static_position.logical_rect.block_start
        ),
        (0.0, 0.0),
        "the K5b record is the pre-inset static position, not the final absolute location",
    );
    let containing_fragment = layout
        .fragments()
        .fragment_ids_for_box(containing)
        .first()
        .copied()
        .expect("containing fragment");
    let containing_fragment_rect = layout
        .fragments()
        .get(containing_fragment)
        .map(TreeFragment::physical_rect)
        .expect("containing fragment geometry");
    let positioned_fragment = layout
        .fragments()
        .fragments_for_box(positioned)
        .next()
        .expect("positioned fragment");
    assert_eq!(
        (positioned_fragment.x, positioned_fragment.y),
        (
            containing_fragment_rect.x + 36.0,
            containing_fragment_rect.y + 11.0
        ),
        "K5d resolves final insets after K5b records the static rectangle",
    );
    assert_eq!(
        positioned_fragment.containing_fragment(),
        Some(containing_fragment),
        "the final fragment attaches to K5a's selected containing fragment",
    );
}

#[test]
fn absolute_auto_inline_size_fills_between_definite_insets_from_buckram_inputs() {
    let dom = StaticDocument::parse(
        "<div id=containing><div id=positioned>unconstrained content</div></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #containing { position: relative; width: 200px; } \
             #positioned { position: absolute; left: 10px; right: 20px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let positioned = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "positioned").expect("positioned"))
        .expect("positioned box");
    let fragment = layout
        .fragments()
        .fragments_for_box(positioned)
        .next()
        .expect("positioned fragment");

    assert_eq!(fragment.width, 170.0);
    assert_eq!(fragment.x, 10.0);
}

#[test]
fn absolute_nonleaf_reformats_at_buckrams_resolved_inline_size() {
    let dom = StaticDocument::parse(
        "<div id=containing><div id=positioned><div id=child></div></div></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #containing { position: relative; width: 200px; height: 100px; } \
             #positioned { position: absolute; left: 10px; right: 20px; top: 7px; } \
             #child { width: 40px; height: 20px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let rect_for = |id| {
        let box_id = layout
            .boxes()
            .principal_box(node_by_id(&dom, dom.document(), id).expect(id))
            .expect("principal box");
        layout
            .fragments()
            .fragments_for_box(box_id)
            .next()
            .map(TreeFragment::physical_rect)
            .expect("fragment")
    };
    let positioned = rect_for("positioned");
    let child = rect_for("child");

    assert_eq!(
        positioned,
        PhysicalRect {
            x: 10.0,
            y: 7.0,
            width: 170.0,
            height: 20.0,
        },
        "the non-leaf root reformats at Buckram's final used width",
    );
    assert_eq!(
        child,
        PhysicalRect {
            x: 10.0,
            y: 7.0,
            width: 40.0,
            height: 20.0,
        },
        "the descendant belongs to the reformatted positioned root",
    );
}

#[test]
fn vertical_absolute_nonleaf_reformats_at_buckrams_resolved_inline_size() {
    let dom = StaticDocument::parse(
        "<div id=containing><div id=positioned><div id=child></div></div></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #containing { position: relative; writing-mode: vertical-rl; width: 100px; height: 200px; } \
             #positioned { position: absolute; writing-mode: vertical-rl; left: 7px; top: 10px; bottom: 20px; } \
             #child { width: 40px; height: 20px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let rect_for = |id| {
        let box_id = layout
            .boxes()
            .principal_box(node_by_id(&dom, dom.document(), id).expect(id))
            .expect("principal box");
        layout
            .fragments()
            .fragments_for_box(box_id)
            .next()
            .map(TreeFragment::physical_rect)
            .expect("fragment")
    };
    let positioned = rect_for("positioned");
    let child = rect_for("child");

    assert_eq!(
        positioned,
        PhysicalRect {
            x: 7.0,
            y: 10.0,
            width: 40.0,
            height: 170.0,
        },
        "the vertical non-leaf root reformats at Buckram's final used inline size",
    );
    assert_eq!(
        child,
        PhysicalRect {
            x: 7.0,
            y: 10.0,
            width: 40.0,
            height: 20.0,
        },
        "the descendant belongs to the reformatted vertical positioned root",
    );
}

#[test]
fn absolute_empty_leaf_uses_buckrams_resolved_border_box() {
    let dom = StaticDocument::parse("<div id=containing><div id=positioned></div></div>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #containing { position: relative; width: 200px; height: 100px; } \
             #positioned { position: absolute; left: 10px; right: 20px; top: 7px; height: 30px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let positioned = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "positioned").expect("positioned"))
        .expect("positioned box");
    let fragment = layout
        .fragments()
        .fragments_for_box(positioned)
        .next()
        .expect("positioned fragment");

    assert_eq!(
        fragment.physical_rect(),
        PhysicalRect {
            x: 10.0,
            y: 7.0,
            width: 170.0,
            height: 30.0,
        }
    );
}

#[test]
fn fixed_leaf_percentage_block_size_uses_the_initial_containing_block() {
    let dom = StaticDocument::parse("<div id=fixed></div>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #fixed { position: fixed; left: 50px; top: 50px; width: 50%; height: 50%; border: 10px solid; }"]),
        &Device::screen(800.0, 600.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 800.0, 600.0).expect("layout");
    let fixed = layout
        .get(node_by_id(&dom, dom.document(), "fixed").expect("fixed node"))
        .expect("fixed fragment")
        .physical_rect();

    assert_eq!(
        fixed,
        PhysicalRect {
            x: 50.0,
            y: 50.0,
            width: 420.0,
            height: 320.0,
        }
    );
}

#[test]
fn absolute_non_leaf_percentage_block_size_uses_the_initial_containing_block() {
    let dom = StaticDocument::parse("<body id=positioned><div></div></body>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #positioned { position: absolute; left: 50px; top: 50px; width: 50%; height: 50%; border: 10px solid; }"]),
        &Device::screen(800.0, 600.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 800.0, 600.0).expect("layout");
    let positioned = layout
        .get(node_by_id(&dom, dom.document(), "positioned").expect("positioned node"))
        .expect("positioned fragment")
        .physical_rect();

    assert_eq!(
        positioned,
        PhysicalRect {
            x: 50.0,
            y: 50.0,
            width: 420.0,
            height: 320.0,
        }
    );
}

#[test]
fn ordinary_block_flow_keeps_an_absolute_subtree_out_of_its_cursor() {
    let dom = StaticDocument::parse(
        "<div id=host><div id=before></div><div id=positioned><div id=inside></div></div><div id=after></div></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #host { position: relative; width: 200px; } \
             #before { height: 20px; } \
             #positioned { position: absolute; left: 25px; width: 80px; } \
             #inside { height: 30px; } \
             #after { height: 10px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let rect = |id| {
        layout
            .get(node_by_id(&dom, dom.document(), id).expect("node"))
            .map(TreeFragment::physical_rect)
            .expect("fragment")
    };
    let host = rect("host");
    let positioned = rect("positioned");
    let after = rect("after");
    let algorithms = layout.block_algorithm_counts();

    assert_eq!(
        host.height, 30.0,
        "the absolute child does not size its block parent"
    );
    assert_eq!((after.x - host.x, after.y - host.y), (0.0, 20.0));
    assert_eq!(
        (
            positioned.x - host.x,
            positioned.y - host.y,
            positioned.width,
            positioned.height
        ),
        (25.0, 20.0, 80.0, 30.0),
    );
    assert_eq!(
        algorithms.taffy, 0,
        "an ordinary block parent and its positioned block subtree stay on Buckram's cursor",
    );
}

#[test]
fn relative_position_moves_its_fragment_subtree_without_reflowing_siblings() {
    let dom = StaticDocument::parse(
        "<div id=relative><div id=child>child</div></div><div id=following>following</div>",
    );
    let static_styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "#relative { width: 120px; } #child { width: 40px; } #following { width: 80px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let relative_styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "#relative { position: relative; left: 21px; top: 13px; width: 120px; } \
             #child { width: 40px; } #following { width: 80px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let static_layout = layout(&dom, &static_styles, 320.0, 240.0).expect("static layout");
    let relative_layout = layout(&dom, &relative_styles, 320.0, 240.0).expect("relative layout");
    let box_for = |layout: &LiveryLayout<_>, id| {
        layout
            .boxes()
            .principal_box(node_by_id(&dom, dom.document(), id).expect("node"))
            .expect("principal box")
    };
    let rect_for = |layout: &LiveryLayout<_>, id| {
        layout
            .fragments()
            .fragments_for_box(box_for(layout, id))
            .next()
            .map(TreeFragment::physical_rect)
            .expect("fragment")
    };

    let static_relative = rect_for(&static_layout, "relative");
    let static_child = rect_for(&static_layout, "child");
    let static_following = rect_for(&static_layout, "following");
    let positioned_relative = rect_for(&relative_layout, "relative");
    let positioned_child = rect_for(&relative_layout, "child");
    let positioned_following = rect_for(&relative_layout, "following");

    assert_eq!(
        (positioned_relative.x, positioned_relative.y),
        (static_relative.x + 21.0, static_relative.y + 13.0),
    );
    assert_eq!(
        (positioned_child.x, positioned_child.y),
        (static_child.x + 21.0, static_child.y + 13.0),
        "the containing-block subtree moves with the relative box",
    );
    assert_eq!(
        positioned_following, static_following,
        "relative positioning does not change following normal-flow geometry",
    );
}

#[test]
fn inline_origin_absolute_position_uses_the_line_fragment_as_its_static_source() {
    let dom = StaticDocument::parse(
        "<div id=container>before <span id=source>source <span id=positioned>item</span></span> after</div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "#container { position: relative; width: 160px; } #source { display: inline; } \
             #positioned { position: absolute; left: 34px; top: 8px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let source = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "source").expect("source"))
        .expect("source box");
    let positioned = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "positioned").expect("positioned"))
        .expect("positioned box");
    let source_fragment = layout
        .fragments()
        .fragment_ids_for_box(source)
        .first()
        .copied()
        .expect("source line fragment");
    let static_position = layout
        .fragments()
        .static_position_for_box(positioned)
        .expect("static position");
    let positioned_fragment = layout
        .fragments()
        .fragments_for_box(positioned)
        .next()
        .expect("positioned fragment");
    let container = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "container").expect("container"))
        .expect("container box");
    let container_fragment = layout
        .fragments()
        .fragments_for_box(container)
        .next()
        .expect("container fragment");

    assert_eq!(
        static_position.source,
        StaticPositionSource::Fragment(source_fragment),
        "an inline-origin positioned child uses its line fragment, not a leaf fallback",
    );
    assert_eq!(
        (positioned_fragment.x, positioned_fragment.y),
        (container_fragment.x + 34.0, container_fragment.y + 8.0),
        "the shared K5d route resolves the inline-origin child's final insets",
    );
}

#[test]
fn inline_origin_absolute_auto_width_refits_to_the_k5d_inline_size() {
    let dom = StaticDocument::parse(
        "<div id=container>before <span id=source>source <span id=positioned>one two three four five six seven eight</span></span> after</div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #container { position: relative; width: 160px; } #source { display: inline; } \
             #positioned { position: absolute; left: 34px; right: 0; top: 8px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let container = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "container").expect("container"))
        .expect("container box");
    let positioned = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "positioned").expect("positioned"))
        .expect("positioned box");
    let source = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "source").expect("source"))
        .expect("source box");
    let container_fragment = layout
        .fragments()
        .fragments_for_box(container)
        .next()
        .expect("container fragment");
    let source_fragment = layout
        .fragments()
        .fragment_ids_for_box(source)
        .first()
        .copied()
        .expect("source line fragment");
    let positioned_fragment = layout
        .fragments()
        .fragments_for_box(positioned)
        .next()
        .expect("positioned fragment");

    assert_eq!(
        layout
            .fragments()
            .static_position_for_box(positioned)
            .expect("static position")
            .source,
        StaticPositionSource::Fragment(source_fragment),
        "the separate formatting root retains its enclosing inline line as the static source",
    );
    assert_eq!(
        (positioned_fragment.x, positioned_fragment.y),
        (container_fragment.x + 34.0, container_fragment.y + 8.0),
    );
    assert_eq!(positioned_fragment.width, 126.0);
    assert!(
        positioned_fragment.height > 20.0,
        "the text reflows at Buckram's 126px used inline size: {positioned_fragment:?}",
    );
}

#[test]
fn inline_origin_fixed_auto_width_refits_to_the_k5d_inline_size() {
    let dom = StaticDocument::parse(
        "<div id=container>before <span id=source>source <span id=positioned>one two three four five six seven eight</span></span> after</div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #container { width: 160px; } #source { display: inline; } \
             #positioned { position: fixed; left: 34px; right: 160px; top: 8px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let source = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "source").expect("source"))
        .expect("source box");
    let positioned = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "positioned").expect("positioned"))
        .expect("positioned box");
    let source_fragment = layout
        .fragments()
        .fragment_ids_for_box(source)
        .first()
        .copied()
        .expect("source line fragment");
    let positioned_fragment = layout
        .fragments()
        .fragments_for_box(positioned)
        .next()
        .expect("positioned fragment");

    assert_eq!(
        layout
            .fragments()
            .static_position_for_box(positioned)
            .expect("static position")
            .source,
        StaticPositionSource::Fragment(source_fragment),
    );
    assert_eq!((positioned_fragment.x, positioned_fragment.y), (34.0, 8.0));
    assert_eq!(positioned_fragment.width, 126.0);
    assert!(
        positioned_fragment.height > 20.0,
        "the fixed text reflows at Buckram's 126px used inline size: {positioned_fragment:?}",
    );
}

#[test]
fn absolute_flex_and_grid_children_keep_their_native_static_rectangles() {
    let dom = StaticDocument::parse(
        "<div id=flex><div id=flex-positioned>flex</div></div>\
         <div id=grid><div id=grid-positioned>grid</div></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #flex { position: relative; display: flex; width: 200px; height: 100px; \
                     justify-content: center; align-items: end; } \
             #grid { position: relative; display: grid; width: 200px; height: 100px; } \
             #flex-positioned, #grid-positioned { position: absolute; left: 18px; top: 9px; \
                                                   width: 30px; height: 20px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let box_for = |id| {
        layout
            .boxes()
            .principal_box(node_by_id(&dom, dom.document(), id).expect(id))
            .expect("principal box")
    };
    let rect_for = |box_id| {
        layout
            .fragments()
            .fragments_for_box(box_id)
            .next()
            .map(TreeFragment::physical_rect)
            .expect("fragment")
    };

    let flex = box_for("flex");
    let flex_positioned = box_for("flex-positioned");
    let grid = box_for("grid");
    let grid_positioned = box_for("grid-positioned");
    let flex_static = layout
        .fragments()
        .static_position_for_box(flex_positioned)
        .expect("flex static rectangle");
    let grid_static = layout
        .fragments()
        .static_position_for_box(grid_positioned)
        .expect("grid static rectangle");

    assert_eq!(
        (
            flex_static.logical_rect.inline_start,
            flex_static.logical_rect.block_start
        ),
        (85.0, 80.0),
        "the flex formatter owns alignment, while Buckram keeps its pre-inset result"
    );
    assert_eq!(
        (
            grid_static.logical_rect.inline_start,
            grid_static.logical_rect.block_start
        ),
        (0.0, 0.0),
        "the grid formatter contributes its grid-area static rectangle"
    );
    assert_eq!(
        grid_static.containing_block_area,
        Some(LogicalRect {
            inline_start: 0.0,
            block_start: 0.0,
            inline_size: 200.0,
            block_size: 100.0,
        }),
        "the direct grid child retains its finalized containing area separately from its static rectangle"
    );

    let flex_rect = rect_for(flex);
    let flex_positioned_rect = rect_for(flex_positioned);
    let grid_rect = rect_for(grid);
    let grid_positioned_rect = rect_for(grid_positioned);
    assert_eq!(
        (flex_positioned_rect.x, flex_positioned_rect.y),
        (flex_rect.x + 18.0, flex_rect.y + 9.0),
    );
    assert_eq!(
        (grid_positioned_rect.x, grid_positioned_rect.y),
        (grid_rect.x + 18.0, grid_rect.y + 9.0),
    );
}

#[test]
fn absolute_grid_self_end_uses_the_grid_content_end() {
    let dom = StaticDocument::parse("<div id=grid><div id=positioned></div></div>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #grid { display: grid; width: 100px; height: 100px; border: 1px solid; } \
             #positioned { position: absolute; width: 50px; height: 50px; align-self: self-end; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    assert_eq!(
        styles
            .get(node_by_id(&dom, dom.document(), "positioned").expect("positioned node"))
            .map(|style| (style.position, style.align_self)),
        Some((CssPosition::Absolute, CssAlignment::SelfEnd)),
        "the style value must survive parsing before layout maps it to the formatter",
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let grid = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "grid").expect("grid node"))
        .expect("grid box");
    let positioned = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "positioned").expect("positioned node"))
        .expect("positioned box");
    let grid_rect = layout
        .fragments()
        .fragments_for_box(grid)
        .next()
        .map(TreeFragment::physical_rect)
        .expect("grid fragment");
    let positioned_rect = layout
        .fragments()
        .fragments_for_box(positioned)
        .next()
        .map(TreeFragment::physical_rect)
        .expect("positioned fragment");
    assert_eq!(
        (positioned_rect.x, positioned_rect.y),
        (grid_rect.x + 1.0, grid_rect.y + 51.0),
        "a same-flow self-end positioned grid item uses the grid content end",
    );
    assert!(
        layout
            .fragments()
            .static_position_for_box(positioned)
            .is_some(),
        "the grid static-position route retains its K5b record",
    );
}

#[test]
fn absolute_grid_static_position_uses_the_placed_area_when_the_grid_is_its_containing_block() {
    let dom = StaticDocument::parse("<div id=grid><div id=positioned></div></div>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #grid { position: relative; display: grid; width: 100px; height: 100px; \
                     grid-template-columns: 20px 80px; grid-template-rows: 30px 70px; } \
             #positioned { position: absolute; grid-area: 2 / 2 / 3 / 3; \
                           width: 20px; height: 10px; align-self: self-end; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let grid = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "grid").expect("grid node"))
        .expect("grid box");
    let positioned = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "positioned").expect("positioned node"))
        .expect("positioned box");
    let grid_rect = layout
        .fragments()
        .fragments_for_box(grid)
        .next()
        .map(TreeFragment::physical_rect)
        .expect("grid fragment");
    let positioned_rect = layout
        .fragments()
        .fragments_for_box(positioned)
        .next()
        .map(TreeFragment::physical_rect)
        .expect("positioned fragment");
    let static_position = layout
        .fragments()
        .static_position_for_box(positioned)
        .expect("grid static position");

    assert_eq!(
        (
            static_position.logical_rect.inline_start,
            static_position.logical_rect.block_start,
        ),
        (20.0, 90.0),
        "CSS Grid 9.2: a grid that generates the containing block aligns the static \
         rectangle in the placed grid area, not its content box"
    );
    assert_eq!(
        static_position.containing_block_area,
        Some(LogicalRect {
            inline_start: 20.0,
            block_start: 30.0,
            inline_size: 80.0,
            block_size: 70.0,
        }),
        "the placed grid area remains the containing block for positioned insets"
    );
    assert_eq!(
        (positioned_rect.x, positioned_rect.y),
        (grid_rect.x + 20.0, grid_rect.y + 90.0),
    );
}

#[test]
fn absolute_grid_static_position_uses_content_edges_when_the_containing_block_is_elsewhere() {
    let dom =
        StaticDocument::parse("<div id=outer><div id=grid><div id=positioned></div></div></div>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #outer { position: relative; width: 100px; height: 100px; } \
             #grid { display: grid; width: 100px; height: 100px; \
                     grid-template-columns: 20px 80px; grid-template-rows: 30px 70px; } \
             #positioned { position: absolute; grid-area: 2 / 2 / 3 / 3; \
                           width: 20px; height: 10px; align-self: self-end; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let grid = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "grid").expect("grid node"))
        .expect("grid box");
    let positioned = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "positioned").expect("positioned node"))
        .expect("positioned box");
    let grid_rect = layout
        .fragments()
        .fragments_for_box(grid)
        .next()
        .map(TreeFragment::physical_rect)
        .expect("grid fragment");
    let positioned_rect = layout
        .fragments()
        .fragments_for_box(positioned)
        .next()
        .map(TreeFragment::physical_rect)
        .expect("positioned fragment");
    let static_position = layout
        .fragments()
        .static_position_for_box(positioned)
        .expect("grid static position");

    assert_eq!(
        (
            static_position.logical_rect.inline_start,
            static_position.logical_rect.block_start,
        ),
        (0.0, 90.0),
        "CSS Grid 9.2: a grid that is only the static-position parent aligns the static \
         rectangle in its content box; its placement lines do not apply"
    );
    assert_eq!(
        (positioned_rect.x, positioned_rect.y),
        (grid_rect.x, grid_rect.y + 90.0),
    );
}

#[test]
fn vertical_grid_static_alignment_uses_the_placed_area_block_end() {
    let dom = StaticDocument::parse("<div id=grid><div id=positioned></div></div>");
    for (writing_mode, expected_x) in [("vertical-rl", 0.0), ("vertical-lr", 80.0)] {
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&[&format!(
                "html, body, div {{ margin: 0; padding: 0; }} \
                     #grid {{ position: relative; display: grid; writing-mode: {writing_mode}; \
                             width: 100px; height: 80px; \
                             grid-template-columns: 20px 60px; grid-template-rows: 30px 70px; }} \
                     #positioned {{ position: absolute; grid-area: 2 / 2 / 3 / 3; \
                                   width: 20px; height: 10px; align-self: end; }}"
            )]),
            &Device::screen(320.0, 240.0),
            &InteractionStates::default(),
        );

        let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
        let grid = layout
            .boxes()
            .principal_box(node_by_id(&dom, dom.document(), "grid").expect("grid node"))
            .expect("grid box");
        let positioned = layout
            .boxes()
            .principal_box(node_by_id(&dom, dom.document(), "positioned").expect("positioned node"))
            .expect("positioned box");
        let grid_rect = layout
            .fragments()
            .fragments_for_box(grid)
            .next()
            .map(TreeFragment::physical_rect)
            .expect("grid fragment");
        let positioned_rect = layout
            .fragments()
            .fragments_for_box(positioned)
            .next()
            .map(TreeFragment::physical_rect)
            .expect("positioned fragment");
        assert_eq!(
            (positioned_rect.x, positioned_rect.y),
            (grid_rect.x + expected_x, grid_rect.y + 20.0),
            "{writing_mode}: block-end alignment uses the placed row's physical end edge, \
             and the inline start is the placed column's start"
        );
    }
}

#[test]
fn grid_static_self_alignment_uses_the_subject_writing_mode() {
    let dom = StaticDocument::parse("<div id=grid><div id=positioned></div></div>");
    let scenarios = [
        (
            "vertical grid, horizontal subject self-start",
            "writing-mode: vertical-rl;",
            "writing-mode: horizontal-tb; align-self: self-start;",
            (0.0, 0.0),
        ),
        (
            "vertical grid, horizontal subject self-end",
            "writing-mode: vertical-rl;",
            "writing-mode: horizontal-tb; align-self: self-end;",
            (80.0, 0.0),
        ),
        (
            "horizontal grid, vertical rtl subject self-start",
            "writing-mode: horizontal-tb;",
            "writing-mode: vertical-rl; direction: rtl; align-self: self-start;",
            (0.0, 70.0),
        ),
        (
            "horizontal grid, vertical rtl subject self-end",
            "writing-mode: horizontal-tb;",
            "writing-mode: vertical-rl; direction: rtl; align-self: self-end;",
            (0.0, 0.0),
        ),
        (
            "horizontal grid, vertical rl subject justify self-start",
            "writing-mode: horizontal-tb;",
            "writing-mode: vertical-rl; justify-self: self-start;",
            (80.0, 0.0),
        ),
        (
            "horizontal grid, vertical rl subject justify self-end",
            "writing-mode: horizontal-tb;",
            "writing-mode: vertical-rl; justify-self: self-end;",
            (0.0, 0.0),
        ),
        (
            "vertical grid, vertical rtl subject justify self-start",
            "writing-mode: vertical-rl;",
            "writing-mode: vertical-rl; direction: rtl; justify-self: self-start;",
            (0.0, 70.0),
        ),
        (
            "vertical grid, vertical rtl subject justify self-end",
            "writing-mode: vertical-rl;",
            "writing-mode: vertical-rl; direction: rtl; justify-self: self-end;",
            (0.0, 0.0),
        ),
    ];

    for (description, grid_writing_mode, subject_writing_mode, expected) in scenarios {
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&[&format!(
                "html, body, div {{ margin: 0; padding: 0; }} \
                 #grid {{ position: relative; display: grid; {grid_writing_mode} \
                         width: 100px; height: 80px; }} \
                 #positioned {{ position: absolute; {subject_writing_mode} \
                               width: 20px; height: 10px; }}"
            )]),
            &Device::screen(320.0, 240.0),
            &InteractionStates::default(),
        );
        let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
        let grid = layout
            .boxes()
            .principal_box(node_by_id(&dom, dom.document(), "grid").expect("grid node"))
            .expect("grid box");
        let positioned = layout
            .boxes()
            .principal_box(node_by_id(&dom, dom.document(), "positioned").expect("positioned node"))
            .expect("positioned box");
        let grid_rect = layout
            .fragments()
            .fragments_for_box(grid)
            .next()
            .map(TreeFragment::physical_rect)
            .expect("grid fragment");
        let positioned_rect = layout
            .fragments()
            .fragments_for_box(positioned)
            .next()
            .map(TreeFragment::physical_rect)
            .expect("positioned fragment");

        assert_eq!(
            (
                positioned_rect.x - grid_rect.x,
                positioned_rect.y - grid_rect.y
            ),
            expected,
            "{description} aligns to the subject's corresponding start or end side",
        );
    }
}

#[test]
fn positioned_grid_area_transforms_from_flow_relative_tracks_to_physical_insets() {
    let dom = StaticDocument::parse("<div id=grid><div id=positioned></div></div>");
    for (writing_mode, direction, expected) in [
        ("vertical-rl", "ltr", (10.0, 25.0)),
        ("vertical-lr", "ltr", (40.0, 25.0)),
        ("vertical-rl", "rtl", (10.0, 5.0)),
        ("vertical-lr", "rtl", (40.0, 5.0)),
    ] {
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&[&format!(
                "html, body, div {{ margin: 0; padding: 0; }} \
                 #grid {{ position: relative; display: grid; writing-mode: {writing_mode}; \
                         direction: {direction}; width: 100px; height: 80px; \
                         grid-template-columns: 20px 60px; grid-template-rows: 30px 70px; }} \
                 #positioned {{ position: absolute; grid-area: 2 / 2 / 3 / 3; \
                               left: 10px; right: 20px; top: 5px; bottom: 15px; \
                               width: 40px; height: 40px; }}"
            )]),
            &Device::screen(320.0, 240.0),
            &InteractionStates::default(),
        );
        let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
        let grid = layout
            .boxes()
            .principal_box(node_by_id(&dom, dom.document(), "grid").expect("grid node"))
            .expect("grid box");
        let positioned = layout
            .boxes()
            .principal_box(node_by_id(&dom, dom.document(), "positioned").expect("positioned node"))
            .expect("positioned box");
        let grid_rect = layout
            .fragments()
            .fragments_for_box(grid)
            .next()
            .map(TreeFragment::physical_rect)
            .expect("grid fragment");
        let positioned_rect = layout
            .fragments()
            .fragments_for_box(positioned)
            .next()
            .map(TreeFragment::physical_rect)
            .expect("positioned fragment");
        let static_position = layout
            .fragments()
            .static_position_for_box(positioned)
            .expect("grid static position");

        assert_eq!(
            static_position.containing_block_area,
            Some(LogicalRect {
                inline_start: 20.0,
                block_start: 30.0,
                inline_size: 60.0,
                block_size: 70.0,
            }),
            "{writing_mode} {direction}: the finalized area is stored in the grid's logical coordinates",
        );
        assert_eq!(
            (
                positioned_rect.x - grid_rect.x,
                positioned_rect.y - grid_rect.y,
                positioned_rect.width,
                positioned_rect.height,
            ),
            (expected.0, expected.1, 40.0, 40.0),
            "{writing_mode} {direction}: physical insets resolve inside the transformed grid area",
        );
    }
}

#[test]
fn positioned_child_uses_the_positioned_ancestor_padding_box() {
    let dom = StaticDocument::parse("<div id=containing><div id=positioned></div></div>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #containing { position: relative; width: 100px; height: 100px; \
                           border-style: solid; border-top-width: 5px; \
                           border-right-width: 10px; border-bottom-width: 15px; \
                           border-left-width: 20px; } \
             #positioned { position: absolute; width: 100%; height: 100%; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let box_for = |id| {
        layout
            .boxes()
            .principal_box(node_by_id(&dom, dom.document(), id).expect(id))
            .expect("principal box")
    };
    let rect_for = |box_id| {
        layout
            .fragments()
            .fragments_for_box(box_id)
            .next()
            .map(TreeFragment::physical_rect)
            .expect("fragment")
    };

    let containing = rect_for(box_for("containing"));
    let positioned = rect_for(box_for("positioned"));
    assert_eq!((containing.width, containing.height), (130.0, 120.0));
    assert_eq!(
        (
            positioned.x,
            positioned.y,
            positioned.width,
            positioned.height
        ),
        (20.0, 5.0, 100.0, 100.0),
        "percentage sizes and auto insets resolve against the padding box"
    );
}

#[test]
fn positioned_child_of_a_split_inline_uses_first_and_last_content_edges() {
    let dom = StaticDocument::parse(
        "<div id=host><span id=containing>one two three four five six <span id=start></span><span id=end></span></span></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #host { width: 70px; font-size: 10px; line-height: 10px; } \
             #containing { display: inline; position: relative; padding: 3px 7px 11px 13px; border: 2px solid; } \
             #start, #end { position: absolute; width: 1px; height: 1px; } \
             #start { top: 0; left: 0; } #end { right: 0; bottom: 0; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let box_for = |id| {
        layout
            .boxes()
            .principal_box(node_by_id(&dom, dom.document(), id).expect(id))
            .expect("principal box")
    };
    let containing_fragments = layout
        .fragments()
        .fragments_for_box(box_for("containing"))
        .map(TreeFragment::physical_rect)
        .collect::<Vec<_>>();
    let positioned = |id| {
        layout
            .fragments()
            .fragments_for_box(box_for(id))
            .next()
            .map(TreeFragment::physical_rect)
            .expect("positioned fragment")
    };
    assert!(
        containing_fragments.len() >= 2,
        "the positioned inline must fragment across multiple lines: {containing_fragments:?}"
    );
    let first = containing_fragments.first().expect("first fragment");
    let last = containing_fragments.last().expect("last fragment");
    assert_eq!(
        positioned("start"),
        PhysicalRect {
            x: first.x + 15.0,
            y: first.y + 5.0,
            width: 1.0,
            height: 1.0,
        }
    );
    assert_eq!(
        positioned("end"),
        PhysicalRect {
            x: last.x + last.width - 10.0,
            y: last.y + last.height - 14.0,
            width: 1.0,
            height: 1.0,
        }
    );
}

#[test]
fn positioned_child_of_inline_split_by_a_block_uses_all_continuations() {
    let dom = StaticDocument::parse(
        "<div id=container><div id=before></div>B<span id=containing><div id=split></div>AA<span id=positioned></span></span></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #container { font-size: 20px; line-height: 20px; width: 100px; height: 100px; } \
             #before { height: 60px; } #split { height: 0; } \
             #containing { display: inline; position: relative; } \
             #positioned { position: absolute; left: 0; top: -60px; width: 100px; height: 100px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let box_for = |id| {
        layout
            .boxes()
            .principal_box(node_by_id(&dom, dom.document(), id).expect(id))
            .expect("principal box")
    };
    let first_containing_fragment = layout
        .fragments()
        .fragments_for_box(box_for("containing"))
        .next()
        .map(TreeFragment::physical_rect)
        .expect("first containing fragment");
    let positioned = box_for("positioned");
    assert_eq!(
        layout
            .boxes()
            .boxes_for_node(node_by_id(&dom, dom.document(), "containing").expect("containing"))
            .len(),
        2,
        "the in-flow block produces a generated inline continuation"
    );
    assert_eq!(
        layout
            .fragments()
            .fragments_for_box(positioned)
            .next()
            .map(TreeFragment::physical_rect),
        Some(PhysicalRect {
            x: first_containing_fragment.x,
            y: first_containing_fragment.y - 60.0,
            width: 100.0,
            height: 100.0,
        }),
        "the -60px top inset resolves from the first continuation, not the child-owning continuation"
    );
}

#[test]
fn absolute_static_position_in_a_block_split_from_inline_includes_margins() {
    let dom = StaticDocument::parse(
        "<div id=before></div><div id=wrapper><span><div id=block><div id=positioned></div></div></span></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #before { height: 50px; } \
             #wrapper { display: flow-root; margin-top: -100px; } \
             #block { margin-top: 100px; } \
             #positioned { position: absolute; width: 100px; height: 100px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let rect = |id| {
        layout
            .get(node_by_id(&dom, dom.document(), id).expect("node"))
            .expect("fragment")
            .physical_rect()
    };
    let positioned = rect("positioned");

    assert_eq!(
        positioned.y,
        50.0,
        "before={:?}, wrapper={:?}, block={:?}, positioned={positioned:?}",
        rect("before"),
        rect("wrapper"),
        rect("block"),
    );
}

#[test]
fn absolute_siblings_in_one_inline_keep_an_empty_first_fragment() {
    let dom = StaticDocument::parse(
        "<div id=container><span id=prefix>BBBBBB</span> <span id=containing><div id=first></div>AA A AA AAAA<div id=second></div></span></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #container { font-size: 20px; line-height: 20px; width: 100px; height: 100px; } \
             #containing { display: inline; position: relative; } \
             #first, #second { position: absolute; top: 0; width: 50px; height: 100px; } \
             #first { left: -30px; } #second { left: -80px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let box_for = |id| {
        layout
            .boxes()
            .principal_box(node_by_id(&dom, dom.document(), id).expect(id))
            .expect("principal box")
    };
    let rect_for = |id| {
        layout
            .fragments()
            .fragments_for_box(box_for(id))
            .next()
            .map(TreeFragment::physical_rect)
            .expect("positioned fragment")
    };
    let prefix = layout
        .fragments()
        .fragments_for_box(box_for("prefix"))
        .next()
        .map(TreeFragment::physical_rect)
        .expect("prefix fragment");
    let containing_fragments = layout
        .fragments()
        .fragments_for_box(box_for("containing"))
        .map(TreeFragment::physical_rect)
        .collect::<Vec<_>>();
    assert_eq!(
        rect_for("first"),
        PhysicalRect {
            x: prefix.x + prefix.width - 30.0,
            y: prefix.y,
            width: 50.0,
            height: 100.0,
        },
        "prefix={prefix:?}, containing={containing_fragments:?}"
    );
    assert_eq!(
        rect_for("second"),
        PhysicalRect {
            x: prefix.x + prefix.width - 80.0,
            y: prefix.y,
            width: 50.0,
            height: 100.0,
        }
    );
}

#[test]
fn positioned_hit_test_respects_stacking_level_and_ancestor_clip() {
    let dom = StaticDocument::parse(
        "<div id=host><div id=behind></div><div id=normal></div><div id=front></div></div>\
         <div id=clip><div id=overlay></div></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #host { position: relative; width: 100px; height: 100px; } \
             #behind, #front { position: absolute; left: 0; top: 0; width: 80px; height: 80px; } \
             #behind { z-index: -1; } #normal { width: 80px; height: 80px; } \
             #front { z-index: 1; } \
             #clip { position: relative; width: 50px; height: 50px; overflow: hidden; } \
             #overlay { position: absolute; left: 0; top: 0; width: 100px; height: 100px; z-index: 1; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let node = |id| node_by_id(&dom, dom.document(), id).expect(id);

    assert_eq!(
        hit_test(&dom, &styles, &layout, 10.0, 10.0),
        Some(node("front"))
    );
    assert_eq!(
        hit_test(&dom, &styles, &layout, 10.0, 110.0),
        Some(node("overlay"))
    );
    assert_ne!(
        hit_test(&dom, &styles, &layout, 75.0, 110.0),
        Some(node("overlay"))
    );
}

#[test]
fn positioned_descendant_paints_above_its_stacking_context_background() {
    let dom =
        StaticDocument::parse("<div id=card><div id=collapse></div><div id=editor></div></div>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #card { position: relative; width: 80px; height: 80px; z-index: 4; } \
             #collapse, #editor { position: absolute; left: 0; top: 0; width: 80px; height: 80px; } \
             #collapse { z-index: 2; } #editor { z-index: 0; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");

    assert_eq!(
        hit_test(&dom, &styles, &layout, 10.0, 10.0),
        Some(node_by_id(&dom, dom.document(), "collapse").expect("collapse node")),
        "a child z-index is ordered within its parent's context, above the parent and lower siblings",
    );
}

#[test]
fn nested_absolute_card_keeps_its_editor_in_the_card_region() {
    let dom = StaticDocument::parse(
        "<div id=canvas><div id=card-root><div id=layer><div id=card><button id=collapse>Collapse</button><div id=editor>Card controls</div></div></div></div></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #canvas { position: relative; width: 520px; height: 260px; } \
             #card-root { position: absolute; left: 0; top: 0; z-index: 4; } \
             #layer { position: absolute; left: 100px; top: 60px; width: 150px; height: 160px; z-index: 4; } \
             #card { position: absolute; left: 0; top: 0; width: 100%; height: 100%; \
                     box-sizing: border-box; overflow: hidden; padding: 10px; z-index: 4; } \
             #collapse { position: absolute; right: 8px; top: 8px; z-index: 2; } \
             #editor { display: flex; flex-wrap: wrap; }"]),
        &Device::screen(640.0, 480.0),
        &InteractionStates::default(),
    );
    let layout = layout(&dom, &styles, 640.0, 480.0).expect("layout");
    let rect = |id| {
        layout
            .get(node_by_id(&dom, dom.document(), id).expect(id))
            .expect("fragment")
    };
    let layer = rect("layer");
    let card = rect("card");
    let collapse = rect("collapse");
    let editor = rect("editor");

    assert!((card.x - layer.x).abs() < 0.01 && (card.y - layer.y).abs() < 0.01);
    assert!((card.width - layer.width).abs() < 0.01 && (card.height - layer.height).abs() < 0.01);
    assert!(editor.x >= card.x && editor.y >= card.y);
    assert!(editor.x + editor.width <= card.x + card.width + 0.01);
    assert!(editor.y + editor.height <= card.y + card.height + 0.01);
    assert!(collapse.x >= card.x && collapse.y >= card.y);
    assert!(collapse.x + collapse.width <= card.x + card.width + 0.01);
    assert!(collapse.y + collapse.height <= card.y + card.height + 0.01);
}

#[test]
fn static_block_z_index_keeps_normal_hit_order() {
    let dom = StaticDocument::parse("<div id=host><div id=front></div><div id=normal></div></div>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #host { width: 80px; height: 80px; } \
             #front { width: 80px; height: 80px; margin-bottom: -80px; z-index: 1; } \
             #normal { width: 80px; height: 80px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");

    assert_eq!(
        hit_test(&dom, &styles, &layout, 10.0, 10.0),
        Some(node_by_id(&dom, dom.document(), "normal").expect("normal node")),
        "a static block's numeric z-index does not outrank later normal content",
    );
}

#[test]
fn grid_item_order_changes_the_topmost_hit_target() {
    let dom =
        StaticDocument::parse("<div id=grid><div id=later></div><div id=earlier></div></div>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #grid { display: grid; width: 80px; height: 80px; \
                     grid-template-columns: 80px; grid-template-rows: 80px; } \
             #later, #earlier { grid-area: 1 / 1 / 2 / 2; width: 80px; height: 80px; } \
             #later { order: 1; } #earlier { order: -1; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");

    assert_eq!(
        hit_test(&dom, &styles, &layout, 10.0, 10.0),
        Some(node_by_id(&dom, dom.document(), "later").expect("later node")),
        "the item painted last in order-modified order receives the hit",
    );
}

#[test]
fn fixed_position_uses_a_transform_fixed_containing_block() {
    let dom = StaticDocument::parse("<div id=trigger><div id=fixed>item</div></div>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "#trigger { transform: translateX(0px); margin-left: 40px; margin-top: 20px; \
             width: 120px; height: 60px; } \
             #fixed { position: fixed; left: 17px; top: 9px; width: 30px; height: 10px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let trigger = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "trigger").expect("trigger"))
        .expect("trigger box");
    let fixed = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "fixed").expect("fixed"))
        .expect("fixed box");
    let trigger_fragment = layout
        .fragments()
        .fragments_for_box(trigger)
        .next()
        .expect("trigger fragment");
    let fixed_fragment = layout
        .fragments()
        .fragments_for_box(fixed)
        .next()
        .expect("fixed fragment");

    assert_eq!(
        (fixed_fragment.x, fixed_fragment.y),
        (trigger_fragment.x + 17.0, trigger_fragment.y + 9.0),
    );
    assert_eq!(
        fixed_fragment.containing_fragment(),
        layout
            .fragments()
            .fragment_ids_for_box(trigger)
            .first()
            .copied(),
    );
}

#[test]
fn absolute_position_converts_between_vertical_static_and_containing_flows() {
    let dom = StaticDocument::parse("<div id=container><div id=positioned>item</div></div>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "#container { position: relative; writing-mode: vertical-rl; \
             width: 120px; height: 100px; } \
             #positioned { position: absolute; left: 13px; top: 8px; \
             width: 20px; height: 30px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let container = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "container").expect("container"))
        .expect("container box");
    let positioned = layout
        .boxes()
        .principal_box(node_by_id(&dom, dom.document(), "positioned").expect("positioned"))
        .expect("positioned box");
    let container_fragment = layout
        .fragments()
        .fragments_for_box(container)
        .next()
        .expect("container fragment");
    let positioned_fragment = layout
        .fragments()
        .fragments_for_box(positioned)
        .next()
        .expect("positioned fragment");

    assert_eq!(
        (positioned_fragment.x, positioned_fragment.y),
        (container_fragment.x + 13.0, container_fragment.y + 8.0),
        "physical insets retain their sides while K5d changes coordinate systems",
    );
    assert_eq!(
        positioned_fragment.containing_fragment(),
        layout
            .fragments()
            .fragment_ids_for_box(container)
            .first()
            .copied(),
    );
}

#[test]
fn relative_table_parts_move_their_retained_fragment_subtree() {
    let dom = StaticDocument::parse(
        "<table id=table><tbody id=group><tr id=row><td id=cell>one</td></tr></tbody>\
         <tbody><tr><td>two</td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "table { display: table; border-collapse: collapse; border-spacing: 0; } \
             tbody { display: table-row-group; } tr { display: table-row; } \
             #group { position: relative; left: 12px; top: 8px; } \
             #row { position: relative; left: 7px; top: 100px; } \
             td { display: table-cell; width: 40px; height: 20px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let by_id = |id| node_by_id(&dom, dom.document(), id).expect("node");
    let group = layout.get(by_id("group")).expect("row-group fragment");
    let row = layout.get(by_id("row")).expect("row fragment");
    let cell = layout.get(by_id("cell")).expect("cell fragment");

    assert_eq!((row.x - group.x, row.y - group.y), (7.0, 100.0));
    assert_eq!((cell.x, cell.y), (row.x, row.y));
    assert!(
        group.x > layout.principal_fragment(by_id("table")).expect("grid").x,
        "the row-group's offset must survive flattening"
    );
}

#[test]
fn html_align_descendants_adjusts_used_margins_without_rewriting_computed_css() {
    let dom = StaticDocument::parse(
        r#"
            <div style="width: 300px">
              <div align="right"><div id="right" style="width: 100px; margin: 10px">right</div></div>
              <center><div id="center" style="width: 100px; margin: 10px">center</div></center>
              <div align="left" style="direction: rtl"><div id="rtl-left" style="width: 100px; margin: 10px">rtl</div></div>
              <div align="right"><div id="auto" style="margin: 10px">auto</div></div>
            </div>
        "#,
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body { margin: 0; }"]),
        &Device::screen(300.0, 240.0),
        &InteractionStates::default(),
    );
    let layout = layout(&dom, &styles, 300.0, 240.0).expect("layout");
    let by_id = |id| node_by_id(&dom, dom.document(), id).expect("node");

    assert_eq!(layout.get(by_id("right")).expect("right").x, 190.0);
    assert_eq!(layout.get(by_id("center")).expect("center").x, 100.0);
    assert_eq!(
        layout.get(by_id("rtl-left")).expect("rtl left").x,
        10.0,
        "line-left remains physical left in horizontal RTL"
    );
    assert_eq!(
        layout.get(by_id("auto")).expect("auto width").x,
        10.0,
        "width:auto is outside the legacy over-constrained rule"
    );
    assert_eq!(
        styles.get(by_id("right")).unwrap().margin_left,
        Margin::Value(CssLengthPercentage::Length(Length::px(10.0))),
        "the adjustment must remain a used value"
    );
}

#[test]
fn absolute_table_root_uses_shared_k5d_wrapper_geometry() {
    let dom = StaticDocument::parse("<table id=table><tbody><tr><td>one</td></tr></tbody></table>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "table { display: table; position: absolute; left: 31px; top: 14px; border-spacing: 0; } \
             tbody { display: table-row-group; } tr { display: table-row; } \
             td { display: table-cell; width: 40px; height: 20px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let table = node_by_id(&dom, dom.document(), "table").expect("table");
    let table_box = layout.boxes().principal_box(table).expect("table grid");
    let positioned_wrapper = layout
        .boxes()
        .boxes_for_node(table)
        .iter()
        .copied()
        .find(|box_id| layout.boxes()[*box_id].positioning == PositioningScheme::Absolute)
        .expect("the table wrapper carries the table root's positioning");
    let wrapper_fragment = layout
        .fragments()
        .fragments_for_box(positioned_wrapper)
        .next()
        .expect("positioned wrapper fragment");
    assert_eq!((wrapper_fragment.x, wrapper_fragment.y), (31.0, 14.0));
    let ledger = layout.table_shadow_ledger();
    assert!(
        !ledger
            .positioning_gaps
            .contains(&crate::table_shadow::TablePositioningGapRecord {
                table: table_box,
                part: table_box,
                gap: crate::table_shadow::TablePositioningGap::Absolute,
            }),
        "the shared wrapper route replaces the root-only table positioning gap: {ledger:?}"
    );
    assert_eq!(
        ledger.block.laid_out, 1,
        "the table stays on Buckram: {ledger:?}"
    );
}

#[test]
fn absolute_table_caption_uses_shared_k5d_wrapper_geometry() {
    let dom = StaticDocument::parse(
        "<table id=table><caption id=caption>caption</caption><tbody><tr><td>cell</td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            r#"table { display: table; position: relative; border-spacing: 0; }
               caption { display: table-caption; position: absolute; left: 31px; top: 14px; width: 240px; height: 20px; }
               tbody { display: table-row-group; } tr { display: table-row; }
               td { display: table-cell; width: 80px; height: 20px; }"#,
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let table = node_by_id(&dom, dom.document(), "table").expect("table");
    let caption = node_by_id(&dom, dom.document(), "caption").expect("caption");
    let table_grid = layout.boxes().principal_box(table).expect("table grid");
    let wrapper = layout.boxes()[table_grid].parent().expect("table wrapper");
    let caption_box = layout.boxes().principal_box(caption).expect("caption box");
    assert_eq!(
        layout.boxes()[caption_box].positioning,
        PositioningScheme::Absolute,
    );
    let wrapper_fragment = layout
        .fragments()
        .fragments_for_box(wrapper)
        .next()
        .expect("wrapper fragment");
    let grid_fragment = layout
        .fragments()
        .fragments_for_box(table_grid)
        .next()
        .expect("grid fragment");
    let caption_fragment = layout
        .fragments()
        .fragments_for_box(caption_box)
        .next()
        .expect("caption fragment");
    let caption_static = layout
        .fragments()
        .static_position_for_box(caption_box)
        .expect("caption static-position record");

    assert_eq!(
        (caption_fragment.x, caption_fragment.y),
        (wrapper_fragment.x + 31.0, wrapper_fragment.y + 14.0),
        "the caption uses the wrapper containing block rather than table tracks",
    );
    assert_eq!(
        wrapper_fragment.width, grid_fragment.width,
        "the out-of-flow caption must not widen the table wrapper",
    );
    // The cell's 80px content width plus its initial 1px inline borders;
    // the 240px out-of-flow caption does not participate in this width.
    assert_eq!(grid_fragment.width, 82.0);
    assert_eq!(caption_fragment.width, 240.0);
    assert_eq!(
        caption_static.containing_block,
        ContainingBlock::Box(wrapper)
    );
    assert_eq!(
        caption_fragment.containing_fragment(),
        layout
            .fragments()
            .fragment_ids_for_box(wrapper)
            .first()
            .copied(),
    );
    assert_eq!(layout.table_shadow_ledger().block.laid_out, 1);
}

#[test]
fn fixed_table_caption_uses_shared_k5d_initial_geometry() {
    let dom = StaticDocument::parse(
        "<table id=table><caption id=caption>caption</caption><tbody><tr><td>cell</td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            r#"table { display: table; position: relative; border-spacing: 0; }
               caption { display: table-caption; position: fixed; left: 31px; top: 14px; width: 240px; height: 20px; }
               tbody { display: table-row-group; } tr { display: table-row; }
               td { display: table-cell; width: 80px; height: 20px; }"#,
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let table = node_by_id(&dom, dom.document(), "table").expect("table");
    let caption = node_by_id(&dom, dom.document(), "caption").expect("caption");
    let table_grid = layout.boxes().principal_box(table).expect("table grid");
    let wrapper = layout.boxes()[table_grid].parent().expect("table wrapper");
    let caption_box = layout.boxes().principal_box(caption).expect("caption box");
    let wrapper_fragment = layout
        .fragments()
        .fragments_for_box(wrapper)
        .next()
        .expect("wrapper fragment");
    let grid_fragment = layout
        .fragments()
        .fragments_for_box(table_grid)
        .next()
        .expect("grid fragment");
    let caption_fragment = layout
        .fragments()
        .fragments_for_box(caption_box)
        .next()
        .expect("caption fragment");
    let caption_static = layout
        .fragments()
        .static_position_for_box(caption_box)
        .expect("caption static-position record");

    assert_eq!(
        layout.boxes()[caption_box].positioning,
        PositioningScheme::Fixed,
    );
    assert_eq!((caption_fragment.x, caption_fragment.y), (31.0, 14.0));
    assert_eq!(
        wrapper_fragment.width, grid_fragment.width,
        "the out-of-flow caption must not widen the table wrapper",
    );
    // The cell's 80px content width plus its initial 1px inline borders;
    // the 240px out-of-flow caption does not participate in this width.
    assert_eq!(grid_fragment.width, 82.0);
    assert_eq!(caption_static.containing_block, ContainingBlock::Initial);
    assert_eq!(caption_fragment.containing_fragment(), None);
    assert_eq!(layout.table_shadow_ledger().block.laid_out, 1);
}

#[test]
fn absolute_table_track_parts_use_zero_track_static_anchors() {
    let dom = StaticDocument::parse(
        "<table id=cell-table><tbody><tr><td>flow</td><td id=cell>cell</td></tr></tbody></table>\
         <table id=row-table><tbody><tr><td>flow</td></tr><tr id=row><td>row</td></tr></tbody></table>\
         <table id=group-table><tbody><tr><td>flow</td></tr></tbody><tbody id=group><tr><td>group</td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            r#"table { display: table; position: relative; border-spacing: 0; }
               tbody { display: table-row-group; } tr { display: table-row; }
               td { display: table-cell; width: 80px; height: 20px; }
               #cell, #row, #group { position: absolute; left: 31px; top: 14px; width: 240px; height: 20px; }"#,
        ]),
        &Device::screen(640.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 640.0, 240.0).expect("layout");
    for (table_id, part_id) in [
        ("cell-table", "cell"),
        ("row-table", "row"),
        ("group-table", "group"),
    ] {
        let table = node_by_id(&dom, dom.document(), table_id).expect("table");
        let part = node_by_id(&dom, dom.document(), part_id).expect("detached table part");
        let grid = layout.boxes().principal_box(table).expect("table grid");
        let wrapper = layout.boxes()[grid].parent().expect("table wrapper");
        let part_box = layout.boxes().principal_box(part).expect("part box");
        let wrapper_fragment = layout
            .fragments()
            .fragments_for_box(wrapper)
            .next()
            .expect("wrapper fragment");
        let grid_fragment = layout
            .fragments()
            .fragments_for_box(grid)
            .next()
            .expect("grid fragment");
        let part_fragment = layout
            .fragments()
            .fragments_for_box(part_box)
            .next()
            .expect("detached part fragment");
        let static_position = layout
            .fragments()
            .static_position_for_box(part_box)
            .expect("zero-track static position");

        assert_eq!(
            (part_fragment.x, part_fragment.y),
            (wrapper_fragment.x + 31.0, wrapper_fragment.y + 14.0),
            "{part_id} resolves through the wrapper containing block",
        );
        assert_eq!(
            grid_fragment.width, 82.0,
            "{part_id} does not widen the grid"
        );
        assert_eq!(static_position.logical_rect, LogicalRect::default());
    }
    assert!(
        layout.table_shadow_ledger().positioning_gaps.is_empty(),
        "post-track parts must not retain a K5 positioning gap: {:?}",
        layout.table_shadow_ledger(),
    );
}

#[test]
fn fixed_table_track_parts_use_zero_track_static_anchors() {
    let dom = StaticDocument::parse(
        "<table id=cell-table><tbody><tr><td>flow</td><td id=cell>cell</td></tr></tbody></table>\
         <table id=row-table><tbody><tr><td>flow</td></tr><tr id=row><td>row</td></tr></tbody></table>\
         <table id=group-table><tbody><tr><td>flow</td></tr></tbody><tbody id=group><tr><td>group</td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            r#"table { display: table; position: relative; border-spacing: 0; }
               tbody { display: table-row-group; } tr { display: table-row; }
               td { display: table-cell; width: 80px; height: 20px; }
               #cell, #row, #group { position: fixed; left: 31px; top: 14px; width: 240px; height: 20px; }"#,
        ]),
        &Device::screen(640.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 640.0, 240.0).expect("layout");
    for (table_id, part_id) in [
        ("cell-table", "cell"),
        ("row-table", "row"),
        ("group-table", "group"),
    ] {
        let table = node_by_id(&dom, dom.document(), table_id).expect("table");
        let part = node_by_id(&dom, dom.document(), part_id).expect("detached table part");
        let grid = layout.boxes().principal_box(table).expect("table grid");
        let part_box = layout.boxes().principal_box(part).expect("part box");
        let grid_fragment = layout
            .fragments()
            .fragments_for_box(grid)
            .next()
            .expect("grid fragment");
        let part_fragment = layout
            .fragments()
            .fragments_for_box(part_box)
            .next()
            .expect("detached part fragment");
        let static_position = layout
            .fragments()
            .static_position_for_box(part_box)
            .expect("zero-track static position");

        assert_eq!(
            (part_fragment.x, part_fragment.y),
            (31.0, 14.0),
            "{part_id} resolves against the initial containing block",
        );
        assert_eq!(
            grid_fragment.width, 82.0,
            "{part_id} does not widen the grid"
        );
        assert_eq!(static_position.logical_rect, LogicalRect::default());
        assert_eq!(static_position.containing_block, ContainingBlock::Initial);
        assert_eq!(part_fragment.containing_fragment(), None);
    }
    assert!(
        layout.table_shadow_ledger().positioning_gaps.is_empty(),
        "post-track parts must not retain a K5 positioning gap: {:?}",
        layout.table_shadow_ledger(),
    );
}

#[test]
fn k4g4_consumes_projected_metrics_on_both_table_axes() {
    let dom = StaticDocument::parse(
        "<table id=table><tbody><tr><td>one</td><td>two</td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["table { display: table; border-collapse: collapse; } \
             tbody { display: table-row-group; } tr { display: table-row; } \
             td { display: table-cell; border: 5px solid; padding: 0; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let ledger = layout.table_shadow_ledger();
    assert_eq!(ledger.collapsed_metrics, 1, "{ledger:?}");
    assert!(
        ledger.skipped.is_empty(),
        "K4g4 must consume B2's projected metrics without a fallback: {ledger:?}"
    );
    assert_eq!(ledger.assigned, 1, "{ledger:?}");
    assert_eq!(ledger.honored, 1, "{ledger:?}");
    assert_eq!(ledger.block.laid_out, 1, "{ledger:?}");
    assert_eq!(ledger.block.agreed, 1, "{ledger:?}");
    let table = node_by_id(&dom, dom.document(), "table").expect("table node");
    let fragment = layout.principal_fragment(table).expect("table fragment");
    assert!(
        (fragment.logical_rect.inline_start - fragment.overflow.inline_start - 2.5).abs() < 0.01,
        "the first outer winner spills beyond the table border box: {fragment:?}"
    );
    assert!(
        (fragment.logical_rect.block_start - fragment.overflow.block_start - 2.5).abs() < 0.01,
        "the block-start winner also propagates into table overflow: {fragment:?}"
    );
}

#[test]
fn ph3_rules_attribute_reaches_k4g_collapsed_border_resolution() {
    let dom = StaticDocument::parse(
        r#"<table id="table" rules="all" bordercolor="red"><tbody><tr><td id="cell">one</td><td>two</td></tr></tbody></table>"#,
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let table = node_by_id(&dom, dom.document(), "table").expect("table");
    let cell = node_by_id(&dom, dom.document(), "cell").expect("cell");

    assert_eq!(
        styles.get(table).unwrap().border_collapse,
        BorderCollapse::Collapse,
        "the HTML attribute must first become an ordinary computed declaration"
    );
    assert_eq!(
        styles.get(cell).unwrap().border_top_style,
        BorderStyle::Solid
    );
    assert_eq!(
        styles.get(cell).unwrap().border_top_color.to_srgb8(),
        Some((0, 0, 0, 255)),
        "the attribute-sensitive UA rule supplies the cell candidate color"
    );

    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let ledger = layout.table_shadow_ledger();
    assert_eq!(ledger.collapsed_metrics, 1, "{ledger:?}");
    assert_eq!(ledger.assigned, 1, "{ledger:?}");
    assert_eq!(ledger.honored, 1, "{ledger:?}");
    assert!(ledger.skipped.is_empty(), "{ledger:?}");
}

fn fixed_table_ledger(spacing: &str) -> crate::table_shadow::TableShadowLedger {
    let dom = StaticDocument::parse(
        "<table><tbody><tr><td id=first>one</td><td>two</td><td>three</td></tr></tbody></table>",
    );
    let css = format!(
        "table {{ display: table; table-layout: fixed; width: 300px; border-spacing: {spacing}; }} tbody {{ display: table-row-group; }} tr {{ display: table-row; }} td {{ display: table-cell; }} #first {{ width: 120px; }}"
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[&css]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    layout(&dom, &styles, 320.0, 240.0)
        .expect("layout")
        .table_shadow_ledger()
        .clone()
}

fn assert_assigned_and_honored(ledger: &crate::table_shadow::TableShadowLedger) {
    assert_eq!(
        ledger.assigned, 1,
        "Buckram did not size the table: {ledger:?}"
    );
    assert_eq!(
        ledger.verified, 1,
        "the assignment was not verified: {ledger:?}"
    );
    assert!(
        ledger.is_silent(),
        "the bridge did not honor Buckram's tracks: {ledger:?}"
    );
    assert_eq!(ledger.honored, 1, "{ledger:?}");
}

/// K4c5b: Buckram owns the fixed algorithm and the painted fragments
/// honor its columns exactly.
#[test]
fn k4c5b_fixed_table_columns_are_buckram_owned() {
    assert_assigned_and_honored(&fixed_table_ledger("0"));
}

/// The first K4c5a divergence, resolved by authority. The deleted live
/// helper omitted `border-spacing` from CSS 2.1 17.5.2.1's distribution
/// and painted 89px columns; Buckram's 85px columns now paint, and the
/// fragment verification proves it.
#[test]
fn k4c5b_fixed_border_spacing_distribution_is_painted() {
    assert_assigned_and_honored(&fixed_table_ledger("2px"));
}

/// K4c5b on the production text path: a fixed table routed through
/// `InlineBuildState` receives Buckram columns before the main pass. This
/// route previously had no fixed sizing at all.
#[test]
fn k4c5b_text_path_fixed_tables_are_buckram_owned() {
    let dom = StaticDocument::parse(
        "<p>before the table</p><table><tbody><tr><td id=first>one</td><td>two</td><td>three</td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "table { display: table; table-layout: fixed; width: 300px; border-spacing: 0; } tbody { display: table-row-group; } tr { display: table-row; } td { display: table-cell; } #first { width: 120px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    assert_assigned_and_honored(layout.table_shadow_ledger());
}

/// A fixed table inside an inline-block builds under an atomic subtree's
/// own `BuildState`; its assignment and verification survive into
/// `LiveryLayout` through the accumulated plane ledger.
///
/// Span-based markup because a `<div>` start tag inside `<p>` closes the
/// paragraph at the HTML parser, before box generation runs.
#[test]
fn k4c5b_tables_inside_atomic_inline_subtrees_are_buckram_owned() {
    let dom = StaticDocument::parse(
        "<p>before <span id=atom><span class=t><span class=tb><span class=row><span class=cell id=first>one</span><span class=cell>two</span><span class=cell>three</span></span></span></span></span> after</p>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "#atom { display: inline-block; } .t { display: table; table-layout: fixed; width: 300px; border-spacing: 0; } .tb { display: table-row-group; } .row { display: table-row; } .cell { display: table-cell; } #first { width: 120px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let ledger = layout.table_shadow_ledger();
    assert!(
        ledger.assigned >= 1,
        "the atomic subtree's table was not sized by Buckram: {ledger:?}"
    );
    assert!(ledger.is_silent(), "{ledger:?}");
}

/// K4d6b: a `height` on a `<tr>` reaches the painted rows.
///
/// It is a row minimum under CSS 2.1 section 17.5.3. Buckram computes 40
/// and 60 and writes the retained row fragments at those sizes.
///
/// The painted rectangles are asserted directly rather than through the
/// ledger. A ledger that agreed with itself would prove nothing.
#[test]
fn k4d6b_row_heights_reach_the_painted_rows() {
    let dom = StaticDocument::parse(
        "<table><tbody><tr id=a><td>one</td><td>two</td></tr><tr id=b><td>three</td><td>four</td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "table { display: table; table-layout: fixed; width: 200px; border-spacing: 0; }                  tbody { display: table-row-group; } tr { display: table-row; }                  td { display: table-cell; padding: 0; } #a { height: 40px; } #b { height: 60px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let ledger = layout.table_shadow_ledger();
    assert_eq!(
        ledger.block.laid_out, 1,
        "Buckram did not lay out the table's block axis: {:?}",
        ledger.block
    );
    assert_eq!(
        ledger.block.agreed, 1,
        "the painted cells must now be the ones Buckram wrote: {:?}",
        ledger.block.divergences
    );

    fn cell_rect(
        dom: &StaticDocument,
        layout: &LiveryLayout<<StaticDocument as LayoutDom>::NodeId>,
        index: usize,
    ) -> PhysicalRect {
        fn cells(
            dom: &StaticDocument,
            node: <StaticDocument as LayoutDom>::NodeId,
            found: &mut Vec<<StaticDocument as LayoutDom>::NodeId>,
        ) {
            if dom
                .element_name(node)
                .is_some_and(|name| name.local.as_ref() == "td")
            {
                found.push(node);
            }
            for child in dom.dom_children(node) {
                cells(dom, child, found);
            }
        }
        let mut found = Vec::new();
        cells(dom, dom.document(), &mut found);
        let box_id = layout.boxes().boxes_for_node(found[index])[0];
        layout
            .fragments()
            .fragments_for_box(box_id)
            .next()
            .expect("cell fragment")
            .physical_rect()
    }
    // The first cell of each row, in document order.
    let first = cell_rect(&dom, &layout, 0);
    let second = cell_rect(&dom, &layout, 2);
    assert!(
        (first.height - 40.0).abs() < 0.5,
        "the first row's cell must be its row's 40px, not its content              height: {first:?}"
    );
    assert!(
        (second.height - 60.0).abs() < 0.5,
        "the second row's cell must be its row's 60px: {second:?}"
    );
    assert!(
        (second.y - first.y - 40.0).abs() < 0.5,
        "the second row must start just below a 40px first row:              {first:?} {second:?}"
    );
}

/// B3: the accepted K4d3 row-group rule must reach live geometry rather
/// than remain a pure constraint. A definite `tbody` height is a minimum
/// shared proportionally by only that group's 20px and 40px rows.
#[test]
fn b3_row_group_height_reaches_the_buckram_table_route() {
    let dom = StaticDocument::parse(
        "<table><tbody><tr><td><i class=small></i></td></tr><tr><td><i class=large></i></td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "table { display: table; table-layout: fixed; width: 200px; border-spacing: 0; } \
             tbody { display: table-row-group; height: 200px; } \
             tr { display: table-row; } td { display: table-cell; padding: 0; } \
             i { display: block; } .small { height: 20px; } .large { height: 40px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let ledger = layout.table_shadow_ledger();
    assert_eq!(ledger.block.laid_out, 1, "block ledger: {:?}", ledger.block);
    assert_eq!(ledger.block.agreed, 1, "block ledger: {:?}", ledger.block);

    let rows = layout
        .boxes()
        .iter()
        .filter(|(_, css_box)| css_box.display.internal_table == Some(InternalTableRole::Row))
        .filter_map(|(box_id, _)| {
            layout
                .fragments()
                .fragments_for_box(box_id)
                .next()
                .map(|fragment| fragment.physical_rect())
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2, "rows: {rows:?}");
    assert!((rows[0].height - 66.67).abs() < 0.5, "rows: {rows:?}");
    assert!((rows[1].height - 133.33).abs() < 0.5, "rows: {rows:?}");
    assert!(
        (rows[0].height + rows[1].height - 200.0).abs() < 0.5,
        "rows: {rows:?}"
    );
}

/// B3: a table's own definite height distributes across its rows once;
/// an auto row group must not receive that height as a second constraint.
/// This is the table geometry used by CSS2 containing-block-029's
/// reference, kept here as a direct layout receipt.
#[test]
fn b3_auto_row_group_does_not_repeat_the_table_height() {
    let html = "<table><col id=first><col id=second><tbody><tr><td></td><td></td></tr>\
                <tr id=last><td></td><td id=orange>.</td></tr></tbody></table>";
    let css = "table { border-spacing: 0; height: 96px; table-layout: fixed; width: 96px; }\
               col#first { width: 72px; } col#second { width: 24px; }\
               td { background-color: blue; padding: 0; }\
               td#orange { background-color: orange; vertical-align: top; }\
               tr { height: 72px; } tr#last { height: 24px; }";
    let grid = table_role_rects(html, css, InternalTableRole::Grid)[0];
    let rows = table_role_rects(html, css, InternalTableRole::Row);
    let cells = table_role_rects(html, css, InternalTableRole::Cell);
    assert_eq!(rows.len(), 2, "rows: {rows:?}");
    assert_eq!(cells.len(), 4, "cells: {cells:?}");
    assert!((grid.height - 96.0).abs() < 0.5, "grid: {grid:?}");
    assert!((rows[0].height - 72.0).abs() < 0.5, "rows: {rows:?}");
    assert!((rows[1].height - 24.0).abs() < 0.5, "rows: {rows:?}");
    assert!((cells[3].height - 24.0).abs() < 0.5, "cells: {cells:?}");
    assert!(
        (rows[0].height + rows[1].height - grid.height).abs() < 0.5,
        "grid: {grid:?}; rows: {rows:?}"
    );
}

/// Lay out one document and return every table-role box's rectangle.
fn table_boxes(html: &str, css: &str) -> Vec<(InternalTableRole, PhysicalRect)> {
    let dom = StaticDocument::parse(html);
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; border: 0; }", css]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    layout
        .boxes()
        .iter()
        .filter_map(|(box_id, css_box)| {
            Some((
                css_box.display.internal_table?,
                layout
                    .fragments()
                    .fragments_for_box(box_id)
                    .next()?
                    .physical_rect(),
            ))
        })
        .collect()
}

/// Every rectangle laid out for one table role, in tree order.
fn table_role_rects(html: &str, css: &str, role: InternalTableRole) -> Vec<PhysicalRect> {
    table_boxes(html, css)
        .into_iter()
        .filter(|(each, _)| *each == role)
        .map(|(_, rect)| rect)
        .collect()
}

/// The table wrapper box and the table grid box, in that order.
fn table_wrapper_and_grid(html: &str, css: &str) -> (PhysicalRect, PhysicalRect) {
    let one = |role| {
        *table_role_rects(html, css, role)
            .first()
            .unwrap_or_else(|| panic!("no fragment for {role:?}"))
    };
    (
        one(InternalTableRole::Wrapper),
        one(InternalTableRole::Grid),
    )
}

#[test]
fn ph2_table_width_hint_reaches_buckram_through_computed_css() {
    let (wrapper, grid) = table_wrapper_and_grid(
        "<div id=host><table width=50%><tr><td></td></tr></table></div>",
        "#host { width: 200px; }\
         table { display: table; table-layout: fixed; border-spacing: 0; }\
         tr { display: table-row; } td { display: table-cell; padding: 0; }",
    );

    assert!((wrapper.width - 100.0).abs() < 0.5, "wrapper: {wrapper:?}");
    assert!((grid.width - 100.0).abs() < 0.5, "grid: {grid:?}");
}

/// K4e1: the wrapper and the grid are two boxes that split one element.
///
/// CSS 2.1 section 17.4 uses `margin-*` on the wrapper and leaves `width`,
/// `border`, and `padding` on the grid. Both are observable at once: the
/// margin has to move the wrapper, and the grid's own border box has to
/// still contain the border and padding the wrapper does not have. CSS
/// Tables 3 section 2.2.1 then makes the two the same width, because the
/// wrapper's width *is* the grid's border-edge width.
#[test]
fn k4e1_the_wrapper_takes_the_margin_and_the_grid_keeps_its_own_box() {
    let (wrapper, grid) = table_wrapper_and_grid(
        "<div id=host><table><tr><td></td></tr></table></div>",
        "#host { width: 300px; }\
         table { display: table; box-sizing: content-box; width: 100px;\
                 margin-left: 20px; border: 5px solid; padding: 3px;\
                 border-spacing: 0; }\
         tr { display: table-row; } td { display: table-cell; padding: 0; }",
    );

    // The margin is the wrapper's, so both boxes start past it.
    assert!((wrapper.x - 20.0).abs() < 0.5, "wrapper: {wrapper:?}");
    assert!((grid.x - 20.0).abs() < 0.5, "grid: {grid:?}");
    // The grid's border edge is 100 of content plus its own padding and
    // border, and the wrapper is exactly that wide - no wider, which is
    // what would happen if it had kept the border and padding too.
    assert!((grid.width - 116.0).abs() < 0.5, "grid: {grid:?}");
    assert!(
        (wrapper.width - grid.width).abs() < 0.5,
        "the wrapper is the grid's border-edge width: {wrapper:?} {grid:?}"
    );
}

/// K4e1: `position` is the wrapper's, and a percentage size skips it.
///
/// CSS 2.1 section 17.4 again: "Percentages on 'width' and 'height' on the
/// table are relative to the table wrapper box's containing block, not the
/// table wrapper box itself." Without that rule the grid's `50%` resolves
/// against a wrapper that is itself waiting on the grid, and the pair
/// collapses to zero - which is what `absolute-tables-012` measures.
#[test]
fn k4e1_a_percentage_table_skips_the_wrapper_it_would_otherwise_wait_on() {
    let (wrapper, grid) = table_wrapper_and_grid(
        "<div id=host><table></table></div>",
        "#host { position: relative; width: 200px; }\
         table { display: table; position: absolute; width: 50%; height: 100px;\
                 table-layout: fixed; border-spacing: 0; }",
    );

    for (name, rect) in [("wrapper", wrapper), ("grid", grid)] {
        assert!((rect.width - 100.0).abs() < 0.5, "{name}: {rect:?}");
        assert!((rect.height - 100.0).abs() < 0.5, "{name}: {rect:?}");
    }
}

/// K4e1: a table flex item is the wrapper, and it does not stretch.
///
/// Inserting the wrapper makes it, not the grid, the flex item. CSS
/// Tables 3 section 2.2.1 keeps its width the grid's, so a column flex
/// container's default `align-items: stretch` has nothing to stretch -
/// the width is not `auto`. `table-as-item-cell-percentage-002` fails the
/// moment the wrapper widens to the container instead.
#[test]
fn k4e1_a_table_flex_item_is_the_wrapper_and_keeps_the_grids_width() {
    let (wrapper, grid) = table_wrapper_and_grid(
        "<div id=host><table><tr><td></td></tr></table></div>",
        "#host { display: flex; flex-direction: column; width: 300px; }\
         table { display: table; width: 100px; height: 100px; border-spacing: 0; }\
         tr { display: table-row; } td { display: table-cell; padding: 0; }",
    );

    assert!((wrapper.width - 100.0).abs() < 0.5, "wrapper: {wrapper:?}");
    assert!((grid.width - 100.0).abs() < 0.5, "grid: {grid:?}");
}

/// K4e2: an `auto`-width wrapper measures the grid rather than filling.
///
/// This is the half of CSS Tables 3 section 2.2.1 that cannot be computed
/// before layout. An ordinary block with `width: auto` would take all 300
/// of the container; the wrapper takes the 80 its two columns come to,
/// through Buckram's intrinsic shrink-to-fit lane rather than through the
/// `float: left` that used to stand in for it.
#[test]
fn k4e2_an_auto_width_wrapper_measures_the_grid_instead_of_filling() {
    let (wrapper, grid) = table_wrapper_and_grid(
        "<div id=host><table><tr><td></td><td></td></tr></table></div>",
        "#host { width: 300px; }\
         table { display: table; border-spacing: 0; }\
         tr { display: table-row; }\
         td { display: table-cell; padding: 0; width: 40px; height: 10px; }",
    );

    assert!((grid.width - 80.0).abs() < 0.5, "grid: {grid:?}");
    assert!((wrapper.width - 80.0).abs() < 0.5, "wrapper: {wrapper:?}");
}

/// K4e2: auto margins centre a table, which a float cannot do.
///
/// The margins are the wrapper's under CSS 2.1 section 17.4, and a float
/// resolves an `auto` margin to zero. Once the wrapper is an in-flow
/// shrink-to-fit block on K3's equations, `margin: 0 auto` on a table
/// centres it the way it does on any other block.
#[test]
fn k4e2_auto_margins_centre_a_table() {
    let (wrapper, grid) = table_wrapper_and_grid(
        "<div id=host><table><tr><td></td></tr></table></div>",
        "#host { width: 300px; }\
         table { display: table; border-spacing: 0; width: 100px;\
                 margin-left: auto; margin-right: auto; }\
         tr { display: table-row; }\
         td { display: table-cell; padding: 0; width: 100px; height: 10px; }",
    );

    assert!((wrapper.width - 100.0).abs() < 0.5, "wrapper: {wrapper:?}");
    assert!((wrapper.x - 100.0).abs() < 0.5, "wrapper: {wrapper:?}");
    assert!((grid.x - 100.0).abs() < 0.5, "grid: {grid:?}");
}

/// K4e3: a captioned table stops deferring.
///
/// The point of the gate. `CaptionMinContribution::PendingK4e` was a named
/// gap rather than a defect, and the 2026-08-03 census counted it firing
/// 17 times; a measured caption closes it, and Buckram sizes the table
/// instead of declining it.
#[test]
fn k4e3_a_captioned_table_no_longer_defers() {
    let dom = StaticDocument::parse(
        "<div id=host><table><caption>a caption</caption>\
         <tr><td>one</td><td>two</td></tr></table></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["#host { width: 400px; } \
             table { display: table; border-spacing: 0; } \
             caption { display: table-caption; margin: 0; padding: 0; } \
             tr { display: table-row; } td { display: table-cell; padding: 0; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");

    let ledger = layout.table_shadow_ledger();
    assert_assigned_and_honored(ledger);
}

/// K4f: `visibility: collapse` removes a row's space, not just its ink.
///
/// CSS 2.1 section 17.5.5 reduces the table's height by exactly what the
/// collapsed row occupied and leaves the other rows the heights they were
/// given. Three 20px rows come to 60; collapsing the middle one leaves 40,
/// with the third row moved up into the gap rather than a hole left where
/// the second used to be.
#[test]
fn k4f_a_collapsed_row_gives_its_space_back_to_the_table() {
    let html = "<div id=host><table><tr id=a><td></td></tr>\
                <tr id=b><td></td></tr><tr id=c><td></td></tr></table></div>";
    let visible = "#host { width: 300px; }\
                   table { display: table; border-spacing: 0; }\
                   tr { display: table-row; }\
                   td { display: table-cell; padding: 0; width: 40px; height: 20px; }";
    let collapsed = format!("{visible} #b {{ visibility: collapse; }}");

    let (_, before) = table_wrapper_and_grid(html, visible);
    let (_, after) = table_wrapper_and_grid(html, &collapsed);
    let rows = table_role_rects(html, &collapsed, InternalTableRole::Row);

    assert!((before.height - 60.0).abs() < 0.5, "before: {before:?}");
    assert!((after.height - 40.0).abs() < 0.5, "after: {after:?}");
    assert!(
        (rows[1].height - 0.0).abs() < 0.5,
        "collapsed: {:?}",
        rows[1]
    );
    assert!(
        (rows[2].y - rows[0].y - 20.0).abs() < 0.5,
        "the third row closes the gap: {:?} {:?}",
        rows[0],
        rows[2]
    );
}

/// K4f: a collapsed column gives its width back the same way.
///
/// Applied through the column group here, which section 17.5.5 also
/// allows: a collapsed `<colgroup>` collapses every column in its range.
#[test]
fn k4f_a_collapsed_column_group_gives_its_width_back() {
    let html = "<div id=host><table><colgroup id=g><col></colgroup>\
                <colgroup><col></colgroup>\
                <tr><td></td><td></td></tr></table></div>";
    let visible = "#host { width: 300px; }\
                   table { display: table; border-spacing: 0; }\
                   colgroup { display: table-column-group; }\
                   col { display: table-column; }\
                   tr { display: table-row; }\
                   td { display: table-cell; padding: 0; width: 40px; height: 20px; }";
    let collapsed = format!("{visible} #g {{ visibility: collapse; }}");

    let (_, before) = table_wrapper_and_grid(html, visible);
    let (wrapper, after) = table_wrapper_and_grid(html, &collapsed);

    assert!((before.width - 80.0).abs() < 0.5, "before: {before:?}");
    assert!((after.width - 40.0).abs() < 0.5, "after: {after:?}");
    // K4e2's rule still holds through the collapse.
    assert!((wrapper.width - after.width).abs() < 0.5, "{wrapper:?}");
}

/// K4e4: used `width` and `height` answer from the grid, not the wrapper.
///
/// The `height` property stayed on the grid under CSS 2.1 section 17.4,
/// so `getComputedStyle(table).height` reports the grid's border box - the
/// 40px of rows, not the 70px wrapper that also contains the caption.
#[test]
fn k4e4_used_height_of_a_captioned_table_is_the_grids() {
    let dom = StaticDocument::parse("<table><caption>above</caption><tr><td>one</td></tr></table>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "table { display: table; table-layout: fixed; width: 200px; border-spacing: 0; } \
             caption { display: table-caption; height: 30px; margin: 0; padding: 0; } \
             tr { display: table-row; height: 40px; } \
             td { display: table-cell; padding: 0; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let table = {
        fn find(
            dom: &StaticDocument,
            node: <StaticDocument as LayoutDom>::NodeId,
        ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
            if dom
                .element_name(node)
                .is_some_and(|name| name.local.as_ref() == "table")
            {
                return Some(node);
            }
            dom.dom_children(node)
                .into_iter()
                .find_map(|child| find(dom, child))
        }
        find(&dom, dom.document()).expect("the table exists")
    };
    let used = used_value_context(&dom, &styles, 320.0, 240.0, table)
        .expect("layout")
        .expect("the table has a fragment");

    assert!((used.border_box.0 - 200.0).abs() < 0.5, "{used:?}");
    assert!(
        (used.border_box.1 - 40.0).abs() < 0.5,
        "the used height is the grid's, without the caption: {used:?}"
    );
}

/// K4e4: an inline-table occupies line space as an atom.
///
/// Deleting K4a's wrapper/grid exclusion from `box_is_inline` lets the
/// wrapper join the inline group, and the atomic-inline lane lays its
/// subtree out separately - the same route an inline-block rides. The
/// receipt is placement: the table sits to the right of the text that
/// precedes it in the same line, at the grid's own width, instead of
/// dropping to a line of its own as the block it used to be built as.
#[test]
fn k4e4_an_inline_table_sits_in_the_text_line() {
    let (wrapper, grid) = table_wrapper_and_grid(
        "<div id=host>before<span class=t><span class=r>\
         <span class=c>cell</span></span></span></div>",
        "#host { width: 300px; font-family: monospace; font-size: 10px;\
                 line-height: 20px; }\
         .t { display: inline-table; border-spacing: 0; }\
         .r { display: table-row; }\
         .c { display: table-cell; padding: 0; width: 50px; height: 12px; }",
    );

    assert!((grid.width - 50.0).abs() < 0.5, "grid: {grid:?}");
    assert!(
        (wrapper.width - grid.width).abs() < 0.5,
        "{wrapper:?} {grid:?}"
    );
    // In the line, after the word, not below it.
    assert!(
        wrapper.x > 30.0,
        "the atom must sit after 'before': {wrapper:?}"
    );
    assert!(
        wrapper.y < 20.0,
        "the atom must sit in the first line: {wrapper:?}"
    );
}

/// B3: K4d5's first table baseline positions a baseline-aligned
/// inline-table. The second row makes the table much taller than its first
/// row, so the old wrapper block-end fallback would put the first cell's
/// text far above its inline peer.
#[test]
fn b3_inline_table_uses_its_first_table_baseline() {
    fn by_id(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "id"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_id(dom, child, expected))
    }

    let dom = StaticDocument::parse(
        "<div id=host><span id=peer>peer</span><span id=table class=t><span class=r>\
         <span id=first class=c>table</span></span>\
         <span class=r><span class=c id=second>lower</span></span></span></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body, div, span { margin: 0; padding: 0; border: 0; }\
             #host { width: 320px; font-family: monospace; font-size: 10px; line-height: 20px; }\
             .t { display: inline-table; border-spacing: 0; vertical-align: baseline; }\
             .r { display: table-row; } .c { display: table-cell; padding: 0; }\
             #first { height: 40px; } #second { height: 60px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let peer_node = by_id(&dom, dom.document(), "peer").expect("peer");
    let peer = layout
        .text_frame()
        .and_then(|frame| frame.first_inline_baseline(peer_node))
        .expect("peer shaped-line baseline");
    let first_cell = layout
        .get(by_id(&dom, dom.document(), "first").expect("first cell"))
        .expect("first cell fragment");
    // This table's default cell baseline is its first row's cell block
    // end. Its row is 40px while the full table is 100px, so the receipt
    // rejects the old 100px wrapper block-end fallback.
    let cell = first_cell.physical_rect().y + first_cell.physical_rect().height;
    let rect = |id| {
        layout
            .get(by_id(&dom, dom.document(), id).expect(id))
            .expect(id)
            .physical_rect()
    };

    assert!(
        (peer - cell).abs() < 0.5,
        "the inline peer and first table-row baseline must agree: peer={peer}, cell={cell}, peer_rect={:?}, cell_rect={:?}, table_rect={:?}",
        rect("peer"),
        rect("first"),
        rect("table"),
    );
}

/// K4e3: a caption wider than the table widens the *grid*.
///
/// The two engines break CSS Tables 3 section 2.2.1 apart here, measured
/// in the K4e1 interop matrix: Chrome grows the grid and its columns to
/// the caption, Firefox leaves the grid at its own content width and lets
/// only the wrapper be caption-wide. Section 2.2.1 says the wrapper's
/// width *is* the grid's border-edge width, which Firefox's answer
/// contradicts and Chrome's keeps, so this keeps the rule.
///
/// C7 of that matrix is the sharpest case to assert, because a specified
/// caption width fixes the expected number without depending on font
/// metrics: the caption's 300 reaches the single column.
#[test]
fn k4e3_a_caption_widens_the_grid_and_its_columns() {
    let html = "<div id=host><table><caption>x</caption>\
                <tr><td></td></tr></table></div>";
    let css = "#host { width: 400px; }\
               table { display: table; border-spacing: 0; }\
               caption { display: table-caption; width: 300px; margin: 0; padding: 0; }\
               tr { display: table-row; }\
               td { display: table-cell; padding: 0; height: 10px; }";
    let (wrapper, grid) = table_wrapper_and_grid(html, css);
    let cells = table_role_rects(html, css, InternalTableRole::Cell);

    assert!((grid.width - 300.0).abs() < 0.5, "grid: {grid:?}");
    assert!((cells[0].width - 300.0).abs() < 0.5, "cell: {:?}", cells[0]);
    // Section 2.2.1 still holds: the wrapper is the grid's width.
    assert!(
        (wrapper.width - grid.width).abs() < 0.5,
        "{wrapper:?} {grid:?}"
    );
}

/// K4e3: a caption's own margins are part of the floor it puts down.
///
/// C5 of the interop matrix, where both engines agree: a 176-wide caption
/// with `margin-left: 30px` contributes 206. Asserted with a specified
/// width so the number does not depend on font metrics.
#[test]
fn k4e3_a_captions_margins_count_toward_what_it_contributes() {
    let html = "<div id=host><table><caption>x</caption>\
                <tr><td></td></tr></table></div>";
    let css = "#host { width: 400px; }\
               table { display: table; border-spacing: 0; }\
               caption { display: table-caption; width: 200px; margin-left: 30px;\
                         padding: 0; }\
               tr { display: table-row; }\
               td { display: table-cell; padding: 0; height: 10px; }";
    let (_, grid) = table_wrapper_and_grid(html, css);

    assert!((grid.width - 230.0).abs() < 0.5, "grid: {grid:?}");
}

/// K4e3: `caption-side` decides which side of the grid a caption lands on.
///
/// CSS 2.1 section 17.4.1 lays a caption above or below the grid inside
/// the wrapper's margins. Buckram's box tree keeps every caption before
/// the grid, so a bottom caption has to be reordered on the way into
/// layout - and C4 of the matrix pins that the side does not change what
/// the caption contributes to sizing.
#[test]
fn k4e3_caption_side_moves_the_caption_without_changing_the_table() {
    let html = "<div id=host><table><caption>x</caption>\
                <tr><td></td></tr></table></div>";
    let above = "#host { width: 400px; }\
                 table { display: table; border-spacing: 0; }\
                 caption { display: table-caption; width: 300px; height: 20px;\
                           margin: 0; padding: 0; }\
                 tr { display: table-row; }\
                 td { display: table-cell; padding: 0; height: 10px; }";
    let below = format!("{above} caption {{ caption-side: bottom; }}");

    let (top_wrapper, top_grid) = table_wrapper_and_grid(html, above);
    let top_caption = table_role_rects(html, above, InternalTableRole::Caption)[0];
    let (bottom_wrapper, bottom_grid) = table_wrapper_and_grid(html, &below);
    let bottom_caption = table_role_rects(html, &below, InternalTableRole::Caption)[0];

    assert!(
        top_caption.y < top_grid.y,
        "a top caption sits above the grid: {top_caption:?} {top_grid:?}"
    );
    assert!(
        bottom_caption.y > bottom_grid.y,
        "a bottom caption sits below the grid: {bottom_caption:?} {bottom_grid:?}"
    );
    // The side is placement only; the sizing it forces is the same.
    assert!((top_grid.width - bottom_grid.width).abs() < 0.5);
    assert!((top_wrapper.height - bottom_wrapper.height).abs() < 0.5);
}

/// B3: `caption-side` is the table wrapper's logical block-axis order.
/// In vertical-rl, block-start is physical right and block-end is left;
/// assigning caption placement to fragmentation would lose that wrapper
/// relationship before any fragmentainer exists.
#[test]
fn b3_vertical_caption_side_uses_the_wrappers_logical_block_axis() {
    let html = "<div id=host><table><caption>x</caption>\
                <tr><td></td></tr></table></div>";
    for (writing_mode, top_at_right) in [("vertical-rl", true), ("vertical-lr", false)] {
        let top = format!(
            "#host {{ width: 300px; height: 200px; }}\
             table {{ display: table; writing-mode: {writing_mode}; height: 100px; border-spacing: 0; }}\
             caption {{ display: table-caption; width: 40px; height: 20px; margin: 0; padding: 0; }}\
             tr {{ display: table-row; }} td {{ display: table-cell; padding: 0; width: 60px; height: 30px; }}"
        );
        let bottom = format!("{top} caption {{ caption-side: bottom; }}");
        let top_caption = table_role_rects(html, &top, InternalTableRole::Caption)[0];
        let top_grid = table_role_rects(html, &top, InternalTableRole::Grid)[0];
        let bottom_caption = table_role_rects(html, &bottom, InternalTableRole::Caption)[0];
        let bottom_grid = table_role_rects(html, &bottom, InternalTableRole::Grid)[0];

        assert_eq!(
            top_caption.x > top_grid.x,
            top_at_right,
            "{writing_mode} top caption must occupy its block-start: {top_caption:?} {top_grid:?}"
        );
        assert_eq!(
            bottom_caption.x < bottom_grid.x,
            top_at_right,
            "{writing_mode} bottom caption must occupy its block-end: {bottom_caption:?} {bottom_grid:?}"
        );
    }
}

/// K4d6: a table row now has a fragment of its own.
///
/// Buckram emits the whole structural subtree from the track model, so
/// each part gets its exact rectangle whether or not a cell covers it.
#[test]
fn k4d6_rows_groups_and_columns_have_their_own_fragments() {
    let dom = StaticDocument::parse(
        "<table><colgroup><col></colgroup><tbody><tr id=a><td>one</td></tr><tr id=b><td>two</td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "table { display: table; table-layout: fixed; width: 200px; border-spacing: 0; }                  tbody { display: table-row-group; } tr { display: table-row; }                  td { display: table-cell; padding: 0; } #a { height: 40px; } #b { height: 60px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");

    let rect_of = |local: &str, nth: usize| {
        fn walk(
            dom: &StaticDocument,
            node: <StaticDocument as LayoutDom>::NodeId,
            local: &str,
            out: &mut Vec<<StaticDocument as LayoutDom>::NodeId>,
        ) {
            if dom
                .element_name(node)
                .is_some_and(|n| n.local.as_ref() == local)
            {
                out.push(node);
            }
            for child in dom.dom_children(node) {
                walk(dom, child, local, out);
            }
        }
        let mut found = Vec::new();
        walk(&dom, dom.document(), local, &mut found);
        // A table element generates a wrapper and a grid; only one of
        // them carries the fragment.
        layout
            .boxes()
            .boxes_for_node(found[nth])
            .iter()
            .find_map(|box_id| layout.fragments().fragments_for_box(*box_id).next())
            .unwrap_or_else(|| panic!("no fragment for {local}[{nth}]"))
            .physical_rect()
    };

    let table = rect_of("table", 0);
    let first = rect_of("tr", 0);
    let second = rect_of("tr", 1);
    let group = rect_of("tbody", 0);
    let column = rect_of("col", 0);

    // Each row spans the grid and holds exactly its own track.
    assert!((first.height - 40.0).abs() < 0.5, "first row: {first:?}");
    assert!((second.height - 60.0).abs() < 0.5, "second row: {second:?}");
    assert!(
        (second.y - first.y - 40.0).abs() < 0.5,
        "rows must tile: {first:?} {second:?}"
    );
    assert!((first.width - 200.0).abs() < 0.5, "{first:?}");

    // A group's rectangle is the exact union of its track range, not a
    // box reconstructed from the cells inside it.
    assert!((group.y - first.y).abs() < 0.5, "{group:?}");
    assert!((group.height - 100.0).abs() < 0.5, "{group:?}");

    // A column runs the table's whole block extent.
    assert!((column.height - table.height).abs() < 0.5, "{column:?}");
    assert!((column.width - 200.0).abs() < 0.5, "{column:?}");
}

/// K4d4c: `min-height` and `max-height` do not reach a table cell.
///
/// CSS 2.1 section 10.7 leaves their effect on table cells, rows, and row
/// groups undefined, and Chrome 150 and Firefox 153 both ignore them
/// outright in all eight measured cases. So a cell carrying them is
/// ordinary work rather than a deferral, and a 100px child keeps its
/// 100px row against a `max-height: 20px` cell.
#[test]
fn k4d4c_cell_min_and_max_height_are_ignored() {
    let dom = StaticDocument::parse(
        "<table><tbody><tr><td><div class=tall></div></td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "table { display: table; table-layout: fixed; width: 100px; border-spacing: 0; }                  tbody { display: table-row-group; } tr { display: table-row; }                  td { display: table-cell; padding: 0; height: 20px; max-height: 20px;                  min-height: 5px; } .tall { height: 100px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let ledger = layout.table_shadow_ledger();
    assert_eq!(
        ledger.block.laid_out, 1,
        "a cell max-height must not defer the table: {:?}",
        ledger.block
    );
    assert!(
        ledger.block.skipped.is_empty(),
        "{:?}",
        ledger.block.skipped
    );
    assert_eq!(
        ledger.block.agreed, 1,
        "the painted cell must be the one Buckram wrote: {:?}",
        ledger.block.divergences
    );
}

fn automatic_table_ledger(table_css: &str) -> crate::table_shadow::TableShadowLedger {
    let dom = StaticDocument::parse(
        "<table><tbody><tr><td>one</td><td>one</td><td>one</td></tr></tbody></table>",
    );
    let css = format!(
        "table {{ display: table; border-spacing: 0; {table_css} }} tbody {{ display: table-row-group; }} tr {{ display: table-row; }} td {{ display: table-cell; padding: 0; }}"
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[&css]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    layout(&dom, &styles, 320.0, 240.0)
        .expect("layout")
        .table_shadow_ledger()
        .clone()
}

/// K4c5b: a shrink-to-fit automatic table is sized by Buckram from cell
/// intrinsics measured through the live machinery before the main pass.
#[test]
fn k4c5b_automatic_shrink_to_fit_is_buckram_owned() {
    assert_assigned_and_honored(&automatic_table_ledger(""));
}

/// The second K4c5a divergence, resolved by authority: an automatic table
/// explicitly wider than its max-content distributes the extra space over
/// its columns (CSS 2.1 17.5.2.2). Buckram's 100px columns paint,
/// verified against the fragments.
#[test]
fn k4c5b_automatic_explicit_width_is_distributed_and_painted() {
    assert_assigned_and_honored(&automatic_table_ledger("width: 300px;"));
}

/// K4c5b on the production text path: automatic tables there previously
/// could not even be shadowed; they are now sized by Buckram.
#[test]
fn k4c5b_text_path_automatic_tables_are_buckram_owned() {
    let dom = StaticDocument::parse(
        "<p>before</p><table><tbody><tr><td>one</td><td>two</td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "table { display: table; border-spacing: 0; } tbody { display: table-row-group; } tr { display: table-row; } td { display: table-cell; padding: 0; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let ledger = layout.table_shadow_ledger();
    assert!(
        ledger.assigned >= 1,
        "the text path's automatic table was not sized by Buckram: {ledger:?}"
    );
    assert!(ledger.is_silent(), "{ledger:?}");
}

/// Regression for the K4a/K4b-window crash in WPT
/// `css/CSS2/css21-errata/s-11-1-1b-005.html`, run byte-exact from the
/// in-repo corpus: the root element styled `display: table` with a bare
/// `table-cell` `<body>` whose `margin-top: -15px` places its baseline
/// above the parent's block-start edge. Baseline propagation asserted
/// offsets were non-negative and panicked on the legitimate negative one.
#[test]
fn a_root_element_table_with_a_negative_margin_cell_does_not_panic() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/wpt/tests/css/CSS2/css21-errata/s-11-1-1b-005.html"
    ))
    .expect("in-repo WPT corpus file");
    let style_start = source.find("<style>").expect("style open") + "<style>".len();
    let style_end = source.find("</style>").expect("style close");
    let css = source[style_start..style_end].to_owned();
    let dom = StaticDocument::parse(&source);
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[&css]),
        &Device::screen(800.0, 600.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    layout_with_text_system(
        &dom,
        &styles,
        800.0,
        600.0,
        ViewportSizes::uniform(800.0, 600.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout must not panic");
}

/// K4d1 adapter fixture: a `TableCellFormatter` over the live algorithm
/// tree formats block and flex cell contents at the exact inline size the
/// table algorithm supplies. The scratch tree contains only cell
/// subtrees, so the table and its row structurally record no backend
/// call, while the flex cell dispatches through its own algorithm; the
/// old bridge's table-as-Grid node does not exist here at all.
#[test]
fn k4d1_cell_formatter_formats_contents_at_exact_inline_sizes() {
    use buckram::{
        FragmentDraft, FragmentDraftTree, TableCellFormatter, TableCellLayoutInput,
        TableCellLayoutOutput, TableRowLayoutError,
    };

    struct TreeFormatter<'a> {
        tree: &'a mut AlgorithmTree<Style, TextMeasure, Option<BoxId>>,
        nodes: HashMap<BoxId, AlgorithmNodeId>,
        formatted: Vec<(BoxId, f32)>,
    }

    impl TableCellFormatter for TreeFormatter<'_> {
        fn format_cell(
            &mut self,
            input: TableCellLayoutInput,
        ) -> Result<TableCellLayoutOutput, TableRowLayoutError> {
            let node =
                *self
                    .nodes
                    .get(&input.box_id)
                    .ok_or(TableRowLayoutError::InvalidCellOutput {
                        box_id: input.box_id,
                    })?;
            self.tree.compute_layout_with_measure(
                node,
                AlgorithmSize::new(
                    AlgorithmAvailableSpace::Definite(input.content_inline_size),
                    AlgorithmAvailableSpace::MaxContent,
                ),
                |known, _, _, context, _| {
                    let Some(context) = context else {
                        return AlgorithmSize::new(0.0, 0.0);
                    };
                    AlgorithmSize::new(
                        known.width.unwrap_or(context.max_width),
                        known.height.unwrap_or(context.height),
                    )
                },
            );
            let layout = self.tree.layout(node);
            self.formatted
                .push((input.box_id, input.content_inline_size));
            let mut fragments = FragmentDraftTree::default();
            fragments.push(FragmentDraft {
                box_id: input.box_id,
                logical_rect: buckram::LogicalRect::default(),
                overflow: buckram::LogicalRect::default(),
                parent: None,
            });
            Ok(TableCellLayoutOutput {
                content_block_size: layout.height,
                border_box_min_block_size: layout.height,
                // K4d5 owns real cell baselines; the contract placeholder
                // synthesizes from the block end.
                baselines: buckram::Baselines::synthesized_from_block_end(layout.height),
                overflow: buckram::LogicalRect::default(),
                fragments,
            })
        }
    }

    // A real grid from a two-cell table document; the algorithm tree gets
    // one block cell and one flex cell, and nothing else.
    let dom = StaticDocument::parse(
        "<table><tbody><tr><td id=blocky>x</td><td id=flexy><i>a</i><i>b</i></td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "table { display: table; border-spacing: 0; } tbody { display: table-row-group; } tr { display: table-row; } td { display: table-cell; padding: 0; } #flexy { display: table-cell; } #flexy i { display: block; height: 7px; }",
        ]),
        &Device::screen(800.0, 600.0),
        &InteractionStates::default(),
    );
    let boxes = GeneratedBoxTree::from_dom(&dom, &styles);
    let table = boxes
        .iter()
        .find_map(|(box_id, css_box)| {
            (css_box.display.internal_table == Some(buckram::InternalTableRole::Grid))
                .then_some(box_id)
        })
        .expect("table grid box");
    let grid = build_table_grid(&boxes, &dom, table);
    assert_eq!(grid.cells.len(), 2);

    let mut tree: AlgorithmTree<Style, TextMeasure, Option<BoxId>> = AlgorithmTree::new();
    let mut nodes = HashMap::new();
    for cell in &grid.cells {
        let kind = if nodes.is_empty() {
            AlgorithmKind::Block
        } else {
            AlgorithmKind::Flex
        };
        let children = if kind == AlgorithmKind::Flex {
            vec![tree.new_with_children(
                AlgorithmKind::Block,
                Style {
                    size: Size {
                        width: Dimension::auto(),
                        height: Dimension::length(7.0),
                    },
                    ..Style::default()
                },
                &[],
                None,
            )]
        } else {
            Vec::new()
        };
        let node = tree.new_with_children(kind, Style::default(), &children, None);
        nodes.insert(cell.source, node);
    }
    let inline = {
        let sizing = buckram::TableInlineSizingInput {
            grid: &grid,
            available_inline_size: Some(150.0),
            table_constraints: buckram::TableInlineConstraints::default(),
            border_metrics: buckram::TableInlineBorderMetrics::Separated(
                buckram::TableSeparatedBorderMetrics::default(),
            ),
            caption_min: buckram::CaptionMinContribution::NoCaption,
            track_visibility: buckram::TableTrackVisibility::all_visible(&grid),
        };
        buckram::TableInlineSizingResult::new(
            &sizing,
            buckram::IntrinsicSizes::new(150.0, 150.0).expect("intrinsic pair"),
            150.0,
            150.0,
            vec![90.0, 60.0],
        )
        .expect("reconciled inline result")
    };
    let input = buckram::TableBlockSizingInput {
        grid: &grid,
        inline: &inline,
        table_constraint: buckram::TableBlockConstraint::Auto,
        table_box_sizing: buckram::TableBoxSizing::BorderBox,
        row_group_constraints: &[],
        border_metrics: buckram::TableBlockBorderMetrics::Separated(
            buckram::TableSeparatedBlockMetrics::default(),
        ),
        available_block_size: None,
        track_visibility: buckram::TableTrackVisibility::all_visible(&grid),
    };
    let mut formatter = TreeFormatter {
        tree: &mut tree,
        nodes,
        formatted: Vec::new(),
    };
    let outputs = buckram::format_table_cells(&input, 0.0, |_, _| 0.0, &mut formatter)
        .expect("formatted cells");

    assert_eq!(outputs.len(), 2);
    // Exact K4c column sizes reached the formatter.
    let widths = formatter
        .formatted
        .iter()
        .map(|(_, width)| *width)
        .collect::<Vec<_>>();
    assert_eq!(widths, vec![90.0, 60.0]);
    // The tree holds only cell subtrees: no node represents the table or
    // the row, so neither can have recorded a backend call.
    assert_eq!(tree.node_ids().count(), 3);
    assert!(
        tree.node_ids()
            .all(|id| tree.kind(id) != AlgorithmKind::Grid),
        "no table-as-Grid node may exist in the K4d dispatch shape"
    );
}

/// K4d2 adapter fixture: real cell contents are formatted at exact K4c
/// inline sizes with an indefinite first-pass block size, and the row
/// minima that follow come from those measured contents. The taller row
/// is taller because its content is, not because anything was assumed.
#[test]
fn k4d2_row_minima_follow_from_formatted_cell_contents() {
    use buckram::{
        FragmentDraftTree, TableCellBlockStyle, TableCellFormatter, TableCellLayoutInput,
        TableCellLayoutOutput, TableCellLayoutPass, TableRowLayoutError,
    };

    struct TreeFormatter<'a> {
        tree: &'a mut AlgorithmTree<Style, TextMeasure, Option<BoxId>>,
        nodes: HashMap<BoxId, AlgorithmNodeId>,
        requests: Vec<TableCellLayoutInput>,
    }

    impl TableCellFormatter for TreeFormatter<'_> {
        fn format_cell(
            &mut self,
            input: TableCellLayoutInput,
        ) -> Result<TableCellLayoutOutput, TableRowLayoutError> {
            self.requests.push(input);
            let node =
                *self
                    .nodes
                    .get(&input.box_id)
                    .ok_or(TableRowLayoutError::InvalidCellOutput {
                        box_id: input.box_id,
                    })?;
            self.tree.compute_layout_with_measure(
                node,
                AlgorithmSize::new(
                    AlgorithmAvailableSpace::Definite(input.content_inline_size),
                    // The first pass is deliberately indefinite in the
                    // block axis: a cell height must not stretch its
                    // content formatting context.
                    AlgorithmAvailableSpace::MaxContent,
                ),
                |known, _, _, context, _| {
                    let Some(context) = context else {
                        return AlgorithmSize::new(0.0, 0.0);
                    };
                    AlgorithmSize::new(
                        known.width.unwrap_or(context.max_width),
                        known.height.unwrap_or(context.height),
                    )
                },
            );
            let layout = self.tree.layout(node);
            Ok(TableCellLayoutOutput {
                content_block_size: layout.height,
                border_box_min_block_size: 0.0,
                baselines: buckram::Baselines::synthesized_from_block_end(layout.height),
                overflow: buckram::LogicalRect::default(),
                fragments: FragmentDraftTree::default(),
            })
        }
    }

    // Row 0's cell is three stacked 9px blocks; row 1's is one.
    let dom = StaticDocument::parse(
        "<table><tbody><tr><td id=tall><i></i><i></i><i></i></td></tr><tr><td id=short><i></i></td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "table { display: table; border-spacing: 0; } tbody { display: table-row-group; } tr { display: table-row; } td { display: table-cell; padding: 0; } i { display: block; height: 9px; }",
        ]),
        &Device::screen(800.0, 600.0),
        &InteractionStates::default(),
    );
    let boxes = GeneratedBoxTree::from_dom(&dom, &styles);
    let table = boxes
        .iter()
        .find_map(|(box_id, css_box)| {
            (css_box.display.internal_table == Some(buckram::InternalTableRole::Grid))
                .then_some(box_id)
        })
        .expect("table grid box");
    let grid = build_table_grid(&boxes, &dom, table);
    assert_eq!(grid.rows.len(), 2);
    assert_eq!(grid.cells.len(), 2);

    let mut tree: AlgorithmTree<Style, TextMeasure, Option<BoxId>> = AlgorithmTree::new();
    let mut nodes = HashMap::new();
    for (index, cell) in grid.cells.iter().enumerate() {
        let blocks = (0..if index == 0 { 3 } else { 1 })
            .map(|_| {
                tree.new_with_children_and_block_style(
                    AlgorithmKind::Block,
                    BlockStyle {
                        size: BlockDimensions::new(
                            BlockSizeValue::Auto,
                            BlockSizeValue::Length(FlowLength::px(9.0)),
                        ),
                        ..BlockStyle::default()
                    },
                    Style {
                        size: Size {
                            width: Dimension::auto(),
                            height: Dimension::length(9.0),
                        },
                        ..Style::default()
                    },
                    &[],
                    None,
                )
            })
            .collect::<Vec<_>>();
        nodes.insert(
            cell.source,
            tree.new_with_children_and_block_style(
                AlgorithmKind::Block,
                BlockStyle::default(),
                Style::default(),
                &blocks,
                None,
            ),
        );
    }

    let inline = {
        let sizing = buckram::TableInlineSizingInput {
            grid: &grid,
            available_inline_size: Some(80.0),
            table_constraints: buckram::TableInlineConstraints::default(),
            border_metrics: buckram::TableInlineBorderMetrics::Separated(
                buckram::TableSeparatedBorderMetrics::default(),
            ),
            caption_min: buckram::CaptionMinContribution::NoCaption,
            track_visibility: buckram::TableTrackVisibility::all_visible(&grid),
        };
        buckram::TableInlineSizingResult::new(
            &sizing,
            buckram::IntrinsicSizes::new(80.0, 80.0).expect("intrinsic pair"),
            80.0,
            80.0,
            vec![80.0],
        )
        .expect("reconciled inline result")
    };
    let input = buckram::TableBlockSizingInput {
        grid: &grid,
        inline: &inline,
        table_constraint: buckram::TableBlockConstraint::Auto,
        table_box_sizing: buckram::TableBoxSizing::BorderBox,
        row_group_constraints: &[],
        border_metrics: buckram::TableBlockBorderMetrics::Separated(
            buckram::TableSeparatedBlockMetrics::default(),
        ),
        available_block_size: None,
        track_visibility: buckram::TableTrackVisibility::all_visible(&grid),
    };
    let mut formatter = TreeFormatter {
        tree: &mut tree,
        nodes,
        requests: Vec::new(),
    };
    let outputs = buckram::format_table_cells(&input, 0.0, |_, _| 0.0, &mut formatter)
        .expect("formatted cells");

    // Every first-pass request carried the exact column size and an
    // indefinite block size.
    assert!(formatter.requests.iter().all(|request| {
        request.content_inline_size == 80.0
            && request.available_block_size.is_none()
            && request.percentage_basis.is_none()
            && request.pass == TableCellLayoutPass::Measure
    }));

    let rows = buckram::measure_single_span_rows(
        &input,
        &[TableCellBlockStyle::default(); 2],
        &outputs,
        &[buckram::TableBlockConstraint::Auto; 2],
    )
    .expect("row measures");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].min_block_size, 27.0);
    assert_eq!(rows[1].min_block_size, 9.0);
    assert!(rows.iter().all(|row| !row.constrained && row.row.is_some()));
}

/// K4d5 adapter fixture: block and flex cell contents return their
/// baselines through the formatter's own output, and the table algorithm
/// aligns from those values alone. Nothing walks a backend descendant to
/// rediscover a baseline, and no physical coordinate is stored as one.
#[test]
fn k4d5_cell_contents_return_baselines_directly() {
    use buckram::{
        FragmentDraftTree, TableCellAlignment, TableCellBlockStyle, TableCellFormatter,
        TableCellLayoutInput, TableCellLayoutOutput, TableRowLayoutError,
    };

    struct BaselineFormatter<'a> {
        tree: &'a mut AlgorithmTree<Style, TextMeasure, Option<BoxId>>,
        nodes: HashMap<BoxId, AlgorithmNodeId>,
    }

    impl TableCellFormatter for BaselineFormatter<'_> {
        fn format_cell(
            &mut self,
            input: TableCellLayoutInput,
        ) -> Result<TableCellLayoutOutput, TableRowLayoutError> {
            let node =
                *self
                    .nodes
                    .get(&input.box_id)
                    .ok_or(TableRowLayoutError::InvalidCellOutput {
                        box_id: input.box_id,
                    })?;
            self.tree.compute_layout_with_measure(
                node,
                AlgorithmSize::new(
                    AlgorithmAvailableSpace::Definite(input.content_inline_size),
                    AlgorithmAvailableSpace::MaxContent,
                ),
                |known, _, _, context, _| {
                    let Some(context) = context else {
                        return AlgorithmSize::new(0.0, 0.0);
                    };
                    AlgorithmSize::new(
                        known.width.unwrap_or(context.max_width),
                        known.height.unwrap_or(context.height),
                    )
                },
            );
            self.tree.propagate_baselines();
            let layout = self.tree.layout(node);
            // The formatting context hands its baselines back directly.
            let baselines = self.tree.baselines(node);
            Ok(TableCellLayoutOutput {
                content_block_size: layout.height,
                border_box_min_block_size: 0.0,
                baselines,
                overflow: buckram::LogicalRect::default(),
                fragments: FragmentDraftTree::default(),
            })
        }
    }

    let dom = StaticDocument::parse(
        "<table><tbody><tr><td id=a><i></i></td><td id=b><i></i><i></i></td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "table { display: table; border-spacing: 0; } tbody { display: table-row-group; } tr { display: table-row; } td { display: table-cell; padding: 0; } i { display: block; height: 12px; }",
        ]),
        &Device::screen(800.0, 600.0),
        &InteractionStates::default(),
    );
    let boxes = GeneratedBoxTree::from_dom(&dom, &styles);
    let table = boxes
        .iter()
        .find_map(|(box_id, css_box)| {
            (css_box.display.internal_table == Some(buckram::InternalTableRole::Grid))
                .then_some(box_id)
        })
        .expect("table grid box");
    let grid = build_table_grid(&boxes, &dom, table);

    let mut tree: AlgorithmTree<Style, TextMeasure, Option<BoxId>> = AlgorithmTree::new();
    let mut nodes = HashMap::new();
    for (index, cell) in grid.cells.iter().enumerate() {
        // A block container's first baseline is its first child's, so the
        // cells differ in their first block rather than their count.
        let height = if index == 0 { 20.0 } else { 12.0 };
        let blocks = vec![tree.new_with_children_and_block_style(
            AlgorithmKind::Block,
            BlockStyle {
                size: BlockDimensions::new(
                    BlockSizeValue::Auto,
                    BlockSizeValue::Length(FlowLength::px(height)),
                ),
                ..BlockStyle::default()
            },
            Style {
                size: Size {
                    width: Dimension::auto(),
                    height: Dimension::length(height),
                },
                ..Style::default()
            },
            &[],
            None,
        )];
        nodes.insert(
            cell.source,
            tree.new_with_children_and_block_style(
                AlgorithmKind::Block,
                BlockStyle::default(),
                Style::default(),
                &blocks,
                None,
            ),
        );
    }

    let inline = {
        let sizing = buckram::TableInlineSizingInput {
            grid: &grid,
            available_inline_size: Some(80.0),
            table_constraints: buckram::TableInlineConstraints::default(),
            border_metrics: buckram::TableInlineBorderMetrics::Separated(
                buckram::TableSeparatedBorderMetrics::default(),
            ),
            caption_min: buckram::CaptionMinContribution::NoCaption,
            track_visibility: buckram::TableTrackVisibility::all_visible(&grid),
        };
        buckram::TableInlineSizingResult::new(
            &sizing,
            buckram::IntrinsicSizes::new(80.0, 80.0).expect("intrinsic pair"),
            80.0,
            80.0,
            vec![40.0, 40.0],
        )
        .expect("reconciled inline result")
    };
    let input = buckram::TableBlockSizingInput {
        grid: &grid,
        inline: &inline,
        table_constraint: buckram::TableBlockConstraint::Auto,
        table_box_sizing: buckram::TableBoxSizing::BorderBox,
        row_group_constraints: &[],
        border_metrics: buckram::TableBlockBorderMetrics::Separated(
            buckram::TableSeparatedBlockMetrics::default(),
        ),
        available_block_size: None,
        track_visibility: buckram::TableTrackVisibility::all_visible(&grid),
    };
    let mut formatter = BaselineFormatter {
        tree: &mut tree,
        nodes,
    };
    let outputs = buckram::format_table_cells(&input, 0.0, |_, _| 0.0, &mut formatter)
        .expect("formatted cells");
    // Each cell reported a real baseline from its own formatting context.
    assert!(
        outputs
            .iter()
            .all(|(_, output)| output.baselines.first.is_some())
    );

    let styles = vec![TableCellBlockStyle::default(); grid.cells.len()];
    let mut measures = buckram::measure_single_span_rows(
        &input,
        &styles,
        &outputs,
        &[buckram::TableBlockConstraint::Auto],
    )
    .expect("measures");
    buckram::apply_baseline_row_minima(&input, &styles, &outputs, &mut measures)
        .expect("baseline minima");
    let sizing = buckram::size_table_rows(&input, &measures, &styles, &outputs).expect("sizing");
    let alignment =
        buckram::align_table_cells(&input, &sizing, &styles, &outputs, 0.0).expect("alignment");

    // Cell a's baseline is 20 and cell b's is 12, so the row takes 20 and
    // shifts b down by 8 to meet it.
    assert!(alignment.rows[0].from_aligned_cell);
    assert!(
        (alignment.rows[0].baseline - 20.0).abs() < 0.05,
        "{alignment:?}"
    );
    assert!(
        (alignment.cells[0].content_block_offset).abs() < 0.05,
        "{alignment:?}"
    );
    assert!(
        (alignment.cells[1].content_block_offset - 8.0).abs() < 0.05,
        "{alignment:?}"
    );
    assert_eq!(
        alignment.baselines.first,
        Some(alignment.rows[0].baseline),
        "the table's first baseline is its first row's"
    );
    // Alignment never touches K4c's columns.
    assert_eq!(inline.column_sizes, vec![40.0, 40.0]);
    assert!(
        alignment
            .cells
            .iter()
            .all(|cell| (cell.rect.inline_size - 40.0).abs() < 0.05)
    );
    assert_eq!(
        buckram::TableCellAlignment::default(),
        TableCellAlignment::Baseline
    );
}

#[test]
fn html_column_and_column_group_spans_are_bounded_at_the_adapter() {
    let dom = StaticDocument::parse(
        "<table id=table><colgroup span=9001></colgroup><col span=9001><tbody><tr><td></td></tr></tbody></table>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "table { display: table; } colgroup { display: table-column-group; } col { display: table-column; } tbody { display: table-row-group; } tr { display: table-row; } td { display: table-cell; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let boxes = GeneratedBoxTree::from_dom(&dom, &styles);
    let table = node_by_id(&dom, dom.document(), "table").expect("table");
    let grid = boxes.principal_box(table).expect("table grid");
    let model = build_table_grid(&boxes, &dom, grid);

    assert_eq!(model.column_groups[0].span, 1_000);
    assert_eq!(model.columns.len(), 2_000);
}

#[test]
fn retained_inline_format_is_not_shaped_again_for_paint() {
    let dom = StaticDocument::parse(
        "<html><body><div class=\"label\"><span id=\"split\">one two three four</span></div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[".label { width: 80px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (styles, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let after_layout = text.shape_count();
    let split = {
        fn find(
            dom: &StaticDocument,
            node: <StaticDocument as LayoutDom>::NodeId,
        ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
            if dom
                .element_name(node)
                .is_some_and(|name| name.local.as_ref() == "span")
            {
                return Some(node);
            }
            dom.dom_children(node).find_map(|child| find(dom, child))
        }
        find(&dom, dom.document()).expect("split span")
    };

    assert!(after_layout > 0);
    assert!(
        layout.fragments_for_node(split).count() >= 2,
        "one inline box must own one fragment per wrapped line"
    );
    let _ = emit_paint_list_with_text_system(
        &dom,
        &styles,
        &layout,
        DeviceIntSize::new(320, 240),
        1,
        &mut text,
    );
    assert_eq!(
        text.shape_count(),
        after_layout,
        "paint must consume the retained inline result"
    );
}

#[test]
fn split_inline_continuations_format_their_own_box_children() {
    let dom = StaticDocument::parse(
        "<html><body><div class=\"host\"><span>before<div class=\"block\">block</div>after</span></div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[".host { width: 120px; } .block { display: block; height: 20px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let split = {
        fn find(
            dom: &StaticDocument,
            node: <StaticDocument as LayoutDom>::NodeId,
        ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
            if dom
                .element_name(node)
                .is_some_and(|name| name.local.as_ref() == "span")
            {
                return Some(node);
            }
            dom.dom_children(node).find_map(|child| find(dom, child))
        }
        find(&dom, dom.document()).expect("split span")
    };
    let boxes = layout.boxes().boxes_for_node(split);
    let first = layout
        .fragments()
        .fragments_for_box(boxes[0])
        .next()
        .expect("first continuation")
        .physical_rect();
    let second = layout
        .fragments()
        .fragments_for_box(boxes[1])
        .next()
        .expect("second continuation")
        .physical_rect();

    assert_eq!(boxes.len(), 2);
    assert!(
        second.y > first.y,
        "the block between continuation boxes must advance block flow"
    );
}

#[test]
fn partial_inline_groups_do_not_share_one_box_intrinsic_cache_entry() {
    fn find(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom
            .element_name(node)
            .is_some_and(|name| name.local.as_ref() == "div")
        {
            return Some(node);
        }
        dom.dom_children(node).find_map(|child| find(dom, child))
    }

    let dom = StaticDocument::parse(
        "<html><body><div>before<span class=\"out\">out</span>after</div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[".out { position: absolute; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let boxes = GeneratedBoxTree::from_dom(&dom, &styles);
    let host = find(&dom, dom.document()).expect("host");
    let host_box = boxes.principal_box(host).expect("host box");

    assert_eq!(
        intrinsic_owner_for_flow_children(&boxes, host_box, boxes[host_box].children()),
        None,
        "two partial inline groups must not alias the parent box query"
    );
}

#[test]
fn ordinary_live_block_flow_uses_buckram_without_backend_dispatch() {
    fn collect_divs(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        output: &mut Vec<<StaticDocument as LayoutDom>::NodeId>,
    ) {
        if dom
            .element_name(node)
            .is_some_and(|name| name.local.as_ref() == "div")
        {
            output.push(node);
        }
        for child in dom.dom_children(node) {
            collect_divs(dom, child, output);
        }
    }

    let dom = StaticDocument::parse(
        "<html><body><div class=\"host\"><div></div><div></div></div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body, div { margin: 0; padding: 0; border: 0; } .host > div { height: 20px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let mut divs = Vec::new();
    collect_divs(&dom, dom.document(), &mut divs);
    let first = layout.get(divs[1]).expect("first child").physical_rect();
    let second = layout.get(divs[2]).expect("second child").physical_rect();
    let algorithms = layout.block_algorithm_counts();

    assert!(
        algorithms.buckram >= 4,
        "the root, html, body, and host block contexts should use Buckram"
    );
    assert_eq!(algorithms.taffy, 0);
    assert_eq!(second.y, first.y + 20.0);
}

#[test]
fn live_orthogonal_normal_flow_preserves_logical_fragment_geometry_and_baseline() {
    use buckram::{Direction, WritingMode};

    let document = StaticDocument::parse(
        "<html><body><div class=\"vertical\"><div>orthogonal text</div></div></body></html>",
    );
    let styles = resolve_styles(
        &document,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; border: 0; } \
             .vertical { writing-mode: vertical-rl; height: 100px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let vertical = document
        .first_with_class(document.document(), "vertical")
        .expect("vertical host");
    assert!(
        styles
            .get(vertical)
            .is_some_and(|style| style.writing_mode.is_vertical()),
        "the cascade must retain vertical-rl for the principal box"
    );
    let mut text = TextSystem::new();
    let (resolved, layout) = layout_with_text_system(
        &document,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("orthogonal layout");

    assert!(
        resolved
            .get(vertical)
            .is_some_and(|style| style.writing_mode.is_vertical()),
        "relative-unit resolution must preserve vertical-rl"
    );

    let vertical_box = layout
        .boxes()
        .principal_box(vertical)
        .expect("vertical principal box");
    assert_eq!(
        layout.boxes()[vertical_box].flow,
        FlowAxes::new(WritingMode::VerticalRl, Direction::Ltr),
        "the generated principal box must preserve the computed flow"
    );
    let fragment = layout
        .fragments()
        .fragments_for_box(vertical_box)
        .next()
        .expect("vertical fragment");
    let physical = fragment.physical_rect();
    assert_eq!(
        fragment.flow(),
        FlowAxes::new(WritingMode::VerticalRl, Direction::Ltr)
    );
    assert_eq!(fragment.logical_rect.inline_size, physical.height);
    assert_eq!(fragment.logical_rect.block_size, physical.width);
    assert!(
        fragment.baselines.first.is_some() && fragment.baselines.last.is_some(),
        "the host must retain its modeled BFC baseline output"
    );
    assert!(
        fragment
            .containing_fragment()
            .and_then(|id| layout.fragments().get(id))
            .is_some(),
        "the orthogonal host must retain its containing fragment"
    );
    assert!(layout.block_algorithm_counts().buckram >= 4);
}

#[test]
fn live_two_level_orthogonal_flow_uses_the_inner_line_block_contribution() {
    let document = StaticDocument::parse(
        "<html><body><div class=\"vertical\"><div class=\"line\">A B C D E F G</div></div><div class=\"after\"></div></body></html>",
    );
    let styles = resolve_styles(
        &document,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; border: 0; } \
             .vertical { writing-mode: vertical-rl; background: yellow; } \
             .line { writing-mode: horizontal-tb; } .after { height: 10px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let vertical = document
        .first_with_class(document.document(), "vertical")
        .expect("vertical host");
    let line = document
        .first_with_class(document.document(), "line")
        .expect("horizontal line");
    let after = document
        .first_with_class(document.document(), "after")
        .expect("following block");
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &document,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("two-level orthogonal layout");
    let vertical = layout
        .get(vertical)
        .expect("vertical fragment")
        .physical_rect();
    let line = layout.get(line).expect("line fragment").physical_rect();
    let after = layout
        .get(after)
        .expect("following fragment")
        .physical_rect();

    assert!(vertical.height > 0.0 && vertical.height < 40.0);
    assert_eq!(vertical.height, line.height);
    assert_eq!(vertical.width, line.width);
    assert_eq!(after.y, vertical.height);
}

#[test]
fn contained_root_keeps_body_writing_mode_local_to_body_content() {
    fn first_text(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.kind(node) == NodeKind::Text && dom.text(node).is_some_and(|text| !text.is_empty()) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| first_text(dom, child))
    }

    fn text_fragment_x(
        document: &StaticDocument,
        css: &str,
    ) -> (
        f32,
        Fragment,
        FlowAxes,
        Option<FlowAxes>,
        (CssSize, CssSize),
    ) {
        let styles = resolve_styles(
            document,
            &StyleSet::cambium(&[css]),
            &Device::screen(800.0, 600.0),
            &InteractionStates::default(),
        );
        let mut text = TextSystem::new();
        let (_, layout) = layout_with_text_system(
            document,
            &styles,
            800.0,
            600.0,
            ViewportSizes::uniform(800.0, 600.0),
            &mut text,
            &HashMap::new(),
        )
        .expect("writing-mode layout");
        let source = first_text(document, document.document()).expect("text node");
        let text_x = layout
            .fragments_for_node(source)
            .next()
            .expect("text fragment")
            .physical_rect()
            .x;
        let parent = document.parent(source).expect("text parent");
        let parent = layout.get(parent).expect("parent fragment").physical_rect();
        let parent_box = layout
            .boxes()
            .principal_box(document.parent(source).expect("text parent"))
            .expect("parent box");
        let flow = layout.boxes()[parent_box].flow;
        let containing_flow = layout.boxes()[parent_box]
            .parent()
            .map(|containing| layout.boxes()[containing].flow);
        let style = styles
            .get(document.parent(source).expect("text parent"))
            .expect("parent style");
        (
            text_x,
            parent,
            flow,
            containing_flow,
            (style.width, style.height),
        )
    }

    let target = StaticDocument::parse(
        "<html><body>This text should run vertically on the left side</body></html>",
    );
    let reference = StaticDocument::parse(
        "<html><body><div>This text should run vertically on the left side</div></body></html>",
    );
    let target_x = text_fragment_x(
        &target,
        "html { contain: paint; } body { writing-mode: vertical-rl; }",
    );
    let reference_x = text_fragment_x(&reference, "div { writing-mode: vertical-rl; }");

    assert_eq!(
        target_x.0, reference_x.0,
        "target={target_x:?} reference={reference_x:?}"
    );
}

#[test]
fn replaced_html_dimensions_use_computed_css_and_canvas_intrinsics() {
    fn find_by_name(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        name: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom
            .element_name(node)
            .is_some_and(|element| element.local.as_ref() == name)
        {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| find_by_name(dom, child, name))
    }

    let dom = StaticDocument::parse(
        "<html><body><div><img width=\"100%\" height=\"3\">\
         <canvas width=\"100\" height=\"100\"></canvas></div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body { margin: 0; } div { position: relative; width: 200px; }\
             img { position: absolute; left: 0; top: 0; } canvas { display: block; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let image = find_by_name(&dom, dom.document(), "img").expect("img");
    assert_eq!(
        styles.get(image).unwrap().width,
        CssSize::Value(CssLengthPercentage::Percentage(1.0))
    );
    assert_eq!(
        styles.get(image).unwrap().height,
        CssSize::Value(CssLengthPercentage::Length(Length::px(3.0)))
    );
    let canvas = find_by_name(&dom, dom.document(), "canvas").expect("canvas");
    assert_eq!(styles.get(canvas).unwrap().width, CssSize::Auto);
    assert_eq!(styles.get(canvas).unwrap().height, CssSize::Auto);
    assert_eq!(
        styles.get(canvas).unwrap().aspect_ratio,
        livery::values::AspectRatio::AutoRatio {
            width: 100.0,
            height: 100.0,
        },
        "canvas dimensions remain natural-size inputs rather than CSS dimensions"
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");

    let image = layout.get(image).expect("image fragment").physical_rect();
    assert_eq!(
        (image.width, image.height),
        (200.0, 3.0),
        "the percentage hint resolves against the positioned containing block"
    );

    let canvas = layout.get(canvas).expect("canvas fragment").physical_rect();
    assert_eq!((canvas.width, canvas.height), (100.0, 100.0));
}

#[test]
fn positioned_replaced_leaf_keeps_its_hint_size_between_definite_insets() {
    fn find_by_name(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        name: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom
            .element_name(node)
            .is_some_and(|element| element.local.as_ref() == name)
        {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| find_by_name(dom, child, name))
    }

    let dom = StaticDocument::parse(
        "<html><body><div><canvas width=\"80\" height=\"40\"></canvas></div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body { margin: 0; } div { position: relative; width: 200px; } \
             canvas { position: absolute; left: 10px; right: 20px; top: 5px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");

    let canvas = find_by_name(&dom, dom.document(), "canvas").expect("canvas");
    let canvas = layout.get(canvas).expect("canvas fragment").physical_rect();
    assert_eq!(
        (canvas.x, canvas.y, canvas.width, canvas.height),
        (10.0, 5.0, 80.0, 40.0)
    );
}

#[test]
fn percentage_height_chain_uses_initial_containing_block_height() {
    fn find_by_name(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        name: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom
            .element_name(node)
            .is_some_and(|element| element.local.as_ref() == name)
        {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| find_by_name(dom, child, name))
    }

    let dom = StaticDocument::parse("<html><body><p>viewport</p></body></html>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, p { height: 100%; margin: 0; padding: 0; border: 0; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");

    for name in ["html", "body", "p"] {
        let node = find_by_name(&dom, dom.document(), name).expect(name);
        assert_eq!(
            layout.get(node).expect(name).physical_rect().height,
            240.0,
            "{name} should resolve 100% against a definite containing block"
        );
    }
    assert_eq!(layout.block_algorithm_counts().taffy, 0);
}

#[test]
fn live_block_flow_keeps_collapsed_margin_chains_in_buckram() {
    fn collect_divs(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        output: &mut Vec<<StaticDocument as LayoutDom>::NodeId>,
    ) {
        if dom
            .element_name(node)
            .is_some_and(|name| name.local.as_ref() == "div")
        {
            output.push(node);
        }
        for child in dom.dom_children(node) {
            collect_divs(dom, child, output);
        }
    }

    let dom = StaticDocument::parse(
        "<html><body><div class=\"host\">\
         <div class=\"parent\"><div class=\"child\"></div></div>\
         <div class=\"after\"></div>\
         <div class=\"chain\"><div class=\"first\"></div><div class=\"empty\"></div>\
         <div class=\"last\"></div></div>\
         </div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body, .host, .chain { margin: 0; padding: 0; border: 0; }\
             .parent { margin: 10px 0 15px; }\
             .child { height: 20px; margin: 30px 0 40px; }\
             .after { height: 10px; margin: 12px 0 0; }\
             .first { height: 10px; margin: 0 0 20px; }\
             .empty { margin: -7px 0 12px; }\
             .last { height: 10px; margin: -15px 0 0; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let mut divs = Vec::new();
    collect_divs(&dom, dom.document(), &mut divs);
    let parent = layout.get(divs[1]).expect("parent").physical_rect();
    let child = layout.get(divs[2]).expect("child").physical_rect();
    let after = layout.get(divs[3]).expect("after").physical_rect();
    let first = layout.get(divs[5]).expect("first").physical_rect();
    let empty = layout.get(divs[6]).expect("empty").physical_rect();
    let last = layout.get(divs[7]).expect("last").physical_rect();
    let algorithms = layout.block_algorithm_counts();

    assert_eq!(child.y, parent.y);
    assert_eq!(after.y, parent.y + 60.0);
    assert_eq!(empty.y, first.y + 23.0);
    assert_eq!(last.y, first.y + 15.0);
    assert!(algorithms.buckram >= 6);
    assert_eq!(algorithms.taffy, 0);
}

#[test]
fn live_bfc_places_blockified_floats_and_direct_clearance_in_buckram() {
    fn by_class(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "class"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_class(dom, child, expected))
    }

    let dom = StaticDocument::parse(
        "<html><body><div class=\"host\">\
         <span class=\"left\"></span><div class=\"right\"></div>\
         <div class=\"clear\"></div></div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body, div, span { margin: 0; padding: 0; border: 0; }\
             .host { width: 200px; overflow-x: hidden; overflow-y: hidden; }\
             .left { float: left; width: 80px; height: 40px; }\
             .right { float: right; width: 60px; height: 70px; }\
             .clear { clear: both; height: 10px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let rect = |class| {
        let node = by_class(&dom, dom.document(), class).expect(class);
        layout.get(node).expect(class).physical_rect()
    };

    let host = rect("host");
    let left = rect("left");
    let right = rect("right");
    let clear = rect("clear");
    let algorithms = layout.block_algorithm_counts();

    assert_eq!((left.x, left.y), (host.x, host.y));
    assert_eq!((right.x, right.y), (host.x + 140.0, host.y));
    assert_eq!((clear.x, clear.y), (host.x, host.y + 70.0));
    assert_eq!(host.height, 80.0);
    assert!(algorithms.buckram >= 4);
    assert_eq!(algorithms.taffy, 0);
}

#[test]
fn live_empty_clearance_keeps_its_following_margin_chain_in_buckram() {
    fn by_id(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "id"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_id(dom, child, expected))
    }

    let dom = StaticDocument::parse(
        "<html><body><div id=\"host\">\
         <div id=\"float\"></div><div id=\"empty\"></div><div id=\"after\"></div>\
         </div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; border: 0; }\
             #host { width: 200px; overflow-x: hidden; overflow-y: hidden; }\
             #float { float: left; width: 80px; height: 40px; }\
             #empty { clear: left; margin-top: 10px; margin-bottom: 20px; }\
             #after { height: 10px; margin-top: 30px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let rect = |id| {
        let node = by_id(&dom, dom.document(), id).expect(id);
        layout.get(node).expect(id).physical_rect()
    };

    let host = rect("host");
    let float = rect("float");
    let empty = rect("empty");
    let after = rect("after");
    let algorithms = layout.block_algorithm_counts();

    assert_eq!((float.x - host.x, float.y - host.y), (0.0, 0.0));
    assert_eq!(
        (empty.y - host.y, empty.height),
        (40.0, 0.0),
        "host={host:?}, float={float:?}, empty={empty:?}, after={after:?}, algorithms={algorithms:?}"
    );
    assert_eq!((after.y - host.y, after.height), (70.0, 10.0));
    assert_eq!(host.height, 80.0);
    assert_eq!(algorithms.taffy, 0);
}

#[test]
fn live_inline_lines_in_an_ordinary_wrapper_share_outer_float_exclusions() {
    fn by_id(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "id"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_id(dom, child, expected))
    }

    let dom = StaticDocument::parse(
        "<html><body><div id=\"host\"><div id=\"float\"></div>\
         <div id=\"wrapper\"><span id=\"copy\">aa aa aa aa aa aa aa aa aa aa aa aa \
         aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa \
         aa aa</span></div>\
         </div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body, div, span { margin: 0; padding: 0; border: 0; }\
             #host { width: 200px; overflow-x: hidden; overflow-y: hidden;\
                     font-family: monospace; font-size: 10px; line-height: 20px; }\
             #float { float: left; width: 80px; height: 40px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let host = by_id(&dom, dom.document(), "host").expect("host");
    let copy = by_id(&dom, dom.document(), "copy").expect("copy");
    let host = layout.get(host).expect("host fragment").physical_rect();
    let algorithms = layout.block_algorithm_counts();
    let mut lines = layout
        .fragments_for_node(copy)
        .map(|fragment| fragment.physical_rect())
        .collect::<Vec<_>>();
    lines.sort_by(|left, right| left.y.total_cmp(&right.y));

    assert!(
        lines.len() >= 4,
        "fixture must produce several line fragments"
    );
    assert!(
        (lines[0].x - (host.x + 80.0)).abs() <= 0.5,
        "host={host:?}, lines={lines:?}, algorithms={algorithms:?}"
    );
    assert!(
        (lines[1].x - (host.x + 80.0)).abs() <= 0.5,
        "host={host:?}, lines={lines:?}, algorithms={algorithms:?}"
    );
    assert!(
        lines
            .iter()
            .filter(|line| line.y >= host.y + 40.0)
            .all(|line| (line.x - host.x).abs() <= 0.5),
        "lines below the float must use the full content column"
    );
    assert_eq!(algorithms.taffy, 0);
}

#[test]
fn live_shape_outside_reference_boxes_change_lines_but_not_float_placement() {
    fn by_id(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "id"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_id(dom, child, expected))
    }

    let hosts = ["none", "margin", "border", "padding", "content", "curved"];
    let markup = hosts
        .iter()
        .map(|name| {
            format!(
                "<div id=\"host-{name}\" class=\"host\"><div class=\"float {name}\"></div>\
                 <div><span id=\"copy-{name}\">aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa \
                 aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa</span></div></div>"
            )
        })
        .collect::<String>();
    let dom = StaticDocument::parse(&format!("<html><body>{markup}</body></html>"));
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body, div, span { margin: 0; padding: 0; border: 0; }\
             .host { width: 200px; overflow-x: hidden; overflow-y: hidden;\
                     font-family: monospace; font-size: 10px; line-height: 20px; }\
             .float { float: left; width: 50px; height: 80px; margin-right: 20px;\
                      padding-right: 10px; border-right: 20px solid; }\
             .margin { shape-outside: margin-box; }\
             .border { shape-outside: border-box; }\
             .padding { shape-outside: padding-box; }\
             .content { shape-outside: content-box; }\
             .curved { shape-outside: content-box; border-radius: 10px; }",
        ]),
        &Device::screen(320.0, 600.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        600.0,
        ViewportSizes::uniform(320.0, 600.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let algorithms = layout.block_algorithm_counts();

    for (name, expected_line_start) in [
        ("none", 100.0),
        ("margin", 100.0),
        ("border", 80.0),
        ("padding", 60.0),
        ("content", 50.0),
        ("curved", 50.0),
    ] {
        let host_node = by_id(&dom, dom.document(), &format!("host-{name}"))
            .unwrap_or_else(|| panic!("host-{name}"));
        let copy_node = by_id(&dom, dom.document(), &format!("copy-{name}"))
            .unwrap_or_else(|| panic!("copy-{name}"));
        let host = layout
            .get(host_node)
            .unwrap_or_else(|| panic!("host-{name} fragment"))
            .physical_rect();
        let float_node = dom
            .dom_children(host_node)
            .next()
            .unwrap_or_else(|| panic!("float-{name}"));
        let float = layout
            .get(float_node)
            .unwrap_or_else(|| panic!("float-{name} fragment"))
            .physical_rect();
        let first_line = layout
            .fragments_for_node(copy_node)
            .map(|fragment| fragment.physical_rect())
            .min_by(|left, right| left.y.total_cmp(&right.y))
            .unwrap_or_else(|| panic!("copy-{name} line"));

        assert_eq!((float.x - host.x, float.y - host.y), (0.0, 0.0));
        assert!(
            (first_line.x - host.x - expected_line_start).abs() <= 0.5,
            "name={name}, host={host:?}, float={float:?}, line={first_line:?}"
        );
        assert!(host.height >= 80.0);
    }
    assert_eq!(algorithms.taffy, 0);
}

#[test]
fn live_shape_outside_keeps_forced_break_lines_in_the_selected_float_band() {
    let dom = StaticDocument::parse(
        "<html><body><div id=\"container\"><div id=\"host\"><div id=\"shape\"></div>\
         <br><br>\n            X\n</div></div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["body { margin: 0; }\
             #container { position: relative; }\
             #host { width: 300px; height: 200px; font-family: monospace;\
                     font-size: 40px; line-height: 40px; }\
             #shape { float: left; width: 150px; height: 150px; margin: 10px;\
                      padding: 10px; border: 10px solid transparent;\
                      shape-outside: border-box; }"]),
        &Device::screen(400.0, 300.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        400.0,
        300.0,
        ViewportSizes::uniform(400.0, 300.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let host = node_by_id(&dom, dom.document(), "host").expect("host");
    let copy = dom
        .dom_children(host)
        .find(|node| dom.text(*node).is_some_and(|text| text.contains('X')))
        .expect("direct text");
    let host = layout.get(host).expect("host fragment").physical_rect();
    let copy = layout
        .fragments_for_node(copy)
        .map(|fragment| fragment.physical_rect())
        .max_by(|left, right| left.width.total_cmp(&right.width))
        .expect("copy line");

    assert!(
        (copy.x - host.x - 200.0).abs() <= 0.5,
        "forced breaks must retain the border-box line origin: host={host:?}, copy={copy:?}"
    );
    assert!(
        (copy.y - host.y - 80.0).abs() <= 0.5,
        "host={host:?}, copy={copy:?}"
    );
    assert_eq!(layout.block_algorithm_counts().taffy, 0);
}

#[test]
fn relative_zero_height_wrapper_retains_its_floated_descendant() {
    let dom = StaticDocument::parse(
        "<html><body><div id=\"outer\"><div id=\"float\"><div></div></div>\
         <div id=\"absolute\"></div></div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["body { margin: 0; }\
             #outer { position: relative; }\
             #float { float: left; }\
             #float > div, #absolute { width: 96px; height: 96px; }\
             #absolute { position: absolute; left: 96px; top: 0; }"]),
        &Device::screen(400.0, 300.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        400.0,
        300.0,
        ViewportSizes::uniform(400.0, 300.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let outer = node_by_id(&dom, dom.document(), "outer").expect("outer");
    let float = node_by_id(&dom, dom.document(), "float").expect("float");
    let absolute = node_by_id(&dom, dom.document(), "absolute").expect("absolute");
    let outer = layout.get(outer).expect("outer fragment").physical_rect();
    let float = layout.get(float).expect("float fragment").physical_rect();
    let absolute = layout
        .get(absolute)
        .expect("absolute fragment")
        .physical_rect();

    assert_eq!(
        (float.x, float.y, float.width, float.height),
        (outer.x, outer.y, 96.0, 96.0)
    );
    assert_eq!((absolute.x, absolute.y), (outer.x + 96.0, outer.y));
}

#[test]
fn live_rounded_shape_boxes_shift_left_and_right_line_edges() {
    fn by_id(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "id"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_id(dom, child, expected))
    }

    let dom = StaticDocument::parse(
        "<html><body>\
         <div id=\"left\" class=\"host\"><div id=\"left-float\" class=\"shape left\"></div>\
         <div><span id=\"left-copy\">aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa</span></div></div>\
         <div id=\"right\" class=\"host right-host\"><div id=\"right-float\" class=\"shape right\"></div>\
         <div><span id=\"right-copy\">aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa</span></div></div>\
         </body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body, div, span { margin: 0; padding: 0; border: 0; }\
             .host { width: 200px; overflow-x: hidden; overflow-y: hidden;\
                     font-family: monospace; font-size: 10px; line-height: 20px; }\
             .shape { width: 80px; height: 80px; shape-outside: border-box; border-radius: 50%; }\
             .left { float: left; }\
             .right { float: right; }\
             .right-host { direction: rtl; text-align: right; }",
        ]),
        &Device::screen(320.0, 300.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        300.0,
        ViewportSizes::uniform(320.0, 300.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let rect = |id| {
        let node = by_id(&dom, dom.document(), id).expect(id);
        layout.get(node).expect(id).physical_rect()
    };
    let first_line = |id| {
        let node = by_id(&dom, dom.document(), id).expect(id);
        layout
            .fragments_for_node(node)
            .map(|fragment| fragment.physical_rect())
            .min_by(|left, right| left.y.total_cmp(&right.y))
            .expect(id)
    };

    let left = rect("left-float");
    let left_line = first_line("left-copy");
    assert!(
        left_line.x > left.x + 50.0 && left_line.x < left.x + 80.0,
        "a rounded left float releases its top-corner interval: host={left:?}, line={left_line:?}"
    );

    let right = rect("right-float");
    let right_line = first_line("right-copy");
    assert!(
        right_line.x + right_line.width > right.x + 5.0
            && right_line.x + right_line.width < right.x + 15.0,
        "a rounded right float releases its top-corner interval: float={right:?}, line={right_line:?}"
    );
    assert_eq!(layout.block_algorithm_counts().taffy, 0);
}

#[test]
fn horizontal_direction_changes_keep_shape_constraints_for_atomic_lines() {
    let dom = StaticDocument::parse(
        r#"<html><body><div id=host><div id=shape></div>
         <div id=a class=box></div> <div id=b class=box></div>
         <div id=c class='box tall'></div> <div id=d class='box tall'></div>
         <div id=e class=box></div> <div id=f class=box></div></div></body></html>"#,
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body { margin: 0; }\
             #host { direction: rtl; width: 200px; line-height: 0; }\
             #shape { float: right; shape-outside: margin-box; border-radius: 50%;\
                      width: 20px; height: 20px; padding: 20px; border: 20px solid;\
                      margin: 10px; }\
             .box { display: inline-block; width: 60px; height: 12px; }\
             .tall { height: 36px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let host = node_by_id(&dom, dom.document(), "host").expect("host");
    let host = layout.get(host).expect("host fragment").physical_rect();
    let actual = ["a", "b", "c", "d", "e", "f"].map(|id| {
        let node = node_by_id(&dom, dom.document(), id).unwrap_or_else(|| panic!("{id}"));
        let rect = layout
            .get(node)
            .unwrap_or_else(|| panic!("{id} fragment"))
            .physical_rect();
        (rect.x - host.x, rect.y - host.y, rect.height)
    });

    assert_eq!(
        actual,
        [
            (44.0, 0.0, 12.0),
            (32.0, 12.0, 12.0),
            (20.0, 24.0, 36.0),
            (20.0, 60.0, 36.0),
            (32.0, 96.0, 12.0),
            (44.0, 108.0, 12.0),
        ]
    );
    assert_eq!(layout.block_algorithm_counts().taffy, 0);
}

#[test]
fn nonlinear_corner_radius_falls_back_only_when_shape_outside_consumes_it() {
    let dom = StaticDocument::parse(
        "<html><body><div id=\"paint\"></div><div id=\"shape\"></div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["#paint, #shape { width: 100px; height: 100px;\
                              border-radius: min(10px, 50%); }\
             #shape { float: left; shape-outside: border-box; }"]),
        &Device::screen(320.0, 300.0),
        &InteractionStates::default(),
    );
    let paint = node_by_id(&dom, dom.document(), "paint").expect("paint");
    let shape = node_by_id(&dom, dom.document(), "shape").expect("shape");
    let paint = styles.get(paint).expect("paint style");
    let shape = styles.get(shape).expect("shape style");

    assert!(length_has_math(shape.border_top_left_radius.0));
    assert!(!shape_outside_has_nonlinear_radius(paint));
    assert!(shape_outside_has_nonlinear_radius(shape));
    assert!(!block_style_has_nonlinear_lengths(paint));
    assert!(!block_style_has_nonlinear_lengths(shape));
}

#[test]
fn live_nonlinear_shape_radius_retains_buckram_and_the_default_margin_area() {
    let dom = StaticDocument::parse(
        "<html><body><div id=\"host\"><div id=\"shape\"></div>\
         <div><span id=\"copy\">aa aa aa aa aa aa aa aa</span></div></div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body, div, span { margin: 0; padding: 0; border: 0; }\
             #host { width: 200px; overflow-x: hidden; overflow-y: hidden;\
                     font-family: monospace; font-size: 10px; line-height: 20px; }\
             #shape { float: left; width: 80px; height: 80px; margin: 10px;\
                      shape-outside: border-box; border-radius: min(10px, 50%); }",
        ]),
        &Device::screen(320.0, 300.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        300.0,
        ViewportSizes::uniform(320.0, 300.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let shape = node_by_id(&dom, dom.document(), "shape").expect("shape");
    let copy = node_by_id(&dom, dom.document(), "copy").expect("copy");
    let shape = layout.get(shape).expect("shape layout").physical_rect();
    let line = layout
        .fragments_for_node(copy)
        .map(|fragment| fragment.physical_rect())
        .min_by(|left, right| left.y.total_cmp(&right.y))
        .expect("copy line");

    assert!((line.x - (shape.x + shape.width + 10.0)).abs() <= 0.01);
    assert_eq!(layout.block_algorithm_counts().taffy, 0);
}

#[test]
fn live_unbreakable_line_retries_inside_a_rounded_bottom_contour() {
    let dom = StaticDocument::parse(
        "<html><body><div id=\"host\"><div id=\"shape\"></div>\
         <div><span id=\"copy\">aaaaaaaaaaaaaaaaaaaa</span></div></div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body, div, span { margin: 0; padding: 0; border: 0; }\
             #host { width: 200px; overflow-x: hidden; overflow-y: hidden;\
                     font-family: monospace; font-size: 10px; line-height: 20px; }\
             #shape { float: left; width: 100px; height: 100px;\
                      shape-outside: border-box; border-radius: 0 0 50% 50%; }\
             #copy { white-space: nowrap; }",
        ]),
        &Device::screen(320.0, 300.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        300.0,
        ViewportSizes::uniform(320.0, 300.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let shape = node_by_id(&dom, dom.document(), "shape").expect("shape");
    let copy = node_by_id(&dom, dom.document(), "copy").expect("copy");
    let shape = layout.get(shape).expect("shape layout").physical_rect();
    let line = layout
        .fragments_for_node(copy)
        .map(|fragment| fragment.physical_rect())
        .min_by(|left, right| left.y.total_cmp(&right.y))
        .expect("copy line");

    assert!(line.width > 100.0 && line.width < 150.0, "line={line:?}");
    assert!(
        line.y > shape.y + 50.0 && line.y < shape.y + shape.height,
        "the line should fit within the widening bottom contour: shape={shape:?}, line={line:?}"
    );
    assert_eq!(layout.block_algorithm_counts().taffy, 0);
}

#[test]
fn live_nowrap_nested_inline_content_uses_float_bands_in_both_directions() {
    fn by_id(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "id"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_id(dom, child, expected))
    }

    let dom = StaticDocument::parse(
        "<html><body>\
         <div id=\"ltr\" class=\"host\"><div class=\"float\"></div>\
         <span id=\"ltr-copy\"><span><span>aa aa aa aa</span></span></span></div>\
         <div id=\"rtl\" class=\"host\"><div class=\"float\"></div>\
         <span id=\"rtl-copy\"><span><span>aa aa aa aa</span></span></span></div>\
         </body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body, div, span { margin: 0; padding: 0; border: 0; }\
             .host { width: 200px; overflow-x: hidden; overflow-y: hidden;\
                     white-space: nowrap; font-family: monospace; font-size: 10px;\
                     line-height: 20px; }\
             .float { float: left; width: 80px; height: 40px; }\
             #rtl { direction: rtl; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let rect = |id| {
        let node = by_id(&dom, dom.document(), id).expect(id);
        layout.get(node).expect(id).physical_rect()
    };
    let copy_lines = |id| {
        let node = by_id(&dom, dom.document(), id).expect(id);
        layout
            .fragments_for_node(node)
            .map(|fragment| fragment.physical_rect())
            .collect::<Vec<_>>()
    };

    let ltr = rect("ltr");
    let rtl = rect("rtl");
    let ltr_lines = copy_lines("ltr-copy");
    let rtl_lines = copy_lines("rtl-copy");
    let algorithms = layout.block_algorithm_counts();

    assert_eq!(ltr_lines.len(), 1, "nowrap must remain one line");
    assert_eq!(rtl_lines.len(), 1, "nowrap must remain one line");
    assert_eq!((ltr_lines[0].x, ltr_lines[0].y), (ltr.x + 80.0, ltr.y));
    assert!(
        rtl_lines[0].x >= rtl.x + 80.0 - 0.5
            && rtl_lines[0].x + rtl_lines[0].width <= rtl.x + rtl.width + 0.5
            && (rtl_lines[0].y - rtl.y).abs() <= 0.5,
        "rtl host={rtl:?}, lines={rtl_lines:?}"
    );
    assert_eq!(algorithms.taffy, 0);
}

#[test]
fn live_nested_float_state_crosses_ordinary_wrappers_but_stops_at_bfcs() {
    fn by_id(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "id"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_id(dom, child, expected))
    }

    let dom = StaticDocument::parse(
        "<html><body>\
         <div id=\"shared\" class=\"host\"><div id=\"wrapper\"><div class=\"float\"></div></div>\
         <div id=\"shared-clear\" class=\"clear\"></div></div>\
         <div id=\"isolated\" class=\"host\"><div id=\"boundary\"><div class=\"float\"></div></div>\
         <div id=\"isolated-clear\" class=\"clear\"></div></div>\
         </body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; border: 0; }\
             .host { width: 200px; overflow-x: hidden; overflow-y: hidden; }\
             .float { float: left; width: 80px; height: 40px; }\
             .clear { clear: left; height: 10px; }\
             #boundary { display: flow-root; height: 0; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let rect = |id| {
        let node = by_id(&dom, dom.document(), id).expect(id);
        layout.get(node).expect(id).physical_rect()
    };

    let shared = rect("shared");
    let wrapper = rect("wrapper");
    let shared_clear = rect("shared-clear");
    let isolated = rect("isolated");
    let boundary = rect("boundary");
    let isolated_clear = rect("isolated-clear");
    let algorithms = layout.block_algorithm_counts();

    assert_eq!(wrapper.height, 0.0);
    assert_eq!(shared_clear.y - shared.y, 40.0);
    assert_eq!(shared.height, 50.0);
    assert_eq!(boundary.height, 0.0);
    assert_eq!(isolated_clear.y, isolated.y);
    assert_eq!(isolated.height, 10.0);
    assert_eq!(algorithms.taffy, 0);
}

#[test]
fn live_generated_block_roots_translate_nested_float_state() {
    fn by_id(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "id"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_id(dom, child, expected))
    }

    let dom = StaticDocument::parse(
        "<html><body><div id=\"host\"><div id=\"outer\"><div id=\"middle\">\
         <div id=\"float\"></div></div></div><div id=\"clear\"></div></div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; border: 0; }\
             #host { width: 200px; overflow-x: hidden; overflow-y: hidden; }\
             #outer { margin-top: 10px; }\
             #middle { margin-top: 20px; padding-top: 5px; border-top: 3px solid; }\
             #float { float: left; width: 80px; height: 40px; }\
             #clear { clear: left; height: 10px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let rect = |id| {
        let node = by_id(&dom, dom.document(), id).expect(id);
        layout.get(node).expect(id).physical_rect()
    };

    let host = rect("host");
    let outer = rect("outer");
    let middle = rect("middle");
    let float = rect("float");
    let clear = rect("clear");
    let algorithms = layout.block_algorithm_counts();

    assert_eq!((outer.y - host.y, middle.y - host.y), (20.0, 20.0));
    assert_eq!(float.y - host.y, 28.0);
    assert_eq!(clear.y - host.y, 68.0);
    assert_eq!(host.height, 78.0);
    assert_eq!(algorithms.taffy, 0);
}

#[test]
fn livery_box_tree_preserves_split_inline_float_provenance() {
    fn by_id(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "id"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_id(dom, child, expected))
    }

    let dom = StaticDocument::parse(
        "<html><body><div id=\"host\"><div id=\"wrapper\"><span id=\"split\">before\
         <span id=\"inline-float\"></span><span id=\"block\"></span>after</span></div>\
         <div id=\"clear\"></div></div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body, div, span { margin: 0; padding: 0; border: 0; }\
             #host { width: 200px; overflow-x: hidden; overflow-y: hidden; }\
             #inline-float { float: left; width: 80px; height: 40px; }\
             #block { display: block; height: 0; }\
             #clear { clear: left; height: 10px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let split = by_id(&dom, dom.document(), "split").expect("split");
    let inline_float = by_id(&dom, dom.document(), "inline-float").expect("inline float");
    let boxes = GeneratedBoxTree::from_dom(&dom, &styles);
    let float_box = boxes.principal_box(inline_float).expect("float box");

    assert_eq!(boxes.boxes_for_node(split).len(), 2);
    assert_eq!(
        boxes[float_box].float_context,
        FloatContextProvenance::Inline
    );
}

#[test]
fn live_block_bfcs_narrow_beside_a_float_or_move_below_it() {
    fn by_id(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "id"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_id(dom, child, expected))
    }

    let dom = StaticDocument::parse(
        "<html><body><div id=\"host\"><div id=\"float\"></div>\
         <div id=\"adjacent\"></div><div id=\"lowered\"></div>\
         </div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; border: 0; }\
             #host { width: 200px; overflow-x: hidden; overflow-y: hidden; }\
             #float { float: left; width: 80px; height: 40px; }\
             #adjacent { height: 20px; overflow-x: hidden; overflow-y: hidden; }\
             #lowered { width: 150px; height: 20px;\
                        overflow-x: hidden; overflow-y: hidden; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let rect = |id| {
        let node = by_id(&dom, dom.document(), id).expect(id);
        layout.get(node).expect(id).physical_rect()
    };

    let host = rect("host");
    let adjacent = rect("adjacent");
    let lowered = rect("lowered");
    let algorithms = layout.block_algorithm_counts();

    assert_eq!(
        (adjacent.x, adjacent.y, adjacent.width, adjacent.height),
        (host.x + 80.0, host.y, 120.0, 20.0)
    );
    assert_eq!(
        (lowered.x, lowered.y, lowered.width, lowered.height),
        (host.x, host.y + 40.0, 150.0, 20.0)
    );
    assert_eq!(host.height, 60.0);
    assert_eq!(algorithms.taffy, 0);
}

#[test]
fn live_bfc_auto_margins_fit_or_move_below_floats_in_both_directions() {
    fn by_id(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "id"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_id(dom, child, expected))
    }

    let dom = StaticDocument::parse(
        "<html><body>\
         <div id=\"ltr\" class=\"host\"><div class=\"right-float\"></div>\
         <div id=\"ltr-bfc\" class=\"bfc\"></div></div>\
         <div id=\"rtl\" class=\"host\"><div class=\"left-float\"></div>\
         <div id=\"rtl-bfc\" class=\"bfc\"></div></div>\
         <div id=\"lowered\" class=\"host\"><div class=\"right-float\"></div>\
         <div id=\"lowered-bfc\" class=\"bfc\"></div></div>\
         </body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; border: 0; }\
             .host { width: 100px; overflow-x: hidden; overflow-y: hidden; }\
             .right-float { float: right; width: 50px; height: 40px; }\
             .left-float { float: left; width: 50px; height: 40px; }\
             .bfc { display: flow-root; width: 30px; height: 20px; }\
             #ltr-bfc { margin-left: auto; }\
             #rtl { direction: rtl; } #rtl-bfc { margin-right: auto; }\
             #lowered-bfc { width: 60px; margin-left: auto; margin-right: 10px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let rect = |id| {
        let node = by_id(&dom, dom.document(), id).expect(id);
        layout.get(node).expect(id).physical_rect()
    };

    let ltr = rect("ltr");
    let ltr_bfc = rect("ltr-bfc");
    let rtl = rect("rtl");
    let rtl_bfc = rect("rtl-bfc");
    let lowered = rect("lowered");
    let lowered_bfc = rect("lowered-bfc");
    let algorithms = layout.block_algorithm_counts();

    assert_eq!(
        (
            ltr_bfc.x - ltr.x,
            ltr_bfc.y - ltr.y,
            ltr_bfc.width,
            ltr_bfc.height,
        ),
        (20.0, 0.0, 30.0, 20.0),
        "ltr={ltr:?}, ltr_bfc={ltr_bfc:?}, rtl={rtl:?}, rtl_bfc={rtl_bfc:?}, lowered={lowered:?}, lowered_bfc={lowered_bfc:?}, algorithms={algorithms:?}"
    );
    assert_eq!(
        (
            rtl_bfc.x - rtl.x,
            rtl_bfc.y - rtl.y,
            rtl_bfc.width,
            rtl_bfc.height,
        ),
        (50.0, 0.0, 30.0, 20.0)
    );
    assert_eq!(
        (
            lowered_bfc.x - lowered.x,
            lowered_bfc.y - lowered.y,
            lowered_bfc.width,
            lowered_bfc.height,
        ),
        (30.0, 40.0, 60.0, 20.0)
    );
    assert_eq!((ltr.height, rtl.height, lowered.height), (40.0, 40.0, 60.0));
    assert_eq!(algorithms.taffy, 0);
}

#[test]
fn live_flex_and_grid_bfcs_use_buckram_float_placement() {
    fn by_id(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "id"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_id(dom, child, expected))
    }

    let dom = StaticDocument::parse(
        "<html><body><div id=\"host\"><div id=\"float\"></div>\
         <div id=\"flex\"><div id=\"flex-child\"></div></div>\
         <div id=\"grid\"><div id=\"grid-child\"></div></div>\
         </div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; border: 0; }\
             #host { width: 100px; overflow-x: hidden; overflow-y: hidden; }\
             #float { float: left; width: 40px; height: 40px; }\
             #flex { display: flex; height: 20px; }\
             #flex-child { width: 20px; height: 10px; }\
             #grid { display: grid; grid-template-columns: 20px; width: 70px; height: 20px; }\
             #grid-child { width: 20px; height: 10px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let rect = |id| {
        let node = by_id(&dom, dom.document(), id).expect(id);
        layout.get(node).expect(id).physical_rect()
    };

    let host = rect("host");
    let flex = rect("flex");
    let flex_child = rect("flex-child");
    let grid = rect("grid");
    let grid_child = rect("grid-child");
    let algorithms = layout.block_algorithm_counts();

    assert_eq!(
        (
            flex.x - host.x,
            flex.y - host.y,
            flex.width,
            flex.height,
            flex_child.x - flex.x,
            flex_child.y - flex.y,
        ),
        (40.0, 0.0, 60.0, 20.0, 0.0, 0.0),
        "host={host:?}, flex={flex:?}, flex_child={flex_child:?}, grid={grid:?}, grid_child={grid_child:?}, algorithms={algorithms:?}"
    );
    assert_eq!(
        (
            grid.x - host.x,
            grid.y - host.y,
            grid.width,
            grid.height,
            grid_child.x - grid.x,
            grid_child.y - grid.y,
        ),
        (0.0, 40.0, 70.0, 20.0, 0.0, 0.0)
    );
    assert_eq!(host.height, 60.0);
    assert!(algorithms.buckram > 0);
    assert_eq!(algorithms.taffy, 0);
}

#[test]
fn live_auto_float_width_clamps_retained_inline_intrinsics() {
    fn by_id(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "id"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_id(dom, child, expected))
    }

    let dom = StaticDocument::parse(
        "<html><body>\
         <div id=\"narrow\" class=\"host\"><span id=\"narrow-float\" class=\"float\">\
         aaaa aaaa aaaa aaaa</span><div class=\"clear\"></div></div>\
         <div id=\"wide\" class=\"host\"><span id=\"wide-float\" class=\"float\">\
         aaaa aaaa aaaa aaaa</span><div class=\"clear\"></div></div>\
         </body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body, div, span { margin: 0; padding: 0; border: 0; }\
             .host { overflow-x: hidden; overflow-y: hidden; }\
             #narrow { width: 80px; } #wide { width: 200px; }\
             .float { float: left; font-family: monospace; font-size: 10px;\
                      line-height: 20px; }\
             .clear { clear: both; height: 1px; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let rect = |id| {
        let node = by_id(&dom, dom.document(), id).expect(id);
        layout.get(node).expect(id).physical_rect()
    };

    let narrow_host = rect("narrow");
    let narrow_float = rect("narrow-float");
    let wide_host = rect("wide");
    let wide_float = rect("wide-float");
    let algorithms = layout.block_algorithm_counts();

    assert!((narrow_float.width - narrow_host.width).abs() <= 0.5);
    assert!(
        wide_float.width > narrow_float.width + 10.0 && wide_float.width < wide_host.width - 10.0,
        "narrow={narrow_float:?}, wide={wide_float:?}"
    );
    assert!(narrow_float.height > wide_float.height);
    assert_eq!(algorithms.taffy, 0);
}

#[test]
fn live_multi_child_float_and_atomic_inline_use_intrinsic_subtrees() {
    fn by_id(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "id"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_id(dom, child, expected))
    }

    let dom = StaticDocument::parse(
        "<html><body>\
         <div id=\"narrow\" class=\"host\"><div id=\"narrow-float\" class=\"float\">\
         <div>aaaa aaaa aaaa aaaa</div><div>aaaa aaaa aaaa aaaa</div></div>\
         <div class=\"clear\"></div></div>\
         <div id=\"wide\" class=\"host\"><div id=\"wide-float\" class=\"float\">\
         <div>aaaa aaaa aaaa aaaa</div><div>aaaa aaaa aaaa aaaa</div></div>\
         <div class=\"clear\"></div></div>\
         </body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; border: 0; }\
             .host { overflow-x: hidden; overflow-y: hidden; }\
             #narrow { width: 80px; } #wide { width: 200px; }\
             .float { float: left; font-family: monospace; font-size: 10px;\
                      line-height: 20px; }\
             .clear { clear: both; height: 1px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let rect = |id| {
        let node = by_id(&dom, dom.document(), id).expect(id);
        layout.get(node).expect(id).physical_rect()
    };

    let narrow_host = rect("narrow");
    let narrow_float = rect("narrow-float");
    let wide_host = rect("wide");
    let wide_float = rect("wide-float");

    assert!((narrow_float.width - narrow_host.width).abs() <= 0.5);
    assert!(
        wide_float.width > narrow_float.width + 10.0 && wide_float.width < wide_host.width - 10.0,
        "narrow={narrow_float:?}, wide={wide_float:?}"
    );
    assert!(narrow_float.height > wide_float.height);
    assert_eq!(layout.block_algorithm_counts().taffy, 0);

    fn atomic_inline_width(viewport_width: f32) -> f32 {
        let dom = StaticDocument::parse(
            "<html><body><span id=\"atomic\">aaaa aaaa aaaa aaaa</span></body></html>",
        );
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&["html, body, span { margin: 0; padding: 0; border: 0; }\
                 span { display: inline-block; font-family: monospace; font-size: 10px;\
                        line-height: 20px; }"]),
            &Device::screen(viewport_width, 240.0),
            &InteractionStates::default(),
        );
        let mut text = TextSystem::new();
        let (_, layout) = layout_with_text_system(
            &dom,
            &styles,
            viewport_width,
            240.0,
            ViewportSizes::uniform(viewport_width, 240.0),
            &mut text,
            &HashMap::new(),
        )
        .expect("atomic inline layout");
        let atomic = by_id(&dom, dom.document(), "atomic").expect("atomic node");
        layout
            .get(atomic)
            .expect("atomic fragment")
            .physical_rect()
            .width
    }

    assert_eq!(atomic_inline_width(30.0), 30.0);
    assert_eq!(atomic_inline_width(80.0), 80.0);
    assert!((atomic_inline_width(200.0) - 104.462_89).abs() <= 0.01);
}

#[test]
fn live_bfc_fragments_expose_text_flex_grid_and_atomic_baselines() {
    fn by_id(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        expected: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.attributes(node).any(|attribute| {
            attribute.name.ns.as_ref().is_empty()
                && attribute.name.local.as_ref() == "id"
                && attribute.value == expected
        }) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| by_id(dom, child, expected))
    }

    let dom = StaticDocument::parse(
        "<html><body><div id=\"host\"><span id=\"text\">text</span>\
         <span id=\"atomic\"></span><div id=\"flex\"><span id=\"flex-text\">flex</span></div>\
         <div id=\"grid\"><span id=\"grid-text\">grid</span></div></div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body, div, span { margin: 0; padding: 0; border: 0; }\
             #host { width: 160px; font-family: monospace; font-size: 10px; line-height: 20px; }\
             #atomic { display: inline-block; width: 20px; height: 12px; }\
             #flex { display: flex; width: 80px; }\
             #grid { display: grid; width: 80px; grid-template-columns: 1fr; }",
        ]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    let fragment = |id| {
        layout
            .get(by_id(&dom, dom.document(), id).expect(id))
            .expect(id)
    };
    let host = fragment("host");
    let text = fragment("text");
    let atomic = fragment("atomic");
    let flex = fragment("flex");
    let grid = fragment("grid");

    for (name, fragment) in [
        ("host", host),
        ("text", text),
        ("atomic", atomic),
        ("flex", flex),
        ("grid", grid),
    ] {
        assert!(
            fragment.baselines.first.is_some() && fragment.baselines.last.is_some(),
            "{name} must expose modeled first and last baselines"
        );
    }
    assert_eq!(
        atomic.baselines,
        Baselines::synthesized_from_block_end(atomic.physical_rect().height),
        "an admitted atomic context keeps its own block-end fallback"
    );
    assert!(
        host.baselines.first.expect("host first baseline") < host.logical_rect.block_size,
        "the independent host keeps its IFC first baseline instead of its block-end fallback"
    );
    assert!(
        host.baselines.first.expect("host first baseline")
            >= text.baselines.first.expect("text baseline"),
        "the host baseline must retain the text IFC contribution"
    );
    assert_eq!(
        flex.baselines,
        Baselines::synthesized_from_block_end(flex.physical_rect().height),
        "the admitted flex BFC returns its own empty-line fallback"
    );
    assert_eq!(
        grid.baselines,
        Baselines::synthesized_from_block_end(grid.physical_rect().height),
        "the admitted grid BFC returns its own empty-line fallback"
    );
    assert_eq!(
        host.baselines.last,
        grid.baselines
            .last
            .map(|baseline| { grid.physical_rect().y - host.physical_rect().y + baseline }),
        "the independent host consumes its admitted grid BFC output"
    );
}

#[test]
fn flex_axis_projects_logical_direction_matrix_to_taffy() {
    use buckram::{Direction, WritingMode};

    let reverse = |direction| match direction {
        FlexDirection::Row => FlexDirection::RowReverse,
        FlexDirection::RowReverse => FlexDirection::Row,
        FlexDirection::Column => FlexDirection::ColumnReverse,
        FlexDirection::ColumnReverse => FlexDirection::Column,
    };
    let cases = [
        (
            FlowAxes::new(WritingMode::HorizontalTb, Direction::Ltr),
            FlexDirection::Row,
            FlexDirection::Column,
        ),
        (
            FlowAxes::new(WritingMode::HorizontalTb, Direction::Rtl),
            FlexDirection::RowReverse,
            FlexDirection::Column,
        ),
        (
            FlowAxes::new(WritingMode::VerticalRl, Direction::Ltr),
            FlexDirection::Column,
            FlexDirection::RowReverse,
        ),
        (
            FlowAxes::new(WritingMode::VerticalRl, Direction::Rtl),
            FlexDirection::ColumnReverse,
            FlexDirection::RowReverse,
        ),
        (
            FlowAxes::new(WritingMode::VerticalLr, Direction::Ltr),
            FlexDirection::Column,
            FlexDirection::Row,
        ),
        (
            FlowAxes::new(WritingMode::VerticalLr, Direction::Rtl),
            FlexDirection::ColumnReverse,
            FlexDirection::Row,
        ),
        (
            FlowAxes::new(WritingMode::SidewaysRl, Direction::Ltr),
            FlexDirection::Column,
            FlexDirection::RowReverse,
        ),
        (
            FlowAxes::new(WritingMode::SidewaysRl, Direction::Rtl),
            FlexDirection::ColumnReverse,
            FlexDirection::RowReverse,
        ),
        (
            FlowAxes::new(WritingMode::SidewaysLr, Direction::Ltr),
            FlexDirection::ColumnReverse,
            FlexDirection::Row,
        ),
        (
            FlowAxes::new(WritingMode::SidewaysLr, Direction::Rtl),
            FlexDirection::Column,
            FlexDirection::Row,
        ),
    ];

    for (flow, row, column) in cases {
        assert_eq!(physical_flex_direction(CssFlexDirection::Row, flow), row);
        assert_eq!(
            physical_flex_direction(CssFlexDirection::RowReverse, flow),
            reverse(row)
        );
        assert_eq!(
            physical_flex_direction(CssFlexDirection::Column, flow),
            column
        );
        assert_eq!(
            physical_flex_direction(CssFlexDirection::ColumnReverse, flow),
            reverse(column)
        );
    }
}

#[test]
fn flex_justify_content_start_and_end_follow_the_logical_main_axis() {
    use buckram::{Direction, WritingMode};

    let opposite = |keyword| match keyword {
        AlignContentKeyword::Start => AlignContentKeyword::End,
        AlignContentKeyword::End => AlignContentKeyword::Start,
        _ => unreachable!("only start/end are expected"),
    };
    let cases = [
        (
            FlowAxes::new(WritingMode::HorizontalTb, Direction::Ltr),
            AlignContentKeyword::Start,
            AlignContentKeyword::Start,
        ),
        (
            FlowAxes::new(WritingMode::HorizontalTb, Direction::Rtl),
            AlignContentKeyword::End,
            AlignContentKeyword::Start,
        ),
        (
            FlowAxes::new(WritingMode::VerticalRl, Direction::Ltr),
            AlignContentKeyword::Start,
            AlignContentKeyword::End,
        ),
        (
            FlowAxes::new(WritingMode::VerticalRl, Direction::Rtl),
            AlignContentKeyword::End,
            AlignContentKeyword::End,
        ),
        (
            FlowAxes::new(WritingMode::VerticalLr, Direction::Ltr),
            AlignContentKeyword::Start,
            AlignContentKeyword::Start,
        ),
        (
            FlowAxes::new(WritingMode::VerticalLr, Direction::Rtl),
            AlignContentKeyword::End,
            AlignContentKeyword::Start,
        ),
        (
            FlowAxes::new(WritingMode::SidewaysRl, Direction::Ltr),
            AlignContentKeyword::Start,
            AlignContentKeyword::End,
        ),
        (
            FlowAxes::new(WritingMode::SidewaysRl, Direction::Rtl),
            AlignContentKeyword::End,
            AlignContentKeyword::End,
        ),
        (
            FlowAxes::new(WritingMode::SidewaysLr, Direction::Ltr),
            AlignContentKeyword::End,
            AlignContentKeyword::Start,
        ),
        (
            FlowAxes::new(WritingMode::SidewaysLr, Direction::Rtl),
            AlignContentKeyword::Start,
            AlignContentKeyword::Start,
        ),
    ];
    let mut computed = ComputedValues::default();
    computed.display = CssDisplay::Flex;

    for (flow, row_start, column_start) in cases {
        for (direction, expected_start) in [
            (CssFlexDirection::Row, row_start),
            (CssFlexDirection::RowReverse, row_start),
            (CssFlexDirection::Column, column_start),
            (CssFlexDirection::ColumnReverse, column_start),
        ] {
            computed.flex_direction = direction;
            computed.justify_content = CssAlignment::Normal;
            assert_eq!(
                physical_flex_justify_content(&computed, flow).keyword,
                AlignContentKeyword::FlexStart
            );
            computed.justify_content = CssAlignment::Start;
            assert_eq!(
                physical_flex_justify_content(&computed, flow).keyword,
                expected_start
            );
            computed.justify_content = CssAlignment::End;
            assert_eq!(
                physical_flex_justify_content(&computed, flow).keyword,
                opposite(expected_start)
            );
        }
    }
}

#[test]
fn vertical_flex_transposes_logical_gap_components() {
    use buckram::{Direction, WritingMode};

    let mut computed = ComputedValues::default();
    computed.display = CssDisplay::Flex;
    computed.row_gap = "7px".parse().expect("row gap");
    computed.column_gap = "11px".parse().expect("column gap");

    assert_eq!(
        physical_flex_gap(
            &computed,
            16.0,
            FlowAxes::new(WritingMode::HorizontalTb, Direction::Ltr)
        ),
        Size {
            width: length(11.0),
            height: length(7.0),
        }
    );
    assert_eq!(
        physical_flex_gap(
            &computed,
            16.0,
            FlowAxes::new(WritingMode::SidewaysLr, Direction::Ltr)
        ),
        Size {
            width: length(7.0),
            height: length(11.0),
        }
    );
}

#[test]
fn flex_cross_axis_projects_logical_start_and_wrap_reversal() {
    let cases = [
        (
            CssWritingMode::VerticalRl,
            CssDirection::Ltr,
            TaffyDirection::Rtl,
        ),
        (
            CssWritingMode::VerticalRl,
            CssDirection::Rtl,
            TaffyDirection::Rtl,
        ),
        (
            CssWritingMode::VerticalLr,
            CssDirection::Ltr,
            TaffyDirection::Ltr,
        ),
        (
            CssWritingMode::VerticalLr,
            CssDirection::Rtl,
            TaffyDirection::Ltr,
        ),
    ];
    let mut computed = ComputedValues::default();
    computed.display = CssDisplay::Flex;

    for (writing_mode, direction, expected_direction) in cases {
        computed.writing_mode = writing_mode;
        computed.direction = direction;

        for flex_direction in [CssFlexDirection::Row, CssFlexDirection::RowReverse] {
            computed.flex_direction = flex_direction;
            for (wrap, expected_wrap) in [
                (CssFlexWrap::Wrap, FlexWrap::Wrap),
                (CssFlexWrap::WrapReverse, FlexWrap::WrapReverse),
            ] {
                computed.flex_wrap = wrap;
                for (alignment, expected_items, expected_content) in [
                    (
                        CssAlignment::Start,
                        AlignItemsKeyword::Start,
                        AlignContentKeyword::Start,
                    ),
                    (
                        CssAlignment::End,
                        AlignItemsKeyword::End,
                        AlignContentKeyword::End,
                    ),
                    (
                        CssAlignment::FlexStart,
                        AlignItemsKeyword::FlexStart,
                        AlignContentKeyword::FlexStart,
                    ),
                    (
                        CssAlignment::FlexEnd,
                        AlignItemsKeyword::FlexEnd,
                        AlignContentKeyword::FlexEnd,
                    ),
                    (
                        CssAlignment::Center,
                        AlignItemsKeyword::Center,
                        AlignContentKeyword::Center,
                    ),
                ] {
                    computed.align_items = alignment;
                    computed.align_content = alignment;
                    let style = to_taffy_style(&computed, 16.0);
                    assert_eq!(style.direction, expected_direction);
                    assert_eq!(style.flex_wrap, expected_wrap);
                    assert_eq!(
                        style.align_items.expect("flex alignment").keyword,
                        expected_items,
                        "{writing_mode:?} {direction:?} {wrap:?} {alignment:?}"
                    );
                    assert_eq!(
                        style.align_content.expect("line alignment").keyword,
                        expected_content,
                        "{writing_mode:?} {direction:?} {wrap:?} {alignment:?}"
                    );
                }
            }
        }
    }

    computed.writing_mode = CssWritingMode::HorizontalTb;
    computed.flex_direction = CssFlexDirection::Column;
    for (direction, expected_direction) in [
        (CssDirection::Ltr, TaffyDirection::Ltr),
        (CssDirection::Rtl, TaffyDirection::Rtl),
    ] {
        computed.direction = direction;
        assert_eq!(
            to_taffy_style(&computed, 16.0).direction,
            expected_direction
        );
    }

    computed.writing_mode = CssWritingMode::VerticalRl;
    computed.direction = CssDirection::Rtl;
    for flex_direction in [CssFlexDirection::Column, CssFlexDirection::ColumnReverse] {
        computed.flex_direction = flex_direction;
        for (wrap, expected_wrap) in [
            (CssFlexWrap::Wrap, FlexWrap::WrapReverse),
            (CssFlexWrap::WrapReverse, FlexWrap::Wrap),
        ] {
            computed.flex_wrap = wrap;
            computed.align_items = CssAlignment::Start;
            computed.align_content = CssAlignment::End;
            let style = to_taffy_style(&computed, 16.0);
            assert_eq!(style.flex_wrap, expected_wrap);
            assert_eq!(
                style.align_items.expect("items").keyword,
                AlignItemsKeyword::End
            );
            assert_eq!(
                style.align_content.expect("content").keyword,
                AlignContentKeyword::Start
            );
        }
    }
    computed.flex_wrap = CssFlexWrap::NoWrap;
    computed.align_items = CssAlignment::FlexStart;
    computed.align_content = CssAlignment::FlexEnd;
    let nowrap = to_taffy_style(&computed, 16.0);
    assert_eq!(nowrap.flex_wrap, FlexWrap::NoWrap);
    assert_eq!(
        nowrap.align_items.expect("nowrap items").keyword,
        AlignItemsKeyword::FlexEnd
    );
    assert_eq!(
        nowrap.align_content.expect("nowrap content").keyword,
        AlignContentKeyword::FlexStart
    );

    computed.display = CssDisplay::Block;
    computed.writing_mode = CssWritingMode::VerticalRl;
    computed.direction = CssDirection::Ltr;
    let block = to_taffy_style(&computed, 16.0);
    assert_eq!(block.direction, TaffyDirection::Ltr);
    assert_eq!(
        block.align_items.expect("block items").keyword,
        AlignItemsKeyword::FlexStart
    );
}

#[test]
fn live_flex_projects_rtl_rows_and_sideways_lr_rows_to_physical_axes() {
    let dom = StaticDocument::parse(
        "<html><body>\
         <div id=rtl><div id=r1></div><div id=r2></div><div id=r3></div></div>\
         <div id=sideways><div id=s1></div><div id=s2></div><div id=s3></div></div>\
         </body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; border: 0; }\
             #rtl { display: flex; direction: rtl; justify-content: flex-start;\
                    width: 100px; height: 20px;\
                    row-gap: 7px; column-gap: 11px; }\
             #rtl > div { flex: 0 0 20px; height: 20px; }\
             #sideways { display: flex; writing-mode: sideways-lr; flex-direction: row;\
                          justify-content: flex-start;\
                          width: 20px; height: 100px;\
                          row-gap: 7px; column-gap: 11px; }\
             #sideways > div { flex: 0 0 20px; width: 20px; height: 20px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("logical flex layout");
    let rect = |id| {
        layout
            .get(node_by_id(&dom, dom.document(), id).expect(id))
            .expect(id)
            .physical_rect()
    };

    let rtl = rect("rtl");
    let r1 = rect("r1");
    let r2 = rect("r2");
    let r3 = rect("r3");
    assert!(
        (r1.x - rtl.x - 80.0).abs() <= 0.01,
        "r1={r1:?}, rtl={rtl:?}"
    );
    assert!(
        (r2.x - rtl.x - 49.0).abs() <= 0.01,
        "r2={r2:?}, rtl={rtl:?}"
    );
    assert!(
        (r3.x - rtl.x - 18.0).abs() <= 0.01,
        "r3={r3:?}, rtl={rtl:?}"
    );

    let sideways = rect("sideways");
    let s1 = rect("s1");
    let s2 = rect("s2");
    let s3 = rect("s3");
    assert!(
        (s1.y - sideways.y - 80.0).abs() <= 0.01,
        "s1={s1:?}, sideways={sideways:?}"
    );
    assert!(
        (s2.y - sideways.y - 49.0).abs() <= 0.01,
        "s2={s2:?}, sideways={sideways:?}"
    );
    assert!(
        (s3.y - sideways.y - 18.0).abs() <= 0.01,
        "s3={s3:?}, sideways={sideways:?}"
    );
}

#[test]
fn live_flex_shorthands_reach_taffy_and_change_wrap_geometry() {
    let dom = StaticDocument::parse(
        "<html><body>\
         <div id=short><div id=s1 class=item></div><div id=s2 class=item></div><div id=s3 class=item></div></div>\
         <div id=explicit><div id=e1 class=item></div><div id=e2 class=item></div><div id=e3 class=item></div></div>\
         <div id=nowrap><div id=n1 class=item></div><div id=n2 class=item></div><div id=n3 class=item></div></div>\
         </body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; border: 0; }\
             #short, #explicit, #nowrap { display: flex; width: 100px; }\
             #short { flex-flow: row wrap; }\
             #explicit { flex-direction: row; flex-wrap: wrap; }\
             #nowrap { flex-flow: row nowrap; }\
             .item { flex: 1 1 40px; height: 20px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let short = node_by_id(&dom, dom.document(), "short").expect("short flex host");
    let explicit = node_by_id(&dom, dom.document(), "explicit").expect("explicit host");
    assert_eq!(
        styles.get(short).expect("short style").flex_direction,
        styles.get(explicit).expect("explicit style").flex_direction
    );
    assert_eq!(
        styles.get(short).expect("short style").flex_wrap,
        styles.get(explicit).expect("explicit style").flex_wrap
    );

    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        320.0,
        240.0,
        ViewportSizes::uniform(320.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("flex shorthand layout");
    let rect = |id| {
        layout
            .get(node_by_id(&dom, dom.document(), id).expect(id))
            .expect(id)
            .physical_rect()
    };
    let short_host = rect("short");
    let explicit_host = rect("explicit");
    let s1 = rect("s1");
    let s2 = rect("s2");
    let s3 = rect("s3");
    let e1 = rect("e1");
    let e2 = rect("e2");
    let e3 = rect("e3");
    let n1 = rect("n1");
    let n2 = rect("n2");
    let n3 = rect("n3");

    assert!((s1.width - 50.0).abs() <= 0.01, "s1={s1:?}");
    assert!(
        (s2.x - s1.x - s1.width).abs() <= 0.01,
        "s1={s1:?}, s2={s2:?}"
    );
    assert!((s3.y - s1.y - 20.0).abs() <= 0.01, "s1={s1:?}, s3={s3:?}");
    assert!((s3.width - 100.0).abs() <= 0.01, "s3={s3:?}");
    assert_eq!((s1.width, s1.height), (e1.width, e1.height));
    assert!((s2.width - e2.width).abs() <= 0.01);
    assert!((s2.y - short_host.y - (e2.y - explicit_host.y)).abs() <= 0.01);
    assert!((s3.width - e3.width).abs() <= 0.01);
    assert!((s3.y - short_host.y - (e3.y - explicit_host.y)).abs() <= 0.01);
    assert_eq!(n1.y, n2.y);
    assert_eq!(n2.y, n3.y);
    assert!(
        n3.x > n2.x,
        "nowrap geometry did not stay on one row: {n1:?}, {n2:?}, {n3:?}"
    );
}

#[test]
fn percentage_bearing_calc_resolves_against_the_same_basis_as_a_bare_percentage() {
    // Stated without depending on which box supplies the basis: whatever the
    // engine resolves a bare 50% against, calc(50% - 10px) must land exactly
    // 10px short of it. A disagreement means the percentage inside calc() went
    // to a different basis -- to zero, before these properties were tagged.
    let probe = |declaration: &str| {
        let dom = StaticDocument::parse("<div id=box><div id=inner></div></div>");
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&[&format!(
                "html, body {{ margin: 0; }} #box {{ width: 200px; {declaration} }} #inner {{ height: 5px; }}"
            )]),
            &Device::screen(320.0, 240.0),
            &InteractionStates::default(),
        );
        let mut text = TextSystem::new();
        let (_, layout) = layout_with_text_system(
            &dom,
            &styles,
            320.0,
            240.0,
            ViewportSizes::uniform(320.0, 240.0),
            &mut text,
            &HashMap::new(),
        )
        .expect("layout");
        layout
            .get(node_by_id(&dom, dom.document(), "inner").expect("inner"))
            .expect("inner fragment")
            .physical_rect()
            .x
    };

    for (property, bare, calc) in [
        (
            "padding-left",
            "padding-left: 50%;",
            "padding-left: calc(50% - 10px);",
        ),
        (
            "margin-left",
            "margin-left: 50%;",
            "margin-left: calc(50% - 10px);",
        ),
    ] {
        let bare_x = probe(bare);
        let calc_x = probe(calc);
        assert_eq!(
            calc_x,
            bare_x - 10.0,
            "{property}: bare 50% gave {bare_x}, calc(50% - 10px) gave {calc_x}"
        );
    }

    let basis = |declaration: &str| {
        let dom = StaticDocument::parse("<div id=flex><div id=item></div></div>");
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&[&format!(
                "html, body {{ margin: 0; }} #flex {{ display: flex; width: 200px; }} #item {{ {declaration} height: 5px; }}"
            )]),
            &Device::screen(320.0, 240.0),
            &InteractionStates::default(),
        );
        let mut text = TextSystem::new();
        let (_, layout) = layout_with_text_system(
            &dom,
            &styles,
            320.0,
            240.0,
            ViewportSizes::uniform(320.0, 240.0),
            &mut text,
            &HashMap::new(),
        )
        .expect("layout");
        layout
            .get(node_by_id(&dom, dom.document(), "item").expect("item"))
            .expect("item fragment")
            .physical_rect()
            .width
    };
    let bare = basis("flex-basis: 50%;");
    let calc = basis("flex-basis: calc(50% - 10px);");
    assert_eq!(
        calc,
        bare - 10.0,
        "flex-basis: bare 50% gave {bare}, calc(50% - 10px) gave {calc}"
    );
}

#[test]
fn replaced_auto_width_is_intrinsic_in_flow_and_stretchable_in_flex_and_grid() {
    let used_size = |css: &str| {
        let dom = StaticDocument::parse(
            "<div id=parent><canvas id=item width=\"100\" height=\"100\"></canvas></div>",
        );
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&[&format!("html, body {{ margin: 0; }} {css}")]),
            &Device::screen(320.0, 240.0),
            &InteractionStates::default(),
        );
        let mut text = TextSystem::new();
        let (_, layout) = layout_with_text_system(
            &dom,
            &styles,
            320.0,
            240.0,
            ViewportSizes::uniform(320.0, 240.0),
            &mut text,
            &HashMap::new(),
        )
        .expect("layout");
        let rect = layout
            .get(node_by_id(&dom, dom.document(), "item").expect("item"))
            .expect("fragment")
            .physical_rect();
        (rect.width, rect.height)
    };

    // CSS 2.1 10.3.4: block-level replaced, `width: auto` is the intrinsic
    // width, not the containing block. This is the case that laid a 100x100
    // canvas out at 200x200.
    assert_eq!(
        used_size("#parent { width: 200px; } #item { display: block; }"),
        (100.0, 100.0),
        "block flow keeps the natural size"
    );
    // Flexbox 9.4: an auto cross size with align-items: stretch fills the
    // line. The intrinsic width must not pin it.
    assert_eq!(
        used_size(
            "#parent { display: flex; flex-direction: column; width: 200px; height: 200px; }"
        ),
        (200.0, 100.0),
        "a column flex item still stretches across the line"
    );
    assert_eq!(
        used_size("#parent { display: flex; width: 200px; } #item { flex-grow: 1; }"),
        (200.0, 100.0),
        "a row flex item still grows along the main axis"
    );
    // Grid: justify-self and align-self default to stretch for a non-ratio'd
    // auto size, and a replaced item must not be exempted by its natural width.
    // css-grid-1 6.2: `normal` behaves as `start`, not `stretch`, for a replaced
    // item with a natural size in the axis. WPT states it outright in
    // css-grid/alignment/grid-align-stretching-replaced-items.html: "default
    // alignment is resolved as 'start' for replaced elements so it prevents
    // stretching to be applied". An explicit `stretch` still stretches, which
    // the next assertion pins.
    assert_eq!(
        used_size("#parent { display: grid; width: 200px; height: 200px; }"),
        (100.0, 100.0),
        "a replaced grid item is start-aligned under `normal`, not stretched"
    );
    assert_eq!(
        used_size(
            "#parent { display: grid; width: 200px; height: 200px; align-items: stretch; justify-items: stretch; }"
        ),
        (200.0, 200.0),
        "an explicit stretch still stretches a replaced grid item"
    );

    // An inline replaced element is 10.3.2 and is already right through the
    // atomic-root path; the block rule must leave it alone. Both halves of a
    // reftest render here, so widening into inline moved the references.
    assert_eq!(
        used_size("#parent { width: 200px; }"),
        (100.0, 100.0),
        "an inline canvas keeps its natural box through the inline path"
    );

    // Each of these is a css-sizing reftest cluster that regressed when the
    // block-flow rule was wider than CSS 2.1 10.3.2 allows.
    assert_eq!(
        used_size("#parent { width: 200px; } #item { display: block; height: 50px; }"),
        (50.0, 50.0),
        "a definite height transfers to the width through the natural ratio"
    );
    assert_eq!(
        used_size(
            "#parent { width: 200px; } #item { display: block; width: max-content; height: 50px; }"
        ),
        (50.0, 50.0),
        "an intrinsic keyword is not `auto` and keeps its ratio-transferred size"
    );
    assert_eq!(
        used_size(
            "#parent { width: 200px; } #item { display: block; aspect-ratio: 2; height: 50px; }"
        ),
        (100.0, 50.0),
        "an author aspect-ratio owns the transfer, not the natural size"
    );
    // Under border-box the box resolves CSS 2.1 10.4 itself and hands Taffy
    // both axes as definite border-box lengths: the natural 100 plus 10px of
    // padding each side. Forcing only a width had bypassed the
    // ratio-preserving min/max clamp and failed box-sizing-replaced-001..003
    // twice; resolving the whole table in content space keeps them green.
    assert_eq!(
        used_size(
            "#parent { width: 200px; } #item { display: block; padding: 10px; box-sizing: border-box; }"
        ),
        (120.0, 120.0),
        "a border-box replaced element measures its natural size plus its edges"
    );
}

#[test]
fn inline_replaced_auto_width_is_intrinsic_whatever_its_display() {
    let used_size = |css: &str| {
        let dom = StaticDocument::parse(
            "<div id=parent><canvas id=item width=\"100\" height=\"100\"></canvas></div>",
        );
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&[&format!("html, body {{ margin: 0; }} {css}")]),
            &Device::screen(320.0, 240.0),
            &InteractionStates::default(),
        );
        let mut text = TextSystem::new();
        let (_, layout) = layout_with_text_system(
            &dom,
            &styles,
            320.0,
            240.0,
            ViewportSizes::uniform(320.0, 240.0),
            &mut text,
            &HashMap::new(),
        )
        .expect("layout");
        let rect = layout
            .get(node_by_id(&dom, dom.document(), "item").expect("item"))
            .expect("fragment")
            .physical_rect();
        (rect.width, rect.height)
    };

    // CSS 2.1 10.3.2 does not consult display: an inline replaced element with
    // `width: auto` uses its intrinsic width. Before the atomic-root wrapper
    // learned to skip replaced roots, only the first of these was right: an
    // `inline-block` replaced root was wrapped in a viewport-sized containing
    // block, formatted under MaxContent, and stretched to 320 with its height
    // then taken from the natural ratio. `img` carries `display: inline-block`
    // from the UA sheet, so every bare image took that path.
    for display in ["inline", "inline-block", "block"] {
        assert_eq!(
            used_size(&format!(
                "#parent {{ width: 200px; }} #item {{ display: {display}; }}"
            )),
            (100.0, 100.0),
            "a replaced element with an auto width keeps its natural box as `display: {display}`"
        );
    }
    // The other two conjuncts of the shrink-to-fit predicate, pinned so a
    // change to either is a deliberate one rather than a silent regression.
    assert_eq!(
        used_size(
            "#parent { width: 200px; } #item { display: inline-block; vertical-align: bottom; }"
        ),
        (100.0, 100.0),
        "a non-baseline atomic root keeps its natural box"
    );
    assert_eq!(
        used_size("#parent { width: 200px; } #item { display: inline-block; width: 40px; }"),
        (40.0, 40.0),
        "a definite width still wins over the intrinsic one"
    );
}

#[test]
fn replaced_element_as_table_cell_lays_out_like_a_cell_around_it() {
    // CSS2/tables/table-anonymous-objects-211, byte for byte: row 1 wraps its
    // images in cell <div>s; row 2 gives the first two images
    // `display: table-cell` directly, under `white-space: pre`. CSS 2.1 17.2.1
    // says a replaced element cannot be an internal table box -- it is treated
    // as inline and an anonymous cell is generated around it and its
    // surrounding whitespace -- so the two rows must lay out identically.
    // Three things had to hold for that: the demotion itself, whitespace-only
    // boxes between table parts being dropped on their content rather than
    // on whether they collapse, and the demoted element still being admitted
    // as an atomic inline when layout re-reads its computed display.
    let html = "<div class=\"table\"><div class=\"row\" id=\"r1\">\n      <div class=\"table-cell\"> <canvas width=\"15\" height=\"15\"></canvas>\t <canvas width=\"15\" height=\"15\"></canvas>   </div>\n      <div class=\"table-cell\"><canvas width=\"15\" height=\"15\"></canvas></div>\t <div class=\"table-cell\"><canvas width=\"15\" height=\"15\"></canvas></div>\n    </div><div class=\"row\" id=\"r2\"> <canvas class=\"table-cell\" width=\"15\" height=\"15\"></canvas>\t <canvas class=\"table-cell\" width=\"15\" height=\"15\"></canvas>   <div class=\"table-cell\"><canvas width=\"15\" height=\"15\"></canvas></div>\t <div class=\"table-cell\"><canvas width=\"15\" height=\"15\"></canvas></div>\n    </div></div>";
    let dom = StaticDocument::parse(html);
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&[
            "html, body { margin: 0; } canvas { display: inline-block; } .table { display: table; white-space: pre; } .row { display: table-row; } .table-cell { display: table-cell; }",
        ]),
        &Device::screen(640.0, 240.0),
        &InteractionStates::default(),
    );
    let mut text = TextSystem::new();
    let (_, layout) = layout_with_text_system(
        &dom,
        &styles,
        640.0,
        240.0,
        ViewportSizes::uniform(640.0, 240.0),
        &mut text,
        &HashMap::new(),
    )
    .expect("layout");
    fn canvases(
        dom: &StaticDocument,
        n: <StaticDocument as LayoutDom>::NodeId,
        out: &mut Vec<<StaticDocument as LayoutDom>::NodeId>,
    ) {
        if dom
            .element_name(n)
            .is_some_and(|e| e.local.as_ref() == "canvas")
        {
            out.push(n);
        }
        for c in dom.dom_children(n) {
            canvases(dom, c, out);
        }
    }
    let row_shape = |rid: &str| {
        let row = node_by_id(&dom, dom.document(), rid).expect(rid);
        let rr = layout.get(row).expect("row").physical_rect();
        let mut cs = Vec::new();
        canvases(&dom, row, &mut cs);
        let rects: Vec<(f32, f32, f32, f32)> = cs
            .iter()
            .map(|&c| {
                let r = layout.get(c).expect("canvas").physical_rect();
                (r.x, r.y - rr.y, r.width, r.height)
            })
            .collect();
        (rr.width, rr.height, rects)
    };
    let r1 = row_shape("r1");
    let r2 = row_shape("r2");
    assert_eq!(r1.2.len(), 4, "row 1 has four images");
    assert_eq!(
        r1, r2,
        "images that are cells lay out exactly like images inside cells"
    );
    assert!(
        r1.2.iter().all(|r| r.2 == 15.0 && r.3 == 15.0),
        "every image keeps its natural 15x15"
    );
}

#[test]
fn replaced_min_max_follows_css21_10_4_for_every_box_sizing_replaced_case() {
    // The sixty cases of css-sizing/box-sizing-replaced-001..003, reduced to
    // content-box inputs. Each test's images share one edge rule:
    // 001 `.with-padding` (5px padding, border-box) = 10px of edges,
    // 002 `.with-borderpadding` (5px padding + 5px border, border-box) = 20px,
    // 003 `.content-box` = none. The reference for all three is a 75x75 image.
    // (natural w, natural h, [min-w, max-w, min-h, max-h] in border-box px)
    let cases: &[(&str, f32, f32, f32, [Option<f32>; 4])] = &[
        // test, natural w, natural h, edges, constraints
        ("001", 75.0, 75.0, 10.0, [None, None, None, None]),
        (
            "001",
            75.0,
            75.0,
            10.0,
            [Some(70.0), Some(115.0), Some(55.0), Some(130.0)],
        ),
        (
            "001",
            150.0,
            150.0,
            10.0,
            [None, Some(85.0), Some(70.0), None],
        ),
        (
            "001",
            300.0,
            150.0,
            10.0,
            [None, Some(85.0), Some(85.0), None],
        ),
        (
            "001",
            25.0,
            25.0,
            10.0,
            [Some(85.0), None, None, Some(110.0)],
        ),
        (
            "001",
            25.0,
            50.0,
            10.0,
            [Some(85.0), None, None, Some(85.0)],
        ),
        (
            "001",
            150.0,
            150.0,
            10.0,
            [Some(70.0), None, None, Some(85.0)],
        ),
        (
            "001",
            150.0,
            300.0,
            10.0,
            [Some(85.0), None, None, Some(85.0)],
        ),
        (
            "001",
            25.0,
            25.0,
            10.0,
            [None, Some(110.0), Some(85.0), None],
        ),
        (
            "001",
            50.0,
            25.0,
            10.0,
            [None, Some(85.0), Some(85.0), None],
        ),
        (
            "001",
            300.0,
            375.0,
            10.0,
            [Some(85.0), Some(160.0), None, Some(85.0)],
        ),
        (
            "001",
            250.0,
            250.0,
            10.0,
            [Some(35.0), Some(235.0), None, Some(85.0)],
        ),
        (
            "001",
            375.0,
            300.0,
            10.0,
            [None, Some(85.0), Some(85.0), Some(160.0)],
        ),
        (
            "001",
            250.0,
            250.0,
            10.0,
            [None, Some(85.0), Some(35.0), Some(235.0)],
        ),
        (
            "001",
            25.0,
            25.0,
            10.0,
            [Some(60.0), Some(110.0), Some(85.0), None],
        ),
        (
            "001",
            50.0,
            25.0,
            10.0,
            [Some(65.0), Some(85.0), Some(85.0), None],
        ),
        (
            "001",
            25.0,
            25.0,
            10.0,
            [Some(85.0), None, Some(60.0), Some(110.0)],
        ),
        (
            "001",
            25.0,
            50.0,
            10.0,
            [Some(85.0), None, Some(65.0), Some(85.0)],
        ),
        (
            "001",
            50.0,
            100.0,
            10.0,
            [Some(85.0), None, None, Some(85.0)],
        ),
        (
            "001",
            100.0,
            50.0,
            10.0,
            [None, Some(85.0), Some(85.0), None],
        ),
        ("002", 75.0, 75.0, 20.0, [None, None, None, None]),
        (
            "002",
            75.0,
            75.0,
            20.0,
            [Some(80.0), Some(125.0), Some(65.0), Some(140.0)],
        ),
        (
            "002",
            150.0,
            150.0,
            20.0,
            [None, Some(95.0), Some(80.0), None],
        ),
        (
            "002",
            300.0,
            150.0,
            20.0,
            [None, Some(95.0), Some(95.0), None],
        ),
        (
            "002",
            25.0,
            25.0,
            20.0,
            [Some(95.0), None, None, Some(120.0)],
        ),
        (
            "002",
            25.0,
            50.0,
            20.0,
            [Some(95.0), None, None, Some(95.0)],
        ),
        (
            "002",
            150.0,
            150.0,
            20.0,
            [Some(80.0), None, None, Some(95.0)],
        ),
        (
            "002",
            150.0,
            300.0,
            20.0,
            [Some(95.0), None, None, Some(95.0)],
        ),
        (
            "002",
            25.0,
            25.0,
            20.0,
            [None, Some(120.0), Some(95.0), None],
        ),
        (
            "002",
            50.0,
            25.0,
            20.0,
            [None, Some(95.0), Some(95.0), None],
        ),
        (
            "002",
            300.0,
            375.0,
            20.0,
            [Some(95.0), Some(170.0), None, Some(95.0)],
        ),
        (
            "002",
            250.0,
            250.0,
            20.0,
            [Some(45.0), Some(245.0), None, Some(95.0)],
        ),
        (
            "002",
            375.0,
            300.0,
            20.0,
            [None, Some(95.0), Some(95.0), Some(170.0)],
        ),
        (
            "002",
            250.0,
            250.0,
            20.0,
            [None, Some(95.0), Some(45.0), Some(245.0)],
        ),
        (
            "002",
            25.0,
            25.0,
            20.0,
            [Some(70.0), Some(120.0), Some(95.0), None],
        ),
        (
            "002",
            50.0,
            25.0,
            20.0,
            [Some(75.0), Some(95.0), Some(95.0), None],
        ),
        (
            "002",
            25.0,
            25.0,
            20.0,
            [Some(95.0), None, Some(70.0), Some(120.0)],
        ),
        (
            "002",
            25.0,
            50.0,
            20.0,
            [Some(95.0), None, Some(75.0), Some(95.0)],
        ),
        (
            "002",
            50.0,
            100.0,
            20.0,
            [Some(95.0), None, None, Some(95.0)],
        ),
        (
            "002",
            100.0,
            50.0,
            20.0,
            [None, Some(95.0), Some(95.0), None],
        ),
        ("003", 75.0, 75.0, 0.0, [None, None, None, None]),
        (
            "003",
            75.0,
            75.0,
            0.0,
            [Some(60.0), Some(125.0), Some(45.0), Some(120.0)],
        ),
        (
            "003",
            150.0,
            150.0,
            0.0,
            [None, Some(75.0), Some(60.0), None],
        ),
        (
            "003",
            300.0,
            150.0,
            0.0,
            [None, Some(75.0), Some(75.0), None],
        ),
        (
            "003",
            25.0,
            25.0,
            0.0,
            [Some(75.0), None, None, Some(100.0)],
        ),
        ("003", 25.0, 50.0, 0.0, [Some(75.0), None, None, Some(75.0)]),
        (
            "003",
            150.0,
            150.0,
            0.0,
            [Some(60.0), None, None, Some(75.0)],
        ),
        (
            "003",
            150.0,
            300.0,
            0.0,
            [Some(75.0), None, None, Some(75.0)],
        ),
        (
            "003",
            25.0,
            25.0,
            0.0,
            [None, Some(100.0), Some(75.0), None],
        ),
        ("003", 50.0, 25.0, 0.0, [None, Some(75.0), Some(75.0), None]),
        (
            "003",
            300.0,
            375.0,
            0.0,
            [Some(75.0), Some(150.0), None, Some(75.0)],
        ),
        (
            "003",
            250.0,
            250.0,
            0.0,
            [Some(25.0), Some(225.0), None, Some(75.0)],
        ),
        (
            "003",
            375.0,
            300.0,
            0.0,
            [None, Some(75.0), Some(75.0), Some(150.0)],
        ),
        (
            "003",
            250.0,
            250.0,
            0.0,
            [None, Some(75.0), Some(25.0), Some(225.0)],
        ),
        (
            "003",
            25.0,
            25.0,
            0.0,
            [Some(50.0), Some(100.0), Some(75.0), None],
        ),
        (
            "003",
            50.0,
            25.0,
            0.0,
            [Some(55.0), Some(75.0), Some(75.0), None],
        ),
        (
            "003",
            25.0,
            25.0,
            0.0,
            [Some(75.0), None, Some(50.0), Some(100.0)],
        ),
        (
            "003",
            25.0,
            50.0,
            0.0,
            [Some(75.0), None, Some(55.0), Some(75.0)],
        ),
        (
            "003",
            50.0,
            100.0,
            0.0,
            [Some(75.0), None, None, Some(75.0)],
        ),
        (
            "003",
            100.0,
            50.0,
            0.0,
            [None, Some(75.0), Some(75.0), None],
        ),
    ];
    for (test, w, h, edges, [min_w, max_w, min_h, max_h]) in cases {
        let content = |v: Option<f32>| v.map(|v| v - edges);
        let got = replaced_min_max(
            (*w, *h),
            content(*min_w),
            content(*max_w),
            content(*min_h),
            content(*max_h),
        );
        assert!(
            (got.0 - 75.0).abs() < 1e-3 && (got.1 - 75.0).abs() < 1e-3,
            "box-sizing-replaced-{test}: {w}x{h} with {:?} resolved to {got:?}, expected 75x75 content",
            [min_w, max_w, min_h, max_h]
        );
    }
}

// Receipts for `layout/hit_testing.rs`. Each names one function of the module
// and states its contract directly, so the module can be judged on its own
// rather than through whichever layout test happens to route a pointer
// through it.

#[test]
fn hit_test_with_scroll_shifts_the_descendants_of_a_scrolled_container() {
    let dom =
        StaticDocument::parse("<div id=scroller><div id=spacer></div><div id=target></div></div>");
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #scroller { width: 100px; height: 100px; overflow: hidden; } \
             #spacer { height: 150px; } \
             #target { height: 50px; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let layout = layout(&dom, &styles, 320.0, 240.0).expect("layout");
    let node = |id| node_by_id(&dom, dom.document(), id).expect(id);

    // Unscrolled, the target sits below the container's clip and the spacer
    // owns the visible area.
    assert_eq!(
        hit_test(&dom, &styles, &layout, 10.0, 10.0),
        Some(node("spacer"))
    );
    assert_ne!(
        hit_test(&dom, &styles, &layout, 10.0, 160.0),
        Some(node("target"))
    );

    // Scrolled by the spacer's height, the target's visible fragment starts
    // at the container's top edge; the container itself does not move.
    let mut offsets = HashMap::new();
    offsets.insert(node("scroller"), (0.0, 150.0));
    assert_eq!(
        hit_test_with_scroll(&dom, &styles, &layout, &offsets, 10.0, 10.0),
        Some(node("target"))
    );
    assert_eq!(
        hit_test_with_scroll(&dom, &styles, &layout, &offsets, 10.0, 60.0),
        Some(node("scroller")),
        "below the scrolled target only the container remains"
    );
}

#[test]
fn z_index_stacking_level_exists_only_where_css_lets_z_index_establish_a_context() {
    let dom = StaticDocument::parse(
        "<div id=static_block></div><div id=relative></div><div id=relative_auto></div>\
         <div id=flex><div id=flex_item></div></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["#static_block { z-index: 3; } \
             #relative { position: relative; z-index: 3; } \
             #relative_auto { position: relative; z-index: auto; } \
             #flex { display: flex; } #flex_item { z-index: 2; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let level = |id| {
        z_index_stacking_level(
            &dom,
            &styles,
            node_by_id(&dom, dom.document(), id).expect(id),
        )
    };

    assert_eq!(
        level("static_block"),
        None,
        "a static block keeps normal paint order"
    );
    assert_eq!(level("relative"), Some(3));
    assert_eq!(
        level("relative_auto"),
        None,
        "z-index: auto establishes nothing"
    );
    assert_eq!(
        level("flex_item"),
        Some(2),
        "a direct flex item may carry a level while static"
    );
}

#[test]
fn order_modified_children_reorders_only_flex_and_grid_containers() {
    let dom = StaticDocument::parse(
        "<div id=block><div id=b_second></div><div id=b_first></div></div>\
         <div id=flex><div id=f_late></div><div id=f_early></div><div id=f_late_too></div></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["#b_second { order: 2; } #b_first { order: 1; } \
             #flex { display: flex; } \
             #f_late { order: 1; } #f_early { order: -1; } #f_late_too { order: 1; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let node = |id| node_by_id(&dom, dom.document(), id).expect(id);

    assert_eq!(
        order_modified_children(&dom, &styles, node("block")),
        vec![node("b_second"), node("b_first")],
        "a block container ignores `order`"
    );
    assert_eq!(
        order_modified_children(&dom, &styles, node("flex")),
        vec![node("f_early"), node("f_late"), node("f_late_too")],
        "a flex container sorts by `order`, stably for equal values"
    );
}

#[test]
fn stacking_paint_children_applies_order_first_and_stacking_level_second() {
    let dom = StaticDocument::parse(
        "<div id=flex><div id=a></div><div id=b></div><div id=c></div></div>",
    );
    let styles = resolve_styles(
        &dom,
        &StyleSet::cambium(&["#flex { display: flex; } \
             #a { order: 1; } \
             #b { order: 0; position: relative; z-index: 5; } \
             #c { order: 0; }"]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let node = |id| node_by_id(&dom, dom.document(), id).expect(id);

    // `order` yields b, c, a; the stable sort by stacking level then lifts b
    // (level 5) past the two level-0 items without reordering them.
    assert_eq!(
        hit_testing::stacking_paint_children(&dom, &styles, node("flex")),
        vec![node("c"), node("a"), node("b")]
    );
}

// Receipts for `layout/taffy_style.rs`: the value converters, stated against
// parsed CSS so the shapes they receive are the ones the cascade produces.

#[test]
fn dimension_converters_tag_percentage_bearing_calc_for_the_basis_and_pass_the_rest_through() {
    let dom = StaticDocument::parse("<div id=box></div>");
    let computed_for = |css: &str| {
        resolve_styles(
            &dom,
            &StyleSet::cambium(&[&format!("#box {{ {css} }}")]),
            &Device::screen(320.0, 240.0),
            &InteractionStates::default(),
        )
    };
    let node = node_by_id(&dom, dom.document(), "box").expect("box");
    reset_calc_scratch();

    let styles = computed_for(
        "width: calc(50% - 10px); min-width: min(100% - 48px, 960px); max-width: 30px; \
         height: 50%; margin-left: calc(25% + 4px); margin-top: auto; column-gap: 10%;",
    );
    let computed = styles.get(node).expect("box style");

    // A calc() mixing a percentage with lengths becomes a tagged slot whose
    // resolution waits for the real basis.
    let width = dimension(computed.width, 16.0).into_raw();
    assert!(
        width.is_calc(),
        "calc(50% - 10px) must not flatten at a zero basis"
    );
    assert_eq!(resolve_taffy_calc(width.calc_value(), 200.0), 90.0);

    // Plain values keep their native Taffy forms.
    assert_eq!(dimension(computed.max_width, 16.0), Dimension::length(30.0));
    assert_eq!(dimension(computed.height, 16.0), Dimension::percent(0.5));
    assert_eq!(dimension(CssSize::Auto, 16.0), Dimension::auto());

    // With a definite basis a math length resolves; without one it is auto,
    // never a value resolved against zero.
    assert_eq!(
        dimension_with_basis(computed.min_width, 16.0, Some(500.0)),
        Dimension::length(452.0)
    );
    assert_eq!(
        dimension_with_basis(computed.min_width, 16.0, None),
        Dimension::auto()
    );
    assert_eq!(resolved_explicit_size(computed.width, 16.0, None), None);
    assert_eq!(
        resolved_explicit_size(computed.width, 16.0, Some(200.0)),
        Some(90.0)
    );
    assert_eq!(
        resolved_explicit_size(computed.max_width, 16.0, None),
        Some(30.0)
    );

    // Margins and gaps take the same three paths.
    let margin_left = margin(computed.margin_left, 16.0).into_raw();
    assert!(margin_left.is_calc());
    assert_eq!(resolve_taffy_calc(margin_left.calc_value(), 200.0), 54.0);
    assert_eq!(
        margin(computed.margin_top, 16.0),
        LengthPercentageAuto::auto()
    );
    assert_eq!(
        gap(computed.column_gap, 16.0),
        LengthPercentage::percent(0.1)
    );

    // A tag is scoped to its pass: after the reset it resolves to nothing.
    reset_calc_scratch();
    assert_eq!(resolve_taffy_calc(width.calc_value(), 200.0), 0.0);
}

#[test]
fn border_line_height_alignment_and_overflow_converters_follow_their_css_tables() {
    assert_eq!(
        border_width_px(BorderStyle::None, BorderWidth::Thick, 16.0),
        0.0
    );
    assert_eq!(
        border_width_px(BorderStyle::Hidden, BorderWidth::Thick, 16.0),
        0.0
    );
    assert_eq!(
        border_width_px(BorderStyle::Solid, BorderWidth::Thin, 16.0),
        1.0
    );
    assert_eq!(
        border_width_px(BorderStyle::Solid, BorderWidth::Medium, 16.0),
        3.0
    );
    assert_eq!(
        border_width_px(BorderStyle::Solid, BorderWidth::Thick, 16.0),
        5.0
    );
    assert_eq!(
        border_width_px(
            BorderStyle::Solid,
            BorderWidth::Length(Length::px(2.5)),
            16.0
        ),
        2.5
    );

    assert_eq!(line_height_px(&LineHeight::Normal, 20.0), 24.0);
    assert_eq!(line_height_px(&LineHeight::Number(1.5), 20.0), 30.0);
    assert_eq!(
        line_height_px(
            &LineHeight::Value(CssLengthPercentage::Percentage(1.5)),
            20.0
        ),
        30.0
    );
    assert_eq!(
        line_height_px(
            &LineHeight::Value(CssLengthPercentage::Length(Length::px(18.0))),
            20.0
        ),
        18.0
    );

    // `auto` self-alignment defers to the container unless a content-keyword
    // size has already defeated stretch.
    assert_eq!(self_alignment(CssAlignment::Auto, CssSize::Auto), None);
    assert_eq!(
        self_alignment(CssAlignment::Auto, CssSize::MinContent).map(|a| a.keyword),
        Some(AlignItemsKeyword::Start)
    );
    assert_eq!(
        self_alignment(CssAlignment::Center, CssSize::Auto).map(|a| a.keyword),
        Some(AlignItemsKeyword::Center)
    );
    assert_eq!(
        align_content(CssAlignment::SpaceEvenly).keyword,
        AlignContentKeyword::SpaceEvenly
    );
    assert_eq!(
        align_content(CssAlignment::Normal).keyword,
        AlignContentKeyword::Stretch
    );

    assert_eq!(overflow(CssOverflow::Visible), Overflow::Visible);
    assert_eq!(overflow(CssOverflow::Hidden), Overflow::Hidden);
    assert_eq!(overflow(CssOverflow::Clip), Overflow::Clip);
    assert_eq!(overflow(CssOverflow::Auto), Overflow::Scroll);
    assert_eq!(overflow(CssOverflow::Scroll), Overflow::Scroll);

    assert_eq!(flex_basis(CssFlexBasis::Auto, 16.0), TaffyFlexBasis::auto());
    assert_eq!(
        flex_basis(CssFlexBasis::Content, 16.0),
        TaffyFlexBasis::content()
    );
    assert_eq!(
        flex_basis(CssFlexBasis::MinContent, 16.0),
        TaffyFlexBasis::auto()
    );
}

#[test]
fn container_alignment_normal_reaches_taffy_as_unset_and_every_other_keyword_passes_through() {
    let dom = StaticDocument::parse("<div id=box></div>");
    let node = node_by_id(&dom, dom.document(), "box").expect("box");
    let projected = |css: &str| {
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&[&format!("#box {{ display: grid; {css} }}")]),
            &Device::screen(320.0, 240.0),
            &InteractionStates::default(),
        );
        let style = to_taffy_style(styles.get(node).expect("box style"), 16.0);
        (
            style.align_items.map(|a| a.keyword),
            style.justify_items.map(|a| a.keyword),
        )
    };

    assert_eq!(
        projected(""),
        (None, None),
        "the initial `normal` is unset for Taffy"
    );
    assert_eq!(
        projected("align-items: stretch; justify-items: center;"),
        (
            Some(AlignItemsKeyword::Stretch),
            Some(AlignItemsKeyword::Center)
        ),
        "an explicit stretch is a real keyword, not `normal`"
    );
    assert_eq!(
        projected("align-items: normal; justify-items: end;"),
        (None, Some(AlignItemsKeyword::End))
    );
}
