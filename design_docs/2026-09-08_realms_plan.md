# Realms: one agent, one realm per browsing context

**Status:** partially committed; continuation acceptance open, 2026-09-09.
`aa12e16eb7c` contains phase one and in-progress engine corrections, per-realm
surfaces, synchronous frame creation, messaging, and live child rendering.
Further corrections remain in the working tree. Historical phase-one automated
and native Ortet receipts below apply to their recorded source, not the
continuation. Fresh continuation gates, including native Ortet receipts, remain
open as detailed under Current gates; no browser-hosted realm receipt is recorded.

**Parent:** [iframes and nested browsing contexts](2026-09-08_iframes_plan.md),
whose §4 named the realm decision as Mark's and left `contentWindow` a stub
rather than dressing it up.

**Related:** [Dedicated Worker](2026-09-07_worker_plan.md) (the second `Runtime`,
and the JSON clone wire this lane's design is the counter-case to),
[reflector identity](2026-09-07_reflector_identity_plan.md) (rooting and the
opaque-root policy, which realms make per-realm), and
[deferred web platform lanes](2026-09-07_deferred_web_platform_lanes_scoping.md).

**Genet commit at lane start:** `4039b859c77`
(`4039b859c7788c3e6b47fdb2b8091fe8c0d3ff87`).

---

## Mark's decision, and what it revised

The iframes lane shipped under **one script `Runtime` per browsing context**
and then discovered that the ruling could not be honoured: two `Runtime`s are
two engine instances, this stack's cross-instance boundary marshals strings,
and a marshalled `contentWindow` cannot carry the object identity HTML requires
of a same-origin frame. §4 of that plan stopped rather than approximate it, and
put three options to Mark.

Mark's revised ruling:

> One `Runtime` per **agent** (the page's event loop), one **realm** per
> browsing context, created through a realm API on the engine contract
> implemented on Boa and Nova. Same-origin frames share the agent and hold
> direct references, so `iframe.contentWindow.document.getElementById(x)` is the
> child's own element with real identity; cross-origin access goes through a
> `WindowProxy` exposing only the allowed members and throwing `SecurityError`
> for the rest.

That is the browser shape, and it is the only one in which the same-origin case
is honest. This plan is its execution.

---

## 1. The realm contract — **landed**

`components/script-engine-api/lib.rs`. Additive: every method carries a default
that refuses, so `script-engine-piccolo` — which implements the trait and has no
realms — compiles unchanged and *says* `supports_realms() == false` rather than
silently answering from the main realm.

### The neutral surface

| Item | What it is |
|---|---|
| `RealmId` (`u32`) | Opaque, engine-assigned identity for a realm inside one engine instance. |
| `MAIN_REALM` (`0`) | The realm `ScriptEngine::new` builds; the one every non-realm-suffixed method acts on. |
| `RealmError` | `Unsupported` / `Refused(&'static str)` / `NoSuchRealm(id)` / `Engine(String)`. |
| `supports_realms` | Cheap capability probe, so a host can pick a degraded path before building anything. |
| `create_realm` / `discard_realm` | A realm with its own global object and its own intrinsics, sharing the engine's heap, job queue and host hooks. |
| `eval_in_realm` | Evaluate in a chosen realm; the value returned is usable from any realm of this engine. |
| `realm_global` | The child's **actual** global object as a value the parent may hold. The primitive `contentWindow` is built from. |
| `set_global_in_realm` / `set_function_in_realm` | The two install primitives, realm-scoped. `set_function_in_realm` reuses the same captures-free trampoline; only the global it lands on differs. |
| `set_host_data_in_realm` | Per-realm `HostData`. |
| `CallCx::current_realm` | Which realm this native callback is running in. |

### Three decisions worth their own line

**`RealmError` is deliberately not `ScriptEngine::Error`.** The defaults have to
be constructible without an engine, and — more importantly — a backend that
cannot do a piece has to be able to say *which* piece and *why*. `Refused` takes
a static string for exactly that: a recorded fact about the backend, not a
runtime failure to retry. Approximating instead is the `Node-baseURI` trap at
engine scale.

**Cross-realm handles need no API.** One engine instance is one **agent**: one
heap, one job queue, one `pump`. A `Self::Value` obtained in realm B is an
ordinary reference in realm A — a Nova `Global` is agent-wide, a Boa `JsValue`
is context-wide. So "hold a handle to another realm's objects across the
boundary" is not a marshalling problem here; it is the absence of one. The
regression set asserts it in both directions: a parent mutating an object it got
from the child is visible to the child, and `back === thing` inside the child is
`true` for a value that made the round trip.

**The realm is the host-state key, and the engine already holds it.** This is
the piece that makes phase 2 cheap. `set_host_data_in_realm` means a child
browsing context's realm carries its own `HostState` — its own document,
history, markup and pins — so every native sink that reaches state through
`CallCx::host_data` lands on the child's DOM **with no call-site change**. The
alternative, rekeying `HostState` by realm, would have touched ~123 sites across
`script-runtime-api` and made every sink learn that realms exist. Instead the
engine answers, because the engine is the only party that actually knows.

### Boa

`Context::create_realm` builds a realm with its own global and intrinsics on the
same heap and job queue; `enter_realm` swaps the active one (it replaces the
*current call frame's* realm, which is why `with_realm` restores on the error
path too); `Realm::host_defined` is a per-realm slot map. One `BoaEngine` is one
agent with as many realms as the host asks for.

Two facts found by doing it:

- **The reflector class is per-realm.** Boa's `host_classes` live on the
  `Realm`, so a realm that will be handed reflectors needs its own
  `register_global_class::<Reflector>()`; without it `Reflector::from_data` in
  that realm cannot find its prototype. `create_realm` does the registration and
  rolls the realm back out of the table if it fails.
- **`Realm::global_object` is `pub(crate)`.** The global is reachable only by
  *entering* the realm and asking the `Context`, which reads the active call
  frame's realm. Recorded rather than patched: the Boa fork under
  `Code/crates/boa` is outside this lane's scope.

### Nova

`GcAgent::create_default_realm` adds a realm to the same agent, `run_in_realm`
runs a closure inside a chosen one, and `[[HostDefined]]` is **already
per-realm** — which is why per-realm host state costs nothing on this backend.
`NovaHostSlot` gained an `id` field, and that is where a native sink learns its
own realm.

### The exact refusals

Recorded rather than approximated, per the brief.

| Backend | Refusal | Why, and whether it matters |
|---|---|---|
| Nova | `GcAgent::run_in_realm` asserts the execution-context stack is empty, so it **cannot nest**. A native callback in realm A cannot ask the host to enter realm B mid-call. | Does not block the design. Values are agent-wide, so reading and calling another realm's objects from inside a call goes through the ordinary object protocol, which pushes the callee's realm itself. Only a *host-driven* realm switch is refused, and only while a call is on the stack. Phase 2's per-realm surface must therefore install from the engine level, never from inside a sink. |
| Nova | At most **256 simultaneous realms** per agent (`RealmRoot` indexes a `u8`). | Surfaced as `RealmError::Refused` rather than a panic. A page with 256 live nested browsing contexts is out of scope; the cap is recorded so the failure is legible if it is ever hit. |
| Boa | `Realm::global_object` is crate-private. | Worked around by entering the realm. No behavioural cost. |
| Boa | `pump` still drains the *whole* job queue for the agent, budget or no (`SimpleJobExecutor` has no sub-drain). | Pre-existing and unchanged by realms. It is the right shape here — the job queue **is** agent-wide — but it means Boa cannot bound one realm's microtask storm separately from another's. |
| piccolo | No realms at all; takes the trait defaults. | Correct and honest: `supports_realms()` is `false` and every realm call returns `Unsupported`. |

### Named regression manifest — phase 1

Seven cases, twinned on both backends. Boa's live in
`components/script-engine-boa/lib.rs`'s test module; Nova's in
`components/script-engine-nova/tests/realms.rs`, asserted through the neutral
trait rather than through Nova types, so the pair is a real both-engine gate.

| Case | What it proves |
|---|---|
| `realms_have_separate_globals` | A binding in one realm is invisible in the other, both directions. |
| `realms_have_separate_intrinsics` | The child's `Object` is not the parent's — the reason a realm, not merely a fresh global, is the unit for a browsing context. |
| `objects_cross_realms_with_identity` | **Same-origin identity.** A child object read in the parent is the same object: mutating it in the parent is visible in the child, and `back === thing` in the child is `true`. This is the assertion the marshalled-proxy design could never satisfy. |
| `realm_global_is_the_childs_own_global` | `realm_global(child) === child's globalThis`, and writing through it is visible to the child. The `contentWindow` primitive. |
| `native_fn_sees_its_own_realm_and_host_data` | One `NativeFn` impl, two realms, two host states, two answers — and the callback never learns realms exist. The per-realm host surface in miniature. |
| `reflectors_cross_realms` | A reflector handed into another realm is still the same node to the host: the reflector bridge is agent-wide. |
| `realm_refusals_are_exact` | `NoSuchRealm` for an unknown id, `Refused` for discarding `MAIN_REALM`, and a discarded id stays discarded. |

Boa 14/14 (7 new + 7 existing), Nova 29/29 (7 new + 22 existing).

---

## 2. Per-realm host surface — **planned**

### The shape

`Runtime::create_child_realm(...) -> RealmId` creates the realm, mints a fresh
`HostState` for the child document (bound to the child's arena from the
browsing-context tree), calls `set_host_data_in_realm`, and runs
`install_host_surface` scoped to that realm with `GlobalScopeKind::Window`.

Shared by the agent, because they *are* agent-wide: timers, microtasks, the job
queue, `fetch`, workers and the drive loop. Keyed by realm, because HTML keys
them by browsing context: the document, `location`, the history entry, `markup`,
`pins`, `tree_roots`.

### The one refactor it needs, sized

`install_host_surface` and the twelve `install_*_surface` functions take
`engine: &mut E` and — this is the load-bearing measurement — use **only two**
engine methods between them: `set_function::<F>` and `eval`. There are 16
`engine: &mut E` signatures and 243 call sites of those two methods.

So the change is a facade, not a rewrite:

```rust
// Illustrative, not compile-ready.
pub(crate) struct Surface<'e, E: ScriptEngine> { engine: &'e mut E, realm: RealmId }
impl<'e, E: ScriptEngine> Surface<'e, E> {
    fn eval(&mut self, src: &str) -> Result<E::Value, SurfaceError<E::Error>>;
    fn set_function<F: NativeFn<E>>(&mut self, name: &str, len: usize)
        -> Result<(), SurfaceError<E::Error>>;
}
```

Sixteen signatures change; the 243 call sites do not. `SurfaceError` exists
because the realm methods return `RealmError` and `E::Error` cannot be
constructed from one — and the main-realm path is structurally unable to produce
the `Realm` variant, since `Surface { realm: MAIN_REALM }` dispatches to the
non-realm methods.

**Nova's non-nesting refusal constrains this**: the install must be driven from
the engine level, never from inside a native sink.

### Done-conditions

- A child realm carries a full DOM surface (`document`, `Node`, `Element`,
  events, the interface table) bound to the child's own arena.
- A runtime regression, on **both** backends: two realms, two documents; an
  element created in the child is invisible to `document.getElementById` in the
  parent and visible in the child.
- Timers scheduled in either realm run on the one agent loop, in a deterministic
  order the scheduler trace records.
- The existing `script-runtime-api` suite (15 binaries, 142 lib tests) is
  unchanged.

---

## 3. `contentWindow`, `contentDocument`, `postMessage` — **planned**

### The design

`contentWindow` on an `HTMLIFrameElement` is the child realm's global
(`realm_global`) when the origins are same-origin, and a `WindowProxy` otherwise.
`contentDocument` is the child realm's `document` when same-origin, `null`
otherwise — HTML says `null`, not a throw.

`parent`, `top`, `frames[i]` become real; `window.opener` stays `null` (nothing
in this host opens a window); `frameElement` follows the origin rules — the
container element same-origin, `null` cross-origin.

### The `WindowProxy`

The cross-origin case is a **whitelist**, not a filtered view of the child. The
allowed set is HTML's `CrossOriginProperties`: `window`, `self`, `location`
(write-only through the setter, and `location.href` write-only), `close`,
`closed`, `focus`, `blur`, `frames`, `length`, `top`, `opener`, `parent`,
`postMessage`, and `Symbol.toStringTag` / `Symbol.hasInstance` /
`Symbol.isConcatSpreadable`. Everything else throws `SecurityError`.

Two rules the implementation must not soften:

- **Throw, do not answer plausibly.** A cross-origin read of a non-allowed
  member is a `SecurityError`, never `undefined`. `undefined` on both sides of
  an equality is the `Node-baseURI` pass-by-absence failure, and the
  cross-origin WPT files are exactly the tests that would report it as green.
- **The proxy is per (realm, target) pair.** HTML requires the same
  `WindowProxy` object for the same pair, so it is cached beside the reflector
  cache and rooted the same way.

### Reflector homing — the piece that must be got right

WebIDL gives a platform object one wrapper **per realm**, and the wrapper a
cross-realm read must return is the one in the object's **own** realm, not the
caller's. Boa's canonical cache is on the `Context` (one cache for all realms);
Nova's is in the realm's `[[HostDefined]]` (one per realm). Neither is what the
rule asks for. Both need the cache keyed by `(realm, ReflectorData)` **and** the
mint performed in the node's home realm — which is the node document's realm,
answered by the browsing-context tree.

This is the phase-3 risk. A reflector minted in the *caller's* realm would make
`parent.frames[0].document.body === childScript.document.body` false while every
other assertion in the file passed, and that is a wrong answer that scores well.

`postMessage` between realms uses the **structured clone across realms** — the
existing clone walker, not the Worker lane's JSON wire, since inside one agent
there is no wire to cross. `MessageEvent.source` is the sender's `WindowProxy`.
Events retarget across realms; the child's `load` fires on the parent's
`<iframe>` element.

### Done-conditions

- Named regressions on both backends: **same-origin identity**
  (`iframe.contentWindow.document.getElementById(x)` is `===` the node the
  child's own script holds) and **cross-origin `SecurityError`** (a
  non-whitelisted read throws, and the whitelisted ones do not).
- `postMessage` across realms delivers a structured clone with the correct
  `source` and `origin`; a cycle survives.
- The child's `load` fires on the parent's iframe element, once.
- A census over the nine directories with every `pass -> fail` explained.

---

## 4. Security — **planned**

Same-origin comparison comes from the browsing-context tree's `Origin`
(`components/genet-documents/src/browsing_context.rs`), which the iframes lane
already built with the two things this needs: an opaque origin carrying a serial
(so two `about:blank` documents are *not* same-origin) and `initial_about_blank`
as a bit rather than a URL test.

Sandbox flags, already parsed there from the attribute's inverted token list and
inherited as a union:

- `allow-same-origin` absent ⇒ the child gets an opaque origin ⇒ every
  `contentWindow` access is the cross-origin `WindowProxy` path.
- `allow-scripts` absent ⇒ the child's realm is created but **no script runs in
  it**. Note the interaction HTML calls out: a sandbox with both
  `allow-scripts` and `allow-same-origin` can remove its own sandbox attribute,
  which is why the two together are not a safe combination and why the flags are
  read at *load* time, not at access time.

`document.domain` remains a residual, as the iframes lane and the scoping
document both already named. It is the one same-origin-domain mutation this
model does not have, and it would move a realm between agent clusters at
runtime.

---

## Findings

**2026-09-08 — a realm is the smallest unit that carries intrinsics.** The
distinction that makes a realm the right object here, rather than "a second
global", is `realms_have_separate_intrinsics`: the child's `Object` is not the
parent's. HTML depends on it (`instanceof` across frames is false, and every
`Array.isArray`-style cross-realm brand check exists because of it), and a fresh
global on shared intrinsics would have passed the global-separation test while
failing every brand check.

**2026-09-08 — per-realm host state is the engine's answer, not the host's.**
The obvious design was to key `HostState` by realm and teach every native sink
to ask which realm it is in: ~123 coupling sites in `script-runtime-api`, every
one a chance to forget. The engine already knows the realm — it *is* the
execution context — so putting `HostData` in the realm's own slot moves the key
to the one party that cannot get it wrong, and leaves the sinks unchanged. Boa
needed a `RealmSlot` added; Nova already had exactly this shape.

**2026-09-08 — an engine's realm switch may be nestable or not, and the
difference decides where installation happens.** Boa's `enter_realm` is a frame
field swap and nests freely. Nova's `GcAgent::run_in_realm` asserts an empty
execution-context stack and cannot. Since the contract must hold on both, the
per-realm surface has to be installed from the engine level; a design that
installed lazily from inside a native sink would have worked on Boa and asserted
on Nova. This is the cross-target lesson in a new place: a green build on one
backend proves nothing about the other's execution model.

**2026-09-08 — a purely additive trait change is still worth a census, and the
lockfile digest is the control that makes the result readable.** The `pre` and
`post` runners produced a byte-identical `Cargo.lock`
(`6a74d1ef…`), which is what lets "zero movement across nine directories"
be read as *this change moved nothing* rather than *two effects cancelled*.
Without the lockfile control it would only have been a coincidence with good
manners.

---

## Before / after

| Surface | Before (`4039b859c77`) | After |
|---|---|---|
| Realms on the engine contract | none — `ScriptEngine` had no realm concept | `RealmId`, `MAIN_REALM`, `RealmError`, eight trait methods and `CallCx::current_realm`, all defaulted to a stated refusal |
| Boa | one realm, one global, one context-wide host-data slot | many realms, per-realm reflector class, per-realm `HostData`, per-realm native functions |
| Nova | one realm; `[[HostDefined]]` per-realm but only ever one realm | many realms (cap 256, surfaced as a refusal), realm id carried in the host slot, per-realm `HostData` |
| piccolo | implements `ScriptEngine` | unchanged; `supports_realms() == false` and every realm call `Unsupported` |
| Cross-realm object identity | not expressible | proven on both backends, both directions |
| `contentWindow` | the iframes lane's stub: a fresh empty document, no browsing-context link | **unchanged** — phase 3 |
| WPT, nine directories | see `pre/` | identical: `subtest_delta 0`, zero status movement |

---

## Receipts

Raw maps and drivers under
`Code/testing/genet/wpt-ledger/2026-09-08_realms/`.

Both runners built with `CARGO_TARGET_DIR=C:/t/laneRe-target` and
`cargo build --release -p genet-wpt --features netfetch`, SHA-256:

| Runner | Digest |
|---|---|
| `pre` — from `4039b859c77`, **before the first edit** | `587300d57f1a68002f77cf3b6c33761e515563c7a774628f8696866849803c70` |
| `post` — the lane's tree | `51951a576b57e57e3f35ef64703cdd30da5302681ffa71008444376231bc8cdf` |

`Cargo.lock` as generated by **both** builds, identical:
`6a74d1efa0398509a9693b34bfcbbc0f4760284caf0d3a38bd5f65742fc57e5d`.

| Gate | Result |
|---|---|
| `cargo test -p script-engine-boa` | 14 passed, 0 failed |
| `cargo test -p script-engine-nova` | 29 passed, 0 failed (9 lib + 7 realms + 5 regress + 8 wtf8) |
| `cargo test -p script-runtime-api` | 15 binaries green, incl. the interface-table drift check |
| `cargo check --workspace --features genet-wpt/netfetch` | clean, 2m02s |
| clippy, rustfmt, touched files | clean; the two remaining Boa warnings and the `type_complexity` one are pre-existing and outside the diff |
| Testharness census, 9 directories, disk/boa/livery, `--jobs 8 --timeout 240` | `subtest_delta 0`, zero status movements, nothing to explain, no repins |
| `check-testharness-baselines.ps1` (14 slices) | `unexpected=0` |
| `check-reftest-baselines.ps1` (2 slices) | `unexpected=0`: `css/mediaqueries` 16 passed / 40 failed, `css/css-position` 45 passed / 73 failed |
| Ortet `article.html`, 3 runs | `0x6377ba8a6bf4dbc9`, unchanged and stable |
| Ortet `frames.html`, 3 runs | `0x97bdd4bd9e03ec02`, unchanged and stable |

---

## Progress

**2026-09-08.** Phase 1 landed: the realm contract on the engine-neutral trait,
implemented on Boa and Nova, with seven twinned regressions on each and every
backend refusal recorded rather than approximated. Zero WPT movement across nine
directories against a runner-controlled baseline with an identical lockfile
digest; both baseline guards and both Ortet receipts unchanged. Phases 2–4 are
specified above with done-conditions and are not started; the per-realm surface
is sized (16 signatures, 243 call sites, two engine methods) and the
reflector-homing risk in phase 3 is named.

## Continuation, 2026-09-09 — in progress

The inherited six-file patch was preserved at HEAD `640477b6138`. A fresh
pre runner was built before editing the integration tree. Its SHA-256 is
`bd2adaddfe934f93b65fa7480a8009d5e58f8e6d1f95c038f1fd9348e99d5ee2`;
Cargo.lock is `6a74d1efa0398509a9693b34bfcbbc0f4760284caf0d3a38bd5f65742fc57e5d`.
The nine-directory pre census completed successfully. Files, maps and inherited
patch snapshots are under `Code/testing/genet/wpt-ledger/2026-09-09_realms-continuation/`.

### Corrected findings

- Platform objects have a relevant realm; accessing a child object does not
  authorize minting another public DOM wrapper in the caller. The earlier
  "one wrapper per realm" explanation above was misleading. The child's own
  methods must return its original JS object and its own prototypes.
- Boa's old context-wide reflector cache could alias equal raw node IDs from
  different arenas in release builds. Debug arena tags concealed that case.
  The continuation scopes canonical caches and root bookkeeping to each realm.
- Nova's child promise counters collided while settlement and GC bookkeeping
  still searched main. Pending promise identity is agent-wide; reflector roots
  and death reports are realm-scoped.
- `GcAgent::run_in_realm` is an outer-entry restriction. Nova's public
  `Agent::create_realm` and `run_script` support synchronous creation inside a
  native callback. No preallocated realm pool or delayed empty window is needed.
- Nova child realms use `Global<Realm>` roots in the continuation, so the old
  256-`RealmRoot` limit does not describe this new path.
- A JavaScript-visible shared timer object exposes foreign callback functions
  and their constructors. The queue handle is retained by the host and passed
  privately during installation, then removed from each global.
- `BrowsingContextTree`, origins and sandbox flags moved into the lower
  `browsing-context-api` crate. `genet-documents::browsing_context` reexports
  that owner; runtime depends downward on it without a documents/runtime cycle.
- Direct native messaging needs caller provenance. Small local Boa/Vano fork
  APIs expose the immediate native caller, with separate focused tests. This is
  explicitly not a complete implementation of HTML incumbent settings across
  arbitrary host callback stacks; the continuation must not claim that scope.

### Current gates

Compilation and new engine/runtime/frame acceptance tests are in progress.
The post census, baseline guards, workspace check, fork tests, clippy, formatter
checks and Ortet receipts remain pending.
