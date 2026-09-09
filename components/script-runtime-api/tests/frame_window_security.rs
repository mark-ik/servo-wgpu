// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Cross-origin reflection and shared-agent timer isolation.
//! https://html.spec.whatwg.org/multipage/nav-history-apis.html#the-windowproxy-exotic-object

use script_engine_api::ScriptEngine;
use script_runtime_api::{NoScriptLoader, Runtime};

fn runtime<E: ScriptEngine>() -> Runtime<E> {
    let mut runtime = Runtime::<E>::new().expect("runtime");
    runtime
        .set_base_url("https://parent.test/")
        .expect("base URL");
    runtime.parse_document_interleaved("<body></body>", &NoScriptLoader);
    runtime
}

fn check<E: ScriptEngine>(runtime: &mut Runtime<E>, source: &str) {
    let result = runtime.eval(source).expect("security fixture");
    assert_eq!(
        runtime.value_to_string(&result).expect("result"),
        "true",
        "{source}"
    );
}

fn foreign_window<E: ScriptEngine>() -> Runtime<E> {
    let mut runtime = runtime::<E>();
    runtime
        .eval(
            r#"
        var frame = document.createElement('iframe');
        frame.setAttribute('sandbox', 'allow-scripts');
        frame.srcdoc = '<body><iframe></iframe><iframe></iframe></body>';
        document.body.appendChild(frame);
        var foreign = frame.contentWindow;
    "#,
        )
        .expect("foreign frame");
    runtime.run_event_loop(100).expect("child tasks");
    runtime
}

fn descriptors_are_own_cached_and_restricted<E: ScriptEngine>() {
    let mut runtime = foreign_window::<E>();
    check(
        &mut runtime,
        r#"
        var first = Object.getOwnPropertyDescriptor(foreign, 'window');
        var second = Object.getOwnPropertyDescriptor(foreign, 'window');
        var post = Object.getOwnPropertyDescriptor(foreign, 'postMessage');
        var location = Object.getOwnPropertyDescriptor(foreign, 'location');
        var href = Object.getOwnPropertyDescriptor(foreign.location, 'href');
        var errors = [];
        try { Object.getOwnPropertyDescriptor(foreign, 'document'); } catch(e) { errors.push(e.name); }
        try { 'document' in foreign; } catch(e) { errors.push(e.name); }
        try { foreign.location.href; } catch(e) { errors.push(e.name); }
        first.get === second.get && first.get.call(foreign) === foreign &&
        first.enumerable === false && first.configurable === true && first.set === undefined &&
        post.value === foreign.postMessage && post.writable === false && post.value.length === 1 &&
        typeof location.get === 'function' && typeof location.set === 'function' &&
        href.get === undefined && typeof href.set === 'function' && href.set.length === 1 &&
        ('window' in foreign) && ('href' in foreign.location) &&
        errors.join(',') === 'SecurityError,SecurityError,SecurityError'
    "#,
    );
}

fn keys_expose_indexes_then_allowlist_then_symbols<E: ScriptEngine>() {
    let mut runtime = foreign_window::<E>();
    check(
        &mut runtime,
        r#"
        var keys = Reflect.ownKeys(foreign);
        var expected = ['0','1','window','self','location','close','closed','focus','blur','frames','length','top','opener','parent','postMessage','then'];
        var names = keys.slice(0, -3);
        var symbols = keys.slice(-3);
        var zero = Object.getOwnPropertyDescriptor(foreign, '0');
        var fallback = Object.getOwnPropertyDescriptor(foreign, 'then');
        var locNames = Object.getOwnPropertyNames(foreign.location);
        names.join(',') === expected.join(',') && Object.keys(foreign).join(',') === '0,1' &&
        symbols[0] === Symbol.toStringTag && symbols[1] === Symbol.hasInstance && symbols[2] === Symbol.isConcatSpreadable &&
        zero.value === foreign[0] && zero.writable === false && zero.enumerable === true && zero.configurable === true &&
        fallback.value === undefined && fallback.writable === false && fallback.enumerable === false && fallback.configurable === true &&
        locNames.join(',') === 'href,replace,then'
    "#,
    );
}

fn prototypes_extensibility_and_mutation_obey_exotic_rules<E: ScriptEngine>() {
    let mut runtime = foreign_window::<E>();
    check(
        &mut runtime,
        r#"
        var objects = [foreign, foreign.location], okay = true;
        for (var i = 0; i < objects.length; i++) {
            var object = objects[i], failures = [];
            okay = okay && Object.getPrototypeOf(object) === null && Object.isExtensible(object);
            okay = okay && Reflect.setPrototypeOf(object, null) && !Reflect.setPrototypeOf(object, {});
            okay = okay && !Reflect.preventExtensions(object) && Object.isExtensible(object);
            try { Object.preventExtensions(object); } catch(e) { failures.push(e.name); }
            try { Object.setPrototypeOf(object, {}); } catch(e) { failures.push(e.name); }
            try { Object.defineProperty(object, 'secret', {value:1}); } catch(e) { failures.push(e.name); }
            try { Reflect.deleteProperty(object, 'then'); } catch(e) { failures.push(e.name); }
            okay = okay && failures.join(',') === 'TypeError,TypeError,SecurityError,SecurityError';
        }
        okay
    "#,
    );
}

fn timer_queue_never_enters_authored_array_methods<E: ScriptEngine>() {
    let mut runtime = runtime::<E>();
    runtime.eval(r#"
        var leaked = false, messages = [];
        addEventListener('message', function(event) { messages.push(event.data); });
        var originalPush = Array.prototype.push, originalSplice = Array.prototype.splice;
        var apply = Reflect.apply;
        function inspect(candidate) {
            if (candidate && typeof candidate === 'object' && typeof candidate.cb === 'function') {
                if (candidate.global && candidate.global !== window) leaked = true;
                if (candidate.cb.constructor !== Function) leaked = true;
            }
        }
        Array.prototype.push = function() {
            for (var i = 0; i < arguments.length; i++) inspect(arguments[i]);
            return apply(originalPush, this, arguments);
        };
        Array.prototype.splice = function() {
            for (var i = 0; i < this.length; i++) inspect(this[i]);
            return apply(originalSplice, this, arguments);
        };
        var frame = document.createElement('iframe');
        frame.setAttribute('sandbox', 'allow-scripts');
        frame.srcdoc = '<script>setTimeout(function(){ parent.postMessage(this === window ? "window-this" : "wrong-this", "*"); }, 0);</script>';
        document.body.appendChild(frame);
    "#).expect("array interception fixture");
    runtime.run_event_loop(100).expect("timer tasks");
    runtime
        .eval("Array.prototype.push = originalPush; Array.prototype.splice = originalSplice;")
        .expect("restore methods");
    check(
        &mut runtime,
        "!leaked && messages.join(',') === 'window-this'",
    );
}

macro_rules! backend {
    ($module:ident, $engine:ty) => {
        mod $module {
            #[test]
            fn descriptors_are_own_cached_and_restricted() {
                super::descriptors_are_own_cached_and_restricted::<$engine>();
            }
            #[test]
            fn keys_expose_indexes_then_allowlist_then_symbols() {
                super::keys_expose_indexes_then_allowlist_then_symbols::<$engine>();
            }
            #[test]
            fn prototypes_extensibility_and_mutation_obey_exotic_rules() {
                super::prototypes_extensibility_and_mutation_obey_exotic_rules::<$engine>();
            }
            #[test]
            fn timer_queue_never_enters_authored_array_methods() {
                super::timer_queue_never_enters_authored_array_methods::<$engine>();
            }
        }
    };
}

backend!(boa, script_engine_boa::BoaEngine);
#[cfg(target_pointer_width = "64")]
backend!(nova, script_engine_nova::NovaEngine);
