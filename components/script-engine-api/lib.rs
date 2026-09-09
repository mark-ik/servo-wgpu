// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Engine-neutral scripting backend contract for genet's scripted tier.
//!
//! Lifted from the Track A (Boa) and Track B (Nova) validation probes — see
//! `docs/2026-05-20_genet_script_engine_plan.md`. The two findings the probes
//! proved are baked into the shape here:
//!
//! - The value surface (`eval`, value→string) and a native-data reflector are
//!   expressible with engine-native types (`JsValue`/`Context`, `Value`/`Agent`,
//!   `EmbedderObject`) fully confined to the backend crates.
//! - Reflector data is recovered by *value extraction* ([`reflector_data`]), not by
//!   invoking a JS method — because Nova's public API can't call a held function, so
//!   the cross-engine bridge has to read native data off a value directly.
//!
//! Backend selection is per-target: **Nova** native (primary), **Boa** on wasm32
//! (Nova is 64-bit-bound, Appendix B). Engines implement these traits; consumers
//! (`genet-scripted-dom`) drive them.

use std::any::Any;
use std::rc::Rc;

/// JS-opaque native data a reflector carries, bridging a JS object back to the host
/// DOM. Packs a genet `NodeId` (the DOM crate owns the `NodeId` ↔ `u64` mapping;
/// this crate stays DOM-neutral).
pub type ReflectorData = u64;

/// Opaque token for a pending host-created ("deferred") promise. Neutral, like
/// [`ReflectorData`]: the engine keeps the real resolve/reject machinery in its own
/// side table keyed by this token, so the neutral host layer can hold and pass the
/// token across the boundary without naming an engine type.
///
/// This is the async-host bridge. A native callback (`fetch`, `callModel`) mints a
/// pending promise with [`CallCx::new_host_promise`], returns it so JS can `await`,
/// and stashes the token in host state; when the backing Rust future completes, the
/// host calls [`ScriptEngine::settle_host_promise`] and then drains the reaction
/// jobs with [`ScriptEngine::pump_microtasks`]. Without it the trait can only *drain*
/// the job queue ([`pump_microtasks`]), never *create* a promise the host resolves.
///
/// [`pump_microtasks`]: ScriptEngine::pump_microtasks
pub type PromiseToken = u64;

/// A bound on how much work one [`ScriptEngine::pump`] call may do, as a runaway
/// guard. The unit is **coarse and backend-defined** (Nova counts reaction *jobs*, a
/// fuel-metered VM counts VM *steps*), so `Steps(n)` is a "stop a script that never
/// settles" cap, not precise accounting. `Unbounded` drains to quiescence (the
/// classic microtask checkpoint).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Budget {
    /// Drain the queue fully (a job storm can hang the caller; use only on trusted
    /// scripts or after their work is known-bounded).
    Unbounded,
    /// Run at most this many coarse steps, then return control even if work remains.
    Steps(u64),
}

/// The result of a [`ScriptEngine::pump`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpOutcome {
    /// The queue drained; no pending microtask work remains.
    Quiescent,
    /// The [`Budget`] was exhausted with work still pending. Call `pump` again to make
    /// more progress (a runaway script keeps returning this, which is how the caller
    /// detects and abandons it).
    Pending,
}

/// Opaque, engine-assigned identity for a **realm** inside one engine instance.
///
/// A realm is ECMAScript's unit of global object + intrinsics. HTML gives one
/// realm to each browsing context and one *agent* (one event loop, one heap) to
/// a group of same-origin-domain contexts that can reach each other
/// synchronously. That is why a child frame is a second realm rather than a
/// second engine: two engine instances are two agents, and this stack's
/// cross-instance boundary marshals strings, which cannot carry the object
/// identity `iframe.contentWindow.document.getElementById(x)` requires.
///
/// Ids are per-engine and are not reused while the realm is live. Values are
/// **agent-wide**, not realm-scoped: a [`ScriptEngine::Value`] obtained from one
/// realm is an ordinary reference usable in another realm of the same engine, so
/// "hold a handle to another realm's objects" needs no marshalling API — see
/// [`realm_global`](ScriptEngine::realm_global).
pub type RealmId = u32;

/// The realm every engine has from [`ScriptEngine::new`]: the agent's initial
/// realm, and the one every non-realm-suffixed method on the trait acts on.
pub const MAIN_REALM: RealmId = 0;

/// Why a realm operation could not be performed.
///
/// Deliberately **not** `ScriptEngine::Error`: the defaults below have to be
/// constructible without an engine, and a backend that cannot do a piece must be
/// able to say so precisely rather than approximate it. A refusal recorded here
/// is a fact about the backend, not a runtime failure to retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealmError {
    /// This backend has no realm concept at all (the trait default).
    Unsupported,
    /// This backend has realms but cannot perform *this* operation, with the
    /// exact reason. Used where an engine's own API forbids the shape rather
    /// than where the operation merely failed.
    Refused(&'static str),
    /// The id does not name a live realm of this engine.
    NoSuchRealm(RealmId),
    /// The engine raised an error; its `describe_error` rendering.
    Engine(String),
}

impl core::fmt::Display for RealmError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unsupported => write!(f, "this backend has no realms"),
            Self::Refused(why) => write!(f, "realm operation refused: {why}"),
            Self::NoSuchRealm(id) => write!(f, "no such realm: {id}"),
            Self::Engine(msg) => write!(f, "engine error: {msg}"),
        }
    }
}

/// Host state shared with native callbacks. A refcounted `Any` the host downcasts
/// (typically `Rc<RefCell<…>>` over the live DOM). Engine-neutral: each backend
/// stashes it in its own host-defined-data slot (Nova realm `[[HostDefined]]`, Boa
/// `Context` host data), never a `thread_local`. The host reaches it inside a
/// callback via [`CallCx::host_data`].
pub type HostData = Rc<dyn Any>;

/// A JavaScript VM instance. Engine-native value/context/callback types deliberately
/// do **not** appear on this trait — they live inside each backend crate.
pub trait ScriptEngine: Sized {
    /// A handle to a JS value. For engines whose values are GC-scoped (Nova), this is
    /// a *rooted* handle so it can be held across calls; for others it is the native
    /// value type (Boa `JsValue`).
    type Value: 'static;
    type Error: core::fmt::Debug;

    /// The per-call context a native callback receives ([`CallCx`]). It is a
    /// separate surface from `&mut Self` because, inside a callback, the VM is
    /// mid-execution (Nova: holding an `Agent` + `GcScope`), so the full engine
    /// API is not reachable. Carries one lifetime; backends with multi-lifetime
    /// internals (Nova's `GcScope`) collapse them onto it.
    type CallCx<'a>: CallCx<Value = Self::Value, Error = Self::Error>
    where
        Self: 'a;

    /// Construct a fresh engine with an empty global scope.
    fn new() -> Result<Self, Self::Error>;

    /// Evaluate `source` in the global scope, returning its completion value.
    fn eval(&mut self, source: &str) -> Result<Self::Value, Self::Error>;

    /// Evaluate `source` as an ECMAScript **module** (module scope, strict mode,
    /// `import` / `export`), driving its load → link → evaluate to completion.
    ///
    /// `base_url` is the entry module's own URL (its `import` specifiers resolve
    /// against it). `resolve` is the host module resolver: given an import
    /// `(specifier, referrer_url)` it returns the imported module's
    /// `(resolved_url, source)`, or `None` if it cannot be resolved/fetched — the
    /// seam through which the host (which owns the fetcher) supplies dependency
    /// source on demand. The engine keys its module cache on `resolved_url`, so a
    /// diamond or cycle resolves each module once.
    ///
    /// Returns `Ok(Some(value))` on success, `Err` if the module (or a dependency)
    /// throws or fails to load, and `Ok(None)` when this backend does not support
    /// module evaluation — the default, so a backend without module support (or one
    /// that has not wired it yet) degrades gracefully rather than failing to compile.
    fn eval_module(
        &mut self,
        _source: &str,
        _base_url: &str,
        _resolve: &mut dyn FnMut(&str, &str) -> Option<(String, String)>,
    ) -> Result<Option<Self::Value>, Self::Error> {
        Ok(None)
    }

    /// Like [`eval`](Self::eval), but bounded: a [`Budget::Steps`] cap stops a
    /// runaway script (e.g. `while true do end`) after roughly that many
    /// coarse VM steps and returns an error instead of hanging. The cap is on
    /// the *main* evaluation, complementing [`pump`](Self::pump)'s cap on
    /// microtask jobs.
    ///
    /// The default ignores the budget and runs [`eval`](Self::eval) unbounded
    /// — correct for backends whose VM cannot be step-metered (Boa). A
    /// fuel-metered backend (piccolo) overrides this; an untrusted-script host
    /// should prefer this method with a [`Budget::Steps`] bound on those
    /// backends.
    fn eval_bounded(&mut self, source: &str, _budget: Budget) -> Result<Self::Value, Self::Error> {
        self.eval(source)
    }

    /// Coerce a value to a Rust string (`ToString`).
    fn value_to_string(&mut self, value: &Self::Value) -> Result<String, Self::Error>;

    /// Render an error to a diagnostic string — the thrown value's `toString`, e.g.
    /// `"TypeError: …"`. The test262 runner uses it to match a `negative:` test's
    /// expected error type. The default uses `Debug`; a backend whose `Error` is
    /// opaque (Boa's `JsError`) overrides it to stringify through the engine.
    fn describe_error(&mut self, error: &Self::Error) -> String {
        format!("{error:?}")
    }

    /// Define `name` on the global object with `value`. The primitive the host
    /// (runtime layer) installs globals from: reflectors (`node`), and later the
    /// browser host objects (`self`, `document`, `addEventListener`, …). Native
    /// callbacks ride a separate primitive (see the plan's `new_function`), because
    /// how a callback reaches host state is engine-specific.
    fn set_global(&mut self, name: &str, value: &Self::Value) -> Result<(), Self::Error>;

    /// Stash host state reachable from native callbacks via [`CallCx::host_data`].
    /// Replaces an existing slot of the same shape; the host is expected to set it
    /// once before running script.
    fn set_host_data(&mut self, data: HostData);

    /// Install `name` on the global as a native function backed by `F`, with arity
    /// `length`. `F` is a zero-sized type, not a closure: both backends register a
    /// bare `fn` pointer (Nova `RegularFn`, Boa `NativeFunctionPointer`), so the
    /// callback is monomorphized per `F` (a distinct trampoline) and captures
    /// nothing. State reaches the callback through [`CallCx::host_data`] and the
    /// reflector arguments, not captures.
    fn set_function<F: NativeFn<Self>>(
        &mut self,
        name: &str,
        length: usize,
    ) -> Result<(), Self::Error>;

    /// Run pending microtasks (Promise reaction jobs) up to `budget`, including jobs
    /// enqueued while running. Returns [`PumpOutcome::Quiescent`] if the queue drained
    /// or [`PumpOutcome::Pending`] if `budget` ran out with work remaining. The host
    /// calls this at task boundaries (after the initial script, between timer tasks) so
    /// Promise continuations resolve; passing a [`Budget::Steps`] bound lets it cap a
    /// runaway script instead of hanging. Errors thrown by a job are swallowed (an
    /// unhandled rejection is not the host's failure).
    ///
    /// Backends honor the budget to the degree their job machinery allows: a
    /// fuel-metered VM bounds by VM step, Nova bounds by job count, and Boa drains
    /// fully (its `SimpleJobExecutor` has no sub-drain) and so always returns
    /// `Quiescent`. The contract a caller can rely on everywhere is "`Quiescent` means
    /// done"; only `Steps`-honoring backends return `Pending`.
    fn pump(&mut self, budget: Budget) -> PumpOutcome;

    /// Drain pending microtasks to quiescence: [`pump`] with [`Budget::Unbounded`]. The
    /// common case at a task boundary; callers that must not hang on a runaway script
    /// use [`pump`] with a [`Budget::Steps`] bound and loop on [`PumpOutcome::Pending`].
    ///
    /// [`pump`]: ScriptEngine::pump
    fn pump_microtasks(&mut self) {
        let _ = self.pump(Budget::Unbounded);
    }

    /// Mint a pending ("deferred") promise at the engine level: the between-tasks
    /// mirror of [`CallCx::new_host_promise`], for a promise the host installs (as a
    /// global, say) before running script. Returns the JS promise value and a
    /// [`PromiseToken`] to settle it later with [`settle_host_promise`].
    ///
    /// [`settle_host_promise`]: ScriptEngine::settle_host_promise
    fn new_host_promise(&mut self) -> Result<(Self::Value, PromiseToken), Self::Error>;

    /// Settle a pending host promise: resolve it with `Ok(value)` or reject it with
    /// `Err(error)`. This enqueues the promise's reaction jobs but does not run them;
    /// the host drains them with [`pump_microtasks`]. The token is consumed: a token
    /// not in the table (already settled, or never minted here) is a silent no-op, so
    /// double-settle is safe. The async-host bridge's resolving half — call it when
    /// the Rust future behind a [`new_host_promise`] completes.
    ///
    /// [`pump_microtasks`]: ScriptEngine::pump_microtasks
    /// [`new_host_promise`]: ScriptEngine::new_host_promise
    fn settle_host_promise(
        &mut self,
        token: PromiseToken,
        outcome: Result<&Self::Value, &Self::Value>,
    ) -> Result<(), Self::Error>;

    /// Report the reflectors whose JS objects have been collected since the last
    /// call, returning their [`ReflectorData`] and forgetting them from the
    /// canonical-reflector cache. The host drains this at the same cadence as
    /// [`pump_microtasks`] and unpins each returned id from its reflector-pin
    /// table, so a detached ("orphaned") node whose last JS reference has died
    /// becomes collectable (the prerequisite for the gc-arena refit, G3).
    ///
    /// The **default is the fallback / epoch-pin mode**: it reports nothing, so
    /// reflector-held ids stay pinned until document teardown. That is today's
    /// behavior and the correct mode for a backend whose GC cannot report object
    /// deaths; navigation-bounded documents lose nothing by it. A backend that
    /// *can* observe deaths (a weakly-held canonical cache) overrides this.
    ///
    /// [`pump_microtasks`]: ScriptEngine::pump_microtasks
    fn drain_dead_reflectors(&mut self) -> Vec<ReflectorData> {
        Vec::new()
    }

    /// Force a full collection of the engine heap, so that
    /// [`drain_dead_reflectors`](Self::drain_dead_reflectors) can observe the deaths of
    /// reflector wrappers script no longer references. The runtime calls this at the GC
    /// tick (`Runtime::collect_garbage`) — a deliberate, not-per-microtask cadence —
    /// immediately before draining. The default is a no-op: an engine in the epoch-pin
    /// fallback (whose drain reports nothing), or one whose GC cannot be forced, loses
    /// nothing by it. A backend with real death-reporting overrides this to drive its
    /// collector, so a just-orphaned node is reaped that same tick (the gc-arena soak's
    /// frame-cadence contract).
    fn force_gc(&mut self) {}

    /// Every [`ReflectorData`] the canonical-reflector cache currently holds an
    /// entry for — the set of nodes script has been handed an object for. The
    /// opaque-root policy iterates this at the GC tick to decide which reflectors
    /// the host roots (see [`root_reflectors`]). Order is unspecified. Default
    /// empty: a backend with no cache has nothing to police.
    ///
    /// [`root_reflectors`]: ScriptEngine::root_reflectors
    fn minted_reflectors(&mut self) -> Vec<ReflectorData> {
        Vec::new()
    }

    /// Take a **strong** engine root on each named reflector, so the collector
    /// cannot take it while the root is held; minting the reflector if the cache
    /// has no live entry. Idempotent per id.
    ///
    /// This is the engine half of the opaque-root policy (see the
    /// reflector-identity plan). The host pin table pins the *node*; nothing
    /// pinned the *reflector*, so the (reflector, wrapper) pair was collectable
    /// while the node was still attached to a live document, and the next handoff
    /// minted a blank wrapper in place of the one carrying the node's listeners
    /// and other JS-side state. A platform object retains its associated realm, so a reachable node's
    /// wrapper identity must be stable
    /// across collections; a rooted reflector is what makes it so.
    ///
    /// [`drain_dead_reflectors`] must never report a rooted id.
    ///
    /// [`drain_dead_reflectors`]: ScriptEngine::drain_dead_reflectors
    fn root_reflectors(&mut self, _data: &[ReflectorData]) {}

    /// Release the roots taken by [`root_reflectors`](Self::root_reflectors). Each
    /// reflector becomes collectable again once script drops its own references,
    /// and `drain_dead_reflectors` resumes reporting its death. Idempotent.
    fn unroot_reflectors(&mut self, _data: &[ReflectorData]) {}

    /// How many reflectors are currently held by an engine root. A diagnostic
    /// readout, not a control: the gc-arena soak asserts on it (live wrappers stay
    /// bounded by the reachable touched nodes) and the reflector-identity
    /// regression tests assert the root is actually taken and actually released.
    /// Default 0 — a backend with no rooting.
    fn rooted_reflector_count(&mut self) -> usize {
        0
    }

    /// Realm-scoped counterpart of [`Self::minted_reflectors`]. IDs are local to the realm's host arena.
    fn minted_reflectors_in_realm(
        &mut self,
        realm: RealmId,
    ) -> Result<Vec<ReflectorData>, RealmError> {
        if realm == MAIN_REALM {
            Ok(self.minted_reflectors())
        } else {
            Err(RealmError::Unsupported)
        }
    }

    /// Realm-scoped counterpart of [`Self::root_reflectors`]. IDs are local to the realm's host arena.
    fn root_reflectors_in_realm(
        &mut self,
        realm: RealmId,
        data: &[ReflectorData],
    ) -> Result<(), RealmError> {
        if realm == MAIN_REALM {
            Ok(self.root_reflectors(data))
        } else {
            Err(RealmError::Unsupported)
        }
    }

    /// Realm-scoped counterpart of [`Self::unroot_reflectors`]. IDs are local to the realm's host arena.
    fn unroot_reflectors_in_realm(
        &mut self,
        realm: RealmId,
        data: &[ReflectorData],
    ) -> Result<(), RealmError> {
        if realm == MAIN_REALM {
            Ok(self.unroot_reflectors(data))
        } else {
            Err(RealmError::Unsupported)
        }
    }

    /// Realm-scoped counterpart of [`Self::rooted_reflector_count`]. IDs are local to the realm's host arena.
    fn rooted_reflector_count_in_realm(&mut self, realm: RealmId) -> Result<usize, RealmError> {
        if realm == MAIN_REALM {
            Ok(self.rooted_reflector_count())
        } else {
            Err(RealmError::Unsupported)
        }
    }

    /// Realm-scoped counterpart of [`Self::drain_dead_reflectors`]. IDs are local to the realm's host arena.
    fn drain_dead_reflectors_in_realm(
        &mut self,
        realm: RealmId,
    ) -> Result<Vec<ReflectorData>, RealmError> {
        if realm == MAIN_REALM {
            Ok(self.drain_dead_reflectors())
        } else {
            Err(RealmError::Unsupported)
        }
    }

    // ---- Realms ------------------------------------------------------------
    //
    // One engine instance is one **agent**: one heap, one event loop, one
    // microtask queue, one job of `pump`. A realm inside it is one global object
    // and one set of intrinsics. HTML's same-origin frame tree is exactly that
    // shape — shared agent, one realm per browsing context — and it is the only
    // shape in which a parent's `iframe.contentWindow.document.getElementById(x)`
    // can return the node the child's own script sees, because both sides then
    // hold ordinary references into one heap.
    //
    // Every method here defaults to [`RealmError::Unsupported`]. A backend
    // without realms therefore compiles unchanged and *says so* when asked,
    // rather than silently answering from the main realm — which would convert
    // a missing feature into a plausible wrong answer.

    /// Create and initialize a realm synchronously inside a native callback.
    /// Installation runs with the child's execution context active. The parent
    /// callback resumes afterwards; returned values remain ordinary heap references.
    fn create_realm_from_call(
        _cx: &mut Self::CallCx<'_>,
        _data: HostData,
        _initialize: impl for<'a> FnOnce(&mut Self::CallCx<'a>) -> Result<(), RealmError>,
    ) -> Result<RealmId, RealmError> {
        Err(RealmError::Unsupported)
    }

    /// Evaluate in the active native callback's realm, including during initialization.
    fn eval_from_call(
        _cx: &mut Self::CallCx<'_>,
        _source: &str,
    ) -> Result<Self::Value, RealmError> {
        Err(RealmError::Unsupported)
    }

    /// Install a native function in the active callback's realm.
    fn set_function_from_call<F: NativeFn<Self>>(
        _cx: &mut Self::CallCx<'_>,
        _name: &str,
        _length: usize,
    ) -> Result<(), RealmError> {
        Err(RealmError::Unsupported)
    }

    /// Evaluate a script in another live realm during a native callback.
    /// Script execution enters the target realm and restores the caller afterwards.
    fn eval_in_realm_from_call(
        _cx: &mut Self::CallCx<'_>,
        _realm: RealmId,
        _source: &str,
    ) -> Result<Self::Value, RealmError> {
        Err(RealmError::Unsupported)
    }

    /// Call an engine function without publishing its arguments on a global.
    fn call_from_call(
        _cx: &mut Self::CallCx<'_>,
        _function: &Self::Value,
        _this: &Self::Value,
        _args: &[Self::Value],
    ) -> Result<Self::Value, RealmError> {
        Err(RealmError::Unsupported)
    }

    /// Fetch an existing realm's actual global while inside a native callback.
    fn realm_global_from_call(
        _cx: &mut Self::CallCx<'_>,
        _realm: RealmId,
    ) -> Result<Self::Value, RealmError> {
        Err(RealmError::Unsupported)
    }

    /// Install a value in the current callback realm's global object.
    fn set_global_from_call(
        _cx: &mut Self::CallCx<'_>,
        _name: &str,
        _value: &Self::Value,
    ) -> Result<(), RealmError> {
        Err(RealmError::Unsupported)
    }

    /// Whether this backend can create realms at all. Cheap to ask, so a host
    /// can choose a degraded path before it starts building one.
    fn supports_realms(&self) -> bool {
        false
    }

    /// Create a fresh realm inside this engine: its own global object and its
    /// own intrinsics, sharing the engine's heap, job queue and host hooks.
    fn create_realm(&mut self) -> Result<RealmId, RealmError> {
        Err(RealmError::Unsupported)
    }

    /// Drop a realm created by [`create_realm`](Self::create_realm), making its
    /// global and intrinsics collectable. [`MAIN_REALM`] cannot be discarded.
    fn discard_realm(&mut self, _realm: RealmId) -> Result<(), RealmError> {
        Err(RealmError::Unsupported)
    }

    /// Evaluate `source` in `realm`'s global scope. The realm-scoped
    /// [`eval`](Self::eval); the returned value is usable from any realm of this
    /// engine.
    fn eval_in_realm(&mut self, _realm: RealmId, _source: &str) -> Result<Self::Value, RealmError> {
        Err(RealmError::Unsupported)
    }

    /// `realm`'s global object, as a value the caller may hold and hand to
    /// another realm. This is the primitive `contentWindow` is built from: a
    /// same-origin parent receives the child's actual global, not a copy.
    fn realm_global(&mut self, _realm: RealmId) -> Result<Self::Value, RealmError> {
        Err(RealmError::Unsupported)
    }

    /// [`set_global`](Self::set_global), scoped to `realm`.
    fn set_global_in_realm(
        &mut self,
        _realm: RealmId,
        _name: &str,
        _value: &Self::Value,
    ) -> Result<(), RealmError> {
        Err(RealmError::Unsupported)
    }

    /// [`set_function`](Self::set_function), scoped to `realm`. The same
    /// captures-free trampoline; only the global it lands on differs.
    fn set_function_in_realm<F: NativeFn<Self>>(
        &mut self,
        _realm: RealmId,
        _name: &str,
        _length: usize,
    ) -> Result<(), RealmError> {
        Err(RealmError::Unsupported)
    }

    /// [`set_host_data`](Self::set_host_data), scoped to `realm`.
    ///
    /// This is what makes a per-realm host surface possible **without** rekeying
    /// the host's own state: a child realm gets its own [`HostData`], so every
    /// native sink that reaches state through [`CallCx::host_data`] lands on the
    /// child's document, history and markup with no call-site change. The realm
    /// is the key, and the engine already knows which realm it is in.
    fn set_host_data_in_realm(
        &mut self,
        _realm: RealmId,
        _data: HostData,
    ) -> Result<(), RealmError> {
        Err(RealmError::Unsupported)
    }
}

/// Engines that can clone an idle VM heap into a fresh, independently-owned
/// runtime image.
///
/// This is a throughput hook for harness/runtime pools. It is intentionally
/// optional: backends without precise heap cloning keep using `ScriptEngine::new`.
pub trait ScriptEngineSnapshot: ScriptEngine {
    /// Clone the current idle engine state.
    ///
    /// The clone must receive fresh host-owned state before script runs. Backends
    /// should return an error if pending host jobs or active execution make the
    /// snapshot unsafe.
    fn snapshot_clone(&mut self) -> Result<Self, Self::Error>;
}

/// A native (Rust) callback exposed to JS, implemented by a zero-sized type so the
/// backend can monomorphize a captures-free trampoline per callback. Written once
/// against [`CallCx`]; the same `impl` drives every backend.
pub trait NativeFn<E: ScriptEngine> {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error>;
}

/// What a native callback can do while the VM is mid-call: read its arguments,
/// reach host state, convert and build values. Distinct from [`ScriptEngine`]
/// because the full engine is not reachable from inside a call.
pub trait CallCx {
    type Value;
    type Error;

    /// Construct a native error from a host failure. This is distinct from a
    /// platform DOMException, which bindings construct with its specified name.
    fn error(&mut self, message: &str) -> Self::Error;

    /// The `i`th argument, or undefined if absent.
    fn arg(&mut self, i: usize) -> Self::Value;

    /// Host state set by [`ScriptEngine::set_host_data`], or `None` if unset.
    fn host_data(&self) -> Option<HostData>;

    /// Coerce a value to a Rust string (`ToString`).
    fn value_to_string(&mut self, value: &Self::Value) -> Result<String, Self::Error>;

    /// Recover reflector native data (the JS → host bridge), or `None` if `value`
    /// is not a reflector.
    fn reflector_data(&mut self, value: &Self::Value) -> Option<ReflectorData>;

    /// Mint a reflector carrying `data`, the in-callback mirror of
    /// [`ScriptEngineLive::make_reflector`]. A native callback that *returns* a host
    /// object (e.g. `document.createElement` handing JS a new `Node`) needs this:
    /// `reflector_data` recovers an incoming node, this mints an outgoing one. Both
    /// backends can build a reflector mid-call from the context they already hold
    /// (Nova's `Agent`, Boa's `Context`), so it sits on the base context rather than
    /// gating DOM callbacks behind a separate live-context trait.
    fn make_reflector(&mut self, data: ReflectorData) -> Result<Self::Value, Self::Error>;

    /// The **canonical** reflector for `data`: minted on first call and cached, so
    /// repeated calls for the same node return a reflector backed by the *same* JS
    /// object (`document.body === document.body`). Unlike [`make_reflector`], which
    /// mints a fresh object every time.
    ///
    /// The cache is necessarily **engine-side** (a cached reflector is an
    /// engine-native value — a Nova `Global`, a Boa `JsValue` — so it cannot live in
    /// neutral [`HostData`] without re-coupling the host layer to an engine). It
    /// lives in the same host-defined slot the engine already owns.
    fn reflector_for(&mut self, data: ReflectorData) -> Result<Self::Value, Self::Error>;

    /// Hold a **strong** engine root on the canonical reflector for `data`, the
    /// in-callback mirror of [`ScriptEngine::root_reflectors`]. Returns whether the
    /// root was taken. Used on the mint path (`dom::reflect_pinned`): a node that
    /// is connected to its document is rooted the moment script is handed it, so
    /// its wrapper identity is stable from the first handoff rather than from the
    /// next GC tick — the window an engine with an allocation-threshold collector
    /// (Boa) would otherwise collect in.
    fn root_reflector(&mut self, _data: ReflectorData) -> bool {
        false
    }

    /// Release the root taken by [`root_reflector`](Self::root_reflector).
    /// Idempotent.
    fn unroot_reflector(&mut self, _data: ReflectorData) {}

    /// Mint a JS string value. The read-surface mirror of [`value_to_string`]: a
    /// callback returning text (`getAttribute`, `tagName`, the `textContent` getter)
    /// builds it with this.
    ///
    /// [`value_to_string`]: CallCx::value_to_string
    fn make_string(&mut self, s: &str) -> Result<Self::Value, Self::Error>;

    /// The `null` value, distinct from [`undefined`]. A miss returns `null`
    /// (`getElementById`, `getAttribute` on an absent attribute), per the DOM.
    ///
    /// [`undefined`]: CallCx::undefined
    fn make_null(&mut self) -> Self::Value;

    /// The `undefined` value (the usual callback return).
    fn undefined(&mut self) -> Self::Value;

    /// Mint a pending ("deferred") promise mid-call. Returns the JS promise value
    /// (the native callback returns it so JS can `await`) and a [`PromiseToken`] the
    /// host stashes (in host data) to settle the promise once the backing Rust future
    /// completes, via [`ScriptEngine::settle_host_promise`]. The in-callback mirror of
    /// [`ScriptEngine::new_host_promise`], and the primitive an async host call
    /// (`fetch`, `callModel`) is built from: return a promise now, resolve it later.
    fn new_host_promise(&mut self) -> Result<(Self::Value, PromiseToken), Self::Error>;

    /// The realm this callback is running in.
    ///
    /// A native sink needs it for exactly one reason: a platform object's
    /// wrapper is per-realm (WebIDL), and the wrapper a cross-realm read must
    /// return is the one in the object's **own** realm, not the caller's. A
    /// backend without realms answers [`MAIN_REALM`], which is true there.
    /// Actual JavaScript receiver of this native invocation.
    fn this_value(&mut self) -> Self::Value {
        self.undefined()
    }

    /// Realm of the immediate caller at native entry, before the callee realm
    /// becomes current. Hosts needing incumbent settings must track those separately.
    fn caller_realm(&mut self) -> RealmId {
        self.current_realm()
    }

    fn current_realm(&mut self) -> RealmId {
        MAIN_REALM
    }
}

/// Live-DOM extension: native-data reflectors (plan Part 3 / Appendix A Finding 2).
/// The reflector is the bridge object — a JS-visible value carrying a host
/// [`ReflectorData`] the host can recover later.
pub trait ScriptEngineLive: ScriptEngine {
    /// Create a JS value carrying `data` as JS-opaque native data. The host hands
    /// this to JS (as a global, a property, a callback argument, …).
    fn make_reflector(&mut self, data: ReflectorData) -> Result<Self::Value, Self::Error>;

    /// Recover the native data from a reflector value (the JS → host bridge).
    /// `None` if `value` is not a reflector created by [`make_reflector`].
    fn reflector_data(&mut self, value: &Self::Value) -> Option<ReflectorData>;
}
