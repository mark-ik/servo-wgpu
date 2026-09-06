# fleece

**fleece** is live-document extraction for the Genet engine.

To fleece a document is to shear the readable substance off a rendered page:
article text, metadata, tables, and structure come away clean, and the
document keeps standing. This is the successor name for `genet-extract`'s
lane: render-free content extraction over the profile-neutral LayoutDom.
Analyze, don't paint.

The boundaries are the point: not import (mere's `import` migrates *stored*
browser data; fleece works a *live* document), not crawl (the frontier
decides what to visit; fleece decides what a visited page said), and not
illume (the lexer names spans in source text; fleece harvests rendered
documents).

It exposes the flat `PageExtract` index shape and the structured `Article`
reader shape over any profile-neutral `LayoutDom`. Its runtime dependencies are
`layout_dom_api`, `sha2`, `unicode-bidi`, and `unicode-segmentation`; parsing,
layout, paint, storage, and network policy stay with callers. The optional
`wire` feature adds Serde and JSON for the preserved canonical-text and embedded
JSON-LD records.

## Text anchors

Fleece exposes `TextAnchor` evidence on every `AnchoredBlock` that
maps to one contiguous source segment. Its sibling `TextPositionSelector` and
`TextQuoteSelector` values refer to the versioned `FleeceDomTextV1` stream:
logical DOM-order, decoded DOM text with markup removed and whitespace
collapsed, with one ASCII separator between contributing DOM text nodes and no
other element-boundary characters. Positions are half-open Unicode code-point
offsets. Reader blocks that are synthetic or combine discontinuous source text
have no anchor.

`ExtractionOptions::quote_context` controls the maximum surrounding context in
code points; Fleece preserves extended grapheme boundaries while truncating it.
Fleece names neither source URLs nor Web Annotations. Consumers supply source
identity and serialize the sibling selectors if they need an annotation. These
are W3C-shaped selector values, not a complete Web Annotation representation or
conformance claim.

Fleece 0.5 gives every `ExtractedDocument`, including a page with no selected
article, an `ExtractionContract`. The contract names the canonical-text hash
IRI, media type, normalization, reader profile, quote-context size, schema, and
producing Fleece version. `anchor_for_range` and `resolve_anchor` mint and
resolve arbitrary canonical-text selections while retaining repeated matches.
The typed selector projection adds the RFC 5147 character fragment required by
the declared Web Annotation `text/plain` profile.

With `wire`, `CanonicalTextRecordV1` preserves the canonical text, extraction
contract, ordered language/direction evidence, and an ordered sequence of
present or absent anchors. Decode verifies the text hash, source spans, and every
retained range, quote, and context before returning a record. This narrow record
is the stable source-selection boundary; complete page, reader, metadata, table,
capture, and storage records remain caller-owned.

Language evidence follows the nearest HTML or XML declaration without
canonicalizing its BCP 47 spelling. Direction evidence follows HTML `dir`,
including first-strong resolution for `auto` and `bdi`. Fleece records logical
DOM ranges and inherited values; transport language, live form values, shadow
trees, rendered bidi order, and platform accessibility state belong to callers.

`text_fragment` projects quote evidence into a `:~:text=...` directive
component using the WICG draft revision pinned in its module documentation. It
does not compose a source URL or implement navigation, activation, indication,
or script-visible URL privacy; those remain browser-host responsibilities. This
is a syntax projection: `FleeceDomTextV1` is not the browser's rendered-text
matching stream, so resolution requires separate host evidence.

## Structured data

Fleece retains every HTML `application/ld+json` script as an
`EmbeddedJsonLdBlock` before projection. Blocks preserve DOM order, duplicate or
empty element IDs, the exact declared type and DOM script text, the complete
parsed JSON root when valid, and an explicit invalid-JSON state. Exact response
bytes remain acquisition-owned.

Parsed object roots are projected into `StructuredData` alongside HTML
Microdata. `types` preserves every declared `@type` or `itemtype` string without
shortening or expansion, while `id` preserves a raw `@id` or `itemid`.
`JsonLdBlockRecordV1` preserves one block under `wire`, hashes its DOM text, and
reparses it on decode to verify the stored syntax outcome.

Microdata follows the HTML item/property traversal, including `itemref`, nested
items, repeated properties, element-specific values, duplicate suppression,
and cycle protection. URL-valued attributes and identifiers remain raw source
strings because Fleece has no source URL or document-base authority.

This is syntax harvesting, not JSON-LD 1.1 expansion, RDF construction,
vocabulary reasoning, URL resolution, or remote-context loading.

## Document metadata and links

Fleece 0.4 retains Open Graph properties as ordered raw evidence and also
projects each root property with the structured properties that immediately
follow it. Unknown and malformed properties remain in the raw list rather than
being discarded or fabricated into a group.

`Metadata::links` records HTML `<link>` elements in document order, including
tokenized relations, raw `href`, media type, language, title, media query, and
other observed attributes. Registered relation names use their lowercase form;
extension-relation IRIs retain their source spelling, with
`DocumentLink::has_relation` providing the case-insensitive comparison required
by Web Linking. `Metadata::canonical` is only the first matching raw-href
projection. Fleece does not resolve URLs or observe HTTP `Link` headers.

## HTML tables

`Block::Table` carries a computed `Table` rather than a shallow row list. The
value retains the source id and caption, row groups, declared headers and spans,
effective grid coordinates, column- and row-group-aware header associations,
and recoverable table-model overlap errors. It honors `rowspan=0`, clamped span
ranges, implied rows, explicit-header precedence, first-id lookup, and nested
table boundaries.

This is HTML table semantics over the supplied DOM, not CSS table layout.
Presentation, geometry, and accessibility UI remain consumer responsibilities.

## License

MPL-2.0
