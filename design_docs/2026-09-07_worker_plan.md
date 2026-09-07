# Dedicated Worker: a second agent of the same engine on its own thread

**Status:** landed 2026-09-07.

The dedicated-worker section of
[`2026-09-07_deferred_web_platform_lanes_scoping.md`](2026-09-07_deferred_web_platform_lanes_scoping.md#dedicated-worker),
promoted to a dated plan and executed. It builds directly on the
[cheap-globals lane](2026-09-07_cheap_globals_plan.md): `structured_clone.rs`
supplies the value walk this lane splits into a transportable record, and
`messaging.rs` supplies the `MessagePort` this lane teaches to cross a thread.

Base commit `1a5d5dbdb1c` ("DOM node model: fragment insertion, CDATA and
doctype nodes, ParentNode and ChildNode, attributes, DOMParser, baseURI"). Two
doc-only commits (`61b40915dea`, `4538a93ba09`) landed alongside this lane and
change no code, so both runners below share `1a5d5dbdb1c`'s tree.
Receipts: `Code/testing/genet/wpt-ledger/2026-09-07_worker/`.

**Not this lane.** `genet-scripted-worker` is the wasm-bindgen entry that runs a
whole `ScriptedDocument` inside a *browser* Web Worker for the wasm target. It
says nothing about the `Worker` a page script constructs, and it is untouched
here. The scoping document's correction on that point stands.

## What the lane added

| Surface | Home | Shape |
|---|---|---|
| `__scSerialize` / `__scDeserialize` — the transportable clone record | `components/script-runtime-api/structured_clone.rs` | pure JS, the same type dispatch as the fused walk, split into two halves over a JSON heap |
| Cross-agent `MessagePort`: remote stubs, `__portDeliver`, `__portTakePending`, a port codec on the transferable registry | `components/script-runtime-api/messaging.rs` | pure JS over one host routing sink |
| `Worker` (constructor, `postMessage`, `terminate`, `onmessage` / `onmessageerror` / `onerror`), `__workerPump` | `components/script-runtime-api/worker.rs` | JS bootstrap over seven native sinks |
| `DedicatedWorkerGlobalScope` (`self`, `name`, no `document` / `window`, `importScripts`, `close`, `postMessage`, `__wsPump`) | same | JS bootstrap over five worker-side sinks |
| The worker thread, its event loop, the link protocol and the page's `pump_workers` | same | Rust |
| `ScriptResourceLoader`, `Runtime::set_script_resource_loader`, `Runtime::pump_workers`, `Runtime::has_worker_work`, `Drop for Runtime` | `components/script-runtime-api/lib.rs` | the page-side seam |
| `.worker.js` and `.any.worker` hosting, `DiskLoader::resource_route`, `meta_directives` | `ports/genet-wpt/src/{harness,main,manifest,conformance}.rs` | the harness seam |

## The thread and the wire

### Placement

Both backends are `!Send`. The worker thread therefore constructs its own
`Runtime<E>` inside itself and owns it for its whole life; no VM handle, and no
`Runtime`, ever crosses the boundary.

The spawn compiles without an `E: 'static` bound because the engine never
appears in the closure. `Runtime::new` records
`worker::worker_main::<E> as fn(WorkerBoot)` in `HostState`, and
`__worker_create` spawns `move || spawn(boot)` — a closure over a plain function
pointer and a `Send` `WorkerBoot` (two channel ends and two `String`s). Adding
`+ 'static` to `impl<E: ScriptEngine> Runtime<E>` would have cascaded through
`genet-wpt`, `genet-scripted` and every harness entry point; the `fn` pointer
avoids it entirely.

### The wire is a byte-neutral string, not the walk's output

**Decision: the cross-thread wire is a JSON encoding of the serialization
record, not the record as live values.** It could not be otherwise here. The
`CallCx` surface marshals exactly one thing across the host boundary — a string
(`make_string` / `value_to_string`); even numbers cross the existing sinks as
strings. There is no engine-neutral value type to hand a channel, so "the
serialized form directly" is not an available option: the record only exists as
JS values inside one agent.

So `__scSerialize(value, transfer)` walks the same type dispatch as `clone()`
and emits `{"r": <root>, "h": [<nodes>]}`. A value is a tagged pair (`['s', …]`,
`['d', …]`, `['r', <heap index>]`); an object is a heap node keyed before its
own recursion, which is what preserves cycles and shared subgraphs. Bytes
(`ArrayBuffer`, typed-array backing stores, `Blob`) travel base64, because a
JSON string is UTF-16 and would mangle a lone byte. `-0`, `Infinity` and `NaN`
carry sentinel strings. `__scDeserialize` rebuilds in the receiving agent.

The cost is one encode and one decode per message. The benefit is that the wire
is inspectable, engine-neutral by construction, and identical for a future
process or wasm boundary. The lane's test asserts the shape that matters:
cycles, aliasing (two views over one buffer come back as two views over *one*
buffer), array holes, `Map` / `Set` / `Date` / `RegExp` / boxed primitives /
`Error`, and `DataCloneError` on a function.

### The link

One `std::sync::mpsc` pair per worker. Page to worker: `Message`,
`PortMessage`, `Resource` (a reply), `Terminate`. Worker to page: `Message`,
`PortMessage`, `Error`, `Resource` (a request), `Idle`, `Closed`. Every payload
is a `String` or plain data; nothing on the enum names an engine type.

**Resource loads are synchronous requests back to the page.** The worker's
classic script, every `importScripts` target and every `fetch()` go over the
link, and the page answers from its own route — a `ScriptResourceLoader` when
one is installed (disk-mode WPT), else the page's `FetchHandler` (server mode
and a hosted page). The worker blocks on the channel with a 30-second ceiling
and buffers any message that arrives meanwhile. That is what "the same resource
route as the page" means concretely, and it keeps the network stack on the
thread that owns it: `script-runtime-api` still links no network stack and is
still `!Send`.

### Quiescence, and the one race that mattered

The page's drive loop may not treat the agent as quiescent while a worker can
still speak, and a worker cannot wake itself — only the page can. So the worker
reports `Idle(n)` before it blocks, where `n` is the number of link messages it
has consumed, and the page believes it only when `n` equals everything the page
has sent to that worker.

The counter is not decoration. The first version sent a bare `Idle`, and
`port_transfer_on_boa` failed about one run in three: the worker's `Idle`,
emitted *before* the page's next message reached it, arrived in a later
`pump_workers` turn than the message it had crossed, so the page marked a
working worker idle and quiesced over live work. Nova happened to interleave
differently and passed. A bare liveness flag cannot express "idle as of what";
the count can.

### The virtual clock stops being virtual while a worker runs

Disk mode drives all three clocks virtually: each turn jumps `now_ms` to the
next timer's due time and never sleeps. With a worker in the picture that is
wrong — testharness.js's own 10-second timeout is a pending timer, so the first
jump fired it while the worker thread was still fetching its script, and every
worker test reported `Test timed out`.

While `has_worker_work()` holds, `drive_virtual` therefore sets the clock to
*wall* elapsed time plus the virtual jumps taken so far, and sleeps 1 ms when
nothing else moved. The clock stays monotone across the transition, the page's
own short timers still fire, and the 15-second drive deadline still backstops a
worker that never idles. `drive_wall` (server mode) already ran on wall time and
needed only the pump and the same "do not spin" wait.

### Cross-agent `MessagePort`

A port transferred out of an agent leaves a **stub** behind. The peer keeps its
entanglement; the stub carries a process-unique port id, and
`MessagePort.postMessage` forwards over the host link instead of dispatching
locally when its peer is a stub. The receiving agent adopts a real port whose
peer is a stub with the same id, so the two sides are symmetric and the host
routes purely by id. Undelivered messages in the sender's port queue are
re-serialized and travel with the port.

Routing is a `(port id, worker index)` table in the page's `HostState`, filled
when `Worker.postMessage` binds the ids the serialization just minted. A record
that itself carries a new port binds it to the same link, so a port sent
*through* a port lands in the right agent.

The in-agent transfer shortcut the cheap-globals plan recorded as a residual is
untouched, and still a shortcut. What is new is that a port leaving the agent
really is detached: the codec marks it `_crossAgent`, which suppresses the
one-task re-enable the in-agent path schedules.

## Phases and done-conditions

### Phase 1 — the transportable record (landed)

Done when the walk has a detached-record form that preserves cycles, aliasing
and transfers, proved on both backends. Met: `wire_record_round_trips` asserts
identity, cycles, shared buffers, holes, every boxed and collection type, the
numeric sentinels, and `DataCloneError` on an uncloneable value.

### Phase 2 — the worker agent (landed)

Done when a second `Runtime` of the same engine runs on its own thread with
`DedicatedWorkerGlobalScope`, no `document`, timers, `fetch`, `structuredClone`,
`performance`, `crypto`, `console`, `importScripts` and `close`, and messages
cross in both directions as tasks in order. Met:
`worker_round_trip_works`, `worker_scope_has_no_document`,
`worker_import_scripts_works`, `worker_timers_and_ordering_work`,
`worker_fetch_uses_the_page_route`, on Boa and Nova.

### Phase 3 — `Worker` on the page (landed)

Done when the constructor enforces the script-URL and `type` rules, `terminate`
stops delivery, and an unhandled worker exception reaches the page. Met:
`module_worker_is_a_named_residual` (a `type: "module"` worker throws
`NotSupportedError`, a zero-argument construction throws `TypeError`),
`worker_terminate_and_close_work`, and `worker_error_reaches_the_page` — which
asserts both halves: the `error` event on the `Worker` object, and, when it is
not canceled there, the report at the window, which is where testharness.js
listens.

### Phase 4 — `MessagePort` across the boundary (landed)

Done when a `MessageChannel` port can be posted to a worker and both directions
work. Met: `message_port_crosses_the_thread_boundary` does a full round trip
(worker to page, then page to worker, then worker to page) on both engines.

`SharedWorker` and cross-agent `BroadcastChannel` did **not** fall out cheaply
and are named residuals below.

### Phase 5 — the harness (landed)

Done when manifest worker variants stop being skipped and report real results,
with shared and service workers keeping their skip reasons. Met: see the tables
below. `ManifestTest::is_worker` became `is_unhostable_variant` (shared worker,
service worker, shadow realm) plus `is_dedicated_worker`; the conformance
crediting guard follows the same split, so a dedicated-worker variant can now
be credited as a pass and a shared-worker variant still cannot.

## Findings

### The engines run happily on a second thread (2026-09-07)

Neither backend needed a change. Both `BoaEngine::new` and `NovaEngine::new`
construct their state locally, so a `Runtime` built inside a spawned thread is
as good as one built on the main thread. Nothing in this lane touches
`components/script-engine-*`; the whole worker is host-surface work over the
existing VM primitives, which is the same result the rakers lesson has produced
in every lane since.

### `window` was a global `var` and could not be deleted (2026-09-07)

`install_host_surface` opened with `var self = globalThis; var window =
globalThis;`. A global `var` binding is a **non-configurable** property, so
`delete globalThis.window` was a no-op on both engines and the worker global
kept a `window` that was itself. `document`, assigned as a plain property by the
DOM bootstrap, deletes cleanly. The line is now two plain assignments, which
observably changes nothing for a page and lets the worker scope delete `window`.

This matters because testharness.js selects its environment on
`'document' in global_scope` — a *presence* test, not a truthiness test. Had
`document` been a `var` too, the worker lane would have needed the DOM surface
to become optional rather than removable.

The worker scope drops the whole window-only name set that WPT's own
`constructors/Worker/unexpected-self-properties.worker.js` enumerates, which
took that file from 41/57 to **57/57**.

### `globalThis instanceof DedicatedWorkerGlobalScope` needs `Symbol.hasInstance`

The engine owns the global object's prototype, and the bootstrap cannot rewrite
it portably. `WorkerGlobalScope` and `DedicatedWorkerGlobalScope` are therefore
ordinary constructors with a `Symbol.hasInstance` that answers true for
`globalThis` and nothing else. testharness.js's `create_test_environment` reads
exactly that, and the worker environment is what makes `done()` and the result
relay work.

### `importScripts` is indirect `eval`, and that is enough (2026-09-07)

`importScripts` fetches synchronously over the link and runs the source through
`(0, eval)(src)`. Indirect eval is specified to evaluate in the *global* scope,
so a helper's top-level `var` is a global on both backends —
`worker_import_scripts_works` asserts a `var` set in `helper.js` is visible in
the importing script. A native re-entry into the engine would have been the
alternative, and `CallCx` deliberately has no `eval`.

### The `.any.worker.js` file WPT's server would have generated (2026-09-07)

A `.any.worker.html` variant loads `<stem>.any.worker.js`, a file that exists
only inside `wpt serve`. Disk mode synthesizes it in
`DiskLoader::resource_route`: the worker-global `self.GLOBAL` stub, then
`importScripts('/resources/testharness.js')`, then the test's own `// META:
script=` helpers, then the test file, then `done()` — which is what
`tools/serve/serve.py` writes. `.worker.js` tests need no synthesis: the file
is real and does its own `importScripts`.

The META header scan moved to `harness::meta_directives` so the window wrapper
and the worker wrapper cannot drift over which helpers a test gets.

### One unexplained interaction, avoided rather than fixed (2026-09-07)

`ErrorEvent` is defined here as an ordinary `Event` subclass. Giving its
prototype a `Symbol.toStringTag` — so `String(errorEvent)` reads
`[object ErrorEvent]`, which
`workers/constructors/Worker/AbstractWorker.onerror.html` asserts — makes an
unrelated regression fail:
`harness::tests::test_driver_action_sequence_synthesizes_a_click` reports
`Test timed out` on Boa.

Bisected to that one line. What the instrumentation shows, and does not
explain: the action payload is queued and taken correctly, the element origin
resolves, the hit test returns the right node, and `__dispatchSynthetic`
returns `true` for `pointerdown` / `mousedown` / `pointerup` / `mouseup` /
`click` — but the node the dispatch reaches has an empty `__listeners`, so the
test's `click` handler never runs. That points at **wrapper identity**: the
object `document.getElementById` hands the test and the object
`wrapNode(__reflectNode(rawId))` hands the dispatch are not the same one, and a
single extra allocation in a bootstrap is enough to change which. `Worker`'s own
`Symbol.toStringTag` is fine, and `MessageEvent` has carried one since the
cheap-globals lane; only a *second* tag on an `Event`-derived prototype trips it.

The line is removed and the reason recorded in the code. `AbstractWorker.onerror`
fails on `filename` regardless (disk mode has no document URL), so nothing is
lost by it. The underlying fragility belongs to whoever owns the reflector cache,
not to this lane, and it is **Mark's call** whether it earns one.

## Residuals

Named, not faked:

- **`SharedWorker`** does not exist. `workers/` still has a large shared-worker
  population and several of its files throw `ReferenceError: SharedWorker is
  not defined` at top level, which is an honest `error`. Shared and service
  worker variants keep their skip reasons.
- **Cross-agent `BroadcastChannel`.** The bootstrap's channel registry is
  per-agent, so a worker and its page do not share a named channel.
- **Module workers** throw `NotSupportedError`, per the plan's staging.
- **Nested workers** run (a worker's own runtime has the whole `Worker`
  surface, and its `pump_workers` drives its children), but
  `workers/semantics/multiple-workers/003.html` shows only the innermost
  worker's contribution arriving — nested message relay through two links is
  not yet ordered correctly.
- **Real `ArrayBuffer` detachment.** The wire copies the bytes, so the
  *receiver* is correct; the sender's handle still uses the cheap-globals
  emulation (an own `byteLength: 0` shadowing the prototype getter) rather than
  releasing storage. That still needs the VM primitive, and this lane did not
  change it.
- **A hosted headed receipt.** The scoping document asks for a scripted hosted
  page exchanging messages with a live worker on each engine. Ortet selects
  `LiverySessionEngine` and has no scripted route, so that receipt is not
  available in this repository and is not claimed. The proof here is the
  runtime contract on both engines plus the WPT maps.
- **`workers/semantics/messaging`** is named in this lane's brief but does not
  exist in the vendored checkout; the equivalent populations are
  `workers/semantics/structured-clone` and `webmessaging`.

## Before and after

Disk mode, engine Boa, renderer Livery, `--jobs 8`, `--timeout 90`, drive
deadline 15s. Both runners built `--release -p genet-wpt --features netfetch`
in `C:/t/lane7-target`, the `pre` one from unmodified `1a5d5dbdb1c` **before**
the first edit.

`pre` runner SHA-256
`3c622685cdac9d239a55dc9a6cc566ec40c3aa27d76a7424b1a3ab0ec2fdc538`;
`post` runner SHA-256
`3d88ff95d3f24ef8643ee7d4e6cdcd93ef85a67f2a726834165154f87369430d`.

Raw maps: `Code/testing/genet/wpt-ledger/2026-09-07_worker/{pre,post}/`.

The file count rises because the `pre` runner dropped every dedicated-worker
variant from the manifest enumeration before running: those 207 files are new
*observations*, not new failures.

| Directory | Files pre -> post | All-pass pre -> post | Errored pre -> post | Subtests pre -> post |
|---|---:|---|---|---|
| workers | 247 -> 294 | 5 -> **74** | 21 -> **12** | 21/574 -> **321/967** |
| webmessaging | 135 -> 160 | 52 -> **65** | 3 -> **2** | 114/216 -> **158/276** |
| html/webappapis | 336 -> 356 | 31 -> **45** | 28 -> 27 | 493/1115 -> **977/1708** |
| dom | 660 -> 698 | 209 -> **217** | 28 -> 28 | 43961/53909 -> **44014/54222** |
| xhr | 348 -> 425 | 64 -> **95** | 28 -> **5** | 282/1191 -> **373/1578** |

Inside `workers`, by the done-condition's subdirectories:

| Subdirectory | Files pre -> post | All-pass pre -> post | Subtests pre -> post |
|---|---:|---|---|
| workers/constructors | 32 -> 35 | 0 -> **8** | 1/57 -> **101/138** |
| workers/semantics | 27 -> 31 | 0 -> **4** | 0/27 -> **67/122** |
| workers/interfaces | 53 -> 69 | 0 -> **30** | 0/61 -> **55/136** |
| workers/modules | 22 -> 25 | 0 -> 0 | 0/210 -> 2/243 |
| workers/baseurl | 7 -> 7 | 0 -> 0 | 0/3 -> 0/3 |

`workers/modules` is the module-worker residual and `workers/baseurl` is
network-dependent; both are expected to stay flat in disk mode.

Aggregate file-status movement, `post` against `pre`:

| Movement | Count |
|---|---:|
| `absent -> pass` (a newly hostable worker variant) | 76 |
| `absent -> fail` / `absent -> no-results` | 125 / 6 |
| `fail -> pass` | 53 |
| `error -> pass` | 5 |
| `error -> fail` | 8 |
| `error -> no-results` | 21 |
| `no-results -> pass` | 1 |
| `fail -> no-results` | 1 |
| `pass -> anything` / any subtest-count drop / any dropped file | **0** |

Subtest passes **+972**, over **+1,746** newly observed subtests; all-pass files
**361 -> 496**.

### Every movement away from a better status, explained

There are 30, and none is a regression in behavior.

- **`error -> no-results`, 21 files.** All are worker tests that need the
  network: `xhr/xmlhttprequest-timeout-worker-*` (13), `xhr/*-worker-origin`,
  and `workers/baseurl/alpha/{importScripts,xhr}-in-worker`. Before, `new
  Worker(...)` threw `ReferenceError: Worker is not defined` at load, which the
  runner recorded as an `error` with a message. Now the worker starts, the test
  waits for a response disk mode cannot give, and the drive deadline ends the
  run with no subtests reported. The honest disk-mode status for a
  network-dependent test is `no-results`; these belong to the census's
  server-mode lane, not to this one.
- **`error -> fail`, 8 files.** The same shape one step further: the file now
  loads and reports subtests rather than throwing on an undefined name.
- **`fail -> no-results`, 1 file.**
  `workers/Worker-postMessage-happens-in-parallel.https.html` spins waiting for
  a worker to observe a page-side value change while the page is blocked. The
  page's loop is cooperative — it runs the worker's messages between its own
  turns — so nothing is observed and the test reaches the deadline. That is a
  real limit of the current scheduler, and it is the one file that names it.
- **`absent -> fail` / `absent -> no-results`, 131 files.** Dedicated-worker
  variants that the `pre` runner never enumerated. A file that had no result
  before cannot regress; 76 of the 207 newly enumerated variants pass outright.

### Two defects the maps caught, fixed rather than explained away

- `terminate()` on a worker whose script is an infinite loop left
  `Drop for Runtime` joining a thread that could never reach the check, and the
  runner hung until its per-test subprocess timeout. `shutdown` now joins with
  a 500 ms ceiling and detaches what has not exited. The intermediate `post` map
  that exposed the hang was discarded and the table above is against the final
  runner, where `workers/Worker-terminate-forever-during-evaluation.html` is
  `fail 1/7` (the rest is the module-worker residual) and
  `workers/interfaces/WorkerUtils/WindowTimers/003.html` is `pass`. A cooperative
  `terminate` still cannot interrupt a running script without a VM interrupt
  seam; that stays a residual, but it no longer hangs the page.
- `terminate()` did not empty the port message queue, so a message already
  drained off the link was still delivered afterwards. `__workerPump` now skips
  a terminated worker, which is what "terminate a worker" specifies:
  `workers/Worker_terminate_event_queue.htm` `fail 0/1` -> **`pass`**.

## Progress

- **2026-09-07** — Lane landed. Phases 1-5 met. Gates: `cargo test` green for
  `script-runtime-api` (249 tests, including 22 new worker tests, 11 bodies on
  both backends), `genet-wpt` (62) and `genet-scripted` (25); clippy clean and
  rustfmt applied on every touched file; `cargo check --workspace --features
  genet-wpt/netfetch` clean. Two pre-existing rustfmt diffs in
  `ports/genet-wpt/src/{render.rs,testdriver/input_events.rs}` are untouched —
  they are not this lane's files. Residuals and the one unexplained interaction
  are above.
