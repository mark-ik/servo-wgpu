// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;
use genet_scripted_dom::{NodeId, ScriptedDom};
use layout_dom_api::QualName;
use paint_list_api::PaintList;

fn attr(name: &str) -> QualName {
    QualName::new(None, Namespace::from(""), LocalName::from(name))
}

fn by_id(dom: &ScriptedDom, id: &str) -> NodeId {
    find_id(dom, dom.document(), id).expect("fixture node")
}

fn generated_ids(
    document: &LiveryDocument<ScriptedDom>,
    node: NodeId,
) -> Vec<(buckram::BoxId, Vec<buckram::FragmentId>)> {
    let layout = document.layout.as_ref().expect("completed frame");
    layout
        .fragments
        .boxes()
        .boxes_for_node(node)
        .iter()
        .copied()
        .map(|box_id| {
            (
                box_id,
                layout
                    .fragments
                    .fragments()
                    .fragment_ids_for_box(box_id)
                    .to_vec(),
            )
        })
        .collect()
}

fn assert_table_paint_sources_are_live(document: &LiveryDocument<ScriptedDom>, node: NodeId) {
    let layout = document.layout.as_ref().expect("completed frame");
    let paint = layout
        .fragments
        .table_paint_for_node(node)
        .expect("retained table paint model");
    for source in paint
        .fragments()
        .iter()
        .filter_map(|fragment| fragment.box_id)
    {
        assert!(
            !layout
                .fragments
                .fragments()
                .fragment_ids_for_box(source)
                .is_empty(),
            "each retained table paint source names a live reconciled box",
        );
    }
}

fn table_wrapper_fragment_id(
    document: &LiveryDocument<ScriptedDom>,
    node: NodeId,
) -> buckram::FragmentId {
    let layout = document.layout.as_ref().expect("completed frame");
    let grid = layout
        .fragments
        .boxes()
        .principal_box(node)
        .expect("table grid box");
    let wrapper = layout.fragments.boxes()[grid]
        .parent()
        .expect("table wrapper box");
    assert_eq!(
        layout.fragments.boxes()[wrapper].display.internal_table,
        Some(buckram::InternalTableRole::Wrapper),
    );
    match layout.fragments.fragments().fragment_ids_for_box(wrapper) {
        [fragment] => *fragment,
        fragments => panic!("one table wrapper fragment, got {fragments:?}"),
    }
}

#[test]
fn retained_text_position_query_is_frame_backed_and_selection_free() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=target>retained geometry</div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&["html, body { margin: 0; } #target { width: 120px; height: 24px; }"]),
        Device::screen(160.0, 120.0),
    );
    let target = by_id(document.dom(), "target");
    let source = document
        .dom()
        .dom_children(target)
        .find(|node| document.dom().kind(*node) == NodeKind::Text)
        .expect("target text source");

    assert!(document.text_position_at_point(0.0, 0.0).is_none());
    document.frame(160, 120).expect("retained frame");
    let caret = document.caret_rect(source, 0).expect("retained caret");
    assert_eq!(
        document.text_position_at_point(caret.x, caret.y + caret.height * 0.5),
        Some((source, 0))
    );
    assert!(document.text_selection().is_none());
}

#[test]
fn retained_layout_borrows_the_completed_frame_without_relayout() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=target>retained geometry</div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&["html, body { margin: 0; } #target { width: 120px; height: 24px; }"]),
        Device::screen(160.0, 120.0),
    );

    assert!(document.retained_layout().is_none());
    document.frame(160, 120).expect("completed frame");
    let target = by_id(document.dom(), "target");
    let completed_generation = document.layout_generation();

    assert!(
        document
            .retained_layout()
            .is_some_and(|layout| layout.get(target).is_some())
    );
    assert_eq!(document.layout_generation(), completed_generation);
}

#[test]
fn retained_relayout_keeps_unrelated_and_table_generated_ids_after_sibling_insertion() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body id=body><div id=changed>changed</div><table id=table><tbody><tr><td>cell</td></tr></tbody></table><div id=outside>outside</div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&[
            "body { margin: 0; } table { display: table; border-spacing: 0; } \
             tbody { display: table-row-group; } tr { display: table-row; } \
             td { display: table-cell; width: 40px; height: 20px; }",
        ]),
        Device::screen(240.0, 160.0),
    );
    document.frame(240, 160).expect("initial frame");
    let table = by_id(document.dom(), "table");
    let outside = by_id(document.dom(), "outside");
    let table_before = generated_ids(&document, table);
    let outside_before = generated_ids(&document, outside);
    assert_table_paint_sources_are_live(&document, table);
    assert!(
        table_before.len() >= 2,
        "the table receipt includes its retained wrapper and grid boxes",
    );

    document.mutate_dom(|dom| {
        let body = by_id(dom, "body");
        let changed = by_id(dom, "changed");
        let inserted = dom.create_element(QualName::new(
            None,
            Namespace::from(""),
            LocalName::from("div"),
        ));
        dom.set_attribute(inserted, attr("id"), "inserted");
        let text = dom.create_text("inserted");
        dom.append_child(inserted, text);
        dom.insert_before(body, inserted, Some(changed));
    });
    document.frame(240, 160).expect("inserted-sibling frame");

    assert_eq!(generated_ids(&document, table), table_before);
    assert_eq!(generated_ids(&document, outside), outside_before);
    assert_table_paint_sources_are_live(&document, table);
    assert!(
        !generated_ids(&document, by_id(document.dom(), "inserted")).is_empty(),
        "the inserted sibling receives separate live identities",
    );
}

#[test]
fn retained_mutation_paints_like_a_fresh_final_document() {
    let initial =
        "<html><body id=body><div id=before>before</div><div id=after>after</div></body></html>";
    let final_document = "<html><body id=body><div id=before>before</div><div id=inserted>inserted</div><div id=after>after</div></body></html>";
    let styles = || {
        StyleSet::cambium(&[
            "html, body { margin: 0; padding: 0; } div { width: 100px; height: 20px; } \
             #inserted { background: blue; }",
        ])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(160.0, 120.0));
    retained.frame(160, 120).expect("initial retained frame");

    retained.mutate_dom(|dom| {
        let body = by_id(dom, "body");
        let after = by_id(dom, "after");
        let inserted = dom.create_element(QualName::new(
            None,
            Namespace::from(""),
            LocalName::from("div"),
        ));
        dom.set_attribute(inserted, attr("id"), "inserted");
        let text = dom.create_text("inserted");
        dom.append_child(inserted, text);
        dom.insert_before(body, inserted, Some(after));
    });
    let retained_paint = retained.frame(160, 120).expect("retained final frame");

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(160.0, 120.0));
    let fresh_paint = fresh.frame(160, 120).expect("fresh final frame");

    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "retained mutation and fresh final-document layout must emit the same paint commands",
    );
    assert_eq!(
        retained.content_height(0),
        fresh.content_height(0),
        "the same final DOM has the same retained document extent",
    );
}

#[test]
fn dom_mutation_records_its_nearest_formatting_context_root() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=flex><div id=existing>existing</div></div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&[
            "html, body { margin: 0; padding: 0; } #flex { display: flex; width: 120px; }",
        ]),
        Device::screen(160.0, 120.0),
    );
    document.frame(160, 120).expect("initial frame");
    let flex = by_id(document.dom(), "flex");

    document.mutate_dom(|dom| {
        let flex = by_id(dom, "flex");
        let inserted = dom.create_element(QualName::new(
            None,
            Namespace::from(""),
            LocalName::from("div"),
        ));
        dom.append_child(flex, inserted);
    });

    assert_eq!(
        document.last_layout_damage(),
        Some(&LayoutDamage {
            kind: LayoutDamageKind::Dom,
            roots: vec![flex],
            full_document: false,
        }),
        "K5h records the flex root whose child list changed",
    );
}

#[test]
fn retained_formatting_root_splice_refreshes_descendants_and_keeps_outside_identity() {
    let initial = "<html><body><div id=flex><div id=child>child</div></div><div id=outside>outside</div></body></html>";
    let final_document = "<html><body><div id=flex style=\"width: 180px\"><div id=child>child</div></div><div id=outside>outside</div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #flex { display: flex; width: 100px; height: 40px; background: red; } \
             #child { width: 40px; height: 20px; background: blue; } \
             #outside { width: 80px; height: 20px; background: green; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 120.0));
    retained.frame(240, 120).expect("initial retained frame");
    let flex = by_id(retained.dom(), "flex");
    let child = by_id(retained.dom(), "child");
    let outside = by_id(retained.dom(), "outside");
    let flex_before = generated_ids(&retained, flex);
    let child_before = generated_ids(&retained, child);
    let outside_before = generated_ids(&retained, outside);
    let layout_generation = retained.layout_generation();

    retained.mutate_dom(|dom| {
        dom.set_attribute(by_id(dom, "flex"), attr("style"), "width: 180px");
    });
    assert_eq!(
        retained.last_layout_damage(),
        Some(&LayoutDamage {
            kind: LayoutDamageKind::Dom,
            roots: vec![flex],
            full_document: false,
        })
    );
    let retained_paint = retained.frame(240, 120).expect("spliced retained frame");

    assert_eq!(retained.layout_generation(), layout_generation + 1);
    assert_eq!(generated_ids(&retained, flex), flex_before);
    assert_ne!(
        generated_ids(&retained, child),
        child_before,
        "the selected formatting root receives fresh descendant fragments"
    );
    assert_eq!(generated_ids(&retained, outside), outside_before);
    assert!(
        retained.identity_source.is_none() && !retained.layout_dirty,
        "the selected root was published into the retained layout"
    );

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 120.0));
    let fresh_paint = fresh.frame(240, 120).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "the spliced root must paint like a fresh final document",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn retained_flex_root_splice_accepts_an_inserted_child_box() {
    let initial = "<html><body><div id=flex><div id=existing>existing</div></div><div id=outside>outside</div></body></html>";
    let final_document = "<html><body><div id=flex><div id=existing>existing</div><div id=inserted>inserted</div></div><div id=outside>outside</div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #flex { display: flex; width: 180px; height: 40px; background: red; } \
             #existing, #inserted { width: 60px; height: 20px; background: blue; } \
             #outside { width: 80px; height: 20px; background: green; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 120.0));
    retained.frame(240, 120).expect("initial retained frame");
    let flex = by_id(retained.dom(), "flex");
    let existing = by_id(retained.dom(), "existing");
    let outside = by_id(retained.dom(), "outside");
    let flex_before = generated_ids(&retained, flex);
    let existing_before = generated_ids(&retained, existing);
    let outside_before = generated_ids(&retained, outside);
    let local_generation = retained.retained_root_relayout_generation;

    retained.mutate_dom(|dom| {
        let flex = by_id(dom, "flex");
        let inserted = dom.create_element(QualName::new(
            None,
            Namespace::from(""),
            LocalName::from("div"),
        ));
        dom.set_attribute(inserted, attr("id"), "inserted");
        let text = dom.create_text("inserted");
        dom.append_child(inserted, text);
        dom.append_child(flex, inserted);
    });
    assert_eq!(
        retained.last_layout_damage(),
        Some(&LayoutDamage {
            kind: LayoutDamageKind::Dom,
            roots: vec![flex],
            full_document: false,
        })
    );
    let retained_paint = retained.frame(240, 120).expect("spliced retained frame");

    assert_eq!(
        retained.retained_root_relayout_generation,
        local_generation + 1,
        "the text-bearing flex root takes the selected-root formatter",
    );
    assert_eq!(generated_ids(&retained, flex), flex_before);
    assert_ne!(generated_ids(&retained, existing), existing_before);
    assert_eq!(generated_ids(&retained, outside), outside_before);
    assert!(
        !generated_ids(&retained, by_id(retained.dom(), "inserted")).is_empty(),
        "the new child is published through the fresh selected-root box tree",
    );
    assert!(retained.text_target("existing").is_some());
    assert!(retained.text_target("inserted").is_some());
    assert!(retained.text_target("outside").is_some());

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 120.0));
    let fresh_paint = fresh.frame(240, 120).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "the selected flex-root splice must paint like a fresh structural mutation",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn retained_grid_root_splice_accepts_an_inserted_child_box() {
    let initial = "<html><body><div id=grid><div id=existing>existing</div></div><div id=outside>outside</div></body></html>";
    let final_document = "<html><body><div id=grid><div id=existing>existing</div><div id=inserted>inserted</div></div><div id=outside>outside</div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #grid { display: grid; grid-template-columns: 60px 60px; width: 180px; height: 40px; background: red; } \
             #existing, #inserted { width: 60px; height: 20px; background: blue; } \
             #outside { width: 80px; height: 20px; background: green; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 120.0));
    retained.frame(240, 120).expect("initial retained frame");
    let grid = by_id(retained.dom(), "grid");
    let existing = by_id(retained.dom(), "existing");
    let outside = by_id(retained.dom(), "outside");
    let grid_before = generated_ids(&retained, grid);
    let existing_before = generated_ids(&retained, existing);
    let outside_before = generated_ids(&retained, outside);
    let local_generation = retained.retained_root_relayout_generation;

    retained.mutate_dom(|dom| {
        let grid = by_id(dom, "grid");
        let inserted = dom.create_element(QualName::new(
            None,
            Namespace::from(""),
            LocalName::from("div"),
        ));
        dom.set_attribute(inserted, attr("id"), "inserted");
        let text = dom.create_text("inserted");
        dom.append_child(inserted, text);
        dom.append_child(grid, inserted);
    });
    assert_eq!(
        retained.last_layout_damage(),
        Some(&LayoutDamage {
            kind: LayoutDamageKind::Dom,
            roots: vec![grid],
            full_document: false,
        })
    );
    let retained_paint = retained.frame(240, 120).expect("spliced retained frame");

    assert_eq!(
        retained.retained_root_relayout_generation,
        local_generation + 1,
        "the text-bearing grid root takes the selected-root formatter",
    );
    assert_eq!(generated_ids(&retained, grid), grid_before);
    assert_ne!(generated_ids(&retained, existing), existing_before);
    assert_eq!(generated_ids(&retained, outside), outside_before);
    assert!(
        !generated_ids(&retained, by_id(retained.dom(), "inserted")).is_empty(),
        "the new child is published through the fresh selected-root box tree",
    );
    assert!(retained.text_target("existing").is_some());
    assert!(retained.text_target("inserted").is_some());
    assert!(retained.text_target("outside").is_some());

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 120.0));
    let fresh_paint = fresh.frame(240, 120).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "the selected grid-root splice must paint like a fresh structural mutation",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn retained_root_formatter_adds_its_first_text_source_in_dom_order() {
    let initial = "<html><body><div id=flex><div id=existing></div></div><div id=outside>outside</div></body></html>";
    let final_document = "<html><body><div id=flex><div id=existing></div><div id=inserted>inside</div></div><div id=outside>outside</div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #flex { display: flex; width: 180px; height: 40px; background: red; } \
             #existing, #inserted { width: 60px; height: 20px; background: blue; } \
             #outside { width: 80px; height: 20px; background: green; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 120.0));
    retained.frame(240, 120).expect("initial retained frame");
    let flex = by_id(retained.dom(), "flex");
    let existing = by_id(retained.dom(), "existing");
    let outside = by_id(retained.dom(), "outside");
    let flex_before = generated_ids(&retained, flex);
    let existing_before = generated_ids(&retained, existing);
    let outside_before = generated_ids(&retained, outside);
    let local_generation = retained.retained_root_relayout_generation;

    retained.mutate_dom(|dom| {
        let flex = by_id(dom, "flex");
        let inserted = dom.create_element(QualName::new(
            None,
            Namespace::from(""),
            LocalName::from("div"),
        ));
        dom.set_attribute(inserted, attr("id"), "inserted");
        let text = dom.create_text("inside");
        dom.append_child(inserted, text);
        dom.append_child(flex, inserted);
    });
    let retained_paint = retained.frame(240, 120).expect("locally formatted frame");

    assert_eq!(
        retained.retained_root_relayout_generation,
        local_generation + 1,
        "the first text source takes the selected-root formatter instead of the complete-layout publication path",
    );
    assert_eq!(generated_ids(&retained, flex), flex_before);
    assert_ne!(generated_ids(&retained, existing), existing_before);
    assert_eq!(generated_ids(&retained, outside), outside_before);
    assert!(
        !generated_ids(&retained, by_id(retained.dom(), "inserted")).is_empty(),
        "the selected-root formatter publishes the inserted descendant",
    );
    assert!(retained.text_target("inside").is_some());
    assert!(retained.text_target("outside").is_some());
    let inserted_text = retained
        .dom()
        .dom_children(by_id(retained.dom(), "inserted"))
        .find(|node| retained.dom().kind(*node) == NodeKind::Text)
        .expect("inserted text source");
    let outside_text = retained
        .dom()
        .dom_children(by_id(retained.dom(), "outside"))
        .find(|node| retained.dom().kind(*node) == NodeKind::Text)
        .expect("outside text source");
    let text_order = retained
        .layout
        .as_ref()
        .and_then(|layout| layout.fragments.text_frame())
        .expect("retained text frame")
        .text_order();
    let inserted_index = text_order
        .iter()
        .position(|source| *source == inserted_text)
        .expect("inserted source stays ordered");
    let outside_index = text_order
        .iter()
        .position(|source| *source == outside_text)
        .expect("outside source stays ordered");
    assert!(inserted_index < outside_index);

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 120.0));
    let fresh_paint = fresh.frame(240, 120).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "the locally formatted flex root paints like a fresh final document",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn retained_root_formatter_reflows_a_text_free_grid_subtree() {
    let initial = "<html><body><div id=grid><div id=existing></div></div><div id=outside></div></body></html>";
    let final_document = "<html><body><div id=grid><div id=existing></div><div id=inserted></div></div><div id=outside></div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #grid { display: grid; grid-template-columns: 60px 60px; width: 180px; height: 40px; background: red; } \
             #existing, #inserted { width: 60px; height: 20px; background: blue; } \
             #outside { width: 80px; height: 20px; background: green; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 120.0));
    retained.frame(240, 120).expect("initial retained frame");
    let grid = by_id(retained.dom(), "grid");
    let existing = by_id(retained.dom(), "existing");
    let outside = by_id(retained.dom(), "outside");
    let grid_before = generated_ids(&retained, grid);
    let existing_before = generated_ids(&retained, existing);
    let outside_before = generated_ids(&retained, outside);
    let local_generation = retained.retained_root_relayout_generation;

    retained.mutate_dom(|dom| {
        let grid = by_id(dom, "grid");
        let inserted = dom.create_element(QualName::new(
            None,
            Namespace::from(""),
            LocalName::from("div"),
        ));
        dom.set_attribute(inserted, attr("id"), "inserted");
        dom.append_child(grid, inserted);
    });
    let retained_paint = retained.frame(240, 120).expect("locally formatted frame");

    assert_eq!(
        retained.retained_root_relayout_generation,
        local_generation + 1,
        "the text-free grid mutation takes the selected-root formatter instead of the complete-layout publication path",
    );
    assert_eq!(generated_ids(&retained, grid), grid_before);
    assert_ne!(generated_ids(&retained, existing), existing_before);
    assert_eq!(generated_ids(&retained, outside), outside_before);
    assert!(
        !generated_ids(&retained, by_id(retained.dom(), "inserted")).is_empty(),
        "the selected-root formatter publishes the inserted descendant",
    );

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 120.0));
    let fresh_paint = fresh.frame(240, 120).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "the locally formatted grid root paints like a fresh final document",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn retained_root_formatter_drops_retired_text_sources() {
    let initial = "<html><body><div id=flex><div id=removed>remove me</div><div id=survives>survives</div></div><div id=outside>outside</div></body></html>";
    let final_document = "<html><body><div id=flex><div id=survives>survives</div></div><div id=outside>outside</div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #flex { display: flex; width: 180px; height: 40px; background: red; } \
             #removed, #survives { width: 60px; height: 20px; background: blue; } \
             #outside { width: 80px; height: 20px; background: green; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 120.0));
    retained.frame(240, 120).expect("initial retained frame");
    let flex = by_id(retained.dom(), "flex");
    let survives = by_id(retained.dom(), "survives");
    let outside = by_id(retained.dom(), "outside");
    let flex_before = generated_ids(&retained, flex);
    let survives_before = generated_ids(&retained, survives);
    let outside_before = generated_ids(&retained, outside);
    let local_generation = retained.retained_root_relayout_generation;

    retained.mutate_dom(|dom| dom.remove_child(by_id(dom, "removed")));
    let retained_paint = retained.frame(240, 120).expect("locally formatted frame");

    assert_eq!(
        retained.retained_root_relayout_generation,
        local_generation + 1,
        "removing text takes the selected-root formatter",
    );
    assert_eq!(generated_ids(&retained, flex), flex_before);
    assert_ne!(generated_ids(&retained, survives), survives_before);
    assert_eq!(generated_ids(&retained, outside), outside_before);
    assert!(retained.text_target("remove me").is_none());
    assert!(retained.text_target("survives").is_some());
    assert!(retained.text_target("outside").is_some());

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 120.0));
    let fresh_paint = fresh.frame(240, 120).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "the retained frame drops text whose selected subtree retired",
    );
    assert_eq!(
        format!("{:?}", retained_paint.fonts()),
        format!("{:?}", fresh_paint.fonts()),
        "retired text cannot retain a font resource in the paint list",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn retained_root_formatter_updates_fixed_root_overflow() {
    let initial = "<html><body><div id=flex><div id=existing></div></div><div id=outside></div></body></html>";
    let final_document = "<html><body><div id=flex><div id=existing></div><div id=inserted></div></div><div id=outside></div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #flex { display: flex; flex-direction: column; width: 100px; height: 40px; background: red; } \
             #existing { flex-shrink: 0; width: 100px; height: 20px; background: blue; } \
             #inserted { flex-shrink: 0; width: 100px; height: 100px; background: blue; } \
             #outside { width: 80px; height: 20px; background: green; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(160.0, 120.0));
    retained.frame(160, 120).expect("initial retained frame");
    let flex = by_id(retained.dom(), "flex");
    let outside = by_id(retained.dom(), "outside");
    let flex_before = generated_ids(&retained, flex);
    let outside_before = generated_ids(&retained, outside);
    let content_height = retained.content_height(0);
    let local_generation = retained.retained_root_relayout_generation;

    retained.mutate_dom(|dom| {
        let flex = by_id(dom, "flex");
        let inserted = dom.create_element(QualName::new(
            None,
            Namespace::from(""),
            LocalName::from("div"),
        ));
        dom.set_attribute(inserted, attr("id"), "inserted");
        dom.append_child(flex, inserted);
    });
    let retained_paint = retained.frame(160, 120).expect("locally formatted frame");

    assert_eq!(
        retained.retained_root_relayout_generation,
        local_generation + 1,
        "fixed-size overflow takes the selected-root formatter",
    );
    assert_eq!(generated_ids(&retained, flex), flex_before);
    assert_eq!(generated_ids(&retained, outside), outside_before);
    assert!(retained.content_height(0) > content_height);

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(160.0, 120.0));
    let fresh_paint = fresh.frame(160, 120).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "overflow from the locally formatted root paints like a fresh document",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn retained_root_formatter_promotes_a_changed_size_to_its_block_parent() {
    let initial = "<html><body><div id=host><div id=flex><div id=existing></div></div><div id=after></div></div><div id=outside></div></body></html>";
    let final_document = "<html><body><div id=host><div id=flex><div id=existing></div><div id=inserted></div></div><div id=after></div></div><div id=outside></div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #host { width: 160px; height: 120px; background: red; } \
             #flex { display: flex; flex-direction: column; width: 100px; background: blue; } \
             #existing { flex-shrink: 0; width: 100px; height: 20px; } \
             #inserted { flex-shrink: 0; width: 100px; height: 60px; } \
             #after { width: 100px; height: 20px; background: yellow; } \
             #outside { width: 80px; height: 20px; background: green; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 180.0));
    retained.frame(240, 180).expect("initial retained frame");
    let host = by_id(retained.dom(), "host");
    let flex = by_id(retained.dom(), "flex");
    let after = by_id(retained.dom(), "after");
    let outside = by_id(retained.dom(), "outside");
    let host_before = generated_ids(&retained, host);
    let flex_before = generated_ids(&retained, flex);
    let after_before = generated_ids(&retained, after);
    let outside_before = generated_ids(&retained, outside);
    let local_generation = retained.retained_root_relayout_generation;

    retained.mutate_dom(|dom| {
        let flex = by_id(dom, "flex");
        let inserted = dom.create_element(QualName::new(
            None,
            Namespace::from(""),
            LocalName::from("div"),
        ));
        dom.set_attribute(inserted, attr("id"), "inserted");
        dom.append_child(flex, inserted);
    });
    assert_eq!(
        retained.last_layout_damage(),
        Some(&LayoutDamage {
            kind: LayoutDamageKind::Dom,
            roots: vec![flex],
            full_document: false,
        })
    );
    let retained_paint = retained.frame(240, 180).expect("promoted retained frame");

    assert_eq!(
        retained.retained_root_relayout_generation,
        local_generation + 1,
        "a changed flex used size promotes to its block formatting parent",
    );
    assert_eq!(generated_ids(&retained, host), host_before);
    assert_ne!(generated_ids(&retained, flex), flex_before);
    assert_ne!(generated_ids(&retained, after), after_before);
    assert_eq!(generated_ids(&retained, outside), outside_before);

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 180.0));
    let fresh_paint = fresh.frame(240, 180).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "the promoted block root paints like a fresh final document",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn retained_root_formatter_promotes_through_a_changed_parent_to_a_stable_ancestor() {
    let initial = "<html><body><div id=host><div id=parent><div id=flex><div id=existing></div></div><div id=after></div></div></div><div id=outside></div></body></html>";
    let final_document = "<html><body><div id=host><div id=parent><div id=flex><div id=existing></div><div id=inserted></div></div><div id=after></div></div></div><div id=outside></div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #host { width: 160px; height: 160px; background: red; } \
             #parent { width: 160px; background: orange; } \
             #flex { display: flex; flex-direction: column; width: 100px; background: blue; } \
             #existing { flex-shrink: 0; width: 100px; height: 20px; } \
             #inserted { flex-shrink: 0; width: 100px; height: 60px; } \
             #after { width: 100px; height: 20px; background: yellow; } \
             #outside { width: 80px; height: 20px; background: green; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 220.0));
    retained.frame(240, 220).expect("initial retained frame");
    let host = by_id(retained.dom(), "host");
    let parent = by_id(retained.dom(), "parent");
    let flex = by_id(retained.dom(), "flex");
    let after = by_id(retained.dom(), "after");
    let outside = by_id(retained.dom(), "outside");
    let host_before = generated_ids(&retained, host);
    let parent_before = generated_ids(&retained, parent);
    let flex_before = generated_ids(&retained, flex);
    let after_before = generated_ids(&retained, after);
    let outside_before = generated_ids(&retained, outside);
    let local_generation = retained.retained_root_relayout_generation;

    retained.mutate_dom(|dom| {
        let flex = by_id(dom, "flex");
        let inserted = dom.create_element(QualName::new(
            None,
            Namespace::from(""),
            LocalName::from("div"),
        ));
        dom.set_attribute(inserted, attr("id"), "inserted");
        dom.append_child(flex, inserted);
    });
    assert_eq!(
        retained.last_layout_damage(),
        Some(&LayoutDamage {
            kind: LayoutDamageKind::Dom,
            roots: vec![flex],
            full_document: false,
        })
    );
    let retained_paint = retained
        .frame(240, 220)
        .expect("ancestor-promoted retained frame");

    assert_eq!(
        retained.retained_root_relayout_generation,
        local_generation + 1,
        "the formatter promotes only after each selected root grows",
    );
    assert_eq!(generated_ids(&retained, host), host_before);
    assert_ne!(generated_ids(&retained, parent), parent_before);
    assert_ne!(generated_ids(&retained, flex), flex_before);
    assert_ne!(generated_ids(&retained, after), after_before);
    assert_eq!(generated_ids(&retained, outside), outside_before);

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 220.0));
    let fresh_paint = fresh.frame(240, 220).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "the stable ancestor paints like a fresh final document",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn retained_root_formatter_replaces_a_fixed_size_table_and_its_paint_plane() {
    let initial = "<html><body><table id=table><tbody><tr id=row><td id=first></td></tr></tbody></table><div id=outside></div></body></html>";
    let final_document = "<html><body><table id=table><tbody><tr id=row><td id=first></td><td id=second></td></tr></tbody></table><div id=outside></div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             table { display: table; table-layout: fixed; width: 120px; height: 80px; border-spacing: 0; background: blue; } \
             tbody { display: table-row-group; } tr { display: table-row; } \
             td { display: table-cell; width: 40px; height: 20px; background: yellow; } \
             #outside { width: 80px; height: 20px; background: green; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 180.0));
    retained.frame(240, 180).expect("initial retained frame");
    let table = by_id(retained.dom(), "table");
    let row = by_id(retained.dom(), "row");
    let first = by_id(retained.dom(), "first");
    let outside = by_id(retained.dom(), "outside");
    let wrapper_before = table_wrapper_fragment_id(&retained, table);
    let table_before = generated_ids(&retained, table);
    let first_before = generated_ids(&retained, first);
    let outside_before = generated_ids(&retained, outside);
    let local_generation = retained.retained_root_relayout_generation;
    assert_table_paint_sources_are_live(&retained, table);

    retained.mutate_dom(|dom| {
        let row = by_id(dom, "row");
        let cell = dom.create_element(QualName::new(
            None,
            Namespace::from(""),
            LocalName::from("td"),
        ));
        dom.set_attribute(cell, attr("id"), "second");
        dom.append_child(row, cell);
    });
    assert_eq!(
        retained.last_layout_damage(),
        Some(&LayoutDamage {
            kind: LayoutDamageKind::Dom,
            roots: vec![row],
            full_document: false,
        })
    );
    let retained_paint = retained.frame(240, 180).expect("retained table frame");

    assert_eq!(
        retained.retained_root_relayout_generation,
        local_generation + 1,
        "the fixed-size table uses the selected-root formatter",
    );
    assert_eq!(table_wrapper_fragment_id(&retained, table), wrapper_before);
    assert_ne!(generated_ids(&retained, table), table_before);
    assert_ne!(generated_ids(&retained, first), first_before);
    assert_eq!(generated_ids(&retained, outside), outside_before);
    assert_table_paint_sources_are_live(&retained, table);

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 180.0));
    let fresh_paint = fresh.frame(240, 180).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "the retained table paint plane matches a fresh final document",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn retained_root_formatter_replaces_a_captioned_fixed_size_table() {
    let initial = "<html><body><table id=table><caption id=caption>caption</caption><tbody><tr id=row><td id=first></td></tr></tbody></table><div id=outside></div></body></html>";
    let final_document = "<html><body><table id=table><caption id=caption>caption</caption><tbody><tr id=row><td id=first></td><td id=second></td></tr></tbody></table><div id=outside></div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             table { display: table; table-layout: fixed; width: 120px; height: 80px; border-spacing: 0; background: blue; } \
             caption { display: table-caption; width: 120px; height: 20px; background: red; } \
             tbody { display: table-row-group; } tr { display: table-row; } \
             td { display: table-cell; width: 40px; height: 20px; background: yellow; } \
             #outside { width: 80px; height: 20px; background: green; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 200.0));
    retained.frame(240, 200).expect("initial retained frame");
    let table = by_id(retained.dom(), "table");
    let row = by_id(retained.dom(), "row");
    let caption = by_id(retained.dom(), "caption");
    let outside = by_id(retained.dom(), "outside");
    let wrapper_before = table_wrapper_fragment_id(&retained, table);
    let caption_before = generated_ids(&retained, caption);
    let outside_before = generated_ids(&retained, outside);
    let local_generation = retained.retained_root_relayout_generation;

    retained.mutate_dom(|dom| {
        let row = by_id(dom, "row");
        let cell = dom.create_element(QualName::new(
            None,
            Namespace::from(""),
            LocalName::from("td"),
        ));
        dom.set_attribute(cell, attr("id"), "second");
        dom.append_child(row, cell);
    });
    assert_eq!(
        retained.last_layout_damage(),
        Some(&LayoutDamage {
            kind: LayoutDamageKind::Dom,
            roots: vec![row],
            full_document: false,
        })
    );
    let retained_paint = retained.frame(240, 200).expect("retained table frame");

    assert_eq!(
        retained.retained_root_relayout_generation,
        local_generation + 1,
        "a stable table wrapper admits its caption and grid together",
    );
    assert_eq!(table_wrapper_fragment_id(&retained, table), wrapper_before);
    assert_ne!(generated_ids(&retained, caption), caption_before);
    assert_eq!(generated_ids(&retained, outside), outside_before);

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 200.0));
    let fresh_paint = fresh.frame(240, 200).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "the captioned table paints like a fresh final document",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn retained_table_root_keeps_an_unrelated_table_paint_plane_live() {
    let initial = "<html><body><table id=changed><tbody><tr id=row><td id=first></td></tr></tbody></table><table id=other><tbody><tr><td id=other-cell></td></tr></tbody></table></body></html>";
    let final_document = "<html><body><table id=changed><tbody><tr id=row><td id=first></td><td id=second></td></tr></tbody></table><table id=other><tbody><tr><td id=other-cell></td></tr></tbody></table></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             table { display: table; table-layout: fixed; width: 120px; height: 80px; border-spacing: 0; background: blue; } \
             tbody { display: table-row-group; } tr { display: table-row; } \
             td { display: table-cell; width: 40px; height: 20px; background: yellow; } \
             #other-cell { position: absolute; top: 0; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 220.0));
    retained.frame(240, 220).expect("initial retained frame");
    let changed = by_id(retained.dom(), "changed");
    let row = by_id(retained.dom(), "row");
    let other = by_id(retained.dom(), "other");
    let changed_wrapper_before = table_wrapper_fragment_id(&retained, changed);
    let other_wrapper_before = table_wrapper_fragment_id(&retained, other);
    let other_before = generated_ids(&retained, other);
    let local_generation = retained.retained_root_relayout_generation;
    assert_table_paint_sources_are_live(&retained, changed);
    assert_table_paint_sources_are_live(&retained, other);
    let initial_ledger = retained
        .table_shadow_ledger()
        .expect("completed table ledger");
    assert_eq!(
        initial_ledger.assigned, 2,
        "one contribution per live table"
    );
    assert_eq!(
        initial_ledger.honored, 1,
        "the zero-contribution table has no cell track to verify",
    );
    assert!(
        initial_ledger.positioning_gaps.is_empty(),
        "the untouched table's absolute cell has a shared K5 route",
    );

    retained.mutate_dom(|dom| {
        let row = by_id(dom, "row");
        let cell = dom.create_element(QualName::new(
            None,
            Namespace::from(""),
            LocalName::from("td"),
        ));
        dom.set_attribute(cell, attr("id"), "second");
        dom.append_child(row, cell);
    });
    assert_eq!(
        retained.last_layout_damage(),
        Some(&LayoutDamage {
            kind: LayoutDamageKind::Dom,
            roots: vec![row],
            full_document: false,
        })
    );
    let retained_paint = retained.frame(240, 220).expect("retained table frame");

    assert_eq!(
        retained.retained_root_relayout_generation,
        local_generation + 1,
        "one canonical table contribution can be replaced in place",
    );
    assert_eq!(
        table_wrapper_fragment_id(&retained, changed),
        changed_wrapper_before,
    );
    assert_eq!(
        table_wrapper_fragment_id(&retained, other),
        other_wrapper_before
    );
    assert_eq!(generated_ids(&retained, other), other_before);
    assert_table_paint_sources_are_live(&retained, changed);
    assert_table_paint_sources_are_live(&retained, other);
    let retained_ledger = retained
        .table_shadow_ledger()
        .expect("retained table ledger");
    assert_eq!(
        retained_ledger.assigned, 2,
        "aggregate keeps both table entries"
    );
    assert_eq!(
        retained_ledger.honored, 1,
        "the untouched zero-contribution table has no cell track to verify",
    );
    assert!(
        retained_ledger.positioning_gaps.is_empty(),
        "the untouched table keeps its shared K5 positioning route",
    );

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 220.0));
    let fresh_paint = fresh.frame(240, 220).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "both retained table paint planes match a fresh final document",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn sticky_table_cell_uses_its_nested_scrollport_without_relayout() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=scroller><div id=content><div id=spacer></div><table id=table><tbody><tr><td id=sticky>sticky</td></tr><tr><td id=tail></td></tr></tbody></table></div></div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #scroller { height: 80px; overflow-y: auto; } \
             #spacer { height: 120px; } \
             table { display: table; table-layout: fixed; width: 100px; border-spacing: 0; } \
             tbody { display: table-row-group; } tr { display: table-row; } \
             td { display: table-cell; width: 100px; } \
             #sticky { position: sticky; top: 0; height: 20px; background: red; } \
             #tail { height: 180px; background: blue; }"]),
        Device::screen(160.0, 120.0),
    );
    document.frame(160, 120).expect("initial table frame");
    let scroller = by_id(document.dom(), "scroller");
    let table = by_id(document.dom(), "table");
    let sticky = by_id(document.dom(), "sticky");
    let ids_before = generated_ids(&document, sticky);
    let table_wrapper_before = table_wrapper_fragment_id(&document, table);
    let static_rect = document
        .layout
        .as_ref()
        .and_then(|layout| layout.fragments.get(sticky))
        .map(|fragment| fragment.physical_rect())
        .expect("static table cell fragment");
    assert_eq!(static_rect.y, 120.0);
    assert!(
        !document
            .table_shadow_ledger()
            .expect("table ledger")
            .positioning_gaps
            .iter()
            .any(|record| record.gap == crate::table_shadow::TablePositioningGap::Sticky),
        "table-cell sticky uses the shared retained sticky solver",
    );

    assert!(document.scroll_at(10.0, 10.0, 0.0, 150.0));
    assert_eq!(
        document.element_scroll().get(&scroller),
        Some(&(0.0, 150.0))
    );
    document.frame(160, 120).expect("scrolled table frame");

    let active = document.sticky_layout(document.layout.as_ref().expect("retained layout"));
    let sticky_rect = active
        .get(sticky)
        .map(|fragment| fragment.physical_rect())
        .expect("active table cell fragment");
    assert_eq!(sticky_rect.y, 150.0);
    assert_eq!(sticky_rect.y - document.element_scroll()[&scroller].1, 0.0);
    assert_eq!(generated_ids(&document, sticky), ids_before);
    assert_eq!(
        table_wrapper_fragment_id(&document, table),
        table_wrapper_before
    );
    assert_eq!(
        document
            .layout
            .as_ref()
            .and_then(|layout| layout.fragments.get(sticky))
            .map(|fragment| fragment.physical_rect()),
        Some(static_rect),
        "scrolling keeps the retained table base layout unchanged",
    );
}

#[test]
fn sticky_table_row_moves_its_cell_subtree_without_relayout() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=scroller><div id=content><div id=spacer></div><table id=table><tbody><tr id=sticky-row><td id=sticky-cell>sticky</td></tr><tr><td id=tail-cell></td></tr></tbody></table></div></div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #scroller { height: 80px; overflow-y: auto; } \
             #spacer { height: 120px; } \
             table { display: table; table-layout: fixed; width: 100px; border-spacing: 0; } \
             tbody { display: table-row-group; } tr { display: table-row; } \
             td { display: table-cell; width: 100px; } \
             #sticky-row { position: sticky; top: 0; height: 20px; background: red; } \
             #tail-cell { height: 180px; background: blue; }"]),
        Device::screen(160.0, 120.0),
    );
    document.frame(160, 120).expect("initial table frame");
    let scroller = by_id(document.dom(), "scroller");
    let table = by_id(document.dom(), "table");
    let sticky_row = by_id(document.dom(), "sticky-row");
    let sticky_cell = by_id(document.dom(), "sticky-cell");
    let row_ids_before = generated_ids(&document, sticky_row);
    let cell_ids_before = generated_ids(&document, sticky_cell);
    let table_wrapper_before = table_wrapper_fragment_id(&document, table);
    let static_row = document
        .layout
        .as_ref()
        .and_then(|layout| layout.fragments.get(sticky_row))
        .map(|fragment| fragment.physical_rect())
        .expect("static table row fragment");
    assert_eq!(static_row.y, 120.0);
    assert!(
        !document
            .table_shadow_ledger()
            .expect("table ledger")
            .positioning_gaps
            .iter()
            .any(|record| record.gap == crate::table_shadow::TablePositioningGap::Sticky),
        "row sticky uses the shared retained sticky solver",
    );

    assert!(document.scroll_at(10.0, 10.0, 0.0, 150.0));
    document.frame(160, 120).expect("scrolled table frame");

    let active = document.sticky_layout(document.layout.as_ref().expect("retained layout"));
    let row_rect = active
        .get(sticky_row)
        .map(|fragment| fragment.physical_rect())
        .expect("active table row fragment");
    let cell_rect = active
        .get(sticky_cell)
        .map(|fragment| fragment.physical_rect())
        .expect("active table cell fragment");
    assert_eq!(row_rect.y, 150.0);
    assert_eq!(cell_rect.y, 150.0);
    assert_eq!(row_rect.y - document.element_scroll()[&scroller].1, 0.0);
    assert_eq!(generated_ids(&document, sticky_row), row_ids_before);
    assert_eq!(generated_ids(&document, sticky_cell), cell_ids_before);
    assert_eq!(
        table_wrapper_fragment_id(&document, table),
        table_wrapper_before
    );
    assert_eq!(
        document
            .layout
            .as_ref()
            .and_then(|layout| layout.fragments.get(sticky_row))
            .map(|fragment| fragment.physical_rect()),
        Some(static_row),
        "scrolling keeps the retained table row base layout unchanged",
    );
}

#[test]
fn sticky_table_row_group_moves_its_row_subtree_without_relayout() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=scroller><div id=content><div id=spacer></div><table id=table><tbody id=sticky-group><tr id=sticky-row><td id=sticky-cell>sticky</td></tr></tbody><tbody><tr><td id=tail-cell></td></tr></tbody></table></div></div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #scroller { height: 80px; overflow-y: auto; } \
             #spacer { height: 120px; } \
             table { display: table; table-layout: fixed; width: 100px; border-spacing: 0; } \
             tbody { display: table-row-group; } tr { display: table-row; } \
             td { display: table-cell; width: 100px; } \
             #sticky-group { position: sticky; top: 0; } \
             #sticky-row { height: 20px; background: red; } \
             #tail-cell { height: 180px; background: blue; }"]),
        Device::screen(160.0, 120.0),
    );
    document.frame(160, 120).expect("initial table frame");
    let scroller = by_id(document.dom(), "scroller");
    let table = by_id(document.dom(), "table");
    let sticky_group = by_id(document.dom(), "sticky-group");
    let sticky_row = by_id(document.dom(), "sticky-row");
    let sticky_cell = by_id(document.dom(), "sticky-cell");
    let group_ids_before = generated_ids(&document, sticky_group);
    let row_ids_before = generated_ids(&document, sticky_row);
    let cell_ids_before = generated_ids(&document, sticky_cell);
    let table_wrapper_before = table_wrapper_fragment_id(&document, table);
    let static_group = document
        .layout
        .as_ref()
        .and_then(|layout| layout.fragments.get(sticky_group))
        .map(|fragment| fragment.physical_rect())
        .expect("static table row-group fragment");
    assert_eq!(static_group.y, 120.0);
    assert!(
        !document
            .table_shadow_ledger()
            .expect("table ledger")
            .positioning_gaps
            .iter()
            .any(|record| record.gap == crate::table_shadow::TablePositioningGap::Sticky),
        "row-group sticky uses the shared retained sticky solver",
    );

    assert!(document.scroll_at(10.0, 10.0, 0.0, 150.0));
    document.frame(160, 120).expect("scrolled table frame");

    let active = document.sticky_layout(document.layout.as_ref().expect("retained layout"));
    let group_rect = active
        .get(sticky_group)
        .map(|fragment| fragment.physical_rect())
        .expect("active table row-group fragment");
    let row_rect = active
        .get(sticky_row)
        .map(|fragment| fragment.physical_rect())
        .expect("active table row fragment");
    let cell_rect = active
        .get(sticky_cell)
        .map(|fragment| fragment.physical_rect())
        .expect("active table cell fragment");
    assert_eq!(group_rect.y, 150.0);
    assert_eq!(row_rect.y, 150.0);
    assert_eq!(cell_rect.y, 150.0);
    assert_eq!(group_rect.y - document.element_scroll()[&scroller].1, 0.0);
    assert_eq!(generated_ids(&document, sticky_group), group_ids_before);
    assert_eq!(generated_ids(&document, sticky_row), row_ids_before);
    assert_eq!(generated_ids(&document, sticky_cell), cell_ids_before);
    assert_eq!(
        table_wrapper_fragment_id(&document, table),
        table_wrapper_before
    );
    assert_eq!(
        document
            .layout
            .as_ref()
            .and_then(|layout| layout.fragments.get(sticky_group))
            .map(|fragment| fragment.physical_rect()),
        Some(static_group),
        "scrolling keeps the retained table row-group base layout unchanged",
    );
}

#[test]
fn sticky_table_header_group_moves_its_row_subtree_without_relayout() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=scroller><div id=content><div id=spacer></div><table id=table><thead id=sticky-group><tr id=sticky-row><td id=sticky-cell>sticky</td></tr></thead><tbody><tr><td id=tail-cell></td></tr></tbody></table></div></div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #scroller { height: 80px; overflow-y: auto; } \
             #spacer { height: 120px; } \
             table { display: table; table-layout: fixed; width: 100px; border-spacing: 0; } \
             thead { display: table-header-group; } tbody { display: table-row-group; } \
             tr { display: table-row; } td { display: table-cell; width: 100px; } \
             #sticky-group { position: sticky; top: 0; } \
             #sticky-row { height: 20px; background: red; } \
             #tail-cell { height: 180px; background: blue; }"]),
        Device::screen(160.0, 120.0),
    );
    document.frame(160, 120).expect("initial table frame");
    let scroller = by_id(document.dom(), "scroller");
    let table = by_id(document.dom(), "table");
    let sticky_group = by_id(document.dom(), "sticky-group");
    let sticky_row = by_id(document.dom(), "sticky-row");
    let sticky_cell = by_id(document.dom(), "sticky-cell");
    let group_ids_before = generated_ids(&document, sticky_group);
    let row_ids_before = generated_ids(&document, sticky_row);
    let cell_ids_before = generated_ids(&document, sticky_cell);
    let table_wrapper_before = table_wrapper_fragment_id(&document, table);
    let static_group = document
        .layout
        .as_ref()
        .and_then(|layout| layout.fragments.get(sticky_group))
        .map(|fragment| fragment.physical_rect())
        .expect("static table header-group fragment");
    assert_eq!(static_group.y, 120.0);

    assert!(document.scroll_at(10.0, 10.0, 0.0, 150.0));
    document.frame(160, 120).expect("scrolled table frame");

    let active = document.sticky_layout(document.layout.as_ref().expect("retained layout"));
    let group_rect = active
        .get(sticky_group)
        .map(|fragment| fragment.physical_rect())
        .expect("active table header-group fragment");
    let row_rect = active
        .get(sticky_row)
        .map(|fragment| fragment.physical_rect())
        .expect("active table row fragment");
    let cell_rect = active
        .get(sticky_cell)
        .map(|fragment| fragment.physical_rect())
        .expect("active table cell fragment");
    assert_eq!(group_rect.y, 150.0);
    assert_eq!(row_rect.y, 150.0);
    assert_eq!(cell_rect.y, 150.0);
    assert_eq!(group_rect.y - document.element_scroll()[&scroller].1, 0.0);
    assert_eq!(generated_ids(&document, sticky_group), group_ids_before);
    assert_eq!(generated_ids(&document, sticky_row), row_ids_before);
    assert_eq!(generated_ids(&document, sticky_cell), cell_ids_before);
    assert_eq!(
        table_wrapper_fragment_id(&document, table),
        table_wrapper_before
    );
    assert_eq!(
        document
            .layout
            .as_ref()
            .and_then(|layout| layout.fragments.get(sticky_group))
            .map(|fragment| fragment.physical_rect()),
        Some(static_group),
        "scrolling keeps the retained header-group base layout unchanged",
    );
}

#[test]
fn sticky_table_footer_group_uses_the_scrollport_end_without_relayout() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=scroller><div id=content><div id=spacer></div><table id=table><tbody><tr><td id=tail-cell></td></tr></tbody><tfoot id=sticky-group><tr id=sticky-row><td id=sticky-cell>sticky</td></tr></tfoot></table></div></div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #scroller { height: 80px; overflow-y: auto; } \
             #spacer { height: 120px; } \
             table { display: table; table-layout: fixed; width: 100px; border-spacing: 0; } \
             tbody { display: table-row-group; } tfoot { display: table-footer-group; } \
             tr { display: table-row; } td { display: table-cell; width: 100px; } \
             #tail-cell { height: 180px; background: blue; } \
             #sticky-group { position: sticky; bottom: 0; } \
             #sticky-row { height: 20px; background: red; }"]),
        Device::screen(160.0, 120.0),
    );
    document.frame(160, 120).expect("initial table frame");
    let scroller = by_id(document.dom(), "scroller");
    let table = by_id(document.dom(), "table");
    let sticky_group = by_id(document.dom(), "sticky-group");
    let sticky_row = by_id(document.dom(), "sticky-row");
    let sticky_cell = by_id(document.dom(), "sticky-cell");
    let group_ids_before = generated_ids(&document, sticky_group);
    let row_ids_before = generated_ids(&document, sticky_row);
    let cell_ids_before = generated_ids(&document, sticky_cell);
    let table_wrapper_before = table_wrapper_fragment_id(&document, table);
    let static_group = document
        .layout
        .as_ref()
        .and_then(|layout| layout.fragments.get(sticky_group))
        .map(|fragment| fragment.physical_rect())
        .expect("static table footer-group fragment");
    assert!(
        static_group.y > 120.0,
        "the footer group starts below the table's ordinary content"
    );

    assert!(document.scroll_at(10.0, 10.0, 0.0, 150.0));
    document.frame(160, 120).expect("scrolled table frame");

    let active = document.sticky_layout(document.layout.as_ref().expect("retained layout"));
    let group_rect = active
        .get(sticky_group)
        .map(|fragment| fragment.physical_rect())
        .expect("active table footer-group fragment");
    let row_rect = active
        .get(sticky_row)
        .map(|fragment| fragment.physical_rect())
        .expect("active table row fragment");
    let cell_rect = active
        .get(sticky_cell)
        .map(|fragment| fragment.physical_rect())
        .expect("active table cell fragment");
    assert_eq!(
        group_rect.y - document.element_scroll()[&scroller].1,
        80.0 - group_rect.height,
        "the footer group's subpixel height stays flush with the scrollport end",
    );
    assert_eq!(row_rect.y, group_rect.y);
    assert_eq!(cell_rect.y, group_rect.y);
    assert_eq!(generated_ids(&document, sticky_group), group_ids_before);
    assert_eq!(generated_ids(&document, sticky_row), row_ids_before);
    assert_eq!(generated_ids(&document, sticky_cell), cell_ids_before);
    assert_eq!(
        table_wrapper_fragment_id(&document, table),
        table_wrapper_before
    );
    assert_eq!(
        document
            .layout
            .as_ref()
            .and_then(|layout| layout.fragments.get(sticky_group))
            .map(|fragment| fragment.physical_rect()),
        Some(static_group),
        "scrolling keeps the retained footer-group base layout unchanged",
    );
}

#[test]
fn sticky_table_caption_uses_its_wrapper_scroll_extent_without_relayout() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=scroller><div id=content><div id=spacer></div><table id=table><caption id=sticky-caption>sticky</caption><tbody><tr><td id=tail-cell></td></tr></tbody></table></div></div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #scroller { height: 80px; overflow-y: auto; } \
             #spacer { height: 120px; } \
             table { display: table; table-layout: fixed; width: 100px; border-spacing: 0; } \
             caption { display: table-caption; } \
             tbody { display: table-row-group; } tr { display: table-row; } \
             td { display: table-cell; width: 100px; } \
             #sticky-caption { position: sticky; top: 0; height: 20px; background: red; } \
             #tail-cell { height: 180px; background: blue; }"]),
        Device::screen(160.0, 120.0),
    );
    document.frame(160, 120).expect("initial table frame");
    let scroller = by_id(document.dom(), "scroller");
    let table = by_id(document.dom(), "table");
    let caption = by_id(document.dom(), "sticky-caption");
    let caption_ids_before = generated_ids(&document, caption);
    let table_wrapper_before = table_wrapper_fragment_id(&document, table);
    let static_caption = document
        .layout
        .as_ref()
        .and_then(|layout| layout.fragments.get(caption))
        .map(|fragment| fragment.physical_rect())
        .expect("static table caption fragment");
    assert_eq!(static_caption.y, 120.0);

    assert!(document.scroll_at(10.0, 10.0, 0.0, 150.0));
    document.frame(160, 120).expect("scrolled table frame");

    let active = document.sticky_layout(document.layout.as_ref().expect("retained layout"));
    let caption_rect = active
        .get(caption)
        .map(|fragment| fragment.physical_rect())
        .expect("active table caption fragment");
    assert_eq!(caption_rect.y, 150.0);
    assert_eq!(caption_rect.y - document.element_scroll()[&scroller].1, 0.0);
    assert_eq!(generated_ids(&document, caption), caption_ids_before);
    assert_eq!(
        table_wrapper_fragment_id(&document, table),
        table_wrapper_before
    );
    assert_eq!(
        document
            .layout
            .as_ref()
            .and_then(|layout| layout.fragments.get(caption))
            .map(|fragment| fragment.physical_rect()),
        Some(static_caption),
        "scrolling keeps the retained caption base layout unchanged",
    );
}

#[test]
fn retained_disjoint_formatting_roots_publish_atomically() {
    let initial = "<html><body><div id=first><div id=first-child>one</div></div><div id=second><div id=second-child>two</div></div><div id=outside>outside</div></body></html>";
    let final_document = "<html><body><div id=first style=\"width: 160px\"><div id=first-child>one</div></div><div id=second style=\"width: 180px\"><div id=second-child>two</div></div><div id=outside>outside</div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #first, #second { display: flex; width: 100px; height: 30px; background: red; } \
             #first-child, #second-child { width: 40px; height: 20px; background: blue; } \
             #outside { width: 80px; height: 20px; background: green; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 160.0));
    retained.frame(240, 160).expect("initial retained frame");
    let first = by_id(retained.dom(), "first");
    let first_child = by_id(retained.dom(), "first-child");
    let second = by_id(retained.dom(), "second");
    let second_child = by_id(retained.dom(), "second-child");
    let outside = by_id(retained.dom(), "outside");
    let first_before = generated_ids(&retained, first);
    let first_child_before = generated_ids(&retained, first_child);
    let second_before = generated_ids(&retained, second);
    let second_child_before = generated_ids(&retained, second_child);
    let outside_before = generated_ids(&retained, outside);

    retained.mutate_dom(|dom| {
        dom.set_attribute(by_id(dom, "first"), attr("style"), "width: 160px");
        dom.set_attribute(by_id(dom, "second"), attr("style"), "width: 180px");
    });
    assert_eq!(
        retained.last_layout_damage(),
        Some(&LayoutDamage {
            kind: LayoutDamageKind::Dom,
            roots: vec![first, second],
            full_document: false,
        })
    );
    let retained_paint = retained.frame(240, 160).expect("spliced retained frame");

    assert_eq!(generated_ids(&retained, first), first_before);
    assert_ne!(generated_ids(&retained, first_child), first_child_before);
    assert_eq!(generated_ids(&retained, second), second_before);
    assert_ne!(generated_ids(&retained, second_child), second_child_before);
    assert_eq!(generated_ids(&retained, outside), outside_before);

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 160.0));
    let fresh_paint = fresh.frame(240, 160).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "every spliced root must publish as one fresh final document",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn retained_root_splice_keeps_an_unrelated_table_paint_plane_live() {
    let initial = "<html><body><div id=flex><div id=child>child</div></div><table id=table><tbody><tr><td>cell</td></tr></tbody></table></body></html>";
    let final_document = "<html><body><div id=flex style=\"width: 180px\"><div id=child>child</div></div><table id=table><tbody><tr><td>cell</td></tr></tbody></table></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #flex { display: flex; width: 100px; height: 40px; background: red; } \
             #child { width: 40px; height: 20px; background: blue; } \
             table { display: table; border-spacing: 0; background: green; } \
             tbody { display: table-row-group; } tr { display: table-row; } \
             td { display: table-cell; width: 60px; height: 20px; background: yellow; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 160.0));
    retained.frame(240, 160).expect("initial retained frame");
    let flex = by_id(retained.dom(), "flex");
    let child = by_id(retained.dom(), "child");
    let table = by_id(retained.dom(), "table");
    let flex_before = generated_ids(&retained, flex);
    let child_before = generated_ids(&retained, child);
    let table_before = generated_ids(&retained, table);
    assert_table_paint_sources_are_live(&retained, table);

    retained.mutate_dom(|dom| {
        dom.set_attribute(by_id(dom, "flex"), attr("style"), "width: 180px");
    });
    let retained_paint = retained.frame(240, 160).expect("spliced retained frame");

    assert_eq!(generated_ids(&retained, flex), flex_before);
    assert_ne!(generated_ids(&retained, child), child_before);
    assert_eq!(generated_ids(&retained, table), table_before);
    assert_table_paint_sources_are_live(&retained, table);

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 160.0));
    let fresh_paint = fresh.frame(240, 160).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "fresh table paint must agree with the retained fragment tree",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn background_color_mutation_repaints_without_a_geometry_pass() {
    let initial = "<html><body><div id=target>target</div></body></html>";
    let final_document =
        "<html><body><div id=target style=\"background-color: blue\">target</div></body></html>";
    let styles = || {
        StyleSet::cambium(&[
            "html, body { margin: 0; padding: 0; } #target { width: 100px; height: 20px; }",
        ])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(160.0, 120.0));
    retained.frame(160, 120).expect("initial frame");
    let target = by_id(retained.dom(), "target");
    let ids_before = generated_ids(&retained, target);
    let layout_generation = retained.layout_generation();

    retained.mutate_dom(|dom| {
        dom.set_attribute(
            by_id(dom, "target"),
            attr("style"),
            "background-color: blue",
        );
    });
    let retained_paint = retained.frame(160, 120).expect("repainted frame");

    assert_eq!(retained.layout_generation(), layout_generation);
    assert_eq!(generated_ids(&retained, target), ids_before);
    assert!(
        retained.identity_source.is_none() && !retained.layout_dirty,
        "the retained geometry was repainted directly rather than rebuilt",
    );

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(160.0, 120.0));
    let fresh_paint = fresh.frame(160, 120).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "paint-only reuse must match a fresh final-document layout",
    );
}

#[test]
fn positioned_inset_mutation_reuses_a_stable_fragment_subtree() {
    let initial = "<html><body><div id=containing><div id=positioned>target</div></div><div id=outside>outside</div></body></html>";
    let final_document = "<html><body><div id=containing><div id=positioned style=\"left: 70px\">target</div></div><div id=outside>outside</div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #containing { position: relative; width: 200px; height: 60px; } \
             #positioned { position: absolute; left: 10px; top: 5px; width: 40px; height: 20px; } \
             #outside { width: 80px; height: 20px; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 120.0));
    retained.frame(240, 120).expect("initial retained frame");
    let positioned = by_id(retained.dom(), "positioned");
    let outside = by_id(retained.dom(), "outside");
    let positioned_ids = generated_ids(&retained, positioned);
    let outside_ids = generated_ids(&retained, outside);
    let layout_generation = retained.layout_generation();

    retained.mutate_dom(|dom| {
        dom.set_attribute(by_id(dom, "positioned"), attr("style"), "left: 70px");
    });
    let retained_paint = retained.frame(240, 120).expect("repositioned frame");

    assert_eq!(retained.layout_generation(), layout_generation + 1);
    assert_eq!(generated_ids(&retained, positioned), positioned_ids);
    assert_eq!(generated_ids(&retained, outside), outside_ids);
    let rect = retained
        .layout
        .as_ref()
        .and_then(|layout| layout.fragments.get(positioned))
        .map(|fragment| fragment.physical_rect())
        .expect("repositioned fragment");
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (70.0, 5.0, 40.0, 20.0)
    );
    assert!(
        retained.identity_source.is_none() && !retained.layout_dirty,
        "the positioned fragment subtree was translated without a fresh layout",
    );

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 120.0));
    let fresh_paint = fresh.frame(240, 120).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "the retained positioned result must match a fresh final-document layout",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn positioned_inset_reuse_updates_nested_scroll_range() {
    let initial =
        "<html><body><div id=scroller><div id=positioned>out of flow</div></div></body></html>";
    let final_document = "<html><body><div id=scroller><div id=positioned style=\"top: 300px\">out of flow</div></div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #scroller { position: relative; width: 100px; height: 80px; overflow-y: auto; } \
             #positioned { position: absolute; left: 0; top: 200px; width: 100px; height: 20px; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(160.0, 120.0));
    retained.frame(160, 120).expect("initial frame");
    let positioned = by_id(retained.dom(), "positioned");
    let scroller = by_id(retained.dom(), "scroller");
    let positioned_ids = generated_ids(&retained, positioned);
    let layout_generation = retained.layout_generation();

    retained.mutate_dom(|dom| {
        dom.set_attribute(by_id(dom, "positioned"), attr("style"), "top: 300px");
    });
    retained.frame(160, 120).expect("repositioned frame");

    assert_eq!(retained.layout_generation(), layout_generation + 1);
    assert_eq!(generated_ids(&retained, positioned), positioned_ids);
    let layout = retained.layout.as_ref().expect("repositioned layout");
    assert_eq!(retained.scroll_extent(layout, scroller), (0.0, 240.0));

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(160.0, 120.0));
    fresh.frame(160, 120).expect("fresh final frame");
    let fresh_scroller = by_id(fresh.dom(), "scroller");
    let fresh_layout = fresh.layout.as_ref().expect("fresh final layout");
    assert_eq!(
        retained.scroll_extent(layout, scroller),
        fresh.scroll_extent(fresh_layout, fresh_scroller),
        "retained repositioning must keep nested scrolling equal to a fresh final layout",
    );
}

#[test]
fn positioned_leaf_geometry_mutation_resizes_the_retained_fragment() {
    let initial = "<html><body><div id=containing><canvas id=positioned width=\"80\" height=\"40\"></canvas></div><div id=outside>outside</div></body></html>";
    let final_document = "<html><body><div id=containing><canvas id=positioned width=\"80\" height=\"40\" style=\"left: 70px; width: 120px; height: 60px\"></canvas></div><div id=outside>outside</div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #containing { position: relative; width: 200px; height: 80px; } \
             #positioned { position: absolute; left: 10px; top: 5px; } \
             #outside { width: 80px; height: 20px; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 120.0));
    retained.frame(240, 120).expect("initial retained frame");
    let positioned = by_id(retained.dom(), "positioned");
    let outside = by_id(retained.dom(), "outside");
    let positioned_ids = generated_ids(&retained, positioned);
    let outside_ids = generated_ids(&retained, outside);
    let layout_generation = retained.layout_generation();

    retained.mutate_dom(|dom| {
        dom.set_attribute(
            by_id(dom, "positioned"),
            attr("style"),
            "left: 70px; width: 120px; height: 60px",
        );
    });
    let retained_paint = retained.frame(240, 120).expect("resized frame");

    assert_eq!(retained.layout_generation(), layout_generation + 1);
    assert_eq!(generated_ids(&retained, positioned), positioned_ids);
    assert_eq!(generated_ids(&retained, outside), outside_ids);
    let rect = retained
        .layout
        .as_ref()
        .and_then(|layout| layout.fragments.get(positioned))
        .map(|fragment| fragment.physical_rect())
        .expect("resized fragment");
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (70.0, 5.0, 120.0, 60.0)
    );

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 120.0));
    let fresh_paint = fresh.frame(240, 120).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "retained leaf resize must match a fresh final-document layout",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn positioned_text_leaf_resize_reformats_instead_of_reusing_shaped_text() {
    let initial = "<html><body><div id=containing><div id=positioned>one two three four five six</div></div><div id=outside>outside</div></body></html>";
    let final_document = "<html><body><div id=containing><div id=positioned style=\"width: 120px\">one two three four five six</div></div><div id=outside>outside</div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #containing { position: relative; width: 200px; height: 100px; } \
             #positioned { position: absolute; left: 10px; top: 5px; width: 70px; } \
             #outside { width: 80px; height: 20px; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(240.0, 140.0));
    retained.frame(240, 140).expect("initial retained frame");

    retained.mutate_dom(|dom| {
        dom.set_attribute(by_id(dom, "positioned"), attr("style"), "width: 120px");
    });
    let retained_paint = retained.frame(240, 140).expect("reformatted frame");

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(240.0, 140.0));
    let fresh_paint = fresh.frame(240, 140).expect("fresh final frame");
    assert_eq!(
        format!("{:?}", retained_paint.commands()),
        format!("{:?}", fresh_paint.commands()),
        "a text-bearing resize must reshape exactly like a fresh final layout",
    );
    assert_eq!(retained.content_height(0), fresh.content_height(0));
}

#[test]
fn positioned_leaf_resize_updates_nested_scroll_range() {
    let initial = "<html><body><div id=scroller><canvas id=positioned width=\"100\" height=\"20\"></canvas></div></body></html>";
    let final_document = "<html><body><div id=scroller><canvas id=positioned width=\"100\" height=\"20\" style=\"top: 200px; width: 120px; height: 60px\"></canvas></div></body></html>";
    let styles = || {
        StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #scroller { position: relative; width: 100px; height: 80px; overflow-y: auto; } \
             #positioned { position: absolute; left: 0; top: 100px; }"])
    };
    let mut dom = ScriptedDom::from_serialized_document(initial);
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut retained = LiveryDocument::new(dom, styles(), Device::screen(160.0, 120.0));
    retained.frame(160, 120).expect("initial frame");
    let positioned = by_id(retained.dom(), "positioned");
    let scroller = by_id(retained.dom(), "scroller");
    let positioned_ids = generated_ids(&retained, positioned);
    let layout_generation = retained.layout_generation();

    retained.mutate_dom(|dom| {
        dom.set_attribute(
            by_id(dom, "positioned"),
            attr("style"),
            "top: 200px; width: 120px; height: 60px",
        );
    });
    retained.frame(160, 120).expect("resized frame");

    assert_eq!(retained.layout_generation(), layout_generation + 1);
    assert_eq!(generated_ids(&retained, positioned), positioned_ids);
    let layout = retained.layout.as_ref().expect("resized retained layout");
    assert_eq!(retained.scroll_extent(layout, scroller), (20.0, 180.0));

    let mut fresh_dom = ScriptedDom::from_serialized_document(final_document);
    let mut fresh_mutations = Vec::new();
    fresh_dom.drain_mutations(&mut fresh_mutations);
    let mut fresh = LiveryDocument::new(fresh_dom, styles(), Device::screen(160.0, 120.0));
    fresh.frame(160, 120).expect("fresh final frame");
    let fresh_scroller = by_id(fresh.dom(), "scroller");
    let fresh_layout = fresh.layout.as_ref().expect("fresh final layout");
    assert_eq!(
        retained.scroll_extent(layout, scroller),
        fresh.scroll_extent(fresh_layout, fresh_scroller),
        "retained leaf resize must keep nested scrolling equal to a fresh final layout",
    );
}

#[test]
fn geometry_mutation_rejects_the_paint_only_reuse_path() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=target>target</div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&[
            "html, body { margin: 0; padding: 0; } #target { width: 100px; height: 20px; }",
        ]),
        Device::screen(160.0, 120.0),
    );
    document.frame(160, 120).expect("initial frame");
    let layout_generation = document.layout_generation();

    document.mutate_dom(|dom| {
        let target = by_id(dom, "target");
        dom.set_attribute(target, attr("style"), "width: 120px");
    });
    document.frame(160, 120).expect("resized frame");

    assert_eq!(document.layout_generation(), layout_generation + 1);
    assert_eq!(
        document
            .layout
            .as_ref()
            .and_then(|layout| layout.fragments.get(by_id(document.dom(), "target")))
            .map(|fragment| fragment.width),
        Some(120.0),
        "a geometry change stays on the full K5g reconciliation path",
    );
}

#[test]
fn retained_document_uses_intrinsic_positioned_width_between_insets() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=containing><div id=positioned>MMMM MMMM MMMM MMMM MMMM MMMM MMMM MMMM</div></div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
             #containing { position: relative; width: 200px; } \
             #positioned { position: absolute; left: 10px; right: 20px; }"]),
        Device::screen(320.0, 240.0),
    );

    document.frame(320, 240).expect("positioned frame");
    let positioned = by_id(document.dom(), "positioned");
    let rect = document
        .layout
        .as_ref()
        .and_then(|layout| layout.fragments.get(positioned))
        .map(|fragment| fragment.physical_rect())
        .expect("positioned fragment");

    assert_eq!((rect.x, rect.width), (10.0, 170.0));
    assert!(
        rect.height > 20.0,
        "the second formatter pass rewraps content at Buckram's used width"
    );
}

#[test]
fn sticky_scrolls_its_retained_fragment_without_relayout() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=spacer></div><div id=sticky>sticky</div><div id=tail></div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&[
            "html, body { margin: 0; padding: 0; } #spacer { height: 120px; } \
             #sticky { position: sticky; top: 0; height: 20px; } #tail { height: 180px; }",
        ]),
        Device::screen(160.0, 80.0),
    );
    document.frame(160, 80).expect("initial sticky frame");
    let sticky = by_id(document.dom(), "sticky");
    let before_ids = generated_ids(&document, sticky);
    let static_rect = document
        .layout
        .as_ref()
        .and_then(|layout| layout.fragments.get(sticky))
        .map(|fragment| fragment.physical_rect())
        .expect("static sticky fragment");
    assert_eq!(static_rect.y, 120.0);

    assert!(document.scroll_by(0.0, 150.0));
    document.frame(160, 80).expect("scrolled sticky frame");

    let active = document.sticky_layout(document.layout.as_ref().expect("retained layout"));
    let sticky_rect = active
        .get(sticky)
        .map(|fragment| fragment.physical_rect())
        .expect("active sticky fragment");
    assert_eq!(sticky_rect.y, 150.0);
    assert_eq!(sticky_rect.y - document.scroll().1, 0.0);
    assert_eq!(generated_ids(&document, sticky), before_ids);
    assert_eq!(
        document
            .layout
            .as_ref()
            .and_then(|layout| layout.fragments.get(sticky))
            .map(|fragment| fragment.physical_rect()),
        Some(static_rect),
        "scrolling never mutates the retained normal-flow base layout",
    );
    assert!(
        !document.layout_dirty,
        "scroll repaint did not trigger relayout"
    );
}

#[test]
fn sticky_uses_the_nearest_nested_scrollport_offset() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=scroller><div id=content><div id=spacer></div><div id=sticky>sticky</div><div id=tail></div></div></div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&[
            "html, body { margin: 0; padding: 0; } #scroller { height: 80px; overflow-y: auto; } \
             #spacer { height: 120px; } #sticky { position: sticky; top: 0; height: 20px; } \
             #tail { height: 180px; }",
        ]),
        Device::screen(160.0, 120.0),
    );
    document
        .frame(160, 120)
        .expect("initial nested sticky frame");
    let scroller = by_id(document.dom(), "scroller");
    let sticky = by_id(document.dom(), "sticky");

    assert!(document.scroll_at(10.0, 10.0, 0.0, 150.0));
    assert_eq!(
        document.element_scroll().get(&scroller),
        Some(&(0.0, 150.0))
    );
    document.frame(160, 120).expect("nested sticky frame");

    let active = document.sticky_layout(document.layout.as_ref().expect("retained layout"));
    let sticky_rect = active
        .get(sticky)
        .map(|fragment| fragment.physical_rect())
        .expect("active nested sticky fragment");
    assert_eq!(sticky_rect.y, 150.0);
    assert_eq!(sticky_rect.y - document.element_scroll()[&scroller].1, 0.0);
}

#[test]
fn accessible_reveal_unwinds_active_nested_scrollports_inside_out() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=outer><div id=outer-top></div><div id=inner><div id=target>target</div><div id=inner-tail></div></div><div id=outer-tail></div></div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #outer { width: 100px; height: 80px; overflow-y: auto; } \
             #outer-top { height: 40px; } \
             #inner { width: 80px; height: 80px; overflow-y: auto; } \
             #target { height: 20px; } \
             #inner-tail, #outer-tail { height: 160px; }"]),
        Device::screen(160.0, 120.0),
    );
    document
        .frame(160, 120)
        .expect("initial nested-scroll frame");
    let outer = by_id(document.dom(), "outer");
    let inner = by_id(document.dom(), "inner");
    let target = by_id(document.dom(), "target");
    document.nested_scroll.insert(outer, (0.0, 120.0));
    document.nested_scroll.insert(inner, (0.0, 60.0));

    assert!(
        document.cached.is_some(),
        "the completed frame populated paint cache"
    );
    assert!(document.scroll_accessible_node_into_view(target));
    assert_eq!(document.element_scroll().get(&inner), Some(&(0.0, 0.0)));
    assert_eq!(document.element_scroll().get(&outer), Some(&(0.0, 40.0)));
    assert_eq!(
        document.fragment_rect(target),
        Some([0.0, 0.0, 80.0, 20.0]),
        "the target is visible inside the final outer scrollport viewport"
    );
    assert!(
        document.cached.is_none(),
        "revealing invalidates only paint cache"
    );
    assert!(
        !document.scroll_accessible_node_into_view(target),
        "an already revealed target has no active nested offset to change"
    );
    assert!(!document.scroll_accessible_node_into_view(document.dom().document()));
}

#[test]
fn accessible_pointer_target_uses_the_visible_clipped_hit_descendant() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=scroller><div id=before></div><a id=target href=/next><span id=label>Open</span></a><div id=tail></div></div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&[
            "html, body { margin: 0; padding: 0; } #scroller { width: 100px; height: 40px; overflow-y: auto; } #before { height: 30px; } #target, #label { display: block; width: 100px; height: 40px; } #tail { height: 120px; }",
        ]),
        Device::screen(160.0, 120.0),
    );
    document.frame(160, 120).expect("initial clipped frame");
    let scroller = by_id(document.dom(), "scroller");
    let target = by_id(document.dom(), "target");
    let label = by_id(document.dom(), "label");
    document.nested_scroll.insert(scroller, (0.0, 45.0));

    let point = document
        .accessible_pointer_target(target)
        .expect("the partly visible link has a retained pointer target");
    assert!(point.1 > 0.0 && point.1 < 25.0, "visible point: {point:?}");
    assert_eq!(
        document.hit_test(point.0, point.1),
        Some(label),
        "the chosen point resolves to the link's painted descendant"
    );
    assert_eq!(
        document.click_at(point.0, point.1),
        ClickOutcome::Navigate("/next".to_owned()),
        "ordinary Livery pointer activation follows the same retained hit"
    );

    document.nested_scroll.insert(scroller, (0.0, 71.0));
    assert_eq!(
        document.accessible_pointer_target(target),
        None,
        "a fully clipped link has no host-guessable pointer coordinate"
    );

    let mut blocked_dom = ScriptedDom::from_serialized_document(
        "<html><body><a id=target href=/blocked>Blocked</a></body></html>",
    );
    let mut blocked_mutations = Vec::new();
    blocked_dom.drain_mutations(&mut blocked_mutations);
    let mut blocked = LiveryDocument::new(
        blocked_dom,
        StyleSet::cambium(&[
            "html, body { margin: 0; padding: 0; } #target { display: block; width: 100px; height: 40px; pointer-events: none; }",
        ]),
        Device::screen(160.0, 120.0),
    );
    blocked.frame(160, 120).expect("pointer-events frame");
    assert_eq!(
        blocked.accessible_pointer_target(by_id(blocked.dom(), "target")),
        None,
        "a pointer-events-disabled target cannot mint a host pointer action"
    );
}

#[test]
fn positioned_descendant_extends_its_scroll_container_range() {
    let mut dom = ScriptedDom::from_serialized_document(
        "<html><body><div id=scroller><div id=positioned>out of flow</div></div></body></html>",
    );
    let mut initial_mutations = Vec::new();
    dom.drain_mutations(&mut initial_mutations);
    let mut document = LiveryDocument::new(
        dom,
        StyleSet::cambium(&["html, body { margin: 0; padding: 0; } \
             #scroller { position: relative; width: 100px; height: 80px; overflow-y: auto; } \
             #positioned { position: absolute; top: 200px; width: 100px; height: 20px; }"]),
        Device::screen(160.0, 120.0),
    );
    document.frame(160, 120).expect("positioned overflow frame");
    let scroller = by_id(document.dom(), "scroller");
    let layout = document.layout.as_ref().expect("retained layout");
    assert_eq!(document.scroll_extent(layout, scroller), (0.0, 140.0));

    assert!(document.scroll_at(10.0, 10.0, 0.0, 200.0));
    assert_eq!(
        document.element_scroll().get(&scroller),
        Some(&(0.0, 140.0)),
        "the positioned fragment contributes to the container's scrollable overflow"
    );
}

#[test]
fn transferred_subtree_pending_mutations_damage_both_retained_documents() {
    let styles = || {
        StyleSet::cambium(&[
            "html, body { margin:0; padding:0; } #holder { display:flex; width:160px; } p { width:40px; height:20px; background:red; } p:empty { background:blue; }",
        ])
    };
    let make = |html: &str| {
        let mut dom = ScriptedDom::from_serialized_document(html);
        dom.drain_mutations(&mut Vec::new());
        LiveryDocument::new(dom, styles(), Device::screen(160.0, 120.0))
    };
    let mut source = make(
        "<html><body><div id=holder><p id=moved>old</p><p id=stay>stay</p></div></body></html>",
    );
    let mut destination = make("<html><body><div id=holder></div></body></html>");
    source.frame(160, 120).unwrap();
    destination.frame(160, 120).unwrap();
    let node = by_id(&source.dom, "moved");
    let text = source.dom.dom_children(node).next().unwrap();
    let former_parent = by_id(&source.dom, "holder");
    let new_parent = by_id(&destination.dom, "holder");
    source.dom.set_text(text, "changed");
    source.dom.set_attribute(node, attr("class"), "changed");
    source.dom.remove_child(node);
    let before = format!("{:?}", source.dom.pending_mutations());
    source
        .dom
        .transfer_detached_subtree_preserving_mutations_to(&mut destination.dom, node)
        .unwrap();
    assert_eq!(format!("{:?}", source.dom.pending_mutations()), before);
    destination.dom.append_child(new_parent, node);
    let mut removed = Vec::new();
    let mut inserted = Vec::new();
    source.dom.drain_mutations(&mut removed);
    destination.dom.drain_mutations(&mut inserted);
    assert!(
        removed
            .iter()
            .any(|m| matches!(m, DomMutation::CharacterDataChanged { node } if *node == text))
    );
    assert!(removed.iter().any(|m| matches!(m, DomMutation::Removed { node: id, former_parent: parent } if *id == node && *parent == former_parent)));
    assert!(inserted.iter().any(|m| matches!(m, DomMutation::Inserted { node: id, parent } if *id == node && *parent == new_parent)));
    source.apply_dom_mutations(&removed);
    destination.apply_dom_mutations(&inserted);
    assert_eq!(
        source.last_layout_damage().unwrap().roots,
        vec![former_parent]
    );
    assert_eq!(
        destination.last_layout_damage().unwrap().roots,
        vec![new_parent]
    );
    source.frame(160, 120).unwrap();
    destination.frame(160, 120).unwrap();
    assert!(source.style_session.styles().get(node).is_none());
    assert_eq!(destination.dom.text(text), Some("changed"));
    assert!(!generated_ids(&destination, node).is_empty());
}
