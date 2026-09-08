// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Command-line surface: the parsed argument record, its parser, and the
//! usage text.

use super::*;

pub(crate) struct Args {
    pub(crate) command: String,
    pub(crate) subset: String,
    pub(crate) tests_root: String,
    pub(crate) verbose: bool,
    pub(crate) engine: harness::Engine,
    pub(crate) renderer: ReftestRenderer,
    /// Device pixels per CSS pixel for reftest and dump rasterization.
    pub(crate) device_scale: f32,
    /// Connect to an already-running `wpt serve` at this origin (server mode).
    pub(crate) server_base: Option<String>,
    /// Spawn (and tear down) a `wpt serve` for the run (server mode).
    pub(crate) spawn_server: bool,
    /// Per-test wall-clock timeout (seconds) for `test262` worker subprocesses: a test
    /// running longer is killed and recorded as a timeout. Generous enough for slow
    /// (but finite) tests; bounds true infinite hangs.
    pub(crate) timeout_secs: u64,
    /// Per-test wall-clock ceiling for the testharness drive loop (seconds). Server
    /// mode spends this on any test awaiting something that never settles, so a run
    /// over a network-shaped directory wants it short.
    pub(crate) drive_deadline_secs: u64,
    /// Run a GC tick at a fixed cadence inside each `testharness` test's drive
    /// loop. **On by default**: without it Nova never collects during a test (its
    /// heap is collected only when the host asks) while Boa's allocation-threshold
    /// collector fires anyway, so the two engines were not being asked the same
    /// question. `--no-harness-gc` restores the old asymmetry for a measurement.
    pub(crate) harness_gc: bool,
    /// Run every `testharness` test in the parent process, the pre-isolation path. A
    /// test that blocks inside the engine then hangs the whole run.
    pub(crate) in_process: bool,
    /// Worker subprocesses `testharness` runs concurrently. Each test is isolated, so
    /// the result map does not depend on this.
    pub(crate) jobs: usize,
    /// The backing file for the single test a `testharness-one` worker runs.
    pub(crate) test_path: Option<String>,
    /// Write the full `test262` worklist (every Nova gap + every timeout, not just the
    /// printed sample) to this path. Essential for a full-corpus run, whose lists run to
    /// thousands.
    pub(crate) worklist_out: Option<String>,
    /// Use the legacy directory walk instead of MANIFEST.json. This is retained as a
    /// diagnostic fallback for custom partial trees; normal WPT commands are
    /// manifest-backed.
    pub(crate) walk_discovery: bool,
    /// Check current per-test statuses against a JSON expectations file.
    pub(crate) expectations: Option<String>,
    /// Write current per-test statuses to a JSON expectations file.
    pub(crate) write_expectations: Option<String>,
    /// Policy written into a new expectations file. Exact baselines cover every
    /// discovered test; opt-in baselines use their `tests` map as the include
    /// list and skip newly discovered tests until they are explicitly added.
    pub(crate) expectation_policy: ExpectationPolicy,
    /// Write aggregate Buckram table-dispatch counters for the documents the
    /// Livery reftest renderer actually laid out.
    pub(crate) write_table_ledger: Option<PathBuf>,
    /// Override the checked-in list of reftests whose visual pass is known not
    /// to verify the capability named by the test/reference pair.
    pub(crate) reference_verification: Option<PathBuf>,
    /// Exact reftest result files joined by the absolute conformance command.
    pub(crate) reftest_results: Vec<PathBuf>,
    /// Exact testharness result files joined by the absolute conformance command.
    pub(crate) testharness_results: Vec<PathBuf>,
    /// Write deterministic absolute conformance JSON.
    pub(crate) write_conformance: Option<PathBuf>,
    /// Compare against a prior absolute conformance report.
    pub(crate) conformance_baseline: Option<PathBuf>,
    /// Permit a diagnostic report with hostable tests absent from its inputs.
    pub(crate) allow_incomplete_conformance: bool,
    /// Permit legacy result maps without pinned manifest and runner identities.
    pub(crate) allow_unpinned_conformance_inputs: bool,
}

pub(crate) fn parse_args() -> Result<Args, String> {
    let mut command = None;
    let mut subset = None;
    let mut tests_root = DEFAULT_TESTS_ROOT.to_string();
    let mut verbose = false;
    let mut engine = harness::Engine::default();
    let mut renderer = ReftestRenderer::Livery;
    let mut device_scale = 1.0f32;
    let mut server_base = None;
    let mut spawn_server = false;
    let mut timeout_secs = 30u64;
    let mut drive_deadline_secs = 15u64;
    let mut harness_gc = true;
    let mut in_process = false;
    let mut jobs = 1usize;
    let mut test_path = None;
    let mut worklist_out = None;
    let mut walk_discovery = false;
    let mut expectations = None;
    let mut write_expectations = None;
    let mut expectation_policy = ExpectationPolicy::Exact;
    let mut write_table_ledger = None;
    let mut reference_verification = None;
    let mut reftest_results = Vec::new();
    let mut testharness_results = Vec::new();
    let mut write_conformance = None;
    let mut conformance_baseline = None;
    let mut allow_incomplete_conformance = false;
    let mut allow_unpinned_conformance_inputs = false;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--tests-root" => {
                tests_root = it.next().ok_or("--tests-root needs a value")?;
            },
            "--engine" => {
                let v = it.next().ok_or("--engine needs a value (boa | nova)")?;
                engine = harness::Engine::parse(&v)
                    .ok_or_else(|| format!("unknown engine: {v} (expected boa | nova)"))?;
            },
            "--renderer" => {
                let value = it.next().ok_or("--renderer needs a value (livery)")?;
                renderer = ReftestRenderer::parse(&value)
                    .ok_or_else(|| format!("unknown renderer: {value} (expected livery)"))?;
            },
            "--device-scale" => {
                let value = it
                    .next()
                    .ok_or("--device-scale needs a positive finite number")?;
                device_scale = value
                    .parse()
                    .map_err(|_| format!("invalid --device-scale: {value}"))?;
                if !device_scale.is_finite() || device_scale <= 0.0 {
                    return Err(format!(
                        "invalid --device-scale: {value} (expected a positive finite number)"
                    ));
                }
            },
            "--server-base" => {
                server_base = Some(it.next().ok_or("--server-base needs a URL")?);
            },
            "--spawn-server" => spawn_server = true,
            "--timeout" => {
                let v = it.next().ok_or("--timeout needs a value (seconds)")?;
                timeout_secs = v.parse().map_err(|_| format!("invalid --timeout: {v}"))?;
            },
            "--drive-deadline" => {
                let v = it
                    .next()
                    .ok_or("--drive-deadline needs a value (seconds)")?;
                drive_deadline_secs =
                    v.parse().ok().filter(|s| *s > 0).ok_or_else(|| {
                        format!("invalid --drive-deadline: {v} (expected 1 or more)")
                    })?;
            },
            "--no-harness-gc" => harness_gc = false,
            "--in-process" => in_process = true,
            "--jobs" => {
                let v = it.next().ok_or("--jobs needs a value")?;
                jobs = v
                    .parse()
                    .ok()
                    .filter(|j| *j > 0)
                    .ok_or_else(|| format!("invalid --jobs: {v} (expected 1 or more)"))?;
            },
            "--test-path" => {
                test_path = Some(it.next().ok_or("--test-path needs a path")?);
            },
            "--worklist-out" => {
                worklist_out = Some(it.next().ok_or("--worklist-out needs a path")?);
            },
            "--walk-discovery" => walk_discovery = true,
            "--expectations" => {
                expectations = Some(it.next().ok_or("--expectations needs a path")?);
            },
            "--write-expectations" => {
                write_expectations = Some(it.next().ok_or("--write-expectations needs a path")?);
            },
            "--expectation-policy" => {
                let value = it
                    .next()
                    .ok_or("--expectation-policy needs a value (exact | opt-in)")?;
                expectation_policy = ExpectationPolicy::parse(&value).ok_or_else(|| {
                    format!("unknown expectation policy: {value} (expected exact | opt-in)")
                })?;
            },
            "--write-table-ledger" => {
                write_table_ledger = Some(PathBuf::from(
                    it.next().ok_or("--write-table-ledger needs a path")?,
                ));
            },
            "--reference-verification" => {
                reference_verification = Some(PathBuf::from(
                    it.next().ok_or("--reference-verification needs a path")?,
                ));
            },
            "--reftest-results" => {
                reftest_results.push(PathBuf::from(
                    it.next().ok_or("--reftest-results needs a path")?,
                ));
            },
            "--testharness-results" => {
                testharness_results.push(PathBuf::from(
                    it.next().ok_or("--testharness-results needs a path")?,
                ));
            },
            "--write-conformance" => {
                write_conformance = Some(PathBuf::from(
                    it.next().ok_or("--write-conformance needs a path")?,
                ));
            },
            "--conformance-baseline" => {
                conformance_baseline = Some(PathBuf::from(
                    it.next().ok_or("--conformance-baseline needs a path")?,
                ));
            },
            "--allow-incomplete-conformance" => allow_incomplete_conformance = true,
            "--allow-unpinned-conformance-inputs" => {
                allow_unpinned_conformance_inputs = true;
            },
            "-v" | "--verbose" => verbose = true,
            "-h" | "--help" => return Err(usage()),
            _ if arg.starts_with('-') => return Err(format!("unknown flag: {arg}\n{}", usage())),
            _ if command.is_none() => command = Some(arg),
            _ if subset.is_none() => subset = Some(arg),
            _ => return Err(format!("unexpected argument: {arg}\n{}", usage())),
        }
    }
    Ok(Args {
        command: command.ok_or(usage())?,
        subset: subset.unwrap_or_default(),
        tests_root,
        verbose,
        engine,
        renderer,
        device_scale,
        server_base,
        spawn_server,
        timeout_secs,
        drive_deadline_secs,
        harness_gc,
        in_process,
        jobs,
        test_path,
        worklist_out,
        walk_discovery,
        expectations,
        write_expectations,
        expectation_policy,
        write_table_ledger,
        reference_verification,
        reftest_results,
        testharness_results,
        write_conformance,
        conformance_baseline,
        allow_incomplete_conformance,
        allow_unpinned_conformance_inputs,
    })
}

pub(crate) fn usage() -> String {
    "\
genet-wpt - genet-native web-platform-tests runner (phase 1: crash-smoke)

Usage:
    genet-wpt list        <subset>   enumerate + classify tests in a subset
    genet-wpt run         <subset>   crash-smoke a subset (parse + layout)
    genet-wpt reftest     <subset>   render + pixel-compare reftests (needs a GPU)
    genet-wpt dump        <subset>   render each reftest and its reference to PNGs
    genet-wpt testharness <subset>   run testharness.js tests + collect results (Boa)
    genet-wpt manifest    <subset>   enumerate from MANIFEST.json (authoritative; H1)
    genet-wpt conformance <subset>   join exact results to the manifest; report absolute totals
    genet-wpt compare     <subset>   run each testharness test on Boa + Nova, diff (H2b)
    genet-wpt test262     <subset>   run test262 on Boa + Nova, diff = Nova's worklist

Options:
    --tests-root <dir>   tests root (default: tests/wpt/tests)
    --timeout <secs>     per-test worker timeout (default: 30) for `test262` and
                         for the `testharness` worker subprocesses
    --drive-deadline <secs>
                         per-test testharness drive-loop deadline (default: 15)
    --no-harness-gc      do not run a GC tick inside the testharness drive loop
    --in-process         run `testharness` tests without worker isolation
    --jobs <n>           `testharness` worker subprocesses in flight (default: 1)
    --worklist-out <f>   write the full `test262` Nova-gap + timeout list to <f>
    --walk-discovery     use the legacy directory walk instead of MANIFEST.json
    --expectations <f>   fail if testharness results differ from JSON expectations
    --write-expectations <f>
                         write current testharness results as JSON expectations
    --expectation-policy <exact|opt-in>
                         policy for --write-expectations (default: exact)
    --write-table-ledger <f>
                         write Livery reftest table-dispatch counters
    --reference-verification <f>
                         override the checked-in reference-verification map
    --reftest-results <f>
                         add an exact reftest result file to `conformance`
    --testharness-results <f>
                         add an exact testharness result file to `conformance`
    --write-conformance <f>
                         write deterministic absolute conformance JSON
    --conformance-baseline <f>
                         print aggregate deltas from a prior conformance report
    --allow-incomplete-conformance
                         permit missing hostable tests in a diagnostic report
    --allow-unpinned-conformance-inputs
                         permit legacy result maps only in a diagnostic report
    --engine <name>      testharness JS engine: boa (default) | nova
    --renderer <name>    style/render route: livery (default)
    --device-scale <n>   device pixels per CSS pixel for reftest/dump (default: 1)
    --server-base <url>  run testharness against a live `wpt serve` at <url>
                         (server mode; needs --features netfetch)
    --spawn-server       spawn + tear down a `wpt serve` for the run
                         (server mode; needs --features netfetch)
    -v, --verbose        print every test, not just failures
    -h, --help

<subset> is a directory or file beneath the tests root, e.g.
    genet-wpt run css/CSS2/floats
    genet-wpt run dom/nodes/Element-classList.html"
        .to_string()
}
