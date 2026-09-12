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
use script_engine_api::{
    CallCx, HostData, MAIN_REALM, NativeFn, RealmError, RealmId, ScriptEngine,
};

use crate::{HostState, SharedHost, Surface, SurfaceError};

#[derive(Default)]
pub(crate) struct FrameState {
    pub(crate) tree: Option<BrowsingContextTree>,
    contexts: BTreeMap<RealmId, BrowsingContextId>,
    records: BTreeMap<RealmId, FrameRecord>,
    /// Contexts detached from the tree but not yet unloaded. HTML's "destroy a
    /// child navigable" splits exactly here: the container stops having a
    /// content navigable synchronously, and the document is unloaded and the
    /// context released in a later task, so no removal runs script.
    pending_teardown: Vec<Vec<(RealmId, Option<BrowsingContextId>)>>,
    pending_main_load: bool,
    /// The browsing-context key each live realm answers for, and the realm each
    /// key currently resolves to. A context's key never changes; the realm
    /// behind it does, every time the context navigates. This pair is the
    /// `WindowProxy`'s `[[Window]]` slot, kept host-side so that the proxy
    /// object itself holds nothing but the key.
    context_keys: BTreeMap<RealmId, ContextKey>,
    context_realms: BTreeMap<ContextKey, RealmId>,
    /// Each browsing context's `WindowProxy`, as the control function its
    /// bootstrap returned. Held here, rooted by the host, rather than named
    /// through the realm it was built in: that realm is discarded on the
    /// context's first navigation out of it, and the proxy has to outlive it.
    ///
    /// The proxy object itself is built natively by the engine adapter, before
    /// a line of script runs in that realm, and is one object in one heap -
    /// which is what makes `frame.contentWindow === childScript.window` hold
    /// across realms. A navigation replaces `context_realms`, and a discarded
    /// context loses its entry there, but a proxy a parent is still holding has
    /// to keep answering, so this entry outlives both.
    window_proxies: BTreeMap<ContextKey, HostData>,
    /// Navigations asked for but not yet performed. HTML's "navigate" is a
    /// task, not a synchronous call, and it has to be: `location.href = ...`
    /// runs *in* the realm the navigation is about to discard, and a realm
    /// cannot free the frame it is running in.
    pending_navigations: Vec<PendingNavigation>,
    /// The origin each realm's document had when it was bound, kept after the
    /// browsing context is gone. A discarded context still has to answer a
    /// parent that is holding its `WindowProxy` - HTML says its `document`
    /// keeps answering and only `closed` changes - and the tree it would be
    /// compared through no longer holds it. Never removed: a realm id is never
    /// reused, and this is the only record that a destroyed context was once
    /// same-origin with its holder.
    last_origins: BTreeMap<RealmId, String>,
}

/// One queued navigation. `target` is the realm the context is showing now -
/// resolved rather than the context key, so a navigation whose context has
/// meanwhile been destroyed is discovered and dropped.
pub(crate) struct PendingNavigation {
    target: RealmId,
    url: String,
    replace: bool,
}

/// A browsing context's stable identity for `WindowProxy` purposes: the id of
/// the first realm the context ever had. Distinct from [`BrowsingContextId`],
/// which the tree owns and which a realm cannot name from JS.
pub(crate) type ContextKey = RealmId;

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
            // One of them is a destroyed context whose `WindowProxy` somebody
            // is still holding. The tree cannot answer for that one any more,
            // so the origin it had when it was bound does; an opaque origin is
            // still never same-origin with anything, itself included.
            _ => {
                let from = self.origin(from);
                from == self.origin(to) && from != "null"
            },
        }
    }

    /// The realm holding `owner`'s nested browsing context, wherever that
    /// context's parent is. Owner ids are agent-wide identities, so this does
    /// not assume the removing script runs in the frame's parent realm -
    /// relocating a live iframe across arenas is exactly the case where it
    /// does not.
    fn realm_for_owner(&self, owner: NodeId) -> Option<RealmId> {
        self.records
            .iter()
            .find_map(|(&realm, record)| (record.owner == owner).then_some(realm))
    }

    /// Whether `owner` still embeds a live nested browsing context.
    pub(crate) fn holds_context(&self, owner: NodeId) -> bool {
        self.realm_for_owner(owner).is_some()
    }

    /// `root` and every realm nested beneath it, ancestor before descendant -
    /// HTML's order for "unload a document and its descendants", and the
    /// reverse of the order their state is released in.
    fn realm_tree(&self, root: RealmId) -> Vec<RealmId> {
        let mut order = vec![root];
        let mut index = 0;
        while index < order.len() {
            let parent = order[index];
            for (&realm, record) in &self.records {
                if record.parent == parent && !order.contains(&realm) {
                    order.push(realm);
                }
            }
            index += 1;
        }
        order
    }

    /// The synchronous half of destroying a child navigable: `root` and every
    /// realm beneath it stop being anyone's content navigable. `contentWindow`,
    /// `window.length` and the adoption preflight all answer from these maps, so
    /// after this the element is context-free even though its document has not
    /// been unloaded yet. Returns whether anything was queued.
    fn detach_subtree(&mut self, root: RealmId) -> bool {
        let group = self.realm_tree(root);
        if group.is_empty() || !self.records.contains_key(&root) {
            return false;
        }
        let detached = group
            .into_iter()
            .map(|realm| {
                self.records.remove(&realm);
                (realm, self.contexts.remove(&realm))
            })
            .collect();
        self.pending_teardown.push(detached);
        true
    }

    /// Bind `realm` to `key`, making it the context's current `Window`. Called
    /// once per realm: with its own id when the context is created, and with the
    /// context's existing key when a navigation replaces its realm.
    pub(crate) fn bind_context(&mut self, realm: RealmId, key: ContextKey) {
        self.context_keys.insert(realm, key);
        self.context_realms.insert(key, realm);
        let origin = self.origin(realm);
        self.last_origins.insert(realm, origin);
    }

    /// The key of the context `realm` currently serves, allocating `realm`'s own
    /// id as a fresh key when it serves none yet. Total, so a realm created
    /// outside the frame surface still gets a `WindowProxy`.
    fn context_key(&mut self, realm: RealmId) -> ContextKey {
        if let Some(&key) = self.context_keys.get(&realm) {
            return key;
        }
        self.bind_context(realm, realm);
        realm
    }

    /// Record the control function of the `WindowProxy` built for context
    /// `key`. Called once per context, when its first realm is created.
    pub(crate) fn set_window_proxy(&mut self, key: ContextKey, control: HostData) {
        self.window_proxies.insert(key, control);
    }

    /// The context's `WindowProxy` control function, or `None` if the context
    /// never got one - a backend without the global-`this` entry, or a realm
    /// created outside the frame surface.
    fn window_proxy_control(&self, key: ContextKey) -> Option<HostData> {
        self.window_proxies.get(&key).cloned()
    }

    /// Release a realm's binding. The key outlives it only if something else has
    /// already claimed it, which is what a navigation does before the old realm
    /// is torn down.
    fn release_context(&mut self, realm: RealmId) {
        if let Some(key) = self.context_keys.remove(&realm) {
            if self.context_realms.get(&key) == Some(&realm) {
                self.context_realms.remove(&key);
            }
        }
    }

    /// The origin of the document `realm` is showing. Falls back to the origin
    /// it was bound with once its browsing context is gone - a destroyed
    /// context still answers a parent that holds its `WindowProxy`, and there
    /// is nothing left in the tree to compare through.
    fn origin(&self, realm: RealmId) -> String {
        self.contexts
            .get(&realm)
            .and_then(|id| self.tree.as_ref()?.get(*id))
            .map(|context| context.document().origin.serialize())
            .or_else(|| self.last_origins.get(&realm).cloned())
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
    // The browsing context's one WindowProxy, whoever is looking. There is no
    // second object for a cross-origin window: the proxy answers differently
    // per accessing realm, which is what keeps `contentWindow` one identity
    // across a navigation that changes the frame's origin. `viewer` is
    // therefore not a parameter of *which* object, only of what it will say.
    let _ = viewer;
    let key = agent.borrow_mut().frames.context_key(target);
    match window_proxy::<E>(cx, key) {
        Ok(value) => Ok(value),
        Err(error) => failure::<E>(cx, error),
    }
}

/// The control function of the `WindowProxy` built for context `key`.
fn proxy_control<E: ScriptEngine>(cx: &E::CallCx<'_>, key: ContextKey) -> Option<HostData> {
    let host = host::<E>(cx)?;
    let agent = host.borrow().agent.upgrade()?;
    let control = agent.borrow().frames.window_proxy_control(key);
    control
}

/// Invoke a context's `WindowProxy` control function. See `window_proxy.js` for
/// the operations.
fn proxy_control_call<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    key: ContextKey,
    operation: &str,
    arguments: Vec<E::Value>,
) -> Result<E::Value, RealmError> {
    let holder = proxy_control::<E>(cx, key)
        .ok_or(RealmError::Refused("the context has no window proxy"))?;
    let name = cx
        .make_string(operation)
        .map_err(|_| RealmError::Refused("could not name a window-proxy operation"))?;
    let this = cx.undefined();
    let mut args = vec![name];
    args.extend(arguments);
    let control = holder
        .downcast_ref::<E::Value>()
        .ok_or(RealmError::Refused(
            "the window proxy belongs to another engine",
        ))?;
    E::call_from_call(cx, control, &this, &args)
        .map_err(|_| RealmError::Refused("the window-proxy control threw"))
}

/// Point the `WindowProxy` of context `key` at `realm`'s global object. This is
/// the `[[Window]]` write: the runtime performs it once when the context is
/// created, and a navigation performs it again with the new document's global.
/// Nothing else moves the slot.
pub(crate) fn bind_window_proxy<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    key: ContextKey,
    realm: RealmId,
) -> Result<(), RealmError> {
    let global = E::realm_global_from_call(cx, realm)?;
    let name = cx
        .make_string(&realm.to_string())
        .map_err(|_| RealmError::Refused("could not name a realm"))?;
    proxy_control_call::<E>(cx, key, "bind", vec![global, name])?;
    Ok(())
}

/// [`bind_window_proxy`], from outside any callback: the `[[Window]]` write
/// that names `realm` and its global object to the context's `WindowProxy`.
pub(crate) fn bind_window_proxy_realm<E: ScriptEngine>(
    engine: &mut E,
    agent: &Rc<RefCell<crate::AgentState>>,
    realm: RealmId,
    key: ContextKey,
) -> Result<(), RealmError> {
    let holder = agent.borrow().frames.window_proxy_control(key);
    let Some(holder) = holder else {
        return Ok(());
    };
    let global = engine.realm_global(realm)?;
    let operation = engine.eval_in_realm(realm, "('bind')")?;
    let name = engine.eval_in_realm(realm, &format!("String({realm})"))?;
    let undefined = engine.eval_in_realm(realm, "undefined")?;
    let control = holder
        .downcast_ref::<E::Value>()
        .ok_or(RealmError::Refused(
            "the window proxy belongs to another engine",
        ))?;
    engine.call_function(control, &undefined, &[operation, global, name])?;
    Ok(())
}

/// [`bind_window_proxy_hooks_from_call`], from outside any callback.
pub(crate) fn bind_window_proxy_hooks_in_realm<E: ScriptEngine>(
    engine: &mut E,
    agent: &Rc<RefCell<crate::AgentState>>,
    realm: RealmId,
    key: ContextKey,
) -> Result<(), RealmError> {
    let holder = agent.borrow().frames.window_proxy_control(key);
    let Some(holder) = holder else {
        return Ok(());
    };
    let hook = engine.eval_in_realm(realm, "__windowProxyHost")?;
    // Parenthesised so it is an expression and not a directive prologue, which
    // would leave the script with an empty completion value.
    let operation = engine.eval_in_realm(realm, "('host')")?;
    let undefined = engine.eval_in_realm(realm, "undefined")?;
    let control = holder
        .downcast_ref::<E::Value>()
        .ok_or(RealmError::Refused(
            "the window proxy belongs to another engine",
        ))?;
    engine.call_function(control, &undefined, &[operation, hook])?;
    Ok(())
}

/// Hand the `WindowProxy` of context `key` the host hook of the realm `cx` is
/// in, now that the realm has a surface. The hook is the native the
/// cross-origin branch dispatches through, and it is replaced on every
/// navigation so that it never outlives the realm that owns it.
pub(crate) fn bind_window_proxy_hooks_from_call<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    key: ContextKey,
) -> Result<(), RealmError> {
    if proxy_control::<E>(cx, key).is_none() {
        return Ok(());
    }
    let hook = E::eval_from_call(cx, "__windowProxyHost")?;
    proxy_control_call::<E>(cx, key, "host", vec![hook])?;
    Ok(())
}

/// The `WindowProxy` for `key`. The object itself never changes and never goes
/// stale: a navigation repoints it, and a discarded context's proxy keeps the
/// `Window` it last held, which is what HTML says a parent still holding it
/// sees.
fn window_proxy<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    key: ContextKey,
) -> Result<E::Value, RealmError> {
    proxy_control_call::<E>(cx, key, "proxy", Vec::new())
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
        let is_view = eval::<E>(cx, "__frameIsWindowView")?;
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
            // `window.postMessage(...)` now calls through the context's
            // WindowProxy, so the receiver is legitimately either object.
            let key = {
                let mut a = agent.borrow_mut();
                a.frames.context_key(realm)
            };
            let view = window_proxy::<E>(cx, key).map_err(|e| cx.error(&e.to_string()))?;
            let global = E::realm_global_from_call(cx, realm)
                .map_err(|error| cx.error(&error.to_string()))?;
            let receiver = cx.this_value();
            let matches = invoke::<E>(cx, &is_view, &[receiver, global, view])?;
            if cx.value_to_string(&matches)? == "true" {
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

    /// Whether a top-level navigation may proceed. HTML gives the user agent
    /// the last word on where the top-level browsing context goes; a host that
    /// shows chrome wants to arbitrate, and a plain scripted runtime does not
    /// (the default is to allow). A child navigation is never asked: a document
    /// navigating its own nested contexts is the page's business.
    pub fn set_top_level_navigation_policy(&mut self, policy: impl Fn(&str) -> bool + 'static) {
        self.agent.borrow_mut().top_level_navigation_policy = Some(Rc::new(policy));
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

/// The deferred half of HTML's "destroy a child navigable": unload each already
/// detached document and release its runtime state. Ancestor documents are
/// unloaded before their descendants, and state is released child-first, so a
/// parent's registrations outlive its children's.
///
/// Every step is best-effort past the first: a realm whose global already threw
/// must not leave its siblings half-torn-down, and this runs from a task with
/// nobody left to report a failure to.
fn run_pending_teardown<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    agent: &Rc<RefCell<crate::AgentState>>,
) -> Result<(), E::Error> {
    let cancel = agent.borrow().cancel_realm_tasks.clone();
    loop {
        let Some(group) = agent.borrow_mut().frames.pending_teardown.pop() else {
            return Ok(());
        };
        for (realm, _) in &group {
            let _ = realm_eval::<E>(
                cx,
                *realm,
                "window.dispatchEvent(new Event('pagehide'));                 window.dispatchEvent(new Event('unload'))",
            );
        }
        for (realm, context) in group.iter().rev() {
            // A parent may still hold this global. Leave it reporting what HTML
            // says a discarded context reports, before the realm goes.
            let _ = realm_eval::<E>(cx, *realm, "__discardBrowsingContext()");
            if let Some(cancel) = cancel
                .as_ref()
                .and_then(|value| value.downcast_ref::<E::Value>())
            {
                let id = cx.make_string(&realm.to_string())?;
                let _ = invoke::<E>(cx, cancel, &[id]);
            }
            {
                let mut a = agent.borrow_mut();
                a.frames.release_context(*realm);
                a.dom_adoption.remove_realm(*realm);
                a.hosts.remove(realm);
                a.opaque_roots.remove(realm);
                a.fetch_realms.retain(|_, owner| owner != realm);
            }
            if let Some(context) = *context {
                if let Some(tree) = agent.borrow_mut().frames.tree.as_mut() {
                    tree.discard(context);
                }
            }
            let _ = E::discard_realm_from_call(cx, *realm);
        }
    }
}

/// The removal half of the frame surface: the bootstrap's mutation funnel hands
/// each iframe leaving a connected tree to this, before the tree moves. The
/// context is detached here and unloaded in the queued task, because HTML's
/// removing steps must not run script.
struct DiscardFrame;
impl<E: ScriptEngine> NativeFn<E> for DiscardFrame {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let value = cx.arg(0);
        let Some(node) = cx.owned_node(&value)? else {
            return Ok(cx.undefined());
        };
        let owner = node.id();
        let container = node.owner_realm();
        drop(node);
        let Some(agent) = host::<E>(cx).and_then(|h| h.borrow().agent.upgrade()) else {
            return Ok(cx.undefined());
        };
        let realm = agent.borrow().frames.realm_for_owner(owner);
        let Some(realm) = realm else {
            return Ok(cx.undefined());
        };
        if !agent.borrow_mut().frames.detach_subtree(realm) {
            return Ok(cx.undefined());
        }
        // Queued on the container's realm, which by construction survives the
        // subtree being destroyed - the same route the initial load takes.
        realm_eval::<E>(
            cx,
            container,
            "setTimeout(function(){ __runFrameTeardown(); },0)",
        )?;
        Ok(cx.undefined())
    }
}

/// Drains whatever `__discardFrame` detached. Installed in every realm because
/// any realm can be the one that removed a frame.
struct RunFrameTeardown;
impl<E: ScriptEngine> NativeFn<E> for RunFrameTeardown {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let Some(agent) = host::<E>(cx).and_then(|h| h.borrow().agent.upgrade()) else {
            return Ok(cx.undefined());
        };
        run_pending_teardown::<E>(cx, &agent)?;
        Ok(cx.undefined())
    }
}

/// How many nested browsing contexts this agent currently holds. The funnel
/// asks before walking a removed subtree, so a document with no frames pays one
/// integer read per removal instead of a `querySelectorAll`.
struct LiveFrameCount;
impl<E: ScriptEngine> NativeFn<E> for LiveFrameCount {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let count = host::<E>(cx)
            .and_then(|h| h.borrow().agent.upgrade())
            .map(|agent| agent.borrow().frames.records.len())
            .unwrap_or(0);
        eval::<E>(cx, &count.to_string())
    }
}

/// Make the `WindowProxy` of the context keyed `key` the global `this` of the
/// realm `cx` is in, before a line of script has run there.
///
/// A context meeting its first document has no proxy yet, and gets one built
/// over a fresh shadow object with the handler that carries the WindowProxy
/// semantics already installed - so the host surface installed next lands on
/// the global object behind the proxy, not on the shadow. A navigation reuses
/// the proxy the context already has, which is the whole point of the object,
/// and only performs the `[[Window]]` write.
///
/// A backend that cannot take a global `this` is a no-op: one browsing context,
/// no navigation, and every identity test still holds against the global.
pub(crate) fn install_window_proxy_from_call<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    key: ContextKey,
) -> Result<(), RealmError> {
    let realm = cx.current_realm();
    let Some(agent) = host::<E>(cx).and_then(|h| h.borrow().agent.upgrade()) else {
        return Err(RealmError::Refused("the realm has left its agent"));
    };
    if proxy_control::<E>(cx, key).is_none() {
        let proxy = match E::new_window_proxy_from_call(cx) {
            Ok(proxy) => proxy,
            Err(RealmError::Unsupported) => return Ok(()),
            Err(error) => return Err(error),
        };
        let global = E::realm_global_from_call(cx, realm)?;
        E::set_global_from_call(cx, crate::WINDOW_PROXY_GLOBAL_SLOT, &global)?;
        E::finish_global_this_from_call(cx, &proxy)?;
        let control = E::eval_from_call(cx, crate::WINDOW_PROXY_BOOTSTRAP)?;
        agent
            .borrow_mut()
            .frames
            .set_window_proxy(key, Rc::new(control));
        // The bootstrap knows the Window it ran in but not that Window's realm
        // id, and the cross-origin branch compares realms. One idempotent
        // `[[Window]]` write supplies it.
        bind_window_proxy::<E>(cx, key, realm)?;
        return Ok(());
    }
    let global = E::realm_global_from_call(cx, realm)?;
    E::set_global_from_call(cx, crate::WINDOW_PROXY_GLOBAL_SLOT, &global)?;
    bind_window_proxy::<E>(cx, key, realm)?;
    let proxy = window_proxy::<E>(cx, key)?;
    E::finish_global_this_from_call(cx, &proxy)
}

struct FrameWindow;
impl<E: ScriptEngine> NativeFn<E> for FrameWindow {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let value = cx.arg(0);
        let Some(raw) = cx.owned_node(&value)? else {
            return Ok(cx.make_null());
        };
        let node = raw.id();
        // The frame's parent is its *container document's* realm, which is the
        // realm that physically owns the element - not the realm whose script
        // happened to read `contentWindow`, and not the realm its reflector was
        // minted in. Relocating a live iframe across arenas separates all three.
        let parent = raw.owner_realm();
        let parent_host = raw.host().clone();
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
        drop(raw);
        let plan = DocumentPlan {
            parent,
            owner: node,
            context,
            key: None,
            document_url: if lazy || url == "about:srcdoc" || url == "about:blank" {
                base
            } else {
                url.clone()
            },
            source,
            scripts,
            lazy,
            viewport_size,
            seams: HostSeams {
                fetch,
                script_loader: loader,
                websocket,
                worker_wake: wake,
                worker_resource_wake: resource_wake,
            },
        };
        match open_document_realm::<E>(cx, &agent, plan) {
            Ok(realm) => view::<E>(cx, realm),
            Err(error) => {
                if let Some(tree) = agent.borrow_mut().frames.tree.as_mut() {
                    tree.discard(context);
                }
                failure::<E>(cx, error)
            },
        }
    }
}

/// The seams a new document realm inherits from whichever document set it
/// loading: its fetch route, script loader, websocket handler and the two
/// worker wakeups. All refcounted handles, so this is a cheap clone of the
/// navigating document's host.
struct HostSeams {
    fetch: Option<Rc<dyn crate::fetch::FetchHandler>>,
    script_loader: Option<Rc<dyn crate::ScriptResourceLoader>>,
    websocket: Option<Rc<dyn crate::WebSocketHandler>>,
    worker_wake: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    worker_resource_wake: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
}

/// Everything needed to put a document in a browsing context, whether it is the
/// context's first document or its fifth. Initial load and navigation differ in
/// exactly two fields: a navigation supplies the context's existing `key`, and
/// it is never `lazy`.
struct DocumentPlan {
    parent: RealmId,
    owner: NodeId,
    context: BrowsingContextId,
    /// The context's WindowProxy key. `None` on a context's first document,
    /// where the new realm's own id becomes the key.
    key: Option<ContextKey>,
    document_url: String,
    source: Option<String>,
    scripts: bool,
    lazy: bool,
    viewport_size: (f32, f32),
    seams: HostSeams,
}

/// Create a realm and a `Document` for `plan`'s browsing context, install the
/// window surface on it, give it the context's `WindowProxy`, and queue the
/// document load. Re-enterable: the initial load of a frame and every later
/// navigation of it both arrive here, and the context, its container element
/// and its proxy are the same objects across all of them.
///
/// The realm is left registered on success. On failure everything it registered
/// is unwound, but the browsing context is the caller's to dispose of - a failed
/// navigation must not destroy a context that a failed *creation* must.
fn open_document_realm<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    agent: &Rc<RefCell<crate::AgentState>>,
    plan: DocumentPlan,
) -> Result<RealmId, RealmError> {
    let DocumentPlan {
        parent,
        owner,
        context,
        key,
        document_url,
        source,
        scripts,
        lazy,
        viewport_size,
        seams,
    } = plan;
    let shared_timers = agent
        .borrow()
        .timer_state
        .clone()
        .ok_or(RealmError::Refused("agent timer state is unavailable"))?;
    let child_host = Rc::new(RefCell::new(HostState {
        dom: ScriptedDom::from_serialized_document(
            "<!doctype html><html><head></head><body></body></html>",
        ),
        base_url: Some(document_url),
        fetch: seams.fetch,
        script_loader: seams.script_loader,
        websocket: seams.websocket,
        worker_wake: seams.worker_wake,
        worker_resource_wake: seams.worker_resource_wake,
        agent: Rc::downgrade(agent),
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
                    owner,
                    source,
                    scripts,
                    lazy,
                    load_started: false,
                    parsed: false,
                    loaded: false,
                },
            );
            // A navigation keeps the context's key, which is what carries the
            // WindowProxy's identity from the old document to this one.
            a.frames.bind_context(realm, key.unwrap_or(realm));
        }
        // Before anything authored is evaluated in this realm:
        // `finish_global_this` is an initialization, and the first line of
        // authored code closes its window.
        let context_key = key.unwrap_or(realm);
        install_window_proxy_from_call::<E>(child, context_key)?;
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
        // The realm has a surface now, so the context's WindowProxy can take
        // this realm's host hook - the native its cross-origin branch
        // dispatches through. Replaced on every navigation, so it never
        // outlives the realm that owns it.
        bind_window_proxy_hooks_from_call::<E>(child, context_key)?;
        if !lazy {
            E::eval_from_call(child, "setTimeout(function(){ __loadFrameDocument(); },0)")?;
        }
        Ok(())
    });
    if result.is_err() {
        if let Some(realm) = attempted_realm {
            let mut agent = agent.borrow_mut();
            agent.dom_adoption.remove_realm(realm);
            agent.hosts.remove(&realm);
            agent.opaque_roots.remove(&realm);
            agent.frames.contexts.remove(&realm);
            agent.frames.records.remove(&realm);
            agent.frames.release_context(realm);
        }
    }
    result
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

/// One property of the browsing context whose current realm is `target`, as
/// seen from `viewer`. The whole of the cross-origin-accessible surface plus
/// the indexed child contexts; anything else answers with the
/// `__security_error__` sentinel and the caller turns that into a
/// `SecurityError` built with the accessing realm's intrinsics.
fn window_property<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    viewer: RealmId,
    target: RealmId,
    key: &str,
) -> Result<E::Value, E::Error> {
    let Some(h) = host::<E>(cx) else {
        return Ok(cx.make_null());
    };
    let Some(agent) = h.borrow().agent.upgrade() else {
        return Ok(cx.make_null());
    };
    if matches!(key, "window" | "self" | "frames") {
        return view_from::<E>(cx, viewer, target);
    }
    if key == "parent" || key == "top" {
        let parent = agent
            .borrow()
            .frames
            .records
            .get(&target)
            .map(|r| r.parent)
            .unwrap_or(target);
        let to = if key == "top" { MAIN_REALM } else { parent };
        return view_from::<E>(cx, viewer, to);
    }
    if key == "opener" {
        return Ok(cx.make_null());
    }
    if key == "closed" {
        let live = agent.borrow().frames.records.contains_key(&target) || target == MAIN_REALM;
        return eval::<E>(cx, if live { "false" } else { "true" });
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
            Some(id) => view_from::<E>(cx, viewer, id),
            None => security_error::<E>(cx),
        };
    }
    security_error::<E>(cx)
}

/// The realm of the child of `target` whose container carries `name`. Named
/// access to a child browsing context is cross-origin-accessible, so this is
/// resolved through the tree rather than through the parent's DOM.
fn named_child<E: ScriptEngine>(
    cx: &E::CallCx<'_>,
    target: RealmId,
    name: &str,
) -> Option<RealmId> {
    let agent = host::<E>(cx)?.borrow().agent.upgrade()?;
    let agent = agent.borrow();
    let frames = &agent.frames;
    let tree = frames.tree.as_ref()?;
    let context = *frames.contexts.get(&target)?;
    let child = tree.named_frame(context, name)?;
    frames
        .contexts
        .iter()
        .find_map(|(&realm, &id)| (id == child).then_some(realm))
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
        let viewer = cx.current_realm();
        window_property::<E>(cx, viewer, target, &key)
    }
}

/// The host half of the `WindowProxy`'s cross-origin branch. The handler runs
/// in the realm its proxy was built in, not in the accessing one, so every
/// operation carries the accessing realm the adapter stamped on the way in;
/// nothing here reads an ambient realm.
///
/// `__windowProxyHost(operation, a, b, c, d)`:
///
/// - `sameOrigin(from, target)` -> `"true"` / `"false"`, compared through the
///   browsing-context tree against the target's *current* document origin.
/// - `denied(from)` -> a `SecurityError` `DOMException` built with `from`'s
///   intrinsics, for the handler to throw.
/// - `property(target, key, from)` -> one cross-origin-accessible value.
/// - `named(target, key, from)` -> the named child context's `WindowProxy`.
/// - `post(target, message, options, transfer, from)`.
/// - `navigate(target, url, mode, from)` -> `location.href =` / `replace()`
///   across origins.
struct WindowProxyHost;
impl<E: ScriptEngine> NativeFn<E> for WindowProxyHost {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let operation = cx.arg(0);
        let operation = cx.value_to_string(&operation)?;
        let realm_arg = |cx: &mut E::CallCx<'_>, index: usize| -> Result<RealmId, E::Error> {
            let value = cx.arg(index);
            Ok(cx
                .value_to_string(&value)?
                .parse::<RealmId>()
                .unwrap_or(MAIN_REALM))
        };
        match operation.as_str() {
            "sameOrigin" => {
                let from = realm_arg(cx, 1)?;
                let target = realm_arg(cx, 2)?;
                let same = host::<E>(cx)
                    .and_then(|h| h.borrow().agent.upgrade())
                    .map(|agent| agent.borrow().frames.same_origin(from, target))
                    .unwrap_or(true);
                eval::<E>(cx, if same { "true" } else { "false" })
            },
            "denied" => {
                let from = realm_arg(cx, 1)?;
                realm_eval::<E>(
                    cx,
                    from,
                    "new DOMException('Cross-origin window access is denied', 'SecurityError')",
                )
            },
            "property" => {
                let target = realm_arg(cx, 1)?;
                let key = cx.arg(2);
                let key = cx.value_to_string(&key)?;
                let from = realm_arg(cx, 3)?;
                window_property::<E>(cx, from, target, &key)
            },
            "named" => {
                let target = realm_arg(cx, 1)?;
                let key = cx.arg(2);
                let key = cx.value_to_string(&key)?;
                let from = realm_arg(cx, 3)?;
                match named_child::<E>(cx, target, &key) {
                    Some(child) => view_from::<E>(cx, from, child),
                    None => Ok(cx.make_null()),
                }
            },
            "post" => {
                let target = realm_arg(cx, 1)?;
                let from = realm_arg(cx, 5)?;
                post_message::<E>(cx, from, target, 2)
            },
            "navigate" => {
                let target = realm_arg(cx, 1)?;
                let url = cx.arg(2);
                let url = cx.value_to_string(&url)?;
                let mode = cx.arg(3);
                let mode = cx.value_to_string(&mode)?;
                let from = realm_arg(cx, 4)?;
                navigate_context::<E>(cx, from, target, &url, mode == "replace")
            },
            _ => Ok(cx.undefined()),
        }
    }
}

/// Run `f` against the browsing context `realm` is showing, if this agent has
/// a context tree at all. A bare runtime - one that never made a frame and
/// never posted a message - has none, and its `history` falls back to the
/// per-document list in [`HostState`].
pub(crate) fn with_context<E: ScriptEngine, R>(
    cx: &E::CallCx<'_>,
    realm: RealmId,
    f: impl FnOnce(&mut browsing_context_api::BrowsingContext) -> R,
) -> Option<R> {
    let agent = host::<E>(cx)?.borrow().agent.upgrade()?;
    let mut agent = agent.borrow_mut();
    let id = *agent.frames.contexts.get(&realm)?;
    let tree = agent.frames.tree.as_mut()?;
    Some(f(tree.get_mut(id)?))
}

/// [`navigate_context`], for the history surface: a traversal whose entry names
/// a different document is a navigation like any other.
pub(crate) fn navigate_from_call<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    realm: RealmId,
    url: &str,
    replace: bool,
) -> Result<E::Value, E::Error> {
    navigate_context::<E>(cx, realm, realm, url, replace)
}

/// A URL with its fragment removed. Two URLs that agree here and differ in
/// their fragment are a fragment navigation: same document, new URL, one
/// `hashchange`.
fn without_fragment(url: &str) -> &str {
    match url.find('#') {
        Some(index) => &url[..index],
        None => url,
    }
}

/// HTML's "navigate", as far as a browsing context in this agent is concerned.
///
/// A fragment-only navigation is performed here and now: it keeps the document,
/// so there is nothing to unload and nothing to discard. Anything else is
/// queued, because a document-replacing navigation ends by discarding the realm
/// it was asked from - `location.href = ...` runs in exactly that realm - and a
/// realm cannot free the frame it is running in. The queue is drained by
/// `__runNavigations`, from a task on a realm that survives.
fn navigate_context<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    from: RealmId,
    target: RealmId,
    input: &str,
    replace: bool,
) -> Result<E::Value, E::Error> {
    let Some(agent) = host::<E>(cx).and_then(|h| h.borrow().agent.upgrade()) else {
        return Ok(cx.undefined());
    };
    let (source_base, target_base) = {
        let a = agent.borrow();
        let base = |realm: RealmId| {
            a.hosts
                .get(&realm)
                .and_then(|host| host.borrow().base_url.clone())
        };
        (base(from), base(target))
    };
    let url = crate::fetch::resolve_against(source_base.as_deref(), input);
    // A `javascript:` URL is not a navigation: HTML evaluates it against the
    // target's Document and, whatever it returns, never unloads, never joins
    // the session history and never fires `load`. genet does not evaluate one
    // at all, so the whole of it is to do nothing.
    if url.len() >= 11 && url[..11].eq_ignore_ascii_case("javascript:") {
        return Ok(cx.undefined());
    }
    let current = target_base.unwrap_or_else(|| "about:blank".into());
    if without_fragment(&url) == without_fragment(&current) && url != current {
        return navigate_fragment::<E>(cx, &agent, target, &url, &current, replace);
    }
    if url == current && url.contains('#') {
        // Same URL including the fragment: HTML performs the navigation but
        // fires no `hashchange`.
        return Ok(cx.undefined());
    }
    // The queue is drained on the container's realm, which by construction
    // outlives the subtree being replaced - the same route the initial load and
    // the teardown both take.
    let container = agent
        .borrow()
        .frames
        .records
        .get(&target)
        .map(|record| record.parent);
    let Some(container) = container else {
        // A top-level context. Its realm is the agent's main realm, which the
        // engine refuses to discard, so there is nothing this lane can replace
        // under it; the document URL moves and the history entry joins.
        return navigate_top_level::<E>(cx, &agent, target, &url, replace);
    };
    agent
        .borrow_mut()
        .frames
        .pending_navigations
        .push(PendingNavigation {
            target,
            url,
            replace,
        });
    realm_eval::<E>(
        cx,
        container,
        "setTimeout(function(){ __runNavigations(); },0)",
    )?;
    Ok(cx.undefined())
}

/// A fragment navigation: the document stays, its URL moves, the session
/// history joins, and one `hashchange` is fired at the Window.
fn navigate_fragment<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    agent: &Rc<RefCell<crate::AgentState>>,
    target: RealmId,
    url: &str,
    current: &str,
    replace: bool,
) -> Result<E::Value, E::Error> {
    {
        let mut a = agent.borrow_mut();
        if let Some(host) = a.hosts.get(&target) {
            host.borrow_mut().base_url = Some(url.to_owned());
        }
        let context = a.frames.contexts.get(&target).copied();
        if let (Some(context), Some(tree)) = (context, a.frames.tree.as_mut()) {
            if let Some(context) = tree.get_mut(context) {
                // A fragment navigation keeps the document, so its entry
                // carries the same document stamp - traversing back to it must
                // not reload.
                let entry = browsing_context_api::HistoryEntry::for_url(url.to_owned())
                    .in_document(context.document_serial());
                if replace {
                    context.history_mut().replace(entry);
                } else {
                    context.history_mut().push(entry);
                }
                context.set_document_url(url.to_owned());
            }
        }
    }
    realm_eval::<E>(
        cx,
        target,
        &format!(
            "(function(o,n){{var e=new Event('hashchange');             Object.defineProperty(e,'oldURL',{{value:o,enumerable:true,configurable:true}});             Object.defineProperty(e,'newURL',{{value:n,enumerable:true,configurable:true}});             window.dispatchEvent(e);}})({},{})",
            crate::js_str(current),
            crate::js_str(url)
        ),
    )?;
    Ok(cx.undefined())
}

/// A top-level, document-replacing navigation. The host policy hook decides
/// whether it may proceed at all; when it does, the document URL and the
/// session history move exactly as a child's do.
///
/// What does *not* happen here is the realm replacement a child gets. The
/// agent's main realm is the one the engine refuses to discard and the one
/// every embedder's `Runtime::eval` resolves against, so replacing it is not a
/// change this lane can make behind the `Runtime` API. See the phase note in
/// the realms plan.
fn navigate_top_level<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    agent: &Rc<RefCell<crate::AgentState>>,
    target: RealmId,
    url: &str,
    replace: bool,
) -> Result<E::Value, E::Error> {
    let allowed = agent
        .borrow()
        .top_level_navigation_policy
        .as_ref()
        .map(|policy| policy(url))
        .unwrap_or(true);
    if !allowed {
        return Ok(cx.undefined());
    }
    let mut a = agent.borrow_mut();
    if let Some(host) = a.hosts.get(&target) {
        host.borrow_mut().base_url = Some(url.to_owned());
    }
    let context = a.frames.contexts.get(&target).copied();
    if let (Some(context), Some(tree)) = (context, a.frames.tree.as_mut()) {
        if let Some(context) = tree.get_mut(context) {
            let origin = browsing_context_api::Origin::of_url(url, 0);
            context.navigate_with(
                ActiveDocument {
                    url: url.to_owned(),
                    origin,
                    initial_about_blank: false,
                },
                replace,
            );
        }
    }
    drop(a);
    Ok(cx.undefined())
}

/// Unload `root`'s document and every document nested under it, ancestor before
/// descendant, and release their runtime state child-first. `root`'s browsing
/// context survives - it is the one being navigated - while its descendants'
/// are destroyed outright.
///
/// Returns the realms to discard, which the caller does only once the
/// replacement realm exists and the context's `WindowProxy` has been repointed
/// at it: nothing may observe a context with no Window.
fn unload_for_navigation<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    agent: &Rc<RefCell<crate::AgentState>>,
    root: RealmId,
) -> Vec<RealmId> {
    let group = agent.borrow().frames.realm_tree(root);
    for realm in &group {
        let _ = realm_eval::<E>(
            cx,
            *realm,
            "window.dispatchEvent(new Event('pagehide'));             window.dispatchEvent(new Event('unload'))",
        );
    }
    let cancel = agent.borrow().cancel_realm_tasks.clone();
    for realm in group.iter().rev() {
        if *realm != root {
            // A parent may still hold this global. Leave it reporting what HTML
            // says a discarded context reports, before the realm goes.
            let _ = realm_eval::<E>(cx, *realm, "__discardBrowsingContext()");
        }
        if let Some(cancel) = cancel
            .as_ref()
            .and_then(|value| value.downcast_ref::<E::Value>())
        {
            if let Ok(id) = cx.make_string(&realm.to_string()) {
                let _ = invoke::<E>(cx, cancel, &[id]);
            }
        }
        let context = {
            let mut a = agent.borrow_mut();
            let context = a.frames.contexts.remove(realm);
            a.frames.records.remove(realm);
            a.frames.release_context(*realm);
            a.dom_adoption.remove_realm(*realm);
            a.hosts.remove(realm);
            a.opaque_roots.remove(realm);
            a.fetch_realms.retain(|_, owner| owner != realm);
            context
        };
        if *realm != root {
            if let Some(context) = context {
                if let Some(tree) = agent.borrow_mut().frames.tree.as_mut() {
                    tree.discard(context);
                }
            }
        }
    }
    group
}

/// Perform one queued navigation: unload, new realm, new `Document`, proxy
/// rebind, interleaved parse. The browsing context, its container element and
/// its `WindowProxy` are the same objects on the other side; everything else is
/// replaced.
fn perform_navigation<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    agent: &Rc<RefCell<crate::AgentState>>,
    navigation: PendingNavigation,
) -> Result<(), E::Error> {
    let PendingNavigation {
        target,
        url,
        replace,
    } = navigation;
    let facts = (|| {
        let a = agent.borrow();
        let record = a.frames.records.get(&target)?;
        let context = *a.frames.contexts.get(&target)?;
        let host = a.hosts.get(&target)?.clone();
        Some((record.parent, record.owner, record.scripts, context, host))
    })();
    // A context destroyed between the request and the task takes its
    // navigation with it.
    let Some((parent, owner, scripts, context, outgoing)) = facts else {
        return Ok(());
    };
    let key = agent.borrow_mut().frames.context_key(target);
    let (seams, viewport_size, loader) = {
        let host = outgoing.borrow();
        (
            HostSeams {
                fetch: host.fetch.clone(),
                script_loader: host.script_loader.clone(),
                websocket: host.websocket.clone(),
                worker_wake: host.worker_wake.clone(),
                worker_resource_wake: host.worker_resource_wake.clone(),
            },
            host.viewport_size,
            host.script_loader.clone(),
        )
    };
    let source = if url == "about:blank" {
        Some(String::new())
    } else {
        loader.as_ref().and_then(|loader| loader.load(&url))
    };
    // The new document's origin, and the history join. HTML's initial
    // `about:blank` replaces; everything else obeys the caller.
    {
        let mut a = agent.borrow_mut();
        if let Some(tree) = a.frames.tree.as_mut() {
            let origin = if url == "about:blank" {
                tree.get(context)
                    .map(|context| context.document().origin.clone())
                    .unwrap_or_else(|| tree.mint_opaque_origin())
            } else {
                tree.mint_origin(&url)
            };
            if let Some(context) = tree.get_mut(context) {
                context.navigate_with(
                    ActiveDocument {
                        url: url.clone(),
                        origin,
                        initial_about_blank: false,
                    },
                    replace,
                );
            }
        }
    }
    let discard = unload_for_navigation::<E>(cx, agent, target);
    let plan = DocumentPlan {
        parent,
        owner,
        context,
        key: Some(key),
        document_url: url,
        source,
        scripts,
        lazy: false,
        viewport_size,
        seams,
    };
    let opened = open_document_realm::<E>(cx, agent, plan);
    // Only now: the context has a Window again, and the proxy points at it.
    for realm in discard {
        let _ = E::discard_realm_from_call(cx, realm);
    }
    match opened {
        Ok(_) => Ok(()),
        Err(error) => Err(cx.error(&error.to_string())),
    }
}

/// Drains whatever `navigate` queued. Installed in every realm because any
/// realm can be the one that asked.
struct RunNavigations;
impl<E: ScriptEngine> NativeFn<E> for RunNavigations {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let Some(agent) = host::<E>(cx).and_then(|h| h.borrow().agent.upgrade()) else {
            return Ok(cx.undefined());
        };
        loop {
            let Some(navigation) = agent.borrow_mut().frames.pending_navigations.pop() else {
                return Ok(cx.undefined());
            };
            perform_navigation::<E>(cx, &agent, navigation)?;
        }
    }
}

/// `__navigate(url, mode)` - the realm's own `location.href = `, `assign`,
/// `replace` and `reload`, from the document that owns the location.
struct Navigate;
impl<E: ScriptEngine> NativeFn<E> for Navigate {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let url = cx.arg(0);
        let url = cx.value_to_string(&url)?;
        let mode = cx.arg(1);
        let mode = cx.value_to_string(&mode)?;
        let realm = cx.current_realm();
        navigate_context::<E>(cx, realm, realm, &url, mode == "replace")
    }
}

/// `__navigateFrame(ref, url)` - a connected `<iframe>` whose `src` changed.
/// HTML navigates the element's *existing* nested browsing context rather than
/// creating one, which is what keeps `contentWindow` the same object across a
/// `src` assignment.
struct NavigateFrame;
impl<E: ScriptEngine> NativeFn<E> for NavigateFrame {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let value = cx.arg(0);
        let Some(node) = cx.owned_node(&value)? else {
            return Ok(cx.undefined());
        };
        let owner = node.id();
        drop(node);
        let url = cx.arg(1);
        let url = cx.value_to_string(&url)?;
        let from = cx.current_realm();
        let Some(agent) = host::<E>(cx).and_then(|h| h.borrow().agent.upgrade()) else {
            return Ok(cx.undefined());
        };
        let target = agent.borrow().frames.realm_for_owner(owner);
        let Some(target) = target else {
            return Ok(cx.undefined());
        };
        navigate_context::<E>(cx, from, target, &url, false)
    }
}

pub(crate) fn install_frame_surface<E: ScriptEngine>(
    surface: &mut Surface<'_, '_, E>,
) -> Result<(), SurfaceError<E::Error>> {
    surface.set_function::<FrameWindow>("__frameWindow", 1)?;
    surface.set_function::<LoadFrameDocument>("__loadFrameDocument", 0)?;
    surface.set_function::<DiscardFrame>("__discardFrame", 1)?;
    surface.set_function::<RunFrameTeardown>("__runFrameTeardown", 0)?;
    surface.set_function::<LiveFrameCount>("__liveFrameCount", 0)?;
    surface.set_function::<WindowRelation>("__windowRelation", 1)?;
    surface.set_function::<WindowProperty>("__windowProperty", 2)?;
    surface.set_function::<WindowProxyHost>("__windowProxyHost", 5)?;
    surface.set_function::<RunNavigations>("__runNavigations", 0)?;
    surface.set_function::<Navigate>("__navigate", 2)?;
    surface.set_function::<NavigateFrame>("__navigateFrame", 2)?;
    surface.set_function::<PostMessage>("__realmPostMessage", 2)?;
    surface.set_function::<PostToWindow>("__postToWindow", 4)?;
    surface.eval(include_str!("frames.js"))?;
    Ok(())
}
