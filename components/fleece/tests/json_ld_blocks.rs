// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use fleece::{
    JsonLdParseStatus, StructuredDataSource, StructuredValue, extract, extract_json_ld_blocks,
    extract_structured_data,
};
use genet_static_dom::StaticDocument;

const HTML: &str = r#"<html><head>
<script id="same" type=" Application/LD+JSON ; charset=utf-8">
  "scalar"
</script>
<script id="same" type="application/ld+json">[{"@id":"one"},2,null]</script>
<script id="" type="application/ld+json"></script>
<script type="application/ld+json">{"broken":</script>
<script type="text/javascript">{"ignored":true}</script>
</head><body>
<script type="application/ld+json">{"@id":"last","@type":"Thing"}</script>
<svg><script type="application/ld+json">{"svg":"ignored"}</script></svg>
</body></html>"#;

#[test]
fn retains_every_html_json_ld_script_before_projection() {
    let dom = StaticDocument::parse(HTML);
    let blocks = extract_json_ld_blocks(&dom);
    assert_eq!(extract(&dom).json_ld_blocks, blocks);
    assert_eq!(blocks.len(), 5);
    assert!(
        blocks
            .windows(2)
            .all(|pair| pair[0].document_order < pair[1].document_order)
    );

    assert_eq!(blocks[0].element_id.as_deref(), Some("same"));
    assert_eq!(
        blocks[0].declared_type,
        " Application/LD+JSON ; charset=utf-8"
    );
    assert_eq!(blocks[0].dom_text, "\n  \"scalar\"\n");
    assert_eq!(
        blocks[0].parse,
        JsonLdParseStatus::Parsed(StructuredValue::String("scalar".into()))
    );

    assert_eq!(blocks[1].element_id.as_deref(), Some("same"));
    assert!(matches!(
        blocks[1].parse,
        JsonLdParseStatus::Parsed(StructuredValue::Array(_))
    ));
    assert_eq!(blocks[2].element_id.as_deref(), Some(""));
    assert_eq!(blocks[2].dom_text, "");
    assert_eq!(blocks[2].parse, JsonLdParseStatus::InvalidJson);
    assert_eq!(blocks[3].parse, JsonLdParseStatus::InvalidJson);
    assert_eq!(blocks[4].element_id, None);
}

#[test]
fn legacy_structured_data_is_a_derived_convenience_projection() {
    let data = extract_structured_data(&StaticDocument::parse(HTML));
    let json_ld = data
        .iter()
        .filter(|item| item.source == StructuredDataSource::JsonLd)
        .collect::<Vec<_>>();
    assert_eq!(json_ld.len(), 2);
    assert_eq!(json_ld[0].id.as_deref(), Some("one"));
    assert_eq!(json_ld[1].id.as_deref(), Some("last"));
}

#[cfg(feature = "wire")]
#[test]
fn wire_record_round_trips_and_reparses_the_retained_dom_text() {
    use fleece::JsonLdBlockRecordV1;

    let blocks = extract_json_ld_blocks(&StaticDocument::parse(HTML));
    for block in &blocks {
        let record = JsonLdBlockRecordV1::from_block(block);
        let encoded = record.encode().expect("valid record encodes");
        let decoded = JsonLdBlockRecordV1::decode(&encoded).expect("valid record decodes");
        assert_eq!(decoded.block().expect("validated block"), *block);
    }
}

#[cfg(feature = "wire")]
#[test]
fn wire_decode_rejects_hash_profile_and_parse_status_tampering() {
    use fleece::{JsonLdBlockRecordV1, WireError, WireJsonLdParseStatus};

    let blocks = extract_json_ld_blocks(&StaticDocument::parse(HTML));
    let mut record = JsonLdBlockRecordV1::from_block(&blocks[0]);
    record.dom_text.push(' ');
    assert_eq!(record.validate(), Err(WireError::JsonLdTextIdentity));

    let mut record = JsonLdBlockRecordV1::from_block(&blocks[0]);
    record.schema_id = "future-schema".into();
    assert_eq!(record.validate(), Err(WireError::JsonLdSchema));

    let mut record = JsonLdBlockRecordV1::from_block(&blocks[0]);
    record.parser_profile = "future-parser".into();
    assert_eq!(record.validate(), Err(WireError::JsonLdParserProfile));

    let mut record = JsonLdBlockRecordV1::from_block(&blocks[0]);
    record.declared_type = "text/javascript".into();
    assert_eq!(record.validate(), Err(WireError::JsonLdMediaType));

    let mut parsed = JsonLdBlockRecordV1::from_block(&blocks[0]);
    parsed.parse_status = WireJsonLdParseStatus::InvalidJson;
    let bytes = serde_json::to_vec(&parsed).unwrap();
    assert_eq!(
        JsonLdBlockRecordV1::decode(&bytes),
        Err(WireError::JsonLdParseStatus)
    );

    let mut invalid = JsonLdBlockRecordV1::from_block(&blocks[3]);
    invalid.parse_status = WireJsonLdParseStatus::Parsed;
    let bytes = serde_json::to_vec(&invalid).unwrap();
    assert_eq!(
        JsonLdBlockRecordV1::decode(&bytes),
        Err(WireError::JsonLdParseStatus)
    );
}
