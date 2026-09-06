// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! One-to-many CSS layout fragments and their tree relationships.

use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    ops::Deref,
};

use crate::{
    BoxId, BreakToken, ContainingBlock, CssBoxTree, FlowAxes, Fragmentainer, FragmentainerId,
    FragmentainerKind, FragmentationContext, FragmentationContextId, LogicalRect, PhysicalOffset,
    PhysicalRect, PhysicalSize,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FragmentId(u32);

impl FragmentId {
    /// Opaque allocation number. Fragment identifiers are retained across a
    /// K5g relayout and do not name a dense storage position.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// The formatting-coordinate source that produced an out-of-flow box's
/// static-position rectangle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StaticPositionSource {
    /// The box was generated at the initial formatting root.
    InitialContainingBlock,
    /// One emitted fragment supplied the local formatting coordinates.
    Fragment(FragmentId),
}

/// A static-position rectangle captured while its source formatting context
/// emits geometry.
///
/// This is deliberately separate from a final fragment: an absolute or fixed
/// box can use a containing block other than the formatting context that
/// supplied its static position. A grid source can additionally supply the
/// selected grid area that replaces the ordinary padding rectangle only when
/// that grid is the K5a-selected containing block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticPosition {
    pub box_id: BoxId,
    pub source: StaticPositionSource,
    pub containing_block: ContainingBlock,
    pub logical_rect: LogicalRect,
    /// An optional grid-area replacement for the selected containing block,
    /// expressed in the source fragment's logical coordinates.
    pub containing_block_area: Option<LogicalRect>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Baselines {
    /// Distance from this fragment's logical block-start edge to its first
    /// baseline.
    pub first: Option<f32>,
    /// Distance from this fragment's logical block-start edge to its last
    /// baseline.
    pub last: Option<f32>,
}

impl Baselines {
    /// Construct finite baseline offsets in the fragment's own logical flow.
    ///
    /// A baseline offset may be negative: a negative margin or offset can
    /// place a child's baseline above this fragment's block-start edge
    /// (WPT `css/CSS2/css21-errata/s-11-1-1b-005.html` does exactly this with
    /// a `margin-top: -15px` table cell). Only a non-finite offset is
    /// invalid.
    pub fn new(first: Option<f32>, last: Option<f32>) -> Option<Self> {
        [first, last]
            .into_iter()
            .flatten()
            .all(f32::is_finite)
            .then_some(Self { first, last })
    }

    /// The synthesized baseline for a formatting context with no line-box
    /// baseline. CSS uses the block-end edge for that fallback in this
    /// unfragmented lane.
    pub fn synthesized_from_block_end(block_size: f32) -> Self {
        Self::new(Some(block_size.max(0.0)), Some(block_size.max(0.0)))
            .expect("a finite non-negative block size has a valid synthesized baseline")
    }
}

/// One fragment produced by one CSS box.
#[derive(Clone, Debug, PartialEq)]
pub struct Fragment {
    id: FragmentId,
    box_id: BoxId,
    parent: Option<FragmentId>,
    containing_fragment: Option<FragmentId>,
    fragmentation_context: FragmentationContextId,
    fragmentainer: Option<FragmentainerId>,
    pub logical_rect: LogicalRect,
    pub continuation: Option<BreakToken>,
    pub baselines: Baselines,
    /// The fragment's own logical ink/scrollable extent before structural
    /// descendants contribute theirs. K5h needs this source value to shrink
    /// an ancestor's aggregate overflow after it replaces one subtree.
    own_overflow: LogicalRect,
    pub overflow: LogicalRect,
    flow: FlowAxes,
    physical_rect: PhysicalRect,
}

impl Fragment {
    /// Construct a K0 fragment from the lane's current horizontal geometry.
    pub fn from_horizontal_physical(box_id: BoxId, rect: PhysicalRect) -> Self {
        let logical_rect = LogicalRect::from_horizontal_physical(rect);
        Self {
            id: FragmentId(u32::MAX),
            box_id,
            parent: None,
            containing_fragment: None,
            fragmentation_context: FragmentationContextId::INITIAL,
            fragmentainer: None,
            logical_rect,
            continuation: None,
            baselines: Baselines::default(),
            own_overflow: logical_rect,
            overflow: logical_rect,
            flow: FlowAxes::HORIZONTAL_LTR,
            physical_rect: rect,
        }
    }

    /// Construct a fragment from standards-owned logical geometry.
    pub fn from_logical(
        box_id: BoxId,
        logical_rect: LogicalRect,
        containing_block: PhysicalSize,
        flow: FlowAxes,
    ) -> Self {
        Self {
            id: FragmentId(u32::MAX),
            box_id,
            parent: None,
            containing_fragment: None,
            fragmentation_context: FragmentationContextId::INITIAL,
            fragmentainer: None,
            logical_rect,
            continuation: None,
            baselines: Baselines::default(),
            own_overflow: logical_rect,
            overflow: logical_rect,
            flow,
            physical_rect: flow.physical_rect(logical_rect, containing_block),
        }
    }

    /// Preserve a physical consumer rectangle while attaching the logical
    /// geometry that produced it. This is used when a host has already
    /// accumulated ancestor origins at its physical fragment edge.
    pub fn from_physical_with_logical(
        box_id: BoxId,
        physical_rect: PhysicalRect,
        logical_rect: LogicalRect,
        flow: FlowAxes,
    ) -> Self {
        Self {
            id: FragmentId(u32::MAX),
            box_id,
            parent: None,
            containing_fragment: None,
            fragmentation_context: FragmentationContextId::INITIAL,
            fragmentainer: None,
            logical_rect,
            continuation: None,
            baselines: Baselines::default(),
            own_overflow: logical_rect,
            overflow: logical_rect,
            flow,
            physical_rect,
        }
    }

    pub fn id(&self) -> FragmentId {
        self.id
    }

    pub fn box_id(&self) -> BoxId {
        self.box_id
    }

    pub fn parent(&self) -> Option<FragmentId> {
        self.parent
    }

    pub fn containing_fragment(&self) -> Option<FragmentId> {
        self.containing_fragment
    }

    pub fn fragmentation_context(&self) -> FragmentationContextId {
        self.fragmentation_context
    }

    pub fn fragmentainer(&self) -> Option<FragmentainerId> {
        self.fragmentainer
    }

    pub fn flow(&self) -> FlowAxes {
        self.flow
    }

    pub fn physical_rect(&self) -> PhysicalRect {
        self.physical_rect
    }

    /// Attach formatting-context baseline outputs before the fragment enters
    /// the tree. The values remain logical offsets and are not derived from
    /// the physical rectangle.
    pub fn with_baselines(mut self, baselines: Baselines) -> Self {
        debug_assert!(Baselines::new(baselines.first, baselines.last).is_some());
        self.baselines = baselines;
        self
    }
}

/// K0 compatibility for physical consumers. The fragment tree still owns the
/// fragment and its logical geometry.
impl Deref for Fragment {
    type Target = PhysicalRect;

    fn deref(&self) -> &Self::Target {
        &self.physical_rect
    }
}

/// Fragments in tree order, indexed independently by box identity.
#[derive(Clone, Debug)]
pub struct FragmentTree {
    roots: Vec<FragmentId>,
    fragments: Vec<Fragment>,
    ids: Vec<FragmentId>,
    slots: HashMap<FragmentId, usize>,
    next_id: u32,
    by_box: HashMap<BoxId, Vec<FragmentId>>,
    static_positions: HashMap<BoxId, StaticPosition>,
    fragmentation_contexts: HashMap<FragmentationContextId, FragmentationContext>,
    fragmentainers: HashMap<FragmentainerId, Fragmentainer>,
    next_context_id: u32,
    next_fragmentainer_id: u32,
}

impl Default for FragmentTree {
    fn default() -> Self {
        let mut fragmentation_contexts = HashMap::new();
        fragmentation_contexts.insert(
            FragmentationContextId::INITIAL,
            FragmentationContext {
                id: FragmentationContextId::INITIAL,
                parent: None,
                flow: FlowAxes::HORIZONTAL_LTR,
                fragmentainers: Vec::new(),
            },
        );
        Self {
            roots: Vec::new(),
            fragments: Vec::new(),
            ids: Vec::new(),
            slots: HashMap::new(),
            next_id: 0,
            by_box: HashMap::new(),
            static_positions: HashMap::new(),
            fragmentation_contexts,
            fragmentainers: HashMap::new(),
            next_context_id: 1,
            next_fragmentainer_id: 0,
        }
    }
}

impl FragmentTree {
    /// Register a formatting context while retaining its parent context and
    /// fragmentainer order in the same layout result.
    pub fn create_fragmentation_context(
        &mut self,
        parent: Option<FragmentationContextId>,
        flow: FlowAxes,
    ) -> FragmentationContextId {
        if let Some(parent) = parent {
            assert!(
                self.fragmentation_contexts.contains_key(&parent),
                "a fragmentation context parent must be registered"
            );
        }
        let id = FragmentationContextId::from_index(self.next_context_id.max(1));
        self.next_context_id = (id.index() as u32)
            .checked_add(1)
            .expect("a fragment tree exceeded u32::MAX fragmentation contexts");
        let previous = self.fragmentation_contexts.insert(
            id,
            FragmentationContext {
                id,
                parent,
                flow,
                fragmentainers: Vec::new(),
            },
        );
        assert!(
            previous.is_none(),
            "a fragmentation context id cannot repeat"
        );
        id
    }

    pub fn fragmentation_context(
        &self,
        id: FragmentationContextId,
    ) -> Option<&FragmentationContext> {
        self.fragmentation_contexts.get(&id)
    }

    /// Register one ordered fragmentainer in an existing context.
    pub fn create_fragmentainer(
        &mut self,
        context: FragmentationContextId,
        logical_rect: LogicalRect,
        kind: FragmentainerKind,
    ) -> FragmentainerId {
        let (flow, parent_context, sequence) = {
            let context_record = self
                .fragmentation_contexts
                .get(&context)
                .expect("a fragmentainer belongs to a live context");
            (
                context_record.flow(),
                context_record.parent(),
                context_record.fragmentainers().len(),
            )
        };
        assert!(
            logical_rect.inline_start.is_finite()
                && logical_rect.block_start.is_finite()
                && logical_rect.inline_size.is_finite()
                && logical_rect.inline_size >= 0.0
                && logical_rect.block_size.is_finite()
                && logical_rect.block_size >= 0.0,
            "a fragmentainer must have a finite non-negative logical size"
        );
        let id = FragmentainerId::from_index(self.next_fragmentainer_id);
        self.next_fragmentainer_id = self
            .next_fragmentainer_id
            .checked_add(1)
            .expect("a fragment tree exceeded u32::MAX fragmentainers");
        let record = Fragmentainer {
            id,
            context,
            parent_context,
            sequence,
            kind,
            flow,
            logical_rect,
        };
        self.fragmentainers.insert(id, record);
        self.fragmentation_contexts
            .get_mut(&context)
            .expect("the context remains live while adding a fragmentainer")
            .fragmentainers
            .push(id);
        id
    }

    pub fn fragmentainer(&self, id: FragmentainerId) -> Option<&Fragmentainer> {
        self.fragmentainers.get(&id)
    }

    pub fn roots(&self) -> &[FragmentId] {
        &self.roots
    }

    pub fn get(&self, id: FragmentId) -> Option<&Fragment> {
        self.slots
            .get(&id)
            .and_then(|slot| self.fragments.get(*slot))
    }

    pub fn fragments_for_box(&self, box_id: BoxId) -> impl Iterator<Item = &Fragment> {
        self.by_box
            .get(&box_id)
            .into_iter()
            .flatten()
            .filter_map(|id| self.get(*id))
    }

    pub fn fragment_ids_for_box(&self, box_id: BoxId) -> &[FragmentId] {
        self.by_box.get(&box_id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The static position captured for an absolute or fixed box.
    pub fn static_position_for_box(&self, box_id: BoxId) -> Option<&StaticPosition> {
        self.static_positions.get(&box_id)
    }

    /// Attach the unique unfragmented static-position record for one box.
    ///
    /// K6 will generalize this to a one-to-many fragmentainer index. Until
    /// then, conflicting duplicate records indicate a formatting integration
    /// error rather than silently choosing one backend result.
    pub fn record_static_position(&mut self, position: StaticPosition) {
        match self.static_positions.entry(position.box_id) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(position);
            },
            std::collections::hash_map::Entry::Occupied(entry) => {
                assert_eq!(
                    entry.get(),
                    &position,
                    "an unfragmented box produced two static-position records"
                );
            },
        }
    }

    /// Replace an earlier provisional record when a retained formatting
    /// handoff publishes the authoritative source rectangle for the same box.
    /// The caller must have already emitted both the box and its first record;
    /// a missing entry is a broken handoff rather than an insertion route.
    pub fn reconcile_static_position(&mut self, position: StaticPosition) {
        let entry = self
            .static_positions
            .get_mut(&position.box_id)
            .expect("static-position reconciliation requires an existing record");
        *entry = position;
    }

    pub fn len(&self) -> usize {
        self.fragments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }

    pub fn push(
        &mut self,
        mut fragment: Fragment,
        parent: Option<FragmentId>,
        containing_fragment: Option<FragmentId>,
    ) -> FragmentId {
        let id = self.allocate_fragment_id();
        fragment.id = id;
        fragment.parent = parent;
        fragment.containing_fragment = containing_fragment;
        let box_id = fragment.box_id;
        self.fragments.push(fragment);
        self.ids.push(id);
        let previous = self.slots.insert(id, self.fragments.len() - 1);
        assert!(previous.is_none(), "a fragment id cannot occupy two slots");
        self.by_box.entry(box_id).or_default().push(id);
        if parent.is_none() {
            self.roots.push(id);
        }
        id
    }

    /// Push a fragment into a registered fragmentainer. This is the only
    /// insertion route that can attach a non-initial fragmentation context.
    pub fn push_in_fragmentainer(
        &mut self,
        mut fragment: Fragment,
        parent: Option<FragmentId>,
        containing_fragment: Option<FragmentId>,
        fragmentainer: FragmentainerId,
    ) -> FragmentId {
        let record = self
            .fragmentainers
            .get(&fragmentainer)
            .expect("a fragment must name a live fragmentainer")
            .clone();
        assert_eq!(
            fragment.flow(),
            record.flow(),
            "a fragment's flow must match its fragmentainer"
        );
        fragment.fragmentation_context = record.context();
        fragment.fragmentainer = Some(fragmentainer);
        self.push(fragment, parent, containing_fragment)
    }

    fn allocate_fragment_id(&mut self) -> FragmentId {
        let id = FragmentId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("a fragment tree exceeded u32::MAX fragments");
        id
    }

    /// Rebuild aggregate overflow from each fragment's own extent and its
    /// current structural children. This is intentionally tree-owned rather
    /// than a paint-side union so a later K5h subtree replacement can shrink
    /// as well as extend an ancestor's scrollable overflow.
    pub(crate) fn recompute_overflow(&mut self) {
        for fragment in &mut self.fragments {
            fragment.overflow = fragment.own_overflow;
        }
        for child in self.ids.clone().into_iter().rev() {
            let Some(parent) = self.get(child).and_then(Fragment::parent) else {
                continue;
            };
            let child_overflow = self
                .get(child)
                .expect("a live fragment has overflow")
                .overflow;
            let parent_fragment = &mut self.fragments[self.slots[&parent]];
            parent_fragment.overflow =
                union_logical_rects(parent_fragment.overflow, child_overflow);
        }
    }

    /// Attach a positioned fragment to the fragment selected by the K5a
    /// containing-block graph. A `None` value names the initial containing
    /// block, which has no ordinary generated fragment.
    pub fn set_containing_fragment(
        &mut self,
        id: FragmentId,
        containing_fragment: Option<FragmentId>,
    ) {
        if let Some(slot) = self.slots.get(&id).copied()
            && let Some(fragment) = self.fragments.get_mut(slot)
        {
            fragment.containing_fragment = containing_fragment;
        }
    }

    /// Reattach a provisional fragment after its retained formatting context
    /// has materialized the structural parent. This preserves the fragment's
    /// identity while restoring tree order for paint and overflow propagation.
    pub fn reconcile_parent(&mut self, id: FragmentId, parent: Option<FragmentId>) {
        if let Some(parent) = parent {
            assert!(
                self.get(parent).is_some(),
                "a reconciled parent must be live"
            );
        }
        let Some(slot) = self.slots.get(&id).copied() else {
            return;
        };
        self.fragments[slot].parent = parent;
        self.recompute_overflow();
    }

    /// Replace one fragment's overflow and union it into every structural
    /// ancestor. Layout phases that add a real out-of-border-box extent use
    /// this after their fragment exists; the fragment tree, not a paint
    /// consumer, remains the owner of the propagated geometry.
    pub fn set_overflow(&mut self, id: FragmentId, overflow: LogicalRect) {
        let Some(slot) = self.slots.get(&id).copied() else {
            return;
        };
        let Some(fragment) = self.fragments.get_mut(slot) else {
            return;
        };
        fragment.own_overflow = overflow;
        self.recompute_overflow();
    }

    /// Translate one emitted fragment and every structural descendant.
    ///
    /// Relative positioning runs after normal-flow geometry exists. The
    /// fragment tree therefore owns the translation: descendants, baselines,
    /// paint, hit testing, and containing-fragment lookup continue to name
    /// the same fragment identities while their physical and logical geometry
    /// move together.
    pub fn translate_subtree(&mut self, root: FragmentId, offset: PhysicalOffset) {
        if self.get(root).is_none() || (offset.x == 0.0 && offset.y == 0.0) {
            return;
        }

        let descendants = self
            .ids
            .iter()
            .copied()
            .filter(|id| {
                let mut cursor = Some(*id);
                while let Some(candidate) = cursor {
                    if candidate == root {
                        return true;
                    }
                    cursor = self.get(candidate).and_then(Fragment::parent);
                }
                false
            })
            .collect::<Vec<_>>();

        for id in descendants {
            let fragment = &mut self.fragments[self.slots[&id]];
            let logical = fragment.flow.logical_offset(offset);
            fragment.logical_rect.inline_start += logical.inline;
            fragment.logical_rect.block_start += logical.block;
            fragment.own_overflow.inline_start += logical.inline;
            fragment.own_overflow.block_start += logical.block;
            fragment.overflow.inline_start += logical.inline;
            fragment.overflow.block_start += logical.block;
            fragment.physical_rect.x += offset.x;
            fragment.physical_rect.y += offset.y;
        }

        self.recompute_overflow();
    }

    /// Replace a leaf fragment's border-box size while preserving its retained
    /// identity and origin. Positioned leaves use this after their
    /// standards-owned used-size calculation; a non-leaf must be reformatted
    /// instead so descendants can receive the new containing size.
    pub fn resize_leaf(&mut self, id: FragmentId, size: PhysicalSize) -> bool {
        let Some(slot) = self.slots.get(&id).copied() else {
            return false;
        };
        if self
            .ids
            .iter()
            .any(|candidate| self.get(*candidate).and_then(Fragment::parent) == Some(id))
        {
            return false;
        }
        let fragment = &mut self.fragments[slot];
        let logical_size = fragment.flow.logical_size(size);
        fragment.logical_rect.inline_size = logical_size.inline;
        fragment.logical_rect.block_size = logical_size.block;
        fragment.own_overflow.inline_size = logical_size.inline;
        fragment.own_overflow.block_size = logical_size.block;
        fragment.overflow.inline_size = logical_size.inline;
        fragment.overflow.block_size = logical_size.block;
        fragment.physical_rect.width = size.width;
        fragment.physical_rect.height = size.height;
        self.recompute_overflow();
        true
    }

    /// Replace one unfragmented structural subtree with fragments produced by
    /// a newly formatted compatible root. The caller reconciles box identity
    /// before this boundary; this operation preserves every fragment outside
    /// `root`, assigns fresh identities to replacement descendants, repairs
    /// the fragment indices, and replaces static-position records atomically.
    ///
    /// An incoming descendant may not refer to a source or containing
    /// fragment outside its replacement subtree. Such a cross-root dependency
    /// needs a wider dirty root, so this bounded K5h primitive declines it.
    pub fn replace_subtree(
        &mut self,
        root: FragmentId,
        replacement: &Self,
        replacement_root: FragmentId,
    ) -> Option<FragmentId> {
        let previous_root = self.get(root)?.clone();
        let replacement_root_fragment = replacement.get(replacement_root)?;
        if replacement_root_fragment.box_id != previous_root.box_id {
            return None;
        }
        let retired = self.subtree_ids(root);
        let retired_set = retired.iter().copied().collect::<HashSet<_>>();
        let retired_boxes = retired
            .iter()
            .map(|id| {
                self.get(*id)
                    .expect("a live retired fragment has a box")
                    .box_id
            })
            .collect::<HashSet<_>>();
        let incoming = replacement.subtree_ids(replacement_root);
        let incoming_set = incoming.iter().copied().collect::<HashSet<_>>();
        let replacement_boxes = incoming
            .iter()
            .map(|id| {
                replacement
                    .get(*id)
                    .expect("a live replacement fragment has a box")
                    .box_id
            })
            .collect::<HashSet<_>>();

        // Fragmentation can give one box several structural fragments. A
        // bounded replacement cannot discard only some of those fragments:
        // their shared box and static-position records would become
        // ambiguous. K6 may widen this to a fragmentation-aware splice.
        if retired_boxes.iter().any(|box_id| {
            self.by_box
                .get(box_id)
                .is_some_and(|ids| ids.iter().any(|id| !retired_set.contains(id)))
        }) || replacement_boxes.iter().any(|box_id| {
            replacement
                .by_box
                .get(box_id)
                .is_some_and(|ids| ids.iter().any(|id| !incoming_set.contains(id)))
        }) {
            return None;
        }

        for source in &incoming {
            let fragment = replacement.get(*source)?;
            if *source != replacement_root
                && (fragment.parent.is_none()
                    || fragment
                        .parent
                        .is_some_and(|parent| !incoming_set.contains(&parent))
                    || fragment
                        .containing_fragment
                        .is_some_and(|containing| !incoming_set.contains(&containing)))
            {
                return None;
            }
        }
        if self.static_positions.values().any(|position| {
            !retired_boxes.contains(&position.box_id)
                && matches!(position.source, StaticPositionSource::Fragment(source) if retired_set.contains(&source))
        }) {
            return None;
        }
        if replacement.static_positions.values().any(|position| {
            replacement_boxes.contains(&position.box_id)
                && matches!(position.source, StaticPositionSource::Fragment(source) if !incoming_set.contains(&source))
        }) {
            return None;
        }
        if replacement.static_positions.values().any(|position| {
            !replacement_boxes.contains(&position.box_id)
                && matches!(position.source, StaticPositionSource::Fragment(source) if incoming_set.contains(&source))
        }) {
            return None;
        }

        let mut identifiers = HashMap::new();
        identifiers.insert(replacement_root, root);
        for source in incoming
            .iter()
            .copied()
            .filter(|id| *id != replacement_root)
        {
            identifiers.insert(source, self.allocate_fragment_id());
        }

        let mut imported = HashMap::new();
        for source in &incoming {
            let mut fragment = replacement
                .get(*source)
                .expect("a checked replacement source stays live")
                .clone();
            fragment.id = identifiers[source];
            if *source == replacement_root {
                fragment.parent = previous_root.parent;
                fragment.containing_fragment = previous_root.containing_fragment;
            } else {
                fragment.parent = fragment.parent.map(|parent| identifiers[&parent]);
                fragment.containing_fragment = fragment
                    .containing_fragment
                    .map(|containing| identifiers[&containing]);
            }
            imported.insert(fragment.id, fragment);
        }

        let mut fragments =
            Vec::with_capacity(self.fragments.len() - retired.len() + incoming.len());
        let mut ids = Vec::with_capacity(fragments.capacity());
        for id in self.ids.clone() {
            if id == root {
                fragments.push(
                    imported
                        .remove(&root)
                        .expect("the replacement root has one imported fragment"),
                );
                ids.push(root);
                for source in incoming
                    .iter()
                    .copied()
                    .filter(|id| *id != replacement_root)
                {
                    let id = identifiers[&source];
                    fragments.push(
                        imported
                            .remove(&id)
                            .expect("each replacement descendant imports once"),
                    );
                    ids.push(id);
                }
            } else if !retired_set.contains(&id) {
                fragments.push(
                    self.get(id)
                        .expect("a retained fragment stays live")
                        .clone(),
                );
                ids.push(id);
            }
        }
        debug_assert!(imported.is_empty());
        self.fragments = fragments;
        self.ids = ids;

        self.static_positions.retain(|box_id, position| {
            !replacement_boxes.contains(box_id)
                && !matches!(position.source, StaticPositionSource::Fragment(source) if retired_set.contains(&source))
        });
        for position in replacement.static_positions.values() {
            if !replacement_boxes.contains(&position.box_id) {
                continue;
            }
            let mut position = *position;
            if let StaticPositionSource::Fragment(source) = position.source {
                position.source = StaticPositionSource::Fragment(identifiers[&source]);
            }
            self.static_positions.insert(position.box_id, position);
        }
        self.rebuild_indices();
        self.recompute_overflow();
        #[cfg(any(debug_assertions, test))]
        self.assert_invariants();
        Some(root)
    }

    /// Rekey dense construction identifiers against retained fragments after
    /// the owning box tree has already reconciled its own identities.
    pub fn reconcile_identifiers(&mut self, previous: &Self, box_ids: &HashMap<BoxId, BoxId>) {
        self.remap_box_identifiers(box_ids);

        let mut mapping = HashMap::new();
        let mut consumed = HashSet::new();
        for current in self.roots.clone() {
            let candidate = previous.roots.iter().copied().find(|candidate| {
                !consumed.contains(candidate)
                    && same_fragment_context(
                        self.get(current).expect("a root fragment is live"),
                        previous
                            .get(*candidate)
                            .expect("a retained root fragment is live"),
                    )
            });
            if let Some(candidate) = candidate {
                self.match_retained_subtree(
                    previous,
                    current,
                    candidate,
                    &mut mapping,
                    &mut consumed,
                );
            }
        }

        let mut next = previous.ids.iter().map(|id| id.0).max().map_or(0, |id| {
            id.checked_add(1)
                .expect("a fragment tree exceeded u32::MAX fragments")
        });
        for current in self.ids.clone() {
            mapping.entry(current).or_insert_with(|| {
                let allocated = FragmentId(next);
                next = next
                    .checked_add(1)
                    .expect("a fragment tree exceeded u32::MAX fragments");
                allocated
            });
        }
        self.remap_fragment_identifiers(&mapping);
        #[cfg(any(debug_assertions, test))]
        self.assert_invariants();
    }

    fn match_retained_subtree(
        &self,
        previous: &Self,
        current: FragmentId,
        prior: FragmentId,
        mapping: &mut HashMap<FragmentId, FragmentId>,
        consumed: &mut HashSet<FragmentId>,
    ) {
        let current_fragment = self.get(current).expect("a retained candidate is live");
        let previous_fragment = previous.get(prior).expect("a retained source is live");
        if !same_fragment_context(current_fragment, previous_fragment) {
            return;
        }
        mapping.insert(current, prior);
        consumed.insert(prior);

        let current_children = self.structural_children(current);
        let previous_children = previous.structural_children(prior);
        for current_child in current_children {
            let candidate = previous_children.iter().copied().find(|candidate| {
                !consumed.contains(candidate)
                    && same_fragment_context(
                        self.get(current_child).expect("a child fragment is live"),
                        previous
                            .get(*candidate)
                            .expect("a retained child fragment is live"),
                    )
            });
            if let Some(candidate) = candidate {
                self.match_retained_subtree(previous, current_child, candidate, mapping, consumed);
            }
        }
    }

    fn structural_children(&self, parent: FragmentId) -> Vec<FragmentId> {
        self.ids
            .iter()
            .copied()
            .filter(|id| self.get(*id).and_then(Fragment::parent) == Some(parent))
            .collect()
    }

    fn subtree_ids(&self, root: FragmentId) -> Vec<FragmentId> {
        self.ids
            .iter()
            .copied()
            .filter(|id| {
                let mut cursor = Some(*id);
                while let Some(candidate) = cursor {
                    if candidate == root {
                        return true;
                    }
                    cursor = self.get(candidate).and_then(Fragment::parent);
                }
                false
            })
            .collect()
    }

    fn rebuild_indices(&mut self) {
        self.slots.clear();
        self.by_box.clear();
        self.roots.clear();
        for (slot, fragment) in self.fragments.iter().enumerate() {
            let id = fragment.id;
            assert_eq!(self.ids[slot], id);
            assert!(
                self.slots.insert(id, slot).is_none(),
                "a fragment id cannot occupy two slots"
            );
            self.by_box.entry(fragment.box_id).or_default().push(id);
            if fragment.parent.is_none() {
                self.roots.push(id);
            }
        }
    }

    fn remap_box_identifiers(&mut self, box_ids: &HashMap<BoxId, BoxId>) {
        for fragment in &mut self.fragments {
            fragment.box_id = box_ids[&fragment.box_id];
        }
        self.by_box.clear();
        for (slot, id) in self.ids.iter().copied().enumerate() {
            self.by_box
                .entry(self.fragments[slot].box_id)
                .or_default()
                .push(id);
        }
        let positions = std::mem::take(&mut self.static_positions);
        self.static_positions = positions
            .into_values()
            .map(|mut position| {
                position.box_id = box_ids[&position.box_id];
                (position.box_id, position)
            })
            .collect();
    }

    fn remap_fragment_identifiers(&mut self, mapping: &HashMap<FragmentId, FragmentId>) {
        for fragment in &mut self.fragments {
            fragment.id = mapping[&fragment.id];
            fragment.parent = fragment.parent.map(|id| mapping[&id]);
            fragment.containing_fragment = fragment.containing_fragment.map(|id| mapping[&id]);
        }
        self.ids = self.ids.iter().map(|id| mapping[id]).collect();
        self.slots = self
            .ids
            .iter()
            .copied()
            .enumerate()
            .map(|(slot, id)| (id, slot))
            .collect();
        self.roots = self.roots.iter().map(|id| mapping[id]).collect();
        for ids in self.by_box.values_mut() {
            for id in ids {
                *id = mapping[id];
            }
        }
        for position in self.static_positions.values_mut() {
            if let StaticPositionSource::Fragment(source) = position.source {
                position.source = StaticPositionSource::Fragment(mapping[&source]);
            }
        }
        self.next_id = self.ids.iter().map(|id| id.0).max().map_or(0, |id| {
            id.checked_add(1)
                .expect("a fragment tree exceeded u32::MAX fragments")
        });
    }

    #[cfg(any(debug_assertions, test))]
    fn assert_invariants(&self) {
        assert_eq!(self.fragments.len(), self.ids.len());
        assert_eq!(self.fragments.len(), self.slots.len());
        for (slot, id) in self.ids.iter().copied().enumerate() {
            assert_eq!(self.slots.get(&id), Some(&slot));
            assert_eq!(self.fragments[slot].id(), id);
        }
        for root in &self.roots {
            assert!(self.slots.contains_key(root));
            assert_eq!(self.get(*root).and_then(Fragment::parent), None);
        }
        for id in self.ids.iter().copied() {
            let fragment = self.get(id).expect("a live fragment has storage");
            if let Some(parent) = fragment.parent() {
                assert!(self.slots.contains_key(&parent));
            }
            if let Some(containing) = fragment.containing_fragment() {
                assert!(self.slots.contains_key(&containing));
            }
            assert!(
                self.by_box
                    .get(&fragment.box_id())
                    .is_some_and(|ids| ids.contains(&id))
            );
            if let Some(fragmentainer) = fragment.fragmentainer() {
                let record = self
                    .fragmentainers
                    .get(&fragmentainer)
                    .expect("a fragmentainer link must name a live record");
                assert_eq!(fragment.fragmentation_context(), record.context());
                assert_eq!(fragment.flow(), record.flow());
                assert!(
                    self.fragmentation_contexts
                        .get(&record.context())
                        .is_some_and(|context| context.fragmentainers.contains(&fragmentainer))
                );
            }
        }
        for (id, context) in &self.fragmentation_contexts {
            assert_eq!(*id, context.id());
            if let Some(parent) = context.parent() {
                assert!(self.fragmentation_contexts.contains_key(&parent));
            }
            for (sequence, fragmentainer) in context.fragmentainers().iter().copied().enumerate() {
                let record = self
                    .fragmentainers
                    .get(&fragmentainer)
                    .expect("a context must own live fragmentainers");
                assert_eq!(record.context(), *id);
                assert_eq!(record.sequence(), sequence);
            }
        }
        for (id, fragmentainer) in &self.fragmentainers {
            assert_eq!(*id, fragmentainer.id());
            assert!(
                self.fragmentation_contexts
                    .contains_key(&fragmentainer.context())
            );
            assert_eq!(
                self.fragmentation_contexts[&fragmentainer.context()].flow(),
                fragmentainer.flow()
            );
        }
        for (box_id, ids) in &self.by_box {
            let mut seen = HashSet::new();
            for id in ids {
                assert!(seen.insert(*id));
                assert_eq!(self.get(*id).map(Fragment::box_id), Some(*box_id));
            }
        }
        for position in self.static_positions.values() {
            assert!(self.by_box.contains_key(&position.box_id));
            if let StaticPositionSource::Fragment(source) = position.source {
                assert!(self.slots.contains_key(&source));
            }
        }
    }
}

fn same_fragment_context(current: &Fragment, previous: &Fragment) -> bool {
    current.box_id == previous.box_id
        && current.flow == previous.flow
        && current.fragmentation_context == previous.fragmentation_context
        && current.fragmentainer == previous.fragmentainer
}

fn union_logical_rects(one: LogicalRect, other: LogicalRect) -> LogicalRect {
    let inline_start = one.inline_start.min(other.inline_start);
    let block_start = one.block_start.min(other.block_start);
    let inline_end =
        (one.inline_start + one.inline_size).max(other.inline_start + other.inline_size);
    let block_end = (one.block_start + one.block_size).max(other.block_start + other.block_size);
    LogicalRect {
        inline_start,
        block_start,
        inline_size: inline_end - inline_start,
        block_size: block_end - block_start,
    }
}

/// The standards-owned result of one layout pass.
#[derive(Clone, Debug)]
pub struct LayoutResult<Id> {
    boxes: CssBoxTree<Id>,
    fragments: FragmentTree,
}

/// The identifier translation produced while reconciling one newly computed
/// layout against its retained predecessor. Consumers that retain side data
/// keyed by generated boxes use this to repair those keys before publication.
#[derive(Clone, Debug)]
pub struct LayoutIdentityMap {
    box_ids: HashMap<BoxId, BoxId>,
}

impl LayoutIdentityMap {
    pub fn box_id(&self, id: BoxId) -> BoxId {
        self.box_ids[&id]
    }
}

impl<Id> LayoutResult<Id>
where
    Id: Copy + Eq + Hash,
{
    pub fn new(boxes: CssBoxTree<Id>, fragments: FragmentTree) -> Self {
        Self { boxes, fragments }
    }

    pub fn boxes(&self) -> &CssBoxTree<Id> {
        &self.boxes
    }

    pub fn fragments(&self) -> &FragmentTree {
        &self.fragments
    }

    pub fn fragments_mut(&mut self) -> &mut FragmentTree {
        &mut self.fragments
    }

    /// Replace generated-box ownership after a retained fragment operation
    /// has admitted the compatible root. Callers use this when a selected
    /// subtree gained or retired boxes: the fragment tree already contains
    /// the reconciled identities, while node-to-box lookup must come from the
    /// newly generated tree.
    pub fn replace_box_tree(&mut self, boxes: CssBoxTree<Id>) {
        self.boxes = boxes;
    }

    /// Reconcile this freshly constructed layout against the previous
    /// continuous-media generation. The geometry is new; only identities with
    /// unchanged generated-box and fragment context are retained.
    pub fn reconcile_identifiers(&mut self, previous: &Self) -> LayoutIdentityMap {
        let box_ids = self.boxes.reconcile_identifiers(&previous.boxes);
        self.fragments
            .reconcile_identifiers(&previous.fragments, &box_ids);
        LayoutIdentityMap { box_ids }
    }

    pub fn fragment_ids_for_node(&self, node: Id) -> Vec<FragmentId> {
        self.boxes
            .boxes_for_node(node)
            .iter()
            .flat_map(|box_id| self.fragments.fragment_ids_for_box(*box_id))
            .copied()
            .collect()
    }

    pub fn fragments_for_node(&self, node: Id) -> impl Iterator<Item = &Fragment> {
        self.boxes
            .boxes_for_node(node)
            .iter()
            .flat_map(|box_id| self.fragments.fragments_for_box(*box_id))
    }

    /// Compatibility lookup for current single-rectangle consumers.
    ///
    /// New fragment-aware consumers use [`Self::fragments_for_node`].
    ///
    /// K4e4 makes the choice this makes explicit: boxes are registered in
    /// materialization order, outermost first, so a node that generates an
    /// anonymous box around its principal box answers with the outer one. For
    /// a table element that is the table wrapper box, which is the box that
    /// participates in flow, carries `transform` and `opacity` under CSS
    /// Tables 3 section 3.6.1, and contains the captions - the right box for
    /// rectangle queries, hit targets, and paint-effect anchors.
    pub fn get(&self, node: Id) -> Option<&Fragment> {
        self.fragments_for_node(node).next()
    }

    /// The fragment of the node's principal box: the element's own box rather
    /// than an anonymous box generated around it.
    ///
    /// A table element's principal box is the table grid box, which owns its
    /// background, borders, and the `width` and `height` properties under
    /// CSS 2.1 section 17.4. Everything else answers with [`Self::get`].
    pub fn principal_fragment(&self, node: Id) -> Option<&Fragment> {
        self.boxes
            .principal_box(node)
            .and_then(|principal| self.fragments.fragments_for_box(principal).next())
            .or_else(|| self.get(node))
    }

    pub fn len(&self) -> usize {
        self.fragments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BoxOrigin, ContainingBlock, CssBox, DisplayRole, PositioningScheme};

    fn push_block_box(
        boxes: &mut CssBoxTree<u8>,
        node: u8,
        parent: Option<BoxId>,
        containing_block: ContainingBlock,
    ) -> BoxId {
        boxes.push(
            CssBox::new(
                BoxOrigin::Element(node),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Static,
                false,
                None,
                containing_block,
            ),
            parent,
            true,
        )
    }

    #[test]
    fn one_box_owns_many_tree_fragments() {
        let mut boxes = CssBoxTree::default();
        let box_id = boxes.push(
            CssBox::new(
                BoxOrigin::Element(1u8),
                DisplayRole::INLINE_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Static,
                false,
                None,
                ContainingBlock::Initial,
            ),
            None,
            true,
        );
        let mut fragments = FragmentTree::default();
        let first = fragments.push(
            Fragment::from_horizontal_physical(
                box_id,
                PhysicalRect {
                    width: 20.0,
                    height: 10.0,
                    ..PhysicalRect::default()
                },
            ),
            None,
            None,
        );
        let second = fragments.push(
            Fragment::from_horizontal_physical(
                box_id,
                PhysicalRect {
                    x: 20.0,
                    width: 30.0,
                    height: 10.0,
                    ..PhysicalRect::default()
                },
            ),
            None,
            None,
        );
        let layout = LayoutResult::new(boxes, fragments);

        assert_eq!(layout.fragment_ids_for_node(1), vec![first, second]);
        assert_eq!(layout.fragments_for_node(1).count(), 2);
        assert_eq!(layout.get(1).map(Fragment::id), Some(first));
    }

    #[test]
    fn fragment_tree_records_parent_and_containing_fragment() {
        let mut boxes = CssBoxTree::default();
        let parent_box = boxes.push(
            CssBox::new(
                BoxOrigin::Element(1u8),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Static,
                false,
                None,
                ContainingBlock::Initial,
            ),
            None,
            true,
        );
        let child_box = boxes.push(
            CssBox::new(
                BoxOrigin::Element(2u8),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Static,
                false,
                None,
                ContainingBlock::Box(parent_box),
            ),
            Some(parent_box),
            true,
        );
        let mut fragments = FragmentTree::default();
        let parent = fragments.push(
            Fragment::from_horizontal_physical(parent_box, PhysicalRect::default()),
            None,
            None,
        );
        let child = fragments.push(
            Fragment::from_horizontal_physical(child_box, PhysicalRect::default()),
            Some(parent),
            Some(parent),
        );

        assert_eq!(
            fragments.get(child).and_then(Fragment::parent),
            Some(parent)
        );
        assert_eq!(
            fragments.get(child).and_then(Fragment::containing_fragment),
            Some(parent)
        );
        assert_eq!(fragments.roots(), &[parent]);
    }

    #[test]
    fn retained_relayout_keeps_fragment_ids_after_an_inserted_sibling() {
        let mut previous_boxes = CssBoxTree::default();
        let previous_root = previous_boxes.push(
            CssBox::new(
                BoxOrigin::Element(1u8),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Static,
                false,
                None,
                ContainingBlock::Initial,
            ),
            None,
            true,
        );
        let previous_child = previous_boxes.push(
            CssBox::new(
                BoxOrigin::Element(2u8),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Static,
                false,
                None,
                ContainingBlock::Initial,
            ),
            Some(previous_root),
            true,
        );
        let mut previous = FragmentTree::default();
        let previous_root_fragment = previous.push(
            Fragment::from_horizontal_physical(previous_root, PhysicalRect::default()),
            None,
            None,
        );
        let previous_child_fragment = previous.push(
            Fragment::from_horizontal_physical(previous_child, PhysicalRect::default()),
            Some(previous_root_fragment),
            Some(previous_root_fragment),
        );

        let mut next_boxes = CssBoxTree::default();
        let inserted_box = next_boxes.push(
            CssBox::new(
                BoxOrigin::Element(3u8),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Static,
                false,
                None,
                ContainingBlock::Initial,
            ),
            None,
            true,
        );
        let next_root = next_boxes.push(
            CssBox::new(
                BoxOrigin::Element(1u8),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Static,
                false,
                None,
                ContainingBlock::Initial,
            ),
            None,
            true,
        );
        let next_child = next_boxes.push(
            CssBox::new(
                BoxOrigin::Element(2u8),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Static,
                false,
                None,
                ContainingBlock::Initial,
            ),
            Some(next_root),
            true,
        );
        let box_ids = next_boxes.reconcile_identifiers(&previous_boxes);

        let mut next = FragmentTree::default();
        let inserted_fragment = next.push(
            Fragment::from_horizontal_physical(inserted_box, PhysicalRect::default()),
            None,
            None,
        );
        let next_root_fragment = next.push(
            Fragment::from_horizontal_physical(next_root, PhysicalRect::default()),
            None,
            None,
        );
        let next_child_fragment = next.push(
            Fragment::from_horizontal_physical(next_child, PhysicalRect::default()),
            Some(next_root_fragment),
            Some(next_root_fragment),
        );

        next.reconcile_identifiers(&previous, &box_ids);

        let inserted = next
            .fragment_ids_for_box(box_ids[&inserted_box])
            .first()
            .copied()
            .expect("inserted fragment");
        assert_ne!(inserted, previous_root_fragment);
        assert_ne!(inserted, previous_child_fragment);
        assert_eq!(
            next.fragment_ids_for_box(previous_root),
            &[previous_root_fragment],
        );
        assert_eq!(
            next.fragment_ids_for_box(previous_child),
            &[previous_child_fragment],
        );
        assert_eq!(
            next.get(previous_child_fragment).and_then(Fragment::parent),
            Some(previous_root_fragment),
        );
        assert_eq!(next.roots(), &[inserted, previous_root_fragment]);
        assert_eq!(
            inserted_fragment.index(),
            0,
            "the test starts dense before reconciliation"
        );
        assert_eq!(
            next_child_fragment.index(),
            2,
            "the test starts dense before reconciliation"
        );
    }

    #[test]
    fn replacing_a_subtree_preserves_outside_identity_and_rebuilds_indices() {
        let mut boxes = CssBoxTree::default();
        let outer_box = push_block_box(&mut boxes, 1, None, ContainingBlock::Initial);
        let root_box = push_block_box(
            &mut boxes,
            2,
            Some(outer_box),
            ContainingBlock::Box(outer_box),
        );
        let child_box = push_block_box(
            &mut boxes,
            3,
            Some(root_box),
            ContainingBlock::Box(root_box),
        );
        let sibling_box = push_block_box(
            &mut boxes,
            4,
            Some(outer_box),
            ContainingBlock::Box(outer_box),
        );

        let mut tree = FragmentTree::default();
        let outer = tree.push(
            Fragment::from_horizontal_physical(
                outer_box,
                PhysicalRect {
                    width: 100.0,
                    height: 20.0,
                    ..PhysicalRect::default()
                },
            ),
            None,
            None,
        );
        let root = tree.push(
            Fragment::from_horizontal_physical(
                root_box,
                PhysicalRect {
                    width: 100.0,
                    height: 30.0,
                    ..PhysicalRect::default()
                },
            ),
            Some(outer),
            Some(outer),
        );
        let old_child = tree.push(
            Fragment::from_horizontal_physical(
                child_box,
                PhysicalRect {
                    width: 100.0,
                    height: 40.0,
                    ..PhysicalRect::default()
                },
            ),
            Some(root),
            Some(root),
        );
        let sibling = tree.push(
            Fragment::from_horizontal_physical(
                sibling_box,
                PhysicalRect {
                    y: 30.0,
                    width: 100.0,
                    height: 10.0,
                    ..PhysicalRect::default()
                },
            ),
            Some(outer),
            Some(outer),
        );
        tree.set_overflow(
            old_child,
            LogicalRect {
                inline_start: 0.0,
                block_start: 0.0,
                inline_size: 100.0,
                block_size: 200.0,
            },
        );
        tree.record_static_position(StaticPosition {
            box_id: child_box,
            source: StaticPositionSource::Fragment(old_child),
            containing_block: ContainingBlock::Box(root_box),
            logical_rect: LogicalRect::default(),
            containing_block_area: None,
        });
        assert_eq!(
            tree.get(outer).map(|fragment| fragment.overflow.block_size),
            Some(200.0)
        );

        let mut replacement = FragmentTree::default();
        let replacement_root = replacement.push(
            Fragment::from_horizontal_physical(
                root_box,
                PhysicalRect {
                    x: 5.0,
                    y: 7.0,
                    width: 90.0,
                    height: 30.0,
                },
            ),
            None,
            None,
        );
        let replacement_child = replacement.push(
            Fragment::from_horizontal_physical(
                child_box,
                PhysicalRect {
                    x: 5.0,
                    y: 7.0,
                    width: 90.0,
                    height: 40.0,
                },
            ),
            Some(replacement_root),
            Some(replacement_root),
        );
        replacement.record_static_position(StaticPosition {
            box_id: child_box,
            source: StaticPositionSource::Fragment(replacement_child),
            containing_block: ContainingBlock::Box(root_box),
            logical_rect: LogicalRect {
                inline_start: 4.0,
                block_start: 6.0,
                inline_size: 0.0,
                block_size: 0.0,
            },
            containing_block_area: None,
        });

        assert_eq!(
            tree.replace_subtree(root, &replacement, replacement_root),
            Some(root)
        );
        let new_child = tree.fragment_ids_for_box(child_box)[0];

        assert_ne!(new_child, old_child);
        assert!(tree.get(old_child).is_none());
        assert_eq!(tree.len(), 4);
        assert_eq!(tree.roots(), &[outer]);
        assert_eq!(tree.ids, vec![outer, root, new_child, sibling]);
        assert_eq!(tree.fragment_ids_for_box(root_box), &[root]);
        assert_eq!(tree.fragment_ids_for_box(child_box), &[new_child]);
        assert_eq!(tree.fragment_ids_for_box(sibling_box), &[sibling]);
        assert_eq!(tree.get(root).and_then(Fragment::parent), Some(outer));
        assert_eq!(
            tree.get(root).and_then(Fragment::containing_fragment),
            Some(outer)
        );
        assert_eq!(tree.get(new_child).and_then(Fragment::parent), Some(root));
        assert_eq!(
            tree.get(root).map(Fragment::physical_rect),
            Some(PhysicalRect {
                x: 5.0,
                y: 7.0,
                width: 90.0,
                height: 30.0,
            })
        );
        assert_eq!(
            tree.get(outer).map(|fragment| fragment.overflow.block_size),
            Some(47.0)
        );
        assert_eq!(
            tree.static_position_for_box(child_box),
            Some(&StaticPosition {
                box_id: child_box,
                source: StaticPositionSource::Fragment(new_child),
                containing_block: ContainingBlock::Box(root_box),
                logical_rect: LogicalRect {
                    inline_start: 4.0,
                    block_start: 6.0,
                    inline_size: 0.0,
                    block_size: 0.0,
                },
                containing_block_area: None,
            })
        );
    }

    #[test]
    fn replacing_a_subtree_rejects_a_cross_root_static_position_source() {
        let mut boxes = CssBoxTree::default();
        let root_box = push_block_box(&mut boxes, 1, None, ContainingBlock::Initial);
        let child_box = push_block_box(
            &mut boxes,
            2,
            Some(root_box),
            ContainingBlock::Box(root_box),
        );
        let external_box = push_block_box(&mut boxes, 3, None, ContainingBlock::Initial);

        let mut tree = FragmentTree::default();
        let root = tree.push(
            Fragment::from_horizontal_physical(
                root_box,
                PhysicalRect {
                    width: 20.0,
                    height: 10.0,
                    ..PhysicalRect::default()
                },
            ),
            None,
            None,
        );

        let mut replacement = FragmentTree::default();
        let replacement_root = replacement.push(
            Fragment::from_horizontal_physical(root_box, PhysicalRect::default()),
            None,
            None,
        );
        replacement.push(
            Fragment::from_horizontal_physical(child_box, PhysicalRect::default()),
            Some(replacement_root),
            Some(replacement_root),
        );
        let external = replacement.push(
            Fragment::from_horizontal_physical(external_box, PhysicalRect::default()),
            None,
            None,
        );
        replacement.record_static_position(StaticPosition {
            box_id: child_box,
            source: StaticPositionSource::Fragment(external),
            containing_block: ContainingBlock::Box(root_box),
            logical_rect: LogicalRect::default(),
            containing_block_area: None,
        });

        assert_eq!(
            tree.replace_subtree(root, &replacement, replacement_root),
            None
        );
        assert_eq!(tree.len(), 1);
        assert_eq!(
            tree.get(root).map(Fragment::physical_rect),
            Some(PhysicalRect {
                width: 20.0,
                height: 10.0,
                ..PhysicalRect::default()
            })
        );
    }

    #[test]
    fn replacing_a_subtree_rejects_an_outgoing_static_position_dependency() {
        let mut boxes = CssBoxTree::default();
        let root_box = push_block_box(&mut boxes, 1, None, ContainingBlock::Initial);
        let external_box = push_block_box(&mut boxes, 2, None, ContainingBlock::Initial);

        let mut tree = FragmentTree::default();
        let root = tree.push(
            Fragment::from_horizontal_physical(root_box, PhysicalRect::default()),
            None,
            None,
        );
        let external = tree.push(
            Fragment::from_horizontal_physical(external_box, PhysicalRect::default()),
            None,
            None,
        );

        let mut replacement = FragmentTree::default();
        let replacement_root = replacement.push(
            Fragment::from_horizontal_physical(root_box, PhysicalRect::default()),
            None,
            None,
        );
        replacement.record_static_position(StaticPosition {
            box_id: external_box,
            source: StaticPositionSource::Fragment(replacement_root),
            containing_block: ContainingBlock::Initial,
            logical_rect: LogicalRect::default(),
            containing_block_area: None,
        });

        assert_eq!(
            tree.replace_subtree(root, &replacement, replacement_root),
            None
        );
        assert_eq!(tree.roots(), &[root, external]);
        assert_eq!(tree.len(), 2);
    }

    #[test]
    fn translating_a_subtree_keeps_identities_and_moves_descendants() {
        let mut boxes = CssBoxTree::default();
        let parent_box = boxes.push(
            CssBox::new(
                BoxOrigin::Element(1u8),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Relative,
                false,
                None,
                ContainingBlock::Initial,
            ),
            None,
            true,
        );
        let child_box = boxes.push(
            CssBox::new(
                BoxOrigin::Element(2u8),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Static,
                false,
                None,
                ContainingBlock::Box(parent_box),
            ),
            Some(parent_box),
            true,
        );
        let mut fragments = FragmentTree::default();
        let parent = fragments.push(
            Fragment::from_horizontal_physical(
                parent_box,
                PhysicalRect {
                    x: 10.0,
                    y: 20.0,
                    width: 30.0,
                    height: 40.0,
                },
            ),
            None,
            None,
        );
        let child = fragments.push(
            Fragment::from_horizontal_physical(
                child_box,
                PhysicalRect {
                    x: 15.0,
                    y: 25.0,
                    width: 10.0,
                    height: 12.0,
                },
            ),
            Some(parent),
            Some(parent),
        );

        fragments.translate_subtree(parent, PhysicalOffset { x: 7.0, y: -4.0 });

        assert_eq!(fragments.fragment_ids_for_box(parent_box), &[parent]);
        assert_eq!(fragments.fragment_ids_for_box(child_box), &[child]);
        assert_eq!(
            fragments.get(parent).map(Fragment::physical_rect),
            Some(PhysicalRect {
                x: 17.0,
                y: 16.0,
                width: 30.0,
                height: 40.0,
            })
        );
        assert_eq!(
            fragments.get(child).map(Fragment::physical_rect),
            Some(PhysicalRect {
                x: 22.0,
                y: 21.0,
                width: 10.0,
                height: 12.0,
            })
        );
    }

    #[test]
    fn static_position_keeps_its_source_separate_from_its_containing_block() {
        let mut boxes = CssBoxTree::default();
        let source_box = boxes.push(
            CssBox::new(
                BoxOrigin::Element(1u8),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Static,
                false,
                None,
                ContainingBlock::Initial,
            ),
            None,
            true,
        );
        let containing_box = boxes.push(
            CssBox::new(
                BoxOrigin::Element(2u8),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Relative,
                false,
                None,
                ContainingBlock::Box(source_box),
            ),
            Some(source_box),
            true,
        );
        let positioned_box = boxes.push(
            CssBox::new(
                BoxOrigin::Element(3u8),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Absolute,
                false,
                None,
                ContainingBlock::Box(containing_box),
            ),
            Some(source_box),
            true,
        );
        let mut fragments = FragmentTree::default();
        let source_fragment = fragments.push(
            Fragment::from_horizontal_physical(source_box, PhysicalRect::default()),
            None,
            None,
        );
        fragments.record_static_position(StaticPosition {
            box_id: positioned_box,
            source: StaticPositionSource::Fragment(source_fragment),
            containing_block: ContainingBlock::Box(containing_box),
            logical_rect: LogicalRect {
                inline_start: 12.0,
                block_start: 8.0,
                inline_size: 0.0,
                block_size: 0.0,
            },
            containing_block_area: None,
        });

        assert_eq!(
            fragments.static_position_for_box(positioned_box),
            Some(&StaticPosition {
                box_id: positioned_box,
                source: StaticPositionSource::Fragment(source_fragment),
                containing_block: ContainingBlock::Box(containing_box),
                logical_rect: LogicalRect {
                    inline_start: 12.0,
                    block_start: 8.0,
                    inline_size: 0.0,
                    block_size: 0.0,
                },
                containing_block_area: None,
            })
        );
    }

    #[test]
    fn logical_fragment_geometry_derives_physical_geometry_at_the_edge() {
        let mut boxes = CssBoxTree::default();
        let flow = FlowAxes::new(crate::WritingMode::VerticalRl, crate::Direction::Ltr);
        let box_id = boxes.push(
            CssBox::new(
                BoxOrigin::Element(1u8),
                DisplayRole::BLOCK_FLOW,
                flow,
                PositioningScheme::Static,
                false,
                None,
                ContainingBlock::Initial,
            ),
            None,
            true,
        );
        let logical = LogicalRect {
            inline_start: 20.0,
            block_start: 30.0,
            inline_size: 40.0,
            block_size: 70.0,
        };
        let fragment = Fragment::from_logical(
            box_id,
            logical,
            PhysicalSize {
                width: 300.0,
                height: 200.0,
            },
            flow,
        );

        assert_eq!(fragment.logical_rect, logical);
        assert_eq!(fragment.flow(), flow);
        assert_eq!(
            fragment.physical_rect(),
            PhysicalRect {
                x: 200.0,
                y: 20.0,
                width: 70.0,
                height: 40.0,
            }
        );
    }

    #[test]
    fn resize_leaf_keeps_its_origin_and_updates_both_geometry_views() {
        let mut boxes = CssBoxTree::default();
        let box_id = boxes.push(
            CssBox::new(
                BoxOrigin::Element(1u8),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Absolute,
                true,
                None,
                ContainingBlock::Initial,
            ),
            None,
            true,
        );
        let mut fragments = FragmentTree::default();
        let leaf = fragments.push(
            Fragment::from_horizontal_physical(
                box_id,
                PhysicalRect {
                    x: 10.0,
                    y: 20.0,
                    width: 30.0,
                    height: 40.0,
                },
            ),
            None,
            None,
        );

        assert!(fragments.resize_leaf(
            leaf,
            PhysicalSize {
                width: 80.0,
                height: 25.0,
            },
        ));
        let fragment = fragments.get(leaf).expect("resized leaf");
        assert_eq!(
            fragment.physical_rect(),
            PhysicalRect {
                x: 10.0,
                y: 20.0,
                width: 80.0,
                height: 25.0,
            }
        );
        assert_eq!(fragment.logical_rect.inline_size, 80.0);
        assert_eq!(fragment.logical_rect.block_size, 25.0);
    }

    /// A negative margin can place a child's baseline above its parent's
    /// block-start edge, so a negative offset is a valid baseline. Rejecting
    /// it crashed baseline propagation on
    /// WPT `css/CSS2/css21-errata/s-11-1-1b-005.html`.
    #[test]
    fn baselines_accept_negative_offsets_and_reject_non_finite_ones() {
        assert!(Baselines::new(Some(-15.0), Some(-15.0)).is_some());
        assert!(Baselines::new(Some(-15.0), Some(20.0)).is_some());
        assert!(Baselines::new(Some(f32::NAN), None).is_none());
        assert!(Baselines::new(None, Some(f32::INFINITY)).is_none());
    }

    #[test]
    fn fragment_baselines_are_logical_outputs_not_physical_coordinates() {
        let mut boxes = CssBoxTree::default();
        let box_id = boxes.push(
            CssBox::new(
                BoxOrigin::Element(1u8),
                DisplayRole::BLOCK_FLOW,
                FlowAxes::HORIZONTAL_LTR,
                PositioningScheme::Static,
                false,
                None,
                ContainingBlock::Initial,
            ),
            None,
            true,
        );
        let baselines = Baselines::new(Some(6.0), Some(18.0)).expect("valid baselines");
        let fragment = Fragment::from_horizontal_physical(
            box_id,
            PhysicalRect {
                x: 40.0,
                y: 90.0,
                width: 120.0,
                height: 30.0,
            },
        )
        .with_baselines(baselines);

        assert_eq!(fragment.baselines, baselines);
        assert_eq!(fragment.physical_rect().y, 90.0);
        assert_ne!(fragment.baselines.first, Some(fragment.physical_rect().y));
        assert_ne!(
            fragment.baselines.last,
            Some(fragment.physical_rect().height)
        );
    }
}
