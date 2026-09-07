// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The `WebSocket` host object over its own host seam. Proves the constructor's
//! URL and subprotocol rules, the four ready states, `send` over every data kind,
//! `close`'s code and reason validation, `bufferedAmount`, `binaryType`, the four
//! events and the spec's error-then-close ordering. Each body runs against both
//! backends, in the pattern of `xhr_binding.rs` (which is Boa-only) crossed with
//! `cheap_globals.rs`'s `both_engines!`.

use std::cell::RefCell;
use std::rc::Rc;

use script_engine_api::ScriptEngine;
use script_runtime_api::{Runtime, WebSocketHandler, WebSocketRequest};

/// What the handler was asked to do, in order — the seam's whole observable
/// surface from the script side.
#[derive(Debug, PartialEq, Eq)]
enum Op {
    Connect(String, Vec<String>),
    Text(String),
    Binary(Vec<u8>),
    Close(Option<u16>, String),
}

/// A handler that records and never answers: the test drives the completions
/// itself, so every ordering is deliberate rather than incidental.
#[derive(Default, Clone)]
struct Recorder {
    ops: Rc<RefCell<Vec<Op>>>,
    ids: Rc<RefCell<Vec<u64>>>,
}

impl WebSocketHandler for Recorder {
    fn connect(&self, id: u64, request: WebSocketRequest) {
        self.ids.borrow_mut().push(id);
        self.ops
            .borrow_mut()
            .push(Op::Connect(request.url, request.protocols));
    }
    fn send_text(&self, _id: u64, text: String) {
        self.ops.borrow_mut().push(Op::Text(text));
    }
    fn send_binary(&self, _id: u64, data: Vec<u8>) {
        self.ops.borrow_mut().push(Op::Binary(data));
    }
    fn close(&self, _id: u64, code: Option<u16>, reason: String) {
        self.ops.borrow_mut().push(Op::Close(code, reason));
    }
}

fn read<E: ScriptEngine>(rt: &mut Runtime<E>, expr: &str) -> String {
    let v = rt.eval(expr).expect("eval");
    rt.value_to_string(&v).expect("stringify")
}

/// A runtime with a base URL (so relative and scheme-swapped URLs resolve) and a
/// recording handler.
fn wired<E: ScriptEngine>() -> (Runtime<E>, Recorder) {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_base_url("http://page.example/dir/doc.html")
        .expect("base url");
    let rec = Recorder::default();
    rt.set_websocket_handler(Box::new(rec.clone()));
    (rt, rec)
}

/// The single socket id the recorder saw.
fn only_id(rec: &Recorder) -> u64 {
    let ids = rec.ids.borrow();
    assert_eq!(ids.len(), 1, "expected exactly one connect");
    ids[0]
}

// ── the interfaces ───────────────────────────────────────────────────────────

fn interfaces_and_constants<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    assert_eq!(
        read(&mut rt, "[typeof WebSocket, typeof CloseEvent].join(',')"),
        "function,function"
    );
    // The four ready states, on the constructor and on the prototype.
    assert_eq!(
        read(
            &mut rt,
            "[WebSocket.CONNECTING, WebSocket.OPEN, WebSocket.CLOSING, WebSocket.CLOSED, \
              WebSocket.prototype.CONNECTING, WebSocket.prototype.OPEN, \
              WebSocket.prototype.CLOSING, WebSocket.prototype.CLOSED].join(',')"
        ),
        "0,1,2,3,0,1,2,3"
    );
    assert_eq!(read(&mut rt, "String(WebSocket.length)"), "1");
    // Interface objects are not enumerable on the global (the XHR lane's F3).
    assert_eq!(
        read(
            &mut rt,
            "String(Object.keys(globalThis).indexOf('WebSocket') === -1 && \
              Object.keys(globalThis).indexOf('CloseEvent') === -1)"
        ),
        "true"
    );
    // CloseEvent is an Event with the three extra attributes.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var e = new CloseEvent('close', { wasClean: true, code: 1000, reason: 'x' }); \
              return [e instanceof Event, e.type, e.wasClean, e.code, e.reason].join(','); })()"
        ),
        "true,close,true,1000,x"
    );
    // Its defaults.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ var e = new CloseEvent('close'); \
              return [e.wasClean, e.code, e.reason].join(','); })()"
        ),
        "false,0,"
    );
}

// ── the constructor ──────────────────────────────────────────────────────────

fn constructor_url_rules<E: ScriptEngine>() {
    let (mut rt, rec) = wired::<E>();
    // http/https are the same URLs spelled differently; relative URLs resolve
    // against the document base.
    assert_eq!(
        read(&mut rt, "new WebSocket('http://a.example/x').url"),
        "ws://a.example/x"
    );
    assert_eq!(
        read(&mut rt, "new WebSocket('https://a.example/x').url"),
        "wss://a.example/x"
    );
    assert_eq!(
        read(&mut rt, "new WebSocket('ws://a.example/x').url"),
        "ws://a.example/x"
    );
    assert_eq!(
        read(&mut rt, "new WebSocket('test').url"),
        "ws://page.example/dir/test"
    );
    assert!(
        !rec.ops.borrow().is_empty(),
        "the host was asked to connect"
    );

    // Every rejected URL is a SyntaxError DOMException.
    for bad in [
        "'ws://foo bar.com/'",
        "'ftp://a.example/'",
        "'mailto:example@example.org'",
        "'about:blank'",
        "'#test'",
        "'ws://a.example/#'",
        "'https://a.example/#frag'",
    ] {
        assert_eq!(
            read(
                &mut rt,
                &format!(
                    "(function(){{ try {{ new WebSocket({bad}); return 'no-throw'; }} \
                      catch (e) {{ return (e instanceof DOMException) + ':' + e.name; }} }})()"
                )
            ),
            "true:SyntaxError",
            "for {bad}"
        );
    }
    // No arguments is a TypeError, not a DOMException.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ try { new WebSocket(); return 'no-throw'; } \
              catch (e) { return e.constructor.name; } })()"
        ),
        "TypeError"
    );
    // Extra arguments are ignored.
    assert_eq!(
        read(
            &mut rt,
            "String(new WebSocket('ws://a.example/', 'echo', 'stray') instanceof WebSocket)"
        ),
        "true"
    );
}

fn constructor_protocol_rules<E: ScriptEngine>() {
    let (mut rt, rec) = wired::<E>();
    rt.eval("var a = new WebSocket('ws://a.example/', ['chat', 'superchat']);")
        .expect("construct");
    assert_eq!(
        rec.ops.borrow()[0],
        Op::Connect(
            "ws://a.example/".to_owned(),
            vec!["chat".to_owned(), "superchat".to_owned()]
        )
    );
    // A bare string is a one-element list.
    rt.eval("var b = new WebSocket('ws://a.example/', 'chat');")
        .expect("construct");
    assert_eq!(
        rec.ops.borrow()[1],
        Op::Connect("ws://a.example/".to_owned(), vec!["chat".to_owned()])
    );

    for bad in [
        "'ec ho'",       // a space is not a token character
        "'ec\\u0009ho'", // nor is a tab
        "'\\u00e9cho'",  // nor is non-ASCII
        "['chat', 'chat']",
        "['chat', 'CHAT']", // duplicates are case-insensitive
        "''",
    ] {
        assert_eq!(
            read(
                &mut rt,
                &format!(
                    "(function(){{ try {{ new WebSocket('ws://a.example/', {bad}); return 'no-throw'; }} \
                      catch (e) {{ return (e instanceof DOMException) + ':' + e.name; }} }})()"
                )
            ),
            "true:SyntaxError",
            "for {bad}"
        );
    }
}

// ── the connection's life ────────────────────────────────────────────────────

fn open_message_and_close<E: ScriptEngine>() {
    let (mut rt, rec) = wired::<E>();
    rt.eval(
        "var log = []; \
         var ws = new WebSocket('ws://a.example/echo', ['chat']); \
         ws.onopen = function(e) { log.push('open:' + e.type + ':' + ws.readyState + ':' + ws.protocol + ':' + ws.extensions); }; \
         ws.onmessage = function(e) { log.push('msg:' + (typeof e.data === 'string' ? e.data : '[binary]') + ':' + e.origin); }; \
         ws.onclose = function(e) { log.push('close:' + e.wasClean + ':' + e.code + ':' + e.reason + ':' + ws.readyState); }; \
         ws.onerror = function() { log.push('error'); };",
    )
    .expect("setup");
    assert_eq!(read(&mut rt, "String(ws.readyState)"), "0");
    assert_eq!(read(&mut rt, "ws.url"), "ws://a.example/echo");
    assert_eq!(read(&mut rt, "ws.binaryType"), "blob");
    assert_eq!(read(&mut rt, "String(ws.bufferedAmount)"), "0");

    let id = only_id(&rec);
    rt.ws_open(id, "chat", "permessage-deflate");
    assert_eq!(read(&mut rt, "String(ws.readyState)"), "1");
    rt.ws_message_text(id, "hello");
    rt.ws_close(id, 1000, "bye", true);
    assert_eq!(read(&mut rt, "String(ws.readyState)"), "3");
    assert_eq!(
        read(&mut rt, "log.join('|')"),
        // `origin` is the serialization of the WebSocket *URL*'s origin, so it
        // carries the ws scheme, not the page's http one.
        "open:open:1:chat:permessage-deflate|msg:hello:ws://a.example|close:true:1000:bye:3"
    );
    // A message after close is dropped, not delivered.
    rt.ws_message_text(id, "late");
    assert_eq!(read(&mut rt, "String(log.length)"), "3");
}

fn binary_messages_follow_binary_type<E: ScriptEngine>() {
    let (mut rt, rec) = wired::<E>();
    rt.eval("var got = null; var ws = new WebSocket('ws://a.example/'); ws.onmessage = function(e) { got = e.data; };")
        .expect("setup");
    let id = only_id(&rec);
    rt.ws_open(id, "", "");

    // Default: a Blob.
    rt.ws_message_binary(id, &[1, 2, 3]);
    assert_eq!(
        read(&mut rt, "[got instanceof Blob, got.size].join(',')"),
        "true,3"
    );
    // Switched to arraybuffer: an ArrayBuffer with the same bytes.
    rt.eval("ws.binaryType = 'arraybuffer';").expect("set");
    assert_eq!(read(&mut rt, "ws.binaryType"), "arraybuffer");
    rt.ws_message_binary(id, &[4, 5, 6, 7]);
    assert_eq!(
        read(
            &mut rt,
            "[got instanceof ArrayBuffer, got.byteLength, Array.prototype.join.call(new Uint8Array(got), '-')].join(',')"
        ),
        "true,4,4-5-6-7"
    );
    // An out-of-enum value is ignored, not thrown.
    rt.eval("ws.binaryType = 'notBlobOrArrayBuffer';")
        .expect("set");
    assert_eq!(read(&mut rt, "ws.binaryType"), "arraybuffer");
}

fn send_covers_every_data_kind<E: ScriptEngine>() {
    let (mut rt, rec) = wired::<E>();
    rt.eval("var ws = new WebSocket('ws://a.example/');")
        .expect("setup");
    // CONNECTING: send throws InvalidStateError.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ try { ws.send('x'); return 'no-throw'; } \
              catch (e) { return (e instanceof DOMException) + ':' + e.name; } })()"
        ),
        "true:InvalidStateError"
    );
    let id = only_id(&rec);
    rt.ws_open(id, "", "");

    rt.eval(
        "ws.send('héllo'); \
         ws.send(new Uint8Array([1,2,3]).buffer); \
         ws.send(new Uint8Array([9,8,7,6]).subarray(1)); \
         ws.send(new Blob([new Uint8Array([42])]));",
    )
    .expect("send");
    let ops = rec.ops.borrow();
    assert_eq!(ops[1], Op::Text("héllo".to_owned()));
    assert_eq!(ops[2], Op::Binary(vec![1, 2, 3]));
    assert_eq!(
        ops[3],
        Op::Binary(vec![8, 7, 6]),
        "a view honours its offset"
    );
    assert_eq!(ops[4], Op::Binary(vec![42]));
    drop(ops);

    // bufferedAmount counted the UTF-8 bytes of each payload: 6 + 3 + 3 + 1.
    assert_eq!(read(&mut rt, "String(ws.bufferedAmount)"), "13");
    rt.ws_flushed(id, 13);
    assert_eq!(read(&mut rt, "String(ws.bufferedAmount)"), "0");

    // After close, a send is counted and dropped rather than throwing.
    rt.eval("ws.close();").expect("close");
    assert_eq!(read(&mut rt, "String(ws.readyState)"), "2");
    rt.eval("ws.send('abc');").expect("send while closing");
    assert_eq!(read(&mut rt, "String(ws.bufferedAmount)"), "3");
    assert_eq!(
        rec.ops
            .borrow()
            .iter()
            .filter(|o| matches!(o, Op::Text(_)))
            .count(),
        1,
        "nothing is sent once closing"
    );
    // send with no argument is a TypeError.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ try { ws.send(); return 'no-throw'; } catch (e) { return e.constructor.name; } })()"
        ),
        "TypeError"
    );
}

fn close_validates_code_and_reason<E: ScriptEngine>() {
    let (mut rt, rec) = wired::<E>();
    rt.eval("var ws = new WebSocket('ws://a.example/');")
        .expect("setup");
    let id = only_id(&rec);
    rt.ws_open(id, "", "");

    // The invalid codes from `websockets/close-invalid.any.js`.
    for bad in ["0", "500", "NaN", "'string'", "null", "0x10000 + 1000"] {
        assert_eq!(
            read(
                &mut rt,
                &format!(
                    "(function(){{ try {{ ws.close({bad}); return 'no-throw'; }} \
                      catch (e) {{ return (e instanceof DOMException) + ':' + e.name; }} }})()"
                )
            ),
            "true:InvalidAccessError",
            "for {bad}"
        );
    }
    // A reason over 123 UTF-8 bytes is a SyntaxError.
    assert_eq!(
        read(
            &mut rt,
            "(function(){ try { ws.close(1000, new Array(125).join('x')); return 'no-throw'; } \
              catch (e) { return (e instanceof DOMException) + ':' + e.name; } })()"
        ),
        "true:SyntaxError"
    );
    // 123 bytes exactly is fine, and reaches the host with its code.
    rt.eval("ws.close(3000, new Array(124).join('x'));")
        .expect("close");
    let reason = "x".repeat(123);
    assert_eq!(
        *rec.ops.borrow().last().unwrap(),
        Op::Close(Some(3000), reason)
    );
    assert_eq!(read(&mut rt, "String(ws.readyState)"), "2");
    // A second close is a no-op.
    rt.eval("ws.close(1000);").expect("close again");
    assert_eq!(
        rec.ops
            .borrow()
            .iter()
            .filter(|o| matches!(o, Op::Close(..)))
            .count(),
        1
    );
}

fn close_with_no_code_sends_a_bare_frame<E: ScriptEngine>() {
    let (mut rt, rec) = wired::<E>();
    rt.eval("var ws = new WebSocket('ws://a.example/');")
        .expect("setup");
    let id = only_id(&rec);
    rt.ws_open(id, "", "");
    // `close()` and `close(undefined)` are the same call: no code on the wire.
    rt.eval("ws.close(undefined);").expect("close");
    assert_eq!(
        *rec.ops.borrow().last().unwrap(),
        Op::Close(None, String::new())
    );
    // The peer answers with "no status received" (1005), still a clean close.
    rt.ws_close(id, 1005, "", true);
    assert_eq!(read(&mut rt, "String(ws.readyState)"), "3");
}

fn a_failure_fires_error_then_close<E: ScriptEngine>() {
    let (mut rt, rec) = wired::<E>();
    rt.eval(
        "var log = []; var ws = new WebSocket('ws://a.example/'); \
         ws.onerror = function(e) { log.push('error:' + e.type + ':' + ws.readyState); }; \
         ws.onclose = function(e) { log.push('close:' + e.wasClean + ':' + e.code + ':' + e.reason + ':' + ws.readyState); };",
    )
    .expect("setup");
    let id = only_id(&rec);
    rt.ws_error(id);
    // The specification's ordering, and the only failure detail script sees.
    assert_eq!(
        read(&mut rt, "log.join('|')"),
        "error:error:0|close:false:1006::3"
    );
    // An unclean close reported by the host takes the same path.
    let (mut rt2, rec2) = wired::<E>();
    rt2.eval(
        "var log = []; var ws = new WebSocket('ws://a.example/'); \
         ws.onerror = function() { log.push('error'); }; \
         ws.onclose = function(e) { log.push('close:' + e.wasClean + ':' + e.code); };",
    )
    .expect("setup");
    let id2 = only_id(&rec2);
    rt2.ws_open(id2, "", "");
    rt2.ws_close(id2, 1006, "", false);
    assert_eq!(read(&mut rt2, "log.join('|')"), "error|close:false:1006");
}

fn with_no_handler_every_connection_fails<E: ScriptEngine>() {
    // No `set_websocket_handler`: the runtime has no network, so the connection
    // fails on the next task rather than hanging.
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_base_url("http://page.example/").expect("base url");
    rt.eval(
        "var log = []; var ws = new WebSocket('ws://a.example/'); \
         ws.onerror = function() { log.push('error'); }; \
         ws.onclose = function(e) { log.push('close:' + e.code + ':' + e.wasClean); };",
    )
    .expect("setup");
    assert_eq!(
        read(&mut rt, "log.join('|')"),
        "",
        "the failure is not synchronous"
    );
    assert_eq!(read(&mut rt, "String(ws.readyState)"), "0");
    rt.run_event_loop(10).expect("drain");
    assert_eq!(read(&mut rt, "log.join('|')"), "error|close:1006:false");
    assert_eq!(read(&mut rt, "String(ws.readyState)"), "3");
}

fn event_handlers_treat_non_callable_as_null<E: ScriptEngine>() {
    let (mut rt, _rec) = wired::<E>();
    rt.eval("var ws = new WebSocket('ws://a.example/');")
        .expect("setup");
    for name in ["open", "message", "error", "close"] {
        assert_eq!(
            read(
                &mut rt,
                &format!(
                    "(function(){{ var a = ws.on{name} === null; ws.on{name} = function(){{}}; \
                      var b = typeof ws.on{name} === 'function'; ws.on{name} = 2; \
                      return [a, b, ws.on{name} === null].join(','); }})()"
                )
            ),
            "true,true,true",
            "for on{name}"
        );
    }
}

fn pending_count_tracks_live_sockets<E: ScriptEngine>() {
    let (mut rt, rec) = wired::<E>();
    assert_eq!(rt.pending_websockets(), 0);
    rt.eval(
        "var a = new WebSocket('ws://a.example/1'); var b = new WebSocket('ws://a.example/2');",
    )
    .expect("setup");
    assert_eq!(rt.pending_websockets(), 2);
    let ids: Vec<u64> = rec.ids.borrow().clone();
    rt.ws_open(ids[0], "", "");
    assert_eq!(rt.pending_websockets(), 2, "an open socket is still live");
    rt.ws_close(ids[0], 1000, "", true);
    assert_eq!(rt.pending_websockets(), 1);
    // The host's deadline sweep fails whatever is left.
    rt.fail_all_websockets();
    assert_eq!(rt.pending_websockets(), 0);
    assert_eq!(read(&mut rt, "String(b.readyState)"), "3");
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
    interfaces_and_constants => (interfaces_on_boa, interfaces_on_nova),
    constructor_url_rules => (constructor_url_on_boa, constructor_url_on_nova),
    constructor_protocol_rules => (constructor_protocols_on_boa, constructor_protocols_on_nova),
    open_message_and_close => (open_message_close_on_boa, open_message_close_on_nova),
    binary_messages_follow_binary_type => (binary_type_on_boa, binary_type_on_nova),
    send_covers_every_data_kind => (send_kinds_on_boa, send_kinds_on_nova),
    close_validates_code_and_reason => (close_validation_on_boa, close_validation_on_nova),
    close_with_no_code_sends_a_bare_frame => (close_bare_on_boa, close_bare_on_nova),
    a_failure_fires_error_then_close => (failure_order_on_boa, failure_order_on_nova),
    with_no_handler_every_connection_fails => (no_handler_on_boa, no_handler_on_nova),
    event_handlers_treat_non_callable_as_null => (handlers_on_boa, handlers_on_nova),
    pending_count_tracks_live_sockets => (pending_on_boa, pending_on_nova),
}
