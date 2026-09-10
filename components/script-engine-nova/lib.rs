// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Nova backend for [`script_engine_api`] — the primary backend on 64-bit targets.
//!
//! Nova is pointer-width-bound (its data-oriented `Value` is `usize`-sized), so
//! this crate is gated to 64-bit targets and compiles to an empty shell on 32-bit
//! targets. That includes Nova on wasm64 and Boa on wasm32. The
//! native-data reflector rides on the patched `EmbedderObject` (genet-embedder
//! branch of the fork). Engine-native types stay confined here.

#[cfg(target_pointer_width = "64")]
mod native {
    use std::any::Any;
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, VecDeque};
    use std::rc::Rc;

    use nova_vm::{
        ecmascript::{
            AbstractModule, Agent, AgentOptions, ArgumentsList, Behaviour, BuiltinFunctionArgs,
            EmbedderObject, ExceptionType, Function, GcAgent, GraphLoadingStateRecord, HostDefined,
            HostHooks, InternalMethods, Job, JsError, ModuleRequest, Object, PromiseCapability,
            PropertyDescriptor, PropertyKey, Realm, RealmRoot, Referrer, SourceTextModule,
            String as JsString, Value, clear_weak_ref_kept_objects, create_builtin_function,
            finish_loading_imported_module, parse_module, parse_script, script_evaluation,
        },
        engine::{Bindable, GcScope, Global, NoGcScope},
    };

    /// The host module resolver for one `eval_module` call: maps an import
    /// `(specifier, referrer_url)` to the imported module's `(resolved_url, source)`,
    /// or `None` when it cannot be resolved/fetched.
    type ModuleResolver<'a> = dyn FnMut(&str, &str) -> Option<(String, String)> + 'a;

    /// Host hooks that capture promise/generic/timeout jobs into a shared queue the
    /// engine drains in `pump_microtasks`. Nova hands jobs to the host via these
    /// hooks (which take only `&self`), so the queue lives here and is shared with
    /// the engine by `Rc`. Jobs are `'static` (they own rooted handles), so queuing
    /// them across GC is safe.
    struct GenetHostHooks {
        jobs: Rc<RefCell<VecDeque<Job>>>,
        /// Raw pointer to the active module resolver, set for one `eval_module` call
        /// (`None` otherwise). The pointee outlives the call (an `eval_module` arg),
        /// so the deref in `load_imported_module` is sound; single-threaded.
        module_resolver: Cell<Option<*mut ModuleResolver<'static>>>,
        /// Parsed modules by resolved URL — the per-call cache (cleared each call), so
        /// a diamond / cycle resolves each module once. `Global` keeps each rooted
        /// across the load (the heap-global root set, like the reflector cache).
        module_cache: RefCell<HashMap<String, Global<SourceTextModule<'static>>>>,
    }

    // `HostHooks: Debug`, but `Job` is not `Debug`, so report the queue length only.
    impl std::fmt::Debug for GenetHostHooks {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("GenetHostHooks")
                .field("queued", &self.jobs.borrow().len())
                .finish()
        }
    }

    impl GenetHostHooks {
        /// Install `resolver` for the duration of `f`, then clear it and the module
        /// cache (dropping the `Global` roots so they do not leak across calls). The
        /// lifetime is erased to `'static` for storage in the leaked (`'static`)
        /// hooks; it is never observed past `f`, where the real resolver lives.
        fn with_resolver<R>(&self, resolver: &mut ModuleResolver<'_>, f: impl FnOnce() -> R) -> R {
            let raw: *mut ModuleResolver<'_> = resolver;
            // SAFETY: erases only the captured-data lifetime; same layout. Cleared below.
            let erased: *mut ModuleResolver<'static> = unsafe { std::mem::transmute(raw) };
            self.module_resolver.set(Some(erased));
            let out = f();
            self.module_resolver.set(None);
            self.module_cache.borrow_mut().clear();
            out
        }
    }

    impl HostHooks for GenetHostHooks {
        fn enqueue_generic_job(&self, job: Job) {
            self.jobs.borrow_mut().push_back(job);
        }
        fn enqueue_promise_job(&self, job: Job) {
            self.jobs.borrow_mut().push_back(job);
        }
        fn enqueue_timeout_job(&self, job: Job, _milliseconds: u64) {
            self.jobs.borrow_mut().push_back(job);
        }
        fn get_host_data(&self) -> &dyn Any {
            // Unused: genet reaches host state through the realm `[[HostDefined]]`
            // slot, not this hook.
            &()
        }

        fn load_imported_module<'gc>(
            &self,
            agent: &mut Agent,
            referrer: Referrer<'gc>,
            module_request: ModuleRequest<'gc>,
            _host_defined: Option<HostDefined>,
            payload: &mut GraphLoadingStateRecord<'gc>,
            gc: NoGcScope<'gc, '_>,
        ) {
            // The import specifier, and the importing module's URL (its
            // `[[HostDefined]]`, set to the URL string when we parsed it).
            let specifier = module_request
                .specifier(agent)
                .to_string_lossy(agent)
                .into_owned();
            let referrer_url = referrer
                .host_defined(agent)
                .and_then(|hd| hd.downcast::<String>().ok())
                .map(|s| (*s).clone())
                .unwrap_or_default();

            // Resolve + fetch through the host resolver active for this call.
            let resolved = self.module_resolver.get().and_then(|ptr| {
                // SAFETY: set by `with_resolver` for the call's duration; single-threaded.
                let resolve = unsafe { &mut *ptr };
                resolve(&specifier, &referrer_url)
            });

            let result = match resolved {
                Some((url, source)) => {
                    // Cache hit: return the same rooted module (so a diamond loads it
                    // once). `Global` is not `Clone`, so test membership first to drop
                    // the borrow before the `else` branch's `borrow_mut` insert.
                    if self.module_cache.borrow().contains_key(&url) {
                        let cache = self.module_cache.borrow();
                        let global = cache.get(&url).expect("just checked present");
                        Ok(AbstractModule::from(global.get(agent, gc)))
                    } else {
                        let realm = agent.current_realm(gc);
                        let src = JsString::from_string(agent, source, gc);
                        match parse_module(
                            agent,
                            src,
                            realm,
                            Some(Rc::new(url.clone()) as HostDefined),
                            gc,
                        ) {
                            Ok(module) => {
                                self.module_cache
                                    .borrow_mut()
                                    .insert(url, Global::new(agent, module.unbind()));
                                Ok(AbstractModule::from(module))
                            },
                            Err(_) => Err(agent.throw_exception_with_static_message(
                                ExceptionType::SyntaxError,
                                "module parse error",
                                gc,
                            )),
                        }
                    }
                },
                None => Err(agent.throw_exception_with_static_message(
                    ExceptionType::Error,
                    "could not resolve module",
                    gc,
                )),
            };
            finish_loading_imported_module(agent, referrer, module_request, payload, result, gc);
        }
    }

    fn leak_host_hooks(jobs: Rc<RefCell<VecDeque<Job>>>) -> &'static GenetHostHooks {
        Box::leak(Box::new(GenetHostHooks {
            jobs,
            module_resolver: Cell::new(None),
            module_cache: RefCell::new(HashMap::new()),
        }))
    }

    use script_engine_api::{
        Budget, CallCx, HostData, MAIN_REALM, NativeFn, PromiseToken, PumpOutcome, RealmError,
        RealmId, ReflectorData, ScriptEngine, ScriptEngineLive, ScriptEngineSnapshot,
    };

    /// A queue of `Global`s awaiting release. Nova's `Global` has no `Drop` (freeing
    /// needs the `Agent`), so [`NovaValue`]'s `Drop` parks its `Global` here; the
    /// engine drains the queue with the agent at the end of each native call and at
    /// each GC tick.
    type ReleaseQueue = Rc<RefCell<Vec<Global<Value<'static>>>>>;

    /// Nova's host-held value: a rooted [`Global`] plus a handle to the engine's
    /// [`ReleaseQueue`]. Because `Global` has no `Drop`, the generic host/DOM code —
    /// which obtains values via `cx.arg`, `make_string`, … and drops them like any
    /// other `Self::Value` — would otherwise leak a permanent `heap.globals` root per
    /// drop (so every reflector passed as a native-fn argument is pinned forever,
    /// defeating GC reaping on Nova). This wrapper's `Drop` parks the `Global` on the
    /// release queue instead; the engine frees it on the next drain.
    pub struct NovaValue {
        global: Option<Global<Value<'static>>>,
        release: ReleaseQueue,
    }

    impl NovaValue {
        fn new(global: Global<Value<'static>>, release: &ReleaseQueue) -> Self {
            Self {
                global: Some(global),
                release: release.clone(),
            }
        }
        /// Read the rooted value without releasing it.
        fn get(&self, agent: &Agent, gc: NoGcScope) -> Value<'static> {
            self.global.as_ref().expect("live NovaValue").get(agent, gc)
        }
        /// Take the inner `Global` out (the `Drop` then no-ops) so the caller can
        /// `take` it against the agent — e.g. the trampoline handing the result to
        /// the VM, or `settle`/`set_global` reading it.
        fn into_global(mut self) -> Global<Value<'static>> {
            self.global.take().expect("live NovaValue")
        }
    }

    impl Drop for NovaValue {
        fn drop(&mut self) {
            if let Some(g) = self.global.take() {
                self.release.borrow_mut().push(g);
            }
        }
    }

    /// Free every `Global` parked on the release queue (`take` each against the
    /// agent). Called at native-call end and at GC ticks, both of which hold the
    /// `Agent`.
    fn drain_release(agent: &Agent, release: &ReleaseQueue) {
        let drained: Vec<Global<Value<'static>>> = release.borrow_mut().drain(..).collect();
        for g in drained {
            g.take(agent);
        }
    }

    /// Nova's realm `[[HostDefined]]` slot: the neutral [`HostData`] (the DOM, set by
    /// the host) plus the canonical-reflector cache and the pending-host-promise table.
    /// The cached `Global`s (reflectors, and the promise values awaiting settlement)
    /// are permanent roots in `agent.heap.globals`, so they survive collection without
    /// the slot itself being traced. All engine-side, off the neutral wall.
    #[derive(Default)]
    struct RealmRegistry {
        realms: RefCell<HashMap<RealmId, Global<Realm<'static>>>>,
        next_realm: Cell<RealmId>,
    }

    #[derive(Default)]
    struct AgentPromises {
        pending: RefCell<HashMap<u64, Global<Value<'static>>>>,
        next_token: Cell<u64>,
    }

    struct NovaHostSlot {
        registry: Rc<RealmRegistry>,
        /// Which realm this slot belongs to. Nova's `[[HostDefined]]` is already
        /// per-realm, so the slot is where a native sink learns its own realm —
        /// the host arena owns its reflector IDs and native state.
        id: RealmId,
        neutral: RefCell<Option<HostData>>,
        reflectors: RefCell<HashMap<u64, Global<Value<'static>>>>,
        /// Strong roots on reflectors the opaque-root policy holds: a `Global` of
        /// the reflector *itself* (not of a `WeakRef` to it), so it is a heap root
        /// for as long as the entry lives. The reflector, its wrapper-`WeakMap`
        /// entry and every expando on that wrapper survive collection with it.
        roots: RefCell<HashMap<u64, Global<Value<'static>>>>,
        /// `PromiseToken → rooted promise value`. We store only the promise (not the
        /// resolve/reject functions): Nova's `PromiseCapability` is reconstructable
        /// from the promise via `from_promise`, so settling rebuilds the capability and
        /// drives it. `must_be_unresolved` is always `true` (every promise here is
        /// minted by `PromiseCapability::new`).
        promises: Rc<AgentPromises>,
        /// Engine-wide release queue (shared with [`NovaEngine`]). The trampoline
        /// reaches it here — it has only the `Agent`, not the engine — to hand each
        /// [`NovaCallCx`] a handle and to drain the call's dropped temporaries.
        release: ReleaseQueue,
    }

    impl NovaHostSlot {
        fn new(
            id: RealmId,
            release: ReleaseQueue,
            promises: Rc<AgentPromises>,
            registry: Rc<RealmRegistry>,
        ) -> Self {
            Self {
                registry,
                id,
                neutral: RefCell::new(None),
                reflectors: RefCell::new(HashMap::new()),
                roots: RefCell::new(HashMap::new()),
                promises,
                release,
            }
        }
    }

    /// Mint a pending promise, root it, and register it in the realm's host slot.
    /// Shared by the engine-level and in-callback `new_host_promise` (both hold an
    /// `&mut Agent` already inside the realm). Returns the rooted promise value to hand
    /// to JS and the [`PromiseToken`] to settle it later.
    fn mint_and_store(
        agent: &mut Agent,
        gc: NoGcScope,
    ) -> Result<(Global<Value<'static>>, PromiseToken), String> {
        let capability = PromiseCapability::new(agent, gc);
        let promise_value = Value::from(capability.promise()).unbind();
        let returned = Global::new(agent, promise_value);
        let stored = Global::new(agent, promise_value);
        let hd = agent
            .current_realm(gc)
            .host_defined(agent)
            .ok_or_else(|| "host slot missing".to_string())?;
        let slot = hd
            .downcast_ref::<NovaHostSlot>()
            .ok_or_else(|| "host slot wrong type".to_string())?;
        let token = slot.promises.next_token.get();
        slot.promises.next_token.set(token + 1);
        slot.promises.pending.borrow_mut().insert(token, stored);
        Ok((returned, token))
    }

    /// The call context handed to a native callback. Nova's `RegularFn` gives
    /// `(&mut Agent, this, ArgumentsList, GcScope<'gc, 'b>)`; `GcScope` is invariant
    /// in its first lifetime but covariant in the second, so the trampoline collapses
    /// the two onto one (`GcScope<'a, 'a>`) and this context carries a single
    /// lifetime, satisfying the engine-neutral one-lifetime [`CallCx`] GAT.
    pub struct NovaCallCx<'a> {
        exceptions: HashMap<String, Global<JsError<'static>>>,
        this: NovaValue,
        agent: &'a mut Agent,
        gc: GcScope<'a, 'a>,
        args: Vec<Global<Value<'static>>>,
        /// Handle to the engine's release queue, so the [`NovaValue`]s this context
        /// mints (args, intermediates) park their `Global` on drop instead of leaking.
        release: ReleaseQueue,
    }

    impl NovaCallCx<'_> {
        fn reflector_host(&self, realm: RealmId) -> Result<HostDefined, RealmError> {
            let current = self
                .agent
                .current_realm(self.gc.nogc())
                .host_defined(self.agent)
                .ok_or(RealmError::Refused("realm host slot missing"))?;
            let slot = current
                .downcast_ref::<NovaHostSlot>()
                .ok_or(RealmError::Refused("realm host slot missing"))?;
            let target = slot
                .registry
                .realms
                .borrow()
                .get(&realm)
                .ok_or(RealmError::NoSuchRealm(realm))?
                .get(self.agent, self.gc.nogc())
                .unbind();
            target
                .host_defined(self.agent)
                .ok_or(RealmError::Refused("realm host slot missing"))
        }

        // Error is a String in this adapter. Callback-local tokens retain the
        // original exception until the native trampoline returns it to the VM.
        fn preserve_exception(&mut self, error: JsError<'static>) -> String {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let token = format!("native JavaScript exception {id}");
            self.exceptions
                .insert(token.clone(), Global::new(self.agent, error));
            token
        }
    }

    impl Drop for NovaCallCx<'_> {
        fn drop(&mut self) {
            for (_, error) in self.exceptions.drain() {
                error.take(self.agent);
            }
        }
    }

    impl CallCx for NovaCallCx<'_> {
        type Value = NovaValue;
        type Error = String;

        fn error(&mut self, message: &str) -> Self::Error {
            message.to_string()
        }

        fn arg(&mut self, i: usize) -> Self::Value {
            match self.args.get(i) {
                Some(g) => {
                    let v = g.get(self.agent, self.gc.nogc()).unbind();
                    NovaValue::new(Global::new(self.agent, v), &self.release)
                },
                None => NovaValue::new(Global::new(self.agent, Value::Undefined), &self.release),
            }
        }

        fn host_data(&self) -> Option<HostData> {
            let agent: &Agent = self.agent;
            let hd = agent.current_realm(self.gc.nogc()).host_defined(agent)?;
            let slot = hd.downcast_ref::<NovaHostSlot>()?;
            let neutral = slot.neutral.borrow().clone();
            neutral
        }

        fn this_value(&mut self) -> Self::Value {
            let value = self.this.get(self.agent, self.gc.nogc()).unbind();
            NovaValue::new(Global::new(self.agent, value), &self.release)
        }

        fn caller_realm(&mut self) -> RealmId {
            self.agent
                .native_caller_realm(self.gc.nogc())
                .and_then(|realm| realm.host_defined(self.agent))
                .and_then(|hd| hd.downcast_ref::<NovaHostSlot>().map(|slot| slot.id))
                .unwrap_or_else(|| self.current_realm())
        }

        fn current_realm(&mut self) -> RealmId {
            let agent: &Agent = self.agent;
            agent
                .current_realm(self.gc.nogc())
                .host_defined(agent)
                .and_then(|hd| hd.downcast_ref::<NovaHostSlot>().map(|slot| slot.id))
                .unwrap_or(MAIN_REALM)
        }

        fn value_to_string(&mut self, value: &Self::Value) -> Result<String, Self::Error> {
            let v = value.get(self.agent, self.gc.nogc()).unbind();
            match v.to_string(self.agent, self.gc.reborrow()) {
                Ok(s) => Ok(s.to_string_lossy(self.agent).into_owned()),
                Err(_) => Err("toString threw".to_string()),
            }
        }

        fn reflector_data(&mut self, value: &Self::Value) -> Option<ReflectorData> {
            match value.get(self.agent, self.gc.nogc()) {
                Value::EmbedderObject(eo) => Some(eo.embedder_data(self.agent)),
                _ => None,
            }
        }

        fn local_reflector_data(&mut self, value: &Self::Value) -> Option<ReflectorData> {
            let current = u64::from(self.current_realm());
            match value.get(self.agent, self.gc.nogc()) {
                Value::EmbedderObject(eo) => {
                    (eo.embedder_owner(self.agent) == current).then(|| eo.embedder_data(self.agent))
                },
                _ => None,
            }
        }

        fn make_reflector(&mut self, data: ReflectorData) -> Result<Self::Value, Self::Error> {
            // We are mid-call, already inside the realm (the trampoline holds the
            // `Agent`), so build the `EmbedderObject` directly rather than via
            // `run_in_realm` (which can't nest). Mirrors the engine-level
            // `ScriptEngineLive::make_reflector`.
            let owner = self.current_realm();
            let eo = EmbedderObject::create_with_owner(self.agent, data, u64::from(owner));
            Ok(NovaValue::new(
                Global::new(self.agent, Value::EmbedderObject(eo).unbind()),
                &self.release,
            ))
        }

        fn reflector_for(&mut self, data: ReflectorData) -> Result<Self::Value, Self::Error> {
            // The canonical cache holds a `WeakRef` to each reflector (rooted via
            // `Global`, target weak), so a cached reflector pins `===` identity
            // only while script still references it, and reports its death once
            // collected (G1). Extract the cached `WeakRef` value first (it ends
            // the host-slot borrow before we deref through `&mut Agent`).
            let cached: Option<Value> = {
                let agent: &Agent = self.agent;
                agent
                    .current_realm(self.gc.nogc())
                    .host_defined(agent)
                    .and_then(|hd| {
                        hd.downcast_ref::<NovaHostSlot>().and_then(|slot| {
                            slot.reflectors
                                .borrow()
                                .get(&data)
                                .map(|g| g.get(self.agent, self.gc.nogc()).unbind())
                        })
                    })
            };
            // Cache hit *and still alive*: hand back the same embedder object.
            if let Some(Value::WeakRef(weak_ref)) = cached {
                if let Some(eo) = EmbedderObject::from_weak_ref(self.agent, weak_ref) {
                    return Ok(NovaValue::new(
                        Global::new(self.agent, Value::EmbedderObject(eo).unbind()),
                        &self.release,
                    ));
                }
            }
            // Miss/dead: mint the reflector, cache a `WeakRef` to it, return it.
            let owner = self.current_realm();
            let eo = EmbedderObject::create_with_owner(self.agent, data, u64::from(owner));
            let weak_ref = eo.into_weak_ref(self.agent);
            {
                let cached = Global::new(self.agent, Value::WeakRef(weak_ref).unbind());
                let agent: &Agent = self.agent;
                if let Some(hd) = agent.current_realm(self.gc.nogc()).host_defined(agent) {
                    if let Some(slot) = hd.downcast_ref::<NovaHostSlot>() {
                        // Drop any superseded (dead) entry's root before inserting.
                        if let Some(old) = slot.reflectors.borrow_mut().insert(data, cached) {
                            old.take(self.agent);
                        }
                    }
                }
            }
            Ok(NovaValue::new(
                Global::new(self.agent, Value::EmbedderObject(eo).unbind()),
                &self.release,
            ))
        }

        fn reflector_for_in_realm(
            &mut self,
            realm: RealmId,
            data: ReflectorData,
        ) -> Result<Self::Value, RealmError> {
            let host = self.reflector_host(realm)?;
            let slot = host
                .downcast_ref::<NovaHostSlot>()
                .ok_or(RealmError::Refused("realm host slot missing"))?;
            let cached = slot
                .reflectors
                .borrow()
                .get(&data)
                .map(|root| root.get(self.agent, self.gc.nogc()).unbind());
            if let Some(Value::WeakRef(weak)) = cached {
                if let Some(object) = EmbedderObject::from_weak_ref(self.agent, weak) {
                    return Ok(NovaValue::new(
                        Global::new(self.agent, Value::EmbedderObject(object).unbind()),
                        &self.release,
                    ));
                }
            }
            let object = EmbedderObject::create_with_owner(self.agent, data, u64::from(realm));
            let weak = object.into_weak_ref(self.agent);
            let cached = Global::new(self.agent, Value::WeakRef(weak).unbind());
            if let Some(previous) = slot.reflectors.borrow_mut().insert(data, cached) {
                previous.take(self.agent);
            }
            Ok(NovaValue::new(
                Global::new(self.agent, Value::EmbedderObject(object).unbind()),
                &self.release,
            ))
        }

        fn root_reflector_in_realm(
            &mut self,
            realm: RealmId,
            data: ReflectorData,
        ) -> Result<(), RealmError> {
            let value = self.reflector_for_in_realm(realm, data)?;
            let host = self.reflector_host(realm)?;
            let slot = host
                .downcast_ref::<NovaHostSlot>()
                .ok_or(RealmError::Refused("realm host slot missing"))?;
            let value = value.get(self.agent, self.gc.nogc()).unbind();
            let root = Global::new(self.agent, value);
            if let Some(previous) = slot.roots.borrow_mut().insert(data, root) {
                previous.take(self.agent);
            }
            Ok(())
        }

        fn unroot_reflector_in_realm(
            &mut self,
            realm: RealmId,
            data: ReflectorData,
        ) -> Result<(), RealmError> {
            let host = self.reflector_host(realm)?;
            let slot = host
                .downcast_ref::<NovaHostSlot>()
                .ok_or(RealmError::Refused("realm host slot missing"))?;
            if let Some(root) = slot.roots.borrow_mut().remove(&data) {
                root.take(self.agent);
            }
            Ok(())
        }

        fn root_reflector(&mut self, data: ReflectorData) -> bool {
            // Root the canonical reflector *itself* in `agent.heap.globals`, beside
            // (not instead of) the weak cache entry: the cache keeps reporting the
            // identity, the root keeps the object alive.
            let Ok(v) = self.reflector_for(data) else {
                return false;
            };
            let value = v.get(self.agent, self.gc.nogc()).unbind();
            let rooted = Global::new(self.agent, value);
            let mut old = None;
            let mut taken = false;
            {
                let agent: &Agent = self.agent;
                if let Some(hd) = agent.current_realm(self.gc.nogc()).host_defined(agent) {
                    if let Some(slot) = hd.downcast_ref::<NovaHostSlot>() {
                        old = slot.roots.borrow_mut().insert(data, rooted);
                        taken = true;
                    }
                }
            }
            if let Some(old) = old {
                old.take(self.agent);
            }
            if !taken {
                // No slot to hold it: do not leak the root we just minted.
                // (Unreachable in practice — the slot is installed at construction.)
            }
            taken
        }

        fn unroot_reflector(&mut self, data: ReflectorData) {
            let removed = {
                let agent: &Agent = self.agent;
                agent
                    .current_realm(self.gc.nogc())
                    .host_defined(agent)
                    .and_then(|hd| {
                        hd.downcast_ref::<NovaHostSlot>()
                            .and_then(|slot| slot.roots.borrow_mut().remove(&data))
                    })
            };
            if let Some(g) = removed {
                g.take(self.agent);
            }
        }

        fn make_string(&mut self, s: &str) -> Result<Self::Value, Self::Error> {
            let js = JsString::from_str(self.agent, s, self.gc.nogc());
            Ok(NovaValue::new(
                Global::new(self.agent, Value::from(js).unbind()),
                &self.release,
            ))
        }

        fn make_null(&mut self) -> Self::Value {
            NovaValue::new(Global::new(self.agent, Value::Null), &self.release)
        }

        fn undefined(&mut self) -> Self::Value {
            NovaValue::new(Global::new(self.agent, Value::Undefined), &self.release)
        }

        fn new_host_promise(&mut self) -> Result<(Self::Value, PromiseToken), Self::Error> {
            // Mid-call: the trampoline holds the `Agent` and we are already in the
            // realm, so mint directly (mirrors `make_reflector`'s in-call path).
            let (g, token) = mint_and_store(self.agent, self.gc.nogc())?;
            Ok((NovaValue::new(g, &self.release), token))
        }
    }

    /// Bare `fn`-pointer trampoline, monomorphized per `F` (Nova builtins capture
    /// nothing; state arrives via host-defined data + the reflector args). Roots the
    /// arguments, runs `F` against a [`NovaCallCx`], then maps the result back.
    fn nova_trampoline<'gc, F: NativeFn<NovaEngine>>(
        agent: &mut Agent,
        this: Value,
        args: ArgumentsList,
        mut gc: GcScope<'gc, '_>,
    ) -> nova_vm::ecmascript::JsResult<'gc, Value<'gc>> {
        let rooted: Vec<Global<Value<'static>>> = (0..args.len())
            .map(|i| Global::new(agent, args.get(i).unbind()))
            .collect();
        // The engine-wide release queue lives in the realm host slot (the trampoline
        // has only the `Agent`, not the engine). The callee's `cx.arg`/`make_*` values
        // park their `Global` here on drop; we drain it once the call returns.
        let release: ReleaseQueue = {
            let a: &Agent = agent;
            a.current_realm(gc.nogc())
                .host_defined(a)
                .and_then(|hd| {
                    hd.downcast_ref::<NovaHostSlot>()
                        .map(|slot| slot.release.clone())
                })
                .expect("host slot present")
        };
        let this = NovaValue::new(Global::new(agent, this.unbind()), &release);
        let (result, args_to_release, mut exceptions) = {
            let mut cx = NovaCallCx {
                exceptions: HashMap::new(),
                this,
                agent: &mut *agent,
                gc: gc.reborrow(),
                args: rooted,
                release: release.clone(),
            };
            let r = F::call(&mut cx);
            // Reclaim the rooted argument handles (ends the `&mut agent` reborrow).
            let args = std::mem::take(&mut cx.args);
            let exceptions = std::mem::take(&mut cx.exceptions);
            (r, args, exceptions)
        };
        // Release the rooted argument handles: a `Global` has no `Drop`, so
        // dropping them would leak a permanent heap-globals root (and pin any
        // reflector passed as an argument, defeating G1 collection).
        for arg in args_to_release {
            arg.take(agent);
        }
        // Drain the `NovaValue` temporaries the callee dropped (its `cx.arg` copies,
        // intermediate `make_*` values). This is the fix for the reflector leak: the
        // copy that rooted each reflector argument is freed here, so the reflector is
        // no longer pinned. The result value is still held in `result` (not dropped),
        // so it is not in the queue.
        drain_release(agent, &release);
        let thrown = result
            .as_ref()
            .err()
            .and_then(|token| exceptions.remove(token));
        for (_, error) in exceptions {
            error.take(agent);
        }
        if let Some(error) = thrown {
            return Err(error.take(agent).bind(gc.into_nogc()));
        }
        match result {
            Ok(value) => {
                // `into_nogc` carries the full `'gc` lifetime, so the bound value can
                // be returned (unlike a `nogc()` borrow, which is local). `into_global`
                // pulls the `Global` out of the wrapper (its `Drop` then no-ops);
                // `take` (not `get`) frees the return value's root, and the VM stack
                // keeps it alive from here.
                let nogc = gc.into_nogc();
                Ok(value.into_global().take(agent).bind(nogc))
            },
            Err(msg) => Err(agent.throw_exception(ExceptionType::Error, msg, gc.into_nogc())),
        }
    }

    /// A Nova-backed scripting engine (native targets only).
    pub struct NovaEngine {
        agent: GcAgent,
        realm: RealmRoot,
        /// Live realms of this engine, [`MAIN_REALM`] included. One `GcAgent` is
        /// one **agent**: every realm here shares the heap, the job queue and the
        /// host hooks, so a `Global` obtained in one is an ordinary reference in
        /// another. Nova caps an agent at 256 simultaneous realms.
        registry: Rc<RealmRegistry>,
        jobs: Rc<RefCell<VecDeque<Job>>>,
        /// The leaked (`'static`) host hooks installed on `agent`; `eval_module` sets
        /// their module resolver per call.
        hooks: &'static GenetHostHooks,
        /// Release queue for dropped [`NovaValue`]s (shared with the realm host slot).
        /// Drained at each native call (in the trampoline) and at each GC tick.
        release: ReleaseQueue,
    }

    impl NovaEngine {
        /// Clone an idle Nova engine from its current VM heap snapshot.
        ///
        /// The clone gets a fresh host hook queue, release queue, and realm host slot.
        /// Host-owned DOM state is intentionally not copied; callers install it with
        /// `set_host_data` before running script in the clone.
        pub fn snapshot_clone(&mut self) -> Result<Self, String> {
            if self.registry.next_realm.get() != MAIN_REALM + 1 {
                return Err("cannot snapshot clone NovaEngine with multiple realms".to_string());
            }
            if !self.jobs.borrow().is_empty() {
                return Err("cannot snapshot clone NovaEngine with pending jobs".to_string());
            }

            let original_release = self.release.clone();
            self.agent.run_in_realm(&self.realm, |agent, _gc| {
                drain_release(agent, &original_release);
            });

            let jobs: Rc<RefCell<VecDeque<Job>>> = Rc::new(RefCell::new(VecDeque::new()));
            let hooks = leak_host_hooks(jobs.clone());
            let release: ReleaseQueue = Rc::new(RefCell::new(Vec::new()));
            let mut agent = self.agent.snapshot_clone(hooks);
            let registry = Rc::new(RealmRegistry {
                realms: RefCell::new(HashMap::new()),
                next_realm: Cell::new(MAIN_REALM + 1),
            });
            agent.run_in_realm(&self.realm, |a, gc| {
                let main = a.current_realm(gc.nogc()).unbind();
                registry
                    .realms
                    .borrow_mut()
                    .insert(MAIN_REALM, Global::new(a, main));
            });
            let previous = self.realm.replace_host_defined(
                &mut agent,
                Some(Rc::new(NovaHostSlot::new(
                    MAIN_REALM,
                    release.clone(),
                    Rc::new(AgentPromises::default()),
                    registry.clone(),
                ))),
            );
            debug_assert!(
                previous.is_none(),
                "GcAgent::snapshot_clone clears realm host-defined state"
            );

            Ok(Self {
                agent,
                realm: self.realm,
                registry,
                jobs,
                hooks,
                release,
            })
        }
    }

    impl ScriptEngine for NovaEngine {
        // Nova's `Global` has no `Drop`, so the held value type is a `NovaValue`
        // wrapper that parks its `Global` on the release queue when dropped.
        type Value = NovaValue;
        type Error = String;
        type CallCx<'a> = NovaCallCx<'a>;

        fn new() -> Result<Self, Self::Error> {
            // The hooks must be `&'static`; leak one per engine (a few words +
            // shared queue handle). Acceptable for the engine lifetime; the proper
            // fix is a non-'static hooks API upstream.
            let jobs: Rc<RefCell<VecDeque<Job>>> = Rc::new(RefCell::new(VecDeque::new()));
            let hooks = leak_host_hooks(jobs.clone());
            let mut agent = GcAgent::new(AgentOptions::default(), hooks);
            let realm = agent.create_default_realm();
            // One release queue, shared between the engine (drained at GC ticks) and
            // the realm host slot (where the trampoline reaches it per native call).
            let release: ReleaseQueue = Rc::new(RefCell::new(Vec::new()));
            // The realm owns the host slot (neutral DOM + reflector cache) for its
            // whole life; `set_host_data` later fills the neutral half.
            let registry = Rc::new(RealmRegistry {
                realms: RefCell::new(HashMap::new()),
                next_realm: Cell::new(MAIN_REALM + 1),
            });
            agent.run_in_realm(&realm, |a, gc| {
                let main = a.current_realm(gc.nogc()).unbind();
                registry
                    .realms
                    .borrow_mut()
                    .insert(MAIN_REALM, Global::new(a, main));
            });
            realm.initialize_host_defined(
                &mut agent,
                Rc::new(NovaHostSlot::new(
                    MAIN_REALM,
                    release.clone(),
                    Rc::new(AgentPromises::default()),
                    registry.clone(),
                )),
            );

            Ok(Self {
                agent,
                realm,
                registry,
                jobs,
                hooks,
                release,
            })
        }

        fn eval(&mut self, source: &str) -> Result<Self::Value, Self::Error> {
            let src = source.to_string();
            let release = self.release.clone(); // captured before the `&mut self.agent` borrow
            let mut out: Result<NovaValue, String> = Err("eval did not run".to_string());
            self.agent.run_in_realm(&self.realm, |agent, mut gc| {
                let realm = agent.current_realm(gc.nogc());
                let source_text = JsString::from_string(agent, src, gc.nogc());
                let script = match parse_script(agent, source_text, realm, false, None, gc.nogc()) {
                    Ok(script) => script,
                    Err(_) => {
                        out = Err("parse error".to_string());
                        return;
                    },
                };
                // The thrown value borrows the match's `gc`; unbind it out of the
                // match, then stringify with a fresh reborrow (better than an opaque
                // "evaluation threw").
                let thrown = match script_evaluation(agent, script.unbind(), gc.reborrow()) {
                    Ok(value) => {
                        out = Ok(NovaValue::new(Global::new(agent, value.unbind()), &release));
                        None
                    },
                    Err(err) => Some(err.value().unbind()),
                };
                if let Some(v) = thrown {
                    let msg = v
                        .to_string(agent, gc.reborrow())
                        .map(|s| s.to_string_lossy(agent).into_owned())
                        .unwrap_or_else(|_| "<unprintable>".to_string());
                    out = Err(format!("evaluation threw: {msg}"));
                }
            });
            out
        }

        fn call_function(
            &mut self,
            function: &Self::Value,
            this: &Self::Value,
            args: &[Self::Value],
        ) -> Result<Self::Value, RealmError> {
            let release = self.release.clone();
            let mut out = Err(RealmError::Refused("host call did not run"));
            self.agent.run_in_realm(&self.realm, |agent, mut gc| {
                let Ok(function) = Function::try_from(function.get(agent, gc.nogc()).unbind())
                else {
                    out = Err(RealmError::Engine("value is not callable".into()));
                    return;
                };
                let this = this.get(agent, gc.nogc()).unbind();
                let mut args: Vec<Value> = args
                    .iter()
                    .map(|value| value.get(agent, gc.nogc()).unbind())
                    .collect();
                let result = function
                    .call(agent, this, &mut args, gc.reborrow())
                    .unbind();
                out = match result {
                    Ok(value) => Ok(NovaValue::new(Global::new(agent, value), &release)),
                    Err(error) => {
                        let message = error
                            .value()
                            .unbind()
                            .to_string(agent, gc.reborrow())
                            .map(|value| value.to_string_lossy(agent).into_owned())
                            .unwrap_or_else(|_| "<unprintable>".into());
                        Err(RealmError::Engine(message))
                    },
                };
            });
            out
        }

        fn eval_module(
            &mut self,
            source: &str,
            base_url: &str,
            resolve: &mut dyn FnMut(&str, &str) -> Option<(String, String)>,
        ) -> Result<Option<Self::Value>, Self::Error> {
            let hooks = self.hooks; // `&'static`, Copy — does not borrow `self`.
            let release = self.release.clone(); // captured before the `&mut self.agent` borrow
            let src = source.to_string();
            let base = base_url.to_string();
            let mut out: Result<Option<NovaValue>, String> =
                Err("eval_module did not run".to_string());
            // The resolver is installed for this call so `load_imported_module` can
            // fetch imports; the entry's `[[HostDefined]]` is `base_url`, the base its
            // imports resolve against. `run_module` drives load → link → evaluate
            // (synchronously, since the resolver fetches synchronously).
            hooks.with_resolver(resolve, || {
                self.agent.run_in_realm(&self.realm, |agent, mut gc| {
                    let realm = agent.current_realm(gc.nogc());
                    let source_text = JsString::from_string(agent, src, gc.nogc());
                    let module = match parse_module(
                        agent,
                        source_text,
                        realm,
                        Some(Rc::new(base) as HostDefined),
                        gc.nogc(),
                    ) {
                        Ok(m) => m,
                        Err(_) => {
                            out = Err("module parse error".to_string());
                            return;
                        },
                    };
                    match agent.run_module(module.unbind(), None, gc.reborrow()) {
                        Ok(value) => {
                            out = Ok(Some(NovaValue::new(
                                Global::new(agent, value.unbind()),
                                &release,
                            )))
                        },
                        Err(err) => {
                            let v = err.value().unbind();
                            let msg = v
                                .to_string(agent, gc.reborrow())
                                .map(|s| s.to_string_lossy(agent).into_owned())
                                .unwrap_or_else(|_| "<unprintable>".to_string());
                            out = Err(format!("module threw: {msg}"));
                        },
                    }
                });
            });
            out
        }

        fn describe_error(&mut self, error: &Self::Error) -> String {
            // Nova's `Error` is already the thrown value's message (e.g. "evaluation
            // threw: TypeError: …" or "parse error").
            error.clone()
        }

        fn value_to_string(&mut self, value: &Self::Value) -> Result<String, Self::Error> {
            let mut out = Err("value_to_string did not run".to_string());
            self.agent.run_in_realm(&self.realm, |agent, gc| {
                let v = value.get(agent, gc.nogc()).unbind();
                match v.to_string(agent, gc) {
                    Ok(s) => out = Ok(s.to_string_lossy(agent).into_owned()),
                    Err(_) => out = Err("toString threw".to_string()),
                }
            });
            out
        }

        fn set_global(&mut self, name: &str, value: &Self::Value) -> Result<(), Self::Error> {
            let name = name.to_string();
            let mut out = Err("set_global did not run".to_string());
            self.agent.run_in_realm(&self.realm, |agent, mut gc| {
                let global = agent.current_realm(gc.nogc()).global_object(agent).unbind();
                let key = PropertyKey::from_str(agent, &name, gc.nogc()).unbind();
                let v = value.get(agent, gc.nogc()).unbind();
                let desc = PropertyDescriptor {
                    value: Some(v),
                    writable: Some(true),
                    enumerable: Some(true),
                    configurable: Some(true),
                    ..Default::default()
                };
                match global.internal_define_own_property(agent, key, desc, gc.reborrow()) {
                    Ok(_) => out = Ok(()),
                    Err(_) => out = Err("define_own_property threw".to_string()),
                }
            });
            out
        }

        fn set_host_data(&mut self, data: HostData) {
            // Fill the neutral half of the realm's host slot (initialized in `new`).
            self.agent.run_in_realm(&self.realm, |agent, gc| {
                if let Some(hd) = agent.current_realm(gc.nogc()).host_defined(agent) {
                    if let Some(slot) = hd.downcast_ref::<NovaHostSlot>() {
                        *slot.neutral.borrow_mut() = Some(data);
                    }
                }
            });
        }

        fn set_function<F: NativeFn<Self>>(
            &mut self,
            name: &str,
            length: usize,
        ) -> Result<(), Self::Error> {
            // Nova wants a `&'static str` function name; builtins are registered a
            // bounded number of times at setup, so leaking is acceptable here.
            let name: &'static str = Box::leak(name.to_string().into_boxed_str());
            let mut out = Err("set_function did not run".to_string());
            self.agent.run_in_realm(&self.realm, |agent, mut gc| {
                let func = create_builtin_function(
                    agent,
                    Behaviour::Regular(nova_trampoline::<F>),
                    BuiltinFunctionArgs::new(length as u32, name),
                    gc.nogc(),
                );
                let global = agent.current_realm(gc.nogc()).global_object(agent).unbind();
                let key = PropertyKey::from_str(agent, &name, gc.nogc()).unbind();
                let desc = PropertyDescriptor {
                    value: Some(func.unbind().into()),
                    ..Default::default()
                };
                match global.internal_define_own_property(agent, key, desc, gc.reborrow()) {
                    Ok(_) => out = Ok(()),
                    Err(_) => out = Err("define_own_property threw".to_string()),
                }
            });
            out
        }

        fn pump(&mut self, budget: Budget) -> PumpOutcome {
            // A job may enqueue more (chained `.then`), so loop. Each job consumes
            // itself in `run`. `Steps(n)` bounds by job count (coarse: one job is one
            // step); when the budget is spent we return `Pending` iff the queue still
            // has work, so a caller can detect a runaway script and stop pumping.
            let mut remaining = match budget {
                Budget::Unbounded => None,
                Budget::Steps(n) => Some(n),
            };
            let outcome = loop {
                if remaining == Some(0) {
                    break if self.jobs.borrow().is_empty() {
                        PumpOutcome::Quiescent
                    } else {
                        PumpOutcome::Pending
                    };
                }
                let Some(job) = self.jobs.borrow_mut().pop_front() else {
                    break PumpOutcome::Quiescent;
                };
                self.agent.run_in_realm(&self.realm, |agent, gc| {
                    let _ = job.run(agent, gc);
                });
                if let Some(r) = remaining.as_mut() {
                    *r -= 1;
                }
            };
            // Microtask checkpoint complete: ClearKeptObjects (spec 9.10), so
            // reflectors only observed through the weak canonical cache since the
            // last pump become collectable again. This is also what makes
            // `drain_dead_reflectors` able to ever observe a death. Also drain any
            // host-held `NovaValue`s dropped between calls (the per-call trampoline
            // drain only covers within-call temporaries).
            let release = self.release.clone();
            self.agent.run_in_realm(&self.realm, |agent, _gc| {
                clear_weak_ref_kept_objects(agent);
                drain_release(agent, &release);
            });
            outcome
        }

        fn new_host_promise(&mut self) -> Result<(Self::Value, PromiseToken), Self::Error> {
            let release = self.release.clone();
            let mut out: Result<(NovaValue, PromiseToken), String> =
                Err("new_host_promise did not run".to_string());
            self.agent.run_in_realm(&self.realm, |agent, gc| {
                out =
                    mint_and_store(agent, gc.nogc()).map(|(g, t)| (NovaValue::new(g, &release), t));
            });
            out
        }

        fn settle_host_promise(
            &mut self,
            token: PromiseToken,
            outcome: Result<&Self::Value, &Self::Value>,
        ) -> Result<(), Self::Error> {
            self.agent.run_in_realm(&self.realm, |agent, gc| {
                // Consume the token: pull the rooted promise out of the slot. An
                // unknown/already-settled token leaves nothing to do.
                let stored = {
                    let Some(hd) = agent.current_realm(gc.nogc()).host_defined(agent) else {
                        return;
                    };
                    let Some(slot) = hd.downcast_ref::<NovaHostSlot>() else {
                        return;
                    };
                    let removed = slot.promises.pending.borrow_mut().remove(&token);
                    removed
                };
                let Some(stored) = stored else { return };
                let Value::Promise(promise) = stored.get(agent, gc.nogc()).unbind() else {
                    return;
                };
                let capability = PromiseCapability::from_promise(promise, true);
                // Resolving enqueues the reaction jobs (Nova hands them to our
                // `HostHooks`); the caller drains them via `pump_microtasks`.
                match outcome {
                    Ok(value) => {
                        let value = value.get(agent, gc.nogc()).unbind();
                        capability.resolve(agent, value, gc);
                    },
                    Err(error) => {
                        let error = error.get(agent, gc.nogc()).unbind();
                        capability.reject(agent, error, gc.nogc());
                    },
                }
            });
            Ok(())
        }

        fn force_gc(&mut self) {
            // Free any host-held `NovaValue`s dropped since the last drain *before*
            // collecting, so their reflector roots are gone and the targets become
            // collectable this tick.
            let release = self.release.clone();
            self.agent.run_in_realm(&self.realm, |agent, _gc| {
                drain_release(agent, &release);
            });
            // Two passes: Nova finalizes the weak references whose targets the first
            // cycle reclaimed on the second, so a just-dropped reflector becomes
            // observable to `drain_dead_reflectors` — the engine half of the GC tick.
            self.agent.gc();
            self.agent.gc();
        }

        fn minted_reflectors(&mut self) -> Vec<ReflectorData> {
            self.minted_reflectors_in_realm(MAIN_REALM)
                .expect("main realm exists")
        }

        fn minted_reflectors_in_realm(
            &mut self,
            realm: RealmId,
        ) -> Result<Vec<ReflectorData>, RealmError> {
            if !self.registry.realms.borrow().contains_key(&realm) {
                return Err(RealmError::NoSuchRealm(realm));
            }
            let registry = self.registry.clone();
            let out = (|| {
                let mut out = Vec::new();
                self.agent.run_in_realm(&self.realm, |agent, gc| {
                    let target = registry
                        .realms
                        .borrow()
                        .get(&realm)
                        .expect("checked realm")
                        .get(agent, gc.nogc())
                        .unbind();
                    if let Some(hd) = target.bind(gc.nogc()).host_defined(agent) {
                        if let Some(slot) = hd.downcast_ref::<NovaHostSlot>() {
                            out = slot.reflectors.borrow().keys().copied().collect();
                        }
                    }
                });
                out
            })();
            Ok(out)
        }

        fn root_reflectors(&mut self, data: &[ReflectorData]) -> () {
            self.root_reflectors_in_realm(MAIN_REALM, data)
                .expect("main realm exists")
        }

        fn root_reflectors_in_realm(
            &mut self,
            realm: RealmId,
            data: &[ReflectorData],
        ) -> Result<(), RealmError> {
            if !self.registry.realms.borrow().contains_key(&realm) {
                return Err(RealmError::NoSuchRealm(realm));
            }
            let registry = self.registry.clone();
            let out = (|| {
                let release = self.release.clone();
                self.agent.run_in_realm(&self.realm, |agent, mut gc| {
                    let target = registry
                        .realms
                        .borrow()
                        .get(&realm)
                        .expect("checked realm")
                        .get(agent, gc.nogc())
                        .unbind();
                    for &d in data {
                        // Resolve through the canonical cache so a root and a
                        // `reflector_for` hit are the same object; a dead or missing
                        // entry is re-minted, which is the right outcome (the node is
                        // reachable again).
                        let cached: Option<Value> = {
                            let a: &Agent = agent;
                            target.bind(gc.nogc()).host_defined(a).and_then(|hd| {
                                hd.downcast_ref::<NovaHostSlot>().and_then(|slot| {
                                    slot.reflectors
                                        .borrow()
                                        .get(&d)
                                        .map(|g| g.get(agent, gc.nogc()).unbind())
                                })
                            })
                        };
                        let eo = match cached {
                            Some(Value::WeakRef(weak_ref)) => {
                                EmbedderObject::from_weak_ref(agent, weak_ref)
                            },
                            _ => None,
                        };
                        let eo = match eo {
                            Some(eo) => eo,
                            None => {
                                let eo =
                                    EmbedderObject::create_with_owner(agent, d, u64::from(realm));
                                let weak_ref = eo.into_weak_ref(agent);
                                let cached = Global::new(agent, Value::WeakRef(weak_ref).unbind());
                                let mut old = None;
                                {
                                    let a: &Agent = agent;
                                    if let Some(hd) = target.bind(gc.nogc()).host_defined(a) {
                                        if let Some(slot) = hd.downcast_ref::<NovaHostSlot>() {
                                            old = slot.reflectors.borrow_mut().insert(d, cached);
                                        }
                                    }
                                }
                                if let Some(old) = old {
                                    old.take(agent);
                                }
                                eo
                            },
                        };
                        let rooted = Global::new(agent, Value::EmbedderObject(eo).unbind());
                        let mut old = None;
                        {
                            let a: &Agent = agent;
                            if let Some(hd) = target.bind(gc.nogc()).host_defined(a) {
                                if let Some(slot) = hd.downcast_ref::<NovaHostSlot>() {
                                    old = slot.roots.borrow_mut().insert(d, rooted);
                                }
                            }
                        }
                        if let Some(old) = old {
                            old.take(agent);
                        }
                        let _ = &mut gc;
                    }
                    drain_release(agent, &release);
                });
            })();
            Ok(out)
        }

        fn unroot_reflectors(&mut self, data: &[ReflectorData]) -> () {
            self.unroot_reflectors_in_realm(MAIN_REALM, data)
                .expect("main realm exists")
        }

        fn unroot_reflectors_in_realm(
            &mut self,
            realm: RealmId,
            data: &[ReflectorData],
        ) -> Result<(), RealmError> {
            if !self.registry.realms.borrow().contains_key(&realm) {
                return Err(RealmError::NoSuchRealm(realm));
            }
            let registry = self.registry.clone();
            let out = (|| {
                self.agent.run_in_realm(&self.realm, |agent, gc| {
                    let target = registry
                        .realms
                        .borrow()
                        .get(&realm)
                        .expect("checked realm")
                        .get(agent, gc.nogc())
                        .unbind();
                    let mut removed = Vec::new();
                    {
                        let a: &Agent = agent;
                        if let Some(hd) = target.bind(gc.nogc()).host_defined(a) {
                            if let Some(slot) = hd.downcast_ref::<NovaHostSlot>() {
                                let mut roots = slot.roots.borrow_mut();
                                for d in data {
                                    if let Some(g) = roots.remove(d) {
                                        removed.push(g);
                                    }
                                }
                            }
                        }
                    }
                    for g in removed {
                        g.take(agent);
                    }
                });
            })();
            Ok(out)
        }

        fn rooted_reflector_count(&mut self) -> usize {
            self.rooted_reflector_count_in_realm(MAIN_REALM)
                .expect("main realm exists")
        }

        fn rooted_reflector_count_in_realm(&mut self, realm: RealmId) -> Result<usize, RealmError> {
            if !self.registry.realms.borrow().contains_key(&realm) {
                return Err(RealmError::NoSuchRealm(realm));
            }
            let registry = self.registry.clone();
            let out = (|| {
                let mut n = 0;
                self.agent.run_in_realm(&self.realm, |agent, gc| {
                    let target = registry
                        .realms
                        .borrow()
                        .get(&realm)
                        .expect("checked realm")
                        .get(agent, gc.nogc())
                        .unbind();
                    if let Some(hd) = target.bind(gc.nogc()).host_defined(agent) {
                        if let Some(slot) = hd.downcast_ref::<NovaHostSlot>() {
                            n = slot.roots.borrow().len();
                        }
                    }
                });
                n
            })();
            Ok(out)
        }

        fn drain_dead_reflectors(&mut self) -> Vec<ReflectorData> {
            self.drain_dead_reflectors_in_realm(MAIN_REALM)
                .expect("main realm exists")
        }

        fn drain_dead_reflectors_in_realm(
            &mut self,
            realm: RealmId,
        ) -> Result<Vec<ReflectorData>, RealmError> {
            if !self.registry.realms.borrow().contains_key(&realm) {
                return Err(RealmError::NoSuchRealm(realm));
            }
            let registry = self.registry.clone();
            let out = (|| {
                // Real death-reporting: deref each cached `WeakRef`; a target that
                // has been collected (deref → `None`) is a dead reflector. Backed by
                // the vendored `EmbedderObject::into_weak_ref`/`from_weak_ref` patch.
                // The host unpins each returned id, freeing the detached node (G3).
                let mut dead = Vec::new();
                self.agent.run_in_realm(&self.realm, |agent, gc| {
                    let target = registry
                        .realms
                        .borrow()
                        .get(&realm)
                        .expect("checked realm")
                        .get(agent, gc.nogc())
                        .unbind();
                    // Snapshot the cached (id, WeakRef value) pairs, ending the
                    // host-slot borrow before derefing through `&mut Agent`.
                    let entries: Vec<(u64, Value)> = {
                        let Some(hd) = target.bind(gc.nogc()).host_defined(agent) else {
                            return;
                        };
                        let Some(slot) = hd.downcast_ref::<NovaHostSlot>() else {
                            return;
                        };
                        let collected: Vec<(u64, Value)> = slot
                            .reflectors
                            .borrow()
                            .iter()
                            .map(|(&d, g)| (d, g.get(agent, gc.nogc()).unbind()))
                            .collect();
                        collected
                    };
                    for (d, value) in entries {
                        let alive = matches!(value, Value::WeakRef(weak_ref)
                        if EmbedderObject::from_weak_ref(agent, weak_ref).is_some());
                        if !alive {
                            dead.push(d);
                        }
                    }
                    if !dead.is_empty() {
                        let Some(hd) = target.bind(gc.nogc()).host_defined(agent) else {
                            return;
                        };
                        let Some(slot) = hd.downcast_ref::<NovaHostSlot>() else {
                            return;
                        };
                        let mut map = slot.reflectors.borrow_mut();
                        for d in &dead {
                            if let Some(g) = map.remove(d) {
                                // Release the `WeakRef`'s root now that it is dead.
                                g.take(agent);
                            }
                        }
                    }
                });
                dead
            })();
            Ok(out)
        }

        // ---- Realms --------------------------------------------------------
        //
        // Nova gives an embedder the whole shape: `GcAgent::create_default_realm`
        // adds a realm to the *same* agent (heap, job queue, host hooks shared),
        // `run_in_realm` runs a closure inside a chosen one, and
        // `[[HostDefined]]` is already per-realm — which is why per-realm host
        // state costs nothing here.
        //
        // One refusal is real and recorded rather than approximated:
        // `GcAgent::run_in_realm` asserts the execution-context stack is empty,
        // so it **cannot nest**. A native callback running in realm A therefore
        // cannot ask the host to enter realm B mid-call. It does not need to:
        // values are agent-wide, so reading and calling another realm's objects
        // from inside a call goes through the ordinary object protocol, which
        // pushes the callee's realm itself. Only a *host-driven* realm switch is
        // refused, and only while a call is on the stack.

        fn create_realm_from_call(
            cx: &mut Self::CallCx<'_>,
            data: HostData,
            initialize: impl for<'a> FnOnce(&mut Self::CallCx<'a>) -> Result<(), RealmError>,
        ) -> Result<RealmId, RealmError> {
            let hd = cx
                .agent
                .current_realm(cx.gc.nogc())
                .host_defined(cx.agent)
                .ok_or(RealmError::Refused("realm carries no host slot"))?;
            let slot = hd
                .downcast_ref::<NovaHostSlot>()
                .ok_or(RealmError::Refused("realm carries no host slot"))?;
            let registry = slot.registry.clone();
            let promises = slot.promises.clone();
            let release = cx.release.clone();
            let id = registry.next_realm.get();
            registry.next_realm.set(
                id.checked_add(1)
                    .ok_or(RealmError::Refused("realm ids exhausted"))?,
            );
            let mut result = Ok(());
            let create_global: Option<for<'a> fn(&mut Agent, GcScope<'a, '_>) -> Object<'a>> = None;
            let create_this: Option<for<'a> fn(&mut Agent, GcScope<'a, '_>) -> Object<'a>> = None;
            cx.agent.create_realm(
                create_global,
                create_this,
                Some(|agent: &mut Agent, _global: Object, gc: GcScope| {
                    let realm = agent.current_realm(gc.nogc()).unbind();
                    let slot = NovaHostSlot::new(id, release.clone(), promises, registry.clone());
                    *slot.neutral.borrow_mut() = Some(data);
                    realm.initialize_host_defined(agent, Rc::new(slot));
                    registry
                        .realms
                        .borrow_mut()
                        .insert(id, Global::new(agent, realm));
                    let this = NovaValue::new(Global::new(agent, Value::Undefined), &release);
                    let mut child = NovaCallCx {
                        exceptions: HashMap::new(),
                        this,
                        agent,
                        gc,
                        args: Vec::new(),
                        release,
                    };
                    result = initialize(&mut child);
                }),
                cx.gc.reborrow(),
            );
            if result.is_err() {
                if let Some(root) = registry.realms.borrow_mut().remove(&id) {
                    root.take(cx.agent);
                }
            }
            result.map(|()| id)
        }

        fn eval_from_call(
            cx: &mut Self::CallCx<'_>,
            source: &str,
        ) -> Result<Self::Value, RealmError> {
            let source = JsString::from_str(cx.agent, source, cx.gc.nogc()).unbind();
            let result = cx.agent.run_script(source, cx.gc.reborrow());
            match result {
                Ok(value) => Ok(NovaValue::new(
                    Global::new(cx.agent, value.unbind()),
                    &cx.release,
                )),
                Err(error) => Err(RealmError::Engine(format!(
                    "evaluation threw: {:?}",
                    error.value()
                ))),
            }
        }

        fn set_function_from_call<F: NativeFn<Self>>(
            cx: &mut Self::CallCx<'_>,
            name: &str,
            length: usize,
        ) -> Result<(), RealmError> {
            let name: &'static str = Box::leak(name.to_string().into_boxed_str());
            let func = create_builtin_function(
                cx.agent,
                Behaviour::Regular(nova_trampoline::<F>),
                BuiltinFunctionArgs::new(length as u32, name),
                cx.gc.nogc(),
            );
            let global = cx
                .agent
                .current_realm(cx.gc.nogc())
                .global_object(cx.agent)
                .unbind();
            let key = PropertyKey::from_str(cx.agent, name, cx.gc.nogc()).unbind();
            let desc = PropertyDescriptor {
                value: Some(func.unbind().into()),
                ..Default::default()
            };
            match global.internal_define_own_property(cx.agent, key, desc, cx.gc.reborrow()) {
                Ok(true) => Ok(()),
                _ => Err(RealmError::Engine(
                    "native function installation refused".into(),
                )),
            }
        }

        fn eval_in_realm_from_call(
            cx: &mut Self::CallCx<'_>,
            realm: RealmId,
            source: &str,
        ) -> Result<Self::Value, Self::Error> {
            let hd = cx
                .agent
                .current_realm(cx.gc.nogc())
                .host_defined(cx.agent)
                .ok_or_else(|| "realm carries no host slot".to_string())?;
            let slot = hd
                .downcast_ref::<NovaHostSlot>()
                .ok_or_else(|| "realm carries no host slot".to_string())?;
            let target = slot
                .registry
                .realms
                .borrow()
                .get(&realm)
                .ok_or_else(|| format!("no such realm: {realm}"))?
                .get(cx.agent, cx.gc.nogc())
                .unbind();
            let source = JsString::from_str(cx.agent, source, cx.gc.nogc());
            let script = parse_script(
                cx.agent,
                source,
                target.bind(cx.gc.nogc()),
                false,
                None,
                cx.gc.nogc(),
            )
            .map(|script| script.unbind())
            .map_err(|_| ());
            let script = match script {
                Ok(script) => script,
                Err(()) => {
                    let error = cx
                        .agent
                        .throw_exception_with_static_message(
                            ExceptionType::SyntaxError,
                            "parse error",
                            cx.gc.nogc(),
                        )
                        .unbind();
                    return Err(cx.preserve_exception(error));
                },
            };
            let result = script_evaluation(cx.agent, script, cx.gc.reborrow()).unbind();
            match result {
                Ok(value) => Ok(NovaValue::new(
                    Global::new(cx.agent, value.unbind()),
                    &cx.release,
                )),
                Err(error) => Err(cx.preserve_exception(error.unbind())),
            }
        }

        fn call_from_call(
            cx: &mut Self::CallCx<'_>,
            function: &Self::Value,
            this: &Self::Value,
            args: &[Self::Value],
        ) -> Result<Self::Value, Self::Error> {
            let function = Function::try_from(function.get(cx.agent, cx.gc.nogc()).unbind())
                .map_err(|_| "value is not a callable function".to_string())?;
            let this = this.get(cx.agent, cx.gc.nogc()).unbind();
            let mut args: Vec<Value> = args
                .iter()
                .map(|v| v.get(cx.agent, cx.gc.nogc()).unbind())
                .collect();
            let result = function
                .call(cx.agent, this, &mut args, cx.gc.reborrow())
                .unbind();
            match result {
                Ok(value) => Ok(NovaValue::new(
                    Global::new(cx.agent, value.unbind()),
                    &cx.release,
                )),
                Err(error) => Err(cx.preserve_exception(error.unbind())),
            }
        }

        fn realm_global_from_call(
            cx: &mut Self::CallCx<'_>,
            realm: RealmId,
        ) -> Result<Self::Value, RealmError> {
            let hd = cx
                .agent
                .current_realm(cx.gc.nogc())
                .host_defined(cx.agent)
                .ok_or(RealmError::Refused("realm carries no host slot"))?;
            let slot = hd
                .downcast_ref::<NovaHostSlot>()
                .ok_or(RealmError::Refused("realm carries no host slot"))?;
            let target = slot
                .registry
                .realms
                .borrow()
                .get(&realm)
                .ok_or(RealmError::NoSuchRealm(realm))?
                .get(cx.agent, cx.gc.nogc())
                .unbind();
            let global = target.global_object(cx.agent).unbind();
            Ok(NovaValue::new(
                Global::new(cx.agent, Value::from(global)),
                &cx.release,
            ))
        }

        fn set_global_from_call(
            cx: &mut Self::CallCx<'_>,
            name: &str,
            value: &Self::Value,
        ) -> Result<(), RealmError> {
            let global = cx
                .agent
                .current_realm(cx.gc.nogc())
                .global_object(cx.agent)
                .unbind();
            let key = PropertyKey::from_str(cx.agent, name, cx.gc.nogc()).unbind();
            let value = value.get(cx.agent, cx.gc.nogc()).unbind();
            let desc = PropertyDescriptor {
                value: Some(value),
                writable: Some(true),
                enumerable: Some(true),
                configurable: Some(true),
                ..Default::default()
            };
            match global.internal_define_own_property(cx.agent, key, desc, cx.gc.reborrow()) {
                Ok(true) => Ok(()),
                _ => Err(RealmError::Engine("global installation refused".into())),
            }
        }

        fn supports_realms(&self) -> bool {
            true
        }

        fn create_realm(&mut self) -> Result<RealmId, RealmError> {
            let release = self.release.clone();
            let mut out = Err(RealmError::Engine("create realm did not run".into()));
            self.agent.run_in_realm(&self.realm, |agent, gc| {
                let this = NovaValue::new(Global::new(agent, Value::Undefined), &release);
                let mut cx = NovaCallCx {
                    exceptions: HashMap::new(),
                    this,
                    agent,
                    gc,
                    args: Vec::new(),
                    release,
                };
                out = Self::create_realm_from_call(&mut cx, Rc::new(()), |child| {
                    if let Some(hd) = child
                        .agent
                        .current_realm(child.gc.nogc())
                        .host_defined(child.agent)
                    {
                        if let Some(slot) = hd.downcast_ref::<NovaHostSlot>() {
                            *slot.neutral.borrow_mut() = None;
                        }
                    }
                    Ok(())
                });
            });
            out
        }

        fn discard_realm(&mut self, realm: RealmId) -> Result<(), RealmError> {
            if realm == MAIN_REALM {
                return Err(RealmError::Refused("the main realm cannot be discarded"));
            }
            let root = self
                .registry
                .realms
                .borrow_mut()
                .remove(&realm)
                .ok_or(RealmError::NoSuchRealm(realm))?;
            self.agent.run_in_realm(&self.realm, |agent, _gc| {
                root.take(agent);
            });
            Ok(())
        }

        fn eval_in_realm(
            &mut self,
            realm: RealmId,
            source: &str,
        ) -> Result<Self::Value, RealmError> {
            if !self.registry.realms.borrow().contains_key(&realm) {
                return Err(RealmError::NoSuchRealm(realm));
            }
            let registry = self.registry.clone();
            let src = source.to_string();
            let release = self.release.clone();
            let mut out: Result<NovaValue, RealmError> =
                Err(RealmError::Engine("eval_in_realm did not run".to_string()));
            self.agent.run_in_realm(&self.realm, |agent, mut gc| {
                let target = registry
                    .realms
                    .borrow()
                    .get(&realm)
                    .expect("checked realm")
                    .get(agent, gc.nogc())
                    .unbind();
                let current = target.bind(gc.nogc());
                let source_text = JsString::from_string(agent, src, gc.nogc());
                let script = match parse_script(agent, source_text, current, false, None, gc.nogc())
                {
                    Ok(script) => script,
                    Err(_) => {
                        out = Err(RealmError::Engine("parse error".to_string()));
                        return;
                    },
                };
                // Same two-step as `eval`: the thrown value borrows the match's
                // `gc`, so unbind it out of the match and stringify with a fresh
                // reborrow rather than reporting an opaque "evaluation threw".
                let thrown = match script_evaluation(agent, script.unbind(), gc.reborrow()) {
                    Ok(value) => {
                        let g = Global::new(agent, value.unbind());
                        out = Ok(NovaValue::new(g, &release));
                        None
                    },
                    Err(err) => Some(err.value().unbind()),
                };
                if let Some(v) = thrown {
                    let msg = v
                        .to_string(agent, gc.reborrow())
                        .map(|s| s.to_string_lossy(agent).into_owned())
                        .unwrap_or_else(|_| "<unprintable>".to_string());
                    out = Err(RealmError::Engine(format!("evaluation threw: {msg}")));
                }
            });
            out
        }

        fn realm_global(&mut self, realm: RealmId) -> Result<Self::Value, RealmError> {
            if !self.registry.realms.borrow().contains_key(&realm) {
                return Err(RealmError::NoSuchRealm(realm));
            }
            let registry = self.registry.clone();
            let release = self.release.clone();
            let mut out: Result<NovaValue, RealmError> =
                Err(RealmError::Engine("realm_global did not run".to_string()));
            self.agent.run_in_realm(&self.realm, |agent, gc| {
                let target = registry
                    .realms
                    .borrow()
                    .get(&realm)
                    .expect("checked realm")
                    .get(agent, gc.nogc())
                    .unbind();
                let global = target.bind(gc.nogc()).global_object(agent).unbind();
                let g = Global::new(agent, Value::from(global).unbind());
                out = Ok(NovaValue::new(g, &release));
            });
            out
        }

        fn set_global_in_realm(
            &mut self,
            realm: RealmId,
            name: &str,
            value: &Self::Value,
        ) -> Result<(), RealmError> {
            if !self.registry.realms.borrow().contains_key(&realm) {
                return Err(RealmError::NoSuchRealm(realm));
            }
            let registry = self.registry.clone();
            let name = name.to_string();
            let mut out = Err(RealmError::Engine(
                "set_global_in_realm did not run".to_string(),
            ));
            self.agent.run_in_realm(&self.realm, |agent, mut gc| {
                let target = registry
                    .realms
                    .borrow()
                    .get(&realm)
                    .expect("checked realm")
                    .get(agent, gc.nogc())
                    .unbind();
                let global = target.bind(gc.nogc()).global_object(agent).unbind();
                let key = PropertyKey::from_str(agent, &name, gc.nogc()).unbind();
                let v = value.get(agent, gc.nogc()).unbind();
                let desc = PropertyDescriptor {
                    value: Some(v),
                    writable: Some(true),
                    enumerable: Some(true),
                    configurable: Some(true),
                    ..Default::default()
                };
                out = match global.internal_define_own_property(agent, key, desc, gc.reborrow()) {
                    Ok(_) => Ok(()),
                    Err(_) => Err(RealmError::Engine("define_own_property threw".to_string())),
                };
            });
            out
        }

        fn set_function_in_realm<F: NativeFn<Self>>(
            &mut self,
            realm: RealmId,
            name: &str,
            length: usize,
        ) -> Result<(), RealmError> {
            if !self.registry.realms.borrow().contains_key(&realm) {
                return Err(RealmError::NoSuchRealm(realm));
            }
            let registry = self.registry.clone();
            // Nova wants a `&'static str` name; the same bounded-setup leak the
            // single-realm `set_function` takes.
            let name: &'static str = Box::leak(name.to_string().into_boxed_str());
            let mut out = Err(RealmError::Engine(
                "set_function_in_realm did not run".to_string(),
            ));
            self.agent.run_in_realm(&self.realm, |agent, mut gc| {
                let target = registry
                    .realms
                    .borrow()
                    .get(&realm)
                    .expect("checked realm")
                    .get(agent, gc.nogc())
                    .unbind();
                let func = create_builtin_function(
                    agent,
                    Behaviour::Regular(nova_trampoline::<F>),
                    BuiltinFunctionArgs::new_with_realm(
                        length as u32,
                        name,
                        target.bind(gc.nogc()),
                    ),
                    gc.nogc(),
                );
                let global = target.bind(gc.nogc()).global_object(agent).unbind();
                let key = PropertyKey::from_str(agent, name, gc.nogc()).unbind();
                let desc = PropertyDescriptor {
                    value: Some(func.unbind().into()),
                    ..Default::default()
                };
                out = match global.internal_define_own_property(agent, key, desc, gc.reborrow()) {
                    Ok(_) => Ok(()),
                    Err(_) => Err(RealmError::Engine("define_own_property threw".to_string())),
                };
            });
            out
        }

        fn set_host_data_in_realm(
            &mut self,
            realm: RealmId,
            data: HostData,
        ) -> Result<(), RealmError> {
            if !self.registry.realms.borrow().contains_key(&realm) {
                return Err(RealmError::NoSuchRealm(realm));
            }
            let registry = self.registry.clone();
            let mut out = Err(RealmError::Refused("realm carries no host slot"));
            self.agent.run_in_realm(&self.realm, |agent, gc| {
                let target = registry
                    .realms
                    .borrow()
                    .get(&realm)
                    .expect("checked realm")
                    .get(agent, gc.nogc())
                    .unbind();
                if let Some(hd) = target.bind(gc.nogc()).host_defined(agent) {
                    if let Some(slot) = hd.downcast_ref::<NovaHostSlot>() {
                        *slot.neutral.borrow_mut() = Some(data);
                        out = Ok(());
                    }
                }
            });
            out
        }
    }

    impl ScriptEngineSnapshot for NovaEngine {
        fn snapshot_clone(&mut self) -> Result<Self, Self::Error> {
            NovaEngine::snapshot_clone(self)
        }
    }

    impl ScriptEngineLive for NovaEngine {
        fn make_reflector(&mut self, data: ReflectorData) -> Result<Self::Value, Self::Error> {
            let release = self.release.clone();
            let mut out = None;
            self.agent.run_in_realm(&self.realm, |agent, _gc| {
                let eo = EmbedderObject::create_with_owner(agent, data, u64::from(MAIN_REALM));
                out = Some(NovaValue::new(
                    Global::new(agent, Value::EmbedderObject(eo).unbind()),
                    &release,
                ));
            });
            Ok(out.expect("run_in_realm ran"))
        }

        fn reflector_data(&mut self, value: &Self::Value) -> Option<ReflectorData> {
            let mut out = None;
            self.agent.run_in_realm(&self.realm, |agent, gc| {
                if let Value::EmbedderObject(eo) = value.get(agent, gc.nogc()) {
                    out = Some(eo.embedder_data(agent));
                }
            });
            out
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // Regression guard for the Nova reflector leak (fixed by the `NovaValue`
        // deferred-release wrapper). Before the fix, `NovaCallCx::arg` returned a bare
        // `Global::new(...)`, and Nova's `Global` has no `Drop` — every heap-rooted
        // `Global` occupies an `agent.heap.globals` slot that must be *explicitly*
        // `take`n or it leaks a permanent root. The per-call `Global`s the generic DOM
        // code obtained via `cx.arg(i)` (and dropped) were never freed, so any
        // reflector passed as a native-fn argument — `parent.appendChild(child)`,
        // `removeChild`, etc. — was pinned forever, defeating GC reaping on Nova.
        // Now `cx.arg` mints a `NovaValue` whose `Drop` parks the `Global` on the
        // release queue, which the trampoline drains at call end. See
        // `docs/2026-06-19_nova_reflector_global_leak.md`.
        #[test]
        fn arg_reflector_dies_after_gc() {
            // A reflector passed as an argument to a native fn must still die once
            // script drops it.
            let mut engine = NovaEngine::new().unwrap();
            struct Canonical;
            impl NativeFn<NovaEngine> for Canonical {
                fn call(cx: &mut NovaCallCx<'_>) -> Result<NovaValue, String> {
                    cx.reflector_for(0x99)
                }
            }
            struct Consume;
            impl NativeFn<NovaEngine> for Consume {
                fn call(cx: &mut NovaCallCx<'_>) -> Result<NovaValue, String> {
                    let _ = cx.arg(0); // touch the arg, then let it drop
                    Ok(cx.undefined())
                }
            }
            engine.set_function::<Canonical>("canonical", 0).unwrap();
            engine.set_function::<Consume>("consume", 1).unwrap();
            engine
                .eval("globalThis.x = canonical(); consume(x); globalThis.x = null;")
                .unwrap();
            engine.pump_microtasks();
            engine.agent.gc();
            engine.agent.gc();
            assert_eq!(
                engine.drain_dead_reflectors(),
                vec![0x99],
                "a reflector passed as a native-fn argument still dies after gc",
            );
        }

        #[test]
        fn reflector_round_trip_survives_gc() {
            let mut engine = NovaEngine::new().unwrap();
            let v = engine.make_reflector(0xDEAD_BEEF).unwrap();
            // Survives collection while reachable only via the rooted Global.
            engine.agent.gc();
            engine.agent.gc();
            assert_eq!(engine.reflector_data(&v), Some(0xDEAD_BEEF));

            // A non-reflector value yields None, and the value surface works.
            let n = engine.eval("1 + 2").unwrap();
            assert_eq!(engine.reflector_data(&n), None);
            assert_eq!(engine.value_to_string(&n).unwrap(), "3");
        }

        #[test]
        fn global_reflector_is_reachable_from_js() {
            let mut engine = NovaEngine::new().unwrap();
            let reflector = engine.make_reflector(0x1234).unwrap();
            engine.set_global("node", &reflector).unwrap();

            // JS reads the global; the value it yields carries the host data.
            let from_js = engine.eval("node").unwrap();
            assert_eq!(engine.reflector_data(&from_js), Some(0x1234));
        }

        #[test]
        fn native_fn_reaches_host_data_and_reflector_arg() {
            use std::cell::RefCell;
            use std::rc::Rc;

            // The host sink a `setText`-style callback writes to (stands in for the DOM).
            type Sink = RefCell<Vec<(ReflectorData, String)>>;
            let sink: Rc<Sink> = Rc::new(RefCell::new(Vec::new()));

            let mut engine = NovaEngine::new().unwrap();
            engine.set_host_data(sink.clone());

            // setText(node, text): recover the node id off the reflector arg, read the
            // text, and record both into host data — reached via Nova [[HostDefined]],
            // not a thread_local.
            struct SetText;
            impl NativeFn<NovaEngine> for SetText {
                fn call(cx: &mut NovaCallCx<'_>) -> Result<NovaValue, String> {
                    let node = cx.arg(0);
                    let text = cx.arg(1);
                    let id = cx.reflector_data(&node).unwrap_or(0);
                    let text = cx.value_to_string(&text)?;
                    if let Some(data) = cx.host_data() {
                        if let Some(sink) = data.downcast_ref::<Sink>() {
                            sink.borrow_mut().push((id, text));
                        }
                    }
                    Ok(cx.undefined())
                }
            }
            engine.set_function::<SetText>("setText", 2).unwrap();

            let node = engine.make_reflector(0x42).unwrap();
            engine.set_global("node", &node).unwrap();
            engine.eval("setText(node, 'hello from JS')").unwrap();

            assert_eq!(*sink.borrow(), vec![(0x42, "hello from JS".to_string())]);
        }

        #[test]
        fn snapshot_clone_preserves_js_state_and_replaces_host_slot() {
            use std::cell::RefCell;
            use std::rc::Rc;

            type Sink = RefCell<Vec<String>>;

            struct Capture;
            impl NativeFn<NovaEngine> for Capture {
                fn call(cx: &mut NovaCallCx<'_>) -> Result<NovaValue, String> {
                    let value = cx.arg(0);
                    let value = cx.value_to_string(&value)?;
                    if let Some(data) = cx.host_data() {
                        if let Some(sink) = data.downcast_ref::<Sink>() {
                            sink.borrow_mut().push(value);
                        }
                    }
                    Ok(cx.undefined())
                }
            }

            let base_sink: Rc<Sink> = Rc::new(RefCell::new(Vec::new()));
            let clone_sink: Rc<Sink> = Rc::new(RefCell::new(Vec::new()));

            let mut engine = NovaEngine::new().unwrap();
            engine.set_function::<Capture>("capture", 1).unwrap();
            engine.set_host_data(base_sink.clone());
            engine
                .eval(
                    "var counter = 1; \
                     function bump() { counter += 1; return counter; } \
                     counter",
                )
                .unwrap();

            let mut clone = engine.snapshot_clone().unwrap();
            clone.set_host_data(clone_sink.clone());

            clone.eval("capture(bump())").unwrap();
            engine.eval("capture(counter)").unwrap();

            assert_eq!(*clone_sink.borrow(), vec!["2".to_string()]);
            assert_eq!(*base_sink.borrow(), vec!["1".to_string()]);
        }

        #[test]
        fn snapshot_clone_refuses_pending_jobs() {
            let mut engine = NovaEngine::new().unwrap();
            engine
                .eval("Promise.resolve().then(() => { globalThis.done = true; });")
                .unwrap();

            match engine.snapshot_clone() {
                Ok(_) => panic!("snapshot clone unexpectedly accepted pending jobs"),
                Err(err) => assert_eq!(err, "cannot snapshot clone NovaEngine with pending jobs"),
            }

            engine.pump_microtasks();
            assert!(engine.snapshot_clone().is_ok());
        }

        #[test]
        fn host_promise_bridges_js_await() {
            let mut engine = NovaEngine::new().unwrap();

            // Resolve path: a parked `await` resumes when the host settles the promise.
            let (promise, token) = engine.new_host_promise().unwrap();
            engine.set_global("p", &promise).unwrap();
            engine
                .eval("globalThis.out = 'pending'; (async () => { globalThis.out = await p; })();")
                .unwrap();
            // Drain the script's own microtasks so the async fn reaches its parked
            // `await` (Nova attaches the fulfill reaction via a job, not synchronously).
            engine.pump_microtasks();
            // The await is parked until the host settles; the post-await line has not run.
            let parked = engine.eval("out").unwrap();
            assert_eq!(engine.value_to_string(&parked).unwrap(), "pending");

            // Parenthesized so the literal is an expression, not a directive prologue
            // (a bare string statement has no completion value, per spec).
            let resolution = engine.eval("('resolved!')").unwrap();
            engine.settle_host_promise(token, Ok(&resolution)).unwrap();
            engine.pump_microtasks();
            let resumed = engine.eval("out").unwrap();
            assert_eq!(engine.value_to_string(&resumed).unwrap(), "resolved!");

            // Reject path: the awaiting `catch` sees the host's error value.
            let (promise2, token2) = engine.new_host_promise().unwrap();
            engine.set_global("q", &promise2).unwrap();
            engine
                .eval(
                    "globalThis.err = 'none'; \
                     (async () => { try { await q; } catch (e) { globalThis.err = e; } })();",
                )
                .unwrap();
            let reason = engine.eval("('boom')").unwrap();
            engine.settle_host_promise(token2, Err(&reason)).unwrap();
            engine.pump_microtasks();
            let caught = engine.eval("err").unwrap();
            assert_eq!(engine.value_to_string(&caught).unwrap(), "boom");

            // Survives collection while pending, and double-settle is a silent no-op.
            engine.settle_host_promise(token, Ok(&resolution)).unwrap();
        }

        #[test]
        fn budgeted_pump_bounds_a_microtask_chain() {
            let mut engine = NovaEngine::new().unwrap();
            // A chain of `.then` continuations, each bumping a global counter. The chain
            // is lazy (each `.then` enqueues the next reaction only as the prior resolves),
            // so it is a run of distinct jobs Nova can step through one at a time.
            engine
                .eval(
                    "globalThis.n = 0; \
                     Promise.resolve() \
                       .then(() => { globalThis.n++; }) \
                       .then(() => { globalThis.n++; }) \
                       .then(() => { globalThis.n++; });",
                )
                .unwrap();

            // One job at a time: the queue stays non-empty until the chain is exhausted,
            // so the first bounded pump reports `Pending`, not `Quiescent`.
            assert_eq!(engine.pump(Budget::Steps(1)), PumpOutcome::Pending);
            let after_one = engine.eval("n").unwrap();
            assert_eq!(engine.value_to_string(&after_one).unwrap(), "1");

            // Drain the rest one step at a time; the loop ends when pump goes Quiescent.
            while engine.pump(Budget::Steps(1)) == PumpOutcome::Pending {}
            let done = engine.eval("n").unwrap();
            assert_eq!(engine.value_to_string(&done).unwrap(), "3");
        }

        #[test]
        fn reflector_for_reports_death_after_gc() {
            let mut engine = NovaEngine::new().unwrap();

            // A callback handing JS the *canonical* reflector for node 0x42.
            struct Canonical;
            impl NativeFn<NovaEngine> for Canonical {
                fn call(cx: &mut NovaCallCx<'_>) -> Result<NovaValue, String> {
                    cx.reflector_for(0x42)
                }
            }
            engine.set_function::<Canonical>("canonical", 0).unwrap();

            // Hold the reflector from JS; canonical identity holds (=== same object)
            // and no death is reported while it is referenced.
            engine
                .eval("globalThis.x = canonical(); globalThis.same = (canonical() === x);")
                .unwrap();
            let same = engine.eval("same").unwrap();
            assert_eq!(engine.value_to_string(&same).unwrap(), "true");
            assert!(engine.drain_dead_reflectors().is_empty());

            // Drop the last JS reference, run the microtask checkpoint (ClearKeptObjects)
            // and the GC: the weak cache now reports the death.
            engine.eval("globalThis.x = null;").unwrap();
            engine.pump_microtasks();
            engine.agent.gc();
            engine.agent.gc();
            assert_eq!(engine.drain_dead_reflectors(), vec![0x42]);

            // The dead entry was swept, so a second drain is empty.
            assert!(engine.drain_dead_reflectors().is_empty());
        }
    }
}

#[cfg(target_pointer_width = "64")]
pub use native::{NovaCallCx, NovaEngine, NovaValue};
