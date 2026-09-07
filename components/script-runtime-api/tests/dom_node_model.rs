// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The DOM node model lane: `DocumentFragment` insertion, `CDATASection` and
//! real `DocumentType` nodes, the `ParentNode` / `ChildNode` mixins,
//! `normalize`, `outerHTML`, namespaced attributes as `Attr` nodes, and
//! `DOMParser` / `XMLSerializer`. Engine-generic bodies instantiated per backend.

use genet_static_dom::StaticDocument;
use script_engine_api::ScriptEngine;
use script_runtime_api::Runtime;

fn console<E: ScriptEngine>(source: &str) -> Vec<String> {
    run::<E>(source, false)
}

/// `pump` runs the microtask checkpoint, which is where observer records are
/// delivered.
fn run<E: ScriptEngine>(source: &str, pump: bool) -> Vec<String> {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.load_dom(&StaticDocument::parse(
        "<!doctype html><html><body></body></html>",
    ));
    rt.eval(source).expect("script");
    if pump {
        rt.run_microtasks();
    }
    let out = rt.host().borrow().console.clone();
    out
}

/// Inserting a fragment moves its children (the DOM's insertion steps), for
/// `appendChild`, `insertBefore` and `replaceChild` alike, and leaves the
/// fragment empty.
fn fragment_insertion_works<E: ScriptEngine>() {
    assert_eq!(
        console::<E>(
            "var host = document.createElement('div'); document.body.appendChild(host);\
             var f = document.createDocumentFragment();\
             f.appendChild(document.createElement('a'));\
             f.appendChild(document.createElement('b'));\
             host.appendChild(f);\
             console.log(host.childNodes.length + ',' + f.childNodes.length + ',' + host.innerHTML);\
             var g = document.createDocumentFragment();\
             g.appendChild(document.createElement('i'));\
             host.insertBefore(g, host.firstChild);\
             console.log(host.innerHTML);\
             var h = document.createDocumentFragment();\
             h.appendChild(document.createElement('u'));\
             host.replaceChild(h, host.firstChild);\
             console.log(host.innerHTML);"
        ),
        vec![
            "2,0,<a></a><b></b>",
            "<i></i><a></a><b></b>",
            "<u></u><a></a><b></b>",
        ],
    );
}

/// A fragment insertion is one `childList` record on the fragment (all children
/// removed) and one on the parent (all children added) — the spec's shape.
fn fragment_insertion_records_two_groups_works<E: ScriptEngine>() {
    assert_eq!(
        run::<E>(
            "var host = document.createElement('div'); document.body.appendChild(host);\
             var f = document.createDocumentFragment();\
             f.appendChild(document.createElement('a'));\
             f.appendChild(document.createElement('b'));\
             var seen = [];\
             var o = new MutationObserver(function(records) {\
               for (var i = 0; i < records.length; i++) {\
                 seen.push(records[i].type + ':' + records[i].addedNodes.length + ':' + records[i].removedNodes.length);\
               }\
               console.log(seen.join('|'));\
             });\
             o.observe(host, { childList: true, subtree: true });\
             o.observe(f, { childList: true });\
             host.appendChild(f);",
            true,
        ),
        vec!["childList:0:2|childList:2:0"],
    );
}

/// `document.doctype` names a real `DocumentType`, `createDocumentType` mints
/// one, and `createCDATASection` is XML-only.
fn doctype_and_cdata_works<E: ScriptEngine>() {
    assert_eq!(
        console::<E>(
            "var dt = document.doctype;\
             console.log((dt ? dt.nodeType + ',' + dt.name + ',' + (dt instanceof DocumentType) : 'none'));\
             var made = document.implementation.createDocumentType('svg', 'pub', 'sys');\
             console.log(made.nodeType + ',' + made.name + ',' + made.publicId + ',' + made.systemId + ',' + made.nodeName);\
             try { document.createCDATASection('x'); console.log('no-throw'); }\
             catch (e) { console.log(e.name); }\
             var xml = document.implementation.createDocument(null, 'root', null);\
             var c = xml.createCDATASection('a<b');\
             xml.documentElement.appendChild(c);\
             console.log(c.nodeType + ',' + c.data + ',' + (c instanceof Text) + ',' + c.nodeName);\
             console.log(new XMLSerializer().serializeToString(xml));"
        ),
        vec![
            "10,html,true",
            "10,svg,pub,sys,svg",
            "NotSupportedError",
            "4,a<b,true,#cdata-section",
            "<root><![CDATA[a<b]]></root>",
        ],
    );
}

/// The `ParentNode` and `ChildNode` mixins, with the spec's node-or-string
/// conversion.
fn mixins_work<E: ScriptEngine>() {
    assert_eq!(
        console::<E>(
            "var host = document.createElement('div'); document.body.appendChild(host);\
             host.append('a', document.createElement('b'), 'c');\
             console.log(host.innerHTML);\
             host.prepend(document.createElement('i'));\
             console.log(host.innerHTML);\
             var b = host.querySelector('b');\
             b.before('X'); b.after('Y');\
             console.log(host.innerHTML);\
             b.replaceWith(document.createElement('u'), 'Z');\
             console.log(host.innerHTML);\
             host.replaceChildren('only');\
             console.log(host.innerHTML + ',' + host.childNodes.length);\
             var t = host.firstChild; t.remove();\
             console.log(host.childNodes.length);"
        ),
        vec![
            "a<b></b>c",
            "<i></i>a<b></b>c",
            "<i></i>aX<b></b>Yc",
            "<i></i>aX<u></u>ZYc",
            "only,1",
            "0",
        ],
    );
}

/// `normalize` merges contiguous text and drops empty text nodes; `outerHTML`
/// reads and replaces the element itself.
fn normalize_and_outer_html_work<E: ScriptEngine>() {
    assert_eq!(
        console::<E>(
            "var host = document.createElement('div'); host.id = 'h'; document.body.appendChild(host);\
             host.appendChild(document.createTextNode('a'));\
             host.appendChild(document.createTextNode(''));\
             host.appendChild(document.createTextNode('b'));\
             host.appendChild(document.createElement('i'));\
             host.appendChild(document.createTextNode('c'));\
             console.log(host.childNodes.length);\
             host.normalize();\
             console.log(host.childNodes.length + ',' + host.firstChild.data);\
             console.log(host.outerHTML);\
             host.outerHTML = '<p>1</p><p>2</p>';\
             console.log(document.body.innerHTML);"
        ),
        vec![
            "5",
            "3,ab",
            "<div id=\"h\">ab<i></i>c</div>",
            "<p>1</p><p>2</p>",
        ],
    );
}

/// Attributes carry a real namespace and surface as live `Attr` nodes through
/// `Element.attributes`, `getAttributeNode*` and `setAttributeNode*`.
fn attributes_are_attr_nodes_works<E: ScriptEngine>() {
    assert_eq!(
        console::<E>(
            "var XLINK = 'http://www.w3.org/1999/xlink';\
             var el = document.createElement('div'); document.body.appendChild(el);\
             el.setAttribute('id', 'one');\
             el.setAttributeNS(XLINK, 'xlink:href', 'u');\
             console.log(el.getAttributeNS(XLINK, 'href') + ',' + el.getAttribute('xlink:href') + ',' + el.hasAttributeNS(XLINK, 'href'));\
             console.log(el.attributes.length + ',' + el.attributes[0].name + ',' + el.attributes[1].name);\
             var a = el.getAttributeNode('id');\
             console.log(a.nodeType + ',' + a.name + ',' + a.value + ',' + (a.ownerElement === el) + ',' + (el.getAttributeNode('id') === a));\
             el.setAttribute('id', 'two');\
             console.log(a.value);\
             a.value = 'three';\
             console.log(el.getAttribute('id'));\
             var ns = el.getAttributeNodeNS(XLINK, 'href');\
             console.log(ns.namespaceURI + ',' + ns.prefix + ',' + ns.localName);\
             var fresh = document.createAttribute('data-x'); fresh.value = 'v';\
             el.setAttributeNode(fresh);\
             console.log(el.getAttribute('data-x') + ',' + el.attributes.length);\
             el.removeAttributeNS(XLINK, 'href');\
             console.log(el.attributes.length + ',' + el.hasAttribute('xlink:href'));\
             console.log(el.getAttributeNames().join('|'));"
        ),
        vec![
            "u,u,true",
            "2,id,xlink:href",
            "2,id,one,true,true",
            "two",
            "three",
            "http://www.w3.org/1999/xlink,xlink,href",
            "v,3",
            "2,false",
            "id|data-x",
        ],
    );
}

/// Cloning preserves comments, processing instructions and doctypes, and the
/// static-to-scripted clone carries them into the live document.
fn clone_preserves_every_kind_works<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.load_dom(&StaticDocument::parse(
        "<!doctype html><html><body><!--keep--><div>t</div></body></html>",
    ));
    rt.eval(
        "console.log(document.doctype ? document.doctype.name : 'none');\
         var kinds = [];\
         var kids = document.body.childNodes;\
         for (var i = 0; i < kids.length; i++) kinds.push(kids[i].nodeType);\
         console.log(kinds.join(','));\
         var deep = document.body.cloneNode(true);\
         var out = [];\
         for (var j = 0; j < deep.childNodes.length; j++) out.push(deep.childNodes[j].nodeType);\
         console.log(out.join(','));\
         var pi = document.createProcessingInstruction('t', 'd');\
         console.log(pi.cloneNode(false).nodeType + ',' + pi.cloneNode(false).target);",
    )
    .expect("script");
    assert_eq!(
        rt.host().borrow().console.clone(),
        vec!["html", "8,1", "8,1", "7,t"],
    );
}

/// `DOMParser.parseFromString` builds a fresh document from both parsers, and
/// `baseURI` is the document URL.
fn dom_parser_and_base_uri_work<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.load_dom(&StaticDocument::parse("<html><body></body></html>"));
    rt.set_base_url("https://example.com/a/b.html")
        .expect("base");
    rt.eval(
        "var doc = new DOMParser().parseFromString('<p id=q>hi</p>', 'text/html');\
         console.log(doc.nodeType + ',' + doc.querySelector('p').textContent + ',' + (doc !== document));\
         var xml = new DOMParser().parseFromString('<r><c a=\"1\"/></r>', 'text/xml');\
         console.log(xml.documentElement.nodeName + ',' + xml.documentElement.firstChild.getAttribute('a'));\
         console.log(new XMLSerializer().serializeToString(xml.documentElement));\
         var bad = new DOMParser().parseFromString('', 'text/xml');\
         console.log(bad.documentElement.localName);\
         console.log(document.body.baseURI + ',' + document.createAttribute('c').baseURI);",
    )
    .expect("script");
    assert_eq!(
        rt.host().borrow().console.clone(),
        vec![
            "9,hi,true",
            "r,1",
            "<r><c a=\"1\"/></r>",
            "parsererror",
            "https://example.com/a/b.html,https://example.com/a/b.html",
        ],
    );
}

macro_rules! per_backend {
    ($body:ident, $boa:ident, $nova:ident) => {
        #[test]
        fn $boa() {
            $body::<script_engine_boa::BoaEngine>();
        }

        #[cfg(not(target_arch = "wasm32"))]
        #[test]
        fn $nova() {
            $body::<script_engine_nova::NovaEngine>();
        }
    };
}

per_backend!(
    fragment_insertion_works,
    fragment_insertion_on_boa,
    fragment_insertion_on_nova
);
per_backend!(
    fragment_insertion_records_two_groups_works,
    fragment_records_on_boa,
    fragment_records_on_nova
);
per_backend!(
    doctype_and_cdata_works,
    doctype_and_cdata_on_boa,
    doctype_and_cdata_on_nova
);
per_backend!(mixins_work, mixins_on_boa, mixins_on_nova);
per_backend!(
    normalize_and_outer_html_work,
    normalize_outer_html_on_boa,
    normalize_outer_html_on_nova
);
per_backend!(
    attributes_are_attr_nodes_works,
    attributes_on_boa,
    attributes_on_nova
);
per_backend!(
    clone_preserves_every_kind_works,
    clone_kinds_on_boa,
    clone_kinds_on_nova
);
per_backend!(
    dom_parser_and_base_uri_work,
    dom_parser_on_boa,
    dom_parser_on_nova
);
