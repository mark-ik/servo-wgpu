// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! HTML serialization of the scripted DOM via html5ever's serializer — the inverse
//! of the `TreeSink` parse path genet-static-dom already implements. Powers
//! `outerHTML` / `innerHTML` and the verso flip's DOM-snapshot layer.
//!
//! This side only walks the tree and names start/end elements, text, comments, and
//! doctype; html5ever owns the correctness it is famous for: void elements (no close
//! tag), attribute and text escaping, and raw-text elements (`<script>`/`<style>`).
//! Mirrors servo's `dom/node.rs` serialize.

use std::io;

use html5ever::serialize::{SerializeOpts, serialize as html5ever_serialize};
use layout_dom_api::NodeKind;
use markup5ever::serialize::{Serialize, Serializer, TraversalScope};

use crate::{NodeId, ScriptedDom};

/// A `(ScriptedDom, NodeId)` pair html5ever's serializer can walk.
struct SerializableNode<'a> {
    dom: &'a ScriptedDom,
    id: NodeId,
    /// Write each **serializable** shadow root back out as the `<template
    /// shadowrootmode>` that would rebuild it. `getHTML({serializableShadowRoots:
    /// true})`; `innerHTML` leaves it false and never crosses the boundary.
    include_shadow: bool,
}

impl SerializableNode<'_> {
    fn child(&self, id: NodeId) -> SerializableNode<'_> {
        SerializableNode {
            dom: self.dom,
            id,
            include_shadow: self.include_shadow,
        }
    }

    /// Serialize every child as its own included node (shared by element bodies,
    /// the document/fragment roots, and `ChildrenOnly`).
    fn serialize_children<S: Serializer>(&self, serializer: &mut S) -> io::Result<()> {
        for &child in &self.dom.node(self.id).children {
            self.child(child)
                .serialize(serializer, TraversalScope::IncludeNode)?;
        }
        Ok(())
    }

    /// Emit `<template shadowrootmode=...>` plus the shadow tree's children, in
    /// front of the host's own light-DOM children — where the parser expects to
    /// find it, so the output round-trips through the declarative pass.
    fn serialize_shadow_root<S: Serializer>(&self, serializer: &mut S) -> io::Result<()> {
        if !self.include_shadow {
            return Ok(());
        }
        let Some(root) = self.dom.shadow_root_of(self.id) else {
            return Ok(());
        };
        let Some(init) = self.dom.shadow_init(root) else {
            return Ok(());
        };
        if !init.serializable {
            return Ok(());
        }
        let name = markup5ever::QualName::new(
            None,
            markup5ever::namespace_url!("http://www.w3.org/1999/xhtml"),
            markup5ever::LocalName::from("template"),
        );
        let attr = |local: &str| {
            markup5ever::QualName::new(
                None,
                markup5ever::Namespace::from(""),
                markup5ever::LocalName::from(local),
            )
        };
        let mut attrs: Vec<(markup5ever::QualName, String)> =
            vec![(attr("shadowrootmode"), init.mode.as_str().to_owned())];
        if init.delegates_focus {
            attrs.push((attr("shadowrootdelegatesfocus"), String::new()));
        }
        if init.clonable {
            attrs.push((attr("shadowrootclonable"), String::new()));
        }
        attrs.push((attr("shadowrootserializable"), String::new()));
        serializer.start_elem(
            name.clone(),
            attrs.iter().map(|(name, value)| (name, value.as_str())),
        )?;
        self.child(root).serialize_children(serializer)?;
        serializer.end_elem(name)
    }
}

impl Serialize for SerializableNode<'_> {
    fn serialize<S: Serializer>(
        &self,
        serializer: &mut S,
        traversal_scope: TraversalScope,
    ) -> io::Result<()> {
        let node = self.dom.node(self.id);
        match traversal_scope {
            TraversalScope::IncludeNode => match node.kind {
                NodeKind::Element => match &node.name {
                    Some(name) => {
                        serializer.start_elem(
                            name.clone(),
                            node.attrs.iter().map(|(n, v)| (n, v.as_str())),
                        )?;
                        self.serialize_shadow_root(serializer)?;
                        self.serialize_children(serializer)?;
                        serializer.end_elem(name.clone())?;
                    },
                    // A nameless "element" is malformed; emit its children rather
                    // than a bare tag.
                    None => self.serialize_children(serializer)?,
                },
                NodeKind::Text => {
                    if let Some(text) = &node.text {
                        serializer.write_text(text)?;
                    }
                },
                NodeKind::Comment => {
                    if let Some(text) = &node.text {
                        serializer.write_comment(text)?;
                    }
                },
                NodeKind::Doctype => {
                    serializer.write_doctype(node.text.as_deref().unwrap_or("html"))?;
                },
                // HTML has no CDATA sections; serializing an XML subtree as HTML
                // emits the character data, which is what the round trip means.
                NodeKind::CdataSection => {
                    if let Some(text) = &node.text {
                        serializer.write_text(text)?;
                    }
                },
                // A shadow root serializes as its children, like any other
                // fragment. Whether a host's shadow tree is reached at all is
                // the caller's question — `innerHTML` never crosses the
                // boundary, `getHTML({serializableShadowRoots})` does.
                NodeKind::Document | NodeKind::DocumentFragment | NodeKind::ShadowRoot => {
                    self.serialize_children(serializer)?;
                },
                NodeKind::ProcessingInstruction => {
                    let target = node.name.as_ref().map_or("", |n| n.local.as_ref());
                    serializer
                        .write_processing_instruction(target, node.text.as_deref().unwrap_or(""))?;
                },
            },
            // Children-only is `innerHTML` / `getHTML`, and HTML's fragment
            // serialization emits a serializable shadow root *before* the
            // host's own children — so a `getHTML` on the host itself carries
            // its root, not only its descendants'.
            TraversalScope::ChildrenOnly(_) => {
                self.serialize_shadow_root(serializer)?;
                self.serialize_children(serializer)?;
            },
        }
        Ok(())
    }
}

impl ScriptedDom {
    /// Serialize `node` and its subtree to HTML (`outerHTML`).
    ///
    /// Routes through html5ever's serializer, so void elements, attribute/text
    /// escaping, and raw-text elements are handled by the engine, not here.
    pub fn outer_html(&self, node: NodeId) -> String {
        self.serialize_scope(node, TraversalScope::IncludeNode)
    }

    /// Serialize only `node`'s children (`innerHTML`).
    pub fn inner_html(&self, node: NodeId) -> String {
        self.serialize_scope(node, TraversalScope::ChildrenOnly(None))
    }

    /// `innerHTML` plus every **serializable** shadow root under `node`, each
    /// written as the `<template shadowrootmode>` that rebuilds it —
    /// `getHTML({serializableShadowRoots: true})`.
    pub fn inner_html_with_shadow_roots(&self, node: NodeId) -> String {
        self.serialize_scope_with(node, TraversalScope::ChildrenOnly(None), true)
    }

    /// `outerHTML` including serializable shadow roots.
    pub fn outer_html_with_shadow_roots(&self, node: NodeId) -> String {
        self.serialize_scope_with(node, TraversalScope::IncludeNode, true)
    }

    fn serialize_scope(&self, node: NodeId, traversal_scope: TraversalScope) -> String {
        self.serialize_scope_with(node, traversal_scope, false)
    }

    fn serialize_scope_with(
        &self,
        node: NodeId,
        traversal_scope: TraversalScope,
        include_shadow: bool,
    ) -> String {
        let mut buf = Vec::new();
        let opts = SerializeOpts {
            traversal_scope,
            ..SerializeOpts::default()
        };
        // Writing to an in-memory Vec is infallible; html5ever emits UTF-8.
        html5ever_serialize(
            &mut buf,
            &SerializableNode {
                dom: self,
                id: node,
                include_shadow,
            },
            opts,
        )
        .expect("serializing the scripted DOM to a Vec is infallible");
        String::from_utf8(buf).expect("html5ever emits valid UTF-8")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use html5ever::{local_name, ns};
    use layout_dom_api::{LayoutDomMut, QualName};

    fn html(local: markup5ever::LocalName) -> QualName {
        QualName::new(None, ns!(html), local)
    }

    #[test]
    fn outer_html_emits_tag_attrs_and_escaped_text() {
        let mut dom = ScriptedDom::new();
        let div = dom.create_element(html(local_name!("div")));
        dom.set_attribute(div, QualName::new(None, ns!(), local_name!("class")), "x");
        let text = dom.create_text("hi & bye");
        dom.append_child(div, text);
        // html5ever owns the `&` escaping.
        assert_eq!(dom.outer_html(div), r#"<div class="x">hi &amp; bye</div>"#);
    }

    #[test]
    fn inner_html_skips_the_node_itself() {
        let mut dom = ScriptedDom::new();
        let div = dom.create_element(html(local_name!("div")));
        let span = dom.create_element(html(local_name!("span")));
        dom.append_child(div, span);
        assert_eq!(dom.inner_html(div), "<span></span>");
    }

    #[test]
    fn void_elements_get_no_close_tag() {
        let mut dom = ScriptedDom::new();
        let br = dom.create_element(html(local_name!("br")));
        assert_eq!(dom.outer_html(br), "<br>");
    }
}
