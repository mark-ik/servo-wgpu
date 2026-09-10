// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Window's view of child browsing contexts, and `postMessage`'s origin
//! check (the iframes plan, phase four).
//!
//! Every body runs against both backends. That matters more here than usual:
//! `window.length` and `window[i]` are properties **of the global object**, and
//! Boa's globals are writable while Nova's are not, so a shape that installs
//! cleanly on one can silently do nothing on the other.

use script_engine_api::ScriptEngine;
use script_runtime_api::Runtime;

fn read<E: ScriptEngine>(rt: &mut Runtime<E>, expr: &str) -> String {
    let value = rt.eval(expr).expect("eval");
    rt.value_to_string(&value).expect("stringify")
}

/// Load a document and refresh the window's document-derived properties, the
/// way the host does after a parse.
fn load<E: ScriptEngine>(rt: &mut Runtime<E>, html: &str) {
    let dom = genet_scripted_dom::ScriptedDom::from_serialized_document(html);
    rt.load_dom(&dom);
    rt.eval("__refreshNamedProperties();").expect("refresh");
}

/// `window.frames` is the window itself, and `window.length` counts the child
/// browsing contexts rather than reading `undefined`.
fn frames_is_the_window_and_length_counts_children<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    assert_eq!(read(&mut rt, "String(frames === window)"), "true");
    assert_eq!(read(&mut rt, "String(window.length)"), "0");
    load(&mut rt, "<body><iframe></iframe><iframe></iframe></body>");
    assert_eq!(read(&mut rt, "String(window.length)"), "2");
    assert_eq!(read(&mut rt, "String(frames.length)"), "2");
}

/// A frame nested inside another element still counts: the tree is walked, not
/// the body's direct children.
fn a_nested_frame_still_counts<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    load(
        &mut rt,
        "<body><div><section><iframe></iframe></section></div></body>",
    );
    assert_eq!(read(&mut rt, "String(window.length)"), "1");
}

/// A `<template>`'s contents are unreachable from the document by
/// construction, so a frame inside one has no browsing context and must not be
/// counted. This is the encapsulation-by-shape rule paying off: nothing here
/// checks for a template.
fn a_frame_inside_a_template_has_no_browsing_context<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    load(
        &mut rt,
        "<body><iframe></iframe><template><iframe></iframe></template></body>",
    );
    assert_eq!(read(&mut rt, "String(window.length)"), "1");
}

/// `window[i]` reaches the i-th child, and an index past the end is
/// `undefined` rather than an error.
fn indexed_access_reaches_the_child_contexts<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    load(
        &mut rt,
        "<body><iframe id=a></iframe><iframe id=b></iframe></body>",
    );
    assert_eq!(read(&mut rt, "typeof window[0]"), "object");
    assert_eq!(read(&mut rt, "typeof window[1]"), "object");
    assert_eq!(read(&mut rt, "typeof window[2]"), "undefined");
    assert_eq!(
        read(
            &mut rt,
            "String(window[0] === document.getElementById('a').contentWindow)"
        ),
        "true"
    );
    assert_eq!(read(&mut rt, "String(window[0] === window[1])"), "false");
}

/// A frame removed from the document stops answering at its old index. The
/// refresh has to *take back* what it installed, or a detached frame outlives
/// its browsing context.
fn a_removed_frame_stops_answering_at_its_index<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    load(
        &mut rt,
        "<body><iframe id=a></iframe><iframe id=b></iframe></body>",
    );
    assert_eq!(read(&mut rt, "String(window.length)"), "2");
    rt.eval(
        "var b = document.getElementById('b'); b.parentNode.removeChild(b); \
         __refreshNamedProperties();",
    )
    .expect("remove");
    assert_eq!(read(&mut rt, "String(window.length)"), "1");
    assert_eq!(read(&mut rt, "typeof window[1]"), "undefined");
}

/// `window.length` is `[Replaceable]`: assigning to it shadows rather than
/// failing silently on a getter-only accessor, which is the shape both
/// backends have to accept.
fn window_length_is_replaceable<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    load(&mut rt, "<body><iframe></iframe></body>");
    assert_eq!(read(&mut rt, "String(window.length)"), "1");
    rt.eval("window.length = 'replaced';").expect("assign");
    assert_eq!(read(&mut rt, "String(window.length)"), "replaced");
}

/// A top-level browsing context has no container element.
fn a_top_level_context_has_no_frame_element<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    assert_eq!(read(&mut rt, "String(window.frameElement)"), "null");
    assert_eq!(read(&mut rt, "String('frameElement' in window)"), "true");
}

/// `postMessage`'s `targetOrigin` is enforced: a mismatched explicit origin
/// discards the message silently, and `'*'` and `'/'` always deliver.
fn post_message_enforces_its_target_origin<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_base_url("https://example.com/page.html")
        .expect("base url");
    rt.eval("var seen = []; addEventListener('message', function(e){ seen.push(e.data); });")
        .expect("listen");
    rt.eval(
        "postMessage('star', '*'); \
         postMessage('slash', '/'); \
         postMessage('exact', 'https://example.com'); \
         postMessage('with-path', 'https://example.com/ignored'); \
         postMessage('other', 'https://other.example'); \
         postMessage('other-port', 'https://example.com:8443');",
    )
    .expect("post");
    rt.run_event_loop(20).expect("drain");
    assert_eq!(
        read(&mut rt, "seen.join(',')"),
        "star,slash,exact,with-path",
        "only the matching target origins are delivered"
    );
}

/// A syntactically invalid `targetOrigin` still throws, and a payload that
/// cannot be cloned still throws even when the origin would discard it: the
/// clone happens before the origin check.
fn post_message_still_validates_before_it_discards<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_base_url("https://example.com/page.html")
        .expect("base url");
    assert_eq!(
        read(
            &mut rt,
            "(function(){ try { postMessage('x', 'not a url'); } catch (e) { return e.name; } \
               return 'none'; })()"
        ),
        "SyntaxError"
    );
    assert_eq!(
        read(
            &mut rt,
            "(function(){ try { postMessage(function(){}, 'https://other.example'); } \
               catch (e) { return e.name; } return 'none'; })()"
        ),
        "DataCloneError"
    );
}

/// A document with an opaque origin never matches an explicit target origin,
/// which is the specification's behaviour rather than a gap.
fn an_opaque_origin_matches_no_explicit_target<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.eval("var seen = []; addEventListener('message', function(e){ seen.push(e.data); });")
        .expect("listen");
    rt.eval("postMessage('star', '*'); postMessage('explicit', 'https://example.com');")
        .expect("post");
    rt.run_event_loop(20).expect("drain");
    assert_eq!(read(&mut rt, "seen.join(',')"), "star");
}

fn parent_is_replaceable_and_top_is_unforgeable<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    load(&mut rt, "<body><iframe id=child></iframe></body>");
    assert_eq!(
        read(
            &mut rt,
            r#"(function () {
        var descriptor = Object.getOwnPropertyDescriptor(window, 'parent');
        var topDescriptor = Object.getOwnPropertyDescriptor(window, 'top');
        var childWindow = document.getElementById('child').contentWindow;
        if (parent !== window || top !== window || childWindow.parent !== window ||
            childWindow.top !== window) return 'initial relations';
        if (!descriptor.enumerable || !descriptor.configurable ||
            typeof descriptor.get !== 'function' || typeof descriptor.set !== 'function')
            return 'parent accessor';
        if (!topDescriptor.enumerable || topDescriptor.configurable ||
            typeof topDescriptor.get !== 'function' || topDescriptor.set !== undefined)
            return 'top accessor';
        window.parent = 17;
        childWindow.parent = 23;
        descriptor = Object.getOwnPropertyDescriptor(window, 'parent');
        if (parent !== 17 || childWindow.parent !== 23 || !descriptor.writable ||
            !descriptor.enumerable || !descriptor.configurable || descriptor.value !== 17)
            return 'replacement';
        if (top !== window || childWindow.top !== window) return 'topology changed';
        if (Reflect.set(window, 'top', 31) || Reflect.deleteProperty(window, 'top') ||
            Reflect.defineProperty(window, 'top', { value: 31 })) return 'top mutated';
        try { (function () { 'use strict'; window.top = 31; })(); return 'strict write'; }
        catch (error) { if (!(error instanceof TypeError)) return 'wrong error'; }
        return 'ok';
    })()"#
        ),
        "ok"
    );
}

macro_rules! both_engines {
    ($($body:ident => ($boa:ident, $nova:ident)),* $(,)?) => {
        $(
            #[test]
            fn $boa() { $body::<script_engine_boa::BoaEngine>(); }

            #[cfg(not(target_arch = "wasm32"))]
            #[test]
            fn $nova() { $body::<script_engine_nova::NovaEngine>(); }
        )*
    };
}

both_engines! {
    parent_is_replaceable_and_top_is_unforgeable
        => (parent_and_top_descriptors_on_boa, parent_and_top_descriptors_on_nova),
    frames_is_the_window_and_length_counts_children
        => (frames_and_length_on_boa, frames_and_length_on_nova),
    a_nested_frame_still_counts
        => (nested_frame_counts_on_boa, nested_frame_counts_on_nova),
    a_frame_inside_a_template_has_no_browsing_context
        => (templated_frame_on_boa, templated_frame_on_nova),
    indexed_access_reaches_the_child_contexts
        => (indexed_access_on_boa, indexed_access_on_nova),
    a_removed_frame_stops_answering_at_its_index
        => (removed_frame_on_boa, removed_frame_on_nova),
    window_length_is_replaceable
        => (length_replaceable_on_boa, length_replaceable_on_nova),
    a_top_level_context_has_no_frame_element
        => (frame_element_on_boa, frame_element_on_nova),
    post_message_enforces_its_target_origin
        => (target_origin_on_boa, target_origin_on_nova),
    post_message_still_validates_before_it_discards
        => (post_message_validation_on_boa, post_message_validation_on_nova),
    an_opaque_origin_matches_no_explicit_target
        => (opaque_origin_on_boa, opaque_origin_on_nova),
}
