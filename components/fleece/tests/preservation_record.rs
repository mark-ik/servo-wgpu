// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Conformance fixtures for the identity and range portions of Fleece's first
//! preserved extraction profile.

use fleece::{
    CANONICAL_TEXT_MEDIA_TYPE, EXTRACTION_RECORD_SCHEMA_V1, ExtractionOptions,
    ReaderSelectionProfile, TextNormalization, TextPositionSelector, anchor_for_range,
    extract_document_with_options, resolve_anchor, valid_range,
};
use genet_static_dom::StaticDocument;

fn document(html: &str) -> StaticDocument {
    StaticDocument::parse(&format!("<html><body>{html}</body></html>"))
}

#[test]
fn article_and_non_article_documents_both_carry_complete_identity() {
    let article = extract_document_with_options(
        &document(
            "<main><h1>Unicode records</h1><p>This article has enough prose to select the reader body.</p></main>",
        ),
        ExtractionOptions { quote_context: 7 },
    );
    assert!(
        article.article.is_some(),
        "fixture should select an article"
    );

    let non_article = extract_document_with_options(
        &document(
            "<nav><a href='/one'>One destination</a><a href='/two'>Another destination</a></nav>",
        ),
        ExtractionOptions { quote_context: 7 },
    );
    assert!(
        non_article.article.is_none(),
        "fixture should remain non-article"
    );

    for record in [&article, &non_article] {
        assert_eq!(record.contract.schema_id, EXTRACTION_RECORD_SCHEMA_V1);
        assert_eq!(
            record.contract.canonical_text.media_type,
            CANONICAL_TEXT_MEDIA_TYPE
        );
        assert!(
            record
                .contract
                .canonical_text
                .iri
                .starts_with("urn:sha256:")
        );
        assert_eq!(
            record.contract.normalization,
            TextNormalization::FleeceDomTextV1
        );
        assert_eq!(
            record.contract.reader_profile,
            ReaderSelectionProfile::FleeceReaderV1
        );
        assert_eq!(record.contract.quote_context, 7);
        assert_eq!(record.contract.fleece_version, env!("CARGO_PKG_VERSION"));
    }

    let article_lineage = &article.article.as_ref().unwrap().lineage;
    assert_eq!(
        article_lineage.normalization,
        article.contract.normalization
    );
    assert_eq!(
        article_lineage.fleece_version,
        article.contract.fleece_version
    );
}

#[test]
fn arbitrary_unicode_ranges_use_code_points_and_whole_graphemes() {
    let text = "a🙂e\u{301}אב";
    let position = TextPositionSelector { start: 1, end: 4 };
    let anchor = anchor_for_range(text, position, 8).expect("valid Unicode range");

    assert_eq!(anchor.position, position);
    assert_eq!(anchor.quote.exact, "🙂e\u{301}");
    assert_eq!(anchor.quote.exact.chars().count(), 3);
    assert_eq!(resolve_anchor(text, &anchor), vec![position]);
    assert!(valid_range(text, position));

    // The combining mark is part of the preceding grapheme and cannot be cut
    // into a second selector range.
    assert!(!valid_range(
        text,
        TextPositionSelector { start: 3, end: 4 }
    ));
    assert!(anchor_for_range(text, TextPositionSelector { start: 3, end: 4 }, 8).is_none());
}

#[test]
fn repeated_quote_resolution_preserves_all_matches() {
    let text = "before repeat between repeat after";
    let first_start = text.find("repeat").unwrap() as u64;
    let first_start = text[..first_start as usize].chars().count() as u64;
    let first = TextPositionSelector {
        start: first_start,
        end: first_start + "repeat".chars().count() as u64,
    };
    let anchor = anchor_for_range(text, first, 0).expect("non-empty range");

    assert_eq!(anchor.quote.prefix, "");
    assert_eq!(anchor.quote.suffix, "");
    assert_eq!(
        resolve_anchor(text, &anchor),
        vec![first, TextPositionSelector { start: 22, end: 28 }]
    );
}

#[test]
fn invalid_and_out_of_bounds_ranges_are_rejected() {
    let text = "e\u{301}abc";
    let count = text.chars().count() as u64;
    for position in [
        TextPositionSelector { start: 4, end: 3 },
        TextPositionSelector {
            start: 0,
            end: count + 1,
        },
        TextPositionSelector { start: 1, end: 2 },
    ] {
        assert!(
            !valid_range(text, position),
            "unexpectedly valid: {position:?}"
        );
        assert!(anchor_for_range(text, position, 4).is_none());
    }
    assert!(valid_range(text, TextPositionSelector { start: 2, end: 2 }));
    assert!(anchor_for_range(text, TextPositionSelector { start: 2, end: 2 }, 4).is_none());

    let malformed = fleece::TextAnchor {
        position: TextPositionSelector {
            start: 99,
            end: 100,
        },
        quote: fleece::TextQuoteSelector {
            exact: String::new(),
            prefix: String::new(),
            suffix: String::new(),
        },
    };
    assert!(resolve_anchor(text, &malformed).is_empty());
}

#[cfg(feature = "wire")]
#[test]
fn wire_record_round_trips_independently_and_keeps_missing_anchor_states() {
    use fleece::CanonicalTextRecordV1;

    let extracted = extract_document_with_options(
        &document("<main><p>A🙂</p></main>"),
        ExtractionOptions { quote_context: 4 },
    );
    assert_eq!(extracted.page.text, "A🙂");
    let anchor = anchor_for_range(
        &extracted.page.text,
        TextPositionSelector { start: 0, end: 2 },
        extracted.contract.quote_context,
    )
    .unwrap();
    let record = CanonicalTextRecordV1::new(
        &extracted.contract,
        extracted.page.text.clone(),
        extracted.language_direction_spans.clone(),
        [Some(anchor), None],
    );

    let encoded = record.encode().expect("valid record encodes");
    let expected = concat!(
        "{\"schema_id\":\"https://merely.dev/ns/fleece/extraction-record/v1\",",
        "\"fleece_version\":\"",
        env!("CARGO_PKG_VERSION"),
        "\",\"normalization\":\"FleeceDomTextV1\",",
        "\"reader_profile\":\"FleeceReaderV1\",\"quote_context\":4,",
        "\"canonical_text_iri\":\"urn:sha256:daadb46c3ede2ecedd4d91b3c1f165f2b9edc3e22d4a09f74c62ba6c20b48e43\",",
        "\"media_type\":\"text/plain; charset=utf-8\",\"canonical_text\":\"A🙂\",",
        "\"language_direction_spans\":[{\"start\":0,\"end\":2,\"language\":{\"declared\":null,\"declaration_is_valid\":null,\"effective\":null},\"direction\":{\"declared\":null,\"declaration_is_valid\":null,\"effective\":\"ltr\"}}],",
        "\"anchors\":[{\"start\":0,\"end\":2,\"exact\":\"A🙂\",\"prefix\":\"\",\"suffix\":\"\"},null]}"
    );
    assert_eq!(String::from_utf8(encoded.clone()).unwrap(), expected);

    let decoded = CanonicalTextRecordV1::decode(&encoded).expect("independent decode");
    assert_eq!(decoded, record);
    assert_eq!(decoded.anchors.len(), 2);
    assert!(decoded.anchors[1].is_none());
    assert_eq!(
        decoded.contract().canonical_text.iri,
        extracted.contract.canonical_text.iri
    );

    let invalid_range = String::from_utf8(encoded.clone()).unwrap().replace(
        "\"end\":2,\"exact\":\"A🙂\"",
        "\"end\":99,\"exact\":\"A🙂\"",
    );
    assert!(matches!(
        CanonicalTextRecordV1::decode(invalid_range.as_bytes()),
        Err(fleece::WireError::AnchorRange)
    ));
    let invalid_exact = String::from_utf8(encoded)
        .unwrap()
        .replace("\"exact\":\"A🙂\"", "\"exact\":\"wrong\"");
    assert!(matches!(
        CanonicalTextRecordV1::decode(invalid_exact.as_bytes()),
        Err(fleece::WireError::AnchorExact)
    ));
}
