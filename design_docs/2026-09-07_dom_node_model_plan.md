# DOM node model — fragment insertion, CDATA, doctypes, and the attribute surface

**Status: landed 2026-09-07.** Base commit `ddee26ef559` (Selection and Range).
Every gate below is green; the census maps are under
`Code/testing/genet/wpt-ledger/2026-09-07_dom_node_model/`.

This lane closes the core DOM residuals the
[MutationObserver](2026-09-07_mutation_observer_plan.md) and
[Selection and Range](2026-09-07_selection_range_plan.md) plans named and left
to Mark: `DocumentFragment` insertion (which changes core insertion semantics),
`CDATASection` and real `DocumentType` nodes (which `dom/common.js` floors two
census directories on), the `ParentNode` / `ChildNode` mixins, `normalize`,
`outerHTML`, attributes with real namespaces as `Attr` nodes, a `cloneNode` that
keeps every node kind, `DOMParser` / `XMLSerializer`, and the `Node-baseURI`
regression.

## What the lane is

Six pieces, in landing order. Each is small on its own; together they are the
node model the rest of the DOM was written against.

### 1. `DocumentFragment` insertion moves the fragment's children

`appendChild` / `insertBefore` / `replaceChild` of a fragment inserted the
fragment **node** and left its children where they were. The DOM's "insert a
node into a parent before a child" says the fragment contributes its children:
they are removed from the fragment first, then inserted under the parent.

The fix is one function at the bootstrap's own insertion path,
`insertNodeInto(parent, node, ref)` in `dom/bootstrap.js`, not in the arena. The
call site is where the live-range steps, the custom-element connect/disconnect
steps and the observer grouping already live, and the arena's `append_child` /
`insert_before` stay exactly as Livery's `DomMutation` consumer expects them.

The record shape falls out of the arena's existing coalescing group: inside one
`moBeginGroup()` the per-child removals merge into **one** `childList` record on
the fragment (`removedNodes` = all children, both siblings null, because the
first removal has no previous sibling and the last has no next), and the
per-child inserts merge into **one** record on the parent. That is exactly the
two records the spec queues. `insertNodeInto` returns before opening the group
when the fragment is empty, matching the spec's "if count is 0, then return".

Two hand-rolled workarounds went away with it: `Range`'s
`appendFragmentChildren` is now one `appendChild`, and `Range.insertNode`'s
fragment branch is one `insertBefore`.

### 2. `CDATASection` and real `DocumentType` nodes

`NodeKind` gains `CdataSection`; `Doctype` already existed but nothing minted
one in the scripted arena, and `createDocumentType` fabricated an
`HTMLUnknownElement` named `!doctype`.

- `layout-dom-api` gains `NodeKind::CdataSection` and a defaulted
  `LayoutDom::doctype_data(id) -> Option<DoctypeView>` carrying name, public id
  and system id. `text()` could not serve: a doctype has three strings and a
  generic tree copier needs all three.
- `genet-scripted-dom` gains `create_cdata_section` and `create_doctype`. The
  doctype's name lives in `text` — which is what the html5ever serializer
  already reads for a doctype — and the two external identifiers ride in the
  node's `attrs` under reserved `__publicId` / `__systemId` keys. A doctype is
  not an element and never reaches the cascade, so no per-node storage is added
  for a kind a document has at most one of.
- `genet-static-dom` implements `doctype_data` over its existing
  `StaticNodeKind::Doctype`. It never produces a `CdataSection`: html5ever has
  no CDATA, and xml5ever's tokenizer emits a CDATA section's contents as
  characters.
- The stats counter folds `CdataSection` into `text`, because `CDATASection` is
  a `Text` in the DOM's own hierarchy and `engine-observables-api`'s
  `DomNodeKindStats` (outside this lane) has no separate field.
- `nodeType` 4 and 10, `nodeName` `#cdata-section` and the doctype's own name,
  `nodeValue` for CDATA and processing instructions, and HTML serialization of
  both (a CDATA section serializes as its character data, since HTML has none;
  a processing instruction now serializes through html5ever's
  `write_processing_instruction` rather than vanishing).
- `document.doctype`, `DOMImplementation.createDocumentType` minting a real
  node, `createDocument` accepting one, and `document.createCDATASection` with
  the HTML-document `NotSupportedError`.
- `new Document()` mints an **XML** document, which is what makes
  `createCDATASection` legal on it. `dom/common.js` does exactly this.

### 3. `ParentNode` and `ChildNode`

`append` / `prepend` / `replaceChildren` were absent; `before` / `after` /
`replaceWith` / `remove` existed on `Element` only and looped over their
arguments. Both mixins now run the spec's "convert nodes into a node" — a lone
node stays itself, a string becomes a text node, and two or more become one
`DocumentFragment` — which is a single insert each, now that inserting a
fragment moves its children. `replaceWith` and `after` use the spec's viable
sibling (the nearest sibling that is not itself being inserted). `ChildNode`
lands on `Element`, `CharacterData` and `DocumentType`; `ParentNode` on
`Element`, `Document` and `DocumentFragment`.

### 4. `normalize`, `outerHTML`, `Attr` / `NamedNodeMap`, `cloneNode`

- **`Node.normalize`** drops empty exclusive `Text` nodes and merges each
  contiguous run into its first node, walking **live** rather than over a
  snapshot (the merge removes siblings as it goes; a snapshot walk hands
  `removeChild` a node that already left). The merge goes through `appendData`
  and the removal through `removeChild`, so the live-range steps at the mutation
  funnel see a real replace-data and a real removal.
- **`outerHTML`** reads through a new `__getOuterHtml` sink over the arena's
  existing `outer_html`. The setter fragment-parses in the parent's context and
  swaps the element for the result inside one group, so an observer sees one
  `childList` record. No parent, or a `Document` parent, is
  `NoModificationAllowedError`.
- **Attributes carry a real namespace.** `set_attribute` already keyed on a
  `QualName`; the bootstrap was passing the *qualified* name as a null-namespace
  local. New sinks `__setAttributeNS` / `__getAttributeNS` /
  `__removeAttributeNS` store and read `(namespace, prefix, local)`, and
  `__getAttribute` / `__removeAttribute` now match on the **qualified** name,
  which is what the DOM's "get an attribute by name" says. So
  `setAttributeNS(XLINK, 'xlink:href')` and `getAttribute('xlink:href')` name
  the same attribute from opposite directions, and a `MutationRecord`'s
  `attributeNamespace` is the namespace rather than `null` — the observer's
  record already encoded `name.ns`, it was always empty.
- **`Attr` is a JS view, not an arena node.** Attributes are a column on their
  element, so an `Attr` binds `(ownerElement, namespace, localName)` and reads
  and writes through the element; that is what "live" means for every test that
  reads `attr.value` after a `setAttribute`. A detached `Attr` from
  `createAttribute` owns its own value until `setAttributeNode` binds it. Views
  are cached per element, so `getAttributeNode('x') === getAttributeNode('x')`.
  `Element.attributes` is a `NamedNodeMap` proxy over
  `__attributeRecords`, with `item` / `getNamedItem(NS)` / `setNamedItem(NS)` /
  `removeNamedItem(NS)`, indexed and named access, plus `getAttributeNames`,
  `hasAttributes`, `hasAttributeNS`, `getAttributeNode(NS)`,
  `setAttributeNode(NS)` and `removeAttributeNode`.
- **`cloneNode`** copies namespaced attributes through
  `setAttributeNS`, and gained the CDATA, processing-instruction and doctype
  cases. Separately, `clone_into` — the static-to-scripted copy behind
  `load_dom` — now carries **every** node kind rather than elements and text
  only, so a parsed document's comments, PIs and `<!DOCTYPE>` reach the live
  document. That is what makes `document.doctype` non-null on a real page and
  what gives `MutationObserver-characterData`'s Comment and PI cases a node to
  observe.

### 5. `DOMParser` and `XMLSerializer`

`__parseDocument(source, kind)` builds a whole new `Document` **in the same
arena** through the static tier's parsers — html5ever for `text/html`, xml5ever
for the four XML types — and copies it in with `clone_into`. It returns
`undefined` when the XML parse produced no root element, which is how the JS
side knows to build the spec's `parsererror` document instead of throwing.
`XMLSerializer.serializeToString` is a pure-JS walk (CDATA sections, PIs,
comments and the doctype included) rather than a second Rust serializer, because
html5ever's is an HTML serializer.

XHR's `responseXML` is wired to it: an HTML or XML final MIME type parses once
and caches, everything else stays `null`, and a failed XML parse is `null`
rather than a `parsererror` document, per XHR. The cache is invalidated wherever
`_respObj` is.

### 6. `Node-baseURI`

`Node.prototype.baseURI` did not exist at all. `dom/nodes/Node-baseURI.html`
asserts `node.baseURI === document.URL` nine times, and it scored 4 of 9 while
**both** sides were `undefined`. `document.URL` was defined on 2026-08-26 by
`dfd42ebfa59` ("Add retained text fragment activation"), two days after the
`dom_boa.json` baseline was pinned on 2026-08-24; from that commit the four
element subtests compared a real URL against `undefined` and the file scored 0.
The four "passes" in the checked baseline were never real.

`baseURI` is now the node document's base URL — `document.URL`, overridden by
the first `<base href>` resolved against it — for every node, attributes
included. With `createAttribute` and `getAttributeNode` from piece 4, the file
is 9 of 9, and that one entry is repinned. Nothing else in `dom_boa.json` was
touched.

## Findings

1. **A false pass is a pin, not a score.** The `Node-baseURI` regression was not
   a regression: defining `document.URL` correctly turned four
   `undefined === undefined` passes into four honest failures. The pin recorded
   the false state for two weeks. When a subtest asserts `a === b` and neither
   side is implemented, it passes; when the census reports a directory going
   *down*, check for a newly-defined name on one side of an equality.
2. **The arena's coalescing group already had the spec's fragment record
   shape.** Nothing new was needed for the two-record shape: opening a group
   around N removals and N inserts merged them per target, and the merge's
   previous/next-sibling rule (first record's previous, last record's next) is
   the spec's. The observer machinery was built one lane earlier for
   `replaceChild` and fit unchanged.
3. **`getAttribute` matches a qualified name, not a local name.** The old store
   put the whole qualified name in the local slot with a null namespace, which
   made `getAttribute('xlink:href')` work by accident and
   `getAttributeNS(XLINK, 'href')` work by ignoring its namespace argument.
   Giving attributes real namespaces means the *by-name* lookups have to compare
   `prefix:local`, or every namespaced attribute becomes unreachable from the
   non-NS methods. Both `__getAttribute` and `__removeAttribute` were changed in
   the same pass.
4. **`normalize` cannot walk a snapshot.** The first version took
   `rawChildNodes` once and then removed merged siblings, so the loop later
   handed `removeChild` a node it had already detached — `NotFoundError` on both
   backends. Any DOM algorithm whose body removes siblings has to walk live.
5. **xml5ever recovers.** `parseFromString('<r>', 'text/xml')` yields a
   document with an `<r>` root, not a parse error; only a source with no root
   element at all reaches the `parsererror` path. Genet's XML parse errors are
   therefore under-reported relative to a browser, and that is a residual, not a
   defect of this lane.
6. **`new Document()` is an XML document.** `dom/common.js` builds its CDATA
   sections from one, and until `Document` was constructible the whole file
   threw before its first subtest — this was the *second* floor under
   `dom/ranges`, behind `CDATASection` itself. The directory went from 14 files
   still erroring to 1 once the constructor landed. This is the "one missing
   name floors a directory" principle firing twice in the same file.
7. **A missing method can hide an interpreter wall.** Six
   `dom/nodes/NodeList-static-length-getter-tampered-*` files were `fail` only
   because `fooRoot.append(...)` threw immediately. With `append` present they
   run their real body — five million proxy `get` traps — and time out; one
   still does not finish in 400 s. They never passed and cannot pass under Boa
   at the current NodeList proxy cost. See the residuals.

## Before and after

Exact `genet-wpt testharness` maps, disk mode, `--engine boa --renderer livery
--jobs 8 --timeout 90`. Both runners are release builds from this lane's own
target directory (`C:/t/lane9-target`); `pre` was built from `ddee26ef559`
**before** the first edit, per the working principle.

| Runner | SHA-256 |
|---|---|
| `pre` | `de2c1f9a2ae226c80c02173b63f90409eb3971a2e9d3bbd8696af0e3bc07784c` |
| `post` | `4dab5a9f775642df1749c3e0732c698ef9025b4d13167f798d020a765f2b4dbb` |

| Directory | files (pre → post, all-pass) | errored | subtests (pre → post) |
|---|---|---|---|
| `dom` | 173 → **209** | 45 → 28 | 2,995/7,098 → **43,961/53,909** |
| `dom/ranges` | 10 → **15** | 15 → **1** | 24/186 → **35,466/41,701** |
| `dom/nodes` | 83 → **108** | 10 → 10 | 2,360/5,663 → **6,491/9,431** |
| `html/dom` | 27 → 28 | 58 → 55 | 42,311/59,971 → 42,313/59,979 |
| `html/semantics/interfaces.html` | 0 → 0 | 0 → 0 | 298/438 → **435/438** |
| `custom-elements` | 5 → 6 | 25 → 24 | 2,095/3,827 → 2,122/3,832 |
| `xhr` | 64 → 64 | 28 → 28 | 282/1,191 → 282/1,191 |
| `selection` | 12 → **31** | 13 → 13 | 28,582/33,621 → **30,573/33,789** |
| `html/syntax` | 8 → **17** | 63 → 63 | 2,425/7,953 → **2,673/7,953** |

Aggregate status movements (`diff.txt`):

| Movement | Files |
|---|---|
| `fail → pass` | 84 |
| `error → pass` | 2 |
| `no-results → pass` | 10 |
| `error → fail` | 47 |
| `no-results → fail` | 8 |
| `fail → error` | 12 |
| `no-results → error` | 2 |
| **pass → anything** | **0** |
| subtest delta | **+82,944** |

`dom` and `dom/nodes` overlap (`dom` contains `dom/nodes` and `dom/ranges`), so
the per-directory movement counts double-count those files; the table is per
measured directory, as the census runs them.

### Every backward movement, explained

There is **no pass-to-fail movement at all**. Fourteen files move from a
non-passing status to `error`, in two groups:

1. **Six `dom/nodes/NodeList-static-length-getter-tampered-*` files,
   `fail → error`** (twelve counts, from `dom` and `dom/nodes` both). These are
   throughput tests: `makeStaticNodeList(100)` then five million NodeList index
   reads. They were `fail` because `fooRoot.append(...)` did not exist and threw
   on the first line. With the `ParentNode` mixin they run for real and hit the
   90 s lane timeout; one measured at 400 s still did not finish. Not a
   correctness regression — the file has never produced a passing subtest — but
   a real cost signal, recorded as residual 1.
2. **`dom/ranges/Range-mutations-dataChange.html`, `no-results → error`** (two
   counts). It reported nothing before because `dom/common.js` aborted. It now
   runs, and at `--timeout 400` it finishes in 107 s with **2,328 of 2,808
   subtests passing**. At the lane's 90 s budget it is killed. A timeout budget
   artifact on a file that gained 2,328 subtest passes, not a defect.

The forward movements land where the plan predicted: `dom/ranges` unfloors
(residual 1 of the Selection and Range plan), the four
`dom/nodes/MutationObserver-*` files all go `fail → pass` (residuals 1-6 of the
MutationObserver plan), `html/semantics/interfaces.html` goes 298 → 435 of 438
because the `useParser` variant now has a `DOMParser` to run through, and
`html/syntax`'s `outerHTML` and `innerHTML` fragment-serializing files pass on
the new `outerHTML`.

`xhr` does not move: its disk-mode corpus is network-bound, and `responseXML`
needs a served response body to parse. The wiring is proven by
`tests/dom_node_model.rs` and the existing `xhr_binding` suite; its census
receipt belongs to a server-mode run, which is the XHR plan's open next proof.

## Residuals

1. **NodeList indexed access is a `Proxy` trap per read.** Six
   `NodeList-static-length-getter-tampered-*` files cannot finish under Boa.
   A static `NodeList` (from `querySelectorAll`) could be a plain object with
   own indexed properties defined once instead of a proxy, which would also make
   the `Object.defineProperty(nodeList, 'length', …)` the tests tamper with
   actually take. That changes the shape of every collection `makeCollection`
   builds and wants its own census; it was not widened into this lane.
2. **`Attr` is not an arena node.** It has no `__ref`, so it is not reachable
   from a tree walk, `Attr` is not a `Range` boundary container beyond the
   guard that rejects it, and `document.adoptNode(attr)` is a no-op. Making
   attributes real nodes is a storage change in the arena, not a bootstrap one.
3. **XML parse errors are under-reported.** xml5ever recovers from most
   malformedness, so `parsererror` only appears when no root element results.
4. **`XMLSerializer` does not invent namespace prefixes.** An element is emitted
   under the prefix it already carries plus the `xmlns` declaration its
   namespace needs; the spec's prefix-generation path is not implemented.
5. **`createElement` always lowercases**, including on an XML document, where
   the DOM keeps the given case. `createElementNS` is exact, and that is what
   `DOMParser`'s XML documents are built from.
6. **`document.doctype` has no quirks-mode consequence in the scripted tier.**
   The arena's doctype is a node, not a mode: quirks still comes from the static
   parse.
7. **`outerHTML`'s setter uses the parent's local name as the fragment-parsing
   context** rather than the full HTML fragment-parsing algorithm with a
   context element, so a `<td>` replaced inside a `<tr>` parses as a `tr`
   context rather than through the table insertion modes.
8. **`normalize` does not run the spec's explicit range steps**; it composes
   `appendData` + `removeChild`, whose funnel steps give the same result for
   every case the corpus exercises but are not the literal algorithm.
9. **`responseXML` has no census receipt.** Server mode is the XHR plan's next
   proof and would measure it.
10. **`DOMParser` and `XMLSerializer` carry no class string.** They are defined
    after `installShapeInterfaces` runs and are not in `dom.idl`, so
    `Object.prototype.toString.call(new DOMParser())` is `[object Object]`.

## Gates

All green on 2026-09-07, genet `ddee26ef559` (and re-verified over
`d8e7ee8bf0b`, below) plus this lane's working tree.

- `cargo test` for every touched crate: `layout-dom-api`, `genet-static-dom`,
  `genet-scripted-dom`, `script-runtime-api` (227 tests, including the new
  16-test `tests/dom_node_model.rs` across both backends), `genet-livery`,
  `genet-scripted` — 0 failed.
- `cargo check --workspace --features genet-wpt/netfetch` — clean.
- `cargo clippy` on the touched crates, `--all-targets` — no warning in any
  touched file (two pre-existing `cmp_owned` warnings in
  `genet-scripted-dom/lib.rs` fixed in passing).
- `cargo fmt` on the touched crates.
- `cargo run -p genet-idl-interface-table -- --check` — `is current`.
- Both checked reftest baselines (`css/mediaqueries`, `css/css-position`,
  Boa/Livery) — `unexpected=0`.
- `cargo run -p ortet -- --url ports/ortet/examples/article.html --frames 3
  --artifact C:/t/lane9-ortet.png` — 3 frames, digest `0x6377ba8a6bf4dbc9`.
- `pre` and `post` census over `dom`, `dom/ranges`, `dom/nodes`, `html/dom`,
  `html/semantics/interfaces.html`, `custom-elements`, `xhr`, `selection` and
  `html/syntax`, both from this lane's own runner builds. Maps, driver, diff and
  digests under
  `Code/testing/genet/wpt-ledger/2026-09-07_dom_node_model/`.

## Progress

- **2026-09-07** — Lane landed. `pre` runner built from `ddee26ef559` before the
  first edit and its census taken; the six pieces implemented in order; `post`
  runner rebuilt from the final tree and its census taken twice with identical
  results. 151 forward file movements, 14 backward-to-`error` movements both
  attributed and neither from a passing status, zero pass-to-fail, +82,944
  subtest passes over nine directories. `dom/nodes/Node-baseURI.html` repinned
  in `ports/genet-wpt/expectations/testharness/dom_boa.json` from 4/9 `fail` to
  9/9 `pass`; nothing else repinned.

## A concurrent commit

Another session committed `d8e7ee8bf0b` ("crypto: take randomness from the
operating system, not an in-tree cipher") while this lane was running. It swept
in this lane's uncommitted `components/script-runtime-api/Cargo.toml` edit (the
`genet-static-dom` promotion from dev-dependency to dependency, which
`DOMParser` needs) under its own message. Nothing else of this lane's work is
committed. Both census runners predate that commit, so the `pre`/`post`
comparison is internally controlled; none of the nine measured directories
exercises `crypto`. The gates above were re-run against the tree with
`d8e7ee8bf0b` in it and are green.

## A decision that is Mark's

`ports/genet-wpt/expectations/testharness/dom_boa.json` and
`dom_nodes_boa.json` are checked baselines
(`support/wpt/check-testharness-baselines.ps1`) last pinned on 2026-08-24. They
were already stale before this lane — the MutationObserver and Selection/Range
lanes moved files in `dom` without repinning — and this lane moves roughly two
hundred more. Repinning them wholesale is a separate, deliberate act; this lane
touched exactly the one entry it was asked to.
