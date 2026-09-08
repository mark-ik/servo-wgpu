/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Mutable scripted-DOM provider.
//!
//! `ScriptedDom` is the mutable sibling of `genet-static-dom`'s `StaticDocument`:
//! a `NodeId`-keyed arena that implements [`LayoutDom`] (read) and [`LayoutDomMut`]
//! (mutate), recording each structural change as a [`DomMutation`] for the
//! retained Livery document to translate into invalidation. The arena owns the
//! node data; JS reflectors bridge back to it by `NodeId` (via
//! `script-engine-api`'s `make_reflector`/`reflector_data`), so the engine never
//! owns DOM data.
//!
//! Scope (2026-05-23): structural mutation + the mutation stream. The reflector
//! bridge and Livery invalidation consumer now live above this render-neutral
//! arena. `set_inner_html` uses the shared fragment parser.

#![deny(unsafe_code)]

use engine_observables_api::{DomArenaStats, DomNodeKindStats};
use genet_static_dom::{StaticDocument, StaticNodeId};
use layout_dom_api::{
    AttributeView, DoctypeView, DomMutation, LayoutDom, LayoutDomMut, LocalName, Namespace,
    NodeKind, QualName,
};

mod forms;
pub mod parser;
mod serialize;
mod shadow;

pub use shadow::{
    AttachShadowError, ShadowRootInit, ShadowRootMode, SlotAssignmentMode, may_host_shadow_tree,
};

/// Opaque node identity: a stable index into the arena (slots are never reused, so
/// ids stay valid for the document's lifetime).
// `usize`-backed (pointer-sized) for the reflector bridge's raw identity. The
// retired Stylo adapter also required this shape; keeping it avoids an ABI-like
// identity migration without retaining that adapter.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId(usize);

impl NodeId {
    /// The raw arena index. The reflector bridge packs this into a
    /// `script_engine_api::ReflectorData` (`u64`) so JS can carry it opaquely.
    pub fn raw(self) -> usize {
        self.0
    }

    /// Rebuild a `NodeId` from a raw index recovered from a reflector.
    pub fn from_raw(raw: usize) -> Self {
        Self(raw)
    }
}

// --- G0: the document fence -------------------------------------------------
//
// Live `ScriptedDom`s are multiplying (chrome, workbench, roster, panes,
// cards, windows). A `NodeId` minted by one document and used against another
// is a silent wrong-node bug. To catch it, each document carries a
// process-unique `doc_tag`; on 64-bit *debug* builds the tag is packed into a
// `NodeId`'s high bits and every accessor `debug_assert`s ownership.
//
// On release and on wasm32 the packing and the asserts compile out entirely,
// so ids are the bare arena index exactly as before and behavior is
// byte-identical. wasm32 has no room to pack (a `usize` is 32 bits there, all
// of it spoken for by the index), and native debug runs already exercise the
// bug class. The packed value rides opaquely through `LayoutDom::opaque_id` and
// the reflector bridge's `raw()`/`from_raw()` u64 round-trip, so the tag survives
// both paths and the assert fires on reconstructed ids too.

#[cfg(all(debug_assertions, target_pointer_width = "64"))]
mod fence {
    /// Low bits of a `NodeId` carrying the arena index. 2^48 nodes per
    /// document is ample; the remaining 16 high bits carry the `doc_tag`.
    pub const INDEX_BITS: u32 = 48;
    pub const INDEX_MASK: usize = (1usize << INDEX_BITS) - 1;
    /// 16-bit tag space. On wraparound (65k+ documents in one process) tags
    /// may alias and the assert weakens to a heuristic; it never miscompares a
    /// same-tag id, so correctness is unaffected.
    pub const TAG_MASK: u64 = (1u64 << (64 - INDEX_BITS)) - 1;

    /// Mint a process-unique tag. Starts at 1 so the first document is nonzero.
    pub fn next_doc_tag() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        COUNTER.fetch_add(1, Ordering::Relaxed) & TAG_MASK
    }
}

/// A tiny deterministic FNV-1a hasher for the node store (G3). std `HashMap`'s
/// `RandomState` is seed-dependent (and its seed is an entropy question on
/// wasm32); fixing the hasher makes the store's iteration order identical on
/// every run and target, so pelt-live's byte-determinism stays airtight even
/// though the store is now a map rather than a dense slab.
#[derive(Default)]
struct FnvHasher(u64);

impl std::hash::Hasher for FnvHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        let mut h = if self.0 == 0 {
            0xcbf2_9ce4_8422_2325
        } else {
            self.0
        };
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.0 = h;
    }
}

/// The node store: a *prunable* map from a monotonic node value to its [`Node`].
/// Keyed by the untagged id value (the low bits the fence's `index` returns), so
/// the G0 doc-tag never reaches the key. Pruning dead entries is what bounds
/// memory to live nodes (G3); the deterministic hasher keeps it order-stable.
type NodeStore = std::collections::HashMap<usize, Node, std::hash::BuildHasherDefault<FnvHasher>>;

struct Node {
    kind: NodeKind,
    name: Option<QualName>,
    attrs: Vec<(QualName, String)>,
    text: Option<String>,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
}

impl Node {
    fn new(kind: NodeKind) -> Self {
        Self {
            kind,
            name: None,
            attrs: Vec::new(),
            text: None,
            parent: None,
            children: Vec::new(),
        }
    }
}

/// A mutable DOM arena. Nodes live in a prunable [`NodeStore`] keyed by a
/// monotonic id; a pruned id is "gone" (ids are never reused). Memory is bounded
/// to *live* nodes by [`collect`](ScriptedDom::collect) (G3), not to every node
/// ever created.
pub struct ScriptedDom {
    nodes: NodeStore,
    /// Monotonic id counter. The next node's untagged value; never decremented,
    /// so ids are never reused even as the store is pruned.
    next_id: usize,
    /// The primary document root (returned by `document()`) — the sole permanent
    /// mark root for `collect`. Everything else (secondary documents, fragments,
    /// detached subtrees) survives only via reachability from here or the host's
    /// pins, so a dropped secondary document collects like any other orphan.
    root: NodeId,
    mutations: Vec<DomMutation<NodeId>>,
    /// Absolute sequence number of the first pending mutation. A second
    /// observer can pair this base with [`Self::pending_mutations`] to read new
    /// facts without stealing them from layout and detect a range it missed.
    mutation_base: u64,
    /// The `MutationObserver` view of the same mutation points. Only recorded
    /// while [`set_observing`](Self::set_observing) is on, so a document nobody
    /// observes pays nothing and behaves exactly as before.
    observed: Vec<ObservedMutation>,
    observing: bool,
    /// Whether a document parser is building into this arena. Its open-element
    /// stack holds ids the arena knows nothing about, so while it is set a
    /// replaced subtree is orphaned rather than freed — `documentElement
    /// .innerHTML = ...` from a script the parser is running would otherwise
    /// pull the tree out from under the tree builder's own handles.
    parsing: bool,
    /// Index into `observed` where the current coalescing group began. A DOM
    /// operation that is one record to script but several arena mutations
    /// (`replaceChild`) opens a group so the childList records for one target
    /// merge into one.
    observed_group: Option<usize>,
    /// Monotonic counter of **structural** mutations — every write to a node's
    /// parent link. The reflector-identity policy caches each reflector's tree
    /// root and re-derives the cache whenever this moves, so a frame that only
    /// sets attributes or text pays nothing for the cache. Deliberately narrower
    /// than the `DomMutation` sequence number, which also advances on attribute
    /// and character-data facts that cannot move a node between trees.
    structure_epoch: u64,
    /// Shadow-root records, keyed by the shadow root's store key. Empty for
    /// every document that never attaches one, which is what lets the flat-tree
    /// and assignment hooks cost a `HashMap::is_empty` there.
    shadow_roots: std::collections::HashMap<usize, shadow::ShadowRootData>,
    /// Host store key -> its shadow root. The reverse of
    /// [`shadow::ShadowRootData::host`], kept because `shadowRoot` and
    /// `flat_children` both ask host-first.
    shadow_hosts: std::collections::HashMap<usize, NodeId>,
    /// Slottable store key -> the slot it is assigned to (`assignedSlot`).
    assigned_slots: std::collections::HashMap<usize, NodeId>,
    /// Slot store key -> its assigned nodes, in tree order. The flat tree's hot
    /// read; the per-root table in `ShadowRootData` is the diffing copy.
    slot_assignments: std::collections::HashMap<usize, Vec<NodeId>>,
    /// Slots whose assignment changed since the last drain. The bootstrap fires
    /// one `slotchange` per entry at the end of the microtask checkpoint.
    slot_changes: Vec<NodeId>,
    /// Template element store key -> its contents `DocumentFragment`. The
    /// fragment is parentless and is **not** in the main tree, so no ordinary
    /// walk reaches it; that is exactly what makes template contents inert.
    template_contents: std::collections::HashMap<usize, NodeId>,
    /// The one inert `Document` that owns every template's contents fragment,
    /// minted on the first `<template>` and shared by all of them — HTML's
    /// "appropriate template contents owner document", which is what makes
    /// `a.content.ownerDocument === b.content.ownerDocument` true.
    template_document: Option<NodeId>,
    /// Process-unique document tag (G0 fence). Only present where the fence is
    /// active; elsewhere ids are untagged and this field would be dead weight.
    #[cfg(all(debug_assertions, target_pointer_width = "64"))]
    doc_tag: u64,
}

impl Default for ScriptedDom {
    fn default() -> Self {
        Self::new()
    }
}

/// A spec-shaped mutation fact for the second consumer of the arena's mutation
/// point: `MutationObserver`. [`DomMutation`] is deliberately lossy — it carries
/// what invalidation needs — so the observer view is a parallel typed record
/// written by the same statements, not a re-derivation. `ancestors` is the
/// target's **inclusive** ancestor chain (target first) captured at mutation
/// time, which is what "interested observers" needs and what the live tree can
/// no longer answer once a later mutation moves the target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObservedMutation {
    /// A `childList` record: children of `target` were added and/or removed.
    ChildList {
        target: NodeId,
        added: Vec<NodeId>,
        removed: Vec<NodeId>,
        previous_sibling: Option<NodeId>,
        next_sibling: Option<NodeId>,
        ancestors: Vec<NodeId>,
    },
    /// An `attributes` record. `old_value` is `None` for a newly added attribute.
    Attributes {
        target: NodeId,
        name: QualName,
        old_value: Option<String>,
        ancestors: Vec<NodeId>,
    },
    /// A `characterData` record for a text or comment node.
    CharacterData {
        target: NodeId,
        old_value: Option<String>,
        ancestors: Vec<NodeId>,
    },
}

/// A set of node ids to treat as **extra mark roots** for
/// [`ScriptedDom::collect`] — nodes the document tree no longer reaches but that
/// must survive anyway. The host pins a node while script can still reach it (it
/// holds a reflector), so a pinned orphan and its whole connected component are
/// spared; unpinning it makes it collectable. The DOM only sees a pin set; the
/// host's word for these is "reflector pins" (it `pin`s on minting a reflector
/// and `retire`s the ones the engine reports dead). Engine-agnostic — it traffics
/// only in [`NodeId`], naming no engine type.
#[derive(Debug, Default, Clone)]
pub struct Pins {
    pinned: std::collections::HashSet<NodeId>,
}

impl Pins {
    /// An empty pin set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pin `id` (script can still reach it). Idempotent.
    pub fn pin(&mut self, id: NodeId) {
        self.pinned.insert(id);
    }

    /// Drop the pin for `id`. Returns `true` if it had been pinned (an id never
    /// pinned here is a harmless no-op).
    pub fn unpin(&mut self, id: NodeId) -> bool {
        self.pinned.remove(&id)
    }

    /// Retire a batch of now-dead ids (the host maps the engine's
    /// `drain_dead_reflectors` output through `NodeId::from_raw` first),
    /// unpinning each. Returns how many were actually pinned.
    pub fn retire_dead(&mut self, dead: impl IntoIterator<Item = NodeId>) -> usize {
        dead.into_iter().filter(|id| self.pinned.remove(id)).count()
    }

    /// Whether `id` is currently pinned.
    pub fn is_pinned(&self, id: NodeId) -> bool {
        self.pinned.contains(&id)
    }

    /// The pinned ids — pass to [`ScriptedDom::collect`] as the extra roots.
    pub fn iter(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.pinned.iter().copied()
    }

    /// Number of pinned ids.
    pub fn len(&self) -> usize {
        self.pinned.len()
    }

    /// Whether nothing is pinned.
    pub fn is_empty(&self) -> bool {
        self.pinned.is_empty()
    }

    /// Drop every pin (document teardown).
    pub fn clear(&mut self) {
        self.pinned.clear();
    }
}

impl ScriptedDom {
    /// A fresh document with an empty `Document` root.
    pub fn new() -> Self {
        let mut dom = Self {
            nodes: NodeStore::default(),
            next_id: 0,
            // Placeholder; overwritten by the `push` below so the root id
            // carries this document's tag like every other node.
            root: NodeId(0),
            mutations: Vec::new(),
            mutation_base: 0,
            observed: Vec::new(),
            observing: false,
            parsing: false,
            observed_group: None,
            structure_epoch: 0,
            shadow_roots: std::collections::HashMap::new(),
            shadow_hosts: std::collections::HashMap::new(),
            assigned_slots: std::collections::HashMap::new(),
            slot_assignments: std::collections::HashMap::new(),
            slot_changes: Vec::new(),
            template_contents: std::collections::HashMap::new(),
            template_document: None,
            #[cfg(all(debug_assertions, target_pointer_width = "64"))]
            doc_tag: fence::next_doc_tag(),
        };
        dom.root = dom.push(Node::new(NodeKind::Document));
        dom
    }

    /// Pack a monotonic id value into a `NodeId`, tagging it with this document
    /// on a fenced build. Off the fence this is the bare value (today's behavior).
    #[cfg(all(debug_assertions, target_pointer_width = "64"))]
    fn pack(&self, value: usize) -> NodeId {
        debug_assert!(value <= fence::INDEX_MASK, "scripted-dom node id overflow");
        NodeId((((self.doc_tag & fence::TAG_MASK) << fence::INDEX_BITS) as usize) | value)
    }
    #[cfg(not(all(debug_assertions, target_pointer_width = "64")))]
    fn pack(&self, value: usize) -> NodeId {
        NodeId(value)
    }

    /// Resolve a `NodeId` to its store key (the untagged id value), asserting it
    /// belongs to this document first. Every accessor that reads the store goes
    /// through here.
    #[cfg(all(debug_assertions, target_pointer_width = "64"))]
    fn index(&self, id: NodeId) -> usize {
        debug_assert!(
            (id.0 >> fence::INDEX_BITS) as u64 == (self.doc_tag & fence::TAG_MASK),
            "NodeId from a different document (id tag {}, this doc {})",
            (id.0 >> fence::INDEX_BITS) as u64,
            self.doc_tag & fence::TAG_MASK,
        );
        id.0 & fence::INDEX_MASK
    }
    #[cfg(not(all(debug_assertions, target_pointer_width = "64")))]
    #[inline]
    fn index(&self, id: NodeId) -> usize {
        id.0
    }

    /// Like [`index`](Self::index) but **non-asserting**: returns `None` for an
    /// id minted by a different document (on a fenced build) instead of
    /// panicking, so [`is_live`](Self::is_live) can answer for any id.
    #[cfg(all(debug_assertions, target_pointer_width = "64"))]
    fn try_index(&self, id: NodeId) -> Option<usize> {
        if (id.0 >> fence::INDEX_BITS) as u64 == (self.doc_tag & fence::TAG_MASK) {
            Some(id.0 & fence::INDEX_MASK)
        } else {
            None
        }
    }
    #[cfg(not(all(debug_assertions, target_pointer_width = "64")))]
    #[inline]
    fn try_index(&self, id: NodeId) -> Option<usize> {
        Some(id.0)
    }

    /// Normalize `id` for capture/replay: strip any debug doc-tag fence and return
    /// the bare arena index the recorder writes.
    pub fn capture_node_id(&self, id: NodeId) -> u64 {
        self.index(id) as u64
    }

    /// Re-mint a captured bare arena index against this document, restoring this
    /// document's tag on a fenced build.
    pub fn remint_node_id(&self, raw: u64) -> NodeId {
        self.pack(usize::try_from(raw).expect("captured node id must fit in usize"))
    }

    /// Non-consuming view of the pending mutation batch. The returned base is
    /// the absolute sequence number of its first mutation. A retained consumer
    /// whose cursor is below the base knows another consumer drained facts it
    /// had not observed and can take a conservative correctness path.
    pub fn pending_mutations(&self) -> (u64, &[DomMutation<NodeId>]) {
        (self.mutation_base, &self.mutations)
    }

    /// Turn the `MutationObserver` record on or off. Off (the default) is
    /// zero-cost: no record is built and no node is kept alive for one. Turning
    /// it off drops whatever has not been taken.
    pub fn set_observing(&mut self, on: bool) {
        self.observing = on;
        if !on {
            self.observed.clear();
        }
    }

    /// Tell the arena a document parser is building into it. See
    /// [`ScriptedDom::parsing`]: while set, nothing frees a subtree, because
    /// the tree builder holds handles the arena cannot see. Cleared when the
    /// parse ends, after which ordinary collection reclaims whatever the
    /// parser left orphaned.
    pub fn set_parsing(&mut self, on: bool) {
        self.parsing = on;
    }

    /// Whether the observer record is being written.
    pub fn is_observing(&self) -> bool {
        self.observing
    }

    /// Take the pending observer records. Unlike [`pending_mutations`](Self::pending_mutations)
    /// this is a drain: the observer registry is the only consumer of this view,
    /// and Livery's [`DomMutation`] stream is untouched by it.
    pub fn take_observed(&mut self) -> Vec<ObservedMutation> {
        self.observed_group = None;
        std::mem::take(&mut self.observed)
    }

    /// Open or close a coalescing group. Inside one, childList records naming
    /// the same target merge — the added and removed lists concatenate, the
    /// group keeps the first record's previous sibling and the last one's next
    /// sibling. That is what makes `replaceChild` one record rather than the
    /// insert and the remove it is built from.
    pub fn set_observer_group(&mut self, on: bool) {
        self.observed_group = if on && self.observing {
            Some(self.observed.len())
        } else {
            None
        };
    }

    /// The target's inclusive ancestor chain, target first.
    fn ancestor_chain(&self, node: NodeId) -> Vec<NodeId> {
        let mut chain = vec![node];
        let mut cursor = node;
        while let Some(parent) = self.node(cursor).parent {
            chain.push(parent);
            cursor = parent;
        }
        chain
    }

    /// The implicit removal of a node from its former parent when it is
    /// inserted elsewhere. It is a record of its own even inside a group: the
    /// spec queues it before the group's own record, not merged into it.
    fn record_implicit_removal(
        &mut self,
        target: NodeId,
        node: NodeId,
        previous_sibling: Option<NodeId>,
        next_sibling: Option<NodeId>,
    ) {
        let group = self.observed_group.take();
        self.record_child_list(
            target,
            Vec::new(),
            vec![node],
            previous_sibling,
            next_sibling,
        );
        self.observed_group = group;
    }

    fn record_child_list(
        &mut self,
        target: NodeId,
        added: Vec<NodeId>,
        removed: Vec<NodeId>,
        previous_sibling: Option<NodeId>,
        next_sibling: Option<NodeId>,
    ) {
        if !self.observing || (added.is_empty() && removed.is_empty()) {
            return;
        }
        if let Some(start) = self.observed_group {
            let merged = self.observed[start..]
                .iter_mut()
                .find_map(|record| match record {
                    ObservedMutation::ChildList {
                        target: existing,
                        added: into_added,
                        removed: into_removed,
                        next_sibling: into_next,
                        ..
                    } if *existing == target => Some((into_added, into_removed, into_next)),
                    _ => None,
                });
            if let Some((into_added, into_removed, into_next)) = merged {
                into_added.extend(added);
                into_removed.extend(removed);
                *into_next = next_sibling;
                return;
            }
        }
        let ancestors = self.ancestor_chain(target);
        self.observed.push(ObservedMutation::ChildList {
            target,
            added,
            removed,
            previous_sibling,
            next_sibling,
            ancestors,
        });
    }

    fn record_attribute(&mut self, target: NodeId, name: QualName, old_value: Option<String>) {
        if !self.observing {
            return;
        }
        let ancestors = self.ancestor_chain(target);
        self.observed.push(ObservedMutation::Attributes {
            target,
            name,
            old_value,
            ancestors,
        });
    }

    fn record_character_data(&mut self, target: NodeId, old_value: Option<String>) {
        if !self.observing {
            return;
        }
        let ancestors = self.ancestor_chain(target);
        self.observed.push(ObservedMutation::CharacterData {
            target,
            old_value,
            ancestors,
        });
    }

    /// Free `node`'s subtree unless an observer is watching. A record naming a
    /// removed node must be able to hand that node to script, so while observing
    /// the subtree is only orphaned; [`collect`](Self::collect) reclaims it once
    /// nothing reaches it.
    fn release_subtree(&mut self, node: NodeId) {
        if !self.observing && !self.parsing {
            self.drop_subtree(node);
        }
    }

    /// DOM `removeChild`: orphan `child` from its parent but keep it (and its
    /// subtree) alive and re-insertable, recording a [`DomMutation::Removed`].
    /// Unlike [`LayoutDomMut::remove`](layout_dom_api::LayoutDomMut::remove), which
    /// also drops the subtree — script may hold a reference to a removed node and
    /// re-insert it, so the scripted DOM orphans rather than frees.
    pub fn remove_child(&mut self, child: NodeId) {
        let former_parent = self.node(child).parent;
        let previous = self.sibling(child, -1);
        let next = self.sibling(child, 1);
        self.detach(child);
        if let Some(former_parent) = former_parent {
            self.mutations.push(DomMutation::Removed {
                node: child,
                former_parent,
            });
            self.record_child_list(former_parent, Vec::new(), vec![child], previous, next);
            self.reassign_for_parent(former_parent, child);
        }
    }

    /// Implement the DOM `textContent` setter without leaving parsed text
    /// children behind. Text and comment nodes change their own character data;
    /// container nodes replace their descendants with one new text node (or no
    /// child for the empty string), so every layout consumer observes the same
    /// tree as script.
    pub fn set_text_content(&mut self, node: NodeId, data: &str) {
        if matches!(
            self.node(node).kind,
            NodeKind::Text
                | NodeKind::Comment
                | NodeKind::CdataSection
                | NodeKind::ProcessingInstruction
        ) {
            let old = self.node(node).text.clone();
            self.node_mut(node).text = Some(data.to_owned());
            self.mutations
                .push(DomMutation::CharacterDataChanged { node });
            self.record_character_data(node, old);
            return;
        }

        let existing = std::mem::take(&mut self.node_mut(node).children);
        let removed = existing.clone();
        for child in existing {
            self.node_mut(child).parent = None;
            self.structure_epoch += 1;
            // Replacement detaches like removeChild: a reflector or queued
            // record can still retain this subtree. Only collect sees all pins.
        }
        self.node_mut(node).text = None;
        let mut added = Vec::new();
        if !data.is_empty() {
            let text = self.create_text(data);
            self.attach_silent(node, text);
            added.push(text);
        }
        self.mutations.push(DomMutation::SubtreeReplaced { node });
        self.record_child_list(node, added, removed, None, None);
        self.reassign_for(node);
    }

    /// Create a detached `Document` node (a second document, for
    /// `DOMImplementation.createDocument` / `createHTMLDocument`). Lives in the same
    /// store as the primary document, so `NodeId`s stay globally unique. It is
    /// **pin-kept**, not a permanent root (G3): while script holds a reflector to
    /// it (or anything in it) the host pins it and `collect` spares the whole
    /// component; once script drops it, it collects like any other orphan.
    pub fn create_document(&mut self) -> NodeId {
        self.push(Node::new(NodeKind::Document))
    }

    /// Create a detached `Comment` node carrying `data`.
    pub fn create_comment(&mut self, data: &str) -> NodeId {
        let mut node = Node::new(NodeKind::Comment);
        node.text = Some(data.to_owned());
        self.push(node)
    }

    /// Create a detached `ProcessingInstruction` node. The target lives in
    /// `name` (it is `nodeName`) and the data in `text`, so the character-data
    /// readers and writers already reach it.
    pub fn create_processing_instruction(&mut self, target: &str, data: &str) -> NodeId {
        let mut node = Node::new(NodeKind::ProcessingInstruction);
        node.name = Some(QualName::new(
            None,
            Namespace::from(""),
            LocalName::from(target),
        ));
        node.text = Some(data.to_owned());
        self.push(node)
    }

    /// Create a detached `DocumentFragment` node (a parentless container).
    pub fn create_fragment(&mut self) -> NodeId {
        self.push(Node::new(NodeKind::DocumentFragment))
    }

    /// Create a detached `CDATASection` node carrying `data`. Only XML documents
    /// may hold one; the caller enforces that.
    pub fn create_cdata_section(&mut self, data: &str) -> NodeId {
        let mut node = Node::new(NodeKind::CdataSection);
        node.text = Some(data.to_owned());
        self.push(node)
    }

    /// Create a detached `DocumentType` node. The name lives in `text` (which is
    /// what the html5ever serializer already reads for a doctype) and the two
    /// external identifiers ride in `attrs` under the reserved names below, so no
    /// per-node storage is added for a node kind a document has at most one of.
    pub fn create_doctype(&mut self, name: &str, public_id: &str, system_id: &str) -> NodeId {
        let mut node = Node::new(NodeKind::Doctype);
        node.text = Some(name.to_owned());
        node.attrs.push((
            QualName::new(
                None,
                Namespace::from(""),
                LocalName::from(DOCTYPE_PUBLIC_ID),
            ),
            public_id.to_owned(),
        ));
        node.attrs.push((
            QualName::new(
                None,
                Namespace::from(""),
                LocalName::from(DOCTYPE_SYSTEM_ID),
            ),
            system_id.to_owned(),
        ));
        self.push(node)
    }

    /// The stored `QualName` of the attribute on `id` whose **qualified** name is
    /// `qname` — what `getAttribute` / `removeAttribute` match on, as opposed to
    /// the namespace + local pair `LayoutDom::attribute` takes.
    pub fn attribute_qual_name(&self, id: NodeId, qname: &str) -> Option<QualName> {
        self.node(id)
            .attrs
            .iter()
            .find(|(name, _)| qualified_name(name) == qname)
            .map(|(name, _)| name.clone())
    }

    /// Every attribute on `id` as (namespace, prefix, local name), in insertion
    /// order — the shape a `NamedNodeMap` enumerates.
    pub fn attribute_names(&self, id: NodeId) -> Vec<(String, String, String)> {
        self.node(id)
            .attrs
            .iter()
            .map(|(name, _)| {
                (
                    name.ns.as_ref().to_string(),
                    name.prefix
                        .as_ref()
                        .map(|p| p.as_ref().to_string())
                        .unwrap_or_default(),
                    name.local.as_ref().to_string(),
                )
            })
            .collect()
    }

    /// Rebuild a fresh document from serialized document children (the same
    /// shape `inner_html(document())` emits for capture).
    pub fn from_serialized_document(html: &str) -> Self {
        let src = StaticDocument::parse(html);
        let mut dom = Self::new();
        let children: Vec<StaticNodeId> = src.dom_children(src.document()).collect();
        for child in children {
            let copied = dom.copy_fragment_node(&src, child);
            dom.attach_silent(dom.root, copied);
        }
        dom.mutations.clear();
        dom
    }

    /// Import exactly one serialized subtree as a detached node.
    pub fn import_serialized_subtree(&mut self, html: &str) -> Result<NodeId, String> {
        let fragment = StaticDocument::parse_fragment_source(html);
        let mut candidates = if let Some(body) = Self::fragment_body(&fragment) {
            let body_children: Vec<StaticNodeId> = fragment.dom_children(body).collect();
            if !body_children.is_empty() {
                body_children
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        if candidates.is_empty() {
            candidates = fragment
                .dom_children(fragment.document())
                .filter(|&child| {
                    !fragment
                        .element_name(child)
                        .is_some_and(|q| q.local.as_ref() == "html")
                })
                .collect();
        }
        if candidates.len() != 1 {
            return Err(format!(
                "serialized subtree must produce exactly one top-level node, got {}",
                candidates.len()
            ));
        }
        Ok(self.copy_fragment_node(&fragment, candidates[0]))
    }

    fn node(&self, id: NodeId) -> &Node {
        self.nodes
            .get(&self.index(id))
            .expect("NodeId refers to a live node")
    }

    fn node_mut(&mut self, id: NodeId) -> &mut Node {
        let i = self.index(id);
        self.nodes
            .get_mut(&i)
            .expect("NodeId refers to a live node")
    }

    fn push(&mut self, node: Node) -> NodeId {
        let value = self.next_id;
        self.next_id += 1;
        self.nodes.insert(value, node);
        self.pack(value)
    }

    fn sibling(&self, id: NodeId, delta: isize) -> Option<NodeId> {
        let parent = self.node(id).parent?;
        let kids = &self.node(parent).children;
        let pos = kids.iter().position(|&c| c == id)?;
        let target = pos as isize + delta;
        if target < 0 {
            return None;
        }
        kids.get(target as usize).copied()
    }

    /// Unlink `child` from its current parent (no mutation recorded).
    fn detach(&mut self, child: NodeId) {
        if let Some(parent) = self.node(child).parent {
            let kids = &mut self.node_mut(parent).children;
            if let Some(pos) = kids.iter().position(|&c| c == child) {
                kids.remove(pos);
            }
        }
        self.node_mut(child).parent = None;
        self.structure_epoch += 1;
    }

    /// The shared tree surgery behind `insert_before` / `move_before`: detach
    /// `child`, re-link it under `parent` before `reference` (no mutation
    /// recorded — the callers record what the operation *means*). The insertion
    /// index resolves *after* detaching (so a move within the same parent
    /// reflects the post-detach positions); a missing or non-child reference
    /// falls back to append.
    fn attach_at(&mut self, parent: NodeId, child: NodeId, reference: Option<NodeId>) {
        self.detach(child);
        self.node_mut(child).parent = Some(parent);
        self.structure_epoch += 1;
        let idx = reference.and_then(|r| self.node(parent).children.iter().position(|&c| c == r));
        let kids = &mut self.node_mut(parent).children;
        match idx {
            Some(i) => kids.insert(i, child),
            None => kids.push(child),
        }
    }

    /// Free a node and its whole subtree (entries removed from the store).
    fn drop_subtree(&mut self, node: NodeId) {
        let children = std::mem::take(&mut self.node_mut(node).children);
        for child in children {
            self.drop_subtree(child);
        }
        let i = self.index(node);
        self.nodes.remove(&i);
    }

    /// Mark-sweep collection (G3 dangle contract). Prune every node not reachable
    /// — by **undirected** walk over parent + child edges — from a document root
    /// or one of `extra_roots`. `extra_roots` are the host's live-reflector pins
    /// ([`ReflectorPins`](../genet_scripted/struct.ReflectorPins.html)); passing
    /// them keeps a pinned orphan's whole connected component alive (JS can walk
    /// `parentNode` up and children down and re-insert any of it). The DOM stays
    /// pin-agnostic — it takes extra roots, not reflector knowledge. Returns the
    /// number of nodes pruned.
    ///
    /// Idempotent and cheap to call often: the host drives it at the
    /// `drain_mutations` boundary, on an idle tick, and right after an unpin.
    pub fn collect(&mut self, extra_roots: impl IntoIterator<Item = NodeId>) -> usize {
        // Mark: undirected reachability from the primary document root + extra
        // roots (secondary documents and fragments survive only via the pins).
        let mut marked: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut stack: Vec<usize> = Vec::new();
        self.seed_mark(self.root, &mut marked, &mut stack);
        for r in extra_roots {
            self.seed_mark(r, &mut marked, &mut stack);
        }
        while let Some(v) = stack.pop() {
            // This node's neighbours (parent + children) as owned ids, dropping
            // the store borrow before recursing.
            // Neighbours are parent + children **plus the shadow edge**: a
            // shadow root is not its host's child, so without the edge a live
            // host would sweep its own shadow tree (and a pinned node inside a
            // shadow tree would not keep its host alive).
            let neighbours: Vec<NodeId> = match self.nodes.get(&v) {
                Some(node) => node
                    .parent
                    .into_iter()
                    .chain(node.children.iter().copied())
                    .chain(self.shadow_hosts.get(&v).copied())
                    .chain(self.shadow_roots.get(&v).map(|data| data.host))
                    .chain(self.template_contents.get(&v).copied())
                    .collect(),
                None => continue,
            };
            for nbr in neighbours {
                self.seed_mark(nbr, &mut marked, &mut stack);
            }
        }

        // Sweep: prune every unmarked entry.
        let before = self.nodes.len();
        self.nodes.retain(|k, _| marked.contains(k));
        let pruned = before - self.nodes.len();
        self.prune_shadow_tables();
        pruned
    }

    /// The structural-mutation counter (see [`structure_epoch`] on the struct).
    /// A consumer that caches anything derived from the parent links — the
    /// reflector-identity policy's tree-root cache — holds this alongside the
    /// cache and rebuilds when it moves.
    ///
    /// [`structure_epoch`]: Self::structure_epoch
    pub fn structure_epoch(&self) -> u64 {
        self.structure_epoch
    }

    /// The root of `id`'s tree: the topmost ancestor reachable by parent links.
    /// For a node in the document that is the document node; for a detached
    /// subtree it is the subtree's top. `None` if `id` is not live.
    ///
    /// This is the *opaque root* of the reflector-identity policy: a wrapper is
    /// alive while its opaque root is alive, and the document is always alive.
    pub fn tree_root(&self, id: NodeId) -> Option<NodeId> {
        let mut cursor = self.try_index(id).filter(|i| self.nodes.contains_key(i))?;
        let mut out = id;
        // Bounded by tree depth; the store is acyclic by construction. A shadow
        // root has no parent, so the walk continues through its **host**: a
        // shadow tree hangs off its host, and the reflector-identity policy
        // needs every wrapper in it to share the host's opaque root, or a
        // connected component's wrappers would split into two liveness groups.
        loop {
            if let Some(parent) = self.nodes.get(&cursor).and_then(|n| n.parent) {
                out = parent;
                cursor = self.index(parent);
                continue;
            }
            match self.shadow_roots.get(&cursor) {
                Some(data) => match self.try_index(data.host) {
                    Some(host) if self.nodes.contains_key(&host) => {
                        out = data.host;
                        cursor = host;
                    },
                    _ => break,
                },
                None => break,
            }
        }
        Some(out)
    }

    /// The number of live nodes currently in the store. Bounded by `collect`
    /// (G3), not by total nodes ever created — a diagnostic for the host and the
    /// regression signal for the bounded-memory churn test.
    pub fn live_node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Cheap live counts plus a rough byte estimate for the DOM arena.
    pub fn stats(&self) -> DomArenaStats {
        let mut node_kinds = DomNodeKindStats::default();
        let mut attribute_count = 0usize;
        let mut estimated_bytes = std::mem::size_of::<Self>()
            + self.nodes.capacity() * (std::mem::size_of::<usize>() + std::mem::size_of::<Node>())
            + self.mutations.capacity() * std::mem::size_of::<DomMutation<NodeId>>();

        for node in self.nodes.values() {
            match node.kind {
                NodeKind::Document => node_kinds.documents += 1,
                NodeKind::DocumentFragment => node_kinds.document_fragments += 1,
                NodeKind::Doctype => node_kinds.doctypes += 1,
                NodeKind::Element => node_kinds.elements += 1,
                // A `CDATASection` is a `Text` in the DOM's own hierarchy, and
                // the observable stats have no separate counter for it.
                NodeKind::Text | NodeKind::CdataSection => node_kinds.text += 1,
                NodeKind::Comment => node_kinds.comments += 1,
                NodeKind::ProcessingInstruction => node_kinds.processing_instructions += 1,
                // A shadow root **is** a DocumentFragment in the DOM's own
                // hierarchy (nodeType 11); the observable stats have no separate
                // counter, so it counts as one.
                NodeKind::ShadowRoot => node_kinds.document_fragments += 1,
            }

            attribute_count += node.attrs.len();
            estimated_bytes += node.attrs.capacity() * std::mem::size_of::<(QualName, String)>();
            estimated_bytes += node.children.capacity() * std::mem::size_of::<NodeId>();
            estimated_bytes += node.name.as_ref().map_or(0, qual_name_bytes);
            estimated_bytes += node.text.as_ref().map_or(0, String::capacity);
            for (name, value) in &node.attrs {
                estimated_bytes += qual_name_bytes(name);
                estimated_bytes += value.capacity();
            }
        }

        DomArenaStats {
            live_nodes: self.nodes.len(),
            node_kinds,
            attribute_count,
            estimated_bytes,
        }
    }

    /// Mark `id`'s store key live (if it resolves to a live node here) and queue
    /// it for the `collect` walk. Skips foreign/dead ids and already-marked ones.
    fn seed_mark(
        &self,
        id: NodeId,
        marked: &mut std::collections::HashSet<usize>,
        stack: &mut Vec<usize>,
    ) {
        if let Some(v) = self.try_index(id) {
            if self.nodes.contains_key(&v) && marked.insert(v) {
                stack.push(v);
            }
        }
    }

    /// Link `child` under `parent` without recording a mutation. Used while
    /// building a parsed subtree, which is covered by one `SubtreeReplaced`.
    fn attach_silent(&mut self, parent: NodeId, child: NodeId) {
        self.node_mut(child).parent = Some(parent);
        self.structure_epoch += 1;
        self.node_mut(parent).children.push(child);
    }

    /// Deep-copy a node from a parsed [`StaticDocument`] into this arena (silent),
    /// returning the new id.
    fn copy_fragment_node(&mut self, src: &StaticDocument, sid: StaticNodeId) -> NodeId {
        let new = match src.kind(sid) {
            NodeKind::Element => {
                let mut node = Node::new(NodeKind::Element);
                node.name = src.element_name(sid).cloned();
                for attr in src.attributes(sid) {
                    node.attrs.push((attr.name.clone(), attr.value.to_owned()));
                }
                self.push(node)
            },
            kind @ (NodeKind::Text | NodeKind::Comment | NodeKind::CdataSection) => {
                let mut node = Node::new(kind);
                node.text = src.text(sid).map(str::to_owned);
                self.push(node)
            },
            NodeKind::ProcessingInstruction => {
                let target = src
                    .element_name(sid)
                    .map(|q| q.local.as_ref().to_string())
                    .unwrap_or_default();
                self.create_processing_instruction(&target, src.text(sid).unwrap_or(""))
            },
            NodeKind::Doctype => match src.doctype_data(sid) {
                Some(d) => self.create_doctype(d.name, d.public_id, d.system_id),
                None => self.create_doctype("html", "", ""),
            },
            other => self.push(Node::new(other)),
        };
        let children: Vec<StaticNodeId> = src.dom_children(sid).collect();
        for child in children {
            let copied = self.copy_fragment_node(src, child);
            self.attach_silent(new, copied);
        }
        // A `<template>`'s contents and an element's shadow root are not among
        // its children, so the child loop above cannot reach them. Without this
        // an `innerHTML` that parsed a template would produce an element whose
        // `content` is empty, and the declarative pass would have nothing to
        // move into the shadow root it then built.
        if let Some(contents) = src.template_contents(sid) {
            let dst_contents = self.ensure_template_contents(new);
            for child in src.dom_children(contents).collect::<Vec<_>>() {
                let copied = self.copy_fragment_node(src, child);
                self.attach_silent(dst_contents, copied);
            }
        }
        if let Some(root) = src.shadow_root(sid) {
            let init = src.shadow_init(sid).unwrap_or_default();
            let dst_root = self.install_shadow_root(new, init, true);
            for child in src.dom_children(root).collect::<Vec<_>>() {
                let copied = self.copy_fragment_node(src, child);
                self.attach_silent(dst_root, copied);
            }
            self.reassign_slots(dst_root);
        }
        new
    }

    /// The `<body>` element of a `parse_document`-parsed fragment, if present.
    fn fragment_body(doc: &StaticDocument) -> Option<StaticNodeId> {
        let html = doc.document_element()?;
        doc.dom_children(html).find(|&c| {
            doc.element_name(c)
                .is_some_and(|q| q.local.as_ref() == "body")
        })
    }
}

/// Reserved `attrs` keys carrying a doctype's external identifiers (see
/// [`ScriptedDom::create_doctype`]). Not attribute names any element can hold:
/// a doctype is not an element and never reaches the cascade.
const DOCTYPE_PUBLIC_ID: &str = "__publicId";
const DOCTYPE_SYSTEM_ID: &str = "__systemId";

/// An attribute's qualified name (`prefix:local`, or just `local`).
pub fn qualified_name(name: &QualName) -> String {
    match name.prefix.as_ref() {
        Some(p) => format!("{}:{}", p.as_ref(), name.local.as_ref()),
        None => name.local.as_ref().to_string(),
    }
}

fn qual_name_bytes(name: &QualName) -> usize {
    name.ns.as_ref().len()
        + name.local.as_ref().len()
        + name
            .prefix
            .as_ref()
            .map_or(0, |prefix| prefix.as_ref().len())
}

impl LayoutDom for ScriptedDom {
    type NodeId = NodeId;

    fn document(&self) -> NodeId {
        self.root
    }

    /// The dangle-contract liveness check (see [`LayoutDom::is_live`]). Live iff
    /// the id belongs to this document and still has an entry in the store
    /// (attached or orphaned-but-kept); a dropped or collected node has no entry.
    /// Never panics, unlike the read accessors.
    fn is_live(&self, id: NodeId) -> bool {
        self.try_index(id)
            .is_some_and(|v| self.nodes.contains_key(&v))
    }

    fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).parent
    }

    fn prev_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.sibling(id, -1)
    }

    fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.sibling(id, 1)
    }

    fn dom_children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.node(id).children.iter().copied()
    }

    fn flat_children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.flat_children_of(id)
    }

    fn has_shadow_trees(&self) -> bool {
        self.has_shadow_roots()
    }

    fn shadow_root(&self, id: NodeId) -> Option<NodeId> {
        self.shadow_root_of(id)
    }

    fn shadow_host(&self, id: NodeId) -> Option<NodeId> {
        self.shadow_host_of(id)
    }

    fn shadow_roots(&self) -> Vec<NodeId> {
        self.shadow_root_ids()
    }

    fn assigned_slot(&self, id: NodeId) -> Option<NodeId> {
        self.assigned_slot_of(id)
    }

    fn assigned_nodes(&self, id: NodeId) -> Vec<NodeId> {
        self.assigned_nodes_of(id)
    }

    fn containing_shadow_root(&self, id: NodeId) -> Option<NodeId> {
        self.containing_shadow_root_of(id)
    }

    fn template_contents(&self, id: NodeId) -> Option<NodeId> {
        self.template_contents_of(id)
    }

    fn shadow_init(&self, id: NodeId) -> Option<ShadowRootInit> {
        self.shadow_root_of(id)
            .and_then(|root| self.shadow_init(root))
    }

    fn kind(&self, id: NodeId) -> NodeKind {
        self.node(id).kind
    }

    fn opaque_id(&self, id: NodeId) -> u64 {
        // Assert ownership (the fence), then return the full packed id through
        // the render-neutral `LayoutDom` identity seam. Off the fence this is
        // just `id.0`, identical to before.
        let _ = self.index(id);
        id.0 as u64
    }

    fn element_name(&self, id: NodeId) -> Option<&QualName> {
        self.node(id).name.as_ref()
    }

    fn attribute(&self, id: NodeId, ns: &Namespace, local: &LocalName) -> Option<&str> {
        self.node(id)
            .attrs
            .iter()
            .find(|(name, _)| &name.ns == ns && &name.local == local)
            .map(|(_, value)| value.as_str())
    }

    fn attributes(&self, id: NodeId) -> impl Iterator<Item = AttributeView<'_>> + '_ {
        self.node(id)
            .attrs
            .iter()
            .map(|(name, value)| AttributeView {
                name,
                value: value.as_str(),
            })
    }

    fn text(&self, id: NodeId) -> Option<&str> {
        self.node(id).text.as_deref()
    }

    fn doctype_data(&self, id: NodeId) -> Option<DoctypeView<'_>> {
        let node = self.node(id);
        if node.kind != NodeKind::Doctype {
            return None;
        }
        let field = |want: &str| {
            node.attrs
                .iter()
                .find(|(name, _)| name.local.as_ref() == want)
                .map_or("", |(_, value)| value.as_str())
        };
        Some(DoctypeView {
            name: node.text.as_deref().unwrap_or(""),
            public_id: field(DOCTYPE_PUBLIC_ID),
            system_id: field(DOCTYPE_SYSTEM_ID),
        })
    }
}

impl LayoutDomMut for ScriptedDom {
    fn create_element(&mut self, name: QualName) -> NodeId {
        let mut node = Node::new(NodeKind::Element);
        node.name = Some(name);
        self.push(node)
    }

    fn create_text(&mut self, data: &str) -> NodeId {
        let mut node = Node::new(NodeKind::Text);
        node.text = Some(data.to_owned());
        self.push(node)
    }

    fn append_child(&mut self, parent: NodeId, child: NodeId) {
        // Spec observability: appending an in-tree node is a remove + insert,
        // and the former parent's consumers must hear the removal. The detach
        // used to be silent here. (moveBefore plan S1.)
        if let Some(former_parent) = self.node(child).parent {
            let previous = self.sibling(child, -1);
            let next = self.sibling(child, 1);
            self.mutations.push(DomMutation::Removed {
                node: child,
                former_parent,
            });
            self.record_implicit_removal(former_parent, child, previous, next);
        }
        self.detach(child);
        self.node_mut(child).parent = Some(parent);
        self.structure_epoch += 1;
        self.node_mut(parent).children.push(child);
        self.mutations.push(DomMutation::Inserted {
            node: child,
            parent,
        });
        let previous = self.sibling(child, -1);
        self.record_child_list(parent, vec![child], Vec::new(), previous, None);
        self.reassign_for_parent(parent, child);
    }

    fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: Option<NodeId>) {
        // As in `append_child`: an in-tree insert is a remove + insert, both
        // observable. State preservation is `move_before`'s contract, not this
        // one's. (moveBefore plan S1.)
        if let Some(former_parent) = self.node(child).parent {
            let previous = self.sibling(child, -1);
            let next = self.sibling(child, 1);
            self.mutations.push(DomMutation::Removed {
                node: child,
                former_parent,
            });
            self.record_implicit_removal(former_parent, child, previous, next);
        }
        self.attach_at(parent, child, reference);
        self.mutations.push(DomMutation::Inserted {
            node: child,
            parent,
        });
        let previous = self.sibling(child, -1);
        let next = self.sibling(child, 1);
        self.record_child_list(parent, vec![child], Vec::new(), previous, next);
        self.reassign_for_parent(parent, child);
    }

    fn move_before(&mut self, parent: NodeId, child: NodeId, reference: Option<NodeId>) {
        let from_parent = self.node(child).parent;
        // Spec pre-move step: a reference of the moving node itself means
        // "before my own next sibling", i.e. stay in place.
        let reference = if reference == Some(child) {
            self.sibling(child, 1)
        } else {
            reference
        };
        // A move resolving to the current position is a no-op: nothing moves,
        // nothing is recorded (and per spec, no state resets either).
        if from_parent == Some(parent) && self.sibling(child, 1) == reference {
            return;
        }
        let was_previous = self.sibling(child, -1);
        let was_next = self.sibling(child, 1);
        self.attach_at(parent, child, reference);
        if let Some(from_parent) = from_parent {
            self.record_child_list(from_parent, Vec::new(), vec![child], was_previous, was_next);
        }
        let previous = self.sibling(child, -1);
        let next = self.sibling(child, 1);
        self.record_child_list(parent, vec![child], Vec::new(), previous, next);
        match from_parent {
            Some(from_parent) => self.mutations.push(DomMutation::Moved {
                node: child,
                from_parent,
                to_parent: parent,
            }),
            // A disconnected node has no subtree state to preserve; record the
            // plain insert this actually is. (The DOM-level `moveBefore` throws
            // there; this layout-side contract stays total.)
            None => self.mutations.push(DomMutation::Inserted {
                node: child,
                parent,
            }),
        }
        if let Some(from_parent) = from_parent {
            self.reassign_for(from_parent);
        }
        self.reassign_for_parent(parent, child);
    }

    fn remove(&mut self, node: NodeId) {
        let former_parent = self.node(node).parent;
        let previous = self.sibling(node, -1);
        let next = self.sibling(node, 1);
        self.detach(node);
        if let Some(former_parent) = former_parent {
            self.mutations.push(DomMutation::Removed {
                node,
                former_parent,
            });
            self.record_child_list(former_parent, Vec::new(), vec![node], previous, next);
            self.reassign_for_parent(former_parent, node);
        }
        self.release_subtree(node);
    }

    fn set_attribute(&mut self, node: NodeId, name: QualName, value: &str) {
        let attrs = &mut self.node_mut(node).attrs;
        // Capture the prior value before overwriting so a retained style owner
        // can classify the mutation after the old value is gone from the live
        // DOM. `None` = newly added.
        let old_value;
        if let Some(existing) = attrs
            .iter_mut()
            .find(|(n, _)| n.ns == name.ns && n.local == name.local)
        {
            old_value = Some(std::mem::replace(&mut existing.1, value.to_owned()));
        } else {
            old_value = None;
            attrs.push((name.clone(), value.to_owned()));
        }
        self.mutations.push(DomMutation::AttributeChanged {
            node,
            name: name.clone(),
            old_value: old_value.clone(),
        });
        self.record_attribute(node, name.clone(), old_value);
        self.reassign_for_attribute(node, &name);
    }

    fn remove_attribute(&mut self, node: NodeId, name: QualName) {
        // Drop the matching attribute and capture its prior value; the
        // borrow ends before we record the mutation. No-op (and no record)
        // when the attribute is absent.
        let removed = {
            let attrs = &mut self.node_mut(node).attrs;
            attrs
                .iter()
                .position(|(n, _)| n.ns == name.ns && n.local == name.local)
                .map(|pos| attrs.remove(pos).1)
        };
        if let Some(old) = removed {
            self.mutations.push(DomMutation::AttributeChanged {
                node,
                name: name.clone(),
                old_value: Some(old.clone()),
            });
            self.record_attribute(node, name.clone(), Some(old));
            self.reassign_for_attribute(node, &name);
        }
    }

    fn set_text(&mut self, node: NodeId, data: &str) {
        let old = self.node(node).text.clone();
        self.node_mut(node).text = Some(data.to_owned());
        self.mutations
            .push(DomMutation::CharacterDataChanged { node });
        self.record_character_data(node, old);
    }

    fn set_inner_html(&mut self, node: NodeId, html: &str) {
        // Orphan the current children; the single SubtreeReplaced covers it.
        let existing = std::mem::take(&mut self.node_mut(node).children);
        let removed = existing.clone();
        for child in existing {
            self.node_mut(child).parent = None;
            self.structure_epoch += 1;
            // Keep retained descendants available until pin-aware collection.
        }
        // Parse via the static parser (a LayoutDom) and copy the explicitly
        // wrapped <body> children in. The wrapper keeps metadata elements such
        // as <style> in the fragment body; parsing a bare string as a complete
        // document would move leading metadata into the synthesized <head>.
        let source = format!("<!doctype html><html><body>{html}</body></html>");
        // Fragment source: `innerHTML` must not realize `<template
        // shadowrootmode>` into a shadow root (only the document parser and
        // `setHTMLUnsafe` do).
        let fragment = StaticDocument::parse_fragment_source(&source);
        let mut added = Vec::new();
        if let Some(body) = Self::fragment_body(&fragment) {
            let body_children: Vec<StaticNodeId> = fragment.dom_children(body).collect();
            for child in body_children {
                let copied = self.copy_fragment_node(&fragment, child);
                self.attach_silent(node, copied);
                added.push(copied);
            }
        }
        self.mutations.push(DomMutation::SubtreeReplaced { node });
        self.record_child_list(node, added, removed, None, None);
        self.reassign_for(node);
    }

    fn drain_mutations(&mut self, out: &mut Vec<DomMutation<NodeId>>) {
        self.mutation_base = self
            .mutation_base
            .saturating_add(self.mutations.len() as u64);
        out.append(&mut self.mutations);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layout_dom_api::CapturedMutation;

    fn qual(local: &str) -> QualName {
        QualName::new(None, Namespace::from(""), LocalName::from(local))
    }

    #[test]
    fn mutate_read_and_record() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();

        let div = dom.create_element(qual("div"));
        dom.append_child(root, div);
        let text = dom.create_text("hello");
        dom.append_child(div, text);
        dom.set_attribute(div, qual("id"), "main");

        // Read surface reflects the mutations.
        assert_eq!(dom.kind(div), NodeKind::Element);
        assert_eq!(dom.element_name(div).unwrap().local, LocalName::from("div"));
        assert_eq!(dom.dom_children(root).collect::<Vec<_>>(), vec![div]);
        assert_eq!(dom.dom_children(div).collect::<Vec<_>>(), vec![text]);
        assert_eq!(dom.parent(text), Some(div));
        assert_eq!(dom.text(text), Some("hello"));
        assert_eq!(
            dom.attribute(div, &Namespace::from(""), &LocalName::from("id")),
            Some("main")
        );

        // Mutation stream: 2 inserts + 1 attribute change, then drained empty.
        let mut muts = Vec::new();
        dom.drain_mutations(&mut muts);
        assert_eq!(muts.len(), 3);
        let mut again = Vec::new();
        dom.drain_mutations(&mut again);
        assert!(again.is_empty());
    }

    #[test]
    fn pending_mutations_can_be_observed_without_stealing_layouts_batch() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let child = dom.create_element(qual("div"));
        dom.append_child(root, child);

        let (base, pending) = dom.pending_mutations();
        assert_eq!(base, 0);
        assert_eq!(pending.len(), 1);

        let mut drained = Vec::new();
        dom.drain_mutations(&mut drained);
        assert_eq!(drained.len(), 1);
        let (next_base, pending) = dom.pending_mutations();
        assert_eq!(next_base, base + 1);
        assert!(pending.is_empty());
    }

    #[test]
    fn observed_record_is_off_until_asked_and_then_spec_shaped() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let host = dom.create_element(qual("div"));
        dom.append_child(root, host);
        // Off by default: the second view records nothing.
        assert!(dom.take_observed().is_empty());

        dom.set_observing(true);
        let a = dom.create_element(qual("a"));
        let b = dom.create_element(qual("b"));
        dom.append_child(host, a);
        dom.append_child(host, b);
        dom.set_attribute(a, qual("id"), "one");
        dom.set_attribute(a, qual("id"), "two");
        dom.remove_child(a);

        let observed = dom.take_observed();
        assert_eq!(observed.len(), 5);
        assert_eq!(
            observed[0],
            ObservedMutation::ChildList {
                target: host,
                added: vec![a],
                removed: vec![],
                previous_sibling: None,
                next_sibling: None,
                ancestors: vec![host, root],
            }
        );
        // The second append sees `a` as its previous sibling.
        match &observed[1] {
            ObservedMutation::ChildList {
                previous_sibling, ..
            } => assert_eq!(*previous_sibling, Some(a)),
            other => panic!("expected childList, got {other:?}"),
        }
        assert_eq!(
            observed[2],
            ObservedMutation::Attributes {
                target: a,
                name: qual("id"),
                old_value: None,
                ancestors: vec![a, host, root],
            }
        );
        match &observed[3] {
            ObservedMutation::Attributes { old_value, .. } => {
                assert_eq!(old_value.as_deref(), Some("one"))
            },
            other => panic!("expected attributes, got {other:?}"),
        }
        assert_eq!(
            observed[4],
            ObservedMutation::ChildList {
                target: host,
                added: vec![],
                removed: vec![a],
                previous_sibling: None,
                next_sibling: Some(b),
                ancestors: vec![host, root],
            }
        );
        // Taking is a drain, and Livery's own stream still carries every fact.
        assert!(dom.take_observed().is_empty());
        let mut layout = Vec::new();
        dom.drain_mutations(&mut layout);
        assert_eq!(layout.len(), 6);
    }

    #[test]
    fn observed_inner_html_and_text_content_carry_both_sides() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let host = dom.create_element(qual("div"));
        dom.append_child(root, host);
        dom.set_observing(true);
        dom.set_inner_html(host, "<p>x</p>");
        let observed = dom.take_observed();
        let ObservedMutation::ChildList { added, removed, .. } = &observed[0] else {
            panic!("expected childList");
        };
        assert_eq!(added.len(), 1);
        assert!(removed.is_empty());
        let paragraph = added[0];

        dom.set_text_content(host, "plain");
        let observed = dom.take_observed();
        let ObservedMutation::ChildList { added, removed, .. } = &observed[0] else {
            panic!("expected childList");
        };
        assert_eq!(removed, &vec![paragraph]);
        assert_eq!(added.len(), 1);
        // While observing, a replaced subtree is orphaned rather than freed, so
        // the record can still hand the removed node to script.
        assert!(dom.is_live(paragraph));

        let text = added[0];
        dom.set_text(text, "next");
        let observed = dom.take_observed();
        assert_eq!(
            observed[0],
            ObservedMutation::CharacterData {
                target: text,
                old_value: Some("plain".to_owned()),
                ancestors: vec![text, host, root],
            }
        );
    }

    #[test]
    fn set_inner_html_builds_subtree() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let div = dom.create_element(qual("div"));
        dom.append_child(root, div);
        let mut drained = Vec::new();
        dom.drain_mutations(&mut drained); // clear the append

        dom.set_inner_html(div, "<p>hi</p><span>x</span>");

        let kids: Vec<_> = dom.dom_children(div).collect();
        assert_eq!(kids.len(), 2);
        assert_eq!(
            dom.element_name(kids[0]).unwrap().local,
            LocalName::from("p")
        );
        assert_eq!(
            dom.element_name(kids[1]).unwrap().local,
            LocalName::from("span")
        );
        // <p>hi</p> — the <p> has a single text child "hi".
        let p_kids: Vec<_> = dom.dom_children(kids[0]).collect();
        assert_eq!(p_kids.len(), 1);
        assert_eq!(dom.text(p_kids[0]), Some("hi"));

        let mut muts = Vec::new();
        dom.drain_mutations(&mut muts);
        assert!(matches!(
            muts.as_slice(),
            [DomMutation::SubtreeReplaced { .. }]
        ));
    }

    #[test]
    fn siblings_and_remove() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let a = dom.create_element(qual("a"));
        let b = dom.create_element(qual("b"));
        dom.append_child(root, a);
        dom.append_child(root, b);

        assert_eq!(dom.next_sibling(a), Some(b));
        assert_eq!(dom.prev_sibling(b), Some(a));
        assert_eq!(dom.prev_sibling(a), None);

        let mut drained = Vec::new();
        dom.drain_mutations(&mut drained); // clear the two inserts

        dom.remove(a);
        assert_eq!(dom.dom_children(root).collect::<Vec<_>>(), vec![b]);
        assert_eq!(dom.next_sibling(b), None);

        let mut muts = Vec::new();
        dom.drain_mutations(&mut muts);
        assert!(matches!(muts.as_slice(), [DomMutation::Removed { .. }]));
    }

    #[test]
    fn insert_before_orders_and_appends() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let a = dom.create_element(qual("a"));
        let c = dom.create_element(qual("c"));
        dom.append_child(root, a);
        dom.append_child(root, c);
        let mut drained = Vec::new();
        dom.drain_mutations(&mut drained); // clear the two appends

        // Insert b before c → [a, b, c].
        let b = dom.create_element(qual("b"));
        dom.insert_before(root, b, Some(c));
        assert_eq!(dom.dom_children(root).collect::<Vec<_>>(), vec![a, b, c]);
        assert_eq!(dom.parent(b), Some(root));

        // reference = None appends → [a, b, c, d].
        let d = dom.create_element(qual("d"));
        dom.insert_before(root, d, None);
        assert_eq!(dom.dom_children(root).collect::<Vec<_>>(), vec![a, b, c, d]);

        // A reference that isn't a child of root falls back to append → [a, b, c, d, e].
        let orphan = dom.create_element(qual("orphan"));
        let e = dom.create_element(qual("e"));
        dom.insert_before(root, e, Some(orphan));
        assert_eq!(
            dom.dom_children(root).collect::<Vec<_>>(),
            vec![a, b, c, d, e]
        );

        // Each insert recorded exactly one Inserted under root (all three
        // children were detached fresh nodes, so no Removed accompanies them).
        let mut muts = Vec::new();
        dom.drain_mutations(&mut muts);
        assert_eq!(muts.len(), 3);
        assert!(
            muts.iter()
                .all(|m| matches!(m, DomMutation::Inserted { parent, .. } if *parent == root))
        );
    }

    #[test]
    fn in_tree_insert_reports_the_removal_too() {
        // Re-inserting an in-tree node is a remove + insert, both observable —
        // the former parent's consumers must hear the child left. (moveBefore
        // plan S1: this detach used to be silent.)
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let a = dom.create_element(qual("a"));
        let b = dom.create_element(qual("b"));
        let child = dom.create_element(qual("child"));
        dom.append_child(root, a);
        dom.append_child(root, b);
        dom.append_child(a, child);
        let mut drained = Vec::new();
        dom.drain_mutations(&mut drained);

        dom.insert_before(b, child, None);
        let mut muts = Vec::new();
        dom.drain_mutations(&mut muts);
        assert_eq!(
            muts,
            vec![
                DomMutation::Removed {
                    node: child,
                    former_parent: a,
                },
                DomMutation::Inserted {
                    node: child,
                    parent: b,
                },
            ]
        );
    }

    #[test]
    fn move_before_moves_across_parents_with_one_moved_record() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let a = dom.create_element(qual("a"));
        let b = dom.create_element(qual("b"));
        let child = dom.create_element(qual("child"));
        let b_kid = dom.create_element(qual("bkid"));
        dom.append_child(root, a);
        dom.append_child(root, b);
        dom.append_child(a, child);
        dom.append_child(b, b_kid);
        let mut drained = Vec::new();
        dom.drain_mutations(&mut drained);

        dom.move_before(b, child, Some(b_kid));
        assert_eq!(dom.parent(child), Some(b));
        assert_eq!(dom.dom_children(b).collect::<Vec<_>>(), vec![child, b_kid]);
        assert!(dom.dom_children(a).next().is_none());

        // One Moved record: not a Removed + Inserted pair, so consumers may
        // keep the subtree's retained state.
        let mut muts = Vec::new();
        dom.drain_mutations(&mut muts);
        assert_eq!(
            muts,
            vec![DomMutation::Moved {
                node: child,
                from_parent: a,
                to_parent: b,
            }]
        );
    }

    #[test]
    fn move_before_same_parent_reorders_and_noops_in_place() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let a = dom.create_element(qual("a"));
        let b = dom.create_element(qual("b"));
        let c = dom.create_element(qual("c"));
        dom.append_child(root, a);
        dom.append_child(root, b);
        dom.append_child(root, c);
        let mut drained = Vec::new();
        dom.drain_mutations(&mut drained);

        // Reorder: move c before a → [c, a, b], one Moved with equal parents.
        dom.move_before(root, c, Some(a));
        assert_eq!(dom.dom_children(root).collect::<Vec<_>>(), vec![c, a, b]);
        let mut muts = Vec::new();
        dom.drain_mutations(&mut muts);
        assert_eq!(
            muts,
            vec![DomMutation::Moved {
                node: c,
                from_parent: root,
                to_parent: root,
            }]
        );

        // A move to the current position records nothing: before its own next
        // sibling, before itself (the spec pre-move step), and a last child
        // "moved" to append.
        dom.move_before(root, c, Some(a));
        dom.move_before(root, c, Some(c));
        dom.move_before(root, b, None);
        let mut noop = Vec::new();
        dom.drain_mutations(&mut noop);
        assert_eq!(noop, Vec::new());
        assert_eq!(dom.dom_children(root).collect::<Vec<_>>(), vec![c, a, b]);
    }

    #[test]
    fn remove_attribute_records_and_noops() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let div = dom.create_element(qual("div"));
        dom.append_child(root, div);
        dom.set_attribute(div, qual("id"), "main");
        let mut drained = Vec::new();
        dom.drain_mutations(&mut drained); // clear the append + set

        // Removing a present attribute drops it and records the old value.
        dom.remove_attribute(div, qual("id"));
        assert_eq!(
            dom.attribute(div, &Namespace::from(""), &LocalName::from("id")),
            None
        );
        let mut muts = Vec::new();
        dom.drain_mutations(&mut muts);
        assert!(matches!(
            muts.as_slice(),
            [DomMutation::AttributeChanged { old_value: Some(v), .. }] if v.as_str() == "main"
        ));

        // Removing an absent attribute is a no-op and records nothing.
        dom.remove_attribute(div, qual("id"));
        let mut again = Vec::new();
        dom.drain_mutations(&mut again);
        assert!(again.is_empty());
    }

    #[test]
    fn drained_mutations_round_trip_through_captured_postcard() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let div = dom.create_element(qual("div"));
        dom.append_child(root, div);
        let mut drained = Vec::new();
        dom.drain_mutations(&mut drained); // clear the insert

        dom.set_attribute(div, qual("id"), "main");
        dom.remove_attribute(div, qual("id"));

        let mut muts = Vec::new();
        dom.drain_mutations(&mut muts);
        let captured: Vec<_> = muts
            .iter()
            .map(|m| CapturedMutation::capture(m, |id| dom.capture_node_id(*id)))
            .collect();
        let bytes = postcard::to_stdvec(&captured).expect("serialize captured mutations");
        let decoded: Vec<CapturedMutation> =
            postcard::from_bytes(&bytes).expect("decode captured mutations");
        let replayed: Vec<_> = decoded
            .into_iter()
            .map(|m| m.replay(|raw| dom.remint_node_id(raw)))
            .collect();

        assert_eq!(replayed, muts);
    }

    #[test]
    fn stats_count_node_kinds_attributes_and_bytes() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let html = dom.create_element(qual("html"));
        let body = dom.create_element(qual("body"));
        let text = dom.create_text("hello");
        let comment = dom.create_comment("note");
        let frag = dom.create_fragment();
        dom.append_child(root, html);
        dom.append_child(html, body);
        dom.append_child(body, text);
        dom.append_child(body, comment);
        dom.append_child(body, frag);
        dom.set_attribute(body, qual("class"), "main");
        dom.set_attribute(body, qual("data-x"), "1");

        let stats = dom.stats();
        assert_eq!(stats.live_nodes, 6);
        assert_eq!(stats.node_kinds.documents, 1);
        assert_eq!(stats.node_kinds.elements, 2);
        assert_eq!(stats.node_kinds.text, 1);
        assert_eq!(stats.node_kinds.comments, 1);
        assert_eq!(stats.node_kinds.document_fragments, 1);
        assert_eq!(stats.attribute_count, 2);
        assert!(stats.estimated_bytes >= std::mem::size_of::<ScriptedDom>());
    }

    // --- G0: the document fence ---------------------------------------------

    #[test]
    fn secondary_root_is_same_document() {
        // `create_document` mints a second root in the *same* arena (same tag),
        // so cross-using ids between the two roots must not trip the fence.
        let mut dom = ScriptedDom::new();
        let primary = dom.document();
        let secondary = dom.create_document();
        let div = dom.create_element(qual("div"));
        dom.append_child(secondary, div);
        assert_eq!(dom.parent(div), Some(secondary));
        assert_ne!(primary, secondary);
        // Both roots resolve without panicking.
        assert_eq!(dom.kind(primary), NodeKind::Document);
        assert_eq!(dom.kind(secondary), NodeKind::Document);
    }

    /// On a fenced build (64-bit debug) a `NodeId` minted by one document used
    /// against another panics. On release/wasm the fence compiles out, so this
    /// test only exists where the assert is live.
    #[cfg(all(debug_assertions, target_pointer_width = "64"))]
    #[test]
    #[should_panic(expected = "different document")]
    fn cross_document_node_id_panics() {
        let mut a = ScriptedDom::new();
        let b = ScriptedDom::new();
        let id_in_a = a.create_element(qual("div"));
        // `id_in_a` carries a's tag; resolving it against b trips the fence.
        let _ = b.kind(id_in_a);
    }

    #[cfg(all(debug_assertions, target_pointer_width = "64"))]
    #[test]
    fn distinct_documents_get_distinct_tags() {
        let a = ScriptedDom::new();
        let b = ScriptedDom::new();
        // Roots share the arena index (0) but differ in the tagged high bits.
        assert_ne!(a.document().raw(), b.document().raw());
    }

    #[test]
    fn captured_node_id_remints_for_another_document() {
        let a = ScriptedDom::new();
        let b = ScriptedDom::new();
        let captured = a.capture_node_id(a.document());
        let reminted = b.remint_node_id(captured);
        assert!(b.is_live(reminted));
        #[cfg(all(debug_assertions, target_pointer_width = "64"))]
        {
            assert!(!b.is_live(a.document()));
            assert_ne!(a.document(), reminted);
        }
    }

    // --- G2: the dangle contract (is_live) ----------------------------------

    /// The contract under create/remove/re-query across frames (the slab
    /// implementation G3 must preserve, allocator aside). Attached → live;
    /// dropped → dead; orphaned-but-kept → still live; cross-document → not live.
    #[test]
    fn dangle_contract_churn_across_frames() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();

        // Frame 1: build a small tree, drain the mutations (the frame boundary).
        let a = dom.create_element(qual("a"));
        let b = dom.create_element(qual("b"));
        dom.append_child(root, a);
        dom.append_child(root, b);
        let mut drained = Vec::new();
        dom.drain_mutations(&mut drained);

        // Attached ids are live.
        assert!(dom.is_live(root));
        assert!(dom.is_live(a));
        assert!(dom.is_live(b));

        // Frame 2: `remove` drops `a`'s subtree. Its id is now dead, and a
        // re-query of the tree no longer contains it.
        dom.remove(a);
        dom.drain_mutations(&mut drained);
        assert!(!dom.is_live(a));
        assert!(dom.is_live(b));
        assert_eq!(dom.dom_children(root).collect::<Vec<_>>(), vec![b]);

        // Frame 3: `remove_child` orphans `b` but keeps it — still live and
        // re-insertable (the orphan semantics the gc-arena refit must honor).
        dom.remove_child(b);
        dom.drain_mutations(&mut drained);
        assert!(dom.is_live(b), "an orphaned node stays live until dropped");
        assert!(dom.dom_children(root).collect::<Vec<_>>().is_empty());
        dom.append_child(root, b); // re-insert the orphan
        assert_eq!(dom.dom_children(root).collect::<Vec<_>>(), vec![b]);
        assert!(dom.is_live(b));
    }

    #[test]
    fn is_live_is_false_for_dropped_and_foreign_ids() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let n = dom.create_element(qual("n"));
        dom.append_child(root, n);
        assert!(dom.is_live(n));
        dom.remove(n);
        assert!(!dom.is_live(n));

        // An id from another document is not live here (no panic — `is_live` is
        // the non-asserting check, unlike the read accessors).
        let other = ScriptedDom::new();
        let foreign = {
            let mut o = other;
            o.create_element(qual("x"))
        };
        assert!(!dom.is_live(foreign));
    }

    // --- G3: mark-sweep collection ------------------------------------------

    const NO_PINS: [NodeId; 0] = [];

    /// `collect` keeps the attached tree and reaps an unpinned orphan, but spares
    /// a pinned orphan *and its whole connected component* (undirected mark).
    #[test]
    fn collect_reaps_unpinned_orphans_keeps_pinned_components() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let attached = dom.create_element(qual("attached"));
        dom.append_child(root, attached);

        // An orphan subtree: parent -> mid -> leaf, all detached from the document.
        let parent = dom.create_element(qual("p"));
        let mid = dom.create_element(qual("mid"));
        let leaf = dom.create_element(qual("leaf"));
        dom.append_child(parent, mid);
        dom.append_child(mid, leaf);

        // With no pins, the orphan subtree is unreachable → collected; the
        // attached tree survives.
        let live_before = dom.live_node_count();
        let pruned = dom.collect(NO_PINS);
        assert_eq!(pruned, 3, "the 3-node orphan subtree is reaped");
        assert_eq!(dom.live_node_count(), live_before - 3);
        assert!(dom.is_live(root) && dom.is_live(attached));
        assert!(!dom.is_live(parent) && !dom.is_live(mid) && !dom.is_live(leaf));

        // Rebuild the orphan and pin only the *deep* leaf: the undirected mark
        // must keep the leaf's ancestors too (JS can walk parentNode up).
        let parent = dom.create_element(qual("p"));
        let mid = dom.create_element(qual("mid"));
        let leaf = dom.create_element(qual("leaf"));
        dom.append_child(parent, mid);
        dom.append_child(mid, leaf);
        let pruned = dom.collect([leaf]); // pin the leaf
        assert_eq!(pruned, 0, "a pin on the leaf spares the whole component");
        assert!(dom.is_live(parent) && dom.is_live(mid) && dom.is_live(leaf));

        // Drop the pin → the component is now collectable.
        assert_eq!(dom.collect(NO_PINS), 3);
        assert!(!dom.is_live(parent) && !dom.is_live(mid) && !dom.is_live(leaf));
    }

    /// The bounded-memory property: sustained create/remove-child/collect cycles
    /// plateau the store, where the never-reuse slab would grow without bound.
    #[test]
    fn collect_bounds_memory_under_churn() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let baseline = dom.live_node_count();

        let mut peak = baseline;
        for _ in 0..500 {
            // Attach a small subtree, then orphan it and collect (no pins) — the
            // SPA churn shape: create nodes, detach them, JS drops the reflectors.
            let host = dom.create_element(qual("host"));
            dom.append_child(root, host);
            for _ in 0..8 {
                let kid = dom.create_element(qual("kid"));
                dom.append_child(host, kid);
            }
            peak = peak.max(dom.live_node_count());
            dom.remove_child(host); // orphan the whole subtree
            dom.collect(NO_PINS); // reap it
        }

        // Monotonic ids kept climbing, but the store is back to baseline — bounded.
        assert!(dom.next_id > 4000, "ids are monotonic (no reuse)");
        assert_eq!(dom.live_node_count(), baseline, "store plateaus, not grows");
        assert!(
            peak < baseline + 32,
            "peak stays tiny — only one subtree at a time"
        );
    }

    /// A quiet document (no further mutations) still reaps orphans on an idle
    /// `collect` — the backgrounded-SPA case.
    #[test]
    fn idle_collect_reaps_orphans_without_mutations() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let o = dom.create_element(qual("o"));
        dom.append_child(root, o);
        dom.remove_child(o); // orphan; no pin
        let mut drained = Vec::new();
        dom.drain_mutations(&mut drained);

        // No mutations happen now; an idle collect still reaps the orphan.
        assert!(dom.is_live(o));
        assert_eq!(dom.collect(NO_PINS), 1);
        assert!(!dom.is_live(o));
    }

    /// A `create_document` secondary is pin-kept (G3 carve-out #3): while pinned
    /// its whole subtree survives `collect`; once the pin drops, it collects like
    /// any other orphan (a dropped `createHTMLDocument` no longer leaks).
    #[test]
    fn secondary_document_is_pin_kept() {
        let mut dom = ScriptedDom::new();
        let secondary = dom.create_document();
        let body = dom.create_element(qual("body"));
        dom.append_child(secondary, body);

        // Pinned (script holds it) → the whole secondary component survives.
        assert_eq!(dom.collect([secondary]), 0);
        assert!(dom.is_live(secondary) && dom.is_live(body));

        // Dropped (no pin) → it collects.
        assert_eq!(dom.collect(NO_PINS), 2);
        assert!(!dom.is_live(secondary) && !dom.is_live(body));
    }

    #[test]
    fn pins_pin_unpin_and_retire() {
        let id = NodeId::from_raw;
        let mut pins = Pins::new();
        assert!(pins.is_empty());

        pins.pin(id(0x10));
        pins.pin(id(0x20));
        pins.pin(id(0x10)); // idempotent
        assert_eq!(pins.len(), 2);
        assert!(pins.is_pinned(id(0x10)));

        assert!(pins.unpin(id(0x20)));
        assert!(!pins.unpin(id(0xAA))); // never pinned → no-op

        pins.pin(id(0x30));
        assert_eq!(pins.retire_dead([id(0x10), id(0x30), id(0xBB)]), 2);
        assert!(pins.is_empty());
    }

    #[test]
    fn pins_clear() {
        let mut pins = Pins::new();
        pins.pin(NodeId::from_raw(1));
        pins.pin(NodeId::from_raw(2));
        assert_eq!(pins.len(), 2);
        pins.clear();
        assert!(pins.is_empty());
    }
}
