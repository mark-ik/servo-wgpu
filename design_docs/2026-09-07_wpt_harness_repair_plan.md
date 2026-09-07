# WPT harness repair: process isolation, disk-mode includes, server-mode drive

**Date:** 2026-09-07

**Status:** landed 2026-09-07. Runner-only change: `ports/genet-wpt` and the
ledger. No engine, renderer or script-runtime source touched.

**Parent:** [`2026-09-06_web_platform_wpt_census.md`](2026-09-06_web_platform_wpt_census.md),
whose "Harness caveats found by the run" section names four gaps. This plan
closes the first three. Caveat 4 (reftests in these directories are unmeasured)
is a separate GPU lane and is untouched here.

## Purpose

The 2026-09-06 census is the baseline every non-CSS web-platform lane will diff
against. Three of the four caveats it recorded are runner defects, not engine
gaps, and each one depresses or distorts that baseline:

1. a test that blocks inside the engine hangs the whole run, so the directory
   containing it cannot be measured at all;
2. the disk loader drops WPT support scripts whose file name merely *ends*
   with `testharness.js`, and hands Python server handlers to the JavaScript
   engine, so hundreds of files report a helper `ReferenceError` or an
   `import.meta` syntax error that says nothing about the engine;
3. server mode spends its full deadline on every test whose only surviving
   work is a timer, which makes a server-mode census of the network-dependent
   families cost days rather than hours.

Fixing them makes the next census delta a measurement of the engine.

## Findings

Dated 2026-09-07, verified in this checkout at `5af76a0cb8c`.

### F1 — the include filter matched a suffix, not a file name

`harness::is_harness_src` (`ports/genet-wpt/src/harness.rs`) tested
`src.ends_with("testharness.js")`. WPT ships support scripts named
`<Something>-testharness.js`; `/resources/SVGAnimationTestCase-testharness.js`
is the one the census caught, and it is the sole definition of
`createSVGElement` (100 files) and `smil_async_test` (63 files). The path
resolution in `harness::resolve` was already correct for `/`-absolute srcs — the
file never reached it. The fix compares the last path segment against the three
names the host surface actually supplies.

The `/css/support/*.js` helpers named in the census (`test_valid_value` and
kin, about 128 files) do **not** share this cause: their file names
(`parsing-testcommon.js`, `computed-testcommon.js`) never matched the filter.
Those files are consumed from `html` and `svg` subtrees and their residual is
re-measured, not assumed, in the diff below.

### F2 — a `.py` include is a server-side handler, not JavaScript

A `<script src="…/echo.py?content=x">` resolves on disk to the Python source.
The engine parsed it and reported a syntax error inside `import.meta`, which
the census had to classify as an engine error class. Disk mode now skips such
an include and the loader records that it did, so the file's reason names the
missing server instead.

### F3 — the drive deadline cannot interrupt a blocked engine call

`drive_virtual` and `drive_wall` check a wall-clock deadline between turns.
A call that never returns to the loop — `Atomics.waitAsync` under Boa, the
census's known case — is never checked. The census worked around it with an
external per-file timeout; the runner now isolates each test in a worker
subprocess, exactly as `test262` has since its async lane was re-enabled.

### F4 — the worker's result does not fit in a pipe buffer

Found while building the isolation, and worth recording because it is the
failure the naive port produces. A worker's encoded outcome carries every
subtest name; for `dom/nodes/Element-classlist.html` (1,420 subtests) that is
far past the OS pipe buffer. Reading the child's stdout only *after* it exits —
the shape `test262`'s worker loop uses, where the payload is two words —
deadlocks the pair: the child blocks writing, the parent blocks waiting, and
the parent's timeout then reports a false `hang-killed`. The first isolated
`dom` run recorded exactly that on the two largest files. The parent now drains
stdout on its own thread.

### F5 — server mode sleeps out the testharness timeout

`drive_wall` breaks only when nothing at all is outstanding. `testharness.js`
arms its own timeout timer at load, so `next_timer` is `Some` for the whole
run of any test that does not finish, and the loop sleeps to it — the full
deadline per test. That is the 29m57s / 179-test figure the census recorded
against 28s in disk mode. When no fetch is in flight and no frame is wanted,
the loop can advance its clock to the next timer instead of sleeping to it,
which is what the disk loop already does.

## Phases

### H1 — per-test process isolation for `testharness`

`testharness` gains a `testharness-one <url> --test-path <file>` worker
command. The parent classifies the cheap skips itself (non-testharness files
and XHTML, 3,700-odd of the census's 3,774) and spawns one worker per remaining
test, `--jobs` in flight, each bounded by `--timeout`. A worker that outlives
its timeout is killed and recorded `error` / `hang-killed`; one that dies
without reporting is `error` / `worker-crashed`. The single-process path
survives behind `--in-process`.

Both shapes call one per-test body (`run_one`), so skip classification,
document load, evaluation and scoring cannot drift between them.

**Done-conditions**

- The known hang is recorded, not survived: the
  `integration-with-the-javascript-agent-formalism` directory completes and
  `atomics-wait-async.https.any.html` carries reason `hang-killed`. **Met.**
- The result map is unchanged for tests that do not hang: `--in-process` and
  `--jobs 8` produce identical `--write-expectations` JSON on `console`, `url`,
  `storage` and `dom/abort`, and the isolated `dom` map is identical to the
  2026-09-06 baseline map. **Met.**
- `--jobs` does not change results: the identity above holds at 8 workers.
  **Met.**

### H2 — disk-mode `<script src>` loading

Only `testharness.js`, `testharnessreport.js`, `testharnesscss.css` and the
synthesized `testdriver-vendor.js` are filtered; every other include resolves
against the tests root (for a `/`-absolute src) or the test directory and
loads. A `.py` include is skipped as a server-side handler and the file's
reason becomes `server-side-handler`.

**Done-conditions**

- `svg/animations` no longer reports `createSVGElement is not defined`. **Met**
  (`animVal-basics.html` moved `error` → `fail`, 0/1).
- Unit coverage pins both predicates against the names that regressed:
  `only_the_harness_itself_is_filtered_from_script_src` and
  `server_side_handlers_are_not_loaded_as_javascript`. **Met.**
- Every movement in the re-run census is attributable to one of the two.
  See the diff below.

### H3 — server-mode drive loop

`--drive-deadline <secs>` sets the per-test ceiling (default 15, unchanged).
It is a process-global set once from the parsed arguments rather than an
argument threaded through six `run_test*` entry points, and the worker inherits
it from the parent.

`drive_wall` keeps a clock that tracks elapsed wall time while the network is
in flight, and when a turn does nothing with no pending fetch, no rAF callback
and no animation, advances that clock to the next timer's due time instead of
sleeping to it. The wall-clock deadline still bounds a runaway self-rearming
timer.

**Done-conditions**

- Disk-mode scoring is untouched: `drive_virtual` is unchanged and the disk
  census reproduces the 2026-09-06 numbers everywhere the two harness fixes do
  not apply. **Met.**
- The deadline is settable from the command line. **Met.**
- A server-mode re-measurement of `dom/events` is **not** claimed here: it
  needs a live `wpt serve` and belongs to whichever lane opens the
  network-dependent families. The change is structural.

### H4 — re-run the census and diff

Same directory list, same disk mode, same engine and renderer, fresh release
runner. Every per-directory status movement must be explainable as one of the
three repairs.

**Done-conditions**

- 82 result files written — the 79 of the shared `DIRS` list plus the three
  `tail_run.log` directories, matching the 2026-09-06 set file for file — and
  no directory killed by the watchdog.
- Per-directory transition counts recorded, each class attributed.
- Anything unattributed is listed rather than absorbed.

## Scope and provenance

| Item | Value |
|---|---|
| genet commit | `5af76a0cb8c` |
| WPT tree | `tests/wpt/tests` as vendored at that commit |
| `MANIFEST.json` SHA-256 prefix | `d5ec5be9bf1a75ed` |
| Runner | `genet-wpt` release, `--features netfetch`, SHA-256 `827d7a7df1980c5d3a63a7ea30a31e008e36459312fbf58b5d7413a4caf93a1c` |
| Lane | `testharness`, engine Boa, renderer Livery, disk mode, `--jobs 8 --timeout 300` |
| Directories | the 2026-09-06 set: 79 from `run_disk.sh` plus 3 from `tail_run.log` (82 result files) |
| Raw results | `Code/testing/genet/wpt-ledger/2026-09-07_harness_repair/` (outside Git) |
| Baseline | `Code/testing/genet/wpt-ledger/2026-09-06_platform_census/` |

## Results

All 82 result files were written and nothing was killed by the watchdog. The
79 driver directories took 3,172 s of wall time against 6,114 s on 2026-09-06,
and that comparison flatters the old run: its 6,114 s includes 1,005 s spent
hanging in `html/webappapis` before it was killed, and excludes the per-file
rerun that directory then needed. Here it completes in 324 s.

### Totals

| | 2026-09-06 | 2026-09-07 | Delta |
|---|---:|---:|---:|
| Files | 21,672 | 21,671 | -1 |
| Pass | 676 | 676 | 0 |
| Fail | 13,845 | 14,044 | +199 |
| Error | 2,392 | 2,191 | -201 |
| No results | 985 | 986 | +1 |
| Skip | 3,774 | 3,774 | 0 |
| Subtests passed / total | 61,448 / 1,579,098 | 61,461 / 1,579,477 | +13 / +379 |

No file went from a passing status to a non-passing one. The single file
difference is a baseline artifact, not a test: the 2026-09-06
`html/webappapis` map was assembled by `merge.py` from a per-file rerun and
carries an extra record keyed `test`. This run measured that directory in one
invocation, so the record is gone.

### The directories that moved

Every other directory's map is byte-identical to its 2026-09-06 counterpart,
`dom` included.

| Directory | Fail | Error | No results | Subtests passed / total |
|---|---:|---:|---:|---:|
| content-security-policy | 529 -> 530 | 252 -> 251 | 19 -> 19 | 102 / 3423 -> 102 / 3424 |
| html/browsers | 527 -> 528 | 134 -> 133 | 37 -> 37 | 143 / 1700 -> 149 / 1708 |
| html/semantics/embedded-content | 566 -> 569 | 41 -> 38 | 33 -> 33 | 338 / 1564 -> 338 / 1574 |
| html/semantics/scripting-1 | 260 -> 272 | 152 -> 139 | 4 -> 5 | 768 / 2045 -> 772 / 2095 |
| html/webappapis | 245 -> 245 | 31 -> 30 | 15 -> 15 | 357 / 1101 -> 357 / 1101 |
| navigation-timing | 35 -> 49 | 15 -> 1 | 6 -> 6 | 0 / 48 -> 2 / 122 |
| resource-timing | 87 -> 90 | 41 -> 38 | 4 -> 4 | 0 / 371 -> 0 / 378 |
| svg | 287 -> 448 | 305 -> 144 | 10 -> 10 | 102 / 1860 -> 102 / 2021 |
| user-timing | 22 -> 26 | 7 -> 3 | 6 -> 6 | 2 / 65 -> 3 / 133 |

### Explaining every movement

204 files changed status or reason. `diff_census.py` records the transitions;
each was then attributed by reading the moved file's own `<script src>` list.

| Count | Transition | Cause |
|---:|---|---|
| 181 | `error/evaluation-threw` -> `fail` | the file includes a support script whose name ends in `testharness.js`; it now loads (H2) |
| 22 | `error/evaluation-threw` -> `fail` (18), `error/server-side-handler` (3), `no-results/server-side-handler` (1) | the file includes a `.py` handler, no longer parsed as JavaScript (H2) |
| 1 | `error/hang-killed-90s` -> `error/hang-killed` | the same test, now killed by the runner instead of by an external timeout (H1) |

**Unattributed: none.** The attribution is mechanical — a moved file is
credited to the include filter if it carries a `*testharness.js` support
script, otherwise to the handler skip if it carries a `.py` src — and every
one of the 204 fell into a class.

The largest block is `svg` (161 files), all of them
`/resources/SVGAnimationTestCase-testharness.js` consumers; `navigation-timing`
(14) and `user-timing` (4) load `resources/webperftestharness.js`, which the
old suffix test also swallowed. That helper name was not in the census's
missing-globals table, so the include filter cost more than the census could
see. The `.py` block is `html/semantics/scripting-1`'s `log.py` script-loading
tests, `resource-timing`'s `status-code.py`, `content-security-policy`'s
`redirect.py`, `html/browsers`' `delay.py` and `embedded-content`'s `slow.py`.

The `/css/support/*.js` helpers the census suspected of sharing the SVG cause
did **not** move: `test_valid_value` (37), `test_computed_value` (36) and
`test_invalid_value` (36) are unchanged in the missing-globals table. F1 above
records why — their file names never matched the filter. Their residual is a
separate, still-open question for whichever lane owns those files.

### Missing globals

The census's harness-attributed entries are gone: `createSVGElement` (was 100)
and `smil_async_test` (was 63) no longer appear. `test_namespace` (was 19) is
also gone. What remains at the head of the table is engine demand — `crypto`
(86), `XMLHttpRequest` (83), `Worker` (51) — unchanged from 2026-09-06, plus
the `/css/support` helpers above.

### Error classes

`import.meta` and property-name syntax errors, about 260 in the census, are
down but not gone: roughly 100 `import.meta` errors survive. They are not all
`.py` handlers — the remainder are real JavaScript the engine cannot parse, and
belong to the engine's residual rather than the harness's.

## Progress

- **2026-09-07** — H1, H2 and H3 landed; H4 re-run recorded. Gates below.

## Gates

| Gate | Result |
|---|---|
| `cargo test -p genet-wpt --features netfetch` | green: 62 passed, 0 failed, 3 ignored (unit) plus `fetch_netfetcher` 1 passed |
| `cargo clippy -p genet-wpt --features netfetch --all-targets` | no new warning in any file touched. The nine `genet-wpt` warnings are pre-existing and all sit in `net.rs`, `conformance.rs`, and the two `#[expect(dead_code)]` / items-after-test-module notes that predate this change |
| `cargo fmt` | applied to the six changed files. `render.rs` and `testdriver/input_events.rs` were also reformatted by a package-wide run and reverted, being another lane's drift |
| `reftest css/mediaqueries` | `unexpected=0` (16 passed / 40 failed / 37 skipped of 93) |
| `reftest css/css-position` | `unexpected=0` |
| Disk census re-run | 82 result files, 21,671 tests, 204 explained movements, 0 unattributed |

### One pre-existing failure, not caused here

`support/wpt/check-testharness-baselines.ps1` fails its first baseline:
`dom/nodes/Node-baseURI.html` is pinned at `fail 4/9` in
`ports/genet-wpt/expectations/testharness/dom_boa.json` and the runner reports
`fail 0/9`. It is **not** this change: the 2026-09-06 census, taken at
`2c47cff7627` on the pre-repair runner, already recorded 0/9, and this run's
`dom` map is byte-identical to that one. Something between `2c47cff7627` and
`5af76a0cb8c` moved `baseURI` and left the checked-in baseline behind. Repinning
it is a decision for the lane that owns it, not a side effect of this one.
