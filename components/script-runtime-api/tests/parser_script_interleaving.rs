// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! HTML's parsing model with scripts interleaved: a script sees the tree only
//! as far as its own position, `document.write` re-enters the token stream at
//! the insertion point, `document.currentScript` names the running script, the
//! readiness transitions and their events fire at HTML's points, and everything
//! that observes the partially built tree during a script — custom element
//! upgrades, declarative shadow roots consulting the registry, and
//! `MutationObserver` records for parser insertions — sees what a browser
//! would.
//!
//! Each body runs against both backends: interleaving is a host-side loop, but
//! every question in it is answered by a script, so a backend difference in
//! evaluation order or microtask timing would show up here.

use script_engine_api::ScriptEngine;
use script_runtime_api::{NoScriptLoader, ParserScriptLoader, Runtime};

fn read<E: ScriptEngine>(rt: &mut Runtime<E>, expr: &str) -> String {
    let v = rt.eval(expr).expect("eval");
    rt.value_to_string(&v).expect("stringify")
}

fn parse<E: ScriptEngine>(html: &str) -> Runtime<E> {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.parse_document_interleaved(html, &NoScriptLoader);
    rt
}

/// A loader over a fixed table, standing in for the document's resource route.
struct TableLoader(Vec<(&'static str, &'static str)>);

impl ParserScriptLoader for TableLoader {
    fn load(&self, src: &str, _charset: Option<&str>, _integrity: Option<&str>) -> Option<String> {
        self.0
            .iter()
            .find(|(name, _)| *name == src)
            .map(|(_, body)| (*body).to_owned())
    }
}

/// The defining difference from parse-then-run: a script sees the tree only as
/// far as its own position in the source.
fn a_script_sees_the_tree_only_as_far_as_itself<E: ScriptEngine>() {
    let mut rt = parse::<E>(
        "<body><p id=before>b</p>\
         <script>window.seen = [!!document.getElementById('before'), \
                                !!document.getElementById('after')];</script>\
         <p id=after>a</p></body>",
    );
    assert_eq!(read(&mut rt, "String(seen)"), "true,false");
    // And by the end of the parse both are there.
    assert_eq!(read(&mut rt, "!!document.getElementById('after')"), "true");
}

/// Document order across inline and external classic scripts, and the two
/// deferred families running after the parse.
fn script_order_is_document_order<E: ScriptEngine>() {
    let loader = TableLoader(vec![
        ("ext.js", "log.push('ext')"),
        ("defer.js", "log.push('defer-ext')"),
        ("async.js", "log.push('async-ext')"),
    ]);
    let mut rt = Runtime::<E>::new().expect("runtime");
    let report = rt.parse_document_interleaved(
        "<body><script>window.log = ['inline-1'];</script>\
         <script src=ext.js></script>\
         <script src=defer.js defer></script>\
         <script src=async.js async></script>\
         <script>log.push('inline-2')</script>\
         <script type=module>log.push('module')</script>\
         <script type=text/plain>log.push('data-block')</script>\
         </body>",
        &loader,
    );
    // Parser-blocking scripts ran during the parse; `defer` and the module ran
    // after it. The data block never ran at all.
    assert_eq!(report.scripts_run, 4, "blocking + async");
    assert_eq!(report.deferred_run, 2, "defer + module");
    let log = read(&mut rt, "log.join(',')");
    // `async` does not block the parser, so the later inline script runs first
    // even though this route's fetch had already completed. This assertion read
    // `inline-1,ext,async-ext,inline-2` until 2026-09-08, when the async script
    // stopped running at the pause — see `ScriptTiming::Async`.
    assert!(
        log.starts_with("inline-1,ext,inline-2,async-ext"),
        "parser-blocking scripts run in document order and async does not block them, got {log}"
    );
    assert!(log.contains("defer-ext"), "defer must run, got {log}");
    assert!(
        !log.contains("data-block"),
        "a non-script type must not run, got {log}"
    );
    let deferred_index = log.find("defer-ext").expect("defer ran");
    let inline2_index = log.find("inline-2").expect("inline-2 ran");
    assert!(
        inline2_index < deferred_index,
        "defer runs after parsing, got {log}"
    );
}

/// `document.write` during parsing inserts at the insertion point — before the
/// source that follows the script, not appended at the end.
fn document_write_lands_at_the_insertion_point<E: ScriptEngine>() {
    let mut rt = parse::<E>(
        "<body><script>document.write('<b id=written>w</b>')</script>\
         <span id=after></span></body>",
    );
    assert_eq!(
        read(&mut rt, "!!document.getElementById('written')"),
        "true"
    );
    assert_eq!(
        read(
            &mut rt,
            "document.getElementById('written').nextElementSibling.id"
        ),
        "after",
        "written markup must precede the source after the script"
    );
    // A tag left open by one write is closed by the next: the two are one
    // source stream, which is only true if they reach the tokenizer.
    let mut rt = parse::<E>(
        "<body><script>document.write('<i id=split>');document.write('x</i>')</script></body>",
    );
    assert_eq!(
        read(&mut rt, "document.getElementById('split').textContent"),
        "x"
    );
}

/// `document.currentScript` names the classic script now running, and is null
/// outside one.
fn current_script_names_the_running_script<E: ScriptEngine>() {
    let mut rt = parse::<E>(
        "<body><script id=one>window.a = document.currentScript && document.currentScript.id;\
         </script><script id=two>window.b = document.currentScript.id;</script>\
         <script type=module>window.c = document.currentScript;</script></body>",
    );
    assert_eq!(read(&mut rt, "a"), "one");
    assert_eq!(read(&mut rt, "b"), "two");
    assert_eq!(
        read(&mut rt, "String(c)"),
        "null",
        "modules set no currentScript"
    );
    assert_eq!(
        read(&mut rt, "String(document.currentScript)"),
        "null",
        "null once nothing is running"
    );
}

/// The readiness transitions and their events, at HTML's points relative to
/// deferred scripts.
fn readiness_transitions_fire_in_order<E: ScriptEngine>() {
    let mut rt = parse::<E>(
        "<body><script>\
           window.log = [];\
           window.log.push('script:' + document.readyState);\
           document.addEventListener('readystatechange', function() { \
             log.push('rsc:' + document.readyState); });\
           document.addEventListener('DOMContentLoaded', function() { \
             log.push('dcl:' + document.readyState); });\
           window.addEventListener('load', function() { \
             log.push('load:' + document.readyState); });\
         </script>\
         <script type=module>log.push('defer:' + document.readyState)</script></body>",
    );
    assert_eq!(
        read(&mut rt, "log.join('|')"),
        "script:loading|rsc:interactive|defer:interactive|dcl:interactive|rsc:complete|load:complete"
    );
}

/// A custom element defined by one script is upgraded for a tag parsed after
/// it, before the next script sees the tree.
fn a_definition_upgrades_elements_parsed_after_it<E: ScriptEngine>() {
    let mut rt = parse::<E>(
        "<body><script>\
           window.constructed = [];\
           class Later extends HTMLElement { constructor() { super(); constructed.push('c'); } }\
           customElements.define('later-el', Later);\
           window.Later = Later;\
         </script>\
         <later-el id=made></later-el>\
         <script>window.upgraded = document.getElementById('made') instanceof Later;</script>\
         </body>",
    );
    assert_eq!(read(&mut rt, "String(upgraded)"), "true");
    assert_eq!(read(&mut rt, "String(constructed.length > 0)"), "true");
}

/// The Shadow DOM lane's first regression: a declarative shadow root must
/// consult the custom-element registry, which is only populated because an
/// earlier script in this same parse ran.
fn a_disabled_definition_refuses_a_declarative_root<E: ScriptEngine>() {
    let mut rt = parse::<E>(
        "<body><script>\
           class D extends HTMLElement { static get disabledFeatures() { return ['shadow']; } }\
           customElements.define('shadow-disabled', D);\
           window.D = D;\
         </script>\
         <shadow-disabled><template shadowrootmode=open><span id=inside></span></template>\
         </shadow-disabled>\
         <script>\
           var el = document.querySelector('shadow-disabled');\
           window.isInstance = el instanceof D;\
           window.keptTemplate = !!el.querySelector('template');\
           window.hasRoot = !!el.shadowRoot;\
         </script></body>",
    );
    assert_eq!(read(&mut rt, "String(isInstance)"), "true");
    assert_eq!(read(&mut rt, "String(keptTemplate)"), "true");
    assert_eq!(read(&mut rt, "String(hasRoot)"), "false");
}

/// A definition with no `disabledFeatures` still gets its declarative root, so
/// the refusal above is the registry talking and not a blanket regression.
fn an_ordinary_custom_element_still_gets_its_declarative_root<E: ScriptEngine>() {
    let mut rt = parse::<E>(
        "<body><script>\
           customElements.define('plain-el', class extends HTMLElement {});\
         </script>\
         <plain-el><template shadowrootmode=open><span id=inside></span></template></plain-el>\
         <script>window.hasRoot = !!document.querySelector('plain-el').shadowRoot;</script>\
         </body>",
    );
    assert_eq!(read(&mut rt, "String(hasRoot)"), "true");
}

/// The Shadow DOM lane's second regression: a `MutationObserver` registered by
/// an early script sees the parser's insertions, and a root it attaches from
/// that callback leaves the following `<template shadowrootmode>` ordinary.
fn a_mutation_observer_sees_parser_insertions<E: ScriptEngine>() {
    let mut rt = parse::<E>(
        "<body><script>\
           window.seen = [];\
           new MutationObserver(function(records) {\
             for (var r of records) for (var n of r.addedNodes) {\
               if (n.nodeType === 1) seen.push(n.localName);\
               if (n.id === 'has-imperative-root') n.attachShadow({ mode: 'open' });\
             }\
           }).observe(document.body, { childList: true, subtree: true });\
         </script>\
         <div id='has-imperative-root'>\
           <script>/* forces the checkpoint the observer runs in */</script>\
           <template id=ordinarytemplate shadowrootmode=open><span id=toreplace></span></template>\
           <script>ordinarytemplate.innerHTML = '<span id=replaced></span>';</script>\
         </div>\
         <script>\
           var host = document.querySelector('#has-imperative-root');\
           window.hasRoot = !!host.shadowRoot;\
           window.rootEmpty = host.shadowRoot && !host.shadowRoot.hasChildNodes();\
           var t = host.querySelector('template#ordinarytemplate');\
           window.keptTemplate = !!t;\
           window.replaced = !!(t && t.content.querySelector('#replaced'));\
           window.oldGone = !!(t && !t.content.querySelector('#toreplace'));\
         </script></body>",
    );
    assert!(
        read(&mut rt, "seen.join(',')").contains("div"),
        "the observer must see the parser insert the div"
    );
    assert_eq!(read(&mut rt, "String(hasRoot)"), "true");
    assert_eq!(read(&mut rt, "String(rootEmpty)"), "true");
    assert_eq!(read(&mut rt, "String(keptTemplate)"), "true");
    assert_eq!(read(&mut rt, "String(replaced)"), "true");
    assert_eq!(read(&mut rt, "String(oldGone)"), "true");
}

/// A node the parser created and an earlier script touched keeps its wrapper
/// across the rest of the parse and a collection — the reflector-identity
/// policy's root-on-insertion path, exercised through the parser.
fn parser_created_nodes_keep_their_wrapper<E: ScriptEngine>() {
    let mut rt = parse::<E>(
        "<body><p id=kept>x</p>\
         <script>window.first = document.getElementById('kept'); first.marked = 7;</script>\
         <p id=filler></p>\
         <script>window.same = document.getElementById('kept') === first;</script></body>",
    );
    assert_eq!(read(&mut rt, "String(same)"), "true");
    rt.collect_garbage();
    assert_eq!(
        read(&mut rt, "String(document.getElementById('kept') === first)"),
        "true"
    );
    assert_eq!(read(&mut rt, "String(first.marked)"), "7");
}

/// `document.open` / `write` / `close` outside a parse: the implied
/// `document.open` replaces the document.
fn document_write_after_parsing_implies_open<E: ScriptEngine>() {
    let mut rt = parse::<E>("<body><p id=original>o</p></body>");
    assert_eq!(read(&mut rt, "document.readyState"), "complete");
    rt.eval("document.open(); document.write('<p id=fresh>f</p>'); document.close();")
        .expect("open/write/close");
    assert_eq!(read(&mut rt, "!!document.getElementById('fresh')"), "true");
    assert_eq!(
        read(&mut rt, "!!document.getElementById('original')"),
        "false",
        "document.open replaces the document"
    );
    assert_eq!(read(&mut rt, "document.readyState"), "complete");
    assert_eq!(read(&mut rt, "typeof document.writeln"), "function");
}

/// A `<script>` in source written after parsing runs, per the script element's
/// insertion steps — and runs *synchronously*, inside the `document.write` call
/// that wrote it, so the next statement of the calling script sees what it did.
fn a_written_script_runs<E: ScriptEngine>() {
    let mut rt = parse::<E>("<body><p>o</p></body>");
    rt.eval(
        "window.ran = 0;
         document.open();
         document.write('<p id=a>a</p><script>window.ran = 1; window.sawA = !!document.getElementById(\"a\");<\\/script>');
         window.afterWrite = window.ran;
         document.close();",
    )
    .expect("write a script");
    assert_eq!(
        read(&mut rt, "String(window.ran)"),
        "1",
        "the written script ran"
    );
    assert_eq!(
        read(&mut rt, "String(window.afterWrite)"),
        "1",
        "it ran inside document.write, not on a later turn"
    );
    assert_eq!(
        read(&mut rt, "String(window.sawA)"),
        "true",
        "it saw the markup written before it"
    );
}

/// The post-parse open stream **appends**: a node from an earlier write keeps
/// its identity across a later one, and a tag split across two writes is one
/// element. Re-materializing the accumulated source could do neither.
fn the_open_stream_appends<E: ScriptEngine>() {
    let mut rt = parse::<E>("<body><p>o</p></body>");
    rt.eval(
        "document.open();
         document.write('<p id=first>1</p>');
         window.held = document.getElementById('first');
         document.write('<p id=sec');
         document.write('ond>2</p>');
         document.close();",
    )
    .expect("two writes");
    assert_eq!(
        read(
            &mut rt,
            "String(window.held === document.getElementById('first'))"
        ),
        "true",
        "the first write's node survives the second"
    );
    assert_eq!(
        read(&mut rt, "String(!!document.getElementById('second'))"),
        "true",
        "a tag split across two writes is one element"
    );
    assert_eq!(read(&mut rt, "String(window.held.textContent)"), "1");
}

/// A parse-time custom element is upgraded at its **creation**, not at the next
/// script pause: a constructor that looks at the element after it sees an
/// element the parser has not reached yet, exactly as in a browser.
fn a_parsed_element_upgrades_at_creation<E: ScriptEngine>() {
    let mut rt = parse::<E>(
        "<body>
         <script>
           window.seen = [];
           customElements.define('x-at', class extends HTMLElement {
             constructor() {
               super();
               window.seen.push(document.querySelectorAll('x-at').length);
             }
           });
         </script>
         <x-at id=one></x-at><x-at id=two></x-at><x-at id=three></x-at>
         </body>",
    );
    // One constructor per element, each running while only the elements up to
    // and including its own have been parsed.
    assert_eq!(read(&mut rt, "String(window.seen.length)"), "3");
    assert_eq!(
        read(&mut rt, "window.seen.join(',')"),
        "1,2,3",
        "each upgrade runs at creation, before the next element is parsed"
    );
}

/// HTML has the parser process written characters *during* the
/// `document.write` call, so the markup is in the DOM before the next statement
/// of the writing script runs. A whole WPT battery is written in exactly this
/// shape, and queueing the source until the script returns fails all of it.
fn a_write_during_parsing_is_visible_to_its_own_script<E: ScriptEngine>() {
    let mut rt = parse::<E>(
        // The script sits in `head`, as WPT's document-write battery has it,
        // so `document.body.textContent` is the written text and nothing else.
        "<head><script>\
           document.write('PASS');\
           window.sameStatement = document.body.textContent;\
           document.write('<b id=w>x</b>');\
           window.sawElement = !!document.getElementById('w');\
         </script></head><span id=after></span>",
    );
    assert_eq!(read(&mut rt, "window.sameStatement"), "PASS");
    assert_eq!(read(&mut rt, "String(window.sawElement)"), "true");
    // And it still landed at the insertion point, before the following source.
    assert_eq!(
        read(
            &mut rt,
            "document.getElementById('w').nextElementSibling.id"
        ),
        "after"
    );
}

/// A `<script>` in a foreign namespace does not pause html5ever's tokenizer, so
/// it has to be collected at creation and run at the next pause — before the
/// HTML script that may name what it defined.
fn an_svg_script_runs_before_the_next_html_script<E: ScriptEngine>() {
    let mut rt = parse::<E>(
        "<body><svg><script>window.fromSvg = 'svg';</script></svg>         <script>window.seenBySecond = window.fromSvg;</script></body>",
    );
    assert_eq!(read(&mut rt, "String(window.fromSvg)"), "svg");
    assert_eq!(
        read(&mut rt, "String(window.seenBySecond)"),
        "svg",
        "the SVG script must have run before the HTML script that follows it"
    );
}

/// HTML never executes a `<script>` found in a template's contents.
fn a_script_in_template_contents_does_not_run<E: ScriptEngine>() {
    let mut rt = parse::<E>(
        "<body><template><script>window.ranInTemplate = true;</script></template>         <script>window.after = true;</script></body>",
    );
    assert_eq!(read(&mut rt, "String(window.ranInTemplate)"), "undefined");
    assert_eq!(read(&mut rt, "String(window.after)"), "true");
}

/// HTML re-prepares a `<script>` that is not parser-inserted when a node is
/// inserted into it while it is connected. `svg/scripted/script-invalid-script-type.html`
/// is the receipt: an SVG script with `type="text/plain"` is a data block, so
/// preparing it during the parse returns *before* the already-started flag is
/// set; giving it a valid `type` and appending a text node prepares it again,
/// and this time it runs.
fn a_script_with_an_invalid_type_reruns_when_its_type_becomes_valid<E: ScriptEngine>() {
    let mut rt = parse::<E>(
        "<body><svg><script type=\"text/plain\">window.scriptRan = true;</script></svg></body>",
    );
    assert_eq!(
        read(&mut rt, "String(window.scriptRan)"),
        "undefined",
        "a data block does not run during the parse"
    );
    let _ = rt.eval(
        "var s = document.querySelector('svg > script');\
         s.type = 'text/javascript';\
         s.appendChild(document.createTextNode(''));",
    );
    assert_eq!(
        read(&mut rt, "String(window.scriptRan)"),
        "true",
        "the insertion re-prepares the script, and the valid type lets it run"
    );
}

/// A script element created and appended by script runs when it becomes
/// connected — the ordinary `createElement('script')` idiom.
fn a_created_script_runs_when_it_becomes_connected<E: ScriptEngine>() {
    let mut rt = parse::<E>("<body></body>");
    let _ = rt.eval(
        "var s = document.createElement('script');\
         s.textContent = 'window.dynamic = 1;';\
         document.body.appendChild(s);",
    );
    assert_eq!(read(&mut rt, "String(window.dynamic)"), "1");
    // And exactly once: the already-started flag survives a move.
    let _ = rt.eval("window.dynamic = 2; document.head.appendChild(s);");
    assert_eq!(
        read(&mut rt, "String(window.dynamic)"),
        "2",
        "re-inserting an already-started script must not run it again"
    );
}

/// A `<script>` an `innerHTML` write produced must never run. The mechanism is
/// the spec's, not a special case: the fragment parser prepares it in a
/// document with no browsing context, which sets already-started at step 10 and
/// then returns at step 13.
fn an_inner_html_script_never_runs<E: ScriptEngine>() {
    let mut rt = parse::<E>("<body><div id=host></div></body>");
    let _ = rt.eval(
        "document.getElementById('host').innerHTML = '<script>window.injected = 1;<\\/script>';",
    );
    assert_eq!(read(&mut rt, "String(window.injected)"), "undefined");
    // Moving it into the document afterwards does not revive it either.
    let _ = rt.eval("document.body.appendChild(document.querySelector('#host script'));");
    assert_eq!(
        read(&mut rt, "String(window.injected)"),
        "undefined",
        "already-started is what stops it, and it travels with the node"
    );
}

/// `execution-timing/120`: a script created in a document with no browsing
/// context must not run, in that document or after being appended to this one.
/// The write must also leave the active document alone — it used to imply
/// `document.open` on *this* page and wipe it.
fn a_script_created_without_a_browsing_context_never_runs<E: ScriptEngine>() {
    let mut rt = parse::<E>("<body><p id=keep>kept</p></body>");
    let _ = rt.eval(
        "var doc = document.implementation.createHTMLDocument('');\
         doc.write('<script>window.ranWithoutContext = 1;<\\/script>');\
         window.moved = doc.head.firstChild;",
    );
    assert_eq!(
        read(&mut rt, "String(window.ranWithoutContext)"),
        "undefined"
    );
    assert_eq!(
        read(&mut rt, "String(document.getElementById('keep') && 1)"),
        "1",
        "a write into another document must not replace this one"
    );
    let _ = rt.eval("document.body.appendChild(window.moved);");
    assert_eq!(
        read(&mut rt, "String(window.ranWithoutContext)"),
        "undefined",
        "the script was already started when it was prepared without a context"
    );
    assert_eq!(
        read(&mut rt, "window.moved.localName"),
        "script",
        "and it really is the script element that moved"
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
    a_script_sees_the_tree_only_as_far_as_itself => (partial_tree_on_boa, partial_tree_on_nova),
    script_order_is_document_order => (script_order_on_boa, script_order_on_nova),
    document_write_lands_at_the_insertion_point => (write_point_on_boa, write_point_on_nova),
    current_script_names_the_running_script => (current_script_on_boa, current_script_on_nova),
    readiness_transitions_fire_in_order => (readiness_on_boa, readiness_on_nova),
    a_definition_upgrades_elements_parsed_after_it => (parse_upgrade_on_boa, parse_upgrade_on_nova),
    a_disabled_definition_refuses_a_declarative_root
        => (disabled_shadow_on_boa, disabled_shadow_on_nova),
    an_ordinary_custom_element_still_gets_its_declarative_root
        => (ordinary_declarative_on_boa, ordinary_declarative_on_nova),
    a_mutation_observer_sees_parser_insertions => (observer_parse_on_boa, observer_parse_on_nova),
    parser_created_nodes_keep_their_wrapper => (wrapper_identity_on_boa, wrapper_identity_on_nova),
    document_write_after_parsing_implies_open => (implied_open_on_boa, implied_open_on_nova),
    a_written_script_runs => (written_script_on_boa, written_script_on_nova),
    a_write_during_parsing_is_visible_to_its_own_script
        => (write_visible_on_boa, write_visible_on_nova),
    the_open_stream_appends => (open_stream_appends_on_boa, open_stream_appends_on_nova),
    a_parsed_element_upgrades_at_creation
        => (upgrade_at_creation_on_boa, upgrade_at_creation_on_nova),
    an_svg_script_runs_before_the_next_html_script
        => (svg_script_on_boa, svg_script_on_nova),
    a_script_in_template_contents_does_not_run
        => (template_script_inert_on_boa, template_script_inert_on_nova),
    a_script_with_an_invalid_type_reruns_when_its_type_becomes_valid
        => (invalid_type_reprepare_on_boa, invalid_type_reprepare_on_nova),
    a_created_script_runs_when_it_becomes_connected
        => (created_script_runs_on_boa, created_script_runs_on_nova),
    an_inner_html_script_never_runs
        => (inner_html_script_inert_on_boa, inner_html_script_inert_on_nova),
    a_script_created_without_a_browsing_context_never_runs
        => (no_browsing_context_on_boa, no_browsing_context_on_nova),
}
