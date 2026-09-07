# Reflector identity and wrapper-held DOM state — scoping

**Status (2026-09-07): research, complete. No code change proposed here.**
Scoping only, per the Worker plan's hand-off: the fragility "belongs to whoever
owns the reflector cache, not to that lane". This document establishes the
mechanism with receipts and lays out the fix options; choosing one is Mark's.

> **Index debt.** DOC_POLICY §6 requires a `DOC_README.md` entry in the same
> session. This lane was read-only against the tracked tree except for this one
> file, and `DOC_README.md` was already dirty under another agent, so the entry
> is **owed, not written**. Suggested line: *"Reflector identity and
> wrapper-held DOM state — why a node's JS wrapper (and its listeners) can be
> silently replaced at a GC tick."*

Related: [`2026-09-07_worker_plan.md`](2026-09-07_worker_plan.md) §"One
unexplained interaction"; `docs/2026-06-11_gc_arena_dom_plan.md` (G1–G3, the
reflector-liveness design this examines).

---

## 1. What exists

Three caches sit between a host `NodeId` and the object script touches.

**The pin table (host, strong).** Every place a binding hands script a node goes
through `dom::reflect_pinned` (`components/script-runtime-api/dom/mod.rs`),
which pins the `NodeId` in `HostState::pins` and then calls
`CallCx::reflector_for`. Pin-on-mint is complete — there is no `make_reflector`
(unpinned) handoff path in the DOM surface. Verified by reading every
`reflector_for` / `make_reflector` call site in the crate: `reflect_pinned` is
the only one.

**The canonical-reflector cache (engine, weak).** `reflector_for`
(`components/script-engine-boa/lib.rs`, `components/script-engine-nova/lib.rs`)
keeps `NodeId → weak(reflector object)`. A hit whose weak still upgrades returns
the same JS object, which is what makes `document.body === document.body` hold.
A dead weak falls through to a fresh mint. `drain_dead_reflectors` sweeps the
dead entries and reports their ids so `Runtime::collect_garbage`
(`components/script-runtime-api/lib.rs`) can unpin the nodes — G1.

**The wrapper cache (JS, weak).** `bootstrap.js` keeps
`wrappers = new WeakMap()` from **reflector object → wrapper object**, and
`wrapNode(ref)` consults it. The comment above it is explicit that a strong
`Map` was tried and rejected: it rooted every reflector for the realm's life and
defeated G1–G3 (the gc-arena soak measured ~12k live nodes under churn).

The wrapper is a plain `Object.create(proto)` carrying `__ref` (a **strong**
reference to its reflector) plus **all** of the node's JS-side state as own
properties: `__listeners` first among them.

So the reachability shape is:

```
script local ──► wrapper ──(__ref, strong)──► reflector ◄──(weak)── engine cache
                    ▲                            │
                    └────(WeakMap value)─────────┘   key = the reflector
```

Nothing else roots either object. **The host pin pins the node, not the
wrapper.** That asymmetry is the whole finding.

---

## 2. The evidence

All runs in a detached worktree of `54f5d163555`, debug profile, Windows.

### 2.1 The Worker plan's reproducer, reproduced

`harness::tests::test_driver_action_sequence_synthesizes_a_click`
(`ports/genet-wpt/src/harness.rs`), engine Boa:

| Tree | Result |
|---|---|
| HEAD as committed | `test result: ok` (2.01s) |
| HEAD + `ErrorEvent.prototype[Symbol.toStringTag] = 'ErrorEvent'` | `FAILED … Some("Test timed out")` |

That is the one line `components/script-runtime-api/worker.rs` currently carries
a comment declining to add.

### 2.2 Instrumenting the engine cache

`reflector_for` was made to print `hit` / `MISS id=… entry_present=<bool>`, and
`drain_dead_reflectors` to print `DEAD id=…` (scratch patch, not landed).
`entry_present=true` on a MISS is the signature of the failure: the cache **had**
an entry for that node and the weak no longer upgraded — the reflector object was
collected and a second one is about to be minted.

Same test, same build, same trace point in all three runs (the node ids are
stable across runs; `…665` is `div#target`, `…664` / `…658` its ancestors):

| Run | stale re-mints (`entry_present=true`) | outcome |
|---|---|---|
| HEAD | `…664`, `…658` | pass |
| HEAD + `toStringTag` | `…665`, `…664`, `…658` | **timeout** |
| HEAD + `toStringTag` + wrapper pin (§5 option B) | `…664`, `…658` | pass |

Boa's collector fires on its own allocation threshold — the WPT harness never
calls `collect_garbage`, and there are **zero** `DEAD` lines in any of these
runs, so `drain_dead_reflectors` is not involved. What the extra allocation
changes is *which* objects are unreachable at the instant the automatic
collection lands. In the baseline, `div#target`'s wrapper happens to still be
reachable; one `Symbol.toStringTag` later, it is not. Its `__listeners` go with
it, `__dispatchSynthetic` re-mints an empty wrapper, and the click reaches
nobody — exactly the symptom the Worker plan recorded.

The blast radius is not the ordering: ids `…664` and `…658` are re-minted in
**every** run, including the two that pass. The bug is firing constantly and
being seen only when it happens to take a node that mattered.

### 2.3 A minimal reproducer with no harness and no `toStringTag`

Added to `components/script-runtime-api/dom/tests.rs` (compile-ready; this is
the regression test §7 asks a fix lane to carry):

```rust
fn listener_survives_gc<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.eval(
        "var d = document.createElement('div');\
         d.setAttribute('id','t');\
         document.appendChild(d);\
         d = null;\
         globalThis.hits = 0;\
         document.getElementById('t').addEventListener('click', function(){ globalThis.hits++; });",
    )
    .expect("register");
    rt.run_microtasks();
    let _ = rt.collect_garbage();
    rt.eval(
        "document.getElementById('t').dispatchEvent(new Event('click'));\
         console.log('hits:' + String(globalThis.hits));",
    )
    .expect("dispatch");
    assert_eq!(rt.host().borrow().console[0], "hits:1");
}
```

Result today, **on both engines**:

```
laneC listener probe: ["registered:true", …, "after-gc:false", "hits:0", …]
collect_garbage -> (reflectors_unpinned, nodes_collected) = (1, 0)
FAILED  dom::tests::lane_c_listener_survives_gc_on_boa
FAILED  dom::tests::lane_c_listener_survives_gc_on_nova
```

Read the numbers. `reflectors_unpinned = 1`: the engine reported the div's
reflector dead and the host unpinned the node. `nodes_collected = 0`: the node
itself is **fine** — it is attached to the document, layout still sees it, it
still renders. Only its JavaScript identity and everything script hung on that
identity is gone. `after-gc:false` is `__listeners` being `undefined` on the
replacement wrapper. The engine trace for the same run:

```
[refl] MISS id=…657 entry_present=false      ← first mint (createElement)
[refl] hit  id=…657                           ← getElementById, addEventListener
[refl] DEAD id=…657                           ← the GC tick
[refl] MISS id=…657 entry_present=false      ← re-mint, empty wrapper
```

### 2.4 Nova shows the same defect, on a different trigger

Nova fails the §2.3 probe identically. The difference is only *when* a
collection happens: Nova's heap is collected when the host calls
`ScriptEngine::force_gc` (`agent.gc()` twice), and Boa's `boa_gc` additionally
self-collects on an allocation threshold. The WPT harness never calls
`collect_garbage`, so Nova never trips there — which is why the Worker plan saw
this as Boa-only. It is not.

**In the real headed host both engines trip it every frame.**
`ScriptedDocument::pump` (`components/genet-scripted/document.rs`) ends every
frame with `self.rt.collect_garbage()`.

### 2.5 Two wrappers do **not** coexist

Worth stating precisely, because it narrows what a fix must do. A second probe
holds a wrapper in a JS global, takes three GC ticks, and compares:

```
laneC identity probe: ["same:true", "mark:1"]   (Boa and Nova, both ok)
```

While script holds wrapper A, `A.__ref` roots reflector A, the weak cache
upgrades, and `wrapNode` hits. No path mints a second reflector for a node whose
first one is alive — the four candidate mechanisms in the brief resolve as:

| Candidate | Verdict |
|---|---|
| JS cache keyed on something other than a canonical reflector | **No.** Keyed on exactly the reflector `reflector_for` returns. |
| Engine returns a non-canonical reflector on some path | **No.** `reflect_pinned` is the sole handoff; no unpinned `make_reflector` path exists in the DOM surface. |
| Weak entry collected and re-minted while a strong wrapper still exists | **No** — ruled out by §2.5. |
| GC timing drops a cache entry still reachable through JS | **No.** The entry dropped was genuinely unreachable *from JS*. |

The real mechanism is a fifth one, and it is a design gap rather than a cache
bug — see below.

---

## 3. The mechanism

**A node's JS wrapper is not kept alive by the node. It is kept alive only by
script's own strong references to it. When the last one goes, the (reflector,
wrapper) pair becomes an unreachable cycle and is collected — even though the
node is attached to a live document — and the next handoff mints a fresh, blank
wrapper in its place.**

Every browser engine has the opposite rule: a node in a document is a GC root,
and its wrapper is reachable *from the node*, so wrapper-held state survives for
as long as the node does. Genet's G1 design inverted that edge deliberately, to
get node collection out of a weak reflector cache — and the inversion is sound
for *node lifetime*. What was not noticed is that the same edge also governs
*wrapper state lifetime*, and there the inversion is simply wrong: script has no
way to say "this node matters" other than holding the wrapper itself, which is
exactly what `addEventListener` semantics promise it need not do.

The caches are all behaving as specified. The specification is missing a rule.

---

## 4. Blast radius

Every piece of DOM state that lives on the wrapper is lost when the wrapper is
replaced. From a sweep of `dom/bootstrap.js` — own-property expandos and
`WeakMap`s keyed by the wrapper:

| State | Where | Rooted today? | What breaks |
|---|---|---|---|
| `__listeners` | `Node.prototype.addEventListener` | **no** | listeners silently stop firing. Proven, §2.3. |
| `__handlers` / `__handlerWrappers` | `on*` IDL attribute pairs | **no** | `el.onclick = f` stops firing; the getter starts returning `null`. |
| `upgradedCustomElements`, `connectedCustomElements` | custom-element upgrade state | **no** | worse than loss: `wrapNode` calls `upgradeCustomElement` on every mint, and the guard is `upgradedCustomElements.get(el) === def`. A replacement wrapper **re-runs the element's constructor** and can fire `connectedCallback` twice. |
| `iframeDocuments`, `iframeWindows` | `contentDocument` / `contentWindow` | **no** | a second child document/window is minted for the same frame; the first one's state is orphaned. |
| `__webglContext` | `HTMLCanvasElement.getContext` | **no** | `getContext()` twice returns two different contexts for one canvas. |
| `namedNodeMaps`, `attrViews` | `el.attributes`, `Attr` nodes | **no** | `el.attributes` and `Attr` identity break across a tick. |
| `inlineStyleStates` | `el.style` | **no** | the cached `CSSStyleDeclaration` and any state on it. |
| `ownerDocuments` | detached node → owning document | **no** | `ownerDocument` silently reverts to the primary `document`. |
| `__moRegs` | MutationObserver registrations | incidental | `moAddRegistration` pushes the wrapper into `observer.__nodes`, so a *reachable* observer roots it. Drop the observer and the registrations vanish while the raw-id census `moRegisteredIds` / `moRegistrationCount` keeps counting them — a leak and a stuck `moActive`. |
| `__sc` / `__ec` on `Range` | boundary containers | yes | the `Range` holds its boundary wrappers strongly, and `rangeIndex` is keyed on the **raw node id**, so ranges are correct. |
| Selection | `selectionRange` module-level | yes | rooted through the live `Range`. |

`document` and `window` are strongly rooted globals, so **document-level**
listeners are safe. The exposure is elements — which is where nearly all page
script lives.

Two aggravating facts:

- **The WPT harness never calls `collect_garbage`**, so every stale weak entry
  it accumulates is also a permanent pin: nodes stay pinned for the run. The
  harness gets the state loss (Boa's automatic collector) without the reclamation
  the loss was supposed to buy.
- **The failure is silent.** No exception, no console output, no test error —
  a listener simply never fires. Anything in the WPT corpus that registers a
  listener without holding the element is failing this way today, attributed to
  whatever feature it was testing.

---

## 5. Fix options

**A — Root the wrapper from the host pin (engine cache goes strong-while-pinned).**
Make `reflector_for` hold the reflector strongly while the host pin is held, and
release it when the pin is retired. Restores browser semantics exactly (node
alive ⇒ wrapper alive). Cost: needs a host→engine "unpin this reflector" call so
the strong root can be dropped, which the current one-way
`drain_dead_reflectors` contract does not have; and pins are only retired *by*
the drain, so the two would need untangling. Reintroduces the rooting the
gc-arena soak rejected — unless the pin set itself is made to shrink, which is
what G3 was for. Biggest change, correct end state.

**B — Root the wrapper on first expando write (JS-side pin).**
`bootstrap.js` keeps a strong `rawId → wrapper` table and inserts on the first
`addEventListener` / `on*` set / `getContext` / `attachShadow`-shaped operation.
**Validated:** the one-line form (pin at `addEventListener`) makes the
`toStringTag` reproducer pass while the other two nodes are still re-minted
(§2.2, row 3). Cheap and immediately effective; the cost is that a pinned
wrapper is never released — a page that adds a listener to a node it later
discards leaks the wrapper, the reflector, and (through the pin) the node, for
the realm's life. Wants a release hook on node removal to be more than a
stopgap.

**C — Move wrapper state off the wrapper, into raw-id-keyed host or JS tables.**
Listeners, handlers, custom-element state and the rest keyed by `NodeId`, so a
replacement wrapper inherits them. Fixes the *state* loss without fixing the
*identity* loss: `el === el` across a tick still fails, and `Range`-style
`===` comparisons against a stale wrapper still misbehave. Also the largest
bootstrap edit of the three. It does have one real advantage: the tables can be
swept by the same host-side node collection that already exists, so it does not
leak.

**D — Do nothing; document the hazard.** Not recommended, but honest as an
interim: the defect is already live, and naming it beats the current situation
where it is attributed to unrelated lanes.

**Not an option:** tuning the GC cadence, disabling Boa's automatic collection,
or avoiding allocations near the hot path. §2.2 shows the collection is firing
constantly and correctly; the bug is what it is allowed to take.

---

## 6. Decisions that are Mark's

1. **Which option**, and whether the answer is a staged A-after-B (ship the pin
   now, do the rooting properly later) or a single pass.
2. **Whether wrapper rooting is allowed to hold nodes alive.** A and B both
   pin the node as a side effect of pinning the wrapper. That is browser-correct
   and it directly contradicts the gc-arena soak's bounded-live-node target. One
   of the two contracts has to give; which one is a product call, not a
   technical one.
3. **Whether `Runtime::collect_garbage` should fire in the WPT harness.** It
   does not today. Turning it on makes Nova reproduce the same failures Boa
   already has, which is arguably the honest configuration and is certainly a
   worse-looking scoreboard.
4. **Whether the double custom-element upgrade is scoped into this lane or
   split.** It is the same root cause but a distinct observable (a constructor
   running twice), and it may deserve its own WPT receipt.
5. **Whether this lane owns the `ErrorEvent[Symbol.toStringTag]` restoration.**
   The line is a one-word change once the wrapper is stable, and it is the
   cleanest single receipt that the fix landed.

---

## 7. Done-conditions for a fix lane

- `dom::tests::lane_c_listener_survives_gc_on_boa` and `…_on_nova` (§2.3, verbatim)
  pass. **They fail today on both engines** — that is the regression test.
- `harness::tests::test_driver_action_sequence_synthesizes_a_click` passes with
  `ErrorEvent.prototype[Symbol.toStringTag] = 'ErrorEvent'` restored in
  `components/script-runtime-api/worker.rs`, and that line is restored, with the
  "avoided, not explained" comment deleted.
- A second regression test covering one non-listener column of §4 — the
  custom-element double upgrade is the strongest, since it is an *extra* effect
  rather than a missing one and cannot be papered over by re-registration.
- An engine-cache assertion, not just a behavioural one: a test that drives a GC
  tick and asserts no `reflector_for` MISS with a live host pin for that id.
  (§2.2's `entry_present=true` line is that condition, observed rather than
  asserted; a fix lane should make it an invariant — see
  `feedback_diagnostics_assert_invariants`.)
- The gc-arena soak still bounds live nodes under churn, or the plan records the
  new bound and Mark has signed off on decision (2).
- `DOC_README.md` indexes this document and the fix plan.

---

## Findings

- **2026-09-07** — `reflect_pinned` is the sole node-handoff path; no unpinned
  `make_reflector` path exists in `components/script-runtime-api/dom/`.
  (Read of every call site.)
- **2026-09-07** — Two wrappers for one node never coexist while script holds
  one; the failure is destruction-and-replacement. Boa and Nova, §2.5.
- **2026-09-07** — The defect is engine-independent. Nova fails the §2.3 probe
  identically; only the collection *trigger* differs (Boa self-collects on an
  allocation threshold, Nova collects when the host asks).
- **2026-09-07** — `ScriptedDocument::pump` calls `collect_garbage` every frame,
  so the headed host is exposed continuously on both engines. The WPT harness
  never calls it, so it gets Boa's automatic collections without the unpinning.
- **2026-09-07** — In the passing baseline of the reproducer, two nodes' wrappers
  are *still* silently replaced. The `toStringTag` line does not create the bug;
  it moves which node the bug takes.
