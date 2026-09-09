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
| [Ortet](2026-09-03_ortet_founding_plan.md) | O0-O5 are landed for the native host. Exact-source Boa/Nova receipts accept native input, timer/microtask completion, bounded failure, idle-woken fetch and Worker delivery, semantic/pixel correlation, and stale-session rejection. Default Ortet stays script-free. Browser-hosted scripting and the AccessKit native bridge receipt retain later gates. Publication is a later packaging decision under the workspace's `publish = false` policy. |
| [K6 fragmentation](../docs/2026-08-15_buckram_k6_fragmentation_execution_plan.md) | K6a typed inputs, K6b's retained context/token model, and a dormant pre-K6c style/input dispatch seam landed. All 6,077 named records match the frozen pre-K6 baseline. Live formatter continuation and multicol geometry remain K6c work. |
| [Flex and grid, Row 18](2026-08-25_livery_flex_shorthand_plan.md#row-18-closure-and-remaining-work) | Bounded vertical-flex slices landed. Remaining work includes mixed-writing-mode baselines, generated/pseudo self edges, shared font metrics and percentage provenance, then a measured grid inventory. The dated guard review records 21 pre-K6 flex/grid pass-to-fail cases. |
| [Floats and shapes, Row 12](../docs/2026-08-25_buckram_horizontal_float_direction_reconciliation.md) | Horizontal box-shape and direction slices landed. Vertical/orthogonal transforms and remaining shape families retain their own unproved boundaries. |
| [Counters, lists and generated content, Row 17](../docs/2026-08-21_buckram_livery_lane_program_plan.md#wave-2-now-unblocked) | Open inventory. Name the computed-content, counter-scope and marker-box consumers and a bounded execution gate before implementation. |
| [K7 foundational sizing and dispatch](../docs/2026-07-26_buckram_css_layout_engine_plan.md#k7-foundational-sizing-and-dispatch-closure) | Reconcile landed sizing fixes with remaining deferrals before choosing a slice. Fragmentainer-dependent work consumes K6; final closure includes deleting CSS-facing Taffy block dispatch. |
| [WPT harness and ledger](../docs/2026-08-24_wpt_harness_ledger_execution_plan.md) | Exact scorer and reference-verification gates landed. Freeze a fresh candidate runner for new work; unsupported test/reference agreement earns no conformance credit. |
| [Servo cone retirement](2026-09-07_servo_cone_retirement_plan.md) | servo-paint's compositor half carved out as genet-compositor; the reftest lane renders through genet-render-host; the constellation trait cone left the graph 2026-09-07. All gates green; next proof is Mere's dependency rename at its next bump. |
| [Web platform WPT census](2026-09-06_web_platform_wpt_census.md) | Baseline exact maps for 41 non-CSS WPT directories (21,672 files, disk mode, Boa/Livery) landed 2026-09-06. Three of its four harness caveats are closed by the harness-repair plan; the reftest caveat and the per-directory lanes remain open. |
| [Standards-to-features ledger](2026-09-07_standards_to_features_ledger.md) | Founded 2026-09-07 by Mark's ruling on mere's lighter-recall brief: fourteen rows pairing a standard, its census count, what adhering means in genet, and what mere and the products unlock; consumers on record. The census is the authority for current numbers. |
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
| [Reflector identity](2026-09-07_reflector_identity_plan.md) | Wrapper liveness follows node reachability: every wrapper has an opaque root, a connected node's wrapper lives as long as its document, a detached subtree's as long as script holds any one of it. Landed 2026-09-07 with zero census movement over six directories under both harness-collection settings; the receipts are the reproducer set and the restored `ErrorEvent` `toStringTag`, not a score. Residual: the between-tick window for detached trees. |
| [Host contract ownership](../docs/2026-08-14_web_platform_host_contract_plan.md) | Genet owns retained session contracts; Mere owns surface orchestration and product adapters. The older S0-S5 receipts need a consumer-side status refresh before resuming those lanes. |
| [Shadow DOM](2026-09-07_shadow_dom_plan.md) | A parentless shadow root in both DOMs, a per-host slot assignment table maintained at the mutation, `flat_children` under Livery's rendering traversals, per-rule tree-scope matching with `:host` / `:host()` / `::slotted()` / `::part()`, event retargeting and `composedPath()`, and the declarative post-parse pass with `<template>.content` in one shared inert document. Landed 2026-09-07/08: `shadow-dom` 6 to 45 all-pass and 24 to 1,512 subtests, +1,761 subtest passes over four directories, one explained pass-to-fail. Residuals: `:host-context()`, `adoptedStyleSheets`, focus delegation, and declarative attachment consulting the custom-element registry (which needs parser/script interleaving — Mark's call). |
| [Parser/script interleaving](2026-09-08_parser_script_interleaving_plan.md) | HTML's parsing model with scripts run at the point the tree builder pops them: an html5ever `TreeSink` over the live arena, `document.write` at the tokenizer's insertion point, `currentScript`, the `readyState` transitions with `DOMContentLoaded` and `load`, parse-time custom-element upgrade, and declarative shadow roots consulting the registry. Landed 2026-09-08 in the engine (part one: +68 subtest passes over eight directories, 30 files `fail -> pass`, zero pass-to-fail, no repins). **Part two, 2026-09-08**, routed the WPT runner and `LiveryScriptedDocument` through the same parse and closed the named residuals: `testharness.js` as a prelude and one `load` dispatch, a two-phase `LiveryCssom` that resolves author sheets from the arena as the parser fills it, `document.write` tokenized inside the call so its markup is visible to the writing script and a written `<script>` runs, the open stream appending through one live tokenizer, upgrades at element creation, foreign-namespace scripts, and scripts in template contents left inert. +181 subtest passes over ten directories and +439 across the 79-directory disk census (excluding two identified timing artifacts), 19 census `pass -> fail` all attributed, six baselines repinned forward-only, all fourteen at `unexpected=0`, Ortet digest unchanged. **Both Shadow DOM declarative regressions now recover in WPT** — the open gate is closed. |

| [iframes and nested browsing contexts](2026-09-08_iframes_plan.md) | HTML's browsing-context tree in `genet-documents` (parent, children, top, each context's active document, origin, sandbox flags and its own session history), loading through the parent's own resource route (`src`, `srcdoc`, `about:blank`, the initial-`about:blank` rules, `sandbox` / `allow` / `loading`), and the child's scene composited into the parent's replaced box as a **paint-list splice** rather than a producer texture, clipped to the content box, with hit testing descending into it. Landed 2026-09-08: +23 subtest passes over eight directories, two files `fail -> pass`, zero pass-to-fail, three baselines repinned forward with six named entries, all fourteen at `unexpected=0` and both reftest guards at `unexpected=0`. Ortet composites child frames through the script-free Livery route with no Ortet code change; the article digest is unchanged at `0x6377ba8a6bf4dbc9` and the new `frames.html` receipt is `0x2e7dd44b19907205`. **The per-context `Runtime` and the cross-origin `WindowProxy` are a decision for Mark**: two `Runtime`s are two engine instances and this stack's cross-instance boundary marshals strings, which cannot carry the same-origin object identity `contentWindow` requires. |
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
- [iframes_plan](2026-09-08_iframes_plan.md)
  (**landed 2026-09-08**: nested browsing contexts. `BrowsingContextTree` in
  `genet-documents` is deliberately *data*, not a session — identity, origin,
  sandbox flags and one session history per context, joined to whichever engine
  renders the document by `BrowsingContextId`, so the script-free and scripted
  routes share one tree without importing each other. An opaque origin carries a
  serial, because HTML's opaque origin is same-origin with nothing but itself;
  `initial_about_blank` is a bit rather than a URL test, because a document can
  navigate to `about:blank` deliberately and that one neither inherits its
  container's origin nor lets its successor replace. `ResolvedDocumentResources`
  gained `frames`, so a child is fetched through **exactly** the route its parent
  used; nesting recurses one level up where an HTML parser lives, guarded by
  HTML's matching-nested-contexts rule and a depth bound. The composite is a
  **paint-list splice**, not the `ExternalTextureDraw` path `paint_list_api` names
  as an iframe candidate: an external texture is composed *after* the scene, so it
  sits outside the scene's clip and transform stack and is invisible to a software
  rasterizer — the reftest lane and Ortet's own capture would both see an empty
  box. Livery already recorded the right position for a custom leaf while its
  ancestors' clips were live, so `<iframe>` got its own `FrameSlot` list (a
  separate key space, because a custom leaf's key is an author attribute and a
  frame's is a node id). Two findings fell out: image keys are per-list ordinals
  while font keys are content-hashed, so splicing a child without re-keying its
  images draws *the parent's* picture inside the frame — a plausible wrong answer,
  not an error; and a recorded index into a command stream is invalidated by every
  later insertion, which `translated` and `scaled_to` both perform at index 0.
  `<iframe>` also became a real replaced element with a **default object size** and
  no natural ratio, which needed its own branch before every ratio rule —
  `width: 600px; height: auto` is 600x150, not 600x300. On the script side
  `window.length` and `window[i]` now read the document (which makes a templated
  frame correctly absent with nothing checking for a template), `frameElement` is
  `null`, and `postMessage` enforces `targetOrigin`. `webmessaging` 65 -> 67
  all-pass, `the-window-object` 62 -> 69 subtests, `dom` +12; +23 over eight
  directories with zero pass-to-fail. The reftest maps are byte-identical, and
  four composite tests in `genet-wpt` prove that means *no reftest exercises a
  frame* rather than *the composite never ran* — two iframe reftests run in those
  directories and neither discriminates. Raw maps under
  `Code/testing/genet/wpt-ledger/2026-09-08_iframes/`. Residuals: the second
  `Runtime` and `contentWindow` — **Mark's call**, because two `Runtime`s are two
  engine instances and `CallCx` marshals strings, so a same-origin
  `contentWindow` needs realms in `ScriptEngine` or stays permanently partial —
  plus `document.domain`, the cross-origin `WindowProxy`, COOP/COEP enforcement,
  child navigation, lazy loading, focus, and a headed script-driven receipt.)
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
- [selection_range_plan](2026-09-07_selection_range_plan.md)
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
- [reflector_identity_scoping](2026-09-07_reflector_identity_scoping.md)
  (**research 2026-09-07**: why a node's JS wrapper — and its listeners — could be
  silently replaced at a GC tick. Establishes the mechanism with receipts on both
  engines and lays out four fix options. The entry this document recorded as
  owed, written by the fix lane below.)
- [reflector_identity_plan](2026-09-07_reflector_identity_plan.md)
  (**landed 2026-09-07**: the fix Mark chose — wrapper liveness follows **node
  reachability**, not wrapper state. Every wrapper has an opaque root, the root of
  its node's tree, and is alive while that root is; a document is always alive.
  The engine contract gained `minted_reflectors` / `root_reflectors` /
  `unroot_reflectors` on Boa and Nova, with `drain_dead_reflectors` still the
  liveness signal for unrooted reflectors. Connected nodes are decided from the
  arena and host-rooted — on the mint, on insertion, and at each tick, against a
  tree-root cache keyed on a new structural-mutation epoch. Detached trees cannot
  be decided by the host at all, because a strong root destroys the evidence for
  the question, so their liveness is handed to the collector as an ephemeron
  cycle in the bootstrap. Policy cost over 4,002 touched nodes: 0.5 ms on a
  quiescent frame, 2–3 ms after a re-parent, against a 16 ms (Boa) / 107 ms
  (Nova) whole tick. The WPT harness now collects once per drive turn, on by
  default. `dom`, `custom-elements`, `html/webappapis`, `workers`, `selection`
  and `html/semantics/interfaces.html` are **byte-identical** before and after
  under both settings — zero pass-to-fail, zero fail-to-pass — with the one
  collection-off difference traced to `--jobs 8` scheduling on a worker test that
  passes both ways in isolation. Seven regression functions on both engines, ten
  of the twelve confirmed failing with the policy switched off; the gc-arena
  soak restated to "bounded by reachable touched nodes" and asserting both
  directions. Baselines `unexpected=0`, Ortet digest unchanged. Raw maps under
  `Code/testing/genet/wpt-ledger/2026-09-07_reflector_identity/`.)
- [parser_script_interleaving_plan](2026-09-08_parser_script_interleaving_plan.md)
  (**landed 2026-09-08 in the engine**: the residual the Shadow DOM lane left to
  Mark. html5ever already had the seam — `Tokenizer::feed` returns
  `TokenizerResult::Script(handle)` when the tree builder pops a `</script>`,
  with the element and its text already in the tree — so the lane is a
  `TreeSink` over the **live arena** plus a drive loop around it. Parsing through
  the arena's own `LayoutDomMut` mutators is why a `MutationObserver` sees parser
  insertions for free: `append_child` already writes the record, maintains slot
  assignment and advances the structural epoch. The arena is **moved** between
  the host state and the parser's cell around each pause rather than shared
  behind a second `RefCell`, because a tree-sink call and a DOM native both want
  `&mut ScriptedDom` and are never live at once. The two questions the tree
  builder asks the script tier — `allow_declarative_shadow_roots` and
  `attach_declarative_shadow` — are answered from a table refreshed **after**
  each script, which is exact rather than approximate because the registry can
  only change while a script runs. `document.write` during a parse is
  `BufferQueue::push_front`, the spec's insertion point literally; after a parse
  it implies `document.open` and re-materializes the document, with the exact
  rule and its two limits named. `readyState` stopped being a bootstrap constant
  and became a host fact the driver moves through loading → interactive →
  complete at HTML's points. `html/syntax` errored **62 → 1** (61 of them one
  undefined `document.write`), `dynamic-markup-insertion` 1 → **30** all-pass,
  `custom-elements` 8 → 9, `html/semantics/scripting-1` errored 125 → 103;
  `dom`, `html/dom/documents` and `html/webappapis/scripting` byte-identical.
  +68 subtest passes, 30 `fail -> pass`, 2 `error -> pass`, **zero pass-to-fail**,
  no baselines repinned, Ortet digest unchanged. `innerHTML` on a `<template>`
  was operating on its children and now operates on its contents. The two Shadow
  DOM declarative WPT files do **not** move: the runner never parses with
  scripts interleaved, and this lane was fenced out of `ports/genet-wpt/src`.
  Raw maps under
  `Code/testing/genet/wpt-ledger/2026-09-08_parser_script_interleaving/`.)
  **Part two, landed 2026-09-08**, closes that gate. `run_test_with_webgl_and_style`
  parses interleaved through a `ParserScriptLoader` over the existing
  `ScriptSrcLoader`, with `testharness.js` and the results bridge as a *prelude*
  and exactly one `load` dispatch (`Runtime::begin_parsed_testharness`); the XML
  corpus keeps parse-then-run because `xml5ever` has no pause driver.
  `LiveryCssom::install_over_parse` is the two-phase form both
  `LiveryScriptedDocument::build` and the runner's rendering session use: a live
  stylesheet source with a fetcher that serves nothing, so sheets enter the
  cascade at the point the parser inserts them. The one tokenizer moved into
  `MarkupState`, because HTML has the parser process written characters
  **during** the `document.write` call — the whole `document-write/0xx` battery
  is written in that shape — with the pause it reaches *stalled* for the driver,
  which keeps script timing where it belongs. Custom-element upgrades run at
  creation, by feeding the tokenizer a tag at a time while any definition
  exists; foreign-namespace `<script>` elements are collected at creation and
  run at the next pause; a script in a template's contents does not run; and the
  arena carries a `parsing` flag so an `innerHTML` from a parser-run script
  cannot free the tree builder's own handles. +181 subtest passes over the named
  subset with two explained `pass -> fail`, +439 across the 79-directory census
  once two identified timing artifacts are removed, 19 census `pass -> fail` all
  attributed — sixteen of them `html/dom/render-blocking/*`, which passed only
  because the old post-parse `document.open` had wiped the document they assert
  about. Six testharness baselines repinned forward-only; all fourteen and both
  reftest guards at `unexpected=0`; Ortet digest `0x6377ba8a6bf4dbc9` unchanged.
  Maps under
  `Code/testing/genet/wpt-ledger/2026-09-08_interleaving_part_two/`.)
- [shadow_dom_plan](2026-09-07_shadow_dom_plan.md)
  (**landed 2026-09-07/08**: Shadow DOM across both DOMs. A shadow root is a real
  node in the same store with **no parent**, hung off its host through two side
  maps rather than through the host's `children`, so every existing
  `dom_children` walk — layout, serialization, `querySelector`, Fleece — skips it
  by construction rather than by being told to; `<template>` contents use the
  same trick, in one inert `Document` shared by a document's templates, per
  Mark's ruling. Slot assignment is recomputed eagerly per affected root at the
  mutation, behind a `shadow_hosts.is_empty()` guard, so a shadow-free document
  pays one hash check per structural mutation. Declarative
  `<template shadowrootmode>` is a **post-parse pass**, not an html5ever change,
  so one set of rules serves both tiers and a script-free page reaches the same
  flat tree. Livery's rendering traversals moved to `flat_children` while the
  DOM-ordered ones (selection sources, `:nth-child` ordinals, the canvas
  background source) deliberately did not; the cascade descends the flat tree
  and numbers siblings in the node tree. Style scoping is decided **per rule**
  from each sheet's owner node, because a flattened cascade has lost the sheet;
  `selectors` 0.39 already implemented `:host` / `::slotted()` / `::part()`, so
  what the lane built was the boundary around them. `tree_root` continuing
  through the host is the entire reflector-identity integration. `shadow-dom`
  6 -> **45** all-pass, 36 -> **13** errored, 24 -> **1,512** subtests;
  `the-template-element` 273 -> **514**; +1,761 subtest passes over four
  directories, 47 files `fail -> pass`, one explained `pass -> fail` and one
  explained `fail -> error`, both from Genet parsing a document before it runs
  its scripts. Three baselines repinned forward. Raw maps under
  `Code/testing/genet/wpt-ledger/2026-09-07_shadow_dom/`.)
- [web_platform_wpt_census](2026-09-06_web_platform_wpt_census.md)
- [standards_to_features_ledger](2026-09-07_standards_to_features_ledger.md)
  (**complete 2026-09-06**: exact `genet-wpt` testharness maps for every
  non-CSS WPT directory a web engine owns, `html` split by subdirectory;
  676 all-pass / 13,845 fail / 2,392 error / 985 no-results / 3,774 skip of
  21,672 files, with the missing-global inventory and four harness caveats.
  Raw maps under `Code/testing/genet/wpt-ledger/2026-09-06_platform_census/`.)

## ortet — the raw host

- [ortet_founding_plan](2026-09-03_ortet_founding_plan.md) (**O0-O5 native
  landed; browser http(s) provisioning and its headed HTTP runtime receipt
  accepted 2026-09-06; AccessKit session-generation custody landed, with its
  native bridge receipt still open**: the one
  headed port that proves the
  engine runs without Mere, over `genet-winit-host`, `genet-render-host`,
  `genet-documents`' Livery lane and `document-session-api`, with a cone
  witness that forbids every Mere crate and a self-driven frame receipt.
  Replaced Pelt as Genet's default host when Pelt moved to Mere.)
  [O5 scripted platform host](2026-09-03_ortet_founding_plan.md#o5-scripted-platform-host-native-route-accepted-2026-09-08)
  is **accepted for native Boa and Nova 2026-09-08**: the script-free/Boa/Nova
  selector, completion-driven scheduling, bounded semantic receipts,
  idle-woken fetch and Worker delivery, and stale-session rejection passed at
  exact source `f3dc1bcf909`. Ortet is
  Genet's reference host; `genet-wpt` owns conformance scoring and Pelt proves
  downstream composition. Browser-hosted scripting and persistent test storage
  require their own later receipts. Crates.io publication is a packaging
  precondition under the workspace's `publish = false` policy, rather than
  Ortet runtime work.

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
- **A subtest that compares two absent things passes.** `Node-baseURI.html`
  scored 4 of 9 for two weeks on `undefined === undefined`; defining
  `document.URL` correctly turned those four into honest failures and the
  checked baseline read it as a regression. When a directory goes *down*, look
  for a newly-defined name on one side of an equality before looking for a bug,
  and treat a pin taken over an unimplemented feature as a record of the pin,
  not of the score.
- **A directory's census can be floored by one missing name.** Two directories
  in this session reported almost nothing because their shared setup file threw
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
- **A virtual clock and a second agent are incompatible.** The disk drive loop
  jumps to the next timer's due time and never sleeps, which is right while one
  agent owns all the work. The moment a worker thread is live, that jump fires
  testharness.js's own 10s timeout before the worker has fetched its script, and
  every worker test reports `Test timed out`. Run on wall time while another
  agent can still speak, and make "can still speak" a *counted* report — an idle
  flag that does not say "idle as of which message" will cross a message in
  flight and quiesce the page over live work. See the Worker plan's Findings.
- **Verify paired forks from a standalone consumer.** Cargo root patches are
  not inherited by downstream workspaces. A coupled dependency must travel with
  its caller; prove that resolution before refreshing product revisions.
- **A liveness question the host cannot answer belongs to the collector.** If
  the host takes a strong root to keep something alive, it can no longer ask
  whether anything else was keeping it alive — the root is the answer's own
  confounder, and no order of unroot, collect and query recovers it without the
  collection taking the object the question was about. Express the *relation*
  instead, as a reference cycle a tracing GC already resolves: a `WeakMap` from
  each member of a group to a shared array of all members is an ephemeron, so
  the group lives exactly while any member is reachable. See the reflector
  identity plan §1.1.
- **Make encapsulation a property of the shape, not a flag every consumer
  checks.** A shadow root and a `<template>`'s contents are both unreachable
  from the document by *construction*: neither has a parent, so no walk that
  descends `dom_children` — layout, serialization, `querySelector`, Fleece's
  extraction, the named-property scan — can enter one, and none of them had to
  be taught anything. The cost of the choice is that a copier must be told
  explicitly, and there were three (`clone_into`, `copy_fragment_node`,
  `cloneNode`); each was silently dropping the content until it was. That trade
  is the right way round: a missed copy is a visibly empty `content`, while a
  missed encapsulation check is a leak nobody notices. See the Shadow DOM plan's
  Findings.
- **A named property script has replaced must survive the next refresh.**
  `__refreshNamedProperties` deleted every name it had installed before
  reinstalling from the document. Its own comment already recorded that an
  `id="test"` element must not shadow testharness's `test()`, and its setter got
  the shadowing right — but the *next* refresh took the script's value back out.
  Nothing triggered a mid-file refresh until this lane's `setHTMLUnsafe` did, and
  then a shadow-DOM file died with `not a callable function` from calling
  `test(...)`. A guard that is correct once and re-run later is not a guard;
  check that what you are removing is still the thing you installed.
- **A container whose contents are deliberately unreachable must have every
  accessor told, one at a time, and each one is silent until exercised.** The
  Shadow DOM lane found three copiers that walked a `<template>`'s children and
  so copied nothing; this lane found `innerHTML`, which had the same shape — the
  getter serialized an always-empty child list and the setter put nodes where no
  walk reaches. Nothing caught it for a day because nothing set a template's
  `innerHTML`. When encapsulation is a property of the shape rather than a flag,
  the bug is never a wrong answer; it is an accessor nobody has pointed at the
  contents yet. Enumerate them deliberately rather than waiting for a
  reproducer.
- **A table a `&self` sink reads must be refreshed after the script, not before
  it.** The tree builder asks the script tier questions from inside a live
  tokenizer, where an engine call would re-enter it, so the answers come from a
  table the driver refreshes at each pause. Refreshing at the *top* of the pause
  is one script too early: the definition the next stretch of tokenizing asks
  about is the one the script that is about to run has not made yet. See the
  parser/script interleaving plan's Findings.
- **A model no consumer exercises is not validated by the tests that do not
  exercise it.** The parser/script lane's part one queued `document.write`
  source until the calling script returned, and passed 22 runtime cases and a
  whole WPT directory doing it — because on both routes the DOM was never read
  back inside the writing script. The first honest consumer turned 21
  `document-write/0xx` files from `pass` to `fail` in one step. When a design
  has a "we apply it later" step, find the test that reads it *now* before
  believing the green.
- **A pass can be the absence of the thing being tested.** Sixteen
  `html/dom/render-blocking/*` files passed because their `document.write`
  helper implied `document.open`, which **wiped the document** their assertion
  looks in; an empty document satisfies `assert_false(!!getElementById(...))`.
  This is the `Node-baseURI` principle at document scale: when a directory goes
  *down*, look for something that used to be absent before looking for a bug.
- **A slower runner reads as a regression at the census timeout.** One
  `shadow-dom` file takes 84.5 s on the pre runner and 90.3 s on the post one,
  against a 90 s census timeout, and its 376 subtests are the whole of that
  directory's apparent loss. Time the file on both runners before attributing a
  status move; the same directory at `--timeout 240` was a gain.
- **The tree builder holds ids the arena is free to reclaim.** While a parser is
  building into the scripted arena, `innerHTML` from a script the parser is
  running must orphan a replaced subtree rather than free it — the open-element
  stack still points at it. The flag has the same shape `observing` already had,
  and the arena's own fence caught it on the first file that could have gone
  wrong silently.
- **A headed digest is not a receipt until it repeats, and a moved digest is not
  a verdict until a control renders the same page without the feature.** An
  Ortet page built for the iframes lane produced a different digest on every
  run; two captures differed by **one pixel** at a glyph edge in 9px text, in
  the parent, outside every frame. The same page with the frames replaced by
  plain boxes was equally unstable and `article.html` was byte-stable, so the
  cause was genet's glyph rasterization at very small sizes, not the new code.
  Take a digest three times before recording it, and keep receipt pages out of
  sub-10px type.
- **A composite that happens after the scene is outside the scene.** An external
  texture is composed by the host in its own pass, so no CSS clip, transform or
  stacking order reaches it and no software rasterizer sees it. Anything that must
  be clipped by its container, travel under an ancestor transform, or appear in a
  captured frame belongs *in* the paint list. See the iframes plan §3.
- **Two resource tables, two key-minting rules, and only one fails loudly.**
  Merging a child paint list into a parent's is a dedupe for content-hashed font
  keys and a re-keying for per-list ordinal image keys. Skip the re-keying and the
  frame draws the parent's image: a plausible wrong picture, never an error. Read
  how a key is minted before merging two lists that carry them.
- **A recorded index into a command stream is a position, and positions move.**
  Livery's host-leaf slots survived for months because nothing wrapped the list
  while a slot was outstanding; `translated` and `scaled_to` both insert at index
  0, so the moment a second slot kind existed the bug was one page-zoom away.
  Every insertion has to shift every outstanding slot.
- **A default object size is not an intrinsic size.** Giving a replaced element
  with no natural dimensions a 300x150 "intrinsic size" hands it a 2:1 natural
  ratio, and `width: 600px; height: auto` then renders 600x300. Axes that never
  speak to each other need their own branch *before* every ratio rule, not a value
  threaded through them.
- **A cross-instance boundary that marshals strings cannot carry object
  identity.** The Worker lane could live with it, because the specification's
  worker boundary is a message queue. A same-origin iframe cannot, because the
  specification's boundary there is a shared object graph:
  `iframe.contentWindow.document.getElementById(x)` must return the node the
  child's own script sees. When a surface's contract is identity rather than
  transport, a marshalled proxy is not a partial implementation of it — it is a
  different thing that scores well. See the iframes plan §4.
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
