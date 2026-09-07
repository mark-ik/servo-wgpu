# Cheap globals: `performance`, `queueMicrotask`, `structuredClone`, messaging, `crypto`

**Status:** landed 2026-09-07.

One lane over the missing-globals inventory in
[`2026-09-06_web_platform_wpt_census.md`](2026-09-06_web_platform_wpt_census.md#missing-globals):
the entries that are a day-scale sink each and move a named WPT directory off
zero, rather than the interface families (Worker, WebSocket, IndexedDB, Web
Audio, Selection) that need a new execution or storage model. Everything here
is a JS bootstrap over the existing VM primitives and one native sink, in the
shape `fetch.rs` and `platform.rs` already use.

Base commit `18fb1e42dfc` ("Generate the scripted tier's interface table from
WPT's WebIDL"). Receipts:
`Code/testing/genet/wpt-ledger/2026-09-07_cheap_globals/`.

## What the lane added

| Global | Home | Shape |
|---|---|---|
| `performance`, `PerformanceEntry` / `Mark` / `Measure`, `PerformanceObserver`, `PerformanceObserverEntryList`, `PerformanceTiming`, `PerformanceNavigation` | `components/script-runtime-api/timing.rs` | pure JS bootstrap, no native sink |
| `queueMicrotask` | the event-loop bootstrap in `lib.rs` | `Promise.resolve().then` onto the VM's own microtask queue |
| `structuredClone`, `__structuredClone`, `__sc_registerTransferable` | `components/script-runtime-api/structured_clone.rs` | pure JS value walk, no native sink |
| `MessageEvent`, `MessageChannel`, `MessagePort`, `BroadcastChannel`, a real `window.postMessage` | `components/script-runtime-api/messaging.rs` | JS bootstrap over the timer task source |
| `crypto` (`Crypto.getRandomValues`, `randomUUID`) | `components/script-runtime-api/crypto.rs` | one native sink `__crypto_random`, plus a `RandomSource` host seam |

`Image`, `Option` and `Audio` needed nothing: the interface-table lane already
declares all three as named constructors
(`dom/html_interfaces_generated.rs`), including `Audio`, which the census's
missing-globals table did not name. The lane's test asserts all three.

## Phases and done-conditions

### Phase 1 — `performance` (landed)

Done when `hr-time`, `user-timing` and `performance-timeline` move off zero and
`performance` is an `EventTarget` with `now`, `timeOrigin`, `toJSON`, `mark`,
`measure`, `getEntries*`, `clearMarks`, `clearMeasures`, and
`PerformanceObserver` with `observe` / `disconnect` / `takeRecords` and the
`buffered` flag delivering marks and measures on the drive loop.

Met. hr-time 0 -> 2 all-pass, user-timing 1 -> 24, performance-timeline 0 -> 17.

### Phase 2 — `queueMicrotask` (landed)

Done when `html/webappapis/microtask-queuing` passes and the ordering
against promise reactions is proved on both engines. Met:
`queue-microtask.any.html` and `queue-microtask-exceptions.any.html` both go
`fail -> pass`.

### Phase 3 — `structuredClone` (landed)

Done when the WPT battery runs and the module is reusable substrate for the
Worker lane. Met: `structured-clone.any.html` 0/150 -> 119/150, and the walker
lives in its own module with a transferable registry that `MessagePort` uses
rather than being special-cased inside it.

### Phase 4 — `MessageChannel` / `MessagePort` / `BroadcastChannel` (landed)

Done when `webmessaging` moves and port delivery is a task, not a synchronous
call. Met: webmessaging 20 -> 52 all-pass, 49 -> 114 subtests.

### Phase 5 — `crypto` (landed)

Done when `crypto` is an object (so feature detection stops throwing
`ReferenceError`), `getRandomValues` and `randomUUID` behave, and `WebCryptoAPI`
stops erroring on the way in. Met: 104 -> 72 errored, `randomUUID.https.any` and
`historical.any` pass, and 6,748 previously unreachable subtests now report.

### Phase 6 — named constructors (landed, no code)

Done when `Image`, `Option` and `Audio` are confirmed present. Met by test, not
by new code.

## Findings

### The clock (2026-09-07)

`performance.now()` reads `__virtualNow()` — the timers' virtual clock in
`EVENT_LOOP_BOOTSTRAP` — plus a 0.001 ms sub-tick per read. Two facts forced
that shape:

- hr-time asserts `performance.now() > 0` and a non-negative difference between
  consecutive reads. A clock that only moves when a timer fires reads 0 at the
  first call and fails. The sub-tick is monotone and deterministic, so a
  virtual-clock harness run stays reproducible.
- The cooperative (disk) timer path never advanced `vnow`; it fired by `delay`
  order and ignored real time. `__runTimers` now advances `vnow` to a fired
  task's `at` in **both** modes (`lib.rs`), so `performance.now()` reports
  elapsed time in disk runs. In real-time mode the `at > vnow` gate already
  guarantees `due.at <= vnow`, so the line is a no-op there and the drive
  loop's behavior is unchanged. A dated probe (below) confirms it changes no
  WPT status by itself.

`timeOrigin` is the engine's `Date.now()` captured when the bootstrap installs.
That needs no native sink and satisfies hr-time's `Date.now()` comparisons.

### `structuredClone` is a fused walk, not a serialization record (2026-09-07)

Serialize and deserialize are one memoised walk. That is what an in-process
clone needs, and nothing in the scripted tier yet needs a record that outlives
the walk. When the Worker lane arrives it will need the record form (a value
crossing an agent boundary), which is a split of `clone()` into two halves over
the same type dispatch, not a rewrite.

The cloneability rule went through one correction. The first version treated
"prototype is not `Object.prototype`" as "platform object, throw
`DataCloneError`". That is wrong: WPT's *Object with property on prototype*
requires an ordinary object with a custom prototype to clone as a plain object,
dropping the inherited property. The landed rule is an explicit platform-object
predicate (the global itself, a `Node`, anything with `nodeType` + `nodeName`,
an `Event`, an `EventTarget`, a `Promise`) plus the registered transferables.
It is a heuristic, not a WebIDL `[Serializable]` check: an unlisted platform
object with no such brand would clone as a plain object rather than throwing.

### Transfer and detachment (2026-09-07)

`ArrayBuffer` transfer uses `ArrayBuffer.prototype.transfer()` when the backend
implements it. Boa does not expose it and Nova exposes it but throws "not
implemented", so both fall back to a copy plus an own `byteLength: 0` data
property shadowing the prototype getter. That makes the sender's handle *read*
detached without releasing storage. A view over a "detached" buffer still works,
so the emulation does not implement detachment. Real detachment needs the VM
primitive; passing handle-level assertions does not prove transferred ownership.

The current `MessagePort` transfer implementation returns the same object,
marks it transferred for the current task, then clears that mark on the next
timer task. **Review correction, 2026-09-07:** this is an implementation
shortcut, not the specified transfer model, even within one agent. Re-enabling
the sender's object does not establish transferred ownership. A conforming
transfer needs a receiving port object and correct endpoint/queue custody
while the source remains detached. See the
[HTML port transfer model](https://html.spec.whatwg.org/multipage/web-messaging.html#message-ports).

### `MessageEvent.data` and the IDL default (2026-09-07)

`MessageEvent`'s `data` member defaults to `null` when absent. Constructing the
delivery event through the init dictionary therefore turned a `postMessage(undefined)`
payload into `null` and broke `webmessaging/{with,without}-ports/010.html`,
which had been passing on the old `Event`-shaped stub. The fix separates the two:
the constructor keeps the IDL default for script-constructed events, and
`internalMessageEvent()` assigns the deserialized value after construction. This
was the lane's only pass-to-fail regression and it is fixed, not explained away.

### `Cargo.lock` is ignored here, so a fresh target directory is not a runner (2026-09-07)

The first `post` census diffed against
`Code/testing/genet/wpt-ledger/2026-09-07_harness_repair/disk/`. `console` and
`encoding` were byte-identical, which looked like a sufficient positive control.
It was not: 13 XML/SVG files under `dom` and one under `html/webappapis` still
moved off `no-results`. A bisect disabled all four new surfaces, then the timer
line, then the `netfetch` feature, and finally built **unmodified `18fb1e42dfc`**
in this lane's own `C:/t/lane3-target` — where the same files still passed,
while the harness-repair and IDL lanes' binaries reported `no-results` on them
standalone. Genet ignores `Cargo.lock` (see DOC_README's working principles), so
a fresh target directory resolves its own dependency versions and is a different
runner. The lane therefore carries its own `pre/` map from the same target
directory, and the movements below are the engine change alone.

The general rule, for the next lane: **an unchanged-directory control proves the
runner is similar, not identical. Only a `pre` map built in the same target
directory isolates the change.**

## Before and after

Disk mode, engine Boa, renderer Livery, `--jobs 8`, `--timeout 90`.
`pre` runner SHA-256
`14ae345ec50bb616cb868a9c5a1b380a70f1291acfbb8b3528cf0c7e499fc209`;
`post` runner SHA-256
`f54de245b30fe3aee5360f70072907f931ad78e9be3840344db7adec8ddeef80`. Both built
`--release -p genet-wpt --features netfetch` at `18fb1e42dfc` plus this lane's
working tree.

| Directory | Files | All-pass pre -> post | Errored pre -> post | Subtests pre -> post |
|---|---:|---|---|---|
| hr-time | 14 | 0 -> **2** | 4 -> 1 | 0/14 -> **7/19** |
| user-timing | 36 | 1 -> **24** | 3 -> 0 | 3/133 -> **151/181** |
| performance-timeline | 51 | 0 -> **17** | 0 -> 0 | 0/73 -> **33/73** |
| webmessaging | 135 | 20 -> **52** | 10 -> 3 | 49/209 -> **114/216** |
| html/webappapis | 336 | 26 -> **29** | 30 -> 30 | 358/1114 -> **487/1114** |
| dom | 660 | 161 -> 161 | 45 -> 45 | 2439 -> **2441** |
| WebCryptoAPI | 138 | 1 -> **3** | 104 -> **72** | 3/199 -> **81/6947** |
| url | 49 | 10 -> 10 | 0 -> 0 | 351/519 -> 351/519 |
| console *(control)* | 14 | 2 -> 2 | 0 -> 0 | 6/29 -> 6/29 |
| encoding *(control)* | 1267 | 3 -> 3 | 1 -> 1 | 7109 -> 7109 |

Aggregate file-status movement, `post` against `pre`:

| Movement | Count |
|---|---:|
| `fail -> pass` | 75 |
| `error -> pass` | 4 |
| `error -> fail` | 41 |
| `no-results -> fail` | 1 |
| `pass -> fail` / `pass -> error` / `fail -> error` | **0** |

Subtest passes **+462**, over **+6,800** newly observed subtests.

`html/webappapis/structured-clone/structured-clone.any.html`: 0/150 ->
**119/150**.

### Explaining every non-forward movement

- **41 `error -> fail`.** All forward: the file used to die on a
  `ReferenceError` before reporting anything and now runs to completion with
  failures recorded. 32 are WebCryptoAPI files that reach `crypto.subtle` (which
  this lane deliberately does not implement) instead of failing at `crypto`;
  3 are hr-time's clamped-time-origin and cross-frame files, which need
  cross-origin isolation and frames; 4 are webmessaging files needing frames or
  workers; 2 are user-timing exception files.
- **1 `no-results -> fail`:**
  `user-timing/measure_exceptions_navigation_timing.html`. It used to report
  nothing and now reports 0/4. It requires `measure()` against a
  navigation-timing attribute whose value is 0 to throw `InvalidAccessError`;
  the lane throws `SyntaxError`. Named residual, below.
- **0 `pass -> fail`.** The one regression the lane did produce
  (`webmessaging/{with,without}-ports/010.html`, from the `MessageEvent.data`
  IDL default) was found in an earlier `post` run and fixed before landing; the
  final map has no pass-to-fail anywhere.
- `dom` **+2** subtests and no status movement: `structuredClone` reached by
  two `dom/events` subtests. `url`, `console` and `encoding` are unchanged.

## Residuals

Named omissions and observable conformance defects; the bounded landing
receipt does not close these:

1. **`crypto.subtle` is absent.** 72 WebCryptoAPI files still error. The whole
   algorithm catalogue is a separate lane; `crypto` existing is what this one
   bought.
2. **The default random source** (decided by Mark 2026-09-07, landed the
   same day): the in-tree ChaCha20 seeded from `std`'s hash keys is gone.
   Native targets fill from the operating system through `getrandom` 0.4,
   cfg-gated off wasm so the wasm cone still carries no `getrandom`; on wasm
   there is no default, `getRandomValues` throws `NotSupportedError` until
   the host installs the browser's source through
   `Runtime::set_random_source`. Nothing in the tree generates randomness
   itself.
3. **`ArrayBuffer` detachment is emulated,** not real, on both backends
   (Findings, above). The battery's *Resizable ArrayBuffer is transferable*,
   *Length-tracking TypedArray/DataView is transferable* and the OOB cases
   fail on this.
4. **`measure()` against a zero-valued navigation-timing attribute** throws
   `SyntaxError` where the spec says `InvalidAccessError`. One file, four
   subtests. It needs the navigation-timing attribute values to be real, which
   the scripted tier does not have.
5. **Absent serializable platform objects:** `FileList`, `ImageData`,
   `ImageBitmap`, `SharedArrayBuffer`, `CryptoKey`, `DOMException`. 16 battery
   subtests. Each arrives with the interface, not with the clone walker.
6. **`RegExp` source escaping.** `new RegExp('/')` must have `source` `\/`;
   the backends do not escape it, so the clone matches the input but not the
   expectation. Three battery subtests. An engine gap, not a clone gap.
7. **`PerformanceObserver.supportedEntryTypes` is `['mark', 'measure']`.**
   `resource`, `navigation`, `paint`, `longtask` and `element` entries need a
   network and rendering timeline the scripted tier does not report.
   `performance.timing` / `performance.navigation` exist as the zero-valued
   legacy shapes `toJSON()` is specified to carry, and nothing more.
8. **Cross-agent messaging is out of scope.** `BroadcastChannel` reaches other
   channels in this runtime only; `postMessage` has no frames or workers to
   reach. Those move with the Worker and iframe lanes, which reuse this
   lane's clone walker and transferable registry.
9. **MessagePort transfer temporarily disables and then re-enables the same
   object.** This does not prove sender detachment, a distinct receiving
   object or queued-message custody. Correct it before relying on transfer
   across agents. The [Worker scoping](2026-09-07_deferred_web_platform_lanes_scoping.md#dedicated-worker)
   requires those assertions plus a transportable serialization record and
   actual ArrayBuffer detachment; reusing the walker alone is insufficient.

## Gates

- `cargo test -p script-runtime-api` — 181 tests green, including
  `tests/cheap_globals.rs`: 10 test bodies run against **both** BoaEngine and
  NovaEngine (20 tests).
- `cargo clippy -p script-runtime-api --all-targets` — no warning in
  `crypto.rs`, `timing.rs`, `messaging.rs`, `structured_clone.rs`, `lib.rs` or
  `tests/cheap_globals.rs`.
- `cargo fmt -p script-runtime-api -- --check` — clean.
- `cargo build --release -p genet-wpt --features netfetch`, then the ten-directory
  disk census above.

## Progress

- **2026-09-07** — Phases 1-6 landed at base `18fb1e42dfc`. Five files touched
  or added under `components/script-runtime-api/`: `lib.rs` (module wiring, the
  `RandomSource` seam and setter, `queueMicrotask`, `__virtualNow`, the
  cooperative-mode clock advance), and the new `timing.rs`,
  `structured_clone.rs`, `messaging.rs`, `crypto.rs`, plus
  `tests/cheap_globals.rs`. Two defects found and fixed inside the lane: the
  `MessageEvent.data` IDL default (the only pass-to-fail) and the
  prototype-based cloneability rule. One measurement defect found and fixed:
  diffing against another lane's target directory is not a controlled
  comparison when `Cargo.lock` is ignored.
