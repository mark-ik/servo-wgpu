// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The dedicated-worker lane: the transportable clone record, a second agent of
//! the same engine on its own thread, `Worker` on the page, and `MessagePort`
//! across the thread boundary. Each body runs against both backends.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use script_engine_api::ScriptEngine;
use script_runtime_api::{FetchOutcome, FetchRequest, Runtime, ScriptResourceLoader};

/// A fixed script route: the disk loader's shape, without a disk.
struct Scripts(Vec<(String, String)>);

impl ScriptResourceLoader for Scripts {
    fn load(&self, url: &str) -> Option<String> {
        self.0
            .iter()
            .find(|(name, _)| name == url)
            .map(|(_, src)| src.clone())
    }
}

struct DeferredScripts;

impl ScriptResourceLoader for DeferredScripts {
    fn load(&self, _url: &str) -> Option<String> {
        None
    }

    fn start(
        &self,
        _id: u64,
        request: FetchRequest,
        complete: Box<dyn FnOnce(FetchOutcome) + Send>,
    ) -> Option<FetchRequest> {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5));
            complete(FetchOutcome {
                network_error: false,
                status: 200,
                status_text: "OK".to_owned(),
                response_type: "basic".to_owned(),
                url: request.url,
                redirected: false,
                headers: vec![("content-type".to_owned(), "text/javascript".to_owned())],
                body: b"postMessage('deferred')".to_vec(),
            });
        });
        None
    }
}

fn scripts(pairs: &[(&str, &str)]) -> Box<Scripts> {
    Box::new(Scripts(
        pairs
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect(),
    ))
}

/// Evaluate `expr` and read its value back as a string.
fn read<E: ScriptEngine>(rt: &mut Runtime<E>, expr: &str) -> String {
    let v = rt.eval(expr).expect("eval");
    rt.value_to_string(&v).expect("stringify")
}

/// Drive the page's event loop until `done` reads `"true"`, or give up. The
/// shape genet-wpt's disk drive loop uses, minus the rendering session.
fn drive_until<E: ScriptEngine>(rt: &mut Runtime<E>, done: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut now_ms = 0.0f64;
    while Instant::now() < deadline {
        rt.run_microtasks();
        let fired = rt.run_timers(64, now_ms);
        let worked = rt.pump_workers();
        if read(rt, done) == "true" {
            return true;
        }
        if let Some(d) = rt.next_timer_delay() {
            now_ms += d.max(0.0);
        }
        if fired == 0 && worked == 0 {
            if !rt.has_worker_work() && rt.next_timer_delay().is_none() {
                // Quiescent: nothing can change without new input.
                return read(rt, done) == "true";
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    false
}

// ── the transportable record ─────────────────────────────────────────────────

fn wire_record_round_trips<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    // Cycles, shared subgraphs, holes, Map/Set, Date/RegExp, boxed primitives,
    // typed-array aliasing over one buffer, and an Error.
    rt.eval(
        "var shared = { k: 1 }; \
         var v = { a: shared, b: shared, d: new Date(1234567), r: /ab+c/gi, \
                   m: new Map([['x', 1], ['y', shared]]), s: new Set([1, 'two']), \
                   n: new Number(7), bo: new Boolean(true), st: new String('s'), \
                   arr: [1, , 3], buf: new Uint8Array([1, 2, 3]), \
                   err: new TypeError('boom'), big: 1, neg: -0, inf: Infinity, nan: NaN }; \
         v.arr.extra = 'x'; v.self = v; \
         var wire = __scSerialize(v); var out = __scDeserialize(wire);",
    )
    .expect("round trip");
    assert_eq!(read(&mut rt, "typeof wire"), "string");
    // Identity inside the graph survives, and the cycle comes back a cycle.
    assert_eq!(read(&mut rt, "String(out.a === out.b)"), "true");
    assert_eq!(read(&mut rt, "String(out.self === out)"), "true");
    assert_eq!(read(&mut rt, "String(out.a !== shared)"), "true");
    assert_eq!(read(&mut rt, "String(out.m.get('y') === out.a)"), "true");
    assert_eq!(read(&mut rt, "String(out.d.getTime())"), "1234567");
    assert_eq!(read(&mut rt, "out.r.source + '/' + out.r.flags"), "ab+c/gi");
    assert_eq!(read(&mut rt, "String(out.s.has('two'))"), "true");
    assert_eq!(read(&mut rt, "String(out.n.valueOf())"), "7");
    assert_eq!(read(&mut rt, "String(out.bo.valueOf())"), "true");
    assert_eq!(read(&mut rt, "out.st.valueOf()"), "s");
    assert_eq!(read(&mut rt, "String(1 in out.arr)"), "false"); // the hole is a hole
    assert_eq!(read(&mut rt, "out.arr.extra"), "x");
    assert_eq!(read(&mut rt, "String(out.buf[2])"), "3");
    assert_eq!(
        read(&mut rt, "out.err.name + ': ' + out.err.message"),
        "TypeError: boom"
    );
    assert_eq!(read(&mut rt, "String(1 / out.neg)"), "-Infinity");
    assert_eq!(read(&mut rt, "String(out.inf)"), "Infinity");
    assert_eq!(read(&mut rt, "String(out.nan !== out.nan)"), "true");
    // Two views over one buffer stay two views over one buffer.
    rt.eval(
        "var b = new ArrayBuffer(8); var u = new Uint8Array(b); var f = new Float64Array(b); \
         u[0] = 9; var two = __scDeserialize(__scSerialize({ u: u, f: f }));",
    )
    .expect("aliasing");
    assert_eq!(
        read(&mut rt, "String(two.u.buffer === two.f.buffer)"),
        "true"
    );
    assert_eq!(read(&mut rt, "String(two.u[0])"), "9");
    // Uncloneable values are rejected on the wire exactly as in-agent.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ try { __scSerialize(function(){}); return 'no'; } \
             catch (e) { return e.name; } })()"
        ),
        "DataCloneError"
    );
}

// ── a second agent ───────────────────────────────────────────────────────────

fn worker_round_trip_works<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_script_resource_loader(scripts(&[(
        "double.js",
        "onmessage = function(e) { postMessage({ n: e.data.n * 2, kind: typeof e.data }); };",
    )]));
    rt.eval(
        "var got = null; \
         var w = new Worker('double.js'); \
         w.onmessage = function(e) { got = e.data; }; \
         w.postMessage({ n: 21 });",
    )
    .expect("start");
    assert!(drive_until(&mut rt, "String(got !== null)"), "no reply");
    assert_eq!(read(&mut rt, "String(got.n)"), "42");
    assert_eq!(read(&mut rt, "got.kind"), "object");
}

fn worker_scope_has_no_document<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_script_resource_loader(scripts(&[(
        "scope.js",
        "postMessage({ \
           doc: typeof document, win: typeof window,            inGlobal: ('document' in globalThis),            winInGlobal: ('window' in globalThis), \
           isScope: self instanceof DedicatedWorkerGlobalScope, \
           isWorkerScope: self instanceof WorkerGlobalScope, \
           name: self.name, \
           hasSelf: self === globalThis, \
           timers: typeof setTimeout, \
           clone: typeof structuredClone, \
           perf: typeof performance.now(), \
           crypto: typeof crypto.getRandomValues, \
           console: typeof console.log, \
           fetch: typeof fetch, \
           importScripts: typeof importScripts, \
           close: typeof close, \
           nav: typeof navigator.userAgent \
         });",
    )]));
    rt.eval(
        "var got = null; var w = new Worker('scope.js', { name: 'w1' }); \
         w.onmessage = function(e) { got = e.data; };",
    )
    .expect("start");
    assert!(drive_until(&mut rt, "String(got !== null)"), "no reply");
    assert_eq!(read(&mut rt, "got.doc"), "undefined");
    assert_eq!(read(&mut rt, "String(got.inGlobal)"), "false");
    assert_eq!(read(&mut rt, "got.win"), "undefined");
    assert_eq!(read(&mut rt, "String(got.winInGlobal)"), "false");
    assert_eq!(read(&mut rt, "String(got.isScope)"), "true");
    assert_eq!(read(&mut rt, "String(got.isWorkerScope)"), "true");
    assert_eq!(read(&mut rt, "got.name"), "w1");
    assert_eq!(read(&mut rt, "String(got.hasSelf)"), "true");
    assert_eq!(read(&mut rt, "got.timers"), "function");
    assert_eq!(read(&mut rt, "got.clone"), "function");
    assert_eq!(read(&mut rt, "got.perf"), "number");
    assert_eq!(read(&mut rt, "got.crypto"), "function");
    assert_eq!(read(&mut rt, "got.console"), "function");
    assert_eq!(read(&mut rt, "got.fetch"), "function");
    assert_eq!(read(&mut rt, "got.importScripts"), "function");
    assert_eq!(read(&mut rt, "got.close"), "function");
    assert_eq!(read(&mut rt, "got.nav"), "string");
}

fn worker_import_scripts_works<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_script_resource_loader(scripts(&[
        ("helper.js", "var HELPER = 'helped';"),
        (
            "main.js",
            "importScripts('helper.js'); \
             var missing = ''; \
             try { importScripts('nope.js'); } catch (e) { missing = e.name; } \
             postMessage({ helper: HELPER, missing: missing });",
        ),
    ]));
    rt.eval("var got = null; var w = new Worker('main.js'); w.onmessage = function(e) { got = e.data; };")
        .expect("start");
    assert!(drive_until(&mut rt, "String(got !== null)"), "no reply");
    assert_eq!(read(&mut rt, "got.helper"), "helped");
    assert_eq!(read(&mut rt, "got.missing"), "NetworkError");
}

fn worker_timers_and_ordering_work<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_script_resource_loader(scripts(&[(
        "order.js",
        "var seen = []; \
         onmessage = function(e) { \
           seen.push(e.data); \
           if (seen.length === 3) setTimeout(function() { postMessage(seen.join(',')); }, 5); \
         };",
    )]));
    rt.eval(
        "var got = null; var w = new Worker('order.js'); \
         w.onmessage = function(e) { got = e.data; }; \
         w.postMessage('a'); w.postMessage('b'); w.postMessage('c');",
    )
    .expect("start");
    assert!(drive_until(&mut rt, "String(got !== null)"), "no reply");
    assert_eq!(read(&mut rt, "got"), "a,b,c");
}

fn worker_error_reaches_the_page<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_script_resource_loader(scripts(&[("boom.js", "throw new Error('worker boom');")]));
    rt.eval(
        "var err = null; var windowErr = null; \
         addEventListener('error', function(e) { windowErr = e.message; }); \
         var w = new Worker('boom.js'); \
         w.onerror = function(e) { err = e.message; };",
    )
    .expect("start");
    assert!(
        drive_until(&mut rt, "String(err !== null)"),
        "no error event"
    );
    assert!(
        read(&mut rt, "err").contains("worker boom"),
        "{}",
        read(&mut rt, "err")
    );
    // Not canceled on the Worker, so it is reported at the window too — which is
    // where testharness.js is listening.
    assert!(read(&mut rt, "String(windowErr)").contains("worker boom"));

    // A script that cannot be fetched is an error event, not a hang.
    let mut rt2 = Runtime::<E>::new().expect("runtime");
    rt2.set_script_resource_loader(scripts(&[]));
    rt2.eval(
        "var err = null; var w = new Worker('missing.js'); \
         w.onerror = function(e) { e.preventDefault(); err = e.message; };",
    )
    .expect("start");
    assert!(
        drive_until(&mut rt2, "String(err !== null)"),
        "no load error"
    );
    assert!(read(&mut rt2, "err").contains("missing.js"));
}

fn worker_terminate_and_close_work<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_script_resource_loader(scripts(&[
        (
            "chatty.js",
            "onmessage = function(e) { postMessage('echo:' + e.data); };",
        ),
        ("selfclose.js", "postMessage('bye'); close();"),
    ]));
    rt.eval(
        "var got = null; var w = new Worker('chatty.js'); \
         w.onmessage = function(e) { got = e.data; }; w.postMessage('one');",
    )
    .expect("start");
    assert!(drive_until(&mut rt, "String(got !== null)"), "no reply");
    assert_eq!(read(&mut rt, "got"), "echo:one");
    // After terminate the handle is inert and the page quiesces.
    rt.eval("w.terminate(); got = null; w.postMessage('two');")
        .expect("terminate");
    assert!(
        !drive_until(&mut rt, "String(got !== null)"),
        "spoke after terminate"
    );

    rt.eval(
        // Not `closed`: that is a readonly `Window` attribute, so a global
        // `var closed = ...` silently keeps the attribute's value.
        "var closeReply = null; var c = new Worker('selfclose.js'); \
         c.onmessage = function(e) { closeReply = e.data; };",
    )
    .expect("close");
    assert!(
        drive_until(&mut rt, "String(closeReply !== null)"),
        "no close reply"
    );
    assert_eq!(read(&mut rt, "closeReply"), "bye");
}

fn module_worker_is_a_named_residual<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_script_resource_loader(scripts(&[("m.js", "postMessage(1);")]));
    assert_eq!(
        read(
            &mut rt,
            "(function(){ try { new Worker('m.js', { type: 'module' }); return 'no'; } \
             catch (e) { return e.name; } })()"
        ),
        "NotSupportedError"
    );
    assert_eq!(
        read(
            &mut rt,
            "(function(){ try { new Worker(); return 'no'; } catch (e) { return e.name; } })()"
        ),
        "TypeError"
    );
}

fn message_port_crosses_the_thread_boundary<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_script_resource_loader(scripts(&[(
        "port.js",
        "onmessage = function(e) { \
           var p = e.data.port; \
           p.onmessage = function(ev) { p.postMessage('worker got ' + ev.data); }; \
           p.postMessage('hello from worker'); \
         };",
    )]));
    rt.eval(
        "var first = null, second = null; \
         var ch = new MessageChannel(); \
         ch.port1.onmessage = function(e) { \
           if (first === null) { first = e.data; ch.port1.postMessage('ping'); } \
           else { second = e.data; } \
         }; \
         var w = new Worker('port.js'); \
         w.postMessage({ port: ch.port2 }, [ch.port2]);",
    )
    .expect("start");
    assert!(
        drive_until(&mut rt, "String(second !== null)"),
        "no port round trip (first={})",
        read(&mut rt, "String(first)")
    );
    assert_eq!(read(&mut rt, "first"), "hello from worker");
    assert_eq!(read(&mut rt, "second"), "worker got ping");
}

fn transferred_buffer_reaches_the_worker<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_script_resource_loader(scripts(&[(
        "buf.js",
        "onmessage = function(e) { var u = new Uint8Array(e.data); postMessage(u[0] + ',' + u.length); };",
    )]));
    rt.eval(
        "var got = null; var b = new ArrayBuffer(4); new Uint8Array(b)[0] = 7; \
         var w = new Worker('buf.js'); w.onmessage = function(e) { got = e.data; }; \
         w.postMessage(b, [b]); var senderLength = b.byteLength;",
    )
    .expect("start");
    assert!(drive_until(&mut rt, "String(got !== null)"), "no reply");
    assert_eq!(read(&mut rt, "got"), "7,4");
    // The sender's handle reads detached (the cheap-globals emulation; real
    // detachment still needs the VM primitive).
    assert_eq!(read(&mut rt, "String(senderLength)"), "0");
}

fn worker_fetch_uses_the_page_route<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_script_resource_loader(scripts(&[
        ("data.txt", "payload"),
        (
            "fetcher.js",
            "fetch('data.txt').then(function(r) { return r.text(); }) \
             .then(function(t) { postMessage(t); }) \
             .catch(function(e) { postMessage('error:' + e); });",
        ),
    ]));
    rt.eval("var got = null; var w = new Worker('fetcher.js'); w.onmessage = function(e) { got = e.data; };")
        .expect("start");
    assert!(drive_until(&mut rt, "String(got !== null)"), "no reply");
    assert_eq!(read(&mut rt, "got"), "payload");
}

fn deferred_worker_resource_wakes_page_service<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    let wake_count = Arc::new(AtomicUsize::new(0));
    rt.set_script_resource_loader(Box::new(DeferredScripts));
    let observed_wakes = Arc::clone(&wake_count);
    rt.set_worker_resource_wake(Arc::new(move |_| {
        observed_wakes.fetch_add(1, Ordering::Relaxed);
    }));
    rt.eval("var got = null; var w = new Worker('deferred.js'); w.onmessage = function(e) { got = e.data; };")
        .expect("start");
    assert!(
        drive_until(&mut rt, "String(got !== null)"),
        "no deferred reply"
    );
    assert_eq!(read(&mut rt, "got"), "deferred");
    assert!(wake_count.load(Ordering::Relaxed) > 0);
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
    wire_record_round_trips => (wire_record_on_boa, wire_record_on_nova),
    worker_round_trip_works => (worker_round_trip_on_boa, worker_round_trip_on_nova),
    worker_scope_has_no_document => (worker_scope_on_boa, worker_scope_on_nova),
    worker_import_scripts_works => (worker_import_scripts_on_boa, worker_import_scripts_on_nova),
    worker_timers_and_ordering_work => (worker_ordering_on_boa, worker_ordering_on_nova),
    worker_error_reaches_the_page => (worker_error_on_boa, worker_error_on_nova),
    worker_terminate_and_close_work => (worker_terminate_on_boa, worker_terminate_on_nova),
    module_worker_is_a_named_residual => (module_worker_on_boa, module_worker_on_nova),
    message_port_crosses_the_thread_boundary => (port_transfer_on_boa, port_transfer_on_nova),
    transferred_buffer_reaches_the_worker => (buffer_transfer_on_boa, buffer_transfer_on_nova),
    worker_fetch_uses_the_page_route => (worker_fetch_on_boa, worker_fetch_on_nova),
    deferred_worker_resource_wakes_page_service => (deferred_worker_resource_on_boa, deferred_worker_resource_on_nova),
}
