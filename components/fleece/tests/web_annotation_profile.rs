// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Conformance fixtures for Fleece's `text/plain` Web Annotation selectors.

use fleece::{
    CANONICAL_TEXT_MEDIA_TYPE, CanonicalTextSelectorProjection, ExtractionOptions,
    FragmentSelector, RFC5147_CONFORMS_TO, TextPositionSelector, anchor_for_range,
    extract_document_with_options,
};
use genet_static_dom::StaticDocument;

fn document(html: &str) -> StaticDocument {
    StaticDocument::parse(&format!("<html><body>{html}</body></html>"))
}

fn slice(text: &str, position: TextPositionSelector) -> String {
    text.chars()
        .skip(position.start as usize)
        .take((position.end - position.start) as usize)
        .collect()
}

#[test]
fn rfc5147_character_ranges_round_trip() {
    let selector = FragmentSelector::new(12, 29);
    assert_eq!(selector.value(), "char=12,29");
    assert_eq!(FragmentSelector::parse(&selector.value()), Some(selector));
    assert_eq!(FragmentSelector::parse("#char=12,29"), Some(selector));
    assert_eq!(
        RFC5147_CONFORMS_TO,
        "https://www.rfc-editor.org/rfc/rfc5147"
    );

    for invalid in [
        "char=29,12",
        "char=12",
        "char=12,",
        "char=x,29",
        "chars=12,29",
        "#char=12,29,30",
    ] {
        assert_eq!(FragmentSelector::parse(invalid), None, "accepted {invalid}");
    }
}

#[test]
fn fragment_quote_and_position_are_sibling_selectors_for_one_resource() {
    let record = extract_document_with_options(
        &document(
            "<main><h1>Annotation profile</h1><p>Choose the preserved text resource as the annotation target.</p></main>",
        ),
        ExtractionOptions { quote_context: 6 },
    );
    let anchor = record
        .article
        .as_ref()
        .unwrap()
        .blocks
        .iter()
        .find_map(|block| block.anchor.as_ref())
        .expect("reader fixture should expose an anchor");
    let projection = record.selector_projection(anchor);

    assert_eq!(
        projection,
        CanonicalTextSelectorProjection {
            resource_iri: record.contract.canonical_text.iri.clone(),
            fragment: FragmentSelector::new(anchor.position.start, anchor.position.end),
            quote: anchor.quote.clone(),
            position: anchor.position,
        }
    );
    assert_eq!(
        projection.fragment.value(),
        format!("char={},{}", anchor.position.start, anchor.position.end)
    );
    assert_eq!(projection.fragment.start, projection.position.start);
    assert_eq!(projection.fragment.end, projection.position.end);
    assert_eq!(
        projection.quote.exact,
        slice(&record.page.text, projection.position)
    );
    assert_eq!(
        record.contract.canonical_text.media_type,
        CANONICAL_TEXT_MEDIA_TYPE
    );
}

#[test]
fn selectors_can_be_projected_for_an_arbitrary_unicode_segment() {
    let text = "नमस्ते 🙂 שלום";
    let position = TextPositionSelector { start: 7, end: 9 };
    let anchor = anchor_for_range(text, position, 4).expect("Unicode segment");
    let projection = CanonicalTextSelectorProjection::from_anchor("urn:sha256:test", &anchor);

    assert_eq!(projection.resource_iri, "urn:sha256:test");
    assert_eq!(projection.fragment, FragmentSelector::new(7, 9));
    assert_eq!(projection.position, position);
    assert_eq!(projection.quote.exact, "🙂 ");
    assert_eq!(
        FragmentSelector::parse(&projection.fragment.value()),
        Some(projection.fragment)
    );
}
