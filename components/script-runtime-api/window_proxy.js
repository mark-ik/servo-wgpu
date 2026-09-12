// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

// HTML's WindowProxy: one per browsing context, forwarding every operation to
// whichever `Window` the context currently holds. Held by the context and not
// by a realm, so a navigation that replaces the realm leaves every reference
// the parent took - `contentWindow`, a `MessageEvent.source`, a stashed
// `frames[0]` - pointing at the same object, now reading the new document.
//
// One object, and the cross-origin decision made *inside* it, per accessing
// realm. HTML has no second object for a cross-origin window: the same
// WindowProxy answers differently depending on who is asking, because
// `frame.contentWindow === frame.contentWindow` has to hold across a navigation
// that changes the frame's origin. The adapter's eleven native trap getters run
// in the accessing script's realm - a Proxy's internal methods do not switch
// realms - and each one stamps that realm on the shadow before handing the trap
// over, so every trap below knows who is asking and asks the host to compare
// origins through the browsing-context tree.
//
// This is the only script that runs between the adapter installing the proxy as
// the realm's global `this` and the proxy having a handler, so it depends on
// nothing but the intrinsics and touches `globalThis` only to read the proxy
// out of it: until it finishes, `globalThis` is transparent to the shadow
// rather than to the global object, and anything else written through it would
// land on the wrong object. The realm's own global object arrives as
// `__windowProxyGlobal` and the shadow as `__windowProxyTarget`, both installed
// by the host with no code running. The shadow slot is deleted here; the global
// one stays, because the surface bootstraps need it to define HTML's
// [LegacyUnforgeable] attributes on the Window rather than through the proxy -
// a non-configurable property defined *through* a Proxy pins its descriptor on
// the proxy's target for good, and these have to survive the navigations that
// replace the Window behind it. Every trap below hides that slot, so nothing
// reached through the WindowProxy can see it.
//
// The shadow is the proxy's [[ProxyTarget]] and holds nothing but the mirrors
// the Proxy invariants demand; the handler holds the current [[Window]] and
// forwards to it. That is what lets the realm a context started in be discarded
// on the first navigation out of it: the proxy, its handler and its shadow are
// all that survive, and none of them is a global object.
//
// Nothing is published on any global. The completion value is a control
// function the host retains, and it is the whole interface:
//
//   control('proxy')               - the WindowProxy itself
//   control('bind', window, realm) - the [[Window]] write, returning the proxy
//   control('window')              - the Window the context currently holds
//   control('host', hooks)         - install the current realm's host hook, the
//                                    native the cross-origin branch dispatches
//                                    through; re-supplied on every navigation,
//                                    so it never outlives its realm
//   control('descriptor', key)     - the shadow's own descriptor for `key`, for
//                                    the runtime's own receipts
(function () {
  'use strict';
  var globalObject = __windowProxyGlobal;
  var shadow = __windowProxyTarget;
  // Already the WindowProxy: the host finished the realm's global `this`
  // initialization before evaluating this.
  var proxy = globalThis;
  var define = Object.defineProperty;
  var create = Object.create;
  var reflect = Reflect;
  var ownDescriptor = Object.getOwnPropertyDescriptor;
  var HANDLER_SLOT = '__windowProxyHandler';
  var TARGET_SLOT = '__windowProxyTarget';
  var GLOBAL_SLOT = '__windowProxyGlobal';
  // Where the adapter's native trap getters stamp the accessing realm.
  var ACCESS_SLOT = '__windowProxyAccess';
  // What the host's property hook answers with when the key is not one of the
  // cross-origin-accessible ones.
  var DENIED = '__security_error__';
  var symbols = [Symbol.toStringTag, Symbol.hasInstance, Symbol.isConcatSpreadable];
  // HTML's CrossOriginProperties(W), in order, as [name, needsGet, needsSet].
  // Everything absent from this list is a SecurityError across origins.
  var CROSS_ORIGIN = [
    ['window', 1, 0], ['self', 1, 0], ['location', 1, 1], ['close', 0, 0],
    ['closed', 1, 0], ['focus', 0, 0], ['blur', 0, 0], ['frames', 1, 0],
    ['length', 1, 0], ['top', 1, 0], ['opener', 1, 0], ['parent', 1, 0],
    ['postMessage', 0, 0]
  ];
  delete globalObject[TARGET_SLOT];

  // The [[Window]] slot, and the realm it belongs to. Held here rather than
  // resolved through the host on each access, which is both faster and what
  // keeps a discarded context's Window answering: a parent holding this proxy
  // still reaches the document and `closed` flag of the context it lost.
  var win = null;
  var winRealm = null;
  // The current realm's host hook, installed once that realm has a surface and
  // replaced on every navigation, so it never outlives the realm that owns it.
  // Absent until the first surface exists, which is why the same-origin path
  // may not depend on it.
  var hooks = null;
  // HTML's CrossOriginPropertyDescriptorMap, keyed by accessing realm: the
  // methods and the cross-origin `Location` a realm sees have to be the same
  // objects every time it looks, or `w.close === w.close` would be false.
  var crossOriginCaches = create(null);

  function indexOf(list, value) {
    for (var i = 0; i < list.length; i++) if (list[i] === value) return i;
    return -1;
  }

  function hidden(property) {
    return property === HANDLER_SLOT || property === TARGET_SLOT
      || property === GLOBAL_SLOT || property === ACCESS_SLOT;
  }

  // ── who is asking ──────────────────────────────────────────────────────────

  // The realm the adapter's trap getter stamped on the way in. Falls back to
  // the context's own realm before any surface exists, and for a host-initiated
  // operation.
  function accessor() {
    var value = shadow[ACCESS_SLOT];
    return value === undefined ? winRealm : value;
  }

  // Whether the accessing realm may see through to the Window, decided against
  // the *current* document's origin: a navigation that changes origin flips
  // every reference the other side is already holding.
  function permitted() {
    if (win === null) return false;
    if (hooks === null || winRealm === null) return true;
    var from = accessor();
    if (from === null || from === undefined || from === winRealm) return true;
    return hooks('sameOrigin', from, winRealm) !== false;
  }

  // A `SecurityError` built with the *accessing* realm's intrinsics, so
  // `instanceof DOMException` holds on the side that catches it.
  function denied() {
    if (hooks !== null) throw hooks('denied', accessor());
    throw new TypeError('Cross-origin window access is denied');
  }

  // ── same-origin: mirrors for the Proxy invariants ──────────────────────────

  // Mirror one non-configurable property of the Window onto the shadow and
  // report the mirror, so the Proxy invariants hold without ever claiming an
  // unforgeable property is configurable.
  //
  // The mirror of an accessor is a *stable* forwarding getter rather than the
  // Window's own, because a non-configurable accessor's descriptor may not
  // change and navigation replaces the Window's getters. That is the one
  // deviation: the function object such a descriptor hands back is the proxy's,
  // not the Window's. Property access itself does not go through it.
  function mirror(property, desc, verbatim) {
    var own = reflect.getOwnPropertyDescriptor(shadow, property);
    if (own && own.configurable === false) return own;
    var copy = create(null);
    copy.configurable = false;
    copy.enumerable = desc.enumerable;
    if ('value' in desc) {
      copy.value = desc.value;
      copy.writable = desc.writable;
    } else if (verbatim) {
      copy.get = desc.get;
      copy.set = desc.set;
    } else {
      if (desc.get) copy.get = function () { return win === null ? undefined : win[property]; };
      if (desc.set) copy.set = function (value) { if (win !== null) win[property] = value; };
    }
    reflect.defineProperty(shadow, property, copy);
    return reflect.getOwnPropertyDescriptor(shadow, property);
  }

  // The shadow's non-configurable own keys, appended to `out`. A Proxy must
  // report every one of them from `ownKeys`, in either branch.
  function pinnedKeys(out) {
    var mirrored = reflect.ownKeys(shadow);
    for (var i = 0; i < mirrored.length; i++) {
      var key = mirrored[i];
      if (hidden(key)) continue;
      var desc = reflect.getOwnPropertyDescriptor(shadow, key);
      if (!desc || desc.configurable !== false) continue;
      if (indexOf(out, key) < 0) out[out.length] = key;
    }
    return out;
  }

  // ── cross-origin ───────────────────────────────────────────────────────────

  function dataDescriptor(value, enumerable, writable) {
    var descriptor = create(null);
    descriptor.value = value;
    descriptor.writable = !!writable;
    descriptor.enumerable = !!enumerable;
    descriptor.configurable = true;
    return descriptor;
  }

  // HTML's CrossOriginPropertyFallback: four keys answer `undefined` rather
  // than throwing, so a cross-origin window is not thenable and does not break
  // `Array.prototype.concat` or `Object.prototype.toString`.
  function fallback(key) {
    if (key === 'then' || indexOf(symbols, key) >= 0) {
      return dataDescriptor(undefined, false, false);
    }
    return denied();
  }

  function cacheFor(from) {
    var key = String(from);
    var cache = crossOriginCaches[key];
    if (cache === undefined) {
      cache = create(null);
      crossOriginCaches[key] = cache;
    }
    return cache;
  }

  // One cross-origin-accessible read, through the host: the value is minted in
  // the browsing-context tree, so a window it answers with is the other
  // context's own WindowProxy and not a copy.
  function crossRead(key) {
    var value = hooks('property', winRealm, String(key), accessor());
    if (value === DENIED) return denied();
    return value;
  }

  // The cross-origin `Location`: `href` is set-only and `replace` navigates.
  // Nothing else is readable, which is the whole of what a cross-origin
  // document may learn about where another one is.
  // Cached under a key no property name can collide with: the descriptor cache
  // this shares is keyed by property name, and `location` is one of them.
  var LOCATION_SLOT = '@location';
  function crossLocation(cache) {
    if (cache[LOCATION_SLOT] !== undefined) return cache[LOCATION_SLOT];
    var target = winRealm;
    var from = accessor();
    var href = create(null);
    href.get = undefined;
    href.set = function (value) { hooks('navigate', target, String(value), 'assign', from); };
    href.enumerable = false;
    href.configurable = true;
    var replace = dataDescriptor(function replace(value) {
      hooks('navigate', target, String(value), 'replace', from);
    }, false, false);
    var location = new Proxy(create(null), crossHandler(function (key) {
      if (key === 'href') return href;
      if (key === 'replace') return replace;
      return fallback(key);
    }, function () {
      var keys = ['href', 'replace'];
      keys[keys.length] = 'then';
      for (var i = 0; i < symbols.length; i++) keys[keys.length] = symbols[i];
      return keys;
    }));
    cache[LOCATION_SLOT] = location;
    return location;
  }

  // The shared shape of a cross-origin exotic object: every operation answers
  // from one descriptor function, the prototype is null, and anything the
  // descriptor function refuses is a SecurityError.
  function crossHandler(descriptor, keys) {
    var handler = create(null);
    handler.getOwnPropertyDescriptor = function (_, key) { return descriptor(key); };
    handler.has = function (_, key) { return descriptor(key) !== undefined; };
    handler.get = function (_, key, receiver) {
      var desc = descriptor(key);
      if ('value' in desc) return desc.value;
      if (!desc.get) return denied();
      return reflect.apply(desc.get, receiver, []);
    };
    handler.set = function (_, key, value, receiver) {
      var desc = descriptor(key);
      if (!desc.set) return denied();
      reflect.apply(desc.set, receiver, [value]);
      return true;
    };
    handler.defineProperty = denied;
    handler.deleteProperty = denied;
    handler.ownKeys = keys;
    handler.getPrototypeOf = function () { return null; };
    handler.setPrototypeOf = function (_, value) { return value === null; };
    handler.isExtensible = function () { return true; };
    handler.preventExtensions = function () { return false; };
    return handler;
  }

  function makeCrossNoop(key) {
    var fn = function () {};
    define(fn, 'name', { value: key, configurable: true });
    return fn;
  }

  function makeCrossPostMessage(from) {
    var target = winRealm;
    return function postMessage(message) {
      return hooks('post', target, message, arguments[1], arguments[2], from);
    };
  }

  function crossAccessor(key, needsGet, needsSet) {
    var descriptor = create(null);
    if (needsGet) {
      descriptor.get = key === 'location'
        ? function () { return crossLocation(cacheFor(accessor())); }
        : function () { return crossRead(key); };
    } else {
      descriptor.get = undefined;
    }
    descriptor.set = needsSet
      ? function (value) {
          hooks('navigate', winRealm, String(value), 'assign', accessor());
        }
      : undefined;
    descriptor.enumerable = false;
    descriptor.configurable = true;
    return descriptor;
  }

  // HTML's CrossOriginGetOwnPropertyHelper, plus the indexed and named child
  // browsing contexts, which are cross-origin-accessible in their own right.
  function crossDescriptor(key) {
    if (typeof key !== 'string') return fallback(key);
    var from = accessor();
    var cache = cacheFor(from);
    if (cache[key] !== undefined) return cache[key];
    for (var i = 0; i < CROSS_ORIGIN.length; i++) {
      var entry = CROSS_ORIGIN[i];
      if (entry[0] !== key) continue;
      if (!entry[1] && !entry[2]) {
        // A method. `close`, `focus` and `blur` are the three a cross-origin
        // document may call and that genet does nothing for.
        var method = key === 'postMessage' ? makeCrossPostMessage(from) : makeCrossNoop(key);
        cache[key] = dataDescriptor(method, false, false);
        return cache[key];
      }
      cache[key] = crossAccessor(key, entry[1], entry[2]);
      return cache[key];
    }
    // Only canonical array indexes enter the indexed-property path.
    var index = Number(key);
    if (index >= 0 && index < 4294967295 && String(index) === key
      && Math.floor(index) === index) {
      var child = hooks('property', winRealm, key, accessor());
      if (child === DENIED || child === undefined || child === null) return denied();
      return dataDescriptor(child, true, false);
    }
    // A named child browsing context is reachable across origins; anything
    // else is not.
    var named = hooks('named', winRealm, key, accessor());
    if (named !== undefined && named !== null && named !== DENIED) {
      return dataDescriptor(named, false, false);
    }
    return fallback(key);
  }

  function crossKeys() {
    var keys = [], length = Number(crossRead('length')) || 0, i;
    for (i = 0; i < length; i++) keys[keys.length] = String(i);
    for (i = 0; i < CROSS_ORIGIN.length; i++) keys[keys.length] = CROSS_ORIGIN[i][0];
    keys[keys.length] = 'then';
    for (i = 0; i < symbols.length; i++) keys[keys.length] = symbols[i];
    // A same-origin inspection before the context went cross-origin can have
    // pinned a mirror on the shadow, and a Proxy must keep reporting it.
    return pinnedKeys(keys);
  }

  // ── the handler ────────────────────────────────────────────────────────────

  var handler = create(null);
  handler.get = function (_, property, receiver) {
    if (win === null || hidden(property)) return undefined;
    if (!permitted()) {
      var desc = crossDescriptor(property);
      if ('value' in desc) return desc.value;
      if (!desc.get) return denied();
      return reflect.apply(desc.get, receiver, []);
    }
    // The receiver forwarded to an accessor is the Window and not the proxy, so
    // host accessors installed on the global see the `this` they were written
    // for.
    return reflect.get(win, property, win);
  };
  handler.set = function (_, property, value, receiver) {
    if (win === null || hidden(property)) return true;
    if (!permitted()) {
      var desc = crossDescriptor(property);
      if (!desc.set) return denied();
      reflect.apply(desc.set, receiver, [value]);
      return true;
    }
    return reflect.set(win, property, value, win);
  };
  handler.has = function (_, property) {
    if (win === null || hidden(property)) return false;
    if (!permitted()) return crossDescriptor(property) !== undefined;
    return reflect.has(win, property);
  };
  handler.deleteProperty = function (_, property) {
    if (win === null || hidden(property)) return true;
    if (!permitted()) return denied();
    return reflect.deleteProperty(win, property);
  };
  handler.ownKeys = function () {
    if (win !== null && !permitted()) return crossKeys();
    var source = win === null ? shadow : win;
    var keys = reflect.ownKeys(source), out = [], i;
    for (i = 0; i < keys.length; i++) {
      if (!hidden(keys[i])) out[out.length] = keys[i];
    }
    // Every non-configurable own key of the target must be reported, and a
    // navigation can leave the shadow holding a mirror the new Window lacks.
    return pinnedKeys(out);
  };
  handler.getOwnPropertyDescriptor = function (_, property) {
    if (win === null || hidden(property)) return undefined;
    if (!permitted()) return crossDescriptor(property);
    var desc = reflect.getOwnPropertyDescriptor(win, property);
    if (!desc) {
      var own = reflect.getOwnPropertyDescriptor(shadow, property);
      return own && own.configurable === false ? own : undefined;
    }
    return desc.configurable ? desc : mirror(property, desc);
  };
  handler.defineProperty = function (_, property, descriptor) {
    if (win === null || hidden(property)) return true;
    if (!permitted()) return denied();
    var ok = reflect.defineProperty(win, property, descriptor);
    if (ok && descriptor.configurable === false) {
      // Verbatim: a [[DefineOwnProperty]] invariant check compares what the
      // caller asked for against the target, so a forwarding stand-in would be
      // rejected. Script defining a non-configurable accessor on a Window is
      // rare, and the descriptor it pins on the shadow is the price.
      var desc = reflect.getOwnPropertyDescriptor(win, property);
      if (desc) mirror(property, desc, true);
    }
    return ok;
  };
  // Cross-origin, a WindowProxy's [[GetPrototypeOf]] is null: the other
  // document's `Window.prototype` is not reachable.
  handler.getPrototypeOf = function () {
    if (win === null) return null;
    return permitted() ? reflect.getPrototypeOf(win) : null;
  };
  // [[SetPrototypeOf]] on a WindowProxy is SetImmutablePrototype: it succeeds
  // only for the prototype the object already has.
  handler.setPrototypeOf = function (_, value) {
    if (win === null) return false;
    if (!permitted()) return value === null;
    return value === reflect.getPrototypeOf(win);
  };
  handler.isExtensible = function () { return true; };
  handler.preventExtensions = function () { return false; };

  define(shadow, HANDLER_SLOT, {
    value: handler, writable: true, enumerable: false, configurable: true
  });
  // The realm this bootstrap runs in is the context's first Window.
  win = globalObject;

  return function control(operation, argument, second) {
    if (operation === 'bind') {
      if (argument !== undefined && argument !== null) {
        win = argument;
        if (second !== undefined) winRealm = second;
        // A new document is a new set of cross-origin method identities.
        crossOriginCaches = create(null);
      }
      return proxy;
    }
    if (operation === 'window') return win;
    if (operation === 'host') {
      hooks = argument === undefined || argument === null ? null : argument;
      return true;
    }
    if (operation === 'descriptor') {
      // Host-side introspection, used by the runtime's own receipts.
      return ownDescriptor(shadow, String(argument));
    }
    return proxy;
  };
})()
