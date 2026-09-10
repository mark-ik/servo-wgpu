/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The structured serialize / deserialize algorithm, and `structuredClone()`.
//!
//! Engine-neutral by construction: a value walk written in JS over the VM's own
//! object model, with no native sink at all. It is its own module because it is
//! substrate — `MessagePort.postMessage`, `BroadcastChannel` and (later) `Worker`
//! all clone through the same walker, and a transferable registers itself with
//! [`__sc_registerTransferable`](STRUCTURED_CLONE_BOOTSTRAP) rather than being
//! special-cased here.
//!
//! Same-realm clones use the fused memoised walk (`__structuredClone`). Realm
//! messages serialize in the sender and deserialize the resulting record in the
//! recipient, so the clone uses the recipient's intrinsics. Worker messages wrap
//! that same record in JSON for transport across agents.
//!
//! Known gaps, in WPT terms: `SharedArrayBuffer` (no shared memory in the scripted
//! tier), `ImageData` / `ImageBitmap` / `CryptoKey` / `DOMException` and the other
//! serializable platform objects that do not exist here yet, and true detachment
//! of a transferred `ArrayBuffer` on a backend without `ArrayBuffer.prototype.transfer`.

use script_engine_api::ScriptEngine;

pub(crate) fn install_structured_clone_surface<E: ScriptEngine>(
    engine: &mut crate::Surface<'_, '_, E>,
) -> Result<(), crate::SurfaceError<E::Error>> {
    engine.eval(STRUCTURED_CLONE_BOOTSTRAP)?;
    Ok(())
}

/// `structuredClone(value, { transfer })` plus the `__structuredClone` /
/// `__sc_registerTransferable` entry points other surfaces clone through.
const STRUCTURED_CLONE_BOOTSTRAP: &str = r#"
(function() {
  'use strict';
  // Capture bootstrap intrinsics before authored code can replace globals.
  // Outgoing records must not retain any source-realm object prototype.
  var objectCreate = Object.create, objectDefine = Object.defineProperty;
  var ownDescriptor = Object.getOwnPropertyDescriptor, ownNames = Reflect.ownKeys;
  var setPrototype = Object.setPrototypeOf, freezeObject = Object.freeze;
  var hasOwn = Object.prototype.hasOwnProperty, apply = Reflect.apply;
  var NativeArray = Array, isArray = Array.isArray, NativeMap = Map;
  var mapGet = Map.prototype.get, mapSet = Map.prototype.set;
  var NativeSet = Set, setAdd = Set.prototype.add;
  var domNodes = globalThis.__agentTimers.domNodes, hasDOMNode = WeakSet.prototype.has;
  var NativeBoolean = Boolean, NativeNumber = Number, NativeString = String;
  var NativeObject = Object, NativeDate = Date, NativeRegExp = RegExp;
  var NativeDataView = DataView, NativeUint8Array = Uint8Array;
  var NativeBlob = globalThis.Blob, NativeFile = globalThis.File;
  var NativeBigInt = globalThis.BigInt, NativeDOMException = DOMException;
  var jsonParse = JSON.parse, jsonStringify = JSON.stringify;
  var decodeBase64 = atob;
  var charCodeAt = String.prototype.charCodeAt;
  function defineData(target, key, value, writable, configurable) {
    var descriptor = objectCreate(null);
    descriptor.value = value;
    descriptor.enumerable = true;
    descriptor.writable = !!writable;
    descriptor.configurable = !!configurable;
    objectDefine(target, key, descriptor);
  }
  function tag(v) { return Object.prototype.toString.call(v); }
  function dataClone(what) {
    return new NativeDOMException(what + " could not be cloned.", "DataCloneError");
  }

  // Transferable kinds registered by the surface that owns them (MessagePort).
  // Each entry: { ctor, transfer(obj) -> record, adopt(record) -> object }.
  var transferables = [];
  globalThis.__sc_registerTransferable = function(entry) { transferables.push(entry); };
  function transferableFor(v) {
    for (var i = 0; i < transferables.length; i++) {
      if (v instanceof transferables[i].ctor) return transferables[i];
    }
    return null;
  }

  var ERROR_NAMES = {
    Error: 1, EvalError: 1, RangeError: 1, ReferenceError: 1,
    SyntaxError: 1, TypeError: 1, URIError: 1
  };
  var VIEWS = ['Int8Array', 'Uint8Array', 'Uint8ClampedArray', 'Int16Array', 'Uint16Array',
               'Int32Array', 'Uint32Array', 'Float32Array', 'Float64Array',
               'BigInt64Array', 'BigUint64Array', 'DataView'];
  var viewConstructors = objectCreate(null), errorConstructors = objectCreate(null);
  for (var vi = 0; vi < VIEWS.length; vi++) viewConstructors[VIEWS[vi]] = globalThis[VIEWS[vi]];
  var errorNames = Object.keys(ERROR_NAMES);
  for (var ei = 0; ei < errorNames.length; ei++) errorConstructors[errorNames[ei]] = globalThis[errorNames[ei]];
  function viewKind(v) {
    for (var i = 0; i < VIEWS.length; i++) {
      var C = globalThis[VIEWS[i]];
      if (C && v instanceof C) return VIEWS[i];
    }
    return null;
  }

  function copyBuffer(buf) {
    return buf.slice(0);
  }

  // Own enumerable string-keyed properties, in own-property order.
  function ownKeys(o) {
    var keys = Object.keys(o), out = [];
    for (var i = 0; i < keys.length; i++) {
      var d = Object.getOwnPropertyDescriptor(o, keys[i]);
      if (d && d.enumerable) out.push(keys[i]);
    }
    return out;
  }

  // The fused serialize/deserialize walk. `memo` preserves identity, so cycles
  // and shared subgraphs come out the far side shared, not duplicated.
  // `moved` maps an already-transferred object to its adopted replacement.
  function clone(v, memo, moved) {
    var t = typeof v;
    if (v === null || t === 'undefined' || t === 'boolean' || t === 'number' ||
        t === 'string' || t === 'bigint') {
      return v;
    }
    if (t === 'symbol') throw dataClone("A symbol");
    if (t === 'function') throw dataClone("A function");

    if (moved.has(v)) return moved.get(v);
    if (memo.has(v)) return memo.get(v);

    var out;
    var kind = tag(v);

    // Boxed primitives.
    if (kind === '[object Boolean]') { out = new Boolean(v.valueOf()); memo.set(v, out); return out; }
    if (kind === '[object Number]') { out = new Number(v.valueOf()); memo.set(v, out); return out; }
    if (kind === '[object String]') { out = new String(v.valueOf()); memo.set(v, out); return out; }
    if (kind === '[object BigInt]') { out = Object(v.valueOf()); memo.set(v, out); return out; }
    if (kind === '[object Date]') { out = new Date(v.getTime()); memo.set(v, out); return out; }
    if (kind === '[object RegExp]') {
      out = new RegExp(v.source, v.flags);
      memo.set(v, out);
      return out;
    }

    if (v instanceof ArrayBuffer) {
      out = copyBuffer(v);
      memo.set(v, out);
      return out;
    }
    var vk = viewKind(v);
    if (vk) {
      var buf = clone(v.buffer, memo, moved);
      out = (vk === 'DataView')
        ? new DataView(buf, v.byteOffset, v.byteLength)
        : new globalThis[vk](buf, v.byteOffset, v.length);
      memo.set(v, out);
      return out;
    }

    // A transferable reached by value (not in the transfer list) is not cloneable.
    var tr = transferableFor(v);
    if (tr) throw dataClone("A " + (tr.name || 'transferable'));

    if (globalThis.Blob && v instanceof globalThis.Blob) {
      if (globalThis.File && v instanceof globalThis.File) {
        out = new globalThis.File([v._b.slice(0)], v.name, { type: v.type, lastModified: v.lastModified });
      } else {
        out = new globalThis.Blob([v._b.slice(0)], { type: v.type });
      }
      memo.set(v, out);
      return out;
    }

    if (kind === '[object Map]') {
      out = new Map();
      memo.set(v, out);
      v.forEach(function(val, key) { out.set(clone(key, memo, moved), clone(val, memo, moved)); });
      copyExtraProps(v, out, memo, moved, null);
      return out;
    }
    if (kind === '[object Set]') {
      out = new Set();
      memo.set(v, out);
      v.forEach(function(val) { out.add(clone(val, memo, moved)); });
      copyExtraProps(v, out, memo, moved, null);
      return out;
    }
    if (Array.isArray(v)) {
      out = new Array(v.length);
      memo.set(v, out);
      for (var i = 0; i < v.length; i++) {
        if (i in v) out[i] = clone(v[i], memo, moved);   // holes stay holes
      }
      copyExtraProps(v, out, memo, moved, /^(?:0|[1-9][0-9]*)$/);
      return out;
    }
    if (v instanceof Error && ERROR_NAMES[v.name]) {
      var C = globalThis[v.name] || Error;
      // Only name, message, cause and stack survive; own extra properties do not.
      // `message` keeps its own-property presence: `new Error` has none.
      var hasMessage = Object.prototype.hasOwnProperty.call(v, 'message');
      out = hasMessage ? new C(String(v.message)) : new C();
      if (!hasMessage && Object.prototype.hasOwnProperty.call(out, 'message')) {
        try { delete out.message; } catch (e) {}
      }
      out.name = v.name;
      memo.set(v, out);
      if (Object.prototype.hasOwnProperty.call(v, 'cause')) {
        out.cause = clone(v.cause, memo, moved);
      }
      if (typeof v.stack === 'string') out.stack = v.stack;
      return out;
    }

    // Platform objects are not cloneable. An ordinary script object is, whatever
    // its prototype: only its own enumerable string-keyed properties travel, so
    // an inherited or non-enumerable property is dropped rather than copied.
    if (isPlatformObject(v)) {
      throw dataClone("An object of type " + (v.constructor && v.constructor.name ? v.constructor.name : kind));
    }
    out = {};
    memo.set(v, out);
    copyExtraProps(v, out, memo, moved, null);
    return out;
  }

  // A platform object: a DOM node, an event, an event target, a promise, or the
  // global itself. The scripted tier has no [Serializable] platform objects
  // beyond Blob / File, which are handled above.
  function isPlatformObject(v) {
    if (v === globalThis) return true;
    if (apply(hasDOMNode, domNodes, [v])) return true;
    if (globalThis.Event && v instanceof globalThis.Event) return true;
    if (globalThis.EventTarget && v instanceof globalThis.EventTarget) return true;
    if (globalThis.Promise && v instanceof globalThis.Promise) return true;
    return false;
  }

  // Own enumerable string keys, skipping those `skip` matches (array indices,
  // already copied). Getters are invoked, as the spec's [[Get]] does.
  function copyExtraProps(src, dst, memo, moved, skip) {
    var keys = ownKeys(src);
    for (var i = 0; i < keys.length; i++) {
      if (skip && skip.test(keys[i])) continue;
      dst[keys[i]] = clone(src[keys[i]], memo, moved);
    }
  }

  // "Transfer" step: each entry leaves its original detached / disentangled and
  // yields the object the receiving side adopts.
  function performTransfer(list) {
    var moved = new Map();
    if (list === undefined || list === null) return moved;
    if (typeof list !== 'object' || typeof list.length !== 'number') {
      throw new TypeError("transfer list is not a sequence");
    }
    var seen = new Set();
    for (var i = 0; i < list.length; i++) {
      var item = list[i];
      if (item === null || typeof item !== 'object') throw dataClone("A non-transferable value");
      if (seen.has(item)) throw dataClone("A duplicate transfer entry");
      seen.add(item);
      if (item instanceof ArrayBuffer) {
        if (item.byteLength === 0 && item.__detached) throw dataClone("A detached ArrayBuffer");
        var adopted = null;
        if (typeof item.transfer === 'function') {
          // Real detachment (ES2024), where the backend implements it.
          try { adopted = item.transfer(); } catch (e) { adopted = null; }
        }
        if (adopted === null) {
          // No backend detach: copy the bytes and shadow the prototype's
          // byteLength getter with an own zero, so the sender's handle reads
          // detached. The buffer's storage is not actually released.
          adopted = item.slice(0);
          try {
            Object.defineProperty(item, 'byteLength', { value: 0, configurable: true });
            Object.defineProperty(item, '__detached', { value: true, configurable: true });
          } catch (e) {}
        }
        moved.set(item, adopted);
        continue;
      }
      var tr = transferableFor(item);
      if (!tr) throw dataClone("An object of type " + (item.constructor && item.constructor.name ? item.constructor.name : 'Object'));
      moved.set(item, tr.transfer(item));
    }
    return moved;
  }

  // The entry point every clone site uses. `transfer` is an optional sequence.
  globalThis.__structuredClone = function(value, transfer) {
    var moved = performTransfer(transfer);
    return clone(value, new Map(), moved);
  };

  globalThis.structuredClone = function(value, options) {
    if (arguments.length === 0) throw new TypeError("structuredClone requires a value");
    var transfer = (options && options.transfer !== undefined) ? options.transfer : undefined;
    return globalThis.__structuredClone(value, transfer);
  };

  // ---- The transportable record (realms and the cross-agent wire) ----
  //
  // The fused walk above cannot cross an agent boundary: its output is live
  // objects in this realm. Serialize in the sender, where its platform-object
  // and transferable brands are registered; deserialize in the recipient so
  // every reconstructed object has the recipient's intrinsics. Inside one agent
  // the record is passed directly. Workers use the JSON wrappers below.

  // Copy only own data descriptors into an immutable, recursively inert graph.
  // This also sanitizes a transferable codec's record, without mutating it.
  // Accessors and executable values are refused rather than called or exposed.
  // Keep Array identity for JSON, but remove its source-realm prototype.
  function inertRecord(value, memo) {
    if (value === null || typeof value === 'undefined' || typeof value === 'string' ||
        typeof value === 'number' || typeof value === 'boolean') return value;
    if (typeof value !== 'object') throw dataClone('An executable clone record');
    var cached = apply(mapGet, memo, [value]);
    if (cached !== undefined) return cached;
    var array = isArray(value), out;
    if (array) {
      var length = ownDescriptor(value, 'length');
      if (!length || !apply(hasOwn, length, ['value'])) throw dataClone('An accessor clone record');
      out = new NativeArray(length.value);
      setPrototype(out, null);
    } else {
      out = objectCreate(null);
    }
    apply(mapSet, memo, [value, out]);
    var keys = ownNames(value);
    for (var i = 0; i < keys.length; i++) {
      var key = keys[i];
      if (typeof key !== 'string') throw dataClone('A symbol clone record key');
      if (array && key === 'length') continue;
      var descriptor = ownDescriptor(value, key);
      if (!descriptor || !apply(hasOwn, descriptor, ['value'])) throw dataClone('An accessor clone record');
      defineData(out, key, inertRecord(descriptor.value, memo), false, false);
    }
    return freezeObject(out);
  }

  function numOut(n) {
    if (n === Infinity) return '+I';
    if (n === -Infinity) return '-I';
    if (n !== n) return 'N';
    if (n === 0 && 1 / n === -Infinity) return '-0';
    return n;
  }
  function numIn(n) {
    if (typeof n === 'number') return n;
    if (n === '+I') return Infinity;
    if (n === '-I') return -Infinity;
    if (n === '-0') return -0;
    return NaN;
  }
  // Bytes travel base64: JSON strings are UTF-16 and would mangle lone bytes.
  function b64(u8) {
    var s = '';
    for (var i = 0; i < u8.length; i++) s += String.fromCharCode(u8[i]);
    return btoa(s);
  }
  function unb64(s) {
    var bin = decodeBase64(s), u = new NativeUint8Array(bin.length);
    for (var i = 0; i < bin.length; i++) u[i] = apply(charCodeAt, bin, [i]);
    return u;
  }

  // Transferables that survive the wire register a `tag` plus `serialize` /
  // `deserialize`; MessagePort is the only one today.
  function codecForTag(t) {
    for (var i = 0; i < transferables.length; i++) {
      if (transferables[i].tag === t) return transferables[i];
    }
    return null;
  }

  function ser(v, h, memo, moved) {
    var t = typeof v;
    if (v === undefined) return ['u'];
    if (v === null) return ['z'];
    if (t === 'boolean') return ['b', v];
    if (t === 'number') return ['d', numOut(v)];
    if (t === 'string') return ['s', v];
    if (t === 'bigint') return ['g', String(v)];
    if (t === 'symbol') throw dataClone("A symbol");
    if (t === 'function') throw dataClone("A function");
    var target = (moved.has(v) && !isTransferableObject(v)) ? moved.get(v) : v;
    if (memo.has(target)) return ['r', memo.get(target)];
    var idx = h.length;
    h.push(null);
    memo.set(target, idx);
    h[idx] = serNode(target, h, memo, moved, moved.has(v));
    return ['r', idx];
  }

  function isTransferableObject(v) {
    return transferableFor(v) !== null;
  }

  function serNode(v, h, memo, moved, transferred) {
    var kind = tag(v);
    if (kind === '[object Boolean]') return { t: 'B', v: v.valueOf() };
    if (kind === '[object Number]') return { t: 'N', v: numOut(v.valueOf()) };
    if (kind === '[object String]') return { t: 'S', v: v.valueOf() };
    if (kind === '[object BigInt]') return { t: 'G', v: String(v.valueOf()) };
    if (kind === '[object Date]') return { t: 'D', v: v.getTime() };
    if (kind === '[object RegExp]') return { t: 'R', s: v.source, f: v.flags };
    if (v instanceof ArrayBuffer) return { t: 'AB', b: b64(new Uint8Array(v)) };
    var vk = viewKind(v);
    if (vk) {
      return {
        t: 'TV', k: vk, o: v.byteOffset,
        l: (vk === 'DataView') ? v.byteLength : v.length,
        buf: ser(v.buffer, h, memo, moved)
      };
    }
    var tr = transferableFor(v);
    if (tr) {
      // A transferable reached by value is not cloneable; one in the transfer
      // list serializes through its own codec.
      if (!transferred || !tr.serialize) throw dataClone("A " + (tr.name || 'transferable'));
      return { t: 'X', x: tr.tag, d: tr.serialize(v) };
    }
    if (globalThis.Blob && v instanceof globalThis.Blob) {
      var rec = { t: 'BL', b: b64(v._b), ty: v.type };
      if (globalThis.File && v instanceof globalThis.File) {
        rec.n = v.name; rec.lm = v.lastModified;
      }
      return rec;
    }
    if (kind === '[object Map]') {
      var mp = [];
      var recM = { t: 'M', p: mp, x2: [] };
      v.forEach(function(val, key) { mp.push([ser(key, h, memo, moved), ser(val, h, memo, moved)]); });
      recM.x2 = serProps(v, h, memo, moved, null);
      return recM;
    }
    if (kind === '[object Set]') {
      var st = [];
      v.forEach(function(val) { st.push(ser(val, h, memo, moved)); });
      return { t: 'E', p: st, x2: serProps(v, h, memo, moved, null) };
    }
    if (Array.isArray(v)) {
      var items = [];
      for (var i = 0; i < v.length; i++) {
        if (i in v) items.push([String(i), ser(v[i], h, memo, moved)]);   // holes stay holes
      }
      return { t: 'A', n: v.length, p: items.concat(serProps(v, h, memo, moved, /^(?:0|[1-9][0-9]*)$/)) };
    }
    if (v instanceof Error && ERROR_NAMES[v.name]) {
      var recE = { t: 'ER', n: v.name };
      if (Object.prototype.hasOwnProperty.call(v, 'message')) recE.m = String(v.message);
      if (Object.prototype.hasOwnProperty.call(v, 'cause')) recE.c = ser(v.cause, h, memo, moved);
      if (typeof v.stack === 'string') recE.st = v.stack;
      return recE;
    }
    if (isPlatformObject(v)) {
      throw dataClone("An object of type " + (v.constructor && v.constructor.name ? v.constructor.name : kind));
    }
    return { t: 'O', p: serProps(v, h, memo, moved, null) };
  }

  function serProps(src, h, memo, moved, skip) {
    var keys = ownKeys(src), out = [];
    for (var i = 0; i < keys.length; i++) {
      if (skip && skip.test(keys[i])) continue;
      out.push([keys[i], ser(src[keys[i]], h, memo, moved)]);
    }
    return out;
  }

  function des(val, h, cache) {
    var t = val[0];
    if (t === 'u') return undefined;
    if (t === 'z') return null;
    if (t === 'b') return val[1];
    if (t === 'd') return numIn(val[1]);
    if (t === 's') return val[1];
    if (t === 'g') return (typeof NativeBigInt === 'function') ? NativeBigInt(val[1]) : NativeNumber(val[1]);
    if (t === 'r') return desNode(val[1], h, cache);
    return undefined;
  }

  function desNode(i, h, cache) {
    if (cache[i] !== undefined) return cache[i];
    var r = h[i], out;
    switch (r.t) {
      case 'B': return (cache[i] = new NativeBoolean(r.v));
      case 'N': return (cache[i] = new NativeNumber(numIn(r.v)));
      case 'S': return (cache[i] = new NativeString(r.v));
      case 'G': return (cache[i] = NativeObject((typeof NativeBigInt === 'function') ? NativeBigInt(r.v) : NativeNumber(r.v)));
      case 'D': return (cache[i] = new NativeDate(r.v));
      case 'R': return (cache[i] = new NativeRegExp(r.s, r.f));
      case 'AB': return (cache[i] = unb64(r.b).buffer);
      case 'TV': {
        var buf = des(r.buf, h, cache);
        out = (r.k === 'DataView') ? new NativeDataView(buf, r.o, r.l) : new viewConstructors[r.k](buf, r.o, r.l);
        return (cache[i] = out);
      }
      case 'BL': {
        var bytes = unb64(r.b);
        out = (r.n !== undefined && NativeFile)
          ? new NativeFile([bytes.buffer], r.n, { type: r.ty, lastModified: r.lm })
          : new NativeBlob([bytes.buffer], { type: r.ty });
        return (cache[i] = out);
      }
      case 'X': {
        var codec = codecForTag(r.x);
        if (!codec || !codec.deserialize) throw dataClone("An unknown transferable");
        return (cache[i] = codec.deserialize(r.d));
      }
      case 'M':
        out = new NativeMap(); cache[i] = out;
        for (var mi = 0; mi < r.p.length; mi++) apply(mapSet, out, [des(r.p[mi][0], h, cache), des(r.p[mi][1], h, cache)]);
        desProps(r.x2, out, h, cache);
        return out;
      case 'E':
        out = new NativeSet(); cache[i] = out;
        for (var si = 0; si < r.p.length; si++) apply(setAdd, out, [des(r.p[si], h, cache)]);
        desProps(r.x2, out, h, cache);
        return out;
      case 'A':
        out = new NativeArray(r.n); cache[i] = out;
        desProps(r.p, out, h, cache);
        return out;
      case 'ER': {
        var C = errorConstructors[r.n] || errorConstructors.Error;
        out = (r.m !== undefined) ? new C(r.m) : new C();
        if (r.m === undefined && apply(hasOwn, out, ['message'])) {
          try { delete out.message; } catch (e) {}
        }
        defineData(out, 'name', r.n, true, true);
        cache[i] = out;
        if (r.c !== undefined) defineData(out, 'cause', des(r.c, h, cache), true, true);
        if (typeof r.st === 'string') defineData(out, 'stack', r.st, true, true);
        return out;
      }
      default:
        out = {}; cache[i] = out;
        desProps(r.p, out, h, cache);
        return out;
    }
  }

  function desProps(pairs, dst, h, cache) {
    for (var i = 0; i < pairs.length; i++) {
      defineData(dst, pairs[i][0], des(pairs[i][1], h, cache), true, true);
    }
  }

  // Build a detached record in the sending realm. It contains data rather than
  // references to the input graph; cycles and aliases are explicit heap indexes.
  function serializeRecord(value, transfer) {
    var moved = performTransfer(transfer);
    var h = [];
    var root = ser(value, h, new Map(), moved);
    return inertRecord({ r: root, h: h }, new NativeMap());
  }
  globalThis.__scSerializeRecord = serializeRecord;

  // Run this function from the receiving realm, even when `rec` was allocated
  // in another realm of this agent. The reconstructed objects are local.
  function deserializeRecord(record) {
    // Validate/copy incoming records by descriptor before following fields.
    // Ordinary accessors are rejected without executing their getter.
    var rec = inertRecord(record, new NativeMap());
    var cache = new NativeArray(rec.h.length);
    setPrototype(cache, null);
    return des(rec.r, rec.h, cache);
  }
  globalThis.__scDeserializeRecord = deserializeRecord;

  // Keep the Worker wire format and entry points unchanged.
  globalThis.__scSerialize = function(value, transfer) {
    return jsonStringify(serializeRecord(value, transfer));
  };
  globalThis.__scDeserialize = function(text) {
    return deserializeRecord(jsonParse(text));
  };
})();
"#;
