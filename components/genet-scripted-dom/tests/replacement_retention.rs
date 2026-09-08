/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! G5 replacement retention: semantic reads, reattachment and bounded reclamation.
use genet_scripted_dom::{ObservedMutation, Pins, ScriptedDom};
use layout_dom_api::{DomMutation, LayoutDom, LayoutDomMut, LocalName, Namespace, QualName};

#[derive(Clone, Copy, Debug)]
enum Replacement {
    Text,
    Fragment,
}

impl Replacement {
    fn apply(self, dom: &mut ScriptedDom, host: genet_scripted_dom::NodeId, empty: bool) {
        match self {
            Self::Text => dom.set_text_content(host, if empty { "" } else { "new" }),
            Self::Fragment => dom.set_inner_html(host, if empty { "" } else { "<b>new</b>" }),
        }
    }
}

fn qual(name: &str) -> QualName {
    QualName::new(
        None,
        Namespace::from("http://www.w3.org/1999/xhtml"),
        LocalName::from(name),
    )
}

fn retained_descendant(replacement: Replacement, observing: bool) {
    for empty in [false, true] {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let host = dom.create_element(qual("div"));
        let removed = dom.create_element(qual("section"));
        let text = dom.create_text("kept");
        dom.append_child(root, host);
        dom.append_child(host, removed);
        dom.append_child(removed, text);
        let mut pins = Pins::new();
        pins.pin(text); // Only the descendant is retained, not its parent.
        let mut mutations = Vec::new();
        dom.drain_mutations(&mut mutations);
        mutations.clear();
        dom.set_observing(observing);

        replacement.apply(&mut dom, host, empty);
        assert!(
            dom.is_live(removed),
            "{replacement:?}, observing={observing}, empty={empty}"
        );
        assert!(dom.is_live(text));
        assert_eq!(dom.parent(removed), None);
        assert_eq!(dom.parent(text), Some(removed));
        assert_eq!(dom.text(text), Some("kept"));
        let expected = match (replacement, empty) {
            (_, true) => "",
            (Replacement::Text, false) => "new",
            (Replacement::Fragment, false) => "<b>new</b>",
        };
        assert_eq!(dom.inner_html(host), expected);
        dom.drain_mutations(&mut mutations);
        assert!(
            matches!(mutations.as_slice(), [DomMutation::SubtreeReplaced { node }] if *node == host)
        );

        let records = dom.take_observed();
        if observing {
            assert!(
                matches!(records.as_slice(), [ObservedMutation::ChildList { target, removed: nodes, .. }] if *target == host && nodes == &[removed])
            );
        } else {
            assert!(records.is_empty());
        }
        dom.collect(pins.iter());
        assert!(dom.is_live(removed) && dom.is_live(text));
        assert_eq!(dom.text(text), Some("kept"));

        dom.append_child(host, removed);
        assert_eq!(dom.parent(removed), Some(host));
        dom.set_text(text, "changed");
        assert!(dom.inner_html(host).ends_with("<section>changed</section>"));

        dom.remove_child(removed);
        dom.take_observed(); // Delivery finished; this test supplies only reflector pins.
        pins.unpin(text);
        dom.collect(pins.iter());
        assert!(!dom.is_live(removed) && !dom.is_live(text));
        assert_eq!(dom.inner_html(host), expected);
    }
}

#[test]
fn text_replacement_retains_descendants_without_observers() {
    retained_descendant(Replacement::Text, false);
}

#[test]
fn text_replacement_retains_descendants_with_observers() {
    retained_descendant(Replacement::Text, true);
}

#[test]
fn fragment_replacement_retains_descendants_without_observers() {
    retained_descendant(Replacement::Fragment, false);
}

#[test]
fn fragment_replacement_retains_descendants_with_observers() {
    retained_descendant(Replacement::Fragment, true);
}

fn replacement_churn(observing: bool) {
    let mut dom = ScriptedDom::new();
    let root = dom.document();
    let host = dom.create_element(qual("div"));
    dom.append_child(root, host);
    let baseline = dom.live_node_count();
    dom.set_observing(observing);
    let mut mutations = Vec::new();
    for _ in 0..1000 {
        for replacement in [Replacement::Text, Replacement::Fragment] {
            replacement.apply(&mut dom, host, false);
            dom.take_observed();
            dom.drain_mutations(&mut mutations);
            mutations.clear();
            dom.collect([]);
            assert!(dom.live_node_count() <= baseline + 2);
        }
    }
    dom.set_text_content(host, "");
    dom.take_observed();
    dom.collect([]);
    assert_eq!(dom.live_node_count(), baseline);
}

#[test]
fn replacement_churn_reclaims_without_observers() {
    replacement_churn(false);
}

#[test]
fn replacement_churn_reclaims_after_observer_delivery() {
    replacement_churn(true);
}
