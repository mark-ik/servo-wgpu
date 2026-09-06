# Buckram K6 corpus census: current-main reconciliation

**Date:** 2026-08-24; baseline refreshed 2026-09-06

**Status:** Complete for the dated sources below. The 2026-09-06 baseline is frozen; K6a candidate comparison is pending.

**Parent:** [Buckram and Livery lane program](2026-08-21_buckram_livery_lane_program_plan.md),
row 13, and the [K6 fragmentation execution plan](2026-08-15_buckram_k6_fragmentation_execution_plan.md).

## 2026-09-06 baseline refresh

The pre-K6 source is `0987ed2527167135431c41afd35c263a28d56c56`, checked
clean in a detached worktree. The release runner completed with
`cargo build --locked --offline --release --all-features -j 1 -p genet-wpt`
and `RUSTFLAGS=-C debuginfo=0`. Its cold build took 104m 25s; existing
scripted-capture, paint, and adapter warnings did not fail the build.

- Runner SHA-256: `DAF60CB93E9A0B2907B9EF52DF67994E3F8BAC27E3F48E814838BACCB6FF7FA7`.
- Lock SHA-256: `0FDF7926973C4498BA0F7608DDA2B03B5E10ACB728804F82C18C37E77EFAE126`.
- Manifest SHA-256: `D5EC5BE9BF1A75ED00D7E7AB28AFE8A694A55E11682BA74305874D70B18DD422`.
- WPT test-tree Git object: `443263e92be6e828c04b5d039f57412855e7a0ba`,
  identical at the archived harness source `f9d5174b68d` and this baseline.

Each family ran through `reftest css/css-<family> --renderer livery` with
an absolute `--tests-root`, `--write-expectations`, and
`--expectation-policy exact`. Direct fragmentation passes retain
`reference-unverified`; they earn no continuation or multicol capability credit.

| Family | Total | Pass | Fail | Skip | Unverified passes |
|---|---:|---:|---:|---:|---:|
| multicol | 708 | 60 | 337 | 311 | 60 |
| break | 1,170 | 83 | 832 | 255 | 83 |
| tables | 328 | 53 | 77 | 198 | 0 |
| position | 344 | 45 | 73 | 226 | 1 |
| flexbox | 1,358 | 486 | 398 | 474 | 11 |
| grid | 1,891 | 295 | 848 | 748 | 2 |
| page | 278 | 0 | 0 | 278 | 0 |

The break subdirectories are included in its total: table is 11/109/44,
flexbox 29/261/39, and grid 3/91/6 (pass/fail/skip). No crash or error status
was emitted. Exact skip reasons, in family order above, are
`non-reftest`: 248, 226, 170, 121, 423, 634, 278, and `needs-script`:
63, 29, 28, 105, 51, 114, 0. Every page test still skips as `non-reftest`.

Comparison with the archived full-CSS map finds 10 named break changes
(five gains and five losses among unverified passes), 180 flexbox changes
(175 gains, five losses), and 42 grid changes (26 gains, sixteen losses).
Multicol, tables, position, and page have identical named results. Thus net
flex/grid gains conceal 21 former guard passes that now fail; these predate
K6a and belong in the [Row 18 review](../design_docs/2026-08-25_livery_flex_shorthand_plan.md#2026-09-06-guard-regression-review).
The causes have not been isolated. Aggregate equality is not a named-result
stability receipt.

Artifacts are retained under
`C:/Users/mark_/Code/scratch/genet-k6-ortet-20260905/`: the frozen
`genet-wpt-baseline.exe`, `baseline-identity.json`, seven
`baseline-css-<family>.json` maps and logs, `baseline-summary.json`,
`baseline-vs-archive.json`, `k6-handoff.json`, and `run-baseline.ps1`.
The summary records every map hash and the full reason counts. The handoff
records verified K5 ancestry and the remaining single-fragment consumers.

## 2026-08-24 ruling

The archived 2026-08-21 census was diagnostically sound but was compiled
under the lane program's uncommitted read-only overlay. Its result maps also
predated the accepted K5 recovery. This reconciliation reruns the census from
accepted current main and carries forward none of that build provenance.

The WPT harness correction supersedes the original current-main score. The
old fuzzy scorer admitted 93 false passes in the K6 inventory. The exact
corrected result is 143 unverified passes in `css/css-multicol` and
`css/css-break`. Neither side consumed a fragmentation input. Another 14
unverified passes sit in the position, flex, and grid guard directories.
These results receive no K6 capability credit.

## Method

The frozen corrected runner wrote one exact full-CSS result map. The classifier
then selected the same family prefixes used by the earlier separate maps:

- `css/css-multicol`
- `css/css-break`
- `css/css-position`
- `css/css-tables`
- `css/css-flexbox`
- `css/css-grid`
- `css/css-page`, as an explicit all-skipped print-hosting receipt

The classifier pairs each result with the checked-in manifest and examines
the passing test, its direct references, and linked local stylesheets for
unconsumed fragmentation declarations: column definition and rule
properties, `break-*`, `page-break-*`, `orphans`, `widows`,
`box-decoration-break`, and `@page`. A pass is unverified when the test,
reference, or both use one of those declarations. `column-gap` alone does not
flag a result because it is independently consumed by flex and grid.

## 2026-08-24 absolute inventory (historical)

Runnable is pass plus fail. The current runner reports no crash results in
these maps; non-reftest and script-dependent items remain skipped.

| Family | Total | Runnable | Pass | Fail | Skip | Unverified | Verified |
|---|---:|---:|---:|---:|---:|---:|---:|
| `css/css-multicol` | 708 | 397 | 60 | 337 | 311 | 60 | 0 |
| `css/css-break` | 1,170 | 915 | 83 | 832 | 255 | 83 | 0 |
| `css/css-break/table` | 164 | 120 | 10 | 110 | 44 | 10 | 0 |
| `css/css-break/flexbox` | 329 | 290 | 27 | 263 | 39 | 27 | 0 |
| `css/css-break/grid` | 100 | 94 | 6 | 88 | 6 | 6 | 0 |
| `css/css-page` | 278 | 0 | 0 | 0 | 278 | 0 | 0 |
| `css/css-tables` | 328 | 130 | 53 | 77 | 198 | 0 | 53 |
| `css/css-position` | 344 | 118 | 45 | 73 | 226 | 1 | 44 |
| `css/css-flexbox` | 1,358 | 884 | 316 | 568 | 474 | 11 | 305 |
| `css/css-grid` | 1,891 | 1,143 | 285 | 858 | 748 | 2 | 283 |

The subdirectory rows are included in the `css/css-break` total and are not
added again. The direct fragmentation total is therefore 143 unverified
passes, not 143 plus its table, flex, and grid breakdowns.

The 14 unverified guard passes are:

- one in `css/css-position`: `position-absolute-multicol-001`;
- eleven in `css/css-flexbox`: the eight `flexbox-break-request-*` cases,
  `flexbox-with-multi-column-property`, and the two
  `flexbox_columns-flexitems*` cases;
- two in `css/css-grid`: `grid-lanes-gap-002` and
  `column-property-should-not-apply-on-grid-container-001`.

The four vertical-rl position guards and two grid-multicol guards previously
reported as unverified passes now fail under correct fuzzy scoring.

## Named ratchets

- `multicol-fill-auto-*` is 1 pass / 7 fail. The remaining pass is
  unverified; `multicol-fill-auto-001.xht` remains the first red K6c target.
- `multicol-basic-001` through `-004` fail; `-005` through `-008` pass
  unverified.
- `multicol-break-000` and `-001` now fail under correct fuzzy scoring.
- `basic-pagination-001-print.html` skips as `non-reftest`, as do all 278
  `css/css-page` files in the current renderer path.

## Current K5 handoff

The blocker list inherited by the archived census is closed or reassigned on
accepted main:

- K5 positioning closure is recorded at `e8db57141f1` with 26/26 named
  files green and ten directory gains without loss.
- the retained text-frame seam and grid static-rectangle seam are reconciled
  at `7eaaaf724a5` and `67c041d0cda`;
- K5d's eight named sizing/text files and nine logical-inset fixtures are
  green at `ed288ef1c3e`; the 36 red shapes cases belong to lane 12's absent
  `shape-outside` exclusions.

K6a can therefore begin from this frozen current-main baseline. Its input
gate still receives no layout credit, and any source change before K6a starts
requires the six guard maps to be refreshed.

## Continuation contracts

`components/genet-livery/tests/k6_fragmentation_contracts.rs` contains six
ignored tests: structural and retained-session contracts for block, inline,
and table roots in a two-column sequential-fill container. They require one
CSS box across continued fragments, distinct containing column fragments,
one non-initial fragmentation context, a continuation on every fragment but
the last, and correct session geometry, text, and hit testing.

The ignore reasons name the still-absent continuation kernel or fragmented
session consumer. They no longer claim K5h is blocking K6.

## Receipts

- Corrected harness source commit: `f9d5174b68d`.
- Manifest SHA-256:
  `D5EC5BE9BF1A75ED00D7E7AB28AFE8A694A55E11682BA74305874D70B18DD422`.
- Frozen runner SHA-256:
  `3CABCBE7C06892FCD1A4DAAA1B9BF45AA44D69BB633964A823023E3DC9636D81`.
  This runner checks WPT's two fuzzy ranges independently and labels every
  surviving coincidence pass `reference-unverified` in its exact result map.
- `cargo test --locked --offline -p genet-livery --test
  k6_fragmentation_contracts -- --list` lists six tests and zero benchmarks.
- Strict no-dependency Clippy for that target is green.
- File-local Rustfmt and `git diff --check` are green.
- The exact full-CSS map, refreshed classifier, per-file classifications, and
  stdout are under `testing/genet/wpt-ledger/2026-08-24_k6_census_v3`.
  The frozen runner, baseline/candidate maps, and exact result diff are under
  `testing/genet/wpt-ledger/2026-08-24_harness_ledger`.

## Done condition

Row 13 closes when current-main manifest and result inventories exist, every
fragmentation-directory pass is classified, six ignored continuation
contracts compile, and the obsolete K5 blocker list is reconciled. Those
conditions are met without engine code.
