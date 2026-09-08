// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The flat tree and style scoping over a **script-free** document.
//!
//! Every case here goes through `StaticDocument::parse`, so what it proves is
//! that a declarative shadow root renders without any script tier at all: the
//! post-parse pass builds the root, slot assignment places the light DOM, the
//! cascade descends the flat tree, and the tree-scope boundary holds in both
//! directions.

use genet_document_resources::{ResolvedDocumentResources, ResolvedStylesheet, StylesheetOwner};
use genet_livery::{Device, InteractionStates, StyleSet, resolve_styles};
use genet_static_dom::StaticDocument;
use layout_dom_api::{LayoutDom, LocalName, Namespace, NodeKind};

type Id = <StaticDocument as LayoutDom>::NodeId;

fn find(dom: &StaticDocument, id: Id, needle: &str) -> Option<Id> {
    if dom.attribute(id, &Namespace::from(""), &LocalName::from("id")) == Some(needle) {
        return Some(id);
    }
    dom.dom_children(id)
        .find_map(|child| find(dom, child, needle))
}

fn by_id(dom: &StaticDocument, needle: &str) -> Id {
    // The DOM walk does not reach into a shadow tree, so search the roots too.
    if let Some(found) = find(dom, dom.document(), needle) {
        return found;
    }
    for root in dom.shadow_roots() {
        if let Some(found) = find(dom, root, needle) {
            return found;
        }
    }
    panic!("no element with id {needle}");
}

const PAGE: &str = "<!doctype html><html><body>\
     <div id='host'>\
       <template shadowrootmode='open'>\
         <style>span { color: rgb(0, 128, 0); } :host { color: rgb(0, 0, 255); }\
                ::slotted(span) { font-style: italic; }</style>\
         <b id='shadow-b'>chrome</b>\
         <slot name='body'><i id='fallback'>none</i></slot>\
         <slot id='empty-slot'><i id='fallback-default'>nothing</i></slot>\
       </template>\
       <span id='light' slot='body'>content</span>\
     </div>\
     <span id='outside'>outside</span>\
     </body></html>";

fn document() -> StaticDocument {
    StaticDocument::parse(PAGE)
}

/// Build a `StyleSet` the way the retained host does: the `<style>` elements
/// the document actually carries, each remembering its owner node — which is
/// what the tree-scope boundary reads. `StyleSet::cambium(&[...])` cannot
/// serve here, because a sheet with no owner node has no tree scope.
fn style_set(dom: &StaticDocument, extra_document_css: &str) -> StyleSet {
    let mut resources = ResolvedDocumentResources::discover(dom, None).stylesheets;
    if !extra_document_css.is_empty() {
        // A document-scope sheet: no owner node, which is exactly what "no
        // containing shadow root" means to the tree-scope table.
        resources.push(ResolvedStylesheet {
            sheet_id: u64::MAX,
            owner: StylesheetOwner::Inline,
            owner_node: None,
            source_url: None,
            requested_url: None,
            content_type: None,
            media: None,
            imports: Vec::new(),
            import_parent: None,
            text: extra_document_css.to_owned(),
            document_order: u64::MAX,
        });
    }
    StyleSet::cambium_resources(&resources)
}

#[test]
fn declarative_template_becomes_a_shadow_root_with_slot_assignment() {
    let dom = document();
    assert!(dom.has_shadow_trees(), "the post-parse pass built no root");
    assert_eq!(dom.shadow_roots().len(), 1);

    let host = by_id(&dom, "host");
    let root = dom.shadow_root(host).expect("host has a shadow root");
    assert_eq!(dom.kind(root), NodeKind::ShadowRoot);
    assert_eq!(dom.shadow_host(root), Some(host));
    // The root is not a child of its host, and the template is gone.
    assert_eq!(dom.parent(root), None);
    assert!(
        !dom.dom_children(host).any(|child| child == root),
        "a shadow root must not appear among its host's children"
    );
    assert!(
        find(&dom, dom.document(), "shadow-b").is_none(),
        "a DOM walk must not reach into the shadow tree"
    );

    // The light-DOM child with `slot='body'` lands on the named slot.
    let light = by_id(&dom, "light");
    let named = dom
        .dom_children(root)
        .find(|id| {
            dom.attribute(*id, &Namespace::from(""), &LocalName::from("name")) == Some("body")
        })
        .expect("named slot");
    assert_eq!(dom.assigned_slot(light), Some(named));
    assert_eq!(dom.assigned_nodes(named), vec![light]);

    // The other slot got nothing, so it keeps its own fallback content.
    let empty = by_id(&dom, "empty-slot");
    assert!(dom.assigned_nodes(empty).is_empty());
}

#[test]
fn flat_children_splice_the_shadow_tree_in_and_the_light_dom_out() {
    let dom = document();
    let host = by_id(&dom, "host");
    let root = dom.shadow_root(host).expect("root");

    // A host's flat children are its shadow root's children, never its own.
    let flat: Vec<_> = dom.flat_children(host).collect();
    let shadow: Vec<_> = dom.dom_children(root).collect();
    assert_eq!(flat, shadow);
    assert!(
        !flat.contains(&by_id(&dom, "light")),
        "a slotted child reaches the flat tree through its slot, not its host"
    );

    // A slot's flat children are its assignment...
    let named = dom.assigned_slot(by_id(&dom, "light")).expect("slot");
    assert_eq!(
        dom.flat_children(named).collect::<Vec<_>>(),
        vec![by_id(&dom, "light")]
    );
    // ...and an unassigned slot's are its own children, the fallback content.
    let empty = by_id(&dom, "empty-slot");
    assert_eq!(
        dom.flat_children(empty).collect::<Vec<_>>(),
        dom.dom_children(empty).collect::<Vec<_>>()
    );

    // Ordinary nodes are untouched, and a document with no shadow tree pays
    // nothing: flat and DOM children agree everywhere else.
    let plain = StaticDocument::parse("<!doctype html><html><body><p><i>x</i></p></body></html>");
    assert!(!plain.has_shadow_trees());
    let mut stack = vec![plain.document()];
    while let Some(id) = stack.pop() {
        assert_eq!(
            plain.flat_children(id).collect::<Vec<_>>(),
            plain.dom_children(id).collect::<Vec<_>>()
        );
        stack.extend(plain.dom_children(id));
    }
}

#[test]
fn a_shadow_tree_styles_itself_and_the_document_does_not_reach_in() {
    let dom = document();
    // The document sheet names `span` and `b`; only the light DOM may hear it.
    let styles = resolve_styles(
        &dom,
        &style_set(
            &dom,
            "span { font-weight: bold; } b { color: rgb(255, 0, 0); }",
        ),
        &Device::screen(800.0, 600.0),
        &InteractionStates::default(),
    );

    // The shadow tree's own `<style>` styles the shadow tree.
    let shadow_b = by_id(&dom, "shadow-b");
    assert_eq!(
        styles.computed_style(shadow_b, "color").as_deref(),
        Some("rgb(0, 0, 255)"),
        "the document's `b` rule must not cross in; `<b>` inherits :host's blue"
    );
    // The shadow tree's own `span` rule styles a span inside the shadow tree,
    // and nothing outside it.
    assert_eq!(
        styles
            .computed_style(by_id(&dom, "outside"), "color")
            .as_deref(),
        Some("rgb(0, 0, 0)"),
        "a shadow tree's stylesheet must not leak into the document"
    );

    // `:host` styles the host from inside the shadow tree.
    let host = by_id(&dom, "host");
    assert_eq!(
        styles.computed_style(host, "color").as_deref(),
        Some("rgb(0, 0, 255)"),
        ":host must style the host element"
    );

    // `::slotted(span)` reaches the light-DOM node assigned into this tree.
    let light = by_id(&dom, "light");
    assert_eq!(
        styles.computed_style(light, "font-style").as_deref(),
        Some("italic"),
        "::slotted() must reach the assigned light-DOM element"
    );
    // ...and only it: a `<span>` outside the host is untouched by that rule.
    let outside = by_id(&dom, "outside");
    assert_eq!(
        styles.computed_style(outside, "font-style").as_deref(),
        Some("normal")
    );
    // The document's own rule still styles the light DOM, which lives in the
    // document's tree scope however it renders.
    assert_eq!(
        styles.computed_style(light, "font-weight").as_deref(),
        Some("bold")
    );
}

#[test]
fn inheritance_follows_the_flat_tree() {
    // The host's colour must reach the slotted content, which inherits through
    // the slot, not through its DOM parent — here they are the same element, so
    // the case that discriminates is a colour set on the *slot*.
    let dom = StaticDocument::parse(
        "<!doctype html><html><body>\
         <div id='host'>\
           <template shadowrootmode='open'>\
             <style>#wrap { color: rgb(0, 128, 0); }</style>\
             <div id='wrap'><slot></slot></div>\
           </template>\
           <span id='light'>content</span>\
         </div></body></html>",
    );
    let styles = resolve_styles(
        &dom,
        &style_set(&dom, ""),
        &Device::screen(800.0, 600.0),
        &InteractionStates::default(),
    );
    let light = by_id(&dom, "light");
    assert_eq!(
        styles.computed_style(light, "color").as_deref(),
        Some("rgb(0, 128, 0)"),
        "a slotted element inherits from its slot's flat-tree parent"
    );
}

#[test]
fn slotted_content_and_fallback_content_both_get_boxes() {
    let dom = document();
    let styles = resolve_styles(
        &dom,
        &style_set(&dom, ""),
        &Device::screen(800.0, 600.0),
        &InteractionStates::default(),
    );
    // Everything the flat tree reaches is styled: the shadow tree's own
    // elements, the slotted light-DOM element, and the unfilled slot's
    // fallback content.
    for id in ["shadow-b", "light", "fallback-default", "host", "outside"] {
        assert!(
            styles.computed_style(by_id(&dom, id), "display").is_some(),
            "{id} should have been styled"
        );
    }
    // A slot itself is `display: contents` in the UA sheet: it never boxes.
    let named = dom.assigned_slot(by_id(&dom, "light")).expect("slot");
    assert_eq!(
        styles.computed_style(named, "display").as_deref(),
        Some("contents")
    );
    // The *filled* slot's fallback content is not in the flat tree at all.
    let fallback = by_id(&dom, "fallback");
    assert!(
        styles.computed_style(fallback, "display").is_none(),
        "fallback content of a filled slot must not be rendered"
    );
}
