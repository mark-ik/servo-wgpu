// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Livery computed-value and DOM adapter for Buckram's CSS box generator.
//!
//! Livery resolves computed values into box-generation input. Buckram owns
//! suppression, flattening, anonymous fixup, inline splitting, roles,
//! provenance, and tree relationships.

use std::{hash::Hash, ops::Deref};

use buckram::{
    BoxGeneration, BoxOrigin, BoxTreeInput, ContainingBlockEstablishment, CssBoxTree, Direction,
    DisplayInside, DisplayOutside, DisplayRole, FloatSide, FlowAxes, InternalTableRole,
    PositioningScheme, WritingMode, generate_box_tree,
};
use layout_dom_api::{LayoutDom, NodeKind};
use livery::{
    ComputedValues,
    values::{
        Direction as ComputedDirection, Display as ComputedDisplay, Float as ComputedFloat,
        Position as ComputedPosition, WhiteSpaceCollapse, WritingMode as ComputedWritingMode,
    },
};

use crate::StylePlane;

/// Livery-generated, Buckram-normalized CSS boxes.
#[derive(Clone, Debug)]
pub(crate) struct GeneratedBoxTree<Id> {
    tree: CssBoxTree<Id>,
}

impl<Id> Deref for GeneratedBoxTree<Id> {
    type Target = CssBoxTree<Id>;

    fn deref(&self) -> &Self::Target {
        &self.tree
    }
}

impl<Id> GeneratedBoxTree<Id>
where
    Id: Copy + Eq + Hash,
{
    /// Resolve the host DOM into Buckram box-generation input.
    pub(crate) fn from_dom<D>(dom: &D, styles: &StylePlane<Id>) -> Self
    where
        D: LayoutDom<NodeId = Id>,
    {
        /// The box-tree descent's one entry per DOM level. It keeps a whole
        /// cloned `ComputedValues` alive in every frame while it walks the
        /// children, which is most of why a level is expensive; see
        /// [`crate::with_recursion_stack`].
        fn collect<D>(
            dom: &D,
            styles: &StylePlane<D::NodeId>,
            node: D::NodeId,
            inherited: Option<&ComputedValues>,
            output: &mut Vec<BoxTreeInput<D::NodeId>>,
        ) where
            D: LayoutDom,
            D::NodeId: Copy + Eq + Hash,
        {
            crate::with_recursion_stack(move || {
                collect_on_this_stack(dom, styles, node, inherited, output)
            })
        }

        fn collect_on_this_stack<D>(
            dom: &D,
            styles: &StylePlane<D::NodeId>,
            node: D::NodeId,
            inherited: Option<&ComputedValues>,
            output: &mut Vec<BoxTreeInput<D::NodeId>>,
        ) where
            D: LayoutDom,
            D::NodeId: Copy + Eq + Hash,
        {
            match dom.kind(node) {
                NodeKind::Document | NodeKind::DocumentFragment => {
                    for child in dom.flat_children(node) {
                        collect(dom, styles, child, inherited, output);
                    }
                },
                NodeKind::Element => {
                    let computed = styles.get(node).cloned().unwrap_or_default();
                    let mut children = Vec::new();
                    for child in dom.flat_children(node) {
                        collect(dom, styles, child, Some(&computed), &mut children);
                    }
                    output.push(
                        BoxTreeInput::new(
                            BoxOrigin::Element(node),
                            display_role(computed.display, is_replaced_element(dom, node)),
                            flow_axes(&computed),
                            positioning_scheme(computed.position),
                            is_replaced_element(dom, node),
                            children,
                        )
                        .with_float(float_side(computed.float))
                        .with_containing_block_establishment(containing_block_establishment(
                            &computed,
                        )),
                    );
                },
                NodeKind::Text => {
                    let preserves_whitespace = inherited.is_some_and(|style| {
                        matches!(
                            style.white_space_collapse,
                            WhiteSpaceCollapse::Preserve | WhiteSpaceCollapse::BreakSpaces
                        )
                    });
                    let whitespace_only = dom.text(node).is_none_or(|text| {
                        text.chars()
                            .all(|ch| matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{c}'))
                    });
                    let collapsible_whitespace = !preserves_whitespace && whitespace_only;
                    output.push(BoxTreeInput::text(
                        BoxOrigin::Text(node),
                        inherited.map(flow_axes).unwrap_or_default(),
                        collapsible_whitespace,
                        whitespace_only,
                    ));
                },
                _ => {},
            }
        }

        let mut roots = Vec::new();
        collect(dom, styles, dom.document(), None, &mut roots);
        Self {
            tree: generate_box_tree(roots),
        }
    }

    pub(crate) fn into_tree(self) -> CssBoxTree<Id> {
        self.tree
    }
}

/// CSS 2.1 17.2.1 and css-display-3: a replaced element cannot become an
/// internal table box. `display: table-cell` on an `<img>` is treated as
/// `inline`, and the anonymous cell is generated around the element and its
/// surrounding whitespace exactly as if it had been inline all along.
fn display_role(display: ComputedDisplay, replaced: bool) -> DisplayRole {
    let normal = |outside, inside| DisplayRole {
        generation: BoxGeneration::Normal,
        outside,
        inside,
        list_item: false,
        internal_table: None,
    };
    let internal = |role| DisplayRole {
        generation: BoxGeneration::Normal,
        outside: None,
        inside: None,
        list_item: false,
        internal_table: Some(role),
    };
    match display {
        ComputedDisplay::None => DisplayRole::NONE,
        ComputedDisplay::Contents => DisplayRole::CONTENTS,
        ComputedDisplay::Inline => DisplayRole::INLINE_FLOW,
        ComputedDisplay::Block => DisplayRole::BLOCK_FLOW,
        ComputedDisplay::FlowRoot => {
            normal(Some(DisplayOutside::Block), Some(DisplayInside::FlowRoot))
        },
        ComputedDisplay::ListItem => DisplayRole {
            list_item: true,
            ..DisplayRole::BLOCK_FLOW
        },
        ComputedDisplay::InlineBlock => {
            normal(Some(DisplayOutside::Inline), Some(DisplayInside::FlowRoot))
        },
        ComputedDisplay::Flex => normal(Some(DisplayOutside::Block), Some(DisplayInside::Flex)),
        ComputedDisplay::Grid => normal(Some(DisplayOutside::Block), Some(DisplayInside::Grid)),
        ComputedDisplay::Table => normal(Some(DisplayOutside::Block), Some(DisplayInside::Table)),
        ComputedDisplay::InlineTable => {
            normal(Some(DisplayOutside::Inline), Some(DisplayInside::Table))
        },
        _ if replaced && is_internal_table_display(display) => {
            // The same role a replaced `display: inline` element takes.
            normal(Some(DisplayOutside::Inline), Some(DisplayInside::FlowRoot))
        },
        ComputedDisplay::TableRowGroup => internal(InternalTableRole::RowGroup),
        ComputedDisplay::TableHeaderGroup => internal(InternalTableRole::HeaderGroup),
        ComputedDisplay::TableFooterGroup => internal(InternalTableRole::FooterGroup),
        ComputedDisplay::TableRow => internal(InternalTableRole::Row),
        ComputedDisplay::TableCell => internal(InternalTableRole::Cell),
        ComputedDisplay::TableColumnGroup => internal(InternalTableRole::ColumnGroup),
        ComputedDisplay::TableColumn => internal(InternalTableRole::Column),
        ComputedDisplay::TableCaption => internal(InternalTableRole::Caption),
    }
}

/// The internal table display values. A replaced element cannot take any
/// of them (CSS 2.1 17.2.1); box generation demotes it to inline and the
/// atomic-inline admission in layout must agree, so both read this.
pub(crate) fn is_internal_table_display(display: ComputedDisplay) -> bool {
    matches!(
        display,
        ComputedDisplay::TableRowGroup
            | ComputedDisplay::TableHeaderGroup
            | ComputedDisplay::TableFooterGroup
            | ComputedDisplay::TableRow
            | ComputedDisplay::TableCell
            | ComputedDisplay::TableColumnGroup
            | ComputedDisplay::TableColumn
            | ComputedDisplay::TableCaption
    )
}

fn positioning_scheme(position: ComputedPosition) -> PositioningScheme {
    match position {
        ComputedPosition::Static => PositioningScheme::Static,
        ComputedPosition::Relative => PositioningScheme::Relative,
        ComputedPosition::Absolute => PositioningScheme::Absolute,
        ComputedPosition::Fixed => PositioningScheme::Fixed,
        ComputedPosition::Sticky => PositioningScheme::Sticky,
    }
}

/// The currently implemented CSS triggers that establish descendant
/// containing blocks. Unsupported triggers are absent from the computed
/// model, so they cannot silently look like an ordinary positioned ancestor.
fn containing_block_establishment(computed: &ComputedValues) -> ContainingBlockEstablishment {
    let transformed = !matches!(computed.transform, livery::values::Transform::None);
    let fixed = transformed || computed.contain.has_layout() || computed.contain.has_paint();
    if fixed {
        ContainingBlockEstablishment::fixed_and_absolute()
    } else if computed.position != ComputedPosition::Static {
        ContainingBlockEstablishment::positioned()
    } else {
        ContainingBlockEstablishment::NONE
    }
}

fn float_side(float: ComputedFloat) -> FloatSide {
    match float {
        ComputedFloat::None => FloatSide::None,
        ComputedFloat::Left => FloatSide::Left,
        ComputedFloat::Right => FloatSide::Right,
    }
}

fn flow_axes(computed: &ComputedValues) -> FlowAxes {
    let writing_mode = match computed.writing_mode {
        ComputedWritingMode::HorizontalTb => WritingMode::HorizontalTb,
        ComputedWritingMode::VerticalRl => WritingMode::VerticalRl,
        ComputedWritingMode::VerticalLr => WritingMode::VerticalLr,
        ComputedWritingMode::SidewaysRl => WritingMode::SidewaysRl,
        ComputedWritingMode::SidewaysLr => WritingMode::SidewaysLr,
    };
    let direction = match computed.direction {
        ComputedDirection::Ltr => Direction::Ltr,
        ComputedDirection::Rtl => Direction::Rtl,
    };
    FlowAxes::new(writing_mode, direction)
}

fn is_replaced_element<D>(dom: &D, node: D::NodeId) -> bool
where
    D: LayoutDom,
{
    dom.element_name(node).is_some_and(|name| {
        name.local.as_ref().eq_ignore_ascii_case("img")
            || name.local.as_ref().eq_ignore_ascii_case("canvas")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Device, InteractionStates, StyleSet, resolve_styles};
    use buckram::ContainingBlock;
    use genet_static_dom::StaticDocument;
    use layout_dom_api::{LocalName, Namespace};

    fn find(
        dom: &StaticDocument,
        node: <StaticDocument as LayoutDom>::NodeId,
        needle: &str,
    ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
        if dom.kind(node) == NodeKind::Element
            && dom.attribute(node, &Namespace::from(""), &LocalName::from("id")) == Some(needle)
        {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| find(dom, child, needle))
    }

    #[test]
    fn generated_tree_applies_suppression_contents_and_comment_rules() {
        fn find(
            dom: &StaticDocument,
            node: <StaticDocument as LayoutDom>::NodeId,
            needle: &str,
        ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
            if dom.kind(node) == NodeKind::Element
                && dom.attribute(node, &Namespace::from(""), &LocalName::from("id")) == Some(needle)
            {
                return Some(node);
            }
            dom.dom_children(node)
                .find_map(|child| find(dom, child, needle))
        }

        let dom = StaticDocument::parse(
            "<html><body id=\"body\">before<!-- boundary --><span id=\"shown\">after</span>\
             <span id=\"hidden\">hidden</span><span id=\"contents\">\
             <b id=\"inside\">inside</b></span></body></html>",
        );
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&["#hidden { display: none; } #contents { display: contents; }"]),
            &Device::screen(800.0, 600.0),
            &InteractionStates::default(),
        );
        let generated = GeneratedBoxTree::from_dom(&dom, &styles);
        let body = find(&dom, dom.document(), "body").expect("body");
        let shown = find(&dom, dom.document(), "shown").expect("shown");
        let hidden = find(&dom, dom.document(), "hidden").expect("hidden");
        let contents = find(&dom, dom.document(), "contents").expect("contents");
        let inside = find(&dom, dom.document(), "inside").expect("inside");
        let body_box = generated.principal_box(body).expect("body principal box");
        let inside_box = generated
            .principal_box(inside)
            .expect("inside principal box");

        assert!(generated.principal_box(shown).is_some());
        assert_eq!(generated.principal_box(hidden), None);
        assert!(generated.boxes_for_node(hidden).is_empty());
        assert_eq!(generated.principal_box(contents), None);
        assert!(generated.boxes_for_node(contents).is_empty());
        assert_eq!(generated[inside_box].parent(), Some(body_box));
    }

    /// Regression for the K4c5a discovery: block-in-inline splitting was
    /// applied to atomic inlines, hoisting a block-level table out of an
    /// enclosing inline-block and leaving the span empty. CSS 2.1 section
    /// 9.2.1.1 splits only non-atomic inlines; an inline-block establishes
    /// its own formatting context and keeps block children.
    ///
    /// The markup is span-based on purpose. A literal `<div>` (or `<table>`)
    /// start tag inside an open `<p>` makes the HTML parser close the
    /// paragraph, restructuring the DOM before box generation ever runs. That
    /// parser behavior is spec-correct and was half of what the original
    /// repro observed; the box-generation split bug was the other half.
    #[test]
    fn an_inline_block_keeps_a_block_level_table_child() {
        let dom = StaticDocument::parse(
            "<p>before <span id=atom><span id=t><span class=row><span class=cell>one</span></span></span></span> after</p>",
        );
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&[
                "#atom { display: inline-block; } #t { display: table; } .row { display: table-row; } .cell { display: table-cell; }",
            ]),
            &Device::screen(800.0, 600.0),
            &InteractionStates::default(),
        );
        let generated = GeneratedBoxTree::from_dom(&dom, &styles);
        let atom = find(&dom, dom.document(), "atom").expect("atom");
        let table = find(&dom, dom.document(), "t").expect("t");

        assert_eq!(
            generated.boxes_for_node(atom).len(),
            1,
            "an atomic inline must not split around its block child"
        );
        let atom_box = generated.principal_box(atom).expect("atom principal box");
        let table_box = generated.principal_box(table).expect("table principal box");
        // The table generates a wrapper, so walk ancestors rather than
        // asserting a direct parent edge.
        let mut ancestor = generated[table_box].parent();
        let mut inside_atom = false;
        while let Some(candidate) = ancestor {
            if candidate == atom_box {
                inside_atom = true;
                break;
            }
            ancestor = generated[candidate].parent();
        }
        assert!(
            inside_atom,
            "the table must stay inside the inline-block, not become its sibling"
        );
    }

    #[test]
    fn generated_boxes_carry_inherited_writing_mode_and_direction() {
        fn find(
            dom: &StaticDocument,
            node: <StaticDocument as LayoutDom>::NodeId,
            needle: &str,
        ) -> Option<<StaticDocument as LayoutDom>::NodeId> {
            if dom.kind(node) == NodeKind::Element
                && dom.attribute(node, &Namespace::from(""), &LocalName::from("id")) == Some(needle)
            {
                return Some(node);
            }
            dom.dom_children(node)
                .find_map(|child| find(dom, child, needle))
        }

        let dom = StaticDocument::parse(
            "<html><body id=\"body\"><span id=\"child\">word</span></body></html>",
        );
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&["#body { writing-mode: sideways-lr; direction: rtl; }"]),
            &Device::screen(800.0, 600.0),
            &InteractionStates::default(),
        );
        let generated = GeneratedBoxTree::from_dom(&dom, &styles);
        let body = find(&dom, dom.document(), "body").expect("body");
        let child = find(&dom, dom.document(), "child").expect("child");
        let text = dom
            .dom_children(child)
            .find(|node| dom.kind(*node) == NodeKind::Text)
            .expect("text");
        let expected = FlowAxes::new(WritingMode::SidewaysLr, Direction::Rtl);

        assert_eq!(
            generated[generated.principal_box(body).expect("body box")].flow,
            expected
        );
        assert_eq!(
            generated[generated.principal_box(child).expect("child box")].flow,
            expected
        );
        assert!(
            generated
                .boxes_for_node(text)
                .iter()
                .all(|box_id| generated[*box_id].flow == expected)
        );
    }

    #[test]
    fn table_display_vocabulary_and_html_ua_roles_reach_buckram() {
        assert_eq!(
            display_role(ComputedDisplay::InlineTable, false).outside,
            Some(DisplayOutside::Inline)
        );
        for (display, role) in [
            (
                ComputedDisplay::TableHeaderGroup,
                InternalTableRole::HeaderGroup,
            ),
            (
                ComputedDisplay::TableFooterGroup,
                InternalTableRole::FooterGroup,
            ),
            (
                ComputedDisplay::TableColumnGroup,
                InternalTableRole::ColumnGroup,
            ),
            (ComputedDisplay::TableColumn, InternalTableRole::Column),
        ] {
            assert_eq!(display_role(display, false).internal_table, Some(role));
            // A replaced element never becomes an internal table box
            // (CSS 2.1 17.2.1); it is treated as inline and an anonymous
            // box is generated around it.
            let demoted = display_role(display, true);
            assert_eq!(demoted.internal_table, None);
            assert_eq!(demoted.outside, Some(DisplayOutside::Inline));
        }

        let dom = StaticDocument::parse(
            "<html><body><table><thead id=\"head\"><tr><th>h</th></tr></thead>\
             <tbody id=\"body\"><tr><td>d</td></tr></tbody><tfoot id=\"foot\">\
             <tr><td>f</td></tr></tfoot><colgroup id=\"group\"><col id=\"column\"></colgroup>\
             </table></body></html>",
        );
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&[]),
            &Device::screen(800.0, 600.0),
            &InteractionStates::default(),
        );
        let generated = GeneratedBoxTree::from_dom(&dom, &styles);
        for (needle, role) in [
            ("head", InternalTableRole::HeaderGroup),
            ("body", InternalTableRole::RowGroup),
            ("foot", InternalTableRole::FooterGroup),
            ("group", InternalTableRole::ColumnGroup),
            ("column", InternalTableRole::Column),
        ] {
            let node = find(&dom, dom.document(), needle).expect(needle);
            let box_id = generated.principal_box(node).expect(needle);
            assert_eq!(generated[box_id].display.internal_table, Some(role));
        }
    }

    #[test]
    fn computed_transform_captures_absolute_and_fixed_descendants_in_buckram() {
        let dom = StaticDocument::parse(
            "<html><body><div id=cb><div id=absolute></div><div id=fixed></div></div></body></html>",
        );
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&[
                "#cb { transform: translateX(0px); } #absolute { position: absolute; } #fixed { position: fixed; }",
            ]),
            &Device::screen(800.0, 600.0),
            &InteractionStates::default(),
        );
        let generated = GeneratedBoxTree::from_dom(&dom, &styles);
        let cb = generated
            .principal_box(find(&dom, dom.document(), "cb").expect("containing block"))
            .expect("containing block box");
        let absolute = generated
            .principal_box(find(&dom, dom.document(), "absolute").expect("absolute"))
            .expect("absolute box");
        let fixed = generated
            .principal_box(find(&dom, dom.document(), "fixed").expect("fixed"))
            .expect("fixed box");

        assert_eq!(
            generated[cb].containing_block_establishment,
            ContainingBlockEstablishment::fixed_and_absolute()
        );
        assert_eq!(
            generated[absolute].containing_block,
            ContainingBlock::Box(cb)
        );
        assert_eq!(generated[fixed].containing_block, ContainingBlock::Box(cb));
    }

    #[test]
    fn computed_paint_containment_captures_absolute_and_fixed_descendants_in_buckram() {
        let dom = StaticDocument::parse(
            "<html><body><div id=cb><div id=absolute></div><div id=fixed></div></div></body></html>",
        );
        let styles = resolve_styles(
            &dom,
            &StyleSet::cambium(&[
                "#cb { contain: paint; } #absolute { position: absolute; } #fixed { position: fixed; }",
            ]),
            &Device::screen(800.0, 600.0),
            &InteractionStates::default(),
        );
        let generated = GeneratedBoxTree::from_dom(&dom, &styles);
        let cb = generated
            .principal_box(find(&dom, dom.document(), "cb").expect("containing block"))
            .expect("containing block box");
        let absolute = generated
            .principal_box(find(&dom, dom.document(), "absolute").expect("absolute"))
            .expect("absolute box");
        let fixed = generated
            .principal_box(find(&dom, dom.document(), "fixed").expect("fixed"))
            .expect("fixed box");

        assert_eq!(
            generated[cb].containing_block_establishment,
            ContainingBlockEstablishment::fixed_and_absolute()
        );
        assert_eq!(
            generated[absolute].containing_block,
            ContainingBlock::Box(cb)
        );
        assert_eq!(generated[fixed].containing_block, ContainingBlock::Box(cb));
    }
}
