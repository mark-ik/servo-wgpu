// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! Private agent coordination. Node storage and canonical wrapper realms are
//! independent; only genuine reflectors cross the native entry boundary.
use super::*;
use crate::{MAIN_REALM, RealmId, SharedHost};
use std::{
    any::Any,
    collections::{BTreeMap, HashMap, HashSet},
    rc::Rc,
};

#[derive(Default)]
pub(crate) struct AgentDomState {
    creation_realms: HashMap<u32, RealmId>,
    hooks: BTreeMap<RealmId, Rc<dyn Any>>,
    observing: HashSet<RealmId>,
    observer_groups: BTreeMap<RealmId, usize>,
}

impl AgentDomState {
    pub(crate) fn register_realm(&mut self, realm: RealmId, arena: u32) {
        self.creation_realms.entry(arena).or_insert(realm);
    }

    pub(crate) fn remove_realm(&mut self, realm: RealmId) {
        self.hooks.remove(&realm);
        self.observing.remove(&realm);
        self.observer_groups.remove(&realm);
    }
}

pub(crate) fn current_host<C: CallCx + ?Sized>(cx: &C) -> Option<SharedHost> {
    cx.host_data()?.downcast::<RefCell<HostState>>().ok()
}

pub(crate) fn owner_host(start: &SharedHost, id: NodeId) -> Option<(RealmId, SharedHost)> {
    let (agent, arena, local) = {
        let host = start.borrow();
        (
            host.agent.upgrade(),
            host.dom.arena_id(),
            host.dom.is_live(id),
        )
    };
    let Some(agent) = agent else {
        return local.then(|| (MAIN_REALM, start.clone()));
    };
    {
        let state = agent.borrow();
        // The host's own arena identifies its realm; the queried node's birth
        // arena does not identify its physical owner after adoption.
        if local {
            if let Some(&realm) = state.dom_adoption.creation_realms.get(&arena) {
                if state
                    .hosts
                    .get(&realm)
                    .is_some_and(|host| Rc::ptr_eq(host, start))
                    && state
                        .dom_adoption
                        .creation_realms
                        .contains_key(&id.origin_arena_id())
                {
                    return Some((realm, start.clone()));
                }
            }
        }
        // Normal registered hosts need no metadata writes or registry copies.
        if state.dom_adoption.creation_realms.contains_key(&arena)
            && state
                .dom_adoption
                .creation_realms
                .contains_key(&id.origin_arena_id())
        {
            return state
                .hosts
                .iter()
                .find(|(_, host)| host.borrow().dom.is_live(id))
                .map(|(&realm, host)| (realm, host.clone()));
        }
    }
    // Public HostState.dom replacement can introduce an arena outside register.
    // Refresh once on an unknown arena, retaining historical birth mappings for
    // imported nodes whose creation store has since been replaced.
    let mut state = agent.borrow_mut();
    let arenas: Vec<_> = state
        .hosts
        .iter()
        .map(|(&realm, host)| (realm, host.borrow().dom.arena_id()))
        .collect();
    for (realm, arena) in arenas {
        state.dom_adoption.register_realm(realm, arena);
    }
    state
        .hosts
        .iter()
        .find(|(_, host)| host.borrow().dom.is_live(id))
        .map(|(&realm, host)| (realm, host.clone()))
}

pub(crate) fn creation_realm(start: &SharedHost, id: NodeId) -> Option<RealmId> {
    let (owner, _) = owner_host(start, id)?;
    let Some(agent) = start.borrow().agent.upgrade() else {
        return Some(owner);
    };
    let realm = agent
        .borrow()
        .dom_adoption
        .creation_realms
        .get(&id.origin_arena_id())
        .copied();
    realm.or(Some(owner))
}

pub(crate) fn host_for_call<E: ScriptEngine>(cx: &mut E::CallCx<'_>) -> Option<SharedHost> {
    let host = current_host(cx)?;
    let arg = cx.arg(0);
    if let Some(raw) = cx.reflector_data(&arg) {
        return owner_host(&host, NodeId::from_raw(raw)).map(|(_, owner)| owner);
    }
    Some(host)
}

pub(crate) fn validate<C: CallCx + ?Sized>(cx: &mut C, raw: u64) -> Result<(), C::Error> {
    let Some(host) = current_host(cx) else {
        return Err(cx.error("DOM host is unavailable"));
    };
    let Some((owner, _)) = owner_host(&host, NodeId::from_raw(raw)) else {
        return Err(cx.error("DOM node is no longer live"));
    };
    let realm = cx.current_realm();
    if let Some(agent) = host.borrow().agent.upgrade() {
        if !agent.borrow().frames.same_origin(realm, owner) {
            return Err(cx.error("SecurityError: cross-origin DOM access"));
        }
    }
    Ok(())
}

pub(crate) fn require_same_owner<C: CallCx + ?Sized>(
    cx: &mut C,
    ids: &[u64],
) -> Result<(), C::Error> {
    let host = current_host(cx).ok_or_else(|| cx.error("DOM host is unavailable"))?;
    let mut first = None;
    for &raw in ids {
        let (realm, _) = owner_host(&host, NodeId::from_raw(raw))
            .ok_or_else(|| cx.error("DOM node is no longer live"))?;
        if first.is_some_and(|first| first != realm) {
            return Err(cx.error("Cross-arena mutation requires an adoption transaction"));
        }
        first = Some(realm);
    }
    Ok(())
}

pub(crate) struct RegisterHooks;
impl<E: ScriptEngine> NativeFn<E> for RegisterHooks {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let hook = cx.arg(0);
        if let Some(host) = current_host(cx) {
            let agent = host.borrow().agent.upgrade();
            if let Some(agent) = agent {
                let realm = cx.current_realm();
                let mut state = agent.borrow_mut();
                state
                    .dom_adoption
                    .hooks
                    .entry(realm)
                    .or_insert_with(|| Rc::new(hook));
                let observing = !state.dom_adoption.observing.is_empty();
                drop(state);
                host.borrow_mut().dom.set_observing(observing);
            }
        }
        Ok(cx.undefined())
    }
}

fn call_hook<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    realm: RealmId,
    op: &str,
    mut args: Vec<E::Value>,
) -> Result<E::Value, E::Error> {
    let hook = current_host(cx)
        .and_then(|h| h.borrow().agent.upgrade())
        .and_then(|a| a.borrow().dom_adoption.hooks.get(&realm).cloned());
    let Some(hook) = hook else {
        return Ok(cx.undefined());
    };
    let Some(function) = hook.downcast_ref::<E::Value>() else {
        return Err(cx.error("DOM hook engine mismatch"));
    };
    args.insert(0, cx.make_string(op)?);
    let this = cx.undefined();
    E::call_from_call(cx, function, &this, &args)
}

pub(crate) fn apply_gc_policy<E: ScriptEngine>(
    rt: &mut crate::Runtime<E>,
    realm: RealmId,
    clear: &str,
    spec: &str,
) -> Result<(), crate::RealmError> {
    let hook = rt.agent.borrow().dom_adoption.hooks.get(&realm).cloned();
    let Some(hook) = hook else { return Ok(()) };
    let function = hook
        .downcast_ref::<E::Value>()
        .ok_or(crate::RealmError::Unsupported)?;
    // Force an expression statement, not a string directive prologue.
    let make = |engine: &mut E, text: &str| {
        engine
            .eval(&format!("({})", crate::js_str(text)))
            .map_err(|_| crate::RealmError::Unsupported)
    };
    let op = make(&mut rt.engine, "gcPolicy")?;
    let clear = make(&mut rt.engine, clear)?;
    let spec = make(&mut rt.engine, spec)?;
    let this = rt
        .engine
        .eval("void 0")
        .map_err(|_| crate::RealmError::Unsupported)?;
    rt.engine
        .call_function(function, &this, &[op, clear, spec])?;
    Ok(())
}

fn same_origin_realms<E: ScriptEngine>(cx: &mut E::CallCx<'_>) -> Vec<RealmId> {
    let Some(agent) = current_host(cx).and_then(|h| h.borrow().agent.upgrade()) else {
        return vec![];
    };
    let realm = cx.current_realm();
    let agent = agent.borrow();
    agent
        .hosts
        .keys()
        .copied()
        .filter(|&r| agent.frames.same_origin(realm, r))
        .collect()
}

pub(crate) struct AgentDispatch;
impl<E: ScriptEngine> NativeFn<E> for AgentDispatch {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let op_arg = cx.arg(0);
        let op = cx.value_to_string(&op_arg)?;
        match op.as_str() {
            "wrap" | "ownerDocument" => {
                let value = cx.arg(1);
                let Some(raw) = cx.reflector_data(&value) else {
                    return Ok(cx.undefined());
                };
                validate(cx, raw)?;
                let host = current_host(cx).unwrap();
                let realm = creation_realm(&host, NodeId::from_raw(raw)).unwrap();
                if op == "wrap" && realm == cx.current_realm() {
                    return Ok(cx.undefined());
                }
                call_hook::<E>(cx, realm, &op, vec![value])
            },
            "recordNode" => {
                let value = cx.arg(1);
                let Ok(raw) = cx.value_to_string(&value)?.parse::<u64>() else {
                    return Ok(cx.make_null());
                };
                if validate(cx, raw).is_err() {
                    return Ok(cx.make_null());
                }
                super::reflect_pinned::<E>(cx, raw)
            },
            "prepareAdoption" | "adopt" => transfer::<E>(cx, op == "adopt"),
            "prepareScripts" => {
                let parent_ref = cx.arg(1);
                let Some(raw) = cx.reflector_data(&parent_ref) else {
                    return Ok(cx.undefined());
                };
                validate(cx, raw)?;
                let host = current_host(cx).unwrap();
                let (realm, _) = owner_host(&host, NodeId::from_raw(raw)).unwrap();
                let parent = cx.arg(2);
                let roots = cx.arg(3);
                call_hook::<E>(cx, realm, "prepareScripts", vec![parent, roots])
            },
            "moGroup" => {
                let target = cx.arg(1);
                let raw = cx
                    .reflector_data(&target)
                    .ok_or_else(|| cx.error("DOM group requires a node reflector"))?;
                validate(cx, raw)?;
                let flag = cx.arg(2);
                let on = cx.value_to_string(&flag)? == "1";
                let host = current_host(cx).unwrap();
                let (realm, owner) = owner_host(&host, NodeId::from_raw(raw)).unwrap();
                let agent = host.borrow().agent.upgrade().unwrap();
                let transition = {
                    let mut state = agent.borrow_mut();
                    let depth = state.dom_adoption.observer_groups.entry(realm).or_default();
                    if on {
                        let first = *depth == 0;
                        *depth += 1;
                        first
                    } else {
                        *depth = depth.saturating_sub(1);
                        *depth == 0
                    }
                };
                if transition {
                    owner.borrow_mut().dom.set_observer_group(on);
                }
                Ok(cx.undefined())
            },
            "moActive" => {
                let value = cx.arg(1);
                let on = cx.value_to_string(&value)? == "true";
                let realm = cx.current_realm();
                if let Some(agent) = current_host(cx).and_then(|h| h.borrow().agent.upgrade()) {
                    let (hosts, any) = {
                        let mut a = agent.borrow_mut();
                        if on {
                            a.dom_adoption.observing.insert(realm);
                        } else {
                            a.dom_adoption.observing.remove(&realm);
                        }
                        (a.hosts.clone(), !a.dom_adoption.observing.is_empty())
                    };
                    for host in hosts.values() {
                        host.borrow_mut().dom.set_observing(any);
                    }
                }
                Ok(cx.undefined())
            },
            "moFlush" => {
                let Some(agent) = current_host(cx).and_then(|h| h.borrow().agent.upgrade()) else {
                    return Ok(cx.undefined());
                };
                let hosts = agent.borrow().hosts.clone();
                for (source, host) in &hosts {
                    if agent
                        .borrow()
                        .dom_adoption
                        .observer_groups
                        .get(source)
                        .copied()
                        .unwrap_or(0)
                        > 0
                    {
                        continue;
                    }
                    let encoded = {
                        let mut host = host.borrow_mut();
                        let records = host.dom.take_observed();
                        super::mutation_observer::encode(&records, &host.dom)
                    };
                    if encoded.is_empty() {
                        continue;
                    }
                    for &realm in hosts.keys() {
                        if agent.borrow().frames.same_origin(*source, realm) {
                            let blob = cx.make_string(&encoded)?;
                            call_hook::<E>(cx, realm, "moRecords", vec![blob])?;
                        }
                    }
                }
                Ok(cx.undefined())
            },
            "owners" | "adopted" | "connect" | "disconnect" | "rangeRemove" | "rangeInsert"
            | "rangeReplace" | "rangeData" | "rangeSplit" => {
                let count = match op.as_str() {
                    "owners" => 2,
                    "adopted" | "rangeSplit" => 3,
                    "rangeData" => 4,
                    _ => 1,
                };
                for realm in same_origin_realms::<E>(cx) {
                    let args = (1..=count).map(|i| cx.arg(i)).collect();
                    call_hook::<E>(cx, realm, &op, args)?;
                }
                Ok(cx.undefined())
            },
            _ => Err(cx.error("Unknown private DOM coordination operation")),
        }
    }
}

fn transfer<E: ScriptEngine>(cx: &mut E::CallCx<'_>, mutate: bool) -> Result<E::Value, E::Error> {
    let parent = cx.arg(1);
    let node = cx.arg(2);
    let (Some(parent), Some(node)) = (cx.reflector_data(&parent), cx.reflector_data(&node)) else {
        return Err(cx.error("DOM adoption requires genuine node reflectors"));
    };
    validate(cx, parent)?;
    validate(cx, node)?;
    let host = current_host(cx).unwrap();
    let (destination_realm, destination) = owner_host(&host, NodeId::from_raw(parent)).unwrap();
    let (source_realm, source) = owner_host(&host, NodeId::from_raw(node)).unwrap();
    if source_realm == destination_realm {
        return Ok(cx.undefined());
    }
    let node = NodeId::from_raw(node);
    let ids = {
        let source = source.borrow();
        let destination = destination.borrow();
        source
            .dom
            .preflight_subtree_transfer_to(&destination.dom, node)
    }
    .map_err(|e| cx.error(&format!("NotSupportedError: cross-arena subtree {e:?}")))?;
    // Contexts and active resource state need their own ownership transaction.
    let unsupported = {
        let source = source.borrow();
        ids.iter().any(|&id| {
            source.dom.element_name(id).is_some_and(|name| {
                matches!(&*name.local, "iframe" | "object" | "embed" | "canvas")
            })
        })
    };
    if unsupported {
        return Err(cx.error("NotSupportedError: cross-arena host-owned element"));
    }
    if !mutate {
        return cx.make_string("transfer");
    }
    let mut source = source.borrow_mut();
    let mut destination = destination.borrow_mut();
    let started: Vec<_> = ids
        .iter()
        .copied()
        .filter(|&id| super::markup_insertion::script_started(&source, id))
        .collect();
    source
        .dom
        .transfer_detached_subtree_preserving_mutations_to(&mut destination.dom, node)
        .map_err(|e| cx.error(&format!("NotSupportedError: cross-arena subtree {e:?}")))?;
    for id in ids {
        if source.pins.unpin(id) {
            destination.pins.pin(id);
        }
    }
    for id in started {
        super::markup_insertion::mark_script_started(&mut destination, id);
    }
    cx.make_string("transfer")
}

#[cfg(test)]
mod owner_lookup_tests {
    use super::*;

    #[test]
    fn public_store_replacement_refreshes_owner_without_reassigning_imported_birth_realm() {
        let agent = Rc::new(RefCell::new(crate::AgentState::default()));
        let source: SharedHost = Rc::new(RefCell::new(HostState::default()));
        let destination: SharedHost = Rc::new(RefCell::new(HostState::default()));
        source.borrow_mut().agent = Rc::downgrade(&agent);
        destination.borrow_mut().agent = Rc::downgrade(&agent);
        agent.borrow_mut().register(MAIN_REALM, source.clone());
        agent
            .borrow_mut()
            .register(MAIN_REALM + 1, destination.clone());
        let imported = source.borrow_mut().dom.create_text("retained");
        source
            .borrow_mut()
            .dom
            .transfer_detached_subtree_to(&mut destination.borrow_mut().dom, imported)
            .unwrap();
        source.borrow_mut().dom = ScriptedDom::new();
        let fresh = source.borrow_mut().dom.create_text("fresh");
        let (realm, owner) = owner_host(&source, fresh).unwrap();
        assert_eq!(realm, MAIN_REALM);
        assert!(Rc::ptr_eq(&owner, &source));
        let (realm, owner) = owner_host(&source, imported).unwrap();
        assert_eq!(realm, MAIN_REALM + 1);
        assert!(Rc::ptr_eq(&owner, &destination));
        assert_eq!(creation_realm(&destination, imported), Some(MAIN_REALM));
    }
}
