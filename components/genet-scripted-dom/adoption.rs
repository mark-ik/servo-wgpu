/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Storage transfer beneath DOM adoption. This module does not run the DOM
//! adoption algorithm or move a JS reflector between realms.

use std::collections::HashSet;

use crate::{NodeId, NodeKind, ScriptedDom};

/// A refused transfer leaves both stores, their queues and epochs unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubtreeTransferError {
    MissingNode,
    AttachedRoot,
    UnsupportedNode,
    AssociatedState,
    PendingSourceWork,
    DestinationCollision,
    InvalidTree,
    EpochExhausted,
}

impl ScriptedDom {
    /// Validate a prospective transfer without detaching anything or consuming
    /// mutation records. An attached root is accepted only when its entire parent
    /// chain is live, acyclic, and agrees with each parent's child list.
    ///
    /// This is not a reservation: callers must finish semantic removal and drain
    /// observer records before the detached transfer revalidates current state.
    /// Pending layout and observer records do not prevent this read-only check.
    pub fn preflight_subtree_transfer_to(
        &self,
        destination: &Self,
        root: NodeId,
    ) -> Result<Vec<NodeId>, SubtreeTransferError> {
        use SubtreeTransferError as Error;
        let root_parent = self
            .nodes
            .get(&root.raw())
            .ok_or(Error::MissingNode)?
            .parent;
        if self.parsing || self.observed_group.is_some() {
            return Err(Error::PendingSourceWork);
        }
        // An attached root still needs a removal epoch before the transfer epoch.
        self.structure_epoch
            .checked_add(if root_parent.is_some() { 2 } else { 1 })
            .ok_or(Error::EpochExhausted)?;
        destination
            .structure_epoch
            .checked_add(1)
            .ok_or(Error::EpochExhausted)?;
        let mut ancestry = HashSet::new();
        let mut cursor = root;
        while let Some(parent) = self
            .nodes
            .get(&cursor.raw())
            .ok_or(Error::MissingNode)?
            .parent
        {
            if !ancestry.insert(cursor) {
                return Err(Error::InvalidTree);
            }
            let parent_node = self.nodes.get(&parent.raw()).ok_or(Error::MissingNode)?;
            if parent_node
                .children
                .iter()
                .filter(|&&child| child == cursor)
                .count()
                != 1
            {
                return Err(Error::InvalidTree);
            }
            cursor = parent;
        }
        let mut members = Vec::new();
        let mut seen = HashSet::new();
        let mut pending = vec![(root, root_parent)];
        while let Some((id, parent)) = pending.pop() {
            let key = id.raw();
            if !seen.insert(key) {
                return Err(Error::InvalidTree);
            }
            let node = self.nodes.get(&key).ok_or(Error::MissingNode)?;
            if node.parent != parent {
                return Err(Error::InvalidTree);
            }
            if matches!(node.kind, NodeKind::Document | NodeKind::ShadowRoot) {
                return Err(Error::UnsupportedNode);
            }
            if self.shadow_hosts.contains_key(&key)
                || self.shadow_roots.contains_key(&key)
                || self.template_contents.contains_key(&key)
                || self.template_content_owners.contains_key(&key)
                || self.assigned_slots.contains_key(&key)
                || self.slot_assignments.contains_key(&key)
                || self.slot_changes.contains(&id)
                || self.template_document == Some(id)
            {
                return Err(Error::AssociatedState);
            }
            if destination.nodes.contains_key(&key) {
                return Err(Error::DestinationCollision);
            }
            members.push(id);
            pending.extend(node.children.iter().rev().map(|&child| (child, Some(id))));
        }
        // Manual slot assignment can retain a requested slottable even while
        // it is detached and has no current assigned-slot entry.
        if self.shadow_roots.values().any(|data| {
            data.manual.iter().any(|(slot, nodes)| {
                seen.contains(&slot.raw()) || nodes.iter().any(|id| seen.contains(&id.raw()))
            })
        }) || self
            .slot_assignments
            .values()
            .any(|nodes| nodes.iter().any(|id| seen.contains(&id.raw())))
        {
            return Err(Error::AssociatedState);
        }

        Ok(members)
    }

    /// Transfer after semantic removal while preserving the source's pending
    /// layout queue and its base sequence exactly. Observer records must already
    /// have been consumed; this method does not deliver or discard them.
    ///
    /// Retained layout records can name nodes that are no longer live in the
    /// source after success. Their consumer must invalidate using the recorded
    /// identities and former parents, without reading removed nodes in this store.
    pub fn transfer_detached_subtree_preserving_mutations_to(
        &mut self,
        destination: &mut Self,
        root: NodeId,
    ) -> Result<Vec<NodeId>, SubtreeTransferError> {
        let pending = std::mem::take(&mut self.mutations);
        let result = self.transfer_detached_subtree_to(destination, root);
        self.mutations = pending;
        result
    }

    /// Move a detached ordinary subtree into `destination`, preserving every
    /// node's identity and data. Return its identities in preorder so the host
    /// can relocate its ownership index and retention roots.
    ///
    /// This is a lower-level storage operation, **not** `Document.adoptNode`.
    /// The caller must coordinate wrapper identity, owning-document metadata,
    /// ranges, observer delivery, script state, layout and host pins before
    /// exposing the result or running collection. A pin retained only in this
    /// source arena cannot keep a transferred node alive in the destination.
    /// No callback runs during transfer, and every recoverable refusal happens
    /// before either store is changed.
    ///
    /// The initial boundary admits detached elements, fragments, doctypes and
    /// character data without shadow/template/slot metadata. Documents and
    /// shadow roots are refused. Source mutation queues must have been consumed
    /// by their owners, and parsing or an observer group must not be active. Insertion
    /// is a separate operation through the destination's ordinary mutation
    /// boundary; this function emits no invented DOM mutation record.
    /// Imported IDs retain their birth namespace, so destination capture goes
    /// through the identity pair (`try_capture_node_identity`), which names the
    /// origin arena this call registers on the destination.
    pub fn transfer_detached_subtree_to(
        &mut self,
        destination: &mut Self,
        root: NodeId,
    ) -> Result<Vec<NodeId>, SubtreeTransferError> {
        use SubtreeTransferError as Error;

        let root_node = self.nodes.get(&root.raw()).ok_or(Error::MissingNode)?;
        if root_node.parent.is_some() {
            return Err(Error::AttachedRoot);
        }
        if self.parsing
            || self.observed_group.is_some()
            || !self.observed.is_empty()
            || !self.mutations.is_empty()
        {
            return Err(Error::PendingSourceWork);
        }
        let source_epoch = self
            .structure_epoch
            .checked_add(1)
            .ok_or(Error::EpochExhausted)?;
        let destination_epoch = destination
            .structure_epoch
            .checked_add(1)
            .ok_or(Error::EpochExhausted)?;
        let members = self.preflight_subtree_transfer_to(destination, root)?;

        // Reserve before removing source data. Normal allocation failure has
        // Rust's usual process-level behavior; it cannot leave a recoverable
        // partially transferred result.
        destination.nodes.reserve(members.len());
        for id in &members {
            let node = self
                .nodes
                .remove(&id.raw())
                .expect("preflight checked every member");
            destination.nodes.insert(id.raw(), node);
        }
        // Register every foreign origin the destination now holds, so capture
        // and replay can translate an imported identity instead of refusing it.
        for id in &members {
            if id.origin_arena_id() != destination.arena_id() {
                destination.record_imported_arena(id.origin_arena_id());
            }
        }
        self.structure_epoch = source_epoch;
        destination.structure_epoch = destination_epoch;
        Ok(members)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layout_dom_api::{LayoutDom, LayoutDomMut};

    #[test]
    fn preflight_checks_parent_consistency_and_reserves_removal_epoch() {
        let mut source = ScriptedDom::new();
        let target = ScriptedDom::new();
        let node = source.create_text("attached");
        let parent = source.document();
        source.append_child(parent, node);
        source
            .nodes
            .get_mut(&parent.raw())
            .unwrap()
            .children
            .push(node);
        assert_eq!(
            source.preflight_subtree_transfer_to(&target, node),
            Err(SubtreeTransferError::InvalidTree)
        );
        source.nodes.get_mut(&parent.raw()).unwrap().children.pop();
        source.structure_epoch = u64::MAX - 1;
        assert_eq!(
            source.preflight_subtree_transfer_to(&target, node),
            Err(SubtreeTransferError::EpochExhausted)
        );
        assert_eq!(source.parent(node), Some(parent));
        assert_eq!(source.structure_epoch, u64::MAX - 1);
    }
}
