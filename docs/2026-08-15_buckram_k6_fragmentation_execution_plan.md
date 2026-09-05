# Buckram K6: fragmentation execution plan

**Date:** 2026-08-15

**Status:** K6a is unblocked from the frozen 2026-08-24 baseline, reconciled
2026-09-05. Planning and the current-main corpus census are complete. Accepted
main now contains the K5 positioning, retained text-frame, grid static-rectangle,
and K5d residual receipts. K6a may begin from the frozen 2026-08-24 baseline;
the census itself adds contracts and documentation, not engine behavior.

**Parent:** [Buckram CSS layout engine plan](2026-07-26_buckram_css_layout_engine_plan.md),
K6.

## Ruling

K6 extends K5's retained `CssBoxTree`, `FragmentTree`, and dirty-root model
across fragmentainers. It does not create a second layout result, a print-only
box tree, or a Livery-owned continuation cache.

The first implementation target is sequential-fill multicol because it gives
continuous media a real fragmentainer while preserving the existing headed
renderer and interaction path. Pagination, table fragmentation, flex, and grid
land only after the ordinary block/inline continuation contract is live.

A WPT pass is never sufficient by itself. Unsupported multicol can make both a
test and its reference fall through ordinary block layout and compare equal.
Every accepted gate therefore needs:

1. a Buckram structural receipt;
2. a live Livery geometry or mutation receipt;
3. a named absolute WPT result where the runner can host the test; and
4. an exact removal or assigned-deferral receipt.

Stylo comparison remains an interoperability ledger. It is not K6 acceptance.

## Handoff state

This plan was prepared against the moving `buckram-k5-positioning` worktree on
2026-08-15. That historical branch is not a K6 base. Current accepted main
closes the K5 seams required by K6a through `e8db57141f1`, `7eaaaf724a5`,
`67c041d0cda`, and `ed288ef1c3e`. The 2026-08-24 current-main census is the
pre-K6 baseline; see the
[census reconciliation](2026-08-24_buckram_k6_corpus_census_reconciliation.md).

K4 is closed by the accepted K4h bridge deletion at `610df0981a8`. The master
plan now records that closure. K6 consumes the accepted K4 table model and does
not reopen K4.

The old K6a blocker list is retired. K5h retained text movement, K5b grid
static rectangles, the named K5 positioning regressions, relative captions,
and the eight K5d sizing/text files are green on accepted main. The 36 red
shape files are assigned to lane 12's absent `shape-outside` exclusions; they
do not block the K6 continuation model.

The current K5 shape is still specific enough to fix ownership:

| Existing seam | Current boundary | K6 action |
|---|---|---|
| `components/buckram/src/fragment_tree.rs` | `FragmentationContextId::INITIAL` is the only context. `BreakToken` is a placeholder `resume_at: u32`. `FragmentTree::by_box` already permits one box to own several fragments. | Add fragmentainer/context records and algorithm-owned continuation tokens to this tree. Do not replace the tree. |
| `FragmentTree::static_positions` | One static-position record per box is asserted as an unfragmented K5 invariant. | Index the record by the fragment or fragmentainer that supplied it. Positioned descendants must resume in the correct containing fragment. |
| `FragmentTree::replace_subtree` | Correctly rejects a replacement that selects only some fragments of a box. | Add a continuation-chain replacement operation. Keep the K5 operation for unfragmented roots. |
| `components/buckram/src/box_tree.rs` | K5 retains generated `BoxId` provenance independently from storage order. | One continued box keeps one `BoxId`; fragmentainers and continuations never synthesize duplicate CSS boxes. |
| `components/buckram/src/block.rs` | Owns ordinary block flow, margin collapse, floats, clearance, and intrinsic block queries in continuous media. | Make its formatter resumable. Its break token owns carried margin, float, clearance, and nested-child state. |
| `components/genet-livery/src/layout.rs` | Produces Buckram fragments and selected-root replacements. Several consumer paths still use `get` or `principal_fragment`, which select one rectangle. | Lower fragmentation inputs, materialize all continued fragments, and remove single-fragment selection from fragment-aware paint, hit-test, scroll, and geometry paths. |
| `components/genet-livery/src/document.rs` | K5 damage selection and fresh-final equivalence are authoritative. Fragmented roots remain outside local replacement. | Promote damage to the fragmentation context when necessary, then replace only the affected continuation chain and compare with a fresh final document. |
| `components/genet-livery/src/{text,paint}.rs` | Text and paint are retained side data keyed by the K5 result. | Consume continued fragment identity and fragmentainer clips. They do not get independent break decisions. |
| `components/genet-livery/src/{table_block,table_shadow,table_wrapper}.rs` | K4/K5 table geometry and retained paint side planes are live. | Table fragmentation consumes the K4 model and publishes split table fragments through the same K6 tree. |
| `components/livery/properties.toml` | `column-count`, `column-width`, and `column-span` are declared unimplemented. `column-fill`, break controls, orphans, widows, and `box-decoration-break` are absent. `column-gap` is implemented. | Add typed values and computed projection in the gate that first consumes each property. Do not add dormant declarations. |
| `components/livery/src/stylesheet.rs` | `CssRule` has style, media, container, and keyframes rules. `@page` is diagnosed as unsupported. | Add page-rule parsing and CSSOM projection with the pagination gate, not the multicol gate. |
| `ports/genet-wpt/src/main.rs` | The manifest distinguishes `PrintReftest`, but `reftest` accepts only `Kind::Reftest` and skips print tests as `non-reftest`. | A print-media page renderer and exact print-reftest result path are prerequisites for paged-media credit. |

## Corpus census

The census used the checked-in 39,279,168-byte
`tests/wpt/meta/MANIFEST.json`, dated 2026-08-12, and the manifest command from
the existing K5 runner. These are inventory counts, not pass counts.

| Family | Total | Reftest | Print reftest | Testharness | Crashtest | Other |
|---|---:|---:|---:|---:|---:|---:|
| `css/css-multicol` | 708 | 460 | 4 | 91 | 151 | 2 |
| `css/css-break` | 1,170 | 944 | 64 | 47 | 114 | 1 |
| `css/css-break/table` | 164 | 122 | 13 | 8 | 21 | 0 |
| `css/css-break/flexbox` | 329 | 291 | 27 | 0 | 11 | 0 |
| `css/css-break/grid` | 100 | 94 | 1 | 0 | 5 | 0 |
| `css/css-page` | 278 | 0 | 224 | 23 | 11 | 20 |
| `css/css-tables` | 328 | 158 | 1 | 133 | 36 | 0 |
| `css/css-flexbox` | 1,358 | 935 | 3 | 358 | 25 | 37 |
| `css/css-grid` | 1,891 | 1,257 | 1 | 606 | 24 | 3 |

The manifest kinds remain unchanged. The frozen current-main Livery results
below replace the old 96-pass note and the archived overlay-backed census.

A routing sample with the corrected harness produces:

- `multicol-fill-auto-001.xht`: fail;
- `multicol-basic-005.xht`: pass, unverified;
- `multicol-break-000.xht`: fail; and
- `multicol-height-002-print.xht`: skip.

The surviving pass receives no capability credit because the route does not
consume multicol computed values. The print skip confirms the pagination
harness gate.

The current exact maps are:

| Family | Pass | Fail | Skip | Unverified pass |
|---|---:|---:|---:|---:|
| `css/css-multicol` | 60 | 337 | 311 | 60 |
| `css/css-break` | 83 | 832 | 255 | 83 |
| `css/css-position` | 45 | 73 | 226 | 1 |
| `css/css-tables` | 53 | 77 | 198 | 0 |
| `css/css-flexbox` | 316 | 568 | 474 | 11 |
| `css/css-grid` | 285 | 858 | 748 | 2 |
| `css/css-page` | 0 | 0 | 278 | 0 |

All 143 passes in the two direct fragmentation directories are unverified;
14 more unverified passes sit in the guard directories. Exact per-file
classification and the named family ratchets are in the
[current-main census reconciliation](2026-08-24_buckram_k6_corpus_census_reconciliation.md).
These corrected maps supersede the original 230-direct / 20-guard score, whose
fuzzy comparison admitted an unlimited number of low-delta pixels. They are
the frozen K6 baseline. Refresh them if accepted source changes before K6a
begins.

## Serialized execution

Only one gate owns source at a time. Each accepted gate lands before the next
gate begins. Corpus and documentation preparation can run beside K5, but K6
source work cannot.

### K6a. Handoff freeze and fragmentation inputs

**Prerequisites**

- K5h is accepted and merged on `main`.
- The final K5 receipt names every fragmentainer-dependent positioning and
  dirty-root fallback routed to K6.
- The K5 branch has no unmerged edits to the K6 file seams above.
- The WPT expectation lane can record exact skip/error reasons without treating
  missing results as green.

**Work**

1. Record the accepted K5 commit and freeze the absolute Livery result maps.
2. Re-run the handoff inventory. Any new K5 single-fragment assumption gets a
   named K6 owner before code changes.
3. Implement typed computed values for the first multicol slice:
   `column-count`, `column-width`, and `column-fill`. Preserve `column-gap` as
   the existing input. Implement the `columns` shorthand through the two
   longhands. Add `column-span` only when K6c4 consumes it.
4. Implement `break-before`, `break-after`, `break-inside`, `orphans`, and
   `widows` before the ordinary break algorithm consumes them.
5. Add CSS parse, computed-value, inheritance, initial-value, shorthand,
   mutation, and CSSOM receipts. Update the consumed-set knockout as soon as a
   K6 formatter reads a column property.

**Files**

- `components/livery/properties.toml`
- `components/livery/src/values/property.rs`
- generated property/cascade output selected by the existing Livery build
- `components/livery/tests/{values,cascade,consumed_set,stylesheet}.rs`
- `components/genet-livery/src/style.rs`
- `components/genet-livery/tests/{cssom,invalidation}.rs`

**Acceptance**

- Every accepted spelling reaches a typed computed value and serializes back
  through CSSOM.
- Changing any consumed column or break input invalidates layout.
- Invalid values remain invalid rather than becoming `auto`.
- No column declaration changes geometry yet. K6a is an input gate and receives
  zero WPT layout credit.

### K6b. Fragmentation context and ordinary break/resume kernel

**Work**

1. Add `FragmentainerId` and explicit fragmentation-context records. Each
   fragmentainer has a logical content rectangle, flow, parent context,
   sequence position, and kind. The first kind is `Column`.
2. Replace the placeholder numeric `BreakToken` with algorithm-owned token
   variants. A block token retains the next child, a nested child token,
   carried collapsed-margin state, float/exclusion state, clearance state, and
   enough inline state to resume without replaying accepted earlier content.
3. Make ordinary block and inline formatting accept a fragmentainer constraint
   and return completed fragments plus an optional continuation token.
4. Implement forced breaks first, then unforced overflow breaks, then
   `break-inside`, widows, and orphans. A monolithic child reports that fact;
   it is not silently clipped or split.
5. Preserve containing-fragment links, baselines, logical coordinate spaces,
   overflow, and one `BoxId` across all fragments.

**Files**

- new `components/buckram/src/fragmentation.rs`
- `components/buckram/src/{lib,fragment_tree,block,intrinsic}.rs`
- `components/genet-livery/src/{layout,text}.rs`

**Model receipt**

A synthetic fixed-size pair of fragmentainers resumes one ordinary block
without reconstructing the box tree. The first fragment owns a typed block
token and the second resumes from its complete state. Tests assert context
ancestry, fragmentainer order, containing fragments, baseline state, carried
float state, and final overflow.

**Stop boundary**

K6b is algorithm/model work. It does not receive a live multicol or WPT pass
until K6c consumes it.

### K6c. Live multicol and first load-bearing continuation

Start with a definite-height sequential-fill container. Balancing, spanners,
column rules, nested multicol, and overflow columns are follow-on sub-gates
inside K6c.

**First load-bearing fixture**

```html
<div id="columns">
  <div id="continued">
    <div class="band"></div><div class="band"></div>
    <div class="band"></div><div class="band"></div>
  </div>
</div>
```

```css
#columns {
  width: 220px;
  height: 100px;
  column-count: 2;
  column-gap: 20px;
  column-fill: auto;
}
.band { height: 50px; }
```

The accepted receipt proves:

- the multicol box creates one fragmentation context and two 100 by 100 column
  fragmentainers at inline offsets 0 and 120;
- `#continued` keeps one `BoxId` and produces exactly two structural
  `FragmentId`s, each contained by the corresponding column fragment;
- the first fragment has the block continuation after the second band and the
  second fragment consumes it;
- all four bands paint once, hit testing in either column returns the right DOM
  node, and fragment-aware geometry returns both rectangles in column order;
- the document extent is the multicol container's extent rather than a
  fictitious 200px continuous block; and
- Livery reports Buckram fragmentation dispatch rather than a continuous-flow
  fallback.

The first WPT target is
`css/css-multicol/multicol-fill-auto-001.xht`, which currently fails in the
planning sample. It can turn green only after the structural fixture above is
green. Add `multicol-fill-auto-*`, `multicol-basic-*`, and `multicol-break-*`
as named ratchets, while retaining exact results for the whole family.

**K6c sub-gates**

| Gate | Outcome |
|---|---|
| K6c1 | definite-size sequential `column-fill: auto` and overflow columns |
| K6c2 | forced/unforced column breaks, break avoidance, widows, and orphans |
| K6c3 | column balancing with bounded convergence and explicit failure |
| K6c4 | implement and consume `column-span`, then add spanners, nested contexts, floats, and positioned descendants |
| K6c5 | implement column-rule and fragment-decoration inputs, then close overflow, scroll, and consumers |

**Files**

- new `components/buckram/src/multicol.rs`
- `components/buckram/src/{lib,fragmentation,fragment_tree,block}.rs`
- `components/genet-livery/src/{style,layout,paint,document,text}.rs`
- new `components/genet-livery/tests/fragmentation.rs`

### K6d. Table fragmentation

Table fragmentation consumes the accepted K4 table grid, track sizing, row
layout, captions, border model, and K5 positioned/static records. It does not
flatten a table into ordinary blocks or reconstruct table widths per page.

Execute in this order:

1. fragment table wrappers and row groups at row boundaries;
2. carry captions, border spacing, collapsed-border ownership, and used column
   widths into every table fragment;
3. repeat header and footer groups with distinct fragments but shared source
   boxes;
4. split a row and cell when allowed, including rowspan continuation and cell
   block alignment;
5. integrate positioned descendants, sticky constraints, overflow, paint, hit
   testing, and retained side planes; and
6. add break avoidance and monolithic-overflow rules.

**Files**

- new `components/buckram/src/table/fragmentation.rs`
- `components/buckram/src/table/{fragments,pipeline,rows,borders}.rs`
- `components/buckram/src/{table,fragmentation,fragment_tree,lib}.rs`
- `components/genet-livery/src/{layout,table_block,table_shadow,table_wrapper,paint}.rs`
- new `components/genet-livery/tests/table_fragmentation.rs`

**Receipt**

A table with one header group, enough body rows for two columns, one continued
rowspan, and one footer produces repeated header/footer fragments, stable
column geometry, and one continuation chain. The test asserts the containing
fragment and source box of every repeated/split part and compares paint and hit
testing to a hand-built reference. Ratchet
`css/css-break/table` separately from unfragmented `css/css-tables`.

### K6e. Pagination and print-media host

K6e begins only after the runner can execute `PrintReftest` as paged media.

**Harness prerequisite**

- `ports/genet-wpt/src/main.rs` must route `Kind::PrintReftest` to a print
  renderer rather than `skip: non-reftest`.
- `ports/genet-wpt/src/render.rs` must accept page size, margins, print media,
  and a deterministic page sequence. Test and reference page counts must be
  compared explicitly.
- The result file records page count, per-page dimensions, pass/fail, and skip
  reason. Missing or unhostable pages are not passes.

**Engine work**

1. Add `@page` parsing, CSSOM projection, named/pseudo page selection, page
   size, and page margins in Livery.
2. Add page fragmentainers and forced left/right/page breaks.
3. Implement page counters and page-sequence geometry before margin boxes.
4. Replicate fixed-position descendants per applicable page using K5's fixed
   containing-block semantics.
5. Integrate page backgrounds, overflow, paint order, hit-test/page coordinate
   projection, and accessibility page grouping.

**Files**

- `components/livery/src/stylesheet.rs`
- `components/livery/properties.toml`
- `components/livery/tests/{stylesheet,values}.rs`
- new `components/buckram/src/pagination.rs`
- `components/buckram/src/{fragmentation,fragment_tree,lib}.rs`
- `components/genet-livery/src/{style,layout,paint,document}.rs`
- `ports/genet-wpt/src/{main,render}.rs`

**Receipt**

Start with `css/css-page/basic-pagination-001-print.html`, then the forced-break
and `fixedpos-*` families. K6e cannot close while all 224 `css/css-page` print
reftests are classified as non-runnable.

### K6f. Flex and grid fragmentation

Flex and grid are separate accepted sub-gates. Each must consume and produce
the K6 continuation contract. A completed Taffy placement is not a break token.

1. Audit the pinned Taffy adapter for fragmentainer inputs, break opportunities,
   resumable child state, and stable item identity.
2. If the upstream algorithm can expose the required facts without owning the
   fragment tree, add a narrow adapter result.
3. If it cannot, invoke the parent plan's fork policy with a written delta and
   an upstreamable interface. Do not emulate fragmentation by clipping a full
   unfragmented layout into pages.
4. Land flex first, then grid. Cover items, containers, nested fragmentainers,
   order, alignment, baselines, monolithic overflow, and positioned children.

**Files**

- `components/buckram/src/{taffy_adapter,fragmentation,fragment_tree}.rs`
- new `components/buckram/src/{flex_fragmentation,grid_fragmentation}.rs` if
  the adapter cannot keep the algorithms isolated
- the pinned Taffy patch directory only after the fork-policy audit
- `components/genet-livery/src/{layout,paint}.rs`
- new `components/genet-livery/tests/{flex_fragmentation,grid_fragmentation}.rs`

**Corpus**

- flex: `css/css-break/flexbox` before the broad `css/css-flexbox` guard;
- grid: `css/css-break/grid` before the broad `css/css-grid` guard.

### K6g. Positioned, sticky, overflow, and continuation-chain relayout

K6g joins K5 positioning and persistence to fragmentainers.

1. Make static-position records fragmentainer-specific.
2. Place positioned descendants in the containing fragment selected by the
   K5 graph and CSS fragmentation rules. Keep fixed replication in the page
   context only.
3. Recompute sticky constraints per fragmentainer and scrollport without
   mutating normal-flow base geometry.
4. Add `FragmentTree::replace_continuation_chain`. It replaces every fragment
   of the affected box in one fragmentation context, repairs context and
   static-position indices, and rejects cross-context dependencies that need a
   wider dirty root.
5. Promote damage to the nearest root that owns the affected break decision.
   Compare every retained result to a fresh layout of the same final document.

**Mutation receipt**

Insert one 50px band between the second and third bands in the K6c fixture. The
affected continuation chain grows from two columns to three. The multicol
context and source `BoxId` stay stable, the unchanged first column retains its
`FragmentId` when its content and token are unchanged, later fragments are
replaced, an unrelated sibling retains all identities, and paint, hit testing,
text search, scroll extent, and document extent match a fresh final layout.

Resource completion, font metric change, viewport change, break-property
mutation, column-count change, and page-size change each need a fresh-final
equivalence receipt. A change that alters an earlier break invalidates the
dependent suffix; it must not pretend later fragmentainers are independent.

### K6h. Consumer, corpus, and deletion closure

Audit every browser-facing fragment consumer:

- paint and clipping;
- hit testing and pointer routing;
- selection, find, caret, and text order;
- CSSOM geometry and fragment navigation;
- accessibility bounds and page/column grouping;
- scrolling, fragment navigation, sticky state, and overflow; and
- retained mutation publication.

Every consumer either iterates the relevant fragments or documents the exact
CSS rule that selects first, last, principal, or union geometry. Generic use of
`LayoutResult::get` or `principal_fragment` on a fragmented box blocks closure.

Delete:

- the numeric placeholder `BreakToken`;
- the initial-only fragmentation-context assumption;
- single-record static-position ownership;
- continuous-flow fallback for accepted multicol/page/table/flex/grid cases;
- any print-only fragment side plane; and
- expectation knockouts that became consumed K6 properties.

Freeze absolute and differential results. The absolute ledger names passes,
failures, skips, errors, and unsupported host capabilities. A positive total
cannot hide a moved regression.

## Gate verification

Use a unique target directory so K5 and parallel WPT work do not contend for
Cargo locks. At every source gate run the smallest focused red/green receipt,
then:

```powershell
cargo test -p buckram -p livery -p genet-livery --offline
cargo test -p genet-livery --all-targets --offline
cargo clippy -p buckram --all-targets --offline -- -D warnings
cargo clippy -p genet-livery --all-targets --no-deps --offline -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Build `genet-wpt --release --all-features` from the same accepted source and
record the executable commit. Run absolute Livery results first. Run Stylo only
as a separate differential result.

Each gate receipt records:

- accepted base and gate commits;
- exact test commands and counts;
- manifest timestamp/hash and runner commit;
- changed WPT statuses by filename;
- structural assertions and dispatch counters;
- removals;
- remaining assigned failures; and
- any unmeasured consumer or host boundary.

## Stop rules

- Stop before source work if K5h is not accepted on `main`.
- Stop if a change creates a parallel box tree, fragment vector, continuation
  cache, or print-only geometry authority.
- Stop if a continued source box receives multiple `BoxId`s.
- Stop if a formatter must replay already accepted content because its break
  token omitted state.
- Stop if a retained splice selects only some fragments of a continuation chain
  without proving the remaining tokens are independent.
- Stop if paint, hit testing, accessibility, CSSOM, or text must guess one
  rectangle through `get` or `principal_fragment`.
- Stop a WPT claim if the structural fixture is absent, the runner skipped the
  test, or test and reference can pass through the same unsupported fallback.
- Stop pagination credit while `PrintReftest` is still `non-reftest`.
- Stop before flex/grid implementation if the Taffy audit cannot expose a
  resumable algorithm boundary. Invoke the fork policy instead.
- Stop before table fragmentation if the live route bypasses the accepted K4
  table model.
- Keep the post-K4 anonymous-table conformance debt separate. It is not a K6
  fragmentation receipt.

## Done condition

K6 is complete when Buckram can break and resume ordinary block/inline,
multicol, table, page, flex, and grid formatting through algorithm-owned tokens;
one box retains identity across all of its fragments; K5 positioning and
dirty-root relayout operate on continuation chains; every browser consumer uses
the continued fragments; print WPT is genuinely runnable; all accepted
fallbacks and knockouts are deleted; and the absolute corpus ledger records
every remaining failure with an owner.

Multicol alone does not close K6. A large WPT pass delta does not close K6.
K6 closes only on the ownership, consumer, deletion, mutation, and corpus
receipts above.
