/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The HTML parser **over the live arena**, driven one pause at a time.
//!
//! The static tier parses a whole document and hands the finished tree over.
//! A scripted document cannot: HTML's parsing model runs a `<script>` at the
//! point the tree builder pops it, against the tree *as far as it has been
//! built*, and the script may then change that tree — or push more source into
//! the token stream with `document.write`. So the scripted tier drives
//! html5ever itself instead of calling `parse_document(...).one(src)`.
//!
//! The seam is html5ever's own: [`Tokenizer::feed`] returns
//! `TokenizerResult::Script(handle)` when the tree builder pops a `</script>`,
//! having already inserted the element and its text. The caller runs it and
//! calls `feed` again. `document.write` is then literally
//! [`BufferQueue::push_front`] on the parser's input buffer — the spec's
//! "insertion point", which is immediately after the currently executing
//! script's own position in the input stream.
//!
//! Two things this file deliberately does **not** own: which scripts run and in
//! what order (that is the host's script-timing model, in `script-runtime-api`),
//! and any engine call. The tree builder asks two questions that only the script
//! tier can answer — whether an intended parent's custom element definition
//! disables shadow, and whether a host already has an imperative shadow root —
//! and both are answered from [`ParserPolicy`], a plain table the host refreshes
//! at each pause. That keeps the engine out of a `&self` tree-sink call, which
//! is re-entrant into a live tokenizer.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use html5ever::interface::ElemName;
use html5ever::interface::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::StrTendril;
use html5ever::{Attribute, ParseOpts, QualName, driver, ns};
use layout_dom_api::{
    LayoutDom, LayoutDomMut, LocalName, Namespace, NodeKind, ShadowRootInit, ShadowRootMode,
    SlotAssignmentMode, may_host_shadow_tree,
};
use markup5ever::TokenizerResult;

use crate::{NodeId, ScriptedDom};

/// Borrowed mutable access to the arena the parser builds into.
///
/// The arena lives inside the host's state, behind its own `RefCell`; the tree
/// sink only ever needs it for the duration of one call, and never across a
/// script run. A trait rather than an `Rc<RefCell<ScriptedDom>>` because the
/// arena is a *field* of the host state, not a separately shared cell.
pub trait DomAccess {
    /// Run `f` with mutable access to the arena.
    fn with<R>(&self, f: impl FnOnce(&mut ScriptedDom) -> R) -> R;
}

/// The questions the tree builder asks that only the script tier can answer,
/// as a table the host refreshes at every pause. The tokenizer is live inside
/// the tree-sink call, so calling the engine there would re-enter it; the
/// registry can only change while a script runs, and scripts only run at
/// pauses, so a table refreshed at each pause is exactly current at every
/// point in between.
#[derive(Default)]
pub struct ParserPolicy {
    inner: RefCell<PolicyState>,
}

#[derive(Default)]
struct PolicyState {
    /// Local names whose custom element definition lists `shadow` in
    /// `disabledFeatures`. HTML refuses a declarative shadow root on those.
    shadow_disabled: HashSet<String>,
    /// Elements the parser created whose local name could name a custom
    /// element, in creation order. The host drains this to run parse-time
    /// upgrades before the next script sees the tree.
    custom_candidates: Vec<NodeId>,
    /// The quirks mode the tree builder inferred from the doctype.
    quirks_mode: QuirksModeRecord,
}

/// The arena has no quirks-mode field (the scripted tier reports `compatMode`
/// as a constant), so the parser records what html5ever decided here rather
/// than dropping it. Named a residual in the lane plan.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum QuirksModeRecord {
    #[default]
    NoQuirks,
    LimitedQuirks,
    Quirks,
}

impl ParserPolicy {
    /// A fresh policy with nothing disabled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the set of local names whose definition disables shadow.
    pub fn set_shadow_disabled(&self, names: impl IntoIterator<Item = String>) {
        self.inner.borrow_mut().shadow_disabled = names.into_iter().collect();
    }

    /// Take the elements the parser created that could name a custom element.
    pub fn take_custom_candidates(&self) -> Vec<NodeId> {
        std::mem::take(&mut self.inner.borrow_mut().custom_candidates)
    }

    /// The quirks mode html5ever inferred.
    pub fn quirks_mode(&self) -> QuirksModeRecord {
        self.inner.borrow().quirks_mode
    }

    fn shadow_is_disabled(&self, local: &str) -> bool {
        self.inner.borrow().shadow_disabled.contains(local)
    }

    fn note_custom_candidate(&self, id: NodeId) {
        self.inner.borrow_mut().custom_candidates.push(id);
    }

    fn set_quirks_mode(&self, mode: QuirksModeRecord) {
        self.inner.borrow_mut().quirks_mode = mode;
    }
}

/// html5ever's borrowed element-name view over the arena's stored `QualName`.
/// Owned rather than borrowed for the same reason the static sink's is: the
/// name lives behind a `RefCell` borrow that ends with the call.
#[derive(Clone)]
pub struct ScriptedElemName(QualName);

impl fmt::Debug for ScriptedElemName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl ElemName for ScriptedElemName {
    fn ns(&self) -> &Namespace {
        &self.0.ns
    }

    fn local_name(&self) -> &LocalName {
        &self.0.local
    }
}

/// The tree sink that builds directly into a live [`ScriptedDom`].
///
/// Every insertion goes through the arena's own `LayoutDomMut` mutators, so a
/// `MutationObserver` registered by an earlier script sees parser insertions as
/// records, slot assignment stays maintained, and the structural epoch the
/// reflector-identity policy caches against advances — none of which a
/// bulk tree copy after the fact can produce.
pub struct ScriptedTreeSink<A: DomAccess> {
    access: A,
    policy: Rc<ParserPolicy>,
    /// Template element -> where its content goes. Normally the template's own
    /// contents fragment; for a template the tree builder turned into a
    /// declarative shadow root, the shadow root itself, so everything parsed
    /// inside lands there directly.
    content_target: RefCell<HashMap<NodeId, NodeId>>,
    /// `<script>` elements the tokenizer marked "already started" (EOF inside
    /// the element). Those must never run.
    already_started: RefCell<HashSet<NodeId>>,
}

impl<A: DomAccess> ScriptedTreeSink<A> {
    fn new(access: A, policy: Rc<ParserPolicy>) -> Self {
        Self {
            access,
            policy,
            content_target: RefCell::new(HashMap::new()),
            already_started: RefCell::new(HashSet::new()),
        }
    }

    /// Append `child` (a node or a run of text) under `parent`, merging text
    /// into a trailing text sibling as html5ever's contract requires.
    fn append_node_or_text(&self, parent: NodeId, child: NodeOrText<NodeId>) {
        match child {
            NodeOrText::AppendNode(node) => self.access.with(|dom| dom.append_child(parent, node)),
            NodeOrText::AppendText(text) => self.access.with(|dom| {
                if let Some(last) = dom.dom_children(parent).last() {
                    if dom.kind(last) == NodeKind::Text {
                        let merged = format!("{}{text}", dom.text(last).unwrap_or_default());
                        dom.set_text(last, &merged);
                        return;
                    }
                }
                let node = dom.create_text(&text);
                dom.append_child(parent, node);
            }),
        }
    }
}

impl<A: DomAccess> TreeSink for ScriptedTreeSink<A> {
    type Handle = NodeId;
    type Output = ();
    type ElemName<'a>
        = ScriptedElemName
    where
        A: 'a;

    fn finish(self) {}

    fn parse_error(&self, _msg: std::borrow::Cow<'static, str>) {}

    fn get_document(&self) -> NodeId {
        self.access.with(|dom| dom.document())
    }

    fn elem_name<'a>(&'a self, target: &'a NodeId) -> ScriptedElemName {
        self.access.with(|dom| {
            ScriptedElemName(
                dom.element_name(*target)
                    .expect("node is not an element")
                    .clone(),
            )
        })
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> NodeId {
        let is_custom_candidate = name.ns == ns!(html)
            && (name.local.contains('-') || attrs.iter().any(|a| *a.name.local == *"is"));
        let id = self.access.with(|dom| {
            let id = dom.create_element(name);
            for attr in &attrs {
                dom.set_attribute(id, attr.name.clone(), &attr.value);
            }
            if flags.template {
                let contents = dom.ensure_template_contents(id);
                self.content_target.borrow_mut().insert(id, contents);
            }
            id
        });
        if is_custom_candidate {
            self.policy.note_custom_candidate(id);
        }
        id
    }

    fn create_comment(&self, text: StrTendril) -> NodeId {
        self.access.with(|dom| dom.create_comment(&text))
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> NodeId {
        self.access
            .with(|dom| dom.create_processing_instruction(&target, &data))
    }

    fn append(&self, parent: &NodeId, child: NodeOrText<NodeId>) {
        self.append_node_or_text(*parent, child);
    }

    fn append_before_sibling(&self, sibling: &NodeId, child: NodeOrText<NodeId>) {
        let sibling = *sibling;
        match child {
            NodeOrText::AppendNode(node) => self.access.with(|dom| {
                let Some(parent) = dom.parent(sibling) else {
                    return;
                };
                dom.insert_before(parent, node, Some(sibling));
            }),
            NodeOrText::AppendText(text) => self.access.with(|dom| {
                let Some(parent) = dom.parent(sibling) else {
                    return;
                };
                // The sibling itself is never a text node here, but its old
                // previous sibling may be; merge into that one if so.
                let kids: Vec<NodeId> = dom.dom_children(parent).collect();
                if let Some(pos) = kids.iter().position(|&c| c == sibling) {
                    if pos > 0 && dom.kind(kids[pos - 1]) == NodeKind::Text {
                        let previous = kids[pos - 1];
                        let merged = format!("{}{text}", dom.text(previous).unwrap_or_default());
                        dom.set_text(previous, &merged);
                        return;
                    }
                }
                let node = dom.create_text(&text);
                dom.insert_before(parent, node, Some(sibling));
            }),
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &NodeId,
        prev_element: &NodeId,
        child: NodeOrText<NodeId>,
    ) {
        let has_parent = self.access.with(|dom| dom.parent(*element).is_some());
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append_node_or_text(*prev_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        self.access.with(|dom| {
            let doctype = dom.create_doctype(&name, &public_id, &system_id);
            let document = dom.document();
            dom.append_child(document, doctype);
        });
    }

    fn mark_script_already_started(&self, node: &NodeId) {
        self.already_started.borrow_mut().insert(*node);
    }

    fn get_template_contents(&self, target: &NodeId) -> NodeId {
        if let Some(&target) = self.content_target.borrow().get(target) {
            return target;
        }
        self.access
            .with(|dom| dom.ensure_template_contents(*target))
    }

    fn same_node(&self, x: &NodeId, y: &NodeId) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.policy.set_quirks_mode(match mode {
            QuirksMode::Quirks => QuirksModeRecord::Quirks,
            QuirksMode::LimitedQuirks => QuirksModeRecord::LimitedQuirks,
            QuirksMode::NoQuirks => QuirksModeRecord::NoQuirks,
        });
    }

    fn add_attrs_if_missing(&self, target: &NodeId, attrs: Vec<Attribute>) {
        self.access.with(|dom| {
            for attr in attrs {
                if dom
                    .attribute(*target, &attr.name.ns, &attr.name.local)
                    .is_none()
                {
                    dom.set_attribute(*target, attr.name, &attr.value);
                }
            }
        });
    }

    fn remove_from_parent(&self, target: &NodeId) {
        self.access.with(|dom| dom.remove_child(*target));
    }

    fn reparent_children(&self, node: &NodeId, new_parent: &NodeId) {
        let children: Vec<NodeId> = self.access.with(|dom| dom.dom_children(*node).collect());
        for child in children {
            self.access.with(|dom| dom.append_child(*new_parent, child));
        }
    }

    fn is_mathml_annotation_xml_integration_point(&self, _handle: &NodeId) -> bool {
        // The arena stores no per-element MathML integration-point flag; the
        // static sink's copy of it is a parse-time fact the tree copy drops
        // too. Named a residual in the lane plan.
        false
    }

    /// HTML: a declarative shadow root is refused when the intended parent's
    /// custom element definition disables the `shadow` feature. That definition
    /// lives in the script tier's registry, which is why this is the whole
    /// point of interleaving: the registry is populated by a script that ran
    /// earlier in *this* parse.
    fn allow_declarative_shadow_roots(&self, intended_parent: &NodeId) -> bool {
        let local = self.access.with(|dom| {
            dom.element_name(*intended_parent)
                .map(|n| n.local.to_string())
        });
        match local {
            Some(local) => !self.policy.shadow_is_disabled(&local),
            None => true,
        }
    }

    fn attach_declarative_shadow(
        &self,
        location: &NodeId,
        template: &NodeId,
        attrs: &[Attribute],
    ) -> bool {
        let host = *location;
        let template = *template;
        let attr = |name: &str| {
            attrs
                .iter()
                .find(|a| *a.name.local == *name)
                .map(|a| a.value.to_string())
        };
        let mode = match attr("shadowrootmode").as_deref() {
            Some("open") => ShadowRootMode::Open,
            Some("closed") => ShadowRootMode::Closed,
            _ => return false,
        };
        let init = ShadowRootInit {
            mode,
            delegates_focus: attr("shadowrootdelegatesfocus").is_some(),
            clonable: attr("shadowrootclonable").is_some(),
            serializable: attr("shadowrootserializable").is_some(),
            slot_assignment: match attr("shadowrootslotassignment").as_deref() {
                Some("manual") => SlotAssignmentMode::Manual,
                _ => SlotAssignmentMode::Named,
            },
        };
        let root = self.access.with(|dom| {
            let local = dom.element_name(host).map(|n| n.local.to_string());
            // Not a shadow host at all, or one that already has a root (an
            // imperative `attachShadow` from a script or a mutation-observer
            // callback earlier in this parse): the template stays ordinary,
            // which is what html5ever's `false` return means.
            if !local.as_deref().is_some_and(may_host_shadow_tree)
                || dom.shadow_root_of(host).is_some()
            {
                return None;
            }
            Some(dom.attach_shadow_unchecked(host, init, true))
        });
        match root {
            Some(root) => {
                self.content_target.borrow_mut().insert(template, root);
                true
            },
            None => false,
        }
    }
}

/// Why [`DocumentParser::resume`] stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParsePause {
    /// The tree builder popped this `<script>` element. It and its text child
    /// are already in the tree; the caller runs it (or not) and resumes.
    Script(NodeId),
    /// The input stream is exhausted. More input may still be pushed (the
    /// caller's own deferred sources); [`DocumentParser::end`] ends the parse.
    Done,
}

/// html5ever driven a pause at a time, over the live arena.
pub struct DocumentParser<A: DomAccess> {
    parser: driver::Parser<ScriptedTreeSink<A>>,
    policy: Rc<ParserPolicy>,
}

impl<A: DomAccess> DocumentParser<A> {
    /// Start a document parse into `access`'s arena.
    pub fn new(access: A, policy: Rc<ParserPolicy>) -> Self {
        let sink = ScriptedTreeSink::new(access, Rc::clone(&policy));
        let parser = driver::parse_document(sink, ParseOpts::default());
        Self { parser, policy }
    }

    /// The policy table the host refreshes at each pause.
    pub fn policy(&self) -> &Rc<ParserPolicy> {
        &self.policy
    }

    /// Append source at the **end** of the input stream. This is the document's
    /// own bytes as they arrive, not `document.write`.
    pub fn push_source(&self, source: &str) {
        if !source.is_empty() {
            self.parser.input_buffer.push_back(StrTendril::from(source));
        }
    }

    /// Insert source **at the insertion point** — immediately after the
    /// currently executing script's position in the input stream. This is
    /// `document.write` during parsing, and it is why the parser has to own its
    /// own buffer queue rather than being handed one string.
    pub fn write_at_insertion_point(&self, source: &str) {
        if !source.is_empty() {
            self.parser
                .input_buffer
                .push_front(StrTendril::from(source));
        }
    }

    /// Whether the tokenizer has consumed everything pushed so far.
    pub fn input_is_empty(&self) -> bool {
        self.parser.input_buffer.is_empty()
    }

    /// Tokenize until a `<script>` is popped or the input runs out.
    pub fn resume(&self) -> ParsePause {
        loop {
            match self.parser.tokenizer.feed(&self.parser.input_buffer) {
                TokenizerResult::Done => return ParsePause::Done,
                TokenizerResult::Script(node) => return ParsePause::Script(node),
                // An encoding indicator on an already-decoded string stream is
                // nothing to act on; keep tokenizing.
                _ => continue,
            }
        }
    }

    /// Whether this `<script>` was marked "already started" (EOF inside it), in
    /// which case HTML says it must not execute.
    pub fn script_already_started(&self, node: NodeId) -> bool {
        self.parser
            .tokenizer
            .sink
            .sink
            .already_started
            .borrow()
            .contains(&node)
    }

    /// End the parse: flush the tokenizer's EOF handling.
    pub fn end(self) {
        // `Parser::finish` would loop to done and assert the buffer is empty;
        // the caller has already driven it to `Done`, and an abandoned parse
        // (a `document.open` mid-stream) may deliberately leave input behind.
        while self.parser.input_buffer.pop_front().is_some() {}
        self.parser.tokenizer.end();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use layout_dom_api::LayoutDom;

    use super::*;

    struct CellAccess(Rc<RefCell<ScriptedDom>>);

    impl DomAccess for CellAccess {
        fn with<R>(&self, f: impl FnOnce(&mut ScriptedDom) -> R) -> R {
            f(&mut self.0.borrow_mut())
        }
    }

    fn parse_pauses(html: &str) -> (Rc<RefCell<ScriptedDom>>, Vec<String>) {
        let dom = Rc::new(RefCell::new(ScriptedDom::new()));
        let policy = Rc::new(ParserPolicy::new());
        let parser = DocumentParser::new(CellAccess(Rc::clone(&dom)), policy);
        parser.push_source(html);
        let mut seen = Vec::new();
        loop {
            match parser.resume() {
                ParsePause::Done => break,
                ParsePause::Script(node) => {
                    let text = {
                        let dom = dom.borrow();
                        dom.dom_children(node)
                            .next()
                            .and_then(|c| dom.text(c).map(str::to_owned))
                            .unwrap_or_default()
                    };
                    seen.push(text);
                },
            }
        }
        parser.end();
        (dom, seen)
    }

    #[test]
    fn pauses_at_each_script_in_document_order() {
        let (_dom, seen) =
            parse_pauses("<body><script>one</script><p>x</p><script>two</script></body>");
        assert_eq!(seen, vec!["one".to_owned(), "two".to_owned()]);
    }

    #[test]
    fn the_tree_is_only_built_as_far_as_the_pause() {
        let dom = Rc::new(RefCell::new(ScriptedDom::new()));
        let policy = Rc::new(ParserPolicy::new());
        let parser = DocumentParser::new(CellAccess(Rc::clone(&dom)), policy);
        parser.push_source("<body><script>s</script><p id=later>x</p></body>");
        assert!(matches!(parser.resume(), ParsePause::Script(_)));
        // The `<p>` after the script has not been parsed yet: this is the whole
        // difference between interleaving and parse-then-run.
        let count = {
            let dom = dom.borrow();
            let mut n = 0;
            let mut stack = vec![dom.document()];
            while let Some(id) = stack.pop() {
                n += 1;
                stack.extend(dom.dom_children(id));
            }
            n
        };
        assert!(
            count < 8,
            "tree should be partial at the pause, got {count}"
        );
        assert!(matches!(parser.resume(), ParsePause::Done));
        parser.end();
    }

    #[test]
    fn document_write_reenters_the_token_stream_at_the_insertion_point() {
        let dom = Rc::new(RefCell::new(ScriptedDom::new()));
        let policy = Rc::new(ParserPolicy::new());
        let parser = DocumentParser::new(CellAccess(Rc::clone(&dom)), policy);
        parser.push_source("<body><script>w</script><span id=after></span></body>");
        assert!(matches!(parser.resume(), ParsePause::Script(_)));
        parser.write_at_insertion_point("<b id=written></b>");
        assert!(matches!(parser.resume(), ParsePause::Done));
        parser.end();
        // The written element must land *before* the source that followed the
        // script, not appended at the end.
        let dom = dom.borrow();
        let mut order = Vec::new();
        let mut stack = vec![dom.document()];
        while let Some(id) = stack.pop() {
            if let Some(name) = dom.element_name(id) {
                order.push(name.local.to_string());
            }
            let kids: Vec<NodeId> = dom.dom_children(id).collect();
            stack.extend(kids.into_iter().rev());
        }
        let b = order.iter().position(|n| n == "b").expect("written <b>");
        let span = order.iter().position(|n| n == "span").expect("<span>");
        assert!(b < span, "document.write must insert before later source");
    }

    #[test]
    fn a_declarative_template_becomes_a_shadow_root_at_parse_time() {
        let (dom, _) = parse_pauses(
            "<body><div id=host><template shadowrootmode=open><span></span></template></div></body>",
        );
        let dom = dom.borrow();
        let host = find_local(&dom, "div").expect("host");
        let root = dom.shadow_root_of(host).expect("declarative shadow root");
        assert_eq!(dom.dom_children(root).count(), 1);
        // The template itself never entered the tree.
        assert!(find_local(&dom, "template").is_none());
    }

    #[test]
    fn a_disabled_definition_leaves_the_template_ordinary() {
        let dom = Rc::new(RefCell::new(ScriptedDom::new()));
        let policy = Rc::new(ParserPolicy::new());
        policy.set_shadow_disabled(["shadow-disabled".to_owned()]);
        let parser = DocumentParser::new(CellAccess(Rc::clone(&dom)), Rc::clone(&policy));
        parser.push_source(
            "<body><shadow-disabled><template shadowrootmode=open><span></span></template>\
             </shadow-disabled></body>",
        );
        while !matches!(parser.resume(), ParsePause::Done) {}
        parser.end();
        let dom = dom.borrow();
        let host = find_local(&dom, "shadow-disabled").expect("host");
        assert!(dom.shadow_root_of(host).is_none());
        assert!(find_local(&dom, "template").is_some());
    }

    #[test]
    fn an_existing_imperative_root_leaves_the_template_ordinary() {
        let dom = Rc::new(RefCell::new(ScriptedDom::new()));
        let policy = Rc::new(ParserPolicy::new());
        let parser = DocumentParser::new(CellAccess(Rc::clone(&dom)), policy);
        parser.push_source("<body><div id=host><script>s</script>");
        assert!(matches!(parser.resume(), ParsePause::Script(_)));
        // Stand in for the mutation-observer callback the WPT case uses.
        {
            let mut dom = dom.borrow_mut();
            let host = find_local(&dom, "div").expect("host");
            dom.attach_shadow_unchecked(host, ShadowRootInit::default(), false);
        }
        parser.write_at_insertion_point(
            "<template id=ordinary shadowrootmode=open><span></span></template></div></body>",
        );
        while !matches!(parser.resume(), ParsePause::Done) {}
        parser.end();
        let dom = dom.borrow();
        let template = find_local(&dom, "template").expect("template stays ordinary");
        assert!(dom.template_contents_of(template).is_some());
    }

    fn find_local(dom: &ScriptedDom, local: &str) -> Option<NodeId> {
        let mut stack = vec![dom.document()];
        while let Some(id) = stack.pop() {
            if dom.element_name(id).is_some_and(|n| n.local == *local) {
                return Some(id);
            }
            stack.extend(dom.dom_children(id));
        }
        None
    }
}
