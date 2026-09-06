# Fleece preservation contract

**Date:** 2026-09-05
**Status:** active; Fleece 0.5 canonical-text preservation, selector projection,
language/direction evidence, and lossless embedded JSON-LD records are implemented
and green; the complete cross-crate Web Annotation profile and the remaining
standards lanes are still in progress

Fleece 0.5 is complete against its current local render-free extraction profile
over a caller-supplied `LayoutDom`. It already owns canonical DOM text,
structured reader blocks,
metadata, structured data, semantic tables, paired quote/position anchors, and
Text Fragment projection. Community snapshots now require those results to
survive hashing, transfer, reopen, and later annotation resolution.

## Ownership

Fleece owns the deterministic meaning of an extraction and anchors within its
canonical text. It does not own fetching, URL resolution, redirect history,
response headers, capture time, raw or replay resources, storage, replication,
or community admission. A caller such as Eidetic binds those capture facts to
the preserved Fleece payload.

## Conformance rule

"W3C compliant" is not a useful crate-wide claim. Every claim names the
specification revision, conformance class, media type, producer or consumer
role, and the evidence that covers its applicable normative requirements.
Until that evidence is green, documentation says `partial support`.

Complete conformance remains the goal for every declared profile. Each profile
has:

- a ledger of applicable `MUST`, `MUST NOT`, `SHOULD`, and `SHOULD NOT`
  requirements;
- an explicit owner for acquisition, extraction, graph processing, projection,
  validation, and serving;
- the official test suite where one exists, plus local fixtures for requirements
  the suite does not exercise; and
- encode, independent-decode, and reopen receipts over the public interchange
  representation.

## Standards ledger

| Standard | Current evidence | Complete target and owner |
|---|---|---|
| [W3C Web Annotation Data Model](https://www.w3.org/TR/annotation-model/) and [Vocabulary](https://www.w3.org/TR/annotation-vocab/) | Fleece produces correct source-relative `TextQuoteSelector` and `TextPositionSelector` values: logical Unicode code-point positions, half-open ranges, whole-grapheme boundaries, and sibling selectors for the same segment. This is partial selector support, not a conforming Annotation representation. | Fleece owns selector values and their typed projection. Eidetic owns the Annotation, SpecificResource, immutable source IRI, state, creator, motivation, rights, and JSON-LD envelope. The first complete profile targets the preserved canonical `text/plain; charset=utf-8` resource, whose required selector set is Fragment, Text Quote, and Text Position. The FragmentSelector uses [RFC 5147](https://www.rfc-editor.org/rfc/rfc5147) character ranges. A separate direct-HTML profile may claim conformance only after Fragment, CSS, XPath, Text Quote, and Text Position combinations are all implemented. |
| [JSON-LD 1.1](https://www.w3.org/TR/json-ld11/) and its [Processing Algorithms and API](https://www.w3.org/TR/json-ld11-api/) | Fleece losslessly retains every matching HTML script's DOM text, element identity, DOM order, exact media type, complete JSON root when valid, and explicit parse failure when invalid. The versioned wire record hashes the DOM text and verifies the syntax outcome by reparsing. This is syntax preservation, not JSON-LD processing. | `mere-linked-data`, already using `oxjsonld`, is the sole JSON-LD processor and proves its claimed algorithms with the official JSON-LD tests. Its string-scanning HTML extractor is retired in favor of Fleece output. |
| [RDFa Core 1.1](https://www.w3.org/TR/rdfa-core/) and [HTML+RDFa 1.1](https://www.w3.org/TR/html-rdfa/) | Missing. JSON-LD plus Microdata does not cover RDFa-bearing pages. | Fleece exposes lossless DOM-carried RDFa evidence with source positions. `mere-linked-data` owns the conforming RDFa processor because the normative output is one RDF graph and processing needs media type, base IRI, language, prefix context, CURIE expansion, lists, reverse relations, datatypes, and blank-node rules. Gate on the W3C RDFa tests. |
| [RDF 1.1](https://www.w3.org/TR/rdf11-concepts/) and [RDF 1.2](https://www.w3.org/TR/rdf12-concepts/) | `mere-linked-data` already owns the richer RDF dataset projection. `chartulary::rdf` is a narrower duplicate with no outside consumer. Mere enables RDF 1.2 triple terms. | Retire the duplicate Chartulary projection in favor of `mere-linked-data`. Claim RDF 1.1 as the stable baseline. Report RDF 1.2 as draft compatibility until its current Candidate Recommendation reaches Recommendation. |
| [WHATWG HTML Microdata](https://html.spec.whatwg.org/multipage/microdata.html) | Fleece implements item/property traversal, `itemref`, nested items, element-specific values, ordering, duplicate suppression, and cycle protection. URL values remain raw, and the public shape is not the standard JSON conversion. | Preserve the raw evidence and add an exact HTML-to-JSON projection that accepts caller-supplied document-base identity for URL resolution. Retain detected errors rather than silently suppressing them. Gate it on applicable Web Platform Tests. This is an HTML Living Standard target, not a W3C claim. |
| [WHATWG HTML table model](https://html.spec.whatwg.org/multipage/tables.html) | Fleece computes the grid, spans, row and column groups, table-model errors, and header associations. Local fixtures are green. It drops `th@abbr`. | Retain `abbr`, audit every branch of the forming-a-table and assigning-header-cells algorithms, and gate the profile on applicable Web Platform Tests plus a normative-step ledger. CSS table layout and accessibility-tree projection remain separate Genet owners. |
| HTML core, metadata, links, language, and direction | Canonical source spans retain raw and effective HTML/XML language plus HTML `ltr`, `rtl`, and first-strong `auto` direction. Mixed inline values survive the wire record. Live form values, shadow trees, rendered bidi order, and transport language remain outside `LayoutDom`. Several other walks still compare local names without checking the HTML namespace; metadata does not retain the full ordered `<base>`, `<meta>`, and `<link>` evidence. | Namespace-gate every HTML projection; preserve ordered base and metadata evidence; distinguish raw response URL from computed document base; and add host-supplied live/shadow evidence where a declared profile requires it. URL parsing and resolution use the WHATWG URL rules at the caller boundary. |
| [WAI-ARIA 1.2](https://www.w3.org/TR/wai-aria-1.2/), [ARIA in HTML](https://www.w3.org/TR/html-aria/), and [Accessible Name 1.1](https://www.w3.org/TR/accname/) | Fleece retains some native semantics such as headings, table headers, and image alternatives. It does not compute an accessibility tree or accessible names. | Fleece retains source evidence useful to the shared semantic plane: native kind, role and `aria-*`, IDREF relations, alternatives, labels, `th@abbr`, language, direction, and supplied live state. Genet owns user-agent conformance, including computed hidden state, native/ARIA conflict rules, names, descriptions, roles, shadow structure, and platform exposure. AccName 1.2 and HTML-AAM remain watched draft targets. WCAG 2.2 applies to complete reader and annotation interfaces. |
| [PROV-O](https://www.w3.org/TR/prov-o/) | Mere linked-data already uses a small PROV vocabulary subset. Fleece has extraction lineage, but no standards projection. | Eidetic projects capture, extraction, contribution, revision, and hosting acts as PROV entities, activities, agents, and derivations while retaining the native signed journal as authority. PROV-O is an interchange view, not the proof mechanism. |
| [SHACL 1.0](https://www.w3.org/TR/shacl/) | Outside Fleece. | `mere-linked-data` validates RDF projections and published community schemas. A moot may publish shapes as flora for contribution structure while keeping admission and rewards in community policy. Keep the 2017 Recommendation as the conformance base and watch SHACL 1.2 separately while it remains a Working Draft. |
| [Web Annotation Protocol](https://www.w3.org/TR/annotation-protocol/) | Outside extraction and absent. | An HTTP Annotation Server owns this profile if one is built. NativeDrop and p2p transports may carry Data-Model-conforming Annotation values without implementing the HTTP protocol. |
| [URL Fragment Text Directives](https://wicg.github.io/scroll-to-text-fragment/) | Fleece has a revision-pinned quote-to-directive syntax projection. Its tests resolve against a Rust string, while browser matching uses rendered text and different visibility and boundary rules. | Name the current claim `TextFragmentSyntaxV1`. Complete browser resolution belongs to Genet through a host-supplied rendered-text/range seam, headed Web Platform Tests, and activation/privacy receipts. The WICG draft contributes no W3C conformance credit. |

Memento, WARC, WACZ, HTTP response metadata, and Web Linking headers apply to
acquisition or replay. They matter to the complete snapshot system, while their
normative processing remains outside Fleece.

## Implementation order

1. **Preserved text target.** Land the versioned extraction record, arbitrary
   range mint/resolve operations, immutable canonical-text identity, RFC 5147
   FragmentSelector, Web Annotation selector projection, effective language and
   direction, and the Eidetic reopen proof.
2. **Lossless page-carried data.** Preserve every JSON-LD block and parse outcome,
   retire `mere-linked-data`'s HTML string scanner, add the caller-resolved
   Microdata projection, and establish the composed RDFa processor seam.
3. **HTML evidence hardening.** Namespace-gate every HTML walk, retain document
   base and full ordered metadata evidence, add `th@abbr`, and close the HTML
   table-model ledger.
4. **Host semantics.** Feed Fleece the shared Genet semantic/accessibility
   projection where reader output needs it, and prove Text Fragment resolution
   against the rendered-text algorithm in headed browser tests.
5. **Graph interchange.** Remove the duplicate Chartulary RDF projection, prove
   JSON-LD and RDFa processing in `mere-linked-data`, add PROV-O capture and
   contribution projection in Eidetic, then apply SHACL to published community
   schemas.

Each step may ship independently. A later step does not weaken the complete
conformance requirement for an earlier declared profile.

## Fleece extraction record v1

Every `ExtractedDocument`, including one with no selected `Article`, carries an
extraction contract with:

- a stable record/schema id;
- `FleeceDomTextV1` as the canonical text profile;
- a versioned reader-selection profile;
- the configured quote-context size; and
- the Fleece implementation version.

A feature-gated wire value preserves the canonical text, extraction contract,
and an ordered sequence of anchors and missing-anchor states. Decoding validates
the content-derived text identity, every half-open code-point range, and each
quote selector against the canonical text. The wider page extract, reader
structure, metadata, structured data, and tables remain outside this first
stable record and require their own versioned preservation shapes.

The same record exposes its canonical text as an immutable UTF-8 `text/plain`
resource with a content-derived IRI. The W3C Annotation target selects that
resource; the captured HTML is retained as its scope and PROV source. This
avoids implying that an independent client can reproduce Fleece's whitespace
normalization from arbitrary HTML. A `FragmentSelector` using RFC 5147
`char=start,end`, a `TextQuoteSelector`, and a `TextPositionSelector` must all
resolve to the same segment.

Fleece also supplies pure `anchor_for_range` and `resolve_anchor` operations over
canonical text. They retain whole grapheme clusters in context and preserve
ambiguity for repeated quotations rather than selecting by arrival order.

The public documentation calls `PageExtract::text` canonical DOM text. It does
not claim CSS visibility: Fleece excludes its pinned inert/non-content subtrees
without running style or layout.

## Done conditions

- Article and non-article pages carry complete extraction identity.
- Unicode and repeated-text selections survive encode, decode, and resolution.
- The declared `text/plain` Web Annotation selector profile covers Fragment,
  Text Quote, and Text Position selectors, and all three resolve identically.
- A complete Annotation JSON-LD value expands and round-trips to an isomorphic
  RDF graph through an implementation independent of Fleece's native wire
  codec.
- Mixed-language and bidirectional fixtures retain effective language and
  direction on each anchored span.
- A golden record reloads byte-identically and retains absent anchors.
- Equivalent static and scripted DOMs produce identical records.
- The normal dependency cone still excludes fetching, storage, networking,
  cascade, layout, and paint.
- An Eidetic consumer binds source/capture identity and raw or replay blobs to
  the Fleece payload hash, transfers it through a NativeDrop, and reopens the
  exact record.
- An annotation bound to an earlier capture cannot silently validate against a
  changed capture.
- Embedded JSON-LD reaches `mere-linked-data` through Fleece rather than its
  string-scanning HTML path, and RDFa has an owned conformance path.

Search remains a derived consumer. This plan does not select an index engine.

## Progress receipt: 2026-09-05 through 2026-09-06

Implemented in Fleece 0.5:

- document-level extraction identity for article and non-article pages;
- SHA-256 canonical-text IRIs and the declared UTF-8 `text/plain` media type;
- versioned normalization, reader profile, quote context, and producer version;
- arbitrary grapheme-safe range minting and all-match quote resolution;
- RFC 5147 fragment, Text Quote, and Text Position sibling projections;
- mixed inline language and HTML direction evidence over canonical source ranges;
- lossless HTML JSON-LD blocks, including scalar roots, duplicate IDs, exact DOM
  text and type, and explicit invalid syntax; and
- feature-gated wire records with hash, range, evidence, media-type, and syntax
  validation on decode.

`cargo test -p fleece --all-features -j 1` passed 68 tests,
`cargo clippy -p fleece --all-targets --all-features -- -D warnings` passed,
and the all-feature crate checks for `wasm32-unknown-unknown`.
Mere's opt-in `mere-document-lanes/eidetic-bridge` now supplies the Annotation
JSON-LD envelope and a green Eidetic/Fjall close/reopen proof over Mere's current
pinned Fleece 0.4 surface. An independent `oxjsonld`/`oxrdf` oracle expands that
envelope against an offline copy of the official Annotation context and compares
the resulting dataset for blank-node isomorphism with a hand-built expected
graph. Releasing and adopting Fleece 0.5 will remove the temporary cross-repository
version gap. The official JSON-LD suites, scripted-DOM equivalence, NativeDrop
transfer, capture-state preservation, and the broader standards lanes remain open.
