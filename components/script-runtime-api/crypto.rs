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
//! The default source is a ChaCha20 stream seeded from `RandomState` (the OS
//! entropy `std` already keeps for hash seeds) plus address entropy. It links no
//! crate outside `std`, so the wasm cone keeps carrying no `getrandom`. A host
//! that has a real OS CSPRNG installs it with [`crate::Runtime::set_random_source`].

use std::cell::RefCell;
use std::hash::{BuildHasher, Hasher};

use script_engine_api::{CallCx, NativeFn, ScriptEngine};

use crate::HostState;

/// The host's cryptographically secure random source, backing
/// `crypto.getRandomValues` and `crypto.randomUUID`. Install one with
/// [`crate::Runtime::set_random_source`]; without it the built-in default runs.
pub trait RandomSource {
    /// Fill `buf` with unpredictable bytes.
    fn fill(&self, buf: &mut [u8]);
}

/// The default source: a ChaCha20 keystream over a seed taken from `std`'s
/// OS-seeded hash keys and an ASLR-dependent address.
#[derive(Default)]
pub(crate) struct DefaultRandom {
    state: RefCell<Option<ChaCha20>>,
}

impl DefaultRandom {
    fn fill(&self, buf: &mut [u8]) {
        let mut slot = self.state.borrow_mut();
        let rng = slot.get_or_insert_with(|| ChaCha20::new(seed_words()));
        rng.fill(buf);
    }
}

/// 256 bits of seed material: four independent `RandomState` keys (each derived
/// from the process's OS-seeded hash seed) mixed with a stack address.
fn seed_words() -> [u32; 8] {
    let mut out = [0u32; 8];
    let probe = 0u64;
    let mut mix = &probe as *const u64 as u64;
    for slot in out.chunks_mut(2) {
        let state = std::collections::hash_map::RandomState::new();
        let mut hasher = state.build_hasher();
        hasher.write_u64(mix);
        let v = hasher.finish();
        mix = mix.rotate_left(17) ^ v;
        slot[0] = v as u32;
        slot[1] = (v >> 32) as u32;
    }
    out
}

/// ChaCha20 as a keystream generator (RFC 8439 block function, 64-bit counter).
struct ChaCha20 {
    key: [u32; 8],
    counter: u64,
    block: [u8; 64],
    used: usize,
}

impl ChaCha20 {
    fn new(key: [u32; 8]) -> Self {
        Self {
            key,
            counter: 0,
            block: [0; 64],
            used: 64,
        }
    }

    fn fill(&mut self, buf: &mut [u8]) {
        for byte in buf.iter_mut() {
            if self.used == 64 {
                self.refill();
            }
            *byte = self.block[self.used];
            self.used += 1;
        }
    }

    fn refill(&mut self) {
        let mut s = [0u32; 16];
        s[0] = 0x6170_7865;
        s[1] = 0x3320_646e;
        s[2] = 0x7962_2d32;
        s[3] = 0x6b20_6574;
        s[4..12].copy_from_slice(&self.key);
        s[12] = self.counter as u32;
        s[13] = (self.counter >> 32) as u32;
        let mut w = s;
        for _ in 0..10 {
            quarter(&mut w, 0, 4, 8, 12);
            quarter(&mut w, 1, 5, 9, 13);
            quarter(&mut w, 2, 6, 10, 14);
            quarter(&mut w, 3, 7, 11, 15);
            quarter(&mut w, 0, 5, 10, 15);
            quarter(&mut w, 1, 6, 11, 12);
            quarter(&mut w, 2, 7, 8, 13);
            quarter(&mut w, 3, 4, 9, 14);
        }
        for i in 0..16 {
            let v = w[i].wrapping_add(s[i]);
            self.block[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        self.counter = self.counter.wrapping_add(1);
        self.used = 0;
    }
}

fn quarter(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(7);
}

/// `__crypto_random(n)` → `n` random bytes as a lossless binary string (each JS
/// char code is one byte), the same convention the fetch sink uses.
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
            None => DEFAULT.with(|d| d.fill(&mut bytes)),
        }
        let s: String = bytes.iter().map(|&b| b as char).collect();
        cx.make_string(&s)
    }
}

thread_local! {
    /// One default generator per thread: seeded on first use, then a stream.
    static DEFAULT: DefaultRandom = DefaultRandom::default();
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
