# Reflector identity — wrapper liveness follows node reachability

**Status (2026-09-07): landed.** Implements the fix
[`2026-09-07_reflector_identity_scoping.md`](2026-09-07_reflector_identity_scoping.md)
scoped and Mark decided. Base commit `c30cc3571d6`.

Related: the scoping document (mechanism, receipts, options A–D);
[`2026-09-07_worker_plan.md`](2026-09-07_worker_plan.md) §"One unexplained
interaction" (the `ErrorEvent[Symbol.toStringTag]` receipt this lane restores);
`docs/2026-06-11_gc_arena_dom_plan.md` (G1–G3, and the soak target this lane
restates).

---

## 1. The design

Mark's ruling, in one line: **wrapper liveness follows node reachability, not
wrapper state.**

WebIDL requires exactly one JavaScript object per platform object per realm, so
the identity of any reachable node's wrapper must be stable across collections
— expandos or not. `getElementById(x) === saved` has to hold, and a user
`WeakMap` keyed on an element has to keep finding its entry. That rules out the
scoping document's option B (pin on the first expando write): pinning on *state*
fixes the columns of §4 one at a time and never fixes identity, and it leaks,
because a pin taken on a write has no natural release.

So every wrapper has an **opaque root**: the root of its node's tree. A wrapper
is alive while its opaque root is alive, and a document is always alive.
Therefore:

- a **connected** node's wrapper lives as long as its document;
- a **detached** subtree's wrappers live as long as script holds any one of them;
- a node script never touched has no wrapper, so nothing is kept for it.

The second and third clauses are what keep the gc-arena soak meaningful. A node
removed from the document whose subtree has no live wrapper is collected on the
next tick, exactly as before.

### 1.1 One design decision inside that, which is Mark's to overrule

The two clauses are enforced by different mechanisms, and the reason is not
taste — one of them is not a question the host can answer.

"Connected" is decided by the arena. The host reads the tree root, compares it
to the document node, and takes a strong engine root. No liveness question
arises.

"Some member of this detached tree is still held by script" is a liveness
question, and **a host-side strong root destroys the evidence for it**: the
moment the host roots the tree's reflectors, every one of them is reachable, and
no subsequent query — `drain_dead_reflectors`, a weak upgrade, a count — can
distinguish "script still holds one" from "the host is holding them all". The
literal instruction ("root it if its tree root is the tree root of any reflector
that is currently alive") is therefore self-satisfying after the first tick: a
detached tree, once rooted, would never be released. Releasing the roots and
collecting to find out is not a way round it either — that collection is the one
that takes the sibling wrapper whose identity the policy exists to preserve.

Only the collector can answer it, so the question is handed to the collector, in
the one form every tracing GC already understands: a **reference cycle**. Each
detached tree gets one array holding every member wrapper strongly, and each
member is a `WeakMap` key mapping to that array. A `WeakMap` entry is an
ephemeron — the value is reachable exactly while the key is — so any reachable
member keeps the array alive, the array keeps every sibling alive, and when the
last member goes the whole group is unreachable and is collected together. That
is the opaque-root rule *resolved* rather than approximated, and it needs no
liveness query at all.

The host still owns the policy: it computes every tree root from the arena at
each tick and tells the bootstrap what changed. The deviation is only in *how*
the second clause is enforced. It is recorded here rather than taken silently.

---

## 2. What changed

### 2.1 The engine contract (`components/script-engine-api/lib.rs`)

Four additions on `ScriptEngine`, two on `CallCx`. All defaulted, so a backend
without rooting (piccolo, the epoch-pin fallback) compiles unchanged.

| Method | Surface | What it does |
|---|---|---|
| `minted_reflectors()` | `ScriptEngine` | every `ReflectorData` the canonical cache has an entry for — the set the policy iterates |
| `root_reflectors(&[…])` | `ScriptEngine` | take a strong engine root on each, minting through the canonical cache if the weak has died |
| `unroot_reflectors(&[…])` | `ScriptEngine` | release those roots |
| `rooted_reflector_count()` | `ScriptEngine` | diagnostic readout; the soak's bound |
| `root_reflector(data) -> bool` | `CallCx` | the in-callback single-id form, for the mint path |
| `unroot_reflector(data)` | `CallCx` | its release |

`drain_dead_reflectors` is unchanged and remains the liveness signal for
*unrooted* reflectors; a rooted id can never appear in it, because its strong
root keeps the weak upgradeable.

Boa holds the roots in a traced `GcRefCell<HashMap<u64, JsObject>>` beside the
weak cache in `HostCell`. Nova holds a `Global` of the reflector itself (not of
a `WeakRef` to it) in `NovaHostSlot::roots`, and `take`s it on release so the
`agent.heap.globals` slot is freed rather than leaked. Roughly half of the
engine half is the saved partial patch from the stopped attempt, reused as-is;
its `__pinWrapper` / `__unpinWrapper` JS-callable half is *not* reused, because
that was the expando-write policy this one supersedes.

### 2.2 The arena (`components/genet-scripted-dom/lib.rs`)

- `structure_epoch()` — a monotonic counter bumped at every one of the six
  writes to a node's `parent` link. Deliberately narrower than the `DomMutation`
  sequence number, which also advances on attribute and character-data facts
  that cannot move a node between trees.
- `tree_root(id)` — the topmost ancestor by parent links, or `None` for a dead
  id.

### 2.3 The host policy (`components/script-runtime-api/`)

`HostState` gains a tree-root cache (`tree_roots`, `tree_root_epoch`) reached
through `tree_root_of`, which drops the whole cache when the epoch moves — a
single re-parent can change the answer for an unbounded number of nodes, and a
per-entry invalidation would have to walk to find out which. `is_connected_node`
is `tree_root_of(id) == document`.

`Runtime::apply_opaque_root_policy` runs at the head of `collect_garbage`,
before `force_gc`, because it is what decides which wrappers the collection is
allowed to take. It classifies every minted reflector, roots the connected ones,
unroots the rest, and sends the bootstrap only the groups whose membership
changed plus the ids that have just become connected (which must leave their old
group, or a now-immortal connected wrapper would hold the group array and keep a
removed subtree alive for the document's life). In the steady state both
arguments are empty.

Two places root outside the tick, and both exist for the same reason — Boa
collects on its own allocation threshold, so a wrapper handed out and decorated
between two ticks would otherwise be collectable in that window:

- `dom::reflect_pinned` roots on the mint when the node is already connected;
- `dom::root_connected_subtree`, called from `__appendChild`, `__insertBefore`
  and `__moveBefore`, roots every already-reflected node in a subtree that has
  just become connected. The host pin table is the list of nodes that have a
  reflector, so the walk roots exactly those and skips freshly parsed subtrees.

The tick releases both again if the node has since left the document. That
asymmetry is also the one bug this lane wrote and caught: unrooting only what
*the tick* had rooted left every node created-and-removed between two ticks
permanently rooted, and the soak's memory direction failed at 12,003 live nodes.
The tick now unroots every detached minted reflector, not only its own records.

### 2.4 The bootstrap (`dom/bootstrap.js`)

`wrapperGroups`, a `WeakMap` from wrapper to its detached tree's member array,
and `globalThis.__gcPolicy(clear, spec)`, the tick's entry point. Nothing else
in the bootstrap changed: there was no state-based liveness workaround in it to
remove — the one workaround in the tree was the *absence* of
`ErrorEvent.prototype[Symbol.toStringTag]`, restored below.

`wrapNode` is unchanged and still cannot re-run a constructor for a node that
already has a wrapper (`wrappers.has(ref)` returns first). What changed is that
a connected element can no longer *reach* that path, because its wrapper is
rooted from the mint or the insertion onward.

### 2.5 The `ErrorEvent` receipt (`components/script-runtime-api/worker.rs`)

`ErrorEvent.prototype[Symbol.toStringTag] = 'ErrorEvent'` restored and the
"avoided, not explained" comment deleted.
`harness::tests::test_driver_action_sequence_synthesizes_a_click` passes with it
in place — the cleanest single receipt that the fix landed, since that line is
what tipped the collection onto `div#target` in the scoping document's §2.2.

### 2.6 The harness (`ports/genet-wpt/`)

A GC tick inside each testharness test's drive loop (both the virtual disk loop
and the wall-clock server loop), **on by default**, cleared by
`--no-harness-gc`, forwarded to the per-test worker subprocesses. Without it
Nova never collected during a test while Boa's collector fired anyway, so the
two engines were not being asked the same question, and the headed host — which
collects at the end of every frame in `ScriptedDocument::pump` — matched
neither.

The cadence is **every drive turn**, and that is a measured choice, not a taste:
see Findings, 2026-09-07 "a cadence that never fires".

---

## 3. Measured cost

`dom::tests::opaque_root_policy_cost_is_bounded`, release profile, Windows, a
document with 4,002 touched (connected, reflected) nodes. The policy is timed
*alone*, not through `collect_garbage`, because `force_gc` is a full heap mark on
both engines and dominates the tick by one to two orders of magnitude.

| | Boa | Nova |
|---|---:|---:|
| policy, frame that re-parented nothing | **528 µs** | **504 µs** |
| policy, frame that re-parented one node | **2,920 µs** | **2,034 µs** |
| one whole `collect_garbage` (for scale) | 16,241 µs | 107,488 µs |

Read it as: on a quiescent frame the policy costs about half a millisecond over
four thousand touched nodes — one `minted_reflectors` walk plus a hash lookup
each, the tree-root cache holding — which is 3% of a Boa tick and 0.5% of a Nova
one. A frame that moves a single node drops the cache and re-walks every
reflector's parent chain, and costs about five times that, still under 3 ms and
still under a fifth of the Boa tick it sits inside. The cheap case is the common
one: the epoch is bumped only by writes to a parent link, never by attribute or
text mutation.

The whole-directory wall time in the WPT census (§4) moved by less than the
run-to-run noise: 569 s with the harness collection on against 565 s with it
off, over the same six directories, with individual directories moving both ways
by up to 40%.

---

## 4. The census — before and after, under both harness settings

`genet-wpt testharness`, Boa, Livery, disk mode, `--jobs 8 --timeout 90`. Raw
maps under
`Code/testing/genet/wpt-ledger/2026-09-07_reflector_identity/` (`pre/`, `post/`,
`post_gcoff/`, plus `nova_control/`).

| runner | SHA-256 |
|---|---|
| `pre` (HEAD `c30cc3571d6`, before the first edit) | `f6a2ad192fd660a6c7d840a52d6bfad2a1ef2580a3e1027be68c83a7d26a40e9` |
| `post` (this lane) | `42edb7ac1e8a1f54dcace7d79abd04dd2ec2c56d835fd028182905584a86177e` |

| directory | files | pre | post, collection **on** | post, collection **off** |
|---|---:|---|---|---|
| `dom` | 698 | 218 all-pass, 44,033/54,222 | **identical** | identical |
| `custom-elements` | 187 | 7 all-pass, 2,126/3,832 | **identical** | identical |
| `html/webappapis` | 356 | 45 all-pass, 977/1,708 | **identical** | identical |
| `workers` | 294 | 76 all-pass, 325/967 | **identical** | 76 all-pass, **326**/967 |
| `selection` | 161 | 31 all-pass, 30,573/33,789 | **identical** | identical |
| `html/semantics/interfaces.html` | 1 | 1 all-pass, 438/438 | **identical** | identical |

**`post` against `pre` is byte-identical**: zero file status movements, zero
subtest delta, in all six directories. Zero pass-to-fail, and zero
fail-to-pass — the lane's census effect is exactly nil.

The one movement anywhere is in the collection-**off** map, and it is explained
rather than fixed: `workers/modules/dedicated-worker-options-type.html` scores
one more subtest pass with collection off, because its "classic" worker-type
subtest happened to beat the drive deadline instead of timing out, which then
let the "module" subtest run and fail. Re-run in isolation at `--jobs 1`, that
file scores 1/5 on **three consecutive runs under each setting**, i.e. the
subtest passes both ways. It is `--jobs 8` scheduling contention on a
wall-clocked worker test, not a policy or cadence effect.

A Nova control was taken over `custom-elements` and `html/webappapis` under both
settings (`nova_control/`): 8 all-pass / 2,132 of 3,847 and 32 all-pass / 495 of
1,117, identical under both. So the setting moves nothing on either engine over
these directories.

**Baselines.** `support/wpt/check-testharness-baselines.ps1` reports
`unexpected=0` for the whole checked set, including every baseline the six
directories touch (`dom_boa`, `dom_abort_boa`, `h4_opt_in_dom_abort_boa`,
`dom_nodes_boa`, `html_webappapis_timers_boa`). Nothing moved, so **no baseline
was repinned**.

**Ortet.** `cargo run -p ortet -- --url ports/ortet/examples/article.html
--frames 3 --artifact C:/t/laneR-ortet.png` — 3 frames at 960x640, digest
`0x6377ba8a6bf4dbc9`, matching the digest on record in the Ortet founding plan
and the DOM node-model plan. The per-frame collection now runs the policy, and
the frame is unchanged.

### Why zero is the honest number

The scoping document did not predict a score movement; it predicted that
"anything in the WPT corpus that registers a listener without holding the
element is failing this way today, attributed to whatever feature it was
testing". Zero movement says that over these six directories, nothing in the
corpus was in that position on Boa — the corpus mostly holds its elements. What
the lane bought is not a score: it is that a whole class of silent, timing-
dependent wrongness can no longer happen, that `ErrorEvent`'s `toStringTag` can
be restored without a timeout, and that Nova is now collected during a test the
way the headed host is.

---

## 5. Regression manifest

Every test below is run on **both** engines (`_on_boa` / `_on_nova`). The ten
marked ✗ were confirmed to fail before the policy, by running the suite with the
policy switched off in the same build — a positive control, so the null result
from the two that pass either way is readable.

| Test (`script_runtime_api::dom::tests`) | What it pins | Fails without the policy |
|---|---|---|
| `listener_survives_gc` | the scoping document's §2.3 reproducer, verbatim: register a listener without holding the element, collect, dispatch | ✗ both engines |
| `connected_wrapper_identity_survives_gc` | `getElementById` twice with a collection between, `===`, plus an expando; and that a root was actually taken | ✗ both engines |
| `user_weakmap_key_survives_gc` | a user `WeakMap` keyed on a connected element still finds its entry after a collection | ✗ both engines |
| `detached_subtree_keeps_sibling_identity` | a detached subtree held by one wrapper keeps its siblings' identity and expandos across two collections | ✗ both engines |
| `custom_element_upgrade_is_not_repeated` | a connected custom element is not upgraded twice across a collection; a detached one held by script keeps identity and constructor state | ✗ both engines |
| `removed_unreferenced_subtree_is_reclaimed` | the other direction: a removed, unreferenced subtree is reclaimed, and collection is idempotent | passes either way (it is the guard that the fix did not break reclamation) |
| `gc_soak_bounds_reachable_touched_nodes` | the restated soak, both directions in one test (§6) | passes either way on memory; ✗ on identity |
| `opaque_root_policy_cost_is_bounded` | the §3 measurement, with a loose ceiling against a regression into quadratic per-tick work | n/a |

Plus, in `genet-wpt`:
`harness::tests::test_driver_action_sequence_synthesizes_a_click` with
`ErrorEvent.prototype[Symbol.toStringTag]` restored — the scoping document's
§2.1 receipt, which timed out on HEAD with that line and passes now.

---

## 6. The soak, restated

`docs/2026-06-11_gc_arena_dom_plan.md` G3's target was "bounded live nodes under
churn". It is now **bounded by reachable touched nodes**, and the plan carries
that restatement. The bound is not weaker; the restatement names what it is made
of.

The soak asserts **both directions in one test**, because either alone passes
for the wrong reason — a fix that kept everything would pass a memory bound only
by accident of the run length, and a fix that kept nothing would pass an
identity check only until the first collection:

- 60 turns x 100 nodes = 6,000 nodes created connected and then removed, at
  frame cadence through `collect_garbage`. Peak live nodes and peak engine roots
  both stay under 1,000.
- A connected node held by nobody keeps its wrapper, its expando **and its
  listener** across all 60 collections.

**The soak that was already in the tree does not run.** `gc_soak_bounds_memory`
lives in `components/genet-scripted/document.rs`, whose entire `mod tests` is
gated on `feature = "render"` — a feature no crate in the workspace enables, and
which the module no longer compiles under when it is enabled (it references
`RefCell`, `IncrementalLayout` and `ComputedStyleHandler` that are not in scope
there). That is 44 `cfg(feature = "render")` sites in `document.rs` alone.
Reviving it is its own lane; it is named here, and in the gc-arena plan, rather
than quietly worked around, and it is why the restated soak was written in
`script-runtime-api`, where it runs on both engines.

---

## 7. Gates

| Gate | Result |
|---|---|
| `cargo test` on every touched crate, both engines | green — `script-engine-api`, `script-engine-boa`, `script-engine-nova`, `genet-scripted-dom`, `script-runtime-api` (141 lib + 9 integration suites), `genet-scripted`, `genet-wpt` (63 + 3 ignored) |
| clippy on the touched crates, `--all-targets` | no new warnings; the ones that fire (`cloned_ref_to_slice_refs` in boa's promise settle, `needless_borrow` in nova, `type_complexity` on `eval_module`, `items_after_test_module` in `harness.rs`) all pre-date this lane |
| rustfmt on touched files | clean |
| `cargo check --workspace --features genet-wpt/netfetch` | green |
| soak under the restated target | green, both engines, both directions |
| WPT census, pre/post x (collection on, off) | §4 — zero movement, every difference explained |
| checked testharness baselines | `unexpected=0`, no repins |
| Ortet receipt | digest `0x6377ba8a6bf4dbc9`, unchanged |

---

## Findings

- **2026-09-07 — a host cannot ask whether script still holds a wrapper it is
  itself rooting.** The strong root is the answer's own confounder, and there is
  no order of unroot / collect / query that recovers it without the collection
  taking the wrapper the question was about. The fix is not a better query: it
  is to stop asking, and to express the liveness *relation* as a reference cycle
  the collector already resolves. A `WeakMap` from each member to a shared array
  of all members is that cycle, in plain JS, on both engines.

- **2026-09-07 — a cadence that never fires is a flag that does nothing.** The
  harness GC tick was first written as "every 30 drive turns". Counted, it fired
  **zero** times across the whole of `custom-elements/reactions` — a testharness
  test's drive loop is only a handful of turns long — and the six-directory
  census under both settings came back byte-identical, which read as a clean
  null result and was in fact an inert flag. Instrumented, one turn per tick
  fires 171 times over that directory at a 9% wall cost (14.9 s to 16.2 s) for
  the same 49/534 subtests. **Count the invocations before believing a null
  result from a switch you just added.**

- **2026-09-07 — root-on-mint is not enough; the insertion path has to root
  too.** The common shape is create detached, decorate, insert — so the node is
  *not* connected at the moment script is handed it, and Boa's
  allocation-threshold collector can fire between the insertion and the next
  tick. The soak caught this as a lost listener on a node that had been
  connected for sixty frames. `root_connected_subtree` at the three insertion
  natives closes it, and costs nothing on parsed content because it roots only
  ids the host pin table already holds.

- **2026-09-07 — releasing only what you took leaks what someone else took.**
  The tick originally unrooted `previously-rooted ∖ now-connected`. Nodes rooted
  by the *mint* path and removed before the first tick that saw them were never
  in that set, so they were rooted forever: 12,003 live nodes in a soak whose
  bound is 1,000. The tick now unroots every detached minted reflector
  unconditionally; unrooting is a hash remove, and the set is small.

- **2026-09-07 — `genet-scripted`'s test module is dormant and rotted.** 44
  `cfg(feature = "render")` sites in `document.rs`, including the whole
  `mod tests` that holds the gc-arena soak and the node-identity guard, behind a
  feature nothing enables. Enabling it does not compile. Not touched by this
  lane beyond recording it (§6).

- **2026-09-07 — the lane's WPT effect is nil, and that is the finding.** Six
  directories, 1,697 files, identical to the byte before and after, under both
  harness-collection settings, on both engines where controlled. The corpus
  mostly holds its elements, so it was mostly not exposed to the defect. The
  receipt for the fix is the reproducer set and the restored `toStringTag`, not
  a score.

---

## Progress

- **2026-09-07** — Landed. Engine contract on Boa and Nova (reusing the saved
  partial patch's engine half), the opaque-root policy in the host with the
  arena's structural epoch behind the tree-root cache, ephemeron groups for
  detached trees in the bootstrap, harness collection on by default at one tick
  per drive turn, `ErrorEvent`'s `Symbol.toStringTag` restored, seven regression
  functions x two engines, the restated two-directional soak, and the cost
  measurement. Census: zero movement across `dom`, `custom-elements`,
  `html/webappapis`, `workers`, `selection` and `html/semantics/interfaces.html`
  under both settings; baselines `unexpected=0`; Ortet digest unchanged.

## Open

- **The between-tick window for detached trees.** Groups are rebuilt at the
  tick, not at the mutation. A detached subtree whose members are first wrapped
  *after* a tick is ungrouped until the next one, so Boa's automatic collector
  can still take a sibling in that window. Connected nodes have no such window
  (mint and insertion both root). Closing it means joining a group in `wrapNode`
  by walking to the tree root, which needs a non-minting parent-id native and a
  non-minting reflector lookup — deliberately not added here.
- **Groups are rebuilt whole per dirty tree**, so a detached tree that gains one
  member re-links all of them. Fine at the sizes detached trees actually reach;
  worth an index if a workload ever shows otherwise.
- **`ScriptedDocument`'s dormant test module** (§6) — its own lane.
- **The scoping document's `iframeDocuments` / `__webglContext` columns** are
  fixed by construction for connected nodes but have no dedicated regression
  test; the manifest covers listeners, expandos, user `WeakMap` keys and
  custom-element upgrade state.
