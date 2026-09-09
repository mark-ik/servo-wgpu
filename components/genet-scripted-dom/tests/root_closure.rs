/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! G5 store-level closure across edges that are not ordinary DOM parent links.
use genet_scripted_dom::{Pins, ScriptedDom, ShadowRootInit};
use layout_dom_api::{LayoutDom, LayoutDomMut, LocalName, Namespace, QualName};

fn qual(name: &str) -> QualName {
    QualName::new(
        None,
        Namespace::from("http://www.w3.org/1999/xhtml"),
        LocalName::from(name),
    )
}

#[test]
fn live_template_retains_its_contents_owner() {
    let mut dom = ScriptedDom::new();
    let root = dom.document();
    let template = dom.create_element(qual("template"));
    dom.append_child(root, template);
    let contents = dom.ensure_template_contents(template);
    let owner = dom.template_owner_document();
    dom.collect([]);
    assert!(dom.is_live(template) && dom.is_live(contents));
    assert!(
        dom.is_live(owner),
        "reachable contents must retain their inert owner"
    );
    assert_eq!(
        dom.parent(contents),
        None,
        "ownership must not become a parent link"
    );
}

#[test]
fn pinned_contents_descendant_retains_owner_without_retaining_template() {
    let mut dom = ScriptedDom::new();
    let template = dom.create_element(qual("template"));
    let contents = dom.ensure_template_contents(template);
    let owner = dom.template_owner_document();
    let text = dom.create_text("inert");
    dom.append_child(contents, text);
    let mut pins = Pins::new();
    pins.pin(text);
    dom.collect(pins.iter());
    assert!(
        !dom.is_live(template),
        "contents do not expose their former template"
    );
    assert!(dom.is_live(contents) && dom.is_live(text));
    assert!(dom.is_live(owner));
    assert!(dom.is_in_template_contents(text));
    assert_eq!(dom.template_contents_of(template), None);
    pins.unpin(text);
    dom.collect(pins.iter());
    assert!(!dom.is_live(contents) && !dom.is_live(owner));
    assert_eq!(dom.template_owner_document_if_any(), None);
}

#[test]
fn swept_template_has_no_dangling_contents_lookup() {
    let mut dom = ScriptedDom::new();
    let template = dom.create_element(qual("template"));
    let contents = dom.ensure_template_contents(template);
    dom.collect([]);
    assert!(!dom.is_live(template) && !dom.is_live(contents));
    assert_eq!(dom.template_contents_of(template), None);
}

#[test]
fn template_owner_is_reclaimed_and_reminted_after_churn() {
    let mut dom = ScriptedDom::new();
    let baseline = dom.live_node_count();
    let mut previous = None;
    for _ in 0..1000 {
        let template = dom.create_element(qual("template"));
        let contents = dom.ensure_template_contents(template);
        let owner = dom.template_owner_document();
        assert_ne!(
            Some(owner),
            previous,
            "a collected owner must not be returned again"
        );
        assert!(dom.is_live(owner));
        dom.collect([]);
        assert!(!dom.is_live(contents) && !dom.is_live(owner));
        assert_eq!(dom.template_owner_document_if_any(), None);
        assert_eq!(dom.live_node_count(), baseline);
        previous = Some(owner);
    }
}

#[test]
fn shared_owner_does_not_retain_a_retired_sibling_fragment() {
    let mut dom = ScriptedDom::new();
    let root = dom.document();
    let first = dom.create_element(qual("template"));
    let second = dom.create_element(qual("template"));
    dom.append_child(root, first);
    dom.append_child(root, second);
    let first_contents = dom.ensure_template_contents(first);
    let second_contents = dom.ensure_template_contents(second);
    let owner = dom.template_owner_document();
    dom.remove_child(first);
    dom.collect([]);
    assert!(dom.is_live(owner) && dom.is_live(second_contents));
    assert!(!dom.is_live(first) && !dom.is_live(first_contents));
    assert_eq!(dom.template_contents_of(first), None);
    assert_eq!(dom.template_contents_of(second), Some(second_contents));
}

#[test]
fn pinned_shadow_descendant_retains_host_after_replacement() {
    let mut dom = ScriptedDom::new();
    let root = dom.document();
    let outer = dom.create_element(qual("div"));
    let host = dom.create_element(qual("div"));
    dom.append_child(root, outer);
    dom.append_child(outer, host);
    let shadow = dom.attach_shadow(host, ShadowRootInit::default()).unwrap();
    let text = dom.create_text("shadow");
    dom.append_child(shadow, text);
    let mut pins = Pins::new();
    pins.pin(text);
    dom.set_text_content(outer, "new");
    dom.collect(pins.iter());
    assert!(dom.is_live(host) && dom.is_live(shadow) && dom.is_live(text));
    assert_eq!(dom.tree_root(text), Some(host));
    assert_eq!(dom.parent(shadow), None);
    assert_eq!(dom.text(text), Some("shadow"));
    pins.unpin(text);
    dom.collect(pins.iter());
    assert!(!dom.is_live(host) && !dom.is_live(shadow) && !dom.is_live(text));
}

#[test]
fn pinning_owner_alone_does_not_retain_template_contents() {
    let mut dom = ScriptedDom::new();
    let template = dom.create_element(qual("template"));
    let contents = dom.ensure_template_contents(template);
    let owner = dom.template_owner_document();
    dom.collect([owner]);
    assert!(dom.is_live(owner));
    assert!(!dom.is_live(template) && !dom.is_live(contents));
    assert_eq!(dom.template_contents_of(template), None);
    dom.collect([]);
    assert!(!dom.is_live(owner));
}
