# Tag-name casing per document type, and the window global's own properties

**Status: landed 2026-09-07.** Two bounded corrections in the scripted tier's
host surface, both of them shape questions the earlier lanes deferred rather
than answered: which names fold to uppercase and when, and what kind of
property `window`, `document` and `self` actually are.

Owns `components/script-runtime-api/dom/`, the `install_host_surface` block of
`components/script-runtime-api/lib.rs`, and the worker scope's global setup.

## Why these two

The IDL interface-table plan closed with an explicit residual — its F6, "the
runtime's `__tagName` uppercases, so a case-preserving
`createElementNS(…, 'foo-BAR')` cannot be told from `foo-bar` at this seam" —
and named it "a `genet-scripted-dom` name question, not a table question".
Three `html/semantics/interfaces.html` subtests were left failing on it, and
`dom/nodes/Element-tagName.html` was pinned at 3 of 6.

The Worker plan closed with a second one. `install_host_surface` opened with
`var self = globalThis; var window = globalThis;`; a global `var` is
non-configurable, so the worker scope could not delete `window`. That lane
changed the line to two plain assignments. Neither shape is the standard's:
plain assignment makes `window` writable, configurable and deletable, which is
the opposite of `[LegacyUnforgeable]`.

## What the standards say

**Tag names.** `Element.tagName` returns the element's *HTML-uppercased
qualified name*: the qualified name (`prefix:local`), ASCII-uppercased **only**
when the element is in the HTML namespace *and its node document is an HTML
document*. `Node.nodeName` for an element is the same string. `localName` never
folds. Both conditions matter and both are dynamic: adoption into another
document changes the answer for a node that has not otherwise been touched.

`Document.createElement` ASCII-lowercases its argument only for an HTML
document; `createElementNS` never folds. The element interface is selected from
the **local name**, case-sensitively, so `createElementNS(html, 'DIV')` is an
`HTMLUnknownElement`, not an `HTMLDivElement`.

A valid custom element name is `PotentialCustomElementName`:
`[a-z] PCENChar* '-' PCENChar*`. The first code point must be ASCII lowercase
and no ASCII uppercase may appear anywhere — but non-ASCII letters are ordinary
`PCENChar`s. That is what makes `foo-BAR` and the a-with-ring name invalid (and
so `HTMLUnknownElement`) while `foo-bar` is valid (and so `HTMLElement`), and
what makes ASCII-only folding load-bearing rather than pedantic.

**The globals.** On `Window`, `window` and `document` are `[LegacyUnforgeable]`
readonly attributes: accessor properties with a getter, no setter,
`enumerable: true` and **`configurable: false`**, defined on the global object
itself. `self` is `[Replaceable]`: an accessor whose setter redefines the
property as an ordinary writable data property, `configurable: true`. In
`DedicatedWorkerGlobalScope`, `self` is an ordinary readonly attribute
(accessor, getter only, `configurable: true`) and there is no `window` at all.

## What changed

### The tag-name seam moved from the arena to the bootstrap

The uppercasing lived in Rust, in two natives that both re-derived it:
`__tagName` (`dom/tree.rs`) and `__nodeName` (`dom/query_traverse.rs`). Neither
could know the node's current node document, because the node document is a JS
notion in this tier — `ownerDocument` is resolved by the bootstrap's
`ownerDocumentOf`, from the tree root or an `ownerDocuments` `WeakMap`, and the
arena has no owner pointer at all.

So the fold moved up, and the natives now only report what the arena stores:

- `__tagName` became `__qualifiedName`, returning the **case-preserved**
  qualified name. The name changed with the semantics deliberately: nothing in
  the tree still asks for a folded name from Rust.
- `__nodeName` returns an element's case-preserved qualified name; the
  non-element kinds (`#text`, `#comment`, a doctype's name, a PI's target) are
  unchanged.
- A shared `qualified_of(&QualName)` in `dom/mod.rs` replaces the two copies of
  the `prefix:local` join.

The bootstrap gained `asciiLower` / `asciiUpper` (ASCII-only, per the DOM's own
definition), `isHtmlDocument(doc)` over the `__isHtml` flag the node-model lane
already put on document wrappers, and one `elementQualifiedName(el)` that both
`Element.prototype.tagName` and `Node.prototype.nodeName` read. It folds per
access rather than caching, because adoption must change the answer.

### Element interface selection became case-sensitive

`wrapNode` chose the prototype from the *uppercased* tag name against a table
keyed by uppercase. It now reads `__localName(ref)` and matches the table's own
lowercase keys directly, so `DIV` no longer collides with `div`. Two
consequences fell out of that:

- `elementSubclassProto` became `Object.create(null)`. Keyed by lowercase local
  names, a `<constructor>` element would otherwise have reached
  `Object.prototype.constructor` and been given a function as its prototype.
- Custom element definitions are keyed by local name, case-sensitively
  (`autonomousCustomElementDefinitions`, `customElementKey`), so a `foo-bar`
  definition cannot claim a `foo-BAR` element. A valid custom element name
  contains no ASCII uppercase, so the exact local name is a sound key.

`isValidCustomElementName` gained the two missing rules: first code point ASCII
lowercase, and no ASCII uppercase anywhere. It previously rejected on
`name !== name.toLowerCase()`, which is Unicode folding — it accepted
the a-with-ring name (wrong) and would reject a spec-legal `b-{a-with-ring}`.

### `importNode`, and the clone that made it possible

`document.importNode` did not exist. Three of `Element-tagName.html`'s six
subtests are exactly "the same node's `tagName` changes when its node document
does", and each of them reaches for it.

`Node.prototype.cloneNode` was refactored into `cloneNodeInto(node, doc, deep)`
— the destination document is now a parameter — and `importNode` is that
function with `this` as the destination. Two repairs travelled with the
refactor: the null-namespace element case went through `createElement`, which
would have ASCII-lowercased an imported XML name, and now goes through
`createElementNS`; and the two raw-native cases (CDATA section, doctype) never
recorded an owner document, so a clone of either fell back to the primary
document.

### `self`, `window` and `document` took their specified shapes

`install_host_surface` now takes a `GlobalScopeKind` (`Window` or `Worker`);
`Runtime::new` is the window scope and a new `Runtime::new_worker` is what
`worker::worker_main` calls. Two small bootstraps replace the one assignment
line: `SELF_WINDOW_BOOTSTRAP` defines `self` as a `[Replaceable]` accessor (its
setter redefines the property as a writable data property) and `window` as a
non-configurable getter with no setter; `SELF_WORKER_BOOTSTRAP` defines only
`self`, getter-only and configurable.

`document` is defined by the DOM bootstrap, which decides between the two
shapes on `globalThis.window === globalThis` — a Window gets the unforgeable
accessor over the retained `document` object, a worker keeps the plain,
deletable property. No scope marker global was added: `window`'s presence *is*
the signal, and it is installed before the DOM surface for that reason.

This inverts the Worker lane's mechanism without changing its result. That lane
deleted `window` and `document` from the worker global; they are now never
defined there, so `WORKER_SCOPE_BOOTSTRAP`'s `drop()` finds nothing to do for
those two names and testharness.js's `'document' in global_scope` still
selects the worker environment.

## Findings

### Both backends accept a non-configurable accessor on the global (2026-09-07)

This was the open engine question, and it needed an answer rather than an
approximation: the MutationObserver lane's finding is that Nova's global
natives cannot be interposed on and that `defineProperty` throws there. That
is about *redefining* an existing engine-owned global. Defining a new
non-configurable accessor is a different operation, and both Boa and Nova
accept it. `unforgeable_globals_on_nova` asserts the full descriptor
(`get` present, `set` undefined, `configurable: false`, `enumerable: true`),
that `delete globalThis.window` returns `false`, and that a redefinition
attempt throws `TypeError`. No residual is recorded, because there is none.

### The fold has to be a property read, not a stored name

`tagName` cannot be computed once at creation. `importNode` produces a node
whose `tagName` differs from its source's by nothing but the document it now
belongs to, and `adoptNode` changes it for an existing node in place. Folding
inside `elementQualifiedName` on every read is what makes both correct, at the
cost of an `ownerDocumentOf` walk per read. That walk is `O(depth)` and the
getter is not on any internal path any more — the custom-element lookups that
used to read `tagName` now read `localName`, which is a single native call.

### A subtest that folds with `toLowerCase` is testing the wrong alphabet

`interfaces.js` derives its `createElement` variant as
`a[0].toUpperCase()`, so the a-with-ring row arrives as a name whose first
character is a *non-ASCII* uppercase letter. `createElement` must leave it
alone while lowercasing the ASCII tail, and the result must then fail the
custom-element name test on its first code point. Three separate ASCII-only
rules have to hold at once for that one subtest, which is why it was the last
of the three the interface-table lane left behind.

### One checked baseline entry is a throughput wall, not a regression

The `dom` baseline check reported `dom/ranges/Range-mutations-replaceData.html`
moving `pass 1146/1146 -> error (hang-killed)`. Run alone it passes on **both**
runners; timed alone it takes 23–30s against a 30s default timeout, on both
(pre: 26.0s, 29.8s; post: 29.8s, 23.4s). The distributions overlap and the file
straddles the limit. It was repinned from a run at `--timeout 120`, where it
passes, so the committed entry is unchanged rather than pinned backwards.

## Gates

- `cargo test -p script-runtime-api`: 285 tests green across twelve targets
  (the twelfth is the concurrent WebSocket lane's, uncommitted in the same
  tree), including the 12 new ones in
  `components/script-runtime-api/tests/tagname_window_globals.rs` (six bodies
  instantiated on Boa and Nova) and the generated-table drift test.
- `cargo clippy -p script-runtime-api --all-targets`: clean.
- `rustfmt` on every touched Rust file.
- Runner: `cargo build --release -p genet-wpt --features netfetch` in
  `C:/t/laneA-target`. `pre` built from `HEAD` **before** the first edit.
  - genet `54f5d163555` (`c9254000b0e` plus the merge that was already in the
    tree at lane start)
  - `pre` SHA-256 `bd05692bd2a23ef0776a61fff531bd48db7596870d9e9e33dc550aec63156b1c`
  - `post` SHA-256 `af36c1c80c77b48334a0c0c6cc754f31c84fef70ec7139e81b74739241714d43`
- Maps under
  `Code/testing/genet/wpt-ledger/2026-09-07_tagname_window_globals/`, `pre/`
  and `post/`, disk mode, Boa/Livery, `--jobs 8 --timeout 90`.

### Measured movement

| directory | subtests pre | post | delta | file movements |
|---|---:|---:|---:|---|
| `dom/nodes` | 6491/9431 | 6510/9431 | **+19** | `Element-tagName.html` fail → pass |
| `custom-elements` | 2122/3832 | 2126/3832 | **+4** | `Document-createElementNS.html` fail → pass |
| `html/semantics/interfaces.html` | 435/438 | 438/438 | **+3** | fail → **pass** (all-pass) |
| `html/browsers/the-window-object` | 60/599 | 62/599 | **+2** | — |
| `html/webappapis` | 977/1708 | 977/1708 | 0 | — |
| `workers` | 320/967 | 325/967 | +5 | 2 files fail → pass |

Five files move `fail -> pass` and **zero move from a passing status**.

Per-file, every moved subtest is named: `Element-tagName` 3/6 → 6/6,
`Document-importNode` 0/5 → 4/5, `Node-properties` 678 → 682,
`Document-createElementNS` 79 → 81, `Document-createElement` 39 → 41,
`Document-getElementsByTagName` 3 → 4, `Element-getElementsByTagName` 4 → 5,
`Node-cloneNode` 132 → 133, `case.html` 254 → 255;
`custom-elements/Document-createElementNS` 3/4 → 4/4,
`Document-createElementNS-prefix-timing` 0/3 → 1/3,
`registries/valid-custom-element-names` 1859 → 1861;
`window-properties.https.html` 36 → 38;
`Worker-replace-self.any.worker` 0/1 → 1/1,
`interfaces/WorkerGlobalScope/self.any.worker` 3/4 → 4/4,
`constructors/Worker/expected-self-properties.worker` 3/5 → 4/5.
`constructors/Worker/unexpected-self-properties.worker` holds at **57/57**.

Two movements are **not** this lane's and are attributed to the concurrent
WebSocket lane, whose surface registration reached
`script-runtime-api/lib.rs` nine minutes after the `pre` runner finished
linking and so is in the `post` binary only (committed afterwards as
`99f8ca5915c`): `workers/semantics/interface-objects/001.worker.html` 36 → 38, whose
two moved subtests are "The WebSocket interface object should be exposed" and
"The CloseEvent interface object should be exposed". This lane's own subtest
delta is therefore **+31**, not +33. The confound is one-directional (the
`post` runner has strictly more surface) and is confined to that one file; no
other measured file names a WebSocket interface.

One status movement is neither a gain nor a loss:
`workers/Worker-postMessage-happens-in-parallel.https.html` goes
`error (hang-killed) -> no-results (no-subtests)`. It scored nothing before and
scores nothing now; the file needs a live `https` server either way.

### Repins

`--write-expectations` was run only on the two baselines whose entries moved,
and every moved entry moves forward:

- `ports/genet-wpt/expectations/testharness/dom_nodes_boa.json` — nine entries:
  `Document-createElement` 39 → 41, `Document-createElementNS` 79 → 81,
  `Document-getElementsByTagName` 3 → 4, `Document-importNode` 0 → 4,
  `Element-getElementsByTagName` 4 → 5, `Element-tagName` fail 3/6 → **pass**
  6/6, `Node-cloneNode` 132 → 133, `Node-properties` 678 → 682, `case.html`
  254 → 255.
- `ports/genet-wpt/expectations/testharness/dom_boa.json` — the same nine
  entries, since this baseline covers `dom/nodes` too. Its 30/30-line diff is
  identical in shape to the one above.

  `--write-expectations` over the whole `dom` subset was **rejected** here. At
  the default timeout it would have pinned the `Range-mutations-replaceData`
  flake backwards; at `--timeout 120` it wrote 11,267 added lines, because the
  longer budget let files that hang-kill at 30s enumerate subtests the check
  script will never see (57,030 reported subtests against the baseline's
  53,076). Either map would have been a worse record than the truth, which is
  that exactly nine entries moved. The nine were carried across from the
  `dom/nodes` map the same runner had just written, with a script that asserts
  each entry keeps its subtest count and does not lose passes, and the
  `runner_sha256` updated to the `post` binary. Verified afterwards by running
  the baseline check at the script's own default timeout: `unexpected=0`.

`dom/abort` (both baselines) and `html/webappapis/timers` reported
`unexpected=0` and were **not** repinned. All five checked baselines were
re-run against the `post` runner at the check script's default settings after
the repins: `unexpected=0` on every one.

## Residuals

- **`createElement` on an XML document still uses the HTML namespace.** The
  DOM makes the namespace `null` unless the document is an HTML document or its
  content type is `application/xhtml+xml`. `__createElement` always builds an
  HTML-namespaced name. Unchanged by this lane, and unmeasured; the case-folding
  half of the same algorithm is now correct.
- **Attribute names still fold on namespace alone.** `setAttribute` /
  `removeAttribute` / `getAttribute` ASCII-lowercase for an HTML-namespaced
  element without also requiring an HTML node document. Correcting it means an
  `ownerDocument` walk on the hottest DOM writer in the bootstrap, so it should
  be guarded on the name actually containing an ASCII uppercase character
  before the walk is paid for. Deliberately out of this lane.
- **`getElementsByTagName` matching is still native and namespace-blind to the
  document type.** `Document-getElementsByTagName` and
  `Element-getElementsByTagName` each gained one subtest here as a side effect
  of the name change; the remaining failures are matching rules, not casing.
- **The `Range-mutations-replaceData` throughput wall** stands: the file needs
  23–30s against a 30s check-script default. Either the file gets faster or the
  checked baseline needs its own timeout.

## Progress

- **2026-09-07** — `pre` runner built from `HEAD` (`54f5d163555`) before the
  first edit; `pre` maps over the six directories recorded.
- **2026-09-07** — Tag-name seam moved to the bootstrap; interface selection
  and custom element keying made case-sensitive; `importNode` added on a
  parameterised `cloneNodeInto`.
- **2026-09-07** — `self` / `window` / `document` given their specified
  property shapes on both scopes; `Runtime::new_worker` added so the worker
  global never defines what it must not have.
- **2026-09-07** — Gates green; `post` maps recorded and diffed; two baselines
  repinned forward.
