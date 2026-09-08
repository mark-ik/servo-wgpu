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
            let inert = stream_script_already_started(host, node);
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
        with_host::<E, _>(cx, open_stream_begin);
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
        with_host::<E, _>(cx, |host| {
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
        let pumped = with_host::<E, _>(cx, |host| {
            if host.markup.parser_active {
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
        with_host::<E, _>(cx, |host| {
            if host.markup.parser_active {
                return;
            }
            open_stream_close(host);
            host.markup.ready_state = ReadyState::Complete;
        });
        cx.make_string("")
    }
}

pub(crate) fn install<E: ScriptEngine>(engine: &mut E) -> Result<(), E::Error> {
    engine.set_function::<ReadyStateOf>("__readyState", 0)?;
    engine.set_function::<CurrentScript>("__currentScript", 0)?;
    engine.set_function::<DocOpen>("__docOpen", 0)?;
    engine.set_function::<DocWrite>("__docWrite", 1)?;
    engine.set_function::<DocPumpStream>("__docPumpStream", 0)?;
    engine.set_function::<DocClose>("__docClose", 0)?;
    Ok(())
}
