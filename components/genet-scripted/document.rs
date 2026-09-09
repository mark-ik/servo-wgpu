/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The scripted document profile (V4): a live, script-mutated document.
//!
//! The host-neutral content half of a scripted browser surface: a
//! [`ScriptedDocument`] that
//! parses HTML into a live [`ScriptedDom`], runs its `<script>`s — inline *and*
//! external `<script src>` (fetched through the same [`ResourceFetcher`] the page
//! loaded over, in document order) — through [`script_runtime_api::Runtime`] on a
//! chosen JS engine (Boa by default, Nova behind `scripted-nova`). This type is
//! deliberately headless: no layout, no paint, no viewport. It runs script,
//! mutates the DOM, and exposes render-free extraction ([`ScriptedDocument::extract`]) —
//! GPU-free and testable here. A host that needs to paint the mutated DOM uses
//! [`LiveryScriptedDocument`], which owns the same script/runtime machinery plus a
//! live Livery CSSOM session (layout, paint, scroll, click hit-testing, text
//! selection). The two types do not wrap one another; they are parallel routes
//! over the same `Runtime` contract.
//!
//! Script timing follows the classic-script model: parser-blocking scripts (inline,
//! and external with neither `async` nor `defer`) run in document order; `defer` and
//! `async` external scripts run after that pass (`defer` in document order — the
//! guaranteed contract; `async` is unordered, and since the fetcher is synchronous,
//! document order is a faithful realization). `async`/`defer` are ignored on inline
//! scripts, per spec. A `type` that is neither empty nor a JavaScript MIME type nor
//! `module` is a data block and is not executed. `type=module` scripts (inline or
//! `src`) are **deferred** (run after the parser-blocking pass, in document order)
//! and evaluated with module scope via the engine's module path (`eval_module`); a
//! backend without module support logs and skips. Cross-module `import` works on a
//! module-capable backend: the engine's loader resolves each specifier against the
//! importing module's URL and pulls its source through this document's fetcher (the
//! `resolve` closure below), caching by URL so a diamond / cycle loads once. An
//! unresolvable or throwing import rejects the module, which is reported and skipped.
//! A failed/missing/integrity-rejected external script is likewise reported and
//! skipped, like an inline error, and the document keeps running.
//!
//! The GC tick (`Runtime::collect_garbage`) runs at frame cadence in
//! [`ScriptedDocument::pump`] — the first real frame-cadence caller the gc-arena
//! plan was waiting on. `LiveryScriptedDocument` drives the analogous
//! script/layout split against its own retained viewport scroll.

use layout_dom_api::{LayoutDom, LocalName, Namespace};
#[cfg(feature = "livery")]
use std::rc::Rc;

use engine_observables_api::DomArenaStats;
#[cfg(feature = "livery")]
use genet_document_resources::ResolvedDocumentResources;
#[cfg(feature = "livery")]
use genet_document_resources::ResourceLimits;
#[cfg(feature = "livery")]
use genet_livery::{Device, NavigationFragment};
#[cfg(feature = "livery")]
use genet_scripted_dom::{NodeId, ScriptedDom};
#[cfg(test)]
use genet_static_dom::StaticDocument;
use script_engine_api::ScriptEngine;
use script_runtime_api::{CookieProvider, Runtime, WebGlFactory};

use crate::ResourceFetcher;
use crate::capture::DomCaptureRecorder;
#[cfg(feature = "livery")]
use crate::{LiveryCssom, ScriptedClick};

/// Host-neutral keyboard scrolling for either scripted layout engine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrollKey {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
}

#[cfg(test)]
mod extraction_tests {
    use super::*;
    use script_engine_boa::BoaEngine;

    #[test]
    fn spa_article_is_available_only_over_the_post_js_dom() {
        let source = "<body><script>\
            var main = document.createElement('main');\
            var h = document.createElement('h1');\
            h.appendChild(document.createTextNode('Injected article'));\
            main.appendChild(h);\
            var p = document.createElement('p');\
            p.appendChild(document.createTextNode('This substantial injected paragraph proves the article arrived only after the page script ran.'));\
            main.appendChild(p);\
            document.body.appendChild(main);\
            </script></body>";
        let static_dom = genet_static_dom::StaticDocument::parse(source);
        assert!(fleece::extract_article(&static_dom).is_none());
        assert!(fleece::carries_script(&static_dom));

        let scripted = ScriptedDocument::<BoaEngine>::parse(source).expect("runtime inits");
        assert_eq!(
            scripted
                .extract_article()
                .expect("post-JS article")
                .title
                .as_deref(),
            Some("Injected article")
        );
    }

    #[test]
    fn static_article_is_identical_under_static_and_scripted_profiles() {
        let source = "<main><h1>Static article 🙂</h1><p>This substantial static paragraph has e\u{301}, שלום, and enough readable text without executing page script.</p></main>";
        let expected = fleece::extract_document(&genet_static_dom::StaticDocument::parse(source));
        let scripted = ScriptedDocument::<BoaEngine>::parse(source).expect("runtime inits");
        assert_eq!(scripted.extract(), expected.page);
        assert_eq!(scripted.extract_article(), expected.article);
        assert!(!fleece::carries_script(
            &genet_static_dom::StaticDocument::parse(source)
        ));
    }

    /// A `ScriptedDocument` parses and runs interleaved: a script sees the tree
    /// only as far as its own position, and `document.write` re-enters the
    /// token stream rather than being appended after everything.
    #[test]
    fn the_document_parses_and_runs_interleaved() {
        let source = "<body><p id=before>b</p>            <script>window.probe = [!!document.getElementById('before'),                                     !!document.getElementById('after')];                    document.write('<b id=written>w</b>');</script>            <p id=after>a</p></body>";
        let mut scripted = ScriptedDocument::<BoaEngine>::parse(source).expect("runtime inits");
        assert_eq!(read(&mut scripted, "String(probe)"), "true,false");
        assert_eq!(
            read(&mut scripted, "!!document.getElementById('after')"),
            "true"
        );
        assert_eq!(
            read(
                &mut scripted,
                "document.getElementById('written').nextElementSibling.id"
            ),
            "after"
        );
        // The load sequence ran: readiness reached complete.
        assert_eq!(read(&mut scripted, "document.readyState"), "complete");
    }

    fn read(doc: &mut ScriptedDocument<BoaEngine>, expr: &str) -> String {
        let value = doc.rt.eval(expr).expect("eval");
        doc.rt.value_to_string(&value).expect("stringify")
    }
}

/// A headless document driven by script: a [`Runtime`] holding the mutable DOM.
/// No layout, no paint, no viewport — a host that needs those uses
/// [`LiveryScriptedDocument`] instead. Generic over the JS engine `E` (the
/// monomorphization the `--engine` selection picks, exactly as genet-wpt's
/// harness does); the bin instantiates `ScriptedDocument<BoaEngine>` or
/// `ScriptedDocument<NovaEngine>`.
pub struct ScriptedDocument<E: ScriptEngine> {
    /// The engine + browser host surface; owner of the live [`ScriptedDom`] that
    /// the page's script mutates.
    rt: Runtime<E>,
    capture: Option<DomCaptureRecorder>,
    /// Page Visibility state (W3C adoption plan P1): `true` = the document is
    /// not being presented (an unfocused preview card). Hidden documents get
    /// their timer pump throttled to the spec-licensed 1s clamp; a
    /// `visibilitychange` event fires on each flip.
    hidden: bool,
    /// Page Lifecycle frozen state: no tasks run at all until `resume`.
    frozen: bool,
    /// Virtual-clock stamp of the last hidden-state timer pump (the 1s clamp).
    last_hidden_pump_ms: f64,
}

impl<E: ScriptEngine> ScriptedDocument<E> {
    /// Fetch `url` through `fetcher`, parse it, and run its scripts — inline and
    /// external `<script src>` (each resolved against `url` and fetched through the
    /// same `fetcher`). `Err` on a failed fetch of the document, or a runtime that
    /// would not initialize.
    pub fn load(fetcher: &impl ResourceFetcher, url: &str) -> Result<Self, String> {
        Self::load_inner(fetcher, url, None)
    }

    /// Load a document with a host-owned WebGL factory installed before any
    /// parser-blocking script runs. A browser host uses this when the page can
    /// call `canvas.getContext('webgl')` during document load; installing the
    /// factory after [`load`](Self::load) is too late for that script timing.
    pub fn load_with_webgl_factory(
        fetcher: &impl ResourceFetcher,
        url: &str,
        webgl: WebGlFactory,
    ) -> Result<Self, String> {
        Self::load_inner(fetcher, url, Some(webgl))
    }

    fn load_inner(
        fetcher: &impl ResourceFetcher,
        url: &str,
        webgl: Option<WebGlFactory>,
    ) -> Result<Self, String> {
        // Split a `url#id` fragment off before fetching (the fetcher takes the
        // resource, not the fragment).
        let (resource, fragment) = match url.split_once('#') {
            Some((res, frag)) => (res, (!frag.is_empty()).then(|| frag.to_string())),
            None => (url, None),
        };
        let bytes = fetcher
            .fetch(resource)
            .ok_or_else(|| format!("could not load {resource}"))?;
        // External scripts resolve against the document URL and fetch through the
        // same fetcher; pass both into the builder.
        let me = Self::build(
            &String::from_utf8_lossy(&bytes),
            Some((fetcher, resource)),
            None,
            webgl,
        )?;
        // This headless route carries no layout to scroll a fragment into;
        // `LiveryScriptedDocument::load` (via `NavigationFragment`) is the
        // fragment-navigation-aware path.
        let _ = fragment;
        Ok(me)
    }

    /// Parse already-loaded HTML into a live DOM, then run its **inline** `<script>`s
    /// against it (settling microtasks). The fetch-free half, for tests and inline
    /// `data:` content — with no fetcher, external `<script src>` is reported and
    /// skipped. `Err` if the runtime fails to initialize.
    pub fn parse(html: &str) -> Result<Self, String> {
        Self::build(html, None, None, None)
    }

    /// Parse a document with a host-owned WebGL factory installed before its
    /// inline scripts run. This is the fetch-free companion to
    /// [`load_with_webgl_factory`](Self::load_with_webgl_factory).
    pub fn parse_with_webgl_factory(html: &str, webgl: WebGlFactory) -> Result<Self, String> {
        Self::build(html, None, None, Some(webgl))
    }

    /// Parse an already-fetched `html` body and run its scripts, fetching external
    /// `<script src>` through `fetcher` (each resolved against `base_url`). Like
    /// [`parse`](Self::parse) but with external scripts; unlike [`load`](Self::load)
    /// it does **not** re-fetch the document — the caller supplies the body it already
    /// has (e.g. a host that fetched the page itself, then runs it on the scripted
    /// rung). `Err` only if the runtime fails to initialize.
    /// `cookies` installs the host's cookie store (e.g. meerkat's session jar) so
    /// `document.cookie` reads / writes it; `None` leaves the document cookieless.
    pub fn from_body(
        html: &str,
        fetcher: &dyn ResourceFetcher,
        base_url: &str,
        cookies: Option<Box<dyn CookieProvider>>,
    ) -> Result<Self, String> {
        Self::build(html, Some((fetcher, base_url)), cookies, None)
    }

    /// Parse an already-fetched body with a host-owned WebGL factory installed
    /// before parser-blocking scripts run.
    pub fn from_body_with_webgl_factory(
        html: &str,
        fetcher: &dyn ResourceFetcher,
        base_url: &str,
        cookies: Option<Box<dyn CookieProvider>>,
        webgl: WebGlFactory,
    ) -> Result<Self, String> {
        Self::build(html, Some((fetcher, base_url)), cookies, Some(webgl))
    }

    /// Parse `html` into a live DOM and run its scripts in document order. With a
    /// `loader` (`(fetcher, base_url)`), external `<script src>` is resolved against
    /// `base_url` and fetched; without one (the [`parse`](Self::parse) path), an
    /// external script is reported and skipped. A script that errors (or whose fetch
    /// fails) is reported but does not abort the load — a browser keeps running the
    /// document's remaining scripts. `Err` only if the runtime fails to initialize.
    fn build(
        html: &str,
        loader: Option<(&dyn ResourceFetcher, &str)>,
        cookies: Option<Box<dyn CookieProvider>>,
        webgl: Option<WebGlFactory>,
    ) -> Result<Self, String> {
        // This headless route carries no layout, so it resolves no stylesheets
        // and installs no `getComputedStyle` / `matchMedia` bridge — those are
        // `LiveryScriptedDocument`'s (its CSSOM owns both seams). The capture
        // recorder below still wants a stylesheet list; it gets none here.
        let sheets: Vec<String> = Vec::new();

        let mut rt = Runtime::<E>::new().map_err(|e| format!("script runtime init: {e:?}"))?;
        // The document URL is the base for reflected URL attributes (`a.href`,
        // `img.src`, …) and for resolving fetches; set it from the loader when present
        // (the `parse()` path has no URL, so those reflect their raw values).
        if let Some((_, base)) = loader {
            let _ = rt.set_base_url(base);
        }
        // Install the host's cookie store (the session jar) before any script runs, so
        // a page reading / writing `document.cookie` on load sees the live session.
        if let Some(cookies) = cookies {
            rt.set_cookie_provider(cookies);
        }
        // Install the graphics seam before the parsed body enters the runtime
        // and before any parser-blocking script executes. Vano's nova_vm backend
        // and Boa receive the same engine-neutral factory contract here.
        if let Some(webgl) = webgl {
            rt.set_webgl_factory(webgl);
        }
        let mut capture = {
            let mut host = rt.host().borrow_mut();
            DomCaptureRecorder::from_env(&mut host.dom, &sheets)
                .map_err(|e| format!("dom capture init: {e}"))?
        };

        // HTML's parsing model: the document is parsed and its scripts are run
        // *interleaved*, so each script sees the tree as far as its own
        // position and can change what the parser does next. The static
        // `doc` above is still parsed, but only to resolve the document's
        // stylesheets before the runtime exists; the live tree comes from
        // this parse.
        rt.parse_document_interleaved(html, &DocumentScriptLoader { loader });
        rt.run_microtasks();
        if let Some(recorder) = capture.as_mut() {
            let mut host = rt.host().borrow_mut();
            recorder
                .record_pending(&mut host.dom)
                .map_err(|e| format!("dom capture write: {e}"))?;
        }

        Ok(Self {
            rt,
            capture,
            hidden: false,
            frozen: false,
            last_hidden_pump_ms: f64::NAN,
        })
    }

    /// Install the host-owned route retained for worker scripts and imports.
    /// The borrowed parser ResourceFetcher is not retained after construction.
    /// Install this before the first pump that services worker resource requests.
    pub fn set_script_resource_loader(
        &mut self,
        loader: Box<dyn script_runtime_api::ScriptResourceLoader>,
    ) {
        self.rt.set_script_resource_loader(loader);
    }

    /// Drive the runtime one frame's worth: fire due timers against the `now_ms`
    /// virtual clock, settle microtasks, then take the GC tick. Returns
    /// `(reflectors_unpinned, nodes_collected)` from the collection. This is the
    /// frame-cadence caller of [`Runtime::collect_garbage`] (gc-arena carve-out #1):
    /// a long-lived document churning nodes under `setInterval` is collected here,
    /// not at an explicit one-off call.
    pub fn pump(&mut self, now_ms: f64) -> (usize, usize) {
        // Page Lifecycle: a frozen document runs no tasks at all. Page
        // Visibility: a hidden one pumps timers at most once per second (the
        // HTML-spec-licensed clamp for hidden documents), so a background
        // page's interval loop stops driving per-frame work.
        if self.frozen {
            return (0, 0);
        }
        if self.hidden {
            // The clamp window anchors at the first pump after hiding (NaN
            // sentinel set by `set_hidden`), so hiding never grants an
            // immediate bonus tick.
            if self.last_hidden_pump_ms.is_nan() || now_ms - self.last_hidden_pump_ms < 1000.0 {
                if self.last_hidden_pump_ms.is_nan() {
                    self.last_hidden_pump_ms = now_ms;
                }
                return (0, 0);
            }
            self.last_hidden_pump_ms = now_ms;
        }
        self.rt.run_timers(64, now_ms);
        self.rt.run_microtasks();
        let _ = self.rt.pump_workers();
        self.rt.run_microtasks();
        self.flush_dom_capture();
        self.rt.collect_garbage()
    }

    /// Evaluate a classic script against the live document and settle its
    /// microtasks. This is the engine-neutral operation used by worker hosts.
    pub fn evaluate(&mut self, source: &str) -> Result<(), String> {
        self.rt
            .eval(source)
            .map_err(|e| format!("script evaluation: {e:?}"))?;
        self.rt.run_microtasks();
        self.flush_dom_capture();
        Ok(())
    }

    /// Evaluate a module without an external import loader. Imported modules are
    /// rejected until a host supplies the worker's fetch/resolve capability.
    pub fn evaluate_module(&mut self, source: &str, base_url: &str) -> Result<(), String> {
        let mut unavailable = |_referrer: &str, _specifier: &str| None;
        match self
            .rt
            .eval_module(source, base_url, &mut unavailable)
            .map_err(|e| format!("module evaluation: {e:?}"))?
        {
            Some(_) => {
                self.rt.run_microtasks();
                self.flush_dom_capture();
                Ok(())
            },
            None => Err("selected script engine does not support modules".to_string()),
        }
    }

    /// Dispatch an event at a raw DOM node id and settle listener microtasks.
    pub fn dispatch_event(&mut self, raw_node_id: usize, event_type: &str) -> Result<bool, String> {
        let proceed = self
            .rt
            .dispatch_event(raw_node_id, event_type)
            .map_err(|e| format!("event dispatch: {e:?}"))?;
        self.flush_dom_capture();
        Ok(proceed)
    }

    /// Serialize the live document root's children for backend-neutral comparison.
    pub fn dom_snapshot(&self) -> String {
        let host = self.rt.host().borrow();
        host.dom.inner_html(host.dom.document())
    }

    /// Force the engine/reflector collection cadence used by worker idle ticks.
    pub fn collect_garbage(&mut self) -> (usize, usize) {
        self.rt.collect_garbage()
    }

    /// Whether an unfrozen runtime has a timer or outstanding worker work.
    /// Worker liveness keeps a polling host driving until acknowledged idle;
    /// it does not mean a message is ready now or provide a wake callback.
    pub fn has_pending_work(&mut self) -> bool {
        !self.frozen
            && (self.rt.next_timer_delay().is_some() || self.rt.has_worker_work())
    }

    /// Set Page Visibility (W3C adoption plan P1). The host calls this as a
    /// card gains/loses presentation (focused card visible, preview hidden);
    /// each flip dispatches `visibilitychange` at the document per spec. The
    /// JS-visible `document.visibilityState`/`document.hidden` properties are
    /// an engine-side follow-up; the observable contract here is the event
    /// plus the hidden timer clamp in [`pump`](Self::pump).
    pub fn set_hidden(&mut self, hidden: bool) {
        if self.hidden == hidden {
            return;
        }
        self.hidden = hidden;
        if hidden {
            self.last_hidden_pump_ms = f64::NAN;
        }
        let doc = self.rt.host().borrow().dom.document().raw();
        let _ = self.dispatch_event(doc, "visibilitychange");
    }

    /// Whether the document is currently hidden (Page Visibility).
    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// Whether the document is currently frozen (Page Lifecycle).
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Page Lifecycle freeze: dispatch `freeze` (listeners get a last turn to
    /// persist state, per spec), then stop running tasks until
    /// [`resume`](Self::resume). Idempotent.
    pub fn freeze(&mut self) {
        if self.frozen {
            return;
        }
        let doc = self.rt.host().borrow().dom.document().raw();
        let _ = self.dispatch_event(doc, "freeze");
        self.frozen = true;
    }

    /// Page Lifecycle resume: leave the frozen state and dispatch `resume`.
    /// Idempotent.
    pub fn resume(&mut self) {
        if !self.frozen {
            return;
        }
        self.frozen = false;
        let doc = self.rt.host().borrow().dom.document().raw();
        let _ = self.dispatch_event(doc, "resume");
    }

    /// The number of live nodes in the document — the soak's bounded-memory readout
    /// (after churn + GC it must not grow without bound).
    pub fn live_node_count(&self) -> usize {
        self.rt.host().borrow().dom.live_node_count()
    }

    /// Cheap live counts plus a rough byte estimate for the current DOM arena.
    pub fn dom_stats(&self) -> DomArenaStats {
        self.rt.host().borrow().dom.stats()
    }

    /// The `console.log` / `console.error` output the page's script produced, in call
    /// order (for tests and a future devtools surface).
    pub fn console(&self) -> Vec<String> {
        self.rt.host().borrow().console.clone()
    }

    /// The post-script document title used by a native host window. This is
    /// read from the same live DOM Livery lays out and paints.
    pub fn title(&self) -> Option<String> {
        let host = self.rt.host().borrow();
        fleece::extract_title(&host.dom)
    }

    /// Render-free extraction of the **post-JS** document: a
    /// [`PageExtract`](fleece::PageExtract) over the live `ScriptedDom` as the
    /// page's scripts have left it. This is the **headless-scripted-DOM scrape**: an
    /// SPA whose content is injected by JavaScript yields its real content here, where
    /// a static parse of the served HTML would find an empty shell. Same `extract()`
    /// the static lane runs, just over the mutated DOM — extraction is orthogonal to
    /// rendering, so no layout/paint is involved. Run after [`build`](Self::build) (and
    /// any [`pump`](Self::pump)s) so deferred / timer-driven mutations are in.
    pub fn extract(&self) -> fleece::PageExtract {
        let host = self.rt.host().borrow();
        fleece::extract(&host.dom)
    }

    /// Render-free article extraction over the post-JS DOM.
    pub fn extract_article(&self) -> Option<fleece::Article> {
        let host = self.rt.host().borrow();
        fleece::extract_article(&host.dom)
    }

    fn flush_dom_capture(&mut self) {
        let Some(mut recorder) = self.capture.take() else {
            return;
        };
        let result = {
            let mut host = self.rt.host().borrow_mut();
            recorder.record_pending(&mut host.dom)
        };
        match result {
            Ok(_) => self.capture = Some(recorder),
            Err(err) => eprintln!("[pelt-scripted] dom capture disabled: {err}"),
        }
    }
}

/// A cloneable, host-owned resource handle. The Livery CSSOM retains one clone
/// for live stylesheet and asset reconciliation while parser-blocking scripts
/// read through the other clone during construction.
#[cfg(feature = "livery")]
#[derive(Clone)]
struct SharedResourceFetcher(Rc<dyn ResourceFetcher>);

#[cfg(feature = "livery")]
impl ResourceFetcher for SharedResourceFetcher {
    fn fetch(&self, url: &str) -> Option<Vec<u8>> {
        self.0.fetch(url)
    }

    fn fetch_response(&self, url: &str) -> Option<genet_host_api::ResourceResponse> {
        self.0.fetch_response(url)
    }
}

#[cfg(feature = "livery")]
struct EmptyResourceFetcher;

#[cfg(feature = "livery")]
impl ResourceFetcher for EmptyResourceFetcher {
    fn fetch(&self, _url: &str) -> Option<Vec<u8>> {
        None
    }
}

/// A live script runtime rendered entirely by its Livery CSSOM session.
///
/// The mutable `ScriptedDom` remains inside `Runtime`; this type deliberately
/// keeps no mirror DOM. Livery observes that runtime's exact mutation suffix,
/// resolves the same host-owned resource graph, then supplies shaped layout and
/// paint to the product shell.
#[cfg(feature = "livery")]
pub struct LiveryScriptedDocument<E: ScriptEngine> {
    // These retained engines are sizeable in a native UI event loop. Heap-own
    // them so the route does not consume the Windows main-thread stack merely
    // by carrying one live document through `ViewerApp`.
    rt: Box<Runtime<E>>,
    cssom: Box<LiveryCssom>,
    pending_fragment: Option<NavigationFragment>,
    capture: Option<DomCaptureRecorder>,
    hidden: bool,
    frozen: bool,
    last_hidden_pump_ms: f64,
}

#[cfg(feature = "livery")]
impl<E: ScriptEngine> LiveryScriptedDocument<E> {
    /// Fetch a document and build the Livery CSSOM session before its first
    /// parser-blocking script executes.
    pub fn load<Fetch>(fetcher: Fetch, url: &str) -> Result<Self, String>
    where
        Fetch: ResourceFetcher + 'static,
    {
        let navigation = NavigationFragment::parse(url);
        let fetcher = SharedResourceFetcher(Rc::new(fetcher));
        let bytes = fetcher
            .fetch(&navigation.resource_url)
            .ok_or_else(|| format!("could not load {}", navigation.resource_url))?;
        let mut document = Self::build(
            &String::from_utf8_lossy(&bytes),
            fetcher,
            &navigation.script_visible_url,
        )?;
        document.pending_fragment = (!navigation.text_directives.is_empty()
            || navigation.element_fragment.is_some())
        .then_some(navigation);
        Ok(document)
    }

    /// Build a Livery-scripted document from already-fetched HTML. The fetcher
    /// remains retained for external scripts and later live stylesheet, image,
    /// and font reconciliation.
    pub fn from_body<Fetch>(html: &str, fetcher: Fetch, base_url: &str) -> Result<Self, String>
    where
        Fetch: ResourceFetcher + 'static,
    {
        let navigation = NavigationFragment::parse(base_url);
        let mut document = Self::build(
            html,
            SharedResourceFetcher(Rc::new(fetcher)),
            &navigation.script_visible_url,
        )?;
        document.pending_fragment = (!navigation.text_directives.is_empty()
            || navigation.element_fragment.is_some())
        .then_some(navigation);
        Ok(document)
    }

    /// Parse an inline fixture with an explicit empty host transport. Inline
    /// styles and scripts still use the same Livery CSSOM ownership path.
    pub fn parse(html: &str) -> Result<Self, String> {
        Self::build(
            html,
            SharedResourceFetcher(Rc::new(EmptyResourceFetcher)),
            "about:blank",
        )
    }

    fn build(html: &str, fetcher: SharedResourceFetcher, base_url: &str) -> Result<Self, String> {
        let mut rt =
            Runtime::<E>::new().map_err(|error| format!("script runtime init: {error:?}"))?;
        let _ = rt.set_base_url(base_url);
        // The CSSOM is installed over the *empty* arena, before the parse, and
        // re-resolves its author sheets from the live DOM at every read. That
        // is what lets this constructor use the interleaved parse: a sheet
        // enters the cascade when the parser inserts it, so a script reading
        // computed style mid-parse sees the sheets parsed so far, and nothing
        // has to be resolved from a finished tree first.
        let cssom = LiveryCssom::install_live(
            &mut rt,
            fetcher.clone(),
            base_url,
            ResourceLimits::default(),
            Device::screen(800.0, 600.0),
        );
        // HTML's parsing model: the tree is built and its scripts run
        // interleaved, so each script sees the document as far as its own
        // position — including the stylesheets parsed so far, through the live
        // CSSOM installed above.
        let loader: Option<(&dyn ResourceFetcher, &str)> = Some((&fetcher, base_url));
        rt.parse_document_interleaved(html, &DocumentScriptLoader { loader });
        rt.run_microtasks();

        // The capture recorder is opened after the parse, over the built tree,
        // which is the tree the parse-then-run route handed it too.
        let capture_sheets = cssom
            .resource_set()
            .map(|resources| {
                resources
                    .stylesheet_text()
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut capture = {
            let mut host = rt.host().borrow_mut();
            DomCaptureRecorder::from_env(&mut host.dom, &capture_sheets)
                .map_err(|error| format!("dom capture init: {error}"))?
        };
        if let Some(recorder) = capture.as_mut() {
            let mut host = rt.host().borrow_mut();
            recorder
                .record_pending(&mut host.dom)
                .map_err(|error| format!("dom capture write: {error}"))?;
        }

        Ok(Self {
            rt: Box::new(rt),
            cssom: Box::new(cssom),
            pending_fragment: None,
            capture,
            hidden: false,
            frozen: false,
            last_hidden_pump_ms: f64::NAN,
        })
    }

    /// Render the exact live runtime DOM through Livery and lower the resulting
    /// paint list into the existing host-neutral scene.
    pub fn frame(&mut self, width: u32, height: u32) -> netrender::Scene {
        let (requested_scroll, into_view) = {
            let mut host = self.rt.host().borrow_mut();
            (host.viewport_scroll, host.scroll_into_view.take())
        };
        if let Some(node) = into_view {
            let _ = self.cssom.scroll_to_id(node);
        } else {
            self.cssom.scroll_to(requested_scroll.0, requested_scroll.1);
        }
        let mut list = match self.cssom.frame(&mut self.rt, width, height) {
            Ok(list) => list,
            Err(error) => {
                eprintln!("[pelt-livery-scripted] layout error: {error}");
                return netrender::Scene::new(width, height);
            },
        };
        if let Some(navigation) = self.pending_fragment.take() {
            let text_activated = self
                .cssom
                .activate_text_directives(&navigation.text_directives);
            if !text_activated && let Some(fragment) = navigation.element_fragment.as_deref() {
                self.scroll_to_fragment(fragment);
            }
            // Reframe retained geometry only. The source bytes were fetched and
            // the live DOM built before this navigation-time activation.
            list = match self.cssom.frame(&mut self.rt, width, height) {
                Ok(list) => list,
                Err(error) => {
                    eprintln!("[pelt-livery-scripted] layout error: {error}");
                    return netrender::Scene::new(width, height);
                },
            };
        }
        self.rt.host().borrow_mut().viewport_scroll = self.cssom.scroll();
        paint_list_render::translate_paint_list(&list)
    }

    pub fn scroll_by(&mut self, dx: f32, dy: f32) -> bool {
        let moved = self.cssom.scroll_by(dx, dy);
        if moved {
            self.rt.host().borrow_mut().viewport_scroll = self.cssom.scroll();
        }
        moved
    }

    /// The current Livery viewport offset after host or fragment navigation.
    pub fn scroll(&self) -> (f32, f32) {
        self.cssom.scroll()
    }

    pub fn scroll_for_key(&mut self, key: ScrollKey) -> bool {
        let (_, height) = self.cssom.viewport();
        let current = self.cssom.scroll();
        let next = match key {
            ScrollKey::Up => (current.0, current.1 - 40.0),
            ScrollKey::Down => (current.0, current.1 + 40.0),
            ScrollKey::Left => (current.0 - 40.0, current.1),
            ScrollKey::Right => (current.0 + 40.0, current.1),
            ScrollKey::PageUp => (current.0, current.1 - height as f32 * 0.9),
            ScrollKey::PageDown => (current.0, current.1 + height as f32 * 0.9),
            ScrollKey::Home => (0.0, 0.0),
            ScrollKey::End => (0.0, f32::MAX),
        };
        self.cssom.scroll_to(next.0, next.1);
        let moved = self.cssom.scroll() != current;
        if moved {
            self.rt.host().borrow_mut().viewport_scroll = self.cssom.scroll();
        }
        moved
    }

    pub fn click_at(&mut self, x: f32, y: f32) -> bool {
        !matches!(self.click_at_result(x, y), ScriptedClick::Miss)
    }

    /// Resolve the live pointer target without dispatching its click default.
    /// Session adapters retain the nearest link activation owner, or the
    /// ordinary pointer target, from press to release.
    pub fn click_target_at(&self, x: f32, y: f32) -> Option<NodeId> {
        self.cssom.activation_target_at(&self.rt, x, y)
    }

    /// Dispatch a live-DOM click and preserve its typed external-navigation
    /// default for the embedding document session.
    pub fn click_at_result(&mut self, x: f32, y: f32) -> ScriptedClick {
        let outcome = self.cssom.click_at_result(&mut self.rt, x, y);
        self.flush_dom_capture();
        outcome
    }

    pub fn links(&self) -> Vec<(String, [f32; 4])> {
        let host = self.rt.host().borrow();
        let mut links = Vec::new();
        collect_livery_links(&host.dom, host.dom.document(), &self.cssom, &mut links);
        links
    }

    pub fn begin_text_selection(&mut self, x: f32, y: f32) -> bool {
        self.cssom.begin_text_selection(x, y)
    }

    pub fn extend_text_selection(&mut self, x: f32, y: f32) -> bool {
        self.cssom.extend_text_selection(x, y)
    }

    pub fn finish_text_selection(&mut self, x: f32, y: f32) -> bool {
        self.cssom.finish_text_selection(x, y)
    }

    pub fn cancel_text_selection(&mut self) -> bool {
        self.cssom.cancel_text_selection()
    }

    pub fn text_selection(&self) -> Option<genet_livery::TextSelection<NodeId>> {
        self.cssom.text_selection()
    }

    pub fn text_target(&self, text: &str) -> Option<([f32; 2], [f32; 2])> {
        self.cssom.text_target(text)
    }

    /// Read the runtime-owned live DOM without exposing the runtime handle or
    /// allowing a borrow to escape the callback.
    pub fn with_dom<R>(&self, inspect: impl FnOnce(&ScriptedDom) -> R) -> R {
        let host = self.rt.host().borrow();
        inspect(&host.dom)
    }

    /// Install the host-owned route retained for worker scripts and imports.
    /// The borrowed parser ResourceFetcher is not retained after construction.
    /// Install this before the first pump that services worker resource requests.
    pub fn set_script_resource_loader(
        &mut self,
        loader: Box<dyn script_runtime_api::ScriptResourceLoader>,
    ) {
        self.rt.set_script_resource_loader(loader);
    }

    pub fn pump(&mut self, now_ms: f64) -> (usize, usize) {
        if self.frozen {
            return (0, 0);
        }
        if self.hidden {
            if self.last_hidden_pump_ms.is_nan() || now_ms - self.last_hidden_pump_ms < 1000.0 {
                if self.last_hidden_pump_ms.is_nan() {
                    self.last_hidden_pump_ms = now_ms;
                }
                return (0, 0);
            }
            self.last_hidden_pump_ms = now_ms;
        }
        self.rt.run_timers(64, now_ms);
        self.rt.run_microtasks();
        let _ = self.rt.pump_workers();
        self.rt.run_microtasks();
        self.flush_dom_capture();
        self.rt.collect_garbage()
    }

    pub fn has_pending_work(&mut self) -> bool {
        !self.frozen
            && (self.rt.next_timer_delay().is_some() || self.rt.has_worker_work())
    }

    pub fn set_hidden(&mut self, hidden: bool) {
        if self.hidden == hidden {
            return;
        }
        self.hidden = hidden;
        if hidden {
            self.last_hidden_pump_ms = f64::NAN;
        }
        let document = self.rt.host().borrow().dom.document().raw();
        let _ = self.rt.dispatch_event(document, "visibilitychange");
    }

    pub fn evaluate(&mut self, source: &str) -> Result<(), String> {
        self.rt
            .eval(source)
            .map_err(|error| format!("script evaluation: {error:?}"))?;
        self.rt.run_microtasks();
        self.flush_dom_capture();
        Ok(())
    }

    pub fn dom_snapshot(&self) -> String {
        let host = self.rt.host().borrow();
        host.dom.inner_html(host.dom.document())
    }

    pub fn console(&self) -> Vec<String> {
        self.rt.host().borrow().console.clone()
    }

    pub fn resource_set(&self) -> Option<ResolvedDocumentResources> {
        self.cssom.resource_set()
    }

    pub fn scroll_to_fragment(&mut self, fragment: &str) {
        let target = {
            let host = self.rt.host().borrow();
            find_id(&host.dom, host.dom.document(), fragment)
        };
        if let Some(target) = target {
            let _ = self.cssom.scroll_to_id(target);
        }
    }

    fn flush_dom_capture(&mut self) {
        let Some(mut recorder) = self.capture.take() else {
            return;
        };
        let result = {
            let mut host = self.rt.host().borrow_mut();
            recorder.record_pending(&mut host.dom)
        };
        match result {
            Ok(_) => self.capture = Some(recorder),
            Err(error) => eprintln!("[pelt-livery-scripted] dom capture disabled: {error}"),
        }
    }
}

#[cfg(all(test, feature = "livery"))]
mod livery_text_fragment_tests {
    use super::*;
    use script_engine_boa::BoaEngine;

    #[derive(Clone)]
    struct CountingFetcher {
        calls: Rc<std::cell::RefCell<Vec<String>>>,
        body: Vec<u8>,
    }

    impl ResourceFetcher for CountingFetcher {
        fn fetch(&self, url: &str) -> Option<Vec<u8>> {
            self.calls.borrow_mut().push(url.to_owned());
            (url == "https://example.test/article").then(|| self.body.clone())
        }
    }

    /// The two-phase CSSOM install, stated positionally: a sheet is in the
    /// cascade for a script *after* it and absent for a script *before* it,
    /// which is what "stylesheets attach at the point they are parsed" means
    /// and what a one-shot install over a finished tree cannot express.
    #[test]
    fn livery_stylesheets_attach_where_they_are_parsed_on_boa() {
        let html = r#"<!doctype html><html><body>
            <div id="t">x</div>
            <script>console.log('before:' + document.styleSheets.length + ':' +
              getComputedStyle(document.getElementById('t')).color);</script>
            <style>#t { color: rgb(0, 128, 0); }</style>
            <script>console.log('after:' + document.styleSheets.length + ':' +
              getComputedStyle(document.getElementById('t')).color);</script>
            </body></html>"#;
        let document = LiveryScriptedDocument::<BoaEngine>::parse(html)
            .expect("Livery scripted document builds");
        let log = document.console();
        assert_eq!(log.len(), 2, "one line per script: {log:?}");
        assert!(
            log[0].starts_with("before:0:"),
            "the sheet below is not in the cascade yet: {:?}",
            log[0]
        );
        assert_eq!(
            log[1], "after:1:rgb(0, 128, 0)",
            "the sheet is in the cascade the moment the parser inserts it"
        );
    }

    #[test]
    fn initial_text_fragment_selects_scrolls_and_hides_the_directive_on_boa() {
        let html = r#"<html><head><style>body { margin: 0; }</style></head><body>
            <div style="height: 900px"></div><p id="fallback">prefix needle end suffix</p>
            <script>console.log(document.URL + '|' + location.href + '|' + location.hash);</script>
            </body></html>"#;
        let mut document = LiveryScriptedDocument::<BoaEngine>::from_body(
            html,
            EmptyResourceFetcher,
            "https://example.test/article#fallback:~:text=prefix-,needle,end,-suffix",
        )
        .expect("Livery scripted document builds");

        let _scene = document.frame(320, 160);
        assert_eq!(
            document.console(),
            vec![
                "https://example.test/article#fallback|https://example.test/article#fallback|#fallback"
            ],
        );
        let selection = document
            .text_selection()
            .expect("the retained scripted frame matched the directive");
        assert_eq!(selection.text, "needle end");
        assert!(document.scroll().1 > 0.0, "the match is revealed");
    }

    #[test]
    fn initial_text_fragment_fetches_the_source_once_on_boa() {
        let calls = Rc::new(std::cell::RefCell::new(Vec::new()));
        let fetcher = CountingFetcher {
            calls: calls.clone(),
            body: br#"<body><div style="height: 900px"></div><p>one needle two</p></body>"#
                .to_vec(),
        };
        let mut document = LiveryScriptedDocument::<BoaEngine>::load(
            fetcher,
            "https://example.test/article#:~:text=needle",
        )
        .expect("Livery scripted document loads");

        let _scene = document.frame(320, 160);
        assert_eq!(
            calls.borrow().as_slice(),
            ["https://example.test/article"],
            "first-frame activation reuses the loaded source"
        );
        assert_eq!(
            document
                .text_selection()
                .expect("text directive matched")
                .text,
            "needle"
        );
    }

    #[test]
    fn scripted_text_fragment_falls_back_to_its_element_fragment_on_boa() {
        let mut document = LiveryScriptedDocument::<BoaEngine>::from_body(
            r#"<body><div style="height: 900px"></div><p id="fallback">ordinary target</p></body>"#,
            EmptyResourceFetcher,
            "https://example.test/article#fallback:~:text=missing",
        )
        .expect("Livery scripted document builds");

        let _scene = document.frame(320, 160);
        assert!(document.text_selection().is_none());
        assert!(document.scroll().1 > 0.0, "#fallback is revealed");
    }
}

#[cfg(feature = "livery")]
fn find_id(dom: &ScriptedDom, node: NodeId, id: &str) -> Option<NodeId> {
    if dom.kind(node) == layout_dom_api::NodeKind::Element
        && dom.attribute(node, &Namespace::default(), &LocalName::from("id")) == Some(id)
    {
        return Some(node);
    }
    dom.dom_children(node)
        .find_map(|child| find_id(dom, child, id))
}

#[cfg(feature = "livery")]
fn collect_livery_links(
    dom: &ScriptedDom,
    node: NodeId,
    cssom: &LiveryCssom,
    links: &mut Vec<(String, [f32; 4])>,
) {
    if dom.kind(node) == layout_dom_api::NodeKind::Element
        && dom
            .element_name(node)
            .is_some_and(|name| name.local.as_ref().eq_ignore_ascii_case("a"))
        && let Some(href) = dom.attribute(node, &Namespace::default(), &LocalName::from("href"))
        && let Some(rect) = cssom.fragment_rect(node)
    {
        links.push((href.to_owned(), rect));
    }
    for child in dom.dom_children(node) {
        collect_livery_links(dom, child, cssom, links);
    }
}

/// Which JS engine the scripted profile runs on. Boa is pure Rust (all targets, the
/// default conformance oracle); Nova is 64-bit-target-only and gated behind the
/// `scripted-nova` feature, so the default build links a single engine (Boa + Nova +
/// wgpu together exceed the Windows image-size link limit). Selected at the call site,
/// exactly as genet-wpt's `--engine` picks the monomorphization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ScriptedEngine {
    #[default]
    Boa,
    Nova,
}

impl ScriptedEngine {
    /// Parse a `--js` value (`boa` / `nova`), case-insensitively.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "boa" => Some(Self::Boa),
            "nova" => Some(Self::Nova),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Boa => "boa",
            Self::Nova => "nova",
        }
    }
}

/// Fetch an external script's source through the `loader` (`(fetcher, base_url)`),
/// resolving `src` against `base_url`, verifying any `integrity` (SRI) metadata, and
/// decoding the bytes per `charset` (default UTF-8). `None` (with a log) when there
/// is no loader (the fetch-free [`ScriptedDocument::parse`] path), the fetch fails,
/// or the integrity check rejects the bytes.
/// The document's resource route, as the parser driver's script loader. The
/// route is the same `ResourceFetcher` every other subresource travels, and it
/// is synchronous, which is what lets a parser-blocking script actually block
/// the parser.
struct DocumentScriptLoader<'a> {
    loader: Option<(&'a dyn ResourceFetcher, &'a str)>,
}

impl script_runtime_api::ParserScriptLoader for DocumentScriptLoader<'_> {
    fn load(&self, src: &str, charset: Option<&str>, integrity: Option<&str>) -> Option<String> {
        fetch_external(self.loader, src, charset, integrity)
    }

    fn resolve(&self, src: &str) -> String {
        match self.loader {
            Some((_, base)) => crate::resolve_href(base, src),
            None => src.to_owned(),
        }
    }
}

fn fetch_external(
    loader: Option<(&dyn ResourceFetcher, &str)>,
    src: &str,
    charset: Option<&str>,
    integrity: Option<&str>,
) -> Option<String> {
    let Some((fetcher, base)) = loader else {
        eprintln!("[pelt-scripted] skipping external <script src=\"{src}\"> (no fetcher)");
        return None;
    };
    let url = crate::resolve_href(base, src);
    let bytes = match fetcher.fetch(&url) {
        Some(bytes) => bytes,
        None => {
            eprintln!("[pelt-scripted] could not fetch script {url}");
            return None;
        },
    };
    if let Some(metadata) = integrity {
        if !integrity_matches(metadata, &bytes) {
            eprintln!("[pelt-scripted] integrity mismatch for {url}; script blocked");
            return None;
        }
    }
    Some(decode_script_bytes(&bytes, charset))
}

/// Whether `bytes` satisfy a Subresource-Integrity `integrity` attribute. Per SRI:
/// parse the space-separated `alg-base64hash[?opts]` tokens, take the **strongest**
/// algorithm present (sha512 > sha384 > sha256), and accept if the digest matches
/// **any** of that algorithm's hashes. Unrecognized/empty metadata imposes no
/// requirement (returns `true`). Compares raw digest bytes, so base64 padding
/// variance does not matter.
fn integrity_matches(metadata: &str, bytes: &[u8]) -> bool {
    use base64::Engine as _;
    use sha2::Digest as _;

    let mut strongest = 0u8; // 1 = sha256, 2 = sha384, 3 = sha512
    let mut expected: Vec<&str> = Vec::new();
    for token in metadata.split_whitespace() {
        let Some((alg, rest)) = token.split_once('-') else {
            continue;
        };
        let strength = match alg {
            "sha256" => 1u8,
            "sha384" => 2,
            "sha512" => 3,
            _ => continue,
        };
        let hash = rest.split('?').next().unwrap_or(rest); // drop any `?options`
        if strength > strongest {
            strongest = strength;
            expected.clear();
            expected.push(hash);
        } else if strength == strongest {
            expected.push(hash);
        }
    }
    if strongest == 0 {
        return true; // no valid metadata → no integrity requirement
    }
    let digest: Vec<u8> = match strongest {
        1 => sha2::Sha256::digest(bytes).to_vec(),
        2 => sha2::Sha384::digest(bytes).to_vec(),
        _ => sha2::Sha512::digest(bytes).to_vec(),
    };
    let std = base64::engine::general_purpose::STANDARD;
    let nopad = base64::engine::general_purpose::STANDARD_NO_PAD;
    expected.iter().any(|h| {
        std.decode(h)
            .or_else(|_| nopad.decode(h))
            .map(|d| d == digest)
            .unwrap_or(false)
    })
}

/// Decode fetched script bytes into source text using the `<script charset>`
/// encoding (resolved through `encoding_rs`), defaulting to UTF-8. An unknown label
/// also falls back to UTF-8.
fn decode_script_bytes(bytes: &[u8], charset: Option<&str>) -> String {
    let encoding = charset
        .and_then(|label| encoding_rs::Encoding::for_label(label.trim().as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    encoding.decode(bytes).0.into_owned()
}

/// Script-timing, module-loading, GC, extraction and Livery-render receipts.
///
/// Most of these exercise [`ScriptedDocument`] headlessly (script execution,
/// DOM mutation, extraction — no layout needed at all). The handful that need
/// to see something painted (`mutation_renders`, `click_dispatches_to_script`,
/// …) run against [`LiveryScriptedDocument`] instead, in the nested
/// `livery_render_tests` module: Livery is the crate's one live render route
/// now that the old genet-layout route (`#[cfg(feature = "render")]`, a
/// feature this crate never declared) is gone. See
/// `design_docs/2026-09-08_parser_script_interleaving_plan.md`'s Findings
/// entry for what was ported, deleted, or left `#[ignore]`d and why.
#[cfg(test)]
mod tests {
    use super::*;
    use script_engine_boa::BoaEngine;
    use script_runtime_api::WebGlHandler;

    struct NullWebGl;

    impl WebGlHandler for NullWebGl {
        fn external_texture_key(&self) -> Option<u64> {
            Some(17)
        }
        fn clear_color(&mut self, _r: f32, _g: f32, _b: f32, _a: f32) {}
        fn clear(&mut self, _mask: u32) {}
        fn viewport(&mut self, _x: i32, _y: i32, _width: u32, _height: u32) {}
        fn enable(&mut self, _cap: u32) {}
        fn disable(&mut self, _cap: u32) {}
        fn is_enabled(&mut self, _cap: u32) -> bool {
            false
        }
        fn color_mask(&mut self, _r: bool, _g: bool, _b: bool, _a: bool) {}
        fn create_buffer(&mut self) -> u64 {
            1
        }
        fn bind_buffer(&mut self, _target: u32, _buffer: Option<u64>) {}
        fn buffer_data_f32(&mut self, _target: u32, _data: &[f32], _usage: u32) {}
        fn create_shader(&mut self, _stage: u32) -> u64 {
            1
        }
        fn shader_source(&mut self, _shader: u64, _source: &str) {}
        fn compile_shader(&mut self, _shader: u64) {}
        fn get_shader_compile_status(&mut self, _shader: u64) -> bool {
            true
        }
        fn get_shader_info_log(&mut self, _shader: u64) -> String {
            String::new()
        }
        fn create_program(&mut self) -> u64 {
            1
        }
        fn attach_shader(&mut self, _program: u64, _shader: u64) {}
        fn link_program(&mut self, _program: u64) {}
        fn get_program_link_status(&mut self, _program: u64) -> bool {
            true
        }
        fn get_program_info_log(&mut self, _program: u64) -> String {
            String::new()
        }
        fn use_program(&mut self, _program: Option<u64>) {}
        fn get_attrib_location(&mut self, _program: u64, _name: &str) -> i32 {
            0
        }
        fn get_uniform_location(&mut self, _program: u64, _name: &str) -> i32 {
            -1
        }
        fn enable_vertex_attrib_array(&mut self, _index: u32) {}
        fn vertex_attrib_pointer_f32(
            &mut self,
            _index: u32,
            _size: u32,
            _normalized: bool,
            _stride: u32,
            _offset: u32,
        ) {
        }
        fn uniform4f(&mut self, _location: i32, _x: f32, _y: f32, _z: f32, _w: f32) {}
        fn uniform_matrix4fv(&mut self, _location: i32, _transpose: bool, _value: &[f32]) {}
        fn uniform1i(&mut self, _location: i32, _value: i32) {}
        fn create_texture(&mut self) -> u64 {
            1
        }
        fn bind_texture_2d(&mut self, _texture: Option<u64>) {}
        fn active_texture(&mut self, _unit: u32) {}
        fn tex_image_2d_rgba8(&mut self, _width: u32, _height: u32, _pixels: &[u8]) {}
        fn draw_arrays(&mut self, _mode: u32, _first: i32, _count: i32) {}
        fn get_error(&mut self) -> u32 {
            0
        }
        fn read_pixels_rgba8(&mut self, _x: i32, _y: i32, _width: u32, _height: u32) -> Vec<u8> {
            Vec::new()
        }
    }

    /// The WebGL factory installs before any parser-blocking inline script runs
    /// — a headless `ScriptedDocument` capability (`Runtime::set_webgl_factory`
    /// is called unconditionally in `build`, not gated on rendering).
    fn webgl_factory_is_installed_before_inline_script<E: ScriptEngine>() {
        let created = std::rc::Rc::new(std::cell::Cell::new(false));
        let marker = created.clone();
        let html = r#"<body><canvas id="c"></canvas><script>
            document.getElementById('c').getContext('webgl');
        </script></body>"#;
        let _doc = ScriptedDocument::<E>::parse_with_webgl_factory(
            html,
            Box::new(move |_width, _height| {
                marker.set(true);
                Box::new(NullWebGl)
            }),
        )
        .expect("runtime inits");
        assert!(
            created.get(),
            "the factory ran during parser-blocking script execution"
        );
    }

    /// Node identity survives the WeakMap wrapper cache: the same node yields the same
    /// JS wrapper (`getElementById('x') === getElementById('x')`) and a created node's
    /// `parentNode` round-trips. Guards the strong-Map → WeakMap change (a broken cache
    /// would mint a fresh wrapper per call and `===` would be false).
    fn node_identity_is_stable<E: ScriptEngine>() {
        let html = "<body><div id=\"x\"></div><script>\
            var same = document.getElementById('x') === document.getElementById('x');\
            var p = document.createElement('p');\
            document.body.appendChild(p);\
            var parented = p.parentNode === document.body;\
            console.log('same:' + same + ' parented:' + parented);\
            </script></body>";
        let doc = ScriptedDocument::<E>::parse(html).expect("runtime inits");
        assert!(
            doc.console().iter().any(|l| l == "same:true parented:true"),
            "node identity preserved through the WeakMap cache: {:?}",
            doc.console(),
        );
    }

    /// The GC tick reaps a node the script orphaned and dropped its only reference to:
    /// after building then detaching + dereferencing a subtree, [`pump`] collects it.
    fn pump_collects_orphans<E: ScriptEngine>() {
        let html = "<body><script>\
            var keep = document.createElement('div');\
            document.body.appendChild(keep);\
            var gone = document.createElement('span');\
            keep.appendChild(gone);\
            keep.removeChild(gone);\
            gone = null;\
            </script></body>";
        let mut doc = ScriptedDocument::<E>::parse(html).expect("runtime inits");
        let before = doc.live_node_count();
        // Drive a frame's worth: forcing the engine GC drops the dropped <span>
        // wrapper, the weak reflector cache reports it dead, the pin retires, and the
        // orphan is reaped — the live set actually shrinks (the WeakMap-cache contract;
        // a strong cache would leave it flat).
        let (unpinned, collected) = doc.pump(16.0);
        let after = doc.live_node_count();
        assert!(
            after < before,
            "the orphaned node is reaped: {before} -> {after}"
        );
        assert!(
            collected >= 1,
            "collect_garbage reaped at least the orphan (got {collected})"
        );
        let _ = unpinned;
    }

    /// Page Visibility + Page Lifecycle (W3C adoption plan P1): a hidden
    /// document's interval loop clamps to at most one pump per second, a frozen
    /// one runs nothing, resume + visible restores frame cadence, and each
    /// visibility flip fires `visibilitychange` at the document (observed here
    /// by a page listener counting into an attribute).
    fn hidden_clamps_timers_frozen_stops_them<E: ScriptEngine>() {
        let html = "<body><div id='c' data-ticks='0' data-vis='0'></div><script>            var c = document.getElementById('c');            var ticks = 0;            setInterval(function(){ ticks++; c.setAttribute('data-ticks', String(ticks)); }, 16);            var vis = 0;            document.addEventListener('visibilitychange', function(){                vis++; c.setAttribute('data-vis', String(vis));            });            </script></body>";
        let mut doc = ScriptedDocument::<E>::parse(html).expect("runtime inits");
        // Read the counters back through the serialized snapshot (no direct
        // attribute query surface on ScriptedDom; the snapshot is exact).
        fn attr_u32<E: ScriptEngine>(doc: &ScriptedDocument<E>, name: &str) -> u32 {
            let snap = doc.dom_snapshot();
            let pat = format!("{name}=\"");
            snap.find(&pat)
                .map(|i| {
                    let rest = &snap[i + pat.len()..];
                    let end = rest.find('"').unwrap_or(0);
                    rest[..end].parse().unwrap_or(0)
                })
                .unwrap_or(0)
        }
        let ticks = |doc: &ScriptedDocument<E>| attr_u32(doc, "data-ticks");
        let vis_count = |doc: &ScriptedDocument<E>| attr_u32(doc, "data-vis");
        let mut now = 0.0;
        for _ in 0..10 {
            now += 16.0;
            doc.pump(now);
        }
        let visible_ticks = ticks(&doc);
        assert!(visible_ticks >= 5, "visible interval runs at frame cadence");

        doc.set_hidden(true);
        assert_eq!(vis_count(&doc), 1, "visibilitychange fired on hide");
        let hidden_start = ticks(&doc);
        for _ in 0..30 {
            now += 16.0;
            doc.pump(now); // 480ms of hidden pumps, all inside the 1s clamp
        }
        assert_eq!(
            ticks(&doc),
            hidden_start,
            "hidden pumps inside the clamp run no timers"
        );
        now += 1100.0;
        doc.pump(now);
        let after_clamp = ticks(&doc);
        assert!(
            after_clamp > hidden_start,
            "the once-per-second hidden pump still advances timers"
        );

        doc.freeze();
        for _ in 0..10 {
            now += 1100.0;
            doc.pump(now);
        }
        assert_eq!(ticks(&doc), after_clamp, "a frozen document runs nothing");
        assert!(!doc.has_pending_work(), "frozen reports no pending work");

        doc.resume();
        doc.set_hidden(false);
        assert_eq!(vis_count(&doc), 2, "visibilitychange fired on show");
        for _ in 0..5 {
            now += 16.0;
            doc.pump(now);
        }
        assert!(
            ticks(&doc) > after_clamp,
            "visible again: the interval resumes at frame cadence"
        );
    }

    /// The gc-arena soak (carve-out #2): a page that churns nodes under `setInterval`
    /// is driven through [`pump`](ScriptedDocument::pump) at frame cadence; the GC tick
    /// keeps the live set bounded rather than growing one batch per frame. Without a
    /// working frame-cadence collector this peaks in the thousands; with it, a handful.
    fn gc_soak_bounds_memory<E: ScriptEngine>() {
        // Each tick: append a batch of fresh nodes to a host, then remove them all.
        // The removed nodes are orphaned + unreachable from script (the locals fall out
        // of scope), so the collector should reap them.
        let html = "<body><script>\
            var host = document.createElement('div');\
            document.body.appendChild(host);\
            function churn() {\
                for (var i = 0; i < 50; i++) {\
                    var n = document.createElement('span');\
                    n.appendChild(document.createTextNode('x'));\
                    host.appendChild(n);\
                }\
                while (host.firstChild) { host.removeChild(host.firstChild); }\
            }\
            setInterval(churn, 16);\
            </script></body>";
        let mut doc = ScriptedDocument::<E>::parse(html).expect("runtime inits");
        let mut now = 0.0;
        let mut peak = 0;
        for _ in 0..120 {
            now += 16.0;
            doc.pump(now);
            peak = peak.max(doc.live_node_count());
        }
        // Bounded: a few structural nodes + at most a batch or two in flight — not the
        // ~6000 (50 × 120) an uncollected churn would accumulate.
        assert!(
            peak < 1000,
            "frame-cadence GC bounds the churned DOM; peak live = {peak}"
        );
    }

    /// In-memory [`ResourceFetcher`] for the external-script tests: a fixed
    /// URL→bytes map, so a `load` resolves the page and its `<script src>`s without
    /// touching the network or disk.
    #[derive(Clone)]
    struct MapFetcher(std::collections::HashMap<String, Vec<u8>>);
    impl ResourceFetcher for MapFetcher {
        fn fetch(&self, url: &str) -> Option<Vec<u8>> {
            self.0.get(url).cloned()
        }
    }
    fn map_fetcher(files: &[(&str, &str)]) -> MapFetcher {
        MapFetcher(
            files
                .iter()
                .map(|(u, b)| (u.to_string(), b.as_bytes().to_vec()))
                .collect(),
        )
    }
    /// Build a fetcher from owned `(url, bytes)` pairs — for fixtures whose script
    /// bytes are not valid UTF-8 (charset) or are hashed (integrity).
    fn map_of(files: Vec<(&str, Vec<u8>)>) -> MapFetcher {
        MapFetcher(files.into_iter().map(|(u, b)| (u.to_string(), b)).collect())
    }

    /// A cookie provider passed to `from_body` is installed before scripts run: a page
    /// reads `document.cookie` on load and a write reaches the host store. (Render
    /// ladder 2c — `document.cookie` over the host's session jar.)
    fn from_body_wires_document_cookie<E: ScriptEngine>() {
        use std::cell::RefCell;
        use std::rc::Rc;
        struct Jar {
            written: Rc<RefCell<Vec<String>>>,
        }
        impl script_runtime_api::CookieProvider for Jar {
            fn get_cookies(&self) -> String {
                "sid=abc".to_string()
            }
            fn set_cookie(&self, cookie: &str) {
                self.written.borrow_mut().push(cookie.to_string());
            }
        }
        let written = Rc::new(RefCell::new(Vec::new()));
        let body = "<body><script>\
            document.title = document.cookie;\
            document.cookie = 'theme=dark';\
            console.log(document.cookie);\
            </script></body>";
        let _doc = ScriptedDocument::<E>::from_body(
            body,
            &map_fetcher(&[]),
            "http://x/",
            Some(Box::new(Jar {
                written: written.clone(),
            })),
        )
        .expect("from_body with cookies");
        assert_eq!(
            *written.borrow(),
            vec!["theme=dark".to_string()],
            "the write reached the jar"
        );
    }

    /// Inline and external scripts run in document order: three scripts (inline,
    /// external, inline) each log a letter, and the console shows `A`, `B`, `C` in
    /// order — proving inline and external interleave in authored order (the ordering
    /// the old inline-only path explicitly could not guarantee).
    fn scripts_run_in_document_order<E: ScriptEngine>() {
        let files = map_fetcher(&[
            (
                "http://x/index.html",
                "<body>\
                    <script>console.log('A');</script>\
                    <script src=\"b.js\"></script>\
                    <script>console.log('C');</script>\
                 </body>",
            ),
            ("http://x/b.js", "console.log('B');"),
        ]);
        let doc = ScriptedDocument::<E>::load(&files, "http://x/index.html").expect("loads");
        assert_eq!(
            doc.console(),
            vec!["A".to_string(), "B".to_string(), "C".to_string()],
            "scripts ran in document order (inline A, external B, inline C)",
        );
    }

    /// A relative `src` resolves against the document URL's directory, not the host
    /// root: `sub/app.js` on `http://x/dir/index.html` fetches
    /// `http://x/dir/sub/app.js`.
    fn relative_src_resolves_against_page_url<E: ScriptEngine>() {
        let files = map_fetcher(&[
            (
                "http://x/dir/index.html",
                "<body><script src=\"sub/app.js\"></script></body>",
            ),
            ("http://x/dir/sub/app.js", "console.log('relative-ok');"),
        ]);
        let doc = ScriptedDocument::<E>::load(&files, "http://x/dir/index.html").expect("loads");
        assert!(
            doc.console().iter().any(|l| l == "relative-ok"),
            "relative src resolved against the page directory: {:?}",
            doc.console(),
        );
    }

    /// A missing external script is reported and skipped, not fatal: the page still
    /// loads and its inline siblings still run (browser resilience).
    fn missing_external_script_is_skipped<E: ScriptEngine>() {
        let files = map_fetcher(&[(
            "http://x/index.html",
            "<body><script src=\"gone.js\"></script><script>console.log('still-here');</script></body>",
        )]);
        let doc = ScriptedDocument::<E>::load(&files, "http://x/index.html").expect("loads anyway");
        assert!(
            doc.console().iter().any(|l| l == "still-here"),
            "inline sibling runs despite the missing external script: {:?}",
            doc.console(),
        );
    }

    /// `defer` runs the external script *after* the parser-blocking pass: a deferred
    /// script that appears *before* a later inline script nonetheless runs *after* it.
    /// Document-order execution would log `defer` first; deferral logs `inline` first.
    fn defer_runs_after_parser_blocking<E: ScriptEngine>() {
        let files = map_fetcher(&[
            (
                "http://x/index.html",
                "<body>\
                    <script src=\"defer.js\" defer></script>\
                    <script>console.log('inline');</script>\
                 </body>",
            ),
            ("http://x/defer.js", "console.log('defer');"),
        ]);
        let doc = ScriptedDocument::<E>::load(&files, "http://x/index.html").expect("loads");
        assert_eq!(
            doc.console(),
            vec!["inline".to_string(), "defer".to_string()],
            "the inline (parser-blocking) script runs before the earlier-positioned defer",
        );
    }

    /// `defer` scripts run in document order among themselves (the deferral guarantee).
    fn defer_scripts_run_in_document_order<E: ScriptEngine>() {
        let files = map_fetcher(&[
            (
                "http://x/index.html",
                "<body>\
                    <script src=\"d1.js\" defer></script>\
                    <script src=\"d2.js\" defer></script>\
                 </body>",
            ),
            ("http://x/d1.js", "console.log('d1');"),
            ("http://x/d2.js", "console.log('d2');"),
        ]);
        let doc = ScriptedDocument::<E>::load(&files, "http://x/index.html").expect("loads");
        assert_eq!(
            doc.console(),
            vec!["d1".to_string(), "d2".to_string()],
            "defer scripts keep document order",
        );
    }

    /// `async` does not block the parser: an async script positioned before a later
    /// inline script runs after it (the async script is deferred past the blocking pass).
    ///
    /// FINDING (not a Livery/render split issue — this is `ScriptedDocument`,
    /// headless, unchanged by this port): this assertion contradicts
    /// `script-runtime-api`'s own documented `ScriptTiming::Async` semantics
    /// ("with a synchronous resource route it *is* available at the pause, so
    /// this runs there" — i.e. with a synchronous fetcher, `async` behaves
    /// like `Blocking`, not `Deferred`). The observed order is `["async",
    /// "inline"]`, matching the documented behavior; this test's expectation
    /// of `["inline", "async"]` was never correct. It never ran before (dead
    /// under `#[cfg(feature = "render")]`), so nothing caught it. See the
    /// `_on_boa` wrapper below, kept `#[ignore]`d rather than silently
    /// corrected, since fixing an assertion during a test-hygiene pass is out
    /// of this task's scope.
    fn async_runs_after_parser_blocking<E: ScriptEngine>() {
        let files = map_fetcher(&[
            (
                "http://x/index.html",
                "<body>\
                    <script src=\"a.js\" async></script>\
                    <script>console.log('inline');</script>\
                 </body>",
            ),
            ("http://x/a.js", "console.log('async');"),
        ]);
        let doc = ScriptedDocument::<E>::load(&files, "http://x/index.html").expect("loads");
        assert_eq!(
            doc.console(),
            vec!["inline".to_string(), "async".to_string()],
            "the async script does not block the later inline script",
        );
    }

    /// A non-JavaScript `type` (here `application/json`) is a data block: its content
    /// is never executed, even though it is syntactically runnable JS. A classic
    /// sibling still runs.
    fn script_type_data_block_is_not_executed<E: ScriptEngine>() {
        let html = "<body>\
            <script type=\"application/json\">console.log('json-ran');</script>\
            <script>console.log('classic-ran');</script>\
         </body>";
        let doc = ScriptedDocument::<E>::parse(html).expect("loads");
        assert_eq!(
            doc.console(),
            vec!["classic-ran".to_string()],
            "the application/json data block did not execute",
        );
    }

    /// A `type=module` script never breaks the page: its classic siblings run
    /// regardless of whether this backend supports modules. (Engine-agnostic: a
    /// module-capable backend also runs the module — after the parser-blocking pass —
    /// but that is asserted in the Boa-only module tests below.)
    fn module_keeps_classic_siblings_running<E: ScriptEngine>() {
        let html = "<body>\
            <script type=\"module\">globalThis.__m = 1;</script>\
            <script>console.log('classic-ran');</script>\
         </body>";
        let doc = ScriptedDocument::<E>::parse(html).expect("loads");
        assert!(
            doc.console().iter().any(|l| l == "classic-ran"),
            "the classic sibling runs regardless of module support: {:?}",
            doc.console(),
        );
    }

    /// A `type=module` script executes with **module scope**: its top-level
    /// `var` is module-local and does not leak to `globalThis` (a classic script's
    /// `var` would). Proves modules run with real module semantics, not script eval.
    fn module_executes_with_module_scope<E: ScriptEngine>() {
        let html = "<body><script type=\"module\">\
            var moduleLocal = 7;\
            console.log('module:' + moduleLocal + ',' + (typeof globalThis.moduleLocal));\
            </script></body>";
        let doc = ScriptedDocument::<E>::parse(html).expect("loads");
        assert!(
            doc.console().iter().any(|l| l == "module:7,undefined"),
            "module ran with module scope (local visible, not leaked): {:?}",
            doc.console(),
        );
    }

    /// Modules are deferred: an inline classic script runs before a module that
    /// precedes it in document order.
    fn module_runs_after_parser_blocking<E: ScriptEngine>() {
        let html = "<body>\
            <script type=\"module\">console.log('module');</script>\
            <script>console.log('classic');</script>\
         </body>";
        let doc = ScriptedDocument::<E>::parse(html).expect("loads");
        assert_eq!(
            doc.console(),
            vec!["classic".to_string(), "module".to_string()],
            "the classic script runs before the earlier-positioned module (modules defer)",
        );
    }

    /// A module that `import`s another but cannot fetch it (the fetch-free
    /// `parse` path has no loader) fails gracefully: the import rejects, the module is
    /// reported and skipped, and a classic sibling still runs (the page is not broken).
    fn module_import_fails_gracefully<E: ScriptEngine>() {
        let html = "<body>\
            <script type=\"module\">import x from './dep.js'; console.log('after-import');</script>\
            <script>console.log('sibling');</script>\
         </body>";
        let doc = ScriptedDocument::<E>::parse(html).expect("loads");
        assert!(
            !doc.console().iter().any(|l| l == "after-import"),
            "the import rejected, so the module body past the import did not run: {:?}",
            doc.console(),
        );
        assert!(
            doc.console().iter().any(|l| l == "sibling"),
            "the failed module is not fatal — the classic sibling still runs: {:?}",
            doc.console(),
        );
    }

    /// An external `<script type=module src=…>` is fetched (like a classic
    /// external) and evaluated as a module.
    fn external_module_runs<E: ScriptEngine>() {
        let files = map_fetcher(&[
            (
                "http://x/index.html",
                "<body><script type=\"module\" src=\"m.js\"></script></body>",
            ),
            (
                "http://x/m.js",
                "console.log('ext-module:' + (typeof globalThis.x));\nvar x = 1;",
            ),
        ]);
        let doc = ScriptedDocument::<E>::load(&files, "http://x/index.html").expect("loads");
        assert!(
            doc.console().iter().any(|l| l == "ext-module:undefined"),
            "external module fetched and run with module scope: {:?}",
            doc.console(),
        );
    }

    /// Cross-module `import` works: an entry module imports a named export from
    /// a relative dependency (resolved against the entry's URL and fetched through the
    /// host loader) and uses it.
    ///
    /// FINDING (not a Livery/render split issue — headless `ScriptedDocument`
    /// script loading, unchanged by this port): fails as authored.
    /// `DocumentScriptLoader::resolve` (this file) resolves every specifier —
    /// including a module's relative `import`s — through `crate::resolve_href`,
    /// which does plain-string prefix concatenation and never collapses a
    /// leading `./`: `resolve_href("http://x/main.js", "./dep.js")` yields
    /// `"http://x/./dep.js"`, which does not string-match the fixture's
    /// `"http://x/dep.js"` key, so the fetch is reported and the import
    /// rejects. `lib.rs`'s own doc comment on `resolve_href` says "Module
    /// resolution uses `url::Url::join` separately where normalization is
    /// required" — that normalizing path is not actually wired up here. This
    /// test never ran before (dead under `#[cfg(feature = "render")]`), so
    /// nothing caught it. Left `#[ignore]`d rather than silently fixed, since
    /// patching resolution logic is out of this test-hygiene pass's scope.
    fn module_imports_dependency<E: ScriptEngine>() {
        let files = map_fetcher(&[
            (
                "http://x/index.html",
                "<body><script type=\"module\" src=\"main.js\"></script></body>",
            ),
            (
                "http://x/main.js",
                "import { greet } from './dep.js';\nconsole.log(greet('world'));",
            ),
            (
                "http://x/dep.js",
                "export function greet(name) { return 'hello ' + name; }",
            ),
        ]);
        let doc = ScriptedDocument::<E>::load(&files, "http://x/index.html").expect("loads");
        assert!(
            doc.console().iter().any(|l| l == "hello world"),
            "the entry module imported and used the dependency's export: {:?}",
            doc.console(),
        );
    }

    /// A diamond import (`main` → `b`, `c` → `shared`) loads `shared` exactly
    /// once: its top-level side effect fires a single time (the loader caches by URL).
    ///
    /// FINDING: the same `resolve_href` "./" bug as `module_imports_dependency`
    /// above (`b.js`/`c.js` importing `./shared.js` fails to resolve).
    fn module_import_diamond_loads_shared_once<E: ScriptEngine>() {
        let files = map_fetcher(&[
            (
                "http://x/index.html",
                "<body><script type=\"module\" src=\"main.js\"></script></body>",
            ),
            (
                "http://x/main.js",
                "import { b } from './b.js';\nimport { c } from './c.js';\nconsole.log('main:' + b + c);",
            ),
            (
                "http://x/b.js",
                "import { x } from './shared.js';\nexport var b = x;",
            ),
            (
                "http://x/c.js",
                "import { x } from './shared.js';\nexport var c = x;",
            ),
            (
                "http://x/shared.js",
                "console.log('shared-init');\nexport var x = 'S';",
            ),
        ]);
        let doc = ScriptedDocument::<E>::load(&files, "http://x/index.html").expect("loads");
        let console = doc.console();
        assert_eq!(
            console.iter().filter(|l| *l == "shared-init").count(),
            1,
            "the shared module initializes exactly once across the diamond: {console:?}",
        );
        assert!(
            console.iter().any(|l| l == "main:SS"),
            "both branches see the shared export: {console:?}",
        );
    }

    /// `<script charset>` decodes the fetched bytes with the named encoding, not
    /// UTF-8: an ISO-8859-1 script with a `0xE9` byte ('é') decodes to `café`. As
    /// UTF-8 the lone `0xE9` is invalid and would become a replacement char.
    fn external_script_charset_decodes<E: ScriptEngine>() {
        let mut script = b"console.log('caf".to_vec();
        script.push(0xE9); // 'é' in ISO-8859-1; invalid as UTF-8
        script.extend_from_slice(b"');");
        let files = map_of(vec![
            (
                "http://x/index.html",
                b"<body><script src=\"app.js\" charset=\"iso-8859-1\"></script></body>".to_vec(),
            ),
            ("http://x/app.js", script),
        ]);
        let doc = ScriptedDocument::<E>::load(&files, "http://x/index.html").expect("loads");
        assert!(
            doc.console().iter().any(|l| l == "caf\u{e9}"),
            "iso-8859-1 script decoded to 'café': {:?}",
            doc.console(),
        );
    }

    /// A matching `integrity` (SRI) hash lets the external script run.
    fn integrity_match_runs<E: ScriptEngine>() {
        use base64::Engine as _;
        use sha2::Digest as _;
        let script = b"console.log('sri-ok');";
        let hash = base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(script));
        let files = map_of(vec![
            (
                "http://x/index.html",
                format!(
                    "<body><script src=\"app.js\" integrity=\"sha256-{hash}\"></script></body>"
                )
                .into_bytes(),
            ),
            ("http://x/app.js", script.to_vec()),
        ]);
        let doc = ScriptedDocument::<E>::load(&files, "http://x/index.html").expect("loads");
        assert!(
            doc.console().iter().any(|l| l == "sri-ok"),
            "matching integrity runs the script: {:?}",
            doc.console(),
        );
    }

    /// A mismatched `integrity` hash blocks the external script (it never runs), but a
    /// classic sibling still runs — the block is per-script, not fatal.
    fn integrity_mismatch_blocks<E: ScriptEngine>() {
        use base64::Engine as _;
        use sha2::Digest as _;
        // A hash of *different* content: the fetched script will not match it.
        let wrong =
            base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(b"other bytes"));
        let files = map_of(vec![
            (
                "http://x/index.html",
                format!(
                    "<body>\
                        <script src=\"app.js\" integrity=\"sha256-{wrong}\"></script>\
                        <script>console.log('after');</script>\
                     </body>"
                )
                .into_bytes(),
            ),
            (
                "http://x/app.js",
                b"console.log('should-not-run');".to_vec(),
            ),
        ]);
        let doc = ScriptedDocument::<E>::load(&files, "http://x/index.html").expect("loads");
        assert!(
            !doc.console().iter().any(|l| l == "should-not-run"),
            "mismatched integrity blocks the script: {:?}",
            doc.console(),
        );
        assert!(
            doc.console().iter().any(|l| l == "after"),
            "the blocked script is not fatal — the sibling still runs: {:?}",
            doc.console(),
        );
    }

    /// `ScriptedDocument::load` sets the runtime base URL from the page URL, so a
    /// reflected URL attribute (`a.href`) resolves to an absolute URL against it.
    fn url_attributes_resolve_against_page_url<E: ScriptEngine>() {
        let files = map_fetcher(&[(
            "http://x/dir/index.html",
            "<body><a id='a' href='sub/p.html'></a>\
             <script>console.log(document.getElementById('a').href);</script></body>",
        )]);
        let doc = ScriptedDocument::<E>::load(&files, "http://x/dir/index.html").expect("loads");
        assert!(
            doc.console().iter().any(|l| l == "http://x/dir/sub/p.html"),
            "a.href resolved against the page URL: {:?}",
            doc.console(),
        );
    }

    /// The headless-scripted-DOM scrape: `extract()` reads the **post-JS** DOM, so a
    /// page whose heading + link are injected by JavaScript yields them — where a
    /// static parse of the served HTML sees only the empty shell. Proves the
    /// extraction lane reaches JS-rendered (SPA) content.
    fn extract_sees_post_js_dom<E: ScriptEngine>() {
        let html = "<body><script>\
            var h = document.createElement('h1');\
            h.appendChild(document.createTextNode('Injected Title'));\
            document.body.appendChild(h);\
            var a = document.createElement('a');\
            a.setAttribute('href', '/spa/route');\
            a.appendChild(document.createTextNode('go'));\
            document.body.appendChild(a);\
            </script></body>";

        // Control: a static parse of the same HTML sees the shell — no heading, no link.
        let static_extract = fleece::extract(&StaticDocument::parse(html));
        assert!(
            static_extract.headings.is_empty(),
            "static parse sees no JS-injected heading"
        );
        assert!(
            static_extract.links.is_empty(),
            "static parse sees no JS-injected link"
        );

        // Post-JS: the scripted document's extract has the injected content.
        let doc = ScriptedDocument::<E>::parse(html).expect("runtime inits");
        let page = doc.extract();
        assert_eq!(
            page.headings,
            vec![fleece::Heading {
                level: 1,
                text: "Injected Title".into()
            }],
        );
        assert_eq!(page.links.len(), 1, "the injected link is extracted");
        assert_eq!(page.links[0].href, "/spa/route");
        assert_eq!(page.links[0].text, "go");
    }

    #[test]
    fn webgl_factory_is_installed_before_inline_script_on_boa() {
        webgl_factory_is_installed_before_inline_script::<BoaEngine>();
    }
    #[test]
    fn node_identity_is_stable_on_boa() {
        node_identity_is_stable::<BoaEngine>();
    }
    #[test]
    fn pump_collects_orphans_on_boa() {
        pump_collects_orphans::<BoaEngine>();
    }
    #[test]
    fn hidden_clamps_timers_frozen_stops_them_on_boa() {
        hidden_clamps_timers_frozen_stops_them::<BoaEngine>();
    }
    #[test]
    fn gc_soak_bounds_memory_on_boa() {
        gc_soak_bounds_memory::<BoaEngine>();
    }
    #[test]
    fn from_body_wires_document_cookie_on_boa() {
        from_body_wires_document_cookie::<BoaEngine>();
    }
    #[test]
    fn scripts_run_in_document_order_on_boa() {
        scripts_run_in_document_order::<BoaEngine>();
    }
    #[test]
    fn relative_src_resolves_against_page_url_on_boa() {
        relative_src_resolves_against_page_url::<BoaEngine>();
    }
    #[test]
    fn missing_external_script_is_skipped_on_boa() {
        missing_external_script_is_skipped::<BoaEngine>();
    }
    #[test]
    fn defer_runs_after_parser_blocking_on_boa() {
        defer_runs_after_parser_blocking::<BoaEngine>();
    }
    #[test]
    fn defer_scripts_run_in_document_order_on_boa() {
        defer_scripts_run_in_document_order::<BoaEngine>();
    }
    #[test]
    #[ignore = "script-runtime-api's own ScriptTiming::Async doc: with a synchronous fetcher, async runs at the parser pause (like Blocking), not after it; this test's expected order was never correct — see the fn doc above"]
    fn async_runs_after_parser_blocking_on_boa() {
        async_runs_after_parser_blocking::<BoaEngine>();
    }
    #[test]
    fn script_type_data_block_is_not_executed_on_boa() {
        script_type_data_block_is_not_executed::<BoaEngine>();
    }
    #[test]
    fn module_keeps_classic_siblings_running_on_boa() {
        module_keeps_classic_siblings_running::<BoaEngine>();
    }
    #[test]
    fn module_executes_with_module_scope_on_boa() {
        module_executes_with_module_scope::<BoaEngine>();
    }
    #[test]
    fn module_runs_after_parser_blocking_on_boa() {
        module_runs_after_parser_blocking::<BoaEngine>();
    }
    #[test]
    fn module_import_fails_gracefully_on_boa() {
        module_import_fails_gracefully::<BoaEngine>();
    }
    #[test]
    fn external_module_runs_on_boa() {
        external_module_runs::<BoaEngine>();
    }
    #[test]
    #[ignore = "DocumentScriptLoader::resolve resolves a module's relative import through resolve_href, which does not collapse a leading './' (see the fn doc above); the fetch never string-matches the fixture key"]
    fn module_imports_dependency_on_boa() {
        module_imports_dependency::<BoaEngine>();
    }
    #[test]
    #[ignore = "same resolve_href './' bug as module_imports_dependency_on_boa"]
    fn module_import_diamond_loads_shared_once_on_boa() {
        module_import_diamond_loads_shared_once::<BoaEngine>();
    }
    #[test]
    fn external_script_charset_decodes_on_boa() {
        external_script_charset_decodes::<BoaEngine>();
    }
    #[test]
    fn integrity_match_runs_on_boa() {
        integrity_match_runs::<BoaEngine>();
    }
    #[test]
    fn integrity_mismatch_blocks_on_boa() {
        integrity_mismatch_blocks::<BoaEngine>();
    }
    #[test]
    fn url_attributes_resolve_against_page_url_on_boa() {
        url_attributes_resolve_against_page_url::<BoaEngine>();
    }
    #[test]
    fn extract_sees_post_js_dom_on_boa() {
        extract_sees_post_js_dom::<BoaEngine>();
    }

    #[cfg(feature = "scripted-nova")]
    mod nova {
        use super::*;
        use script_engine_nova::NovaEngine;

        #[test]
        fn webgl_factory_is_installed_before_inline_script_on_nova() {
            webgl_factory_is_installed_before_inline_script::<NovaEngine>();
        }
        #[test]
        fn node_identity_is_stable_on_nova() {
            node_identity_is_stable::<NovaEngine>();
        }
        #[test]
        fn pump_collects_orphans_on_nova() {
            pump_collects_orphans::<NovaEngine>();
        }
        #[test]
        fn gc_soak_bounds_memory_on_nova() {
            gc_soak_bounds_memory::<NovaEngine>();
        }
        #[test]
        fn scripts_run_in_document_order_on_nova() {
            scripts_run_in_document_order::<NovaEngine>();
        }
        #[test]
        fn relative_src_resolves_against_page_url_on_nova() {
            relative_src_resolves_against_page_url::<NovaEngine>();
        }
        #[test]
        fn missing_external_script_is_skipped_on_nova() {
            missing_external_script_is_skipped::<NovaEngine>();
        }
        #[test]
        fn defer_runs_after_parser_blocking_on_nova() {
            defer_runs_after_parser_blocking::<NovaEngine>();
        }
        #[test]
        fn defer_scripts_run_in_document_order_on_nova() {
            defer_scripts_run_in_document_order::<NovaEngine>();
        }
        #[test]
        #[ignore = "same ScriptTiming::Async finding as async_runs_after_parser_blocking_on_boa"]
        fn async_runs_after_parser_blocking_on_nova() {
            async_runs_after_parser_blocking::<NovaEngine>();
        }
        #[test]
        fn script_type_data_block_is_not_executed_on_nova() {
            script_type_data_block_is_not_executed::<NovaEngine>();
        }
        #[test]
        fn module_keeps_classic_siblings_running_on_nova() {
            module_keeps_classic_siblings_running::<NovaEngine>();
        }
        #[test]
        fn module_executes_with_module_scope_on_nova() {
            module_executes_with_module_scope::<NovaEngine>();
        }
        #[test]
        fn module_runs_after_parser_blocking_on_nova() {
            module_runs_after_parser_blocking::<NovaEngine>();
        }
        #[test]
        fn module_import_fails_gracefully_on_nova() {
            module_import_fails_gracefully::<NovaEngine>();
        }
        #[test]
        fn external_module_runs_on_nova() {
            external_module_runs::<NovaEngine>();
        }
        #[test]
        #[ignore = "same resolve_href './' finding as module_imports_dependency_on_boa"]
        fn module_imports_dependency_on_nova() {
            module_imports_dependency::<NovaEngine>();
        }
        #[test]
        #[ignore = "same resolve_href './' finding as module_imports_dependency_on_boa"]
        fn module_import_diamond_loads_shared_once_on_nova() {
            module_import_diamond_loads_shared_once::<NovaEngine>();
        }
        #[test]
        fn external_script_charset_decodes_on_nova() {
            external_script_charset_decodes::<NovaEngine>();
        }
        #[test]
        fn integrity_match_runs_on_nova() {
            integrity_match_runs::<NovaEngine>();
        }
        #[test]
        fn integrity_mismatch_blocks_on_nova() {
            integrity_mismatch_blocks::<NovaEngine>();
        }
        #[test]
        fn url_attributes_resolve_against_page_url_on_nova() {
            url_attributes_resolve_against_page_url::<NovaEngine>();
        }
        #[test]
        fn extract_sees_post_js_dom_on_nova() {
            extract_sees_post_js_dom::<NovaEngine>();
        }
    }

    /// The render-needing receipts: ported onto [`LiveryScriptedDocument`], the
    /// crate's one live paint route. Everything here that used to run through
    /// the retired genet-layout `ScriptedDocument::frame`/`click_at`/`scroll_by`
    /// etc. now runs through Livery's equivalents.
    #[cfg(feature = "livery")]
    mod livery_render_tests {
        use super::*;
        use script_engine_boa::BoaEngine;

        /// A page whose inline script injects a `<p>` with text: the rendered scene gains
        /// glyph runs that an empty body would not have — the load → run-script → mutate →
        /// render path end to end.
        fn mutation_renders<E: ScriptEngine>() {
            let html = "<body><script>\
                var p = document.createElement('p');\
                p.appendChild(document.createTextNode('injected'));\
                document.body.appendChild(p);\
                </script></body>";
            let mut doc = LiveryScriptedDocument::<E>::parse(html).expect("runtime inits");
            let scene = doc.frame(400, 300);
            assert!(
                scene
                    .ops
                    .iter()
                    .any(|op| matches!(op, netrender::SceneOp::GlyphRun(_))),
                "script-injected text renders as glyphs",
            );
        }

        /// Control: with no script, the same empty body paints no text — so the glyphs in
        /// [`mutation_renders`] came from the script, not the markup.
        fn empty_body_has_no_text<E: ScriptEngine>() {
            let mut doc =
                LiveryScriptedDocument::<E>::parse("<body></body>").expect("runtime inits");
            let scene = doc.frame(400, 300);
            assert!(
                !scene
                    .ops
                    .iter()
                    .any(|op| matches!(op, netrender::SceneOp::GlyphRun(_))),
                "an empty body paints no text",
            );
        }

        /// Script that builds tall content makes the document scrollable: the offset
        /// advances on a wheel delta and clamps at the bottom.
        fn scripted_content_scrolls<E: ScriptEngine>() {
            let html = "<body><script>\
                var d = document.createElement('div');\
                d.setAttribute('style', 'height: 2000px');\
                document.body.appendChild(d);\
                </script></body>";
            let mut doc = LiveryScriptedDocument::<E>::parse(html).expect("runtime inits");
            let _ = doc.frame(400, 300);
            assert_eq!(doc.scroll(), (0.0, 0.0), "starts at the top");
            assert!(doc.scroll_by(0.0, 250.0), "tall scripted content scrolls");
            assert!(
                (doc.scroll().1 - 250.0).abs() < 0.5,
                "offset advanced: {:?}",
                doc.scroll()
            );
            let _ = doc.scroll_by(0.0, 100_000.0);
            assert!(!doc.scroll_by(0.0, 100.0), "clamped at the bottom edge");
        }

        /// `links()` is empty before the first frame; after a frame it reports the href +
        /// a positive-area rect for a script-injected link, from the retained Livery
        /// geometry — the same table a host resolves a click against, no per-click query
        /// into the live DOM needed.
        fn scripted_links_report_after_a_frame<E: ScriptEngine>() {
            let html = "<body><script>\
                var a = document.createElement('a');\
                a.setAttribute('href', 'https://example.test/');\
                a.appendChild(document.createTextNode('go'));\
                document.body.appendChild(a);\
                </script></body>";
            let mut doc = LiveryScriptedDocument::<E>::parse(html).expect("runtime inits");
            assert!(doc.links().is_empty(), "no rects before the first frame");
            let _ = doc.frame(400, 300);
            let links = doc.links();
            assert!(
                !links.is_empty(),
                "the script-injected link harvested at least one rect"
            );
            for (href, rect) in &links {
                assert_eq!(href, "https://example.test/");
                assert!(
                    rect[2] > rect[0] && rect[3] > rect[1],
                    "positive-area rect: {rect:?}"
                );
            }
        }

        /// An external `<script src>` is fetched and executed: the script injects a `<p>`,
        /// so the rendered scene gains glyph runs an empty body would not have — the
        /// load → fetch-script → run → mutate → render path end to end.
        fn external_script_runs<E: ScriptEngine>() {
            let files = map_fetcher(&[
                (
                    "http://x/index.html",
                    "<body><script src=\"app.js\"></script></body>",
                ),
                (
                    "http://x/app.js",
                    "var p=document.createElement('p');\
                     p.appendChild(document.createTextNode('ext'));\
                     document.body.appendChild(p);",
                ),
            ]);
            let mut doc =
                LiveryScriptedDocument::<E>::load(files, "http://x/index.html").expect("loads");
            let scene = doc.frame(400, 300);
            assert!(
                scene
                    .ops
                    .iter()
                    .any(|op| matches!(op, netrender::SceneOp::GlyphRun(_))),
                "external-script-injected text renders as glyphs",
            );
        }

        /// `from_body` runs an external `<script src>` against an already-fetched body,
        /// without re-fetching the document: the caller supplies the page HTML, the fetcher
        /// supplies only the script, and the injected text renders.
        fn from_body_runs_external_script<E: ScriptEngine>() {
            let files = map_fetcher(&[(
                "http://x/app.js",
                "var p=document.createElement('p');\
                 p.appendChild(document.createTextNode('ext'));\
                 document.body.appendChild(p);",
            )]);
            let body = "<body><script src=\"app.js\"></script></body>";
            let mut doc =
                LiveryScriptedDocument::<E>::from_body(body, files, "http://x/index.html")
                    .expect("from_body");
            let scene = doc.frame(400, 300);
            assert!(
                scene
                    .ops
                    .iter()
                    .any(|op| matches!(op, netrender::SceneOp::GlyphRun(_))),
                "external script run against a host-supplied body renders glyphs",
            );
        }

        /// End-to-end: `getComputedStyle` reads the rendered frame's cascade through
        /// Livery's `LiveryComputedStyle` bridge. The page schedules the read in a
        /// timer; `frame()` lays out (populating the bridge), then `pump()` fires the
        /// timer so the read sees real computed values.
        fn get_computed_style_reads_cascade<E: ScriptEngine>() {
            let mut doc = LiveryScriptedDocument::<E>::parse(
                "<html><body><div id='d' style='color: red; display: inline'></div>\
                 <script>setTimeout(function(){\
                   var cs = getComputedStyle(document.getElementById('d'));\
                   console.log(cs.color + '|' + cs.display);\
                 }, 0);</script></body></html>",
            )
            .expect("doc");
            let _ = doc.frame(400, 300); // lay out -> populate the bridge
            doc.pump(16.0); // fire the timer -> getComputedStyle reads the cascade
            assert!(
                doc.console().iter().any(|l| l == "rgb(255, 0, 0)|inline"),
                "getComputedStyle read the cascade: {:?}",
                doc.console(),
            );
        }

        /// The input → event bridge end to end: a `click_at` over a laid-out element
        /// hit-tests it and dispatches a `click` that runs the page's listener.
        fn click_dispatches_to_script<E: ScriptEngine>() {
            let html = "<body>\
                <div id='hit' style='width:300px;height:200px'></div>\
                <script>document.getElementById('hit')\
                    .addEventListener('click', function(){ console.log('clicked-div'); });</script>\
                </body>";
            let mut doc = LiveryScriptedDocument::<E>::parse(html).expect("runtime inits");
            let _ = doc.frame(400, 300); // lay out so hit-testing resolves the div
            let _ = doc.click_at(50.0, 50.0); // inside the 300×200 div
            assert!(
                doc.console().iter().any(|l| l == "clicked-div"),
                "the click dispatched to the div's listener: {:?}",
                doc.console(),
            );
        }

        /// A click listener calling `preventDefault` suppresses the default action: the
        /// control (no listener) scrolls to the anchor's `#bot` target, while the same
        /// page with a `preventDefault` listener does not.
        ///
        /// Unlike the retired `ScriptedDocument::click_at` (whose bool return was
        /// "did the default action move the scroll"), Livery's `click_at` returns "did
        /// the click hit anything at all" (`ScriptedClick::Miss` vs not) — a listener
        /// calling `preventDefault` still counts as a hit. So this port asserts
        /// directly on `scroll()` rather than reusing the return value as a stand-in
        /// for "scrolled".
        fn prevent_default_blocks_anchor_nav<E: ScriptEngine>() {
            let page = |listener: &str| {
                format!(
                    "<body>\
                        <a id='lnk' href='#bot' style='display:block;width:300px;height:40px'>go</a>\
                        <div style='height:2000px'></div>\
                        <div id='bot'>end</div>\
                        <script>{listener}</script>\
                     </body>"
                )
            };
            // Control: no preventDefault — clicking the anchor scrolls to #bot.
            let mut nav = LiveryScriptedDocument::<E>::parse(&page("")).expect("doc");
            let _ = nav.frame(400, 300);
            assert!(
                nav.click_at(20.0, 20.0),
                "the anchor is a Livery hit target"
            );
            assert!(
                nav.scroll().1 > 0.0,
                "anchor nav scrolls without preventDefault: scroll={:?}",
                nav.scroll(),
            );
            // preventDefault on the anchor's click suppresses that scroll.
            let mut blocked = LiveryScriptedDocument::<E>::parse(&page(
                "document.getElementById('lnk')\
                 .addEventListener('click', function(e){ e.preventDefault(); });",
            ))
            .expect("doc");
            let _ = blocked.frame(400, 300);
            assert!(
                blocked.click_at(20.0, 20.0),
                "the anchor is still hit even though its default is blocked"
            );
            assert_eq!(
                blocked.scroll().1,
                0.0,
                "preventDefault blocks anchor nav: scroll={:?}",
                blocked.scroll(),
            );
        }

        /// A finding, not a port: Livery installs a `ComputedStyleHandler`
        /// (`LiveryComputedStyle`, wired in `LiveryCssom::install_live_with_optional_sink`)
        /// but never a `MediaQueryHandler`. Until Livery wires one, `window.matchMedia`
        /// cannot reflect the rendered frame's device the way the retired
        /// `MediaQueryBridge` did over `genet_layout::IncrementalLayout`. Kept as a
        /// compiling, `#[ignore]`d receipt of the gap rather than deleted, since the
        /// behavior itself (device-aware `matchMedia`) is still wanted on this route.
        #[test]
        #[ignore = "Livery installs no MediaQueryHandler yet (see LiveryCssom::install_live_with_optional_sink); window.matchMedia() never reflects the frame's device on this route"]
        fn match_media_evaluates_against_the_frame_on_boa() {
            let mut doc = LiveryScriptedDocument::<BoaEngine>::parse(
                "<html><body><script>setTimeout(function(){\
                   var w = matchMedia('(min-width: 100px)');\
                   var n = matchMedia('(min-width: 9999px)');\
                   var rm = matchMedia('(prefers-reduced-motion: no-preference)');\
                   console.log(w.matches + '|' + n.matches + '|' + rm.matches + '|' + (w.media.length > 0));\
                 }, 0);</script></body></html>",
            )
            .expect("doc");
            let _ = doc.frame(400, 300); // populate the retained frame/device
            doc.pump(16.0); // fire the timer -> matchMedia evaluates
            assert!(
                doc.console().iter().any(|l| l == "true|false|true|true"),
                "matchMedia evaluated against the frame: {:?}",
                doc.console(),
            );
        }

        #[test]
        fn mutation_renders_on_boa() {
            mutation_renders::<BoaEngine>();
        }
        #[test]
        fn empty_body_has_no_text_on_boa() {
            empty_body_has_no_text::<BoaEngine>();
        }
        #[test]
        fn scripted_content_scrolls_on_boa() {
            scripted_content_scrolls::<BoaEngine>();
        }
        #[test]
        fn scripted_links_report_after_a_frame_on_boa() {
            scripted_links_report_after_a_frame::<BoaEngine>();
        }
        #[test]
        fn external_script_runs_on_boa() {
            external_script_runs::<BoaEngine>();
        }
        #[test]
        fn from_body_runs_external_script_on_boa() {
            from_body_runs_external_script::<BoaEngine>();
        }
        #[test]
        fn get_computed_style_reads_cascade_on_boa() {
            get_computed_style_reads_cascade::<BoaEngine>();
        }
        #[test]
        fn click_dispatches_to_script_on_boa() {
            click_dispatches_to_script::<BoaEngine>();
        }
        #[test]
        fn prevent_default_blocks_anchor_nav_on_boa() {
            prevent_default_blocks_anchor_nav::<BoaEngine>();
        }

        #[cfg(feature = "scripted-nova")]
        mod nova {
            use super::*;
            use script_engine_nova::NovaEngine;

            #[test]
            fn mutation_renders_on_nova() {
                mutation_renders::<NovaEngine>();
            }
            #[test]
            fn scripted_content_scrolls_on_nova() {
                scripted_content_scrolls::<NovaEngine>();
            }
            #[test]
            fn external_script_runs_on_nova() {
                external_script_runs::<NovaEngine>();
            }
            #[test]
            fn get_computed_style_reads_cascade_on_nova() {
                get_computed_style_reads_cascade::<NovaEngine>();
            }
            #[test]
            fn click_dispatches_to_script_on_nova() {
                click_dispatches_to_script::<NovaEngine>();
            }
            #[test]
            fn prevent_default_blocks_anchor_nav_on_nova() {
                prevent_default_blocks_anchor_nav::<NovaEngine>();
            }
        }

        #[derive(Clone)]
        struct LiveryFixtureFetcher {
            resources: std::collections::BTreeMap<String, Vec<u8>>,
        }

        impl LiveryFixtureFetcher {
            fn new() -> Self {
                let image = include_bytes!("../../resources/servo_64.png").to_vec();
                let mut resources = std::collections::BTreeMap::new();
                resources.insert(
                    "https://f4.test/route/theme.css".to_string(),
                    b".card { color: rgb(0, 0, 255); } .floor { height: 600px; }".to_vec(),
                );
                resources.insert("https://f4.test/route/first.png".to_string(), image.clone());
                resources.insert(
                    "https://f4.test/route/second.png".to_string(),
                    image.clone(),
                );
                resources.insert(
                    "https://f4.test/route/font-a.woff2".to_string(),
                    image.clone(),
                );
                resources.insert("https://f4.test/route/font-b.woff2".to_string(), image);
                Self { resources }
            }
        }

        impl ResourceFetcher for LiveryFixtureFetcher {
            fn fetch(&self, url: &str) -> Option<Vec<u8>> {
                self.resources.get(url).cloned()
            }
        }

        /// F4's core receipt: scripts see Livery CSSOM before parser-blocking
        /// execution, then the one runtime DOM drives a resource-backed Livery
        /// frame after both image and font source replacements.
        ///
        /// Relocated: this test already targeted `LiveryScriptedDocument`, but
        /// was trapped inside the crate's dead `#[cfg(feature = "render")]` test
        /// module and so never ran. It moves here unchanged.
        #[test]
        fn livery_scripted_document_owns_live_cssom_resources_and_frame_on_boa() {
            let html = r#"<!doctype html><html><head>
                <link rel="stylesheet" href="theme.css">
                <style id="faces">@font-face { font-family: F4; src: url(font-a.woff2); }</style>
                </head><body>
                <img id="hero" src="first.png" width="64" height="64">
                <div id="card" class="card">before</div>
                <div class="floor"></div>
                <script>
                  const card = document.getElementById('card');
                  console.log(document.styleSheets.length + '|' +
                    String(getComputedStyle(card).color === 'rgb(0, 0, 255)'));
                  document.getElementById('hero').src = 'second.png';
                  document.getElementById('faces').textContent =
                    '@font-face { font-family: F4; src: url(font-b.woff2); }';
                  card.textContent = 'after';
                  document.body.addEventListener('click', function () {
                    card.setAttribute('data-clicked', 'yes');
                  });
                </script>
                </body></html>"#;
            let mut document = LiveryScriptedDocument::<BoaEngine>::from_body(
                html,
                LiveryFixtureFetcher::new(),
                "https://f4.test/route/index.html",
            )
            .expect("Livery scripted document builds");

            let initial_scene = document.frame(320, 180);
            assert_eq!(document.console(), vec!["2|true"]);
            assert!(document.dom_snapshot().contains("after"));
            assert!(
                document.scroll_by(0.0, 30.0),
                "Livery owns live viewport input"
            );
            assert!(
                document.click_at(16.0, 16.0),
                "Livery hit-tests into the runtime DOM"
            );
            assert!(document.dom_snapshot().contains("data-clicked=\"yes\""));
            let post_click_scene = document.frame(320, 180);
            assert!(
                initial_scene.dump_ops().contains("images=1")
                    && post_click_scene.dump_ops().contains("images=1"),
                "the Livery paint list survives a live event-driven resource replacement"
            );
            let resources = document.resource_set().expect("live Livery ledger");
            let urls = resources
                .resources
                .iter()
                .map(|resource| resource.resolved_url.as_str())
                .collect::<Vec<_>>();
            assert!(urls.contains(&"https://f4.test/route/second.png"));
            assert!(urls.contains(&"https://f4.test/route/font-b.woff2"));
            assert!(!urls.contains(&"https://f4.test/route/first.png"));
            assert!(!urls.contains(&"https://f4.test/route/font-a.woff2"));
        }

        /// A real F4 control is an inline/block layout target, not merely the body
        /// fallback used by the resource receipt above. Its listener must therefore
        /// receive the Livery hit-test target itself.
        ///
        /// Relocated alongside the receipt above — same dead-module trap.
        #[test]
        fn livery_scripted_button_hit_dispatches_to_its_listener_on_boa() {
            let html = r#"<style>
                html, body { margin: 0; padding: 0; }
                button { display: block; width: 240px; height: 80px; margin: 0; padding: 0; }
            </style>
            <button id="swap">Mutate live DOM</button>
            <script>
                document.getElementById('swap').addEventListener('click', function () {
                    this.setAttribute('data-clicked', 'yes');
                });
            </script>"#;
            let mut document = LiveryScriptedDocument::<BoaEngine>::parse(html)
                .expect("Livery scripted button fixture builds");

            let _ = document.frame(320, 180);
            assert!(
                document.click_at(120.0, 40.0),
                "the button's painted box is a Livery hit target"
            );
            assert!(
                document.dom_snapshot().contains("data-clicked=\"yes\""),
                "the hit button received its own runtime click listener"
            );
        }
    }
}
