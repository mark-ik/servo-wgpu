// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The `document` / `Node` construction surface — the live-DOM rung of the host
//! layer (`pluggable_engines_testharness_plan` step 2).
//!
//! Same shape as the rest of the host surface: native sinks (here, mutators of the
//! [`ScriptedDom`] in host state) plus a JS bootstrap that assembles the ergonomic
//! `document` object and wraps node handles. The JS→DOM bridge is the **reflector**
//! — a JS-opaque value carrying a `NodeId` (proven by `genet-scripted`'s `setText`,
//! generalized here and made engine-neutral). Incoming nodes are recovered with
//! `CallCx::reflector_data`; outgoing nodes (`createElement`, `getElementById`) are
//! returned via `CallCx::reflector_for`, which mints **canonical** reflectors (one
//! per node), so the JS wrapper cache keyed on them gives identity
//! (`getElementById('x') === getElementById('x')`).
//!
//! Construction/mutation half: `createElement`, `createTextNode`, `appendChild`,
//! `setAttribute`, `textContent` (setter), `getElementById`. Read half
//! (`getAttribute`, `tagName`, `textContent` getter, `getElementsByTagName`,
//! `parentNode`), via `CallCx::make_string` / `make_null`. Generic over the
//! backend; tested on Boa + Nova like the rest of the host surface.
//!
//! Dispatch is prototype-based with an `Element` / `Text` / `Document` split over
//! `Node` (`instanceof`, `nodeType`); nodes are `EventTarget`s with real tree
//! propagation (capture → target → bubble over `parentNode`, `stopPropagation`).
//! A source document can be cloned in ([`clone_into`] /
//! [`crate::Runtime::load_dom`]), with `document.body` / `documentElement` /
//! `head` over it. The `Element` surface: `getAttribute` / `setAttribute` /
//! `hasAttribute` / `removeAttribute` / `toggleAttribute`, `id` / `className`
//! reflection, `classList`, and `querySelector` / `querySelectorAll` / `matches`
//! (via [`crate::selector`]). Node traversal: `childNodes` / `firstChild` /
//! siblings / element-filtered views, `nodeName` / `nodeValue`, the mutators
//! `removeChild` / `insertBefore` / `replaceChild` (throwing `NotFoundError`), and
//! the `ChildNode` mixin. Namespaces: `localName` / `namespaceURI` / `prefix`,
//! namespace-gated `tagName`, `createElementNS`. A **reflected IDL attribute**
//! layer on `Element.prototype` (DOMString / boolean / approximate-enumerated /
//! long kinds, table-driven) and `TreeWalker` / `NodeIterator` / `NodeFilter`.
//! `Comment` / `DocumentFragment` node types (`createComment` /
//! `createDocumentFragment`, nodeType 8 / 11), `cloneNode` (shallow + deep), and
//! **live** `HTMLCollection`s — `getElementsByTagName` / `getElementsByClassName` /
//! `children` are Proxy-backed and re-walked per access, so they reflect later
//! mutations — plus `DOMTokenList` (`classList` / `relList`), `dataset`, and
//! `NodeList` (`childNodes`). The reflected-attribute table carries `tokenlist`
//! (`t`, e.g. `relList`) and `url` (`u`, e.g. `href` / `src`, resolved against the
//! document base URL) kinds alongside DOMString / boolean / enumerated / long.
//! Verified by `dom_fragment_clone`, `dom_collections_works`,
//! `dom_tokenlist_dataset_works`, `dom_url_reflection_works`. Only the `double`
//! reflected kind remains. See
//! `docs/2026-05-26_pluggable_engines_testharness_plan.md`.

use std::cell::RefCell;
use std::rc::Rc;

use genet_scripted_dom::{NodeId, ScriptedDom};
use layout_dom_api::{LayoutDom, LayoutDomMut, LocalName, Namespace, NodeKind, QualName};
use markup5ever::Prefix;
use script_engine_api::{CallCx, NativeFn, ScriptEngine};

use crate::HostState;

/// The XHTML namespace — HTML elements live here; `tagName` upper-cases only in it.
const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";

/// A node's qualified name (`prefix:local`, or just `local`), case preserved.
fn qualified_of(name: &QualName) -> String {
    match name.prefix.as_ref() {
        Some(p) => format!("{}:{}", p.as_ref(), name.local.as_ref()),
        None => name.local.as_ref().to_string(),
    }
}

/// Clone `src`'s tree (elements with attributes + text) under `dst_parent` in the
/// scripted DOM, recursively. Backs [`crate::Runtime::load_dom`] and
/// `DOMParser.parseFromString`: a parsed document (any [`LayoutDom`]) becomes a
/// live document scripts query. Every node kind survives — comments, processing
/// instructions, CDATA sections and the doctype included, so `document.doctype`
/// and a `MutationObserver` on a parsed comment have something to name.
pub(crate) fn clone_into<D: LayoutDom>(
    src: &D,
    src_node: D::NodeId,
    dst: &mut ScriptedDom,
    dst_parent: NodeId,
) {
    for child in src.dom_children(src_node) {
        match src.kind(child) {
            NodeKind::Element => {
                let Some(name) = src.element_name(child) else {
                    continue;
                };
                let el = dst.create_element(name.clone());
                for attr in src.attributes(child) {
                    dst.set_attribute(el, attr.name.clone(), attr.value);
                }
                dst.append_child(dst_parent, el);
                clone_into(src, child, dst, el);
                // A `<template>`'s contents are not its children: the copier has
                // to ask for the fragment, or the scripted tier would show an
                // empty `template.content` for a parsed template.
                if let Some(contents) = src.template_contents(child) {
                    let dst_contents = dst.ensure_template_contents(el);
                    clone_into(src, contents, dst, dst_contents);
                }
                // Nor is a shadow root. The static tier's declarative post-parse
                // pass has already run, so what arrives here is a real shadow
                // root; carrying it over is what makes a scripted page and a
                // script-free page agree on the flat tree.
                if let Some(root) = src.shadow_root(child) {
                    let init = src.shadow_init(child).unwrap_or_default();
                    let dst_root = dst.attach_shadow_unchecked(el, init, true);
                    clone_into(src, root, dst, dst_root);
                }
            },
            NodeKind::Text => {
                let t = dst.create_text(src.text(child).unwrap_or(""));
                dst.append_child(dst_parent, t);
            },
            NodeKind::CdataSection => {
                let t = dst.create_cdata_section(src.text(child).unwrap_or(""));
                dst.append_child(dst_parent, t);
            },
            NodeKind::Comment => {
                let c = dst.create_comment(src.text(child).unwrap_or(""));
                dst.append_child(dst_parent, c);
            },
            NodeKind::ProcessingInstruction => {
                let target = src
                    .element_name(child)
                    .map_or(String::new(), |q| q.local.as_ref().to_string());
                let pi = dst.create_processing_instruction(&target, src.text(child).unwrap_or(""));
                dst.append_child(dst_parent, pi);
            },
            NodeKind::Doctype => {
                let d = match src.doctype_data(child) {
                    Some(d) => dst.create_doctype(d.name, d.public_id, d.system_id),
                    None => dst.create_doctype("html", "", ""),
                };
                dst.append_child(dst_parent, d);
            },
            NodeKind::Document | NodeKind::DocumentFragment | NodeKind::ShadowRoot => {},
        }
    }
}

/// First element child of `node` (e.g. `<html>` under the document).
fn first_element_child(dom: &ScriptedDom, node: NodeId) -> Option<NodeId> {
    dom.dom_children(node)
        .find(|&c| matches!(dom.kind(c), NodeKind::Element))
}

/// Install the `document`/`Node` surface: native sinks, then the JS bootstrap that
/// builds `document` and the node wrappers over them.
pub(crate) fn install_dom_surface<E: ScriptEngine>(engine: &mut E) -> Result<(), E::Error> {
    shadow::install(engine)?;
    markup_insertion::install(engine)?;
    engine.set_function::<DocumentRoot>("__documentRoot", 0)?;
    engine.set_function::<ReflectNode>("__reflectNode", 1)?;
    engine.set_function::<CreateElement>("__createElement", 1)?;
    engine.set_function::<CreateTextNode>("__createTextNode", 1)?;
    engine.set_function::<AppendChild>("__appendChild", 2)?;
    engine.set_function::<SetAttribute>("__setAttribute", 3)?;
    engine.set_function::<SetTextContent>("__setTextContent", 2)?;
    engine.set_function::<GetInnerHtml>("__getInnerHtml", 1)?;
    engine.set_function::<SetInnerHtml>("__setInnerHtml", 2)?;
    engine.set_function::<GetElementById>("__getElementById", 2)?;
    engine.set_function::<GetAttribute>("__getAttribute", 2)?;
    engine.set_function::<QualifiedName>("__qualifiedName", 1)?;
    engine.set_function::<GetTextContent>("__getTextContent", 1)?;
    engine.set_function::<CookieGet>("__cookieGet", 0)?;
    engine.set_function::<CookieSet>("__cookieSet", 1)?;
    engine.set_function::<ParentNode>("__parentNode", 1)?;
    engine.set_function::<ElementsByTagNameCount>("__elementsByTagNameCount", 2)?;
    engine.set_function::<ElementsByTagNameItem>("__elementsByTagNameItem", 3)?;
    engine.set_function::<DocumentElement>("__documentElement", 1)?;
    engine.set_function::<DocumentBody>("__documentBody", 1)?;
    engine.set_function::<DocumentHead>("__documentHead", 1)?;
    engine.set_function::<CreateDocument>("__createDocument", 0)?;
    engine.set_function::<CreateComment>("__createComment", 1)?;
    engine.set_function::<CreateFragment>("__createFragment", 0)?;
    engine.set_function::<CreateProcessingInstruction>("__createProcessingInstruction", 2)?;
    engine.set_function::<CreateCdataSection>("__createCDATASection", 1)?;
    engine.set_function::<CreateDoctype>("__createDoctype", 3)?;
    engine.set_function::<DoctypeField>("__doctypeField", 2)?;
    engine.set_function::<DocumentDoctype>("__documentDoctype", 1)?;
    engine.set_function::<GetOuterHtml>("__getOuterHtml", 1)?;
    engine.set_function::<ParseDocument>("__parseDocument", 2)?;
    engine.set_function::<AttributeRecords>("__attributeRecords", 1)?;
    engine.set_function::<SetAttributeNS>("__setAttributeNS", 4)?;
    engine.set_function::<GetAttributeNS>("__getAttributeNS", 3)?;
    engine.set_function::<RemoveAttributeNS>("__removeAttributeNS", 3)?;
    engine.set_function::<NodeType>("__nodeType", 1)?;
    engine.set_function::<NodeRawId>("__nodeRawId", 1)?;
    engine.set_function::<RemoveAttribute>("__removeAttribute", 2)?;
    engine.set_function::<Matches>("__matches", 2)?;
    engine.set_function::<QuerySelector>("__querySelector", 2)?;
    engine.set_function::<QuerySelectorAllCount>("__querySelectorAllCount", 2)?;
    engine.set_function::<QuerySelectorAllItem>("__querySelectorAllItem", 3)?;
    engine.set_function::<FirstChild>("__firstChild", 1)?;
    engine.set_function::<LastChild>("__lastChild", 1)?;
    engine.set_function::<NextSibling>("__nextSibling", 1)?;
    engine.set_function::<PrevSibling>("__prevSibling", 1)?;
    engine.set_function::<ChildNodesCount>("__childNodesCount", 1)?;
    engine.set_function::<ChildNodesItem>("__childNodesItem", 2)?;
    engine.set_function::<NodeName>("__nodeName", 1)?;
    engine.set_function::<NodeValue>("__nodeValue", 1)?;
    engine.set_function::<RemoveChild>("__removeChild", 2)?;
    engine.set_function::<InsertBefore>("__insertBefore", 3)?;
    engine.set_function::<MoveBefore>("__moveBefore", 3)?;
    engine.set_function::<LocalNameOf>("__localName", 1)?;
    engine.set_function::<NamespaceUri>("__namespaceURI", 1)?;
    engine.set_function::<PrefixOf>("__prefix", 1)?;
    engine.set_function::<CreateElementNS>("__createElementNS", 2)?;
    engine.set_function::<AttributeNames>("__attributeNames", 1)?;
    engine.set_function::<InlineStyleValue>("__inlineStyleValue", 2)?;
    engine.set_function::<InlineStyleShorthandComponents>("__inlineStyleShorthandComponents", 1)?;
    engine.set_function::<InlineStyleShorthands>("__inlineStyleShorthands", 0)?;
    engine.set_function::<InlineStyleShorthandValue>("__inlineStyleShorthandValue", 2)?;
    engine.set_function::<SupportsStyleValue>("__supportsStyleValue", 2)?;
    engine.set_function::<ComputedStyleValue>("__computedStyleValue", 2)?;
    engine.set_function::<ComputedStyleValueInContext>("__computedStyleValueInContext", 3)?;
    engine.set_function::<StyleSheetCount>("__styleSheetCount", 0)?;
    engine.set_function::<StyleSheetKey>("__styleSheetKey", 1)?;
    engine.set_function::<StyleSheetRuleCount>("__styleSheetRuleCount", 2)?;
    engine.set_function::<StyleSheetRuleKindNative>("__styleSheetRuleKind", 2)?;
    engine.set_function::<StyleSheetRuleText>("__styleSheetRuleText", 2)?;
    engine.set_function::<StyleSheetRuleSelectorText>("__styleSheetRuleSelectorText", 2)?;
    engine.set_function::<StyleSheetRuleStyleText>("__styleSheetRuleStyleText", 2)?;
    engine.set_function::<StyleSheetRuleConditionText>("__styleSheetRuleConditionText", 2)?;
    engine.set_function::<StyleSheetRuleName>("__styleSheetRuleName", 2)?;
    engine.set_function::<StyleSheetRuleKeyText>("__styleSheetRuleKeyText", 2)?;
    engine.set_function::<StyleSheetRuleChildCount>("__styleSheetRuleChildCount", 2)?;
    engine.set_function::<StyleSheetOwnerNode>("__styleSheetOwnerNode", 1)?;
    engine.set_function::<StyleSheetImportHref>("__styleSheetImportHref", 2)?;
    engine.set_function::<StyleSheetImportMedia>("__styleSheetImportMedia", 2)?;
    engine.set_function::<StyleSheetImportChildKey>("__styleSheetImportChildKey", 2)?;
    engine.set_function::<StyleSheetOwnerImportParentKey>("__styleSheetOwnerImportParentKey", 1)?;
    engine.set_function::<StyleSheetOwnerImportIndex>("__styleSheetOwnerImportIndex", 1)?;
    engine.set_function::<InsertRule>("__insertRule", 3)?;
    engine.set_function::<DeleteRule>("__deleteRule", 2)?;
    engine.set_function::<MatchMedia>("__matchMedia", 1)?;
    engine.set_function::<EvaluateXPath>("__xpathEvaluate", 2)?;
    engine.set_function::<mutation_observer::SetObserving>("__moObserving", 1)?;
    engine.set_function::<mutation_observer::TakeRecords>("__moTake", 0)?;
    engine.set_function::<mutation_observer::SetGroup>("__moGroup", 1)?;
    engine.set_function::<selection::RangeRects>("__rangeRects", 4)?;
    engine.set_function::<selection::VisualSelection>("__selectionVisual", 4)?;
    let html_interfaces = html_interfaces::bootstrap_script();
    engine.eval(&html_interfaces)?;
    engine.eval(DOM_BOOTSTRAP)?;
    Ok(())
}

/// An HTML-namespaced element name matching the retained Livery cascade key.
fn html_qual(local: &str) -> QualName {
    QualName::new(
        None,
        Namespace::from("http://www.w3.org/1999/xhtml"),
        LocalName::from(local),
    )
}

/// A null-namespace attribute name (the common case: `id`, `class`, …).
fn attr_qual(local: &str) -> QualName {
    QualName::new(None, Namespace::from(""), LocalName::from(local))
}

/// A namespaced attribute name from a namespace and a qualified name, splitting
/// `prefix:local` the way `createElementNS` does. An empty namespace is null.
fn ns_attr_qual(ns: &str, qname: &str) -> QualName {
    let (prefix, local) = match qname.split_once(':') {
        Some((p, l)) => (Some(Prefix::from(p)), l),
        None => (None, qname),
    };
    QualName::new(prefix, Namespace::from(ns), LocalName::from(local))
}

/// Run `f` against the host's [`ScriptedDom`], recovered from the engine host-data
/// slot (a `RefCell<HostState>`). `None` if no host state is set.
fn with_dom<E: ScriptEngine, R>(
    cx: &mut E::CallCx<'_>,
    f: impl FnOnce(&mut ScriptedDom) -> R,
) -> Option<R> {
    let data = cx.host_data()?;
    let cell = data.downcast_ref::<RefCell<HostState>>()?;
    let mut host = cell.borrow_mut();
    Some(f(&mut host.dom))
}

/// The host's computed-style seam for `getComputedStyle`. Mirrors
/// [`FetchHandler`](crate::FetchHandler): the runtime never links a layout
/// engine, so a host that has one (e.g. pelt's `ScriptedDocument` over
/// `IncrementalLayout`) implements this to serialize a node's **computed** value
/// for a CSS longhand. `node` is the reflector's raw id; `property` is a longhand
/// name. `None` (unstyled / unsupported / no handler) surfaces to script as `""`.
/// Install with [`Runtime::set_computed_style_handler`](crate::Runtime::set_computed_style_handler).
pub trait ComputedStyleHandler {
    fn computed_value(&self, node: u64, property: &str) -> Option<String>;

    /// Resolve a value inside a nested browsing context. `context` identifies
    /// the embedding element whose laid-out content box establishes the child
    /// viewport. Handlers without nested-document support retain the ordinary
    /// behavior.
    fn computed_value_in_context(
        &self,
        _context: u64,
        node: u64,
        property: &str,
    ) -> Option<String> {
        self.computed_value(node, property)
    }
}

/// Result of asking the selected CSS engine to normalize one inline specified
/// value. `PassThrough` is essential at this shared boundary: a bounded engine
/// cannot declare unfamiliar but valid full-web syntax invalid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InlineStyleValueResult {
    PassThrough,
    Invalid,
    Canonical(String),
    /// A recognized value whose custom-property substitution must remain
    /// deferred. It is supported, but cannot yet be expanded into longhands.
    SupportedDeferred,
    /// A canonical shorthand expansion in the CSS engine's longhand order.
    Expanded(Vec<(String, String)>),
}

/// The selected CSS engine's specified-value seam for `element.style`.
pub trait InlineStyleHandler {
    fn canonicalize(&self, property: &str, value: &str) -> InlineStyleValueResult;

    /// Ordered longhands for a recognized shorthand. The runtime uses this
    /// metadata for generic declaration-map replacement and removal.
    fn shorthand_components(&self, _property: &str) -> Vec<String> {
        Vec::new()
    }

    /// Supported shorthand names, in a stable serialization order.  This lets
    /// the shared declaration surface serialize complete component sets without
    /// knowing any engine-specific shorthand names.
    fn shorthands(&self) -> Vec<String> {
        Vec::new()
    }

    /// Reconstruct a shorthand from its complete current longhand set.
    fn reconstruct_shorthand(
        &self,
        _property: &str,
        _components: &[(String, String)],
    ) -> Option<String> {
        None
    }
}

fn host_inline_style<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
) -> Option<Rc<dyn InlineStyleHandler>> {
    let data = cx.host_data()?;
    let cell = data.downcast_ref::<RefCell<HostState>>()?;
    let handler = cell.borrow().inline_style.clone();
    handler
}

/// Escape a CSS text field for the compact line protocol shared with the JS
/// bootstrap.  Escaping keeps embedded tabs/newlines from changing record
/// boundaries without making the protocol depend on a JSON implementation.
fn escape_inline_style_field(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn unescape_inline_style_field(value: &str) -> Option<String> {
    let mut unescaped = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            unescaped.push(character);
            continue;
        }
        match characters.next()? {
            '\\' => unescaped.push('\\'),
            'n' => unescaped.push('\n'),
            'r' => unescaped.push('\r'),
            't' => unescaped.push('\t'),
            _ => return None,
        }
    }
    Some(unescaped)
}

fn encode_inline_style_components(
    components: impl IntoIterator<Item = (String, String)>,
) -> String {
    components
        .into_iter()
        .map(|(name, value)| {
            format!(
                "{}\t{}",
                escape_inline_style_field(&name),
                escape_inline_style_field(&value)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_inline_style_components(record: &str) -> Option<Vec<(String, String)>> {
    if record.is_empty() {
        return Some(Vec::new());
    }
    record
        .split('\n')
        .map(|line| {
            let (name, value) = line.split_once('\t')?;
            if value.contains('\t') {
                return None;
            }
            Some((
                unescape_inline_style_field(name)?,
                unescape_inline_style_field(value)?,
            ))
        })
        .collect()
}

/// `__inlineStyleValue(property, value)` -> an escaped line record consumed by
/// the JS CSSStyleDeclaration.
struct InlineStyleValue;
impl<E: ScriptEngine> NativeFn<E> for InlineStyleValue {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let property_value = cx.arg(0);
        let property = cx.value_to_string(&property_value)?;
        let input_value = cx.arg(1);
        let value = cx.value_to_string(&input_value)?;
        let result = host_inline_style::<E>(cx)
            .map_or(InlineStyleValueResult::PassThrough, |handler| {
                handler.canonicalize(&property, &value)
            });
        let record = match result {
            InlineStyleValueResult::PassThrough => "pass\n".to_string(),
            InlineStyleValueResult::Invalid => "invalid\n".to_string(),
            InlineStyleValueResult::Canonical(value) => {
                format!("canonical\n{}", escape_inline_style_field(&value))
            },
            InlineStyleValueResult::SupportedDeferred => "deferred\n".to_string(),
            InlineStyleValueResult::Expanded(components) => {
                format!("expanded\n{}", encode_inline_style_components(components))
            },
        };
        cx.make_string(&record)
    }
}

/// `__inlineStyleShorthandComponents(property)` -> escaped newline-separated
/// ordered longhand names, or the empty string for an unknown shorthand.
struct InlineStyleShorthandComponents;
impl<E: ScriptEngine> NativeFn<E> for InlineStyleShorthandComponents {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let property_value = cx.arg(0);
        let property = cx.value_to_string(&property_value)?;
        let components = host_inline_style::<E>(cx)
            .map(|handler| handler.shorthand_components(&property))
            .unwrap_or_default();
        cx.make_string(
            &components
                .into_iter()
                .map(|component| escape_inline_style_field(&component))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

/// `__inlineStyleShorthands()` -> escaped newline-separated shorthand names in
/// the CSS engine's stable serialization order.
struct InlineStyleShorthands;
impl<E: ScriptEngine> NativeFn<E> for InlineStyleShorthands {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let shorthands = host_inline_style::<E>(cx)
            .map(|handler| handler.shorthands())
            .unwrap_or_default();
        cx.make_string(
            &shorthands
                .into_iter()
                .map(|shorthand| escape_inline_style_field(&shorthand))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

/// `__inlineStyleShorthandValue(property, components)` -> the reconstructed
/// shorthand, or the empty string. Components use the escaped `name\tvalue`
/// line record used by shorthand expansion.
struct InlineStyleShorthandValue;
impl<E: ScriptEngine> NativeFn<E> for InlineStyleShorthandValue {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let property_value = cx.arg(0);
        let property = cx.value_to_string(&property_value)?;
        let record_value = cx.arg(1);
        let record = cx.value_to_string(&record_value)?;
        let value = decode_inline_style_components(&record)
            .and_then(|components| {
                host_inline_style::<E>(cx)
                    .and_then(|handler| handler.reconstruct_shorthand(&property, &components))
            })
            .unwrap_or_default();
        cx.make_string(&escape_inline_style_field(&value))
    }
}

/// `__supportsStyleValue(property, value)` -> a textual boolean. Native call
/// contexts expose strings but not a boolean constructor, so the bootstrap
/// performs the final conversion.
struct SupportsStyleValue;
impl<E: ScriptEngine> NativeFn<E> for SupportsStyleValue {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let property_value = cx.arg(0);
        let property = cx.value_to_string(&property_value)?;
        let input_value = cx.arg(1);
        let value = cx.value_to_string(&input_value)?;
        let supported = property.starts_with("--")
            || host_inline_style::<E>(cx).is_some_and(|handler| {
                matches!(
                    handler.canonicalize(&property, &value),
                    InlineStyleValueResult::Canonical(_)
                        | InlineStyleValueResult::SupportedDeferred
                        | InlineStyleValueResult::Expanded(_)
                )
            });
        cx.make_string(if supported { "true" } else { "false" })
    }
}

/// Clone the computed-style handler out of host state (so it is not borrowed
/// while invoked).
fn host_computed_style<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
) -> Option<Rc<dyn ComputedStyleHandler>> {
    let data = cx.host_data()?;
    let cell = data.downcast_ref::<RefCell<HostState>>()?;
    let handler = cell.borrow().computed_style.clone();
    handler
}

/// `__computedStyleValue(nodeRef, property)` -> the host's serialized computed
/// value for the longhand, or `null` when there is no value / no handler.
struct ComputedStyleValue;
impl<E: ScriptEngine> NativeFn<E> for ComputedStyleValue {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let el = cx.arg(0);
        let Some(node) = cx.reflector_data(&el) else {
            return Ok(cx.make_null());
        };
        let a1 = cx.arg(1);
        let property = cx.value_to_string(&a1)?;
        let value = host_computed_style::<E>(cx).and_then(|h| h.computed_value(node, &property));
        match value {
            Some(v) => cx.make_string(&v),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__computedStyleValueInContext(contextRef, nodeRef, property)` is the
/// nested-browsing-context counterpart of [`ComputedStyleValue`].
struct ComputedStyleValueInContext;
impl<E: ScriptEngine> NativeFn<E> for ComputedStyleValueInContext {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let context_value = cx.arg(0);
        let node_value = cx.arg(1);
        let (Some(context), Some(node)) = (
            cx.reflector_data(&context_value),
            cx.reflector_data(&node_value),
        ) else {
            return Ok(cx.make_null());
        };
        let property_value = cx.arg(2);
        let property = cx.value_to_string(&property_value)?;
        let value = host_computed_style::<E>(cx)
            .and_then(|handler| handler.computed_value_in_context(context, node, &property));
        match value {
            Some(value) => cx.make_string(&value),
            None => Ok(cx.make_null()),
        }
    }
}

/// A stylesheet mutation failure translated into its CSSOM exception class.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StyleSheetMutationError {
    IndexSize,
    Syntax(String),
}

/// CSSOM metadata for a leading `@import` rule supplied by the selected style
/// engine. The runtime retains only this browser-facing projection; resource
/// discovery and child loading remain engine-neutral.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyleSheetImportRule {
    pub href: String,
    pub media: Option<String>,
    pub child_sheet_key: Option<String>,
}

/// The parent import rule for an imported CSSOM stylesheet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyleSheetImportOwner {
    pub parent_sheet_key: String,
    pub import_index: usize,
}

/// Every CSS rule kind the selected style engine can project today. The names
/// deliberately stay engine-neutral; the JavaScript bootstrap maps them onto
/// the browser-facing constructors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StyleSheetRuleKind {
    Style,
    Import,
    FontFace,
    Media,
    Container,
    Keyframes,
    Keyframe,
}

/// A read-only CSS rule object supplied by the selected style engine. Rule
/// mutations remain rooted at `CSSStyleSheet` until nested-group mutation owns
/// a parser and resource-graph transaction of its own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyleSheetRule {
    pub kind: StyleSheetRuleKind,
    pub css_text: String,
    pub selector_text: Option<String>,
    pub style_text: Option<String>,
    pub condition_text: Option<String>,
    pub name: Option<String>,
    pub key_text: Option<String>,
    pub child_count: usize,
}

/// The host's retained author-stylesheet seam. The runtime owns the JS CSSOM
/// wrappers, while the selected CSS engine owns rule parsing, mutation, and
/// generation tracking behind this object-safe contract.
pub trait StyleSheetHandler {
    fn sheet_count(&self) -> usize;
    fn rule_count(&self, sheet: usize) -> Option<usize>;
    fn insert_rule(
        &self,
        sheet: usize,
        rule: &str,
        index: usize,
    ) -> Result<usize, StyleSheetMutationError>;
    fn delete_rule(&self, sheet: usize, index: usize) -> Result<(), StyleSheetMutationError>;

    /// Stable opaque identity for a document-list sheet. The default preserves
    /// the earlier positional contract for fixed sheet lists; live hosts
    /// override it with the owning DOM element's identity.
    fn sheet_key(&self, sheet: usize) -> Option<String> {
        (sheet < self.sheet_count()).then(|| sheet.to_string())
    }

    /// Look up a CSSOM sheet through its stable key.
    fn rule_count_by_key(&self, key: &str) -> Option<usize> {
        key.parse().ok().and_then(|sheet| self.rule_count(sheet))
    }

    /// Mutate a CSSOM sheet through its stable key.
    fn insert_rule_by_key(
        &self,
        key: &str,
        rule: &str,
        index: usize,
    ) -> Result<usize, StyleSheetMutationError> {
        key.parse()
            .map_or(Err(StyleSheetMutationError::IndexSize), |sheet| {
                self.insert_rule(sheet, rule, index)
            })
    }

    /// Remove a CSSOM rule through its stable sheet key.
    fn delete_rule_by_key(&self, key: &str, index: usize) -> Result<(), StyleSheetMutationError> {
        key.parse()
            .map_or(Err(StyleSheetMutationError::IndexSize), |sheet| {
                self.delete_rule(sheet, index)
            })
    }

    /// The direct `<style>` / `<link>` owner for a document-list sheet.
    /// `None` is correct for host-supplied anonymous sheets and stale wrappers.
    fn owner_node_by_key(&self, _key: &str) -> Option<u64> {
        None
    }

    /// A leading import rule in this sheet's CSSOM rule list.
    fn import_rule_by_key(&self, _key: &str, _index: usize) -> Option<StyleSheetImportRule> {
        None
    }

    /// The import rule which owns an imported child sheet.
    fn import_owner_by_key(&self, _key: &str) -> Option<StyleSheetImportOwner> {
        None
    }

    /// Look up one root-to-child rule path. The compatibility default still
    /// exposes a leading import for older hosts that only implemented the
    /// R5c import callback; full engines override it for every supported rule.
    fn rule_by_key(&self, key: &str, path: &[usize]) -> Option<StyleSheetRule> {
        let [index] = path else {
            return None;
        };
        self.import_rule_by_key(key, *index).map(|import| {
            let media = import.media.filter(|media| !media.trim().is_empty());
            let escaped_href = import.href.replace('"', "\\\"");
            StyleSheetRule {
                kind: StyleSheetRuleKind::Import,
                css_text: format!(
                    "@import url(\"{escaped_href}\"){};",
                    media
                        .as_deref()
                        .map_or_else(String::new, |media| format!(" {media}"))
                ),
                selector_text: None,
                style_text: None,
                condition_text: media,
                name: None,
                key_text: None,
                child_count: 0,
            }
        })
    }
}

/// Clone the stylesheet handler out of host state before invoking it.
fn host_stylesheets<E: ScriptEngine>(cx: &mut E::CallCx<'_>) -> Option<Rc<dyn StyleSheetHandler>> {
    let data = cx.host_data()?;
    let cell = data.downcast_ref::<RefCell<HostState>>()?;
    let handler = cell.borrow().stylesheets.clone();
    handler
}

fn index_arg<E: ScriptEngine>(cx: &mut E::CallCx<'_>, argument: usize) -> Option<usize> {
    let value = cx.arg(argument);
    cx.value_to_string(&value).ok()?.parse().ok()
}

fn sheet_key_arg<E: ScriptEngine>(cx: &mut E::CallCx<'_>, argument: usize) -> Option<String> {
    let value = cx.arg(argument);
    cx.value_to_string(&value).ok()
}

fn rule_path_arg<E: ScriptEngine>(cx: &mut E::CallCx<'_>, argument: usize) -> Option<Vec<usize>> {
    let value = cx.arg(argument);
    let value = cx.value_to_string(&value).ok()?;
    if value.is_empty() {
        return Some(Vec::new());
    }
    value
        .split('/')
        .map(str::parse)
        .collect::<Result<Vec<_>, _>>()
        .ok()
}

fn stylesheet_rule_arg<E: ScriptEngine>(cx: &mut E::CallCx<'_>) -> Option<StyleSheetRule> {
    let key = sheet_key_arg::<E>(cx, 0)?;
    let path = rule_path_arg::<E>(cx, 1)?;
    host_stylesheets::<E>(cx)?.rule_by_key(&key, &path)
}

fn mutation_record(result: Result<usize, StyleSheetMutationError>) -> String {
    match result {
        Ok(index) => format!("ok\n{index}"),
        Err(StyleSheetMutationError::IndexSize) => "index\nrule index out of range".to_string(),
        Err(StyleSheetMutationError::Syntax(message)) => format!("syntax\n{message}"),
    }
}

/// `__styleSheetCount()` -> retained author-sheet count.
struct StyleSheetCount;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetCount {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let count = host_stylesheets::<E>(cx).map_or(0, |handler| handler.sheet_count());
        cx.make_string(&count.to_string())
    }
}

/// `__styleSheetKey(index)` -> stable key for the live list slot, or `-1`.
struct StyleSheetKey;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetKey {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let key = index_arg::<E>(cx, 0)
            .and_then(|sheet| host_stylesheets::<E>(cx)?.sheet_key(sheet))
            .map_or_else(|| "-1".to_string(), |key| key.to_string());
        cx.make_string(&key)
    }
}

/// `__styleSheetRuleCount(sheet, path)` -> the list length at `path`, or `-1`
/// for a stale sheet/rule wrapper. An empty path addresses the sheet root.
struct StyleSheetRuleCount;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetRuleCount {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let count = match (sheet_key_arg::<E>(cx, 0), rule_path_arg::<E>(cx, 1)) {
            (Some(key), Some(path)) if path.is_empty() => {
                host_stylesheets::<E>(cx).and_then(|handler| handler.rule_count_by_key(&key))
            },
            (Some(key), Some(path)) => host_stylesheets::<E>(cx)
                .and_then(|handler| handler.rule_by_key(&key, &path))
                .map(|rule| rule.child_count),
            _ => None,
        }
        .map_or_else(|| "-1".to_string(), |count| count.to_string());
        cx.make_string(&count)
    }
}

fn rule_kind_name(kind: StyleSheetRuleKind) -> &'static str {
    match kind {
        StyleSheetRuleKind::Style => "style",
        StyleSheetRuleKind::Import => "import",
        StyleSheetRuleKind::FontFace => "font-face",
        StyleSheetRuleKind::Media => "media",
        StyleSheetRuleKind::Container => "container",
        StyleSheetRuleKind::Keyframes => "keyframes",
        StyleSheetRuleKind::Keyframe => "keyframe",
    }
}

/// `__styleSheetRuleKind(sheet, path)` -> one engine-neutral kind name, or
/// `null` for an invalid/stale rule path.
struct StyleSheetRuleKindNative;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetRuleKindNative {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        match stylesheet_rule_arg::<E>(cx) {
            Some(rule) => cx.make_string(rule_kind_name(rule.kind)),
            None => Ok(cx.make_null()),
        }
    }
}

struct StyleSheetRuleText;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetRuleText {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        match stylesheet_rule_arg::<E>(cx) {
            Some(rule) => cx.make_string(&rule.css_text),
            None => Ok(cx.make_null()),
        }
    }
}

fn optional_rule_string<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    value: Option<String>,
) -> Result<E::Value, E::Error> {
    match value {
        Some(value) => cx.make_string(&value),
        None => Ok(cx.make_null()),
    }
}

struct StyleSheetRuleSelectorText;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetRuleSelectorText {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let value = stylesheet_rule_arg::<E>(cx).and_then(|rule| rule.selector_text);
        optional_rule_string::<E>(cx, value)
    }
}

struct StyleSheetRuleStyleText;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetRuleStyleText {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let value = stylesheet_rule_arg::<E>(cx).and_then(|rule| rule.style_text);
        optional_rule_string::<E>(cx, value)
    }
}

struct StyleSheetRuleConditionText;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetRuleConditionText {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let value = stylesheet_rule_arg::<E>(cx).and_then(|rule| rule.condition_text);
        optional_rule_string::<E>(cx, value)
    }
}

struct StyleSheetRuleName;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetRuleName {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let value = stylesheet_rule_arg::<E>(cx).and_then(|rule| rule.name);
        optional_rule_string::<E>(cx, value)
    }
}

struct StyleSheetRuleKeyText;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetRuleKeyText {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let value = stylesheet_rule_arg::<E>(cx).and_then(|rule| rule.key_text);
        optional_rule_string::<E>(cx, value)
    }
}

struct StyleSheetRuleChildCount;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetRuleChildCount {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let count = stylesheet_rule_arg::<E>(cx)
            .map(|rule| rule.child_count.to_string())
            .unwrap_or_else(|| "-1".to_owned());
        cx.make_string(&count)
    }
}

/// `__styleSheetOwnerNode(key)` -> the owner element's canonical reflector, or
/// `null` for anonymous / stale sheets.
struct StyleSheetOwnerNode;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetOwnerNode {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let owner = sheet_key_arg::<E>(cx, 0)
            .and_then(|key| host_stylesheets::<E>(cx)?.owner_node_by_key(&key));
        match owner {
            Some(owner) => reflect_pinned::<E>(cx, owner),
            None => Ok(cx.make_null()),
        }
    }
}

fn import_rule_arg<E: ScriptEngine>(cx: &mut E::CallCx<'_>) -> Option<StyleSheetImportRule> {
    let key = sheet_key_arg::<E>(cx, 0)?;
    let index = index_arg::<E>(cx, 1)?;
    host_stylesheets::<E>(cx)?.import_rule_by_key(&key, index)
}

/// `__styleSheetImportHref(parent, index)` -> the authored import URL, or
/// `null` when that CSSOM rule is not an import.
struct StyleSheetImportHref;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetImportHref {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        match import_rule_arg::<E>(cx) {
            Some(rule) => cx.make_string(&rule.href),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__styleSheetImportMedia(parent, index)` -> the import media text, or
/// `null` when that CSSOM rule is not an import.
struct StyleSheetImportMedia;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetImportMedia {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        match import_rule_arg::<E>(cx) {
            Some(rule) => cx.make_string(rule.media.as_deref().unwrap_or("")),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__styleSheetImportChildKey(parent, index)` -> the imported child sheet key,
/// or `null` when the rule has no loaded child.
struct StyleSheetImportChildKey;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetImportChildKey {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        match import_rule_arg::<E>(cx).and_then(|rule| rule.child_sheet_key) {
            Some(key) => cx.make_string(&key),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__styleSheetOwnerImportParentKey(child)` -> the parent sheet key for an
/// imported child, or `null` for direct and stale sheets.
struct StyleSheetOwnerImportParentKey;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetOwnerImportParentKey {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let owner = sheet_key_arg::<E>(cx, 0)
            .and_then(|key| host_stylesheets::<E>(cx)?.import_owner_by_key(&key));
        match owner {
            Some(owner) => cx.make_string(&owner.parent_sheet_key),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__styleSheetOwnerImportIndex(child)` -> the parent import-rule index, or
/// `-1` for direct and stale sheets.
struct StyleSheetOwnerImportIndex;
impl<E: ScriptEngine> NativeFn<E> for StyleSheetOwnerImportIndex {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let index = sheet_key_arg::<E>(cx, 0)
            .and_then(|key| host_stylesheets::<E>(cx)?.import_owner_by_key(&key))
            .map_or_else(|| "-1".to_owned(), |owner| owner.import_index.to_string());
        cx.make_string(&index)
    }
}

/// `__insertRule(sheet, rule, index)` -> a line record consumed by the JS
/// wrapper: `ok`, `index`, or `syntax` plus its result/message.
struct InsertRule;
impl<E: ScriptEngine> NativeFn<E> for InsertRule {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let sheet = sheet_key_arg::<E>(cx, 0);
        let rule_value = cx.arg(1);
        let rule = cx.value_to_string(&rule_value)?;
        let index = index_arg::<E>(cx, 2);
        let result = match (host_stylesheets::<E>(cx), sheet, index) {
            (Some(handler), Some(sheet), Some(index)) => {
                handler.insert_rule_by_key(&sheet, &rule, index)
            },
            _ => Err(StyleSheetMutationError::IndexSize),
        };
        cx.make_string(&mutation_record(result))
    }
}

/// `__deleteRule(sheet, index)` -> the same mutation record as insertRule.
struct DeleteRule;
impl<E: ScriptEngine> NativeFn<E> for DeleteRule {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let sheet = sheet_key_arg::<E>(cx, 0);
        let index = index_arg::<E>(cx, 1);
        let result = match (host_stylesheets::<E>(cx), sheet, index) {
            (Some(handler), Some(sheet), Some(index)) => {
                handler.delete_rule_by_key(&sheet, index).map(|()| index)
            },
            _ => Err(StyleSheetMutationError::IndexSize),
        };
        cx.make_string(&mutation_record(result))
    }
}

/// The host's media-query seam for `window.matchMedia`. Mirrors
/// [`ComputedStyleHandler`]: the runtime links no layout engine, so a host with
/// one (evaluating against its `IncrementalLayout` device) implements this to
/// parse + evaluate a media query string. Returns the *serialized* (normalized)
/// query and whether it currently matches. Install with
/// [`Runtime::set_media_query_handler`](crate::Runtime::set_media_query_handler).
pub trait MediaQueryHandler {
    fn evaluate(&self, query: &str) -> (String, bool);
}

/// Clone the media-query handler out of host state (not borrowed while invoked).
fn host_media_query<E: ScriptEngine>(cx: &mut E::CallCx<'_>) -> Option<Rc<dyn MediaQueryHandler>> {
    let data = cx.host_data()?;
    let cell = data.downcast_ref::<RefCell<HostState>>()?;
    let handler = cell.borrow().media_query.clone();
    handler
}

/// `__matchMedia(query)` -> `"<0|1>\n<serialized media>"` (matches flag + the
/// normalized query), which the `matchMedia` shim splits into a MediaQueryList.
/// With no handler bound, returns `"0\n<raw query>"`.
struct MatchMedia;
impl<E: ScriptEngine> NativeFn<E> for MatchMedia {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let a0 = cx.arg(0);
        let query = cx.value_to_string(&a0)?;
        let (media, matches) = match host_media_query::<E>(cx) {
            Some(h) => h.evaluate(&query),
            None => (query.clone(), false),
        };
        cx.make_string(&format!("{}\n{}", u8::from(matches), media))
    }
}

/// The host's cookie store for `document.cookie` (e.g. meerkat's view over the
/// netfetcher session jar). The runtime owns no networking, so the host supplies the
/// document's cookies. [`get_cookies`](Self::get_cookies) returns the *script-visible*
/// cookies for the current document — HttpOnly excluded, the host's job — as a
/// `"n1=v1; n2=v2"` string; [`set_cookie`](Self::set_cookie) takes one
/// `Set-Cookie`-style assignment (`"name=value; Path=/; ..."`). `None` = no store, so
/// `document.cookie` reads `""` and a write is a no-op. Install with
/// [`Runtime::set_cookie_provider`](crate::Runtime::set_cookie_provider).
pub trait CookieProvider {
    fn get_cookies(&self) -> String;
    fn set_cookie(&self, cookie: &str);
}

/// Clone the cookie provider out of host state (so it is not borrowed while invoked).
fn host_cookies<E: ScriptEngine>(cx: &mut E::CallCx<'_>) -> Option<Rc<dyn CookieProvider>> {
    let data = cx.host_data()?;
    let cell = data.downcast_ref::<RefCell<HostState>>()?;
    let provider = cell.borrow().cookies.clone();
    provider
}

/// `__cookieGet()` -> the document's script-visible cookies as a header string
/// (`""` when there is no provider).
struct CookieGet;
impl<E: ScriptEngine> NativeFn<E> for CookieGet {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let cookies = host_cookies::<E>(cx)
            .map(|h| h.get_cookies())
            .unwrap_or_default();
        cx.make_string(&cookies)
    }
}

/// `__cookieSet(value)` -> record one `Set-Cookie`-style assignment (no-op without a
/// provider).
struct CookieSet;
impl<E: ScriptEngine> NativeFn<E> for CookieSet {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let a0 = cx.arg(0);
        let cookie = cx.value_to_string(&a0)?;
        if let Some(provider) = host_cookies::<E>(cx) {
            provider.set_cookie(&cookie);
        }
        Ok(cx.undefined())
    }
}

/// Hand script the **canonical** reflector for the node with raw id `raw`, and
/// **pin** that node (G1/G3). Every place a binding returns a node to script
/// routes through here: the node is now script-reachable, so the host pins it
/// until [`Runtime::collect_garbage`](crate::Runtime::collect_garbage) sees the
/// engine report the reflector dead. Pinning must be complete — a node handed
/// out unpinned could be swept while script still holds it — which is why this
/// is the sole node-handoff path (no binding calls `reflector_for` directly).
/// Take an engine root on every already-reflected node in the subtree rooted at
/// `node`, if that subtree is now connected to the document.
///
/// The mint path roots a node that is *already* connected when script is handed
/// it, but the common shape is the other order: create detached, decorate, then
/// insert. Between the insertion and the next GC tick, Boa's allocation-threshold
/// collector can fire, and without this the just-connected wrapper - carrying the
/// listeners and the custom-element upgrade state script just put on it - would
/// be collectable in that window. The host pin table is the list of nodes that
/// have a reflector, so the walk roots exactly those and skips freshly parsed
/// subtrees entirely.
pub(crate) fn root_connected_subtree<E: ScriptEngine>(cx: &mut E::CallCx<'_>, node: NodeId) {
    let Some(data) = cx.host_data() else { return };
    let Some(cell) = data.downcast_ref::<RefCell<HostState>>() else {
        return;
    };
    let ids: Vec<u64> = {
        let mut host = cell.borrow_mut();
        if !host.is_connected_node(node) {
            return;
        }
        let mut out = Vec::new();
        let mut stack = vec![node];
        while let Some(id) = stack.pop() {
            if host.pins.is_pinned(id) {
                out.push(id.raw() as u64);
            }
            let kids: Vec<NodeId> = host.dom.dom_children(id).collect();
            stack.extend(kids);
            // A shadow tree hangs off its host rather than sitting among its
            // children, so its wrappers need the host's rooting explicitly.
            if let Some(root) = host.dom.shadow_root_of(id) {
                stack.push(root);
            }
        }
        out
    };
    for d in ids {
        cx.root_reflector(d);
    }
}

fn reflect_pinned<E: ScriptEngine>(cx: &mut E::CallCx<'_>, raw: u64) -> Result<E::Value, E::Error> {
    let mut connected = false;
    if let Some(data) = cx.host_data() {
        if let Some(cell) = data.downcast_ref::<RefCell<HostState>>() {
            let id = NodeId::from_raw(raw as usize);
            let mut host = cell.borrow_mut();
            host.pins.pin(id);
            connected = host.is_connected_node(id);
        }
    }
    let value = cx.reflector_for(raw)?;
    if connected {
        // Root on the mint, not only at the next GC tick: Boa collects on its own
        // allocation threshold, so a wrapper handed out and given listeners
        // between two ticks would otherwise be collectable in that window. The
        // tick releases the root again if the node has since left the document.
        cx.root_reflector(raw);
    }
    Ok(value)
}

/// Depth-first search under `root` for the first element whose null-namespace `id`
/// equals `target`. `root` lets queries scope to a created document, not just the
/// primary one.
fn find_by_id(dom: &ScriptedDom, root: NodeId, target: &str) -> Option<NodeId> {
    fn walk(dom: &ScriptedDom, node: NodeId, target: &str) -> Option<NodeId> {
        if dom.attribute(node, &Namespace::from(""), &LocalName::from("id")) == Some(target) {
            return Some(node);
        }
        for child in dom.dom_children(node).collect::<Vec<_>>() {
            if let Some(found) = walk(dom, child, target) {
                return Some(found);
            }
        }
        None
    }
    walk(dom, root, target)
}

mod html_interfaces;
mod html_interfaces_generated;
pub(crate) mod markup_insertion;
mod mutation_observer;
mod query_traverse;
mod selection;
mod shadow;
pub use selection::SelectionHandler;
mod tree;
mod xpath_eval;

#[cfg(test)]
mod tests;

use query_traverse::*;
use tree::*;
use xpath_eval::*;

/// `document` plus node wrappers. A wrapper is a plain object carrying its reflector
/// (`__ref`) and the methods that drive the native sinks. ES5-style (no arrows /
/// classes / let) for the widest backend coverage, matching the other bootstraps.
const DOM_BOOTSTRAP: &str = include_str!("bootstrap.js");
