# MutationObserver: a second consumer of the arena's mutation record

**Status:** landed 2026-09-07.

`genet-scripted-dom`'s arena already records every structural change as a
`DomMutation` for Livery to turn into invalidation. `MutationObserver` is the
same facts, delivered to script. This lane adds the observation registry, the
record queue, the "notify mutation observers" microtask, and the interface-table
declaration — and answers, in code, how two consumers share one mutation point.

Base commit `98762466586` ("Cheap globals: performance and timing,
queueMicrotask, structuredClone, messaging, crypto"). Receipts:
`Code/testing/genet/wpt-ledger/2026-09-07_mutation_observer/`.

## The fan-out decision

The lane's one architectural choice was whether the observer registry taps the
`DomMutation` stream before Livery drains it, or whether the arena fans out at
the mutator into a second typed record. **It fans out at the mutator.**

`DomMutation` cannot answer what `MutationObserver` asks, and could not without
becoming a different type:

| `MutationRecord` needs | `DomMutation` carries |
|---|---|
| `previousSibling` / `nextSibling` around an insertion or removal | nothing — `Inserted`/`Removed` name the node and the parent |
| `characterData` `oldValue` | nothing — `CharacterDataChanged { node }` |
| `addedNodes` / `removedNodes` for `innerHTML` and `textContent` | one `SubtreeReplaced { node }`, deliberately collapsed |
| the target's ancestor chain **as it stood at mutation time** | nothing; the live tree has moved on by delivery |

Widening `DomMutation` to carry all four would change the type every retained
Livery consumer matches on, in a crate this lane is fenced out of, to carry
fields invalidation does not want. Tapping the stream and re-deriving the
missing fields from the live tree is worse still: the sibling either side of a
removal and an attribute's old value are gone by the time any consumer looks,
which is exactly why `AttributeChanged` already carries `old_value`.

So there is one **source of truth per mutation** — the same statement in the
same mutator function writes both views — and two typed projections of it.
`ScriptedDom::record_child_list` / `record_attribute` / `record_character_data`
sit beside the `self.mutations.push(...)` lines they mirror. Livery's stream and
its `drain_mutations` cursor are untouched: the observer view is drained by
`take_observed`, a separate vector, so neither consumer can starve the other.

The record is **off by default**. `set_observing(false)` (the initial state)
makes every `record_*` an early return, so a document nobody observes builds no
records, keeps no removed subtrees alive, and behaves byte-for-byte as before.
The bootstrap flips it on with the first registration and off with the last.

## What the lane added

| Piece | Home |
|---|---|
| `ObservedMutation` (`ChildList` / `Attributes` / `CharacterData`, each with the mutation-time ancestor chain), `set_observing`, `take_observed`, `set_observer_group` | `components/genet-scripted-dom/lib.rs` |
| `__moObserving`, `__moTake`, `__moGroup` — the observing switch, the drain and its line protocol, the coalescing window | `components/script-runtime-api/dom/mutation_observer.rs` |
| The registry, `MutationObserverInit` validation, the interested-observer walk, transient registered observers, `MutationRecord`, `MutationObserver`, the notify microtask | `components/script-runtime-api/dom/bootstrap.js` |
| `__moPump()` at the head of every microtask checkpoint, while observing | `components/script-runtime-api/lib.rs` |
| `MutationObserver` / `MutationRecord` declared by the generated table | `support/idl-interface-table/src/lib.rs`, `dom/html_interfaces_generated.rs` |

Rust carries only what the arena knows and JS cannot recover afterwards. Every
policy decision — which observers are interested, whether an old value is
wanted, when a transient registration dies — is in the bootstrap, where the
registry lives.

## Phases and done-conditions

### Phase 1 — the arena's observer record (landed)

Done when the arena writes a spec-shaped record beside `DomMutation` for every
mutator, at zero cost when unobserved, without disturbing `drain_mutations`.

Met. `remove_child`, `set_text_content`, `append_child`, `insert_before`,
`move_before`, `remove`, `set_attribute`, `remove_attribute`, `set_text` and
`set_inner_html` all write both views;
`observed_record_is_off_until_asked_and_then_spec_shaped` asserts the record is
empty until asked for, that the drained layout batch still has all six of its
mutations, and that ids, siblings and old values are the spec's.

### Phase 2 — the registry and the record queue (landed)

Done when registration, the option defaulting and its four `TypeError`s, the
interested-observer walk over the target's inclusive ancestors, `subtree`,
`attributeFilter`, `attributeOldValue` and `characterDataOldValue` all behave.
Met; see the gates.

### Phase 3 — delivery (landed)

Done when delivery is one microtask, batched per observer, with `takeRecords`,
`disconnect` and transient registered observers for a removed subtree. Met:
`dom/nodes/MutationObserver-sanity.html` 0/13 → **13/13**,
`-takeRecords.html` 0/3 → **3/3**, `-disconnect.html` 0/2 → **2/2**,
`-callback-arguments.html` 0/1 → **1/1**.

### Phase 4 — the interface table and both engines (landed)

Done when `MutationObserver` is declared through the generated table and the
tests run on Boa and Nova. Met: both names left the generator's
`SHAPE_ONLY_DENY` list, the shape pass now finds the bootstrap's real
implementations and only stamps their class strings, and the drift test
regenerates and byte-compares (41 → 43 shape-only interfaces).
`tests/mutation_observer.rs` runs 8 bodies against both backends.

## Findings

### Records are queued at mutation time, through twelve call sites (2026-09-07)

The spec queues a mutation record — and schedules the notify microtask — at the
instant the tree changes, and WPT depends on it:
`MutationObserver-textContent.html` calls `await Promise.resolve()` after
setting `textContent` and reassigns the observer variable in the continuation,
so the callback must already have run. Attributing records at the checkpoint
instead loses that test.

The first attempt wrapped the mutating natives by assigning over
`globalThis.__appendChild` and friends. That works on Boa and **silently does
nothing on Nova**, whose global natives are non-writable and non-configurable
(`defineProperty` throws there too). The landed shape needs no interposition:
the bootstrap already funnels every DOM mutation through exactly **twelve** call
sites over eight natives, so those call sites now go through `moAppendChild`,
`moSetAttribute`, … — one-line wrappers that flush after the native returns.

`__moPump()` at the head of each microtask checkpoint stays as the backstop for
mutations that originate in Rust rather than in script, and `observe`,
`takeRecords` and `disconnect` each flush first, so a registration that appears
mid-task never inherits records from before it.

### The ancestor chain is captured, not walked later (2026-09-07)

"Queue a mutation record" walks the target's inclusive ancestors. By the time
the registry sees a record the tree may have moved again — the removal of a node
followed by the removal of its former parent is enough. Each `ObservedMutation`
therefore carries the chain as it stood at mutation time, and the transient
registrations for a removed subtree are derived from that same chain. This is
what makes the flush point a matter of scheduling rather than of correctness.

### A removed subtree must outlive its record (2026-09-07)

`LayoutDomMut::remove`, `set_text_content` and `set_inner_html` freed the
subtree they detached. A `MutationRecord` has to hand those nodes to script, so
while observing they are orphaned instead (`release_subtree`); `collect`'s
mark-sweep reclaims them once nothing reaches them. Unobserved, the free is
unchanged.

### One record for a compound operation (2026-09-07)

`replaceChild` is one `childList` record with both `addedNodes` and
`removedNodes`, but two arena mutations. `set_observer_group` opens a window in
which childList records naming the same target merge — added and removed
concatenate, the group keeps the first record's `previousSibling` and the last
one's `nextSibling` — and `replaceChild` opens one.

The *implicit* removal of the inserted node from its former parent is
deliberately excluded from the group (`record_implicit_removal`): the spec
queues it as its own record before the group's, which
`MutationObserver-childList.html`'s "internal replacement mutation" asserts
directly. Fixing that also forced `replaceChild` into spec order — resolve the
reference, detach the new node, then remove-and-insert — which repaired the
self-replacement case (`n.replaceChild(n.firstChild, n.firstChild)` used to
empty the parent).

### Two defects found in neighbouring code, both fixed here (2026-09-07)

- `DOMTokenList.add` / `remove` / `toggle` / `replace` validated nothing.
  `classList.add("c01", "", "c03")` must throw `SyntaxError` **before writing
  anything**; it was writing `c01` first, which is both a wrong attribute value
  and a spurious mutation record. All four now validate every token up front.
- `replaceChild` accepted a non-`Node` first argument and only failed later,
  after the removal. `document.body = "text"` must throw `TypeError` with the
  body untouched. This was the lane's only pass-to-fail regression, caught in
  the first `post` map and fixed before landing; the final map has none.

## Before and after

Disk mode, engine Boa, renderer Livery, `--jobs 8`, `--timeout 90`.
`pre` runner SHA-256
`2df9414611d08fff847009360c0242981b53af57ae681e1ad24f79e814d5707e`;
`post` runner SHA-256
`911ea5029f924d64fe642d4bb5ba3f84a4c0a9d224f33653a55fc810b2c71b1f`. Both
`--release -p genet-wpt --features netfetch`, built in this lane's own
`C:/t/lane4-target` at `98762466586`, `pre` from the unmodified tree.

| Directory | Files | All-pass pre -> post | Errored pre -> post | Subtests pre -> post |
|---|---:|---|---|---|
| dom | 660 | 161 -> **165** | 45 -> 45 | 2441/7079 -> **2934/7082** |
| custom-elements | 187 | 5 -> 5 | 26 -> **25** | 2093/3822 -> **2095/3827** |
| html/dom | 385 | 27 -> 27 | 59 -> **58** | 42309/59970 -> 42309/59971 |

Aggregate file-status movement, `post` against `pre`:

| Movement | Count |
|---|---:|
| `fail -> pass` | 4 |
| `error -> fail` | 2 |
| `pass -> fail` / `pass -> error` / `fail -> error` | **0** |

Subtest passes **+495**. Per observer file:

| `dom/nodes/MutationObserver-*` | pre | post |
|---|---|---|
| `sanity.html` | fail 0/13 | **pass 13/13** |
| `takeRecords.html` | fail 0/3 | **pass 3/3** |
| `disconnect.html` | fail 0/2 | **pass 2/2** |
| `callback-arguments.html` | fail 0/1 | **pass 1/1** |
| `attributes.html` | fail 0/42 | fail **35/42** |
| `childList.html` | fail 0/38 | fail **18/38** |
| `characterData.html` | fail 0/23 | fail **13/23** |
| `textContent.html` | fail 0/1 | fail **3/4** |
| `inner-outer.html` | fail 0/3 | fail **2/3** |
| `document.html` | fail 0/4 | fail **1/4** |
| `nested-crash.html` | skip | skip (manifest) |
| `cross-realm-callback-report-exception.html` | error | error (needs a second realm) |

### Explaining every non-forward movement

- **2 `error -> fail`, both forward.**
  `custom-elements/microtasks-and-constructors.html` and
  `html/dom/partial-updates/tentative/template-for-mutation-records.html` died
  on a `ReferenceError` at `MutationObserver` before reporting anything; both
  now run to completion, the first with +2 subtest passes.
- **0 `pass -> fail`, and no test anywhere lost a passing subtest.** Verified
  by name, not only by file status: a per-subtest diff over all three
  directories reports no subtest that passed in `pre` and does not pass in
  `post`. The one regression the lane produced —
  `html/dom/documents/dom-tree-accessors/Document.body.html`, "Setting
  document.body to a string" — appeared in the first `post` map and is fixed
  (Findings, above), not explained away.
- **`html/dom` +0 subtest passes, +1 observed.** The newly-running
  `template-for-mutation-records.html` reports one failing subtest; nothing else
  in the directory moved.
- **`custom-elements` holds.** Reaction ordering next to observers is unchanged:
  both are microtask-queue work and the observer's notify microtask does not
  reorder the custom-element reaction queue.

## Residuals

Named, and every one of them is a gap *outside* the observer that its tests
happen to exercise:

1. **`DocumentFragment` insertion does not move children.** `insertBefore` /
   `appendChild` insert the fragment node itself, so the four fragment subtests
   in `MutationObserver-childList.html` fail and two time out. This is a
   pre-existing DOM defect — a probe confirms `host.appendChild(fragment)`
   leaves the fragment's children where they are — not an observer defect; the
   record machinery already coalesces, so the fix is `insert` moving the
   children inside one `moBeginGroup()`. **This is a decision that is Mark's:**
   it changes core insertion semantics well beyond this lane and wants its own
   before/after census.
2. **`Range` is shape-only.** Ten `childList` and eight `characterData`
   subtests construct a `Range` and fail at construction.
3. **`Node.normalize` is absent.** Two `childList` subtests.
4. **`Element.attributes` is shape-only and attributes have no namespace.**
   Attributes are stored by qualified name with a null namespace, so
   `setAttributeNS` / `removeAttributeNS` records carry the qname as
   `attributeName` and `null` as `attributeNamespace`. Seven
   `MutationObserver-attributes.html` subtests.
5. **Comments and processing instructions do not survive the static-to-scripted
   clone.** `clone_into` keeps elements and text only, so
   `MutationObserver-characterData.html`'s Comment and PI cases have no node to
   observe.
6. **`outerHTML` is absent** — one `inner-outer.html` subtest, which times out.
7. **Parser-driven mutations are not observed.** `MutationObserver-document.html`
   wants records for nodes the parser inserts while the document loads; genet
   parses the document before script runs.
8. **Shadow trees are out of scope.** No shadow-including ancestor walk, so a
   `MutationObserver` never sees across a shadow boundary. It moves with the
   Shadow DOM lane.
9. **Cross-realm callbacks.** `-cross-realm-callback-report-exception.html`
   needs a second realm (an iframe) and still errors. A callback that throws is
   reported through `reportError` when the host defines one and otherwise
   swallowed, so one observer's exception never stops the others.

## Gates

- `cargo test -p genet-scripted-dom` — 31 tests green, including the two new
  observer-record bodies.
- `cargo test -p script-runtime-api` — 197 tests green across the suites,
  including `tests/mutation_observer.rs`: 8 bodies against **both** BoaEngine
  and NovaEngine (16 tests), and the interface-table drift test.
- `cargo clippy -p script-runtime-api -p genet-scripted-dom --all-targets` — no
  warning in `mutation_observer.rs`, `bootstrap.js`'s crate, `lib.rs` or the new
  test. The two warnings `genet-scripted-dom` still emits are on pre-existing
  lines (617, 840) this lane does not touch.
- `cargo fmt -p script-runtime-api -p genet-scripted-dom` — clean.
- `cargo build --release -p genet-wpt --features netfetch`, then the
  three-directory disk census above, `pre` and `post` from the same target
  directory.

## Progress

- **2026-09-07** — Phases 1-4 landed at base `98762466586`. Touched:
  `components/genet-scripted-dom/lib.rs`,
  `components/script-runtime-api/{lib.rs, dom/mod.rs, dom/bootstrap.js,
  dom/html_interfaces_generated.rs}`, new
  `components/script-runtime-api/dom/mutation_observer.rs` and
  `components/script-runtime-api/tests/mutation_observer.rs`, and
  `support/idl-interface-table/src/lib.rs`. Three defects found and fixed inside
  the lane: global-native interposition is impossible on Nova (so the flush
  moved to the twelve call sites), `DOMTokenList` wrote before validating, and
  `replaceChild` mutated before rejecting a non-`Node` — the last being the
  lane's only pass-to-fail, caught in the first `post` map. One measurement
  defect found and fixed: the first `pre` runner was built while the working
  tree was already being edited, so it was discarded and rebuilt from `HEAD`
  sources in the same target directory before any map was taken.
