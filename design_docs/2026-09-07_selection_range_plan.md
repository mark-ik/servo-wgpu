# Selection and Range: boundary points over the scripted arena

**Status:** landed 2026-09-07.

`Range` is the DOM's second addressing scheme. Everything else in the scripted
tier names a *node*; a range names a *point between* nodes or inside character
data, survives mutation of the tree it points into, and is the object the
Selection API, `execCommand` and every editing test are written against. This
lane adds `Range`, `StaticRange`, `AbstractRange`, `Selection` and
`getSelection` over the existing arena, plus the seam through which the
script-owned selection reaches Livery's range-rect primitive.

Base commit `f4995933d86` ("MutationObserver as a second consumer of the
scripted DOM's mutation point"). Receipts:
`Code/testing/genet/wpt-ledger/2026-09-07_selection_range/`.

## The one-source-of-truth decision

There are two selections in the tree already: the `TextRange<NodeId>` Livery
paints (`genet-scripted`'s `LiveryState.selection_range`, and
`LiveryDocument.selection_range` on the static path), in **rendered byte space**
over shaped text nodes, and the per-focused-element `TextCursor.selection` in
`genet-render`, in the same space. The Selection API's boundary points are
**DOM space**: a container node — often an element, not text — and a UTF-16
offset. The two spaces are not interchangeable, and neither is a superset.

**The script-owned `Range` is the source of truth. The visual selection is a
projection of it, and the projection is explicit and one-directional at a
time.**

- Script → visual. Every mutator of the Selection's own range calls
  `selectionMayHaveChanged`, which pushes the boundary points through
  `__selectionVisual` to the host's `SelectionHandler`
  (`components/script-runtime-api/dom/selection.rs`). `genet-scripted`'s
  `LiverySelection` resolves each point to (text node, byte offset) —
  descending to the nearest text node when the boundary is a container, and
  converting UTF-16 units to bytes — and writes `state.selection_range`. That
  field has exactly one writer set: this handler and the pointer gestures.
- Visual → script. A host pointer gesture calls `__selectionFromHost` with the
  boundary points it resolved, which replaces the Selection's range and fires
  `selectionchange`. Nothing else writes the selection from outside script.

`Range.getClientRects` reads the *same* primitive the overlay paints from
(`LiveryLayout::text_selection`), so geometry and paint cannot disagree. With no
layout bound the handler is absent and the rect list is empty, which is what the
spec asks of a range in a document with no box tree.

`genet-render`'s `TextCursor.selection` is deliberately **not** wired to this.
It is the caret-and-selection of a focused editable element in a host that has
no scripted selection at all (Ortet's raw path), and folding it in would make
the paint list a second authority. It moves with the editing lane, if that lane
ever gives editable elements a scripted selection.

## The live-range mechanism

The DOM specifies range updating as steps run *inside* the tree mutation
primitives: the removing steps need the child's index and the boundary offsets
as they stood before the removal, the insertion steps need the reference
child's index, replace-data needs `(offset, count)` and split needs the split
offset. The arena's `ObservedMutation` record — this lane's obvious
alternative, since the MutationObserver lane already fans out there — carries
none of those as such: it is drained after the fact and reports what changed,
not the pre-mutation indices the arithmetic needs.

So the range steps run at **the bootstrap's own mutation funnel**, the same
twelve call sites the observer lane established, immediately either side of the
native: `rangeWillRemove` before `__removeChild` / the implicit removal in
`__appendChild` / `__insertBefore` / `__moveBefore`, `rangeDidInsert` after,
`rangeWillReplaceAll` before `__setTextContent` / `__setInnerHtml`. Character
data gets its own funnel: every `CharacterData` mutator now goes through
`cdReplaceData`, which is DOM "replace data" and carries the real
`(offset, count, data)` rather than a whole new string, and `splitText` was
reordered into spec order so the boundary steps run before the truncation.

The arena is untouched by this lane. So is Livery's `DomMutation` stream.

### Boundaries are indexed by node, not held in a list

The first implementation kept one flat array of live ranges and walked it at
every mutation. That is quadratic, and it showed: eight `editing/run/*` files
that had merely *failed* before were killed as hangs (`delete.html`,
`forwarddelete.html`, `insertparagraph.html`, `multitest.html` and their query
shards; the directory went from 56s to 645s).

The landed shape indexes each boundary by the node it sits in. A boundary owns
one index entry `[weakref, which, resolved]` and the range keeps a handle to its
own two entries, so retiring one when the boundary moves is O(1) and a dead
range's entries are swept the next time their bucket is read. `rangeDidInsert`,
`cdReplaceData` and `rangeDidSplit` touch one bucket. `rangeWillRemove` still
walks the removed subtree — but that cost is the tree's, not the range
population's, which is the property that matters. The same directory now runs in
468s with all eight files reporting.

## What the lane added

| Piece | Home |
|---|---|
| `AbstractRange`, `StaticRange`, `Range` (boundary validation and ordering, `compareBoundaryPoints` / `comparePoint` / `isPointInRange` / `intersectsNode`, `deleteContents` / `extractContents` / `cloneContents` / `insertNode` / `surroundContents`, `toString`, `createContextualFragment`), `Selection`, `getSelection`, the boundary index and the live-range steps | `components/script-runtime-api/dom/bootstrap.js` |
| `SelectionHandler`, `__rangeRects`, `__selectionVisual` | `components/script-runtime-api/dom/selection.rs` |
| `Runtime::set_selection_handler`, `HostState::selection` | `components/script-runtime-api/lib.rs` |
| `LiverySelection`: DOM boundary points to shaped-text positions, the rect read and the projection sink | `components/genet-scripted/livery.rs` |
| `ProcessingInstruction` — the arena node, its native, its `nodeName`, its interface | `components/genet-scripted-dom/lib.rs`, `components/script-runtime-api/dom/{tree.rs, query_traverse.rs, mod.rs, bootstrap.js}` |
| `Selection` declared by the generated table (`selection-api.idl` joins the shape sources) | `support/idl-interface-table/src/lib.rs`, `dom/html_interfaces_generated.rs` |

`Range`, `StaticRange`, `AbstractRange` and `ProcessingInstruction` were already
on the generated shape-only list from `dom.idl`; the bootstrap now defines all
four before `installShapeInterfaces()` runs, so the shape pass finds the names
taken and only stamps their class strings. `Selection` lives in
`selection-api.idl`, which the generator did not read; it does now, and the
table declares it. No name needed adding to `SHAPE_ONLY_DENY`, and none needed
removing.

## Phases and done-conditions

### Phase 1 — Range and StaticRange (landed)

Done when the boundary points, their validation and ordering, the five content
operations and the live-range steps behave, over both backends.

Met. `dom/ranges` 0 → **10 all-pass** files and 1/182 → **24/186** subtests:
every file in the directory that runs at all improved, and the ten that are
pure boundary arithmetic (`Range-attributes`, `Range-constructor`,
`Range-commonAncestorContainer-2`, `Range-comparePoint-2`,
`Range-intersectsNode-2`, `Range-intersectsNode-binding`, `Range-detach`,
`Range-stringifier`, `Range-adopt-test`, `Range-extractContents-dynamic-end`)
now all-pass.
`tests/selection_range.rs` asserts the collapse-on-cross rule, the four
comparison entry points, `cloneContents` / `extractContents` over a partially
contained text pair, `insertNode`'s text split, and `surroundContents` with both
of its exceptions in spec order.

### Phase 2 — the live range steps (landed)

Done when insertion, removal, replace-data, split and replace-all move
boundaries the way the spec says, at zero cost to a document with no ranges, and
without making any mutation linear in the range population.

Met; see "Boundaries are indexed by node" above and the
`live_range_updating` test body. The zero-cost claim is the
`if (!liveRangeCount)` guard at the head of every step: a document that never
constructs a range does no bucket lookup at all.

### Phase 3 — Selection (landed)

Done when the Selection API's whole surface works over one live range, the range
it hands out is the same object it holds, and `selectionchange` fires at the
document.

Met. `selection` 0 → **12 all-pass** files, 58 → **13** errored, 0/280 →
**28,582/33,621** subtests.

### Phase 4 — geometry and the interface table (landed)

Done when `getClientRects` / `getBoundingClientRect` read Livery's range-rect
primitive where a frame exists and return empty lists otherwise, `Selection` is
declared through the generated table, the drift check passes, and the tests run
on both engines.

Met. `cargo run -p genet-idl-interface-table -- --check` is current;
`tests/selection_range.rs` runs seven bodies against BoaEngine and NovaEngine
(14 tests), including the geometry seam with and without a handler.

## Findings

### The census directories were capped by one missing node type (2026-09-07)

`dom/ranges` and `selection` share nothing with each other except a setup file
each (`dom/common.js`, `selection/common.js`) that builds a menagerie of node
types before a single subtest runs. Both call
`xmlDoc.createProcessingInstruction(...)`, which threw "not a callable
function" and aborted the *whole file*. That is why the first `post` map showed
`selection` at 50/384 subtests with 43 files errored while every runnable file
had improved: the measurement was floored by a node type, not by Range.

`ProcessingInstruction` is therefore part of this lane: a real arena node
(`NodeKind::ProcessingInstruction` already existed in `layout-dom-api` and had
no constructor), its target in the node's `name` and its data in `text`, so the
character-data readers and writers reach it unchanged. That one addition took
`selection` from 384 reported subtests to 33,621.

It also forced a correction next door: `first_element_child` in
`script-runtime-api/dom/mod.rs` tested for an element by asking whether the node
had a name, and a processing instruction now has one. `document.documentElement`
would have picked up a PI. It tests `kind` now, and
`processing_instruction_on_{boa,nova}` asserts it.

### `dom/ranges` is still floored, by `CDATASection` and `ParentNode.append` (2026-09-07)

The same probe run against `dom/common.js` after the PI fix reports two more
absences, in this order: `new Document().createCDATASection` is `undefined`, and
`ParentNode.append` is not a function. `selection/common.js` needs neither,
which is why the two directories moved so differently. `CDATASection` is a new
arena node kind in `components/shared/layout-dom` — outside this lane's write
paths and the wrong thing to fake, since the tests assert `nodeType === 4`.
**This is a decision that is Mark's:** completing the DOM node model
(`CDATASection`, real `DocumentType` nodes and `document.doctype`, which
`createDocumentType` currently fabricates as an `HTMLUnknownElement` with
`nodeType` 1) is a lane of its own, and it is what unlocks the fifteen
`dom/ranges` files still erroring.

### A DocumentFragment is inserted whole, so Range moves its children itself (2026-09-07)

The MutationObserver plan's first residual — `insertBefore` / `appendChild`
insert a `DocumentFragment` node rather than moving its children — is still
open, and `Range.insertNode`, `surroundContents` and the recursive halves of
`extractContents` / `cloneContents` all "append a fragment". Rather than change
core insertion semantics from inside this lane, each of those sites moves the
fragment's children explicitly (`appendFragmentChildren`). When the fragment fix
lands, those loops become one `appendChild` each; until then Range is correct
and the underlying defect is unchanged.

### `Selection` is not in `dom.idl` (2026-09-07)

The generator read `dom.idl` and `cssom.idl` for its shape pass, so `Range`,
`StaticRange`, `AbstractRange` and `ProcessingInstruction` were already
declared but `Selection` was not declared anywhere: it lives in
`selection-api.idl`, which WPT vendors alongside them. Adding that file to the
shape sources is the whole change — one new `ShapeInterface` row — and the drift
test byte-compares it.

## Before and after

Disk mode, engine Boa, renderer Livery, `--jobs 8`, `--timeout 90`.
`pre` runner SHA-256
`c6ae425e0915541e99c794536b6cd3b48373da09fd143e1cc7ed1b1b85d6c66c`;
`post` runner SHA-256
`3dc9df36eed83a0f02740438e9994832da516022154284803837c232b3eade5e`. Both
`--release -p genet-wpt --features netfetch`, built in this lane's own
`C:/t/lane6-target` at `f4995933d86`, `pre` from the unmodified tree before the
first edit.

| Directory | Files | All-pass pre -> post | Errored pre -> post | Subtests pre -> post |
|---|---:|---|---|---|
| selection | 161 | 0 -> **12** | 58 -> **13** | 0/280 -> **28582/33621** |
| dom/ranges | 57 | 0 -> **10** | 15 -> 15 | 1/182 -> **24/186** |
| dom/nodes | 330 | 81 -> **83** | 10 -> 10 | 2318/5651 -> **2360/5663** |
| html/editing | 424 | 26 -> 26 | 6 -> 6 | 83/760 -> 83/760 |
| editing | 843 | 1 -> 1 | 414 -> **411** | 1/97687 -> **480/97716** |

Aggregate file-status movement, `post` against `pre`:

| Movement | Count |
|---|---:|
| `error -> fail` | 45 |
| `error -> pass` | 3 |
| `fail -> pass` | 21 |
| `no-results -> fail` | 1 |
| `pass -> fail` / `pass -> error` / `fail -> error` | **0** |

Subtest passes **+29,126**.

The MutationObserver files that were waiting on `Range`:

| `dom/nodes/MutationObserver-*` | pre | post |
|---|---|---|
| `characterData.html` | fail 13/23 | fail **21/23** |
| `childList.html` | fail 18/38 | fail **32/38** |

Residual 2 of the observer plan ("`Range` is shape-only", ten `childList` and
eight `characterData` subtests) is closed exactly: `characterData` gains its
eight, `childList` its ten plus four more that construct a range on the way to
something else.

### Explaining every non-forward movement

There are none. Every movement is forward, and a per-subtest diff by name over
all five directories reports **zero** subtests that passed in `pre` and do not
pass in `post`.

- **45 `error -> fail`, 3 `error -> pass`.** Files whose shared setup threw at
  `createProcessingInstruction` and reported nothing; all now run to
  completion. 42 of them are `selection/`, three are `editing/`.
- **1 `no-results -> fail`**, `dom/nodes/CharacterData-remove.html`: it reported
  no subtests at all and now reports twelve. They fail — `ChildNode.remove` on
  a comment or PI is a separate gap — but the file runs.
- **`editing` is 645s -> 468s and lost no file.** The eight
  `editing/run/*` files the flat range list hung are back, and the directory's
  three `error -> fail` movements are them plus their shards reporting.
- **`html/editing` holds exactly**, 83/760 both sides. Nothing in it constructs
  a Range, and nothing in it regressed.

## Residuals

1. **`CDATASection`, real `DocumentType` nodes and `document.doctype`.** The
   fifteen `dom/ranges` files still erroring are floored on these, in
   `dom/common.js`. See the Findings; **Mark's decision**, and the largest
   single unlock left in this census.
2. **`ParentNode.append` / `prepend` / `replaceChildren` are absent.** Small,
   pure-JS over `insertBefore`, but outside this lane's surface and with its own
   census to take. `dom/common.js` hits it immediately after `CDATASection`.
3. **`Selection.modify` is a no-op.** Word and line granularity are shaped-text
   facts the script tier does not own; it needs the same Livery seam
   `getClientRects` uses, running the other way. It does not throw, so a feature
   test still succeeds.
4. **`getComposedRanges` ignores its `shadowRoots` argument** and reports the
   one live range as a `StaticRange`. It moves with the Shadow DOM lane, as does
   any shadow-including boundary walk.
5. **Multiple ranges per selection are not modelled.** `addRange` on a
   selection that already has a range returns without effect, which is what the
   spec says for a single-range implementation, and `rangeCount` never exceeds
   1. No WPT file in these directories asks for more.
6. **The `selectionchange` task is a microtask.** The spec queues a task; this
   queues a microtask, coalesced to one per checkpoint. A test that distinguishes
   the two would see the event too early.
7. **Fragment insertion still inserts the fragment node**, so Range moves
   children by hand (Findings). Unchanged from the observer plan's residual 1.
8. **`Range.getClientRects` resolves a container boundary to the nearest text
   node.** A range whose endpoints are element boundaries reports the rectangles
   of the shaped text between them, not of the elements' own boxes. Element box
   geometry needs `Element.getBoundingClientRect`, which the scripted tier does
   not have at all.

## Gates

- `cargo test -p script-runtime-api` — 211 tests green across the suites,
  including `tests/selection_range.rs`: 7 bodies against **both** BoaEngine and
  NovaEngine (14 tests), and the interface-table drift test.
- `cargo test -p genet-scripted-dom` — 31 green.
- `cargo test -p genet-scripted` — 25 green.
- `cargo run -p genet-idl-interface-table -- --check` — current.
- `cargo clippy -p script-runtime-api -p genet-scripted --all-targets` — no
  warning in any file this lane touches.
- `cargo fmt -p script-runtime-api -p genet-scripted -p genet-scripted-dom
  -p genet-idl-interface-table` — clean.
- `cargo build --release -p genet-wpt --features netfetch`, then the
  five-directory disk census above, `pre` and `post` from the same target
  directory.

## Progress

- **2026-09-07** — Phases 1-4 landed at base `f4995933d86`. Touched:
  `components/script-runtime-api/{lib.rs, dom/mod.rs, dom/bootstrap.js,
  dom/tree.rs, dom/query_traverse.rs, dom/html_interfaces_generated.rs}`, new
  `components/script-runtime-api/dom/selection.rs` and
  `components/script-runtime-api/tests/selection_range.rs`,
  `components/genet-scripted/livery.rs`,
  `components/genet-scripted-dom/lib.rs`, and
  `support/idl-interface-table/src/lib.rs`. Two defects found and fixed inside
  the lane: the flat live-range list was quadratic and hung eight
  `editing/run/*` files that had previously reported (caught in the first `post`
  map, replaced by the per-node boundary index), and `first_element_child`
  identified elements by the presence of a name, which a processing instruction
  now also has. One measurement finding: both census directories were floored by
  a missing `createProcessingInstruction`, so the first `post` map understated
  the lane by roughly 28,500 subtest passes.
