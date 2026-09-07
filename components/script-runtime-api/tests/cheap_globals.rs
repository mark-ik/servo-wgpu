// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The "cheap globals" lane: `performance` (hr-time, user timing, the
//! performance timeline), `queueMicrotask`, `structuredClone`, `MessageChannel`
//! / `MessagePort` / `BroadcastChannel`, `crypto`, and the named constructors
//! (`Image` / `Option` / `Audio`). Each body runs against both backends.

use script_engine_api::ScriptEngine;
use script_runtime_api::Runtime;

/// Evaluate `expr` and read its value back as a string.
fn read<E: ScriptEngine>(rt: &mut Runtime<E>, expr: &str) -> String {
    let v = rt.eval(expr).expect("eval");
    rt.value_to_string(&v).expect("stringify")
}

// ── performance ──────────────────────────────────────────────────────────────

fn performance_clock_works<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    assert_eq!(read(&mut rt, "typeof performance"), "object");
    assert_eq!(read(&mut rt, "typeof performance.now()"), "number");
    // Positive and never decreasing, which hr-time asserts directly.
    assert_eq!(read(&mut rt, "String(performance.now() > 0)"), "true");
    assert_eq!(
        read(
            &mut rt,
            "String((function(){var a=performance.now(),b=performance.now();return b-a>=0;})())"
        ),
        "true"
    );
    // timeOrigin is wall-clock-ish, so Date.now() is not before it.
    assert_eq!(
        read(&mut rt, "String(Date.now() + 30 >= performance.timeOrigin)"),
        "true"
    );
    // Performance is an EventTarget.
    rt.eval(
        "var got = false; \
         performance.addEventListener('x', function(){ got = true; }); \
         performance.dispatchEvent(new Event('x'));",
    )
    .expect("dispatch");
    assert_eq!(read(&mut rt, "String(got)"), "true");
    // toJSON carries timeOrigin plus the legacy timing / navigation shapes.
    assert_eq!(
        read(
            &mut rt,
            "String(performance.toJSON().timeOrigin === performance.timeOrigin && \
             performance.toJSON().timing.navigationStart === performance.timing.navigationStart && \
             performance.toJSON().navigation.type === performance.navigation.type)"
        ),
        "true"
    );
    // The clock is the timers' clock: firing a 50ms timer advances now().
    rt.eval("var before = performance.now(); var after = 0; setTimeout(function(){ after = performance.now(); }, 50);")
        .expect("schedule");
    rt.run_event_loop(10).expect("drain");
    assert_eq!(read(&mut rt, "String(after - before >= 50)"), "true");
}

fn user_timing_works<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.eval(
        "performance.mark('a'); performance.mark('a'); \
         var m = performance.mark('b', { startTime: 123, detail: { k: 1 } });",
    )
    .expect("marks");
    assert_eq!(
        read(&mut rt, "String(performance.getEntriesByName('a').length)"),
        "2"
    );
    assert_eq!(read(&mut rt, "String(m.startTime)"), "123");
    assert_eq!(read(&mut rt, "String(m.detail.k)"), "1");
    assert_eq!(read(&mut rt, "String(m.entryType)"), "mark");
    assert_eq!(
        read(&mut rt, "Object.prototype.toString.call(m)"),
        "[object PerformanceMark]"
    );
    // measure with both endpoints, and with only an end mark.
    rt.eval("var e1 = performance.measure('A', undefined, 'b'); var e2 = performance.measure('B', 'b', 'b');")
        .expect("measures");
    assert_eq!(read(&mut rt, "String(e1.startTime)"), "0");
    assert_eq!(read(&mut rt, "String(e1.startTime + e1.duration)"), "123");
    assert_eq!(read(&mut rt, "String(e2.startTime)"), "123");
    assert_eq!(read(&mut rt, "String(e2.duration)"), "0");
    assert_eq!(
        read(&mut rt, "Object.prototype.toString.call(e2)"),
        "[object PerformanceMeasure]"
    );
    // A missing start mark is a SyntaxError; a reserved name cannot be marked.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ try { performance.measure('X', 'nope'); } catch (e) { return e.name; } return 'none'; })()"
        ),
        "SyntaxError"
    );
    assert_eq!(
        read(
            &mut rt,
            "(function(){ try { performance.mark('navigationStart'); } catch (e) { return e.name; } return 'none'; })()"
        ),
        "SyntaxError"
    );
    // clearMarks / clearMeasures, by name and wholesale.
    rt.eval("performance.clearMarks('a');").expect("clear one");
    assert_eq!(
        read(
            &mut rt,
            "String(performance.getEntriesByType('mark').length)"
        ),
        "1"
    );
    rt.eval("performance.clearMarks(); performance.clearMeasures();")
        .expect("clear all");
    assert_eq!(
        read(&mut rt, "String(performance.getEntries().length)"),
        "0"
    );
}

fn performance_observer_works<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.eval(
        "var seen = []; \
         var po = new PerformanceObserver(function(list){ \
           list.getEntries().forEach(function(e){ seen.push(e.entryType + ':' + e.name); }); \
         }); \
         po.observe({ entryTypes: ['mark', 'measure'] }); \
         performance.mark('m1'); performance.measure('me1');",
    )
    .expect("observe");
    // Delivery is a task, not synchronous with mark().
    assert_eq!(read(&mut rt, "String(seen.length)"), "0");
    rt.run_event_loop(10).expect("drain");
    assert_eq!(read(&mut rt, "seen.join(',')"), "measure:me1,mark:m1");

    // takeRecords drains without invoking the callback; disconnect stops delivery.
    rt.eval("performance.mark('m2'); var taken = po.takeRecords();")
        .expect("take");
    assert_eq!(read(&mut rt, "String(taken.length)"), "1");
    rt.run_event_loop(10).expect("drain 2");
    assert_eq!(read(&mut rt, "seen.join(',')"), "measure:me1,mark:m1");
    rt.eval("po.disconnect(); performance.mark('m3');")
        .expect("disconnect");
    rt.run_event_loop(10).expect("drain 3");
    assert_eq!(read(&mut rt, "seen.join(',')"), "measure:me1,mark:m1");

    // The buffered flag replays entries already on the timeline.
    rt.eval(
        "var buffered = []; \
         new PerformanceObserver(function(l){ l.getEntries().forEach(function(e){ buffered.push(e.name); }); }) \
           .observe({ type: 'mark', buffered: true });",
    )
    .expect("buffered");
    rt.run_event_loop(10).expect("drain 4");
    assert_eq!(read(&mut rt, "buffered.join(',')"), "m1,m2,m3");
    assert_eq!(
        read(&mut rt, "PerformanceObserver.supportedEntryTypes.join(',')"),
        "mark,measure"
    );
}

// ── queueMicrotask ───────────────────────────────────────────────────────────

fn queue_microtask_works<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    assert_eq!(read(&mut rt, "typeof queueMicrotask"), "function");
    // Non-callables throw synchronously.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ try { queueMicrotask(); } catch (e) { return e.constructor.name; } return 'none'; })()"
        ),
        "TypeError"
    );
    // It runs asynchronously, interleaved with promise reactions in order.
    rt.eval(
        "var order = []; var ran = false; \
         Promise.resolve().then(function(){ order.push('a'); }); \
         queueMicrotask(function(){ order.push('b'); ran = true; }); \
         Promise.reject(1).catch(function(){ order.push('c'); });",
    )
    .expect("queue");
    assert_eq!(read(&mut rt, "String(ran)"), "false");
    rt.run_microtasks();
    assert_eq!(read(&mut rt, "order.join(',')"), "a,b,c");

    // An exception is reported at the global, not swallowed.
    rt.eval(
        "var reported = null; \
         addEventListener('error', function(e){ reported = e.message; }); \
         queueMicrotask(function(){ throw new Error('boom'); });",
    )
    .expect("throwing microtask");
    rt.run_microtasks();
    assert_eq!(read(&mut rt, "String(reported)"), "boom");
}

// ── structuredClone ──────────────────────────────────────────────────────────

fn structured_clone_works<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    // Primitives pass through by value.
    assert_eq!(
        read(
            &mut rt,
            "[structuredClone(1), structuredClone('s'), String(structuredClone(null)), \
              String(structuredClone(undefined)), String(structuredClone(true))].join(',')"
        ),
        "1,s,null,undefined,true"
    );
    // Date, RegExp.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var d = new Date(1234567); var c = structuredClone(d); \
               return String(c !== d && c instanceof Date && c.getTime() === d.getTime()); })()"
        ),
        "true"
    );
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var r = /ab+c/gi; var c = structuredClone(r); \
               return String(c !== r && c.source === 'ab+c' && c.flags === 'gi'); })()"
        ),
        "true"
    );
    // Map, Set, Array (holes preserved), plain objects.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var m = new Map([[1, 'a']]); var s = new Set(['x']); \
               var c = structuredClone({ m: m, s: s }); \
               return String(c.m.get(1) === 'a' && c.s.has('x') && c.m !== m); })()"
        ),
        "true"
    );
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var a = [1, , 3]; a.extra = 'e'; var c = structuredClone(a); \
               return String(c.length === 3 && !(1 in c) && c[2] === 3 && c.extra === 'e'); })()"
        ),
        "true"
    );
    // ArrayBuffer + a view over it, sharing the cloned buffer.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var b = new Uint8Array([1,2,3]); var c = structuredClone({ v: b, buf: b.buffer }); \
               return String(c.v[1] === 2 && c.v.buffer === c.buf && c.v.buffer !== b.buffer); })()"
        ),
        "true"
    );
    // Error types keep their name and message.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var c = structuredClone(new TypeError('bad')); \
               return c.name + ':' + c.message + ':' + String(c instanceof TypeError); })()"
        ),
        "TypeError:bad:true"
    );
    // Blob / File round-trip their bytes and metadata.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var f = new File(['hey'], 'n.txt', { type: 'text/plain' }); \
               var c = structuredClone(f); \
               return String(c instanceof File && c.name === 'n.txt' && c.type === 'text/plain' && c.size === 3); })()"
        ),
        "true"
    );
    // Cycles and shared references are preserved, not duplicated or hung on.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var o = { n: 1 }; o.self = o; var shared = { k: 2 }; o.a = shared; o.b = shared; \
               var c = structuredClone(o); \
               return String(c.self === c && c.a === c.b && c.a !== shared); })()"
        ),
        "true"
    );
    // Functions, symbols and platform objects are DataCloneError.
    for expr in [
        "structuredClone(function(){})",
        "structuredClone(Symbol('s'))",
        "structuredClone(document.createElement('div'))",
        "structuredClone(new Event('x'))",
    ] {
        let js = format!(
            "(function(){{ try {{ {expr}; }} catch (e) {{ return e.name; }} return 'none'; }})()"
        );
        assert_eq!(read(&mut rt, &js), "DataCloneError", "for {expr}");
    }
    // An Error's cause travels; a custom property and the prototype chain do not.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var e = new RangeError('m', { cause: 'why' }); e.foo = 1;                var c = structuredClone(e);                return String(c.cause === 'why' && c.foo === undefined &&                  !Object.prototype.hasOwnProperty.call(structuredClone(new Error()), 'message')); })()"
        ),
        "true"
    );
    // An ordinary object with a custom prototype clones as a plain object.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ function Foo(){ this.own = 1; } Foo.prototype = { inherited: 2 };                var c = structuredClone(new Foo());                return String(c.own === 1 && !('inherited' in c)); })()"
        ),
        "true"
    );
    // A transferred ArrayBuffer moves rather than copying.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var b = new ArrayBuffer(8); var c = structuredClone(b, { transfer: [b] }); \
               return String(c.byteLength === 8 && b.byteLength === 0); })()"
        ),
        "true"
    );
}

// ── messaging ────────────────────────────────────────────────────────────────

fn message_channel_works<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.eval(
        "var log = []; var mc = new MessageChannel(); \
         mc.port2.onmessage = function(e){ log.push('p2:' + e.data.v); }; \
         mc.port1.postMessage({ v: 7 });",
    )
    .expect("post");
    // Delivery is a task, not synchronous.
    assert_eq!(read(&mut rt, "String(log.length)"), "0");
    rt.run_event_loop(10).expect("drain");
    assert_eq!(read(&mut rt, "log.join(',')"), "p2:7");
    // The payload is cloned, not shared.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var got; var c = new MessageChannel(); var o = { a: 1 }; \
               c.port2.onmessage = function(e){ got = e.data; }; c.port1.postMessage(o); \
               return String(got === undefined); })()"
        ),
        "true"
    );
    // A port not started buffers, then flushes on start().
    rt.eval(
        "var held = []; var c2 = new MessageChannel(); \
         c2.port2.addEventListener('message', function(e){ held.push(e.data); }); \
         c2.port1.postMessage('one');",
    )
    .expect("buffered");
    rt.run_event_loop(10).expect("drain 2");
    assert_eq!(read(&mut rt, "String(held.length)"), "0");
    rt.eval("c2.port2.start();").expect("start");
    rt.run_event_loop(10).expect("drain 3");
    assert_eq!(read(&mut rt, "held.join(',')"), "one");
    // A port in the transfer list arrives on event.ports and stays entangled.
    rt.eval(
        "var relayed = []; var a = new MessageChannel(), b = new MessageChannel(); \
         a.port2.onmessage = function(e){ \
           var p = e.ports[0]; relayed.push('ports:' + e.ports.length); \
           p.onmessage = function(ev){ relayed.push('via:' + ev.data); }; \
         }; \
         a.port1.postMessage('carry', [b.port2]);",
    )
    .expect("transfer port");
    rt.run_event_loop(20).expect("drain 4");
    rt.eval("b.port1.postMessage('hi');").expect("through");
    rt.run_event_loop(20).expect("drain 5");
    assert_eq!(read(&mut rt, "relayed.join(',')"), "ports:1,via:hi");
    // A port passed by value (not transferred) is not cloneable.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var c = new MessageChannel(); \
               try { c.port1.postMessage(c.port2); } catch (e) { return e.name; } return 'none'; })()"
        ),
        "DataCloneError"
    );
    // close() stops delivery.
    rt.eval(
        "var after = []; var c3 = new MessageChannel(); \
         c3.port2.onmessage = function(e){ after.push(e.data); }; \
         c3.port2.close(); c3.port1.postMessage('x');",
    )
    .expect("close");
    rt.run_event_loop(10).expect("drain 6");
    assert_eq!(read(&mut rt, "String(after.length)"), "0");
}

fn broadcast_channel_works<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.eval(
        "var got = []; \
         var a = new BroadcastChannel('room'); \
         var b = new BroadcastChannel('room'); \
         var c = new BroadcastChannel('other'); \
         a.onmessage = function(e){ got.push('a:' + e.data); }; \
         b.onmessage = function(e){ got.push('b:' + e.data); }; \
         c.onmessage = function(e){ got.push('c:' + e.data); }; \
         a.postMessage('hello');",
    )
    .expect("broadcast");
    rt.run_event_loop(10).expect("drain");
    // The sender does not receive its own message; a different name does not either.
    assert_eq!(read(&mut rt, "got.join(',')"), "b:hello");
    assert_eq!(read(&mut rt, "a.name"), "room");
    rt.eval("b.close(); a.postMessage('again');")
        .expect("closed");
    rt.run_event_loop(10).expect("drain 2");
    assert_eq!(read(&mut rt, "got.join(',')"), "b:hello");
    assert_eq!(
        read(
            &mut rt,
            "(function(){ try { b.postMessage('x'); } catch (e) { return e.name; } return 'none'; })()"
        ),
        "InvalidStateError"
    );
}

fn window_post_message_works<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.eval(
        "var ev = null; \
         addEventListener('message', function(e){ ev = e; }); \
         postMessage({ n: 5 }, '*');",
    )
    .expect("post");
    rt.run_event_loop(10).expect("drain");
    assert_eq!(
        read(
            &mut rt,
            "String(ev instanceof MessageEvent) + ',' + String(ev.data.n) + ',' + String(ev.source === window) + ',' + String(ev.ports.length)"
        ),
        "true,5,true,0"
    );
    // An `undefined` payload stays undefined: the IDL default (absent -> null)
    // applies to a script-constructed MessageEvent, not to an internal delivery.
    rt.eval("var u = 'unset'; addEventListener('message', function(e){ u = typeof e.data; }); postMessage(undefined, '*');")
        .expect("post undefined");
    rt.run_event_loop(10).expect("drain 2");
    assert_eq!(read(&mut rt, "u"), "undefined");
    // A constructed MessageEvent still defaults its data to null.
    assert_eq!(
        read(&mut rt, "String(new MessageEvent('message').data)"),
        "null"
    );
}

// ── crypto ───────────────────────────────────────────────────────────────────

fn crypto_works<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    assert_eq!(read(&mut rt, "typeof crypto"), "object");
    assert_eq!(read(&mut rt, "typeof crypto.getRandomValues"), "function");
    // Fills in place and returns the same view.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var a = new Uint8Array(32); var r = crypto.getRandomValues(a); \
               var nonzero = 0; for (var i = 0; i < a.length; i++) if (a[i] !== 0) nonzero++; \
               return String(r === a && nonzero > 20); })()"
        ),
        "true"
    );
    // Two draws differ.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var a = new Uint32Array(8), b = new Uint32Array(8); \
               crypto.getRandomValues(a); crypto.getRandomValues(b); \
               return String(a.join(',') !== b.join(',')); })()"
        ),
        "true"
    );
    // Float views and oversized views are rejected the way Web Crypto says.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ try { crypto.getRandomValues(new Float32Array(4)); } catch (e) { return e.name; } return 'none'; })()"
        ),
        "TypeMismatchError"
    );
    assert_eq!(
        read(
            &mut rt,
            "(function(){ try { crypto.getRandomValues(new Uint8Array(65537)); } catch (e) { return e.name; } return 'none'; })()"
        ),
        "QuotaExceededError"
    );
    // randomUUID is a well-formed, unique version 4 UUID.
    assert_eq!(
        read(
            &mut rt,
            "String(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(crypto.randomUUID()))"
        ),
        "true"
    );
    assert_eq!(
        read(
            &mut rt,
            "String(crypto.randomUUID() !== crypto.randomUUID())"
        ),
        "true"
    );
}

// ── named constructors ───────────────────────────────────────────────────────

fn named_constructors_work<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    assert_eq!(
        read(
            &mut rt,
            "[typeof Image, typeof Option, typeof Audio].join(',')"
        ),
        "function,function,function"
    );
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var i = new Image(); return i.localName + ':' + String(i instanceof HTMLImageElement); })()"
        ),
        "img:true"
    );
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var o = new Option('t', 'v'); return o.localName + ':' + o.textContent + ':' + o.value; })()"
        ),
        "option:t:v"
    );
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var a = new Audio(); return a.localName + ':' + String(a instanceof HTMLAudioElement); })()"
        ),
        "audio:true"
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
    performance_clock_works => (performance_clock_on_boa, performance_clock_on_nova),
    user_timing_works => (user_timing_on_boa, user_timing_on_nova),
    performance_observer_works => (performance_observer_on_boa, performance_observer_on_nova),
    queue_microtask_works => (queue_microtask_on_boa, queue_microtask_on_nova),
    structured_clone_works => (structured_clone_on_boa, structured_clone_on_nova),
    message_channel_works => (message_channel_on_boa, message_channel_on_nova),
    broadcast_channel_works => (broadcast_channel_on_boa, broadcast_channel_on_nova),
    window_post_message_works => (window_post_message_on_boa, window_post_message_on_nova),
    crypto_works => (crypto_on_boa, crypto_on_nova),
    named_constructors_works_alias => (named_constructors_on_boa, named_constructors_on_nova),
}

fn named_constructors_works_alias<E: ScriptEngine>() {
    named_constructors_work::<E>();
}
