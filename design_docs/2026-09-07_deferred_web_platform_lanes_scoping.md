# Deferred web platform lanes: scoping

**Date:** 2026-09-07

**Status:** research. Nothing here is scheduled. Each section ends with the
decisions that are Mark's and the done-conditions a plan would carry, so a
section can be promoted to a dated plan without re-deriving its ground.

**Parent:** [Web platform WPT census](2026-09-06_web_platform_wpt_census.md).
The eight lanes that census made assignable (harness repair, IDL-generated
interfaces, cheap globals, MutationObserver, XMLHttpRequest, Selection and
Range, dedicated Worker, the reftest lane) are being executed under their
own plans. This document scopes what was held back because each needs a
design decision before code.

Counts below are the 2026-09-06 disk-mode census: files as pass / fail /
error / no-results / skip, then subtests passed of total.

## How to read a section

Each section has the same shape:

- **What exists.** Verified against the tree at `5af76a0cb8c`, with the
  crate or file named.
- **What the lane is.** The engine work, in the order it has to land.
- **Decisions.** Choices with more than one defensible answer. These are not
  made here.
- **Done-conditions.** What a plan would gate on, against the census
  baseline.

## Shadow DOM

Census: `shadow-dom` 6 / 215 / 42 / 2 / 49, subtests 18 of 8,654.
`custom-elements` 3 / 144 / 29 / 1 / 10, subtests 2,041 of 3,674, is the
sibling that already works and the reason this lane matters: real
component libraries pair the two.

**What exists.** `layout-dom-api` already has the seam: `flat_children`
on `LayoutDom` is documented as "slot-assigned for shadow hosts, otherwise
DOM order" and defaults to `dom_children`, so Livery's tree walk can be
switched to the flat tree without changing the trait. Livery's selector
element implementation answers `is_html_slot_element`. The scripted DOM
arena (`genet-scripted-dom`) has no shadow root node kind, no host
pointer, and no slot assignment. The event path in the bootstrap is built
as target to root plus window, with a comment that there is no retargeting
or composed boundary yet. `<template>` is display:none in the UA sheet and
parses as an ordinary element; there is no template contents fragment.

**What the lane is.**

1. A `ShadowRoot` node kind in the arena: host link, mode, delegatesFocus,
   and a per-host slot assignment table maintained on mutation (named
   slots, the default slot, `slotchange`).
2. `flat_children` implemented on the arena from that table, and Livery's
   style walk, box construction and hit testing moved from `dom_children`
   to `flat_children`. The static DOM keeps the default.
3. Style scoping: a shadow tree's stylesheets apply inside the root, with
   `:host`, `:host()`, `:host-context()`, `::slotted()` and `::part()`; the
   document's author sheets do not cross in, inheritance does. Livery's
   cascade takes an origin and scope today; the lane adds the scope
   boundary and the four pseudo-classes to `livery/src/selector.rs`.
4. Event retargeting and `composedPath()` in the bootstrap's dispatch,
   plus `composed` on Event init.
5. `attachShadow`, `shadowRoot`, `assignedSlot`, `assignedNodes`,
   `getRootNode` with `composed`, and `<template>.content` as an inert
   DocumentFragment owned by a separate inert document.

**Decisions.**

- Whether `<template>` contents get their own inert document (spec) or a
  flag on the fragment. The spec route costs a second document per
  template; the flag route breaks `ownerDocument` observability.
- Whether the static DOM (Fleece, the reader lanes) should also learn the
  flat tree. Declarative shadow DOM (`<template shadowrootmode>`) is
  parser-level and would otherwise render slots wrong in script-free pages.
- Declarative shadow DOM parsing in html5ever versus a post-parse pass.

**Done-conditions.** `shadow-dom` subtests move from 18; `custom-elements`
holds; the reftest slice under `shadow-dom` renders slotted content; Livery
and Buckram unit suites hold; a Fleece extraction receipt shows whether
slotted text is or is not extracted, whichever the decision above chooses.

## Canvas 2D

Census: `html/canvas` 33 / 2,160 / 6 / 2 / 472, subtests 33 of 4,142. Of
the 2,673 files, the `element` and `offscreen` families dominate; the
`offscreen` half also needs Worker and OffscreenCanvas.

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
2. A rasterizer behind it. Two candidates already in the graph: netrender's
   Vello path (GPU, same device as the page, external-texture composite for
   free) and `vello_cpu` (deterministic, no device, good for tests and
   wasm without WebGPU).
3. The DOM side: `getContext('2d')` with the context-mode rules (a canvas
   is either 2d or WebGL, never both), `width`/`height` reset semantics,
   `ImageBitmap`, and `OffscreenCanvas` once Worker exists.

**Decisions.**

- Backend: Vello on the page device, `vello_cpu`, or both behind a pref
  the way the WPT subsuite config assumed. Pixel-exact WPT expectations
  favor a CPU path for conformance and a GPU path for pages; two backends
  is the configurable design but doubles the raster contract to test.
- Whether text on canvas reuses Parley through Livery's text system or
  Vello's own glyph path. Reuse keeps one shaping engine in the stack.
- Where the raster lives across frames: retained in the context (spec) and
  composited by key each frame, which is how WebGL already works.

**Done-conditions.** `html/canvas/element` moves from 33 subtests, with the
`2d.fillStyle`, `2d.path`, `2d.transformation` and `2d.drawImage` families
called out; a reftest receipt of a canvas composited into a page in Ortet;
a wasm build proof for whichever backend the wasm lane picks.

## Service workers

Census: `service-workers` 0 / 266 / 17 / 0 / 9, subtests 0 of 1,526. Every
file here also needs a live server, so the true floor is measured only in
server mode.

**What exists.** netfetcher is the network stack: cache, cookies, CORS,
HSTS, redirects, referrer policy, HTTP/3 and WebSocket are real. The
`FetchHandler` seam in `script-runtime-api/fetch.rs` is where a page's
`fetch()` leaves the runtime; it already runs asynchronously with abort and
streaming. There is no second event loop, no registration store, no Cache
API, and no interception point between a document's fetch and the network.

**What the lane is.**

1. An interception seam in front of `FetchHandler`: a document's requests
   pass through a matcher that can hand them to a controlling worker and
   accept a synthesized `Response`.
2. A worker execution context with its own event loop, global
   (`ServiceWorkerGlobalScope`), lifecycle (install, activate, waiting,
   redundant), and `clients`.
3. A registration store keyed by origin and scope, persisted, with update
   checks and byte-for-byte script comparison.
4. The Cache API and `caches`, which is a storage engine of its own.
5. `navigator.serviceWorker`, `postMessage` both ways, and the `fetch`,
   `install`, `activate`, `message` events.

**Decisions.**

- This lane depends on dedicated Worker (below) for the execution context
  and on a persistent storage substrate shared with IndexedDB and the
  Cache API. Which of the three lands first sets the substrate's shape.
- Whether navigation requests are intercepted at all in the first cut.
  Subresource-only interception is much smaller and covers most offline
  use.
- Process or thread placement. A worker on a thread in the same process is
  the natural fit for the current single-process host; the isolation Mere
  wants for untrusted content is a separate question.

**Done-conditions.** A server-mode run of `service-workers` moves from
zero, with the `registration`, `fetch-event` and `postmessage` families
named; the registration survives a session restart in Ortet; a receipt
that an intercepted subresource never hits netfetcher.

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
with no parent link. Ortet is one window, one document by design.

**What the lane is.**

1. A browsing-context tree in `genet-documents`: parent, children, the
   top-level context, and a session history per context.
2. Loading: `src`, `srcdoc`, `about:blank`, `sandbox`, `allow`, and the
   load and error events on the element, with the child's document fetched
   through the same resource route as the parent.
3. Rendering: the child session's scene composited into the parent's
   replaced box each frame, with clipping and scrolling, and hit testing
   descending into it.
4. Script: `contentWindow` as the child's real global with the
   cross-origin `WindowProxy` rules, `parent`, `top`, `frames`,
   `postMessage` with origin checks, and `frameElement`.
5. Security policy: same-origin checks, sandbox flags, COOP and COEP as
   they apply to a single process.

**Decisions.**

- Nesting depth and whether every child gets its own script runtime. One
  runtime per context is the spec's model and the simplest to isolate;
  sharing an engine across same-origin contexts is cheaper.
- Whether Ortet's raw target stays one document, with iframes proved only
  in genet-wpt and Mere's hosts.
- Session history shape. This lane and the History API share it, and
  `history.pushState` already has a stub in the bootstrap.

**Done-conditions.** `html/semantics/embedded-content/the-iframe-element`
moves from its current count; `html/browsers/windows` and
`html/browsers/the-window-object` gain; a reftest shows a child document
rendered inside a parent in the WPT lane; a cross-origin `contentWindow`
access throws as the spec says.

## Dedicated Worker

Census: `workers` 1 / 222 / 22 / 2 / 0, subtests 17 of 574; `webmessaging`
20 / 101 / 10 / 4 / 0, subtests 49 of 209.

**What exists, corrected.** The census summary called `genet-scripted-worker`
a 158-line start on this lane. It is not: that crate is the wasm-bindgen
entry that runs a whole `ScriptedDocument` inside a browser Web Worker for
the wasm target, with an engine chosen by feature. It says nothing about the
`Worker` global a page script constructs. What does exist is the engine
seam (`script-engine-api`), a runtime per engine instance, and
`postMessage` on the window. `structuredClone` is absent; it is in the
cheap-globals lane and this lane's transfer path builds on it.

**What the lane is.**

1. A second runtime on a thread with its own event loop and
   `DedicatedWorkerGlobalScope`: `self`, timers, `fetch`, `importScripts`,
   `location`, `navigator`, no DOM.
2. `Worker`, `MessagePort` and `MessageChannel` with structured clone and
   transfer across the thread boundary, `onmessage` and `onerror` on both
   sides, `terminate`.
3. Script fetching for the worker script and `importScripts` through the
   same resource route as the page.
4. `SharedWorker` and `BroadcastChannel` as follow-ons once the port
   machinery exists.

**Decisions.**

- Engine pairing: a worker gets the same engine as its page. Boa and Nova
  are both `!Send` runtimes today, so the thread owns its engine and only
  cloned data crosses; the question is whether the structured-clone
  format is engine-neutral bytes or engine-specific.
- Whether the worker's event loop is the harness drive loop generalized
  or a new one.

**Done-conditions.** `workers/constructors`, `workers/semantics/messaging`
and `webmessaging/message-channels` gain; a page in Ortet spawns a worker
and receives a message; the harness records worker-only WPT variants it
still cannot host as a named skip.

## WebSocket

Census: `websockets` 0 / 375 / 137 / 3 / 0, subtests 0 of 1,392; needs a
live server for anything past constructor and URL validation.

**What exists.** `netfetcher::websocket` is a complete RFC 6455 client
over tokio-tungstenite with connect, send and receive, native only, and
its own doc names "the eventual JS WebSocket binding" as its consumer. The
JS global is absent. The deferred fetch seam shows the pattern for an
asynchronous host object whose completions arrive through the drive loop.

**What the lane is.** A `WebSocket` host object with the four ready
states, `send` for strings, ArrayBuffer and Blob, `close` with code and
reason, `bufferedAmount`, `binaryType`, the `open`, `message`, `error`
and `close` events, subprotocol negotiation, and the mixed-content and
origin rules netfetcher already enforces for fetch. Small: one host object
over an existing transport.

**Decisions.** Whether the wasm build binds the browser's WebSocket
(netfetcher's doc assumes so) or stays native-only for now.

**Done-conditions.** The constructor and URL families in `websockets` pass
in disk mode; a server-mode run of `websockets/basic` and
`websockets/interfaces` moves from zero.

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

1. A persistent origin-keyed storage substrate under the host, used by
   `localStorage`, cookies, IndexedDB and later the Cache API. This is
   the shared decision with service workers.
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
- Whether Mere owns the substrate as a platform service and genet only
  the provider traits, which is the current boundary's direction.

**Done-conditions.** `IndexedDB` `idbfactory_open`, `keypath`,
`transaction-*` and `idbcursor_*` families gain; `storage` moves from
zero; a value written in one Ortet run is read in the next.

## Web Animations

Census: `web-animations` 1 / 135 / 2 / 0 / 36, subtests 64 of 1,449;
`scroll-animations` 0 / 166 / 7 / 24 / 45, subtests 323 of 1,922;
`css/css-animations` testharness 8 / 109 / 1 / 0 / 113.

**What exists.** Livery parses `@keyframes`, `animation-*` and
`transition-*`; the CSS animations plan in `docs/2026-07-09` landed A1 and
A2 with events, and the bootstrap has `AnimationEvent` and
`TransitionEvent`. The animation clock is driven by the harness and by
Ortet's frame loop. There is no `Animation` object, no `KeyframeEffect`,
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

**Done-conditions.** `web-animations/interfaces/Animation` and
`KeyframeEffect` gain; `css/css-animations` testharness moves from 8; the
existing A2 event receipts hold.

## Editing and contenteditable

Census: `editing` 1 / 242 / 414 / 40 / 146, subtests 1 of 97,687;
`html/editing` 26 / 161 / 8 / 8 / 221, subtests 83 of 754; `selection`
0 of 280.

**What exists.** The text editing primitive plan in `docs/2026-07-25`
landed T0 through T4: a shared caret and selection primitive, platform
IME composition, and the visual caret and selection in `genet-render`.
That is the editing substrate for form controls. `contenteditable` and
`designMode` are not implemented; there is no editing host, no
`execCommand`, and the Selection API lane (already assigned) is the
prerequisite for anything here.

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

**Done-conditions.** `selection` and `dom/ranges` are green first, from
the assigned lane; `html/editing/editing-0` and `editing/event` gain; an
Ortet receipt types into a contenteditable region and reads the DOM back.

## Ordering across these

Dependencies, not preferences:

- Worker is a prerequisite for service workers and for OffscreenCanvas.
- The storage substrate is shared by IndexedDB, service workers' Cache API
  and persistent localStorage; whichever lands first defines it.
- Selection and Range precede editing.
- Shadow DOM and iframes both touch Livery's tree walk and event dispatch;
  doing them back to back keeps that surgery in one head.
- WebSocket and Web Animations depend on nothing here and can fill gaps.
