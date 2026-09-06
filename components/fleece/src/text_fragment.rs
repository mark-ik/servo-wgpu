// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Pure syntax projection of a [`crate::TextQuoteSelector`] into a URL text directive.
//!
//! The syntax and encoding are pinned to the WICG draft at commit
//! [`b0ac8732fae68380674c86a5825bf3c2152c6439`](https://github.com/WICG/scroll-to-text-fragment/tree/b0ac8732fae68380674c86a5825bf3c2152c6439),
//! `index.bs` (URL Fragment Text Directives). This module generates the
//! directive component only; it does not compose a URL, implement activation,
//! or claim that Fleece's canonical DOM-text stream matches the browser's
//! rendered-text matching stream.

use crate::TextQuoteSelector;

/// A generated `text=` fragment component, without a leading `#` or URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFragment {
    /// The complete fragment-directive component, for example
    /// `:~:text=prefix-,start,-suffix`.
    pub directive: String,
}

impl TextFragment {
    /// Project quote evidence into a text directive.
    ///
    /// An empty exact match cannot identify a text range and therefore returns
    /// `None`. Prefix and suffix are optional context terms.
    pub fn from_quote(quote: &TextQuoteSelector) -> Option<Self> {
        if quote.exact.is_empty() {
            return None;
        }

        let mut value = String::from(":~:text=");
        if !quote.prefix.is_empty() {
            value.push_str(&encode_term(&quote.prefix));
            value.push_str("-,");
        }
        value.push_str(&encode_term(&quote.exact));
        if !quote.suffix.is_empty() {
            value.push_str(",-");
            value.push_str(&encode_term(&quote.suffix));
        }
        Some(Self { directive: value })
    }
}

/// Project a quote selector without requiring callers to name the output type.
pub fn text_fragment(quote: &TextQuoteSelector) -> Option<TextFragment> {
    TextFragment::from_quote(quote)
}

/// Encode a text-directive term as UTF-8 bytes, leaving only URI unreserved
/// characters literal. In particular, delimiters and punctuation are escaped
/// so they cannot change the directive grammar.
fn encode_term(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        // A hyphen is URI-unreserved, but reserved by the text-directive
        // prefix/suffix grammar, so encode it inside every term.
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~') {
            encoded.push(*byte as char);
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    encoded
}
