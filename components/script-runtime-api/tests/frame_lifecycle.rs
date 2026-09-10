// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Browsing-context lifecycle: what HTML's iframe removing steps destroy, in
//! what order, and what the parent can still see afterwards.
//!
//! The three cycles matter. One removal proves the teardown runs; three prove it
//! runs *to completion* each time - a realm, a host registration, an arena or a
//! rooting entry that was only mostly released would show up here as a growing
//! host table, a reused realm id, or an arena that never returns to baseline.

use script_engine_api::{MAIN_REALM, RealmId, ScriptEngine};
use script_runtime_api::{NoScriptLoader, Runtime};

fn runtime<E: ScriptEngine>() -> Runtime<E> {
    let mut runtime = Runtime::<E>::new().expect("runtime");
    runtime
        .set_base_url("https://parent.test/page.html")
        .expect("base URL");
    runtime.parse_document_interleaved(
        "<html><head></head><body><div id='slot'></div></body></html>",
        &NoScriptLoader,
    );
    runtime.eval("globalThis.log = [];").expect("log");
    runtime
}

fn read<E: ScriptEngine>(runtime: &mut Runtime<E>, expression: &str) -> String {
    let value = runtime.eval(expression).expect("eval");
    runtime.value_to_string(&value).expect("stringify")
}

/// A child holding one nested grandchild, both recording their unload sequence
/// into the parent's log. `srcdoc` keeps them same-origin, which is what lets a
/// child reach `parent.log` at all.
const ATTACH: &str = concat!(
    "var frame = document.createElement('iframe'); ",
    "frame.srcdoc = '<html><body><iframe id=inner srcdoc=\"<body>leaf</body>\">",
    "</iframe></body></html>'; ",
    "document.getElementById('slot').appendChild(frame);"
);

const INSTRUMENT: &str = concat!(
    "var childWindow = frame.contentWindow; ",
    "var innerWindow = childWindow.document.getElementById('inner').contentWindow; ",
    "function watch(win, tag) { ",
    "  win.addEventListener('pagehide', function() { parent.log.push(tag + ':pagehide'); }); ",
    "  win.addEventListener('unload', function() { parent.log.push(tag + ':unload'); }); ",
    // Far enough out that the teardown task, queued at 0ms when the frame is
    // removed, is unambiguously ahead of it: if this ever fires, the context's
    // timers survived its destruction rather than losing a race.
    "  win.setTimeout(function() { parent.log.push(tag + ':timer-ran'); }, 50); ",
    "} ",
    "watch(childWindow, 'child'); watch(innerWindow, 'inner');"
);

fn realms<E: ScriptEngine>(runtime: &Runtime<E>) -> Vec<RealmId> {
    let mut all: Vec<_> = runtime
        .frame_realms(MAIN_REALM)
        .into_iter()
        .map(|(_, realm)| realm)
        .collect();
    for realm in all.clone() {
        all.extend(runtime.frame_realms(realm).into_iter().map(|(_, r)| r));
    }
    all
}

/// Attach, instrument, remove; three times over. Everything asserted here is
/// per-cycle: the same facts must hold on the third pass as on the first.
fn removing_a_frame_destroys_its_context_and_releases_every_registration<E: ScriptEngine>() {
    let mut runtime = runtime::<E>();
    let baseline_nodes = runtime.host().borrow().dom.live_node_count();
    let mut seen: Vec<RealmId> = Vec::new();

    for cycle in 1..=3 {
        runtime.eval(ATTACH).expect("attach");
        runtime.run_event_loop(100).expect("load");
        runtime.eval(INSTRUMENT).expect("instrument");

        let live = realms(&runtime);
        assert_eq!(
            live.len(),
            2,
            "cycle {cycle}: the child and its nested grandchild are both live"
        );
        for realm in &live {
            assert!(
                !seen.contains(realm),
                "cycle {cycle}: realm {realm} was reused after being discarded - a fresh \
                 insertion must create a fresh browsing context, not revive the old one"
            );
            assert!(runtime.host_in_realm(*realm).is_ok());
        }

        runtime
            .eval(
                "globalThis.savedDocument = childWindow.document;                  globalThis.log = []; frame.remove();",
            )
            .expect("remove");

        // 1. The removing steps run no script. HTML destroys the child navigable
        //    in a queued task, which is what
        //    `dom/nodes/insertion-removing-steps/insertion-removing-steps-iframe`
        //    is checking, so nothing has been dispatched yet.
        assert_eq!(
            read(&mut runtime, "log.join(',')"),
            "",
            "cycle {cycle}: iframe destruction ran script synchronously"
        );
        // ...but the container already has no content navigable.
        assert_eq!(read(&mut runtime, "String(frame.contentWindow)"), "null");
        assert_eq!(read(&mut runtime, "String(window.length)"), "0");
        assert!(realms(&runtime).is_empty(), "cycle {cycle}: live records");

        runtime.run_event_loop(100).expect("drain");

        // 2. Unload order: each document reports `pagehide` then `unload`, and an
        //    ancestor is unloaded before its descendant.
        assert_eq!(
            read(&mut runtime, "log.join(',')"),
            "child:pagehide,child:unload,inner:pagehide,inner:unload",
            "cycle {cycle}: unload sequence"
        );

        // 3. Nothing the destroyed contexts had scheduled runs at all.
        assert!(
            !read(&mut runtime, "log.join(',')").contains("timer-ran"),
            "cycle {cycle}: a destroyed context's timer still ran"
        );

        // 4. What a parent still holding the window sees. `closed` flips; the
        //    document deliberately does not - a Window's document is its
        //    document, and WPT's `document-attribute.window.js` asserts it keeps
        //    answering after the context is discarded.
        assert_eq!(
            read(&mut runtime, "String(childWindow.closed)"),
            "true",
            "cycle {cycle}: a discarded context reports closed"
        );
        assert_eq!(
            read(
                &mut runtime,
                "String(childWindow.document === savedDocument)"
            ),
            "true",
            "cycle {cycle}: a discarded context lost its document"
        );

        // 5. Every host registration, rooting entry and realm is gone.
        for realm in &live {
            assert!(
                runtime.host_in_realm(*realm).is_err(),
                "cycle {cycle}: realm {realm} kept its host registration"
            );
        }
        seen.extend(live);

        // 6. No arena or reflector is left behind: after a collection tick the
        //    parent is back at the node count it started the cycle with.
        runtime
            .eval(
                "frame = undefined; childWindow = undefined;                  innerWindow = undefined; savedDocument = undefined;",
            )
            .expect("drop");
        runtime.collect_garbage();
        assert_eq!(
            runtime.host().borrow().dom.live_node_count(),
            baseline_nodes,
            "cycle {cycle}: the parent arena did not return to its baseline"
        );
    }
}

/// `moveBefore` is the exception HTML carves out: the nested browsing context
/// survives the move, so the same realm is still there afterwards.
fn move_before_preserves_the_nested_context<E: ScriptEngine>() {
    let mut runtime = runtime::<E>();
    runtime
        .eval(
            "var a = document.createElement('div'), b = document.createElement('div'); \
             document.body.appendChild(a); document.body.appendChild(b); \
             var frame = document.createElement('iframe'); frame.srcdoc = '<body>x</body>'; \
             a.appendChild(frame);",
        )
        .expect("attach");
    runtime.run_event_loop(100).expect("load");
    let before = realms(&runtime);
    assert_eq!(before.len(), 1);
    runtime
        .eval("globalThis.held = frame.contentWindow; b.moveBefore(frame, null);")
        .expect("moveBefore");
    assert_eq!(read(&mut runtime, "String(frame.parentNode === b)"), "true");
    assert_eq!(
        realms(&runtime),
        before,
        "moveBefore must not destroy and recreate the nested browsing context"
    );
    assert_eq!(read(&mut runtime, "String(held.closed)"), "false");
    assert_eq!(
        read(&mut runtime, "String(held === frame.contentWindow)"),
        "true",
        "the preserved context keeps the window the parent already held"
    );
}

macro_rules! both_engines {
    ($($body:ident => ($boa:ident, $nova:ident)),* $(,)?) => {
        $(
            #[test]
            fn $boa() { $body::<script_engine_boa::BoaEngine>(); }

            #[cfg(target_pointer_width = "64")]
            #[test]
            fn $nova() { $body::<script_engine_nova::NovaEngine>(); }
        )*
    };
}

both_engines! {
    removing_a_frame_destroys_its_context_and_releases_every_registration =>
        (teardown_releases_everything_on_boa, teardown_releases_everything_on_nova),
    move_before_preserves_the_nested_context =>
        (move_before_preserves_context_on_boa, move_before_preserves_context_on_nova),
}
