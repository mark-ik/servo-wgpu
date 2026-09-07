// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The expectation ledger: recorded and actual results, their subtest
//! normalisation, and the compare/check/write cycle that keeps a run
//! honest against what was previously accepted.

use super::*;

/// The cross-engine pass predicate: a caught run that did not panic or throw,
/// produced results, and every subtest passed.
pub(crate) fn outcome_passes(
    result: &Result<harness::HarnessOutcome, Box<dyn std::any::Any + Send>>,
) -> bool {
    matches!(
        result,
        Ok(harness::HarnessOutcome::Ran(results))
            if !results.is_empty() && results.iter().all(|r| r.passed())
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActualSubtest {
    pub(crate) name: String,
    pub(crate) status: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedSubtest {
    pub(crate) name: String,
    pub(crate) status: String,
    /// Human policy metadata. Unlike the file-level `reason`, this does not
    /// mirror a runner outcome; it records why a non-pass remains opted in.
    pub(crate) reason: Option<String>,
}

pub(crate) fn subtest_status(status: i64) -> String {
    match status {
        0 => "pass".to_string(),
        1 => "fail".to_string(),
        2 => "timeout".to_string(),
        3 => "not-run".to_string(),
        4 => "precondition-failed".to_string(),
        other => format!("unknown-{other}"),
    }
}

pub(crate) struct ActualRecord {
    pub(crate) test: String,
    pub(crate) status: &'static str,
    pub(crate) reason: Option<String>,
    /// `(passed, total)` subtest counts for a testharness file that produced
    /// results. `None` for reftests, skips, errors, and no-results files.
    pub(crate) subtests: Option<(usize, usize)>,
    /// Named testharness results. Kept separately from the count pair so old
    /// count-only expectation files stay valid while opted-in files can pin the
    /// exact assertion that moved.
    pub(crate) subtest_results: Option<Vec<ActualSubtest>>,
}

impl ActualRecord {
    pub(crate) fn new(test: &TestCase, status: &'static str) -> ActualRecord {
        ActualRecord {
            test: test.name().to_string(),
            status,
            reason: None,
            subtests: None,
            subtest_results: None,
        }
    }

    pub(crate) fn with_reason(
        test: &TestCase,
        status: &'static str,
        reason: impl Into<String>,
    ) -> ActualRecord {
        ActualRecord {
            test: test.name().to_string(),
            status,
            reason: Some(reason.into()),
            subtests: None,
            subtest_results: None,
        }
    }

    pub(crate) fn with_subtests(
        test: &TestCase,
        status: &'static str,
        results: &[script_runtime_api::TestResult],
    ) -> ActualRecord {
        let total = results.len();
        let passed = results.iter().filter(|result| result.passed()).count();
        ActualRecord {
            test: test.name().to_string(),
            status,
            reason: None,
            subtests: Some((passed, total)),
            subtest_results: Some(
                results
                    .iter()
                    .map(|result| ActualSubtest {
                        name: result.name.clone(),
                        status: subtest_status(result.status),
                    })
                    .collect(),
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedRecord {
    pub(crate) status: String,
    pub(crate) reason: Option<String>,
    /// Pinned `(passed, total)` subtest counts. When present, the actual run
    /// must report exactly these counts, so a regression *inside* a still-
    /// failing file is caught, not just a status flip. Absent in files written
    /// before the counts existed, which then pin status only.
    pub(crate) subtests: Option<(usize, usize)>,
    /// Named expected results. `None` preserves the status/count-only behavior
    /// of older version-1 JSON files.
    pub(crate) subtest_results: Option<Vec<ExpectedSubtest>>,
}

impl ExpectedRecord {
    pub(crate) fn matches(&self, actual: &ActualRecord) -> bool {
        self.status == actual.status
            && self
                .reason
                .as_deref()
                .is_none_or(|reason| actual.reason.as_deref() == Some(reason))
            && self
                .subtests
                .is_none_or(|counts| actual.subtests == Some(counts))
            && self.subtest_results.as_ref().is_none_or(|expected| {
                actual.subtest_results.as_ref().is_some_and(|actual| {
                    normalized_expected_subtests(expected) == normalized_actual_subtests(actual)
                })
            })
    }

    pub(crate) fn describe(&self) -> String {
        let mut out = match self.reason.as_deref() {
            Some(reason) => format!("{} ({reason})", self.status),
            None => self.status.clone(),
        };
        if let Some((passed, total)) = self.subtests {
            out.push_str(&format!(" {passed}/{total}"));
        }
        if let Some(subtests) = &self.subtest_results {
            let reasoned = subtests
                .iter()
                .filter(|subtest| subtest.reason.is_some())
                .count();
            out.push_str(&format!(
                " with {} named subtests ({reasoned} reasoned)",
                subtests.len()
            ));
        }
        out
    }

    pub(crate) fn mismatch(&self, actual: &ActualRecord) -> String {
        if self.status != actual.status
            || self
                .reason
                .as_deref()
                .is_some_and(|reason| actual.reason.as_deref() != Some(reason))
            || self
                .subtests
                .is_some_and(|counts| actual.subtests != Some(counts))
        {
            return format!(
                "expected {}, got {}",
                self.describe(),
                ActualRecordDisplay(actual)
            );
        }

        if let Some(expected) = &self.subtest_results {
            let expected = normalized_expected_subtests(expected);
            let actual = actual
                .subtest_results
                .as_deref()
                .map(normalized_actual_subtests)
                .unwrap_or_default();
            for name in expected
                .keys()
                .chain(actual.keys())
                .collect::<BTreeSet<_>>()
            {
                match (expected.get(name), actual.get(name)) {
                    (Some(expected), Some(actual)) if expected != actual => {
                        return format!(
                            "subtest `{name}` expected {}, got {}",
                            expected.join(", "),
                            actual.join(", ")
                        );
                    },
                    (Some(expected), None) => {
                        return format!(
                            "subtest `{name}` expected {}, but was not reported",
                            expected.join(", ")
                        );
                    },
                    (None, Some(actual)) => {
                        return format!(
                            "unexpected subtest `{name}` reported {}",
                            actual.join(", ")
                        );
                    },
                    _ => {},
                }
            }
        }

        format!(
            "expected {}, got {}",
            self.describe(),
            ActualRecordDisplay(actual)
        )
    }
}

pub(crate) fn normalized_expected_subtests(
    subtests: &[ExpectedSubtest],
) -> BTreeMap<String, Vec<String>> {
    let mut normalized = BTreeMap::<String, Vec<String>>::new();
    for subtest in subtests {
        normalized
            .entry(subtest.name.clone())
            .or_default()
            .push(subtest.status.clone());
    }
    for statuses in normalized.values_mut() {
        statuses.sort();
    }
    normalized
}

pub(crate) fn normalized_actual_subtests(
    subtests: &[ActualSubtest],
) -> BTreeMap<String, Vec<String>> {
    let mut normalized = BTreeMap::<String, Vec<String>>::new();
    for subtest in subtests {
        normalized
            .entry(subtest.name.clone())
            .or_default()
            .push(subtest.status.clone());
    }
    for statuses in normalized.values_mut() {
        statuses.sort();
    }
    normalized
}

pub(crate) struct ActualRecordDisplay<'a>(&'a ActualRecord);

impl std::fmt::Display for ActualRecordDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.reason.as_deref() {
            Some(reason) => write!(f, "{} ({reason})", self.0.status)?,
            None => f.write_str(self.0.status)?,
        }
        if let Some((passed, total)) = self.0.subtests {
            write!(f, " {passed}/{total}")?;
        }
        Ok(())
    }
}

pub(crate) struct ResultFileMetadata<'a> {
    pub(crate) command: &'a str,
    pub(crate) engine: &'a str,
    pub(crate) renderer: &'a str,
    pub(crate) subset: &'a str,
    pub(crate) manifest_sha256: Option<&'a str>,
    pub(crate) runner_sha256: Option<&'a str>,
    pub(crate) policy: ExpectationPolicy,
}

pub(crate) fn finish_expectations(args: &Args, command: &str, actuals: &[ActualRecord]) {
    if let Some(out) = &args.write_expectations {
        let provenance = if args.walk_discovery {
            None
        } else {
            let manifest = manifest_path(&args.tests_root);
            let manifest_sha256 = match conformance::sha256_file(&manifest) {
                Ok(digest) => digest,
                Err(error) => {
                    eprintln!(
                        "failed to fingerprint WPT manifest {}: {error}",
                        manifest.display()
                    );
                    std::process::exit(1);
                },
            };
            let executable = match std::env::current_exe() {
                Ok(path) => path,
                Err(error) => {
                    eprintln!("failed to locate genet-wpt executable: {error}");
                    std::process::exit(1);
                },
            };
            let runner_sha256 = match conformance::sha256_file(&executable) {
                Ok(digest) => digest,
                Err(error) => {
                    eprintln!(
                        "failed to fingerprint genet-wpt executable {}: {error}",
                        executable.display()
                    );
                    std::process::exit(1);
                },
            };
            Some((manifest_sha256, runner_sha256))
        };
        if let Err(e) = write_expectations(
            out,
            ResultFileMetadata {
                command,
                engine: if command == "reftest" {
                    "none"
                } else {
                    args.engine.label()
                },
                renderer: args.renderer.label(),
                subset: &args.subset,
                manifest_sha256: provenance.as_ref().map(|(manifest, _)| manifest.as_str()),
                runner_sha256: provenance.as_ref().map(|(_, runner)| runner.as_str()),
                policy: args.expectation_policy,
            },
            actuals,
        ) {
            eprintln!("failed to write expectations to {out}: {e}");
            std::process::exit(1);
        }
        println!("expectations written to {out} ({} tests)", actuals.len());
    }
    if let Some(path) = &args.expectations {
        match check_expectations(path, args.renderer.label(), actuals) {
            Ok(()) => println!("expectations: unexpected=0"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            },
        }
    }
}

pub(crate) fn write_expectations(
    path: &str,
    metadata: ResultFileMetadata<'_>,
    actuals: &[ActualRecord],
) -> Result<(), String> {
    let mut tests = BTreeMap::new();
    for actual in actuals {
        let named_subtests = actual.subtest_results.as_ref().map(|subtests| {
            subtests
                .iter()
                .map(|subtest| {
                    let mut value = serde_json::json!({
                        "name": subtest.name,
                        "status": subtest.status,
                    });
                    if metadata.policy == ExpectationPolicy::OptIn && subtest.status != "pass" {
                        value["reason"] = serde_json::Value::String(
                            "TODO: record the owning capability or issue".to_string(),
                        );
                    }
                    value
                })
                .collect::<Vec<_>>()
        });
        let value = match (actual.reason.as_deref(), actual.subtests) {
            (Some(reason), None) => serde_json::json!({
                "status": actual.status,
                "reason": reason,
            }),
            (Some(reason), Some((passed, total))) => serde_json::json!({
                "status": actual.status,
                "reason": reason,
                "subtests_passed": passed,
                "subtests_total": total,
            }),
            (None, Some((passed, total))) => serde_json::json!({
                "status": actual.status,
                "subtests_passed": passed,
                "subtests_total": total,
            }),
            (None, None) => serde_json::Value::String(actual.status.to_string()),
        };
        let mut value = value;
        if let Some(subtests) = named_subtests {
            value["subtests"] = serde_json::Value::Array(subtests);
        }
        tests.insert(actual.test.clone(), value);
    }
    let value = serde_json::json!({
        "version": 1,
        "command": metadata.command,
        "engine": metadata.engine,
        "renderer": metadata.renderer,
        "subset": metadata.subset.trim_matches('/').replace('\\', "/"),
        "manifest_sha256": metadata.manifest_sha256,
        "runner_sha256": metadata.runner_sha256,
        "policy": metadata.policy.label(),
        "tests": tests,
    });
    let out = Path::new(path);
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(
        out,
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

pub(crate) struct Expectations {
    /// The renderer the file was written under. Absent in files written before
    /// the field existed; those cannot distinguish a Stylo from a Livery
    /// baseline by content.
    pub(crate) renderer: Option<String>,
    pub(crate) policy: ExpectationPolicy,
    pub(crate) tests: BTreeMap<String, ExpectedRecord>,
}

pub(crate) fn retain_opted_in_tests(tests: &mut Vec<TestCase>, expectations: &Expectations) {
    debug_assert_eq!(expectations.policy, ExpectationPolicy::OptIn);
    tests.retain(|test| expectations.tests.contains_key(test.name()));
}

pub(crate) fn load_expectations(path: &str) -> Result<Expectations, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("expectations read failed ({path}): {e}"))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("expectations parse failed ({path}): {e}"))?;
    let renderer = value
        .get("renderer")
        .and_then(serde_json::Value::as_str)
        .map(str::to_ascii_lowercase);
    let command = value
        .get("command")
        .and_then(serde_json::Value::as_str)
        .map(str::to_ascii_lowercase);
    let policy = match value.get("policy") {
        None | Some(serde_json::Value::Null) => ExpectationPolicy::Exact,
        Some(serde_json::Value::String(policy)) => ExpectationPolicy::parse(policy)
            .ok_or_else(|| format!("expectations file {path} has unknown policy `{policy}`"))?,
        Some(_) => {
            return Err(format!(
                "expectations file {path} must carry a string `policy`"
            ));
        },
    };
    let tests = value.get("tests").unwrap_or(&value);
    let obj = tests.as_object().ok_or_else(|| {
        format!("expectations file {path} must be an object or carry a `tests` object")
    })?;
    let mut out = BTreeMap::new();
    for (name, expected) in obj {
        let record = match expected {
            serde_json::Value::String(status) => ExpectedRecord {
                status: status.to_ascii_lowercase(),
                reason: None,
                subtests: None,
                subtest_results: None,
            },
            serde_json::Value::Object(fields) => {
                let Some(status) = fields.get("status").and_then(serde_json::Value::as_str) else {
                    return Err(format!(
                        "expectation for {name} must carry a string `status` field"
                    ));
                };
                let reason = match fields.get("reason") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(serde_json::Value::String(reason)) => Some(reason.clone()),
                    Some(_) => {
                        return Err(format!(
                            "expectation for {name} must carry a string `reason` field"
                        ));
                    },
                };
                let count = |field: &str| -> Result<Option<usize>, String> {
                    match fields.get(field) {
                        None | Some(serde_json::Value::Null) => Ok(None),
                        Some(value) => value.as_u64().map(|n| Some(n as usize)).ok_or_else(|| {
                            format!("expectation for {name} must carry an integer `{field}`")
                        }),
                    }
                };
                let subtests = match (count("subtests_passed")?, count("subtests_total")?) {
                    (Some(passed), Some(total)) => Some((passed, total)),
                    (None, None) => None,
                    _ => {
                        return Err(format!(
                            "expectation for {name} must carry both subtest count fields or \
                             neither"
                        ));
                    },
                };
                let subtest_results = match fields.get("subtests") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(serde_json::Value::Array(subtests)) => {
                        let mut parsed = Vec::with_capacity(subtests.len());
                        for (index, subtest) in subtests.iter().enumerate() {
                            let Some(subtest) = subtest.as_object() else {
                                return Err(format!(
                                    "expectation for {name} subtest {index} must be an object"
                                ));
                            };
                            let Some(subtest_name) =
                                subtest.get("name").and_then(serde_json::Value::as_str)
                            else {
                                return Err(format!(
                                    "expectation for {name} subtest {index} lacks a string `name`"
                                ));
                            };
                            let Some(status) =
                                subtest.get("status").and_then(serde_json::Value::as_str)
                            else {
                                return Err(format!(
                                    "expectation for {name} subtest `{subtest_name}` lacks a string \
                                     `status`"
                                ));
                            };
                            let status = status.to_ascii_lowercase();
                            if !matches!(
                                status.as_str(),
                                "pass" | "fail" | "timeout" | "not-run" | "precondition-failed"
                            ) {
                                return Err(format!(
                                    "expectation for {name} subtest `{subtest_name}` has unknown \
                                     status `{status}`"
                                ));
                            }
                            let reason = match subtest.get("reason") {
                                None | Some(serde_json::Value::Null) => None,
                                Some(serde_json::Value::String(reason))
                                    if !reason.trim().is_empty()
                                        && !reason
                                            .trim()
                                            .to_ascii_lowercase()
                                            .starts_with("todo") =>
                                {
                                    Some(reason.clone())
                                },
                                Some(serde_json::Value::String(_)) => {
                                    return Err(format!(
                                        "expectation for {name} subtest `{subtest_name}` needs a \
                                         meaningful `reason`, not an empty or TODO placeholder"
                                    ));
                                },
                                Some(_) => {
                                    return Err(format!(
                                        "expectation for {name} subtest `{subtest_name}` must carry \
                                         a string `reason`"
                                    ));
                                },
                            };
                            if policy == ExpectationPolicy::OptIn
                                && status != "pass"
                                && reason.is_none()
                            {
                                return Err(format!(
                                    "opt-in expectation for {name} subtest `{subtest_name}` must \
                                     explain the expected `{status}` result with `reason`"
                                ));
                            }
                            parsed.push(ExpectedSubtest {
                                name: subtest_name.to_string(),
                                status,
                                reason,
                            });
                        }
                        Some(parsed)
                    },
                    Some(_) => {
                        return Err(format!(
                            "expectation for {name} must carry an array `subtests` field"
                        ));
                    },
                };
                if let (Some((passed, total)), Some(named)) = (subtests, &subtest_results) {
                    let named_passed = named
                        .iter()
                        .filter(|subtest| subtest.status == "pass")
                        .count();
                    if named.len() != total || named_passed != passed {
                        return Err(format!(
                            "expectation for {name} says {passed}/{total} subtests but its named \
                             metadata says {named_passed}/{}",
                            named.len()
                        ));
                    }
                }
                ExpectedRecord {
                    status: status.to_ascii_lowercase(),
                    reason,
                    subtests,
                    subtest_results,
                }
            },
            _ => {
                return Err(format!(
                    "expectation for {name} must be a string or an object with `status`"
                ));
            },
        };
        if policy == ExpectationPolicy::OptIn {
            match record.status.as_str() {
                "pass" => {},
                "fail"
                    if command.as_deref() == Some("testharness")
                        && record.subtest_results.is_none() =>
                {
                    return Err(format!(
                        "opt-in failing expectation for {name} must carry named `subtests` \
                         metadata"
                    ));
                },
                "fail" => {},
                "skip" | "error" | "no-results" => {
                    return Err(format!(
                        "opt-in expectation for {name} cannot accept file status `{}`: the \
                         runtime `reason` is not a human policy reason; opt in after the file \
                         reaches testharness results",
                        record.status
                    ));
                },
                other => {
                    return Err(format!(
                        "opt-in expectation for {name} has unknown file status `{other}`"
                    ));
                },
            }
        }
        out.insert(name.clone(), record);
    }
    Ok(Expectations {
        renderer,
        policy,
        tests: out,
    })
}

pub(crate) fn check_expectations(
    path: &str,
    renderer: &str,
    actuals: &[ActualRecord],
) -> Result<(), String> {
    let expectations = load_expectations(path)?;
    if let Some(pinned) = expectations.renderer.as_deref()
        && pinned != renderer
    {
        return Err(format!(
            "expectations: baseline {path} was written under renderer `{pinned}`, but this run \
             used `--renderer {renderer}`"
        ));
    }
    let expected = expectations.tests;
    let actual_names: BTreeSet<&str> = actuals.iter().map(|a| a.test.as_str()).collect();
    let mut unexpected = Vec::new();
    for actual in actuals {
        match expected.get(&actual.test) {
            Some(record) if record.matches(actual) => {},
            Some(record) => {
                unexpected.push(format!("{}: {}", actual.test, record.mismatch(actual)))
            },
            None => unexpected.push(format!(
                "{}: missing expectation, got {}",
                actual.test,
                ActualRecordDisplay(actual)
            )),
        }
    }
    for expected_name in expected.keys() {
        if !actual_names.contains(expected_name.as_str()) {
            let status = expected
                .get(expected_name)
                .map(ExpectedRecord::describe)
                .unwrap_or_else(|| "<missing>".to_string());
            unexpected.push(format!(
                "{expected_name}: expected {status}, but test was not run"
            ));
        }
    }
    if unexpected.is_empty() {
        return Ok(());
    }
    let mut msg = format!("expectations: unexpected={} ({path})", unexpected.len());
    for line in unexpected.iter().take(40) {
        msg.push_str("\n  ");
        msg.push_str(line);
    }
    if unexpected.len() > 40 {
        msg.push_str(&format!("\n  … and {} more", unexpected.len() - 40));
    }
    Err(msg)
}

/// Phase 3 / harness-exactness H2b: run each testharness test on **both** engines
/// (Boa + Nova) and diff. A test that passes on Boa but fails on Nova is a **Nova
/// JS-engine gap** (Nova's worklist, the fork-improvement signal); a test that
/// fails on both is a **genet-platform gap** (layout / DOM). Disk mode only.
pub(crate) fn compare(tests: &[TestCase], args: &Args) {
    let tests_root = Path::new(&args.tests_root);
    let testharness_js = match fs::read_to_string(tests_root.join("resources/testharness.js")) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("testharness.js not found under {}", tests_root.display());
            std::process::exit(2);
        },
    };
    // Boa / Nova can panic on unimplemented paths; swallow the hooks like `testharness`.
    let prev = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));

    let (mut both_pass, mut both_fail, mut boa_only, mut nova_only, mut skipped) = (0, 0, 0, 0, 0);
    let mut nova_worklist: Vec<String> = Vec::new();

    for test in tests {
        let html = match build_test_html_disk(test) {
            TestHtml::Html(h) => h,
            TestHtml::Skip(_) => {
                skipped += 1;
                continue;
            },
            TestHtml::ReadError => {
                skipped += 1;
                continue;
            },
        };
        let base_dir = test.path.parent().unwrap_or(tests_root);
        let disk = harness::DiskLoader::new(base_dir, tests_root);
        let doc_url = test.disk_doc_url();
        let run = |engine| {
            panic::catch_unwind(AssertUnwindSafe(|| {
                harness::run_test(
                    &testharness_js,
                    &html,
                    &disk,
                    Some(&doc_url),
                    None,
                    None,
                    engine,
                )
            }))
        };
        let boa = run(harness::Engine::Boa);
        let nova = run(harness::Engine::Nova);
        let name = test.name();
        match (outcome_passes(&boa), outcome_passes(&nova)) {
            (true, true) => both_pass += 1,
            (false, false) => both_fail += 1,
            (false, true) => nova_only += 1,
            (true, false) => {
                boa_only += 1;
                nova_worklist.push(name.to_string());
                if args.verbose {
                    println!("NOVA-GAP  {name}");
                }
            },
        }
    }
    panic::set_hook(prev);

    println!(
        "\ncompare [{}]: both-pass={both_pass} both-fail={both_fail} (genet-platform gap) \
         boa-only={boa_only} (Nova gap) nova-only={nova_only} skipped={skipped}",
        if args.subset.is_empty() {
            "<all>"
        } else {
            &args.subset
        },
    );
    if !nova_worklist.is_empty() {
        println!(
            "\nNova worklist (pass on Boa, fail on Nova) — {} test(s):",
            nova_worklist.len()
        );
        for name in nova_worklist.iter().take(40) {
            println!("  {name}");
        }
        if nova_worklist.len() > 40 {
            println!("  … and {} more", nova_worklist.len() - 40);
        }
    }
}
