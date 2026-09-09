# Scope: fleece — reader and extraction lane

**Date:** 2026-08-22
**Status:** implemented and focused-verified 2026-08-23. First scope of
the reader/extraction capability; before this work nothing else in genet or
mere had planned it beyond the one-line U3 item in Turnstone's
[user-agent taxonomy](../../turnstone/design_docs/2026-08-03_user_agent_taxonomy_plan.md).
Mere-side context: the port law in the
[family composition thesis](../../mere/design_docs/2026-08-12_family_composition_thesis_brief.md).

**2026-08-26 follow-through:** F0's one-release compatibility allowance was not
retired after Fleece 0.2; `genet-extract` survived through 0.3.0. It is now
retired under the
[Fleece follow-through plan](../design_docs/2026-08-26_fleece_followthrough_plan.md):
Mere imports through Fleece directly, and the Genet shim and its workspace/CI
requirements are gone. Published historical versions remain available.
The lowering-home and scripted-sequencing questions below were resolved by the
implementation receipt. Only the Gazette reading-room product decision remains
open.

## Grounding (verified against the tree, 2026-08-22)

- `components/fleece` is a 21-line name reservation. Its README states the
  boundaries: not import (stored browser data), not crawl (the frontier), not
  illume (source spans); fleece "works a live document."
- `components/genet-extract` (`lib.rs`, 712 lines) is the lane fleece
  succeeded. Its historical single dependency was `layout_dom_api`. Fleece 0.5
  now requires that API, SHA-256, and Unicode segmentation and bidi support;
  optional Serde/JSON remains isolated behind `wire`. Feature-aware default and
  `wire` cones witness that extraction pulls no render, network, or storage
  stack. The original lane exposed `extract`, `extract_links`,
  `extract_title`, `extract_headings`, `extract_metadata`, `extract_text`,
  `extract_main_text`, and `PageExtract { title, metadata, headings, text,
  main_text, links }`. The readability heuristic is real but small: a semantic
  `<main>` wins outright, else the best of `div | section | article | td`
  scored by a class/id signal, a tag bonus, and paragraph density capped at
  50. Output is text only: no blocks, no images, no tables, no inline links.
- The only consumer is `turnstone/src/browse.rs:459`, which takes `.title`
  from a `genet_static_dom::StaticDocument` and discards the rest. Pelt does
  not consume it. The leverage census lists `genet-extract` as consumed
  precisely because of that one call.
- The reader surface does not exist in any host. Inker's routing table
  (`components/inker/src/routing.rs`) has `genet.*`, `nematic.*`,
  `scrying.web`, `graft.servo`, `weld.chromium`, and `host.external-protocol`
  ids; no reader id. `genet-host-api::EngineProfile` has `Viewer` / `Static`
  for script-free documents and `ContentSource::Open { kind, id }` as the
  host-named tail.
- Nematic lowers fifteen authored and protocol formats into Inker's
  `EngineDocument`. That is the existing path from "a structured document"
  to "a themed, rendered, selectable page," and it is the path a reader
  rendering should reuse rather than growing a second renderer.
- Downstream wants the text and does not have it: the
  [search surface wiring plan](../../mere/design_docs/mere_docs/implementation_strategy/2026-08-12_search_surface_wiring_plan.md)
  landed W1 capture, but `BrowsingTrace` carries `PageRef`s and transitions,
  not page text; W2/W3 need a corpus. The
  [gazette brief](../../mere/design_docs/mere_docs/research/2026-08-10_credential_port_gazette_brief.md)
  pipeline is "gazette discovers, nematic parses, eidetic stores, a surface
  composes," and a feed item's linked article is exactly a live document to
  fleece. `mere-crawl` is held "until the gazette feed pipeline lands."

## What fleece is, under the port law

The capability is **render-free extraction of a live document into a
structured article**. Fleece the component is the capability's embeddable
half: any host calls it, none inherits a product. The first-party
embodiment is the **reader lane**: an engine id every Genet host can route a
tile to, rendering the article through the existing document engine in the
host's theme. There is no standalone fleece application in this scope; the
anti-shell receipt is that two hosts (Turnstone and pelt) route to the same
lane without either owning it.

Whether a standalone reading-room product exists is an open decision (below),
and it is a gazette/alembic question, not a fleece one: fleece produces
articles, it does not keep them.

## Slices (done-conditions, not dates)

### F0. Absorb genet-extract

Move the lane into `components/fleece`; `genet-extract` becomes a
re-exporting shim for one release, then is deleted. Keep the single-dependency
witness (`layout_dom_api` only; `genet-static-dom` dev-only). Turnstone's
`browse.rs` call site moves to `fleece::extract`.
**Done when:** `genet-extract` has no source of its own, the dependency-cone
witness still passes, and Turnstone builds and enriches titles as before.

**Follow-through complete 2026-08-26:** the temporary shim is deleted. The last
Mere import call now names Fleece, and the dependency-cone witness requires
Fleece without requiring the retired package.

### F1. Article, not text

Add `fleece::Article`: `title`, `byline`, `published`, `lang`, `site`,
`canonical`, `lead_image`, and `blocks: Vec<Block>` where a block is a
heading (level), paragraph, list (ordered/unordered, nested), quote, code
(with language hint), table (rows of cells, header row flagged), figure
(image `src` as the raw attribute, `alt`, caption), or rule. Inline runs keep
links (raw `href`, the caller resolves), emphasis, and code. `PageExtract`
stays as the flat index shape; `Article` is the reader shape; both come
from one walk.

The root selection is the existing heuristic extended with the signals it
lacks today: link density (a block that is mostly links is chrome), negative
class/id terms (comment, share, related, promo), sibling absorption (adjacent
candidate blocks with article-grade density join the root), and `<article>`
count (several `<article>` elements means an index page, return `None`).

**Done when:** a fixture corpus of at least twenty real pages (news, blog,
documentation, recipe, forum thread, SPA shell, link index) has hand-labelled
expected main text, and fleece's main-text precision and recall against those
labels are reported by a test, with the SPA shell and link index yielding
`None`. Numbers are the receipt; no threshold is set here because none has
been measured yet.

### F2. The reader lane

Lower `Article` to Inker's `EngineDocument` the way nematic lowers gemtext or
markdown, and register `genet.reader` in Inker's routing table as a
selectable engine over `text/html`. The lane renders through the document
engine with the host's theme tokens, so tabard's output applies to it
unchanged. Source of truth is the bytes the node already holds: switching a
tile to the reader lane re-extracts from the held document, and switching
back re-renders the original. No second fetch, no mutation of the node.

**Done when:** Turnstone's U3 holds (a cluttered page has a readable lane and
switching back is lossless) **and** pelt offers the same lane from its engine
picker. Two hosts, one lane, neither owning it.

### F3. Lineage

Every reader rendering records how it was made: fleece version, the root
selector that won (`main` or scored candidate plus score), and the block
count. In Turnstone this is a facet on the node (the taxonomy plan's
"extraction lineage on reader renderings"); in pelt it is shown in the tile's
posture chrome and not persisted.

**Done when:** a Turnstone node rendered through the reader lane carries the
lineage facet, and re-extracting with a newer fleece updates it.

### F4. Scripted-DOM extraction

The same functions over a post-JS DOM (`genet-scripted-dom`) for pages whose
article arrives by script. Static stays the cheap path; scripted is selected
only when the static article is `None` and the page carries script.

**Done when:** a fixture SPA that renders its article client-side yields an
`Article` under the scripted profile, and a static page yields byte-identical
articles under both.

### F5. Consumers

Each a one-line wiring receipt, in the consumer's own plan:

- **eidetic-search W2/W3:** `PageExtract.text` and `main_text` as the
  indexed corpus for a visited page.
- **gazette feed pipeline:** a feed item's linked page fleeced into an
  `Article` at poll time, stored as the item's body.
- **crawl:** `links` as the frontier source; already the documented intent.
- **alembic:** the article as the distillation input for a page engram
  instead of raw bytes.

**Done when:** each consumer names fleece in its manifest and its plan records
the receipt; fleece itself gains no dependency on any of them.

**2026-08-26 audit:** this done-condition recorded dependency intent, not live
consumption. Crawl, Gazette, and eidetic-search still had no Fleece call in
source. The follow-through plan replaces those manifest-only receipts with two
real extraction seams and removes the dependency from the wrong layer.

### F6. Structured data harvest

Page-carried metadata beyond OpenGraph: JSON-LD and microdata blocks
extracted as typed values (`Recipe`, `Event`, `Person`, `Article`, ...). The
consumer is scholia's ingest roadmap and Turnstone's browser taxonomy (a page
that declares itself an Event can mint an event-shaped node). Fleece only
harvests; typing and minting are the consumer's.

**Done when:** a fixture page with JSON-LD yields its blocks as parsed values
with their `@type`, and a page without yields an empty list.

## Boundaries (restated so the scope has edges)

- Not nematic: nematic lowers authored and protocol formats; fleece reads
  HTML documents that were never authored for reading. The reader lane
  *reuses* nematic's lowering target (`EngineDocument`); it does not join
  nematic's engine family.
- Not a keeper: highlights, read-later, annotations belong to Knot, eidetic,
  or the gazette reading room. Fleece returns an `Article` and forgets it.
- Not a fetcher: the host supplies the document. Fleece never touches the
  network, so the single-dependency witness keeps holding.
- Prior art is read for technique, not copied: Mozilla Readability
  (Apache-2.0), trafilatura, and the Chromium DOM Distiller all publish
  their scoring signals; the F1 signal list is drawn from what they agree on.

## Open decisions (Mark's)

1. **A standalone reading-room product.** Read-later plus feeds plus
   highlights is a recognizable product (Pocket, Omnivore). If it exists in
   the family it is the gazette port's view over fleeced articles with Knot
   holding the highlights, not a fleece port. Decide when gazette is planned.
2. **Resolved 2026-08-23, lowering home.** `Article -> EngineDocument` lives in
   `genet-documents` beside the existing lowerings; Fleece remains render-free.
3. **Resolved 2026-08-23, scripted sequencing.** F4 ran over the scripted
   profile and its static/scripted equivalence fixture passed.

## Relative size

F0 small; F1 the bulk (the corpus and the labelling are most of it); F2
medium, mostly routing and a lowering; F3 small; F4 small once F1 holds; F5
zero in fleece; F6 small.

## Implementation receipt (2026-08-23)

- **F0:** `fleece` owns the implementation with `layout_dom_api` as its only
  normal dependency and `genet-static-dom` dev-only. `genet-extract` is a
  pure `pub use fleece::*` compatibility shim. Genet, Pelt, and Turnstone
  internal consumers now name `fleece`. **2026-08-26:** the compatibility shim
  was deleted after the remaining Mere importer moved to Fleece directly.
- **F1:** `Article`, block and inline structure, metadata, link-density and
  negative-term scoring, sibling absorption, and multi-article rejection are
  live. The checked-in labelled corpus has 20 full-page representative
  fixtures with navigation, sidebars, promos, comments, SPA shell, and link
  index. Its current word-set receipt is precision `0.833`, recall `1.000`,
  `tp=295`, `fp=59`, `fn=0`. This is an algorithmic baseline, not a claim of
  broad-web quality.
- **F2/F3:** `genet.reader` lowers held HTML through `EngineDocument` and the
  shared document canvas. Pelt exposes `--engine reader`; Turnstone exposes
  the same viewer pin, respawns from its held bytes, and projects typed
  extraction lineage to `web.reader-lineage`. Pelt shows lineage in the
  window posture.
- **F4:** static extraction returns an explicit `Article | NeedsScriptedDom |
  NotReadable` decision. `ScriptedDocument::extract_article` runs the same
  Fleece function over the post-JS DOM. The SPA and static-equivalence tests
  pass on Boa.
- **F5:** the original release recorded manifest-level intent. **2026-08-26:**
  crawl now extracts and resolves links from supplied HTML, Gazette exposes a
  supplied-HTML-to-`Article` seam, eidetic-search dropped its misplaced direct
  dependency, and Alembic remains dependency-free until Article-consuming
  distillation exists. Fetch, polling, storage, and reading-room composition
  remain consumer-owned work.
- **F6:** JSON-LD and microdata harvest into dependency-free typed values,
  retaining uninterpreted fields for downstream typing.

Focused gates: `cargo test -p fleece --offline` (21 passed),
`cargo test -p genet-scripted --offline extraction_tests` (2 passed),
`cargo test -p genet-documents --features reader --offline` (6 passed),
the explicit Inker reader-route test, the `genet-extract` shim build,
`cargo check -p pelt --features reader --offline`, Turnstone's three `reader_`
tests, and checks for all four Mere consumer packages.
