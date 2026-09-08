# genet Documentation Index

The canonical index for `genet/design_docs/`, per [`DOC_POLICY.md`](DOC_POLICY.md)
§6. If any other index disagrees with this file, this file wins.

Founded 2026-08-24, when the canonical policy core was distributed across the
workspace and the component documents for inker, nematic and verso-tile were
repatriated here from mere.

> **Boundary correction landed, 2026-09-03:** Cambium and Genet's upper
> application components moved to Mere under
> `mere/design_docs/mere_docs/implementation_strategy/2026-09-02_platform_boundary_and_repository_topology_plan.md`.
> Their documents moved with the code and are indexed in Mere. Genet retains
> web-platform implementation, observable behavior, raw host contracts, WPT,
> and a minimal engine host.

> **Read the policy's "Two doc homes" section first.** This repository also has
> a flat `docs/` directory of 166 older engine documents. The current engine
> work section below gives entry points into that corpus; it is not a complete
> inventory or a migration. The split is deliberate, and migration remains
> open, unscheduled work. New documents go here.

## Required reading order

1. The root [`README.md`](../README.md) — what genet is.
2. [`DOC_POLICY.md`](DOC_POLICY.md) — the shared core plus this repo's addendum,
   including the `docs/` boundary and the smolweb split.
3. The section you are working in, below. genet has no topic area root left:
   the last three moved to mere with their code on 2026-09-03, and active
   plans sit flat in `design_docs/`.

## Current engine work (2026-09-07)

Use the linked plan's current gate and dated receipts before implementing a
lane. Historical corpus totals are measurements of their named source, not a
fresh baseline for this checkout. This map also covers selected plans in the
older `docs/` corpus without changing their location or governance.

| Work | Current boundary and next proof |
|---|---|
| [Arena semantic contract](../docs/2026-06-11_gc_arena_dom_plan.md#g5-arena-semantic-contract-agreed-planned-2026-09-07) | G5 agreed and planned 2026-09-07: distinct arena/node/document/tree-scope identity, coordinated mutation and reference-driven lifetime. Runtime regressions plus a retain/detach/collect/adopt/mutate/render/release receipt through scripted Ortet. Implementation and acceptance open. |
| [Font fallback](2026-09-04_common_script_font_fallback_plan.md) | Windows T1 is accepted with focused regressions and Ortet readback. Consumer revision adoption, upstream disposition, and other-platform measurements remain open. |
| [Ortet](2026-09-03_ortet_founding_plan.md) | O0-O4 landed. O5a's structural engine selector now drives feature-gated Livery, Boa and Nova through the real headed session contract while default Ortet stays script-free. Per-engine headed receipts, deadline/idle wake, asynchronous resource and Worker integration, stable accessibility IDs, and publication remain open. |
| [K6 fragmentation](../docs/2026-08-15_buckram_k6_fragmentation_execution_plan.md) | K6a typed inputs, K6b's retained context/token model, and a dormant pre-K6c style/input dispatch seam landed. All 6,077 named records match the frozen pre-K6 baseline. Live formatter continuation and multicol geometry remain K6c work. |
| [Flex and grid, Row 18](2026-08-25_livery_flex_shorthand_plan.md#row-18-closure-and-remaining-work) | Bounded vertical-flex slices landed. Remaining work includes mixed-writing-mode baselines, generated/pseudo self edges, shared font metrics and percentage provenance, then a measured grid inventory. The dated guard review records 21 pre-K6 flex/grid pass-to-fail cases. |
| [Floats and shapes, Row 12](../docs/2026-08-25_buckram_horizontal_float_direction_reconciliation.md) | Horizontal box-shape and direction slices landed. Vertical/orthogonal transforms and remaining shape families retain their own unproved boundaries. |
| [Counters, lists and generated content, Row 17](../docs/2026-08-21_buckram_livery_lane_program_plan.md#wave-2-now-unblocked) | Open inventory. Name the computed-content, counter-scope and marker-box consumers and a bounded execution gate before implementation. |
| [K7 foundational sizing and dispatch](../docs/2026-07-26_buckram_css_layout_engine_plan.md#k7-foundational-sizing-and-dispatch-closure) | Reconcile landed sizing fixes with remaining deferrals before choosing a slice. Fragmentainer-dependent work consumes K6; final closure includes deleting CSS-facing Taffy block dispatch. |
| [WPT harness and ledger](../docs/2026-08-24_wpt_harness_ledger_execution_plan.md) | Exact scorer and reference-verification gates landed. Freeze a fresh candidate runner for new work; unsupported test/reference agreement earns no conformance credit. |
| [Servo cone retirement](2026-09-07_servo_cone_retirement_plan.md) | servo-paint's compositor half carved out as genet-compositor; the reftest lane renders through genet-render-host; the constellation trait cone left the graph 2026-09-07. All gates green; next proof is Mere's dependency rename at its next bump. |
| [Web platform WPT census](2026-09-06_web_platform_wpt_census.md) | Baseline exact maps for 41 non-CSS WPT directories (21,672 files, disk mode, Boa/Livery) landed 2026-09-06. Three of its four harness caveats are closed by the harness-repair plan; the reftest caveat and the per-directory lanes remain open. |
| [WPT harness repair](2026-09-07_wpt_harness_repair_plan.md) | Per-test worker isolation, the disk-mode include and `.py` fixes, and a configurable, quiescing server-mode deadline landed 2026-09-07, with a re-run census whose 204 movements are all attributed. Next proof: a server-mode measurement of the network-dependent families on a live `wpt serve`. |
| [XMLHttpRequest](2026-09-07_xhr_plan.md) | XHR as a state machine over the fetch seam, landed 2026-09-07: xhr 53 to 281 subtests in disk mode, 831 of 1,336 in server mode, fetch holds. Residuals: responseXML needs DOMParser; 28 errors are Worker and document.domain demand. |
| [Cheap globals](2026-09-07_cheap_globals_plan.md) | `performance` (+ `PerformanceObserver`), `queueMicrotask`, `structuredClone`, `MessageChannel` / `MessagePort` / `BroadcastChannel` and `crypto` landed 2026-09-07: 79 forward file movements, zero pass-to-fail, +462 subtest passes over ten directories. Next proof is `crypto.subtle`, real `ArrayBuffer` detachment, and the cross-agent reuse of the clone walker by the Worker lane. |
| [IDL interface table](2026-09-07_idl_interface_table_plan.md) | The scripted tier's HTML interface table is generated from WPT's vendored WebIDL plus its tag map, with 41 reasoned overrides and a drift test. 72 interfaces / 338 reflected attributes / 41 shape-only DOM-CSSOM interfaces. Next proof is extending the shape pass past `html`, `dom` and `cssom`, and the reflection-algorithm gaps (`ReflectRange` clamping, invalid-value defaults). |
| [MutationObserver](2026-09-07_mutation_observer_plan.md) | The arena's mutation point now has two consumers: Livery's `DomMutation` stream and a spec-shaped observer record fanned out at the same mutators, off until something observes. Landed 2026-09-07 with four `dom/nodes/MutationObserver-*` files all-pass, +495 subtest passes and zero pass-to-fail. `Range` landed 2026-09-07 and closed that residual (`childList` 18/38 to 32/38, `characterData` 13/23 to 21/23). The DOM node model lane then closed the rest on 2026-09-07 — fragment insertion, `normalize`, `outerHTML`, attribute namespaces and the static-to-scripted clone of comments and PIs — and the four `MutationObserver-*` files now pass. |
| [Selection and Range](2026-09-07_selection_range_plan.md) | `Range` / `StaticRange` / `Selection` over the scripted arena, with the live-range steps at the bootstrap's mutation funnel and a boundary index keyed by node. Landed 2026-09-07: `selection` 0/280 to 28,582/33,621 subtests, `dom/ranges` 0 to 10 all-pass, +29,126 subtest passes and zero pass-to-fail. The DOM node model lane supplied all three on 2026-09-07 (plus a constructible `Document`, the second floor in the same file) and `dom/ranges` unfloored to 15 all-pass / 35,466 subtests. |
| [DOM node model](2026-09-07_dom_node_model_plan.md) | `DocumentFragment` insertion, `CDATASection` and real `DocumentType` nodes, the `ParentNode` / `ChildNode` mixins, `normalize`, `outerHTML`, namespaced attributes as live `Attr` nodes, `DOMParser` / `XMLSerializer` and `Node.baseURI`. Landed 2026-09-07: `dom/ranges` unfloored (15 errored to 1, 24 to 35,466 subtests), `html/semantics/interfaces.html` 298 to 435 of 438, +82,944 subtest passes over nine directories with zero pass-to-fail. Next proof is a server-mode receipt for `responseXML`, and static `NodeList` indexed access, which is a `Proxy` trap per read. |
| [Tag-name casing and the window globals](2026-09-07_tagname_window_globals_plan.md) | `tagName` / `nodeName` fold only for an HTML-namespaced element whose current node document is an HTML document, element interfaces and custom element names match case-sensitively, `document.importNode` exists, and `window` / `document` / `self` have their `[LegacyUnforgeable]` and `[Replaceable]` shapes on both the window and the worker global. Landed 2026-09-07: `Element-tagName.html` 3/6 to 6/6, `html/semantics/interfaces.html` to all-pass, +31 subtest passes with zero pass-to-fail. Residuals: `createElement`'s namespace on an XML document, and attribute-name folding, which still turns on the namespace alone. |
| [Dedicated Worker](2026-09-07_worker_plan.md) | A second `Runtime` of the same engine on its own thread with `DedicatedWorkerGlobalScope`, `Worker` on the page, `MessagePort` across the boundary, and a JSON encoding of the clone record as the cross-agent wire. Landed 2026-09-07: `workers` 5 to 74 all-pass and 21 to 321 subtests, genet-wpt hosts `.worker.js` and `.any.worker` variants, +972 subtest passes with zero pass-to-fail. Residuals: `SharedWorker`, cross-agent `BroadcastChannel`, module workers, nested-worker relay ordering, real `ArrayBuffer` detachment, and a hosted headed receipt. |
| [WebSocket](2026-09-07_websocket_plan.md) | The `WebSocket` host object over netfetcher's transport, with the browser policy the Fetch algorithm does not wrap it in enforced on the connection path. Landed 2026-09-07: `websockets` 0 to 1,090 of 1,877 subtests in disk mode, 254 all-pass and 1,144/1,586 in the first server-mode run; `fetch` and `xhr` byte-identical. Residuals: worker-hosted sockets (214 files), `WebSocketStream`, real backpressure. |
| [Host contract ownership](../docs/2026-08-14_web_platform_host_contract_plan.md) | Genet owns retained session contracts; Mere owns surface orchestration and product adapters. The older S0-S5 receipts need a consumer-side status refresh before resuming those lanes. |

The [Buckram master](../docs/2026-07-26_buckram_css_layout_engine_plan.md)
defines ownership and the [lane program](../docs/2026-08-21_buckram_livery_lane_program_plan.md)
assigns residuals. The linked execution plans carry their current gate; a
completed corpus census or bounded slice does not close its enclosing feature.

## Servo cone retirement

- [servo_cone_retirement_plan](2026-09-07_servo_cone_retirement_plan.md)
  (**landed 2026-09-07**: retired servo-paint's message painter,
  paint-api and the embedder/constellation trait cone; keeps the platform
  compositor as `genet-compositor` and absorbs the testdriver input path into
  genet-wpt. Media, WebGL and Piccolo stay by consumer.)

## Deferred web platform lanes

- [deferred_web_platform_lanes_scoping](2026-09-07_deferred_web_platform_lanes_scoping.md)
  (**research 2026-09-07**: Shadow DOM, Canvas 2D, service workers, iframes
  and nested browsing contexts, dedicated Worker, WebSocket, IndexedDB and
  storage, Web Animations, and editing. Reviewed against the landed globals,
  observer and selection lanes: required semantics, remaining design choices,
  shared scripted/persistent host prerequisites, and exact-slice acceptance
  requirements. Census counts stay historical. API lanes require dated plans;
  the shared scripted-host direction is planned in Ortet O5.)

## WPT census — the web platform beyond CSS

- [selection_range_plan](2026-09-07_selection_range_plan.md)
- [tagname_window_globals_plan](2026-09-07_tagname_window_globals_plan.md)
  (**landed 2026-09-07**: the two shape residuals the interface-table and Worker
  lanes left named. The HTML uppercasing moved out of the arena's `__tagName` /
  `__nodeName` and into the bootstrap, because the rule depends on the node's
  *current* node document being an HTML document — which only the JS tier tracks,
  and which `importNode` and `adoptNode` change — so the natives now report a
  case-preserved qualified name and one `elementQualifiedName` folds per read.
  With case preserved, element-interface selection reads the **local name**
  case-sensitively (`createElementNS(html, 'DIV')` is an `HTMLUnknownElement`),
  custom element definitions are keyed by local name so `foo-bar` cannot claim
  `foo-BAR`, and `isValidCustomElementName` gained the first-code-point and
  no-ASCII-uppercase rules it was missing. `document.importNode` was added on a
  `cloneNode` refactored to take its destination document. `window` and
  `document` became `[LegacyUnforgeable]` accessors and `self` a `[Replaceable]`
  one, with a `Runtime::new_worker` so the worker global never defines what it
  must not have rather than deleting it afterwards — both backends, Nova
  included, accept a non-configurable accessor on the global.
  `dom/nodes/Element-tagName.html` 3/6 -> 6/6,
  `html/semantics/interfaces.html` 435/438 -> **438/438** (all-pass),
  `Document-importNode` 0/5 -> 4/5, `custom-elements/Document-createElementNS`
  to all-pass, `unexpected-self-properties.worker` holds at 57/57; +31 subtest
  passes over six directories, five files `fail -> pass`, zero pass-to-fail.
  Two baselines repinned forward. Raw maps under
  `Code/testing/genet/wpt-ledger/2026-09-07_tagname_window_globals/`.)
- [worker_plan](2026-09-07_worker_plan.md)
  (**landed 2026-09-07**: the dedicated Worker. Both engines are `!Send`, so the
  worker thread constructs its own `Runtime` and owns it; the spawn needs no
  engine bound because `Runtime::new` records `worker_main::<E>` as a plain
  `fn` pointer. The cross-agent wire is a **JSON encoding** of the clone record,
  which is forced rather than chosen: `CallCx` marshals only strings, so the
  cheap-globals fused walk was split into `__scSerialize` / `__scDeserialize`
  over a heap of tagged nodes that preserves cycles, aliasing, holes and
  transfers. Resource loads — the classic script, `importScripts`, `fetch` — are
  synchronous requests back to the page, answered from a new
  `ScriptResourceLoader` route or the page's `FetchHandler`, so no network stack
  enters the worker thread. A transferred `MessagePort` leaves a stub behind and
  the host routes by port id, in both directions. Two scheduling facts fell out:
  a worker's idle report must carry the count of link messages it has consumed
  (a bare flag quiesced the page over live work, one Boa run in three), and the
  disk drive loop must run on wall time while a worker is live, or its first
  virtual jump fires testharness.js's own timeout before the worker has started.
  genet-wpt now hosts `.worker.js` and `.any.worker` variants — synthesizing the
  `.any.worker.js` file `wpt serve` would have generated — and keeps the skip
  reasons for shared and service workers. `workers` 5 → 74 all-pass and
  21/574 → 321/967 subtests, `workers/constructors` 0 → 8 all-pass,
  `workers/interfaces` 0 → 30, `xhr` 64 → 95 with errors 28 → 5,
  `html/webappapis` 31 → 45; +972 subtest passes over five directories, 207
  previously unenumerated variants now reporting, and zero pass-to-fail. Raw
  maps under `Code/testing/genet/wpt-ledger/2026-09-07_worker/`.)
- [websocket_plan](2026-09-07_websocket_plan.md)
  (**landed 2026-09-07**: `WebSocket` and `CloseEvent` as a script-visible state
  machine over an extended netfetcher transport. Two halves, because WebSocket is
  not a shape of `fetch()`: the Fetch algorithm does not wrap the connection
  path, so scheme rules, Fetch's bad-port list, HSTS, mixed-content blocking, the
  CSP `connect-src` hook and redirect *refusal* are enforced in
  `netfetcher::websocket::connect` against the same caller-owned `FetchContext`
  and the same cookie jar the fetch path uses — one test per rule — and every one
  of them reaches script only as the specification's single `error` event. The
  transport gained requested/selected subprotocols, negotiated extensions, the
  `Origin` header, typed `WsError`s in place of `bool`/`Option`, close
  code/reason/cleanliness and buffered-byte accounting, with no tungstenite type
  in its public API. Above it, a `WebSocketHandler` seam beside `FetchHandler`
  (one new `HostState` field, six completion entry points) and an implementation
  on genet-wpt's existing tokio worker: one task per socket, `select!`ing between
  its command channel and its frames, delivered through the drive loop exactly as
  a deferred fetch settles. The bootstrap does no URL parsing of its own — it
  reuses the fetch surface's `__resolve_url` / `__url_parse` sinks, which is why
  the constructor and URL families pass with no network at all. Three findings
  worth carrying: `__ws` was already the *worker* scope's prefix;
  `MessageEvent.origin` on a socket message is the **socket URL's** origin, so it
  carries the `ws` scheme, not the page's; and a 512-**byte** probe cut inside a
  U+FFFD had been panicking three `constructor/016.html` variants in the runner
  itself. `websockets` 0 -> 76 all-pass and 0/1,874 -> 1,090/1,877 subtests in
  disk mode; the first server-mode run is 254 all-pass, **zero errors**, and
  1,144/1,586 subtests, with all 214 `no-results` files being
  `.any.worker.html` — a dedicated Worker has no socket relay yet. `fetch` and
  `xhr` maps are byte-identical, and there is no pass-to-fail movement anywhere.
  Raw maps under `Code/testing/genet/wpt-ledger/2026-09-07_websocket/`.)
- [reflector_identity_scoping](2026-09-07_reflector_identity_scoping.md)
  (**research 2026-09-07**: why a node's JS wrapper, and every expando it
  carries (listeners, handlers, custom-element state, iframe documents, WebGL
  contexts), can vanish while the node lives: the wrapper is rooted only by
  script, so a collection re-mints a blank one. Reproduced on both engines;
  the headed host collects every frame. Fix options, blast radius and the
  decision on pinning against the gc-arena soak target are Mark's.)

- [dom_node_model_plan](2026-09-07_dom_node_model_plan.md)
  (**landed 2026-09-07**: the core DOM residuals the MutationObserver and
  Selection/Range plans left to Mark. Inserting a `DocumentFragment` moves its
  children, as one coalescing group, so the spec's two `childList` records fall
  out of the machinery the observer lane already built. `CDATASection` joins
  `NodeKind` and `DocumentType` becomes a real arena node — name in `text`,
  external identifiers in reserved `attrs` keys, read back through a new
  defaulted `LayoutDom::doctype_data` — with `document.doctype`,
  `createDocumentType`, `createCDATASection` and `nodeType` 4 / 10. The
  `ParentNode` / `ChildNode` mixins run the spec's node-or-string conversion,
  which is one insert now that fragments move. `normalize` (walked live, because
  the merge removes siblings), `outerHTML` both ways, and attributes with real
  namespaces surfaced as cached live `Attr` views through a `NamedNodeMap` —
  which also makes `MutationRecord.attributeNamespace` non-null. `clone_into`
  now carries every node kind, so a parsed page's comments, PIs and doctype
  reach the live document. `DOMParser.parseFromString` builds a new `Document`
  in the same arena through html5ever or xml5ever, `XMLSerializer` walks it
  back, and XHR's `responseXML` is wired to both. `Node.baseURI` closes a
  four-subtest false pass: `document.URL` landed two days after the baseline was
  pinned, so `undefined === undefined` had been scoring. `dom/ranges` 10 → 15
  all-pass and 24 → 35,466 subtests, `dom` 173 → 209 all-pass,
  `html/semantics/interfaces.html` 298 → 435 of 438, `selection` 12 → 31
  all-pass; +82,944 subtest passes over nine directories, zero pass-to-fail, and
  both `error` regressions are throughput walls on files that never passed. Raw
  maps under `Code/testing/genet/wpt-ledger/2026-09-07_dom_node_model/`.)
  (**landed 2026-09-07**: `Range`, `StaticRange`, `AbstractRange`, `Selection`
  and `getSelection` over the scripted arena. The DOM's live-range steps run at
  the bootstrap's own twelve-call-site mutation funnel rather than off the
  arena's observer record, because they need the child index and the boundary
  offsets as they stood *before* the mutation; boundaries are indexed by the
  node they sit in, since a flat list is quadratic and hung eight
  `editing/run/*` files in the first `post` map. One source of truth: the
  script-owned `Range` is it, and Livery's `TextRange` selection is a projection
  pushed through a new `SelectionHandler` seam, which also serves
  `Range.getClientRects` from the same range-rect primitive the overlay paints
  from. `ProcessingInstruction` became a real arena node in the same lane — both
  census directories' shared `common.js` aborted on its absence before a single
  subtest ran. `Selection` is declared by the generated table, which now reads
  `selection-api.idl`. `selection` 0 → 12 all-pass and 0/280 → 28,582/33,621
  subtests, `dom/ranges` 0 → 10 all-pass, `MutationObserver-childList`
  18/38 → 32/38 and `-characterData` 13/23 → 21/23; +29,126 subtest passes over
  five directories with zero pass-to-fail. Raw maps under
  `Code/testing/genet/wpt-ledger/2026-09-07_selection_range/`.)
- [mutation_observer_plan](2026-09-07_mutation_observer_plan.md)
  (**landed 2026-09-07**: `MutationObserver` as a second consumer of
  `genet-scripted-dom`'s mutation point. The arena fans out at the mutator into
  a spec-shaped `ObservedMutation` — siblings around a removal, old values,
  `innerHTML` / `textContent` as added and removed node lists, and the target's
  ancestor chain captured at mutation time — rather than widening or tapping
  the `DomMutation` stream Livery drains, which carries none of those and is
  fenced off from this lane. The record is off until something observes. The
  registry, `MutationObserverInit` validation, the interested-observer walk,
  transient registered observers and the notify microtask live in the JS
  bootstrap over three native sinks (`__moObserving`, `__moTake`, `__moGroup`);
  records are queued at mutation time through the bootstrap's twelve mutating
  call sites, because Nova's global natives cannot be interposed on.
  `MutationObserver` and `MutationRecord` left the generator's
  `SHAPE_ONLY_DENY` list, so the table declares them and the shape pass defers
  to the implementation. `sanity` 0/13 → 13/13, `takeRecords` 0/3 → 3/3,
  `disconnect` 0/2 → 2/2, `callback-arguments` 0/1 → 1/1, `attributes` 0/42 →
  35/42, `childList` 0/38 → 18/38; +495 subtest passes over `dom`,
  `custom-elements` and `html/dom` with zero pass-to-fail. Raw maps under
  `Code/testing/genet/wpt-ledger/2026-09-07_mutation_observer/`.)
- [idl_interface_table_plan](2026-09-07_idl_interface_table_plan.md)
  (**landed 2026-09-07**: the scripted tier's hand-maintained HTML interface
  table is replaced by one generated offline from WPT's vendored WebIDL
  (`tests/wpt/tests/interfaces/{html,dom,cssom}.idl`) plus its tag map
  (`html/semantics/interfaces.js`), by a dependency-free generator at
  `support/idl-interface-table`. 65 → 72 interfaces, 277 → 338 reflected
  attributes, 74 → 148 tag names, and 41 shape-only DOM/CSSOM interfaces;
  342 hand-written rows become 41 overrides, each with a stated reason. A
  drift test regenerates and byte-compares. Sixteen measured directories move
  5 `error -> fail` and 5 `fail -> pass` with zero pass-to-fail, +2,049
  subtest passes, and `html/semantics/interfaces.html` goes 0/438 → 298/438.
  Raw maps under
  `Code/testing/genet/wpt-ledger/2026-09-07_idl_interface_table/`.)
- [cheap_globals_plan](2026-09-07_cheap_globals_plan.md)
  (**landed 2026-09-07**: the day-scale entries from the census's
  missing-globals inventory, each a JS bootstrap over the existing VM
  primitives. `performance` with hr-time, User Timing and a
  `PerformanceObserver` on the drive loop (`timing.rs`); `queueMicrotask` on
  the existing microtask checkpoint; the structured serialize/deserialize
  algorithm as an engine-neutral value walk with a transferable registry
  (`structured_clone.rs`), the substrate the Worker lane reuses;
  `MessageEvent` / `MessageChannel` / `MessagePort` / `BroadcastChannel` and a
  real `window.postMessage` (`messaging.rs`); and `crypto` over a
  `RandomSource` host seam with a dependency-free ChaCha20 default
  (`crypto.rs`). `Image` / `Option` / `Audio` needed no code — the interface
  table already declares them. hr-time 0 to 2 all-pass, user-timing 1 to 24,
  performance-timeline 0 to 17, webmessaging 20 to 52, WebCryptoAPI 104 to 72
  errored; the structured-clone battery 0/150 to 119/150. Raw maps under
  `Code/testing/genet/wpt-ledger/2026-09-07_cheap_globals/`.)
- [xhr_plan](2026-09-07_xhr_plan.md)
  (**landed 2026-09-07**: XMLHttpRequest, XMLHttpRequestUpload and
  ProgressEvent over the deferred fetch seam, synchronous XHR through
  `FetchHandler::fetch_blocking`; first server-mode xhr baseline.)

- [wpt_harness_repair_plan](2026-09-07_wpt_harness_repair_plan.md)
  (**landed 2026-09-07**: `genet-wpt testharness` now runs every test in a
  worker subprocess, as `test262` does, so the `Atomics.waitAsync` hang is
  recorded rather than survived; the disk loader stops swallowing WPT support
  scripts whose name merely ends in `testharness.js` and stops handing `.py`
  server handlers to the engine; the server-mode drive loop takes
  `--drive-deadline` and advances its clock to the next timer instead of
  sleeping to it. The re-run census moves 204 files, every one attributed,
  none from a passing status. Raw maps under
  `Code/testing/genet/wpt-ledger/2026-09-07_harness_repair/`.)
- [web_platform_wpt_census](2026-09-06_web_platform_wpt_census.md)
  (**complete 2026-09-06**: exact `genet-wpt` testharness maps for every
  non-CSS WPT directory a web engine owns, `html` split by subdirectory;
  676 all-pass / 13,845 fail / 2,392 error / 985 no-results / 3,774 skip of
  21,672 files, with the missing-global inventory and four harness caveats.
  Raw maps under `Code/testing/genet/wpt-ledger/2026-09-06_platform_census/`.)

## ortet — the raw host

- [ortet_founding_plan](2026-09-03_ortet_founding_plan.md) (**O0-O4 landed;
  browser http(s) provisioning and its headed HTTP runtime receipt accepted
  2026-09-06; stable accessibility IDs and publication remain open**: the one
  headed port that proves the
  engine runs without Mere, over `genet-winit-host`, `genet-render-host`,
  `genet-documents`' Livery lane and `document-session-api`, with a cone
  witness that forbids every Mere crate and a self-driven frame receipt.
  Replaced Pelt as Genet's default host when Pelt moved to Mere.)
  [O5 scripted platform host](2026-09-03_ortet_founding_plan.md#o5-scripted-platform-host-in-progress-2026-09-07)
  is **in progress 2026-09-07**: the script-free/Boa/Nova selector landed
  structurally; real session scheduling/resource integration and native headed
  per-engine receipts remain open. Ortet is
  Genet's reference host; `genet-wpt` owns conformance scoring and Pelt proves
  downstream composition. Browser-hosted scripting and persistent test storage
  require their own later receipts.

## fleece — reader extraction

- [fleece_preservation_contract_plan](2026-09-05_fleece_preservation_contract_plan.md)
  (**active 2026-09-05**: preserves Fleece extraction identity, wire values, and
  arbitrary passage anchors for hashing, peer transfer, reopen, and annotations;
  adds named Web Annotation, structured-data, HTML, provenance, validation, and
  accessibility conformance gates with caller-owned capture and custody.)
- [fleece_followthrough_plan](2026-08-26_fleece_followthrough_plan.md)
  (**complete 2026-08-26**: the
  `genet-extract` shim is retired; retained static/scripted hosts activate
  Fleece-generated Text Directives with element fallback, indication, scrolling,
  one-fetch behavior, and script-visible URL privacy; Mere crawl and Gazette now
  consume supplied documents while eidetic-search drops its misplaced edge.
  Focused automated gates and the headed activation/indication receipt are
  green.)

## layout and styling

- [common_script_font_fallback_plan](2026-09-04_common_script_font_fallback_plan.md)
  (**Windows repair accepted 2026-09-05**: the paired Parley/Fontique patch
  queries actual text after authored faces fail. Both disclosure markers now
  paint through system fallback in Ortet, while Latin and explicit-font controls
  remain verified. Downstream product pins, upstream disposition, and other
  platforms retain separate gates.)

- [livery_flex_shorthand_plan](2026-08-25_livery_flex_shorthand_plan.md)
  (**complete flex-shorthand slice; Row 18 remains in progress**: Livery now
  expands `flex` and `flex-flow` into the longhand style fields already lowered
  to Taffy. The exact 1,358-file flexbox map records 115 gains, three assigned
  downstream false-pass losses, and the numeric-basis parser repair forced by
  the first candidate. The current eight-input Taffy seam is published and
  consumed as `genet-taffy 0.14.0`, published and tagged
  `genet-taffy-v0.14.0` at the Row 18 closure; 0.13.1 was the eight-input
  seam before it.)

## cambium — the desktop host

- Cambium, Workbench and `mere-surface-api` left genet for mere on 2026-09-03
  under the platform boundary plan; the `host_ui_zoom_plan` and the
  `workbench_component_plan` travelled with them and are now in mere's
  `design_docs/`.

## inker_docs/, nematic_docs/, verso_docs/ — moved to mere

- The engine-management layer left genet for mere on 2026-09-03 under the
  platform boundary plan: `inker`, `document-canvas`, the scrying/graft/weld
  engine adapters, `verso-tile`, `nematic`, `illume`, `errand` and `tinct`.
  These three area roots travelled with their code and are now in mere's
  `design_docs/`.

## codebase structure

- [orchestrator_decomposition_plan](2026-08-28_orchestrator_decomposition_plan.md)
  (**seven extraction phases complete 2026-08-29; follow-through recorded
  through 2026-09-02**: the original phases preserved behavior. Later entries
  separately record the layout transaction split, sizing repairs and their
  bounded receipts. The two-builder assessment recommends factoring shared
  table helpers if pursued. Pelt's code now lives in Mere.)

## archive_docs/ — completed plans

Per policy §4 and §8: a plan moves here once complete and once its open
points have a home elsewhere. Links into a moved plan are repaired in the
same session; links out of it are rewritten for its new depth.

- [2026-09-02/fleece_standards_adoption_plan](archive_docs/2026-09-02/2026-08-24_fleece_standards_adoption_plan.md)
  (**complete 2026-08-25, archived 2026-09-02**: Fleece 0.2 shipped canonical
  DOM-text coordinates, W3C Text Quote and Text Position selectors, and a Text
  Fragment projection; 0.3 hardened JSON-LD syntax harvesting and HTML
  Microdata; 0.4 added ordered Open Graph grouping, DOM document links, and
  semantic HTML table grids and header associations. No open points carried.)
- [2026-09-02/knot_evaluation_export_plan](archive_docs/2026-09-02/2026-06-12_knot_evaluation_export_plan.md)
  (**reconciled and complete for the first production capability set
  2026-07-27, archived 2026-09-02**; `include` closed, TOFU location rehomed to
  the fidelity plan, badge default carried by the block resolver plan. `include` transclusion fences over errand's smolweb transports,
  `lua eval` / `rhai eval` script fences via the `BlockEvaluator` slice,
  `to_gemtext` and gophermap exporters, the Knot production effect bridge,
  Turnstone consent, and the sealed attributable resolve cache. The production
  Knot adapter supplies anonymous HTTP(S) plus read-only Gemini, Gopher,
  Finger, Spartan, Nex and Guppy; Titan stays excluded.)

## Working principles

- **New docs go in `design_docs/`, never `docs/`.** See the policy's two-homes
  section for why both exist and what it would cost to merge them.
- **The smolweb boundary is spec versus use.** What a protocol *is* belongs to
  the smolweb workspace; what a browser *does with it* belongs here. Cite
  across the boundary by path — relative links do not survive it.
- **Prefer runtime verification to extended static tracing.** If runtime
  diagnostics are blocked, surface that blocker early rather than continuing to
  read code.
- **State the exact standards layer implemented.** Selector values are not the
  Web Annotation Protocol; JSON-LD syntax harvesting is not JSON-LD processing;
  raw URL attributes are not resolved links. Keep these boundaries visible in
  public types and receipts.
- **Trace the values the consumer actually receives.** Input normalization,
  script itemization and host projection can change the value under review.
  A diagnostic records that effective value and the selected implementation;
  an initially passing control is evidence to interpret, not a faulty test by
  definition.
- **Keep current gates synchronized after integration.** Update the parent,
  execution plan and this work map together. Preserve old receipts with their
  dates and source identities. When a dependency move retires a positive
  control, replace its exercised path or explicitly reopen that proof.
- **Freeze dependency resolution with a measured runner.** `Cargo.lock` is
  intentionally ignored here. Retain the generated lockfile, its digest,
  target/features and local-override facts with the source and binary receipt.
- **Scope behavior separately from implementation choices.** Tree/realm,
  event, transfer and storage semantics constrain backend and host placement.
  A supported slice names required assertions and residuals; aggregate WPT
  gains alone do not close it. A hosted proof names its actual session engine
  and providers, including any scripted or persistent-host prerequisite.
- **A lane's baseline map must come from the lane's own runner build.**
  `Cargo.lock` is ignored and rewritten by builds, so two runners built at
  different times can embed different dependency revisions even from the
  same tree; the cheap-globals lane saw 14 files move under an unmodified
  tree for that reason. Another lane's `post` maps are therefore not a
  controlled baseline. Build the unmodified tree in your own target
  directory, run `pre`, then run `post`, and record both runner digests.
  Build `pre` **before** the first edit, or from `HEAD` sources restored for the
  build: a baseline build started in the background while the tree is being
  edited compiles whatever the crate looks like when the compiler reaches it,
  and the resulting binary is not a baseline. See the cheap-globals plan's
  Findings and the MutationObserver plan's Progress.
- **A per-mutation walk over a script-created population is quadratic.** The
  bootstrap's mutation funnel runs on every DOM change, so anything hung off it
  must be indexed by the node it concerns, not scanned. The Selection and Range
  lane's first live-range list turned eight `editing/run/*` files that had
  merely failed into hangs, at four times the directory's wall time; keyed by
  node, the same directory ran faster than its baseline. A `post` map that
  slows a directory down is reporting a complexity defect, not noise.
- **A directory's census can be floored by one missing name.** Two directories
  in this session reported almost nothing because their shared setup file threw
- **A subtest that compares two absent things passes.** `Node-baseURI.html`
  scored 4 of 9 for two weeks on `undefined === undefined`; defining
  `document.URL` correctly turned those four into honest failures and the
  checked baseline read it as a regression. When a directory goes *down*, look
  for a newly-defined name on one side of an equality before looking for a bug,
  and treat a pin taken over an unimplemented feature as a record of the pin,
  not of the score.
  on a node type neither lane was about, aborting every file before its first
  subtest. Probe the shared `common.js` of a directory that will not move before
  concluding anything about the feature under test. Supplying the missing name
  took `selection` from 384 reported subtests to 33,621. The same file floored
  `dom/ranges` **twice**: `CDATASection` and, behind it, a constructible
  `Document`. Re-probe after each unfloor rather than assume one name was the
  only one.
- **A global native cannot be interposed on from the bootstrap.** Boa's host
  globals are writable and Nova's are not (`defineProperty` throws there too),
  so wrapping `globalThis.__someNative` works on one backend and silently does
  nothing on the other. Wrap at the bootstrap's own call sites instead — they
  are few, because the bootstrap already funnels — and prove the behavior on
  both backends. See the MutationObserver plan's Findings.
- **Verify paired forks from a standalone consumer.** Cargo root patches are
  not inherited by downstream workspaces. A coupled dependency must travel with
- **A virtual clock and a second agent are incompatible.** The disk drive loop
  jumps to the next timer's due time and never sleeps, which is right while one
  agent owns all the work. The moment a worker thread is live, that jump fires
  testharness.js's own 10s timeout before the worker has fetched its script, and
  every worker test reports `Test timed out`. Run on wall time while another
  agent can still speak, and make "can still speak" a *counted* report — an idle
  flag that does not say "idle as of which message" will cross a message in
  flight and quiesce the page over live work. See the Worker plan's Findings.
  its caller; prove that resolution before refreshing product revisions.
- **Parallel work needs commit fences as well as file fences.** Pin one base,
  give each worker a disposable detached worktree and disjoint write paths,
  inspect staged paths before committing, and remove the worktree immediately
  after integration.

## Status

Founded 2026-08-24; current work map reconciled 2026-09-07, including the IDL
interface-table, cheap-globals, MutationObserver, Selection/Range and DOM node
model lanes. The index covers
the flat plans sectioned above and two archived plans; the count in this line
was stale before 2026-09-07 and is now stated by the sections themselves.
All three former component area roots now live in Mere. The older `docs/`
corpus has selected execution entry points above; its full migration and
governance remain deferred under the policy's local addendum.
