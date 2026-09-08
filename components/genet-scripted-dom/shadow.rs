/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Shadow trees over the scripted arena: the shadow-root records, the per-host
//! slot assignment table, and the flat-tree children built from it.
//!
//! A shadow root is a real node in the same store (kind [`NodeKind::ShadowRoot`])
//! with **no parent**: it hangs off its host through the two maps here rather
//! than through the host's `children` vector, so `dom_children`, `childNodes`
//! and every existing DOM walk stay exactly what they were. The flat tree is
//! the only place the link is spliced back in, which is what
//! [`LayoutDom::flat_children`] means.
//!
//! Assignment is recomputed **eagerly**, per affected shadow root, at the
//! mutation that could change it — but every entry point exits on
//! `shadow_hosts.is_empty()` first, so a document with no shadow tree pays one
//! `HashMap::is_empty` per structural mutation and nothing else. A lazy cache
//! would need interior mutability on a `&self` read path for no gain: the
//! recompute is bounded by one host's children plus its shadow tree's slots,
//! not by the document.

use layout_dom_api::{LayoutDom, LocalName, Namespace, NodeKind, QualName};

use crate::{NodeId, ScriptedDom};

pub use layout_dom_api::{
    ShadowRootInit, ShadowRootMode, SlotAssignmentMode, may_host_shadow_tree,
};

/// Per-shadow-root state: the host link, the init dictionary, and the current
/// slot assignment table.
#[derive(Clone, Debug)]
pub(crate) struct ShadowRootData {
    pub(crate) host: NodeId,
    pub(crate) init: ShadowRootInit,
    /// Slots of this tree in tree order, each with the nodes assigned to it.
    /// The table, not a per-read walk: `assignedNodes` and `flat_children` are
    /// hot and must not re-derive assignment.
    pub(crate) slots: Vec<(NodeId, Vec<NodeId>)>,
    /// Manual assignment as last set by `HTMLSlotElement.assign()`, keyed by
    /// slot. Retained even while the mode is `named` (the spec keeps the
    /// manually assigned nodes list on the slot regardless) so switching modes
    /// is not lossy.
    pub(crate) manual: Vec<(NodeId, Vec<NodeId>)>,
    /// Built by the declarative post-parse pass from a `<template
    /// shadowrootmode>`, and not yet claimed by an `attachShadow`. HTML lets
    /// exactly one `attachShadow` reuse such a root instead of throwing.
    pub(crate) declarative: bool,
}

/// Why an [`assignment error`](ScriptedDom::attach_shadow) was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttachShadowError {
    /// The element is not one shadow roots may be attached to.
    NotSupported,
    /// The element already has a shadow root that cannot be reused.
    AlreadyAttached,
}

/// The children a flat-tree consumer sees, without allocating: every case
/// borrows a slice already held by the arena.
pub enum FlatChildren<'a> {
    /// A borrowed id slice — a node with no shadow involvement or a slot with
    /// nothing assigned (its own children are its fallback content) yields its
    /// `children`; a host yields its shadow root's; a slot with an assignment
    /// yields that assignment. Every case is a slice already in the arena, so
    /// the flat tree allocates nothing per child access.
    Dom(std::iter::Copied<std::slice::Iter<'a, NodeId>>),
}

impl Iterator for FlatChildren<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        match self {
            Self::Dom(iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Dom(iter) => iter.size_hint(),
        }
    }
}

impl ScriptedDom {
    /// Attach a shadow root to `host`, returning the new root's id. The caller
    /// has already decided that `host` is allowed one (`may_host_shadow_tree`
    /// plus its own custom-element policy); this enforces only the "already
    /// attached" rule, where a **declarative** root of the same mode is reused
    /// and emptied, per HTML's `attachShadow` step 5.
    pub fn attach_shadow(
        &mut self,
        host: NodeId,
        init: ShadowRootInit,
    ) -> Result<NodeId, AttachShadowError> {
        if self.node(host).kind != NodeKind::Element {
            return Err(AttachShadowError::NotSupported);
        }
        if let Some(existing) = self.shadow_root_of(host) {
            let data = &self.shadow_roots[&self.index(existing)];
            if !data.declarative || data.init.mode != init.mode {
                return Err(AttachShadowError::AlreadyAttached);
            }
            // Reuse the declarative root: empty it and clear the declarative
            // flag, so a second attachShadow on the same host now throws.
            self.replace_shadow_children(existing, Vec::new());
            let key = self.index(existing);
            if let Some(data) = self.shadow_roots.get_mut(&key) {
                data.declarative = false;
                data.init = init;
            }
            self.reassign_slots(existing);
            return Ok(existing);
        }
        Ok(self.install_shadow_root(host, init, false))
    }

    /// Attach a shadow root to `host` without the "already attached" check —
    /// the tree-copy path, which is reproducing a root that was already
    /// validated on the source tree.
    pub fn attach_shadow_unchecked(
        &mut self,
        host: NodeId,
        init: ShadowRootInit,
        declarative: bool,
    ) -> NodeId {
        self.install_shadow_root(host, init, declarative)
    }

    /// Install a fresh shadow root on `host`. `declarative` marks a root the
    /// post-parse pass built from a `<template shadowrootmode>`, which
    /// `attachShadow` is allowed to reuse once.
    pub(crate) fn install_shadow_root(
        &mut self,
        host: NodeId,
        init: ShadowRootInit,
        declarative: bool,
    ) -> NodeId {
        let root = self.push(crate::Node::new(NodeKind::ShadowRoot));
        let root_key = self.index(root);
        let host_key = self.index(host);
        self.shadow_roots.insert(
            root_key,
            ShadowRootData {
                host,
                init,
                slots: Vec::new(),
                manual: Vec::new(),
                declarative,
            },
        );
        self.shadow_hosts.insert(host_key, root);
        self.structure_epoch += 1;
        self.reassign_slots(root);
        root
    }

    /// The shadow root attached to `host`, regardless of mode.
    pub fn shadow_root_of(&self, host: NodeId) -> Option<NodeId> {
        if self.shadow_hosts.is_empty() {
            return None;
        }
        self.try_index(host)
            .and_then(|key| self.shadow_hosts.get(&key))
            .copied()
    }

    /// The host of shadow root `root`.
    pub fn shadow_host_of(&self, root: NodeId) -> Option<NodeId> {
        if self.shadow_roots.is_empty() {
            return None;
        }
        self.try_index(root)
            .and_then(|key| self.shadow_roots.get(&key))
            .map(|data| data.host)
    }

    /// The init dictionary a shadow root was created with.
    pub fn shadow_init(&self, root: NodeId) -> Option<ShadowRootInit> {
        self.try_index(root)
            .and_then(|key| self.shadow_roots.get(&key))
            .map(|data| data.init)
    }

    /// Whether `root` is still a declarative root `attachShadow` may reuse.
    pub fn shadow_is_declarative(&self, root: NodeId) -> bool {
        self.try_index(root)
            .and_then(|key| self.shadow_roots.get(&key))
            .is_some_and(|data| data.declarative)
    }

    /// Whether any shadow root exists in this document. Every flat-tree
    /// consumer's cheap first question.
    pub fn has_shadow_roots(&self) -> bool {
        !self.shadow_roots.is_empty()
    }

    /// Every shadow root in this document, in creation order (the store key is
    /// the monotonic mint order), so the enumeration is stable across runs.
    pub fn shadow_root_ids(&self) -> Vec<NodeId> {
        let mut keys: Vec<usize> = self.shadow_roots.keys().copied().collect();
        keys.sort_unstable();
        keys.into_iter().map(|key| self.pack(key)).collect()
    }

    /// The number of live shadow roots (a diagnostic and a test signal).
    pub fn shadow_root_count(&self) -> usize {
        self.shadow_roots.len()
    }

    /// The slot `node` is assigned to, if any.
    pub fn assigned_slot_of(&self, node: NodeId) -> Option<NodeId> {
        if self.shadow_hosts.is_empty() {
            return None;
        }
        self.try_index(node)
            .and_then(|key| self.assigned_slots.get(&key))
            .copied()
    }

    /// The nodes assigned to slot `slot`, in tree order. Empty when nothing is
    /// assigned, which is when the slot renders its own children instead.
    ///
    /// Read straight out of the flat slot -> nodes index rather than out of the
    /// owning root's table: `flat_children` calls this on every slot it walks,
    /// and finding the owning root first would make each call an ancestor walk.
    pub fn assigned_nodes_of(&self, slot: NodeId) -> Vec<NodeId> {
        if self.slot_assignments.is_empty() {
            return Vec::new();
        }
        self.try_index(slot)
            .and_then(|key| self.slot_assignments.get(&key))
            .cloned()
            .unwrap_or_default()
    }

    /// Borrowed form of [`assigned_nodes_of`](Self::assigned_nodes_of).
    pub(crate) fn assigned_nodes_slice(&self, slot: NodeId) -> Option<&[NodeId]> {
        if self.slot_assignments.is_empty() {
            return None;
        }
        self.try_index(slot)
            .and_then(|key| self.slot_assignments.get(&key))
            .map(Vec::as_slice)
    }

    /// The shadow root whose tree contains `node`, walking parent links only —
    /// a shadow root's own parent is `None`, so the walk stops at the tree it
    /// belongs to and never crosses into the host's tree.
    pub fn containing_shadow_root_of(&self, node: NodeId) -> Option<NodeId> {
        if self.shadow_roots.is_empty() {
            return None;
        }
        let mut current = node;
        loop {
            let entry = self.try_index(current).and_then(|k| self.nodes.get(&k))?;
            match entry.parent {
                Some(parent) => current = parent,
                None => break,
            }
        }
        (self.node(current).kind == NodeKind::ShadowRoot).then_some(current)
    }

    /// `HTMLSlotElement.assign()`: replace `slot`'s manually assigned nodes.
    /// Only meaningful while the containing root's mode is `manual`, but the
    /// list is stored either way (the spec keeps it on the slot).
    pub fn set_manual_assignment(&mut self, slot: NodeId, nodes: Vec<NodeId>) {
        let Some(root) = self.containing_shadow_root_of(slot) else {
            return;
        };
        let key = self.index(root);
        if let Some(data) = self.shadow_roots.get_mut(&key) {
            data.manual.retain(|(id, _)| *id != slot);
            data.manual.push((slot, nodes));
        }
        self.reassign_slots(root);
    }

    /// Drain the slots whose assigned-node list changed since the last drain.
    /// The bootstrap fires one `slotchange` per drained slot.
    pub fn take_slot_changes(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.slot_changes)
    }

    /// Flat-tree children of `id` — the rendering traversal's child iteration.
    /// See [`LayoutDom::flat_children`] for the three cases.
    pub(crate) fn flat_children_of(&self, id: NodeId) -> FlatChildren<'_> {
        if self.shadow_roots.is_empty() {
            return FlatChildren::Dom(self.node(id).children.iter().copied());
        }
        if let Some(root) = self.shadow_root_of(id) {
            return FlatChildren::Dom(self.node(root).children.iter().copied());
        }
        if let Some(assigned) = self.assigned_nodes_slice(id) {
            if !assigned.is_empty() {
                return FlatChildren::Dom(assigned.iter().copied());
            }
        }
        FlatChildren::Dom(self.node(id).children.iter().copied())
    }

    /// Whether `id` is an HTML `<slot>` element.
    pub(crate) fn is_slot_element(&self, id: NodeId) -> bool {
        self.node(id).kind == NodeKind::Element
            && self
                .node(id)
                .name
                .as_ref()
                .is_some_and(|name| name.local.as_ref() == "slot")
    }

    /// Re-run "assign slottables for a tree" on the root that `node` belongs to
    /// or hosts, if any. The one hook the mutators call; it exits on the empty
    /// map before touching anything, so a shadow-free document pays a hash
    /// emptiness check per mutation.
    pub(crate) fn reassign_for(&mut self, node: NodeId) {
        if self.shadow_hosts.is_empty() {
            return;
        }
        if let Some(root) = self.shadow_root_of(node) {
            self.reassign_slots(root);
        }
        if let Some(root) = self.containing_shadow_root_of(node) {
            self.reassign_slots(root);
        }
    }

    /// Re-run assignment after an attribute write, but only for the two
    /// attributes that can change it: `slot` on a light-DOM child, and `name`
    /// on a slot. Every other attribute write costs one namespace-and-local
    /// comparison.
    pub(crate) fn reassign_for_attribute(&mut self, node: NodeId, name: &QualName) {
        if self.shadow_hosts.is_empty() || !name.ns.as_ref().is_empty() {
            return;
        }
        match name.local.as_ref() {
            "slot" => {
                if let Some(parent) = self.node(node).parent {
                    self.reassign_for(parent);
                }
            },
            "name" => self.reassign_for(node),
            _ => {},
        }
    }

    /// Re-run assignment for the root that `parent` is the host of, or that
    /// contains `parent`. Used at insertion and removal, where the *parent* is
    /// what identifies the affected tree (a removed child no longer has one).
    pub(crate) fn reassign_for_parent(&mut self, parent: NodeId, child: NodeId) {
        if self.shadow_hosts.is_empty() {
            return;
        }
        self.reassign_for(parent);
        // A node leaving a host's children keeps a stale `assignedSlot` if the
        // host's own root was not the tree we just recomputed.
        if let Some(key) = self.try_index(child) {
            if self.assigned_slots.contains_key(&key) && self.node(child).parent.is_none() {
                self.assigned_slots.remove(&key);
            }
        }
    }

    /// "Assign slottables for a tree": recompute the whole assignment table of
    /// one shadow root and record a slot change for every slot whose list moved.
    ///
    /// Bounded by the host's children plus this tree's slots. It is *not* a
    /// document walk: the slot search descends the shadow tree but stops at a
    /// nested shadow host, whose slots belong to that host's own tree.
    pub(crate) fn reassign_slots(&mut self, root: NodeId) {
        let Some(key) = self.try_index(root) else {
            return;
        };
        let Some(data) = self.shadow_roots.get(&key) else {
            return;
        };
        let host = data.host;
        let mode = data.init.slot_assignment;
        let manual = data.manual.clone();
        let previous = data.slots.clone();

        let mut slots = Vec::new();
        self.collect_slots(root, &mut slots);

        let mut table: Vec<(NodeId, Vec<NodeId>)> =
            slots.iter().map(|slot| (*slot, Vec::new())).collect();

        match mode {
            SlotAssignmentMode::Named => {
                let children: Vec<NodeId> = self.node(host).children.clone();
                for child in children {
                    if !self.is_slottable(child) {
                        continue;
                    }
                    let name = self.slot_attribute(child);
                    if let Some(entry) = table
                        .iter_mut()
                        .find(|(slot, _)| self.slot_name(*slot) == name)
                    {
                        entry.1.push(child);
                    }
                }
            },
            SlotAssignmentMode::Manual => {
                for (slot, nodes) in &manual {
                    let Some(entry) = table.iter_mut().find(|(id, _)| id == slot) else {
                        continue;
                    };
                    for node in nodes {
                        // Only a child of the host is assignable, and a node is
                        // assigned to at most one slot.
                        if self.is_live(*node)
                            && self.node(*node).parent == Some(host)
                            && self.is_slottable(*node)
                        {
                            entry.1.push(*node);
                        }
                    }
                }
            },
        }

        // Refresh the node -> slot reverse index for this tree's host children,
        // and the slot -> nodes index `flat_children` reads.
        let host_children: Vec<NodeId> = self.node(host).children.clone();
        for child in host_children {
            if let Some(k) = self.try_index(child) {
                self.assigned_slots.remove(&k);
            }
        }
        for (slot, _) in &previous {
            if let Some(k) = self.try_index(*slot) {
                self.slot_assignments.remove(&k);
            }
        }
        for (slot, nodes) in &table {
            for node in nodes {
                if let Some(k) = self.try_index(*node) {
                    self.assigned_slots.insert(k, *slot);
                }
            }
            if let Some(k) = self.try_index(*slot) {
                if nodes.is_empty() {
                    self.slot_assignments.remove(&k);
                } else {
                    self.slot_assignments.insert(k, nodes.clone());
                }
            }
        }

        // Record slotchange for every slot whose assigned list actually moved,
        // including a slot that lost its whole list by being removed.
        for (slot, nodes) in &table {
            let before = previous
                .iter()
                .find(|(id, _)| id == slot)
                .map(|(_, nodes)| nodes.as_slice());
            match before {
                Some(before) if before == nodes.as_slice() => {},
                // A slot appearing for the first time with nothing assigned has
                // not changed anything script can observe.
                None if nodes.is_empty() => {},
                _ => self.slot_changes.push(*slot),
            }
        }
        for (slot, nodes) in &previous {
            if !nodes.is_empty() && !table.iter().any(|(id, _)| id == slot) {
                self.slot_changes.push(*slot);
            }
        }

        if let Some(data) = self.shadow_roots.get_mut(&key) {
            data.slots = table;
        }
    }

    /// Every `<slot>` in `root`'s tree, in tree order, not descending into a
    /// nested shadow host's own tree.
    fn collect_slots(&self, root: NodeId, out: &mut Vec<NodeId>) {
        let children: Vec<NodeId> = self.node(root).children.clone();
        for child in children {
            if self.node(child).kind != NodeKind::Element {
                continue;
            }
            if self.is_slot_element(child) {
                out.push(child);
            }
            self.collect_slots(child, out);
        }
    }

    /// A slottable is an element or a text node (DOM "slottable").
    fn is_slottable(&self, node: NodeId) -> bool {
        matches!(
            self.node(node).kind,
            NodeKind::Element | NodeKind::Text | NodeKind::CdataSection
        )
    }

    /// The `slot` attribute of a light-DOM child, or `""` for a text node and
    /// for an element without one.
    fn slot_attribute(&self, node: NodeId) -> String {
        self.attribute(node, &Namespace::from(""), &LocalName::from("slot"))
            .unwrap_or("")
            .to_owned()
    }

    /// A slot's `name` attribute, or `""`, which is the default slot.
    fn slot_name(&self, slot: NodeId) -> String {
        self.attribute(slot, &Namespace::from(""), &LocalName::from("name"))
            .unwrap_or("")
            .to_owned()
    }

    /// Replace a shadow root's children wholesale (used when `attachShadow`
    /// reuses a declarative root).
    fn replace_shadow_children(&mut self, root: NodeId, new_children: Vec<NodeId>) {
        let existing = std::mem::take(&mut self.node_mut(root).children);
        for child in existing {
            self.node_mut(child).parent = None;
            self.structure_epoch += 1;
        }
        for child in new_children {
            self.attach_silent(root, child);
        }
    }

    /// Drop the shadow-root bookkeeping for every entry the store no longer
    /// holds. Called after a sweep, so a collected host or root leaves no
    /// dangling table entry.
    pub(crate) fn prune_shadow_tables(&mut self) {
        if self.shadow_roots.is_empty() && self.assigned_slots.is_empty() {
            return;
        }
        let dead_roots: Vec<usize> = self
            .shadow_roots
            .iter()
            .filter(|(key, data)| {
                !self.nodes.contains_key(key)
                    || self
                        .try_index(data.host)
                        .is_none_or(|k| !self.nodes.contains_key(&k))
            })
            .map(|(key, _)| *key)
            .collect();
        for key in dead_roots {
            if let Some(data) = self.shadow_roots.remove(&key) {
                if let Some(host_key) = self.try_index(data.host) {
                    self.shadow_hosts.remove(&host_key);
                }
            }
        }
        let live_keys = std::mem::take(&mut self.nodes);
        self.shadow_hosts
            .retain(|host, _| live_keys.contains_key(host));
        self.assigned_slots
            .retain(|node, _| live_keys.contains_key(node));
        self.slot_assignments
            .retain(|slot, _| live_keys.contains_key(slot));
        self.nodes = live_keys;
        let live: Vec<NodeId> = self
            .slot_changes
            .iter()
            .copied()
            .filter(|slot| {
                self.try_index(*slot)
                    .is_some_and(|k| self.nodes.contains_key(&k))
            })
            .collect();
        self.slot_changes = live;
    }
}

/// `<template>` contents and the inert document that owns them.
///
/// Mark's ruling: template contents live in a **separate inert Document**, per
/// spec — one per owning document, shared by all its templates. The fragment
/// is parentless and unreferenced by the main tree, so every existing walk
/// (layout, serialization, selector matching, extraction) skips it without
/// being told to; inertness is a property of the shape, not a flag consumers
/// have to remember to check.
impl ScriptedDom {
    /// The shared inert template-contents owner document, minting it on first
    /// use. It is reachable from nothing, so `collect` keeps it alive only
    /// while a template's contents fragment does.
    pub fn template_owner_document(&mut self) -> NodeId {
        if let Some(id) = self.template_document.filter(|id| self.is_live(*id)) {
            return id;
        }
        let id = self.push(crate::Node::new(NodeKind::Document));
        self.template_document = Some(id);
        id
    }

    /// The contents fragment of template element `id`, if it has one.
    pub fn template_contents_of(&self, id: NodeId) -> Option<NodeId> {
        if self.template_contents.is_empty() {
            return None;
        }
        self.try_index(id)
            .filter(|key| self.nodes.contains_key(key))
            .and_then(|key| self.template_contents.get(&key))
            .copied()
            .filter(|contents| self.is_live(*contents))
    }

    /// The contents fragment of template element `id`, creating it if absent.
    pub fn ensure_template_contents(&mut self, id: NodeId) -> NodeId {
        if let Some(existing) = self.template_contents_of(id) {
            return existing;
        }
        let owner = self.template_owner_document();
        let fragment = self.push(crate::Node::new(NodeKind::DocumentFragment));
        let key = self.index(id);
        self.template_contents.insert(key, fragment);
        // Reachable contents retain their owner, not the other way around:
        // keeping the owner must not keep retired sibling fragments alive.
        let fragment_key = self.index(fragment);
        self.template_content_owners.insert(fragment_key, owner);
        fragment
    }

    /// Whether `id` is inside some template's contents fragment.
    ///
    /// A template's contents have no parent, so the topmost ancestor of
    /// anything inside one is the fragment itself; a node in the document
    /// proper tops out at the document node. HTML never executes a `<script>`
    /// found in template contents, and this is how the parser driver tells.
    pub fn is_in_template_contents(&self, id: NodeId) -> bool {
        if self.template_content_owners.is_empty() {
            return false;
        }
        let mut node = id;
        while let Some(parent) = LayoutDom::parent(self, node) {
            node = parent;
        }
        self.try_index(node)
            .is_some_and(|key| self.template_content_owners.contains_key(&key))
    }

    /// The live inert document that owns template contents, if one exists.
    pub fn template_owner_document_if_any(&self) -> Option<NodeId> {
        self.template_document.filter(|id| self.is_live(*id))
    }

    /// Retire metadata along with swept nodes. A pinned fragment can outlive
    /// its template, so its owner edge is maintained independently.
    pub(crate) fn prune_template_tables(&mut self) {
        let mut contents = std::mem::take(&mut self.template_contents);
        contents.retain(|template, fragment| {
            self.nodes.contains_key(template) && self.is_live(*fragment)
        });
        self.template_contents = contents;

        let mut owners = std::mem::take(&mut self.template_content_owners);
        owners.retain(|fragment, owner| self.nodes.contains_key(fragment) && self.is_live(*owner));
        self.template_content_owners = owners;
        self.template_document = self.template_owner_document_if_any();
    }

    /// Realize declarative shadow roots over the scripted arena: every
    /// `<template shadowrootmode>` whose parent may host one becomes that
    /// parent's shadow root, and the template leaves the tree. The scripted
    /// twin of `StaticDocument::realize_declarative_shadow_roots`, run after a
    /// parsed tree is cloned in, so a page reaches the same flat tree whether
    /// or not scripting is on.
    ///
    /// Returns the roots it created.
    pub fn realize_declarative_shadow_roots(&mut self, from: NodeId) -> Vec<NodeId> {
        let mut templates = Vec::new();
        self.collect_declarative_templates(from, &mut templates);
        let mut roots = Vec::new();
        for template in templates {
            if let Some(root) = self.realize_declarative_template(template) {
                roots.push(root);
            }
        }
        for root in &roots {
            self.reassign_slots(*root);
        }
        roots
    }

    fn collect_declarative_templates(&self, node: NodeId, out: &mut Vec<NodeId>) {
        if self.declarative_shadow_init(node).is_some() {
            out.push(node);
        }
        for child in self.node(node).children.clone() {
            self.collect_declarative_templates(child, out);
        }
        if let Some(contents) = self.template_contents_of(node) {
            self.collect_declarative_templates(contents, out);
        }
    }

    /// The shadow-root init a `<template>` declares, if it declares one on a
    /// parent that may host a shadow tree and does not already have one.
    fn declarative_shadow_init(&self, node: NodeId) -> Option<ShadowRootInit> {
        if !self.is_element_named(node, "template") {
            return None;
        }
        let attr = |want: &str| {
            self.attribute(node, &Namespace::from(""), &LocalName::from(want))
                .map(str::to_owned)
        };
        let mode = ShadowRootMode::parse(&attr("shadowrootmode")?.to_ascii_lowercase())?;
        let parent = self.node(node).parent?;
        let local = self.node(parent).name.as_ref()?.local.as_ref().to_owned();
        if !may_host_shadow_tree(&local) || self.shadow_root_of(parent).is_some() {
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

    fn realize_declarative_template(&mut self, template: NodeId) -> Option<NodeId> {
        let init = self.declarative_shadow_init(template)?;
        let host = self.node(template).parent?;
        let root = self.install_shadow_root(host, init, true);
        if let Some(contents) = self.template_contents_of(template) {
            let children = std::mem::take(&mut self.node_mut(contents).children);
            for child in children {
                self.node_mut(child).parent = None;
                self.attach_silent(root, child);
            }
        }
        // Drop the template from the tree; HTML never leaves one behind, and a
        // leftover would be an extra host child for slot assignment to place.
        let kids = &mut self.node_mut(host).children;
        if let Some(pos) = kids.iter().position(|c| *c == template) {
            kids.remove(pos);
        }
        self.node_mut(template).parent = None;
        self.structure_epoch += 1;
        Some(root)
    }

    fn is_element_named(&self, id: NodeId, local: &str) -> bool {
        self.node(id).kind == NodeKind::Element
            && self
                .node(id)
                .name
                .as_ref()
                .is_some_and(|name| name.local.as_ref() == local)
    }
}

#[cfg(test)]
mod template_collection_tests {
    use super::*;
    use layout_dom_api::LayoutDomMut;

    #[test]
    fn template_bookkeeping_returns_to_baseline_after_collection() {
        let mut dom = ScriptedDom::new();
        for _ in 0..1000 {
            let template = dom.create_element(QualName::new(
                None,
                Namespace::from("http://www.w3.org/1999/xhtml"),
                LocalName::from("template"),
            ));
            dom.ensure_template_contents(template);
            dom.collect([]);
            assert!(dom.template_contents.is_empty());
            assert!(dom.template_content_owners.is_empty());
            assert!(dom.template_document.is_none());
        }
    }
}
