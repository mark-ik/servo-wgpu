# Shadow DOM

**Status: landed 2026-09-07/08.** Promoted from the Shadow DOM section of
[`2026-09-07_deferred_web_platform_lanes_scoping.md`](2026-09-07_deferred_web_platform_lanes_scoping.md#shadow-dom)
under Mark's three decisions for the lane (below). Genet commit at start:
`14e7b0200c8`. Residuals are named in **Residuals**; the regression manifest is
in **Regression manifest**.

## Mark's decisions this lane executes

1. **`<template>` contents live in a separate inert `Document`, per spec** —
   one per owning document, shared by all its templates.
2. **Both DOMs learn the flat tree**, so script-free pages and Fleece see
   declarative shadow roots correctly.
3. **Declarative shadow DOM is recognized by a post-parse pass over both DOMs**,
   not by modifying html5ever.

## Done-conditions, and where each one stands

The five items are the scoping document's, in its order. Each is either met with
a named receipt or carries a named residual; none is left implicit.

| # | Done-condition | State |
|---|---|---|
| 1 | Shadow-root node kind in both DOMs, host link and init dictionary, per-host slot assignment maintained on mutation (named, default, `slotchange`, manual `assign`), `flat_children` on both DOMs, Livery's style walk / box construction / hit testing on `flat_children`, layout of slotted and fallback content | **met** — `genet-livery --test shadow_flat_tree` (script-free) and `script-runtime-api --test shadow_dom` (both engines); `shadow-dom` 6 → 45 all-pass |
| 2 | Style scoping: shadow sheets inside, document sheets out, inheritance through, `:host`, `:host()`, `:host-context()`, `::slotted()`, `::part()` with `exportparts`, `adoptedStyleSheets` | **met except two named residuals** — `:host-context()` (no parse arm in `selectors` 0.39) and `adoptedStyleSheets` (no constructable `CSSStyleSheet` to adopt) |
| 3 | Event retargeting, `composedPath()`, `composed` on Event init, focus delegation, the opaque-root rule extended | **met except focus delegation**, which is a named residual; the opaque-root extension is one line in `tree_root` |
| 4 | `attachShadow` and its allowed-elements rule, `shadowRoot`, `assignedSlot`, `assignedNodes` / `assignedElements` with `flatten`, `getRootNode` with `composed`, `closest` and `querySelector*` scoping, `<template>.content` in its inert document, `cloneNode` of hosts and clonable roots, `getHTML` / `innerHTML` serialization, the declarative pass with `shadowrootdelegatesfocus` and friends, interfaces through the generated table | **met**; `Element.part` as a live `DOMTokenList` is a named residual |
| 5 | Fleece: decide and record whether slotted text is extracted in flat-tree order | **met** — decided **DOM order**, with the consequences written out in §5 |

Two acceptance requirements could **not** be satisfied in this checkout and are
recorded as such rather than reported as met: the reftest lane has no shadow-DOM
test family here, and two of the named census directories do not exist. Both are
under **Gates and receipts**.

## 1. The flat tree across both DOMs

### The shape

A shadow root is a **real node in the same store** with kind
`NodeKind::ShadowRoot` and **no parent**. It hangs off its host through two
side maps, not through the host's `children` vector. That single choice is what
keeps the rest of the engine unchanged: `dom_children`, `childNodes`,
`querySelector`, serialization, extraction and every existing walk stay exactly
what they were, and a shadow tree is invisible to all of them unless they ask.
The flat tree is the one place the link is spliced back in.

`LayoutDom` grew the seam the scoping document said already existed as a
default, plus the questions a shadow-aware consumer has to be able to ask:

| Method | Answers |
|---|---|
| `flat_children(id)` | host → its root's children; slot → its assignment, else its own children (fallback); anything else → `dom_children` |
| `has_shadow_trees()` | the cheap guard every consumer asks first |
| `shadow_root(host)` / `shadow_host(root)` / `shadow_init(host)` | the host link and the init dictionary |
| `assigned_slot(node)` / `assigned_nodes(slot)` | the assignment table, both directions |
| `shadow_roots()` | enumeration, for the style pass's scope table |
| `node_tree_root` / `composed_tree_root` / `containing_shadow_root` | `getRootNode()`, `getRootNode({composed})`, and the selector adapter's containing host |
| `template_contents(id)` | the inert fragment, which no tree walk reaches |

`walk_flat_subtree` is the rendering twin of `walk_subtree`.

`NodeKind::ShadowRoot` is a `DocumentFragment` in the DOM's own hierarchy, so
`NodeKind::is_document_fragment()` is provided and every consumer that treats a
fragment as a transparent container treats a shadow root the same
(`nodeType` 11, `nodeName` `#document-fragment`, serializes as its children).

### Scripted DOM (`components/genet-scripted-dom/shadow.rs`)

Four side maps on `ScriptedDom`, all empty for a document that never attaches a
root: `shadow_roots` (root key → host, init, per-root slot table, `declarative`
flag, manual-assignment list), `shadow_hosts` (host key → root),
`assigned_slots` (slottable → slot) and `slot_assignments` (slot → nodes).

Assignment is recomputed **eagerly, per affected shadow root**, at the mutation
that could change it. Every entry point exits on `shadow_hosts.is_empty()`
first, so a document with no shadow tree pays one `HashMap::is_empty` per
structural mutation and nothing else. A recompute is bounded by one host's
children plus that tree's slots — it is not a document walk, and the slot search
stops at a nested shadow host, whose slots belong to its own tree. This follows
the working principle that a per-mutation walk over a script-created population
is quadratic: the trigger is a hash lookup, and the work is proportional to the
one component that changed.

Attribute writes cost one namespace-and-local comparison: only `slot` on a
light-DOM child and `name` on a slot can change assignment.

`slot_changes` records the slots whose assigned list actually moved; the
bootstrap drains it at the mutation funnel and fires one `slotchange` each.

Two existing arena contracts were extended rather than duplicated:

- **`collect`** marks through the host ↔ root edge and the template-contents
  edge. Without them a live host would sweep its own shadow tree, and a pinned
  node inside a shadow tree would not keep its host alive.
- **`tree_root`** — the reflector-identity policy's opaque root — continues
  through the **host** when it reaches a parentless shadow root. This is the
  whole of what
  [the reflector identity plan](2026-09-07_reflector_identity_plan.md) needed:
  a shadow tree hangs off its host, so every wrapper in it shares the host's
  opaque root and one connected component stays one liveness group.
  `is_connected_node` is then right for shadow-tree nodes for free.

### Static DOM (`components/genet-static-dom/shadow.rs`)

The declarative post-parse pass. `StaticDocument::parse` (and `parse_xml`) walk
the finished tree, convert every `<template shadowrootmode>` whose parent may
host a shadow tree into a real shadow root, move the template's contents into
it, drop the template, then compute slot assignment once. A static document
never mutates afterwards, so the tables are built once and read forever.

`parse_fragment_source` is the same parse **without** the pass: HTML creates a
declarative shadow root from the document parser and from `setHTMLUnsafe`, never
from `innerHTML`, and the scripted `innerHTML` setter parses through it.

Doing the recognition here rather than in html5ever (decision 3) means one set
of rules serves both tiers: the scripted arena inherits realized roots through
the tree copy rather than re-deciding.

### Livery

`flat_children` replaced `dom_children` at the **rendering** traversals only,
never at the DOM-order ones the scoping document warned about:

| Switched to `flat_children` | Left on `dom_children` |
|---|---|
| box construction (`box_tree.rs`) | `text_sources_in_dom_order` (selection is node-tree ordered) |
| hit testing (`layout/hit_testing.rs`) | canvas-background source selection (root element and its `<body>` are document-tree facts) |
| paint's visibility and inline-decoration predicates | `:nth-child` ordinals (see below) |
| text collection and inline grouping (`text.rs`) | `SelectorTree`'s sibling neighbourhood |
| the cascade descent, container-query and relative-length subtree walks | |
| style-plane subtree removal on invalidation | |

The cascade's child step splits the two trees deliberately. **Descent follows
the flat tree**, because inheritance does: a slotted element inherits from its
slot's flat-tree parent. **Ordinals follow the node tree**, because
`:nth-child` is defined over an element's parent in its own tree and slotting
must not renumber a light-DOM child. `cascade_children` computes one
`child_tree_counts` per distinct node-tree parent rather than one per child, so
a slot's whole assignment costs one walk; on a document with no shadow tree it
is the old function verbatim.

`slot { display: contents }` was added to the UA sheet: a slot is a transparent
box in the flat tree and never generates one of its own.

## 2. Style scoping

### The boundary

A shadow tree's stylesheets apply inside the root; the document's author sheets
do not cross in; inheritance follows the flat tree. The boundary is decided
**per rule**, because a flattened cascade has already lost the sheet a rule came
from. `StyleSet` therefore records, alongside `rules`, the author-sheet index
each flattened rule came from (`rule_sheets`, `None` for UA). `tree_scopes()`
turns that into a per-rule `(shadow root opaque id, host)` by resolving each
sheet's `owner_node` through a one-pass index of the shadow trees. A document
with no shadow tree builds an empty table and the matching loop is untouched.

`genet-document-resources` descends into shadow roots when collecting
stylesheets — a shadow tree's `<style>` is not among its host's children, so
without that descent a shadow tree could never carry a stylesheet at all. The
sheets it yields are scoped at match time by their owner node; collecting them
does not make them document-wide.

At match time: a rule matches normally only inside its own tree scope. UA rules
are never scoped (`slot { display: contents }` has to reach inside every shadow
tree). Across a boundary only the three constructs that are *defined* to cross
are offered.

### `:host`, `:host()`, `::slotted()`, `::part()`

The `selectors` 0.39 crate already implements all four against
`MatchingContext::shadow_host`, `Element::assigned_slot`, `Element::is_part` and
`Element::imported_part`. Livery only had to say yes: `parse_host`,
`parse_slotted` and `parse_part` now return `true`, and
`genet-livery/src/dom.rs` answers `containing_shadow_host`,
`parent_node_is_shadow_root`, `assigned_slot`, `is_part` (the `part` attribute's
token list) and `imported_part` (the `exportparts` mapping, `inner:outer` or a
bare name).

`SelectorList` classifies each selector's **reach** once at parse time —
`Inner`, `Host`, `Slotted`, `Part` — from the crate's own `is_slotted` /
`is_part` flags and a `Component::is_host` scan of the rightmost compound. The
cascade's per-element loop is then a comparison, not a selector inspection:
across a boundary, only a non-`Inner` reach is offered to `matches_selector`, and
the crate's own matching decides the rest with `current_host` set to the
*stylesheet's* scope host.

`SelectorTree` was taught to descend into shadow roots when minting selector
identities and to climb host links when closing over a restyle root's ancestors;
without that, nothing inside a shadow tree would have an identity and every
selector would silently skip the subtree.

## 3. Events and roots

`Event` gained `composed` on its init dictionary. The bootstrap's dispatch
builds the **shadow-including** ancestor chain: at the top of a shadow tree it
continues through the host, but only for a `composed` event — a non-composed one
stops at the shadow root, which is what makes a shadow tree's events private.

Each path entry carries the target its listeners must see (**retargeting**).
Inside the shadow tree that is the real target; once the path crosses out
through a host, every node at or above it sees the *host*, because the outer
tree may not learn about nodes it cannot reach. `event.target` is set per firing
node and restored after dispatch.

`composedPath()` returns the recorded path minus what the currently-firing node
may not see: an **open** shadow tree is visible to everyone on the path, a
**closed** one only from inside itself.

`getRootNode(options)` honours `composed`. `rootDocument` / `ownerDocumentOf`
now cross the shadow boundary through the host, and fall back to the tree root's
recorded owner — which is how everything inside a `<template>`'s contents
reports the shared inert template document rather than the page's.

Focus delegation is a **named residual**: `delegatesFocus` is stored and
reported, and `ShadowRoot.activeElement` returns `null` rather than a wrong
answer dressed up.

## 4. The DOM surface

Natives in `components/script-runtime-api/dom/shadow.rs`; the DOM surface over
them at the end of `dom/bootstrap.js`. **Mode policy lives in the bootstrap, not
the arena**: layout, serialization and the reflector policy all need a closed
root, so the arena answers honestly and the DOM surface withholds.

Landed: `attachShadow` with the allowed-elements rule (HTML's list plus any
name a tree can tell is a valid custom element name) and the declarative-root
reuse rule; `ShadowRoot` with `mode` / `delegatesFocus` / `clonable` /
`serializable` / `slotAssignment` / `host` / `innerHTML`; `Element.shadowRoot`
withheld for a closed root; `Node.assignedSlot` (on `Node`, because a text node
is a slottable too); `HTMLSlotElement.assignedNodes` / `assignedElements` with
`flatten` (fallback content stands in for an empty slot, and a nested slot is
replaced by what it would itself show) and `assign()`; `slotchange` at the
mutation funnel; `getRootNode` with `composed`; `<template>.content` in the
shared inert document; `cloneNode` of a host with a **clonable** root, always
deeply, even for `cloneNode(false)`; `getHTML({serializableShadowRoots})` and
`setHTMLUnsafe`; and `__realizeDeclarativeShadow` for the `setHTMLUnsafe` path.

`querySelector` / `querySelectorAll` / `getElementById` scope to a shadow root
already, because the root is a real node and those natives take a scope node.

**`ParentNode`'s element views** (`children`, `firstElementChild`,
`lastElementChild`, `childElementCount`) were defined on `Element.prototype`
alone, so a `DocumentFragment` — and therefore a `ShadowRoot` — reported
`undefined` for all four. They now reach `DocumentFragment.prototype`.
`Document`'s own copy of that gap is left alone: it is separate, and this lane
measures what it changes.

### The interface table

`HTMLTemplateElement` carried `tags: &[]`, because WPT's
`html/semantics/interfaces.js` — a *test* of the element-interface mapping, not
a complete one — omits `template`. Without a tag row a `<template>` wrapped as
`HTMLElement` and `HTMLTemplateElement.prototype.content` reached no element.
The generator grew one documented addition, `TAG_ADDITIONS`, and the table was
regenerated; the drift test passes and the counts are unchanged (72 interfaces /
338 reflected attributes / 41 shape interfaces). `ShadowRoot` and
`HTMLSlotElement` were already in the tables.

## 5. Fleece receipt

**Decision recorded: Fleece extracts in DOM order, not flat-tree order.**

Fleece's extraction walks `dom_children` throughout, and this lane deliberately
did not change that. Under the
[preservation contract](2026-09-05_fleece_preservation_contract_plan.md), an
extracted passage's source anchors must name where the text *is in the source
document*, and slotted text is authored in the light DOM. A flat-tree walk would
emit the same characters under anchors that point at the shadow tree, which is
not where an author or a citation would find them.

The consequences, stated so they are not discovered later:

- Text authored in the light DOM of a shadow host is extracted, once, at its
  authored position — the same position it had before this lane existed.
- Text inside a shadow tree (a component's own chrome, and a slot's **fallback**
  content) is **not** extracted, because no `dom_children` walk reaches a shadow
  root. For a declarative component that is usually right: the shadow tree is
  presentation the component supplied, not the document's prose.
- Rendered-text order can therefore differ from extraction order when slots
  reorder the light DOM. Any future rendered-text traversal must say so
  explicitly, per the preservation contract's rule that a rendered-text
  traversal is named rather than assumed.

No Fleece code changed; the receipt is that its 1,000-plus lines of
`dom_children` walks are unaffected because a shadow root has no parent and no
walk from the document reaches one.

## Findings

- **A shadow root with no parent costs the rest of the engine nothing.** Every
  existing walk is a `dom_children` descent from the document; a parentless
  shadow root is unreachable from it by construction. Inertness and
  encapsulation are then properties of the *shape*, not flags each consumer has
  to remember to check — which is also why `<template>` contents use the same
  trick.
- **WPT's `interfaces.js` is a test, not a mapping.** It omits `template`
  entirely, and a table generated from it alone silently loses
  `HTMLTemplateElement`. A generator reading a conformance test as its source of
  truth needs a named place for what the test does not cover; the 41 reasoned
  overrides had one for attributes and none for tags.
- **A `<template>`'s contents are not its children, in three separate copiers.**
  `clone_into` (static → scripted), `copy_fragment_node` (`innerHTML`) and
  `cloneNodeInto` (`cloneNode`) each walked children only, so a parsed template
  reached the scripted tier empty. The same is true of a shadow root. Each
  copier had to be told; there is no traversal that would have caught all three,
  because "not a child" is exactly the point.
- **A named property that script has replaced must not be deleted on refresh.**
  `__refreshNamedProperties` deleted every name it had installed and
  reinstalled from the document. Its own comment already recorded that an
  `id="test"` element must not shadow testharness's `test()`, and its setter
  handled the shadowing correctly — but the *next* refresh deleted the
  script's data property and put the element accessor back. Nothing triggered a
  mid-file refresh until `setHTMLUnsafe` did, and then
  `shadow-dom/shadow-root-clonable.html` failed with `not a callable function`
  from calling `test(...)`. A refresh now removes only a name that is still the
  accessor it installed.
- **The `selectors` crate had the shadow work done already.** `:host`,
  `:host()`, `::slotted()` and `::part()` are all implemented in 0.39 behind
  three `Parser` flags and five `Element` methods. What the lane actually had to
  build was the *scope boundary* around them — which rules are offered to which
  element — because that is engine policy the crate deliberately leaves out.
  `:host-context()` is the exception: 0.39 has no parse arm for it.
- **The reflector-identity policy needed one line.** `tree_root` continuing
  through the host is the entire integration: `is_connected_node`, the opaque
  root and the pin/collect grouping all read from it. That is the payoff of the
  earlier lane expressing liveness as a *relation* rather than a set of roots.

## Gates and receipts

Runner digests (SHA-256), both built in `C:/t/laneS-target`:

| Runner | Digest |
|---|---|
| `pre` (built from `14e7b0200c8`, before the first edit) | `5460c127cb88cea570b612d066781a88f28e072898113099036b1e7778a4ee7f` |
| `post` | `183a3c7f1f8a3aa1da106d42900451af33b8740daeee6833d7362ef8267347f3` |

Both runs: `genet-wpt testharness <dir> --engine boa --renderer livery --jobs 8
--timeout 240`, disk mode, on the same tree of vendored WPT. Reftest lane:
`genet-wpt reftest <dir> --engine boa --renderer livery --timeout 240`. Raw maps,
logs, the run scripts and the diff under
`Code/testing/genet/wpt-ledger/2026-09-07_shadow_dom/`.

### Before / after, testharness lane

| Directory | files all-pass | errored | subtests passed |
|---|---|---|---|
| `shadow-dom` | 6 → **45** | 36 → **13** | 24 / 8,694 → **1,512 / 8,804** |
| `custom-elements` | 7 → **8** | 24 → **23** | 2,126 / 3,832 → **2,149 / 3,837** |
| `dom` | 218 → **222** | 27 → **23** | 46,361 / 57,030 → **46,370 / 57,171** |
| `html/semantics/scripting-1/the-template-element` | 0 → **2** | 0 → 0 | 273 / 658 → **514 / 658** |

Aggregate movement: **+1,761 subtest passes**, 47 files `fail → pass`, 29
`error → fail`, 1 `fail → error`, 1 `pass → fail`.

### Before / after, reftest lane

| Directory | pre | post |
|---|---|---|
| `shadow-dom` | 0 passed, 0 failed, 314 skipped | 0 passed, 0 failed, 314 skipped |
| `custom-elements` | 0 passed, 0 failed, 187 skipped | 0 passed, 0 failed, 187 skipped |

**The reftest lane has nothing to measure in these two directories**: every
file in both is a testharness test, so the reftest runner skips all 501. The
scoping document's "reftests prove slot geometry and style changes after
reassignment" done-condition therefore has **no test family in this checkout to
satisfy it**, and stays open as a residual rather than being reported as met.

### Two named census directories do not exist here

`css/css-scoping` and `html/semantics/scripting-1/the-slot-element` are **absent
from this vendored WPT tree** (`tests/wpt/tests/css/` has 92 entries and no
`css-scoping`; `scripting-1/` has only `the-noscript-element`,
`the-script-element` and `the-template-element`). They were censused as
"MISSING" rather than silently dropped. Slot coverage in this checkout lives
inside `shadow-dom/slots*`, which is measured above; `:host` / `::slotted` /
`::part` coverage that would live in `css/css-scoping` has no home here, so the
style-scoping work is proved by the lane's own Livery suite instead — see
**Regression manifest**.

### Other gates

- `cargo test` green for every crate touched: `layout-dom-api`,
  `genet-static-dom`, `genet-scripted-dom`, `livery`, `genet-livery`,
  `genet-document-resources`, `script-runtime-api`, `genet-idl-interface-table`.
  Scripted cases run on **both** Boa and Nova.
- One **pre-existing** failure, unrelated and untouched:
  `livery --test catalog_contract::generated_property_names_round_trip` asserts
  every property's `source_url` starts with `https://www.w3.org/`, and
  `clip-path`'s is `https://drafts.fxtf.org/css-masking-1/#propdef-clip-path`.
  Both `properties.toml` and `build.rs` are byte-identical to `HEAD`, and the
  line is present in `git show HEAD:components/livery/properties.toml`.
- `cargo clippy` clean on every file this lane touched; `cargo fmt` applied.
- The IDL interface-table drift test passes against the regenerated table.
- `cargo check --workspace --features genet-wpt/netfetch` clean.

### Repins

Three baselines repinned forward, by subset, every movement an improvement.

**`ports/genet-wpt/expectations/testharness/dom_boa.json`** — eleven unexpected
entries:

| Test | Movement |
|---|---|
| `dom/events/Event-dispatch-single-activation-behavior.html` | error → fail 0/132 |
| `dom/events/pointer-event-document-move.html` | error → fail 0/1 |
| `dom/events/relatedTarget.window.html` | error → fail 0/6 |
| `dom/events/shadow-relatedTarget.html` | error → fail 0/2 |
| `dom/nodes/DocumentFragment-getElementById.html` | fail 3/5 → 4/5 |
| `dom/nodes/Node-cloneNode.html` | fail 133/135 → 134/135 |
| `dom/nodes/ParentNode-replaceChildren.html` | fail 13/29 → 14/29 |
| `dom/nodes/moveBefore/moveBefore-shadow-root.html` | fail 0/1 → **pass** |
| `dom/nodes/rootNode.html` | fail 4/5 → **pass** |
| `dom/nodes/svg-template-querySelector.html` | fail 0/3 → **pass** |
| `dom/ranges/Range-intersectsNode-shadow.html` | fail 0/1 → **pass** |

**`ports/genet-wpt/expectations/testharness/dom_nodes_boa.json`** — the six of
those eleven that also fall inside the narrower `dom/nodes` subset
(`DocumentFragment-getElementById`, `Node-cloneNode`, `ParentNode-replaceChildren`,
`moveBefore/moveBefore-shadow-root`, `rootNode`, `svg-template-querySelector`).

**`ports/genet-wpt/expectations/testharness/css_position_boa.json`** — two, both
files that previously aborted before their first subtest:

| Test | Movement |
|---|---|
| `css/css-position/animations/position-interpolation.html` | error → fail 44/97 |
| `css/css-position/position-absolute-fit-content-auto-margin.html` | error → fail 0/45 |

After the repins the full guard reports `WPT testharness baselines:
unexpected=0` across all nine checked slices plus the five Livery math files, and
`WPT reftest baselines: unexpected=0` for both checked reftest slices
(`css/mediaqueries`, `css/css-position`).

**Ortet headed receipt.** `cargo run -p ortet -- --url
ports/ortet/examples/article.html --frames 3 --artifact C:/t/laneS-ortet.png`:
engine `genet.livery`, backend livery, 3 frames at 960x640, digest
`0x6377ba8a6bf4dbc9`. The whole frame was examined, not only the changed
feature: heading, italic lede, link run and separator, section heading, body
text and the gradient swatch all render as before. The fixture has no shadow
tree, so this is the control that the `flat_children` switch and the new UA
`slot` rule cost a shadow-free document nothing.

## Regression manifest

The named suites that must keep passing, and what each one is holding:

| Suite | Holds |
|---|---|
| `script-runtime-api --test shadow_dom` (14 cases, Boa + Nova) | `attachShadow` and the allowed-host rule, open/closed visibility, named and manual assignment, `slotchange`, `assignedNodes({flatten})`, retargeting and `composedPath()` through open and closed roots, `getRootNode({composed})`, the declarative pass and the `innerHTML` / `setHTMLUnsafe` split, `getHTML` round trip, the shared inert template document, `cloneNode` of clonable roots |
| `genet-livery --test shadow_flat_tree` (5 cases) | the whole lane **script-free**: the post-parse pass, `flat_children` in all three cases, the tree-scope boundary in both directions, `:host`, `::slotted()`, inheritance through the flat tree, and that a filled slot's fallback content is not rendered |
| `script-runtime-api --test mutation_observer` | the mutation funnel the slot bookkeeping hangs off |
| `script-runtime-api --test selection_range` | the same funnel's live-range steps |
| `script-runtime-api --test dom_node_model`, `--test tagname_window_globals` | the node-kind and interface-shape contracts a new `NodeKind` variant could break |
| `script-runtime-api` IDL drift test | the regenerated interface table |
| `genet-livery` suite (248 unit + 30 integration targets) | the `flat_children` switch causing no movement on shadow-free documents |
| `check-testharness-baselines.ps1`, `check-reftest-baselines.ps1` | `unexpected=0` on the checked slices |

### Explained movements

**`shadow-dom/declarative/declarative-with-disabled-shadow.html`, `pass → fail`
(1 subtest).** The only pass-to-fail in the lane. It passed *because the feature
did not exist*: with no declarative pass, the template stayed in the tree and no
shadow root appeared, which is exactly what the test asserts. Now the pass
realizes the root, because a custom element's `disabledFeatures: ['shadow']`
lives in the script tier's custom-element registry and **the registry is empty
when the post-parse pass runs** — Genet parses the whole document and then runs
scripts, where HTML interleaves them. Deferring realization for every hyphenated
name until the registry is known would break the common case (a declarative root
on a custom element) to fix the rare one, so it is not done. **This is a
decision for Mark**, recorded in Residuals.

**`shadow-dom/declarative/innerhtml-on-ordinary-template.html`, `fail → error`
(0/1 before), the lane's only `fail → error`.** Same structural cause. The test attaches an imperative root from
a `MutationObserver` *during parsing*, after which the following
`<template shadowrootmode>` must stay an ordinary template. Genet's pass runs
after parsing, sees no imperative root, and consumes the template — so the
window named property `ordinarytemplate` the test then references does not
exist, and the reference throws before any subtest runs. No pass was lost.

**29 `error → fail`.** Files that previously aborted before their first subtest
now run. This is the "a directory's census can be floored" pattern: the shared
`shadow-dom/resources/shadow-dom.js` helper builds every test tree through
`template.content` and `attachShadow`, so with neither implemented the helper
threw and the whole file errored.

**The four `dom/events/*` error → fail repins** are the same thing in the `dom`
directory: those files reference `attachShadow` or `composedPath` during setup.

## Residuals

Named, not silently deferred:

1. **`:host-context()`** — `selectors` 0.39 has no parse arm for it (only a
   comment referencing it). Supporting it needs either an upstream addition or a
   local parse of a selector-argument pseudo-class outside the crate's
   `NonTSPseudoClass` shape.
2. **`adoptedStyleSheets`** — the CSSOM has no constructable `CSSStyleSheet`, so
   there is nothing to adopt. Blocked on constructable sheets, not on scoping:
   the per-rule scope table would carry an adopted sheet unchanged.
3. **Focus delegation.** `delegatesFocus` is stored, reported and serialized;
   focusing a host does not move focus, and `ShadowRoot.activeElement` returns
   `null`. Focus is not tracked per tree scope anywhere in the engine yet.
4. **`Element.part` as a live `DOMTokenList`.** The cascade reads the `part` and
   `exportparts` attributes directly, so `::part()` matching works; the IDL
   attribute is not reflected as a token list.
5. **Declarative attachment does not consult the custom-element registry.** See
   the two explained movements above. It needs parser/script interleaving, which
   Genet does not have. **Mark's call.**
6. **The reftest lane has no shadow-DOM test family in this checkout.** All 501
   files in `shadow-dom` and `custom-elements` are testharness tests. Slot
   geometry and post-reassignment style changes are proved by
   `genet-livery --test shadow_flat_tree` over the static DOM instead, which is
   a layout-level rather than a pixel-level receipt.
7. **`css/css-scoping` and `the-slot-element` are absent** from the vendored WPT
   tree; the style-scoping and slot-element families cannot be censused here.
8. **`ShadowRoot.styleSheets` / `activeElement` / `pointerLockElement`** and the
   rest of `DocumentOrShadowRoot` beyond what is listed above.
9. **Slot assignment ignores `slot` on a non-child descendant** — correct per
   spec (only a host's children are slottables), but worth naming because the
   fallback-content rule interacts with it: a slot nested inside another slot's
   fallback content is reached by `assignedNodes({flatten})` and by the flat
   tree, and `shadow-dom/slots-fallback.html` still fails one of its thirteen
   assertions on exactly that case (12/13).
10. **No headed receipt.** Ortet renders the lane's UA-sheet and flat-tree
    changes (see below), but no Ortet fixture exercises a shadow tree; per
    [Ortet O5](2026-09-03_ortet_founding_plan.md) the headed gate for scripted
    behavior stays open.

## Progress

**2026-09-07/08 — landed.** Arena and static shadow roots, slot assignment,
`flat_children` across both DOMs and Livery's rendering traversals, the
tree-scope boundary with `:host` / `:host()` / `::slotted()` / `::part()`, event
retargeting and `composedPath()`, the DOM surface, the declarative post-parse
pass, `<template>.content` in its shared inert document, and the
`HTMLTemplateElement` tag row. +1,761 subtest passes over four directories, one
explained pass-to-fail, one explained fail-to-error, one baseline repinned
forward. Receipts under
`Code/testing/genet/wpt-ledger/2026-09-07_shadow_dom/`.
