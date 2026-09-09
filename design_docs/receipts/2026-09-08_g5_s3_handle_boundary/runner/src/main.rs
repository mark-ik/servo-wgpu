use genet_scripted_dom::{NodeId, ScriptedDom};
use layout_dom_api::LayoutDom;
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Instant;

/// Candidate A: a document/runtime boundary token in ReflectorData's existing
/// u64, while the hot LayoutDom NodeId stays pointer-sized and arena-local.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct BoundaryHandle(u64);

impl BoundaryHandle {
    const LOCAL_BITS: u32 = 40;
    const LOCAL_MASK: u64 = (1u64 << Self::LOCAL_BITS) - 1;
    const MAX_ARENA: u32 = (1u32 << (64 - Self::LOCAL_BITS)) - 1;

    fn checked(arena: u32, local: u64) -> Option<Self> {
        (arena <= Self::MAX_ARENA && local <= Self::LOCAL_MASK)
            .then_some(Self((u64::from(arena) << Self::LOCAL_BITS) | local))
    }

    fn arena(self) -> u32 {
        (self.0 >> Self::LOCAL_BITS) as u32
    }

    fn local(self) -> u64 {
        self.0 & Self::LOCAL_MASK
    }
}

struct Boundary<'a> {
    arena: u32,
    dom: &'a ScriptedDom,
}

impl<'a> Boundary<'a> {
    fn capture(&self, node: NodeId) -> Option<BoundaryHandle> {
        BoundaryHandle::checked(self.arena, self.dom.capture_node_id(node))
    }

    fn resolve(&self, handle: BoundaryHandle) -> Option<NodeId> {
        if handle.arena() != self.arena {
            return None;
        }
        let node = self.dom.remint_node_id(handle.local());
        self.dom.is_live(node).then_some(node)
    }
}

fn main() {
    let a = ScriptedDom::new();
    let b = ScriptedDom::new();
    let a_root = a.document();
    let b_root = b.document();
    let raw_alias = a_root.raw() == b_root.raw();

    let a_boundary = Boundary { arena: 101, dom: &a };
    let b_boundary = Boundary { arena: 102, dom: &b };
    let a_handle = a_boundary.capture(a_root).unwrap();
    let b_handle = b_boundary.capture(b_root).unwrap();

    assert_ne!(a_handle, b_handle);
    assert_eq!(a_boundary.resolve(a_handle), Some(a_root));
    assert_eq!(b_boundary.resolve(b_handle), Some(b_root));
    assert_eq!(b_boundary.resolve(a_handle), None);
    assert!(BoundaryHandle::checked(BoundaryHandle::MAX_ARENA, BoundaryHandle::LOCAL_MASK).is_some());
    assert!(BoundaryHandle::checked(BoundaryHandle::MAX_ARENA + 1, 0).is_none());
    assert!(BoundaryHandle::checked(1, BoundaryHandle::LOCAL_MASK + 1).is_none());

    // Candidate B cost comparison: a side table preserves the full native
    // NodeId range but adds one HashMap entry and lookup per boundary handle.
    let count = 100_000u64;
    let mut registry = HashMap::with_capacity(count as usize);
    for local in 0..count {
        registry.insert(BoundaryHandle(local + 1), NodeId::from_raw(local as usize));
    }
    let iterations = 1_000_000u64;
    let start = Instant::now();
    let mut direct_hits = 0usize;
    for i in 0..iterations {
        let h = BoundaryHandle::checked(101, i % count).unwrap();
        direct_hits ^= black_box(h.local() as usize);
    }
    let direct_elapsed = start.elapsed();
    let start = Instant::now();
    let mut table_hits = 0usize;
    for i in 0..iterations {
        let h = BoundaryHandle((i % count) + 1);
        table_hits ^= black_box(registry.get(&h).unwrap().raw());
    }
    let table_elapsed = start.elapsed();
    black_box((direct_hits, table_hits));

    println!(
        "{{\"profile\":\"{}\",\"pointer_bytes\":{},\"node_id_bytes\":{},\"reflector_data_bytes\":8,\"current_roots_raw_alias\":{},\"packed_handles_distinct\":true,\"foreign_handle_refused\":true,\"arena_bits\":24,\"local_bits\":40,\"packed_arena_limit\":{},\"packed_local_limit\":{},\"registry_entries\":{},\"registry_capacity\":{},\"direct_iterations\":{},\"direct_nanos\":{},\"table_iterations\":{},\"table_nanos\":{}}}",
        if cfg!(debug_assertions) { "debug" } else { "release" },
        std::mem::size_of::<usize>(),
        std::mem::size_of::<NodeId>(),
        raw_alias,
        BoundaryHandle::MAX_ARENA,
        BoundaryHandle::LOCAL_MASK,
        registry.len(),
        registry.capacity(),
        iterations,
        direct_elapsed.as_nanos(),
        iterations,
        table_elapsed.as_nanos(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_dom_roots_are_distinct_at_the_boundary() {
        let a = ScriptedDom::new();
        let b = ScriptedDom::new();
        let a_boundary = Boundary { arena: 1, dom: &a };
        let b_boundary = Boundary { arena: 2, dom: &b };
        let a_handle = a_boundary.capture(a.document()).unwrap();
        let b_handle = b_boundary.capture(b.document()).unwrap();
        assert_ne!(a_handle, b_handle);
        assert!(a_boundary.resolve(a_handle).is_some());
        assert!(b_boundary.resolve(a_handle).is_none());
    }

    #[test]
    fn packed_boundary_has_explicit_local_exhaustion() {
        assert!(BoundaryHandle::checked(BoundaryHandle::MAX_ARENA, BoundaryHandle::LOCAL_MASK).is_some());
        assert!(BoundaryHandle::checked(BoundaryHandle::MAX_ARENA + 1, 0).is_none());
        assert!(BoundaryHandle::checked(7, BoundaryHandle::LOCAL_MASK + 1).is_none());
    }

    #[test]
    fn same_actual_node_gets_same_canonical_boundary_key() {
        let dom = ScriptedDom::new();
        let boundary = Boundary { arena: 9, dom: &dom };
        assert_eq!(boundary.capture(dom.document()), boundary.capture(dom.document()));
    }
}
