// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The browsing context's `WindowProxy` is the realm's global `this`, at every
//! level, and every name that denotes a window answers with it.

use script_engine_api::ScriptEngine;
use script_runtime_api::{NoScriptLoader, Runtime, ScriptResourceLoader};

struct Documents;

impl ScriptResourceLoader for Documents {
    fn load(&self, url: &str) -> Option<String> {
        match url {
            "https://parent.test/child.html" => Some(
                "<body><p id=child>same origin</p><script>\
                 globalThis.__seen = {\
                   windowIsGlobalThis: window === globalThis,\
                   selfIsWindow: self === window,\
                   framesIsWindow: frames === window,\
                   topIsParent: top === parent,\
                 };\
                 </script></body>"
                    .to_owned(),
            ),
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
}

fn check<E: ScriptEngine>(runtime: &mut Runtime<E>, expression: &str) {
    let value = runtime.eval(expression).expect("assertion");
    let result = runtime.value_to_string(&value).expect("assertion result");
    assert_eq!(result, "true", "{expression}");
}

fn top_level_window_is_the_global_this<E: ScriptEngine>() {
    let mut runtime = runtime::<E>();
    check(&mut runtime, "globalThis === window");
    check(&mut runtime, "self === window");
    check(&mut runtime, "frames === window");
    check(&mut runtime, "top === window");
    check(&mut runtime, "parent === window");
    check(&mut runtime, "(function () { return this; })() === window");
    // The proxy forwards, so the Window's own surface is intact through it.
    check(&mut runtime, "typeof window.document === 'object'");
    check(&mut runtime, "window.document === document");
    check(
        &mut runtime,
        "(window.__probe = 7, globalThis.__probe === 7 && window.__probe === 7)",
    );
    check(&mut runtime, "'document' in window");
    check(&mut runtime, "Object.keys(window).indexOf('__probe') >= 0");
    check(
        &mut runtime,
        "delete window.__probe, window.__probe === undefined",
    );
    check(
        &mut runtime,
        "Object.getPrototypeOf(window) === Object.getPrototypeOf(globalThis)",
    );
}

fn a_child_frames_window_is_one_object_everywhere<E: ScriptEngine>() {
    let mut runtime = runtime::<E>();
    runtime
        .eval(
            "var frame = document.createElement('iframe');\
             frame.src = 'https://parent.test/child.html';\
             document.body.appendChild(frame);",
        )
        .expect("insert frame");
    runtime.run_event_loop(100).expect("frame tasks");
    runtime.run_event_loop(100).expect("frame tasks");
    check(&mut runtime, "frame.contentWindow === frames[0]");
    check(&mut runtime, "frame.contentWindow === window[0]");
    check(
        &mut runtime,
        "frame.contentWindow === frame.contentWindow.window",
    );
    check(
        &mut runtime,
        "frame.contentWindow.self === frame.contentWindow",
    );
    check(&mut runtime, "frame.contentWindow.parent === window");
    check(&mut runtime, "frame.contentWindow.top === window");
    check(&mut runtime, "frame.contentWindow !== window");
    // What the child itself saw, from inside its own realm.
    check(
        &mut runtime,
        "frame.contentWindow.__seen.windowIsGlobalThis",
    );
    check(&mut runtime, "frame.contentWindow.__seen.selfIsWindow");
    check(&mut runtime, "frame.contentWindow.__seen.framesIsWindow");
    check(&mut runtime, "frame.contentWindow.__seen.topIsParent");
}

#[test]
fn top_level_window_is_the_global_this_on_boa() {
    top_level_window_is_the_global_this::<script_engine_boa::BoaEngine>();
}

#[test]
fn top_level_window_is_the_global_this_on_nova() {
    top_level_window_is_the_global_this::<script_engine_nova::NovaEngine>();
}

#[test]
fn a_child_frames_window_is_one_object_everywhere_on_boa() {
    a_child_frames_window_is_one_object_everywhere::<script_engine_boa::BoaEngine>();
}

#[test]
fn a_child_frames_window_is_one_object_everywhere_on_nova() {
    a_child_frames_window_is_one_object_everywhere::<script_engine_nova::NovaEngine>();
}

/// The slot the native traps resolve the handler through lives on the global
/// object, and no operation on the proxy admits it.
fn the_handler_slot_is_invisible<E: ScriptEngine>() {
    let mut runtime = runtime::<E>();
    check(&mut runtime, "window.__windowProxyHandler === undefined");
    check(&mut runtime, "!('__windowProxyHandler' in window)");
    check(
        &mut runtime,
        "Object.getOwnPropertyNames(window).indexOf('__windowProxyHandler') < 0",
    );
    check(
        &mut runtime,
        "Object.getOwnPropertyDescriptor(window, '__windowProxyHandler') === undefined",
    );
}

#[test]
fn the_handler_slot_is_invisible_on_boa() {
    the_handler_slot_is_invisible::<script_engine_boa::BoaEngine>();
}

#[test]
fn the_handler_slot_is_invisible_on_nova() {
    the_handler_slot_is_invisible::<script_engine_nova::NovaEngine>();
}
