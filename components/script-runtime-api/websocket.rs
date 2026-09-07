// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The `WebSocket` host object and its [`WebSocketHandler`] seam.
//!
//! WebSocket is a second network protocol, not a second shape of `fetch()`: the
//! handshake upgrades a connection and what follows is a bidirectional message
//! stream with its own closing handshake. So it gets its own seam beside
//! [`crate::FetchHandler`], with the same layering — no network stack enters this
//! crate, only the trait — and the same delivery discipline: a host starts the
//! connection, returns immediately, and later drives the runtime's completion
//! entry points ([`Runtime::ws_open`] and friends) from its drive loop, exactly
//! as a deferred fetch settles through [`Runtime::settle_fetch`].
//!
//! The JS surface (`WebSocket`, `CloseEvent`) is a bootstrap over four native
//! sinks — `__websocket_connect` / `_send_text` / `_send_binary` / `_close` —
//! and five host-driven entry points. Everything the specification makes
//! *script-visible* lives in the bootstrap: the four ready states and their
//! constants, `send` for strings, `ArrayBuffer`, views and `Blob`, `close`'s code
//! and reason validation, `bufferedAmount`, `binaryType`, `extensions`,
//! `protocol`, `url`, the `open` / `message` / `error` / `close` events and the
//! spec's error-then-close ordering. Everything the specification makes *policy*
//! — scheme rules past the URL parse, mixed content, HSTS, redirect rejection,
//! blocked ports — lives below the seam, in the host's transport
//! (`netfetcher::websocket` for the runner), because that is where the network is.
//!
//! Absent: a real `Blob` streaming path (the bootstrap's `Blob` is already
//! resident bytes, so `send(blob)` is synchronous), `permessage-deflate`
//! negotiation beyond reporting what the server chose, and `WebSocketStream`.

use std::cell::RefCell;

use script_engine_api::{CallCx, NativeFn, ScriptEngine};

use crate::{HostState, Runtime, js_str};

/// A connection the host should open on script's behalf.
#[derive(Clone, Debug)]
pub struct WebSocketRequest {
    /// The serialized `ws:` / `wss:` URL, already validated by the bootstrap.
    pub url: String,
    /// The subprotocols script requested, in order.
    pub protocols: Vec<String>,
    /// The initiating document's origin (the `Origin` header), or empty. The
    /// host derives "is this a secure context" from its scheme, which is what
    /// mixed-content blocking keys on.
    pub origin: String,
}

/// The host seam for `WebSocket`, beside [`crate::FetchHandler`].
///
/// Every method is fire-and-forget: the host does the work off-thread and
/// reports back through [`Runtime::ws_open`], [`Runtime::ws_message_text`],
/// [`Runtime::ws_message_binary`], [`Runtime::ws_flushed`],
/// [`Runtime::ws_error`] and [`Runtime::ws_close`]. `id` is the routing key the
/// bootstrap minted; it is unique within one runtime.
///
/// With no handler installed every connection fails, which the bootstrap
/// reports as the spec's error-then-close pair — the same shape as a real
/// handshake failure, so a test that runs without a network still exercises the
/// failure path rather than hanging.
pub trait WebSocketHandler {
    /// Open a connection. The host answers later with [`Runtime::ws_open`] or
    /// [`Runtime::ws_error`].
    fn connect(&self, id: u64, request: WebSocketRequest);
    /// Send a text frame.
    fn send_text(&self, id: u64, text: String);
    /// Send a binary frame.
    fn send_binary(&self, id: u64, data: Vec<u8>);
    /// Start the closing handshake. `code` is `None` for a bare close frame.
    fn close(&self, id: u64, code: Option<u16>, reason: String);
}

/// Clone the installed handler out from under the `HostState` borrow, so the
/// handler call holds no borrow (it must not be live if the handler re-enters a
/// native sink). `None` = no handler installed.
fn host_handler<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
) -> Option<std::rc::Rc<dyn WebSocketHandler>> {
    let data = cx.host_data()?;
    let cell = data.downcast_ref::<RefCell<HostState>>()?;
    let h = cell.borrow().websocket.clone();
    h
}

fn arg_string<E: ScriptEngine>(cx: &mut E::CallCx<'_>, i: usize) -> Result<String, E::Error> {
    let a = cx.arg(i);
    cx.value_to_string(&a)
}

fn arg_u64<E: ScriptEngine>(cx: &mut E::CallCx<'_>, i: usize) -> Result<u64, E::Error> {
    Ok(arg_string::<E>(cx, i)?.parse::<u64>().unwrap_or(0))
}

/// A "binary string" (each char code 0-255 is one byte) back to bytes. The same
/// lossless convention the fetch sink uses for bodies.
fn binary_string_to_bytes(s: &str) -> Vec<u8> {
    s.chars().map(|c| c as u8).collect()
}

/// `__websocket_connect(id, url, protocols)` — start a connection. `protocols` is
/// newline-delimited (a subprotocol token can never contain a newline). Returns
/// `""` when a handler took it, `"no-handler"` when none is installed, so the
/// bootstrap can fail the connection on the next task instead of waiting forever.
pub(crate) struct WsConnect;

impl<E: ScriptEngine> NativeFn<E> for WsConnect {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let id = arg_u64::<E>(cx, 0)?;
        let url = arg_string::<E>(cx, 1)?;
        let flat = arg_string::<E>(cx, 2)?;
        let origin = arg_string::<E>(cx, 3)?;
        let protocols: Vec<String> = if flat.is_empty() {
            Vec::new()
        } else {
            flat.split('\n').map(str::to_owned).collect()
        };
        match host_handler::<E>(cx) {
            Some(handler) => {
                handler.connect(
                    id,
                    WebSocketRequest {
                        url,
                        protocols,
                        origin,
                    },
                );
                cx.make_string("")
            },
            None => cx.make_string("no-handler"),
        }
    }
}

/// `__websocket_send_text(id, text)`.
pub(crate) struct WsSendText;

impl<E: ScriptEngine> NativeFn<E> for WsSendText {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let id = arg_u64::<E>(cx, 0)?;
        let text = arg_string::<E>(cx, 1)?;
        if let Some(handler) = host_handler::<E>(cx) {
            handler.send_text(id, text);
        }
        Ok(cx.undefined())
    }
}

/// `__websocket_send_binary(id, binaryString)`.
pub(crate) struct WsSendBinary;

impl<E: ScriptEngine> NativeFn<E> for WsSendBinary {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let id = arg_u64::<E>(cx, 0)?;
        let data = binary_string_to_bytes(&arg_string::<E>(cx, 1)?);
        if let Some(handler) = host_handler::<E>(cx) {
            handler.send_binary(id, data);
        }
        Ok(cx.undefined())
    }
}

/// `__websocket_close(id, code, reason)` — `code` is `""` for a bare close frame.
pub(crate) struct WsClose;

impl<E: ScriptEngine> NativeFn<E> for WsClose {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let id = arg_u64::<E>(cx, 0)?;
        let code_str = arg_string::<E>(cx, 1)?;
        let reason = arg_string::<E>(cx, 2)?;
        let code = code_str.parse::<u16>().ok();
        if let Some(handler) = host_handler::<E>(cx) {
            handler.close(id, code, reason);
        }
        Ok(cx.undefined())
    }
}

/// Install the WebSocket sinks and the `WebSocket` / `CloseEvent` bootstrap.
pub(crate) fn install_websocket_surface<E: ScriptEngine>(engine: &mut E) -> Result<(), E::Error> {
    engine.set_function::<WsConnect>("__websocket_connect", 4)?;
    engine.set_function::<WsSendText>("__websocket_send_text", 2)?;
    engine.set_function::<WsSendBinary>("__websocket_send_binary", 2)?;
    engine.set_function::<WsClose>("__websocket_close", 3)?;
    engine.eval(WEBSOCKET_BOOTSTRAP)?;
    Ok(())
}

impl<E: ScriptEngine> Runtime<E> {
    /// Install the host's WebSocket seam. Until set, every `new WebSocket(...)`
    /// fails the connection (`error` then `close`), which is the right answer for
    /// a runtime with no network — and is what disk-mode WPT sees.
    pub fn set_websocket_handler(&mut self, handler: Box<dyn WebSocketHandler>) {
        self.host().borrow_mut().websocket = Some(std::rc::Rc::from(handler));
    }

    /// The handshake succeeded: `protocol` is the selected subprotocol (or `""`)
    /// and `extensions` the negotiated extension list (or `""`). Fires `open`.
    pub fn ws_open(&mut self, id: u64, protocol: &str, extensions: &str) {
        let js = format!(
            "globalThis.__websocketOpen({},{},{});",
            id,
            js_str(protocol),
            js_str(extensions)
        );
        let _ = self.eval(&js);
        self.run_microtasks();
    }

    /// A text frame arrived. Fires `message` with a string `data`.
    pub fn ws_message_text(&mut self, id: u64, text: &str) {
        let js = format!(
            "globalThis.__websocketMessageText({},{});",
            id,
            js_str(text)
        );
        let _ = self.eval(&js);
        self.run_microtasks();
    }

    /// A binary frame arrived. Fires `message` with a `Blob` or an `ArrayBuffer`
    /// depending on the socket's `binaryType`. Bytes cross as a JS array literal
    /// (no string-escape hazard), the same way a streamed fetch chunk does.
    pub fn ws_message_binary(&mut self, id: u64, bytes: &[u8]) {
        let mut lit = String::with_capacity(bytes.len() * 4 + 2);
        lit.push('[');
        for (i, b) in bytes.iter().enumerate() {
            if i > 0 {
                lit.push(',');
            }
            lit.push_str(&b.to_string());
        }
        lit.push(']');
        let _ = self.eval(&format!(
            "globalThis.__websocketMessageBinary({},{});",
            id, lit
        ));
        self.run_microtasks();
    }

    /// `n` bytes left the send queue, so `bufferedAmount` drops by that much.
    pub fn ws_flushed(&mut self, id: u64, n: u64) {
        let _ = self.eval(&format!("globalThis.__websocketFlushed({},{});", id, n));
        self.run_microtasks();
    }

    /// The connection failed — a handshake failure, a policy refusal, or a drop
    /// with no closing handshake. The bootstrap fires `error` and then `close`
    /// (1006, not clean), which is the ordering the specification requires and
    /// the only failure detail it lets script see.
    pub fn ws_error(&mut self, id: u64) {
        let _ = self.eval(&format!("globalThis.__websocketError({});", id));
        self.run_microtasks();
    }

    /// The closing handshake completed. Fires `close` with the peer's code and
    /// reason; `was_clean` is false when the connection dropped instead.
    pub fn ws_close(&mut self, id: u64, code: u16, reason: &str, was_clean: bool) {
        let js = format!(
            "globalThis.__websocketClose({},{},{},{});",
            id,
            code,
            js_str(reason),
            was_clean
        );
        let _ = self.eval(&js);
        self.run_microtasks();
    }

    /// How many `WebSocket`s are not yet `CLOSED`. The host drive loop reads this
    /// the way it reads `pending_fetches`: a live socket can still speak, so the
    /// agent is not quiescent while one is open.
    pub fn pending_websockets(&mut self) -> usize {
        self.eval("String(globalThis.__websocketPending ? __websocketPending() : 0)")
            .ok()
            .and_then(|v| self.value_to_string(&v).ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    /// Fail every live `WebSocket`, the way [`Runtime::fail_all_pending`] rejects
    /// outstanding fetches: the host calls it at its deadline so a test awaiting a
    /// socket that will never answer records a failure rather than hanging.
    pub fn fail_all_websockets(&mut self) {
        let _ = self.eval("globalThis.__websocketFailAll && __websocketFailAll();");
        self.run_microtasks();
    }
}

/// The `WebSocket` / `CloseEvent` JS surface.
///
/// URL validation runs on the `__url_parse` / `__resolve_url` sinks the fetch
/// bootstrap already installs, so there is one WHATWG URL parser in the runtime.
/// The rules the constructor enforces are the specification's: resolve against
/// the base URL, map `http`/`https` onto `ws`/`wss`, reject any other scheme,
/// reject a fragment, and reject a subprotocol list that is not made of distinct
/// HTTP tokens — each a `SyntaxError` `DOMException`.
const WEBSOCKET_BOOTSTRAP: &str = r#"
(function() {
  var CONNECTING = 0, OPEN = 1, CLOSING = 2, CLOSED = 3;
  var sockets = Object.create(null);
  var nextId = 1;

  function syntaxError(message) {
    return new DOMException(message, 'SyntaxError');
  }

  // UTF-8 byte length; `send` and `close`'s reason are measured in bytes.
  function utf8Len(s) {
    var n = 0;
    for (var i = 0; i < s.length; i++) {
      var c = s.charCodeAt(i);
      if (c < 0x80) n += 1;
      else if (c < 0x800) n += 2;
      else if (c >= 0xD800 && c <= 0xDBFF && i + 1 < s.length &&
               s.charCodeAt(i + 1) >= 0xDC00 && s.charCodeAt(i + 1) <= 0xDFFF) { n += 4; i++; }
      else n += 3;
    }
    return n;
  }
  // UTF-8 encode, with lone surrogates replaced by U+FFFD (what a real socket
  // does with an unpaired surrogate in a text frame).
  function utf8Bytes(s) {
    var out = [];
    for (var i = 0; i < s.length; i++) {
      var c = s.charCodeAt(i);
      if (c >= 0xD800 && c <= 0xDBFF) {
        var d = i + 1 < s.length ? s.charCodeAt(i + 1) : 0;
        if (d >= 0xDC00 && d <= 0xDFFF) { c = 0x10000 + ((c - 0xD800) << 10) + (d - 0xDC00); i++; }
        else c = 0xFFFD;
      } else if (c >= 0xDC00 && c <= 0xDFFF) c = 0xFFFD;
      if (c < 0x80) out.push(c);
      else if (c < 0x800) out.push(0xC0 | (c >> 6), 0x80 | (c & 63));
      else if (c < 0x10000) out.push(0xE0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
      else out.push(0xF0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    }
    return out;
  }
  function bytesToBinaryString(bytes) {
    var s = '';
    for (var i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i] & 255);
    return s;
  }

  // An `on<type>` property with [TreatNonCallableAsNull]: assigning a
  // non-function clears it.
  function defineHandler(proto, type) {
    var slot = '_on' + type;
    Object.defineProperty(proto, 'on' + type, {
      configurable: true,
      enumerable: true,
      get: function() { return this[slot] || null; },
      set: function(fn) {
        if (this[slot]) this.removeEventListener(type, this[slot]);
        this[slot] = (typeof fn === 'function') ? fn : null;
        if (this[slot]) this.addEventListener(type, this[slot]);
      }
    });
  }
  function readOnly(proto, name, get) {
    Object.defineProperty(proto, name, { configurable: true, enumerable: true, get: get });
  }
  function defineGlobal(name, ctor, length) {
    // Non-enumerable, like the XHR family: `for (p in window)` must not yield an
    // interface object (see the XHR plan's F3).
    Object.defineProperty(ctor, 'length', { value: length, writable: false, configurable: true });
    Object.defineProperty(ctor, 'name', { value: name, writable: false, configurable: true });
    Object.defineProperty(ctor, 'prototype', { writable: false });
    if (typeof Symbol !== 'undefined' && Symbol.toStringTag) {
      ctor.prototype[Symbol.toStringTag] = name;
    }
    Object.defineProperty(globalThis, name, {
      value: ctor, writable: true, enumerable: false, configurable: true
    });
  }

  // ---- CloseEvent ----
  function CloseEvent(type, init) {
    if (arguments.length < 1) throw new TypeError('CloseEvent requires a type');
    Event.call(this, type, init);
    init = init || {};
    this.wasClean = !!init.wasClean;
    this.code = init.code === undefined ? 0 : (Number(init.code) >>> 0) & 0xFFFF;
    this.reason = init.reason === undefined ? '' : String(init.reason);
  }
  CloseEvent.prototype = Object.create(Event.prototype);
  CloseEvent.prototype.constructor = CloseEvent;
  defineGlobal('CloseEvent', CloseEvent, 1);

  // ---- URL and subprotocol validation ----
  // The token production for a subprotocol: HTTP tokens, so no separators, no
  // space and nothing outside printable ASCII.
  var TOKEN = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;

  function parseWsUrl(input) {
    var resolved = __resolve_url(String(input));
    var json = __url_parse(resolved, '');
    if (!json) throw syntaxError('Invalid WebSocket URL');
    var u = JSON.parse(json);
    var scheme = u.protocol;
    if (scheme === 'http:' || scheme === 'ws:') scheme = 'ws';
    else if (scheme === 'https:' || scheme === 'wss:') scheme = 'wss';
    else throw syntaxError('The URL\'s scheme must be either ws or wss');
    if (u.hash !== '') throw syntaxError('The URL contains a fragment identifier');
    if (u.protocol !== scheme + ':') {
      var re = __url_with(u.href, 'protocol', scheme);
      if (!re) throw syntaxError('Invalid WebSocket URL');
      u = JSON.parse(re);
    }
    return u;
  }

  function normalizeProtocols(protocols) {
    var list;
    if (protocols === undefined) list = [];
    else if (typeof protocols === 'string') list = [protocols];
    else if (protocols === null) list = ['null'];
    else if (typeof protocols === 'object' && typeof protocols.length === 'number') {
      list = [];
      for (var i = 0; i < protocols.length; i++) list.push(String(protocols[i]));
    } else list = [String(protocols)];
    var seen = Object.create(null);
    for (var j = 0; j < list.length; j++) {
      var p = list[j];
      if (!TOKEN.test(p)) throw syntaxError('The subprotocol \'' + p + '\' is invalid');
      var key = p.toLowerCase();
      if (seen[key]) throw syntaxError('The subprotocol \'' + p + '\' is duplicated');
      seen[key] = true;
    }
    return list;
  }

  // ---- WebSocket ----
  function WebSocket(url, protocols) {
    if (!(this instanceof WebSocket)) throw new TypeError("Constructor WebSocket requires 'new'");
    if (arguments.length < 1) throw new TypeError('WebSocket requires a url');
    EventTarget.call(this);
    var parsed = parseWsUrl(url);
    var list = normalizeProtocols(protocols);

    this._id = nextId++;
    this._url = parsed.href;
    this._origin = parsed.origin;
    this._readyState = CONNECTING;
    this._buffered = 0;
    this._protocol = '';
    this._extensions = '';
    this._binaryType = 'blob';
    sockets[this._id] = this;

    var pageOrigin = '';
    try { pageOrigin = (typeof location !== 'undefined' && location.origin) ? location.origin : ''; }
    catch (e) { pageOrigin = ''; }
    var answer = __websocket_connect(String(this._id), this._url, list.join('\n'), pageOrigin);
    if (answer === 'no-handler') {
      // No network under this runtime. Fail on the next task, so the failure is
      // observable in the same order a real handshake failure would be: the
      // constructor returns first, then error, then close.
      var self = this;
      setTimeout(function() { fail(self); }, 0);
    }
  }
  WebSocket.prototype = Object.create(EventTarget.prototype);
  WebSocket.prototype.constructor = WebSocket;

  readOnly(WebSocket.prototype, 'url', function() { return this._url; });
  readOnly(WebSocket.prototype, 'readyState', function() { return this._readyState; });
  readOnly(WebSocket.prototype, 'bufferedAmount', function() { return this._buffered; });
  readOnly(WebSocket.prototype, 'protocol', function() { return this._protocol; });
  readOnly(WebSocket.prototype, 'extensions', function() { return this._extensions; });
  Object.defineProperty(WebSocket.prototype, 'binaryType', {
    configurable: true, enumerable: true,
    get: function() { return this._binaryType; },
    // An IDL enum attribute: a value outside the enum is ignored, not thrown.
    set: function(v) { v = String(v); if (v === 'blob' || v === 'arraybuffer') this._binaryType = v; }
  });
  ['open', 'message', 'error', 'close'].forEach(function(t) { defineHandler(WebSocket.prototype, t); });

  var CONSTANTS = { CONNECTING: CONNECTING, OPEN: OPEN, CLOSING: CLOSING, CLOSED: CLOSED };
  Object.keys(CONSTANTS).forEach(function(name) {
    var d = { value: CONSTANTS[name], writable: false, enumerable: true, configurable: false };
    Object.defineProperty(WebSocket, name, d);
    Object.defineProperty(WebSocket.prototype, name, d);
  });

  WebSocket.prototype.send = function(data) {
    if (arguments.length < 1) throw new TypeError('send requires an argument');
    if (this._readyState === CONNECTING) {
      throw new DOMException('Still in CONNECTING state.', 'InvalidStateError');
    }
    var bytes = null, text = null, len = 0;
    if (typeof Blob !== 'undefined' && data instanceof Blob) {
      bytes = data._b; len = data.size;
    } else if (data instanceof ArrayBuffer) {
      bytes = new Uint8Array(data.slice(0)); len = bytes.length;
    } else if (ArrayBuffer.isView(data)) {
      bytes = new Uint8Array(data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength));
      len = bytes.length;
    } else {
      text = String(data); len = utf8Len(text);
    }
    // CLOSING / CLOSED: the bytes are counted and dropped, per the spec.
    if (this._readyState !== OPEN) { this._buffered += len; return; }
    this._buffered += len;
    if (text !== null) __websocket_send_text(String(this._id), text);
    else __websocket_send_binary(String(this._id), bytesToBinaryString(bytes));
  };

  WebSocket.prototype.close = function(code, reason) {
    var wire = '';
    if (code !== undefined) {
      // [Clamp] unsigned short: NaN and non-numbers become 0, which is invalid.
      var c = Number(code);
      if (isNaN(c)) c = 0;
      c = Math.round(c);
      if (c < 0) c = 0;
      if (c > 65535) c = 65535;
      if (!(c === 1000 || (c >= 3000 && c <= 4999))) {
        throw new DOMException('The close code must be 1000 or in 3000-4999.', 'InvalidAccessError');
      }
      wire = String(c);
    }
    var r = (reason === undefined || reason === null) ? '' : String(reason);
    if (utf8Len(r) > 123) {
      throw syntaxError('The close reason must not be longer than 123 UTF-8 bytes.');
    }
    if (this._readyState === CLOSING || this._readyState === CLOSED) return;
    this._readyState = CLOSING;
    __websocket_close(String(this._id), wire, r);
  };

  // ---- host-driven completions ----
  function live(id) {
    var ws = sockets[id];
    return (ws && ws._readyState !== CLOSED) ? ws : null;
  }
  function finish(ws, code, reason, clean) {
    ws._readyState = CLOSED;
    delete sockets[ws._id];
    var e = new CloseEvent('close', { wasClean: clean, code: code, reason: reason });
    ws.dispatchEvent(e);
  }
  // Fail the connection: error, then close(1006, '', wasClean false). The order
  // is the specification's and is what `remove-own-iframe-during-onerror` and the
  // constructor tests observe.
  function fail(ws) {
    if (ws._readyState === CLOSED) return;
    ws.dispatchEvent(new Event('error'));
    finish(ws, 1006, '', false);
  }

  globalThis.__websocketOpen = function(id, protocol, extensions) {
    var ws = live(id);
    if (!ws || ws._readyState !== CONNECTING) return;
    ws._protocol = protocol || '';
    ws._extensions = extensions || '';
    ws._readyState = OPEN;
    ws.dispatchEvent(new Event('open'));
  };
  globalThis.__websocketMessageText = function(id, text) {
    var ws = live(id);
    if (!ws || ws._readyState !== OPEN) return;
    ws.dispatchEvent(new MessageEvent('message', { data: text, origin: ws._origin }));
  };
  globalThis.__websocketMessageBinary = function(id, arr) {
    var ws = live(id);
    if (!ws || ws._readyState !== OPEN) return;
    var bytes = new Uint8Array(arr);
    var data = ws._binaryType === 'arraybuffer' ? bytes.buffer : new Blob([bytes]);
    ws.dispatchEvent(new MessageEvent('message', { data: data, origin: ws._origin }));
  };
  globalThis.__websocketFlushed = function(id, n) {
    var ws = sockets[id];
    if (!ws) return;
    ws._buffered -= n;
    if (ws._buffered < 0) ws._buffered = 0;
  };
  globalThis.__websocketError = function(id) {
    var ws = live(id);
    if (ws) fail(ws);
  };
  globalThis.__websocketClose = function(id, code, reason, clean) {
    var ws = live(id);
    if (!ws) return;
    if (!clean) { fail(ws); return; }
    finish(ws, code, reason || '', true);
  };
  globalThis.__websocketPending = function() {
    var n = 0;
    for (var k in sockets) if (sockets[k]._readyState !== CLOSED) n++;
    return n;
  };
  globalThis.__websocketFailAll = function() {
    for (var k in sockets) {
      var ws = sockets[k];
      if (ws) fail(ws);
    }
  };

  defineGlobal('WebSocket', WebSocket, 1);
})();
"#;
