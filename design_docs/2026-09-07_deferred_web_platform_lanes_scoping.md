# Deferred web platform lanes: scoping

**Date:** 2026-09-07

**Status:** research, reviewed 2026-09-07. This inventory does not schedule
its API lanes. The shared scripted-host direction is recorded in Ortet O5. Each
section separates implementation choices from required observable behavior
and names the proof a promoted execution plan must carry.

**Parent:** [Web platform WPT census](2026-09-06_web_platform_wpt_census.md).
The [harness repair](2026-09-07_wpt_harness_repair_plan.md),
[IDL interfaces](2026-09-07_idl_interface_table_plan.md),
[cheap globals](2026-09-07_cheap_globals_plan.md),
[MutationObserver](2026-09-07_mutation_observer_plan.md),
[XMLHttpRequest](2026-09-07_xhr_plan.md), and
[Selection and Range](2026-09-07_selection_range_plan.md) have dated plans
and bounded landing receipts. Their residuals remain prerequisites where
named below. Dedicated Worker is scoped here; this document does not assign
an execution owner or claim that a separate Worker plan exists.

Counts below are the 2026-09-06 disk-mode census: files as pass / fail /
error / no-results / skip, then subtests passed of total. They remain
historical, including where later lanes have improved them. A parent-directory
count is not a baseline for one of its subdirectories.

## How to read a section

Each section has the same shape:

- **What exists.** Initial inventory at `5af76a0cb8c`, reconciled against
  committed `ddee26ef559` and the linked landing plans on 2026-09-07.
  Concurrent DOM node-model edits are outside this review's receipt.
- **What the lane is.** Engine work and its ordering constraints.
- **Decisions.** Implementation, host-policy and staging choices. Standards
  requirements are constraints; a staged omission must be named as a residual.
- **Done-conditions.** Required behaviors and candidate test families. A
  promoted plan must freeze the exact selection before implementation.

### Shared acceptance and host prerequisites

Every promoted slice records the Genet/WPT revisions, dependency lock and
runner hashes, features, engine, renderer, server mode, exact test/variant
manifest and pre/post result maps. Use a comparable pre-run with the same
dependency graph and configuration; the September 6 census alone cannot
isolate engine changes from the subsequent harness repairs. Measure
network-dependent behavior against a live server with the required origins
and HTTPS. Record unsupported variants and every residual by name.

The selected behavior slice must pass its required assertions, including
negative cases; increased aggregate counts are progress, not completion.
Hold a named regression manifest and explain any changed result. Unit and
script-runtime contracts should exercise both Boa and Nova where supported;
the historical Boa census is not a Nova receipt. Pixel gates identify the
backend and the test's prescribed comparison tolerance.

[Ortet O5](2026-09-03_ortet_founding_plan.md#o5-scripted-platform-host-native-route-accepted-2026-09-08)
is the accepted native Genet-owned route for JS-driven headed receipts below. Mark's
[testing-role ruling](2026-09-03_ortet_founding_plan.md#platform-testing-roles-marks-ruling-2026-09-07)
keeps `genet-wpt` responsible for automated conformance/scoring, Ortet for
headed engine/session proof, and Pelt for downstream Mere/Genet composition.
Pelt is not a prerequisite for accepting Genet's own hosted behavior.

Ortet now selects script-free, Boa and Nova modes and has accepted native
production scheduling/resource integration at exact source `f3dc1bcf909`.
That qualifies Ortet as the native headed route; it does not qualify a later
API merely by hosting its fixture. Storage restart receipts still name an
explicit test provider and directory, and browser-hosted scripting retains a
separate gate. Later API lanes supply their own fixtures through Ortet.

Genet owns web-observable semantics and raw provider contracts. Mere's hosts
own profile selection, persistence provisioning and product policy. A test
provider in Genet can prove engine behavior without importing Mere.

## Shadow DOM

Census: `shadow-dom` 6 / 215 / 42 / 2 / 49, subtests 18 of 8,654.
`custom-elements` 3 / 144 / 29 / 1 / 10, subtests 2,041 of 3,674, is the
sibling with partial coverage and the reason this lane matters: real
component libraries pair the two.

**What exists.** `layout-dom-api` already has the seam: `flat_children`
on `LayoutDom` is documented as "slot-assigned for shadow hosts, otherwise
DOM order" and defaults to `dom_children`. This is a child-iteration hook,
not a complete shadow-tree contract. Livery's selector adapter in
`components/genet-livery/src/dom.rs` answers `is_html_slot_element`, but
`parent_node_is_shadow_root` returns false and `containing_shadow_host`
returns None. The scripted DOM
arena (`genet-scripted-dom`) has no shadow root node kind, no host
pointer, and no slot assignment. The event path in the bootstrap is built
as target to root plus window, with a comment that there is no retargeting
or composed boundary yet. `<template>` is display:none in the UA sheet and
parses as an ordinary element; there is no template contents fragment.

**What the lane is.**

1. A shadow-root representation in the arena: host link, mode, delegatesFocus,
   and a per-host slot assignment table maintained on mutation (named
   slots, the default slot, `slotchange`).
2. Explicit DOM, shadow-including and flat-tree relationships. Implement
   assigned/fallback children and the corresponding ancestry needed by
   layout and inheritance. Audit style traversal, box construction, paint,
   hit testing, invalidation, selection and accessibility individually.
   Selector combinators and DOM APIs retain their specified tree and scope;
   replacing every `dom_children` call would be incorrect.
3. Style scoping: a shadow tree's stylesheets apply inside the root, with
   `:host`, `:host()`, `:host-context()`, `::slotted()` and `::part()`; the
   ordinary document selectors do not cross in, while inheritance follows
   the flat tree. Implement host/slotted pseudo-classes and pseudo-elements,
   part exposure, scope-aware matching and cascade ordering across
   `livery/src/selector.rs` and `genet-livery/src/dom.rs` and style/invalidation
   consumers. Existing cascade scope plumbing does not establish that boundary.
4. Event retargeting and `composedPath()` in the bootstrap's dispatch,
   plus `composed` on Event init.
5. `attachShadow`, `shadowRoot`, `assignedSlot`, `assignedNodes`,
   `getRootNode` with `composed`, and `<template>.content` as an inert
   DocumentFragment owned by the appropriate inert template document.
   Template contents use one associated inert document per owning document,
   shared by its templates; cloning and adoption preserve the specified
   ownership. See the [HTML template model](https://html.spec.whatwg.org/multipage/scripting.html#appropriate-template-contents-owner-document)
   and [CSS shadow/flat-tree model](https://drafts.csswg.org/css-shadow-1/#flat-tree).

**Decisions.**

- How the arena represents the shared inert document and shadow roots while
  preserving `ownerDocument` and tree-scope observability.
- Whether declarative shadow DOM lands in the first slice. When supported,
  script-free rendering must also realize its flat tree. Fleece's extraction
  policy is separate: preserve source identity and make any rendered-text
  traversal explicit under its [preservation contract](2026-09-05_fleece_preservation_contract_plan.md).
- Declarative shadow DOM parsing in html5ever versus a post-parse pass.

**Done-conditions.** Freeze shadow-dom tests for named/default slots, fallback
content, reassignment after mutation, style isolation and inheritance,
retargeting and closed-root visibility. All selected assertions pass; named
custom-elements, MutationObserver and Selection/Range regressions hold.
Reftests prove slot geometry and style changes after reassignment. Template
tests prove shared owner identity, inertness, clone and adoption behavior.
Hold Livery/Buckram suites and a Fleece receipt identifying the traversal and
source anchors of extracted passages. Declarative support, if deferred, stays
a named parser/static-rendering residual.

## Canvas 2D

Census: `html/canvas` 33 / 2,160 / 6 / 2 / 472, subtests 33 of 4,142. Of
the 2,673 files, the `element` and `offscreen` families dominate; the
worker variants need a worker host. OffscreenCanvas itself is exposed on
Window as well as Worker and need not wait for dedicated Worker.

**What exists.** `HTMLCanvasElement.getContext` in the bootstrap returns a
WebGL context (via `webgl-wgpu`) for `webgl` and `experimental-webgl` and
`null` for everything else. The canvas element is a replaced box in Livery
with width and height presentational hints and an external-texture key that
the paint list carries to netrender, so a raster produced anywhere on the
same wgpu device can be composited into the page today. The workspace still
declares `vello_cpu` and the WPT tree has a `vello_canvas` subsuite config
naming a `dom_canvas_backend=vello` pref; nothing in the current tree reads
either.

**What the lane is.**

1. A `CanvasRenderingContext2D` state machine: the path and transform
   stack, fill and stroke styles, gradients and patterns, line styles,
   shadows, compositing and blending modes, clipping, text with the
   Livery text system, `drawImage` from images, canvases and video,
   `ImageData`, and `toDataURL` / `toBlob`.
2. A rasterizer behind it. Candidates are netrender's Vello path on the
   page device and the workspace-declared `vello_cpu`. The latter still needs
   integration and target verification. Both need an explicit backing-bitmap,
   color/alpha, readback and texture-lifetime contract; an external-texture
   key alone does not supply it.
3. The DOM side: `getContext('2d')` with the context-mode rules (a canvas
   is either 2d or WebGL, never both), `width`/`height` reset semantics,
   `ImageBitmap`, origin-clean/taint propagation and protected readback and
   export. Window-side `OffscreenCanvas` can precede its transfer/worker path.
   See the [HTML canvas model](https://html.spec.whatwg.org/multipage/canvas.html#the-offscreencanvas-interface).

**Decisions.**

- Backend: Vello, `vello_cpu`, or both behind a host setting. Measure the same
  selected contract on each supported backend. A CPU implementation is not
  automatically a conformance oracle, and CPU passes do not prove GPU output.
- How canvas text consumes the shared Parley shaping and font-resolution seam.
  Vello's glyph raster path does not replace text shaping; measure canvas text
  independently of DOM layout.
- Physical backing allocation and synchronization. The logical bitmap must
  persist between drawing calls and obey resets regardless of allocation.

**Done-conditions.** Freeze element-canvas tests from `2d.fillStyle`, `2d.path`,
`2d.transformation` and `2d.drawImage` for the selected first slice. Require
state save/restore, repeated dimension-set reset, context-mode exclusion,
pixel readback and tainted-source rejection. Name unsupported image sources,
text and offscreen variants rather than claiming the entire context. A
scripted-host reftest proves page composition, resize and texture retirement;
the chosen wasm backend has build and runtime pixel receipts.

## Service workers

Census: `service-workers` 0 / 266 / 17 / 0 / 9, subtests 0 of 1,526.
Registration, lifecycle and interception behavior require a live server;
disk-mode interface assertions do not measure them.

**What exists.** netfetcher is the network stack: cache, cookies, CORS,
HSTS, redirects, referrer policy, HTTP/3 and WebSocket are real. The
`FetchHandler` seam in `script-runtime-api/fetch.rs` is where a page's
`fetch()` leaves the runtime; it already runs asynchronously with abort and
streaming. Document loads and external scripts also use `ResourceFetcher`
(`genet-scripted/document.rs`); style/image resources use it through the
retained resource path. Wrapping script `fetch()` alone misses those routes.
There is no service-worker execution context, registration store or Cache API.

**What the lane is.**

1. A request-routing contract shared by script `FetchHandler` and the
   selected `ResourceFetcher` consumers. Carry client/controller identity,
   request destination, credentials/mode and cancellation through interception
   to a synthesized response or network fallback. Enumerate navigation,
   script, stylesheet, image and script-fetch coverage, including bypass and
   recursion rules. Genet owns interception semantics; hosts supply transport.
2. A worker execution context with its own event loop, global
   (`ServiceWorkerGlobalScope`), lifecycle (install, activate, waiting,
   redundant), `waitUntil`/`respondWith`, termination/restart and `clients`.
3. A registration store keyed by storage key and scope, persisted, with update
   checks and byte-for-byte script comparison.
4. The Cache API and `caches`, which is a storage engine of its own.
5. `navigator.serviceWorker`, `postMessage` both ways, and the `fetch`,
   `install`, `activate`, `message` events.

**Decisions.**

- Reuse a worker execution/event-loop substrate with dedicated Worker; the
  `Worker` constructor is not itself a service-worker prerequisite. Agree
  storage identity, transactions and host provisioning with the storage lane
  before choosing a backend. Cache API and IndexedDB can be staged separately.
- Whether navigation requests are intercepted at all in the first cut.
  Subresource-only interception is a bounded first slice. It cannot prove
  reopening an offline page because its navigation still needs the network.
- Process or thread placement. A worker on a thread in the same process is
  the natural fit for the current single-process host; the isolation Mere
  wants for untrusted content is a separate question.

**Done-conditions.** Freeze server-mode tests for registration/scope matching,
install/activate/update, fetch-event and messaging in the supported slice.
Prove controller selection, synthesized response delivery, network fallback,
abort and interception bypass. A persistent scripted host reloads a
registration after restart and delivers another event after worker termination.
Transport counters prove interception for each declared request class.
Navigation exclusion remains explicit; an offline-reopen claim additionally
requires a cached navigation receipt. See the [Service Workers specification](https://w3c.github.io/ServiceWorker/).

## iframes and nested browsing contexts

Census: `html/browsers` 40 / 527 / 134 / 37 / 44, subtests 143 of 1,700;
`html/semantics/embedded-content` 64 / 566 / 41 / 33 / 94, subtests 338 of
1,564; `html/anonymous-iframe` 0 of 34; the COOP, COEP and
document-isolation subtrees 1 of 1,177 combined.

**What exists.** `<iframe>` is a replaced box with width and height hints
in Livery. In the bootstrap, `contentDocument` lazily creates an empty
HTML document and `contentWindow` a bare object with `document`,
`getComputedStyle` and inner size read from the frame's computed style;
nothing loads `src`, nothing renders the child, and `window.frames` is the
window itself. `genet-documents` holds one retained session per document
with no parent link. Ortet hosts one top-level document in one window;
that does not exclude engine-owned nested browsing contexts.

**What the lane is.**

1. A browsing-context/navigation model integrated with `genet-documents`:
   parent, children, top-level context, document replacement and coordinated
   session-history traversal. Product tab/workspace history stays in Mere.
2. Loading: `src`, `srcdoc`, `about:blank`, `sandbox`, `allow`, and the
   specified load/failure observability, with the child's document fetched
   through the same resource route as the parent. Establish origin inheritance
   and sandbox policy before executing child script.
3. Rendering: the child session's scene composited into the parent's
   replaced box each frame, with clipping and scrolling, and hit testing
   descending into it.
4. Script: a stable `WindowProxy` for `contentWindow`, forwarding to the
   active document's global across navigation, with cross-origin access
   rules, `parent`, `top`, `frames`,
   `postMessage` with origin checks, and `frameElement`.
5. Security policy: same-origin checks, sandbox flags, COOP and COEP as
   they apply to a single process.

**Decisions.**

- Engine instance and thread placement, with configurable resource limits.
  A child's active document has an associated realm/global and environment
  settings, distinct from the parent's; document replacement follows HTML's
  reuse rules. The specification does not require an engine instance per iframe.
  Same-origin synchronous access, shared object identity and agent/event-loop
  rules constrain placement. Verify engine multi-realm support before choosing
  an arrangement. See the [HTML execution model](https://html.spec.whatwg.org/multipage/webappapis.html#realms-settings-objects-global-objects).
- How the child-session model is exercised through Ortet's single top-level
  session. The iframe lane owns nested contexts; Ortet remains the headed
  Genet proof host, with Pelt adding downstream integration evidence.
- Session history shape. This lane and the History API share it, and
  `history.pushState` already has a stub in the bootstrap.

**Done-conditions.** Select exact tests from
`html/semantics/embedded-content/the-iframe-element`, `html/browsers/windows`
and `html/browsers/the-window-object`. Prove src/srcdoc/about:blank loading,
same-origin object access, stable proxy identity across navigation,
cross-origin rejection and permitted proxy operations, sandbox restrictions,
and origin-checked messaging. A reftest plus input receipt proves child
clipping, scrolling, hit testing and focus. Child replacement/destruction
retires its runtime and scene resources. Name remaining history and isolation
policy families independently.

## Dedicated Worker

Census: `workers` 1 / 222 / 22 / 2 / 0, subtests 17 of 574; `webmessaging`
20 / 101 / 10 / 4 / 0, subtests 49 of 209.

**What exists.** `genet-scripted-worker` is the wasm-bindgen
entry that runs a whole `ScriptedDocument` inside a browser Web Worker for
the wasm target, with an engine chosen by feature. It says nothing about the
`Worker` global a page script constructs. What does exist is the engine
seam (`script-engine-api`), a runtime per engine instance, and
the [cheap-globals lane](2026-09-07_cheap_globals_plan.md)'s `structuredClone`,
`MessagePort`, `MessageChannel`, `BroadcastChannel` and window `postMessage`.
`structured_clone.rs` currently performs a fused in-agent clone walk, not a
transportable serialization record. Buffer detachment and port transfer have
observable shortcuts recorded in that plan; cross-agent use is unproved.

**What the lane is.**

1. A worker-owned runtime and event loop (a native thread is one placement)
   with `DedicatedWorkerGlobalScope`: `self`, timers, `fetch`, `importScripts`,
   `location`, `navigator`, no DOM.
2. `Worker` plus serialization/deserialization and transferable custody
   across the agent boundary. Extend the existing port APIs with real
   entanglement/transfer and buffer detachment; add `onmessage`, error and
   deserialization-failure delivery, and termination cleanup.
3. Classic worker script and `importScripts` fetching through the shared
   resource/policy route. Module workers need module loading and a separate
   declared slice.
4. `SharedWorker` and cross-agent `BroadcastChannel` delivery as follow-ons;
   the in-agent BroadcastChannel bootstrap already exists.

**Decisions.**

- Engine pairing and native/wasm host placement. Construct a thread-confined
  engine inside its owning thread; never transport VM handles. Prefer a
  versioned engine-neutral clone record at the existing pluggable-engine seam,
  but prove the selected representation preserves cycles, aliasing and transfers.
- Which production scheduler seam the worker uses. The harness drive loop
  can exercise it; a per-test harness process is not a page Worker runtime.

**Done-conditions.** Select exact constructor, worker messaging and
message-channel tests from the vendored manifest. Prove asynchronous ordering,
cycle/alias preservation, rejection of uncloneable values, actual sender
buffer detachment (including existing views), port transfer ownership,
script-load errors and termination with pending work. A scripted hosted page
exchanges messages with a live worker on each supported engine/target.
Record unsupported worker globals and module variants as named residuals;
retain the cheap-globals messaging regression manifest.

## WebSocket

Census: `websockets` 0 / 375 / 137 / 3 / 0, subtests 0 of 1,392; needs a
live server for anything past constructor and URL validation.

**What exists.** `components/netfetcher/src/websocket.rs` is a native
tokio-tungstenite wrapper with connect/send/receive and an echo test.
`connect` accepts only a URL and discards the handshake response; send and
receive collapse errors to bool/Option; `WsMessage::Close` drops code/reason.
It does not expose the handshake metadata, close details or buffered-byte
accounting needed by the browser API. The JS global is absent. The deferred
fetch seam supplies an asynchronous-completion pattern, not WebSocket policy.

**What the lane is.** A `WebSocket` host object with the four ready
states, `send` for strings, ArrayBuffer and Blob, `close` with code and
reason, `bufferedAmount`, `binaryType`, the `open`, `message`, `error`
and `close` events. Extend the transport contract with requested/selected
subprotocols, origin and credentials integration, typed internal errors,
close code/reason/cleanliness and queue accounting. Enforce browser policy
on this connection path, including mixed-content restrictions and redirect
rejection; ordinary fetch protections do not automatically wrap it. Preserve
the specification's intentionally limited script-visible failure details.
See the [WebSockets handshake and API](https://websockets.spec.whatwg.org/).

**Decisions.** Whether the wasm build binds the browser's WebSocket
(netfetcher's doc assumes so) or stays native-only for now.

**Done-conditions.** Freeze constructor/URL tests plus live-server cases for
text/binary messages, subprotocol selection/rejection, origin/credentials,
mixed-content/redirect rejection, bufferedAmount, close details and event
ordering. All selected assertions pass; connection teardown after document
destruction is measured. A wasm browser binding, if selected, has a separate
hosted receipt rather than inheriting the native transport result.

## IndexedDB and storage

Census: `IndexedDB` 1 / 201 / 29 / 0 / 0, subtests 5 of 880; `storage`
0 of 75; `FileAPI` 3 / 55 / 3 / 0 / 15, subtests 276 of 633.

**What exists.** `localStorage` works over a `StorageProvider` trait in
`script-runtime-api/platform.rs` (get, set, remove, clear, key, length),
tested on both engines. No host in this repository implements the trait
persistently; nothing implements it in Mere either. `document.cookie`
routes to a provider the same way. `Blob` and `File` exist in the fetch
bootstrap embedded in `script-runtime-api/fetch.rs`, which is where
`FileAPI`'s 276 passing subtests come from; `IDBKeyRange` and friends do
not exist.

**What the lane is.**

1. Host-provisioned storage with explicit storage keys, profile/private-mode
   separation, quota and deletion contracts for localStorage, IndexedDB and
   Cache API. Cookie persistence must preserve cookie domain/path, expiry and
   credentials semantics rather than treating cookies as an origin-keyed map.
   Sharing a physical store is optional; sharing clear identity and lifetime
   contracts is the decision with service workers. See the
   [Storage Standard](https://storage.spec.whatwg.org/#storage-keys).
2. IndexedDB itself: the object-store and index model, key comparison and
   key paths, transactions with the spec's scheduling, cursors, and the
   request and event plumbing that makes it asynchronous on the drive
   loop.
3. `navigator.storage` estimate and persist, and the storage-pressure
   eviction policy.

**Decisions.**

- Substrate: an embedded key-value store, SQLite through a Rust binding,
  or files. IndexedDB's transaction semantics fit a real transactional
  store; localStorage and cookies do not need one.
- Which backend the host provisions. Genet owns IndexedDB/Cache/localStorage
  observable semantics and provider contracts; Mere owns application profiles,
  storage provisioning and user policy. Keep transactions and request/event
  scheduling in the engine contract even when Mere supplies the database.
  Genet tests can supply an independent persistent provider.

**Done-conditions.** Select exact IndexedDB open/upgrade, key-path,
transaction and cursor tests. Require ordering, atomic commit/abort,
blocked upgrade/versionchange, isolation and key comparison for the declared
slice. A persistent scripted host reads committed data after restart and
rejects access from another storage key; abort leaves no partial data.
Quota/deletion and private-profile behavior have named provider tests.
Navigator storage estimates/persistence claims get separate assertions;
an Ortet restart requires the shared host prerequisites above.

## Web Animations

Census: `web-animations` 1 / 135 / 2 / 0 / 36, subtests 64 of 1,449;
`scroll-animations` 0 / 166 / 7 / 24 / 45, subtests 323 of 1,922;
`css/css-animations` testharness 8 / 109 / 1 / 0 / 113.

**What exists.** Livery parses `@keyframes`, `animation-*` and
`transition-*`. The [CSS animations plan](../docs/2026-07-09_css_animations_plan.md)
records A1/A2 and A3 lifecycle/event work on the historical `genet-layout`
route; the bootstrap has `AnimationEvent` and `TransitionEvent`. Current
Livery has a host-driven clock in `genet-livery/src/document/animation.rs`,
but historical event receipts do not establish that every event consumer is
wired on this route. Reconcile its schedule/sample/event path before adding
the JS object model. There is no `Animation` object, no `KeyframeEffect`,
no `document.timeline`, no `element.animate`, and no
`getAnimations`.

**What the lane is.** The Web Animations object model over the timeline
Livery already advances: `Animation` with play, pause, reverse, finish,
`currentTime` and `playbackRate`, `KeyframeEffect` with computed keyframes
and composite modes, `DocumentTimeline`, and CSS animations and
transitions exposed as `CSSAnimation` and `CSSTransition` objects so that
`getAnimations` returns them. `ScrollTimeline` and `ViewTimeline` after.

**Decisions.** Whether the animation engine stays in Livery with a JS
object model over it, or moves to a neutral crate the way `livery` and
`buckram` are split. The former is the smaller change.

**Done-conditions.** Select exact Animation/KeyframeEffect tests for play,
pause, seek, reverse, finish and cancellation, pending/ready/finished promise
ordering, and specified timing/composition behavior. CSS-generated animations
and transitions appear in `getAnimations` with stable object identity.
Re-run named CSS animation/transition lifecycle and event tests through
current Livery on both supported engines, with a host-clock receipt matching
sampled values to rendered output. Keep scroll/view timelines as a separate
slice rather than counting their interface exposure as completion.

## Editing and contenteditable

Census: `editing` 1 / 242 / 414 / 40 / 146, subtests 1 of 97,687;
`html/editing` 26 / 161 / 8 / 8 / 221, subtests 83 of 754; `selection`
0 of 280.

**What exists.** The [text editing primitive plan](../docs/2026-07-25_text_editing_primitive_plan.md)
landed T0 through T4: a shared caret and selection primitive, platform
IME composition, and the visual caret and selection in `genet-render`.
Those are primitive/first-consumer receipts; that plan still lists T5 forms
and T6 contenteditable separately, and its Cambium ownership predates the
move to Mere. Genet web editing must consume engine-side geometry/input
contracts without importing Cambium. `contenteditable`, `designMode` and
`execCommand` behavior remain open. The [Selection/Range lane](2026-09-07_selection_range_plan.md)
landed at `ddee26ef559`; its DOM boundary points are authoritative and visual
byte offsets are a projection. Its node-model, fragment, geometry and
selectionchange-ordering residuals must be reconciled for the editing slice.

**What the lane is.** An editing host over the existing primitive:
`contenteditable` and `designMode` attributes, caret movement and
selection inside arbitrary DOM, the insert, delete, and line-break
commands with the spec's DOM mutations, `beforeinput` and `input` events
with `inputType`, undo and redo, and clipboard integration through
`genet-clipboard`. `execCommand` is a compatibility surface over the same
commands and is where `editing`'s 97,687 subtests live.

**Decisions.** How much of `execCommand` to implement. Browsers disagree
with each other and with the spec, and the corpus rewards the legacy
behavior. A `beforeinput`-first implementation with a minimal
`execCommand` subset is the modern choice.

**Done-conditions.** Hold the Selection/Range landing manifest and close the
specific mutation, fragment and event-ordering prerequisites the slice uses;
whole-directory green is not a prerequisite for starting bounded editing.
Freeze editing tests for insertion, deletion, line breaks, selection
replacement, cancelled beforeinput, inputType and undo/redo. Prove DOM and
visual selection agree after mutation, including non-ASCII and bidi text.
A scripted host types and composes into a contenteditable region, reads the
resulting DOM and selection, and verifies undo and clipboard behavior for
the supported commands. Name deferred execCommand operations individually.

## Ordering across these

The dependency contracts are:

- Worker and service workers share execution, scheduling and clone/transfer
  machinery. Dedicated Worker API completion is not a mandatory first step
  for service workers. Worker-side OffscreenCanvas requires execution and
  transfer support; Window-side OffscreenCanvas can land independently.
- IndexedDB, Cache API, registrations and persistent localStorage agree storage
  identity and host lifetime contracts before a backend is selected. Their
  implementation order must not accidentally define the shared contract.
- Editing consumes the landed Selection/Range model and resolves the
  residuals its selected commands exercise. Shadow DOM extends that model
  with shadow-aware boundaries and composed ranges.
- Shadow DOM and iframes share style/input consumers but introduce different
  relationships: shadow scopes within documents versus nested documents with
  realms, origins and navigation. Coordinate changes at those consumers;
  neither feature is a prerequisite for the other.
- WebSocket can start from its transport/API contract; Web Animations can
  start from Livery's timeline/event audit. Both still require production
  scheduling and a named scripted host for their hosted proofs.

## Review findings and related ownership (2026-09-07)

- Reconciled the historical inventory with committed `ddee26ef559` and the
  linked cheap-globals, observer and selection plans. Concurrent node-model
  WIP is not treated as a landed fix or included in a new test receipt.
- Verified shadow selector stubs in `genet-livery/src/dom.rs`, the split
  resource routes in `genet-scripted/{document,livery}.rs`, and the limited
  WebSocket wrapper in `netfetcher/src/websocket.rs`.
- Verified Ortet's native and wasm paths select `LiverySessionEngine`
  (`ports/ortet/src/{shell,web}.rs` and its manifest). Scripted and persistent
  host gates above are new prerequisites, not a reinterpretation of O0-O4.
- Reviewed the older animation and editing plans as historical receipts;
  their original route/consumer scope is retained. This document owns the
  deferred web-platform scoping; promoted plans own exact manifests, current
  code reconciliation and implementation receipts.
