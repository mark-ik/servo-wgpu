// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use genet_livery::{Device, InteractionStates, StyleSet, layout, resolve_styles};
use genet_static_dom::StaticDocument;
use layout_dom_api::{LayoutDom, LocalName, Namespace, NodeKind};

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

fn rects(css: &str) -> [(f32, f32); 3] {
    let document = StaticDocument::parse(
        "<!doctype html><html id='root'><body id='body'><div id='outer' style='height: 40px'>\
         <div id='nested' style='width: 120px; height: 20px'></div>\
         </div></body></html>",
    );
    let styles = resolve_styles(
        &document,
        &StyleSet::cambium(&[css]),
        &Device::screen(800.0, 600.0),
        &InteractionStates::default(),
    );
    let fragments = layout(&document, &styles, 800.0, 600.0).expect("layout");
    let rect = |name| {
        let node = find(&document, document.document(), name).expect(name);
        fragments
            .get(node)
            .unwrap_or_else(|| panic!("{name} has a fragment"))
            .physical_rect()
    };

    ["body", "outer", "nested"].map(|name| {
        let rect = rect(name);
        (rect.x, rect.width)
    })
}

#[test]
fn rtl_root_keeps_auto_body_and_nested_blocks_inside_default_margins() {
    let [body, outer, nested] = rects("html { direction: rtl; }");
    assert_eq!(body, (8.0, 784.0));
    assert_eq!(outer, (8.0, 784.0));
    assert_eq!(nested, (672.0, 120.0));
}

#[test]
fn ltr_root_keeps_auto_body_and_nested_blocks_inside_default_margins() {
    let [body, outer, nested] = rects("");
    assert_eq!(body, (8.0, 784.0));
    assert_eq!(outer, (8.0, 784.0));
    assert_eq!(nested, (8.0, 120.0));
}

#[test]
fn auto_body_border_and_padding_shrink_inside_default_margins() {
    let [body, outer, nested] =
        rects("html { direction: rtl; } body { border: 2px solid; padding: 4px; }");
    assert_eq!(body, (8.0, 784.0));
    assert_eq!(outer, (14.0, 772.0));
    assert_eq!(nested, (666.0, 120.0));
}

#[test]
fn authored_full_inline_size_still_overflows_by_default_margins() {
    let [body, outer, nested] = rects("html { direction: rtl; } body { inline-size: 100%; }");
    assert_eq!(body, (-8.0, 800.0));
    assert_eq!(outer, (-8.0, 800.0));
    assert_eq!(nested, (672.0, 120.0));
}

#[test]
fn vertical_root_keeps_auto_body_inside_default_inline_margins() {
    let document = StaticDocument::parse(
        "<!doctype html><html><body id='body'><div style='width: 40px; height: 20px'></div>\
         </body></html>",
    );
    let styles = resolve_styles(
        &document,
        &StyleSet::cambium(&["html { writing-mode: vertical-rl; width: 800px; height: 600px; }"]),
        &Device::screen(800.0, 600.0),
        &InteractionStates::default(),
    );
    let fragments = layout(&document, &styles, 800.0, 600.0).expect("layout");
    let body = find(&document, document.document(), "body").expect("body");
    let body = fragments.get(body).expect("body fragment").physical_rect();

    assert_eq!((body.y, body.height), (8.0, 584.0));
}
