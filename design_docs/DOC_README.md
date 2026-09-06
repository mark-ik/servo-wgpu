# genet Documentation Index

The canonical index for `genet/design_docs/`, per [`DOC_POLICY.md`](DOC_POLICY.md)
§6. If any other index disagrees with this file, this file wins.

Founded 2026-08-24, when the canonical policy core was distributed across the
workspace and the component documents for inker, nematic and verso-tile were
repatriated here from mere.

> **Boundary correction landed, 2026-09-03:** Cambium and Genet's upper
> application components moved to Mere under
> `mere/design_docs/mere_docs/implementation_strategy/2026-09-02_platform_boundary_and_repository_topology_plan.md`.
> Their documents moved with the code and are indexed in Mere. Genet retains
> web-platform implementation, observable behavior, raw host contracts, WPT,
> and a minimal engine host.

> **Read the policy's "Two doc homes" section first.** This repository also has
> a flat `docs/` directory of 166 older engine documents. The current engine
> work section below gives entry points into that corpus; it is not a complete
> inventory or a migration. The split is deliberate, and migration remains
> open, unscheduled work. New documents go here.

## Required reading order

1. The root [`README.md`](../README.md) — what genet is.
2. [`DOC_POLICY.md`](DOC_POLICY.md) — the shared core plus this repo's addendum,
   including the `docs/` boundary and the smolweb split.
3. The section you are working in, below. genet has no topic area root left:
   the last three moved to mere with their code on 2026-09-03, and active
   plans sit flat in `design_docs/`.

## Current engine work (2026-09-05)

Use the linked plan's current gate and dated receipts before implementing a
lane. Historical corpus totals are measurements of their named source, not a
fresh baseline for this checkout. This map also covers selected plans in the
older `docs/` corpus without changing their location or governance.

| Work | Current boundary and next proof |
|---|---|
| [Font fallback](2026-09-04_common_script_font_fallback_plan.md) | Windows T1 is accepted with focused regressions and Ortet readback. Consumer revision adoption, upstream disposition, and other-platform measurements remain open. |
| [Ortet](2026-09-03_ortet_founding_plan.md) | O0-O4 landed. Native accessibility and the raw wasm canvas host have platform receipts; the wasm cone excludes AccessKit and winit. Browser http(s) fetching, stable accessibility IDs, and publication remain separate. |
| [K6 fragmentation](../docs/2026-08-15_buckram_k6_fragmentation_execution_plan.md) | K6a typed inputs and K6b's retained context/token model landed. All 6,077 named records match the frozen pre-K6 baseline. Live formatter continuation and multicol geometry remain K6c work. |
| [Flex and grid, Row 18](2026-08-25_livery_flex_shorthand_plan.md#row-18-closure-and-remaining-work) | Bounded vertical-flex slices landed. Remaining work includes mixed-writing-mode baselines, generated/pseudo self edges, shared font metrics and percentage provenance, then a measured grid inventory. The dated guard review records 21 pre-K6 flex/grid pass-to-fail cases. |
| [Floats and shapes, Row 12](../docs/2026-08-25_buckram_horizontal_float_direction_reconciliation.md) | Horizontal box-shape and direction slices landed. Vertical/orthogonal transforms and remaining shape families retain their own unproved boundaries. |
| [Counters, lists and generated content, Row 17](../docs/2026-08-21_buckram_livery_lane_program_plan.md#wave-2-now-unblocked) | Open inventory. Name the computed-content, counter-scope and marker-box consumers and a bounded execution gate before implementation. |
| [K7 foundational sizing and dispatch](../docs/2026-07-26_buckram_css_layout_engine_plan.md#k7-foundational-sizing-and-dispatch-closure) | Reconcile landed sizing fixes with remaining deferrals before choosing a slice. Fragmentainer-dependent work consumes K6; final closure includes deleting CSS-facing Taffy block dispatch. |
| [WPT harness and ledger](../docs/2026-08-24_wpt_harness_ledger_execution_plan.md) | Exact scorer and reference-verification gates landed. Freeze a fresh candidate runner for new work; unsupported test/reference agreement earns no conformance credit. |
| [Host contract ownership](../docs/2026-08-14_web_platform_host_contract_plan.md) | Genet owns retained session contracts; Mere owns surface orchestration and product adapters. The older S0-S5 receipts need a consumer-side status refresh before resuming those lanes. |

The [Buckram master](../docs/2026-07-26_buckram_css_layout_engine_plan.md)
defines ownership and the [lane program](../docs/2026-08-21_buckram_livery_lane_program_plan.md)
assigns residuals. The linked execution plans carry their current gate; a
completed corpus census or bounded slice does not close its enclosing feature.

## ortet — the raw host

- [ortet_founding_plan](2026-09-03_ortet_founding_plan.md) (**O0, O1 and O4
  landed 2026-09-03; O2 Windows accessibility accepted 2026-09-05;
  publication and O3 web target open**: the one headed port that proves the
  engine runs without Mere, over `genet-winit-host`, `genet-render-host`,
  `genet-documents`' Livery lane and `document-session-api`, with a cone
  witness that forbids every Mere crate and a self-driven frame receipt.
  Replaced Pelt as Genet's default host when Pelt moved to Mere.)

## fleece — reader extraction

- [fleece_preservation_contract_plan](2026-09-05_fleece_preservation_contract_plan.md)
  (**active 2026-09-05**: preserves Fleece extraction identity, wire values, and
  arbitrary passage anchors for hashing, peer transfer, reopen, and annotations;
  adds named Web Annotation, structured-data, HTML, provenance, validation, and
  accessibility conformance gates with caller-owned capture and custody.)
- [fleece_followthrough_plan](2026-08-26_fleece_followthrough_plan.md)
  (**complete 2026-08-26**: the
  `genet-extract` shim is retired; retained static/scripted hosts activate
  Fleece-generated Text Directives with element fallback, indication, scrolling,
  one-fetch behavior, and script-visible URL privacy; Mere crawl and Gazette now
  consume supplied documents while eidetic-search drops its misplaced edge.
  Focused automated gates and the headed activation/indication receipt are
  green.)

## layout and styling

- [common_script_font_fallback_plan](2026-09-04_common_script_font_fallback_plan.md)
  (**Windows repair accepted 2026-09-05**: the paired Parley/Fontique patch
  queries actual text after authored faces fail. Both disclosure markers now
  paint through system fallback in Ortet, while Latin and explicit-font controls
  remain verified. Downstream product pins, upstream disposition, and other
  platforms retain separate gates.)

- [livery_flex_shorthand_plan](2026-08-25_livery_flex_shorthand_plan.md)
  (**complete flex-shorthand slice; Row 18 remains in progress**: Livery now
  expands `flex` and `flex-flow` into the longhand style fields already lowered
  to Taffy. The exact 1,358-file flexbox map records 115 gains, three assigned
  downstream false-pass losses, and the numeric-basis parser repair forced by
  the first candidate. The current eight-input Taffy seam is published and
  consumed as `genet-taffy 0.14.0`, published and tagged
  `genet-taffy-v0.14.0` at the Row 18 closure; 0.13.1 was the eight-input
  seam before it.)

## cambium — the desktop host

- Cambium, Workbench and `mere-surface-api` left genet for mere on 2026-09-03
  under the platform boundary plan; the `host_ui_zoom_plan` and the
  `workbench_component_plan` travelled with them and are now in mere's
  `design_docs/`.

## inker_docs/, nematic_docs/, verso_docs/ — moved to mere

- The engine-management layer left genet for mere on 2026-09-03 under the
  platform boundary plan: `inker`, `document-canvas`, the scrying/graft/weld
  engine adapters, `verso-tile`, `nematic`, `illume`, `errand` and `tinct`.
  These three area roots travelled with their code and are now in mere's
  `design_docs/`.

## codebase structure

- [orchestrator_decomposition_plan](2026-08-28_orchestrator_decomposition_plan.md)
  (**seven extraction phases complete 2026-08-29; follow-through recorded
  through 2026-09-02**: the original phases preserved behavior. Later entries
  separately record the layout transaction split, sizing repairs and their
  bounded receipts. The two-builder assessment recommends factoring shared
  table helpers if pursued. Pelt's code now lives in Mere.)

## archive_docs/ — completed plans

Per policy §4 and §8: a plan moves here once complete and once its open
points have a home elsewhere. Links into a moved plan are repaired in the
same session; links out of it are rewritten for its new depth.

- [2026-09-02/fleece_standards_adoption_plan](archive_docs/2026-09-02/2026-08-24_fleece_standards_adoption_plan.md)
  (**complete 2026-08-25, archived 2026-09-02**: Fleece 0.2 shipped canonical
  DOM-text coordinates, W3C Text Quote and Text Position selectors, and a Text
  Fragment projection; 0.3 hardened JSON-LD syntax harvesting and HTML
  Microdata; 0.4 added ordered Open Graph grouping, DOM document links, and
  semantic HTML table grids and header associations. No open points carried.)
- [2026-09-02/knot_evaluation_export_plan](archive_docs/2026-09-02/2026-06-12_knot_evaluation_export_plan.md)
  (**reconciled and complete for the first production capability set
  2026-07-27, archived 2026-09-02**; `include` closed, TOFU location rehomed to
  the fidelity plan, badge default carried by the block resolver plan. `include` transclusion fences over errand's smolweb transports,
  `lua eval` / `rhai eval` script fences via the `BlockEvaluator` slice,
  `to_gemtext` and gophermap exporters, the Knot production effect bridge,
  Turnstone consent, and the sealed attributable resolve cache. The production
  Knot adapter supplies anonymous HTTP(S) plus read-only Gemini, Gopher,
  Finger, Spartan, Nex and Guppy; Titan stays excluded.)

## Working principles

- **New docs go in `design_docs/`, never `docs/`.** See the policy's two-homes
  section for why both exist and what it would cost to merge them.
- **The smolweb boundary is spec versus use.** What a protocol *is* belongs to
  the smolweb workspace; what a browser *does with it* belongs here. Cite
  across the boundary by path — relative links do not survive it.
- **Prefer runtime verification to extended static tracing.** If runtime
  diagnostics are blocked, surface that blocker early rather than continuing to
  read code.
- **State the exact standards layer implemented.** Selector values are not the
  Web Annotation Protocol; JSON-LD syntax harvesting is not JSON-LD processing;
  raw URL attributes are not resolved links. Keep these boundaries visible in
  public types and receipts.
- **Trace the values the consumer actually receives.** Input normalization,
  script itemization and host projection can change the value under review.
  A diagnostic records that effective value and the selected implementation;
  an initially passing control is evidence to interpret, not a faulty test by
  definition.
- **Keep current gates synchronized after integration.** Update the parent,
  execution plan and this work map together. Preserve old receipts with their
  dates and source identities. When a dependency move retires a positive
  control, replace its exercised path or explicitly reopen that proof.
- **Freeze dependency resolution with a measured runner.** `Cargo.lock` is
  intentionally ignored here. Retain the generated lockfile, its digest,
  target/features and local-override facts with the source and binary receipt.
- **Verify paired forks from a standalone consumer.** Cargo root patches are
  not inherited by downstream workspaces. A coupled dependency must travel with
  its caller; prove that resolution before refreshing product revisions.
- **Parallel work needs commit fences as well as file fences.** Pin one base,
  give each worker a disposable detached worktree and disjoint write paths,
  inspect staged paths before committing, and remove the worktree immediately
  after integration.

## Status

Founded 2026-08-24; current work map reconciled 2026-09-05. The index covers
five flat plans and two archived plans. All three former component area roots
now live in Mere. The older `docs/` corpus has selected execution entry points
above; its full migration and governance remain deferred under the policy's
local addendum.
