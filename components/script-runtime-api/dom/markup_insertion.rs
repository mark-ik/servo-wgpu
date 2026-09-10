// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Dynamic markup insertion and the document's readiness, as the small pieces
//! of state a native can reach.
//!
//! Two halves, and they are genuinely different:
//!
//! - **During a parse**, `document.write` is not a DOM operation at all: it
//!   inserts source at the *insertion point* of the tokenizer's input stream,
//!   immediately after the running script's own position. The native therefore
//!   only queues the text; the parser driver in [`crate::parse`] pops the queue
//!   when the script returns and pushes it onto the buffer queue, which is the
//!   spec's insertion point exactly. Nothing else can be correct — the DOM has
//!   no way to express "half an open tag".
//!
//! - **After a parse**, `document.write` implies `document.open`, which
//!   *replaces* the document — and then opens a source stream of its own. That
//!   stream is a real tokenizer over this same arena, built here and fed at its
//!   insertion point by each later write, so writes **append** instead of
//!   re-materializing the document, nodes keep their identity across two
//!   writes, and a `<script>` in written source runs.
//!
//! A native cannot re-enter the engine, so it cannot *run* that script itself.
//! The split is one function deep: `__docPumpStream` drives the tokenizer to
//! its next `<script>` and hands the source back to the bootstrap's own
//! `document.write`, which evaluates it and pumps again. The loop is therefore
//! in JavaScript, where re-entry is ordinary, and `document.write` stays
//! synchronous — the markup, and anything a written script did, is visible to
//! the next statement of the calling script.
//!
//! `document.currentScript` and `document.readyState` live here for the same
//! reason: they are host facts a native reads, set by the parser driver at the
//! spec's points rather than guessed by the bootstrap.

use crate::OwnerResolvedCx as _;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use genet_scripted_dom::parser::{DocumentParser, ParsePause, ParserPolicy};
use layout_dom_api::{LayoutDom, LocalName, Namespace};

use super::*;
use crate::parse::ParkedDom;

/// The document's readiness, per HTML.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReadyState {
    /// The parser is running.
    Loading,
    /// Parsing finished; deferred scripts have run or are about to.
    Interactive,
    /// Everything that delays the load event is done.
    #[default]
    Complete,
}

impl ReadyState {
    /// The IDL string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Interactive => "interactive",
            Self::Complete => "complete",
        }
    }
}

/// Host state for dynamic markup insertion and readiness.
///
/// The default is a document that is already `complete` with no parser — which
/// is what every entry point that does not drive [`crate::parse`] has always
/// implied, so nothing changes for those callers.
#[derive(Default)]
pub struct MarkupState {
    /// The document's readiness. Only [`crate::parse`] moves it off the
    /// default.
    pub ready_state: ReadyState,
    /// Whether the **document's own** parser is driving this stream, rather
    /// than a post-parse `document.open`. The difference is only who runs the
    /// scripts it stops at: [`crate::parse`]'s driver, which owns HTML's
    /// script-timing model, or the bootstrap's own `document.write` loop.
    pub parser_active: bool,
    /// `document.currentScript`, set for the duration of a classic script.
    pub current_script: Option<NodeId>,
    /// The document's source stream: one live tokenizer over this arena, used
    /// both for the document's own parse and for a post-parse
    /// `document.open`. `None` = nothing is parsing.
    pub open_stream: Option<OpenStream>,
    /// A pause the `document.write` pump reached that the document parser's
    /// driver has not consumed yet. `document.write` tokenizes its own source
    /// immediately (HTML: the parser processes the inserted characters), so it
    /// can reach the next `<script>` before the driver's loop does; the driver
    /// takes it from here rather than resuming past it.
    pub stalled: Option<ParsePause>,
    /// How many `document.write` calls re-entered the token stream, for the
    /// parse report.
    pub writes_applied: usize,
    /// HTML's **already started** flag, per `<script>` element.
    ///
    /// It lives here rather than on the parser because the flag outlives the
    /// parse and travels with the node: "prepare the script element" sets it at
    /// step 10, *before* the scripting-disabled check at step 13, which is
    /// exactly why a script parsed into a document with no browsing context
    /// never runs when it is later moved into one. The parser's own EOF marking
    /// (`DocumentParser::script_already_started`) is a second, stream-scoped
    /// source of the same answer and both are consulted.
    pub already_started: HashSet<NodeId>,
    /// `document.write` source per document that is **not** the active one.
    ///
    /// Such a document has no browsing context and so no script timing to
    /// honour; there is no live tokenizer for it either, so each write appends
    /// to this buffer and re-materializes the document's contents from the
    /// whole of it. Its `<script>` elements are flagged already started as the
    /// fragment parser would have.
    pub detached_streams: HashMap<NodeId, String>,
    /// `<script>` elements staged by `__stageScripts` for the bootstrap's
    /// re-preparation loop, in tree order.
    pub pending_prepare: VecDeque<NodeId>,
}

/// The document's source stream: a tokenizer over the live arena.
///
/// The arena is *moved* into `cell` around each tokenizer call and moved back:
/// a tree-sink call and a DOM native both want `&mut ScriptedDom`, and they are
/// never live at the same instant. There is exactly one of these at a time,
/// which is what lets a `document.write` from inside a native feed the very
/// parser the driver is running.
pub struct OpenStream {
    cell: Rc<RefCell<ScriptedDom>>,
    parser: DocumentParser<ParkedDom>,
}

/// Empty the document and begin a source stream over it.
pub(crate) fn open_stream_begin(host: &mut HostState) {
    let document = host.dom.document();
    let children: Vec<NodeId> = host.dom.dom_children(document).collect();
    for child in children {
        host.dom.remove_child(child);
    }
    // The tree builder caches a handle to the document node at construction, so
    // the *real* arena is parked before `DocumentParser::new`; a handle minted
    // from a placeholder carries the placeholder's document tag and trips the
    // arena's cross-document fence on the first append.
    let cell = Rc::new(RefCell::new(std::mem::replace(
        &mut host.dom,
        ScriptedDom::new(),
    )));
    let parser = DocumentParser::new(ParkedDom(Rc::clone(&cell)), Rc::new(ParserPolicy::new()));
    host.dom = std::mem::replace(&mut *cell.borrow_mut(), ScriptedDom::new());
    host.dom.set_parsing(true);
    host.markup.open_stream = Some(OpenStream { cell, parser });
    host.markup.stalled = None;
    host.markup.ready_state = ReadyState::Loading;
}

/// Append `source` at the **end** of the stream's input — the document's own
/// bytes, not a `document.write`.
pub(crate) fn stream_push_source(host: &mut HostState, source: &str) {
    if let Some(stream) = &host.markup.open_stream {
        stream.parser.push_source(source);
    }
}

/// The policy table the tree builder reads, so the driver can refresh it.
pub(crate) fn stream_policy(host: &HostState) -> Option<Rc<ParserPolicy>> {
    host.markup
        .open_stream
        .as_ref()
        .map(|stream| Rc::clone(stream.parser.policy()))
}

/// The driver's `resume`: a pause `document.write` already reached, else one
/// more turn of the tokenizer.
pub(crate) fn stream_resume(host: &mut HostState) -> ParsePause {
    if let Some(pause) = host.markup.stalled.take() {
        return pause;
    }
    let Some(stream) = host.markup.open_stream.take() else {
        return ParsePause::Done;
    };
    let pause = with_parked(host, &stream, || stream.parser.resume());
    host.markup.open_stream = Some(stream);
    pause
}

/// Whether the tokenizer marked this `<script>` "already started".
pub(crate) fn stream_script_already_started(host: &HostState, node: NodeId) -> bool {
    host.markup
        .open_stream
        .as_ref()
        .is_some_and(|stream| stream.parser.script_already_started(node))
}

/// Tokenize written source now, up to the next `<script>` (which the document
/// parser's driver must run, so it is stalled rather than consumed) or the end
/// of what has been written.
///
/// HTML's `document.write` has the parser process the inserted characters
/// during the call. Queuing them until the calling script returns is visibly
/// wrong: `document.write("PASS"); assert(document.body.textContent == "PASS")`
/// is the shape a whole WPT battery is written in.
fn write_pump(host: &mut HostState) {
    let Some(stream) = host.markup.open_stream.take() else {
        return;
    };
    let pause = with_parked(host, &stream, || stream.parser.pump_written());
    host.markup.open_stream = Some(stream);
    if pause != ParsePause::Done {
        host.markup.stalled = Some(pause);
    }
}

/// Run `f` with the arena parked in the stream's cell, where the tokenizer
/// needs it.
fn with_parked<R>(host: &mut HostState, stream: &OpenStream, f: impl FnOnce() -> R) -> R {
    *stream.cell.borrow_mut() = std::mem::replace(&mut host.dom, ScriptedDom::new());
    let out = f();
    host.dom = std::mem::replace(&mut *stream.cell.borrow_mut(), ScriptedDom::new());
    out
}

/// Drive the stream to its next pause for the bootstrap's post-parse
/// `document.write` loop. `Some(source)` is a classic script it must now
/// evaluate (empty for one HTML says must not run, so the pump keeps its
/// shape); `None` means the stream has consumed everything written so far.
fn open_stream_pump(host: &mut HostState) -> Option<String> {
    match stream_resume(host) {
        ParsePause::Script(node) => {
            let inert = script_started(host, node);
            clear_parser_inserted(host, node);
            mark_script_started(host, node);
            host.markup.current_script = Some(node);
            Some(if inert {
                String::new()
            } else {
                script_source(&host.dom, node)
            })
        },
        ParsePause::Created | ParsePause::Done => {
            host.markup.current_script = None;
            None
        },
    }
}

/// End the stream, flushing the tokenizer's EOF handling.
pub(crate) fn open_stream_close(host: &mut HostState) {
    let Some(stream) = host.markup.open_stream.take() else {
        return;
    };
    let OpenStream { cell, parser } = stream;
    *cell.borrow_mut() = std::mem::replace(&mut host.dom, ScriptedDom::new());
    parser.end();
    host.dom = std::mem::replace(&mut *cell.borrow_mut(), ScriptedDom::new());
    host.dom.set_parsing(false);
    host.markup.stalled = None;
    host.markup.current_script = None;
}

/// Steps 5, 6 and 8 of HTML's "prepare the script element": the script's own
/// text, its `src`, and the script kind its `type`/`language` name. `None` when
/// the element is not a script, has neither a `src` nor any text, or names a
/// type that is not a script at all (a data block) — the three ways prepare
/// returns *before* step 10, and so without setting the already-started flag.
fn script_prepare_gate(
    dom: &ScriptedDom,
    node: NodeId,
) -> Option<(crate::parse::ScriptKind, Option<String>, String)> {
    // Both the HTML and the SVG `script` element take these steps; the SVG one
    // is why `svg/scripted/script-invalid-script-type.html` is a receipt here.
    if dom.element_name(node)?.local.as_ref() != "script" {
        return None;
    }
    let html = Namespace::from("");
    let attr = |name: &str| {
        dom.attribute(node, &html, &LocalName::from(name))
            .map(str::to_owned)
    };
    let text: String = dom
        .dom_children(node)
        .filter_map(|c| dom.text(c))
        .collect::<String>();
    let src = attr("src").filter(|s| !s.is_empty());
    if src.is_none() && text.is_empty() {
        return None;
    }
    let kind = crate::parse::classify(attr("type").as_deref(), attr("language").as_deref())?;
    Some((kind, src, text))
}

/// Whether `node` is a `<script>` the document's own parser created and has not
/// prepared yet — HTML's "parser document is non-null". Such an element is
/// **not** re-prepared by a DOM mutation; the parser will prepare it when it
/// pops it. `execution-timing/026.html` writes an empty `<script>` and then sets
/// its `src`, and expects exactly that.
pub(crate) fn script_is_parser_inserted(host: &HostState, node: NodeId) -> bool {
    host.markup
        .open_stream
        .as_ref()
        .is_some_and(|stream| stream.parser.policy().is_parser_created_script(node))
}

/// Prepare's step 3: the element stops being parser-inserted as the parser
/// prepares it.
pub(crate) fn clear_parser_inserted(host: &HostState, node: NodeId) {
    if let Some(stream) = host.markup.open_stream.as_ref() {
        stream.parser.policy().clear_parser_created_script(node);
    }
}

/// Whether `node` carries HTML's already-started flag, from either source.
pub(crate) fn script_started(host: &HostState, node: NodeId) -> bool {
    host.markup.already_started.contains(&node) || stream_script_already_started(host, node)
}

/// Step 10: set the already-started flag, if the element got that far.
pub(crate) fn mark_script_started(host: &mut HostState, node: NodeId) {
    if script_prepare_gate(&host.dom, node).is_some() {
        host.markup.already_started.insert(node);
    }
}

/// Flag every `<script>` in `root`'s subtree already started.
///
/// This is what a fragment parse does by construction: `innerHTML`,
/// `DOMParser.parseFromString` and a write into a document with no browsing
/// context all prepare their scripts in a document that has none, so each one
/// reaches step 10, sets the flag, and returns at step 13 without running. It
/// is also the only reason inserting such a subtree into the live document does
/// not execute it.
pub(crate) fn mark_subtree_scripts_started(host: &mut HostState, root: NodeId) {
    let scripts = collect_scripts(&host.dom, root);
    for node in scripts {
        mark_script_started(host, node);
    }
}

/// The `<script>` elements at or under `root`, in tree order.
fn collect_scripts(dom: &ScriptedDom, root: NodeId) -> Vec<NodeId> {
    fn walk(dom: &ScriptedDom, node: NodeId, out: &mut Vec<NodeId>) {
        if dom
            .element_name(node)
            .is_some_and(|name| name.local.as_ref() == "script")
        {
            out.push(node);
        }
        for child in dom.dom_children(node).collect::<Vec<_>>() {
            walk(dom, child, out);
        }
    }
    let mut out = Vec::new();
    if dom.is_live(root) {
        walk(dom, root, &mut out);
    }
    out
}

/// Whether `node` is in the active document — the scripted tier's only browsing
/// context. A `DOMParser` result and a `createHTMLDocument` document are not,
/// so "scripting is disabled" for their contents (prepare, step 13).
fn in_active_document(dom: &ScriptedDom, node: NodeId) -> bool {
    dom.tree_root(node) == Some(dom.document())
}

/// HTML's "prepare the script element" for a `<script>` the parser did not
/// insert: one that just became connected, had a node inserted into it while
/// connected, or had its `src` set while connected.
///
/// Returns the classic source to evaluate, or `None` when the element must not
/// run — and note the ordering that makes step 10 load-bearing: the flag is set
/// *before* the scripting-disabled check, so a script prepared in a document
/// with no browsing context can never run later either.
pub(crate) fn prepare_script(host: &mut HostState, node: NodeId) -> Option<String> {
    if script_started(host, node) {
        return None; // step 1
    }
    let (kind, src, text) = script_prepare_gate(&host.dom, node)?; // steps 5, 6, 8
    host.markup.already_started.insert(node); // step 10
    if !in_active_document(&host.dom, node) {
        return None; // steps 12-13
    }
    match kind {
        // A module needs its whole graph, a base URL and the deferred queue the
        // parser driver owns; this seam runs one classic script. Named as a
        // residual in the lane plan rather than half-run here.
        crate::parse::ScriptKind::Module => None,
        crate::parse::ScriptKind::Classic => match src {
            None => Some(text),
            Some(src) => {
                let url = crate::fetch::resolve_against(host.base_url.as_deref(), &src);
                let loader = host.script_loader.clone()?;
                loader.load(&url)
            },
        },
    }
}

/// `document.write` into a document that is not the active one.
///
/// There is no tokenizer for such a document, and no script timing to honour,
/// so the whole written stream is re-parsed and cloned in each time. Every
/// `<script>` it produces is flagged already started, exactly as the fragment
/// parser would.
fn detached_write(host: &mut HostState, document: NodeId, text: &str) {
    let source = {
        let buffer = host.markup.detached_streams.entry(document).or_default();
        buffer.push_str(text);
        buffer.clone()
    };
    let children: Vec<NodeId> = host.dom.dom_children(document).collect();
    for child in children {
        host.dom.remove_child(child);
    }
    let parsed = genet_static_dom::StaticDocument::parse(&source);
    let parsed_document = parsed.document();
    clone_into(&parsed, parsed_document, &mut host.dom, document);
    mark_subtree_scripts_started(host, document);
}

/// The document a `document.write` / `open` / `close` call names, and whether it
/// is the active one. A reflector for something that is not a document node at
/// all cannot happen from the bootstrap, which only reaches these through
/// `Document.prototype`.
fn write_target(host: &HostState, node: Option<NodeId>) -> (NodeId, bool) {
    let active = host.dom.document();
    match node {
        Some(node) if node != active && host.dom.is_live(node) => (node, false),
        _ => (active, true),
    }
}

/// The source a written `<script>` runs, per HTML's "prepare the script
/// element": its own text for a classic inline script, nothing for a module, a
/// data block, or an external one (the stream has no resource route).
fn script_source(dom: &ScriptedDom, node: NodeId) -> String {
    let html = Namespace::from("");
    let attr = |name: &str| dom.attribute(node, &html, &LocalName::from(name));
    if attr("src").is_some_and(|s| !s.is_empty()) {
        return String::new();
    }
    if crate::parse::classify(attr("type"), attr("language"))
        != Some(crate::parse::ScriptKind::Classic)
    {
        return String::new();
    }
    dom.dom_children(node)
        .filter_map(|c| dom.text(c))
        .collect::<String>()
}

fn with_host<E: ScriptEngine, R>(
    cx: &mut E::CallCx<'_>,
    f: impl FnOnce(&mut HostState) -> R,
) -> Option<R> {
    let data = cx.host_data()?;
    let cell = data.downcast_ref::<RefCell<HostState>>()?;
    let mut host = cell.borrow_mut();
    Some(f(&mut host))
}

// Stream parsing and its JS continuation belong to the callback's realm. A
// foreign document must never fall through write_target to this active document.
fn stream_target<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    value: &E::Value,
) -> Result<Option<NodeId>, E::Error> {
    let owned = cx.owned_node(value)?;
    let target = owned.as_ref().map(super::adoption::OwnedNode::id);
    if let Some(node) = target {
        // The stream's arena must be the callback realm's own, not merely some
        // same-origin store the owner-resolved accessor would happily reach.
        let local = with_host::<E, _>(cx, |host| host.dom.is_live(node)).unwrap_or(false);
        if !local {
            return Err(cx.error("document stream requires its owning document realm"));
        }
    }
    Ok(target)
}

/// `__readyState()` → `"loading"` / `"interactive"` / `"complete"`.
pub(crate) struct ReadyStateOf;
impl<E: ScriptEngine> NativeFn<E> for ReadyStateOf {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let state =
            with_host::<E, _>(cx, |host| host.markup.ready_state).unwrap_or(ReadyState::Complete);
        cx.make_string(state.as_str())
    }
}

/// `__currentScript()` → the reflector of the classic script now running, or
/// null. Modules never set it, per HTML.
pub(crate) struct CurrentScript;
impl<E: ScriptEngine> NativeFn<E> for CurrentScript {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let current = with_host::<E, _>(cx, |host| host.markup.current_script).flatten();
        match current {
            Some(id) => reflect_pinned::<E>(cx, id.raw() as u64),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__docOpen()` — empty the document and begin an open source stream. The
/// return value is unused; `document.open()` returns the document itself, which
/// the bootstrap already has.
pub(crate) struct DocOpen;
impl<E: ScriptEngine> NativeFn<E> for DocOpen {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let a0 = cx.arg(0);
        let target = stream_target::<E>(cx, &a0)?;
        with_host::<E, _>(cx, |host| match write_target(host, target) {
            (_, true) => open_stream_begin(host),
            // A document with no browsing context has no stream to open; its
            // buffer is simply emptied, which is what "replace all" comes to
            // for it.
            (document, false) => {
                host.markup.detached_streams.insert(document, String::new());
                let children: Vec<NodeId> = host.dom.dom_children(document).collect();
                for child in children {
                    host.dom.remove_child(child);
                }
            },
        });
        cx.make_string("")
    }
}

/// `__docWrite(text)` — `document.write` / `writeln`.
///
/// The text goes to the insertion point of the document's source stream —
/// implying `document.open` if nothing is parsing — and is tokenized straight
/// away, so the markup is in the DOM before `document.write` returns.
///
/// Who runs a `<script>` it reaches differs, and only that: while the
/// document's own parser is active the pause is stalled for
/// [`crate::parse`]'s driver, which owns HTML's script-timing model
/// (`async`, `defer`, modules, external sources); otherwise the bootstrap's
/// own loop pumps and evaluates it.
pub(crate) struct DocWrite;
impl<E: ScriptEngine> NativeFn<E> for DocWrite {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let text_v = cx.arg(0);
        let text = cx.value_to_string(&text_v)?;
        let a1 = cx.arg(1);
        let target = stream_target::<E>(cx, &a1)?;
        with_host::<E, _>(cx, |host| {
            // A write into a document that is not the active one must not touch
            // the active one — which is what this did until 2026-09-08, so
            // `doc.write(...)` on a `createHTMLDocument` result wiped the page.
            if let (document, false) = write_target(host, target) {
                detached_write(host, document, &text);
                return;
            }
            if host.markup.open_stream.is_none() {
                // The implied `document.open`: the document is replaced, so
                // whatever is in it now goes.
                open_stream_begin(host);
            }
            if let Some(stream) = &host.markup.open_stream {
                stream.parser.write_at_insertion_point(&text);
            }
            host.markup.writes_applied += 1;
            if host.markup.parser_active {
                write_pump(host);
            }
        });
        cx.make_string("")
    }
}

/// `__docPumpStream()` — tokenize the open stream up to its next `<script>`,
/// returning that script's source for the bootstrap to evaluate, or `null` when
/// the stream has consumed everything written so far.
///
/// This is the one seam that makes a written `<script>` run: the tokenizer
/// cannot call the engine, so the loop lives in `document.write` itself.
pub(crate) struct DocPumpStream;
impl<E: ScriptEngine> NativeFn<E> for DocPumpStream {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let a0 = cx.arg(0);
        let target = stream_target::<E>(cx, &a0)?;
        let pumped = with_host::<E, _>(cx, |host| {
            if host.markup.parser_active || !write_target(host, target).1 {
                return None;
            }
            open_stream_pump(host)
        })
        .flatten();
        match pumped {
            Some(source) => cx.make_string(&source),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__docClose()` — end an open source stream. A no-op while a parser is
/// driving the document (HTML ignores `close()` from a script the parser is
/// running).
pub(crate) struct DocClose;
impl<E: ScriptEngine> NativeFn<E> for DocClose {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let a0 = cx.arg(0);
        let target = stream_target::<E>(cx, &a0)?;
        with_host::<E, _>(cx, |host| {
            if host.markup.parser_active || !write_target(host, target).1 {
                return;
            }
            open_stream_close(host);
            host.markup.ready_state = ReadyState::Complete;
        });
        cx.make_string("")
    }
}

/// `__stageScripts(node, selfOnly)` — queue the `<script>` elements HTML says
/// must be re-prepared after a DOM insertion, and return how many.
///
/// `selfOnly` is the "a node was inserted into a connected script element" case:
/// the script to prepare is the insertion *parent*, not anything under it.
/// Otherwise the inserted root and its descendants are the candidates. Either
/// way nothing is queued unless the element is connected, which is prepare's
/// step 7.
///
/// The walk is here rather than in the bootstrap so that an ordinary
/// `appendChild` of a script-free subtree costs one arena traversal and no JS
/// wrappers at all.
pub(crate) struct StageScripts;
impl<E: ScriptEngine> NativeFn<E> for StageScripts {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let a0 = cx.arg(0);
        let Some(id) = cx.owned_node(&a0)? else {
            return cx.make_string("0");
        };
        let a1 = cx.arg(1);
        let self_only = cx.value_to_string(&a1).unwrap_or_default() == "1";
        let node = id.id();
        let staged = with_host::<E, _>(cx, |host| {
            if !host.dom.is_live(node) || !in_active_document(&host.dom, node) {
                return 0;
            }
            if script_is_parser_inserted(host, node) {
                return 0;
            }
            let is_script = host
                .dom
                .element_name(node)
                .is_some_and(|name| name.local.as_ref() == "script");
            let candidates = match (self_only, is_script) {
                (true, true) => vec![node],
                (true, false) => Vec::new(),
                (false, _) => collect_scripts(&host.dom, node),
            };
            let mut staged = 0;
            for found in candidates {
                if script_started(host, found) {
                    continue;
                }
                host.markup.pending_prepare.push_back(found);
                staged += 1;
            }
            staged
        })
        .unwrap_or(0);
        cx.make_string(&staged.to_string())
    }
}

/// `__nextPreparedScript()` — run "prepare the script element" for the next
/// staged `<script>` and hand back the classic source to evaluate (`""` for one
/// that must not run), or `null` when the queue is empty.
///
/// The loop lives in the bootstrap for the same reason `document.write`'s does:
/// a native cannot re-enter the engine, and preparing a script is exactly a
/// request to.
pub(crate) struct NextPreparedScript;
impl<E: ScriptEngine> NativeFn<E> for NextPreparedScript {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let next = with_host::<E, _>(cx, |host| {
            let node = host.markup.pending_prepare.pop_front()?;
            if !host.dom.is_live(node) {
                return Some(String::new());
            }
            let source = prepare_script(host, node).unwrap_or_default();
            host.markup.current_script = Some(node);
            Some(source)
        })
        .flatten();
        match next {
            Some(source) => cx.make_string(&source),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__copyScriptStarted(from, to)` — HTML's cloning steps for a `script`
/// element: "set copy's already started to el's already started".
///
/// Without it a clone of a script that has already run runs again, which is
/// what `execution-timing/061`–`067` measure.
pub(crate) struct CopyScriptStarted;
impl<E: ScriptEngine> NativeFn<E> for CopyScriptStarted {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let a0 = cx.arg(0);
        let a1 = cx.arg(1);
        let from = cx.owned_node(&a0)?;
        let to = cx.owned_node(&a1)?;
        if let (Some(from), Some(to)) = (from, to) {
            let from = from.id();
            let to = to.id();
            with_host::<E, _>(cx, |host| {
                if script_started(host, from) {
                    host.markup.already_started.insert(to);
                }
            });
        }
        cx.make_string("")
    }
}

/// `__prepareScriptEnd()` — clear `document.currentScript` after a prepared
/// script has run.
pub(crate) struct PrepareScriptEnd;
impl<E: ScriptEngine> NativeFn<E> for PrepareScriptEnd {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        with_host::<E, _>(cx, |host| host.markup.current_script = None);
        cx.make_string("")
    }
}

pub(crate) fn install<E: ScriptEngine>(
    engine: &mut crate::Surface<'_, '_, E>,
) -> Result<(), crate::SurfaceError<E::Error>> {
    engine.set_function::<ReadyStateOf>("__readyState", 0)?;
    engine.set_function::<CurrentScript>("__currentScript", 0)?;
    engine.set_function::<DocOpen>("__docOpen", 1)?;
    engine.set_function::<DocWrite>("__docWrite", 2)?;
    engine.set_function::<DocPumpStream>("__docPumpStream", 1)?;
    engine.set_function::<DocClose>("__docClose", 1)?;
    engine.set_function::<StageScripts>("__stageScripts", 2)?;
    engine.set_function::<NextPreparedScript>("__nextPreparedScript", 0)?;
    engine.set_function::<PrepareScriptEnd>("__prepareScriptEnd", 0)?;
    engine.set_function::<CopyScriptStarted>("__copyScriptStarted", 2)?;
    Ok(())
}
