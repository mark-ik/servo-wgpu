# Livery flex shorthand plan

**Date:** 2026-08-25

**Status:** In progress, reconciled 2026-09-05. The `flex` and `flex-flow`
cascade, bounded specified/computed CSSOM, distinct `flex-basis` modeling,
generic inline declaration reflection, and physical flex main-axis, alignment,
and gap projection are complete. The flex-basis content used-value,
automatic-minimum-size, bounded vertical-flex cross-axis, element-owned
self-alignment, and orthogonal auto-block row slices are complete. Row 18
remains open for mixed-writing-mode baseline and generated/pseudo self-edge
work, shared `ex` and zero-percentage provenance, and grid.

**Parent:** [Buckram and Livery lane program](../docs/2026-08-21_buckram_livery_lane_program_plan.md),
row 18.

## Target

Make Livery expand the two core flex shorthands into the longhand values that
Genet-Livery already lowers into Taffy. This slice does not introduce another
flex model or widen Buckram's block-formatting ownership. The existing
`ComputedValues` fields remain authoritative and Taffy remains the flex layout
algorithm.

## Phase 1: cascade surface

Implement `flex` and `flex-flow` in Livery's generated shorthand catalog and
remove them from the known-unimplemented list.

`flex` expands `none`, factor-only, factor-pair, basis-only, and combined
factor/basis forms. The basis may occur before, between, or after the factor
group when its token is unambiguous. Three entirely numeric tokens retain the
conventional grow/shrink/basis interpretation: a trailing unitless zero is a
valid zero basis, while a non-zero unitless basis is invalid. Omitted values
use the Flexbox shorthand defaults, including `0%` for an omitted basis.

`flex-flow` accepts direction and wrapping in either order and supplies the
longhand initial value for an omitted component. Both shorthands expand CSS-wide
keywords across every longhand and preserve `!important`.

**Done condition:** catalog generation succeeds, native cascade tests cover
keywords, arities, basis order, invalid numeric bases, CSS-wide values, and
importance, and the property-space ledger removes exactly two shorthand gaps.

## Phase 2: live style bridge

Add one Genet-Livery test that resolves authored shorthand declarations, checks
`flex-flow` against an equivalent pair of longhands, and observes Taffy
geometry. A 100px wrapping host produces two 50px items followed by one 100px
item; the equivalent longhand host matches it; and `nowrap` keeps all three
items on one line.

**Done condition:** the focused receipt and the full Genet-Livery library wall
pass without adding a new layout path or a Buckram fallback claim.

## Phase 3: exact WPT ledger

The accepted directory is `css/css-flexbox`, enumerated from the pinned WPT
manifest. The baseline runner is the final horizontal-float runner from source
commit `0af65021c56`. The candidate was rebased across the Fleece 0.4 and Pelt
receipt commits that landed during its release build. A preserved pre-rebase
runner and the post-Fleece pre-fix runner produced status-identical maps across
all 1,358 identities, so those intervening commits contribute zero flexbox
status movement.

| Exact receipt | Baseline | Candidate | Movement |
|---|---:|---:|---:|
| `css/css-flexbox` reftests | 318 pass / 566 fail / 474 skip | 430 pass / 454 fail / 474 skip | 115 fail-to-pass, 3 assigned false-pass losses |
| `css/css-flexbox/parsing` testharness | 68 / 183 subtests | 68 / 183 subtests | status-identical |
| `flexbox-align-items-center-nested-001.html` | fail | pass | one verified gain |
| `flexbox-flex-wrap-horiz-001.html` | fail | pass | one verified gain |

The 115 directory gains divide into 54 `flexbox_flex-*` value-matrix files,
19 named `flex-flow`/wrapping files, and 42 other live flex geometry files.
There is no skip movement. Four initially exposed unitless-basis losses forced
the numeric-token repair before acceptance and pass in the final map.

At Phase 3 acceptance, three pass-to-fail results were assigned rather than
hidden:

- `flex-minimum-height-flex-items-029.html` begins exercising the existing
  nested column-flex automatic-minimum-size gap once its valid `flex`
  declarations are no longer ignored.
- `flexbox-writing-mode-slr-row-mix.html` uses `flex-flow` in its reference and
  exposes the existing sideways-writing flex-axis transform gap.
- `gap-001-rtl.html` uses `flex: 1 1 auto` in both files and exposes the
  existing RTL flex-gap versus logical-margin equivalence gap.

Those were baseline coincidences caused by dropping the shorthand. They are
downstream layout residuals, not evidence that the expanded longhands differ
from their authored values. Phase 5 repairs the direction and gap residuals;
the automatic-minimum-size residual remains open under its separate owner.

**Done condition:** every directory identity is accounted for, invalid
unitless bases retain their four baseline passes, and every remaining loss has
a named downstream owner.

## Phase 4: specified and computed CSSOM

Reuse Livery's shorthand cascade parser as the grammar authority for
`element.style` and `CSS.supports()`, while requiring that the synthetic parse
produce exactly the expected longhands. Extra declarations, custom
declarations, embedded `!important`, empty values, and parser diagnostics are
invalid at this seam. Canonical specified `flex` values use
grow/shrink/basis order; canonical `flex-flow` values use direction/wrap order
with initial components elided.

Genet-Livery reconstructs computed `flex` and `flex-flow` from their computed
longhands. A unitless zero flex basis serializes as the computed length `0px`,
while the shorthand's omitted basis remains `0%`.

This phase does not implement shorthand-to-longhand reflection in the generic
JavaScript `CSSStyleDeclaration` store. Variable-bearing shorthand values
continue through the existing pass-through path, so `CSS.supports()` for those
values remains assigned to that generic seam rather than counted here.

**Done condition:** native Livery, Genet-Livery, and Genet-Scripted receipts
prove canonical specified values, invalid-value retention, non-variable
`CSS.supports()` classification, inline-to-computed flow, and computed
shorthand reads. The exact flexbox parsing map attributes every movement to the
six named `flex`/`flex-flow` valid, invalid, and computed files with no
pass-to-fail movement.

## Phase 5: physical flex direction and gaps

Project the container's logical `flex-direction` through Buckram `FlowAxes`
before handing the style to Taffy's physical row/column model. Transpose
`row-gap` and `column-gap` into Taffy's physical width/height slots for vertical
flex containers only; grid retains its separate lowering path. Project
explicit `justify-content: start/end` through the same logical main axis while
preserving `normal` as its distinct computed value and flex-start used value.

**Done condition:** a native matrix covers all five supported writing modes in
both directions, unequal gaps prove the physical component mapping, live RTL
and `sideways-lr` geometry passes, both assigned direction/gap WPT residuals
move fail-to-pass, and the full flexbox reftest map has no unexplained loss.

## Phase 6: generic inline declaration reflection

Keep shorthand grammar and component metadata in Livery while making the
shared Script Runtime declaration store generic. Normal shorthand assignments
expand atomically into ordered longhands. Complete longhand sets reconstruct
for shorthand reads and declaration-block serialization. A variable-bearing
shorthand creates one hidden pending-substitution entry per longhand; those
longhands serialize as empty individually, while a complete set from one
origin reflects the authored shorthand value.

The native string bridge escapes record delimiters, verifies the exact
component count and order before mutation, and rejects malformed expansion as
an atomic no-op. The Runtime receives supported shorthand names, components,
expansion, and reconstruction from the selected CSS engine rather than naming
`flex` or `flex-flow` itself.

**Done condition:** focused Boa and Nova receipts cover expansion, removal,
`cssText` reconstruction, pending substitution, later longhand mutation,
external attribute reparse, malformed expansion, and escaped fields. The exact
flex parsing map moves only `flex-shorthand.html` and
`flex-flow-shorthand.html`, with zero loss.

## Findings

### 2026-09-05

- The September 1/2 decomposition receipts moved tagged `calc()` length
  handling through `flex_basis`, `length_percentage`, and
  `length_percentage_auto` in `9df392f42d8`. The remaining Row 18
  zero-percentage item concerns authored-syntax provenance, separate from the
  repaired premature-resolution path. See the
  [decomposition follow-through](2026-08-28_orchestrator_decomposition_plan.md#outcome).
- The same follow-through records measured block/grid border-box sizing
  (`72dc32cbf3e`) and inline replaced-root sizing (`afc09d06ea4`). Percentage
  min/max constraints on replaced elements remain an explicit K7 boundary.
  These receipts update the work inventory; this reconciliation does not
  claim a fresh full-suite or WPT run.

### 2026-08-25

- CSS Flexbox's `||` basis ordering cannot be implemented by unconstrained
  token permutation. Ambiguous all-numeric triples use factor/factor/basis
  order; otherwise an interior zero can incorrectly validate `flex: 0 1 4`.
- Livery's declaration cascade and the test runner's CSSOM shorthand
  serialization are separate seams. Layout and reftest gains are live while
  the 27-file parsing-testharness map remains unchanged.
- Correct shorthand admission exposes real downstream flex gaps that an
  ignored declaration could accidentally hide. Those gaps must retain their
  own owners rather than weakening shorthand parsing.
- Current main changed twice during the release build. The first rebase changed
  compiled Fleece inputs but produced a status-identical flexbox map; the
  second reconciliation added only `design_docs`, with no non-document tree
  difference between the built candidate and the final rebased candidate.
- A Turnstone published-source consumer exposed that Genet's root-local Parley
  patch was not inheritable. Current Genet-Livery uses the fork's
  `last_line_alignment` and `TabSize` APIs, but a downstream root silently
  selected crates.io Parley 0.10 and failed before reaching consumer code.
  `support/patches/parley` is now a workspace package so consumers can patch
  crates.io to the Genet git source explicitly.
- The twelve already-published Knot/Livery support commits had remained on the
  clean `release/knot-editor-host-0.1.0` lane. Their version and publishability
  changes are now merged into main, including Livery 0.0.3, Host API 0.1.1,
  Inker 0.1.1, and Nematic 0.1.1.
- That release lane's `genet-taffy 0.13.0` routing was stale against current
  Buckram: the published trait had five parameters while current Buckram's
  static-position implementation requires eight. The corrected fork is now
  published as `genet-taffy 0.13.1`, and Buckram plus Genet-Livery route to that
  package identity while the root patch keeps workspace builds on the same
  source.

### 2026-08-26

- “CSSOM shorthand serialization” spans three independent seams: specified
  admission/canonicalization, computed shorthand reads, and generic
  `CSSStyleDeclaration` shorthand-to-longhand reflection. This continuation
  closes the first two only.
- A declaration-block parser is safe to reuse for one CSSOM value only when
  the caller proves the exact expanded property set. Without that guard,
  `1; color: red` and embedded `!important` were accepted as a `flex` value.
- `flex-flow`'s cascade expander admitted an empty component list as the two
  initial longhands. Empty authored and CSSOM values are invalid and now have
  a native regression receipt.
- Taffy's flex direction is physical while CSS `flex-direction` is logical.
  The missing `FlowAxes` projection jointly owned
  `flexbox-writing-mode-slr-row-mix.html` and `gap-001-rtl.html`; vertical gap
  components must be transposed after the same projection.
- Physicalizing the flex main axis also requires projecting explicit
  `justify-content: start/end`. The first candidate exposed two RTL losses.
  Treating every `start` as authored then exposed that Livery had collapsed
  the CSS initial `normal` value to `start`; retaining `normal` and lowering
  its flex used value to `flex-start` repaired that distinction.
- The surviving `flex-minimum-height-flex-items-029.html` residual is separate:
  vendored Taffy's min-content collection creates one item per line and then
  retains only the longest line. That repair requires its own Taffy release
  lane.
- `flex-basis` now has a distinct specified/computed value model. It admits
  `content` and the intrinsic sizing keywords, rejects width-family `none`,
  resolves definite font- and environment-relative values, reapplies its
  non-negative computed range after deferred `ch` and container-unit
  resolution, and retains its own interpolation and CSSOM serialization. The
  temporary Genet-Livery adapter still lowers intrinsic and `content` bases to
  Taffy `auto`; this is an explicit compatibility boundary, not used-value
  support.
- Two focused parsing residuals remain outside that value model:
  `calc(2em + 3ex)` needs shared `ex` font metrics, while
  `calc(0% + 10px)` needs the generic length-percentage representation to
  retain a syntactic zero-percentage term.
- The prior plan records the final shorthand runner name and hash, but the
  executable was not present in the preserved ledger directory on 2026-08-26.
  The continuation therefore rebuilds and freezes both current-main baseline
  and candidate runners instead of treating the old record as a live artifact.

## Phase 1-3 historical native and frozen receipts

- Livery: all package tests pass, including 33 cascade tests.
- Genet-Livery: 214 / 214 library tests pass.
- Final source commits: `4d22d7e7d07`, `0902dfd9ccb`, and `7da01dd6293`.
- Frozen runner: `candidate-7da01dd6293-genet-wpt.exe`.
- Runner SHA-256:
  `F3FA180E095DACD37AB043AF69AD339F962970B06DA7B701569802BDB607EE40`.
- Manifest SHA-256:
  `D5EC5BE9BF1A75ED00D7E7AB28AFE8A694A55E11682BA74305874D70B18DD422`.
- Final flexbox map SHA-256:
  `524A0BB1609DF3FA4C75BBC53EB95D4CD04299FE2CE44C313C2B8C2E71FE436E`.
- Final parsing map SHA-256:
  `F0D1B4815A135BBCC0A810C56A697BD21354535B46AC7563F91CD122B42D3D9E`.

The ignored workspace lock was regenerated offline after Fleece's 0.4 version
bump. The accepted release runner then built with `--locked --offline -j 1`.
All frozen artifacts are under
`testing/genet/wpt-ledger/2026-08-25_flex_shorthands`.

## Phase 4-5 native and frozen receipts

The continuation baseline is accepted main at `bad78dda19f`. The accepted
source ends at `838167fc179`, followed only by receipt documentation. Both
source trees use Cargo.lock SHA-256
`9618C76EFD48385C11C36933A04287332E78F80453BFB27A5FB306B2D78D03D6`.

- Final frozen runner: `genet-wpt-final-838167fc179-nodebug.exe`.
- Runner SHA-256:
  `45EC1D090656C6CDD229E07AA258864B3A554D0E8C161EF00A8239C56D0090FC`.
- Manifest SHA-256:
  `D5EC5BE9BF1A75ED00D7E7AB28AFE8A694A55E11682BA74305874D70B18DD422`.
- Livery: 189 passed, zero failed, four ignored.
- Genet-Livery: 223 passed, zero failed.
- Genet-Scripted: 25 passed, zero failed.

| Exact receipt | Baseline | Final | Movement |
|---|---:|---:|---:|
| `css/css-flexbox` reftests | 430 pass / 454 fail / 474 skip / 0 error | 449 pass / 435 fail / 474 skip / 0 error | 19 fail-to-pass, 0 pass-to-fail |
| `css/css-flexbox/parsing` testharness | 68 / 183 subtests | 98 / 183 subtests | 30 fail-to-pass, 0 pass-to-fail |
| `flexbox-writing-mode-slr-row-mix.html` | fail | pass | assigned direction residual retired |
| `gap-001-rtl.html` | fail | pass | assigned gap residual retired |
| `flex-minimum-height-flex-items-029.html` | fail | fail | separate automatic-minimum-size residual unchanged |

The first candidate produced 15 reftest gains but regressed the two RTL
`justify-content: start/end` files. The alignment projection repaired those;
a second focused candidate then exposed the collapsed `normal` initial value.
Both rejected runners and their focused maps remain frozen beside the accepted
artifacts. The final full map has 19 exact gains and no loss. Its SHA-256 is
`E1C7DECAAF59AC0152AB8394B23AFD7A89910ACDD0548DD57CE3758DE4C2C285`;
the final parsing map SHA-256 is
`F197F29880341D72B89A87E1906B2186D1A9902803B0775F38FB31AFBA7E5EEF`.

The reproducible commands, all runner hashes, exact transition identities,
and focused maps are recorded under
`testing/genet/wpt-ledger/2026-08-26_flex_axis_cssom`.

## Phase 6 native and frozen receipts

The accepted predecessor is `6f0fd483a460`; the generic reflection
implementation ends at `48e4ce58123` after rebasing onto `6b65d8df327`.
Current main's intervening resource work required one
offline lock update, adding only `genet-document-resources` to
`pelt-desktop`. The resulting Cargo.lock SHA-256 is
`D7B7F329F75C91E975F4328E8439F47C76CD512CEA304E5EE072C8C4538D34D8`.

- Frozen runner: `genet-wpt-candidate-48e4ce58123-nodebug.exe`.
- Runner SHA-256:
  `4FAA9FDD155419A816E5C87FCE7A42D77C433BE7E3D7F2F0838A1B42CEB96E09`.
- Manifest SHA-256:
  `D5EC5BE9BF1A75ED00D7E7AB28AFE8A694A55E11682BA74305874D70B18DD422`.
- Livery: 192 passed, zero failed, four ignored.
- Genet-Livery: 224 / 224 library tests and every integration binary passed.
- Genet-Scripted: 26 / 26 passed.
- Script Runtime: 122 / 122 library, 16 / 16 fetch, and 7 / 7 WebGL tests
  passed.

| Exact receipt | Predecessor | Final | Movement |
|---|---:|---:|---:|
| `css/css-flexbox/parsing` testharness | 113 / 183 | 159 / 183 | 46 fail-to-pass, 0 pass-to-fail |
| `flex-shorthand.html` | 0 / 40 | 40 / 40 | 40 fail-to-pass |
| `flex-flow-shorthand.html` | 0 / 6 | 6 / 6 | 6 fail-to-pass |

All 183 identities are present in both maps. Every other subtest is
status-identical. The final map SHA-256 is
`639BB0A3E4293A1254786C6E8AB4DACA20ED9F2FA205457BC38349BD1A241D6F`.
The pre-final `b68957e555b` map is status-identical to the final map and remains
in the ledger for provenance.
The executable, predecessor and final maps, native details, and commands are
recorded under
`testing/genet/wpt-ledger/2026-08-27_flex_cssom_reflection`.

## Row 18 closure and remaining work

Container-level vertical flex row/column cross-axis and wrap projection is
measured: the 2026-08-27 receipt passes writing-mode 002/003/005/006.
Element-owned child `align-self` is also measured through its parent-context
keyword matrix and five live fixtures, including inherited alignment,
subject-relative edges, content-keyword stretch fallback, and anonymous-item
provenance containment. The two auto-width row-wrap cases are also measured:
the bounded Buckram admission keeps the horizontal parent block lane while
Taffy derives the vertical flex container's automatic physical width, and both
target WPTs pass twice beside writing-mode 002/003/005/006. The exact receipt
is under `testing/genet/wpt-ledger/2026-08-28_flex_vertical_auto_block`.
Mixed-writing-mode baseline alignment and generated or pseudo inherited
self-edge projection remain open; this is not general vertical-flex
completion. Shared `ex` metrics and zero-percentage provenance remain separate
value-resolution work, followed by grid auto tracks and template areas.

The `flex-basis: content` slice is complete: `content` bypasses the preferred
main-size fallback while `auto` retains it; the row and column content queries
measure Buckram-owned content at the required max-content or fit-content size;
and all eight `flexbox-flex-basis-content-*` reftests pass twice under exact
release receipts in
`testing/genet/wpt-ledger/2026-08-27_flex_basis_content`. The Taffy complete
delta is regenerated and verified. The automatic-minimum-size release is also
receipt-complete: its focused Genet-Livery regression passes 1 / 1, and
the exact Livery receipt covers that case plus the eight content-basis
adjacency cases, all nine passing from HEAD `9d23efc433d`. The receipt is
recorded at
`testing/genet/wpt-ledger/2026-08-27_flex_automatic_minimum`, with runner
SHA-256
`1711e2b1a3a5b943ac92aa83ac4816fb02ed69272b4c17eed525efcfeb5454c4` and WPT
manifest SHA-256
`d5ec5be9bf1a75ed00d7e7ab28afe8a694a55e11682ba74305874d70b18dd422`.
The package-gate baseline remains Livery 192 passed / 0 failed / 4 ignored,
Genet-Livery 223 passed plus the known canvas baseline failure, and Buckram
255 / 255. The focused released-package native receipts are Genet-Livery
automatic minimum 1 / 1, flex-basis content 2 / 2, and its content-basis repro
6 / 6. The canvas result is unchanged: the pre-existing
`replaced_html_dimensions_use_computed_css_and_canvas_intrinsics` assertion
observes `200 x 200` instead of `100 x 100` on the untouched baseline.
`genet-taffy 0.14.0` was published from `e8a67b06a4b`, merged to `main` at
`8b4e14e7853`, and tagged `genet-taffy-v0.14.0`. The 69-file registry archive
has SHA-256 `50F2A560C4025930D7138D18BA78C55B3C40E562A2927B3D58E871771DB0676D`;
consumer resolution is clean.

- Continue vertical-flex work only at the remaining mixed-writing-mode
  baseline and generated/pseudo self-edge boundaries.
- Add shared `ex` font metrics and generic zero-percentage provenance rather
  than encoding either exception inside `FlexBasis`.
- Inventory and implement the remaining grid surface, including auto tracks
  and template areas, under its own current-main receipt.

### 2026-09-06 guard regression review

The [refreshed K6 baseline](../docs/2026-08-24_buckram_k6_corpus_census_reconciliation.md#2026-09-06-baseline-refresh)
uses clean pre-K6 source `0987ed25271` and the same WPT test-tree object as
the archived harness receipt. Flexbox improves by 170 net passes and grid by
10, but the named comparison also exposes five flexbox and sixteen grid
pass-to-fail changes. These precede K6a. Their causes remain unisolated;
they are review work, not a newly accepted regression allowance.

The complete names and old/current records are in
`C:/Users/mark_/Code/scratch/genet-k6-ortet-20260905/baseline-vs-archive.json`.
Flex losses cover image expansion, minimum heights, a vertical image case,
and `gap-010-ltr`. Grid losses cover negative positioned indices,
max-content/border sizing, grid-lanes row sizing/alignment/gaps, and subgrid
line names, orthogonal writing mode, and auto-fill.

Before claiming a wider Row 18 closure, reproduce these 21 cases with pinned
old/current runners on the same host, then isolate stable source regressions
and assign each result to its owning sizing, positioning, gap, or subgrid seam.
Keep them separate from the 143 direct fragmentation passes, which remain
reference-unverified and carry no K6 layout credit.

## Progress

### 2026-08-25

- Added and generated the two shorthand catalog entries.
- Implemented bounded, order-aware expansion and CSS-wide reset semantics.
- Added native cascade and live Taffy geometry receipts.
- Repaired the invalid numeric-basis ambiguity exposed by the first exact map.
- Completed the 1,358-file flexbox comparison with 115 gains and three assigned
  false-pass losses.
- Published the Parley fork as a git-addressable Genet workspace package and
  proved Genet-Livery from the workspace root before retrying the Turnstone
  consumer.
- Merged the twelve-commit published release history into current main,
  retained its valid package-version changes, and rejected only the stale
  published-Taffy routing after a compiler proof. `cargo check -p genet-livery
  -j 1` passes again on the current fork.

### 2026-08-26

- Published `genet-taffy 0.13.1` from commit `506d84a6c659` and tagged that
  source as `genet-taffy-v0.13.1`.
- Regenerated the complete fork delta against upstream Taffy 0.13.0 and proved
  that it reproduces all 50 vendored source files byte for byte.
- Ran all 131 library tests from the packaged all-features source and all 254
  Buckram tests against the exact release candidate. A clean standalone
  Buckram check then resolved and compiled the published registry crate.
- Split the CSSOM continuation into specified admission, computed reads, and
  still-open generic declaration reflection rather than treating them as one
  surface.
- Added exact-value guards around shorthand canonicalization, rejected empty
  `flex-flow`, and added native specified, computed, inline, and
  `CSS.supports()` receipts.
- Projected flex main directions through `FlowAxes`, transposed vertical flex
  gaps, and added a ten-case axis matrix plus unequal-gap live geometry.
- Projected explicit flex `justify-content: start/end`, restored the distinct
  `normal` initial and computed value, and retained `normal`'s flex-start used
  behavior through physical axis lowering.
- Froze current-main baseline and accepted candidate runners. The exact
  parsing map gains 30 subtests, and the full 1,358-file flexbox map gains 19
  reftests with zero pass-to-fail movement.
- Added a distinct `FlexBasis` type through generated property dispatch,
  cascade, computed-value resolution, animation interpolation, CSSOM, and an
  explicit temporary Taffy adapter. Native Livery, Genet-Livery, and scripted
  Boa receipts are green.
- Reapplied the non-negative computed range at each deferred relative-length
  boundary. A focused native regression retains `calc(10cqw - 1em)` until its
  container is known, then clamps the resolved negative length to zero.
- Rebuilt matched baseline and candidate runners with one ignored root lockfile
  copied byte-for-byte between worktrees. The focused flex-basis surface moves
  from 14 / 27 to 25 / 27; the complete flex parsing map moves from 98 / 183 to
  113 / 183 with 15 exact gains and zero losses.
- Characterized all eight `flexbox-flex-basis-content-001a` through `004b`
  reftests. They remain local failures in both maps, so this slice makes no
  content used-value or layout claim.

### 2026-08-27

- Added generic inline shorthand expansion and reconstruction metadata through
  Livery, Genet-Livery, Script Runtime, and Genet-Scripted.
- Modeled variable-bearing shorthands as per-longhand pending-substitution
  values, including empty longhand serialization, common-origin shorthand
  reads, later longhand mutation, and external style-attribute reparse.
- Replaced the raw line bridge with escaped fields and exact ordered expansion
  validation before mutation.
- Froze the rebased WPT runner. The complete parsing map moves from 113 / 183
  to 159 / 183 through the 40 `flex` and six `flex-flow` shorthand subtests,
  with zero losses and no other identity movement.
- Completed the content-basis used-value path. A `content` basis now bypasses
  a preferred main size while `auto` still consults one, including the
  aspect-ratio Step B correction that preserves an explicitly authored cross
  size. Buckram supplies the narrow row max-content and column fit-content
  measurements, including direct replaced leaves, nested flex, and BFC float
  lines; Livery keeps blockified flex/grid items on the block paint route.
- Regenerated the authoritative Taffy complete delta and proved a clean
  pristine-source apply reproduces the vendored `src/` tree byte for byte.
  The standalone all-features Taffy receipt records 131 unit and 5 doc tests
  passing.
- Rebased the Row 18 series onto current `main`, rebuilt the release WPT
  runner (`eb236a1c866e105810f1d46240f30cc9fb4030aa1a9c5a2b62dfa1378b136b60`),
  and ran `flexbox-flex-basis-content-001a` through `004b` twice with the
  exact policy. Each run-one/run-two map pair is byte-identical; all sixteen
  maps are checked in under `testing/genet/wpt-ledger/2026-08-27_flex_basis_content`.
- Re-ran the focused Livery receipt (6 / 6) and the full Buckram wall
  (254 / 254) after the rebase. The rebase preserved every Row 18 compiled
  source blob byte for byte, so the already-measured full Livery wall remains
  applicable: 223 passed and one pre-existing canvas assertion failed. The
  same `replaced_html_dimensions_use_computed_css_and_canvas_intrinsics`
  failure, which observes `200 × 200` instead of its expected `100 × 100`,
  occurs on the untouched baseline and is not attributed to this slice.
- Repaired the nested wrapped-column automatic-minimum-size residual in the
  vendored Taffy release and added the focused Genet-Livery geometry
  regression. The exact frozen Livery receipt passes the automatic-minimum
  case and all eight content-basis adjacency cases; its runner, manifest, and
  per-case maps are recorded under
  `testing/genet/wpt-ledger/2026-08-27_flex_automatic_minimum`.
- Published `genet-taffy 0.14.0` from `e8a67b06a4b`; merged it to `main` at
  `8b4e14e7853` and annotated the release source as `genet-taffy-v0.14.0`.
  The 69-file registry archive has SHA-256
  `50F2A560C4025930D7138D18BA78C55B3C40E562A2927B3D58E871771DB0676D`.
  Consumer resolution is clean. Release-native gates are Buckram 255 / 255,
  Genet-Livery automatic minimum 1 / 1, flex-basis content 2 / 2, and
  content-basis repro 6 / 6; all nine focused WPT cases pass.
- Projected element-owned flex-item `align-self` only after the parent's
  physical axes are known. The lowering distinguishes logical and
  flex-relative edges, resolves inherited and explicit `self-start`/`self-end`
  against the subject flow, and limits content-keyword fallback to effective
  stretch. The native receipt is Genet-Livery library 229 pass with the known
  canvas baseline filtered, live vertical fixtures 5 / 5, automatic minimum
  1 / 1, content basis 2 / 2, content repro 6 / 6, and Buckram 255 / 255.

### 2026-08-28

- Admitted the bounded vertical flex row/row-reverse automatic block-size
  shape into Buckram's horizontal parent walk while leaving its physical width
  unknown for Taffy. Exact containing-flow, sizing, direction, positioning,
  float, and containment guards keep the admission local.
- Made the live regression faithful to the WPT paragraph and ignored
  `float:right` flex-item declarations. The merged native receipt is
  Genet-Livery 6 / 6 and Buckram 257 / 257.
- Built and froze runner
  `E833A632E9F2CD590DE5B4D983A7EAC923E155814749BDBE662BE8C76A7F3D6D`.
  Both target auto-width WPTs pass twice with byte-identical maps, and
  writing-mode 002/003/005/006 remain 4 / 4 green under the exact policy.

### 2026-09-05

- Reconciled Row 18's present frontier against the September 1/2
  decomposition receipts and separated shared `ex` metrics from
  zero-percentage authored-syntax provenance. Historical native/WPT counts
  above remain attached to their original source and date.
