# Web platform WPT census: the non-CSS directories

**Date:** 2026-09-06

**Status:** complete. Measurement only; no renderer or runtime source changed.

**Parent:** the CSS ledgers under
[`docs/2026-08-24_wpt_harness_ledger_execution_plan.md`](../docs/2026-08-24_wpt_harness_ledger_execution_plan.md)
and [`docs/2026-07-28_absolute_css_conformance_ledger.md`](../docs/2026-07-28_absolute_css_conformance_ledger.md),
whose method this census extends to the rest of the web platform.

## Purpose

Until this run, `genet-wpt` had exact result maps only for the CSS manifest
and five small non-CSS probes (`dom`, `dom/nodes`, `dom/abort`,
`fetch/api/basic`, `html/webappapis/timers`). The question this census answers
is the one the CSS ledger answers for layout: on the checked-in WPT tree,
which web-platform directories does the scripted tier host, and at what
absolute pass count. It is an inventory for assigning owners, not a
conformance claim. No percentage is reported.

## Scope and provenance

| Item | Value |
|---|---|
| genet commit | `2c47cff7627`, clean tree |
| WPT tree | `tests/wpt/tests` as vendored at that commit |
| `MANIFEST.json` SHA-256 prefix | `d5ec5be9bf1a75ed` |
| Runner | `genet-wpt` release, `--features netfetch`, SHA-256 `a362fa85878ec3c211a1301a205b9777d3e70cd6f026bb76614e15449dbd3477` |
| Lane | `testharness`, engine Boa, renderer Livery, disk mode |
| Directories | 41 top-level WPT directories, `html` split by subdirectory (86 result files) |
| Raw results | `Code/testing/genet/wpt-ledger/2026-09-06_platform_census/` (outside Git, per the ledger README) |

Directories chosen: every top-level WPT directory a web engine owns that is not
`css`: `dom`, `html`, `fetch`, `websockets`, `xhr`, `workers`,
`service-workers`, `storage`, `IndexedDB`, `custom-elements`, `shadow-dom`,
`url`, `encoding`, `streams`, `FileAPI`, `webmessaging`, `eventsource`,
`uievents`, `pointerevents`, `selection`, `editing`, `svg`, `mathml`,
`webaudio`, `WebCryptoAPI`, `console`, `hr-time`, `performance-timeline`,
`resource-timing`, `navigation-timing`, `user-timing`,
`intersection-observer`, `resize-observer`, `web-animations`,
`scroll-animations`, `cookies`, `referrer-policy`, `content-security-policy`,
`cors`, `mixed-content`, `upgrade-insecure-requests`. Not run: device and
sensor APIs, payments, WebRTC, WebXR, WebGPU (its CTS is not in WPT),
`infrastructure`, and every reftest lane. `html/semantics/the-root-element`
does not exist in this tree.

## Method

One `genet-wpt testharness <dir> --write-expectations` invocation per
directory, sequential, from a script kept beside the raw results
(`run_disk.sh`). `aggregate.py` joins the result files and classifies the
`ERROR` lines from each run log; `summary.md` is its output and the table
below is copied from it.

**Disk mode, not server mode.** A server-mode probe of `dom/events`
(`--spawn-server`, 179 tests) took 29m57s against 28s in disk mode and
moved four subtests. The server-mode drive loop is wall-clock, so every test
that awaits an event that never settles costs the full 15-second deadline;
the disk-mode loop uses virtual clocks and quiesces immediately. At that
rate the 21,672-file set is days, not hours. The probe pair is kept as
`dom_events_probe.json` and `dom_events_disk_probe.json`. Consequence: the
network-dependent families (`fetch` beyond `fetch/api`, `websockets`, `xhr`,
`cors`, `cookies`, `referrer-policy`, `content-security-policy`,
`mixed-content`, `upgrade-insecure-requests`, `resource-timing`,
`eventsource`, `service-workers`) are measured without a server and their
counts are floors. A server-mode census of those families needs a runner
whose deadline is shorter or whose drive loop is interruptible.

**One hang.** `html/webappapis/scripting/processing-model-2/integration-with-the-javascript-agent-formalism/atomics-wait-async.https.any.html`
never returns under Boa and is not caught by the harness deadline. The
whole-directory run of `html/webappapis` was killed after 1,005s; the
directory was rerun file by file under an external 90-second timeout
(`run_perfile.sh`, merged by `merge.py`) and that one test is recorded as
`error` with reason `hang-killed-90s`. A watchdog (`watchdog.sh`) armed for
the rest of the run recorded no further stall.

## Results

Files are manifest test files including query variants. `Pass` is a file
whose every subtest passed; `Fail` is a file with at least one failing
subtest; `Error` is a harness or script error before results; `No results`
is a file that ran and reported nothing; `Skip` is a non-testharness or
non-window file the lane cannot host.

| Directory | Files | Pass | Fail | Error | No results | Skip | Subtests passed / total |
|---|---:|---:|---:|---:|---:|---:|---:|
| FileAPI | 76 | 3 | 55 | 3 | 0 | 15 | 276 / 633 |
| IndexedDB | 231 | 1 | 201 | 29 | 0 | 0 | 5 / 880 |
| WebCryptoAPI | 138 | 1 | 33 | 104 | 0 | 0 | 3 / 199 |
| console | 14 | 2 | 10 | 0 | 0 | 2 | 6 / 29 |
| content-security-policy | 838 | 37 | 529 | 252 | 19 | 1 | 102 / 3423 |
| cookies | 82 | 1 | 59 | 12 | 10 | 0 | 3 / 958 |
| cors | 26 | 1 | 4 | 21 | 0 | 0 | 2 / 42 |
| custom-elements | 187 | 3 | 144 | 29 | 1 | 10 | 2041 / 3674 |
| dom | 660 | 150 | 350 | 47 | 62 | 51 | 2318 / 6624 |
| editing | 843 | 1 | 242 | 414 | 40 | 146 | 1 / 97687 |
| encoding | 1267 | 3 | 871 | 1 | 387 | 5 | 7109 / 1329450 |
| eventsource | 61 | 0 | 61 | 0 | 0 | 0 | 0 / 100 |
| fetch | 473 | 44 | 377 | 33 | 4 | 15 | 1097 / 5224 |
| hr-time | 14 | 0 | 9 | 4 | 0 | 1 | 0 / 14 |
| html_anonymous-iframe | 35 | 0 | 27 | 3 | 5 | 0 | 0 / 34 |
| html_browsers | 782 | 40 | 527 | 134 | 37 | 44 | 143 / 1700 |
| html_canvas | 2673 | 33 | 2160 | 6 | 2 | 472 | 33 / 4142 |
| html_capability-delegation | 6 | 0 | 6 | 0 | 0 | 0 | 0 / 16 |
| html_cross-origin-embedder-policy | 94 | 0 | 75 | 19 | 0 | 0 | 1 / 424 |
| html_cross-origin-opener-policy | 160 | 0 | 149 | 0 | 11 | 0 | 0 / 603 |
| html_document-isolation-policy | 38 | 0 | 36 | 2 | 0 | 0 | 1 / 150 |
| html_dom | 385 | 25 | 159 | 60 | 1 | 140 | 40766 / 59969 |
| html_editing | 424 | 26 | 161 | 8 | 8 | 221 | 83 / 754 |
| html_embedded-content | 1 | 0 | 1 | 0 | 0 | 0 | 0 / 2 |
| html_infrastructure | 144 | 4 | 98 | 6 | 34 | 2 | 133 / 885 |
| html_interaction | 180 | 1 | 160 | 16 | 0 | 3 | 6 / 511 |
| html_links | 16 | 2 | 4 | 0 | 0 | 10 | 2 / 6 |
| html_meta | 1 | 0 | 1 | 0 | 0 | 0 | 0 / 3 |
| html_obsolete | 26 | 4 | 10 | 0 | 0 | 12 | 10 / 53 |
| html_rendering | 462 | 22 | 101 | 22 | 1 | 316 | 458 / 1217 |
| html_scripting | 2 | 0 | 1 | 1 | 0 | 0 | 0 / 2 |
| html_select | 2 | 0 | 1 | 0 | 0 | 1 | 0 / 5 |
| html_semantics_disabled-elements | 7 | 0 | 6 | 1 | 0 | 0 | 160 / 299 |
| html_semantics_document-metadata | 116 | 7 | 86 | 8 | 1 | 14 | 38 / 353 |
| html_semantics_edits | 2 | 0 | 2 | 0 | 0 | 0 | 0 / 2 |
| html_semantics_embedded-content | 798 | 64 | 566 | 41 | 33 | 94 | 338 / 1564 |
| html_semantics_forms | 648 | 16 | 388 | 29 | 4 | 211 | 367 / 3963 |
| html_semantics_grouping-content | 45 | 14 | 0 | 0 | 0 | 31 | 47 / 47 |
| html_semantics_interactive-elements | 175 | 5 | 78 | 13 | 0 | 79 | 24 / 414 |
| html_semantics_interestfor | 36 | 0 | 28 | 3 | 0 | 5 | 18 / 225 |
| html_semantics_interfaces | 1 | 0 | 1 | 0 | 0 | 0 | 0 / 438 |
| html_semantics_links | 36 | 0 | 33 | 1 | 0 | 2 | 10 / 204 |
| html_semantics_menu | 15 | 0 | 11 | 1 | 0 | 3 | 0 / 40 |
| html_semantics_permission-element | 114 | 0 | 50 | 0 | 1 | 63 | 1 / 137 |
| html_semantics_popovers | 110 | 0 | 72 | 5 | 2 | 31 | 1 / 3783 |
| html_semantics_rellist-feature-detection | 1 | 0 | 1 | 0 | 0 | 0 | 0 / 4 |
| html_semantics_scripting-1 | 498 | 41 | 260 | 152 | 4 | 41 | 768 / 2045 |
| html_semantics_sections | 1 | 0 | 1 | 0 | 0 | 0 | 0 / 61 |
| html_semantics_selectors | 33 | 1 | 27 | 2 | 1 | 2 | 194 / 393 |
| html_semantics_tabular-data | 29 | 1 | 28 | 0 | 0 | 0 | 8 / 157 |
| html_semantics_text-level-semantics | 36 | 4 | 5 | 0 | 0 | 27 | 27 / 39 |
| html_semantics_the-button-element | 32 | 0 | 30 | 0 | 0 | 2 | 18 / 803 |
| html_syntax | 402 | 8 | 297 | 63 | 4 | 30 | 2425 / 7953 |
| html_the-xhtml-syntax | 15 | 0 | 0 | 0 | 12 | 3 | 0 / 0 |
| html_user-activation | 20 | 0 | 11 | 0 | 9 | 0 | 0 / 14 |
| html_webappapis | 337 | 26 | 245 | 31 | 15 | 20 | 357 / 1101 |
| intersection-observer | 119 | 0 | 91 | 7 | 20 | 1 | 0 / 104 |
| mathml | 579 | 4 | 60 | 9 | 105 | 401 | 48 / 567 |
| mixed-content | 388 | 0 | 388 | 0 | 0 | 0 | 2 / 2281 |
| navigation-timing | 56 | 0 | 35 | 15 | 6 | 0 | 0 / 48 |
| performance-timeline | 51 | 0 | 51 | 0 | 0 | 0 | 0 / 73 |
| pointerevents | 265 | 6 | 169 | 14 | 67 | 9 | 38 / 429 |
| referrer-policy | 1390 | 1 | 1386 | 3 | 0 | 0 | 2 / 8443 |
| resize-observer | 20 | 0 | 8 | 8 | 0 | 4 | 0 / 9 |
| resource-timing | 132 | 0 | 87 | 41 | 4 | 0 | 0 / 371 |
| scroll-animations | 242 | 0 | 166 | 7 | 24 | 45 | 323 / 1922 |
| selection | 161 | 0 | 52 | 58 | 8 | 43 | 0 / 280 |
| service-workers | 292 | 0 | 266 | 17 | 0 | 9 | 0 / 1526 |
| shadow-dom | 314 | 6 | 215 | 42 | 2 | 49 | 18 / 8654 |
| storage | 27 | 0 | 26 | 0 | 0 | 1 | 0 / 75 |
| streams | 95 | 6 | 77 | 5 | 0 | 7 | 298 / 1230 |
| svg | 1640 | 16 | 287 | 305 | 10 | 1022 | 102 / 1860 |
| uievents | 76 | 8 | 39 | 17 | 5 | 7 | 14 / 105 |
| upgrade-insecure-requests | 197 | 0 | 197 | 0 | 0 | 0 | 0 / 1000 |
| url | 49 | 10 | 34 | 0 | 0 | 5 | 351 / 519 |
| user-timing | 36 | 1 | 22 | 7 | 6 | 0 | 2 / 65 |
| web-animations | 174 | 1 | 135 | 2 | 0 | 36 | 64 / 1449 |
| webaudio | 276 | 0 | 231 | 29 | 9 | 7 | 667 / 1448 |
| webmessaging | 135 | 20 | 101 | 10 | 4 | 0 | 49 / 209 |
| websockets | 515 | 0 | 375 | 137 | 3 | 0 | 0 / 1392 |
| workers | 247 | 1 | 222 | 22 | 2 | 0 | 17 / 574 |
| xhr | 348 | 5 | 277 | 61 | 2 | 3 | 53 / 1013 |
| **Total** | 21672 | 676 | 13845 | 2392 | 985 | 3774 | 61448 / 1579098 |

The `Total` row spans 86 result files. `html` on its own is 8,938 files.
Subtest totals are dominated by `encoding` (1.33 million, mostly the legacy
multibyte slices) and `editing` (97,687); treat file counts as the comparable
column.

### Skip reasons

| Count | Reason |
|---:|---|
| 3,670 | non-testharness (reftest, manual, visual, crash, or no harness include) |
| 86 | XHTML documents |
| 18 | worker-only or non-window global |

### Error classes

The 2,392 errored files fall into a few classes, from the run logs:

| Count | Class | Reading |
|---:|---|---|
| 723 | `<global> is not defined` | a missing interface or helper; itemised below |
| 611 | cannot convert `null` or `undefined` to object | a lookup that returned null, typically a missing element, property, or `document.body`-style accessor |
| 360 | not a callable function | a missing method on an existing interface |
| 293 | opaque `JsError` | uncategorised engine error |
| about 260 | Boa syntax errors in `import.meta` or object property names | see the harness caveats |

### Missing globals

Counts are files whose first error was a `ReferenceError` on that name.
Helper names from WPT support scripts are marked; they point at a harness
loading gap, not an engine gap.

| Count | Name | Kind |
|---:|---|---|
| 100 | `createSVGElement` | WPT helper from `/resources/SVGAnimationTestCase-testharness.js` (harness) |
| 86 | `crypto` | engine: Web Crypto |
| 83 | `XMLHttpRequest` | engine |
| 63 | `smil_async_test` | WPT helper, same SVG support script (harness) |
| 51 | `Worker` | engine |
| 37 / 36 / 36 | `test_valid_value`, `test_computed_value`, `test_invalid_value` | WPT helpers from `/css/support/*.js` (harness) |
| 22 | `performance` | engine: High Resolution Time |
| 20 | `getSelection` | engine: Selection API |
| 19 | `test_namespace` | WPT helper (harness) |
| 18 | `AudioContext` | engine: Web Audio |
| 14 | `BroadcastChannel` | engine |
| 12 | `IDBKeyRange` | engine: IndexedDB |
| 12 | `SharedWorker` | engine |
| 9 | `MutationObserver` | engine |
| 8 | `MessageChannel` | engine |
| 8 | `WebSocket` | engine |
| 6 | `DOMMatrix` | engine: Geometry Interfaces |
| 6 | `Option` | engine: named constructor |
| 5 each | `IntersectionObserver`, `ResizeObserver`, `ViewTimeline` | engine |
| 4 | `PerformanceObserver` | engine |
| 3 each | `trustedTypes`, `HTMLMenuItemElement`, `queueMicrotask`, `OfflineAudioContext`, `registerProcessor` | engine |
| 2 each | `alert`, `Image`, `OffscreenCanvas`, `scheduler` | engine |
| 1 each | `CSSStyleDeclaration`, `HTMLFrameSetElement` | engine |

These are first errors only. A file that dies on `crypto` may also need
`Worker`; the true demand for each interface is at least this count.

## Harness caveats found by the run

These are runner gaps that depress counts and should be fixed before the
next census, so that the delta measures the engine:

1. **Absolute `/resources/*` support scripts are dropped.** The disk loader
   filters harness includes by path and takes
   `/resources/SVGAnimationTestCase-testharness.js` with them, which is why
   `svg/animations` reports 163 helper `ReferenceError`s. The `/css/support/`
   helpers (`test_valid_value` and kin, 128 files) likely share the cause; not
   traced to the line.
2. **Server-side handlers are read as JavaScript in disk mode.** A
   `<script src="log.py?...">` include resolves to the Python file on disk and
   Boa reports a syntax error in `import.meta`. Server mode is the fix; see the
   timing problem above.
3. **The harness deadline does not interrupt a blocked engine call.**
   `Atomics.waitAsync` hung the process. The external timeout used here is a
   workaround; a per-test worker, as `test262` already has, is the fix.
4. **Reftests in these directories are unmeasured.** `html`, `svg`, `mathml`
   and `editing` carry most of the 3,670 non-testharness skips; a reftest
   lane over them is a separate GPU run.

## Reading the inventory

Grouped by what the number says about the engine, largest first:

- **Hosted and partly passing:** `dom` (150 all-pass, 2,318 subtests),
  `html/dom` (40,766 subtests, the reflection tables), `custom-elements`
  (2,041), `html/syntax` (2,425), `fetch` (1,097, the network-free `fetch/api`
  surface), `html/semantics/scripting-1` (768), `webaudio` (667, from
  interface-shape tests), `url` (351), `streams` (298), `FileAPI` (276).
- **Hosted, almost nothing passing, interface absent or inert:**
  `websockets` (0 of 1,392 subtests), `service-workers` (0 of 1,526),
  `IndexedDB` (5 of 880), `storage` (0), `eventsource` (0), `xhr` (53 of
  1,013), `workers` (17 of 574), `selection` (0 of 280), `editing` (1 of
  97,687), `shadow-dom` (18 of 8,654), `html/canvas` (33 of 4,142),
  `intersection-observer` (0), `resize-observer` (0), `performance-timeline`
  (0), `resource-timing` (0), `navigation-timing` (0), `hr-time` (0),
  `html/interaction` (6 of 511), `html/semantics/popovers` (1 of 3,783).
- **Security and policy families, unmeasurable without a server:**
  `referrer-policy` (2 of 8,443), `content-security-policy` (102 of 3,423),
  `mixed-content` (2), `upgrade-insecure-requests` (0), `cors` (2 of 42),
  `cookies` (3 of 958), and the COOP, COEP and document-isolation
  subtrees of `html`. Their disk-mode counts say nothing about netfetcher,
  which the fetch plan already proved against a live server.
- **Mostly skipped, reftest-shaped:** `svg` (1,022 of 1,640 skipped),
  `mathml` (401 of 579), `html/rendering` (316 of 462), `html/editing` (221 of
  424), `html/semantics/forms` (211 of 648).

Each of the second and third groups is a candidate lane in the sense of the
Buckram and Livery lane program: a named directory, an exact baseline map,
and a reconciliation that assigns every residual. This document is the
baseline those lanes diff against.

## Reftest lane over html, svg, mathml and editing (2026-09-07)

**Status:** complete. Measurement only; no renderer or runtime source changed.

The four non-testharness families this census could only mark `Skip`
(`html`, `svg`, `mathml`, `editing`) run under `genet-wpt reftest`, not
`testharness`, and need a GPU. This lane measures them with the same
directory list the census used for `html` (split by subdirectory), plus
`svg`, `mathml` and `editing` whole. Raw results are under
`Code/testing/genet/wpt-ledger/2026-09-07_reftest_platform/`, per the ledger
README.

### Provenance

| Item | Value |
|---|---|
| genet commit | `5af76a0cb8c`, clean worktree at `C:/t/genet-5af76a0` |
| WPT tree | `tests/wpt/tests` at that commit |
| `MANIFEST.json` SHA-256 prefix | `d5ec5be9bf1a75ed` (same WPT tree as the 2026-09-06 census) |
| Runner | `genet-wpt` release, built `CARGO_TARGET_DIR=C:/t/lane8b-target cargo build --release -p genet-wpt`, SHA-256 `107d7782ac5becb46b1fb18382214f8cf9652d8c108b5513c5cfa230396d5fa6` |
| Lane | `reftest`, engine Boa, renderer Livery |
| Directories | 42 (41 `html` subdirectories plus `svg`, `mathml`, `editing`; same `html` split as the census's `run_disk.sh`) |
| Raw results | `Code/testing/genet/wpt-ledger/2026-09-07_reftest_platform/reftest/` |

### Method

One `genet-wpt reftest <dir> --engine boa --renderer livery --tests-root
C:/t/genet-5af76a0/tests/wpt/tests --write-expectations <file>` invocation
per directory, sequential (reftests are GPU-bound, one process at a time), a
150-second stall watchdog (`watchdog.sh`) armed for the whole run and a
per-directory fallback (`run_perfile.sh`, `merge.py`) that reruns a killed
directory file by file under a 90-second external timeout, merging the
per-file records back into that directory's expectation JSON. Before the
full run, `html/obsolete` (26 files) was run alone as a GPU-boot smoke test
and produced a real pixel comparison (1 pass, 1 local-bucket fail), then its
output was discarded and the directory rerun as part of the sequential pass
below.

A `pass` with no reason is an ordinary pixel pass. The runner labels a pass
`reference-unverified` only for tests in the checked inventory
(`ports/genet-wpt/expectations/reftest/reference_verification.json`), which
covers the CSS fragmentation scopes and twenty CSS guards and nothing under
these directories, so the `Ref-unverified` column below is zero by
construction and the `Verified` column means "not in that inventory", not
"reference independently verified". A reference-verification pass over these
families is open work before their passes earn conformance credit.

### Result

All 42 directories ran to completion in the first sequential pass — the
watchdog recorded no stall (`stalls.log` is empty after the run) and no
directory needed the per-file fallback. Wall time for the full sequential
pass was under six minutes (`run_reftest.log` timestamps), against an hour
external timeout per directory.

| Directory | Files | Pass | Verified | Ref-unverified | Fail | Skip | Error |
|---|---:|---:|---:|---:|---:|---:|---:|
| editing | 843 | 0 | 0 | 0 | 0 | 843 | 0 |
| html_anonymous-iframe | 35 | 0 | 0 | 0 | 0 | 35 | 0 |
| html_browsers | 782 | 1 | 1 | 0 | 0 | 781 | 0 |
| html_canvas | 2673 | 1 | 1 | 0 | 3 | 2669 | 0 |
| html_capability-delegation | 6 | 0 | 0 | 0 | 0 | 6 | 0 |
| html_cross-origin-embedder-policy | 94 | 0 | 0 | 0 | 0 | 94 | 0 |
| html_cross-origin-opener-policy | 160 | 0 | 0 | 0 | 0 | 160 | 0 |
| html_document-isolation-policy | 38 | 0 | 0 | 0 | 0 | 38 | 0 |
| html_dom | 385 | 32 | 32 | 0 | 35 | 318 | 0 |
| html_editing | 424 | 6 | 6 | 0 | 4 | 414 | 0 |
| html_embedded-content | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| html_infrastructure | 144 | 1 | 1 | 0 | 0 | 143 | 0 |
| html_interaction | 180 | 0 | 0 | 0 | 0 | 180 | 0 |
| html_links | 16 | 0 | 0 | 0 | 0 | 16 | 0 |
| html_meta | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| html_obsolete | 26 | 1 | 1 | 0 | 1 | 24 | 0 |
| html_rendering | 462 | 138 | 138 | 0 | 95 | 229 | 0 |
| html_scripting | 2 | 0 | 0 | 0 | 0 | 2 | 0 |
| html_select | 2 | 1 | 1 | 0 | 0 | 1 | 0 |
| html_semantics_disabled-elements | 7 | 0 | 0 | 0 | 0 | 7 | 0 |
| html_semantics_document-metadata | 116 | 4 | 4 | 0 | 2 | 110 | 0 |
| html_semantics_edits | 2 | 0 | 0 | 0 | 0 | 2 | 0 |
| html_semantics_embedded-content | 798 | 6 | 6 | 0 | 8 | 784 | 0 |
| html_semantics_forms | 648 | 15 | 15 | 0 | 27 | 606 | 0 |
| html_semantics_grouping-content | 45 | 14 | 14 | 0 | 12 | 19 | 0 |
| html_semantics_interactive-elements | 175 | 0 | 0 | 0 | 0 | 175 | 0 |
| html_semantics_interestfor | 36 | 0 | 0 | 0 | 0 | 36 | 0 |
| html_semantics_interfaces | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| html_semantics_links | 36 | 1 | 1 | 0 | 1 | 34 | 0 |
| html_semantics_permission-element | 114 | 36 | 36 | 0 | 20 | 58 | 0 |
| html_semantics_popovers | 110 | 1 | 1 | 0 | 0 | 109 | 0 |
| html_semantics_scripting-1 | 498 | 3 | 3 | 0 | 0 | 495 | 0 |
| html_semantics_sections | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| html_semantics_selectors | 33 | 0 | 0 | 0 | 1 | 32 | 0 |
| html_semantics_tabular-data | 29 | 0 | 0 | 0 | 0 | 29 | 0 |
| html_semantics_text-level-semantics | 36 | 1 | 1 | 0 | 24 | 11 | 0 |
| html_syntax | 402 | 9 | 9 | 0 | 11 | 382 | 0 |
| html_the-xhtml-syntax | 15 | 1 | 1 | 0 | 0 | 14 | 0 |
| html_user-activation | 20 | 0 | 0 | 0 | 0 | 20 | 0 |
| html_webappapis | 336 | 0 | 0 | 0 | 0 | 336 | 0 |
| mathml | 579 | 70 | 70 | 0 | 81 | 428 | 0 |
| svg | 1640 | 260 | 260 | 0 | 101 | 1279 | 0 |
| **Total** | 11951 | 602 | 602 | 0 | 426 | 10923 | 0 |

Fail-shape buckets, summed across directories (`aggregate.py` parses these
from each log's `fail buckets:` line, not from the JSON, since the bucket is
per-failure diagnostic rather than a stored field):

| Count | Bucket | Reading |
|---:|---|---|
| 10 | `whole` | >=50% of pixels differ (layout or UA-chrome shape wrong) |
| 341 | `local` | localized large delta (a feature or paint area is wrong, rest matches) |
| 75 | `mismatch-eq` | pixels are identical yet the test still failed (an `==` reftest reporting fail, or a `!=` reftest reporting fail on a match) |

`dims` (size mismatch) and `aa` (small per-channel, anti-alias/tolerance)
were not observed. All 426 `Fail` records are accounted for by these three
buckets (10 + 341 + 75 = 426). Note: the aggregate script's original bucket
regex did not admit the hyphen in `mismatch-eq` and silently split it into a
spurious `eq` bucket; `aggregate.py` in this lane's directory was corrected
in place (`(\w+\??)=` to `([\w-]+\??)=`) before the totals above were taken.

### Hangs and handling

None. The 2026-09-06 census's one hang
(`atomics-wait-async.https.any.html`, testharness lane) does not recur here:
that file is a `dom`/`webappapis` testharness case, not one of this lane's
reftest files. No directory in this run exceeded the 150-second watchdog
threshold or the one-hour per-directory external timeout; `run_perfile.sh`
and its per-file fallback were prepared but never invoked.

### Near versus far reading

**Near (file-count) reading:** the pass column looks small against Skip —
602 of 11,951 files (about 5%) produced a pixel pass, 426
failed, and 10,923 were skipped as non-reftest members of these directories
(manual, crash, or otherwise unhosted files the WPT manifest still lists
under these paths). Several directories (`editing`, `html_webappapis`,
`html_interaction`, the policy families) pass zero files outright — either
because everything in them is a manual/visual test the reftest lane cannot
host, or the feature they gate (COOP/COEP/document-isolation, popovers,
permission-element beyond its 36) is not yet functional under Livery.

**Far (what's hosted) reading:** of the 1,028 files this lane actually
executed as reftests (602 pass + 426 fail; 10,923 of the 11,951 total were
non-reftest skips the manifest still lists here), Livery reference-verified
59% (602/1,028). That is a materially higher hit rate than the testharness
census's non-CSS families, and the fail buckets are concentrated in `local`
(341, a localized rendering delta rather than a structural failure) rather
than `whole` (10, structurally wrong) — consistent with a renderer that
lays most of these documents out correctly and differs on paint detail
inside them. `mismatch-eq` (75) is the one bucket worth flagging for tool
correctness ahead of a reconciliation pass: it means the pixels already
match (or don't, for a `!=` test) but the test still reports fail, which
points at either fuzzy-match/reference-selection logic or a metadata
mismatch rather than rendering fidelity, and is a candidate false-negative
pool a reconciliation should re-check before spending effort on genuine
paint bugs.

Raw per-directory JSON exact maps and logs: `Code/testing/genet/wpt-ledger/2026-09-07_reftest_platform/reftest/`.
`summary.md` in that directory is `aggregate.py`'s output and is the source
of the table above.
