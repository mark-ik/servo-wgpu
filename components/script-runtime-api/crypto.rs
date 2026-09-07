/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! `crypto.getRandomValues` / `crypto.randomUUID` over a host random source.
//!
//! Same shape as the other surfaces: one native sink reaching [`HostState`], plus
//! a JS bootstrap. `crypto.subtle` is deliberately absent — Web Crypto's
//! algorithms are not implemented — but `crypto` is a real object with
//! `Crypto.prototype`, so feature detection works instead of throwing a
//! `ReferenceError` on the way in.
//!
//! The default source is the operating system's CSPRNG through the `getrandom`
//! crate on native targets. On wasm there is no default: the host installs the
//! browser's own source with [`crate::Runtime::set_random_source`], and until it
//! does `getRandomValues` throws `NotSupportedError` rather than return bytes
//! that are not random. Nothing in this file generates randomness itself.

use std::cell::RefCell;

use script_engine_api::{CallCx, NativeFn, ScriptEngine};

use crate::HostState;

/// The host's cryptographically secure random source, backing
/// `crypto.getRandomValues` and `crypto.randomUUID`. Install one with
/// [`crate::Runtime::set_random_source`]; without it the built-in default runs.
pub trait RandomSource {
    /// Fill `buf` with unpredictable bytes.
    fn fill(&self, buf: &mut [u8]);
}

/// Fill `buf` from the platform default, or report that there is none.
#[cfg(not(target_arch = "wasm32"))]
fn default_fill(buf: &mut [u8]) -> bool {
    getrandom::fill(buf).is_ok()
}

/// No platform default on wasm: the host must install a source.
#[cfg(target_arch = "wasm32")]
fn default_fill(_buf: &mut [u8]) -> bool {
    false
}

/// `__crypto_random(n)` → `n` random bytes as a lossless binary string (each JS
/// char code is one byte), the same convention the fetch sink uses, or `null`
/// when no random source exists on this host.
struct CryptoRandom;
impl<E: ScriptEngine> NativeFn<E> for CryptoRandom {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let arg = cx.arg(0);
        let n = cx
            .value_to_string(&arg)?
            .trim()
            .parse::<usize>()
            .unwrap_or(0)
            .min(65536);
        let mut bytes = vec![0u8; n];
        let source = cx.host_data().and_then(|d| {
            d.downcast_ref::<RefCell<HostState>>()
                .and_then(|h| h.borrow().random.clone())
        });
        match source {
            Some(source) => source.fill(&mut bytes),
            None => {
                if !default_fill(&mut bytes) {
                    return Ok(cx.make_null());
                }
            },
        }
        let s: String = bytes.iter().map(|&b| b as char).collect();
        cx.make_string(&s)
    }
}

pub(crate) fn install_crypto_surface<E: ScriptEngine>(engine: &mut E) -> Result<(), E::Error> {
    engine.set_function::<CryptoRandom>("__crypto_random", 1)?;
    engine.eval(CRYPTO_BOOTSTRAP)?;
    Ok(())
}

/// `Crypto` with `getRandomValues` (integer typed arrays only, 65536-byte cap)
/// and `randomUUID` (a version 4 UUID). `subtle` is absent by design.
const CRYPTO_BOOTSTRAP: &str = r#"
(function() {
  var INTEGER_VIEWS = {
    Int8Array: 1, Uint8Array: 1, Uint8ClampedArray: 1, Int16Array: 1,
    Uint16Array: 1, Int32Array: 1, Uint32Array: 1, BigInt64Array: 1, BigUint64Array: 1
  };
  function randomBytes(n) {
    var s = __crypto_random(n), out = new Uint8Array(n);
    if (s === null) {
      throw new DOMException("No random source is installed on this host", "NotSupportedError");
    }
    for (var i = 0; i < n; i++) { out[i] = s.charCodeAt(i) & 0xFF; }
    return out;
  }
  function Crypto() {}
  Crypto.prototype.getRandomValues = function(view) {
    if (!ArrayBuffer.isView(view) || !INTEGER_VIEWS[view.constructor && view.constructor.name]) {
      throw new DOMException("Argument is not an integer-typed ArrayBufferView", "TypeMismatchError");
    }
    if (view.byteLength > 65536) {
      throw new DOMException("getRandomValues: view is longer than 65536 bytes", "QuotaExceededError");
    }
    var bytes = randomBytes(view.byteLength);
    var dst = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
    dst.set(bytes);
    return view;
  };
  var HEX = [];
  for (var i = 0; i < 256; i++) { HEX[i] = (i + 0x100).toString(16).slice(1); }
  Crypto.prototype.randomUUID = function() {
    var b = randomBytes(16);
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant 1
    return HEX[b[0]] + HEX[b[1]] + HEX[b[2]] + HEX[b[3]] + '-' +
           HEX[b[4]] + HEX[b[5]] + '-' + HEX[b[6]] + HEX[b[7]] + '-' +
           HEX[b[8]] + HEX[b[9]] + '-' +
           HEX[b[10]] + HEX[b[11]] + HEX[b[12]] + HEX[b[13]] + HEX[b[14]] + HEX[b[15]];
  };
  if (typeof Symbol !== 'undefined' && Symbol.toStringTag) {
    Crypto.prototype[Symbol.toStringTag] = 'Crypto';
  }
  globalThis.Crypto = Crypto;
  globalThis.crypto = new Crypto();
})();
"#;
