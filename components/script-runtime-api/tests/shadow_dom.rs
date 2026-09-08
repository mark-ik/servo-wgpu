// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Shadow DOM over the scripted arena: `attachShadow` and the allowed-host
//! rule, open vs closed visibility, named and manual slot assignment,
//! `slotchange`, the flat tree `assignedNodes({flatten})` reports, event
//! retargeting and `composedPath()`, `getRootNode({composed})`, the declarative
//! post-parse pass, `<template>.content` in its shared inert document, and the
//! `getHTML` round trip. Each body runs against both backends.

use genet_static_dom::StaticDocument;
use script_engine_api::ScriptEngine;
use script_runtime_api::Runtime;

fn read<E: ScriptEngine>(rt: &mut Runtime<E>, expr: &str) -> String {
    let v = rt.eval(expr).expect("eval");
    rt.value_to_string(&v).expect("stringify")
}

fn runtime<E: ScriptEngine>(html: &str) -> Runtime<E> {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.load_dom(&StaticDocument::parse(html));
    rt
}

const HOST_PAGE: &str = "<html><body><div id='host'><span id='a' slot='s'>one</span>\
     <span id='b'>two</span></div></body></html>";

/// The interface, the allowed-elements rule, and open vs closed visibility.
fn attach_shadow_and_modes<E: ScriptEngine>() {
    let mut rt = runtime::<E>(HOST_PAGE);
    assert_eq!(read(&mut rt, "typeof ShadowRoot"), "function");
    rt.eval(
        "function thrown(fn){ try { fn(); return 'no-throw'; } catch(e){ return e.name; } }\
         var host = document.getElementById('host');",
    )
    .expect("setup");

    // mode is required and must be one of the two enumeration values.
    assert_eq!(
        read(&mut rt, "thrown(function(){ host.attachShadow({}); })"),
        "TypeError"
    );
    assert_eq!(
        read(
            &mut rt,
            "thrown(function(){ host.attachShadow({ mode: 'ajar' }); })"
        ),
        "TypeError"
    );

    // `<span>` *is* on HTML's allowed-elements list; `<b>` is not.
    assert_eq!(
        read(
            &mut rt,
            "thrown(function(){ document.getElementById('a').attachShadow({mode:'open'}); })"
        ),
        "no-throw"
    );
    assert_eq!(
        read(
            &mut rt,
            "thrown(function(){ document.createElement('b').attachShadow({mode:'open'}); })"
        ),
        "NotSupportedError"
    );
    // A valid custom element name may host one.
    assert_eq!(
        read(
            &mut rt,
            "thrown(function(){ document.createElement('my-card').attachShadow({mode:'open'}); })"
        ),
        "no-throw"
    );

    rt.eval("var root = host.attachShadow({ mode: 'open' });")
        .expect("attach");
    assert_eq!(read(&mut rt, "root instanceof ShadowRoot"), "true");
    assert_eq!(read(&mut rt, "root instanceof DocumentFragment"), "true");
    assert_eq!(read(&mut rt, "root.nodeType"), "11");
    assert_eq!(read(&mut rt, "root.mode"), "open");
    assert_eq!(read(&mut rt, "root.host === host"), "true");
    assert_eq!(read(&mut rt, "host.shadowRoot === root"), "true");
    // The shadow root is not a child of its host.
    assert_eq!(read(&mut rt, "root.parentNode"), "null");
    assert_eq!(read(&mut rt, "host.childNodes.length"), "2");

    // A second attach throws; the root is not replaced.
    assert_eq!(
        read(
            &mut rt,
            "thrown(function(){ host.attachShadow({mode:'open'}); })"
        ),
        "NotSupportedError"
    );

    // Closed: the handle the attacher holds works, `shadowRoot` does not.
    rt.eval(
        "var d = document.createElement('div'); document.body.appendChild(d);\
         var closed = d.attachShadow({ mode: 'closed' });",
    )
    .expect("closed");
    assert_eq!(read(&mut rt, "closed.mode"), "closed");
    assert_eq!(read(&mut rt, "d.shadowRoot"), "null");
    assert_eq!(read(&mut rt, "closed.host === d"), "true");
}

/// Named assignment, the default slot, fallback content, reassignment after a
/// mutation, and `slotchange`.
fn named_slot_assignment<E: ScriptEngine>() {
    let mut rt = runtime::<E>(HOST_PAGE);
    rt.eval(
        "var host = document.getElementById('host');\
         var root = host.attachShadow({ mode: 'open' });\
         root.innerHTML = \"<slot name='s'>fallback-s</slot><slot>fallback-default</slot>\";\
         var named = root.childNodes[0]; var dflt = root.childNodes[1];",
    )
    .expect("setup");

    // `slot='s'` goes to the named slot, the unnamed child to the default one.
    assert_eq!(
        read(
            &mut rt,
            "named.assignedNodes().map(function(n){return n.id;}).join(',')"
        ),
        "a"
    );
    assert_eq!(
        read(
            &mut rt,
            "dflt.assignedNodes().map(function(n){return n.id;}).join(',')"
        ),
        "b"
    );
    assert_eq!(
        read(
            &mut rt,
            "document.getElementById('a').assignedSlot === named"
        ),
        "true"
    );
    assert_eq!(
        read(
            &mut rt,
            "document.getElementById('b').assignedSlot === dflt"
        ),
        "true"
    );
    // `assignedElements` filters to elements; both here are elements.
    assert_eq!(read(&mut rt, "named.assignedElements().length"), "1");

    // slotchange fires at the slot and bubbles.
    rt.eval(
        "var fired = [];\
         root.addEventListener('slotchange', function(e){ fired.push(e.target.name || 'default'); });",
    )
    .expect("listener");
    // Moving `b` into the named slot moves it off the default one: two slots
    // change, so two events.
    rt.eval("document.getElementById('b').setAttribute('slot', 's');")
        .expect("reassign");
    assert_eq!(
        read(
            &mut rt,
            "named.assignedNodes().map(function(n){return n.id;}).join(',')"
        ),
        "a,b"
    );
    assert_eq!(read(&mut rt, "dflt.assignedNodes().length"), "0");
    assert_eq!(read(&mut rt, "fired.sort().join('|')"), "default|s");

    // An empty slot shows fallback content under `flatten`.
    assert_eq!(
        read(
            &mut rt,
            "dflt.assignedNodes({flatten:true}).map(function(n){return n.textContent;}).join('')"
        ),
        "fallback-default"
    );
    // A slot with an assignment reports the assignment, flattened or not.
    assert_eq!(
        read(&mut rt, "named.assignedNodes({flatten:true}).length"),
        "2"
    );

    // Removing a light-DOM child clears its slot.
    rt.eval("host.removeChild(document.getElementById('a'));")
        .expect("remove");
    assert_eq!(
        read(
            &mut rt,
            "named.assignedNodes().map(function(n){return n.id;}).join(',')"
        ),
        "b"
    );
}

/// `slotAssignment: 'manual'` and `HTMLSlotElement.assign()`.
fn manual_slot_assignment<E: ScriptEngine>() {
    let mut rt = runtime::<E>(HOST_PAGE);
    rt.eval(
        "var host = document.getElementById('host');\
         var root = host.attachShadow({ mode: 'open', slotAssignment: 'manual' });\
         root.innerHTML = \"<slot name='s'></slot>\";\
         var slot = root.querySelector('slot');\
         var a = document.getElementById('a'); var b = document.getElementById('b');",
    )
    .expect("setup");
    assert_eq!(read(&mut rt, "root.slotAssignment"), "manual");
    // In manual mode the `slot` attribute assigns nothing.
    assert_eq!(read(&mut rt, "slot.assignedNodes().length"), "0");
    rt.eval("slot.assign(b);").expect("assign");
    assert_eq!(
        read(
            &mut rt,
            "slot.assignedNodes().map(function(n){return n.id;}).join(',')"
        ),
        "b"
    );
    assert_eq!(read(&mut rt, "b.assignedSlot === slot"), "true");
    assert_eq!(read(&mut rt, "a.assignedSlot"), "null");
    // Reassigning replaces rather than appends.
    rt.eval("slot.assign(a);").expect("reassign");
    assert_eq!(
        read(
            &mut rt,
            "slot.assignedNodes().map(function(n){return n.id;}).join(',')"
        ),
        "a"
    );
}

/// Retargeting, `composedPath()`, `composed`, and `getRootNode`.
fn retargeting_and_roots<E: ScriptEngine>() {
    let mut rt = runtime::<E>(HOST_PAGE);
    rt.eval(
        "var host = document.getElementById('host');\
         var root = host.attachShadow({ mode: 'open' });\
         root.innerHTML = '<b id=\"inner\">x</b>';\
         var inner = root.getElementById('inner');",
    )
    .expect("setup");

    // getRootNode stops at the shadow root; composed continues to the document.
    assert_eq!(read(&mut rt, "inner.getRootNode() === root"), "true");
    assert_eq!(
        read(&mut rt, "inner.getRootNode({composed:true}) === document"),
        "true"
    );
    assert_eq!(read(&mut rt, "host.getRootNode() === document"), "true");

    // A composed event escapes the boundary; outside it, the target is the host.
    rt.eval(
        "var seen = [];\
         document.body.addEventListener('ping', function(e){ seen.push('body:' + e.target.id); });\
         root.addEventListener('ping', function(e){ seen.push('root:' + e.target.id); });\
         inner.dispatchEvent(new Event('ping', { bubbles: true, composed: true }));",
    )
    .expect("composed dispatch");
    assert_eq!(read(&mut rt, "seen.join('|')"), "root:inner|body:host");

    // A non-composed event stops at the shadow root.
    rt.eval(
        "seen = [];\
         inner.dispatchEvent(new Event('ping', { bubbles: true }));",
    )
    .expect("uncomposed dispatch");
    assert_eq!(read(&mut rt, "seen.join('|')"), "root:inner");

    // composedPath through an open root shows everything, from either side.
    rt.eval(
        "var path = null;\
         document.body.addEventListener('probe', function(e){ path = e.composedPath(); });\
         inner.dispatchEvent(new Event('probe', { bubbles: true, composed: true }));",
    )
    .expect("path dispatch");
    assert_eq!(read(&mut rt, "path.indexOf(inner) >= 0"), "true");
    assert_eq!(read(&mut rt, "path.indexOf(root) >= 0"), "true");
    assert_eq!(read(&mut rt, "path.indexOf(host) >= 0"), "true");
    // Outside a dispatch it is empty.
    assert_eq!(read(&mut rt, "new Event('x').composedPath().length"), "0");

    // A closed root hides its nodes from a viewer outside it.
    rt.eval(
        "var d = document.createElement('div'); document.body.appendChild(d);\
         var shut = d.attachShadow({ mode: 'closed' });\
         shut.innerHTML = '<i id=\"hidden\">y</i>';\
         var hidden = shut.getElementById('hidden');\
         var outerPath = null;\
         document.body.addEventListener('shut', function(e){ outerPath = e.composedPath(); });\
         hidden.dispatchEvent(new Event('shut', { bubbles: true, composed: true }));",
    )
    .expect("closed dispatch");
    assert_eq!(read(&mut rt, "outerPath.indexOf(hidden) >= 0"), "false");
    assert_eq!(read(&mut rt, "outerPath.indexOf(d) >= 0"), "true");
}

/// The declarative post-parse pass, and that `innerHTML` does not run it while
/// `setHTMLUnsafe` does.
fn declarative_shadow_roots<E: ScriptEngine>() {
    let mut rt = runtime::<E>(
        "<html><body><div id='host'><template shadowrootmode='open' shadowrootserializable>\
         <slot></slot></template><span id='light'>lit</span></div></body></html>",
    );
    // The template is gone; a real shadow root stands in its place.
    assert_eq!(
        read(
            &mut rt,
            "document.getElementById('host').shadowRoot !== null"
        ),
        "true"
    );
    assert_eq!(
        read(&mut rt, "document.querySelectorAll('template').length"),
        "0"
    );
    assert_eq!(
        read(&mut rt, "document.getElementById('host').shadowRoot.mode"),
        "open"
    );
    // Its `<slot>` picked up the light-DOM child.
    assert_eq!(
        read(
            &mut rt,
            "document.getElementById('host').shadowRoot.querySelector('slot')\
             .assignedNodes().map(function(n){return n.id;}).join(',')"
        ),
        "light"
    );

    // getHTML writes a serializable root back out; innerHTML does not.
    assert_eq!(
        read(
            &mut rt,
            "document.getElementById('host').getHTML().indexOf('shadowrootmode') >= 0"
        ),
        "false"
    );
    assert_eq!(
        read(
            &mut rt,
            "document.getElementById('host').getHTML({serializableShadowRoots:true})\
             .indexOf('shadowrootmode') >= 0"
        ),
        "true"
    );

    // innerHTML leaves a declarative template as an ordinary template.
    rt.eval(
        "var plain = document.createElement('div'); document.body.appendChild(plain);\
         plain.innerHTML = \"<div id='p'><template shadowrootmode='open'><b>t</b></template></div>\";",
    )
    .expect("innerHTML");
    assert_eq!(
        read(&mut rt, "document.getElementById('p').shadowRoot"),
        "null"
    );
    // setHTMLUnsafe does run the pass.
    rt.eval(
        "var unsafe = document.createElement('div'); document.body.appendChild(unsafe);\
         unsafe.setHTMLUnsafe(\"<div id='u'><template shadowrootmode='open'><b>t</b></template></div>\");",
    )
    .expect("setHTMLUnsafe");
    assert_eq!(
        read(&mut rt, "document.getElementById('u').shadowRoot !== null"),
        "true"
    );
}

/// `<template>.content` is a fragment in a shared inert document, out of the
/// main tree.
fn template_contents<E: ScriptEngine>() {
    let mut rt = runtime::<E>(
        "<html><body><template id='t1'><p id='inside'>hi</p></template>\
         <template id='t2'><b>other</b></template></body></html>",
    );
    rt.eval("var t1 = document.getElementById('t1'); var t2 = document.getElementById('t2');")
        .expect("setup");
    assert_eq!(read(&mut rt, "t1.content.nodeType"), "11");
    // The contents are not the template's children.
    assert_eq!(read(&mut rt, "t1.childNodes.length"), "0");
    assert_eq!(read(&mut rt, "t1.content.childNodes.length"), "1");
    assert_eq!(read(&mut rt, "t1.content.firstChild.id"), "inside");
    // Inert: the main tree does not reach into it.
    assert_eq!(read(&mut rt, "document.getElementById('inside')"), "null");
    // One inert owner document, shared by every template in this document.
    assert_eq!(
        read(
            &mut rt,
            "t1.content.ownerDocument === t2.content.ownerDocument"
        ),
        "true"
    );
    assert_eq!(
        read(&mut rt, "t1.content.ownerDocument === document"),
        "false"
    );
    // Stable identity across reads.
    assert_eq!(read(&mut rt, "t1.content === t1.content"), "true");
}

/// `cloneNode` of a host carries a **clonable** shadow root, always deeply.
fn clone_node_carries_clonable_roots<E: ScriptEngine>() {
    let mut rt = runtime::<E>(HOST_PAGE);
    rt.eval(
        "var plain = document.createElement('div');         var plainRoot = plain.attachShadow({ mode: 'open' });         plainRoot.innerHTML = '<i>a</i>';         var keep = document.createElement('div');         var keepRoot = keep.attachShadow({ mode: 'open', clonable: true });         keepRoot.innerHTML = '<input><div><span></span></div>';",
    )
    .expect("setup");
    // Not clonable: the clone has no shadow root at all.
    assert_eq!(read(&mut rt, "plain.cloneNode(true).shadowRoot"), "null");

    // Clonable: the root travels, keeps its flags, and its tree comes deep
    // even from a shallow clone of the host.
    rt.eval("var deep = keep.cloneNode(true); var shallow = keep.cloneNode(false);")
        .expect("clone");
    assert_eq!(read(&mut rt, "deep.shadowRoot !== null"), "true");
    assert_eq!(read(&mut rt, "deep.shadowRoot.clonable"), "true");
    assert_eq!(read(&mut rt, "deep.shadowRoot !== keepRoot"), "true");
    assert_eq!(read(&mut rt, "deep.shadowRoot.childNodes.length"), "2");
    assert_eq!(
        read(&mut rt, "deep.shadowRoot.childNodes[0].localName"),
        "input"
    );
    assert_eq!(
        read(
            &mut rt,
            "deep.shadowRoot.childNodes[1].childNodes[0].localName"
        ),
        "span"
    );
    assert_eq!(read(&mut rt, "shallow.shadowRoot.childNodes.length"), "2");
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
    attach_shadow_and_modes => (attach_shadow_on_boa, attach_shadow_on_nova),
    named_slot_assignment => (named_slots_on_boa, named_slots_on_nova),
    manual_slot_assignment => (manual_slots_on_boa, manual_slots_on_nova),
    retargeting_and_roots => (retargeting_on_boa, retargeting_on_nova),
    declarative_shadow_roots => (declarative_on_boa, declarative_on_nova),
    template_contents => (template_contents_on_boa, template_contents_on_nova),
    clone_node_carries_clonable_roots => (clone_clonable_on_boa, clone_clonable_on_nova),
}
