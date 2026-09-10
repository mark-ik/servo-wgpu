# Realms: one agent, one realm per browsing context

**Status:** ordinary cross-arena adoption verified, with the owner-resolved
accessor and imported-identity capture/replay landed on top of it; broad DOM
guard remains red, 2026-09-10. The release runtime suite passes 520 tests with
two live-iframe fixtures ignored. Required Boa/Vano patches are published and
pinned. The ordinary-adoption DOM census gained 203 passing subtests and retains
one historical iframe loss; that pass remains protected in the baseline. The
accessor/replay phase's own census moves nothing in any of its four subsets.
Twelve other canonical subsets and both reftest guards match. Imported-ID
capture and replay now translate through the importing store's registry. Full
G5, browsing-context relocation and associated shadow/template transfers remain
open. Earlier phases below retain their historical results; final acceptance is
recorded at the end of this plan.

**Parent:** [iframes and nested browsing contexts](2026-09-08_iframes_plan.md),
whose Â§4 named the realm decision as Mark's and left `contentWindow` a stub
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
of a same-origin frame. Â§4 of that plan stopped rather than approximate it, and
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

> Historical design snapshot: sections 1â€“4 preserve the initial phase-one
> design and its then-planned steps. Installation restrictions and backend caps
> were superseded during continuation. The dated continuation below records
> the implemented behavior, current gates and remaining boundaries.

## 1. The realm contract â€” **landed**

`components/script-engine-api/lib.rs`. Additive: every method carries a default
that refuses, so `script-engine-piccolo` â€” which implements the trait and has no
realms â€” compiles unchanged and *says* `supports_realms() == false` rather than
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
be constructible without an engine, and â€” more importantly â€” a backend that
cannot do a piece has to be able to say *which* piece and *why*. `Refused` takes
a static string for exactly that: a recorded fact about the backend, not a
runtime failure to retry.

Continuation distinction: callback `eval_in_realm_from_call` and `call_from_call`
return `ScriptEngine::Error`, preserving original JavaScript exceptions and
thrown-object identity. `RealmError` remains the host-facing realm-operation error.

**Cross-realm handles need no API.** One engine instance is one **agent**: one
heap, one job queue, one `pump`. A `Self::Value` obtained in realm B is an
ordinary reference in realm A â€” a Nova `Global` is agent-wide, a Boa `JsValue`
is context-wide. So "hold a handle to another realm's objects across the
boundary" is not a marshalling problem here; it is the absence of one. The
regression set asserts it in both directions: a parent mutating an object it got
from the child is visible to the child, and `back === thing` inside the child is
`true` for a value that made the round trip.

**The realm is the host-state key, and the engine already holds it.** This is
the piece that makes phase 2 cheap. `set_host_data_in_realm` means a child
browsing context's realm carries its own `HostState` â€” its own document,
history, markup and pins â€” so every native sink that reaches state through
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
per-realm** â€” which is why per-realm host state costs nothing on this backend.
`NovaHostSlot` gained an `id` field, and that is where a native sink learns its
own realm.

### The exact refusals

Recorded rather than approximated, per the brief.

| Backend | Refusal | Why, and whether it matters |
|---|---|---|
| Nova | `GcAgent::run_in_realm` asserts the execution-context stack is empty, so it **cannot nest**. A native callback in realm A cannot ask the host to enter realm B mid-call. | Does not block the design. Values are agent-wide, so reading and calling another realm's objects from inside a call goes through the ordinary object protocol, which pushes the callee's realm itself. Only a *host-driven* realm switch is refused, and only while a call is on the stack. Phase 2's per-realm surface must therefore install from the engine level, never from inside a sink. |
| Nova | At most **256 simultaneous realms** per agent (`RealmRoot` indexes a `u8`). | Surfaced as `RealmError::Refused` rather than a panic. A page with 256 live nested browsing contexts is out of scope; the cap is recorded so the failure is legible if it is ever hit. |
| Boa | `Realm::global_object` is crate-private. | Worked around by entering the realm. No behavioural cost. |
| Boa | `pump` still drains the *whole* job queue for the agent, budget or no (`SimpleJobExecutor` has no sub-drain). | Pre-existing and unchanged by realms. It is the right shape here â€” the job queue **is** agent-wide â€” but it means Boa cannot bound one realm's microtask storm separately from another's. |
| piccolo | No realms at all; takes the trait defaults. | Correct and honest: `supports_realms()` is `false` and every realm call returns `Unsupported`. |

### Named regression manifest â€” phase 1

Seven cases, twinned on both backends. Boa's live in
`components/script-engine-boa/lib.rs`'s test module; Nova's in
`components/script-engine-nova/tests/realms.rs`, asserted through the neutral
trait rather than through Nova types, so the pair is a real both-engine gate.

| Case | What it proves |
|---|---|
| `realms_have_separate_globals` | A binding in one realm is invisible in the other, both directions. |
| `realms_have_separate_intrinsics` | The child's `Object` is not the parent's â€” the reason a realm, not merely a fresh global, is the unit for a browsing context. |
| `objects_cross_realms_with_identity` | **Same-origin identity.** A child object read in the parent is the same object: mutating it in the parent is visible in the child, and `back === thing` in the child is `true`. This is the assertion the marshalled-proxy design could never satisfy. |
| `realm_global_is_the_childs_own_global` | `realm_global(child) === child's globalThis`, and writing through it is visible to the child. The `contentWindow` primitive. |
| `native_fn_sees_its_own_realm_and_host_data` | One `NativeFn` impl, two realms, two host states, two answers â€” and the callback never learns realms exist. The per-realm host surface in miniature. |
| `reflectors_cross_realms` | A reflector handed into another realm is still the same node to the host: the reflector bridge is agent-wide. |
| `realm_refusals_are_exact` | `NoSuchRealm` for an unknown id, `Refused` for discarding `MAIN_REALM`, and a discarded id stays discarded. |

Boa 14/14 (7 new + 7 existing), Nova 29/29 (7 new + 22 existing).

---

## 2. Per-realm host surface â€” **planned**

### The shape

`Runtime::create_child_realm(...) -> RealmId` creates the realm, mints a fresh
`HostState` for the child document (bound to the child's arena from the
browsing-context tree), calls `set_host_data_in_realm`, and runs
`install_host_surface` scoped to that realm with `GlobalScopeKind::Window`.

The implemented agent shares task scheduling and microtask execution. Each
realm host retains fetch completion, worker routing, document, location and DOM
bookkeeping; the browsing-context tree retains session history.

### The one refactor it needs, sized

`install_host_surface` and the twelve `install_*_surface` functions take
`engine: &mut E` and â€” this is the load-bearing measurement â€” use **only two**
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
constructed from one â€” and the main-realm path is structurally unable to produce
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

## 3. `contentWindow`, `contentDocument`, `postMessage` â€” **planned**

### The design

`contentWindow` on an `HTMLIFrameElement` is the child realm's global
(`realm_global`) when the origins are same-origin, and a `WindowProxy` otherwise.
`contentDocument` is the child realm's `document` when same-origin, `null`
otherwise â€” HTML says `null`, not a throw.

`parent`, `top`, `frames[i]` become real; `window.opener` stays `null` (nothing
in this host opens a window); `frameElement` follows the origin rules â€” the
container element same-origin, `null` cross-origin.

### The `WindowProxy`

The cross-origin case is a **whitelist**, not a filtered view of the child. The
allowed set is HTML's `CrossOriginProperties`: `window`, `self`, `location`
(write-only through the setter, and `location.href` write-only), `close`,
`closed`, `focus`, `blur`, `frames`, `length`, `top`, `opener`, `parent`,
`postMessage`, and `Symbol.toStringTag` / `Symbol.hasInstance` /
`Symbol.isConcatSpreadable`, with the allowed `then` fallback. Other protected
reads throw `SecurityError`. The implementation exposes the permitted Location
descriptor shape, but Location writes and `replace()` remain unsupported and
throw; navigation remains open.

Two rules the implementation must not soften:

- **Throw, do not answer plausibly.** A cross-origin read of a non-allowed
  member is a `SecurityError`, never `undefined`. `undefined` on both sides of
  an equality is the `Node-baseURI` pass-by-absence failure, and the
  cross-origin WPT files are exactly the tests that would report it as green.
- **The proxy is per (realm, target) pair.** HTML requires the same
  `WindowProxy` object for the same pair. Implemented cross-origin views are
  cached by target in each viewer realm's private JS `Map`; they do not use the
  reflector cache.

### Reflector homing â€” the piece that must be got right

Child DOM methods return their original canonical wrappers. Native reflector
caches and root bookkeeping are realm-scoped, preventing equal raw IDs from
separate document arenas from aliasing. A read from another realm retains that
object's identity and creation-realm prototypes.

This is the phase-3 risk. A reflector minted in the *caller's* realm would make
`parent.frames[0].document.body === childScript.document.body` false while every
other assertion in the file passed, and that is a wrong answer that scores well.

`postMessage` between realms uses the **structured clone across realms** â€” the
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

## 4. Security â€” **planned**

Same-origin comparison comes from the browsing-context tree's `Origin`
(`components/genet-documents/src/browsing_context.rs`), which the iframes lane
already built with the two things this needs: an opaque origin carrying a serial
(so two `about:blank` documents are *not* same-origin) and `initial_about_blank`
as a bit rather than a URL test.

Sandbox flags, already parsed there from the attribute's inverted token list and
inherited as a union:

- `allow-same-origin` absent â‡’ the child gets an opaque origin â‡’ every
  `contentWindow` access is the cross-origin `WindowProxy` path.
- `allow-scripts` absent â‡’ the child's realm is created but **no script runs in
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

**2026-09-08 â€” a realm is the smallest unit that carries intrinsics.** The
distinction that makes a realm the right object here, rather than "a second
global", is `realms_have_separate_intrinsics`: the child's `Object` is not the
parent's. HTML depends on it (`instanceof` across frames is false, and every
`Array.isArray`-style cross-realm brand check exists because of it), and a fresh
global on shared intrinsics would have passed the global-separation test while
failing every brand check.

**2026-09-08 â€” per-realm host state is the engine's answer, not the host's.**
The obvious design was to key `HostState` by realm and teach every native sink
to ask which realm it is in: ~123 coupling sites in `script-runtime-api`, every
one a chance to forget. The engine already knows the realm â€” it *is* the
execution context â€” so putting `HostData` in the realm's own slot moves the key
to the one party that cannot get it wrong, and leaves the sinks unchanged. Boa
needed a `RealmSlot` added; Nova already had exactly this shape.

**2026-09-08 â€” an engine's realm switch may be nestable or not, and the
difference decides where installation happens.** Boa's `enter_realm` is a frame
field swap and nests freely. Nova's `GcAgent::run_in_realm` asserts an empty
execution-context stack and cannot. Since the contract must hold on both, the
per-realm surface has to be installed from the engine level; a design that
installed lazily from inside a native sink would have worked on Boa and asserted
on Nova. This is the cross-target lesson in a new place: a green build on one
backend proves nothing about the other's execution model.

**2026-09-08 â€” a purely additive trait change is still worth a census, and the
lockfile digest is the control that makes the result readable.** The `pre` and
`post` runners produced a byte-identical `Cargo.lock`
(`6a74d1efâ€¦`), which is what lets "zero movement across nine directories"
be read as *this change moved nothing* rather than *two effects cancelled*.
Without the lockfile control it would only have been a coincidence with good
manners.

---

## Before / after

| Surface | Before (`4039b859c77`) | After |
|---|---|---|
| Realms on the engine contract | none â€” `ScriptEngine` had no realm concept | `RealmId`, `MAIN_REALM`, `RealmError`, eight trait methods and `CallCx::current_realm`, all defaulted to a stated refusal |
| Boa | one realm, one global, one context-wide host-data slot | many realms, per-realm reflector class, per-realm `HostData`, per-realm native functions |
| Nova | one realm; `[[HostDefined]]` per-realm but only ever one realm | many realms (cap 256, surfaced as a refusal), realm id carried in the host slot, per-realm `HostData` |
| piccolo | implements `ScriptEngine` | unchanged; `supports_realms() == false` and every realm call `Unsupported` |
| Cross-realm object identity | not expressible | proven on both backends, both directions |
| `contentWindow` | the iframes lane's stub: a fresh empty document, no browsing-context link | **unchanged** â€” phase 3 |
| WPT, nine directories | see `pre/` | identical: `subtest_delta 0`, zero status movement |

---

## Receipts

Raw maps and drivers under
`Code/testing/genet/wpt-ledger/2026-09-08_realms/`.

Both runners built with `CARGO_TARGET_DIR=C:/t/laneRe-target` and
`cargo build --release -p genet-wpt --features netfetch`, SHA-256:

| Runner | Digest |
|---|---|
| `pre` â€” from `4039b859c77`, **before the first edit** | `587300d57f1a68002f77cf3b6c33761e515563c7a774628f8696866849803c70` |
| `post` â€” the lane's tree | `51951a576b57e57e3f35ef64703cdd30da5302681ffa71008444376231bc8cdf` |

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
digest; both baseline guards and both Ortet receipts unchanged. Phases 2â€“4 are
specified above with done-conditions and are not started; the per-realm surface
is sized (16 signatures, 243 call sites, two engine methods) and the
reflector-homing risk in phase 3 is named.

## Continuation, 2026-09-09 â€” in progress

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
  APIs expose the nearest authored ECMAScript caller, skipping native call/apply
  trampolines, with separate focused tests. This is
  explicitly not a complete implementation of HTML incumbent settings across
  arbitrary host callback stacks; the continuation must not claim that scope.

### Current gates

The corrected aggregate run passed 667 tests with none failed or ignored:
script-runtime-api 446, genet-scripted 112 (110 library plus two integration),
genet-documents 49, Boa adapter 22 and Nova adapter 38. This includes the
interface-table drift check, lazy loading and arena-locality regressions.
Receipt: `test-post-census-final.log`. Engine API has no tests; the unchanged
Piccolo adapter previously passed 10/10 and browsing-context-api 13/13.
The focused local Boa caller regression and both Vano fork regressions passed
before the final locality field; the final Nova suite covers locality through GC.

Clippy passed all touched Genet crates with no errors (warnings remain in the
log). Rustfmt passed all touched Rust files. Workspace check with
`genet-wpt/netfetch` and the corrected release build passed. The final post census is net +222 subtest passes, with four valid adoption
assertions still failing and one removed duplicate registration. Twelve of
fourteen testharness slices pass after three forward-only repins; dom and
dom/nodes remain red. Both reftest slices have unexpected=0. Ortet article
matches three times; frames matches on three consecutive captures after the
one-pixel outlier documented below.

An external writer committed the shared integration tree as `aa12e16eb7c`
(`Realms on the engine contract, phase one`) while continuation verification
was still running. The coordinator did not create this commit. It includes
the in-progress per-realm surface and frame integration, so its title does
not establish a phase boundary or a green gate. Later fixes remain working
tree changes. The required pre/post runner comparison still uses the captured
pre source above, with the final post source recorded separately.

Current measured continuation receipts include 24 frame acceptance tests, 8
cross-origin reflection tests, 18 clone/exception tests, 6 host/task-routing tests,
4 animation-frame tests and 2 descriptor tests, all on both engines. The first
hosted both-engine run was 106 passed / 4 failed; the final combined run above
supersedes that diagnostic snapshot.
The final verification tree is f949210ca32 plus the recorded realms patch.

### Continuation implementation and remaining boundaries (2026-09-09)

The continuation uses one engine and one agent task drive. `AgentState` retains
per-realm hosts; each host owns its document and location. A private rooted task
state joins timers and animation callbacks without exposing foreign callback
objects to script. Fetch completion and worker routing retain their realm.
`frames.rs` creates the child realm synchronously on connected iframe insertion,
using the canonical origins and sandbox flags in `browsing-context-api`.

Same-origin `contentWindow` exposes the actual child global. Child DOM methods
return the child's canonical wrappers. Cross-origin windows use a viewer-realm
proxy with explicit read, reflection and mutation rules. Messaging records are
serialized in the sender and decoded with recipient intrinsics; `source` is
resolved for the recipient's origin. Native caller provenance follows authored
script through call/apply trampolines. This is not the complete HTML backup
incumbent-settings stack.

The hosted Livery route installs child CSSOM before authored scripts and paints
the child's live arena into the parent's iframe slot. Its regression changes
the child DOM from the parent and checks the resulting scene.

Named continuation regression manifest (each runtime case runs on Boa and Nova):

| File under `components/script-runtime-api/tests/` | Boundary |
|---|---|
| `frame_realms.rs` | Synchronous creation; `child_script_and_parent_share_node_identity`; parent/top/frameElement; `cross_origin_window_rejects_non_whitelisted_reads`; independent sandbox origin/script flags; one load event; recipient clone/source; borrowed postMessage receiver and caller |
| `frame_window_security.rs` | Cross-origin Window and Location reflection and allowed-member access |
| `realms.rs` | Separate documents, shared timer ordering, child fetch completion |
| `realm_animation_frames.rs` | Shared drive and private callbacks |
| `realm_clone.rs` | Recipient intrinsics, cycles/aliases/buffers, worker wire compatibility, inert records, captured intrinsics, accessor refusal, identity-based DOM wrapper branding |

The Boa/Vano fork tests additionally cover native caller provenance and Vano
foreign builtin prototypes across realm removal and collection. Local fork
patches are required; the public git dependency alone does not contain them.

Open boundaries remain: navigation with a stable WindowProxy, complete removed-
frame lifecycle cleanup, full HTML incumbent-settings behavior, dynamic DOM
ordering of animation callbacks, and `document.domain`. These are not accepted
by the bounded creation, identity, scheduling, messaging and rendering tests.
The final census and guard results below apply to this continuation and retain
its unresolved adoption failures.

### Verification checkpoint, 2026-09-09

The full runtime run completed with 418 passes and 14 failures. All 52 new
focused tests passed on that snapshot. Six failures exposed Window.parent missing its [Replaceable] setter; original
fixtures are retained and the descriptor is being corrected against html.idl.
Two failures require installing the fixture style provider on the child host.
Six further failures
exposed missing messaging trace marks, undefined payload conversion, and flattened
exception types. Corrections require a fresh runtime run. The nested-frame load
ordering regression was added after that snapshot and is also pending.

The focused Boa native-caller test passed. Both exact Vano fork tests passed
through Genet's dependency graph, including native TypeError/ReferenceError
creation-realm prototypes. Temporary fork test targets were removed afterward.
The new browsing-context-api crate passed all 13 tests. Post census and final
acceptance remain open. Another writer committed in-progress integration as
`aa12e16eb7c`; this coordinator has made no commits.

### Parser and initial-load completion correction (2026-09-09)

A scriptless parent did not discover parser-inserted iframes before returning
from parsing. Its first contentWindow read created the realm after the host had
already pumped tasks. Parser completion now refreshes that discovery before
readiness events. When child initial loads remain, the parent stays Interactive;
final child completion advances Complete and dispatches load once. Removing a
pending iframe cancels only its initial source/events and releases this wait.
The retained realm still follows the separately open teardown/lifetime boundary.

The hosted child-paint fixture also misread fragment_rect's [x, y, width, height]
contract. Its dimension assertions now use width/height directly; the requested
20/75/30 sizes and paint assertions are retained. Focused hosted tests passed 2/2
and frame tests passed 22/22 before adding the pending-removal case. The complete
runtime/hosted rerun in test-runtime-hosted-final.log includes that final case;
passed all 554 tests and supersedes the earlier full-run failures.

### Static document fixture corrections (2026-09-09)

The separate script-free document suite initially passed 47/49. Its append and
submission fixture clicked the start-caret text anchor before expecting an
append; it now sends End explicitly. Its input-rejection fixture reused retained
geometry after focus invalidated layout; it now renders before each subsequent
hit query and final sequential-focus action. The original HTML and all behavior
assertions remain. These corrections change tests only. The rerun passed 49/49
in `test-documents-corrections.log`. The rejected intrinsic-sizing hypothesis in
the diagnostic log is superseded by the reproduced stale-layout cause.

### Census-driven corrections and the adoption acceptance boundary

The initial continuation census is preserved as `post-initial/`, with its runner
`genet-wpt-post-initial.exe` (SHA-256
`5e5879d10995104a0f663ae3ffa234cc2482726afc94cb094807ccf5dac22251`).
It gained 230 named subtest passes and lost 21, net +209. This is a diagnostic
snapshot, not an accepted baseline repin.

Two issues were reproduced. The runtime recorded `loading=lazy` but still
acquired and scheduled the child source. Lazy contexts now retain their initial
about:blank window and inherited origin while deferring source acquisition,
scripts and load events. They do not delay ancestor load. This matches the
existing static loader policy; viewport-triggered or lazy-to-eager activation
is not implemented by this slice.

Real child documents also exposed cross-arena node adoption. Passing a
parent-created Node to a child-native DOM method treated its raw identity as a
slot in the child arena. Optimized builds could alias a different node or panic;
debug builds could trip the arena fence. `CallCx::reflector_is_local` now checks
immutable native provenance on both engines, independently of raw ids or public
prototypes. Runtime DOM entry points reject a foreign arena before dereferencing
its NodeId. The authored append/adopt path reports WrongDocumentError before
detachment or ownerDocument changes. Same-arena secondary-document adoption and
ordinary access to child-created objects retain their existing behavior.

This refusal prevents corruption; it does not implement cross-arena adoption.
The owning-document/store migration remains the G5 boundary documented in
`docs/2026-06-11_gc_arena_dom_plan.md`. Prior valid adoption passes cannot be
removed from checked baselines to accept this continuation. Final census and
guard receipts below must keep this distinction explicit.

Named new regressions: `tests/frame_lazy.rs` and
`tests/frame_arena_boundary.rs`, each on Boa and Nova (4/4 focused runtime tests
passed). Adapter `reflector_locality_uses_owner_not_raw_id` tests canonical and
uncached equal raw ids, prototype changes, non-reflectors and forced GC. The
Vano embedder-object provenance field is native, cloned with heap snapshots and
ignored by GC tracing. No clone-by-value fallback is used for Node adoption.

### Final continuation census (2026-09-09)

The final disk/Boa/Livery census completed all nine directories with eight
workers and a 240-second process timeout. Each map's runner digest was checked
against the executable. Source is `f949210ca32` plus
`post-final-genet-source.patch` and the local Boa/Vano patches in the continuation
ledger. The last change after the 667-test aggregate was the specified
HierarchyRequestError for cross-document moveBefore; its focused Boa/Nova
regressions, clippy, workspace check and fresh release build passed.

| Runner | SHA-256 |
|---|---|
| Pre, inherited phase-one source at `640477b6138` | `bd2adaddfe934f93b65fa7480a8009d5e58f8e6d1f95c038f1fd9348e99d5ee2` |
| Final post | `f3ccc4454686b5612d0588cba41e823db201397fe5647d9a0d7ee0f62d19f598` |

The post lockfile digest is
`e64b015a646a1b628d17b1658cedf9e4a445ab3d40b522d61345f9d19c22e05d`.
Its only changes from the captured pre lockfile are the new browsing-context-api
package and dependency edges from genet-documents and script-runtime-api.
Dependency versions are unchanged. Required local fork heads and file hashes
are recorded in `post-final-source-hashes.json`.

| Directory | Passing subtests before | After | Delta | Files gained / lost |
|---|---:|---:|---:|---:|
| `dom` | 46,391 | 46,528 | +137 | 5 / 1 |
| `html/browsers/origin` | 2 | 12 | +10 | 2 / 0 |
| `html/browsers/sandboxing` | 0 | 0 | +0 | 0 / 0 |
| `html/browsers/the-window-object` | 69 | 83 | +14 | 3 / 0 |
| `html/browsers/windows` | 4 | 7 | +3 | 2 / 0 |
| `html/dom` | 42,334 | 42,377 | +43 | 1 / 0 |
| `html/semantics/embedded-content/the-iframe-element` | 17 | 23 | +6 | 1 / 0 |
| `webmessaging` | 160 | 169 | +9 | 5 / 0 |
| `workers` | 325 | 325 | +0 | 0 / 0 |

Total: 227 newly passing named subtests, five lost recorded passes, net +222;
19 files gain all-pass status and one loses it. Four losses are valid assertions
that require cross-arena adoption; they remain acceptance failures:

| File | Former pass now failing |
|---|---|
| `dom/nodes/Node-appendChild.html` | Adopting an orphan |
| `dom/nodes/Node-isConnected.html` | Test with iframes (also the sole all-pass file regression) |
| `html/browsers/the-window-object/accessing-other-browsing-contexts/window_length.html` | Child browsing context has a child browsing context |
| `html/dom/partial-updates/tentative/template-for-html-setters.html` | Setter createContextualFragment should not patch existing target in head |

The fifth recorded loss is occurrence two of the top-level frameElement-null
assertion in `html/browsers/windows/nested-browsing-contexts/frameElement.sub.html`.
The fixture defines four tests once; the pre map contains all four registrations
twice, while the post map contains one copy of each. The unique assertion still
passes. The duplicate count is preserved as a named movement, not silently
removed from the comparison.

Forward-only repin: `ports/genet-wpt/expectations/testharness/dom_abort_boa.json`,
only `dom/abort/reason-constructor.html`, fail 0/1 to pass 1/1:
"AbortSignal.reason.constructor should be from iframe". A fresh exact-subset
candidate measured 13 files and 59/72 passing subtests; all former named passes
and membership were retained. `dom_boa` and `dom_nodes_boa` remain unchanged
because each contains the two regressed adoption entries. Their unpromoted gains
and losses are listed in `baseline-review.json`; H4 opt-in membership is unchanged.
The completed guard and native Ortet results follow.

### Final gates, repins and retained failures

| Gate | Result |
|---|---|
| Aggregate runtime, hosted, document and Boa/Nova suites | 667 passed, zero failed or ignored; `test-post-census-final.log` |
| Last localized moveBefore exception correction | Four focused runtime cases passed on Boa/Nova; `test-final-move-error.log` |
| Engine API, Piccolo, browsing-context-api | API has zero tests; unchanged Piccolo 10/10 and browsing-context-api 13/13 passed earlier |
| Clippy and workspace check | Passed, warnings recorded; final runtime clippy and workspace check rerun after the localized correction |
| Rustfmt and diff whitespace checks | Passed |
| Final release runner and nine-directory census | Complete, exact runner digest verified in every post map |
| Original testharness guard | Failed on dom, unexpected=44; `guard-testharness-final.log` |
| Remaining testharness slices | Audited with eight workers and the same 30-second timeout; after reviewed repins, twelve of fourteen slices are unexpected=0. dom/nodes remains red with dom. |
| Reftest guards | Both unexpected=0: mediaqueries 16 pass / 40 fail; css-position 45 pass / 73 fail |
| Ortet article | Three captures at `0x6377ba8a6bf4dbc9`, unchanged |
| Ortet frames | Runs 2â€“4 exactly match `0x97bdd4bd9e03ec02`; initial one-pixel outlier retained |

Two additional forward-only repins record completed failures, without gaining or
losing passing assertions:

- `css_mediaqueries_boa.json`: `media-query-matches-in-iframe.html`, the
  aspect-ratio change-event assertion, not-run to fail; and
  `mq-dynamic-empty-children.html`, its dynamic-media-query assertion, timeout
  to fail. Fresh subset: 86/384 subtests, 93 files.
- `css_animations_boa.json`:
  `responsive/fill-forwards-viewport-units.html`, "fill: forwards with viewport
  units updates on viewport resize", timeout to fail. Fresh subset: 360/1219
  subtests, 231 files.

Both exact-subset checks passed with unexpected=0 after repinning. Together with
the abort-constructor improvement, this is three baseline files and four named
entries. All old passing assertions and exact membership were preserved.
`baseline-review.json` retains the unpromoted broad DOM changes. The original
guard reports those newly measured outcomes as well as the adoption regressions;
its failure must not be described as a green ratchet.

The original 30-second guard also killed `dom/ranges/Range-mutations-dataChange.html`.
A sequential single-file control killed both the pre runner (31.52 seconds wall)
and the final post runner (30.69 seconds wall) at the same 30-second cap. Both
240-second census maps complete that file at 2328/2808 passing subtests. This
failure is not specific to the continuation. The timeout and expectation remain
unchanged; `range-30s-pre/post.json` and their logs retain the control.

The first frames capture was `0x14e0e60a47fe2be3`. Compared with the archived
iframe-lane reference it differs at one pixel, (347,356), in the parent's caption:
RGBA (169,163,155,255) became (169,163,154,255). Geometry and child-frame pixels
match. Captures 2, 3 and 4 exactly match the reference; `ortet/pixel-comparison.json`
and all images/logs preserve the outlier. Three consecutive matching captures
meet the repeat check, but this fixture is not unconditionally byte-stable.

No new architectural ruling was substituted for Mark's one-Runtime-per-agent
decision. Completing cross-arena DOM adoption and ownership-aware routing is the
next required implementation boundary; stable WindowProxy navigation, full
teardown, incumbent settings, dynamic rAF ordering, lazy-frame activation and
document.domain remain separately open. This continuation is not accepted as a
closed realms lane. The coordinator created no commits or stashes.


### Cross-arena adoption foundation (verified, 2026-09-09)

Mark authorized working on the adoption blocker after the continuation report.
The first implementation boundary is permanent node identity and detached
storage transfer. It does not enable authored `adoptNode` or insertion across
arenas. The four named WPT regressions above remain acceptance failures until
the complete runtime mutation and lifetime path is implemented.

**Identity.** `NodeId` becomes an opaque u64 on every target: a checked 24-bit
allocation-arena namespace and 40-bit monotonic serial. Namespace and serial
exhaustion fail before reuse. Native host, reflector, accessibility and render
boundaries carry the full value. Storage keys preserve the complete identity;
physical membership, rather than birth namespace, determines the owning store.
Capture/replay keeps an explicit local-serial translation and refuses imported
identities until an import translation exists.

**Transfer.** `ScriptedDom::transfer_detached_subtree_to` preflights an ordinary
detached subtree before moving its records with unchanged IDs. It returns the
moved IDs for host ownership/root relocation. Pending source work, documents,
shadow/template/slot state, invalid trees and collisions are refused before
mutation. This lower-level operation does not implement DOM adoption steps or
move JS wrappers, ranges, observer registrations, script state or live iframe
contexts. Runtime cross-arena refusal remains in place.

**Foundation verification:** 674 distinct native tests pass with zero failures
and zero ignored: 448 script-runtime-api, 112 genet-scripted, 49 genet-documents
and 65 store tests. The same 65 store tests also pass in release, including
refusal atomicity, round-trip transfer, pin relocation and reclamation. Both
engines preserve an odd ID above 2^53 through host dispatch and GC. The wasm32
store probe compiles and executes in Node v24.11.0 without host imports,
preserving `9011597301252097` through its u64 boundary and checking transfer and
reclamation. This standalone probe has its own archived lockfile; it is not
browser-hosted realm acceptance.

The interface-table drift test, workspace check with `genet-wpt/netfetch`,
Clippy on the storage/runtime/document crates and formatting checks pass;
warnings remain recorded. A release control using the original f949210ca324
store sources fails as expected because two stores both identify their root
as `NodeId(0)`. Final sources, inherited patches, locks and logs are recorded in
`testing/genet/wpt-ledger/2026-09-09_dom-adoption-foundation` outside the repo.
The WPT harness binary unit suite additionally passes 69 tests with its three
existing manifest/test262/diagnostic ignores retained; the worker library suite
passes all four tests on each native backend. The workspace lock is
unchanged. This slice performs no fresh WPT census,
baseline repin or headed rendering receipt.

**Next runtime boundary:** maintain NodeId-to-current-owner independently of
NodeId-to-creation-realm. Route every node read and mutation to the former,
return canonical wrappers through the latter, and relocate pin/death accounting
before collection. Source and destination range/observer state must participate
in one semantic adoption operation. Removing only the append/adopt guard is
insufficient. Shadow/template ownership, capture translation and instantiated
iframe movement require their own explicit supported semantics.


**Runtime coordination inventory, 2026-09-09.** Review of the actual bootstrap
confirms that `__listeners` and handler properties belong to the retained
wrapper. A second listener registry is unnecessary. `Node.dispatchEvent` must
end its propagation path at the current root document's `defaultView`; its
creation-realm `globalThis.window` becomes wrong after adoption. `ownerDocuments`
and custom-element upgrade/connection maps need coordination in each affected
wrapper realm. Range endpoints belong to Range objects, but `rangeIndex` and
mutation hooks must reach every realm indexing affected nodes. Observer
registrations (`node.__moRegs`) follow the wrapper; destination capture interest
and source-created observers' pending records/notification queues must remain
connected. Neither observer nor Range participation can be inferred solely
from the node's wrapper realm.

The next private agent boundary therefore separates current host ownership,
canonical wrapper realm, trusted rooted per-realm coordination hooks, and
node-indexed Range/observer participants. No new public mutation API is needed.
DOM [adopting steps](https://dom.spec.whatwg.org/#concept-node-adopt), checked
2026-09-09, additionally require descendant and attribute document ownership,
shadow-tree handling and custom-element reactions. Storage transfer alone does
not supply them. Existing authored adoption refusals remain explicit until the
semantic boundary and its two-engine identity/observer/range tests pass.


### Runtime cross-arena adoption (in verification, 2026-09-09)

**Status:** implementation is in the working tree; new runtime gates are still
running. This phase supersedes the foundation's authored-adoption refusal for
ordinary subtrees only. It does not close the enclosing realms lane.

The private agent coordinator in `script-runtime-api/dom/adoption.rs` resolves
physical storage ownership independently of each node's creation realm. Genuine
reflectors are origin-checked before routing. Canonical wrappers, their original
prototypes, expandos and listeners remain in the creation realm. Private rooted
per-realm hooks coordinate owner documents, custom-element reactions, ranges,
observer delivery and GC groups; snapshot restores re-register their own cloned
hooks. Failed realm installation drops its hooks and failed reflection does not
commit a storage pin.

Adoption preflights the complete ordinary subtree before detachment, consumes
source observer records, preserves pending layout invalidation records, moves
storage with stable IDs, and relocates pins and already-started script state.
Insertion validation precedes removal, including document child count/order and
reference membership. Native multi-node mutation sinks refuse mixed owners that
bypass this transaction. Replacement observer groups follow the destination
owner, with agent-side nesting; delivery waits until the group closes. Fresh
adopted scripts prepare in the destination realm, and event bubbling ends at the
current document's window.

GC resolves physical components across creation realms before collection and
retires dead-reflector pins at their current owner. Retaining only a descendant
must retain ancestor/sibling wrappers and their state; releasing all script roots
must allow reclamation. Retained layout consumers tolerate transferred nodes in
old mutation records while invalidating the source and destination parents.

**Done-conditions:** both-engine adoption/identity, observer, range, script,
replacement, native-boundary and reclamation regressions pass; snapshot and
engine callback-root tests pass; touched-crate tests, table drift, workspace
check, Clippy and formatting complete; exact-source receipts distinguish fresh
WPT evidence from historical realm census results.

**Remaining boundaries:** associated shadow/template/slot trees and iframe,
object, embed and canvas ownership transfer still refuse explicitly. Active
parser/observer-group transfers refuse. Host document stream operations borrowed
across arenas refuse before mutation. Capture import translation, full browsing
context teardown/navigation and native headed G5 acceptance remain open. The
initial coordinator scans hosts for ownership and fans participant hooks out to
same-origin realms; this is a correctness path, not a measured scalability claim.

Verification logs and final source receipts belong under
`Code/testing/genet/wpt-ledger/2026-09-09_dom-adoption-runtime`. Until the results
below are recorded, this phase has no passing runtime acceptance claim.


**Verification progress, 2026-09-10:** the storage suite passes 68 tests in debug.
The first integrated runtime run passed all 8 replacement, 4 associated-tree/
full-width identity, 2 native-bypass and 6 owner-native-boundary cases. Its five
identity/lifetime failures exposed incorrect detached document lookup and a
Nova-only mixed-component lifetime failure. Canonical-realm private document
lookup fixes the former. The next run passes 15/16 identity/lifetime cases and
all 6 queued async/deferred/module adoption cases; Nova still loses an ancestor
wrapper's expando after collection with only a foreign-created descendant held.
The pre-collection identity checks pass. This is an acceptance failure under
investigation, not a timing artifact or a permitted weakened identity contract.

The GC-policy rewrite now roots its starting inventory until every participant
realm has installed its component edges, then releases detached temporary roots
before collection. That closes an allocation-time collection window but did not
alone resolve Nova's observed failure. Parser deferred queues now retain node
IDs, and pending async/deferred execution skips nodes transferred out of the
preparation arena. Same-arena adoption into another inert document still needs
an explicit native node-document identity seam; physical membership alone does
not establish that case. Logs retain each failed build/run and its successor.


**Further verification, 2026-09-10:** the storage suite also passes all 68 tests
in release. The native Vano GC unit and integration gates pass; a new native
Symbol/WeakMap regression additionally proves retention through a late-marked
key and release after its strong root is removed. The collector resolves pending
ephemerons after all strong queues in each marking pass. The regression is
constructed to expose the previous queue ordering; an old-order negative binary
was not built. The final Nova realm suite passes all 19 tests, including the
retained-callable and cross-realm ephemeron cases.

A separate integration defect was found in GC-policy arguments: evaluating a
bare quoted string as a script can produce an undefined completion for a directive
prologue on Nova. Policy operation and payload strings now use parenthesized
expressions, and unknown private hook operations throw. The retained-callable
fixture used the same bare-string form and was corrected with a value assertion.
The full runtime/document rerun is still required to establish that the remaining
mixed-component lifetime failure is closed; the collector change alone is not
claimed as its demonstrated cause.

Capture recording now rejects an unrepresentable imported identity or an exported
node requiring current-value readback before consuming the journal or writing a
batch. Historical removal records remain representable. The new recorder tests
are included in the pending full scripted gate. Capture identity translation and
general filesystem write-failure atomicity remain outside this change.

**Contract handoff and corrected core gate, 2026-09-10:** the first full core
batch passed 49 document, 495 layout, 114 scripted and 482 runtime tests; five
runtime tests failed. The layout suite retained six pre-existing ignored tests.
All original cross-arena adoption cases passed, including Nova mixed-component
retention and release. Two soak fixtures incorrectly appended two document
roots; they now use one container without weakening their lifetime bounds.
Two policy-cost tests exceeded the unchanged 50 ms bound. The policy now avoids
re-rooting already persistent roots and borrows the sole store once for its
single-realm classification path. Both focused cost gates pass. The fifth
failure exposed a Range index retaining resolved weak targets strongly; the
index now retains weak entries and resolves targets only in temporary results.
The focused snapshot reclamation test passes, and a full corrected core rerun
is in progress. A new two-engine Range release regression accompanies the fix.

Every collection policy tick visits MAIN_REALM and all registered child hosts.
All realm policies are installed before temporary roots are released; death
inventories retire pins at current physical owners before stores collect.
Creation-realm locality is therefore not a sufficient native storage contract
for adopted nodes. A follow-on should make current-store access and explicit
owner-resolved access distinct, while preserving canonical wrapper creation.
The page-level Boa WeakRef versus native reclamation report needs its own job-
boundary regression; accounting-only lifetime tests do not settle it.

The earlier 44-unexpected DOM guard is historical evidence, not this slice's
result. A fresh runner and canonical guards are still required; lost passes
must not be repinned away. Required local Boa/Vano patches must be published on
the consumed branches with reproducible dependency revisions before a clean-
clone acceptance claim. Local source archives do not satisfy that condition.
This working-tree slice makes no commit and does not close broader G5, context
navigation/removal, associated-tree transfer or capture identity translation.

**Page weak reachability, 2026-09-10:** all four focused local/adopted page
WeakRef regressions pass on Boa and Nova (`page-weakref-3.log`). They check page
weak survivors against original wrapper identity and current native liveness,
and eventual reclamation after actual Promise-job checkpoints. The previously
reported Boa stale-wrapper behavior did not reproduce. An initial Nova failure
was an invalid fixture assumption: its host evaluation return ends the job and
clears kept objects. The corrected fixture keeps within-job checks in one
script evaluation and explicitly roots a successful weak result across later
host checks. This receipt does not close every host lifecycle boundary.

**Second corrected gate and fixture reconciliation, 2026-09-10:** the full core
rerun recovered both touched-node soaks and snapshot reclamation. Its runtime
unit suite passed 142/143, with Nova reparent-plus-policy cost at 72.6 ms
against the unchanged 50 ms ceiling (quiet policy 11.6 ms). The second bounded
optimization reuses captured inventory and skips duplicate connected rooting
only in the single-realm path. Multi-realm refresh/rooting remains intact for
wrappers minted by preceding hooks. Cleanup borrows root sets, releasing only
the original inventory after every hook returns. The next focused cost gate
passes: Boa quiet 7.9 ms/reparent 25.2 ms; Nova quiet 8.0 ms/reparent 44.8 ms.
That gate overlapped release compilation.

The new mutated-Range case initially failed on Boa because it never completed
a JavaScript job after WeakRef resolution. It now runs an actual Promise job
before reclamation checks. This fixture correction preserves the Range-index
strong-reference leak fix. A revised runtime suite is running.

A separate fixtures-only merge moved main to `8ad0c6b6d1a` before this core
rerun; this lane made no commit. Its ignored fixtures were reviewed: 18 ordinary
cases are now enabled, correcting connected-removal Range assertions,
source-captured custom-element owner comparisons, and detached-component GC
checks. Two live-iframe relocation cases remain ignored with the explicit
browsing-context ownership boundary; two negative controls remain enabled.
Final manifests record the actual base and working files; earlier receipts
retain their historical bases.

**Runtime acceptance and WPT regression repair, 2026-09-10:** the revised
runtime gate passes 513 tests, with only two live-iframe relocation fixtures
ignored. The focused page WeakRef and Range-release cases also pass. The first
fresh runner (SHA25613c8df89016192cad69fd0965ca6bbddcd389d20f1f9066f089fb39b39f4ede6)
reports both reftest guards at zero unexpected and the DOM testharness guard at
56 unexpected files. This is a mix of gains and losses, not an acceptance claim.
Named comparisons found three null-argument type-error regressions and two
Document-clone regressions introduced by stricter insertion validation. The
argument fix validates conversion before hierarchy; its 48 active targeted
tests pass. Document cloning now starts empty and assigns recursive ownership
to the cloned document; its two-engine regressions are queued in release.
Full named-result comparison and corrected-runner guards remain pending.

Other focused limitations are distinguished in the receipt WPT_NOTES.md:
inherited asynchronous custom-element reaction delivery fails synchronous
CEReactions expectations even within one document; iframe refusal includes
fresh iframe admission as well as live-context relocation; popup support and
template-associated/partial-update APIs remain open. None of these failures
is repinned away by this lane.

A separate disk-reclamation task removed both target dependency caches while
the follow-up scripted GC build ran. The user confirmed that cleanup cause.
Sources, completed passing gate receipts and the archived runner remain intact.
The release cache is rebuilding; remaining targeted tests use release artifacts.

**Remaining WPT losses and routing cost, 2026-09-10:** Document cloning and
argument/ancestor/reference validation order are repaired; the final focused
release tests pass on both engines. Named WPT comparison confirms that
appendChild preserves all prior passes and gains five (11/11), and insertBefore
preserves all prior passes and gains fourteen (26/40). The source keeps defensive
argument checks after authored getters and validates before detachment.

The Range replaceData file still exceeded the canonical 30-second limit with
builds finished, while a separate 120-second diagnostic passed all 1,146
assertions. This is an unresolved timing gate, not 1,146 assertion failures.
Owner lookup now records creation arenas when a realm registers and avoids
copying/repopulating the registry on every native read. Physical membership and
registered-host identity guard the local fast path; a slow refresh handles public
host DOM replacement without reassigning an imported node's creation realm.
A new unit regression covers that replacement. Full runtime and unchanged-policy
WPT validation of this optimization remain in progress.

**Publication, 2026-09-10:** after the user authorized commits and pushes, the
required fork changes landed as Boa `5a58112579cecaba6ba59d3310901992003fbb31`
on `origin/genet` and Vano `abfe3e4641de01f0f0a2b99667fd7397b262cb8f` on
`origin/main`. Both engine manifests now use those immutable revisions.
An isolated Cargo graph outside local path overrides resolved boa_engine,
boa_gc and nova_vm to those exact Git sources; metadata and its generated lock
are retained in the runtime receipt. The graph was seeded with the workspace's
known registry versions after an unconstrained resolver search was stopped.
This proves published dependency selection, not a full fresh-clone build.
The full release runtime rerun passes 518 tests, zero failed, two live-iframe
fixtures ignored. Clippy also passes with its recorded warnings.

The rebuilt runner `62f294050e0e30fc894ad95f01b1208580af5eb15d4b97135d20640b76140e0c`
passes the quiet replaceData gate at the unchanged 30-second worker limit,
with all 1,146 subtests passing. Whole-command wall time is 31.0 seconds,
including runner startup/reporting; this is not a changed worker deadline.
The full named baseline sweep is running with the same default policy.

**Full DOM named census, final runner:** 203 checked-baseline subtests improve
to pass. One prior pass remains lost: `Node-isConnected.html`,
`Test with iframes`, already failing in the earlier realms-continuation receipt.
Every former ordinary insertion/clone loss is restored. Range replaceData passes
all 1,146 assertions in the full default-policy run as well as in isolation.
The broad DOM guard still reports 59 unexpected files because its checked
expectations also differ on gains and nonpassing statuses. It is not green, and
the iframe pass is not repinned away. Against the historical longer-timeout
realms-post map, the only lost names are in Range dataChange (2,328); the checked
30-second baseline already marks that file hang-killed, so those cross-policy
counts do not describe a new adoption assertion regression. Remaining canonical
subsets are still running.

## Ordinary adoption lane: final verification and handoff, 2026-09-10

The bounded ordinary-subtree implementation is ready for publication. It
preserves canonical creation-realm identity while moving physical ownership,
with same-origin checks, complete preflight before detach, Range/observer
participation, script preparation and owner-store collection. It does not
accept associated template/shadow or browsing-context-bearing subtree transfer.

- Release script-runtime-api: 518 passed, zero failed, two live-iframe fixtures
  ignored. Both engines are covered; counts are combined, not per engine.
- Release scripted GC follow-up: two passed; runtime Clippy and release runner
  build completed successfully. Earlier document/layout/store/engine gates and
  corrected failed attempts remain in the receipt with their source boundaries.
- Final runner: `62f294050e0e30fc894ad95f01b1208580af5eb15d4b97135d20640b76140e0c`.
  Range replaceData passes 1,146/1,146 both alone at the unchanged 30-second
  worker limit and inside the full DOM sweep.
- All 14 canonical testharness subsets were evaluated using the standard
  defaults. The receipt wrapper adds named-result output and continues after
  failures; it does not change policy or repository expectations. Twelve
  subsets report zero unexpected. Broad DOM reports 59 unexpected files and
  DOM/nodes reports 35. These overlapping guards include gains/status changes
  and the same single lost pass, `Node-isConnected.html: Test with iframes`.
  That loss already existed in the prior realms continuation and remains
  protected. The DOM census gains 203 named passes, with every introduced
  ordinary insertion/clone regression restored.
- Both reftest guards report zero unexpected. Focused WPT fixtures on both
  engines retain the documented synchronous CEReactions, iframe/popup and
  template/partial-update boundaries; passing local approximations are not
  substituted for those exact fixtures.

The three earlier realms expectation repins preserve every prior pass and are
distinct from this adoption verification. No adoption loss was repinned away.
The ownership audit covers 55 continuation paths plus the two immutable engine
manifests. Boa `5a58112579cecaba6ba59d3310901992003fbb31` and Vano
`abfe3e4641de01f0f0a2b99667fd7397b262cb8f` are published; isolated Cargo
resolution selects those exact revisions without local overrides, and all 25
changed fork source/test files match the fetched copies modulo line endings.
This is dependency-selection proof, not a full fresh-clone build receipt.

The next owner can take broader G5 context/associated-state transfer and the
owner-resolved accessor refinement separately. Opaque rooting already iterates
every registered realm at each tick. Capture now refuses unsupported imported
identities before consuming the mutation journal; replay translation remains
open. Four page WeakRef regressions passed, without reproducing the reported
Boa stale-wrapper case. Full headed G5 and frame teardown/navigation remain
unaccepted. Final source/runner archives, publication identities and cache
release status are under `testing/genet/wpt-ledger/2026-09-09_dom-adoption-runtime`
in the shared Code workspace.

## Phase: owner-resolved accessor and imported-identity replay, 2026-09-10

**Genet commit at phase start:** `741bf726eb4`
(`741bf726eb42283a11961d24dd0da14cb08d0f5b`), clean tree. This phase makes no
commit. Receipts: `testing/genet/wpt-ledger/2026-09-10_accessor_replay` in the
shared Code workspace.

The ordinary-adoption handoff left two items: "the owner-resolved accessor
refinement", and "capture now refuses unsupported imported identities before
consuming the mutation journal; replay translation remains open". Both are the
same fact seen twice. Adoption made a node's **creation realm** and its
**owning store** independent, and two places still assumed they were one: a
native sink that decoded an agent-wide reflector and then read "the current
document", and a capture record that carried a bare serial with no arena.

### 1. Owner-resolved accessor

The engine contract gains `CallCx::local_reflector_data`, replacing
`reflector_is_local`. It returns the reflector's data **only** when the
reflector was minted in the callback's current realm, so the locality test and
the decode cannot be separated — the previous pair invited "decode now, check
the arena later", and its own doc line said so. Its doc names what it answers
for: the realm `current_realm` reports, and that realm's host arena. Boa reads
the immutable `Reflector { owner, data }` provenance in one downcast; Nova
reads `EmbedderObject`'s owner and embedder data in one match; neither consults
a raw id or a public prototype, so neither is forgeable from script.
Single-realm backends keep the trait default, which is exactly `reflector_data`
— true there, since one realm has one arena.

`CallCx::reflector_data` stays agent-wide and unchanged. It says *which node*,
never *which arena may dereference it*.

Above it, `script-runtime-api` replaces the old `LocalReflectorCx` helper with
`OwnerResolvedCx::owned_node`, which resolves a reflector to an `OwnedNode`:
the host that **physically owns** the node, plus the `NodeId` that host's arena
stores it under. `OwnedNode::with_dom` / `with_host` are the only way to
dereference it, so the arena is not a separate choice a sink can get wrong. The
arena-local case is answered by the engine accessor; a same-origin reflector
from another realm is a deliberate cross-arena read and falls through to
`reflector_data`, resolved to *its* owner rather than read in this one. A dead
node or a cross-origin owner still throws exactly the errors it did before —
`validate` is now that resolution with the result dropped, so there is one
implementation of the rule instead of two.

`require_same_owner` follows: the operands arrive already resolved, so it
compares the stores they resolved to (`Rc::ptr_eq`) instead of resolving each
one a second time.

**Counts.** Before: **60 native sinks**, across 72 call sites in eight files,
took a bare `ReflectorData` from the agent-wide accessor and then dereferenced
it — safety by convention at every one. After: **0**. All 60 go through
`owned_node`, at 71 call sites (two reference-node decodes merged, one added).
The wide `reflector_data` remains at **7 call sites in three files**, all of
them intended cross-arena reads: `dom/adoption.rs` (`host_for_call`, the
`wrap`/`ownerDocument` hook, `prepareScripts`, `moGroup`, `transfer`),
`dom/tree.rs` (`NodeRealmState`, the foreign/local report the bootstrap refuses
adoption with), and the fallback inside `owned_node` itself. Structured clone,
event retargeting and messaging decode no `NodeId` natively — they run through
the bootstrap — so they needed no site here.

The one sink that must *not* be owner-resolved is `stream_target`
(`document.write` and friends): a stream belongs to the callback realm's own
arena, so it resolves and then additionally requires local physical membership,
refusing a same-origin foreign document rather than writing into it.

### 2. Imported-identity capture and replay

`genet-scripted-dom` gains `CapturedNodeId { arena, serial }` — an identity, as
against a serial, which is not one. Once adoption moves nodes with their
identity intact, two arenas can hold the same serial, so a journal carrying
only the serial replays onto whichever node the replaying arena happens to have
allocated at that index. That is the consumer audit's finding 2, and it is
worse than the panic in finding 1 because it is silent.

`ScriptedDom` now keeps an **import registry**: the set of arenas it has
imported nodes from, written by `transfer_detached_subtree_to` as part of the
transfer it already performs. `try_capture_node_identity` records the origin
arena instead of refusing a foreign one it can account for;
`try_remint_node_identity` translates a local origin through the existing
serial check, translates a registered imported origin by packing the identity
and checking physical membership, and refuses an unregistered origin with the
new `NodeIdentityError::UnknownOriginArena` rather than reminting.

`genet-scripted`'s `RecordedMutation` carries `CapturedNodeId` in every node
field, and gains `replay_node` / `replay_ids`, which translate through the
registry and return the typed `ReplayError::UnknownOrigin` / `Unresolvable`.
Every identity in a record resolves before a replayer touches the document, so
one unknown origin refuses the record rather than half-applying it. The
fallible path is the only path in `capture.rs`, including its tests, which
previously asserted against the panicking `capture_node_id`.

The wire format changed rather than gaining a version shim, per the doc
policy's §3. The retired `layout` field stays as it was.

**What stopped refusing, deliberately.** The old
`recorder_refuses_imported_batch_without_consuming_live_mutations` asserted the
placeholder: an adopted node's first destination mutation refused the whole
batch. That is now the supported case, and the test became
`recorder_records_an_adopted_node_and_replay_resolves_the_same_live_node`. The
exported-readback refusal, the historical-removal representability and the
identity-space refusals are unchanged.

### Named regression manifest

| Regression | Where |
|---|---|
| `reflector_locality_uses_owner_not_raw_id` (Boa and Nova) | `script-engine-boa/lib.rs`, `script-engine-nova/tests/realms.rs` — now driving `local_reflector_data`; canonical and uncached equal raw ids, prototype changes, non-reflectors, forced GC |
| `owner_resolved_native_reads` (Boa and Nova) | `script-runtime-api/tests/cross_arena_adoption.rs` — after adoption, the attribute, `textContent` and `innerHTML` sinks reached from the creation realm all write the owning arena, and the creation arena no longer holds the node |
| `recorder_records_an_adopted_node_and_replay_resolves_the_same_live_node` | `genet-scripted/capture.rs` — the record names the origin arena, and every identity in it replays to the live imported node |
| `replay_refuses_a_serial_from_an_unregistered_arena` | `genet-scripted/capture.rs` — two stores whose serials collide; replay refuses with `UnknownOrigin` instead of resolving the decoy, and the same record still replays in its own arena |
| `adopted_node_capture_replays_to_the_same_live_node` (Boa and Nova) | `genet-scripted/document.rs` — a child-realm `<p>` adopted into the parent document, mutated there, recorded and replayed to the same live node through a real two-realm document |
| `recorder_refuses_exported_readback_without_consuming_source_journal` | `genet-scripted/capture.rs` — retained unchanged |

### Gates

Runner SHA-256s: `pre`
`81e63419d128b501e5ed71e278fbd5c956cbaf6a2c0708b530e5bf1e5c886ec0`, built from
`741bf726eb4` **before the first edit**; `post`
`60f5600817c9f900f174dea601dec89356d7fd98e7d704bf094513cb720f1a24`.
`Cargo.lock` is git-ignored here, so both digests are recorded in the receipt;
the whole difference between them is one added `serde` edge on
`genet-scripted-dom`, with no revision movement — which is what makes the null
census below readable rather than two effects cancelling.

Census, disk mode, `--engine boa --renderer livery --jobs 8 --timeout 240`:

| Subset | Files | Subtests passed (pre → post) | File moves | Subtest moves |
|---|---|---|---|---|
| `dom` | 698 | 46,593 → 46,593 | 0 | 0 |
| `html/semantics/embedded-content/the-iframe-element` | 164 | 23 → 23 | 0 | 0 |
| `shadow-dom` | 314 | 1,522 → 1,522 | 0 | 0 |
| `webmessaging` | 160 | 169 → 169 | 0 | 0 |

Zero pass-to-fail movements, so nothing needed explaining or fixing. Twelve of
the fourteen canonical testharness slices report `unexpected=0`; broad `dom`
reports 59 and `dom/nodes` 35, the same counts the ordinary-adoption lane
recorded, and the census shows this phase moved neither. `dom` and `dom/nodes`
were not repinned, and the protected loss `Node-isConnected.html: Test with
iframes` was not touched. Both reftest guards report `unexpected=0`. Both Ortet
receipts are unchanged — `article` `0x6377ba8a6bf4dbc9` and `frames`
`0x97bdd4bd9e03ec02`, three consecutive matching captures each.

Native: the release `script-runtime-api` suite passes **520 tests, zero failed,
two ignored** (the two live-iframe relocation fixtures, which belong to the next
lane) — 518 before this phase plus its two accessor regressions. The debug suite
matches. `genet-scripted` with `scripted-nova` passes 115 library and 2
integration tests; `genet-scripted-dom`, both engine adapters and the engine API
pass. Clippy on all six touched crates with `--all-targets` exits clean
(warnings retained in the log), rustfmt is clean on every touched file, and
`cargo check --workspace --features genet-wpt/netfetch` passes.

### What this phase does not do

It does not widen the adoption boundary: associated template/shadow trees and
browsing-context-bearing subtrees still refuse. It does not implement a capture
*replayer* — it implements the identity translation a replayer needs, and proves
it against real records. It does not touch full G5, headed acceptance, or frame
teardown and navigation.
