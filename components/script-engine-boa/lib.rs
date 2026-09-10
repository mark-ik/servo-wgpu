// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Boa 0.21 backend for [`script_engine_api`]. Pure Rust → the wasm32 scripting
//! backend, and the native conformance oracle. Engine-native types (`JsValue`,
//! `Context`, the reflector `Class`) stay confined to this crate.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::rc::Rc;

use boa_engine::{
    Context, JsData, JsError, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction,
    Source,
    builtins::promise::PromiseState,
    class::{Class, ClassBuilder},
    module::{Module, ModuleLoader, ModuleRequest, Referrer},
    object::{
        WeakJsObject,
        builtins::{JsFunction, JsPromise},
    },
    realm::Realm,
};
use boa_gc::{Finalize, Gc, GcRefCell, Trace};
use script_engine_api::{
    Budget, CallCx, HostData, MAIN_REALM, NativeFn, PromiseToken, PumpOutcome, RealmError, RealmId,
    ReflectorData, ScriptEngine, ScriptEngineLive,
};

/// Native-data reflector (Appendix A Finding 2): a JS object carrying only the host
/// [`ReflectorData`]. The DOM node's data lives in the host arena, never the JS heap.
#[derive(Debug, Trace, Finalize, JsData)]
struct Reflector {
    #[unsafe_ignore_trace]
    data: ReflectorData,
    #[unsafe_ignore_trace]
    owner: RealmId,
}

impl Class for Reflector {
    const NAME: &'static str = "Reflector";
    const LENGTH: usize = 0;

    fn data_constructor(_t: &JsValue, _a: &[JsValue], _c: &mut Context) -> JsResult<Self> {
        Err(JsNativeError::typ()
            .with_message("Reflectors are host-created, not `new`-able")
            .into())
    }

    fn init(_builder: &mut ClassBuilder<'_>) -> JsResult<()> {
        Ok(())
    }
}

/// The resolve/reject functions of a pending host promise, held until the host
/// settles it. Traced: the `JsFunction`s are live JS objects that must survive
/// collection while the promise is pending.
#[derive(Trace, Finalize)]
struct PendingPromise {
    resolve: JsFunction,
    reject: JsFunction,
}

#[derive(Trace, Finalize)]
struct RealmRegistry {
    realms: GcRefCell<HashMap<RealmId, Realm>>,
    #[unsafe_ignore_trace]
    next_realm: Cell<RealmId>,
}

/// Host-data slot stored in Boa's `Context` host-defined data. Holds the
/// engine-neutral [`HostData`] (the `Rc<dyn Any>` is not traced — it holds host
/// state, never JS values) plus the canonical-reflector cache (`NodeId →
/// reflector`). The cache holds each reflector **weakly** (a [`WeakJsObject`]):
/// it pins canonical identity (`document.body === document.body`) only while
/// script still references the reflector, and reports the death once script
/// drops it (G1 reflector liveness — see
/// [`drain_dead_reflectors`](ScriptEngine::drain_dead_reflectors)). The `pending`
/// table is traced: live resolving functions for host promises awaiting
/// settlement, keyed by [`PromiseToken`]. All engine-side, never in neutral host
/// state.
#[derive(Trace, Finalize, JsData)]
struct HostCell {
    registry: Gc<RealmRegistry>,
    #[unsafe_ignore_trace]
    data: RefCell<Option<HostData>>,
    pending: GcRefCell<HashMap<u64, PendingPromise>>,
    #[unsafe_ignore_trace]
    next_token: Cell<u64>,
}

impl HostCell {
    fn new(registry: Gc<RealmRegistry>) -> Self {
        Self {
            registry,
            data: RefCell::new(None),
            pending: GcRefCell::new(HashMap::new()),
            next_token: Cell::new(0),
        }
    }
}

/// Per-realm host slot, stored in Boa's `Realm::host_defined()`.
///
/// Each slot owns its document's reflector identity and strong-root policy.
/// Raw reflector IDs may repeat in different host arenas. A platform object's
/// associated realm stays fixed when script in another realm references it.
///
/// `HostData` is `Rc<dyn Any>` and holds host state, never JS values, so it is
/// not traced — the same reasoning as [`HostCell::data`].
#[derive(Trace, Finalize, JsData)]
struct RealmSlot {
    #[unsafe_ignore_trace]
    id: RealmId,
    #[unsafe_ignore_trace]
    data: RefCell<Option<HostData>>,
    reflectors: GcRefCell<HashMap<u64, WeakJsObject>>,
    /// Strong roots on reflectors the opaque-root policy holds. Traced, so a
    /// rooted reflector — and through the bootstrap's wrapper `WeakMap`, its
    /// wrapper and every expando on it — survives collection for as long as its
    /// node is reachable.
    roots: GcRefCell<HashMap<u64, JsObject>>,
}

/// Read the current realm's id, or [`MAIN_REALM`] if the realm carries no slot
/// (which cannot happen for a realm this crate created, but keeps the read total).
fn realm_id_of(ctx: &Context) -> RealmId {
    ctx.realm()
        .host_defined()
        .get::<RealmSlot>()
        .map_or(MAIN_REALM, |slot| slot.id)
}

/// Mint a pending promise and register its resolving functions in the host cell.
/// Shared by the engine-level and in-callback `new_host_promise`, both of which hold
/// a `&mut Context`.
fn make_pending(ctx: &mut Context) -> JsResult<(JsValue, PromiseToken)> {
    let (promise, resolvers) = JsPromise::new_pending(ctx);
    let Some(cell) = ctx.get_data::<HostCell>() else {
        return Err(JsNativeError::typ()
            .with_message("host cell missing")
            .into());
    };
    let token = cell.next_token.get();
    cell.next_token.set(token + 1);
    cell.pending.borrow_mut().insert(
        token,
        PendingPromise {
            resolve: resolvers.resolve,
            reject: resolvers.reject,
        },
    );
    Ok((promise.into(), token))
}

/// The host module resolver for one `eval_module` call: maps an import
/// `(specifier, referrer_url)` to the imported module's `(resolved_url, source)`,
/// or `None` when it cannot be resolved/fetched.
type ModuleResolver<'a> = dyn FnMut(&str, &str) -> Option<(String, String)> + 'a;

/// Boa [`ModuleLoader`] backed by a host resolver. The loader lives on the
/// `Context` for the engine's whole life, but the resolver borrows host state (the
/// page fetcher) and lives only for one `eval_module` call — so it is injected as a
/// scoped raw pointer, set for the duration of the call and cleared after.
/// `load_imported_module` reads it to fetch + parse each dependency on demand,
/// caching by resolved URL so a diamond / cycle loads each module once.
#[derive(Default)]
struct HostModuleLoader {
    /// Parsed modules by resolved URL — the per-call cache, cleared each call.
    cache: RefCell<HashMap<String, Module>>,
    /// Raw pointer to the active resolver, set for one `eval_module` call (`None`
    /// otherwise). The pointee outlives the call (it is an `eval_module` argument),
    /// so the deref in `load_imported_module` is sound; single-threaded, no reentrancy.
    resolver: Cell<Option<*mut ModuleResolver<'static>>>,
}

impl HostModuleLoader {
    /// Install `resolver` for the duration of `f`, then clear it and the module
    /// cache. The lifetime is erased to `'static` for storage in the long-lived
    /// loader; it is never observed past `f`, where the real resolver lives.
    fn with_resolver<R>(&self, resolver: &mut ModuleResolver<'_>, f: impl FnOnce() -> R) -> R {
        let raw: *mut ModuleResolver<'_> = resolver;
        // SAFETY: erases only the trait object's captured-data lifetime, not the
        // (HRTB) argument lifetimes; same fat-pointer layout. Cleared below before
        // the real lifetime ends.
        let erased: *mut ModuleResolver<'static> = unsafe { std::mem::transmute(raw) };
        self.resolver.set(Some(erased));
        let out = f();
        self.resolver.set(None);
        self.cache.borrow_mut().clear();
        out
    }
}

impl ModuleLoader for HostModuleLoader {
    fn load_imported_module(
        self: Rc<Self>,
        referrer: Referrer,
        request: ModuleRequest,
        context: &RefCell<&mut Context>,
    ) -> impl Future<Output = JsResult<Module>> {
        let result = (|| {
            let specifier = request.specifier().to_std_string_escaped();
            // The importing module's URL (its `path`, set when we parsed it) is the
            // base its relative imports resolve against; empty for the entry's realm.
            let referrer_url = referrer
                .path()
                .and_then(Path::to_str)
                .unwrap_or("")
                .to_string();

            let Some(ptr) = self.resolver.get() else {
                return Err(JsNativeError::typ()
                    .with_message("no module resolver active")
                    .into());
            };
            // SAFETY: `ptr` was set by `with_resolver` to a resolver that outlives
            // this call (cleared after); single-threaded, non-reentrant access.
            let resolve = unsafe { &mut *ptr };
            let Some((url, source)) = resolve(&specifier, &referrer_url) else {
                return Err(JsNativeError::typ()
                    .with_message(format!("could not resolve module '{specifier}'"))
                    .into());
            };

            if let Some(module) = self.cache.borrow().get(&url).cloned() {
                return Ok(module);
            }
            let module = Module::parse(
                Source::from_bytes(source.as_bytes()).with_path(Path::new(&url)),
                None,
                &mut context.borrow_mut(),
            )?;
            self.cache.borrow_mut().insert(url, module.clone());
            Ok(module)
        })();
        async { result }
    }
}

/// A Boa-backed scripting engine.
pub struct BoaEngine {
    ctx: Context,
    /// The module loader installed on `ctx`; `eval_module` sets its resolver per call.
    loader: Rc<HostModuleLoader>,
    /// Live realms of this engine (this is one **agent**), including
    /// [`MAIN_REALM`], which is the realm `Context::default()` built. Boa's
    /// `Realm` is a `Gc` handle, so holding one here roots it; `discard_realm`
    /// drops the handle and the realm becomes collectable.
    registry: Gc<RealmRegistry>,
}

impl BoaEngine {
    /// Enter `realm`, run `f`, and restore the previous realm — including on the
    /// error path, which is why this is a helper rather than three lines at each
    /// call site. Boa's `enter_realm` swaps the *current call frame's* realm, so
    /// the restore has to happen before the frame is used again.
    fn with_realm<R>(
        &mut self,
        realm: RealmId,
        f: impl FnOnce(&mut Context) -> R,
    ) -> Result<R, RealmError> {
        let Some(target) = self.registry.realms.borrow().get(&realm).cloned() else {
            return Err(RealmError::NoSuchRealm(realm));
        };
        let previous = self.ctx.enter_realm(target);
        let out = f(&mut self.ctx);
        let _ = self.ctx.enter_realm(previous);
        Ok(out)
    }
}

/// The call context handed to a native callback. Boa's callback gives
/// `(this, &[JsValue], &mut Context)`, so one lifetime suffices.
pub struct BoaCallCx<'a> {
    this: JsValue,
    ctx: &'a mut Context,
    args: &'a [JsValue],
}

impl BoaCallCx<'_> {
    fn with_reflector_realm<R>(
        &mut self,
        realm: RealmId,
        operation: impl FnOnce(&mut Self) -> Result<R, RealmError>,
    ) -> Result<R, RealmError> {
        let target = self
            .ctx
            .get_data::<HostCell>()
            .ok_or(RealmError::Refused("host cell missing"))?
            .registry
            .realms
            .borrow()
            .get(&realm)
            .cloned()
            .ok_or(RealmError::NoSuchRealm(realm))?;
        let previous = self.ctx.enter_realm(target);
        let result = operation(self);
        self.ctx.enter_realm(previous);
        result
    }
}

impl CallCx for BoaCallCx<'_> {
    type Value = JsValue;
    type Error = JsError;

    fn error(&mut self, message: &str) -> Self::Error {
        JsNativeError::error()
            .with_message(message.to_owned())
            .into()
    }

    fn arg(&mut self, i: usize) -> JsValue {
        self.args.get(i).cloned().unwrap_or_default()
    }

    fn host_data(&self) -> Option<HostData> {
        let hd = self.ctx.realm().host_defined();
        let slot = hd.get::<RealmSlot>()?;
        if slot.id != MAIN_REALM {
            return slot.data.borrow().clone();
        }
        let data = slot.data.borrow().clone();
        data.or_else(|| {
            self.ctx
                .get_data::<HostCell>()
                .and_then(|c| c.data.borrow().clone())
        })
    }

    fn this_value(&mut self) -> Self::Value {
        self.this.clone()
    }

    fn caller_realm(&mut self) -> RealmId {
        self.ctx
            .native_caller_realm()
            .and_then(|realm| realm.host_defined().get::<RealmSlot>().map(|slot| slot.id))
            .unwrap_or_else(|| realm_id_of(self.ctx))
    }

    fn current_realm(&mut self) -> RealmId {
        realm_id_of(self.ctx)
    }

    fn reflector_for(&mut self, data: ReflectorData) -> Result<JsValue, JsError> {
        // Cache hit *and still alive*: return the same object so reflectors compare
        // `===`. A dead weak (script dropped it) falls through to a fresh mint.
        if let Some(cell) = self.ctx.realm().host_defined().get::<RealmSlot>() {
            if let Some(obj) = cell
                .reflectors
                .borrow()
                .get(&data)
                .and_then(WeakJsObject::upgrade)
            {
                return Ok(obj.into());
            }
        }
        let v = self.make_reflector(data)?;
        if let Some(cell) = self.ctx.realm().host_defined().get::<RealmSlot>() {
            if let Some(obj) = v.as_object() {
                cell.reflectors.borrow_mut().insert(data, obj.downgrade());
            }
        }
        Ok(v)
    }

    fn reflector_for_in_realm(
        &mut self,
        realm: RealmId,
        data: ReflectorData,
    ) -> Result<Self::Value, RealmError> {
        self.with_reflector_realm(realm, |cx| {
            cx.reflector_for(data)
                .map_err(|error| RealmError::Engine(format!("{error:?}")))
        })
    }

    fn root_reflector_in_realm(
        &mut self,
        realm: RealmId,
        data: ReflectorData,
    ) -> Result<(), RealmError> {
        self.with_reflector_realm(realm, |cx| {
            let value = cx
                .reflector_for(data)
                .map_err(|error| RealmError::Engine(format!("{error:?}")))?;
            let object = value
                .as_object()
                .ok_or(RealmError::Refused("reflector is not an object"))?;
            let host = cx.ctx.realm().host_defined();
            let slot = host
                .get::<RealmSlot>()
                .ok_or(RealmError::Refused("realm host slot missing"))?;
            slot.roots.borrow_mut().insert(data, object);
            Ok(())
        })
    }

    fn unroot_reflector_in_realm(
        &mut self,
        realm: RealmId,
        data: ReflectorData,
    ) -> Result<(), RealmError> {
        self.with_reflector_realm(realm, |cx| {
            cx.unroot_reflector(data);
            Ok(())
        })
    }

    fn root_reflector(&mut self, data: ReflectorData) -> bool {
        let Ok(v) = self.reflector_for(data) else {
            return false;
        };
        let Some(obj) = v.as_object() else {
            return false;
        };
        match self.ctx.realm().host_defined().get::<RealmSlot>() {
            Some(cell) => {
                cell.roots.borrow_mut().insert(data, obj);
                true
            },
            None => false,
        }
    }

    fn unroot_reflector(&mut self, data: ReflectorData) {
        if let Some(cell) = self.ctx.realm().host_defined().get::<RealmSlot>() {
            cell.roots.borrow_mut().remove(&data);
        }
    }

    fn value_to_string(&mut self, value: &JsValue) -> Result<String, JsError> {
        Ok(value.to_string(self.ctx)?.to_std_string_escaped())
    }

    fn reflector_data(&mut self, value: &JsValue) -> Option<ReflectorData> {
        value
            .as_object()
            .and_then(|o| o.downcast_ref::<Reflector>().map(|r| r.data))
    }

    fn reflector_is_local(&mut self, value: &JsValue) -> bool {
        let current = realm_id_of(self.ctx);
        value
            .as_object()
            .and_then(|o| o.downcast_ref::<Reflector>().map(|r| r.owner == current))
            .unwrap_or(false)
    }

    fn make_reflector(&mut self, data: ReflectorData) -> Result<JsValue, JsError> {
        // The `Reflector` class is registered at engine construction, so building one
        // from the held `Context` is the in-callback mirror of the engine-level
        // `ScriptEngineLive::make_reflector`.
        let obj: JsObject = Reflector::from_data(
            Reflector {
                data,
                owner: realm_id_of(self.ctx),
            },
            self.ctx,
        )?;
        Ok(obj.into())
    }

    fn make_string(&mut self, s: &str) -> Result<JsValue, JsError> {
        Ok(JsValue::from(JsString::from(s)))
    }

    fn make_null(&mut self) -> JsValue {
        JsValue::null()
    }

    fn undefined(&mut self) -> JsValue {
        JsValue::undefined()
    }

    fn new_host_promise(&mut self) -> Result<(JsValue, PromiseToken), JsError> {
        make_pending(self.ctx)
    }
}

impl ScriptEngine for BoaEngine {
    type Value = JsValue;
    type Error = JsError;
    type CallCx<'a> = BoaCallCx<'a>;

    fn new() -> Result<Self, Self::Error> {
        let loader = Rc::new(HostModuleLoader::default());
        let mut ctx = Context::builder().module_loader(loader.clone()).build()?;
        ctx.register_global_class::<Reflector>()?;
        let registry = Gc::new(RealmRegistry {
            realms: GcRefCell::new(HashMap::new()),
            next_realm: Cell::new(MAIN_REALM + 1),
        });
        ctx.insert_data(HostCell::new(registry.clone()));
        let main = ctx.realm().clone();
        main.host_defined_mut().insert(RealmSlot {
            id: MAIN_REALM,
            data: RefCell::new(None),
            reflectors: GcRefCell::new(HashMap::new()),
            roots: GcRefCell::new(HashMap::new()),
        });
        registry.realms.borrow_mut().insert(MAIN_REALM, main);
        Ok(Self {
            ctx,
            loader,
            registry,
        })
    }

    fn eval(&mut self, source: &str) -> Result<Self::Value, Self::Error> {
        self.ctx.eval(Source::from_bytes(source))
    }

    fn call_function(
        &mut self,
        function: &Self::Value,
        this: &Self::Value,
        args: &[Self::Value],
    ) -> Result<Self::Value, RealmError> {
        let function = function
            .as_callable()
            .ok_or_else(|| RealmError::Engine("value is not callable".into()))?;
        function
            .call(this, args, &mut self.ctx)
            .map_err(|error| RealmError::Engine(self.describe_error(&error)))
    }

    fn eval_module(
        &mut self,
        source: &str,
        base_url: &str,
        resolve: &mut dyn FnMut(&str, &str) -> Option<(String, String)>,
    ) -> Result<Option<Self::Value>, Self::Error> {
        // Install the host resolver for this call so the loader can fetch imports,
        // then parse the entry (its `path` = `base_url`, the base its imports resolve
        // against) and drive load → link → evaluate. `run_jobs` settles the promise
        // (synchronously, since the resolver fetches synchronously).
        let loader = Rc::clone(&self.loader);
        let state = loader.with_resolver(resolve, || {
            let module = Module::parse(
                Source::from_bytes(source.as_bytes()).with_path(Path::new(base_url)),
                None,
                &mut self.ctx,
            )?;
            let promise = module.load_link_evaluate(&mut self.ctx);
            let _ = self.ctx.run_jobs();
            Ok::<_, JsError>(promise.state())
        })?;
        match state {
            PromiseState::Fulfilled(_) => Ok(Some(JsValue::undefined())),
            PromiseState::Rejected(reason) => Err(JsError::from_opaque(reason)),
            PromiseState::Pending => Err(JsNativeError::typ()
                .with_message("module evaluation did not settle synchronously")
                .into()),
        }
    }

    fn value_to_string(&mut self, value: &Self::Value) -> Result<String, Self::Error> {
        Ok(value.to_string(&mut self.ctx)?.to_std_string_escaped())
    }

    fn describe_error(&mut self, error: &Self::Error) -> String {
        // `JsError` is opaque; the thrown value's `toString` ("TypeError: …") is what
        // a `negative:` match needs. Fall back to the error's own Debug.
        if let Ok(thrown) = error.clone().into_opaque(&mut self.ctx) {
            if let Ok(s) = thrown.to_string(&mut self.ctx) {
                return s.to_std_string_escaped();
            }
        }
        format!("{error:?}")
    }

    fn set_global(&mut self, name: &str, value: &Self::Value) -> Result<(), Self::Error> {
        let global = self.ctx.global_object();
        global.set(JsString::from(name), value.clone(), false, &mut self.ctx)?;
        Ok(())
    }

    fn set_host_data(&mut self, data: HostData) {
        if let Some(cell) = self.ctx.get_data::<HostCell>() {
            *cell.data.borrow_mut() = Some(data);
        }
    }

    fn set_function<F: NativeFn<Self>>(
        &mut self,
        name: &str,
        length: usize,
    ) -> Result<(), Self::Error> {
        // A captures-free trampoline, monomorphized per `F` to a distinct fn
        // pointer — Boa's cheap native-function path, matching Nova's.
        fn trampoline<F: NativeFn<BoaEngine>>(
            this: &JsValue,
            args: &[JsValue],
            ctx: &mut Context,
        ) -> JsResult<JsValue> {
            let mut cx = BoaCallCx {
                ctx,
                args,
                this: this.clone(),
            };
            F::call(&mut cx)
        }
        self.ctx.register_global_builtin_callable(
            JsString::from(name),
            length,
            NativeFunction::from_fn_ptr(trampoline::<F>),
        )
    }

    fn pump(&mut self, _budget: Budget) -> PumpOutcome {
        // Boa's default executor (SimpleJobExecutor) runs the promise-job queue to
        // completion and exposes no sub-drain, so the budget cannot be honored: drain
        // fully and report `Quiescent`. Step-bounding is a Nova/piccolo capability; on
        // Boa a runaway microtask loop still hangs here (acceptable: Boa is the
        // wasm/oracle backend, the runaway-sensitive actors run on native Nova).
        let _ = self.ctx.run_jobs();
        PumpOutcome::Quiescent
    }

    fn new_host_promise(&mut self) -> Result<(Self::Value, PromiseToken), Self::Error> {
        make_pending(&mut self.ctx)
    }

    fn settle_host_promise(
        &mut self,
        token: PromiseToken,
        outcome: Result<&Self::Value, &Self::Value>,
    ) -> Result<(), Self::Error> {
        // Take the resolving functions out of the table (consume the token), then call
        // the matching one. Calling enqueues the reaction jobs; the host drains them
        // with `pump_microtasks`. An unknown/already-settled token is a no-op.
        let pending = self
            .ctx
            .get_data::<HostCell>()
            .and_then(|cell| cell.pending.borrow_mut().remove(&token));
        let Some(pending) = pending else {
            return Ok(());
        };
        let undefined = JsValue::undefined();
        match outcome {
            Ok(value) => {
                pending
                    .resolve
                    .call(&undefined, &[value.clone()], &mut self.ctx)?;
            },
            Err(error) => {
                pending
                    .reject
                    .call(&undefined, &[error.clone()], &mut self.ctx)?;
            },
        }
        Ok(())
    }

    fn force_gc(&mut self) {
        // Drive Boa's collector so the weak canonical-cache entries for reflectors
        // script no longer references become dead before `drain_dead_reflectors`
        // sweeps them — the engine half of the frame-cadence GC tick.
        boa_gc::force_collect();
    }

    fn minted_reflectors(&mut self) -> Vec<ReflectorData> {
        self.minted_reflectors_in_realm(MAIN_REALM)
            .expect("main realm exists")
    }

    fn minted_reflectors_in_realm(
        &mut self,
        realm: RealmId,
    ) -> Result<Vec<ReflectorData>, RealmError> {
        let target = self
            .registry
            .realms
            .borrow()
            .get(&realm)
            .cloned()
            .ok_or(RealmError::NoSuchRealm(realm))?;
        let previous = self.ctx.enter_realm(target);
        let out = (|| {
            self.ctx
                .realm()
                .host_defined()
                .get::<RealmSlot>()
                .map(|cell| cell.reflectors.borrow().keys().copied().collect())
                .unwrap_or_default()
        })();
        self.ctx.enter_realm(previous);
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
        let target = self
            .registry
            .realms
            .borrow()
            .get(&realm)
            .cloned()
            .ok_or(RealmError::NoSuchRealm(realm))?;
        let previous = self.ctx.enter_realm(target);
        let out = (|| {
            for &d in data {
                // Through the canonical cache, so a root and a `reflector_for` hit are
                // the same object; an id whose weak has already died is re-minted here,
                // which is the correct outcome (the node is reachable again).
                let obj = match self.ctx.realm().host_defined().get::<RealmSlot>() {
                    Some(cell) => cell
                        .reflectors
                        .borrow()
                        .get(&d)
                        .and_then(WeakJsObject::upgrade),
                    None => None,
                };
                let obj = match obj {
                    Some(o) => o,
                    None => {
                        let Ok(v) = ScriptEngineLive::make_reflector(self, d) else {
                            continue;
                        };
                        let Some(o) = v.as_object() else { continue };
                        if let Some(cell) = self.ctx.realm().host_defined().get::<RealmSlot>() {
                            cell.reflectors.borrow_mut().insert(d, o.downgrade());
                        }
                        o
                    },
                };
                if let Some(cell) = self.ctx.realm().host_defined().get::<RealmSlot>() {
                    cell.roots.borrow_mut().insert(d, obj);
                }
            }
        })();
        self.ctx.enter_realm(previous);
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
        let target = self
            .registry
            .realms
            .borrow()
            .get(&realm)
            .cloned()
            .ok_or(RealmError::NoSuchRealm(realm))?;
        let previous = self.ctx.enter_realm(target);
        let out = (|| {
            if let Some(cell) = self.ctx.realm().host_defined().get::<RealmSlot>() {
                let mut roots = cell.roots.borrow_mut();
                for d in data {
                    roots.remove(d);
                }
            }
        })();
        self.ctx.enter_realm(previous);
        Ok(out)
    }

    fn rooted_reflector_count(&mut self) -> usize {
        self.rooted_reflector_count_in_realm(MAIN_REALM)
            .expect("main realm exists")
    }

    fn rooted_reflector_count_in_realm(&mut self, realm: RealmId) -> Result<usize, RealmError> {
        let target = self
            .registry
            .realms
            .borrow()
            .get(&realm)
            .cloned()
            .ok_or(RealmError::NoSuchRealm(realm))?;
        let previous = self.ctx.enter_realm(target);
        let out = (|| {
            self.ctx
                .realm()
                .host_defined()
                .get::<RealmSlot>()
                .map(|cell| cell.roots.borrow().len())
                .unwrap_or(0)
        })();
        self.ctx.enter_realm(previous);
        Ok(out)
    }

    // ---- Realms ------------------------------------------------------------
    //
    // Boa gives an embedder everything this needs: `Context::create_realm`
    // builds a realm with its own global and intrinsics on the *same* heap and
    // job queue, `enter_realm` swaps the active one, and `Realm::host_defined`
    // is a per-realm slot map. One `BoaEngine` is therefore one agent with as
    // many realms as the host asks for, and a value from one is an ordinary
    // `JsValue` in another — no marshalling anywhere.

    fn create_realm_from_call(
        cx: &mut Self::CallCx<'_>,
        data: HostData,
        initialize: impl for<'a> FnOnce(&mut Self::CallCx<'a>) -> Result<(), RealmError>,
    ) -> Result<RealmId, RealmError> {
        let registry = cx
            .ctx
            .get_data::<HostCell>()
            .expect("host cell")
            .registry
            .clone();
        let id = registry.next_realm.get();
        registry.next_realm.set(
            id.checked_add(1)
                .ok_or(RealmError::Refused("realm ids exhausted"))?,
        );
        let realm = cx
            .ctx
            .create_realm()
            .map_err(|e| RealmError::Engine(format!("{e:?}")))?;
        realm.host_defined_mut().insert(RealmSlot {
            id,
            data: RefCell::new(Some(data)),
            reflectors: GcRefCell::new(HashMap::new()),
            roots: GcRefCell::new(HashMap::new()),
        });
        registry.realms.borrow_mut().insert(id, realm.clone());
        let previous = cx.ctx.enter_realm(realm);
        let result = cx
            .ctx
            .register_global_class::<Reflector>()
            .map_err(|e| RealmError::Engine(format!("{e:?}")))
            .and_then(|_| {
                initialize(&mut BoaCallCx {
                    this: JsValue::undefined(),
                    ctx: cx.ctx,
                    args: &[],
                })
            });
        cx.ctx.enter_realm(previous);
        if result.is_err() {
            registry.realms.borrow_mut().remove(&id);
        }
        result.map(|()| id)
    }

    fn eval_from_call(cx: &mut Self::CallCx<'_>, source: &str) -> Result<Self::Value, RealmError> {
        cx.ctx
            .eval(Source::from_bytes(source))
            .map_err(|e| RealmError::Engine(format!("{e:?}")))
    }

    fn set_function_from_call<F: NativeFn<Self>>(
        cx: &mut Self::CallCx<'_>,
        name: &str,
        length: usize,
    ) -> Result<(), RealmError> {
        fn trampoline<F: NativeFn<BoaEngine>>(
            this: &JsValue,
            args: &[JsValue],
            ctx: &mut Context,
        ) -> JsResult<JsValue> {
            F::call(&mut BoaCallCx {
                ctx,
                args,
                this: this.clone(),
            })
        }
        cx.ctx
            .register_global_builtin_callable(
                JsString::from(name),
                length,
                NativeFunction::from_fn_ptr(trampoline::<F>),
            )
            .map_err(|e| RealmError::Engine(format!("{e:?}")))
    }

    fn eval_in_realm_from_call(
        cx: &mut Self::CallCx<'_>,
        realm: RealmId,
        source: &str,
    ) -> Result<Self::Value, Self::Error> {
        let target = cx
            .ctx
            .get_data::<HostCell>()
            .expect("host cell")
            .registry
            .realms
            .borrow()
            .get(&realm)
            .cloned()
            .ok_or_else(|| {
                JsNativeError::error().with_message(format!("no such realm: {realm}"))
            })?;
        let previous = cx.ctx.enter_realm(target);
        let result = cx.ctx.eval(Source::from_bytes(source));
        cx.ctx.enter_realm(previous);
        result
    }

    fn call_from_call(
        cx: &mut Self::CallCx<'_>,
        function: &Self::Value,
        this: &Self::Value,
        args: &[Self::Value],
    ) -> Result<Self::Value, Self::Error> {
        let function = function
            .as_callable()
            .ok_or_else(|| JsNativeError::typ().with_message("value is not callable"))?;
        function.call(this, args, cx.ctx)
    }

    fn realm_global_from_call(
        cx: &mut Self::CallCx<'_>,
        realm: RealmId,
    ) -> Result<Self::Value, RealmError> {
        let target = cx
            .ctx
            .get_data::<HostCell>()
            .expect("host cell")
            .registry
            .realms
            .borrow()
            .get(&realm)
            .cloned()
            .ok_or(RealmError::NoSuchRealm(realm))?;
        let previous = cx.ctx.enter_realm(target);
        let global = cx.ctx.global_object().into();
        cx.ctx.enter_realm(previous);
        Ok(global)
    }

    fn set_global_from_call(
        cx: &mut Self::CallCx<'_>,
        name: &str,
        value: &Self::Value,
    ) -> Result<(), RealmError> {
        let global = cx.ctx.global_object();
        global
            .set(JsString::from(name), value.clone(), false, cx.ctx)
            .map(|_| ())
            .map_err(|e| RealmError::Engine(format!("{e:?}")))
    }

    fn supports_realms(&self) -> bool {
        true
    }

    fn create_realm(&mut self) -> Result<RealmId, RealmError> {
        let realm = self
            .ctx
            .create_realm()
            .map_err(|e| RealmError::Engine(format!("{e:?}")))?;
        let id = self.registry.next_realm.get();
        self.registry.next_realm.set(
            id.checked_add(1)
                .ok_or(RealmError::Refused("realm ids exhausted"))?,
        );
        realm.host_defined_mut().insert(RealmSlot {
            id,
            data: RefCell::new(None),
            reflectors: GcRefCell::new(HashMap::new()),
            roots: GcRefCell::new(HashMap::new()),
        });
        // The reflector class is per-realm in Boa (`host_classes` lives on the
        // `Realm`), so a realm that will be handed reflectors needs its own
        // registration; without it `Reflector::from_data` in this realm cannot
        // find its prototype.
        self.registry.realms.borrow_mut().insert(id, realm);
        if let Err(e) = self
            .with_realm(id, |ctx| ctx.register_global_class::<Reflector>())
            .and_then(|r| r.map_err(|e| RealmError::Engine(format!("{e:?}"))))
        {
            self.registry.realms.borrow_mut().remove(&id);
            return Err(e);
        }
        Ok(id)
    }

    fn discard_realm(&mut self, realm: RealmId) -> Result<(), RealmError> {
        if realm == MAIN_REALM {
            return Err(RealmError::Refused(
                "the main realm is the agent's initial realm and cannot be discarded",
            ));
        }
        match self.registry.realms.borrow_mut().remove(&realm) {
            Some(_) => Ok(()),
            None => Err(RealmError::NoSuchRealm(realm)),
        }
    }

    fn eval_in_realm(&mut self, realm: RealmId, source: &str) -> Result<Self::Value, RealmError> {
        let src = source.to_string();
        self.with_realm(realm, |ctx| ctx.eval(Source::from_bytes(src.as_bytes())))?
            .map_err(|e| RealmError::Engine(format!("{e:?}")))
    }

    fn realm_global(&mut self, realm: RealmId) -> Result<Self::Value, RealmError> {
        // `Realm::global_object` is `pub(crate)` in Boa, so the global is only
        // reachable by *entering* the realm and asking the `Context` — which
        // reads the active call frame's realm and so answers for the realm we
        // just entered. Recorded rather than patched: the fork under
        // `Code/crates/boa` is outside this lane.
        self.with_realm(realm, |ctx| JsValue::from(ctx.global_object()))
    }

    fn set_global_in_realm(
        &mut self,
        realm: RealmId,
        name: &str,
        value: &Self::Value,
    ) -> Result<(), RealmError> {
        let name = JsString::from(name);
        let value = value.clone();
        self.with_realm(realm, |ctx| {
            let global = ctx.global_object();
            global.set(name, value, false, ctx)
        })?
        .map(|_| ())
        .map_err(|e| RealmError::Engine(format!("{e:?}")))
    }

    fn set_function_in_realm<F: NativeFn<Self>>(
        &mut self,
        realm: RealmId,
        name: &str,
        length: usize,
    ) -> Result<(), RealmError> {
        fn trampoline<F: NativeFn<BoaEngine>>(
            this: &JsValue,
            args: &[JsValue],
            ctx: &mut Context,
        ) -> JsResult<JsValue> {
            let mut cx = BoaCallCx {
                ctx,
                args,
                this: this.clone(),
            };
            F::call(&mut cx)
        }
        let name = JsString::from(name);
        self.with_realm(realm, |ctx| {
            ctx.register_global_builtin_callable(
                name,
                length,
                NativeFunction::from_fn_ptr(trampoline::<F>),
            )
        })?
        .map_err(|e| RealmError::Engine(format!("{e:?}")))
    }

    fn set_host_data_in_realm(&mut self, realm: RealmId, data: HostData) -> Result<(), RealmError> {
        match self.registry.realms.borrow().get(&realm) {
            Some(r) => {
                let hd = r.host_defined();
                let Some(slot) = hd.get::<RealmSlot>() else {
                    return Err(RealmError::Refused("realm carries no host slot"));
                };
                *slot.data.borrow_mut() = Some(data);
                Ok(())
            },
            None => Err(RealmError::NoSuchRealm(realm)),
        }
    }

    fn drain_dead_reflectors(&mut self) -> Vec<ReflectorData> {
        self.drain_dead_reflectors_in_realm(MAIN_REALM)
            .expect("main realm exists")
    }

    fn drain_dead_reflectors_in_realm(
        &mut self,
        realm: RealmId,
    ) -> Result<Vec<ReflectorData>, RealmError> {
        let target = self
            .registry
            .realms
            .borrow()
            .get(&realm)
            .cloned()
            .ok_or(RealmError::NoSuchRealm(realm))?;
        let previous = self.ctx.enter_realm(target);
        let out = (|| {
            // Real death-reporting: sweep the weak canonical cache and report (and
            // forget) the reflectors whose JS objects have been collected since the
            // last call. A rooted reflector (`root_reflectors`) cannot appear here:
            // its strong root keeps the weak upgradeable. Backed by the vendored boa patch (`JsObject::downgrade` /
            // `WeakJsObject::upgrade`). The host unpins each returned id, freeing the
            // underlying detached node for collection (G3).
            let mut dead = Vec::new();
            if let Some(cell) = self.ctx.realm().host_defined().get::<RealmSlot>() {
                cell.reflectors.borrow_mut().retain(|&data, weak| {
                    if weak.upgrade().is_some() {
                        true
                    } else {
                        dead.push(data);
                        false
                    }
                });
            }
            dead
        })();
        self.ctx.enter_realm(previous);
        Ok(out)
    }
}

impl ScriptEngineLive for BoaEngine {
    fn make_reflector(&mut self, data: ReflectorData) -> Result<Self::Value, Self::Error> {
        let obj: JsObject = Reflector::from_data(
            Reflector {
                data,
                owner: realm_id_of(&self.ctx),
            },
            &mut self.ctx,
        )?;
        Ok(obj.into())
    }

    fn reflector_data(&mut self, value: &Self::Value) -> Option<ReflectorData> {
        value
            .as_object()
            .and_then(|o| o.downcast_ref::<Reflector>().map(|r| r.data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflector_round_trip() {
        let mut engine = BoaEngine::new().unwrap();
        let v = engine.make_reflector(0xDEAD_BEEF).unwrap();
        assert_eq!(engine.reflector_data(&v), Some(0xDEAD_BEEF));
        // A non-reflector value yields None.
        let other = engine.eval("({})").unwrap();
        assert_eq!(engine.reflector_data(&other), None);
    }

    #[test]
    fn value_surface() {
        let mut engine = BoaEngine::new().unwrap();
        let v = engine.eval("'a' + (1 + 2)").unwrap();
        assert_eq!(engine.value_to_string(&v).unwrap(), "a3");
    }

    #[test]
    fn global_reflector_is_reachable_from_js() {
        let mut engine = BoaEngine::new().unwrap();
        let reflector = engine.make_reflector(0x1234).unwrap();
        engine.set_global("node", &reflector).unwrap();

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

        let mut engine = BoaEngine::new().unwrap();
        engine.set_host_data(sink.clone());

        // setText(node, text): recover the node id off the reflector arg, read the
        // text, and record both into host data — the JS→host write path.
        struct SetText;
        impl NativeFn<BoaEngine> for SetText {
            fn call(cx: &mut BoaCallCx<'_>) -> JsResult<JsValue> {
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
    fn host_promise_bridges_js_await() {
        let mut engine = BoaEngine::new().unwrap();

        // Resolve path: a parked `await` resumes when the host settles the promise.
        let (promise, token) = engine.new_host_promise().unwrap();
        engine.set_global("p", &promise).unwrap();
        engine
            .eval("globalThis.out = 'pending'; (async () => { globalThis.out = await p; })();")
            .unwrap();
        // Drain the script's own microtasks so the async fn reaches its parked `await`.
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

        // Double-settle is a silent no-op (the token was consumed), not an error.
        engine.settle_host_promise(token, Ok(&resolution)).unwrap();
    }

    #[test]
    fn reflector_for_reports_death_after_gc() {
        let mut engine = BoaEngine::new().unwrap();

        // A callback handing JS the *canonical* reflector for node 0x42.
        struct Canonical;
        impl NativeFn<BoaEngine> for Canonical {
            fn call(cx: &mut BoaCallCx<'_>) -> JsResult<JsValue> {
                cx.reflector_for(0x42)
            }
        }
        engine.set_function::<Canonical>("canonical", 0).unwrap();

        // Hold the reflector from JS: while reachable, no death is reported, and
        // the canonical identity holds (=== the same object).
        let same = engine
            .eval("globalThis.x = canonical(); globalThis.x === canonical()")
            .unwrap();
        assert_eq!(engine.value_to_string(&same).unwrap(), "true");
        assert!(engine.drain_dead_reflectors().is_empty());

        // Drop the last JS reference and collect: the weak cache reports the death.
        engine.eval("globalThis.x = null;").unwrap();
        boa_gc::force_collect();
        assert_eq!(engine.drain_dead_reflectors(), vec![0x42]);

        // The dead entry was swept, so a second drain is empty.
        assert!(engine.drain_dead_reflectors().is_empty());
    }

    #[test]
    fn pump_drains_fully_regardless_of_budget() {
        let mut engine = BoaEngine::new().unwrap();
        engine
            .eval("globalThis.n = 0; Promise.resolve().then(() => { globalThis.n++; });")
            .unwrap();
        // Boa cannot sub-drain, so even a tight `Steps` budget drains to quiescence.
        assert_eq!(engine.pump(Budget::Steps(1)), PumpOutcome::Quiescent);
        let n = engine.eval("n").unwrap();
        assert_eq!(engine.value_to_string(&n).unwrap(), "1");
    }

    // ---- Realms ------------------------------------------------------------
    //
    // The named regression set for the realm contract. Every case here has a
    // Nova twin in `script-engine-nova`; the pair is the both-engine gate,
    // because the contract's whole point is that a child browsing context works
    // the same on either backend.

    /// A realm has its own global object: a binding made in one is invisible in
    /// the other, in both directions.
    #[test]
    fn realms_have_separate_globals() {
        let mut engine = BoaEngine::new().unwrap();
        assert!(engine.supports_realms());
        let child = engine.create_realm().unwrap();
        assert_ne!(child, MAIN_REALM);

        engine.eval("globalThis.here = 'parent'").unwrap();
        engine
            .eval_in_realm(child, "globalThis.here = 'child'")
            .unwrap();

        let a = engine.eval("here").unwrap();
        assert_eq!(engine.value_to_string(&a).unwrap(), "parent");
        let b = engine.eval_in_realm(child, "here").unwrap();
        assert_eq!(engine.value_to_string(&b).unwrap(), "child");
        let missing = engine.eval("typeof globalThis.childOnly").unwrap();
        assert_eq!(engine.value_to_string(&missing).unwrap(), "undefined");
    }

    /// A realm has its own intrinsics: its `Object` is not the parent's, which
    /// is what makes `instanceof` cross-realm-false and is the reason a realm is
    /// the right unit for a browsing context rather than a fresh global alone.
    #[test]
    fn realms_have_separate_intrinsics() {
        let mut engine = BoaEngine::new().unwrap();
        let child = engine.create_realm().unwrap();
        let child_object = engine.eval_in_realm(child, "Object").unwrap();
        engine.set_global("childObject", &child_object).unwrap();
        let same = engine.eval("childObject === Object").unwrap();
        assert_eq!(engine.value_to_string(&same).unwrap(), "false");
    }

    /// The whole point of one agent: a value made in the child realm is an
    /// **ordinary reference** in the parent, with identity preserved across the
    /// boundary in both directions. No clone, no wire, no marshalling.
    #[test]
    fn objects_cross_realms_with_identity() {
        let mut engine = BoaEngine::new().unwrap();
        let child = engine.create_realm().unwrap();
        engine
            .eval_in_realm(child, "globalThis.thing = { tag: 'child-object' }")
            .unwrap();
        let handle = engine.eval_in_realm(child, "thing").unwrap();

        // Parent sees the child's actual object...
        engine.set_global("fromChild", &handle).unwrap();
        let tag = engine.eval("fromChild.tag").unwrap();
        assert_eq!(engine.value_to_string(&tag).unwrap(), "child-object");
        // ...and mutating it in the parent is visible to the child, which is
        // only true of a shared reference.
        engine.eval("fromChild.tag = 'touched-by-parent'").unwrap();
        let seen = engine.eval_in_realm(child, "thing.tag").unwrap();
        assert_eq!(engine.value_to_string(&seen).unwrap(), "touched-by-parent");
        // Identity, stated as the engine sees it.
        engine.set_global_in_realm(child, "back", &handle).unwrap();
        let identical = engine.eval_in_realm(child, "back === thing").unwrap();
        assert_eq!(engine.value_to_string(&identical).unwrap(), "true");
    }

    /// `realm_global` hands back the child's actual global — the primitive
    /// `contentWindow` is built from.
    #[test]
    fn realm_global_is_the_childs_own_global() {
        let mut engine = BoaEngine::new().unwrap();
        let child = engine.create_realm().unwrap();
        engine
            .eval_in_realm(child, "globalThis.marker = 41")
            .unwrap();
        let global = engine.realm_global(child).unwrap();
        engine.set_global("contentWindow", &global).unwrap();
        engine.eval("contentWindow.marker += 1").unwrap();
        let seen = engine.eval_in_realm(child, "marker").unwrap();
        assert_eq!(engine.value_to_string(&seen).unwrap(), "42");
        let is_global = engine.eval_in_realm(child, "globalThis").unwrap();
        engine.set_global("childGlobalThis", &is_global).unwrap();
        let same = engine.eval("contentWindow === childGlobalThis").unwrap();
        assert_eq!(engine.value_to_string(&same).unwrap(), "true");
    }

    /// A native function installed per realm reports that realm from inside the
    /// call, and reaches that realm's own host data. This is the per-realm host
    /// surface in miniature: the same `NativeFn` impl, two realms, two answers,
    /// and the callback never learns that realms exist.
    #[test]
    fn native_fn_sees_its_own_realm_and_host_data() {
        use std::cell::RefCell;
        use std::rc::Rc;

        type Seen = RefCell<Vec<RealmId>>;

        struct WhereAmI;
        impl NativeFn<BoaEngine> for WhereAmI {
            fn call(cx: &mut BoaCallCx<'_>) -> Result<JsValue, JsError> {
                let realm = cx.current_realm();
                let hits = cx
                    .host_data()
                    .and_then(|d| d.downcast::<Seen>().ok())
                    .map(|seen| {
                        seen.borrow_mut().push(realm);
                        seen.borrow().len()
                    })
                    .unwrap_or(0);
                cx.make_string(&format!("realm={realm} hits={hits}"))
            }
        }

        let mut engine = BoaEngine::new().unwrap();
        let parent_state: Rc<Seen> = Rc::new(RefCell::new(Vec::new()));
        let child_state: Rc<Seen> = Rc::new(RefCell::new(Vec::new()));
        engine.set_host_data(parent_state.clone());
        engine.set_function::<WhereAmI>("whereAmI", 0).unwrap();

        let child = engine.create_realm().unwrap();
        engine
            .set_host_data_in_realm(child, child_state.clone())
            .unwrap();
        engine
            .set_function_in_realm::<WhereAmI>(child, "whereAmI", 0)
            .unwrap();

        let a = engine.eval("whereAmI()").unwrap();
        assert_eq!(engine.value_to_string(&a).unwrap(), "realm=0 hits=1");
        let b = engine.eval_in_realm(child, "whereAmI()").unwrap();
        assert_eq!(
            engine.value_to_string(&b).unwrap(),
            format!("realm={child} hits=1")
        );
        // Two host states, one per realm; neither saw the other's call.
        assert_eq!(*parent_state.borrow(), vec![MAIN_REALM]);
        assert_eq!(*child_state.borrow(), vec![child]);
    }

    /// A reflector handed into the child realm is recoverable there: the
    /// reflector bridge is agent-wide, so a node crossing realms is still the
    /// same node to the host.
    #[test]
    fn reflectors_cross_realms() {
        let mut engine = BoaEngine::new().unwrap();
        let child = engine.create_realm().unwrap();
        let reflector = ScriptEngineLive::make_reflector(&mut engine, 0x99).unwrap();
        engine
            .set_global_in_realm(child, "node", &reflector)
            .unwrap();
        let back = engine.eval_in_realm(child, "node").unwrap();
        assert_eq!(engine.reflector_data(&back), Some(0x99));
    }

    /// Refusals are stated, not approximated: an unknown id and the main realm
    /// both say exactly what is wrong.
    #[test]
    fn realm_refusals_are_exact() {
        let mut engine = BoaEngine::new().unwrap();
        assert_eq!(
            engine.eval_in_realm(4242, "1").unwrap_err(),
            RealmError::NoSuchRealm(4242)
        );
        assert!(matches!(
            engine.discard_realm(MAIN_REALM),
            Err(RealmError::Refused(_))
        ));
        let child = engine.create_realm().unwrap();
        assert_eq!(engine.discard_realm(child), Ok(()));
        assert_eq!(
            engine.discard_realm(child).unwrap_err(),
            RealmError::NoSuchRealm(child)
        );
    }

    #[test]
    fn equal_raw_reflector_ids_are_isolated_and_rooted_per_realm() {
        struct Reflect;
        impl<E: ScriptEngine> NativeFn<E> for Reflect {
            fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
                cx.reflector_for(7)
            }
        }
        let mut engine = BoaEngine::new().unwrap();
        let child = engine.create_realm().unwrap();
        engine.set_function::<Reflect>("reflect", 0).unwrap();
        engine
            .set_function_in_realm::<Reflect>(child, "reflect", 0)
            .unwrap();
        engine
            .eval("globalThis.node = reflect(); node.marker = 'parent'")
            .unwrap();
        engine
            .eval_in_realm(child, "globalThis.node = reflect(); node.marker = 'child'")
            .unwrap();
        let child_node = engine.eval_in_realm(child, "node").unwrap();
        engine.set_global("childNode", &child_node).unwrap();
        let result = engine.eval("node !== childNode && reflect() === node && node.marker === 'parent' && childNode.marker === 'child'").unwrap();
        assert_eq!(engine.value_to_string(&result).unwrap(), "true");
        engine.root_reflectors(&[7]);
        engine.root_reflectors_in_realm(child, &[7]).unwrap();
        engine.unroot_reflectors(&[7]);
        assert_eq!(engine.rooted_reflector_count(), 0);
        assert_eq!(engine.rooted_reflector_count_in_realm(child).unwrap(), 1);
        engine.force_gc();
        assert!(
            engine
                .drain_dead_reflectors_in_realm(child)
                .unwrap()
                .is_empty()
        );
        assert_eq!(engine.minted_reflectors_in_realm(child).unwrap(), vec![7]);
        engine.unroot_reflectors_in_realm(child, &[7]).unwrap();
        assert_eq!(engine.rooted_reflector_count_in_realm(child).unwrap(), 0);
        assert_eq!(
            engine.minted_reflectors_in_realm(99999),
            Err(RealmError::NoSuchRealm(99999))
        );
    }

    #[test]
    fn child_host_promises_settle_without_parent_token_collisions() {
        use std::{cell::RefCell, rc::Rc};
        struct Deferred;
        impl<E: ScriptEngine> NativeFn<E> for Deferred {
            fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
                let (value, token) = cx.new_host_promise()?;
                let data = cx.host_data().unwrap();
                data.downcast_ref::<RefCell<Vec<u64>>>()
                    .unwrap()
                    .borrow_mut()
                    .push(token);
                Ok(value)
            }
        }
        let mut engine = BoaEngine::new().unwrap();
        let child = engine.create_realm().unwrap();
        let tokens = Rc::new(RefCell::new(Vec::<u64>::new()));
        engine.set_host_data(tokens.clone());
        engine
            .set_host_data_in_realm(child, tokens.clone())
            .unwrap();
        engine.set_function::<Deferred>("deferred", 0).unwrap();
        engine
            .set_function_in_realm::<Deferred>(child, "deferred", 0)
            .unwrap();
        engine
            .eval("globalThis.answer = 'pending'; deferred().then(v => answer = v)")
            .unwrap();
        engine
            .eval_in_realm(
                child,
                "globalThis.answer = 'pending'; deferred().then(v => answer = v)",
            )
            .unwrap();
        let ids = tokens.borrow().clone();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        let value = engine.eval("('child settled')").unwrap();
        assert_eq!(engine.value_to_string(&value).unwrap(), "child settled");
        engine.settle_host_promise(ids[1], Ok(&value)).unwrap();
        engine.pump_microtasks();
        let value = engine.eval("answer").unwrap();
        assert_eq!(engine.value_to_string(&value).unwrap(), "pending");
        let value = engine.eval_in_realm(child, "answer").unwrap();
        assert_eq!(engine.value_to_string(&value).unwrap(), "child settled");
        let value = engine.eval("('parent settled')").unwrap();
        assert_eq!(engine.value_to_string(&value).unwrap(), "parent settled");
        engine.settle_host_promise(ids[0], Ok(&value)).unwrap();
        engine.pump_microtasks();
        let value = engine.eval("answer").unwrap();
        assert_eq!(engine.value_to_string(&value).unwrap(), "parent settled");
    }

    #[test]
    fn uninitialized_child_never_inherits_parent_host_data() {
        use std::rc::Rc;
        struct Check;
        impl<E: ScriptEngine> NativeFn<E> for Check {
            fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
                let found = cx.host_data().is_some();
                cx.make_string(if found { "present" } else { "absent" })
            }
        }
        let mut engine = BoaEngine::new().unwrap();
        engine.set_host_data(Rc::new(17_u32));
        let child = engine.create_realm().unwrap();
        engine
            .set_function_in_realm::<Check>(child, "check", 0)
            .unwrap();
        let value = engine.eval_in_realm(child, "check()").unwrap();
        assert_eq!(engine.value_to_string(&value).unwrap(), "absent");
    }

    #[test]
    fn discarding_realm_does_not_revoke_retained_functions() {
        struct Marker;
        impl<E: ScriptEngine> NativeFn<E> for Marker {
            fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
                let id = cx.current_realm();
                cx.make_string(&id.to_string())
            }
        }
        let mut engine = BoaEngine::new().unwrap();
        let child = engine.create_realm().unwrap();
        engine
            .set_function_in_realm::<Marker>(child, "marker", 0)
            .unwrap();
        let function = engine.eval_in_realm(child, "() => marker()").unwrap();
        engine.set_global("retained", &function).unwrap();
        engine.discard_realm(child).unwrap();
        engine.force_gc();
        let value = engine.eval("retained()").unwrap();
        assert_eq!(engine.value_to_string(&value).unwrap(), child.to_string());
    }

    #[test]
    fn native_callback_creates_and_initializes_child_synchronously() {
        use std::{cell::RefCell, rc::Rc};
        struct Who;
        impl<E: ScriptEngine> NativeFn<E> for Who {
            fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
                let realm = cx.current_realm();
                cx.make_string(&realm.to_string())
            }
        }
        struct Spawn;
        impl<E: ScriptEngine> NativeFn<E> for Spawn {
            fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
                let parent = cx.current_realm();
                let data = cx.host_data().unwrap();
                let records = data.downcast_ref::<RefCell<Vec<RealmId>>>().unwrap();
                let mut global = None;
                let id = E::create_realm_from_call(cx, data.clone(), |child| {
                    E::set_function_from_call::<Who>(child, "who", 0)?;
                    E::eval_from_call(
                        child,
                        "globalThis.childRealm = who(); globalThis.obj = { answer: 42 }",
                    )?;
                    global = Some(E::eval_from_call(child, "globalThis")?);
                    Ok(())
                })
                .unwrap();
                records.borrow_mut().push(id);
                assert_eq!(cx.current_realm(), parent);
                Ok(global.unwrap())
            }
        }
        let mut engine = BoaEngine::new().unwrap();
        let records = Rc::new(RefCell::new(Vec::<RealmId>::new()));
        engine.set_host_data(records.clone());
        engine.set_function::<Spawn>("spawn", 0).unwrap();
        engine
            .eval("globalThis.child = spawn(); child.obj.answer += 1")
            .unwrap();
        let id = records.borrow()[0];
        let value = engine
            .eval_in_realm(id, "obj.answer === 43 && childRealm === who()")
            .unwrap();
        assert_eq!(engine.value_to_string(&value).unwrap(), "true");
        let value = engine.eval("typeof who").unwrap();
        assert_eq!(engine.value_to_string(&value).unwrap(), "undefined");
        let global = engine.realm_global(id).unwrap();
        engine.set_global("sameChild", &global).unwrap();
        let value = engine.eval("sameChild === child").unwrap();
        assert_eq!(engine.value_to_string(&value).unwrap(), "true");
    }

    #[test]
    fn direct_native_child_method_observes_caller_and_receiver() {
        struct Method;
        impl<E: ScriptEngine> NativeFn<E> for Method {
            fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
                let callee = cx.current_realm();
                let caller = cx.caller_realm();
                let receiver = cx.this_value();
                E::set_global_from_call(cx, "received", &receiver).unwrap();
                let function =
                    E::eval_from_call(cx, "(function(value) { return value; })").unwrap();
                let undefined = cx.undefined();
                let same = E::call_from_call(cx, &function, &undefined, &[receiver]).unwrap();
                E::set_global_from_call(cx, "receivedCall", &same).unwrap();
                E::eval_in_realm_from_call(cx, caller, "globalThis.calledBack = 'parent'").unwrap();
                assert_eq!(cx.current_realm(), callee);
                let _ = E::eval_in_realm_from_call(cx, caller, "throw new Error('restore')")
                    .err()
                    .expect("target throw");
                assert_eq!(cx.current_realm(), callee);
                cx.make_string(&format!("{callee}:{caller}"))
            }
        }
        let mut engine = BoaEngine::new().unwrap();
        let child = engine.create_realm().unwrap();
        engine
            .set_function_in_realm::<Method>(child, "method", 0)
            .unwrap();
        let global = engine.realm_global(child).unwrap();
        engine.set_global("child", &global).unwrap();
        let value = engine.eval("child.method()").unwrap();
        assert_eq!(
            engine.value_to_string(&value).unwrap(),
            format!("{child}:0")
        );
        let value = engine.eval("child.received === child && child.receivedCall === child && calledBack === 'parent' && typeof child.calledBack === 'undefined'").unwrap();
        assert_eq!(engine.value_to_string(&value).unwrap(), "true");
    }

    #[test]
    fn host_installed_globals_are_writable_configurable_and_deletable() {
        struct Install;
        impl<E: ScriptEngine> NativeFn<E> for Install {
            fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
                let value = cx.arg(0);
                E::set_global_from_call(cx, "callbackValue", &value)
                    .map_err(|error| cx.error(&error.to_string()))?;
                Ok(cx.undefined())
            }
        }
        let mut engine = BoaEngine::new().unwrap();
        let child = engine.create_realm().unwrap();
        let value = engine.eval("42").unwrap();
        engine.set_global("installedValue", &value).unwrap();
        engine
            .set_global_in_realm(child, "installedValue", &value)
            .unwrap();
        engine.set_function::<Install>("installValue", 1).unwrap();
        engine
            .set_function_in_realm::<Install>(child, "installValue", 1)
            .unwrap();
        let script = "installValue(42); ['installedValue','callbackValue'].every(function(name){var d=Object.getOwnPropertyDescriptor(globalThis,name); var flags=d.writable && d.enumerable && d.configurable; globalThis[name]=99; return flags && globalThis[name]===99 && delete globalThis[name] && !(name in globalThis);})";
        let value = engine.eval(script).unwrap();
        assert_eq!(engine.value_to_string(&value).unwrap(), "true");
        let value = engine.eval_in_realm(child, script).unwrap();
        assert_eq!(engine.value_to_string(&value).unwrap(), "true");
    }
    #[test]
    fn reflector_locality_uses_owner_not_raw_id() {
        type E = BoaEngine;
        struct Mint;
        impl NativeFn<E> for Mint {
            fn call(
                cx: &mut <E as ScriptEngine>::CallCx<'_>,
            ) -> Result<<E as ScriptEngine>::Value, <E as ScriptEngine>::Error> {
                cx.make_reflector(17)
            }
        }
        struct Canonical;
        impl NativeFn<E> for Canonical {
            fn call(
                cx: &mut <E as ScriptEngine>::CallCx<'_>,
            ) -> Result<<E as ScriptEngine>::Value, <E as ScriptEngine>::Error> {
                cx.reflector_for(17)
            }
        }
        struct Local;
        impl NativeFn<E> for Local {
            fn call(
                cx: &mut <E as ScriptEngine>::CallCx<'_>,
            ) -> Result<<E as ScriptEngine>::Value, <E as ScriptEngine>::Error> {
                let value = cx.arg(0);
                let local = cx.reflector_is_local(&value);
                let raw = cx.reflector_data(&value);
                cx.make_string(&format!("{local}:{raw:?}"))
            }
        }
        let mut engine = E::new().unwrap();
        let child = engine.create_realm().unwrap();
        engine.set_function::<Mint>("mint", 0).unwrap();
        engine.set_function::<Canonical>("canonical", 0).unwrap();
        engine.set_function::<Local>("local", 1).unwrap();
        engine
            .set_function_in_realm::<Mint>(child, "mint", 0)
            .unwrap();
        engine
            .set_function_in_realm::<Canonical>(child, "canonical", 0)
            .unwrap();
        engine
            .set_function_in_realm::<Local>(child, "local", 1)
            .unwrap();
        let global = engine.realm_global(child).unwrap();
        engine.set_global("child", &global).unwrap();
        let result = engine.eval("var a=mint(), b=child.mint(), c=canonical(), d=child.canonical(); Object.setPrototypeOf(b,null); local(a)==='true:Some(17)' && local(b)==='false:Some(17)' && child.local(a)==='false:Some(17)' && child.local(b)==='true:Some(17)' && local(c)==='true:Some(17)' && local(d)==='false:Some(17)' && child.local(c)==='false:Some(17)' && child.local(d)==='true:Some(17)' && local({})==='false:None'").unwrap();
        assert_eq!(engine.value_to_string(&result).unwrap(), "true");
        engine.force_gc();
        let result = engine.eval("local(a)==='true:Some(17)' && local(b)==='false:Some(17)' && child.local(a)==='false:Some(17)' && child.local(b)==='true:Some(17)' && local(c)==='true:Some(17)' && child.local(d)==='true:Some(17)'").unwrap();
        assert_eq!(engine.value_to_string(&result).unwrap(), "true");
    }
    #[test]
    fn callback_reflector_selection_preserves_identity_and_roots() {
        type E = BoaEngine;
        struct Select;
        impl NativeFn<E> for Select {
            fn call(
                cx: &mut <E as ScriptEngine>::CallCx<'_>,
            ) -> Result<<E as ScriptEngine>::Value, <E as ScriptEngine>::Error> {
                let value = cx.arg(0);
                let realm = cx.value_to_string(&value)?.parse::<RealmId>().unwrap();
                let value = cx.arg(1);
                let mode = cx.value_to_string(&value)?;
                let current = cx.current_realm();
                let result = match mode.as_str() {
                    "root" => cx
                        .root_reflector_in_realm(realm, 91)
                        .map(|()| cx.undefined()),
                    "release" => cx
                        .unroot_reflector_in_realm(realm, 91)
                        .map(|()| cx.undefined()),
                    _ => cx.reflector_for_in_realm(realm, 91),
                };
                assert_eq!(cx.current_realm(), current);
                match result {
                    Ok(value) => Ok(value),
                    Err(RealmError::NoSuchRealm(id)) => cx.make_string(&format!("missing:{id}")),
                    Err(error) => Err(cx.error(&error.to_string())),
                }
            }
        }
        let mut engine = E::new().unwrap();
        let child = engine.create_realm().unwrap();
        engine.set_function::<Select>("select", 2).unwrap();
        engine
            .set_function_in_realm::<Select>(child, "select", 2)
            .unwrap();
        let global = engine.realm_global(child).unwrap();
        engine.set_global("child", &global).unwrap();
        drop(global);
        let script = format!(
            "var original=child.select({child},'get'); original.marker=73; var other=select(0,'get'); select({child},'root'); select({child},'root'); select({child},'get')===original && child.select(0,'get')===other && other!==original && select(4294967295,'get')==='missing:4294967295' && select(4294967295,'root')==='missing:4294967295' && select(4294967295,'release')==='missing:4294967295'"
        );
        let result = engine.eval(&script).unwrap();
        assert_eq!(engine.value_to_string(&result).unwrap(), "true");
        drop(result);
        assert_eq!(engine.rooted_reflector_count_in_realm(child).unwrap(), 1);
        assert_eq!(
            engine.rooted_reflector_count_in_realm(MAIN_REALM).unwrap(),
            0
        );
        engine.eval("original=null; other=null").unwrap();
        engine.force_gc();
        engine.force_gc();
        assert!(
            engine
                .drain_dead_reflectors_in_realm(child)
                .unwrap()
                .is_empty()
        );
        let result = engine
            .eval(&format!("select({child},'get').marker===73"))
            .unwrap();
        assert_eq!(engine.value_to_string(&result).unwrap(), "true");
        drop(result);
        engine
            .eval(&format!(
                "select({child},'release'); select({child},'release');"
            ))
            .unwrap();
        assert_eq!(engine.rooted_reflector_count_in_realm(child).unwrap(), 0);
        engine.force_gc();
        engine.force_gc();
        assert_eq!(
            engine.drain_dead_reflectors_in_realm(child).unwrap(),
            vec![91]
        );
        engine.discard_realm(child).unwrap();
        let result = engine.eval(&format!("select({child},'get')==='missing:{child}' && select({child},'root')==='missing:{child}' && select({child},'release')==='missing:{child}'")).unwrap();
        assert_eq!(engine.value_to_string(&result).unwrap(), "true");
    }
    #[test]
    fn host_calls_retained_child_function_without_global_lookup() {
        let mut engine = BoaEngine::new().unwrap();
        let child = engine.create_realm().unwrap();
        let function = engine.eval_in_realm(child,
            "globalThis.label = 'child'; (function (value) { return label + ':' + this.tag + ':' + value; })"
        ).unwrap();
        let receiver = engine.eval("({tag: 'receiver'})").unwrap();
        let argument = engine.eval("'argument'").unwrap();
        engine.eval("globalThis.label = 'parent'").unwrap();
        engine.force_gc();
        let result = engine
            .call_function(&function, &receiver, &[argument])
            .unwrap();
        assert_eq!(
            engine.value_to_string(&result).unwrap(),
            "child:receiver:argument"
        );
        assert!(engine.call_function(&receiver, &receiver, &[]).is_err());
        let throwing = engine
            .eval_in_realm(
                child,
                "(function () { throw new TypeError('host failure'); })",
            )
            .unwrap();
        assert!(matches!(
            engine.call_function(&throwing, &receiver, &[]),
            Err(RealmError::Engine(_))
        ));
        let parent = engine.eval("label").unwrap();
        assert_eq!(engine.value_to_string(&parent).unwrap(), "parent");
    }
}
