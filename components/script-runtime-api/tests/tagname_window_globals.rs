// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tag-name casing per document type, and the window global's own properties.
//!
//! `tagName` / `nodeName` uppercase only for an HTML-namespaced element whose
//! *current* node document is an HTML document, so adoption changes the answer;
//! `localName` never folds; custom element names are case-sensitive. `window`
//! and `document` are `[LegacyUnforgeable]` on a Window and `self` is
//! `[Replaceable]`. Engine-generic bodies instantiated per backend.

use genet_static_dom::StaticDocument;
use script_engine_api::ScriptEngine;
use script_runtime_api::Runtime;

fn console<E: ScriptEngine>(source: &str) -> Vec<String> {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.load_dom(&StaticDocument::parse(
        "<!doctype html><html><body></body></html>",
    ));
    rt.eval(source).expect("script");
    let out = rt.host().borrow().console.clone();
    out
}

/// The HTML-document rule: HTML-namespaced names uppercase (prefix included),
/// other namespaces never do, and `localName` always preserves case.
fn tag_name_folds_only_html_in_html_document<E: ScriptEngine>() {
    assert_eq!(
        console::<E>(
            "var H = 'http://www.w3.org/1999/xhtml';\
             var S = 'http://www.w3.org/2000/svg';\
             console.log([document.createElementNS(H, 'I').tagName,\
                          document.createElementNS(H, 'i').tagName,\
                          document.createElementNS(H, 'x:b').tagName].join(','));\
             console.log([document.createElementNS(S, 'svg').tagName,\
                          document.createElementNS(S, 'SVG').tagName,\
                          document.createElementNS(S, 's:SVG').tagName,\
                          document.createElementNS(S, 'textPath').tagName].join(','));\
             console.log(document.createElementNS('http://example.com/', 'mixedCase').tagName);\
             var el = document.createElementNS(H, 'I');\
             console.log(el.localName + ',' + el.nodeName + ',' + el.prefix);"
        ),
        vec!["I,I,X:B", "svg,SVG,s:SVG,textPath", "mixedCase", "I,I,null",],
    );
}

/// An XML document preserves case, and importing into the HTML document folds
/// the same node's name — the property is read per access, not stored.
fn tag_name_follows_the_node_document<E: ScriptEngine>() {
    assert_eq!(
        console::<E>(
            "var H = 'http://www.w3.org/1999/xhtml';\
             var xml = new DOMParser().parseFromString(\
               '<div xmlns=\"http://www.w3.org/1999/xhtml\">Test</div>', 'text/xml');\
             console.log(xml.documentElement.tagName + ',' +\
                         document.importNode(xml.documentElement, true).tagName);\
             var d1 = document.implementation.createDocument(H, 'div', null);\
             console.log(d1.documentElement.tagName + ',' +\
                         document.importNode(d1.documentElement, true).tagName);\
             var d2 = document.implementation.createDocument(H, 'foo:div', null);\
             console.log(d2.documentElement.tagName + ',' +\
                         document.importNode(d2.documentElement, true).tagName);\
             console.log(d1.createElement('DiV').tagName + ',' +\
                         document.createElement('DiV').tagName);"
        ),
        vec!["div,DIV", "div,DIV", "foo:div,FOO:DIV", "DiV,DIV"],
    );
}

/// Custom element names are case-sensitive and must start with an ASCII
/// lowercase letter, so `foo-BAR` and the a-with-ring name are
/// `HTMLUnknownElement` while `foo-bar` is `HTMLElement`. `createElement` folds
/// only ASCII, which is what keeps them distinguishable.
fn custom_element_names_are_case_sensitive<E: ScriptEngine>() {
    assert_eq!(
        console::<E>(
            "var H = 'http://www.w3.org/1999/xhtml';\
             function iface(e) { return Object.prototype.toString.call(e).slice(8, -1); }\
             console.log([iface(document.createElementNS(H, 'foo-bar')),\
                          iface(document.createElementNS(H, 'foo-BAR')),\
                          iface(document.createElementNS(H, '\\u00e5-bar')),\
                          iface(document.createElement('\\u00c5-BAR')),\
                          iface(document.createElementNS(H, 'DIV')),\
                          iface(document.createElement('DIV'))].join(','));\
             console.log(document.createElementNS(H, 'foo-BAR').tagName + ',' +\
                         document.createElementNS(H, 'foo-BAR').localName);\
             console.log(document.createElement('\\u00c5-BAR').localName);"
        ),
        vec![
            "HTMLElement,HTMLUnknownElement,HTMLUnknownElement,\
             HTMLUnknownElement,HTMLUnknownElement,HTMLDivElement",
            "FOO-BAR,foo-BAR",
            "\u{c5}-bar",
        ],
    );
}

/// A `foo-bar` definition must not claim a `foo-BAR` element.
fn custom_element_definitions_do_not_fold<E: ScriptEngine>() {
    assert_eq!(
        console::<E>(
            "var H = 'http://www.w3.org/1999/xhtml';\
             class FooBar extends HTMLElement {}\
             customElements.define('foo-bar', FooBar);\
             console.log((document.createElement('foo-bar') instanceof FooBar) + ',' +\
                         (document.createElementNS(H, 'foo-BAR') instanceof FooBar));\
             var bad = false;\
             try { customElements.define('foo-BAR', class extends HTMLElement {}); }\
             catch (e) { bad = e.name; }\
             console.log(String(bad));"
        ),
        vec!["true,false", "SyntaxError"],
    );
}

/// `window` and `document` are `[LegacyUnforgeable]`: getter, no setter,
/// non-configurable, so neither can be deleted or redefined. This is also the
/// engine question the plan records — a backend that refuses a non-configurable
/// accessor on its global fails here rather than silently approximating.
fn window_and_document_are_unforgeable<E: ScriptEngine>() {
    assert_eq!(
        console::<E>(
            "function d(n) {\
               var x = Object.getOwnPropertyDescriptor(globalThis, n);\
               return n + ':' + (typeof x.get === 'function') + ',' + (x.set === undefined) +\
                      ',' + x.configurable + ',' + x.enumerable;\
             }\
             console.log(d('window'));\
             console.log(d('document'));\
             console.log((window === globalThis) + ',' + (document === globalThis.document));\
             var deleted = true;\
             try { deleted = delete globalThis.window; } catch (e) { deleted = 'threw'; }\
             console.log(String(deleted) + ',' + ('window' in globalThis));\
             var threw = 'no';\
             try { Object.defineProperty(globalThis, 'window', { value: 1 }); }\
             catch (e) { threw = e.name; }\
             console.log(threw + ',' + (window === globalThis));"
        ),
        vec![
            "window:true,true,false,true",
            "document:true,true,false,true",
            "true,true",
            "false,true",
            "TypeError,true",
        ],
    );
}

/// `self` is `[Replaceable]`: an accessor whose setter redefines it as an
/// ordinary writable data property.
fn self_is_replaceable_on_window<E: ScriptEngine>() {
    assert_eq!(
        console::<E>(
            "var x = Object.getOwnPropertyDescriptor(globalThis, 'self');\
             console.log((typeof x.get === 'function') + ',' + (typeof x.set === 'function') +\
                         ',' + x.configurable + ',' + x.enumerable + ',' + (self === globalThis));\
             self = 42;\
             var y = Object.getOwnPropertyDescriptor(globalThis, 'self');\
             console.log(self + ',' + y.value + ',' + y.writable + ',' + y.configurable +\
                         ',' + y.enumerable + ',' + (y.get === undefined));"
        ),
        vec!["true,true,true,true,true", "42,42,true,true,true,true"],
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
    tag_name_folds_only_html_in_html_document,
    tag_name_html_document_on_boa,
    tag_name_html_document_on_nova
);
per_backend!(
    tag_name_follows_the_node_document,
    tag_name_node_document_on_boa,
    tag_name_node_document_on_nova
);
per_backend!(
    custom_element_names_are_case_sensitive,
    custom_element_case_on_boa,
    custom_element_case_on_nova
);
per_backend!(
    custom_element_definitions_do_not_fold,
    custom_element_define_case_on_boa,
    custom_element_define_case_on_nova
);
per_backend!(
    window_and_document_are_unforgeable,
    unforgeable_globals_on_boa,
    unforgeable_globals_on_nova
);
per_backend!(
    self_is_replaceable_on_window,
    replaceable_self_on_boa,
    replaceable_self_on_nova
);
