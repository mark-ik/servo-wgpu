// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! HTML's parsing model with scripts interleaved: the drive loop that owns
//! *when* a script runs, over the tree sink that owns *what* the tree looks
//! like.
//!
//! The regression this exists against is structural. Genet used to parse a
//! whole document, copy the finished tree into the arena, and only then run its
//! scripts in document order. Everything that observes the tree *as it is being
//! built* is wrong under that model, and each case fails differently: a custom
//! element defined by an early script is not yet in the registry when a later
//! tag is parsed; a declarative `<template shadowrootmode>` cannot consult a
//! registry that is still empty; a `MutationObserver` registered by an early
//! script sees one bulk tree instead of the parser's inserts; and
//! `document.write` has no token stream to write into.
//!
//! The loop is small because html5ever hands over at exactly the right point:
//!
//! ```text
//! open the document's source stream, push source
//! loop {
//!     resume()                       -> Done | Created | Script(node)
//!     Created:
//!         upgrade the elements just created  (a definition exists)
//!     Script(node):
//!         upgrade parser-created custom elements
//!         classify the element       (classic / module, inline / external, async / defer)
//!         run it, with currentScript set
//!         microtask checkpoint       (this is where a MutationObserver fires)
//!         refresh the policy table   (registry facts the tree builder asks for)
//! }
//! end()
//! readyState = interactive; readystatechange
//! deferred scripts, in document order
//! DOMContentLoaded
//! readyState = complete; readystatechange; window load
//! ```
//!
//! `document.write` is not in that loop, because HTML has the parser process
//! written characters *during* the `write()` call. The tokenizer therefore
//! lives in [`crate::dom::markup_insertion`]'s host state rather than here, so
//! the native can feed it; when the written source reaches a `<script>` the
//! native stalls the pause and this loop takes it, because script *timing* —
//! `async`, `defer`, modules, external sources — belongs here.
//!
//! The arena is *moved* between the host state and the stream's cell around
//! each tokenizer call, rather than shared behind a second `RefCell`. A
//! tree-sink call and a native call both want `&mut ScriptedDom`, and they are
//! never live at the same instant — the tokenizer has returned before any
//! script runs — so moving is both sufficient and cheap (`ScriptedDom` is a
//! handful of maps behind one pointer each). Sharing it instead would put a
//! re-entrant borrow one careless native away from a panic.

use std::cell::RefCell;
use std::rc::Rc;

use genet_scripted_dom::ScriptedDom;
use genet_scripted_dom::parser::{DomAccess, ParsePause, ParserPolicy};
use layout_dom_api::{LayoutDom, LocalName, Namespace};
use script_engine_api::ScriptEngine;

use crate::dom::markup_insertion;
use crate::dom::markup_insertion::ReadyState;
use crate::{NodeId, Runtime};

/// Where a parser-blocking `<script src>` gets its source.
///
/// Deliberately synchronous: the scripted tier's existing resource route is,
/// and a parser-blocking script blocks the parser by definition. An `async`
/// script is fetched through the same route, which makes it available
/// immediately — see [`ScriptTiming`].
pub trait ParserScriptLoader {
    /// The text of an external classic or module script, or `None` to skip it
    /// (a missing resource, a blocked one, or a route that does not serve this
    /// URL).
    fn load(&self, src: &str, charset: Option<&str>, integrity: Option<&str>) -> Option<String>;

    /// The base URL an external script's own imports resolve against.
    fn resolve(&self, src: &str) -> String {
        src.to_owned()
    }
}

/// A loader that serves nothing — the `parse()` entry point with no document
/// URL, where an external script has no route to travel.
pub struct NoScriptLoader;

impl ParserScriptLoader for NoScriptLoader {
    fn load(&self, _src: &str, _charset: Option<&str>, _integrity: Option<&str>) -> Option<String> {
        None
    }
}

/// How a `<script>` the parser popped is timed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScriptTiming {
    /// Runs now, blocking the parser: an inline classic, or an external classic
    /// with neither `async` nor `defer`.
    Blocking,
    /// External classic with `async`. HTML runs it "as soon as it is
    /// available", by *queuing a task* — it never blocks the parser, and a
    /// later parser-blocking script therefore runs first even though the async
    /// script's fetch finished earlier.
    ///
    /// The resource route here is synchronous, so the fetch completes during
    /// the parse and the task is queued during the parse; the earliest the
    /// event loop can turn is when the tokenizer stops. So the driver collects
    /// these and runs them at the end of the token stream, after readiness
    /// becomes `interactive` and before the deferred list — HTML's "stop
    /// parsing" step 3 then its spin-the-event-loop at step 5. Ordering among
    /// async scripts is not promised by HTML; the driver runs them in
    /// fetch-completion order, which for a synchronous route is document order.
    Async,
    /// External classic with `defer`, and every module script: after parsing,
    /// in document order, before `DOMContentLoaded`.
    Deferred,
    /// A data block (`type` naming neither a classic nor a module script), or a
    /// script already marked "already started" — by the tokenizer's EOF
    /// handling, by a fragment parse, or by an earlier preparation. Never runs.
    Skipped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScriptKind {
    Classic,
    Module,
}

/// What the driver needs to know about one popped `<script>`.
struct ScriptFacts {
    kind: ScriptKind,
    timing: ScriptTiming,
    src: Option<String>,
    charset: Option<String>,
    integrity: Option<String>,
    text: String,
}

/// `type`/`language` classification, per HTML's "prepare the script element".
pub(crate) fn classify(ty: Option<&str>, language: Option<&str>) -> Option<ScriptKind> {
    const CLASSIC: &[&str] = &[
        "application/ecmascript",
        "application/javascript",
        "application/x-ecmascript",
        "application/x-javascript",
        "text/ecmascript",
        "text/javascript",
        "text/javascript1.0",
        "text/javascript1.1",
        "text/javascript1.2",
        "text/javascript1.3",
        "text/javascript1.4",
        "text/javascript1.5",
        "text/jscript",
        "text/livescript",
        "text/x-ecmascript",
        "text/x-javascript",
    ];
    match ty.map(str::trim) {
        // A `type` attribute that is present but empty makes the script
        // classic outright: `language` is only consulted when there is no
        // `type` at all, per HTML's "prepare the script element".
        Some("") => Some(ScriptKind::Classic),
        None => match language {
            // `language="javascript"` is the legacy spelling of a classic script.
            Some(lang) if !lang.is_empty() => {
                let mime = format!("text/{}", lang.to_ascii_lowercase());
                CLASSIC
                    .contains(&mime.as_str())
                    .then_some(ScriptKind::Classic)
            },
            _ => Some(ScriptKind::Classic),
        },
        // `type` is matched ASCII case-insensitively, so `type="MODULE"` is a
        // module script.
        Some(ty) if ty.eq_ignore_ascii_case("module") => Some(ScriptKind::Module),
        Some(ty) => {
            let mime = ty
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            CLASSIC
                .contains(&mime.as_str())
                .then_some(ScriptKind::Classic)
        },
    }
}

/// The arena, parked in a cell while the tokenizer runs. See the module note:
/// the arena is moved in and out around each `resume`, never shared.
pub(crate) struct ParkedDom(pub(crate) Rc<RefCell<ScriptedDom>>);

impl DomAccess for ParkedDom {
    fn with<R>(&self, f: impl FnOnce(&mut ScriptedDom) -> R) -> R {
        f(&mut self.0.borrow_mut())
    }
}

/// What a completed parse reports back.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ParseReport {
    /// Classic scripts that ran, parser-blocking and async.
    pub scripts_run: usize,
    /// Deferred classic and module scripts that ran after parsing.
    pub deferred_run: usize,
    /// `document.write` calls whose source re-entered the token stream.
    pub writes_applied: usize,
}

impl<E: ScriptEngine> Runtime<E> {
    /// Parse `html` into this runtime's document **with scripts interleaved**,
    /// per HTML's parsing model, and run the document's load sequence
    /// (`readyState` transitions, deferred scripts, `DOMContentLoaded`,
    /// `load`).
    ///
    /// This is the scripted tier's document entry point. The script-free route
    /// (`StaticDocument::parse`) is untouched and still parses in one pass.
    pub fn parse_document_interleaved(
        &mut self,
        html: &str,
        loader: &dyn ParserScriptLoader,
    ) -> ParseReport {
        self.parse_document_interleaved_with(html, loader, true)
    }

    /// [`parse_document_interleaved`](Self::parse_document_interleaved) with
    /// control over the final `load` event.
    ///
    /// A host that owns its own completion handshake passes `false` and
    /// dispatches `load` itself. Exactly one of the two may: the WPT runner
    /// arms `testharness.js` before the parse and completes on the `load` it
    /// dispatches afterwards, so a second dispatch from here would report the
    /// suite twice.
    pub fn parse_document_interleaved_with(
        &mut self,
        html: &str,
        loader: &dyn ParserScriptLoader,
        dispatch_load: bool,
    ) -> ParseReport {
        let mut report = ParseReport::default();
        // One tokenizer, and it lives in the host's markup state rather than
        // here, because `document.write` must be able to feed it from inside a
        // native: HTML has the parser process written characters during the
        // `write()` call, not after the calling script returns.
        let policy = {
            let mut host = self.host.borrow_mut();
            markup_insertion::open_stream_begin(&mut host);
            markup_insertion::stream_push_source(&mut host, html);
            host.markup.parser_active = true;
            host.markup.writes_applied = 0;
            host.markup.ready_state = ReadyState::Loading;
            markup_insertion::stream_policy(&host).expect("stream just opened")
        };
        // The document object and the window's named properties are bound
        // before the first script can look at either.
        let _ = self
            .engine
            .eval("globalThis.__rebindDocument(); globalThis.__refreshNamedProperties()");

        let mut deferred: Vec<(NodeId, ScriptFacts)> = Vec::new();
        // Async scripts do not block the parser, so they are not run here. See
        // `ScriptTiming::Async`.
        let mut async_pending: Vec<(NodeId, ScriptFacts)> = Vec::new();
        loop {
            let pause = markup_insertion::stream_resume(&mut self.host.borrow_mut());
            let node = match pause {
                ParsePause::Done => break,
                // A definition exists and the parser just made a candidate:
                // upgrade it here, at creation, rather than at the next script.
                ParsePause::Created => {
                    self.upgrade_parser_created(&policy);
                    continue;
                },
                ParsePause::Script(node) => node,
            };
            // Upgrade first, so the script about to run sees the elements the
            // parser created since the last pause already upgraded.
            self.upgrade_parser_created(&policy);
            // Foreign-namespace scripts do not pause the tokenizer, so run any
            // the parser has completed since the last pause — they precede this
            // one in the document, and this one may name what they defined.
            report.scripts_run +=
                self.run_foreign_scripts(&policy, loader, &mut deferred, &mut async_pending);
            let already_started = markup_insertion::script_started(&self.host.borrow(), node);
            // HTML never executes a `<script>` found in a template's contents.
            let inert = already_started || self.host.borrow().dom.is_in_template_contents(node);
            let facts = self.script_facts(node, inert);
            // Prepare's step 10, and it happens whatever the timing: a deferred
            // or async script is already started the moment it is prepared, so
            // moving it later cannot re-run it.
            markup_insertion::clear_parser_inserted(&self.host.borrow(), node);
            if !inert {
                markup_insertion::mark_script_started(&mut self.host.borrow_mut(), node);
            }
            match facts.timing {
                ScriptTiming::Skipped => {},
                ScriptTiming::Deferred => deferred.push((node, facts)),
                ScriptTiming::Async => async_pending.push((node, facts)),
                ScriptTiming::Blocking => {
                    self.run_parser_script(node, &facts, loader);
                    report.scripts_run += 1;
                },
            }
            // Refresh *after* the script: it may have defined a custom
            // element, and the next stretch of tokenizing is what asks.
            self.refresh_parser_policy(&policy);
        }
        {
            let mut host = self.host.borrow_mut();
            markup_insertion::open_stream_close(&mut host);
            host.markup.parser_active = false;
            report.writes_applied = host.markup.writes_applied;
        }
        self.refresh_parser_policy(&policy);
        self.upgrade_parser_created(&policy);
        report.scripts_run +=
            self.run_foreign_scripts(&policy, loader, &mut deferred, &mut async_pending);

        // The final tokenizer stretch may contain frames without a following
        // blocking script. Discover their contexts and queue document loads
        // before handing the completed parse back to the host event loop.
        let _ = self
            .engine
            .eval("globalThis.__refreshNamedProperties && __refreshNamedProperties()");

        // HTML, "the end". Readiness first, then the tasks already queued (the
        // async scripts, whose fetches completed while the parser ran), then
        // the deferred list, then DOMContentLoaded, then the load event.
        self.set_ready_state(ReadyState::Interactive);
        for (node, facts) in &async_pending {
            if !self.host.borrow().dom.is_live(*node) {
                continue;
            }
            self.run_parser_script(*node, facts, loader);
            report.scripts_run += 1;
        }
        for (node, facts) in &deferred {
            // A preceding queued script may have adopted this prepared node
            // into another arena. Execution cannot follow it there or run in
            // the old preparation realm after its document has changed.
            if !self.host.borrow().dom.is_live(*node) {
                continue;
            }
            self.run_parser_script_deferred(facts, loader);
            report.deferred_run += 1;
        }
        let _ = self
            .engine
            .eval("document.dispatchEvent(new Event('DOMContentLoaded', { bubbles: true }));");
        self.run_microtasks();
        if dispatch_load && self.agent.borrow_mut().frames.defer_main_load() {
            // The final child completion performs Complete/readystatechange/load.
            return report;
        }
        self.set_ready_state(ReadyState::Complete);
        if dispatch_load {
            let _ = self.engine.eval("window.dispatchEvent(new Event('load'));");
            self.run_microtasks();
        }
        report
    }

    /// Refresh the table the tree builder reads at every question it asks: the
    /// custom element names whose definition disables shadow. The registry can
    /// only change while a script runs, so refreshing here makes the table
    /// exactly current for the whole next stretch of tokenizing.
    fn refresh_parser_policy(&mut self, policy: &Rc<ParserPolicy>) {
        let names = self
            .engine
            .eval("globalThis.__ceShadowDisabledNames ? __ceShadowDisabledNames() : ''")
            .ok()
            .and_then(|v| self.engine.value_to_string(&v).ok())
            .unwrap_or_default();
        policy.set_shadow_disabled(
            names
                .split(',')
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
        );
        // While anything is defined, the driver takes control at element
        // creation so the constructor runs there rather than at the next
        // script pause. Nothing is defined in the overwhelming majority of
        // documents, and then the parser feeds whole buffers as before.
        let defined = self
            .engine
            .eval("globalThis.__ceDefinedCount ? __ceDefinedCount() : 0")
            .ok()
            .and_then(|v| self.engine.value_to_string(&v).ok())
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);
        policy.set_upgrade_at_creation(defined > 0);
    }

    /// Run the custom-element upgrade for elements the parser created since the
    /// last pause, so a definition made by an earlier script has taken effect
    /// on the tree the next script sees.
    fn upgrade_parser_created(&mut self, policy: &Rc<ParserPolicy>) {
        let created = policy.take_custom_candidates();
        if created.is_empty() {
            return;
        }
        let live: Vec<String> = {
            let host = self.host.borrow();
            created
                .into_iter()
                .filter(|&id| host.dom.is_live(id))
                .map(|id| id.raw().to_string())
                .collect()
        };
        if live.is_empty() {
            return;
        }
        let _ = self.engine.eval(&format!(
            "globalThis.__ceUpgradeParsed && __ceUpgradeParsed('{}')",
            live.join(",")
        ));
    }

    /// Run the `<script>` elements the parser created in a foreign namespace
    /// (SVG, MathML).
    ///
    /// html5ever only hands the driver a pause for an HTML-namespace script, so
    /// these are collected at creation and drained at the next pause and again
    /// at the end of the parse. That puts each one before the next HTML script,
    /// which is the ordering the SVG corpus actually depends on; the residual
    /// deviation — HTML would run it the moment its own end tag is popped — is
    /// named in the lane plan.
    fn run_foreign_scripts(
        &mut self,
        policy: &Rc<ParserPolicy>,
        loader: &dyn ParserScriptLoader,
        deferred: &mut Vec<(NodeId, ScriptFacts)>,
        async_pending: &mut Vec<(NodeId, ScriptFacts)>,
    ) -> usize {
        let nodes = policy.take_foreign_scripts();
        let mut ran = 0;
        for node in nodes {
            let skip = {
                let host = self.host.borrow();
                !host.dom.is_live(node) || host.dom.is_in_template_contents(node)
            };
            if skip {
                continue;
            }
            let started = markup_insertion::script_started(&self.host.borrow(), node);
            let facts = self.script_facts(node, started);
            markup_insertion::clear_parser_inserted(&self.host.borrow(), node);
            markup_insertion::mark_script_started(&mut self.host.borrow_mut(), node);
            match facts.timing {
                ScriptTiming::Skipped => continue,
                // A module in foreign content is deferred like any other, and
                // must still run before the load event.
                ScriptTiming::Deferred => deferred.push((node, facts)),
                ScriptTiming::Async => async_pending.push((node, facts)),
                ScriptTiming::Blocking => {
                    self.run_parser_script(node, &facts, loader);
                    ran += 1;
                },
            }
        }
        ran
    }

    fn set_ready_state(&mut self, state: ReadyState) {
        self.host.borrow_mut().markup.ready_state = state;
        let _ = self
            .engine
            .eval("document.dispatchEvent(new Event('readystatechange'));");
    }

    /// Read the `<script>` element's attributes and text out of the arena.
    fn script_facts(&mut self, node: NodeId, already_started: bool) -> ScriptFacts {
        let host = self.host.borrow();
        let dom = &host.dom;
        let html = Namespace::from("");
        let attr = |name: &str| {
            dom.attribute(node, &html, &LocalName::from(name))
                .map(str::to_owned)
        };
        let text = dom
            .dom_children(node)
            .filter_map(|c| dom.text(c))
            .collect::<String>();
        let src = attr("src").filter(|s| !s.is_empty());
        let kind = classify(attr("type").as_deref(), attr("language").as_deref());
        let timing = match (kind, already_started) {
            (None, _) | (_, true) => ScriptTiming::Skipped,
            (Some(ScriptKind::Module), _) => ScriptTiming::Deferred,
            (Some(ScriptKind::Classic), _) => match &src {
                // Inline classic: always parser-blocking; `async` and `defer`
                // have no effect without `src`.
                None => ScriptTiming::Blocking,
                Some(_) if attr("async").is_some() => ScriptTiming::Async,
                Some(_) if attr("defer").is_some() => ScriptTiming::Deferred,
                Some(_) => ScriptTiming::Blocking,
            },
        };
        ScriptFacts {
            kind: kind.unwrap_or(ScriptKind::Classic),
            timing,
            src,
            charset: attr("charset"),
            integrity: attr("integrity"),
            text,
        }
    }

    /// Execute one parser-blocking or async classic script, with
    /// `document.currentScript` set for its duration.
    fn run_parser_script(
        &mut self,
        node: NodeId,
        facts: &ScriptFacts,
        loader: &dyn ParserScriptLoader,
    ) {
        let source = match &facts.src {
            None => Some(facts.text.clone()),
            Some(src) => loader.load(src, facts.charset.as_deref(), facts.integrity.as_deref()),
        };
        let Some(source) = source else {
            return;
        };
        // Window named properties are live over the tree, and the tree just
        // grew: a script that names an element parsed since the last pause
        // (`ordinarytemplate.innerHTML = ...`) must find it.
        let _ = self
            .engine
            .eval("globalThis.__refreshNamedProperties && __refreshNamedProperties()");
        self.host.borrow_mut().markup.current_script = Some(node);
        let _ = self.engine.eval(&source);
        self.flush_host_trace_events();
        self.host.borrow_mut().markup.current_script = None;
        // The microtask checkpoint after a script is where a MutationObserver
        // callback registered by an earlier script actually runs — and the
        // reason the parser can see a shadow root that callback attached.
        self.run_microtasks();
        let _ = self
            .engine
            .eval("globalThis.__refreshNamedProperties && __refreshNamedProperties()");
    }

    /// Execute one deferred classic or module script after parsing.
    fn run_parser_script_deferred(&mut self, facts: &ScriptFacts, loader: &dyn ParserScriptLoader) {
        match facts.kind {
            ScriptKind::Classic => {
                let source = match &facts.src {
                    None => Some(facts.text.clone()),
                    Some(src) => {
                        loader.load(src, facts.charset.as_deref(), facts.integrity.as_deref())
                    },
                };
                if let Some(source) = source {
                    let _ = self.engine.eval(&source);
                    self.flush_host_trace_events();
                }
            },
            ScriptKind::Module => {
                let (source, base) = match &facts.src {
                    None => (Some(facts.text.clone()), String::new()),
                    Some(src) => (
                        loader.load(src, facts.charset.as_deref(), facts.integrity.as_deref()),
                        loader.resolve(src),
                    ),
                };
                if let Some(source) = source {
                    // An import's own URL is resolved by the loader, which is
                    // the document's resource route; the referrer is already
                    // folded into that route's base.
                    //
                    // `ScriptEngine::eval_module`'s resolver returns
                    // `(resolved_url, source)` — in that order. This closure had
                    // them swapped, so every backend parsed the *URL string* as
                    // the dependency's source and the import rejected with the
                    // failure swallowed. Nothing caught it because the only
                    // tests that reach a second module were themselves ignored
                    // over `resolve_href`.
                    let mut resolve = |specifier: &str, _referrer: &str| {
                        let url = loader.resolve(specifier);
                        loader.load(specifier, None, None).map(|text| (url, text))
                    };
                    let _ = self.engine.eval_module(&source, &base, &mut resolve);
                    self.flush_host_trace_events();
                }
            },
        }
        self.run_microtasks();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_script_types() {
        assert_eq!(classify(None, None), Some(ScriptKind::Classic));
        assert_eq!(classify(Some(""), None), Some(ScriptKind::Classic));
        assert_eq!(classify(Some("module"), None), Some(ScriptKind::Module));
        assert_eq!(classify(Some("MODULE"), None), Some(ScriptKind::Module));
        assert_eq!(classify(Some("Module"), None), Some(ScriptKind::Module));
        // A present-but-empty `type` is classic outright: `language` only
        // speaks when there is no `type` at all.
        assert_eq!(
            classify(Some(""), Some("vbscript")),
            Some(ScriptKind::Classic)
        );
        assert_eq!(
            classify(Some("text/javascript; charset=utf-8"), None),
            Some(ScriptKind::Classic)
        );
        assert_eq!(classify(Some("text/plain"), None), None);
        assert_eq!(classify(Some("application/json"), None), None);
        assert_eq!(
            classify(None, Some("javascript")),
            Some(ScriptKind::Classic)
        );
        assert_eq!(classify(None, Some("vbscript")), None);
    }
}
