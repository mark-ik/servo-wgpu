/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Typed selector values for Fleece's preserved `text/plain` resource.

use unicode_segmentation::UnicodeSegmentation;

use crate::{TextAnchor, TextPositionSelector, TextQuoteSelector};

/// The RFC 5147 conformance IRI for character-range fragments.
pub const RFC5147_CONFORMS_TO: &str = "https://www.rfc-editor.org/rfc/rfc5147";

/// A character-range fragment for the canonical `text/plain; charset=utf-8`
/// resource. Positions are zero-based Unicode code-point offsets and the end is
/// exclusive, matching [`TextPositionSelector`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentSelector {
    pub start: u64,
    pub end: u64,
}

impl FragmentSelector {
    /// Construct an RFC 5147 `char=start,end` selector.
    pub const fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }

    /// The RFC 5147 fragment value, without a leading `#`.
    pub fn value(self) -> String {
        format!("char={},{}", self.start, self.end)
    }

    /// Parse an RFC 5147 character-range fragment value, with or without its
    /// leading `#`.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.strip_prefix('#').unwrap_or(value);
        let (start, end) = value.strip_prefix("char=")?.split_once(',')?;
        let start = start.parse().ok()?;
        let end = end.parse().ok()?;
        (start <= end).then_some(Self { start, end })
    }
}

/// The selector triple for one segment of a Fleece canonical-text resource.
///
/// These are sibling Web Annotation selector values. A projection consumer
/// emits `FragmentSelector`, `TextQuoteSelector`, and `TextPositionSelector`
/// for the same [`Self::resource_iri`], rather than treating any one as a
/// refinement of another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalTextSelectorProjection {
    pub resource_iri: String,
    pub fragment: FragmentSelector,
    pub quote: TextQuoteSelector,
    pub position: TextPositionSelector,
}

impl CanonicalTextSelectorProjection {
    /// Construct the typed selector projection for one anchor.
    pub fn from_anchor(resource_iri: impl Into<String>, anchor: &TextAnchor) -> Self {
        Self {
            resource_iri: resource_iri.into(),
            fragment: FragmentSelector::new(anchor.position.start, anchor.position.end),
            quote: anchor.quote.clone(),
            position: anchor.position,
        }
    }

    /// Check that the fragment, quote, and position siblings name the same
    /// segment of one canonical-text value.
    pub fn resolves_against(&self, text: &str) -> bool {
        if self.fragment.start != self.position.start
            || self.fragment.end != self.position.end
            || !valid_range(text, self.position)
            || self.position.start == self.position.end
        {
            return false;
        }
        let exact = text_slice(text, self.position.start, self.position.end);
        if exact != self.quote.exact {
            return false;
        }
        resolve_anchor(
            text,
            &TextAnchor {
                position: self.position,
                quote: self.quote.clone(),
            },
        )
        .into_iter()
        .any(|candidate| candidate == self.position)
    }
}

/// Mint quote and position evidence for a canonical-text range.
///
/// The range must be non-empty, within `text`, and begin and end on grapheme
/// cluster boundaries. `quote_context` is a maximum in Unicode code points on
/// each side; the actual context is shortened instead of splitting a grapheme.
pub fn anchor_for_range(
    text: &str,
    position: TextPositionSelector,
    quote_context: usize,
) -> Option<TextAnchor> {
    if !valid_range(text, position) || position.start == position.end {
        return None;
    }
    let exact = text_slice(text, position.start, position.end);
    Some(TextAnchor {
        position,
        quote: TextQuoteSelector {
            prefix: prefix_context(text, position.start, quote_context),
            exact,
            suffix: suffix_context(text, position.end, quote_context),
        },
    })
}

/// Resolve every canonical-text range consistent with `anchor`.
///
/// The result deliberately retains all matching ranges. A repeated quotation
/// with insufficient quote context is ambiguous evidence, not permission to
/// choose the first occurrence. An invalid quote selector resolves to no range.
pub fn resolve_anchor(text: &str, anchor: &TextAnchor) -> Vec<TextPositionSelector> {
    if anchor.quote.exact.is_empty() {
        return Vec::new();
    }

    text.match_indices(&anchor.quote.exact)
        .filter_map(|(byte_start, exact)| {
            let byte_end = byte_start.checked_add(exact.len())?;
            let before = text.get(..byte_start)?;
            let after = text.get(byte_end..)?;
            (before.ends_with(&anchor.quote.prefix) && after.starts_with(&anchor.quote.suffix))
                .then(|| TextPositionSelector {
                    start: before.chars().count() as u64,
                    end: before.chars().count() as u64 + exact.chars().count() as u64,
                })
        })
        .filter(|position| valid_range(text, *position))
        .collect()
}

/// Whether a position selector names a whole-grapheme, half-open range in
/// `text`. This is also the validation shared by preserved-wire decoding.
pub fn valid_range(text: &str, position: TextPositionSelector) -> bool {
    if position.start > position.end || position.end > text.chars().count() as u64 {
        return false;
    }
    let mut boundary = 0_u64;
    let mut start_found = position.start == 0;
    let mut end_found = position.end == 0;
    for grapheme in UnicodeSegmentation::graphemes(text, true) {
        boundary += grapheme.chars().count() as u64;
        start_found |= boundary == position.start;
        end_found |= boundary == position.end;
    }
    start_found && end_found
}

fn text_slice(text: &str, start: u64, end: u64) -> String {
    text.chars()
        .skip(start as usize)
        .take((end - start) as usize)
        .collect()
}

fn prefix_context(text: &str, end: u64, limit: usize) -> String {
    let before = text_slice(text, 0, end);
    let mut remaining = limit;
    let mut pieces = Vec::new();
    for grapheme in UnicodeSegmentation::graphemes(before.as_str(), true).rev() {
        let width = grapheme.chars().count();
        if width > remaining {
            break;
        }
        pieces.push(grapheme);
        remaining -= width;
    }
    pieces.reverse();
    pieces.concat()
}

fn suffix_context(text: &str, start: u64, limit: usize) -> String {
    let after = text_slice(text, start, text.chars().count() as u64);
    let mut remaining = limit;
    let mut result = String::new();
    for grapheme in UnicodeSegmentation::graphemes(after.as_str(), true) {
        let width = grapheme.chars().count();
        if width > remaining {
            break;
        }
        result.push_str(grapheme);
        remaining -= width;
    }
    result
}
