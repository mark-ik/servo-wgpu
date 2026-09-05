# Buckram and Livery lane program

**Date:** 2026-08-21
**Status:** Reconciled 2026-09-05 against accepted main. Lane 8, the
anonymous-table continuation, K5 rows 1+2+7, 3, 4, and 5+6, K6 row 13,
css-text row 14, paint row 15, fonts row 16, writing-modes row 10, and harness
rows 19+20+21 are complete. Row 12's rectangular, horizontal rounded,
relative float-state, and horizontal direction slices are complete, while the
row remains in progress. Row 18's flex shorthand, bounded CSSOM, distinct
flex-basis specified/computed model, generic declaration reflection, and
physical main-axis, alignment, and gap projection slices are complete, as are
the flex-basis content used-value and automatic-minimum-size slices. Row 18
remains in progress for the remaining vertical-flex work, shared `ex` and
zero-percentage provenance, and grid.
Wave 2 is unblocked. Every other row remains an inventory item until its
current-main receipt is named below.

**Parent:** [Buckram CSS layout engine plan](2026-07-26_buckram_css_layout_engine_plan.md)
and the [Livery fullweb cutover plan](2026-07-24_livery_fullweb_cutover_and_servo_retirement_plan.md).

## Provenance ruling

The original parallel launch from `k5-regression-repair` was retired because
some lane results depended on uncommitted source and a shared build directory.
Archived `lane/*` branches and `7499aff278b` are forensic evidence only. New
work starts from accepted `origin/main`, in an isolated worktree and target
directory. A lane may reuse a historical diagnosis, but not its unverified
receipt or dirty overlay.

The accepted recovery chain and later focused continuations are the source of
truth. Lane 8 closed on `ac73b07badb` and its receipt commit `0e2a6bebed3`.
The anonymous-table construction and sibling-table continuation are recorded
in [their recovery plan](2026-08-23_buckram_anonymous_table_recovery_plan.md).

## Rules every lane follows

- **Ownership.** Edit only the files and regions named by the lane. Record a
  required cross-lane change as a seam request.
- **Base.** Fetch and start from accepted `origin/main`. Inspect status before
  staging and stage only owned paths.
- **Build isolation.** Give each worktree its own `CARGO_TARGET_DIR`. On
  Windows use `CARGO_PROFILE_TEST_DEBUG=0` when PDB pressure blocks runnable
  tests, and serialize jobs when linker or disk pressure requires it.
- **Runner identity.** Build the release `genet-wpt` runner from the candidate,
  copy it into the external ledger, and record its commit and SHA-256. A shared
  `target/release/genet-wpt.exe` is not a frozen receipt.
- **Measuring.** Keep ledgers under `testing/genet/wpt-ledger/<dated-lane>/`.
  Run the lane directories before and after from frozen runners. Any
  unexplained pass-to-fail result stops the lane.
- **Native wall.** Run focused fixtures, full affected-crate tests, scoped
  strict Clippy, formatting, and `git diff --check`.
- **Done means measured.** A commit closes a row only when its plan names the
  native and WPT receipts that prove the done condition.

## Current lane ledger

| # | Lane | State on 2026-08-24 | Done condition or next proof |
|---|---|---|---|
| 8 | Block-formatter admission | **Complete** | Independent tables and flow roots stay opaque to the containing block formatter; CSS-facing Taffy block runs are zero, backend scratch sizing is counted separately, and CSS2 tables plus css-position are byte-identical to baseline. See the [lane plan](2026-08-23_buckram_block_formatter_admission_execution_plan.md). |
| 1+2+7 | K5 positioning closure, Livery side | **Complete** | The named inventory is 26 pass / 0 fail, with 10 full-directory gains and no loss across css-position, CSS2 abspos, and CSS2 tables. See the [lane plan](2026-08-24_k5_positioning_closure_execution_plan.md). |
| 3 | K5h retained text frame | **Complete** | Accepted main pairs retained positioned-fragment translation with retained shaped text, rejects text-bearing leaf resize, and proves geometry-only leaf resize plus scroll exports against fresh final layouts. See the [current-main reconciliation](2026-08-24_k5h_retained_text_frame_reconciliation.md). |
| 4 | K5b grid static rectangle | **Complete** | Both `tests/grid_abspos.rs` receipts pass. Livery selects the K5a relationship, Buckram owns the narrow provider switch, and the grid callback chooses content box versus finalized grid area. See the [current-main reconciliation](2026-08-24_k5b_grid_static_rectangle_reconciliation.md). |
| 5+6 | K5d sizing and vertical-mode insets | **Complete** | All eight named files pass on current main and all nine native logical-inset receipts are green. The stale 33-shape count resolves to 36 honest failures owned by absent `shape-outside` exclusions in lane 12. See the [current-main reconciliation](2026-08-24_k5d_sizing_logical_insets_reconciliation.md). |
| 11 | Anonymous-table construction and sibling tables | **Complete** | The 059-098 family is 32 pass / 8 explained compositor residuals; column backgrounds, nested tables, block children, and sibling geometry have live receipts, with zero directory losses. |
| 13 | K6 corpus census | **Complete** | Corrected exact maps classify all 143 direct fragmentation passes as unverified and 14 more in guard directories. Six ignored block, inline, and table continuation contracts compile and Clippy clean. See the [current-main reconciliation](2026-08-24_buckram_k6_corpus_census_reconciliation.md). |
| 14 | css-text | **Complete** | The exact 1,964-file directory moved from 663 pass / 723 fail to 979 pass / 407 fail, with 316 fail-to-pass changes and zero pass-to-fail changes. All remaining failures are assigned by family and 16 focused native receipts are green. See the [current-main reconciliation](2026-08-24_livery_css_text_reconciliation.md). |
| 15 | Backgrounds, masking, images | **Complete** | The exact 1,981-file baseline had 786 failures. This lane repairs 77, assigns the 709 historical residuals, and explains 16 newly exposed false passes in the still-unimplemented embedded-object seam. Document and stylesheet-relative resources retain authored and resolved identities through the host boundary. See the [current-main reconciliation](2026-08-24_livery_paint_reconciliation.md). |
| 16 | Fonts and WOFF2 | **Complete** | Ordered family selection plus validated WOFF2-to-SFNT registration move the exact WOFF2 directory from 0 / 298 / 2 to 292 / 6 / 2 and css-fonts from 240 / 90 / 209 to 255 / 75 / 209, with 307 gains and no losses. All residuals are assigned in the [current-main reconciliation](2026-08-24_livery_fonts_woff2_reconciliation.md). |
| 19+20+21 | Harness and ledger | **Complete** | Accepted at `f9d5174b68d`. Synchronized GPU readback, failure buckets, and the expectation guard were already live. The corrected scorer checks both WPT fuzzy ranges and chosen-reference metadata; exact ledgers separate verified from `reference-unverified` passes. Full CSS exposes 3,650 prior false passes, gains 19 correctly selected-reference passes, and labels 157 surviving K6 coincidences. Two full candidate maps are identical. See the [current-main execution plan](2026-08-24_wpt_harness_ledger_execution_plan.md). |

## Wave 2, now unblocked

Lane 8 admitted independent block roots without widening flex or grid. These
lanes can now start independently from current main, subject to their own
plans and receipts.

| # | Lane | Owned surface | State |
|---|---|---|---|
| 9 | Intrinsic sizing contributions | **Complete** | K3m/K3q's box-keyed queries, validated cache, subtree contributions, and shrink-to-fit consumers remain live on current main. Buckram is 237/237, the focused live Livery receipt is green, and corrected css-sizing is 163 verified pass / 349 fail / 220 skip / 0 error. Normal-flow used sizing for content keywords remains an explicit K7 dispatch gap. See the [current-main reconciliation](2026-08-24_buckram_intrinsic_sizing_reconciliation.md). |
| 10 | Writing modes | **Complete** | Orthogonal auto inline sizing now uses the direct perpendicular block child's intrinsic block contribution. The exact writing-modes map moves from 186 to 193 verified passes with seven gains and zero losses. Text orientation, text combine, fragmentation, and algorithm-owned positioned/table/flex/grid residuals retain their named owners. See the [current-main reconciliation](2026-08-24_buckram_writing_modes_reconciliation.md). |
| 12 | Floats and shapes | **In progress** | Horizontal box-valued shapes use rectangular or circular rounded line-exclusion areas while margin-box placement remains separate. Relative blocks retain float state, and LTR/RTL boundaries mirror that state through descendant content coordinates. The latest exact shape-box map is 27 pass / 15 assigned failures; full CSS has 11 gains and 3 assigned false-pass losses. See the [rectangular](2026-08-25_buckram_float_shape_boxes_reconciliation.md), [rounded](2026-08-25_buckram_rounded_shape_boxes_reconciliation.md), [relative float-state](2026-08-25_buckram_relative_float_state_reconciliation.md), and [horizontal direction](2026-08-25_buckram_horizontal_float_direction_reconciliation.md) reconciliations. |
| 17 | Counters, lists, generated content | Livery `content`/`counter-*` cascade and marker boxes | Open |
| 18 | Flex and grid | **In progress** | The shorthand, CSSOM/reflection, flex-basis content sizing, automatic minimum, self-alignment, and bounded vertical row/row-reverse automatic-block-size slices are complete. Remaining order is mixed-writing-mode baseline and generated/pseudo inherited self-edge projection, shared `ex` and zero-percentage provenance, then grid auto tracks and template areas. See the [flex shorthand plan](../design_docs/2026-08-25_livery_flex_shorthand_plan.md). |

Lane 8 retained `with_out_of_flow_children_excluded`. It still has named
fallback and backend-sizing call sites, so deletion is not a condition for
Wave 2. Its stronger receipt is zero CSS-facing fallback for admitted
table/flow-root cases.

## Stop rules

- Stop on an unexplained WPT loss.
- Stop before using an archived lane as an integration base.
- Stop if an implementation crosses another lane's owned boundary without a
  written seam and a focused receipt.
- Stop if a backend scratch run is reported as a CSS-facing fallback.

## Done condition

The program closes when every row in both waves has a current-main plan and
receipt, the current K5 ledger has no unattributed red file, the corpus
ratchet has no unexplained loss, and each result is integrated into accepted
main from isolated, reproducible inputs.
