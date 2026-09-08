// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The testharness.js lane: run a subset and report per-subtest results.
//!
//! Two execution shapes over one per-test body ([`run_one`]). The default runs
//! each test in a `testharness-one` worker subprocess, as `test262` already
//! does, so a test that blocks *inside* the engine (the drive loop's deadline
//! cannot interrupt a native call) is killed and recorded rather than hanging
//! the run. `--in-process` keeps the older single-process path.

use super::*;

/// One test's outcome: the ledger record plus the lines the run log carries.
pub(crate) struct OneOutcome {
    pub(crate) record: ActualRecord,
    /// The engine/harness message behind an `error`, or the `(passed/total)`
    /// tail of a pass or fail. Printed; the ledger tools classify errors from it.
    pub(crate) detail: Option<String>,
    /// `[status] name message` per failing subtest, printed under `--verbose`.
    pub(crate) failures: Vec<String>,
}

/// Per-run state a single test needs: the harness source, the optional server,
/// and the Nova template (in-process only; a worker builds its own).
struct RunCtx<'a> {
    testharness_js: &'a str,
    #[cfg(feature = "netfetch")]
    server: &'a Option<net::ServerCtx>,
    nova_template: Option<&'a mut harness::NovaHarnessTemplate>,
}

/// Phase 3: run testharness.js tests and report per-subtest results.
pub(crate) fn testharness(tests: &[TestCase], args: &Args) {
    let tests_root = Path::new(&args.tests_root);
    let th_path = tests_root.join("resources/testharness.js");
    let testharness_js = match fs::read_to_string(&th_path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("testharness.js not found at {}", th_path.display());
            std::process::exit(2);
        },
    };

    // Server mode (netfetch): connect to / spawn a `wpt serve` so `fetch()` hits a
    // real server, `<script src>` is fetched (`.sub.js` substituted), and the
    // document base URL resolves relative URLs. Disk mode leaves this `None`.
    #[cfg(feature = "netfetch")]
    let server = setup_server(args);
    #[cfg(not(feature = "netfetch"))]
    if args.spawn_server || args.server_base.is_some() {
        eprintln!("server mode (--server-base / --spawn-server) needs `--features netfetch`");
        std::process::exit(2);
    }

    // Boa / the bridge can panic on unimplemented paths; report, don't spam.
    let prev = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));

    let actuals = if args.in_process {
        let mut nova_template = nova_template(args, &testharness_js);
        let mut ctx = RunCtx {
            testharness_js: &testharness_js,
            #[cfg(feature = "netfetch")]
            server: &server,
            nova_template: nova_template.as_mut(),
        };
        let mut actuals = Vec::with_capacity(tests.len());
        for test in tests {
            let outcome = run_one(test, args, &mut ctx);
            print_outcome(&outcome, args.verbose);
            actuals.push(outcome.record);
        }
        actuals
    } else {
        #[cfg(feature = "netfetch")]
        let base = server.as_ref().map(|s| s.origin.clone());
        #[cfg(not(feature = "netfetch"))]
        let base: Option<String> = None;
        run_isolated(tests, args, base.as_deref())
    };

    panic::set_hook(prev);

    let mut counts = [0usize; 5]; // pass, fail, error, no-results, skip
    let (mut sub_passed, mut sub_total) = (0usize, 0usize);
    for record in &actuals {
        counts[status_index(record.status)] += 1;
        if let Some((passed, total)) = record.subtests {
            sub_passed += passed;
            sub_total += total;
        }
    }
    println!(
        "\ntestharness [{}]: {} all-pass, {} with-failures, {} errored, \
         {} no-results, {} skipped (of {} files); \
         subtests {sub_passed}/{sub_total} passed",
        args.engine.label(),
        counts[0],
        counts[1],
        counts[2],
        counts[3],
        counts[4],
        tests.len(),
    );
    finish_expectations(args, "testharness", &actuals);
}

fn status_index(status: &str) -> usize {
    match status {
        "pass" => 0,
        "fail" => 1,
        "error" => 2,
        "no-results" => 3,
        _ => 4,
    }
}

fn nova_template(args: &Args, testharness_js: &str) -> Option<harness::NovaHarnessTemplate> {
    if args.engine != harness::Engine::Nova {
        return None;
    }
    match harness::NovaHarnessTemplate::new(testharness_js) {
        Ok(template) => Some(template),
        Err(e) => {
            eprintln!("Nova harness template init failed: {e}");
            std::process::exit(2);
        },
    }
}

/// Statuses the ledger writes, as the `&'static str` [`ActualRecord`] holds.
fn static_status(status: &str) -> &'static str {
    match status {
        "pass" => "pass",
        "fail" => "fail",
        "no-results" => "no-results",
        "skip" => "skip",
        _ => "error",
    }
}

/// Run one test end to end. Everything the lane records about a file — skip
/// classification, document load, evaluation, scoring — happens here, so the
/// in-process and worker paths cannot drift.
fn run_one(test: &TestCase, args: &Args, ctx: &mut RunCtx<'_>) -> OneOutcome {
    let tests_root = Path::new(&args.tests_root);
    let plain = |record: ActualRecord| OneOutcome {
        record,
        detail: None,
        failures: Vec::new(),
    };

    if test.kind != Kind::Testharness {
        return plain(ActualRecord::with_reason(test, "skip", "non-testharness"));
    }
    let ext = test.path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("xhtml") || ext.eq_ignore_ascii_case("xht") {
        return plain(ActualRecord::with_reason(test, "skip", "xhtml"));
    }

    // Build the testharness HTML: a real .html document's contents, or a
    // synthesized wrapper for a `.any.js` / `.window.js` test.
    #[cfg(feature = "netfetch")]
    let html = if let Some(s) = ctx.server {
        match net::http_get(&s.doc_url(test.name())) {
            Some(t) => t,
            None => {
                return OneOutcome {
                    record: ActualRecord::with_reason(test, "error", "fetch-load-failed"),
                    detail: Some("fetch".to_string()),
                    failures: Vec::new(),
                };
            },
        }
    } else {
        match build_test_html_disk(test) {
            TestHtml::Html(h) => h,
            TestHtml::Skip(reason) => {
                return plain(ActualRecord::with_reason(test, "skip", reason));
            },
            TestHtml::ReadError => {
                return OneOutcome {
                    record: ActualRecord::with_reason(test, "error", "read-failed"),
                    detail: Some("read".to_string()),
                    failures: Vec::new(),
                };
            },
        }
    };
    #[cfg(not(feature = "netfetch"))]
    let html = match build_test_html_disk(test) {
        TestHtml::Html(h) => h,
        TestHtml::Skip(reason) => {
            return plain(ActualRecord::with_reason(test, "skip", reason));
        },
        TestHtml::ReadError => {
            return OneOutcome {
                record: ActualRecord::with_reason(test, "error", "read-failed"),
                detail: Some("read".to_string()),
                failures: Vec::new(),
            };
        },
    };

    let base_dir = test.path.parent().unwrap_or(tests_root);
    let disk = harness::DiskLoader::new(base_dir, tests_root);
    let testharness_js = ctx.testharness_js;
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        // Server mode: a fresh per-test fetch-event channel feeds the drive loop,
        // so deferred fetches settle out of band, mid-flight abort works, and a
        // hung fetch hits the per-test deadline. The shared worker routes replies
        // to this channel; a late reply from a prior test lands on a dropped
        // channel and is harmlessly discarded.
        #[cfg(feature = "netfetch")]
        if let Some(s) = ctx.server {
            let (ev_tx, ev_rx) = std::sync::mpsc::channel::<net::FetchEvent>();
            let doc_url = s.doc_url(test.name());
            let loader = s.loader(&doc_url);
            let handler = net::NetFetchHandler::new(ev_tx.clone());
            // The `WebSocket` seam rides the same per-test channel: `wpt serve`
            // hosts ws/wss endpoints, so server mode is where a socket can
            // actually connect.
            let sockets = net::NetWebSocketHandler::new(ev_tx);
            let completion = net::ChannelCompletion::new(ev_rx);
            if let Some(template) = ctx.nova_template.as_mut() {
                return template.run_test_with_style_and_ws(
                    &html,
                    &loader,
                    Some(&doc_url),
                    Some(Box::new(handler)),
                    Some(Box::new(sockets)),
                    Some(&completion),
                    args.renderer.harness_style(),
                );
            }
            return harness::run_test_with_style_and_ws(
                testharness_js,
                &html,
                &loader,
                Some(&doc_url),
                Some(Box::new(handler)),
                Some(Box::new(sockets)),
                Some(&completion),
                args.engine,
                args.renderer.harness_style(),
            );
        }
        let doc_url = test.disk_doc_url();
        if let Some(template) = ctx.nova_template.as_mut() {
            return template.run_test_with_style(
                &html,
                &disk,
                Some(&doc_url),
                None,
                None,
                args.renderer.harness_style(),
            );
        }
        harness::run_test_with_style(
            testharness_js,
            &html,
            &disk,
            Some(&doc_url),
            None,
            None,
            args.engine,
            args.renderer.harness_style(),
        )
    }));

    // A `<script src="…py">` include is a WPT server-side handler. Disk mode
    // skipped it rather than handing Python to the engine; say so, so the
    // residual is attributed to the missing server and not to a syntax error.
    let handler_skipped = disk.saw_server_handler();
    let reason = |fallback: &'static str| -> &'static str {
        if handler_skipped {
            "server-side-handler"
        } else {
            fallback
        }
    };

    match result {
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("non-string panic")
                .to_string();
            OneOutcome {
                record: ActualRecord::with_reason(test, "error", "panic"),
                detail: Some(format!("({message})")),
                failures: Vec::new(),
            }
        },
        Ok(harness::HarnessOutcome::Threw(msg)) => OneOutcome {
            record: ActualRecord::with_reason(test, "error", reason("evaluation-threw")),
            detail: Some(format!("({msg})")),
            failures: Vec::new(),
        },
        Ok(harness::HarnessOutcome::Ran(results)) => {
            let total = results.len();
            let passed = results.iter().filter(|r| r.passed()).count();
            if total == 0 {
                OneOutcome {
                    record: ActualRecord::with_reason(test, "no-results", reason("no-subtests")),
                    detail: Some("(harness ran but reported no subtests)".to_string()),
                    failures: Vec::new(),
                }
            } else {
                let status = if passed == total { "pass" } else { "fail" };
                OneOutcome {
                    record: ActualRecord::with_subtests(test, status, &results),
                    detail: None,
                    failures: results
                        .iter()
                        .filter(|r| !r.passed())
                        .map(|r| {
                            let msg = r.message.as_deref().unwrap_or("");
                            format!("        [{}] {} {msg}", r.status, r.name)
                        })
                        .collect(),
                }
            }
        },
    }
}

/// The run log, unchanged in shape: failures and errors always, the rest under
/// `--verbose`.
fn print_outcome(outcome: &OneOutcome, verbose: bool) {
    let record = &outcome.record;
    let name = &record.test;
    let detail = outcome.detail.as_deref().unwrap_or("");
    let (passed, total) = record.subtests.unwrap_or((0, 0));
    match record.status {
        "skip" => {
            if verbose {
                let reason = record.reason.as_deref().unwrap_or("");
                println!("SKIP  {reason:16} {name}");
            }
        },
        "error" => match record.reason.as_deref() {
            Some("fetch-load-failed") => println!("ERROR fetch   {name}"),
            Some("read-failed") => println!("ERROR read    {name}"),
            Some("panic") => println!("ERROR panic   {name}  {detail}"),
            _ => println!("ERROR {name}  {detail}"),
        },
        "no-results" => {
            if verbose {
                println!("NORES {name}  {detail}");
            }
        },
        "pass" => {
            if verbose {
                println!("PASS  {name}  ({passed}/{total})");
            }
        },
        _ => {
            println!("FAIL  {name}  ({passed}/{total} subtests)");
            if verbose {
                for line in &outcome.failures {
                    println!("{line}");
                }
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Worker isolation
// ---------------------------------------------------------------------------

/// The wire form of one outcome, printed by a `testharness-one` worker on a
/// single line and parsed back by the parent. Everything the ledger writes
/// travels here, so an isolated run's result map is identical to an in-process
/// one for every test that does not hang.
const RESULT_PREFIX: &str = "__thresult ";

fn encode_outcome(outcome: &OneOutcome) -> String {
    let record = &outcome.record;
    let mut value = serde_json::json!({
        "status": record.status,
        "reason": record.reason,
        "detail": outcome.detail,
        "failures": outcome.failures,
    });
    if let Some((passed, total)) = record.subtests {
        value["subtests_passed"] = passed.into();
        value["subtests_total"] = total.into();
    }
    if let Some(subtests) = &record.subtest_results {
        value["subtests"] = serde_json::Value::Array(
            subtests
                .iter()
                .map(|s| serde_json::json!({ "name": s.name, "status": s.status }))
                .collect(),
        );
    }
    format!("{RESULT_PREFIX}{value}")
}

fn decode_outcome(test: &TestCase, line: &str) -> Option<OneOutcome> {
    let value: serde_json::Value = serde_json::from_str(line.strip_prefix(RESULT_PREFIX)?).ok()?;
    let status = static_status(value.get("status")?.as_str()?);
    let subtest_results = value.get("subtests").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|s| {
                Some(ActualSubtest {
                    name: s.get("name")?.as_str()?.to_string(),
                    status: s.get("status")?.as_str()?.to_string(),
                })
            })
            .collect::<Vec<_>>()
    });
    let subtests = match (
        value.get("subtests_passed").and_then(|v| v.as_u64()),
        value.get("subtests_total").and_then(|v| v.as_u64()),
    ) {
        (Some(p), Some(t)) => Some((p as usize, t as usize)),
        _ => None,
    };
    Some(OneOutcome {
        record: ActualRecord {
            test: test.name().to_string(),
            status,
            reason: value
                .get("reason")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            subtests,
            subtest_results,
        },
        detail: value
            .get("detail")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        failures: value
            .get("failures")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// `testharness-one <url> --test-path <file>`: run exactly one test and print
/// its encoded outcome. The parent spawns this per test so a blocked engine
/// call kills only this process.
pub(crate) fn testharness_one(args: &Args) {
    use std::io::Write;
    panic::set_hook(Box::new(|_| {}));

    let Some(path) = args.test_path.as_ref() else {
        eprintln!("testharness-one needs --test-path");
        std::process::exit(2);
    };
    let tests_root = Path::new(&args.tests_root);
    let testharness_js = match fs::read_to_string(tests_root.join("resources/testharness.js")) {
        Ok(s) => s,
        Err(_) => std::process::exit(2),
    };
    #[cfg(feature = "netfetch")]
    let server = setup_server(args);
    let test = TestCase::single(PathBuf::from(path), args.subset.clone());
    let mut template = nova_template(args, &testharness_js);
    let mut ctx = RunCtx {
        testharness_js: &testharness_js,
        #[cfg(feature = "netfetch")]
        server: &server,
        nova_template: template.as_mut(),
    };
    let outcome = run_one(&test, args, &mut ctx);
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{}", encode_outcome(&outcome));
    let _ = out.flush();
}

/// Isolated run: one worker subprocess per test, `--jobs` in flight, each
/// bounded by `--timeout`. A worker that outlives its timeout is killed and its
/// test recorded as `error` / `hang-killed`; one that dies without reporting is
/// `error` / `worker-crashed`. Cheap skips (non-testharness files, XHTML) are
/// classified here rather than paying a process for them.
fn run_isolated(tests: &[TestCase], args: &Args, server_base: Option<&str>) -> Vec<ActualRecord> {
    let Some(exe) = std::env::current_exe().ok() else {
        eprintln!("cannot locate the genet-wpt executable for worker isolation");
        std::process::exit(2);
    };
    let timeout = std::time::Duration::from_secs(args.timeout_secs);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let jobs = args.jobs.min(tests.len().max(1));
    println!(
        "testharness [{}]: {} files on {jobs} worker proc(s) (timeout {}s, drive deadline {}s)…",
        args.engine.label(),
        tests.len(),
        args.timeout_secs,
        args.drive_deadline_secs,
    );

    let mut collected: Vec<(usize, OneOutcome)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..jobs)
            .map(|_| {
                let exe = exe.as_path();
                let next = &next;
                scope.spawn(move || {
                    let mut out: Vec<(usize, OneOutcome)> = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if i >= tests.len() {
                            break;
                        }
                        out.push((i, run_worker(&tests[i], args, exe, server_base, timeout)));
                    }
                    out
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    collected.sort_by_key(|(i, _)| *i);

    let mut actuals = Vec::with_capacity(collected.len());
    for (_, outcome) in &collected {
        print_outcome(outcome, args.verbose);
    }
    for (_, outcome) in collected {
        actuals.push(outcome.record);
    }
    actuals
}

/// Run one test in a worker subprocess, or classify it without one when the
/// answer needs no engine.
fn run_worker(
    test: &TestCase,
    args: &Args,
    exe: &Path,
    server_base: Option<&str>,
    timeout: std::time::Duration,
) -> OneOutcome {
    let plain = |record: ActualRecord| OneOutcome {
        record,
        detail: None,
        failures: Vec::new(),
    };
    if test.kind != Kind::Testharness {
        return plain(ActualRecord::with_reason(test, "skip", "non-testharness"));
    }
    let ext = test.path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("xhtml") || ext.eq_ignore_ascii_case("xht") {
        return plain(ActualRecord::with_reason(test, "skip", "xhtml"));
    }

    let mut command = std::process::Command::new(exe);
    command
        .arg("testharness-one")
        .arg(test.name())
        .arg("--test-path")
        .arg(test.path.as_os_str())
        .arg("--tests-root")
        .arg(&args.tests_root)
        .arg("--engine")
        .arg(args.engine.label())
        .arg("--renderer")
        .arg(args.renderer.label())
        .arg("--drive-deadline")
        .arg(args.drive_deadline_secs.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    if !args.harness_gc {
        command.arg("--no-harness-gc");
    }
    if let Some(base) = server_base {
        command.arg("--server-base").arg(base);
    }
    let Ok(mut child) = command.spawn() else {
        return OneOutcome {
            record: ActualRecord::with_reason(test, "error", "worker-spawn-failed"),
            detail: Some("(cannot spawn worker)".to_string()),
            failures: Vec::new(),
        };
    };

    // Drain stdout on its own thread. The encoded outcome carries every subtest
    // name, which for the largest files is far past the pipe buffer: reading it
    // only after the child exits deadlocks the pair.
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut text = String::new();
        if let Some(so) = stdout.as_mut() {
            let _ = so.read_to_string(&mut text);
        }
        text
    });

    let start = std::time::Instant::now();
    let mut killed = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) => {},
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            killed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let text = reader.join().unwrap_or_default();
    if let Some(outcome) = text
        .lines()
        .rev()
        .find(|l| l.starts_with(RESULT_PREFIX))
        .and_then(|l| decode_outcome(test, l))
    {
        return outcome;
    }
    if killed {
        // The engine blocked in a native call the drive loop's deadline cannot
        // interrupt (`Atomics.waitAsync` is the known case).
        return OneOutcome {
            record: ActualRecord::with_reason(test, "error", "hang-killed"),
            detail: Some(format!("(killed after {}s)", timeout.as_secs())),
            failures: Vec::new(),
        };
    }
    OneOutcome {
        record: ActualRecord::with_reason(test, "error", "worker-crashed"),
        detail: Some("(worker died without reporting)".to_string()),
        failures: Vec::new(),
    }
}
