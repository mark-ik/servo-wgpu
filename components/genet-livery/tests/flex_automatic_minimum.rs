// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use genet_livery::{Device, InteractionStates, StyleSet, layout, resolve_styles};
use genet_static_dom::StaticDocument;
use layout_dom_api::{LayoutDom, LocalName, Namespace, NodeKind};

fn find(
    document: &StaticDocument,
    node: <StaticDocument as LayoutDom>::NodeId,
    id: &str,
) -> Option<<StaticDocument as LayoutDom>::NodeId> {
    if document.kind(node) == NodeKind::Element
        && document.attribute(node, &Namespace::from(""), &LocalName::from("id")) == Some(id)
    {
        return Some(node);
    }
    document
        .dom_children(node)
        .find_map(|child| find(document, child, id))
}

#[test]
fn nested_wrapped_column_uses_the_sum_of_intrinsic_item_heights_for_auto_minimum() {
    let document = StaticDocument::parse(
        r#"<html><body>
          <div id="outer" style="display: flex; flex-direction: column;">
            <div id="inner" style="display: flex; flex-direction: column; flex-wrap: wrap; flex: 1 0 0px; height: 500px">
              <div id="first" style="flex: 1 0 0px; width: 100px; background: green;">
                <div style="height: 50px;"></div>
              </div>
              <div id="second" style="flex: 1 0 0px; width: 100px; background: green;">
                <div style="height: 50px;"></div>
              </div>
            </div>
          </div>
        </body></html>"#,
    );
    let styles = resolve_styles(
        &document,
        &StyleSet::cambium(&[]),
        &Device::screen(320.0, 240.0),
        &InteractionStates::default(),
    );
    let fragments = layout(&document, &styles, 320.0, 240.0).expect("layout");

    let rect = |id: &str| {
        let node = find(&document, document.document(), id).expect(id);
        let fragment = fragments.get(node).expect("layout fragment");
        let rect = fragment.physical_rect();
        (rect.x, rect.y, rect.width, rect.height)
    };

    for id in ["outer", "inner", "first", "second"] {
        let node = find(&document, document.document(), id).expect(id);
        assert!(fragments.get(node).is_some(), "missing fragment for {id}");
    }

    let outer = rect("outer");
    let inner = rect("inner");
    let first = rect("first");
    let second = rect("second");

    assert_eq!(outer.3, 100.0);
    assert_eq!(inner.3, 100.0);
    // The auto-width body contains this flex tree inside its two default
    // 8px margins, so its descendants receive 320 - 16 pixels.
    assert_eq!(inner.2, 304.0);
    assert_eq!((first.0 - inner.0, first.1 - inner.1), (0.0, 0.0));
    assert_eq!((second.0 - inner.0, second.1 - inner.1), (0.0, 50.0));
    assert_eq!((first.2, first.3), (100.0, 50.0));
    assert_eq!((second.2, second.3), (100.0, 50.0));
}
