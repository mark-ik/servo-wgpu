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
//! Serialize and deserialize are fused into one memoised walk (`__structuredClone`),
//! which is what an in-process clone needs; nothing in the scripted tier yet needs a
//! detached serialization record that outlives the walk.
//!
//! Known gaps, in WPT terms: `SharedArrayBuffer` (no shared memory in the scripted
//! tier), `ImageData` / `ImageBitmap` / `CryptoKey` / `DOMException` and the other
//! serializable platform objects that do not exist here yet, and true detachment
//! of a transferred `ArrayBuffer` on a backend without `ArrayBuffer.prototype.transfer`.

use script_engine_api::ScriptEngine;

pub(crate) fn install_structured_clone_surface<E: ScriptEngine>(
    engine: &mut E,
) -> Result<(), E::Error> {
    engine.eval(STRUCTURED_CLONE_BOOTSTRAP)?;
    Ok(())
}

/// `structuredClone(value, { transfer })` plus the `__structuredClone` /
/// `__sc_registerTransferable` entry points other surfaces clone through.
const STRUCTURED_CLONE_BOOTSTRAP: &str = r#"
(function() {
  function tag(v) { return Object.prototype.toString.call(v); }
  function dataClone(what) {
    return new DOMException(what + " could not be cloned.", "DataCloneError");
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
    if (globalThis.Node && v instanceof globalThis.Node) return true;
    if (typeof v.nodeType === 'number' && typeof v.nodeName === 'string') return true;
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
})();
"#;
