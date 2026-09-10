// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Browsing contexts belonging to the runtime's single script agent.

use crate::OwnerResolvedCx as _;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use browsing_context_api::{
    ActiveDocument, BrowsingContextId, BrowsingContextTree, FrameAttributes, SandboxFlags,
};
use genet_scripted_dom::{NodeId, ScriptedDom};
use layout_dom_api::LayoutDom;
use script_engine_api::{CallCx, MAIN_REALM, NativeFn, RealmError, RealmId, ScriptEngine};

use crate::{HostState, SharedHost, Surface, SurfaceError};

#[derive(Default)]
pub(crate) struct FrameState {
    pub(crate) tree: Option<BrowsingContextTree>,
    contexts: BTreeMap<RealmId, BrowsingContextId>,
    records: BTreeMap<RealmId, FrameRecord>,
    pending_main_load: bool,
}

struct FrameRecord {
    parent: RealmId,
    owner: NodeId,
    source: Option<String>,
    scripts: bool,
    lazy: bool,
    load_started: bool,
    parsed: bool,
    loaded: bool,
}

impl FrameState {
    pub(crate) fn defer_main_load(&mut self) -> bool {
        self.pending_main_load = self
            .records
            .values()
            .any(|record| record.parent == MAIN_REALM && !record.lazy && !record.loaded);
        self.pending_main_load
    }

    fn initialize(&mut self, url: &str) {
        if self.tree.is_none() {
            let tree = BrowsingContextTree::new(url);
            self.contexts.insert(MAIN_REALM, tree.top());
            self.tree = Some(tree);
        }
    }

    pub(crate) fn same_origin(&self, from: RealmId, to: RealmId) -> bool {
        if from == to {
            return true;
        }
        match (
            self.tree.as_ref(),
            self.contexts.get(&from),
            self.contexts.get(&to),
        ) {
            (Some(tree), Some(from), Some(to)) => tree.is_same_origin(*from, *to),
            _ => false,
        }
    }

    fn origin(&self, realm: RealmId) -> String {
        self.contexts
            .get(&realm)
            .and_then(|id| self.tree.as_ref()?.get(*id))
            .map(|context| context.document().origin.serialize())
            .unwrap_or_else(|| "null".into())
    }
}

fn host<E: ScriptEngine>(cx: &E::CallCx<'_>) -> Option<SharedHost> {
    cx.host_data()?.downcast::<RefCell<HostState>>().ok()
}

fn failure<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    error: impl std::fmt::Display,
) -> Result<E::Value, E::Error> {
    Err(cx.error(&error.to_string()))
}

fn security_error<E: ScriptEngine>(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
    cx.make_string("__security_error__")
}

fn eval<E: ScriptEngine>(cx: &mut E::CallCx<'_>, source: &str) -> Result<E::Value, E::Error> {
    E::eval_from_call(cx, source).map_err(|error| cx.error(&error.to_string()))
}

fn realm_eval<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    realm: RealmId,
    source: &str,
) -> Result<E::Value, E::Error> {
    E::eval_in_realm_from_call(cx, realm, source)
}

fn view<E: ScriptEngine>(cx: &mut E::CallCx<'_>, target: RealmId) -> Result<E::Value, E::Error> {
    let viewer = cx.current_realm();
    view_from::<E>(cx, viewer, target)
}

fn view_from<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    viewer: RealmId,
    target: RealmId,
) -> Result<E::Value, E::Error> {
    let Some(host) = host::<E>(cx) else {
        return Ok(cx.make_null());
    };
    let Some(agent) = host.borrow().agent.upgrade() else {
        return Ok(cx.make_null());
    };
    let same = agent.borrow().frames.same_origin(viewer, target);
    if same {
        match E::realm_global_from_call(cx, target) {
            Ok(value) => Ok(value),
            Err(error) => failure::<E>(cx, error),
        }
    } else {
        // Only the opaque realm id reaches the JS proxy factory. A foreign
        // global never enters the caller's heap-visible object graph.
        realm_eval::<E>(cx, viewer, &format!("__makeCrossOriginWindow({target})"))
    }
}

fn invoke<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    function: &E::Value,
    args: &[E::Value],
) -> Result<E::Value, E::Error> {
    let this = cx.undefined();
    E::call_from_call(cx, function, &this, args)
}

fn post_message<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    source: RealmId,
    target: RealmId,
    offset: usize,
) -> Result<E::Value, E::Error> {
    let Some(h) = host::<E>(cx) else {
        return Ok(cx.undefined());
    };
    let Some(agent) = h.borrow().agent.upgrade() else {
        return Ok(cx.undefined());
    };
    let base = h
        .borrow()
        .base_url
        .clone()
        .unwrap_or_else(|| "about:blank".into());
    agent.borrow_mut().frames.initialize(&base);
    let normalize = realm_eval::<E>(cx, source, "__frameNormalize")?;
    let options = cx.arg(offset + 1);
    let origin_value = invoke::<E>(cx, &normalize, &[options])?;
    let requested = cx.value_to_string(&origin_value)?;
    let serialize = realm_eval::<E>(cx, source, "__frameSerialize")?;
    let payload = cx.arg(offset);
    let options = cx.arg(offset + 1);
    let transfer = cx.arg(offset + 2);
    let record = invoke::<E>(cx, &serialize, &[payload, options, transfer])?;
    let (source_origin, allowed) = {
        let a = agent.borrow();
        let source_origin = a.frames.origin(source);
        let allowed = requested == "*"
            || if requested == "/" {
                a.frames.same_origin(source, target)
            } else {
                requested != "null" && requested == a.frames.origin(target)
            };
        (source_origin, allowed)
    };
    if !allowed {
        return Ok(cx.undefined());
    }
    let sender = view_from::<E>(cx, target, source)?;
    let origin = cx.make_string(&source_origin)?;
    let deliver = realm_eval::<E>(cx, target, "__frameDeliver")?;
    invoke::<E>(cx, &deliver, &[record, sender, origin])?;
    Ok(cx.undefined())
}

struct PostMessage;
impl<E: ScriptEngine> NativeFn<E> for PostMessage {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let source = cx.caller_realm();
        let mut target = cx.current_realm();
        let compare = eval::<E>(cx, "__frameSameReceiver")?;
        let Some(h) = host::<E>(cx) else {
            return Ok(cx.undefined());
        };
        let Some(agent) = h.borrow().agent.upgrade() else {
            return Ok(cx.undefined());
        };
        let realms: Vec<_> = agent.borrow().hosts.keys().copied().collect();
        let mut valid = false;
        for realm in realms {
            let global = E::realm_global_from_call(cx, realm)
                .map_err(|error| cx.error(&error.to_string()))?;
            let receiver = cx.this_value();
            let matches = invoke::<E>(cx, &compare, &[receiver, global])?;
            let matches = cx.value_to_string(&matches)?;
            if matches == "undefined" {
                valid = true;
                break;
            }
            if matches == "true" {
                target = realm;
                valid = true;
                break;
            }
        }
        if !valid {
            return Err(cx.error("postMessage receiver is not a Window"));
        }
        post_message::<E>(cx, source, target, 0)
    }
}
struct PostToWindow;
impl<E: ScriptEngine> NativeFn<E> for PostToWindow {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let id = cx.arg(0);
        let target = cx
            .value_to_string(&id)?
            .parse::<RealmId>()
            .unwrap_or(MAIN_REALM);
        let source = cx.current_realm();
        post_message::<E>(cx, source, target, 1)
    }
}

impl<E: ScriptEngine> crate::Runtime<E> {
    /// Bind each child's host providers before any authored child script runs.
    pub fn set_child_host_initializer(
        &mut self,
        initializer: impl Fn(RealmId, &SharedHost) + 'static,
    ) {
        self.agent.borrow_mut().child_host_initializer = Some(Rc::new(initializer));
    }
    /// Live child document realms keyed by the embedding element's opaque id.
    pub fn frame_realms(&self, parent: RealmId) -> Vec<(u64, RealmId)> {
        self.agent
            .borrow()
            .frames
            .records
            .iter()
            .filter(|(_, record)| record.parent == parent)
            .map(|(&realm, record)| (record.owner.raw() as u64, realm))
            .collect()
    }
}

struct FrameWindow;
impl<E: ScriptEngine> NativeFn<E> for FrameWindow {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let value = cx.arg(0);
        let Some(raw) = cx.owned_node(&value)? else {
            return Ok(cx.make_null());
        };
        let node = raw.id();
        let parent = cx.current_realm();
        let Some(parent_host) = host::<E>(cx) else {
            return Ok(cx.make_null());
        };
        let Some(agent) = parent_host.borrow().agent.upgrade() else {
            return Ok(cx.make_null());
        };
        if !parent_host.borrow().dom.is_live(node) {
            return Err(cx.error("frame access requires its owning document realm"));
        }
        if !parent_host.borrow_mut().is_connected_node(node) {
            return Ok(cx.make_null());
        }
        let existing = agent
            .borrow()
            .frames
            .records
            .iter()
            .find_map(|(&realm, record)| {
                (record.parent == parent && record.owner == node).then_some(realm)
            });
        if let Some(realm) = existing {
            return view::<E>(cx, realm);
        }
        let (attrs, source, url, base, fetch, loader, websocket, wake, resource_wake) = {
            let mut h = parent_host.borrow_mut();
            if !h.is_connected_node(node) {
                return Ok(cx.make_null());
            }
            let attr = |name: &str| {
                h.dom
                    .attribute(
                        node,
                        &layout_dom_api::Namespace::from(""),
                        &layout_dom_api::LocalName::from(name),
                    )
                    .map(str::to_owned)
            };
            let attrs = FrameAttributes {
                name: attr("name"),
                sandbox: attr("sandbox"),
                allow: attr("allow"),
                loading: attr("loading"),
            };
            let srcdoc = attr("srcdoc");
            let src = attr("src").unwrap_or_default();
            let base = h.base_url.clone().unwrap_or_else(|| "about:blank".into());
            let url = if srcdoc.is_some() {
                "about:srcdoc".into()
            } else if src.is_empty() {
                "about:blank".into()
            } else {
                crate::fetch::resolve_against(Some(&base), &src)
            };
            (
                attrs,
                srcdoc,
                url,
                base,
                h.fetch.clone(),
                h.script_loader.clone(),
                h.websocket.clone(),
                h.worker_wake.clone(),
                h.worker_resource_wake.clone(),
            )
        };
        // Match the existing host frame-loader policy: lazy frames retain an
        // initial context but have no loading task until a host activates them.
        let lazy = attrs
            .loading
            .as_deref()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("lazy"));
        let source = source.or_else(|| {
            if lazy {
                return None;
            }
            if url == "about:blank" {
                Some(String::new())
            } else {
                loader.as_ref().and_then(|loader| loader.load(&url))
            }
        });
        let (context, scripts) = {
            let mut a = agent.borrow_mut();
            a.frames.initialize(&base);
            let parent_context = a.frames.contexts[&parent];
            let tree = a.frames.tree.as_mut().expect("initialized");
            let context = tree
                .create_child(parent_context, raw.raw(), &attrs)
                .expect("live parent");
            let flags = tree.get(context).expect("new child").sandbox();
            let origin = if flags.contains(SandboxFlags::ORIGIN) {
                tree.get(context)
                    .expect("new child")
                    .document()
                    .origin
                    .clone()
            } else if lazy || url == "about:blank" || url == "about:srcdoc" {
                tree.get(parent_context)
                    .expect("parent")
                    .document()
                    .origin
                    .clone()
            } else {
                tree.mint_origin(&url)
            };
            tree.get_mut(context)
                .expect("new child")
                .navigate(ActiveDocument {
                    url: if lazy {
                        "about:blank".into()
                    } else {
                        url.clone()
                    },
                    origin,
                    initial_about_blank: lazy || url == "about:blank",
                });
            (context, !flags.contains(SandboxFlags::SCRIPTS))
        };
        let shared_timers = agent
            .borrow()
            .timer_state
            .clone()
            .expect("agent timer state");
        let style = parent_host.borrow().computed_style.clone();
        let dimension = |name: &str, fallback: f32| {
            style
                .as_ref()
                .and_then(|handler| handler.computed_value(raw.raw(), name))
                .and_then(|value| {
                    value
                        .trim()
                        .strip_suffix("px")
                        .and_then(|value| value.parse::<f32>().ok())
                })
                .filter(|value| value.is_finite() && *value >= 0.0)
                .unwrap_or(fallback)
        };
        let viewport_size = (dimension("width", 300.0), dimension("height", 150.0));
        let child_host = Rc::new(RefCell::new(HostState {
            dom: ScriptedDom::from_serialized_document(
                "<!doctype html><html><head></head><body></body></html>",
            ),
            base_url: Some(if lazy || url == "about:srcdoc" || url == "about:blank" {
                base
            } else {
                url.clone()
            }),
            fetch,
            script_loader: loader,
            websocket,
            worker_wake: wake,
            worker_resource_wake: resource_wake,
            agent: Rc::downgrade(&agent),
            viewport_size,
            worker_spawn: Some(crate::worker::worker_main::<E> as fn(_)),
            ..HostState::default()
        }));
        let child_for_install = child_host.clone();
        let mut attempted_realm = None;
        let result = E::create_realm_from_call(cx, child_host, |child| {
            let realm = child.current_realm();
            attempted_realm = Some(realm);
            {
                let mut a = agent.borrow_mut();
                a.register(realm, child_for_install.clone());
                a.frames.contexts.insert(realm, context);
                a.frames.records.insert(
                    realm,
                    FrameRecord {
                        parent,
                        owner: node,
                        source,
                        scripts,
                        lazy,
                        load_started: false,
                        parsed: false,
                        loaded: false,
                    },
                );
            }
            let initialize_host = agent.borrow().child_host_initializer.clone();
            if let Some(initialize) = initialize_host {
                initialize(realm, &child_for_install);
            }
            E::set_global_from_call(
                child,
                "__agentTimers",
                shared_timers
                    .downcast_ref::<E::Value>()
                    .expect("agent engine"),
            )?;
            E::eval_from_call(child, &format!("globalThis.__realmId={realm}"))?;
            if let Err(error) = crate::install_host_surface::<E>(
                &mut Surface::<E>::Callback(child),
                crate::GlobalScopeKind::Window,
            ) {
                return Err(match error {
                    SurfaceError::Realm(error) => error,
                    SurfaceError::Engine(error) => RealmError::Engine(format!("{error:?}")),
                });
            }
            E::eval_from_call(
                child,
                "delete globalThis.__agentTimers; delete globalThis.__realmId;",
            )?;
            if !lazy {
                E::eval_from_call(child, "setTimeout(function(){ __loadFrameDocument(); },0)")?;
            }
            Ok(())
        });
        match result {
            Ok(realm) => view::<E>(cx, realm),
            Err(error) => {
                if let Some(realm) = attempted_realm {
                    let mut agent = agent.borrow_mut();
                    agent.dom_adoption.remove_realm(realm);
                    agent.hosts.remove(&realm);
                    agent.opaque_roots.remove(&realm);
                    agent.frames.contexts.remove(&realm);
                    agent.frames.records.remove(&realm);
                }
                if let Some(tree) = agent.borrow_mut().frames.tree.as_mut() {
                    tree.discard(context);
                }
                failure::<E>(cx, error)
            },
        }
    }
}

struct LoadFrameDocument;
impl<E: ScriptEngine> NativeFn<E> for LoadFrameDocument {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let Some(h) = host::<E>(cx) else {
            return Ok(cx.undefined());
        };
        let Some(agent) = h.borrow().agent.upgrade() else {
            return Ok(cx.undefined());
        };
        let realm = cx.current_realm();
        let facts = {
            let mut a = agent.borrow_mut();
            a.frames.records.get_mut(&realm).and_then(|record| {
                if record.lazy || record.load_started {
                    return None;
                }
                record.load_started = true;
                Some((
                    record.parent,
                    record.owner,
                    record.source.take(),
                    record.scripts,
                ))
            })
        };
        let Some((parent, owner, source, scripts)) = facts else {
            return Ok(cx.undefined());
        };
        let parent_host = agent.borrow().hosts.get(&parent).cloned();
        let connected = parent_host.is_some_and(|host| host.borrow_mut().is_connected_node(owner));
        if let Some(source) = source.filter(|_| connected) {
            if !source.is_empty() {
                if scripts {
                    eval::<E>(
                        cx,
                        &format!(
                            "document.open();document.write({});document.close();",
                            crate::js_str(&source)
                        ),
                    )?;
                } else {
                    let parsed = ScriptedDom::from_serialized_document(&source);
                    {
                        let mut h = h.borrow_mut();
                        let root = h.dom.document();
                        let children: Vec<_> = h.dom.dom_children(root).collect();
                        for child in children {
                            h.dom.remove_child(child);
                        }
                        crate::dom::clone_into(&parsed, parsed.document(), &mut h.dom, root);
                    }
                    eval::<E>(cx, "__rebindDocument();__refreshNamedProperties();")?;
                }
            }
        }
        {
            let mut a = agent.borrow_mut();
            let record = a.frames.records.get_mut(&realm).expect("live frame");
            record.parsed = true;
            // Cancel only this pending initial load. Retain its realm/identity,
            // but release the parent's barrier without running source or events.
            record.loaded = !connected;
        }
        // Parsing creates descendants synchronously, but their document tasks
        // run later. A completed descendant releases its waiting ancestors only
        // after both of its load events have been delivered.
        let mut completing = if connected { realm } else { parent };
        loop {
            if completing == MAIN_REALM {
                let main_host = {
                    let mut a = agent.borrow_mut();
                    let waiting = a.frames.records.values().any(|record| {
                        record.parent == MAIN_REALM && !record.lazy && !record.loaded
                    });
                    if a.frames.pending_main_load && !waiting {
                        a.frames.pending_main_load = false;
                        a.hosts.get(&MAIN_REALM).cloned()
                    } else {
                        None
                    }
                };
                if let Some(main_host) = main_host {
                    main_host.borrow_mut().markup.ready_state = crate::ReadyState::Complete;
                    realm_eval::<E>(
                        cx,
                        MAIN_REALM,
                        "document.dispatchEvent(new Event('readystatechange'));window.dispatchEvent(new Event('load'))",
                    )?;
                }
                break;
            }
            let ready = {
                let mut a = agent.borrow_mut();
                let waiting =
                    a.frames.records.values().any(|record| {
                        record.parent == completing && !record.lazy && !record.loaded
                    });
                a.frames.records.get_mut(&completing).and_then(|record| {
                    if !record.parsed || record.loaded || waiting {
                        return None;
                    }
                    // Set before callbacks to make reentrant native calls inert.
                    record.loaded = true;
                    Some((record.parent, record.owner))
                })
            };
            let Some((parent, owner)) = ready else { break };
            realm_eval::<E>(cx, completing, "window.dispatchEvent(new Event('load'))")?;
            realm_eval::<E>(
                cx,
                parent,
                &format!(
                    "__dispatchSynthetic({}, 'load', {{bubbles:false}})",
                    crate::js_str(&owner.raw().to_string())
                ),
            )?;
            completing = parent;
        }
        Ok(cx.undefined())
    }
}

struct WindowRelation;
impl<E: ScriptEngine> NativeFn<E> for WindowRelation {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let key = cx.arg(0);
        let key = cx.value_to_string(&key)?;
        let realm = cx.current_realm();
        let Some(h) = host::<E>(cx) else {
            return Ok(cx.make_null());
        };
        let Some(agent) = h.borrow().agent.upgrade() else {
            return Ok(cx.make_null());
        };
        let relation = agent
            .borrow()
            .frames
            .records
            .get(&realm)
            .map(|r| (r.parent, r.owner));
        if key == "frameElement" {
            let Some((parent, owner)) = relation else {
                return Ok(cx.make_null());
            };
            if !agent.borrow().frames.same_origin(realm, parent) {
                return Ok(cx.make_null());
            }
            return realm_eval::<E>(
                cx,
                parent,
                &format!(
                    "__frameElementById({})",
                    crate::js_str(&owner.raw().to_string())
                ),
            );
        }
        let parent = relation.map(|r| r.0).unwrap_or(realm);
        view::<E>(cx, if key == "top" { MAIN_REALM } else { parent })
    }
}

struct WindowProperty;
impl<E: ScriptEngine> NativeFn<E> for WindowProperty {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let id = cx.arg(0);
        let target = cx
            .value_to_string(&id)?
            .parse::<RealmId>()
            .unwrap_or(MAIN_REALM);
        let key = cx.arg(1);
        let key = cx.value_to_string(&key)?;
        let Some(h) = host::<E>(cx) else {
            return Ok(cx.make_null());
        };
        let Some(agent) = h.borrow().agent.upgrade() else {
            return Ok(cx.make_null());
        };
        if matches!(key.as_str(), "window" | "self" | "frames") {
            return view::<E>(cx, target);
        }
        if key == "parent" || key == "top" {
            let parent = agent
                .borrow()
                .frames
                .records
                .get(&target)
                .map(|r| r.parent)
                .unwrap_or(target);
            return view::<E>(cx, if key == "top" { MAIN_REALM } else { parent });
        }
        if key == "opener" {
            return Ok(cx.make_null());
        }
        if key == "closed" {
            return eval::<E>(cx, "false");
        }
        if key == "length" {
            let n = agent
                .borrow()
                .frames
                .records
                .values()
                .filter(|r| r.parent == target)
                .count();
            return eval::<E>(cx, &n.to_string());
        }
        if let Ok(index) = key.parse::<usize>() {
            let child = agent
                .borrow()
                .frames
                .records
                .iter()
                .filter(|(_, r)| r.parent == target)
                .nth(index)
                .map(|(&id, _)| id);
            return match child {
                Some(id) => view::<E>(cx, id),
                None => security_error::<E>(cx),
            };
        }
        security_error::<E>(cx)
    }
}

pub(crate) fn install_frame_surface<E: ScriptEngine>(
    surface: &mut Surface<'_, '_, E>,
) -> Result<(), SurfaceError<E::Error>> {
    surface.set_function::<FrameWindow>("__frameWindow", 1)?;
    surface.set_function::<LoadFrameDocument>("__loadFrameDocument", 0)?;
    surface.set_function::<WindowRelation>("__windowRelation", 1)?;
    surface.set_function::<WindowProperty>("__windowProperty", 2)?;
    surface.set_function::<PostMessage>("__realmPostMessage", 2)?;
    surface.set_function::<PostToWindow>("__postToWindow", 4)?;
    surface.eval(include_str!("frames.js"))?;
    Ok(())
}
