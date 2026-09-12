// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! HTML's "navigate", over a browsing context that already has a document.
//!
//! What a navigation keeps is the point: the browsing context, its container
//! element and its `WindowProxy` are the same objects on the other side, and
//! everything else - the realm, the `Window`, the `Document`, the arena - is
//! replaced. Three navigations in a row prove it runs to completion each time
//! rather than leaking a realm or a host registration per pass.

use script_engine_api::{MAIN_REALM, RealmId, ScriptEngine};
use script_runtime_api::{NoScriptLoader, Runtime, ScriptResourceLoader};

struct Documents;

impl ScriptResourceLoader for Documents {
    fn load(&self, url: &str) -> Option<String> {
        let body = |n: &str| {
            format!(
                "<body><p id=mark>{n}</p><script>\
                 window.__page = '{n}';\
                 window.addEventListener('pagehide', function(){{ parent.log.push('{n}:pagehide'); }});\
                 window.addEventListener('unload', function(){{ parent.log.push('{n}:unload'); }});\
                 </script></body>"
            )
        };
        match url {
            "https://parent.test/one.html" => Some(body("one")),
            "https://parent.test/two.html" => Some(body("two")),
            "https://parent.test/three.html" => Some(body("three")),
            "https://parent.test/four.html" => Some(body("four")),
            // A different origin: the parent's view of the frame has to flip to
            // the cross-origin branch of the very same WindowProxy.
            "https://other.test/elsewhere.html" => {
                Some("<body><p id=secret>elsewhere</p></body>".to_owned())
            },
            _ => None,
        }
    }
}

fn runtime<E: ScriptEngine>() -> Runtime<E> {
    let mut runtime = Runtime::<E>::new().expect("runtime");
    runtime
        .set_base_url("https://parent.test/page.html")
        .expect("base URL");
    runtime.set_script_resource_loader(Box::new(Documents));
    runtime.parse_document_interleaved("<html><body></body></html>", &NoScriptLoader);
    runtime
        .eval(
            "globalThis.log = [];\
             var frame = document.createElement('iframe');\
             frame.src = 'https://parent.test/one.html';\
             document.body.appendChild(frame);\
             frame.addEventListener('load', function(){ log.push('load'); });",
        )
        .expect("attach");
    runtime.run_event_loop(100).expect("first load");
    runtime
}

fn read<E: ScriptEngine>(runtime: &mut Runtime<E>, expression: &str) -> String {
    let value = runtime.eval(expression).expect(expression);
    runtime.value_to_string(&value).expect("stringify")
}

fn check<E: ScriptEngine>(runtime: &mut Runtime<E>, expression: &str) {
    assert_eq!(read(runtime, expression), "true", "{expression}");
}

fn child_realms<E: ScriptEngine>(runtime: &Runtime<E>) -> Vec<RealmId> {
    runtime
        .frame_realms(MAIN_REALM)
        .into_iter()
        .map(|(_, realm)| realm)
        .collect()
}

/// Three navigations of one child, driven by `iframe.src`. The identity the
/// parent took before the first one survives all three.
fn a_child_navigates_three_times_and_keeps_its_window_proxy<E: ScriptEngine>() {
    let mut runtime = runtime::<E>();
    let baseline_nodes = runtime.host().borrow().dom.live_node_count();
    runtime
        .eval("globalThis.held = frame.contentWindow; log.length = 0;")
        .expect("hold the proxy");
    check(&mut runtime, "held.__page === 'one'");
    let mut discarded: Vec<RealmId> = Vec::new();

    for (pass, page) in ["two", "three", "four"].iter().enumerate() {
        let before = child_realms(&runtime);
        assert_eq!(before.len(), 1, "pass {pass}: exactly one child realm");
        let outgoing = before[0];
        runtime
            .eval(&format!("frame.src = 'https://parent.test/{page}.html';"))
            .expect("navigate");
        runtime.run_event_loop(100).expect("navigation tasks");

        // 1. One identity across every navigation, and it reads the new
        //    document.
        check(&mut runtime, "held === frame.contentWindow");
        check(&mut runtime, "held === frames[0]");
        check(&mut runtime, &format!("held.__page === '{page}'"));
        check(
            &mut runtime,
            &format!("held.document.getElementById('mark').textContent === '{page}'"),
        );
        // 2. The realm behind it is a new one, and the outgoing realm is gone.
        let after = child_realms(&runtime);
        assert_eq!(after.len(), 1, "pass {pass}: still one child realm");
        assert_ne!(after[0], outgoing, "pass {pass}: the realm was replaced");
        assert!(
            runtime.host_in_realm(outgoing).is_err(),
            "pass {pass}: the outgoing realm {outgoing} was not discarded"
        );
        assert!(
            !discarded.contains(&after[0]),
            "pass {pass}: realm {} was reused",
            after[0]
        );
        discarded.push(outgoing);
    }

    // 3. Unload order, once per navigation, pagehide before unload; and one
    //    `load` on the container element per navigation.
    assert_eq!(
        read(&mut runtime, "log.join(',')"),
        "one:pagehide,one:unload,load,two:pagehide,two:unload,load,three:pagehide,three:unload,load",
        "unload sequence and load count"
    );

    // 4. The arena returns to its baseline once the navigations are over: the
    //    parent holds the proxy, not any of the three documents.
    runtime.run_event_loop(100).expect("settle");
    let live = runtime.host().borrow().dom.live_node_count();
    assert_eq!(
        live, baseline_nodes,
        "the parent arena did not return to baseline after three navigations"
    );
}

/// A fragment-only navigation keeps the document, moves the URL and fires one
/// `hashchange`.
fn a_fragment_navigation_keeps_the_document<E: ScriptEngine>() {
    let mut runtime = runtime::<E>();
    runtime
        .eval(
            "globalThis.held = frame.contentWindow;\
             held.__hash = [];\
             held.addEventListener('hashchange', function(e){ held.__hash.push(e.oldURL + '>' + e.newURL); });",
        )
        .expect("watch");
    let realm = child_realms(&runtime)[0];
    runtime
        .eval("held.location.href = 'https://parent.test/one.html#alpha';")
        .expect("fragment");
    runtime.run_event_loop(20).expect("tasks");
    check(&mut runtime, "held.__page === 'one'");
    check(
        &mut runtime,
        "held.location.href === 'https://parent.test/one.html#alpha'",
    );
    check(&mut runtime, "held.location.hash === '#alpha'");
    check(&mut runtime, "held.__hash.length === 1");
    check(
        &mut runtime,
        "held.__hash[0] === 'https://parent.test/one.html>https://parent.test/one.html#alpha'",
    );
    assert_eq!(
        child_realms(&runtime),
        vec![realm],
        "a fragment navigation replaced the realm"
    );
}

/// `location.assign`, `replace` and `reload`, and the session history they
/// join. `history.length` and `back()` read the browsing context's own list.
fn location_and_history_move_together<E: ScriptEngine>() {
    let mut runtime = runtime::<E>();
    runtime
        .eval("globalThis.held = frame.contentWindow;")
        .expect("hold");
    check(&mut runtime, "held.history.length === 1");
    runtime
        .eval("held.location.assign('https://parent.test/two.html');")
        .expect("assign");
    runtime.run_event_loop(100).expect("tasks");
    check(&mut runtime, "held.__page === 'two'");
    check(&mut runtime, "held.history.length === 2");
    // A replace does not grow the list.
    runtime
        .eval("held.location.replace('https://parent.test/three.html');")
        .expect("replace");
    runtime.run_event_loop(100).expect("tasks");
    check(&mut runtime, "held.__page === 'three'");
    check(&mut runtime, "held.history.length === 2");
    check(
        &mut runtime,
        "held.location.href === 'https://parent.test/three.html'",
    );
}

/// `pushState` / `replaceState` / `popstate`, over the browsing context's
/// session history rather than a per-document list.
fn push_state_and_traversal_fire_popstate<E: ScriptEngine>() {
    let mut runtime = runtime::<E>();
    runtime
        .eval(
            "globalThis.held = frame.contentWindow;\
             held.__pops = [];\
             held.addEventListener('popstate', function(e){ held.__pops.push(JSON.stringify(e.state)); });\
             held.history.pushState({n:1}, '', 'https://parent.test/one.html?a');\
             held.history.pushState({n:2}, '', 'https://parent.test/one.html?b');",
        )
        .expect("push");
    check(&mut runtime, "held.history.length === 3");
    check(
        &mut runtime,
        "JSON.stringify(held.history.state) === '{\"n\":2}'",
    );
    check(
        &mut runtime,
        "held.location.href === 'https://parent.test/one.html?b'",
    );
    runtime.eval("held.history.back();").expect("back");
    runtime.run_event_loop(20).expect("tasks");
    check(
        &mut runtime,
        "JSON.stringify(held.history.state) === '{\"n\":1}'",
    );
    check(&mut runtime, "held.__pops.join(',') === '{\"n\":1}'");
    runtime.eval("held.history.forward();").expect("forward");
    runtime.run_event_loop(20).expect("tasks");
    check(
        &mut runtime,
        "JSON.stringify(held.history.state) === '{\"n\":2}'",
    );
    check(&mut runtime, "held.__pops.length === 2");
    runtime
        .eval("held.history.replaceState({n:9}, '');")
        .expect("replaceState");
    check(&mut runtime, "held.history.length === 3");
    check(
        &mut runtime,
        "JSON.stringify(held.history.state) === '{\"n\":9}'",
    );
}

/// A navigation that changes the frame's origin flips the parent's view to the
/// cross-origin branch of the very same `WindowProxy`.
fn an_origin_change_flips_the_parents_view<E: ScriptEngine>() {
    let mut runtime = runtime::<E>();
    runtime
        .eval("globalThis.held = frame.contentWindow;")
        .expect("hold");
    check(&mut runtime, "held.document !== undefined");
    runtime
        .eval("frame.src = 'https://other.test/elsewhere.html';")
        .expect("navigate");
    runtime.run_event_loop(100).expect("tasks");
    check(&mut runtime, "held === frame.contentWindow");
    check(
        &mut runtime,
        "(function(){try{held.document;return false;}catch(e){return e.name === 'SecurityError';}})()",
    );
    check(&mut runtime, "Object.getPrototypeOf(held) === null");
    check(&mut runtime, "held.self === held");
    check(&mut runtime, "typeof held.postMessage === 'function'");
    check(&mut runtime, "held.closed === false");
}

macro_rules! backend {
    ($name:ident, $engine:ty) => {
        mod $name {
            #[test]
            fn navigates_three_times() {
                super::a_child_navigates_three_times_and_keeps_its_window_proxy::<$engine>();
            }
            #[test]
            fn fragment_keeps_the_document() {
                super::a_fragment_navigation_keeps_the_document::<$engine>();
            }
            #[test]
            fn location_and_history() {
                super::location_and_history_move_together::<$engine>();
            }
            #[test]
            fn push_state_and_popstate() {
                super::push_state_and_traversal_fire_popstate::<$engine>();
            }
            #[test]
            fn origin_change_flips_the_view() {
                super::an_origin_change_flips_the_parents_view::<$engine>();
            }
        }
    };
}
backend!(boa, script_engine_boa::BoaEngine);
#[cfg(target_pointer_width = "64")]
backend!(nova, script_engine_nova::NovaEngine);
