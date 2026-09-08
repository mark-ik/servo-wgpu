/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! `MessageEvent`, `MessageChannel` / `MessagePort` and `BroadcastChannel`.
//!
//! One runtime, one agent: an entangled port pair and every `BroadcastChannel`
//! with the same name live in this global. Delivery is a task on the drive loop
//! (a zero-delay timer), never synchronous, so ordering matches a real agent's
//! port message queue. Messages are cloned through [`crate::structured_clone`];
//! a `MessagePort` in the transfer list is disentangled from the sender and
//! adopted by the receiver through the transferable registry.
//!
//! `window.postMessage` is upgraded here from the shell's `Event`-shaped stub to
//! a real `MessageEvent` with a cloned payload, `origin`, `source` and `ports`.
//!
//! Absent: cross-agent messaging (no `Worker`, no `iframe`), `targetOrigin`
//! filtering beyond the syntax check, and `CryptoKey` / `ImageBitmap` payloads.

use script_engine_api::ScriptEngine;

pub(crate) fn install_messaging_surface<E: ScriptEngine>(engine: &mut E) -> Result<(), E::Error> {
    engine.eval(MESSAGING_BOOTSTRAP)?;
    Ok(())
}

const MESSAGING_BOOTSTRAP: &str = r#"
(function() {
  function defineTag(ctor, name) {
    if (typeof Symbol !== 'undefined' && Symbol.toStringTag) {
      ctor.prototype[Symbol.toStringTag] = name;
    }
  }

  // ---- MessageEvent ----
  function MessageEvent(type, init) {
    Event.call(this, type, init);
    init = init || {};
    this.data = init.data === undefined ? null : init.data;
    this.origin = init.origin === undefined ? '' : String(init.origin);
    this.lastEventId = init.lastEventId === undefined ? '' : String(init.lastEventId);
    this.source = init.source === undefined ? null : init.source;
    this.ports = Object.freeze((init.ports || []).slice());
  }
  MessageEvent.prototype = Object.create(Event.prototype);
  MessageEvent.prototype.constructor = MessageEvent;
  MessageEvent.prototype.initMessageEvent = function(type, bubbles, cancelable, data, origin, lastEventId, source, ports) {
    this.type = String(type);
    this.bubbles = !!bubbles;
    this.cancelable = !!cancelable;
    this.data = data === undefined ? null : data;
    this.origin = origin === undefined ? '' : String(origin);
    this.lastEventId = lastEventId === undefined ? '' : String(lastEventId);
    this.source = source === undefined ? null : source;
    this.ports = Object.freeze((ports || []).slice());
    this.__initialized = true;
  };
  defineTag(MessageEvent, 'MessageEvent');
  globalThis.MessageEvent = MessageEvent;

  // An `on<type>` property that behaves like the one listener HTML gives it.
  function defineHandler(proto, type, onSet) {
    var slot = '_on' + type;
    Object.defineProperty(proto, 'on' + type, {
      configurable: true,
      enumerable: true,
      get: function() { return this[slot] || null; },
      set: function(fn) {
        if (this[slot]) this.removeEventListener(type, this[slot]);
        this[slot] = (typeof fn === 'function') ? fn : null;
        if (this[slot]) this.addEventListener(type, this[slot]);
        if (onSet) onSet.call(this);
      }
    });
  }

  // ---- MessagePort ----
  function MessagePort() {
    EventTarget.call(this);
    this._peer = null;
    this._queue = [];          // "port message queue", held until enabled
    this._enabled = false;
    this._closed = false;
    this._transferred = false;
    this._remote = null;       // non-null on a stub standing in for another agent
  }
  MessagePort.prototype = Object.create(EventTarget.prototype);
  MessagePort.prototype.constructor = MessagePort;

  function deliver(port, event) {
    // Port message queue task source: never synchronous with postMessage.
    setTimeout(function() {
      if (port._closed) return;
      port.dispatchEvent(event);
    }, 0);
  }
  function enable(port) {
    if (port._enabled) return;
    port._enabled = true;
    var pending = port._queue;
    port._queue = [];
    for (var i = 0; i < pending.length; i++) deliver(port, pending[i]);
  }

  MessagePort.prototype.start = function() { enable(this); };
  MessagePort.prototype.close = function() {
    this._closed = true;
    this._queue = [];
    if (this._peer) { this._peer._peer = null; this._peer = null; }
  };
  MessagePort.prototype.postMessage = function(message, transferOrOptions) {
    if (this._transferred) {
      throw new DOMException("The port is detached.", "InvalidStateError");
    }
    var transfer = normalizeTransfer(transferOrOptions);
    var ports = portsIn(transfer);
    for (var i = 0; i < ports.length; i++) {
      if (ports[i] === this) throw dataClone("The source port");
    }
    var peer = this._peer;
    if (peer && peer._remote !== null) {
      // The entangled endpoint lives in another agent: serialize to the wire
      // and let the host route it by port id.
      var payload = globalThis.__scSerialize(message, transfer);
      var minted = globalThis.__portTakePending();
      globalThis.__portForward(peer._remote, payload, minted.join(','));
      return;
    }
    var data = globalThis.__structuredClone(message, transfer);
    if (!peer || peer._closed) return;  // no entangled port: the message is dropped
    var event = internalMessageEvent(data, { ports: ports, origin: '' });
    if (peer._enabled) deliver(peer, event); else peer._queue.push(event);
  };
  defineHandler(MessagePort.prototype, 'message', function() { enable(this); });
  defineHandler(MessagePort.prototype, 'messageerror', null);
  defineTag(MessagePort, 'MessagePort');
  globalThis.MessagePort = MessagePort;

  // A MessageEvent for an internal delivery. `data` is assigned after
  // construction so an `undefined` payload stays undefined: the IDL default
  // (absent -> null) applies to script-constructed events, not to delivery.
  function internalMessageEvent(data, init) {
    var event = new MessageEvent('message', init);
    event.data = data;
    return event;
  }

  function dataClone(what) {
    return new DOMException(what + " could not be cloned.", "DataCloneError");
  }
  // `transfer` is either a sequence (legacy) or `{ transfer: sequence }`.
  function normalizeTransfer(arg) {
    if (arg === undefined || arg === null) return undefined;
    if (typeof arg === 'object' && typeof arg.length === 'number') return arg;
    if (typeof arg === 'object' && arg.transfer !== undefined) return arg.transfer;
    return undefined;
  }
  function portsIn(transfer) {
    var out = [];
    if (!transfer) return out;
    for (var i = 0; i < transfer.length; i++) {
      if (transfer[i] instanceof MessagePort) out.push(transfer[i]);
    }
    return out;
  }

  // ---- Cross-agent endpoints ----
  // A port transferred to another agent leaves a *stub* behind: the peer keeps
  // its entanglement, but delivery forwards over the host link keyed by a
  // process-unique port id instead of dispatching locally.
  var remoteStubs = {};      // port id -> stub port
  var pendingPortIds = [];   // ids minted by the serialization in progress

  function remoteStub(id) {
    var p = new MessagePort();
    p._remote = id;
    remoteStubs[id] = p;
    return p;
  }
  // The ids this agent minted while serializing; the caller binds them to the
  // link the record is about to travel on.
  globalThis.__portTakePending = function() {
    var ids = pendingPortIds;
    pendingPortIds = [];
    return ids;
  };
  // A message arriving from the other agent for port `id`.
  globalThis.__portDeliver = function(id, payload) {
    var stub = remoteStubs[id];
    if (!stub) return;
    var target = stub._peer;
    if (!target || target._closed) return;
    var event;
    try {
      event = internalMessageEvent(globalThis.__scDeserialize(payload), { origin: '' });
    } catch (e) {
      event = new MessageEvent('messageerror', { origin: '' });
    }
    if (target._enabled) deliver(target, event); else target._queue.push(event);
  };

  // A transferred port keeps its identity and entanglement inside one agent;
  // only the sender's handle is detached, which is what "disentangle then
  // entangle" means there. Crossing an agent takes the `serialize` half.
  globalThis.__sc_registerTransferable({
    ctor: MessagePort,
    name: 'MessagePort',
    tag: 'port',
    transfer: function(port) {
      if (port._transferred) throw dataClone("An already-transferred MessagePort");
      port._transferred = true;
      // The receiving side re-enables it; clear the sender's enabled flag so a
      // queued message is not delivered to the old owner's listeners. A port
      // that left the agent entirely stays detached.
      setTimeout(function() { if (!port._crossAgent) port._transferred = false; }, 0);
      return port;
    },
    serialize: function(port) {
      var id = String(globalThis.__portAllocId());
      port._crossAgent = true;
      // Undelivered messages travel with the port.
      var queued = [];
      for (var i = 0; i < port._queue.length; i++) {
        queued.push(globalThis.__scSerialize(port._queue[i].data));
      }
      port._queue = [];
      var peer = port._peer;
      var stub = remoteStub(id);
      if (peer) { peer._peer = stub; stub._peer = peer; }
      port._peer = null;
      port._closed = true;
      pendingPortIds.push(id);
      return { id: id, q: queued };
    },
    deserialize: function(rec) {
      var p = new MessagePort();
      var stub = remoteStub(rec.id);
      p._peer = stub;
      stub._peer = p;
      for (var i = 0; i < rec.q.length; i++) {
        p._queue.push(internalMessageEvent(globalThis.__scDeserialize(rec.q[i]), { origin: '' }));
      }
      return p;
    }
  });

  // ---- MessageChannel ----
  function MessageChannel() {
    var a = new MessagePort(), b = new MessagePort();
    a._peer = b; b._peer = a;
    this.port1 = a;
    this.port2 = b;
  }
  defineTag(MessageChannel, 'MessageChannel');
  globalThis.MessageChannel = MessageChannel;

  // ---- BroadcastChannel ----
  var channels = {};   // name -> live channels, in creation order
  function BroadcastChannel(name) {
    if (arguments.length === 0) throw new TypeError("BroadcastChannel requires a name");
    EventTarget.call(this);
    this._name = String(name);
    this._closed = false;
    (channels[this._name] || (channels[this._name] = [])).push(this);
  }
  BroadcastChannel.prototype = Object.create(EventTarget.prototype);
  BroadcastChannel.prototype.constructor = BroadcastChannel;
  Object.defineProperty(BroadcastChannel.prototype, 'name', {
    configurable: true, enumerable: true, get: function() { return this._name; }
  });
  BroadcastChannel.prototype.postMessage = function(message) {
    if (this._closed) throw new DOMException("The channel is closed.", "InvalidStateError");
    var peers = (channels[this._name] || []).slice();
    var self_ = this;
    for (var i = 0; i < peers.length; i++) {
      (function(peer) {
        if (peer === self_ || peer._closed) return;
        // Clone once per destination, as separate agents would.
        var data = globalThis.__structuredClone(message);
        setTimeout(function() {
          if (peer._closed) return;
          peer.dispatchEvent(internalMessageEvent(data, { origin: originOfDocument() }));
        }, 0);
      })(peers[i]);
    }
  };
  BroadcastChannel.prototype.close = function() {
    this._closed = true;
    var list = channels[this._name];
    if (!list) return;
    var i = list.indexOf(this);
    if (i !== -1) list.splice(i, 1);
  };
  defineHandler(BroadcastChannel.prototype, 'message', null);
  defineHandler(BroadcastChannel.prototype, 'messageerror', null);
  defineTag(BroadcastChannel, 'BroadcastChannel');
  globalThis.BroadcastChannel = BroadcastChannel;

  function originOfDocument() {
    var loc = globalThis.location;
    if (!loc || !loc.origin || loc.origin === 'null') return '';
    return loc.origin;
  }

  // ---- window.postMessage ----
  // Replaces the shell stub: a real MessageEvent, a cloned payload, and the
  // transfer list's ports on `event.ports`.
  var nextMessageId = 1;
  globalThis.postMessage = function(message, targetOriginOrOptions, transfer) {
    var targetOrigin = '*';
    if (typeof targetOriginOrOptions === 'string') {
      targetOrigin = targetOriginOrOptions;
    } else if (targetOriginOrOptions && typeof targetOriginOrOptions === 'object') {
      if (targetOriginOrOptions.targetOrigin !== undefined) targetOrigin = String(targetOriginOrOptions.targetOrigin);
      if (transfer === undefined) transfer = targetOriginOrOptions.transfer;
    }
    if (targetOrigin !== '*' && targetOrigin !== '/') {
      // A targetOrigin that is neither literal must be a valid absolute URL.
      var parsed = globalThis.URL && globalThis.URL.parse ? globalThis.URL.parse(targetOrigin) : null;
      if (!parsed) throw new DOMException("Invalid target origin '" + targetOrigin + "'.", "SyntaxError");
    }
    var ports = portsIn(transfer);
    var data = globalThis.__structuredClone(message, transfer);
    var id = String(nextMessageId++);
    __traceProtocol('post_message', 'enqueue', id);
    // The origin check is part of the algorithm, not of delivery: if
    // `targetOrigin` names an origin that is not the target's, the message is
    // **discarded silently** -- no event, no error. The clone still happens
    // first, so a payload that cannot be cloned throws whatever the target
    // origin was.
    if (!targetOriginPermits(targetOrigin)) return;
    setTimeout(function() {
      __traceProtocol('post_message', 'deliver', id);
      dispatchEvent(internalMessageEvent(data, {
        origin: originOfDocument(), source: globalThis, ports: ports
      }));
    }, 0);
  };

  // Whether `targetOrigin` permits delivery to *this* window. `'*'` always
  // does; `'/'` means the sender's own origin, which for a same-window post is
  // always true; anything else is compared as a serialized origin. A document
  // with an opaque origin (`about:`, `data:`, `file:`) serializes to nothing,
  // so an explicit target origin never matches it -- which is the specified
  // behaviour and not a gap.
  function targetOriginPermits(targetOrigin) {
    if (targetOrigin === '*' || targetOrigin === '/') return true;
    var here = originOfDocument();
    if (!here) return false;
    var parsed = globalThis.URL && globalThis.URL.parse
      ? globalThis.URL.parse(targetOrigin)
      : null;
    return !!parsed && parsed.origin === here;
  }
  defineHandler(globalThis, 'message', null);
  defineHandler(globalThis, 'messageerror', null);

  // Shared with the worker surface, which needs the same transfer-list
  // normalization and the same `on<type>` property shape.
  globalThis.__normalizeTransfer = normalizeTransfer;
  globalThis.__defineEventHandler = defineHandler;
  globalThis.__internalMessageEvent = internalMessageEvent;
})();
"#;
