// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use fleece::{ExtractionOptions, TextDirection, extract_document_with_options};
use genet_static_dom::StaticDocument;

fn document(html: &str) -> StaticDocument {
    StaticDocument::parse(html)
}

fn span_for<'a>(
    record: &'a fleece::ExtractedDocument,
    exact: &str,
) -> &'a fleece::LanguageDirectionSpan {
    record
        .language_direction_spans
        .iter()
        .find(|span| {
            record
                .page
                .text
                .chars()
                .skip(span.position.start as usize)
                .take((span.position.end - span.position.start) as usize)
                .collect::<String>()
                == exact
        })
        .unwrap_or_else(|| panic!("missing canonical text range {exact:?}"))
}

#[test]
fn canonical_ranges_retain_inherited_mixed_and_invalid_language_direction() {
    let record = extract_document_with_options(
        &document(
            r#"<html lang="en-GB" dir="rtl"><body><main><p>
            Parent prose is long enough to make this a reader document.
            <span lang="fr" dir="ltr">bonjour</span>
            <span lang="not a tag" dir="sideways">broken</span>
            <bdi>שלום</bdi>
            </p></main></body></html>"#,
        ),
        ExtractionOptions { quote_context: 8 },
    );

    let parent = span_for(
        &record,
        "Parent prose is long enough to make this a reader document.",
    );
    assert_eq!(parent.language.declared.as_deref(), Some("en-GB"));
    assert_eq!(parent.language.declaration_is_valid, Some(true));
    assert_eq!(parent.language.effective.as_deref(), Some("en-GB"));
    assert_eq!(parent.direction.declared.as_deref(), Some("rtl"));
    assert_eq!(parent.direction.effective, TextDirection::Rtl);

    let french = span_for(&record, "bonjour");
    assert_eq!(french.language.declared.as_deref(), Some("fr"));
    assert_eq!(french.language.effective.as_deref(), Some("fr"));
    assert_eq!(french.direction.declared.as_deref(), Some("ltr"));
    assert_eq!(french.direction.effective, TextDirection::Ltr);

    let invalid = span_for(&record, "broken");
    assert_eq!(invalid.language.declared.as_deref(), Some("not a tag"));
    assert_eq!(invalid.language.declaration_is_valid, Some(false));
    assert_eq!(invalid.language.effective.as_deref(), Some("not a tag"));
    assert_eq!(invalid.direction.declared.as_deref(), Some("sideways"));
    assert_eq!(invalid.direction.declaration_is_valid, Some(false));
    assert_eq!(invalid.direction.effective, TextDirection::Rtl);

    let bidi = span_for(&record, "שלום");
    assert_eq!(bidi.language.effective.as_deref(), Some("en-GB"));
    assert_eq!(bidi.direction.declared.as_deref(), Some("rtl"));
    assert_eq!(bidi.direction.effective, TextDirection::Rtl);

    let paragraph = record
        .article
        .as_ref()
        .and_then(|article| {
            article
                .blocks
                .iter()
                .find_map(|block| block.anchor.as_ref())
        })
        .expect("reader paragraph anchor");
    let covered = record.language_direction_for_anchor(paragraph);
    assert!(
        covered
            .iter()
            .any(|span| span.language.effective.as_deref() == Some("fr"))
    );
    assert!(
        covered
            .iter()
            .any(|span| span.direction.effective == TextDirection::Rtl)
    );
}

#[test]
fn xml_lang_precedes_html_lang_and_auto_uses_first_strong_character() {
    let record = extract_document_with_options(
        &StaticDocument::parse_xml(
            r#"<html xmlns="http://www.w3.org/1999/xhtml" lang="en"><body><main><p><span xml:lang="fr" lang="de">bonjour</span>
            <span dir="AUTO"><span dir="ltr">ignored nested direction</span>שלום</span>
            enough surrounding prose to select this reader body.</p></main></body></html>"#,
        ),
        ExtractionOptions::default(),
    );

    let xml = span_for(&record, "bonjour");
    assert_eq!(xml.language.declared.as_deref(), Some("fr"));
    assert_eq!(xml.language.effective.as_deref(), Some("fr"));

    let auto = span_for(&record, "שלום");
    assert_eq!(auto.direction.declared.as_deref(), Some("AUTO"));
    assert_eq!(auto.direction.declaration_is_valid, Some(true));
    assert_eq!(auto.direction.effective, TextDirection::Rtl);
}

#[cfg(feature = "wire")]
#[test]
fn wire_record_preserves_mixed_language_and_bidi_ranges() {
    use fleece::{CanonicalTextRecordV1, WireError};

    let extracted = extract_document_with_options(
        &document(
            r#"<html lang="en" dir="rtl"><body><main><p>English source text
            <span lang="fr" dir="ltr">bonjour</span> <bdi>שלום</bdi>
            with enough prose for a reader article.</p></main></body></html>"#,
        ),
        ExtractionOptions::default(),
    );
    let record = CanonicalTextRecordV1::from_document(&extracted);
    let encoded = record.encode().expect("valid mixed-language record");
    let decoded = CanonicalTextRecordV1::decode(&encoded).expect("validated record");
    assert_eq!(
        decoded.language_direction_spans,
        record.language_direction_spans
    );
    assert!(
        decoded
            .language_direction_spans
            .iter()
            .any(|span| span.language.effective.as_deref() == Some("fr"))
    );
    assert!(
        decoded
            .language_direction_spans
            .iter()
            .any(|span| span.direction.effective == fleece::WireTextDirection::Rtl)
    );

    let encoded = String::from_utf8(encoded).unwrap();
    let invalid_language = encoded.clone().replace(
        "\"declaration_is_valid\":true",
        "\"declaration_is_valid\":false",
    );
    assert!(matches!(
        CanonicalTextRecordV1::decode(invalid_language.as_bytes()),
        Err(WireError::LanguageEvidence)
    ));
    let invalid_direction = encoded.replace(
        "\"declared\":\"rtl\",\"declaration_is_valid\":true,\"effective\":\"rtl\"",
        "\"declared\":\"rtl\",\"declaration_is_valid\":true,\"effective\":\"auto\"",
    );
    assert!(matches!(
        CanonicalTextRecordV1::decode(invalid_direction.as_bytes()),
        Err(WireError::DirectionEvidence)
    ));
}
