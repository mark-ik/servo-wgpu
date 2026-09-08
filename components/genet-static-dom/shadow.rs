/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Declarative shadow roots on the script-free tree.
//!
//! Mark's ruling for this lane: `<template shadowrootmode>` is recognized by a
//! **post-parse pass**, not by modifying html5ever. The parser therefore behaves
//! exactly as before and produces an ordinary `<template>` element with its
//! contents fragment; this pass then walks the finished tree, converts each
//! qualifying template into a shadow root on its parent, and computes the
//! resulting slot assignment once. There is no mutation afterwards on a static
//! document, so the tables are built once and read forever.
//!
//! Doing it here rather than in the parser also means the *same* recognition
//! rules serve the scripted tier, which runs the equivalent pass over its arena
//! after cloning this tree in.

use std::collections::HashMap;

use layout_dom_api::{ShadowRootInit, ShadowRootMode, SlotAssignmentMode, may_host_shadow_tree};

use crate::{StaticDocument, StaticNode, StaticNodeId, StaticNodeKind};

/// The per-document shadow tables. Empty (and free) for every document with no
/// declarative shadow root, which is nearly all of them.
#[derive(Clone, Debug, Default)]
pub(crate) struct ShadowTables {
    /// Host -> its shadow root.
    pub(crate) hosts: HashMap<StaticNodeId, StaticNodeId>,
    /// Slottable -> the slot it is assigned to.
    pub(crate) assigned_slot: HashMap<StaticNodeId, StaticNodeId>,
    /// Slot -> its assigned nodes, in tree order.
    pub(crate) assignment: HashMap<StaticNodeId, Vec<StaticNodeId>>,
}

impl ShadowTables {
    pub(crate) fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }
}

impl StaticDocument {
    /// Convert every `<template shadowrootmode="open|closed">` whose parent may
    /// host a shadow tree into a real shadow root, then compute slot assignment
    /// for each root produced. Idempotent: a second call finds no templates left.
    pub(crate) fn realize_declarative_shadow_roots(&mut self) {
        let mut templates = Vec::new();
        self.collect_declarative_templates(self.document, &mut templates);
        if templates.is_empty() {
            return;
        }
        for template in templates {
            self.realize_one(template);
        }
        let roots: Vec<StaticNodeId> = self.shadow.hosts.values().copied().collect();
        for root in roots {
            self.assign_slottables(root);
        }
    }

    /// Pre-order search for qualifying templates. Descends template contents
    /// too, so a declarative root nested inside another one is found.
    fn collect_declarative_templates(&self, node: StaticNodeId, out: &mut Vec<StaticNodeId>) {
        if self.declarative_shadow_init(node).is_some() {
            out.push(node);
        }
        for child in self.nodes[node.0].children.clone() {
            self.collect_declarative_templates(child, out);
        }
        if let Some(contents) = self.template_contents(node) {
            self.collect_declarative_templates(contents, out);
        }
    }

    /// The shadow-root init a `<template>` declares, if it declares one and its
    /// parent may host a shadow tree.
    fn declarative_shadow_init(&self, node: StaticNodeId) -> Option<ShadowRootInit> {
        let StaticNodeKind::Element { name, attrs, .. } = &self.nodes[node.0].kind else {
            return None;
        };
        if name.local.as_ref() != "template" {
            return None;
        }
        let attr = |want: &str| {
            attrs
                .iter()
                .find(|a| a.name.ns.as_ref().is_empty() && a.name.local.as_ref() == want)
                .map(|a| a.value.as_ref())
        };
        let mode = ShadowRootMode::parse(&attr("shadowrootmode")?.to_ascii_lowercase())?;
        let parent = self.nodes[node.0].parent?;
        let parent_local = match &self.nodes[parent.0].kind {
            StaticNodeKind::Element { name, .. } => name.local.as_ref().to_owned(),
            _ => return None,
        };
        if !may_host_shadow_tree(&parent_local) || self.shadow.hosts.contains_key(&parent) {
            return None;
        }
        Some(ShadowRootInit {
            mode,
            delegates_focus: attr("shadowrootdelegatesfocus").is_some(),
            clonable: attr("shadowrootclonable").is_some(),
            serializable: attr("shadowrootserializable").is_some(),
            slot_assignment: attr("shadowrootslotassignment")
                .and_then(|value| SlotAssignmentMode::parse(&value.to_ascii_lowercase()))
                .unwrap_or(SlotAssignmentMode::Named),
        })
    }

    /// Turn one qualifying `<template>` into its parent's shadow root: mint the
    /// root node, move the template's contents into it, and drop the template
    /// from the tree. HTML's own steps do exactly this at the parser; running it
    /// afterwards reaches the same tree.
    fn realize_one(&mut self, template: StaticNodeId) {
        let Some(init) = self.declarative_shadow_init(template) else {
            return;
        };
        let Some(host) = self.nodes[template.0].parent else {
            return;
        };
        let contents = self.template_contents(template);
        let root = StaticNodeId(self.nodes.len());
        self.nodes.push(StaticNode {
            parent: None,
            children: Vec::new(),
            kind: StaticNodeKind::ShadowRoot { host, init },
        });
        if let Some(contents) = contents {
            let children = std::mem::take(&mut self.nodes[contents.0].children);
            for child in &children {
                self.nodes[child.0].parent = Some(root);
            }
            self.nodes[root.0].children = children;
        }
        // Unlink the template itself: HTML never leaves a declarative template
        // in the tree, and leaving it would give the host an extra child that
        // slot assignment would then try to place.
        self.nodes[host.0]
            .children
            .retain(|child| *child != template);
        self.nodes[template.0].parent = None;
        self.shadow.hosts.insert(host, root);
    }

    /// "Assign slottables for a tree" on one static shadow root. Runs once, at
    /// parse time; a static document never mutates.
    fn assign_slottables(&mut self, root: StaticNodeId) {
        let StaticNodeKind::ShadowRoot { host, init } = self.nodes[root.0].kind.clone() else {
            return;
        };
        let mut slots = Vec::new();
        self.collect_slots(root, &mut slots);
        if slots.is_empty() {
            return;
        }
        // A static document has no `assign()` call site, so manual assignment
        // leaves every slot empty and every slot shows its fallback content.
        if init.slot_assignment == SlotAssignmentMode::Manual {
            return;
        }
        for child in self.nodes[host.0].children.clone() {
            if !matches!(
                self.nodes[child.0].kind,
                StaticNodeKind::Element { .. } | StaticNodeKind::Text(_)
            ) {
                continue;
            }
            let name = self
                .attribute_value(child, "slot")
                .unwrap_or_default()
                .to_owned();
            let Some(slot) = slots
                .iter()
                .copied()
                .find(|slot| self.attribute_value(*slot, "name").unwrap_or_default() == name)
            else {
                continue;
            };
            self.shadow.assignment.entry(slot).or_default().push(child);
            self.shadow.assigned_slot.insert(child, slot);
        }
    }

    /// Every `<slot>` in `root`'s tree in tree order, not descending into a
    /// nested host's own shadow tree (which is not a child of the host).
    fn collect_slots(&self, root: StaticNodeId, out: &mut Vec<StaticNodeId>) {
        for child in &self.nodes[root.0].children {
            let StaticNodeKind::Element { name, .. } = &self.nodes[child.0].kind else {
                continue;
            };
            if name.local.as_ref() == "slot" {
                out.push(*child);
            }
            self.collect_slots(*child, out);
        }
    }

    fn attribute_value(&self, id: StaticNodeId, local: &str) -> Option<&str> {
        let StaticNodeKind::Element { attrs, .. } = &self.nodes[id.0].kind else {
            return None;
        };
        attrs
            .iter()
            .find(|a| a.name.ns.as_ref().is_empty() && a.name.local.as_ref() == local)
            .map(|a| a.value.as_ref())
    }

    /// The shadow root attached to `host`, if any.
    pub fn shadow_root_of(&self, host: StaticNodeId) -> Option<StaticNodeId> {
        self.shadow.hosts.get(&host).copied()
    }

    /// The host of shadow root `root`, if `root` is one.
    pub fn shadow_host_of(&self, root: StaticNodeId) -> Option<StaticNodeId> {
        match &self.nodes[root.0].kind {
            StaticNodeKind::ShadowRoot { host, .. } => Some(*host),
            _ => None,
        }
    }

    /// The init dictionary a shadow root was declared with.
    pub fn shadow_init_of(&self, root: StaticNodeId) -> Option<ShadowRootInit> {
        match &self.nodes[root.0].kind {
            StaticNodeKind::ShadowRoot { init, .. } => Some(*init),
            _ => None,
        }
    }

    /// Every shadow root in this document, in node-id order.
    pub fn shadow_root_ids(&self) -> Vec<StaticNodeId> {
        let mut roots: Vec<StaticNodeId> = self.shadow.hosts.values().copied().collect();
        roots.sort_unstable_by_key(|id| id.0);
        roots
    }

    /// Whether this document holds any shadow root at all.
    pub fn has_shadow_roots(&self) -> bool {
        !self.shadow.is_empty()
    }

    /// The slot `node` is assigned to.
    pub fn assigned_slot_of(&self, node: StaticNodeId) -> Option<StaticNodeId> {
        self.shadow.assigned_slot.get(&node).copied()
    }

    /// The nodes assigned to `slot`, in tree order.
    pub fn assigned_nodes_of(&self, slot: StaticNodeId) -> &[StaticNodeId] {
        self.shadow
            .assignment
            .get(&slot)
            .map_or(&[][..], Vec::as_slice)
    }

    /// Flat-tree children of `id`. See [`LayoutDom::flat_children`].
    pub(crate) fn flat_children_of(&self, id: StaticNodeId) -> std::slice::Iter<'_, StaticNodeId> {
        if self.shadow.is_empty() {
            return self.nodes[id.0].children.iter();
        }
        if let Some(root) = self.shadow.hosts.get(&id) {
            return self.nodes[root.0].children.iter();
        }
        match self.shadow.assignment.get(&id) {
            Some(assigned) if !assigned.is_empty() => assigned.iter(),
            _ => self.nodes[id.0].children.iter(),
        }
    }
}
