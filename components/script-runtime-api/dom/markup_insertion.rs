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
//!   *replaces* the document. There is no tokenizer to feed, so the native
//!   keeps the written source in a buffer and re-materializes the document's
//!   contents from it. The exact rule and what it does not do are in the lane
//!   plan's Residuals.
//!
//! `document.currentScript` and `document.readyState` live here for the same
//! reason: they are host facts a native reads, set by the parser driver at the
//! spec's points rather than guessed by the bootstrap.

use genet_static_dom::StaticDocument;
use layout_dom_api::LayoutDom;

use super::*;

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
    /// Whether a document parser is currently driving this document. While it
    /// is, `document.write` feeds the token stream instead of replacing the
    /// document.
    pub parser_active: bool,
    /// Source written by `document.write` since the driver last drained it.
    /// Applied at the insertion point when the running script returns.
    pub pending_writes: Vec<String>,
    /// `document.currentScript`, set for the duration of a classic script.
    pub current_script: Option<NodeId>,
    /// The accumulated source of an implied-`document.open` stream, used when
    /// there is no parser to feed. `None` = no open stream.
    pub open_stream: Option<String>,
}

impl MarkupState {
    /// Take everything `document.write` queued during the last script.
    pub fn take_pending_writes(&mut self) -> String {
        if self.pending_writes.is_empty() {
            return String::new();
        }
        std::mem::take(&mut self.pending_writes).concat()
    }
}

/// Replace the document's children with the tree `source` parses to.
///
/// The post-parse `document.write` path. Re-materializing rather than appending
/// is what "`document.write` implies `document.open`" means: `open` empties the
/// document, and every write since then is one source stream.
fn materialize_open_stream(dom: &mut ScriptedDom, source: &str) {
    let document = dom.document();
    let children: Vec<NodeId> = dom.dom_children(document).collect();
    for child in children {
        dom.remove_child(child);
    }
    let parsed = StaticDocument::parse(source);
    super::clone_into(&parsed, parsed.document(), dom, document);
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
        with_host::<E, _>(cx, |host| {
            host.markup.open_stream = Some(String::new());
            host.markup.ready_state = ReadyState::Loading;
            let document = host.dom.document();
            let children: Vec<NodeId> = host.dom.dom_children(document).collect();
            for child in children {
                host.dom.remove_child(child);
            }
        });
        cx.make_string("")
    }
}

/// `__docWrite(text)` — `document.write` / `writeln`.
///
/// While a parser is driving this document the text is queued for the insertion
/// point and the driver applies it when the running script returns. Otherwise
/// it joins (and implies) an open source stream, which is re-materialized now
/// so the markup is visible to the rest of the calling script.
pub(crate) struct DocWrite;
impl<E: ScriptEngine> NativeFn<E> for DocWrite {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let text_v = cx.arg(0);
        let text = cx.value_to_string(&text_v)?;
        with_host::<E, _>(cx, |host| {
            if host.markup.parser_active {
                host.markup.pending_writes.push(text);
                return;
            }
            if host.markup.open_stream.is_none() {
                // The implied `document.open`: the document is replaced, so
                // whatever is in it now goes.
                host.markup.open_stream = Some(String::new());
                host.markup.ready_state = ReadyState::Loading;
                let document = host.dom.document();
                let children: Vec<NodeId> = host.dom.dom_children(document).collect();
                for child in children {
                    host.dom.remove_child(child);
                }
            }
            let source = {
                let stream = host.markup.open_stream.as_mut().expect("just set");
                stream.push_str(&text);
                stream.clone()
            };
            materialize_open_stream(&mut host.dom, &source);
        });
        cx.make_string("")
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
            if host.markup.open_stream.take().is_some() {
                host.markup.ready_state = ReadyState::Complete;
            }
        });
        cx.make_string("")
    }
}

pub(crate) fn install<E: ScriptEngine>(engine: &mut E) -> Result<(), E::Error> {
    engine.set_function::<ReadyStateOf>("__readyState", 0)?;
    engine.set_function::<CurrentScript>("__currentScript", 0)?;
    engine.set_function::<DocOpen>("__docOpen", 0)?;
    engine.set_function::<DocWrite>("__docWrite", 1)?;
    engine.set_function::<DocClose>("__docClose", 0)?;
    Ok(())
}
