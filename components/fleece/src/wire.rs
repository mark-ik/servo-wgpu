/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Optional stable wire values for canonical text, source anchors, and embedded
//! JSON-LD syntax.
//!
//! This is deliberately a narrow preservation boundary: it carries Fleece's
//! deterministic canonical text, contract, a caller-selected sequence of
//! present or absent anchors, and lossless page-carried JSON-LD blocks. Storage,
//! capture identity, JSON-LD semantic processing, and Annotation envelopes remain
//! caller-owned.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    AnchoredBlock, Block, CANONICAL_TEXT_MEDIA_TYPE, DirectionEvidence,
    EXTRACTION_RECORD_SCHEMA_V1, EmbeddedJsonLdBlock, ExtractedDocument, ExtractionContract,
    JSON_LD_BLOCK_RECORD_SCHEMA_V1, JSON_LD_PARSER_PROFILE_V1, JsonLdParseStatus,
    LanguageDirectionSpan, LanguageEvidence, ReaderSelectionProfile, TextAnchor, TextDirection,
    TextPositionSelector, TextQuoteSelector, resolve_anchor, valid_range,
};

/// Stable JSON record for one embedded HTML JSON-LD script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonLdBlockRecordV1 {
    pub schema_id: String,
    pub parser_profile: String,
    pub dom_text_sha256: String,
    pub document_order: u64,
    pub element_id: Option<String>,
    pub declared_type: String,
    pub dom_text: String,
    pub parse_status: WireJsonLdParseStatus,
}

/// Portable JSON syntax outcome for a preserved JSON-LD block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireJsonLdParseStatus {
    Parsed,
    InvalidJson,
}

/// Stable JSON record for canonical text and a sequence of optional anchors.
///
/// `anchors` retains `None` entries so synthetic reader blocks and other missing
/// anchor states do not become indistinguishable from omitted records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalTextRecordV1 {
    pub schema_id: String,
    pub fleece_version: String,
    pub normalization: String,
    pub reader_profile: String,
    pub quote_context: u64,
    pub canonical_text_iri: String,
    pub media_type: String,
    pub canonical_text: String,
    /// Ordered source-node language/direction ranges in canonical text.
    pub language_direction_spans: Vec<WireLanguageDirectionSpan>,
    pub anchors: Vec<Option<WireTextAnchor>>,
}

/// Serializable language/direction evidence for one canonical-text range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireLanguageDirectionSpan {
    pub start: u64,
    pub end: u64,
    pub language: WireLanguageEvidence,
    pub direction: WireDirectionEvidence,
}

/// Serializable raw and effective language evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireLanguageEvidence {
    pub declared: Option<String>,
    pub declaration_is_valid: Option<bool>,
    pub effective: Option<String>,
}

/// The portable spelling of an effective HTML direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WireTextDirection {
    Ltr,
    Rtl,
    Auto,
}

/// Serializable raw and effective direction evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireDirectionEvidence {
    pub declared: Option<String>,
    pub declaration_is_valid: Option<bool>,
    pub effective: WireTextDirection,
}

/// Serializable quote and position evidence for one preserved segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireTextAnchor {
    pub start: u64,
    pub end: u64,
    pub exact: String,
    pub prefix: String,
    pub suffix: String,
}

/// A validation or JSON-codec failure while reopening a Fleece wire record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    Json(String),
    Schema,
    Normalization,
    ReaderProfile,
    QuoteContext,
    MediaType,
    CanonicalTextIdentity,
    LanguageDirectionRange,
    LanguageDirectionOrder,
    LanguageEvidence,
    DirectionEvidence,
    AnchorRange,
    AnchorExact,
    JsonLdSchema,
    JsonLdParserProfile,
    JsonLdMediaType,
    JsonLdTextIdentity,
    JsonLdParseStatus,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "invalid Fleece wire record: {self:?}")
    }
}

impl std::error::Error for WireError {}

impl JsonLdBlockRecordV1 {
    /// Preserve one extracted JSON-LD script without duplicating its parsed
    /// value. Reopening reparses the retained DOM text under the named profile.
    pub fn from_block(block: &EmbeddedJsonLdBlock) -> Self {
        Self {
            schema_id: JSON_LD_BLOCK_RECORD_SCHEMA_V1.to_string(),
            parser_profile: JSON_LD_PARSER_PROFILE_V1.to_string(),
            dom_text_sha256: format!("{:x}", Sha256::digest(block.dom_text.as_bytes())),
            document_order: block.document_order,
            element_id: block.element_id.clone(),
            declared_type: block.declared_type.clone(),
            dom_text: block.dom_text.clone(),
            parse_status: WireJsonLdParseStatus::from(&block.parse),
        }
    }

    /// Encode this record as UTF-8 JSON after validation.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| WireError::Json(error.to_string()))
    }

    /// Decode, reparse, and validate one record before returning it.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let record: Self =
            serde_json::from_slice(bytes).map_err(|error| WireError::Json(error.to_string()))?;
        record.validate()?;
        Ok(record)
    }

    /// Validate schema identity, parser profile, DOM-text identity, and the
    /// stored syntax outcome by reparsing the retained text.
    pub fn validate(&self) -> Result<(), WireError> {
        if self.schema_id != JSON_LD_BLOCK_RECORD_SCHEMA_V1 {
            return Err(WireError::JsonLdSchema);
        }
        if self.parser_profile != JSON_LD_PARSER_PROFILE_V1 {
            return Err(WireError::JsonLdParserProfile);
        }
        if !crate::structured::is_json_ld_media_type(&self.declared_type) {
            return Err(WireError::JsonLdMediaType);
        }
        let expected = format!("{:x}", Sha256::digest(self.dom_text.as_bytes()));
        if self.dom_text_sha256 != expected {
            return Err(WireError::JsonLdTextIdentity);
        }
        let reparsed = crate::structured::parse_json_ld_dom_text(&self.dom_text);
        if self.parse_status != wire_json_ld_parse_status(reparsed.as_ref()) {
            return Err(WireError::JsonLdParseStatus);
        }
        Ok(())
    }

    /// Reopen the lossless extraction value after validation.
    pub fn block(&self) -> Result<EmbeddedJsonLdBlock, WireError> {
        self.validate()?;
        let parse = crate::structured::parse_json_ld_dom_text(&self.dom_text)
            .map_or(JsonLdParseStatus::InvalidJson, JsonLdParseStatus::Parsed);
        Ok(EmbeddedJsonLdBlock {
            document_order: self.document_order,
            element_id: self.element_id.clone(),
            declared_type: self.declared_type.clone(),
            dom_text: self.dom_text.clone(),
            parse,
        })
    }
}

impl From<&JsonLdParseStatus> for WireJsonLdParseStatus {
    fn from(status: &JsonLdParseStatus) -> Self {
        match status {
            JsonLdParseStatus::Parsed(_) => Self::Parsed,
            JsonLdParseStatus::InvalidJson => Self::InvalidJson,
        }
    }
}

fn wire_json_ld_parse_status(value: Option<&crate::StructuredValue>) -> WireJsonLdParseStatus {
    if value.is_some() {
        WireJsonLdParseStatus::Parsed
    } else {
        WireJsonLdParseStatus::InvalidJson
    }
}

impl CanonicalTextRecordV1 {
    /// Preserve a document's canonical text and every reader anchor in stable
    /// block traversal order. The optional article remains absent for pages
    /// without a selected reader root.
    pub fn from_document(document: &ExtractedDocument) -> Self {
        let mut anchors = Vec::new();
        if let Some(article) = &document.article {
            collect_anchors(&article.blocks, &mut anchors);
        }
        Self::new(
            &document.contract,
            document.page.text.clone(),
            document.language_direction_spans.clone(),
            anchors,
        )
    }

    /// Create a record using an extraction contract and the canonical text it
    /// identifies. The caller must provide source language/direction spans so a
    /// manually assembled record cannot silently discard them. `anchors` may
    /// retain arbitrary missing-anchor states.
    pub fn new(
        contract: &ExtractionContract,
        canonical_text: impl Into<String>,
        language_direction_spans: impl IntoIterator<Item = LanguageDirectionSpan>,
        anchors: impl IntoIterator<Item = Option<TextAnchor>>,
    ) -> Self {
        Self {
            schema_id: contract.schema_id.to_string(),
            fleece_version: contract.fleece_version.clone(),
            normalization: "FleeceDomTextV1".to_string(),
            reader_profile: "FleeceReaderV1".to_string(),
            quote_context: contract.quote_context as u64,
            canonical_text_iri: contract.canonical_text.iri.clone(),
            media_type: contract.canonical_text.media_type.to_string(),
            canonical_text: canonical_text.into(),
            language_direction_spans: language_direction_spans
                .into_iter()
                .map(WireLanguageDirectionSpan::from)
                .collect(),
            anchors: anchors
                .into_iter()
                .map(|anchor| anchor.map(WireTextAnchor::from))
                .collect(),
        }
    }

    /// Encode the stable record as UTF-8 JSON.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| WireError::Json(error.to_string()))
    }

    /// Decode and validate a record before returning it to a caller.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let record: Self =
            serde_json::from_slice(bytes).map_err(|error| WireError::Json(error.to_string()))?;
        record.validate()?;
        Ok(record)
    }

    /// Validate the canonical-text identity, source language/direction spans,
    /// and every retained source anchor.
    pub fn validate(&self) -> Result<(), WireError> {
        if self.schema_id != EXTRACTION_RECORD_SCHEMA_V1 {
            return Err(WireError::Schema);
        }
        if self.normalization != "FleeceDomTextV1" {
            return Err(WireError::Normalization);
        }
        if self.reader_profile != "FleeceReaderV1" {
            return Err(WireError::ReaderProfile);
        }
        if usize::try_from(self.quote_context).is_err() {
            return Err(WireError::QuoteContext);
        }
        if self.media_type != CANONICAL_TEXT_MEDIA_TYPE {
            return Err(WireError::MediaType);
        }
        let expected_iri = crate::CanonicalTextResource::for_text(&self.canonical_text).iri;
        if self.canonical_text_iri != expected_iri {
            return Err(WireError::CanonicalTextIdentity);
        }
        let mut previous_end = 0;
        for span in &self.language_direction_spans {
            let position = TextPositionSelector {
                start: span.start,
                end: span.end,
            };
            if !valid_range(&self.canonical_text, position) || position.start >= position.end {
                return Err(WireError::LanguageDirectionRange);
            }
            if span.start < previous_end {
                return Err(WireError::LanguageDirectionOrder);
            }
            previous_end = span.end;
            validate_language_evidence(&span.language)?;
            validate_direction_evidence(&span.direction)?;
        }
        for anchor in self.anchors.iter().flatten() {
            let position = TextPositionSelector {
                start: anchor.start,
                end: anchor.end,
            };
            if !valid_range(&self.canonical_text, position) || position.start >= position.end {
                return Err(WireError::AnchorRange);
            }
            let exact = self
                .canonical_text
                .chars()
                .skip(position.start as usize)
                .take((position.end - position.start) as usize)
                .collect::<String>();
            if exact != anchor.exact {
                return Err(WireError::AnchorExact);
            }
            let anchor = TextAnchor::from(anchor.clone());
            if !resolve_anchor(&self.canonical_text, &anchor)
                .into_iter()
                .any(|candidate| candidate == anchor.position)
            {
                return Err(WireError::AnchorExact);
            }
        }
        Ok(())
    }

    /// Reconstruct the extraction contract. Use [`Self::decode`] before
    /// reopening untrusted bytes; it validates this value first.
    pub fn contract(&self) -> ExtractionContract {
        ExtractionContract {
            schema_id: EXTRACTION_RECORD_SCHEMA_V1,
            canonical_text: crate::CanonicalTextResource {
                iri: self.canonical_text_iri.clone(),
                media_type: CANONICAL_TEXT_MEDIA_TYPE,
            },
            normalization: crate::TextNormalization::FleeceDomTextV1,
            reader_profile: ReaderSelectionProfile::FleeceReaderV1,
            quote_context: self.quote_context as usize,
            fleece_version: self.fleece_version.clone(),
        }
    }
}

fn validate_language_evidence(evidence: &WireLanguageEvidence) -> Result<(), WireError> {
    if evidence.declaration_is_valid
        != evidence
            .declared
            .as_deref()
            .map(crate::is_language_declaration)
    {
        return Err(WireError::LanguageEvidence);
    }
    (evidence.effective == evidence.declared)
        .then_some(())
        .ok_or(WireError::LanguageEvidence)
}

fn validate_direction_evidence(evidence: &WireDirectionEvidence) -> Result<(), WireError> {
    if evidence.declaration_is_valid
        != evidence
            .declared
            .as_deref()
            .map(|value| crate::parse_direction(value).is_some())
    {
        return Err(WireError::DirectionEvidence);
    }
    if evidence.effective == WireTextDirection::Auto {
        return Err(WireError::DirectionEvidence);
    }
    if let Some(direction) = evidence
        .declared
        .as_deref()
        .and_then(crate::parse_direction)
        && direction != TextDirection::Auto
        && WireTextDirection::from(direction) != evidence.effective
    {
        return Err(WireError::DirectionEvidence);
    }
    Ok(())
}

fn collect_anchors(blocks: &[AnchoredBlock], out: &mut Vec<Option<TextAnchor>>) {
    for anchored in blocks {
        out.push(anchored.anchor.clone());
        match &anchored.block {
            Block::List { items, .. } => {
                for item in items {
                    collect_anchors(item, out);
                }
            },
            Block::Quote { blocks } => collect_anchors(blocks, out),
            _ => {},
        }
    }
}

impl From<TextAnchor> for WireTextAnchor {
    fn from(anchor: TextAnchor) -> Self {
        Self {
            start: anchor.position.start,
            end: anchor.position.end,
            exact: anchor.quote.exact,
            prefix: anchor.quote.prefix,
            suffix: anchor.quote.suffix,
        }
    }
}

impl From<WireTextAnchor> for TextAnchor {
    fn from(anchor: WireTextAnchor) -> Self {
        Self {
            position: TextPositionSelector {
                start: anchor.start,
                end: anchor.end,
            },
            quote: TextQuoteSelector {
                exact: anchor.exact,
                prefix: anchor.prefix,
                suffix: anchor.suffix,
            },
        }
    }
}

impl From<LanguageDirectionSpan> for WireLanguageDirectionSpan {
    fn from(span: LanguageDirectionSpan) -> Self {
        Self {
            start: span.position.start,
            end: span.position.end,
            language: WireLanguageEvidence::from(span.language),
            direction: WireDirectionEvidence::from(span.direction),
        }
    }
}

impl From<WireLanguageDirectionSpan> for LanguageDirectionSpan {
    fn from(span: WireLanguageDirectionSpan) -> Self {
        Self {
            position: TextPositionSelector {
                start: span.start,
                end: span.end,
            },
            language: LanguageEvidence::from(span.language),
            direction: DirectionEvidence::from(span.direction),
        }
    }
}

impl From<LanguageEvidence> for WireLanguageEvidence {
    fn from(evidence: LanguageEvidence) -> Self {
        Self {
            declared: evidence.declared,
            declaration_is_valid: evidence.declaration_is_valid,
            effective: evidence.effective,
        }
    }
}

impl From<WireLanguageEvidence> for LanguageEvidence {
    fn from(evidence: WireLanguageEvidence) -> Self {
        Self {
            declared: evidence.declared,
            declaration_is_valid: evidence.declaration_is_valid,
            effective: evidence.effective,
        }
    }
}

impl From<DirectionEvidence> for WireDirectionEvidence {
    fn from(evidence: DirectionEvidence) -> Self {
        Self {
            declared: evidence.declared,
            declaration_is_valid: evidence.declaration_is_valid,
            effective: WireTextDirection::from(evidence.effective),
        }
    }
}

impl From<WireDirectionEvidence> for DirectionEvidence {
    fn from(evidence: WireDirectionEvidence) -> Self {
        Self {
            declared: evidence.declared,
            declaration_is_valid: evidence.declaration_is_valid,
            effective: TextDirection::from(evidence.effective),
        }
    }
}

impl From<TextDirection> for WireTextDirection {
    fn from(direction: TextDirection) -> Self {
        match direction {
            TextDirection::Ltr => Self::Ltr,
            TextDirection::Rtl => Self::Rtl,
            TextDirection::Auto => Self::Auto,
        }
    }
}

impl From<WireTextDirection> for TextDirection {
    fn from(direction: WireTextDirection) -> Self {
        match direction {
            WireTextDirection::Ltr => Self::Ltr,
            WireTextDirection::Rtl => Self::Rtl,
            WireTextDirection::Auto => Self::Auto,
        }
    }
}
