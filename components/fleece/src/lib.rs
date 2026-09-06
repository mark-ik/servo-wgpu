/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Fleece: render-free content extraction over [`LayoutDom`].
//!
//! "We don't just want to render the web, we want to analyze it too." This crate
//! turns a parsed document into the structured content a crawler or the eidetic
//! browsing corpus wants — links, title, headings, main text, metadata, reader
//! blocks, and page-carried structured data —
//! with **no cascade, layout, or paint**. Its single dependency is the
//! profile-neutral [`layout_dom_api`], so the dep graph itself is the witness that
//! extraction pulls none of the render stack (the render ladder's witness
//! discipline, applied to the orthogonal extraction axis).
//!
//! Extraction is **not a lower render rung**: it is a different *output* (data, not
//! pixels) that can draw from any rung's DOM. The cheap path runs over a no-JS
//! [`genet_static_dom::StaticDocument`] (static-parse extract); the same functions
//! run over a script-mutated DOM for the post-JS / SPA case (headless-scripted-DOM
//! extract), since both are just `LayoutDom`s.
//!
//! All output is **unresolved and rect-free**: an `href` is the raw attribute value
//! (the caller owns the document base URL and resolves it), and there is no geometry — this
//! is the counterpart to the layout-coupled `LinkHit` (`href` + rect), for code that
//! wants the link graph without laying the page out.

#![deny(unsafe_code)]

use std::collections::{HashMap, HashSet};

use layout_dom_api::{LayoutDom, LocalName, Namespace};
use sha2::{Digest, Sha256};
use unicode_bidi::{BidiClass, bidi_class};
use unicode_segmentation::UnicodeSegmentation;

mod annotation;
mod metadata;
mod structured;
mod table;
mod text_fragment;
#[cfg(feature = "wire")]
mod wire;

pub use annotation::{
    CanonicalTextSelectorProjection, FragmentSelector, RFC5147_CONFORMS_TO, anchor_for_range,
    resolve_anchor, valid_range,
};
pub use metadata::{DocumentLink, Metadata, OpenGraphGroup, extract_metadata};
pub use structured::{
    EmbeddedJsonLdBlock, JsonLdParseStatus, StructuredData, StructuredDataSource, StructuredValue,
    extract_json_ld_blocks, extract_structured_data,
};
pub use table::{
    Table, TableCell, TableHeader, TableModelError, TableRow, TableRowGroup, TableRowGroupKind,
    TableScope, extract_table,
};
pub use text_fragment::{TextFragment, text_fragment};
#[cfg(feature = "wire")]
pub use wire::{
    CanonicalTextRecordV1, JsonLdBlockRecordV1, WireDirectionEvidence, WireError,
    WireJsonLdParseStatus, WireLanguageDirectionSpan, WireLanguageEvidence, WireTextAnchor,
    WireTextDirection,
};

/// Stable schema identifier for one preserved embedded JSON-LD block.
pub const JSON_LD_BLOCK_RECORD_SCHEMA_V1: &str =
    "https://merely.dev/ns/fleece/json-ld-block-record/v1";

/// Syntax parser used to interpret the DOM text of preserved JSON-LD blocks.
pub const JSON_LD_PARSER_PROFILE_V1: &str = "FleeceJsonSyntaxV1";

/// The textual representation against which Fleece selectors are measured.
///
/// `FleeceDomTextV1` walks the supplied DOM in logical DOM order, excludes
/// `head`, `script`, `style`, `template`, and `noscript` subtrees, removes markup,
/// and collapses every Unicode whitespace run to one ASCII space. Each contributing
/// DOM text node is separated from the next by one ASCII space; element boundaries
/// add no other characters. Text is decoded DOM text, never source bytes or
/// visual/layout order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextNormalization {
    #[default]
    FleeceDomTextV1,
}

/// A half-open Unicode-code-point range in [`TextNormalization::FleeceDomTextV1`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextPositionSelector {
    pub start: u64,
    pub end: u64,
}

/// Quote evidence for one source segment. Prefix and suffix are adjacent context,
/// not a refinement relationship with the position selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextQuoteSelector {
    pub exact: String,
    pub prefix: String,
    pub suffix: String,
}

/// Sibling position and quote descriptions of one source segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextAnchor {
    pub position: TextPositionSelector,
    pub quote: TextQuoteSelector,
}

/// The closest declared HTML language and the language effective at a canonical
/// text range. Fleece preserves the raw declaration even when it is not a
/// plausible BCP 47 tag; it does not canonicalize language tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageEvidence {
    /// The raw nearest `lang` value, including an empty or invalid value.
    pub declared: Option<String>,
    /// Whether [`Self::declared`] is a syntactically plausible HTML language
    /// declaration. `None` means this range inherits its language.
    pub declaration_is_valid: Option<bool>,
    /// The nearest inherited raw language declaration. `None` means Fleece found
    /// no DOM declaration and leaves caller/transport fallback outside its scope.
    pub effective: Option<String>,
}

/// The HTML text direction represented by a `dir` declaration or default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextDirection {
    #[default]
    Ltr,
    Rtl,
    Auto,
}

/// The closest declared HTML direction and the direction effective at a
/// canonical text range. This is DOM evidence only, never a claim about
/// rendered bidi resolution or visual order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectionEvidence {
    /// The raw nearest `dir` value, including an invalid value.
    pub declared: Option<String>,
    /// Whether [`Self::declared`] is one of `ltr`, `rtl`, or `auto` after ASCII
    /// case-folding. `None` means this range inherits its direction.
    pub declaration_is_valid: Option<bool>,
    /// The inherited effective HTML direction.
    pub effective: TextDirection,
}

/// Declared and inherited HTML language/direction evidence for a contiguous
/// canonical-text range. Separator spaces inserted by Fleece normalization do
/// not pretend to originate from a DOM text node and therefore have no span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageDirectionSpan {
    pub position: TextPositionSelector,
    pub language: LanguageEvidence,
    pub direction: DirectionEvidence,
}

/// Stable schema identifier for the first preserved Fleece extraction record.
pub const EXTRACTION_RECORD_SCHEMA_V1: &str = "https://merely.dev/ns/fleece/extraction-record/v1";

/// Media type of Fleece's immutable canonical-text annotation resource.
pub const CANONICAL_TEXT_MEDIA_TYPE: &str = "text/plain; charset=utf-8";

/// The reader root-selection algorithm used for this extraction.
///
/// Later reader algorithms receive a new variant so a stored record never
/// relies on an implementation version to infer its selection rules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReaderSelectionProfile {
    #[default]
    FleeceReaderV1,
}

/// Identity and representation facts for the canonical Fleece text resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalTextResource {
    /// Content-derived, immutable IRI for exactly [`Self::media_type`] bytes.
    pub iri: String,
    pub media_type: &'static str,
}

impl CanonicalTextResource {
    pub(crate) fn for_text(text: &str) -> Self {
        let digest = Sha256::digest(text.as_bytes());
        Self {
            iri: format!("urn:sha256:{digest:x}"),
            media_type: CANONICAL_TEXT_MEDIA_TYPE,
        }
    }

    /// Project sibling Web Annotation selector values for one canonical-text
    /// anchor. The caller supplies the surrounding Annotation and target state.
    pub fn selector_projection(&self, anchor: &TextAnchor) -> CanonicalTextSelectorProjection {
        CanonicalTextSelectorProjection::from_anchor(self.iri.clone(), anchor)
    }
}

/// Deterministic facts needed to interpret a Fleece extraction after reopening.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionContract {
    pub schema_id: &'static str,
    pub canonical_text: CanonicalTextResource,
    pub normalization: TextNormalization,
    pub reader_profile: ReaderSelectionProfile,
    pub quote_context: usize,
    pub fleece_version: String,
}

impl ExtractionContract {
    fn new(text: &str, options: ExtractionOptions) -> Self {
        Self {
            schema_id: EXTRACTION_RECORD_SCHEMA_V1,
            canonical_text: CanonicalTextResource::for_text(text),
            normalization: TextNormalization::FleeceDomTextV1,
            reader_profile: ReaderSelectionProfile::FleeceReaderV1,
            quote_context: options.quote_context,
            fleece_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// Options for the selector-bearing extraction entry points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractionOptions {
    /// Maximum Unicode code points retained on either side of a quote. The
    /// resulting context is extended to whole grapheme clusters.
    pub quote_context: usize,
}

impl Default for ExtractionOptions {
    fn default() -> Self {
        Self { quote_context: 32 }
    }
}

/// One extracted hyperlink — the rect-free counterpart to a laid-out `LinkHit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The raw `href` attribute value, **unresolved**: extraction owns no URL
    /// context, so the caller resolves it against the document base URL.
    pub href: String,
    /// The anchor's descendant DOM text, whitespace-collapsed.
    pub text: String,
    /// The `rel` token list, if present (`nofollow`, `noopener`, …). A crawler
    /// honours `nofollow` when building its frontier; extraction just reports it.
    pub rel: Option<String>,
}

/// One extracted heading: its level (`1`–`6` for `<h1>`–`<h6>`) and collapsed text.
/// The document outline — structure for the corpus and a summarization signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// The heading level, `1`–`6`.
    pub level: u8,
    /// The heading's descendant DOM text, whitespace-collapsed.
    pub text: String,
}

/// A rich inline run in a reader article. URLs remain raw attributes; callers
/// resolve them against the source document's computed base URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Link { href: String, runs: Vec<Inline> },
    Emphasis { strong: bool, runs: Vec<Inline> },
    Code(String),
}

/// A reader block together with optional evidence in the canonical Fleece text.
///
/// A missing anchor means the reader block is synthetic or joins discontinuous
/// source text. Consumers must not treat it as an unquoted source selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchoredBlock {
    pub anchor: Option<TextAnchor>,
    pub block: Block,
}

/// Structured reader blocks, independent of layout and paint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        runs: Vec<Inline>,
    },
    Paragraph {
        runs: Vec<Inline>,
    },
    List {
        ordered: bool,
        items: Vec<Vec<AnchoredBlock>>,
    },
    Quote {
        blocks: Vec<AnchoredBlock>,
    },
    Code {
        language: Option<String>,
        text: String,
    },
    Table {
        table: Table,
    },
    Figure {
        src: String,
        alt: String,
        caption: Option<Vec<Inline>>,
    },
    Rule,
}

/// The root-selection evidence attached to a reader rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootSelector {
    Main,
    ScoredCandidate { tag: String, score: i32 },
}

/// Reproducibility information for a reader extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionLineage {
    pub fleece_version: String,
    /// The textual representation used by every selector in this article.
    pub normalization: TextNormalization,
    pub root_selector: RootSelector,
    pub block_count: usize,
}

/// The structured reader shape. Fleece returns it and retains no state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Article {
    pub title: Option<String>,
    pub byline: Option<String>,
    pub published: Option<String>,
    pub lang: Option<String>,
    pub site: Option<String>,
    pub canonical: Option<String>,
    pub lead_image: Option<String>,
    pub blocks: Vec<AnchoredBlock>,
    pub lineage: ExtractionLineage,
}

/// The two extraction views produced from one selected live document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedDocument {
    /// The deterministic extraction profile and canonical text identity. This is
    /// present even when reader selection found no article.
    pub contract: ExtractionContract,
    pub page: PageExtract,
    pub article: Option<Article>,
    /// Source language and direction evidence for contributing canonical-text
    /// ranges, in logical DOM order.
    pub language_direction_spans: Vec<LanguageDirectionSpan>,
}

impl ExtractedDocument {
    /// Project the required selector triple for this document's canonical text
    /// resource.
    pub fn selector_projection(&self, anchor: &TextAnchor) -> CanonicalTextSelectorProjection {
        self.contract.canonical_text.selector_projection(anchor)
    }

    /// Return all source language/direction spans that overlap an anchored
    /// canonical-text range. A mixed-language block can therefore retain more
    /// than one effective language without inventing a single block-wide value.
    pub fn language_direction_for_anchor(
        &self,
        anchor: &TextAnchor,
    ) -> Vec<&LanguageDirectionSpan> {
        self.language_direction_spans
            .iter()
            .filter(|span| {
                span.position.start < anchor.position.end
                    && anchor.position.start < span.position.end
            })
            .collect()
    }
}

/// A render-free extraction of a parsed document: the structured content a crawler
/// or the eidetic corpus wants, with no cascade / layout / paint. `Default` is
/// the empty extract.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageExtract {
    /// The document `<title>` text, whitespace-collapsed, if present and non-empty.
    pub title: Option<String>,
    /// The page's declared metadata (description / canonical / OpenGraph).
    pub metadata: Metadata,
    /// The `<h1>`–`<h6>` outline in document order.
    pub headings: Vec<Heading>,
    /// The page's full canonical **DOM text**, whitespace-collapsed (pinned
    /// non-content subtrees — `<script>` / `<style>` / `<head>` / … — excluded).
    /// This is the indexing/corpus text (page chrome included);
    /// for the article body alone, see [`main_text`](Self::main_text).
    pub text: String,
    /// The **reader-mode article body**: the main content block by a
    /// readability heuristic (semantic landmarks + paragraph density + class/id
    /// signals), with page chrome (nav / header / footer / aside) dropped. `None`
    /// when no contentful block stands out (a link list, an app shell). This is the
    /// per-page payload for a reader-mode crawl.
    pub main_text: Option<String>,
    /// Every `<a href>` in document order — the crawl frontier's source.
    pub links: Vec<Link>,
    /// Convenience projection of parsed JSON-LD objects and HTML Microdata.
    pub structured_data: Vec<StructuredData>,
    /// Every HTML JSON-LD script retained before the convenience projection.
    pub json_ld_blocks: Vec<EmbeddedJsonLdBlock>,
}

/// Extract the structured content of `dom` without rendering it. The one-call entry
/// for the eidetic sink; the field functions below are the à-la-carte pieces.
pub fn extract<D: LayoutDom>(dom: &D) -> PageExtract {
    extract_with_options(dom, ExtractionOptions::default())
}

/// Extract the flat index shape with caller-selected quote context.
pub fn extract_with_options<D: LayoutDom>(dom: &D, options: ExtractionOptions) -> PageExtract {
    extract_document_with_options(dom, options).page
}

/// Extract the flat index shape and structured reader shape together.
pub fn extract_document<D: LayoutDom>(dom: &D) -> ExtractedDocument {
    extract_document_with_options(dom, ExtractionOptions::default())
}

/// Extract both shapes with caller-selected quote context.
pub fn extract_document_with_options<D: LayoutDom>(
    dom: &D,
    options: ExtractionOptions,
) -> ExtractedDocument {
    let text_index = FleeceTextIndex::build(dom);
    let selected = select_content(dom);
    let main_text = selected.as_ref().and_then(|selected| {
        let text = chrome_free_text(dom, &selected.roots);
        (!text.is_empty()).then_some(text)
    });
    let (json_ld_blocks, structured_data) = structured::extract_page_carried_data(dom);
    let page = PageExtract {
        title: extract_title(dom),
        metadata: extract_metadata(dom),
        headings: extract_headings(dom),
        text: text_index.text.clone(),
        main_text,
        links: extract_links(dom),
        structured_data,
        json_ld_blocks,
    };
    let article = selected
        .and_then(|selected| extract_article_with_page(dom, &page, selected, &text_index, options));
    let language_direction_spans = text_index.language_direction_spans(dom);
    let contract = ExtractionContract::new(&page.text, options);
    ExtractedDocument {
        contract,
        page,
        article,
        language_direction_spans,
    }
}

/// Extract only the structured reader shape.
pub fn extract_article<D: LayoutDom>(dom: &D) -> Option<Article> {
    extract_article_with_options(dom, ExtractionOptions::default())
}

/// Extract only the structured reader shape with caller-selected quote context.
pub fn extract_article_with_options<D: LayoutDom>(
    dom: &D,
    options: ExtractionOptions,
) -> Option<Article> {
    extract_document_with_options(dom, options).article
}

/// Whether the document carries a script element.
///
/// Hosts use this only as a fallback signal: a static parse that already
/// yields an [`Article`] stays on the cheap path, while an empty scripted shell
/// may be retried over its post-JS DOM.
pub fn carries_script<D: LayoutDom>(dom: &D) -> bool {
    find_first(dom, dom.document(), "script").is_some()
}

/// Every `<a href>` in the document, in document (pre-order) order. The **rect-free
/// anchor enumerator**: the link extractor for a crawl frontier, with no layout.
/// Anchors without an `href` (named anchors / placeholders) are skipped — they are
/// not navigable targets.
pub fn extract_links<D: LayoutDom>(dom: &D) -> Vec<Link> {
    let mut out = Vec::new();
    walk_links(dom, dom.document(), &mut out);
    out
}

fn walk_links<D: LayoutDom>(dom: &D, id: D::NodeId, out: &mut Vec<Link>) {
    if local_name(dom, id) == Some("a") {
        if let Some(href) = attr(dom, id, "href") {
            out.push(Link {
                href,
                text: text_of(dom, id),
                rel: attr(dom, id, "rel"),
            });
        }
    }
    for child in dom.dom_children(id) {
        walk_links(dom, child, out);
    }
}

/// The document `<title>` text (whitespace-collapsed), or `None` if absent/empty.
pub fn extract_title<D: LayoutDom>(dom: &D) -> Option<String> {
    let id = find_first(dom, dom.document(), "title")?;
    let text = text_of(dom, id);
    (!text.is_empty()).then_some(text)
}

/// The first element with local name `name` in pre-order, or `None`.
fn find_first<D: LayoutDom>(dom: &D, id: D::NodeId, name: &str) -> Option<D::NodeId> {
    if local_name(dom, id) == Some(name) {
        return Some(id);
    }
    for child in dom.dom_children(id) {
        if let Some(found) = find_first(dom, child, name) {
            return Some(found);
        }
    }
    None
}

/// The `<h1>`–`<h6>` outline in document (pre-order) order, each with its level and
/// collapsed text. Empty headings are skipped (no text to outline).
pub fn extract_headings<D: LayoutDom>(dom: &D) -> Vec<Heading> {
    let mut out = Vec::new();
    walk_headings(dom, dom.document(), &mut out);
    out
}

fn walk_headings<D: LayoutDom>(dom: &D, id: D::NodeId, out: &mut Vec<Heading>) {
    if let Some(level) = local_name(dom, id).and_then(heading_level) {
        let text = text_of(dom, id);
        if !text.is_empty() {
            out.push(Heading { level, text });
        }
    }
    for child in dom.dom_children(id) {
        walk_headings(dom, child, out);
    }
}

/// `1`–`6` for `h1`–`h6`, else `None`.
fn heading_level(name: &str) -> Option<u8> {
    match name.as_bytes() {
        [b'h', d @ b'1'..=b'6'] => Some(d - b'0'),
        _ => None,
    }
}

/// The page's full canonical **DOM text**, whitespace-collapsed: every text node
/// except those under pinned non-content elements (`<script>` / `<style>` / `<template>` /
/// `<noscript>` / the document `<head>`). The indexing/corpus text — deliberately
/// *not* a main-content heuristic (which would drop nav/footer chrome); that
/// readability pass is a later slice that can build on this.
pub fn extract_text<D: LayoutDom>(dom: &D) -> String {
    FleeceTextIndex::build(dom).text
}

/// Names of subtrees excluded from the canonical DOM-text profile.
fn is_non_rendered(name: &str) -> bool {
    matches!(name, "script" | "style" | "template" | "noscript" | "head")
}

#[derive(Debug, Clone, Copy)]
struct TextRange {
    start: u64,
    end: u64,
}

/// The one normalized page stream and the source-node ranges that contributed to
/// it. Composite extraction builds this once so page text and reader anchors share
/// one coordinate system.
struct FleeceTextIndex<Id> {
    text: String,
    ranges: HashMap<Id, TextRange>,
    source_ranges: Vec<(Id, TextRange)>,
    grapheme_boundaries: HashSet<u64>,
    pending_space: bool,
}

impl<Id: std::hash::Hash + Eq + Copy> FleeceTextIndex<Id> {
    fn append_text(&mut self, source: &str) -> TextRange {
        let mut start = self.text.chars().count() as u64;
        let mut wrote_content = false;
        for character in source.chars() {
            if character.is_whitespace() {
                self.pending_space = true;
            } else {
                if self.pending_space && !self.text.is_empty() && !self.text.ends_with(' ') {
                    self.text.push(' ');
                    if !wrote_content {
                        start = self.text.chars().count() as u64;
                    }
                }
                self.text.push(character);
                wrote_content = true;
                self.pending_space = false;
            }
        }
        TextRange {
            start,
            end: self.text.chars().count() as u64,
        }
    }

    fn range_for<D: LayoutDom<NodeId = Id>>(&self, dom: &D, id: Id) -> Option<TextRange> {
        if local_name(dom, id).is_some_and(is_non_rendered) {
            return None;
        }
        if let Some(range) = self
            .ranges
            .get(&id)
            .copied()
            .filter(|range| range.start < range.end)
        {
            return Some(range);
        }
        let mut result: Option<TextRange> = None;
        for child in dom.dom_children(id) {
            let Some(child_range) = self.range_for(dom, child) else {
                continue;
            };
            result = Some(match result {
                Some(range) => TextRange {
                    start: range.start.min(child_range.start),
                    end: range.end.max(child_range.end),
                },
                None => child_range,
            });
        }
        result
    }
}

impl<Id: std::hash::Hash + Eq + Copy> FleeceTextIndex<Id> {
    fn build<D: LayoutDom<NodeId = Id>>(dom: &D) -> Self {
        let mut index = Self {
            text: String::new(),
            ranges: HashMap::new(),
            source_ranges: Vec::new(),
            grapheme_boundaries: HashSet::new(),
            pending_space: false,
        };
        index.collect(dom, dom.document());
        let mut boundary = 0_u64;
        index.grapheme_boundaries.insert(boundary);
        for grapheme in UnicodeSegmentation::graphemes(index.text.as_str(), true) {
            boundary += grapheme.chars().count() as u64;
            index.grapheme_boundaries.insert(boundary);
        }
        index
    }

    fn collect<D: LayoutDom<NodeId = Id>>(&mut self, dom: &D, id: Id) {
        if local_name(dom, id).is_some_and(is_non_rendered) {
            return;
        }
        if let Some(text) = dom.text(id) {
            let range = self.append_text(text);
            self.ranges.insert(id, range);
            if range.start < range.end {
                self.source_ranges.push((id, range));
            }
            // Preserve Fleece 0.1's explicit separator between adjacent DOM text
            // nodes, even where HTML source omitted whitespace between elements.
            self.pending_space = true;
        }
        for child in dom.dom_children(id) {
            self.collect(dom, child);
        }
    }

    fn language_direction_spans<D: LayoutDom<NodeId = Id>>(
        &self,
        dom: &D,
    ) -> Vec<LanguageDirectionSpan> {
        self.source_ranges
            .iter()
            .map(|(id, range)| LanguageDirectionSpan {
                position: TextPositionSelector {
                    start: range.start,
                    end: range.end,
                },
                language: language_evidence(dom, *id),
                direction: direction_evidence(dom, *id),
            })
            .collect()
    }
}

fn has_reader_excluded_descendant<D: LayoutDom>(dom: &D, id: D::NodeId) -> bool {
    local_name(dom, id).is_some_and(is_chrome_or_non_rendered)
        || dom
            .dom_children(id)
            .any(|child| has_reader_excluded_descendant(dom, child))
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

fn anchor_for_node<D: LayoutDom>(
    dom: &D,
    id: D::NodeId,
    text_index: &FleeceTextIndex<D::NodeId>,
    options: ExtractionOptions,
) -> Option<TextAnchor> {
    if has_reader_excluded_descendant(dom, id) {
        return None;
    }
    let range = text_index.range_for(dom, id)?;
    if !text_index.grapheme_boundaries.contains(&range.start)
        || !text_index.grapheme_boundaries.contains(&range.end)
    {
        return None;
    }
    let exact = text_slice(&text_index.text, range.start, range.end);
    (!exact.is_empty()).then(|| TextAnchor {
        position: TextPositionSelector {
            start: range.start,
            end: range.end,
        },
        quote: TextQuoteSelector {
            prefix: prefix_context(&text_index.text, range.start, options.quote_context),
            exact,
            suffix: suffix_context(&text_index.text, range.end, options.quote_context),
        },
    })
}

fn anchored_block<D: LayoutDom>(
    dom: &D,
    id: D::NodeId,
    block: Block,
    text_index: &FleeceTextIndex<D::NodeId>,
    options: ExtractionOptions,
) -> AnchoredBlock {
    AnchoredBlock {
        anchor: anchor_for_node(dom, id, text_index, options),
        block,
    }
}

// ---- reader-mode / main-content extraction ------------------------------------
//
// A compact readability heuristic (technique borrowed from readability.js): a
// semantic `<main>` wins outright; otherwise score block containers by paragraph
// density and class/id signal and take the best. Then emit that block's text with
// chrome (nav / header / footer / aside) and non-rendered subtrees dropped. The
// per-page payload for a reader-mode crawl.

/// Positive class/id signals: an element whose `class`/`id` contains one of these is
/// likely the article body (readability.js's positive lexicon, trimmed).
const POSITIVE_HINTS: &[&str] = &[
    "article", "body", "content", "entry", "main", "page", "post", "text", "blog", "story",
    "column", "prose",
];

/// Negative class/id signals: chrome, boilerplate, furniture. An element matching one
/// is penalized as unlikely to be the article body.
const NEGATIVE_HINTS: &[&str] = &[
    "nav",
    "menu",
    "header",
    "footer",
    "sidebar",
    "comment",
    "ads",
    "banner",
    "sponsor",
    "social",
    "share",
    "related",
    "promo",
    "masthead",
    "widget",
    "byline",
    "breadcrumb",
];

/// The page's **reader-mode article body**: locate the main content block and return
/// its chrome-free text, whitespace-collapsed. `None` when nothing contentful stands
/// out (an app shell, a pure link list). The per-page payload for a reader-mode crawl.
pub fn extract_main_text<D: LayoutDom>(dom: &D) -> Option<String> {
    let selected = select_content(dom)?;
    let text = chrome_free_text(dom, &selected.roots);
    (!text.is_empty()).then_some(text)
}

struct SelectedContent<Id> {
    roots: Vec<Id>,
    selector: RootSelector,
}

/// Select the article root and absorb adjacent article-grade siblings. Several
/// `<article>` elements identify an index page rather than one readable article.
fn select_content<D: LayoutDom>(dom: &D) -> Option<SelectedContent<D::NodeId>> {
    if count_elements(dom, dom.document(), "article") > 1 {
        return None;
    }
    if let Some(main) = find_first(dom, dom.document(), "main")
        && is_article_grade(dom, main)
    {
        return Some(SelectedContent {
            roots: absorb_siblings(dom, main),
            selector: RootSelector::Main,
        });
    }
    let mut best: Option<(i32, D::NodeId)> = None;
    score_candidates(dom, dom.document(), &mut best);
    let (score, root) = best.filter(|(score, id)| *score > 0 && is_article_grade(dom, *id))?;
    Some(SelectedContent {
        roots: absorb_siblings(dom, root),
        selector: RootSelector::ScoredCandidate {
            tag: local_name(dom, root).unwrap_or("unknown").to_string(),
            score,
        },
    })
}

fn count_elements<D: LayoutDom>(dom: &D, id: D::NodeId, name: &str) -> usize {
    usize::from(local_name(dom, id) == Some(name))
        + dom
            .dom_children(id)
            .map(|child| count_elements(dom, child, name))
            .sum::<usize>()
}

fn is_article_grade<D: LayoutDom>(dom: &D, id: D::NodeId) -> bool {
    let text_len = chrome_free_text(dom, &[id]).len();
    text_len >= 40 && link_density_percent(dom, id) < 50
}

fn absorb_siblings<D: LayoutDom>(dom: &D, root: D::NodeId) -> Vec<D::NodeId> {
    let Some(parent) = dom.parent(root) else {
        return vec![root];
    };
    let siblings = dom.dom_children(parent).collect::<Vec<_>>();
    let Some(index) = siblings.iter().position(|candidate| *candidate == root) else {
        return vec![root];
    };
    let absorbable = |id| {
        is_candidate_block(dom, id)
            && paragraph_text_len(dom, id) >= 60
            && link_density_percent(dom, id) < 40
            && classid_signal(dom, id) >= 0
    };
    let mut start = index;
    while start > 0 && absorbable(siblings[start - 1]) {
        start -= 1;
    }
    let mut end = index + 1;
    while end < siblings.len() && absorbable(siblings[end]) {
        end += 1;
    }
    siblings[start..end].to_vec()
}

/// Score every candidate block in the tree, tracking the maximum.
fn score_candidates<D: LayoutDom>(dom: &D, id: D::NodeId, best: &mut Option<(i32, D::NodeId)>) {
    if is_candidate_block(dom, id) {
        let score = score_block(dom, id);
        let better = match *best {
            Some((s, _)) => score > s,
            None => true,
        };
        if better {
            *best = Some((score, id));
        }
    }
    for child in dom.dom_children(id) {
        score_candidates(dom, child, best);
    }
}

/// The block-level containers that can be the article root.
fn is_candidate_block<D: LayoutDom>(dom: &D, id: D::NodeId) -> bool {
    matches!(
        local_name(dom, id),
        Some("div" | "section" | "article" | "td")
    )
}

/// A block's readability score: a tag bonus, the class/id signal, and paragraph
/// density (the dominant term — an article body is mostly paragraph text).
fn score_block<D: LayoutDom>(dom: &D, id: D::NodeId) -> i32 {
    let mut score = classid_signal(dom, id);
    score += match local_name(dom, id) {
        Some("article") => 25,
        Some("section") => 8,
        Some("td") => 3,
        _ => 0,
    };
    // Paragraph density in ~50-char units, capped so one giant block doesn't wholly
    // swamp the class/tag signal.
    score += (paragraph_text_len(dom, id) / 50).min(50) as i32;
    // Link-heavy candidates are navigation/index furniture, even when their labels
    // happen to be long enough to resemble prose.
    score -= (link_density_percent(dom, id) / 2) as i32;
    score
}

fn link_density_percent<D: LayoutDom>(dom: &D, id: D::NodeId) -> usize {
    let total = text_of(dom, id).len();
    if total == 0 {
        return 100;
    }
    let mut linked = 0;
    linked_text_len(dom, id, false, &mut linked);
    linked.saturating_mul(100) / total
}

fn linked_text_len<D: LayoutDom>(dom: &D, id: D::NodeId, in_link: bool, total: &mut usize) {
    let in_link = in_link || local_name(dom, id) == Some("a");
    if in_link && let Some(text) = dom.text(id) {
        *total += text.len();
    }
    for child in dom.dom_children(id) {
        linked_text_len(dom, child, in_link, total);
    }
}

/// Sum of descendant `<p>` text lengths under `id` (tiny paragraphs ignored — UI
/// labels, not prose).
fn paragraph_text_len<D: LayoutDom>(dom: &D, id: D::NodeId) -> usize {
    let mut total = 0;
    walk_paragraph_len(dom, id, &mut total);
    total
}

fn walk_paragraph_len<D: LayoutDom>(dom: &D, id: D::NodeId, total: &mut usize) {
    if local_name(dom, id) == Some("p") {
        let len = text_of(dom, id).len();
        if len >= 25 {
            *total += len;
        }
    }
    for child in dom.dom_children(id) {
        walk_paragraph_len(dom, child, total);
    }
}

/// The class/id signal: `+25` if any positive hint and `-25` if any negative hint
/// appears in the element's `class` or `id` (substring, lowercased), as readability does.
fn classid_signal<D: LayoutDom>(dom: &D, id: D::NodeId) -> i32 {
    let haystack = format!(
        "{} {}",
        attr(dom, id, "class").unwrap_or_default(),
        attr(dom, id, "id").unwrap_or_default(),
    )
    .to_ascii_lowercase();
    let mut score = 0;
    if POSITIVE_HINTS.iter().any(|h| haystack.contains(h)) {
        score += 25;
    }
    if NEGATIVE_HINTS.iter().any(|h| haystack.contains(h)) {
        score -= 25;
    }
    score
}

/// Text under `root` with chrome (nav / header / footer / aside) and non-rendered
/// (script / style / …) subtrees dropped — the reader-mode body text.
fn chrome_free_text<D: LayoutDom>(dom: &D, roots: &[D::NodeId]) -> String {
    let mut out = String::new();
    for root in roots {
        collect_main_text(dom, *root, &mut out);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_main_text<D: LayoutDom>(dom: &D, id: D::NodeId, out: &mut String) {
    if local_name(dom, id).is_some_and(is_chrome_or_non_rendered) {
        return; // skip chrome / non-rendered subtrees
    }
    if let Some(t) = dom.text(id) {
        out.push_str(t);
        out.push(' ');
    }
    for child in dom.dom_children(id) {
        collect_main_text(dom, child, out);
    }
}

/// Subtrees excluded from reader-mode body text: non-rendered content plus page
/// chrome (the landmarks that are not the article).
fn is_chrome_or_non_rendered(name: &str) -> bool {
    is_non_rendered(name) || matches!(name, "nav" | "header" | "footer" | "aside")
}

// ---- structured reader extraction --------------------------------------------

fn extract_article_with_page<D: LayoutDom>(
    dom: &D,
    page: &PageExtract,
    selected: SelectedContent<D::NodeId>,
    text_index: &FleeceTextIndex<D::NodeId>,
    options: ExtractionOptions,
) -> Option<Article> {
    let mut blocks = Vec::new();
    for root in &selected.roots {
        collect_blocks(dom, *root, &mut blocks, true, text_index, options);
    }
    if blocks.is_empty() {
        return None;
    }
    let title = open_graph_value(&page.metadata, "title")
        .map(str::to_string)
        .or_else(|| first_heading_text(dom, &selected.roots))
        .or_else(|| page.title.clone());
    let byline = first_meta_content(dom, &["author", "byl"])
        .or_else(|| first_class_text(dom, dom.document(), &["byline", "author"]));
    let published = first_meta_content(
        dom,
        &["article:published_time", "date", "datepublished", "pubdate"],
    )
    .or_else(|| first_time_value(dom, dom.document()));
    let lang = find_first(dom, dom.document(), "html").and_then(|html| attr(dom, html, "lang"));
    let site = open_graph_value(&page.metadata, "site_name").map(str::to_string);
    let canonical = page.metadata.canonical.clone();
    let lead_image = open_graph_value(&page.metadata, "image")
        .map(str::to_string)
        .or_else(|| first_attr_in_roots(dom, &selected.roots, "img", "src"));
    let block_count = blocks.iter().map(count_block_tree).sum();
    Some(Article {
        title,
        byline,
        published,
        lang,
        site,
        canonical,
        lead_image,
        blocks,
        lineage: ExtractionLineage {
            fleece_version: env!("CARGO_PKG_VERSION").to_string(),
            normalization: TextNormalization::FleeceDomTextV1,
            root_selector: selected.selector,
            block_count,
        },
    })
}

fn open_graph_value<'a>(metadata: &'a Metadata, key: &str) -> Option<&'a str> {
    metadata
        .open_graph
        .iter()
        .find_map(|(candidate, value)| (candidate == key).then_some(value.as_str()))
}

fn first_heading_text<D: LayoutDom>(dom: &D, roots: &[D::NodeId]) -> Option<String> {
    for root in roots {
        if let Some(heading) = find_first_heading(dom, *root) {
            let text = text_of(dom, heading);
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

fn find_first_heading<D: LayoutDom>(dom: &D, id: D::NodeId) -> Option<D::NodeId> {
    if local_name(dom, id).and_then(heading_level).is_some() {
        return Some(id);
    }
    dom.dom_children(id)
        .find_map(|child| find_first_heading(dom, child))
}

fn first_meta_content<D: LayoutDom>(dom: &D, names: &[&str]) -> Option<String> {
    fn walk<D: LayoutDom>(dom: &D, id: D::NodeId, names: &[&str]) -> Option<String> {
        if local_name(dom, id) == Some("meta") {
            let key = attr(dom, id, "property").or_else(|| attr(dom, id, "name"));
            if key
                .as_deref()
                .is_some_and(|key| names.iter().any(|name| key.eq_ignore_ascii_case(name)))
            {
                return attr(dom, id, "content").filter(|value| !value.is_empty());
            }
        }
        dom.dom_children(id)
            .find_map(|child| walk(dom, child, names))
    }
    walk(dom, dom.document(), names)
}

fn first_class_text<D: LayoutDom>(dom: &D, id: D::NodeId, hints: &[&str]) -> Option<String> {
    let class_id = format!(
        "{} {}",
        attr(dom, id, "class").unwrap_or_default(),
        attr(dom, id, "id").unwrap_or_default()
    )
    .to_ascii_lowercase();
    if hints.iter().any(|hint| class_id.contains(hint)) {
        let text = text_of(dom, id);
        if !text.is_empty() {
            return Some(text);
        }
    }
    dom.dom_children(id)
        .find_map(|child| first_class_text(dom, child, hints))
}

fn first_time_value<D: LayoutDom>(dom: &D, id: D::NodeId) -> Option<String> {
    if local_name(dom, id) == Some("time") {
        return attr(dom, id, "datetime").or_else(|| {
            let text = text_of(dom, id);
            (!text.is_empty()).then_some(text)
        });
    }
    dom.dom_children(id)
        .find_map(|child| first_time_value(dom, child))
}

fn first_attr_in_roots<D: LayoutDom>(
    dom: &D,
    roots: &[D::NodeId],
    tag: &str,
    attribute: &str,
) -> Option<String> {
    roots
        .iter()
        .find_map(|root| first_attr(dom, *root, tag, attribute))
}

fn first_attr<D: LayoutDom>(dom: &D, id: D::NodeId, tag: &str, attribute: &str) -> Option<String> {
    if local_name(dom, id) == Some(tag)
        && let Some(value) = attr(dom, id, attribute).filter(|value| !value.is_empty())
    {
        return Some(value);
    }
    dom.dom_children(id)
        .find_map(|child| first_attr(dom, child, tag, attribute))
}

fn count_block_tree(block: &AnchoredBlock) -> usize {
    1 + match &block.block {
        Block::List { items, .. } => items.iter().flatten().map(count_block_tree).sum::<usize>(),
        Block::Quote { blocks } => blocks.iter().map(count_block_tree).sum(),
        _ => 0,
    }
}

fn collect_blocks<D: LayoutDom>(
    dom: &D,
    id: D::NodeId,
    out: &mut Vec<AnchoredBlock>,
    include_self: bool,
    text_index: &FleeceTextIndex<D::NodeId>,
    options: ExtractionOptions,
) {
    let name = local_name(dom, id);
    if name.is_some_and(is_chrome_or_non_rendered) {
        return;
    }
    if include_self {
        if let Some(level) = name.and_then(heading_level) {
            push_inline_block(
                out,
                |runs| Block::Heading { level, runs },
                inline_runs(dom, id),
                dom,
                id,
                text_index,
                options,
            );
            return;
        }
        match name {
            Some("p") => {
                push_inline_block(
                    out,
                    |runs| Block::Paragraph { runs },
                    inline_runs(dom, id),
                    dom,
                    id,
                    text_index,
                    options,
                );
                return;
            },
            Some("ul" | "ol") => {
                let ordered = name == Some("ol");
                let items = list_items(dom, id, text_index, options);
                if !items.is_empty() {
                    out.push(anchored_block(
                        dom,
                        id,
                        Block::List { ordered, items },
                        text_index,
                        options,
                    ));
                }
                return;
            },
            Some("blockquote") => {
                let mut blocks = Vec::new();
                for child in dom.dom_children(id) {
                    collect_blocks(dom, child, &mut blocks, true, text_index, options);
                }
                if blocks.is_empty() {
                    push_inline_block(
                        &mut blocks,
                        |runs| Block::Paragraph { runs },
                        inline_runs(dom, id),
                        dom,
                        id,
                        text_index,
                        options,
                    );
                }
                if !blocks.is_empty() {
                    out.push(anchored_block(
                        dom,
                        id,
                        Block::Quote { blocks },
                        text_index,
                        options,
                    ));
                }
                return;
            },
            Some("pre") => {
                let language = code_language(dom, id);
                let text = text_of(dom, id);
                if !text.is_empty() {
                    out.push(anchored_block(
                        dom,
                        id,
                        Block::Code { language, text },
                        text_index,
                        options,
                    ));
                }
                return;
            },
            Some("table") => {
                let table = extract_table(dom, id);
                if !table.rows.is_empty() {
                    out.push(anchored_block(
                        dom,
                        id,
                        Block::Table { table },
                        text_index,
                        options,
                    ));
                }
                return;
            },
            Some("figure") => {
                if let Some(figure) = figure_block(dom, id) {
                    let anchor_id = find_first(dom, id, "figcaption").unwrap_or(id);
                    let anchor = matches!(
                        &figure,
                        Block::Figure {
                            caption: Some(_),
                            ..
                        }
                    )
                    .then(|| anchor_for_node(dom, anchor_id, text_index, options))
                    .flatten();
                    out.push(AnchoredBlock {
                        anchor,
                        block: figure,
                    });
                }
                return;
            },
            Some("img") => {
                if let Some(src) = attr(dom, id, "src").filter(|src| !src.is_empty()) {
                    out.push(AnchoredBlock {
                        anchor: None,
                        block: Block::Figure {
                            src,
                            alt: attr(dom, id, "alt").unwrap_or_default(),
                            caption: None,
                        },
                    });
                }
                return;
            },
            Some("hr") => {
                out.push(AnchoredBlock {
                    anchor: None,
                    block: Block::Rule,
                });
                return;
            },
            _ => {},
        }
    }
    for child in dom.dom_children(id) {
        collect_blocks(dom, child, out, true, text_index, options);
    }
}

fn push_inline_block<D: LayoutDom>(
    out: &mut Vec<AnchoredBlock>,
    make: impl FnOnce(Vec<Inline>) -> Block,
    runs: Vec<Inline>,
    dom: &D,
    id: D::NodeId,
    text_index: &FleeceTextIndex<D::NodeId>,
    options: ExtractionOptions,
) {
    if !inline_plain_text(&runs).trim().is_empty() {
        out.push(anchored_block(dom, id, make(runs), text_index, options));
    }
}

fn list_items<D: LayoutDom>(
    dom: &D,
    list: D::NodeId,
    text_index: &FleeceTextIndex<D::NodeId>,
    options: ExtractionOptions,
) -> Vec<Vec<AnchoredBlock>> {
    dom.dom_children(list)
        .filter(|child| local_name(dom, *child) == Some("li"))
        .filter_map(|item| {
            let mut blocks = Vec::new();
            let shallow = inline_runs_shallow(dom, item);
            if !inline_plain_text(&shallow).trim().is_empty() {
                let has_structural_child = dom
                    .dom_children(item)
                    .any(|child| local_name(dom, child).is_some_and(is_structural_block));
                blocks.push(AnchoredBlock {
                    // The shallow paragraph skips structural children. Once one
                    // exists, the contributing inline ranges are not represented
                    // by the full `li` range and may be discontinuous around it.
                    anchor: (!has_structural_child)
                        .then(|| anchor_for_node(dom, item, text_index, options))
                        .flatten(),
                    block: Block::Paragraph { runs: shallow },
                });
            }
            for child in dom.dom_children(item) {
                if local_name(dom, child).is_some_and(is_structural_block) {
                    collect_blocks(dom, child, &mut blocks, true, text_index, options);
                }
            }
            (!blocks.is_empty()).then_some(blocks)
        })
        .collect()
}

fn is_structural_block(name: &str) -> bool {
    matches!(
        name,
        "h1" | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "p"
            | "ul"
            | "ol"
            | "blockquote"
            | "pre"
            | "table"
            | "figure"
            | "hr"
    )
}

fn inline_runs<D: LayoutDom>(dom: &D, id: D::NodeId) -> Vec<Inline> {
    let mut runs = Vec::new();
    for child in dom.dom_children(id) {
        collect_inline(dom, child, &mut runs, false);
    }
    trim_inline_edges(&mut runs);
    runs
}

fn inline_runs_shallow<D: LayoutDom>(dom: &D, id: D::NodeId) -> Vec<Inline> {
    let mut runs = Vec::new();
    for child in dom.dom_children(id) {
        if local_name(dom, child).is_some_and(is_structural_block) {
            continue;
        }
        collect_inline(dom, child, &mut runs, true);
    }
    trim_inline_edges(&mut runs);
    runs
}

fn collect_inline<D: LayoutDom>(dom: &D, id: D::NodeId, out: &mut Vec<Inline>, shallow: bool) {
    if let Some(text) = dom.text(id) {
        let collapsed = collapse_inline_text(text);
        if !collapsed.is_empty() {
            push_text_run(out, collapsed);
        }
        return;
    }
    let name = local_name(dom, id);
    if name.is_some_and(is_chrome_or_non_rendered)
        || shallow && name.is_some_and(is_structural_block)
    {
        return;
    }
    let nested = |dom: &D, id, shallow| {
        let mut runs = Vec::new();
        for child in dom.dom_children(id) {
            collect_inline(dom, child, &mut runs, shallow);
        }
        trim_inline_edges(&mut runs);
        runs
    };
    match name {
        Some("a") => {
            let runs = nested(dom, id, shallow);
            if let Some(href) = attr(dom, id, "href")
                && !runs.is_empty()
            {
                out.push(Inline::Link { href, runs });
            } else {
                out.extend(runs);
            }
        },
        Some("strong" | "b" | "em" | "i") => {
            let runs = nested(dom, id, shallow);
            if !runs.is_empty() {
                out.push(Inline::Emphasis {
                    strong: matches!(name, Some("strong" | "b")),
                    runs,
                });
            }
        },
        Some("code") => {
            let text = text_of(dom, id);
            if !text.is_empty() {
                out.push(Inline::Code(text));
            }
        },
        Some("br") => push_text_run(out, "\n".to_string()),
        _ => out.extend(nested(dom, id, shallow)),
    }
}

fn collapse_inline_text(text: &str) -> String {
    let leading = text.chars().next().is_some_and(char::is_whitespace);
    let trailing = text.chars().next_back().is_some_and(char::is_whitespace);
    let core = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if core.is_empty() {
        return if leading {
            " ".to_string()
        } else {
            String::new()
        };
    }
    format!(
        "{}{}{}",
        if leading { " " } else { "" },
        core,
        if trailing { " " } else { "" }
    )
}

fn push_text_run(out: &mut Vec<Inline>, text: String) {
    if let Some(Inline::Text(previous)) = out.last_mut() {
        previous.push_str(&text);
    } else {
        out.push(Inline::Text(text));
    }
}

fn trim_inline_edges(runs: &mut Vec<Inline>) {
    if let Some(Inline::Text(first)) = runs.first_mut() {
        *first = first.trim_start().to_string();
    }
    if let Some(Inline::Text(last)) = runs.last_mut() {
        *last = last.trim_end().to_string();
    }
    runs.retain(|run| !matches!(run, Inline::Text(text) if text.is_empty()));
}

fn inline_plain_text(runs: &[Inline]) -> String {
    let mut out = String::new();
    for run in runs {
        match run {
            Inline::Text(text) | Inline::Code(text) => out.push_str(text),
            Inline::Link { runs, .. } | Inline::Emphasis { runs, .. } => {
                out.push_str(&inline_plain_text(runs));
            },
        }
    }
    out
}

fn code_language<D: LayoutDom>(dom: &D, pre: D::NodeId) -> Option<String> {
    let code = find_first(dom, pre, "code")?;
    let class = attr(dom, code, "class")?;
    class.split_whitespace().find_map(|token| {
        token
            .strip_prefix("language-")
            .or_else(|| token.strip_prefix("lang-"))
            .map(str::to_string)
    })
}

fn figure_block<D: LayoutDom>(dom: &D, figure: D::NodeId) -> Option<Block> {
    let image = find_first(dom, figure, "img")?;
    let src = attr(dom, image, "src").filter(|src| !src.is_empty())?;
    let caption = find_first(dom, figure, "figcaption")
        .map(|node| inline_runs(dom, node))
        .filter(|runs| !runs.is_empty());
    Some(Block::Figure {
        src,
        alt: attr(dom, image, "alt").unwrap_or_default(),
        caption,
    })
}

// ---- source language and direction evidence ---------------------------------

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

fn language_evidence<D: LayoutDom>(dom: &D, id: D::NodeId) -> LanguageEvidence {
    let declared = nearest_language_declaration(dom, id);
    let declaration_is_valid = declared.as_deref().map(is_language_declaration);
    let effective = declared.clone();
    LanguageEvidence {
        declared,
        declaration_is_valid,
        effective,
    }
}

/// Find the closest language declaration by the HTML language inheritance
/// lookup: XML `xml:lang` wins on each element, followed by no-namespace `lang`
/// on HTML or SVG elements. Caller/transport fallback remains outside Fleece.
fn nearest_language_declaration<D: LayoutDom>(dom: &D, id: D::NodeId) -> Option<String> {
    let mut current = Some(id);
    while let Some(node) = current {
        if let Some(value) = attr_in_namespace(dom, node, XML_NAMESPACE, "lang") {
            return Some(value);
        }
        if is_html_or_svg_element(dom, node)
            && let Some(value) = attr(dom, node, "lang")
        {
            return Some(value);
        }
        current = dom.parent(node);
    }
    None
}

/// A conservative syntax check that separates raw invalid evidence from a
/// plausible BCP 47 language tag without claiming complete BCP 47 validation.
fn is_language_declaration(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    let mut subtags = value.split('-');
    let Some(primary) = subtags.next() else {
        return false;
    };
    let primary_valid = if matches!(primary, "x" | "X" | "i" | "I") {
        true
    } else {
        (2..=8).contains(&primary.len()) && primary.bytes().all(|byte| byte.is_ascii_alphabetic())
    };
    primary_valid
        && subtags.all(|subtag| {
            (1..=8).contains(&subtag.len())
                && subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        && !value.ends_with('-')
}

fn direction_evidence<D: LayoutDom>(dom: &D, id: D::NodeId) -> DirectionEvidence {
    let declared = nearest_direction_declaration(dom, id);
    let declaration_is_valid = declared
        .as_deref()
        .map(|value| parse_direction(value).is_some());
    DirectionEvidence {
        declared,
        declaration_is_valid,
        effective: effective_direction(dom, id),
    }
}

fn nearest_direction_declaration<D: LayoutDom>(dom: &D, id: D::NodeId) -> Option<String> {
    let mut current = Some(id);
    while let Some(node) = current {
        if is_html_element(dom, node)
            && let Some(value) = attr(dom, node, "dir")
        {
            return Some(value);
        }
        current = dom.parent(node);
    }
    None
}

fn effective_direction<D: LayoutDom>(dom: &D, id: D::NodeId) -> TextDirection {
    let mut current = Some(id);
    while let Some(node) = current {
        if is_html_element(dom, node) {
            if let Some(value) = attr(dom, node, "dir")
                && let Some(direction) = parse_direction(&value)
            {
                return match direction {
                    TextDirection::Auto => auto_direction(dom, node),
                    direction => direction,
                };
            }
            if local_name(dom, node) == Some("bdi") {
                return auto_direction(dom, node);
            }
        }
        current = dom.parent(node);
    }
    TextDirection::Ltr
}

fn parse_direction(value: &str) -> Option<TextDirection> {
    if value.eq_ignore_ascii_case("ltr") {
        Some(TextDirection::Ltr)
    } else if value.eq_ignore_ascii_case("rtl") {
        Some(TextDirection::Rtl)
    } else if value.eq_ignore_ascii_case("auto") {
        Some(TextDirection::Auto)
    } else {
        None
    }
}

/// Resolve `dir=auto` and an unlabelled `<bdi>` from their first strong
/// contained character. The walk excludes nested `bdi`, scripting/style and
/// textarea content, and descendants with a defined `dir`, following the
/// content boundary relevant to HTML auto directionality. The no-strong-
/// character fallback is `ltr`; form control and shadow-tree directionality are
/// host semantics and are not represented by a `LayoutDom` text walk.
fn auto_direction<D: LayoutDom>(dom: &D, id: D::NodeId) -> TextDirection {
    let mut text = String::new();
    collect_auto_direction_text(dom, id, id, &mut text);
    for character in text.chars() {
        match bidi_class(character) {
            BidiClass::L => return TextDirection::Ltr,
            BidiClass::R | BidiClass::AL => return TextDirection::Rtl,
            _ => {},
        }
    }
    TextDirection::Ltr
}

fn collect_auto_direction_text<D: LayoutDom>(
    dom: &D,
    id: D::NodeId,
    root: D::NodeId,
    out: &mut String,
) {
    if id != root
        && is_html_element(dom, id)
        && (matches!(
            local_name(dom, id),
            Some("bdi" | "script" | "style" | "textarea")
        ) || attr(dom, id, "dir")
            .as_deref()
            .is_some_and(|value| parse_direction(value).is_some()))
    {
        return;
    }
    if let Some(text) = dom.text(id) {
        out.push_str(text);
    }
    for child in dom.dom_children(id) {
        collect_auto_direction_text(dom, child, root, out);
    }
}

// ---- small DOM helpers (rect-free, allocation-light) --------------------------

/// `id`'s element local name as `&str`, or `None` for non-elements.
fn local_name<D: LayoutDom>(dom: &D, id: D::NodeId) -> Option<&str> {
    dom.element_name(id).map(|q| q.local.as_ref())
}

/// A null-namespace attribute (`href`, `rel`, `id`, … — the HTML common case).
fn attr<D: LayoutDom>(dom: &D, id: D::NodeId, name: &str) -> Option<String> {
    dom.attribute(id, &Namespace::from(""), &LocalName::from(name))
        .map(str::to_string)
}

fn attr_in_namespace<D: LayoutDom>(
    dom: &D,
    id: D::NodeId,
    namespace: &str,
    name: &str,
) -> Option<String> {
    dom.attribute(id, &Namespace::from(namespace), &LocalName::from(name))
        .map(str::to_string)
}

fn is_html_element<D: LayoutDom>(dom: &D, id: D::NodeId) -> bool {
    dom.element_name(id)
        .is_some_and(|name| name.ns.as_ref() == HTML_NAMESPACE)
}

fn is_html_or_svg_element<D: LayoutDom>(dom: &D, id: D::NodeId) -> bool {
    dom.element_name(id)
        .is_some_and(|name| matches!(name.ns.as_ref(), HTML_NAMESPACE | SVG_NAMESPACE))
}

/// The whitespace-collapsed concatenation of all descendant text under `id` — an
/// element's descendant DOM text for extraction (script/style content is parsed as text
/// children, but anchors and titles do not contain them, so no filtering is needed
/// at this slice; a main-text extractor will skip `<script>`/`<style>`).
fn text_of<D: LayoutDom>(dom: &D, id: D::NodeId) -> String {
    let mut raw = String::new();
    collect_text(dom, id, &mut raw);
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_text<D: LayoutDom>(dom: &D, id: D::NodeId, out: &mut String) {
    if let Some(t) = dom.text(id) {
        out.push_str(t);
        out.push(' '); // a separator so adjacent inline text nodes don't fuse
    }
    for child in dom.dom_children(id) {
        collect_text(dom, child, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genet_static_dom::StaticDocument;

    #[test]
    fn extracts_anchors_with_text_and_rel() {
        let doc = StaticDocument::parse(
            "<html><body>\
                <a href=\"/one\">First</a>\
                <p>not a link</p>\
                <a href=\"https://example.com/two\" rel=\"nofollow\">Second <b>bold</b></a>\
                <a name=\"anchor\">no href, skipped</a>\
             </body></html>",
        );
        let links = extract_links(&doc);
        assert_eq!(
            links.len(),
            2,
            "two href anchors; the named anchor is skipped: {links:?}"
        );
        assert_eq!(
            links[0],
            Link {
                href: "/one".into(),
                text: "First".into(),
                rel: None
            }
        );
        assert_eq!(
            links[1],
            Link {
                href: "https://example.com/two".into(),
                text: "Second bold".into(), // descendant text concatenated + collapsed
                rel: Some("nofollow".into()),
            },
        );
    }

    #[test]
    fn anchor_href_is_unresolved_raw_attribute() {
        // Extraction owns no URL context: the relative href comes back verbatim, for
        // the caller to resolve against the document base URL.
        let doc = StaticDocument::parse("<body><a href=\"../sub/page.html\">x</a></body>");
        assert_eq!(extract_links(&doc)[0].href, "../sub/page.html");
    }

    #[test]
    fn extracts_the_title_collapsed() {
        let doc = StaticDocument::parse(
            "<html><head><title>  Hello   World  </title></head><body></body></html>",
        );
        assert_eq!(extract_title(&doc).as_deref(), Some("Hello World"));
    }

    #[test]
    fn no_title_is_none() {
        let doc = StaticDocument::parse("<body><p>no title here</p></body>");
        assert_eq!(extract_title(&doc), None);
    }

    #[test]
    fn extract_bundles_title_and_links() {
        let doc = StaticDocument::parse(
            "<html><head><title>T</title></head><body><a href=\"/a\">A</a></body></html>",
        );
        let page = extract(&doc);
        assert_eq!(page.title.as_deref(), Some("T"));
        assert_eq!(page.links.len(), 1);
        assert_eq!(page.links[0].href, "/a");
    }

    #[test]
    fn empty_document_extracts_nothing() {
        let doc = StaticDocument::parse("");
        assert_eq!(extract(&doc), PageExtract::default());
    }

    #[test]
    fn extracts_the_heading_outline() {
        let doc = StaticDocument::parse(
            "<body>\
                <h1>Title</h1>\
                <h2>Section <em>one</em></h2>\
                <p>body</p>\
                <h3></h3>\
                <h2>Section two</h2>\
             </body>",
        );
        assert_eq!(
            extract_headings(&doc),
            vec![
                Heading {
                    level: 1,
                    text: "Title".into()
                },
                Heading {
                    level: 2,
                    text: "Section one".into()
                },
                // the empty <h3> is skipped
                Heading {
                    level: 2,
                    text: "Section two".into()
                },
            ],
        );
    }

    #[test]
    fn extracts_description_canonical_and_open_graph() {
        let doc = StaticDocument::parse(
            "<html><head>\
                <meta name=\"description\" content=\"A page about things.\">\
                <link rel=\"canonical\" href=\"https://example.com/page\">\
                <meta property=\"og:title\" content=\"Things\">\
                <meta property=\"og:image\" content=\"https://example.com/og.png\">\
             </head><body></body></html>",
        );
        let md = extract_metadata(&doc);
        assert_eq!(md.description.as_deref(), Some("A page about things."));
        assert_eq!(md.canonical.as_deref(), Some("https://example.com/page"));
        assert_eq!(
            md.open_graph,
            vec![
                ("title".to_string(), "Things".to_string()),
                (
                    "image".to_string(),
                    "https://example.com/og.png".to_string()
                ),
            ],
        );
    }

    #[test]
    fn canonical_rel_is_a_token_list() {
        // `rel` is a space-separated token list; `canonical` need not be the only token.
        let doc =
            StaticDocument::parse("<head><link rel=\"alternate canonical\" href=\"/c\"></head>");
        assert_eq!(extract_metadata(&doc).canonical.as_deref(), Some("/c"));
    }

    #[test]
    fn missing_metadata_is_all_none() {
        let doc = StaticDocument::parse("<body><p>no meta</p></body>");
        assert_eq!(extract_metadata(&doc), Metadata::default());
    }

    #[test]
    fn visible_text_excludes_script_and_style() {
        let doc = StaticDocument::parse(
            "<html><head><title>T</title><style>p{color:red}</style></head>\
             <body><h1>Heading</h1><p>Para one.</p>\
             <script>var x = 'not visible';</script>\
             <p>Para two.</p></body></html>",
        );
        // <head> (title + style) and the inline <script> are excluded; body text is
        // concatenated and whitespace-collapsed.
        assert_eq!(extract_text(&doc), "Heading Para one. Para two.");
    }

    #[test]
    fn full_extract_carries_text() {
        let doc = StaticDocument::parse(
            "<html><head><title>T</title></head><body><p>Hello world.</p></body></html>",
        );
        assert_eq!(extract(&doc).text, "Hello world.");
    }

    #[test]
    fn anchored_blocks_share_the_page_code_point_stream() {
        let doc = StaticDocument::parse(
            "<body><main><h1>Title 🙂</h1><p>First <em>e&#x301;</em> paragraph.</p>\
             <blockquote><p>Nested quote.</p></blockquote></main></body>",
        );
        let extracted = extract_document_with_options(&doc, ExtractionOptions { quote_context: 4 });
        assert_eq!(
            extracted.page.text,
            "Title 🙂 First é paragraph. Nested quote."
        );
        let article = extracted.article.expect("article");
        let heading = article.blocks.first().expect("heading anchor");
        let anchor = heading.anchor.as_ref().expect("source anchor");
        assert_eq!(anchor.quote.exact, "Title 🙂");
        assert_eq!(
            text_slice(
                &extracted.page.text,
                anchor.position.start,
                anchor.position.end
            ),
            anchor.quote.exact
        );
        assert_eq!(anchor.position.end - anchor.position.start, 7);
        let quote = article.blocks.last().expect("quote block");
        assert!(
            quote.anchor.is_some(),
            "contiguous parent quote is anchored"
        );
        let Block::Quote { blocks } = &quote.block else {
            panic!("quote block");
        };
        assert!(blocks[0].anchor.is_some(), "nested child is anchored");
    }

    #[test]
    fn quote_context_preserves_grapheme_boundaries() {
        let doc = StaticDocument::parse(
            "<body><main>e&#x301;<p>target text is long enough for the reader to retain as article prose.</p></main></body>",
        );
        let article = extract_article_with_options(&doc, ExtractionOptions { quote_context: 3 })
            .expect("article");
        let paragraph = article.blocks.last().expect("paragraph");
        let anchor = paragraph.anchor.as_ref().expect("paragraph anchor");
        assert_eq!(
            anchor.quote.exact,
            "target text is long enough for the reader to retain as article prose."
        );
        assert_eq!(anchor.quote.prefix, "é ");

        let article = extract_article_with_options(&doc, ExtractionOptions { quote_context: 2 })
            .expect("article");
        let anchor = article.blocks.last().unwrap().anchor.as_ref().unwrap();
        assert_eq!(anchor.quote.prefix, " ");
    }

    #[test]
    fn main_text_prefers_the_main_landmark_and_drops_chrome() {
        let doc = StaticDocument::parse(
            "<body>\
                <nav><a href='/'>Home</a> Menu Junk Links</nav>\
                <header>Site Title Boilerplate Banner</header>\
                <main>\
                    <h1>Article Heading</h1>\
                    <p>This is the first paragraph of the real article body, long enough to count.</p>\
                    <p>And a second paragraph continuing the genuine article content here.</p>\
                    <aside>Inline aside promo junk to drop</aside>\
                </main>\
                <footer>Copyright Footer Junk</footer>\
             </body>",
        );
        let main = extract_main_text(&doc).expect("an article body");
        assert!(
            main.contains("first paragraph of the real article body"),
            "{main}"
        );
        assert!(main.contains("second paragraph"), "{main}");
        // Chrome outside the landmark, and an aside *inside* it, are all dropped.
        assert!(!main.contains("Menu Junk"), "nav dropped: {main}");
        assert!(!main.contains("Footer Junk"), "footer dropped: {main}");
        assert!(
            !main.contains("Boilerplate Banner"),
            "header dropped: {main}"
        );
        assert!(
            !main.contains("aside promo junk"),
            "inline aside dropped: {main}"
        );
    }

    #[test]
    fn main_text_scores_content_over_sidebar() {
        // No <main>: the heuristic must pick the article div over the sidebar by
        // paragraph density + class signal, and drop an inline footer within it.
        let doc = StaticDocument::parse(
            "<body>\
                <div class='sidebar'><p>Ads and promo links and sponsor junk over here.</p></div>\
                <div class='article-content'>\
                    <p>The genuine article body paragraph one, with substantial readable prose.</p>\
                    <p>Paragraph two of the genuine article, with more substantial readable content.</p>\
                    <footer>inline footer junk to drop</footer>\
                </div>\
             </body>",
        );
        let main = extract_main_text(&doc).expect("an article body");
        assert!(
            main.contains("genuine article body paragraph one"),
            "{main}"
        );
        assert!(
            main.contains("Paragraph two of the genuine article"),
            "{main}"
        );
        assert!(
            !main.contains("Ads and promo"),
            "sidebar lost to scoring: {main}"
        );
        assert!(
            !main.contains("footer junk"),
            "inline footer dropped: {main}"
        );
    }

    #[test]
    fn main_text_is_none_for_a_link_list() {
        // A nav-only page (an app shell / index) has no article body.
        let doc = StaticDocument::parse(
            "<body><nav><a href='/a'>A</a><a href='/b'>B</a><a href='/c'>C</a></nav></body>",
        );
        assert_eq!(extract_main_text(&doc), None);
    }

    #[test]
    fn full_extract_carries_main_text() {
        let doc = StaticDocument::parse(
            "<body><main><p>The article body paragraph with enough prose to register.</p></main></body>",
        );
        assert!(
            extract(&doc)
                .main_text
                .as_deref()
                .is_some_and(|m| m.contains("article body paragraph")),
        );
    }

    #[test]
    fn article_keeps_structure_inline_runs_metadata_and_lineage() {
        let doc = StaticDocument::parse(
            r#"<html lang="en-GB"><head>
                <title>Fallback title</title>
                <link rel="canonical" href="/canonical">
                <meta property="og:title" content="Reader title">
                <meta property="og:site_name" content="The Example">
                <meta property="og:image" content="/lead.jpg">
                <meta name="author" content="Ada Example">
                <meta property="article:published_time" content="2026-08-22">
              </head><body><main><h1>On extraction</h1>
                <p>Keep <em>quiet emphasis</em>, <strong>strong emphasis</strong>,
                <a href="/source">linked words</a>, and <code>inline()</code> intact.</p>
                <ul><li>first item<ul><li>nested item</li></ul></li><li>second item</li></ul>
                <blockquote><p>A quoted paragraph with enough substance to remain readable.</p></blockquote>
                <pre><code class="language-rust">fn main() {}</code></pre>
                <table><thead><tr><th>Name</th><th>Value</th></tr></thead>
                <tbody><tr><td>fleece</td><td>reader</td></tr></tbody></table>
                <figure><img src="/figure.png" alt="diagram"><figcaption>The figure</figcaption></figure>
                <hr><p>A final paragraph makes this unmistakably article-grade prose.</p>
              </main></body></html>"#,
        );
        let article = extract_article(&doc).expect("structured article");
        assert_eq!(article.title.as_deref(), Some("Reader title"));
        assert_eq!(article.byline.as_deref(), Some("Ada Example"));
        assert_eq!(article.published.as_deref(), Some("2026-08-22"));
        assert_eq!(article.lang.as_deref(), Some("en-GB"));
        assert_eq!(article.site.as_deref(), Some("The Example"));
        assert_eq!(article.canonical.as_deref(), Some("/canonical"));
        assert_eq!(article.lead_image.as_deref(), Some("/lead.jpg"));
        assert!(matches!(article.lineage.root_selector, RootSelector::Main));
        assert_eq!(article.lineage.fleece_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(article.lineage.block_count, 14);
        let paragraph = article
            .blocks
            .iter()
            .find_map(|block| match block {
                AnchoredBlock {
                    block: Block::Paragraph { runs },
                    ..
                } if inline_plain_text(runs).contains("linked words") => Some(runs),
                _ => None,
            })
            .expect("rich paragraph");
        assert!(
            paragraph
                .iter()
                .any(|run| matches!(run, Inline::Link { href, .. } if href == "/source"))
        );
        assert!(
            paragraph
                .iter()
                .any(|run| matches!(run, Inline::Emphasis { strong: false, .. }))
        );
        assert!(
            paragraph
                .iter()
                .any(|run| matches!(run, Inline::Emphasis { strong: true, .. }))
        );
        assert!(
            paragraph
                .iter()
                .any(|run| matches!(run, Inline::Code(code) if code == "inline()"))
        );
        assert!(
            article.blocks.iter().any(
                |block| matches!(&block.block, Block::Table { table } if table.rows[0].header)
            )
        );
        assert!(article.blocks.iter().any(|block| matches!(&block.block, Block::Figure { src, caption: Some(_), .. } if src == "/figure.png")));
    }

    #[test]
    fn sibling_absorption_joins_article_grade_neighbors() {
        let doc = StaticDocument::parse(
            "<body><div class='article-content'><p>The opening section carries substantial prose for the reader.</p></div>\
             <section><p>The adjacent continuation also carries substantial article-grade prose.</p></section>\
             <div class='related'><p>Related promotional furniture should not join the article.</p></div></body>",
        );
        let text = extract_main_text(&doc).expect("article");
        assert!(text.contains("opening section"));
        assert!(text.contains("adjacent continuation"));
        assert!(!text.contains("promotional furniture"));
    }

    #[test]
    fn multiple_articles_identify_an_index_page() {
        let doc = StaticDocument::parse(
            "<body><article><p>The first summary contains enough prose to score.</p></article>\
             <article><p>The second summary contains enough prose to score.</p></article></body>",
        );
        assert_eq!(extract_article(&doc), None);
        assert_eq!(extract_main_text(&doc), None);
    }

    #[test]
    fn harvests_json_ld_and_microdata_as_typed_values() {
        let doc = StaticDocument::parse(
            r#"<head><script type="application/ld+json">
              {"@context":"https://schema.org","@type":"Recipe","name":"Tea","servings":2}
              </script></head><body>
              <section itemscope itemtype="https://schema.org/Event">
                <meta itemprop="name" content="Reading group">
                <time itemprop="startDate" datetime="2026-09-01">September 1</time>
              </section></body>"#,
        );
        let data = extract_structured_data(&doc);
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].types, ["Recipe"]);
        assert_eq!(data[0].id, None);
        assert_eq!(data[0].source, StructuredDataSource::JsonLd);
        assert_eq!(
            data[0].value.get("name").and_then(StructuredValue::as_str),
            Some("Tea")
        );
        assert_eq!(data[1].types, ["https://schema.org/Event"]);
        assert_eq!(data[1].id, None);
        assert_eq!(data[1].source, StructuredDataSource::Microdata);
        assert_eq!(
            data[1]
                .value
                .get("startDate")
                .and_then(StructuredValue::as_str),
            Some("2026-09-01")
        );
        assert!(
            extract(&StaticDocument::parse(
                "<main><p>ordinary page prose long enough to read cleanly.</p></main>"
            ))
            .structured_data
            .is_empty()
        );
    }

    #[test]
    fn labelled_reader_corpus_reports_precision_and_recall() {
        struct Fixture {
            name: &'static str,
            html: &'static str,
            expected: Option<&'static str>,
        }
        let fixtures = [
            Fixture {
                name: "news",
                html: "<html><head><title>Metro Desk</title></head><body><header><nav><a href='/local'>Local</a><a href='/weather'>Weather</a></nav></header><div class='page'><aside>Most read and market data</aside><article class='story-body'><div class='byline'>By A Reporter</div><h1>City council approves the river plan</h1><p>Councillors approved the river restoration plan after a long public hearing.</p><div class='share-tools'>Share this report</div><section class='comments'>Reader comments</section></article></div><footer>Subscriptions and legal notices</footer></body></html>",
                expected: Some(
                    "City council approves the river plan Councillors approved the river restoration plan after a long public hearing.",
                ),
            },
            Fixture {
                name: "blog",
                html: "<html><body><header><a href='/'>Notebook home</a></header><div class='columns'><article class='post-content'><h1>A small garden in August</h1><p>The tomatoes finally reached the kitchen window after weeks of patient tying.</p><div class='related-posts'><a href='/spring'>Earlier garden notes</a></div></article><aside>Archive categories and newsletter signup</aside></div><footer>Copyright notice</footer></body></html>",
                expected: Some(
                    "A small garden in August The tomatoes finally reached the kitchen window after weeks of patient tying.",
                ),
            },
            Fixture {
                name: "documentation",
                html: "<html><body><header>Product documentation</header><div class='docs-shell'><nav><a href='/install'>Install</a><a href='/api'>API</a></nav><section class='documentation-content'><h1>Configure the reader lane</h1><p>Choose the reader engine from the tile menu to render held HTML as an article.</p></section><aside>On this page and version picker</aside></div><footer>Edit this page</footer></body></html>",
                expected: Some(
                    "Configure the reader lane Choose the reader engine from the tile menu to render held HTML as an article.",
                ),
            },
            Fixture {
                name: "recipe",
                html: "<html><body><header><nav>Breakfast Lunch Dinner</nav></header><main class='recipe-content'><h1>Weeknight lentil soup</h1><div class='promo'>Save this recipe and join the club</div><p>Simmer the lentils with onion and stock until tender, then finish with lemon.</p><section class='related-recipes'>More soups and stews</section></main><footer>Kitchen index</footer></body></html>",
                expected: Some(
                    "Weeknight lentil soup Simmer the lentils with onion and stock until tender, then finish with lemon.",
                ),
            },
            Fixture {
                name: "forum-thread",
                html: "<html><body><header><nav>Forums Members Search</nav></header><main id='thread-content'><h1>Repairing a noisy tape machine</h1><div class='post'><p>Cleaning the capstan solved the flutter, while a new belt fixed the remaining drift.</p></div><div class='comment controls'>Reply Quote Report</div></main><aside>Recent discussions and active users</aside></body></html>",
                expected: Some(
                    "Repairing a noisy tape machine Cleaning the capstan solved the flutter, while a new belt fixed the remaining drift.",
                ),
            },
            Fixture {
                name: "essay",
                html: "<html><body><header>Independent essays</header><div class='layout'><div class='article-content'><h1>Notes on durable interfaces</h1><p>An interface becomes durable when a second consumer proves the abstraction under different pressure.</p></div><div class='promo-card'>Support the magazine</div><aside>Contents and author archive</aside></div><footer>Colophon</footer></body></html>",
                expected: Some(
                    "Notes on durable interfaces An interface becomes durable when a second consumer proves the abstraction under different pressure.",
                ),
            },
            Fixture {
                name: "newspaper-column",
                html: "<html><body><header><nav>News Arts Books</nav></header><div id='column-layout'><section class='story-body'><div class='byline'>The Saturday columnist</div><h1>The library at closing time</h1><p>At closing time the reading room gathers its scattered silence into one place.</p><div class='share'>Email Print Share</div></section><aside>Recommended columns</aside></div><footer>Subscriber services</footer></body></html>",
                expected: Some(
                    "The library at closing time At closing time the reading room gathers its scattered silence into one place.",
                ),
            },
            Fixture {
                name: "api-guide",
                html: "<html><body><header>API guide</header><nav><a href='/types'>Types</a><a href='/traits'>Traits</a></nav><article class='documentation'><h1>Resolve links at the boundary</h1><p>Fleece returns raw attributes so each caller resolves addresses against its own source URL.</p><div class='related'>Related functions</div></article><footer>Source and license</footer></body></html>",
                expected: Some(
                    "Resolve links at the boundary Fleece returns raw attributes so each caller resolves addresses against its own source URL.",
                ),
            },
            Fixture {
                name: "food-column",
                html: "<html><body><header><nav>Recipes Techniques Equipment</nav></header><main><h1>Why toast needs patience</h1><div class='author byline'>Test kitchen</div><p>Moderate heat dries the surface before browning and leaves the center pleasantly tender.</p><aside>Recommended toaster settings</aside><div class='comments'>Join the discussion</div></main><footer>Magazine links</footer></body></html>",
                expected: Some(
                    "Why toast needs patience Moderate heat dries the surface before browning and leaves the center pleasantly tender.",
                ),
            },
            Fixture {
                name: "travel",
                html: "<html><body><header>Travel journal</header><div class='page-grid'><section class='post-content'><h1>Walking the old canal</h1><p>The towpath follows stone locks, quiet warehouses, and gardens built above the water.</p><div class='social-share'>Share itinerary</div></section><aside>Map, lodging, and nearby trips</aside></div><footer>Travel archive</footer></body></html>",
                expected: Some(
                    "Walking the old canal The towpath follows stone locks, quiet warehouses, and gardens built above the water.",
                ),
            },
            Fixture {
                name: "science",
                html: "<html><body><header><nav>Research Field notes Collections</nav></header><article id='research-story'><h1>A closer look at moth wings</h1><p>Microscopic scales scatter light in patterns that change as the viewing angle shifts.</p><section class='related-research'>More microscopy reports</section></article><aside>Institutional announcements</aside><footer>Data policy</footer></body></html>",
                expected: Some(
                    "A closer look at moth wings Microscopic scales scatter light in patterns that change as the viewing angle shifts.",
                ),
            },
            Fixture {
                name: "opinion",
                html: "<html><body><header>Opinion</header><main class='opinion-body'><div class='byline'>Guest essay</div><h1>Public benches are small infrastructure</h1><p>A place to pause changes who can cross a neighborhood and how long they can remain.</p><div class='share-bar'>Share Comment</div></main><aside>Other opinions</aside><footer>Editorial standards</footer></body></html>",
                expected: Some(
                    "Public benches are small infrastructure A place to pause changes who can cross a neighborhood and how long they can remain.",
                ),
            },
            Fixture {
                name: "changelog",
                html: "<html><body><header><nav>Guide Reference Releases</nav></header><div class='release-layout'><section class='content release-notes'><h1>Version two point four</h1><p>This release adds reader lineage, nested lists, and table headers to exported articles.</p></section><aside>Previous versions and downloads</aside></div><footer>Security policy</footer></body></html>",
                expected: Some(
                    "Version two point four This release adds reader lineage, nested lists, and table headers to exported articles.",
                ),
            },
            Fixture {
                name: "how-to",
                html: "<html><body><header>Practical testing</header><nav>Basics Tools Examples</nav><article class='how-to article-body'><h1>Label a reader fixture</h1><p>Keep the expected article words separate from navigation labels and promotional furniture.</p><div class='promo'>Download the complete handbook</div></article><aside>Table of contents</aside><footer>Feedback</footer></body></html>",
                expected: Some(
                    "Label a reader fixture Keep the expected article words separate from navigation labels and promotional furniture.",
                ),
            },
            Fixture {
                name: "profile",
                html: "<html><body><header><nav>People Places Work</nav></header><div id='feature-layout'><div id='story' class='article-content'><h1>The instrument repairer</h1><p>For thirty years she has restored concertinas whose reeds arrived bent, dusty, or silent.</p><div class='related'>More maker profiles</div></div><aside>Portrait gallery</aside></div><footer>About this series</footer></body></html>",
                expected: Some(
                    "The instrument repairer For thirty years she has restored concertinas whose reeds arrived bent, dusty, or silent.",
                ),
            },
            Fixture {
                name: "interview",
                html: "<html><body><header>Conversations</header><main><h1>A conversation about local software</h1><div class='byline'>Recorded interview</div><p>The useful question is where authority lives when the network disappears.</p><section class='comments'>Transcript corrections and reader notes</section></main><aside>Recent interviews</aside><footer>Podcast feeds</footer></body></html>",
                expected: Some(
                    "A conversation about local software The useful question is where authority lives when the network disappears.",
                ),
            },
            Fixture {
                name: "review",
                html: "<html><body><header><nav>Music Film Books</nav></header><div class='review-page'><section class='article-content review-body'><h1>A patient record of winter songs</h1><p>The album favors room sound, spare arrangements, and performances allowed to breathe.</p><div class='share'>Share review</div></section><aside>Score, credits, and related records</aside></div><footer>Review policy</footer></body></html>",
                expected: Some(
                    "A patient record of winter songs The album favors room sound, spare arrangements, and performances allowed to breathe.",
                ),
            },
            Fixture {
                name: "dispatch",
                html: "<html><body><header>Field dispatches</header><div class='site-grid'><article class='dispatch story-body'><h1>Dispatch from the shoreline</h1><p>Morning fog hid the opposite bank while fishing boats moved slowly through the channel.</p><div class='author byline'>Filed from the coast</div><div class='related'>Earlier dispatches</div></article><aside>Weather and tide table</aside></div><footer>Archive</footer></body></html>",
                expected: Some(
                    "Dispatch from the shoreline Morning fog hid the opposite bank while fishing boats moved slowly through the channel.",
                ),
            },
            Fixture {
                name: "spa-shell",
                html: "<html><head><title>Client application</title></head><body><header><nav><a href='/'>Home</a></nav></header><main id='app'><div class='loading'>Loading</div></main><script>renderArticleLater()</script><footer>Application shell</footer></body></html>",
                expected: None,
            },
            Fixture {
                name: "link-index",
                html: "<html><body><header>Topic directory</header><nav>Browse A to Z</nav><div class='content link-index'><h1>All destinations</h1><a href='/1'>A long directory label for the first destination</a><a href='/2'>A long directory label for the second destination</a><a href='/3'>A long directory label for the third destination</a><a href='/4'>A long directory label for the fourth destination</a></div><footer>Directory help</footer></body></html>",
                expected: None,
            },
        ];
        assert_eq!(fixtures.len(), 20);

        let mut true_positive = 0usize;
        let mut false_positive = 0usize;
        let mut false_negative = 0usize;
        for fixture in fixtures {
            let actual = extract_main_text(&StaticDocument::parse(fixture.html));
            match fixture.expected {
                None => assert_eq!(
                    actual, None,
                    "{} should not read as an article",
                    fixture.name
                ),
                Some(expected) => {
                    let actual =
                        actual.unwrap_or_else(|| panic!("{} lost its article", fixture.name));
                    let expected_words = word_set(expected);
                    let actual_words = word_set(&actual);
                    true_positive += actual_words.intersection(&expected_words).count();
                    false_positive += actual_words.difference(&expected_words).count();
                    false_negative += expected_words.difference(&actual_words).count();
                },
            }
        }
        let precision = true_positive as f64 / (true_positive + false_positive) as f64;
        let recall = true_positive as f64 / (true_positive + false_negative) as f64;
        eprintln!(
            "fleece labelled corpus: 20 pages; precision={precision:.3}; recall={recall:.3}; tp={true_positive}; fp={false_positive}; fn={false_negative}"
        );
        assert!(precision.is_finite() && recall.is_finite());
    }

    fn word_set(text: &str) -> std::collections::BTreeSet<String> {
        text.split(|character: char| !character.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_ascii_lowercase)
            .collect()
    }
}
