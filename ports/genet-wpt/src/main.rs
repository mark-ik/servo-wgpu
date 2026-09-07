/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! genet-native web-platform-tests runner.
//!
//! Runs a **selectable subset** of `tests/wpt` against genet, so a single
//! subsystem can be checked without the whole 1.2 GB suite.
//!
//! Phase 1 (this binary) is a **crash-smoke**: each runnable test is loaded
//! through the owned Livery + Buckram route, wrapped in `catch_unwind`. A test "passes" if
//! loading does not panic. That finds layout panics across real pages, the
//! highest-leverage early signal, and needs no GPU and no JS. Reftest pixel
//! comparison and testharness.js are later phases.
//!
//! Cf. `docs/2026-05-26_wpt_runner_plan.md`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use genet_livery::{
    table_block::TableBlockSkip,
    table_shadow::{TableShadowLedger, TableShadowSkip},
};
use genet_static_dom::StaticDocument;
use layout_dom_api::{LayoutDom, LocalName};
use script_engine_api::ScriptEngine;

mod args;
mod expectations;
#[cfg(feature = "netfetch")]
mod net;
mod reftest;
mod test262_cmd;
mod testharness;
#[cfg(test)]
mod tests;

use args::*;
use expectations::*;
use reftest::*;
use test262_cmd::*;
use testharness::*;

mod conformance;
mod harness;
mod manifest;
mod render;
mod test262;
mod testdriver;
#[cfg(test)]
mod webgl_conformance;

// The upstream WPT checkout lives under `tests/wpt/tests/`
// (`tests/wpt/mozilla/` holds servo-specific tests).
const DEFAULT_TESTS_ROOT: &str = "tests/wpt/tests";
const VIEWPORT_W: f32 = 800.0;
const VIEWPORT_H: f32 = 600.0;
// Reftest render size (the WPT default viewport).
const REFTEST_W: u32 = 800;
const REFTEST_H: u32 = 600;

// GPU anti-aliasing jitter floor. Vello rasterization is not bit-exact
// run-to-run: two renders of identical input differ by up to ~1/255 on a
// sub-1% sliver of (anti-aliased edge) pixels. Exact-match scoring (0,0)
// therefore flips borderline tests between runs, making the pass count
// non-deterministic. This floor — at most `FUZZ_FLOOR_DIFF` per-channel
// delta on at most one hundredth of the physical raster pixels — absorbs exactly that
// jitter and nothing near a real paint bug (those differ by 255 over a
// localized region). Applied as a *lower bound* on every comparison: a
// test's own `<meta name=fuzzy>` still wins where it is looser.
const FUZZ_FLOOR_DIFF: u16 = 1;

fn fuzz_floor_pixels(viewport: render::RenderViewport) -> u64 {
    let (width, height) = viewport.device_size();
    (width as u64 * height as u64) / 100
}

/// WPT test classification (convention-based; see the plan doc).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Reference,
    Manual,
    Reftest,
    PrintReftest,
    Crashtest,
    Testharness,
    Load,
}

impl Kind {
    fn from_manifest(kind: manifest::TestKind) -> Kind {
        match kind {
            manifest::TestKind::Testharness => Kind::Testharness,
            manifest::TestKind::Reftest => Kind::Reftest,
            manifest::TestKind::PrintReftest => Kind::PrintReftest,
            manifest::TestKind::Crashtest => Kind::Crashtest,
            manifest::TestKind::Manual
            | manifest::TestKind::Visual
            | manifest::TestKind::Wdspec => Kind::Manual,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Reference => "reference",
            Kind::Manual => "manual",
            Kind::Reftest => "reftest",
            Kind::PrintReftest => "print-reftest",
            Kind::Crashtest => "crashtest",
            Kind::Testharness => "testharness",
            Kind::Load => "load",
        }
    }

    /// Phase 1 runs the crash-smoke on everything except references and
    /// manual tests. (Reftest/testharness still only get the load-smoke
    /// here; their real verification is phases 2/3.)
    fn runs_in_phase1(self) -> bool {
        !matches!(self, Kind::Reference | Kind::Manual)
    }
}

fn is_html(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("html" | "htm" | "xht" | "xhtml")
    )
}

/// True for XHTML/XML documents (parse with xml5ever, not html5ever), keyed on the
/// file extension — the reliable signal. Content sniffing misroutes HTML files
/// that merely mention "xhtml" in a doctype or comment.
fn is_xml_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("xht" | "xhtml" | "xml")
    )
}

/// Classify a test by filename + path conventions and a cheap content scan.
fn classify(path: &Path, contents: &str) -> Kind {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    let path_str = path.to_string_lossy().replace('\\', "/");

    // References are not tests themselves.
    if stem.ends_with("-ref")
        || stem.ends_with(".ref")
        || stem.starts_with("ref-")
        || path_str.contains("/reference/")
    {
        return Kind::Reference;
    }
    if stem.ends_with("-manual") {
        return Kind::Manual;
    }
    if path_str.contains("/crashtests/") || stem.ends_with("-crash") {
        return Kind::Crashtest;
    }
    if contents.contains("rel=\"match\"")
        || contents.contains("rel=match")
        || contents.contains("rel=\"mismatch\"")
        || contents.contains("rel=mismatch")
    {
        return Kind::Reftest;
    }
    if contents.contains("testharness.js") {
        return Kind::Testharness;
    }
    Kind::Load
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Passed,
    Failed,
    Skipped,
    ReadError,
}

#[derive(Clone)]
struct TestCase {
    /// Backing file inside `tests_root`. Generated WPT variants such as
    /// `/foo.any.html` point back to `foo.any.js`.
    path: PathBuf,
    /// Runnable WPT URL, tests-root-relative and without the leading slash. This
    /// is the stable identity used for listings, expectations, and server mode.
    url: String,
    kind: Kind,
    refs: Vec<(String, manifest::RefMatch)>,
    fuzzy: Option<FuzzyRange>,
    long_timeout: bool,
    from_manifest: bool,
}

impl TestCase {
    fn from_walk(path: PathBuf, tests_root: &str) -> TestCase {
        let contents = fs::read(&path)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let kind = classify(&path, &contents);
        TestCase {
            url: rel(&path, tests_root),
            path,
            kind,
            refs: Vec::new(),
            fuzzy: None,
            long_timeout: false,
            from_manifest: false,
        }
    }

    fn from_manifest(test: manifest::ManifestTest, tests_root: &Path) -> Option<TestCase> {
        if test.is_unhostable_variant() {
            return None;
        }
        let source_path = strip_url_path(&test.source_path);
        if source_path.is_empty() {
            return None;
        }
        Some(TestCase {
            path: tests_root.join(source_path),
            url: normalize_test_url(&test.url),
            kind: Kind::from_manifest(test.kind),
            refs: test.refs,
            fuzzy: manifest_fuzzy_range(test.fuzzy),
            long_timeout: test.long_timeout,
            from_manifest: true,
        })
    }

    /// The one test a `testharness-one` worker was handed: its backing file and
    /// its runnable URL, which the parent already resolved from the manifest.
    fn single(path: PathBuf, url: String) -> TestCase {
        TestCase {
            path,
            url: normalize_test_url(&url),
            kind: Kind::Testharness,
            refs: Vec::new(),
            fuzzy: None,
            long_timeout: false,
            from_manifest: true,
        }
    }

    fn name(&self) -> &str {
        &self.url
    }

    fn disk_doc_url(&self) -> String {
        format!("http://web-platform.test/{}", self.url)
    }
}

fn normalize_test_url(url: &str) -> String {
    url.trim_start_matches('/').replace('\\', "/")
}

fn strip_url_path(url_or_path: &str) -> &str {
    url_or_path
        .trim_start_matches('/')
        .split(['#', '?'])
        .next()
        .unwrap_or(url_or_path)
}

type FuzzyRange = ((u16, u16), (u64, u64));

fn manifest_fuzzy_range(fuzzy: Option<((u32, u32), (u32, u32))>) -> Option<FuzzyRange> {
    fuzzy.and_then(|((diff_lo, diff_hi), (total_lo, total_hi))| {
        (diff_lo <= diff_hi && total_lo <= total_hi).then_some((
            (
                diff_lo.min(u32::from(u16::MAX)) as u16,
                diff_hi.min(u32::from(u16::MAX)) as u16,
            ),
            (u64::from(total_lo), u64::from(total_hi)),
        ))
    })
}

/// Crash-smoke one test: parse + cascade + layout, catching panics.
fn smoke_test(test: &TestCase) -> (Kind, Outcome) {
    let kind = test.kind;
    if !kind.runs_in_phase1() {
        return (kind, Outcome::Skipped);
    }
    let html = match load_test_document_disk(test) {
        TestHtml::Html(html) => html,
        TestHtml::Skip(_) => return (kind, Outcome::Skipped),
        TestHtml::ReadError => return (kind, Outcome::ReadError),
    };

    let is_xml = is_xml_path(&test.path);
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        let document = if is_xml {
            StaticDocument::parse_xml(&html)
        } else {
            StaticDocument::parse(&html)
        };
        let sheets = genet_document_resources::ResolvedDocumentResources::discover(&document, None)
            .stylesheets
            .into_iter()
            .map(|sheet| sheet.text)
            .collect::<Vec<_>>();
        let sheet_refs: Vec<&str> = sheets.iter().map(String::as_str).collect();
        let mut session = genet_livery::LiveryDocument::new(
            document,
            genet_livery::StyleSet::cambium(&sheet_refs),
            genet_livery::Device::screen(VIEWPORT_W, VIEWPORT_H),
        );
        let _ = session.frame(VIEWPORT_W as u32, VIEWPORT_H as u32);
    }));

    (
        kind,
        if result.is_ok() {
            Outcome::Passed
        } else {
            Outcome::Failed
        },
    )
}

/// True for a WPT `.any.js` / `.window.js` / `.worker.js` test (a JS file the
/// harness wraps into a generated HTML page rather than a standalone document).
fn is_any_js(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.ends_with(".any.js") || name.ends_with(".window.js") || name.ends_with(".worker.js")
}

/// Collect HTML + `.any.js`-style test files under `root` (a dir or a single file).
fn collect(root: &Path, out: &mut Vec<PathBuf>) {
    if root.is_file() {
        if is_html(root) || is_any_js(root) {
            out.push(root.to_path_buf());
        }
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            // WPT excludes `tools/` and `support/` directories from test
            // collection: they hold test-generation templates and helper
            // resources (images, fragments referenced by path), not tests. A
            // `tools/*-template.html` carries a `rel=match` to a ref that does
            // not exist, so collecting it produces a spurious `ref-missing`
            // error. Hidden dirs (`.git`, …) are skipped too.
            if matches!(
                path.file_name().and_then(|n| n.to_str()),
                Some("tools" | "support")
            ) || path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'))
            {
                continue;
            }
            collect(&path, out);
        } else if is_html(&path) || is_any_js(&path) {
            out.push(path);
        }
    }
}

/// Synthesize the testharness HTML wrapper for a `.any.js` / `.window.js` test:
/// load testharness.js + the test's `// META: script=...` helpers + the test
/// file itself, exactly as WPT's build step generates the `.any.html` variant.
/// `run_test` then resolves those `<script src>` the usual way. Returns `None`
/// for worker-only tests (`.worker.js`, or `.any.js` whose `global=` excludes
/// window), which this window-shaped runner can't host.
fn synthesize_any_js(path: &Path, variant_url: Option<&str>) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    if name.ends_with(".worker.js") {
        return None;
    }
    let src = fs::read_to_string(path).ok()?;
    let (scripts, globals) = harness::meta_directives(&src);
    // window-shaped run: only `.any.js` whose globals include window (or the
    // dedicated-window aliases) is hostable here.
    let window_ok = globals.as_deref().is_none_or(|g| {
        g.split(',').any(|tok| {
            matches!(tok.trim(), "window" | "default") || tok.trim().starts_with("window")
        })
    });
    // `.window.js` is inherently window-scoped; the global directive only gates
    // `.any.js`.
    if name.ends_with(".any.js") && !window_ok {
        return None;
    }
    // `self.GLOBAL` is injected by WPT's `.any.html` wrapper (tools/serve/serve.py)
    // before testharness.js; tests branch on it (`GLOBAL.isWorker()`), so synthesize
    // the window-shaped stub here too or those files throw at load.
    let mut html = String::from(
        "<!doctype html><meta charset=utf-8>\n\
         <script>self.GLOBAL={isWindow:function(){return true;},isWorker:function(){return false;},isShadowRealm:function(){return false;}};</script>\n\
         <script src=\"/resources/testharness.js\"></script>\n\
         <script src=\"/resources/testharnessreport.js\"></script>\n",
    );
    for s in scripts {
        html.push_str(&format!("<script src=\"{s}\"></script>\n"));
    }
    let query = variant_url
        .and_then(|u| {
            u.split_once('?')
                .map(|(_, q)| q.split('#').next().unwrap_or(q))
        })
        .filter(|q| !q.is_empty());
    let test_src = match query {
        Some(q) => format!("{name}?{q}"),
        None => name.to_string(),
    };
    html.push_str(&format!("<script src=\"{test_src}\"></script>\n"));
    Some(html)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ExpectationPolicy {
    #[default]
    Exact,
    OptIn,
}

impl ExpectationPolicy {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "exact" => Some(Self::Exact),
            "opt-in" | "optin" => Some(Self::OptIn),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::OptIn => "opt-in",
        }
    }
}

fn main() {
    // Deep JS recursion (e.g. re-entrant DOM event dispatch that never terminates) reaches
    // Nova's 3500-execution-context limit, but each level's native footprint (host dispatch +
    // bytecode-interpreter frames) is large enough to overflow the OS thread stack first on
    // the default (~1MB on Windows) — crashing the whole process instead of throwing a
    // catchable "Maximum call stack size exceeded". Run the entire runner on a large stack so
    // that limit is reached first. Engine-agnostic (Boa benefits too); also hardens the
    // test262 workers, whose crash would otherwise count as a skip.
    let handle = std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(real_main)
        .expect("failed to spawn runner thread");
    if handle.join().is_err() {
        std::process::exit(101); // the panic message was already printed by the default hook
    }
}

fn real_main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        },
    };

    harness::set_drive_deadline_secs(args.drive_deadline_secs);

    // bench needs only the tests root (for resources/testharness.js), not a subset
    // walk; handle it before the corpus collection below.
    if args.command == "bench" {
        harness::bench(&args.tests_root);
        return;
    }

    // `manifest` enumerates from MANIFEST.json (the authoritative WPT test list),
    // not the directory walk — handled before the corpus collection so it can be
    // diffed against `collect` (harness-exactness H1: spot-check the counts match).
    if args.command == "manifest" {
        manifest_list(&args);
        return;
    }

    if args.command == "conformance" {
        conformance_cmd(&args);
        return;
    }

    // `test262` runs its own vendored corpus (third_party/test262), not the WPT walk.
    if args.command == "test262" {
        test262_cmd(&args);
        return;
    }
    // The per-test worker the parent `test262` run spawns for hang isolation.
    if args.command == "test262-one" {
        test262_one(&args);
        return;
    }
    // The same, for `testharness`: one test, one process, one encoded result.
    if args.command == "testharness-one" {
        testharness_one(&args);
        return;
    }

    if !args.reftest_results.is_empty()
        || !args.testharness_results.is_empty()
        || args.write_conformance.is_some()
        || args.conformance_baseline.is_some()
        || args.allow_incomplete_conformance
        || args.allow_unpinned_conformance_inputs
    {
        eprintln!("conformance result flags are supported only for `conformance`");
        std::process::exit(2);
    }

    if (args.expectations.is_some() || args.write_expectations.is_some())
        && !matches!(args.command.as_str(), "testharness" | "reftest")
    {
        eprintln!(
            "--expectations / --write-expectations are supported for `testharness` and `reftest`"
        );
        std::process::exit(2);
    }

    if args.expectation_policy != ExpectationPolicy::Exact && args.write_expectations.is_none() {
        eprintln!("--expectation-policy is used only with --write-expectations");
        std::process::exit(2);
    }

    if args.write_table_ledger.is_some()
        && !(args.command == "reftest" && args.renderer == ReftestRenderer::Livery)
    {
        eprintln!("--write-table-ledger requires `reftest --renderer livery`");
        std::process::exit(2);
    }

    if args.reference_verification.is_some() && args.command != "reftest" {
        eprintln!("--reference-verification is supported only for `reftest`");
        std::process::exit(2);
    }

    let mut tests = discover_tests(&args);
    if tests.is_empty() {
        eprintln!(
            "no runnable tests found for '{}' under {}",
            if args.subset.is_empty() {
                "<all>"
            } else {
                &args.subset
            },
            args.tests_root
        );
        std::process::exit(1);
    }

    if let Some(path) = &args.expectations {
        let expectations = match load_expectations(path) {
            Ok(expectations) => expectations,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            },
        };
        if expectations.policy == ExpectationPolicy::OptIn {
            retain_opted_in_tests(&mut tests, &expectations);
            if args.verbose {
                println!(
                    "expectations: opt-in policy selected {} test(s)",
                    tests.len()
                );
            }
        }
    }

    match args.command.as_str() {
        "list" => list(&tests, &args),
        "run" => run(&tests, &args),
        "reftest" => reftest(&tests, &args),
        "dump" => dump(&tests, &args),
        "testharness" => testharness(&tests, &args),
        "compare" => compare(&tests, &args),
        other => {
            eprintln!("unknown command: {other}\n{}", usage());
            std::process::exit(2);
        },
    }
}

fn conformance_cmd(args: &Args) {
    if args.walk_discovery {
        eprintln!("`conformance` requires manifest discovery");
        std::process::exit(2);
    }
    if args.expectations.is_some() || args.write_expectations.is_some() {
        eprintln!(
            "`conformance` reads exact result files; use --reftest-results and \
             --testharness-results"
        );
        std::process::exit(2);
    }
    let path = manifest_path(&args.tests_root);
    let manifest_sha256 = match conformance::sha256_file(&path) {
        Ok(digest) => digest,
        Err(error) => {
            eprintln!(
                "cannot fingerprint WPT manifest {}: {error}",
                path.display()
            );
            std::process::exit(1);
        },
    };
    let manifest = match manifest::Manifest::load(&path) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("cannot load WPT manifest {}: {error}", path.display());
            std::process::exit(1);
        },
    };
    let tests = manifest.tests_under(&args.subset);
    if tests.is_empty() {
        eprintln!(
            "no manifest tests found for '{}'",
            if args.subset.is_empty() {
                "<all>"
            } else {
                &args.subset
            }
        );
        std::process::exit(1);
    }
    let report = match conformance::build_report(
        &tests,
        conformance::ReportInputs {
            subset: &args.subset,
            renderer: args.renderer.label(),
            testharness_engine: args.engine.label(),
            manifest_sha256: &manifest_sha256,
            reftest_results: &args.reftest_results,
            testharness_results: &args.testharness_results,
            allow_incomplete: args.allow_incomplete_conformance,
            allow_unpinned: args.allow_unpinned_conformance_inputs,
        },
    ) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("conformance report failed: {error}");
            std::process::exit(1);
        },
    };
    println!("{}", report.human_summary());
    if let Some(path) = &args.write_conformance {
        if let Err(error) = report.write(path) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        println!("conformance report written to {}", path.display());
    }
    if let Some(path) = &args.conformance_baseline {
        let baseline = match conformance::ConformanceReport::read(path) {
            Ok(report) => report,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            },
        };
        match report.delta_from(&baseline) {
            Ok(delta) => println!("{delta}"),
            Err(error) => {
                eprintln!("cannot compare conformance reports: {error}");
                std::process::exit(1);
            },
        }
    }
}

fn manifest_path(tests_root: &str) -> PathBuf {
    Path::new(tests_root)
        .parent()
        .map(|p| p.join("meta/MANIFEST.json"))
        .unwrap_or_else(|| PathBuf::from("MANIFEST.json"))
}

fn discover_tests(args: &Args) -> Vec<TestCase> {
    let tests_root = Path::new(&args.tests_root);
    if args.walk_discovery {
        let root = tests_root.join(&args.subset);
        if !root.exists() {
            eprintln!("subset path does not exist: {}", root.display());
            std::process::exit(2);
        }
        let mut paths = Vec::new();
        collect(&root, &mut paths);
        return paths
            .into_iter()
            .map(|path| TestCase::from_walk(path, &args.tests_root))
            .collect();
    }

    let path = manifest_path(&args.tests_root);
    let manifest = match manifest::Manifest::load(&path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!(
                "manifest load failed ({}): {e}; pass --walk-discovery to use the legacy directory walk",
                path.display()
            );
            std::process::exit(1);
        },
    };
    manifest
        .tests_under(&args.subset)
        .into_iter()
        .filter_map(|test| TestCase::from_manifest(test, tests_root))
        .collect()
}

/// Enumerate tests under a subset from MANIFEST.json (harness-exactness H1), for
/// diffing the authoritative manifest enumeration against the directory walk. The
/// manifest sits at `<tests-root>/../meta/MANIFEST.json`. Shared/service-worker and
/// shadow-realm variants are counted but excluded from the runnable total; dedicated
/// workers are hostable and count as runnable.
fn manifest_list(args: &Args) {
    let manifest_path = manifest_path(&args.tests_root);
    let manifest = match manifest::Manifest::load(&manifest_path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("manifest load failed ({}): {e}", manifest_path.display());
            std::process::exit(1);
        },
    };
    let tests = manifest.tests_under(&args.subset);
    let mut total = 0usize;
    let mut unhostable = 0usize;
    let mut workers = 0usize;
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for t in &tests {
        if t.is_unhostable_variant() {
            unhostable += 1;
            continue;
        }
        total += 1;
        if t.is_dedicated_worker() {
            workers += 1;
        }
        *counts.entry(t.kind.label()).or_default() += 1;
        if args.verbose {
            println!("{:<12} {}", t.kind.label(), t.url);
        }
    }
    let by_kind: Vec<String> = counts.iter().map(|(k, n)| format!("{k}={n}")).collect();
    println!(
        "manifest: {total} runnable test(s) under '{}' ({}); {workers} dedicated-worker variant(s); {unhostable} unhostable variant(s) skipped",
        if args.subset.is_empty() {
            "<all>"
        } else {
            &args.subset
        },
        by_kind.join(", "),
    );
}

fn rel(path: &Path, tests_root: &str) -> String {
    path.strip_prefix(tests_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn list(tests: &[TestCase], _args: &Args) {
    let mut counts = [0usize; 7];
    let mut manifest_backed = 0usize;
    for test in tests {
        let kind = test.kind;
        if test.from_manifest {
            manifest_backed += 1;
        }
        counts[kind as usize] += 1;
        let timeout = if test.long_timeout { " long" } else { "" };
        println!("{:<12} {}{}", kind.label(), test.name(), timeout);
    }
    println!(
        "\n{} test variant(s): {} reftest, {} print-reftest, {} testharness, {} crashtest, {} load, {} manual, {} reference{}",
        tests.len(),
        counts[Kind::Reftest as usize],
        counts[Kind::PrintReftest as usize],
        counts[Kind::Testharness as usize],
        counts[Kind::Crashtest as usize],
        counts[Kind::Load as usize],
        counts[Kind::Manual as usize],
        counts[Kind::Reference as usize],
        if manifest_backed == tests.len() {
            " (manifest-backed)"
        } else {
            ""
        },
    );
}

fn run(tests: &[TestCase], args: &Args) {
    // Quiet the default panic hook so crash-smoke failures do not spam
    // backtraces; the runner reports them itself.
    let prev = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));

    let (mut passed, mut failed, mut skipped, mut errored) = (0, 0, 0, 0);
    for test in tests {
        let (kind, outcome) = smoke_test(test);
        match outcome {
            Outcome::Passed => {
                passed += 1;
                if args.verbose {
                    println!("PASS  {:<12} {}", kind.label(), test.name());
                }
            },
            Outcome::Failed => {
                failed += 1;
                println!("FAIL  {:<12} {}", kind.label(), test.name());
            },
            Outcome::ReadError => {
                errored += 1;
                println!("ERROR read    {}", test.name());
            },
            Outcome::Skipped => {
                skipped += 1;
                if args.verbose {
                    println!("SKIP  {:<12} {}", kind.label(), test.name());
                }
            },
        }
    }

    panic::set_hook(prev);

    println!(
        "\ncrash-smoke: {} passed, {} failed, {} errored, {} skipped (of {} files)",
        passed,
        failed,
        errored,
        skipped,
        tests.len()
    );
    if failed > 0 || errored > 0 {
        std::process::exit(1);
    }
}

/// Resolve the server-mode context from the args: spawn a `wpt serve`, connect to
/// one, or `None` (disk mode). `--spawn-server` wins over `--server-base`. A
/// requested-but-unreachable server is fatal (the run would silently fall back to
/// network errors otherwise).
#[cfg(feature = "netfetch")]
fn setup_server(args: &Args) -> Option<net::ServerCtx> {
    if !args.spawn_server && args.server_base.is_none() {
        return None;
    }
    // The WPT server's https / h2 origins use a self-signed CA; trust any
    // certificate so https:// and h2 fetches reach it. Must run before the first
    // request (the readiness probe below builds the shared client).
    netfetcher::accept_invalid_certs();
    let result = if args.spawn_server {
        eprintln!("spawning `wpt serve` under {} ...", args.tests_root);
        net::ServerCtx::spawn(Path::new(&args.tests_root))
    } else {
        net::ServerCtx::connect(args.server_base.clone().unwrap())
    };
    match result {
        Ok(s) => {
            eprintln!("server mode: driving fetch against {}", s.origin);
            net::set_page_origin(&s.origin);
            Some(s)
        },
        Err(e) => {
            eprintln!("server mode setup failed: {e}");
            std::process::exit(2);
        },
    }
}

/// Disk-mode testharness HTML for a test path: the file's contents (testharness
/// only; XHTML and non-testharness skipped) or a synthesized `.any.js` wrapper.
/// Mirrors the disk branch of [`testharness`], shared by [`compare`].
enum TestHtml {
    Html(String),
    Skip(&'static str),
    ReadError,
}

/// Which global a `.any.js`-style variant runs in. Dedicated workers became
/// hostable with the worker lane; shared and service workers and shadow realms
/// keep their skip reasons.
enum VariantGlobal {
    Window,
    DedicatedWorker,
    Unhostable(&'static str),
}

fn variant_global(test: &TestCase) -> VariantGlobal {
    let name = test.name();
    if name.contains(".sharedworker.") {
        return VariantGlobal::Unhostable("sharedworker-unsupported");
    }
    if name.contains(".serviceworker.") {
        return VariantGlobal::Unhostable("serviceworker-unsupported");
    }
    if name.contains(".shadowrealm-") {
        return VariantGlobal::Unhostable("shadowrealm-unsupported");
    }
    let file = test
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if file.ends_with(".worker.js") || name.contains(".worker.") || name.contains(".worker?") {
        return VariantGlobal::DedicatedWorker;
    }
    // Walk discovery has no variant expansion, so the `global=` directive is the
    // only thing that says which global a `.any.js` belongs in.
    if file.ends_with(".any.js") {
        let src = fs::read_to_string(&test.path).unwrap_or_default();
        let (_, globals) = harness::meta_directives(&src);
        let Some(globals) = globals else {
            return VariantGlobal::Window;
        };
        let tokens: Vec<&str> = globals.split(',').map(str::trim).collect();
        let window_ok = tokens
            .iter()
            .any(|t| *t == "default" || t.starts_with("window"));
        if !window_ok {
            let worker_ok = tokens
                .iter()
                .any(|t| matches!(*t, "worker" | "dedicatedworker" | "default"));
            return if worker_ok {
                VariantGlobal::DedicatedWorker
            } else {
                VariantGlobal::Unhostable("non-window-global")
            };
        }
    }
    VariantGlobal::Window
}

/// WPT's generated `<name>.worker.html` / `<stem>.any.worker.html`: a window that
/// starts the worker and pipes its testharness results back through the same
/// results bridge. The worker script itself comes off the resource route.
fn synthesize_worker_wrapper(test: &TestCase) -> Option<String> {
    let name = test.path.file_name()?.to_str()?;
    let script = if name.ends_with(".worker.js") {
        name.to_owned()
    } else {
        format!("{}.any.worker.js", name.strip_suffix(".any.js")?)
    };
    let query = test
        .name()
        .split_once('?')
        .map(|(_, q)| q.split('#').next().unwrap_or(q))
        .filter(|q| !q.is_empty());
    let script = match query {
        Some(q) => format!("{script}?{q}"),
        None => script,
    };
    Some(format!(
        "<!doctype html><meta charset=utf-8>\n\
         <script src=\"/resources/testharness.js\"></script>\n\
         <script src=\"/resources/testharnessreport.js\"></script>\n\
         <div id=log></div>\n\
         <script>fetch_tests_from_worker(new Worker(\"{script}\"));</script>\n"
    ))
}

fn load_test_document_disk(test: &TestCase) -> TestHtml {
    if is_any_js(&test.path) {
        return match variant_global(test) {
            VariantGlobal::Unhostable(reason) => TestHtml::Skip(reason),
            VariantGlobal::DedicatedWorker => match synthesize_worker_wrapper(test) {
                Some(h) => TestHtml::Html(h),
                None => TestHtml::Skip("dedicated-worker-unsupported"),
            },
            VariantGlobal::Window => match synthesize_any_js(&test.path, Some(test.name())) {
                Some(h) => TestHtml::Html(h),
                None => TestHtml::Skip("non-window-global"),
            },
        };
    }
    let Ok(bytes) = fs::read(&test.path) else {
        return TestHtml::ReadError;
    };
    TestHtml::Html(String::from_utf8_lossy(&bytes).into_owned())
}

fn build_test_html_disk(test: &TestCase) -> TestHtml {
    if test.kind != Kind::Testharness {
        return TestHtml::Skip("non-testharness");
    }
    let ext = test.path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("xhtml") || ext.eq_ignore_ascii_case("xht") {
        return TestHtml::Skip("xhtml"); // XML parse mode genet's HTML parser doesn't handle
    }
    load_test_document_disk(test)
}

/// Render each reftest in the subset + its reference to side-by-side
/// PNGs under `.cargo-check-logs/dump/`, for eyeball diagnosis of a
/// `local`-bucket failure. Writes `<stem>.test.png` / `<stem>.ref.png`.
fn dump(tests: &[TestCase], args: &Args) {
    let renderer = match render::Renderer::boot() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cannot boot renderer (needs a GPU): {e}");
            std::process::exit(1);
        },
    };
    let viewport = render::RenderViewport::new(REFTEST_W, REFTEST_H, args.device_scale)
        .expect("parse_args validates --device-scale");
    let tests_root = Path::new(&args.tests_root);
    let out_dir = Path::new(".cargo-check-logs/dump");
    let _ = fs::create_dir_all(out_dir);
    for test in tests {
        let Ok(bytes) = fs::read(&test.path) else {
            continue;
        };
        let test_html = String::from_utf8_lossy(&bytes).into_owned();
        if test.kind != Kind::Reftest {
            continue;
        }
        let (kind, href) = if let Some((href, cmp)) = test.refs.first() {
            let kind = match cmp {
                manifest::RefMatch::Equal => MatchKind::Match,
                manifest::RefMatch::NotEqual => MatchKind::Mismatch,
            };
            (kind, href.clone())
        } else {
            let Some((kind, href)) = reftest_ref(&test_html) else {
                continue;
            };
            (kind, href)
        };
        let Ok(Some((ref_path, ref_html))) =
            resolve_reftest_reference(&test.path, &href, kind, tests_root)
        else {
            continue;
        };
        let test_dir = test.path.parent().unwrap_or(tests_root);
        let ref_dir = ref_path.parent().unwrap_or(tests_root);
        let render = |html: &str, dir: &Path, xml: bool| {
            renderer
                .render_html(html, dir, tests_root, viewport, xml)
                .image
        };
        let t = render(&test_html, test_dir, is_xml_path(&test.path));
        let r = render(&ref_html, ref_dir, is_xml_path(&ref_path));
        let stem = test
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("dump");
        let scale_suffix = if viewport.device_scale() == 1.0 {
            String::new()
        } else {
            format!("@{}x", viewport.device_scale())
        };
        let tp = out_dir.join(format!("{stem}{scale_suffix}.test.png"));
        let rp = out_dir.join(format!("{stem}{scale_suffix}.ref.png"));
        let _ = t.save(&tp);
        let _ = r.save(&rp);
        let s = diff_stats(&t, &r);
        let pct = (s.differing * 100).checked_div(s.total).unwrap_or(0);
        println!(
            "DUMP {} -> {} / {}  (diff={pct}% maxδ={})",
            test.name(),
            tp.display(),
            rp.display(),
            s.max_channel_diff
        );
    }
}
