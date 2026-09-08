# Parser / script interleaving

**Status: landed 2026-09-08 in the engine; one named gate blocked by a lane
fence.** Genet commit at start: `9a994bb4193`. Mark's ruling for the lane:
*genet's scripted document must interleave parsing and script execution per the
HTML parsing model rather than parsing the whole document first.* The engine now
does. The WPT runner does not yet route through it — see
**The one gate this lane could not reach**, which is the decision for Mark.

## Why this lane exists

The [Shadow DOM plan](2026-09-07_shadow_dom_plan.md) traced its only
`pass -> fail` and its only `fail -> error` to one structural cause, and named
it a residual for Mark: *"Genet parses a document before it runs its scripts."*
Under that model every consumer of the tree *as it is being built* is wrong, and
each is wrong differently:

| Observer | What parse-then-run gives it |
|---|---|
| the custom-element registry | empty when a later tag is parsed, so nothing upgrades at parse time |
| a declarative `<template shadowrootmode>` | consults an empty registry, so `disabledFeatures: ['shadow']` cannot refuse it |
| a `MutationObserver` | one bulk tree, not the parser's inserts |
| `document.write` | no token stream to write into, so the method could not exist at all |
| `document.currentScript` | no "running script" to name |
| `document.readyState` | a constant, because there is no parse to be in the middle of |

These are not five features. They are one missing seam, and html5ever already
has it: [`Tokenizer::feed`] returns `TokenizerResult::Script(handle)` when the
tree builder pops a `</script>`, having already inserted the element and its
text, and expects the caller to run it and call `feed` again.

## Design

### The shape

```text
push source
loop {
    resume()                         -> Done | Script(node)
    Script(node):
        upgrade parser-created custom elements   (the script must see them upgraded)
        refresh window named properties          (the tree just grew)
        classify: classic / module, inline / external, async / defer
        run it, with document.currentScript set
        microtask checkpoint                     (a MutationObserver fires here)
        refresh the policy table                 (the script may have defined an element)
        apply document.write at the insertion point
}
end()
readyState = interactive; readystatechange
deferred scripts (defer classics and modules), in document order
DOMContentLoaded
readyState = complete; readystatechange; window load
```

Three files, and the split between them is deliberate:

| File | Owns |
|---|---|
| `components/genet-scripted-dom/parser.rs` | the html5ever `TreeSink` over the live arena, and the pause-at-a-time driver around `Tokenizer::feed` |
| `components/script-runtime-api/parse.rs` | *when* a script runs — HTML's script-timing model, the readiness transitions, and the load sequence |
| `components/script-runtime-api/dom/markup_insertion.rs` | the host facts a native reads: `readyState`, `currentScript`, and the `document.open` / `write` / `close` stream |

`genet-scripted-dom` names no engine and makes no engine call; `parse.rs` makes
every engine call and touches no tokenizer state directly.

### `ScriptedTreeSink`: parsing into the live arena

Before this lane there was exactly one `TreeSink` in the tree —
`StaticTreeSink` in `genet-static-dom`, which builds a `StaticDocument` that is
then copied into the arena by `clone_into`. Interleaving cannot use it: the
parser's open-element stack and insertion point live in whatever tree the sink
built, so a script that appends to `document.body` between two tokens must
append to *that* tree, not to a copy made later.

`ScriptedTreeSink` therefore builds directly into `ScriptedDom`, through the
arena's own `LayoutDomMut` mutators rather than a private back door. That is the
whole reason the `MutationObserver` case falls out for free: `append_child`
already writes the observer record, already maintains slot assignment, and
already advances the structural-mutation epoch the reflector-identity policy
caches against. A bulk tree copy produces none of those.

The sink reaches the arena through a `DomAccess` trait rather than owning it,
because the arena is a *field* of the host state, not a separately shared cell.

### Moving the arena rather than sharing it

A tree-sink call and a DOM native both want `&mut ScriptedDom`, and the arena
lives inside `RefCell<HostState>`. Two `RefCell`s over the same value would put
a re-entrant borrow one careless native away from a panic.

They are never live at the same instant — the tokenizer has returned before any
script runs — so the driver *moves* the arena into the parser's cell around each
`resume` and moves it back before running anything. `ScriptedDom` is a handful
of maps behind one pointer each, so the move is cheap, and the ids are unchanged
because they are ids.

One consequence had to be discovered rather than designed: the tree builder
caches a handle to the document node *at construction*, so the real arena has to
be parked before `DocumentParser::new` is called. A handle minted from a
placeholder arena carries the placeholder's document tag and trips the arena's
G0 cross-document fence on the very first `append`. See **Findings**.

### The two questions the tree builder asks the script tier

html5ever 0.39's `TreeSink` has `allow_declarative_shadow_roots(intended_parent)`
and `attach_declarative_shadow(location, template, attrs)` — exactly the seam
the Shadow DOM plan's residual needed. Both are `&self` calls made *inside* a
live tokenizer, so neither may call the engine.

They are answered from `ParserPolicy`, a plain table the driver refreshes at
every pause. This is not an approximation: the registry can only change while a
script runs, and scripts only run at pauses, so a table refreshed at each pause
is exactly current for the whole stretch of tokenizing that follows. The order
matters and was wrong in the first draft — the refresh has to happen **after**
the script runs, not before it, because the definition the next stretch of
tokenizing asks about is the one that script just made.

`attach_declarative_shadow` returns `false` (leaving the template ordinary) in
two cases, both HTML's:

1. the intended parent's definition disables `shadow`;
2. the host already has a shadow root — including one attached by a
   `MutationObserver` callback earlier in this same parse.

When it succeeds, the template element is never inserted into the tree at all
(html5ever's `insert_foreign_element(..., only_add_to_element_stack: true)`), and
the sink maps the template to the shadow root as its content target, so
everything parsed inside lands in the root directly.

The static DOM's post-parse declarative pass is untouched: the script-free route
still parses in one pass and realizes declarative roots afterwards, which is
right, because a script-free document has no registry to consult.

### `document.write`, in two genuinely different halves

**During a parse** `document.write` is not a DOM operation. It inserts source at
the *insertion point* of the tokenizer's input stream — immediately after the
running script's own position — which is literally `BufferQueue::push_front`.
The native therefore only queues the text; the driver pops the queue when the
script returns and pushes it at the front. Nothing else can be correct, because
the DOM has no way to express "half an open tag", and
`document.write('<i>'); document.write('x</i>')` has to be one source stream.

**After a parse** `document.write` implies `document.open`, which *replaces* the
document. There is no tokenizer to feed, so the native keeps the written source
in a buffer and re-materializes the document's contents from it on each write.
The exact rule and what it does not do are under **Residuals**.

### Script timing

| Form | Timing |
|---|---|
| inline classic | parser-blocking, at the pause (`async`/`defer` have no effect without `src`) |
| external classic, no `async`/`defer` | parser-blocking: fetched through the document's resource route and awaited at the pause |
| external classic, `async` | run at the pause without an ordering promise — the route is synchronous, so the script *is* available there, which is what "as soon as available" means for it |
| external classic, `defer` | after parsing, in document order, before `DOMContentLoaded` |
| inline or external **module** | after parsing, in document order, before `DOMContentLoaded` (modules are defer by default) |
| a `type` naming neither, or a script the tokenizer marked "already started" | never runs |

`document.currentScript` is set for the duration of each classic script and left
null for modules, per HTML.

### Readiness

`document.readyState` stopped being the constant `'complete'` in the bootstrap
and became a host fact the driver moves. A document nobody parsed — the WPT
harness route, a `DOMParser` result — still reads `complete`, which is what the
old getter returned unconditionally, so no existing caller changes.

Order, per HTML's "the end": readiness to `interactive` and its
`readystatechange` **before** the deferred list runs, then `DOMContentLoaded`,
then `complete` with its `readystatechange`, then `load` on the window.

## Done-conditions, and where each one stands

| # | Done-condition | State |
|---|---|---|
| 1 | Drive html5ever so the parser pauses at each popped `<script>`, runs it, and resumes; parser-blocking in document order, external fetched through the resource route and awaited, `async` unblocking, `defer` after parsing before `DOMContentLoaded`, modules deferred; `currentScript` set | **met** — `script-runtime-api --test parser_script_interleaving`, 11 cases on both engines |
| 2 | `document.write` / `writeln` at the insertion point during parsing; `open` semantics after; `close`; `readyState` transitions with `readystatechange`, `DOMContentLoaded` and `load` at their spec points | **met for the parsing half and the readiness half**; the post-parse `open` stream is implemented with a named rule and two named residuals |
| 3 | Custom elements upgrade at parse time; declarative shadow roots consult the registry; `MutationObserver` sees parser insertions; the reflector root-on-insertion path keeps wrapper identity | **met** — four dedicated cases on both engines, including both Shadow DOM regressions reproduced at the engine level |
| 4 | The static DOM (script-free route) is unchanged | **met** — no file under `genet-static-dom` was touched; `dom` and `html/dom/documents` maps are byte-identical |
| 5 | The two Shadow DOM WPT regressions recover | **not met, and not reachable from this lane** — see below |

## The one gate this lane could not reach

**`shadow-dom/declarative/declarative-with-disabled-shadow.html` and
`innerhtml-on-ordinary-template.html` are byte-identical before and after.**
They were not forgotten and the fix is not missing: both behaviours are proved
at the engine level by `disabled_shadow_on_{boa,nova}` and
`observer_parse_on_{boa,nova}`, which are those two WPT files rewritten as
runtime cases.

They do not move because **the WPT runner never parses a document with scripts
interleaved**. `ports/genet-wpt/src/harness.rs` does, in three statements:

```rust
let doc = parse_doc(html);                                  // whole document, no scripts
let mut scripts = Vec::new();
collect_scripts(&doc, doc.document(), loader, &mut scripts);
let test_src = scripts.join("\n;\n");                       // every script as one blob
```

and then `run_with(..., &test_src, &doc, ...)` calls `rt.load_dom(doc)` and
evaluates the blob. Parse-then-run is not a property of the engine any more; it
is a property of that file. This lane was fenced out of `ports/genet-wpt/src`,
so the change was not made.

**The change itself is small and named here so it is not rediscovered:**
`run_test_with_webgl_and_style` calls `rt.parse_document_interleaved(html, &L)`
in place of `parse_doc` + `collect_scripts` + `load_dom` + the blob eval, where
`L` is a `ParserScriptLoader` over the existing `ScriptSrcLoader`. Two knots to
untie while doing it, both visible from here:

1. `testharness.js` is currently filtered *out* of `collect_scripts` and loaded
   separately before the test body. Under interleaving the loader can simply
   serve it, but the harness bridge must still be installed before the first
   script runs.
2. `begin_loaded_testharness` dispatches `load` itself. The interleaved parse
   dispatches `load` at the end of its own sequence, so one of the two has to
   stop — the parse's, most likely, with the runner keeping the completion
   handshake it already owns.

**This is Mark's call**: whether the runner route is a follow-on lane, and
whether the two Shadow DOM entries stay recorded as regressions until it lands.

## Gates and receipts

Runner digests (SHA-256), both built in `C:/t/laneP-target` with
`cargo build --release -p genet-wpt --features netfetch`:

| Runner | Digest |
|---|---|
| `pre` (built from `9a994bb4193`, before the first edit) | `49cdc86ac52e66bb84dc02834ae20252ccd8c958ac450ed2260627efbc345243` |
| `post` | `ca894cfa77a68a9419c6dc1733c0917aca4a3942080ed6f572921c2ab6bca886` |

Both runs: `genet-wpt testharness <dir> --engine boa --renderer livery --jobs 6
--timeout 240`, disk mode, over the same vendored WPT tree. Raw maps, logs, the
run script and the diff under
`Code/testing/genet/wpt-ledger/2026-09-08_parser_script_interleaving/`.

### Before / after

| Directory | files all-pass | errored | subtests passed |
|---|---|---|---|
| `html/syntax` | 20 → **21** | 62 → **1** | 2,698 / 8,174 → **2,717 / 8,237** |
| `html/semantics/scripting-1` | 53 → **54** | 125 → **103** | 1,353 / 2,816 → **1,364 / 2,849** |
| `html/webappapis/dynamic-markup-insertion` | 1 → **30** | 3 → 3 | 11 / 356 → **45 / 338** |
| `html/dom/documents` | 13 → 13 | 1 → 1 | 89 / 240 → 89 / 240 (identical) |
| `dom` | 222 → 222 | 23 → 23 | 46,370 / 57,171 → identical |
| `custom-elements` | 8 → **9** | 23 → **22** | 2,149 / 3,837 → **2,150 / 3,839** |
| `shadow-dom` | 45 → 45 | 13 → 13 | 1,512 / 8,804 → **1,515 / 8,804** |
| `html/webappapis/scripting` | 15 → 15 | 19 → 19 | 80 / 266 → 80 / 266 (identical) |

Aggregate: **+68 subtest passes**, 30 files `fail -> pass`, 2 `error -> pass`,
82 `error -> fail`, and **zero `pass -> fail`**.

### Explained movements

- **`html/syntax` errored 62 → 1.** Every one of the 61 recovered files is
  `speculative-parsing/generated/document-write/*.tentative.sub.html`. They call
  `document.write` in setup; with the method undefined the file threw before its
  first subtest. They now run and fail on their actual subject (speculative
  parsing, which genet does not do), which is an honest `fail`, not a pass.
- **`dynamic-markup-insertion` 1 → 30 all-pass, 29 files `fail -> pass`.** The
  `document-write/0xx` battery plus two `opening-the-input-stream` files: the
  implied-`document.open` stream. Its subtest *total* went **down** by 18
  because several files enumerate their subtests only after the first write
  succeeds, and a file that now completes reports fewer stub subtests than one
  that threw partway. Passes went up 11 → 45 in the same directory.
- **`custom-elements/parser/parser-constructs-custom-element-in-document-write.html`,
  `error -> pass`.** The one file in that directory whose subject *is* this
  lane, reached through the post-parse write path.
- **`html/syntax/parsing/html5lib_innerHTML_template.html`, `fail -> pass`.**
  `innerHTML` on a `<template>` is defined over its **template contents**, not
  its children; the bootstrap had it on children, so the getter serialized an
  always-empty list and the setter put nodes where no walk reaches. Found by the
  `innerhtml-on-ordinary-template` reproducer and fixed here.
- **21 `error -> fail` in `the-script-element/execution-timing/`.** Same
  document-write setup pattern. These are the files that measure the very thing
  this lane implements, and they will only *pass* once the runner routes through
  the interleaved parse.
- **`dom`, `html/dom/documents`, `html/webappapis/scripting` byte-identical.**
  The control: the runner's own route did not change, so the crates this lane
  touched cost the unchanged paths nothing.

### Other gates

- `cargo test` green for every crate touched, on **both** engines:
  `genet-scripted-dom` (37 unit, 6 of them new), `script-runtime-api`
  (142 unit + 13 integration targets, including 22 new interleaving cases on
  Boa and Nova), `genet-scripted` (26, one new).
- `cargo clippy` clean on every file this lane touched; `cargo fmt` applied and
  re-checked.
- `cargo check --workspace --features genet-wpt/netfetch` clean (the only
  warnings are pre-existing `nova_vm` ones from the vendored fork).
- **No baselines repinned.** `check-testharness-baselines.ps1 -NoBuild` against
  the `post` runner reports `unexpected=0` on all fourteen checked slices and
  `WPT testharness baselines: unexpected=0` overall
  (`post_testharness_baselines.log` in the ledger directory).
- **Both reftest guards at `unexpected=0`.** `check-reftest-baselines.ps1
  -NoBuild` on `css/mediaqueries` and `css/css-position`
  (`post_reftest.log`).
- **Ortet receipt unchanged.** `cargo run -p ortet -- --url
  ports/ortet/examples/article.html --frames 3 --artifact C:/t/laneP-ortet.png`:
  engine `genet.livery`, backend livery, 3 frames at 960x640, digest
  `0x6377ba8a6bf4dbc9` — identical to the Shadow DOM lane's. The whole frame was
  examined, not only the changed feature: heading, italic lede, link run and
  separator, section heading, body text and the gradient swatch all render as
  before. Ortet is script-free, so this is the control that the new parse costs
  the script-free route nothing.

## Findings

- **The tree builder caches the document handle at construction.** The real
  arena has to be parked *before* `DocumentParser::new`, not at the first
  `resume`. A handle minted from a placeholder arena carries the placeholder's
  document tag, and the arena's G0 fence caught it on the first `append` with
  `NodeId from a different document (id tag 2, this doc 1)` — which is the fence
  doing exactly its job, on the first day something could have gone wrong
  silently.
- **A policy table read by a `&self` sink must be refreshed *after* the script,
  not before it.** The first draft refreshed at the top of the pause, which is
  one script too early: the definition the *next* stretch of tokenizing asks
  about is the one the script that is about to run has not made yet. The
  `declarative-with-disabled-shadow` case failed for exactly that reason and is
  now the regression that holds the ordering.
- **Window named properties are live, and a mid-parse script names elements
  parsed since the last pause.** `ordinarytemplate.innerHTML = ...` in the WPT
  reproducer resolves a window named property for an element that did not exist
  at the previous pause. The refresh has to run *before* each script as well as
  after it.
- **`innerHTML` on a `<template>` was operating on its children.** A template
  has no children in the tree — its content is a parentless fragment, by the
  Shadow DOM lane's own "encapsulation is a property of the shape" choice — so
  the getter serialized an always-empty list and the setter put nodes somewhere
  no walk reaches. Nothing had caught it because nothing set a template's
  `innerHTML` until this lane's reproducer did. That is the second time this
  shape has cost a bug (the first was three copiers, in the Shadow DOM plan's
  Findings): a container whose contents are deliberately unreachable needs its
  *own* accessors told, one at a time, and each one is silent until exercised.
- **A method that does not exist floors a directory more thoroughly than a
  method that is wrong.** 61 of the 62 `html/syntax` errors were one undefined
  `document.write`. This is the "a directory's census can be floored by one
  missing name" principle again, and the missing name was not in a shared
  `common.js` this time — it was in the tests' own setup, which is why no probe
  of the helper would have found it.
- **A subtest total can fall while passes rise.** `dynamic-markup-insertion`
  lost 18 enumerated subtests and gained 34 passes, because a file that throws
  partway through setup can still have reported stub subtests that a completing
  file does not. Read the pass count, not the total, when a write path starts
  working.

## Residuals

Named, not silently deferred:

1. **The WPT runner still parses then runs.** The gate above, and Mark's call.
   Until it is routed, every `execution-timing/*` file and the two Shadow DOM
   declarative files measure the runner, not the engine.
2. **`LiveryScriptedDocument::build` is still on the old path.** Its
   `LiveryCssom::install_live` resolves the document's stylesheets from the live
   DOM *and* must be installed before any script runs; interleaving needs both
   at once, so the CSSOM install needs a two-phase form (resolve-later, or a
   mutation-cursor-only install over an empty document) before that constructor
   can move. `ScriptedDocument::build` — the tested one — is converted.
3. **A `<script>` written by `document.write` after parsing does not execute.**
   The write native runs inside a `CallCx` and cannot re-enter the engine, so
   the markup is inserted synchronously (visible to the rest of the calling
   script) and any script element in it is inert. The exact rule: *source
   written while no parser is active is parsed and materialized, but not
   executed.* During a parse the same source **does** execute, because it goes
   through the tokenizer.
4. **The post-parse `document.open` stream re-materializes rather than
   appends.** Each `document.write` with no active parser re-parses the whole
   accumulated stream and replaces the document's children, so nodes from an
   earlier write in the same stream do not keep their identity across a later
   one. Quadratic in the number of writes, and wrong for a test that holds a
   reference across two writes; right for `open(); write(...); close()`, which
   is what the recovered battery does.
5. **`document.open(url, name, features)` throws `NotSupportedError`.** That
   form is `window.open`, and the scripted tier has no browsing context.
6. **Quirks mode is recorded but dropped.** The arena has no quirks-mode field
   (the bootstrap reports `compatMode` as the constant `'CSS1Compat'`), so
   `ParserPolicy` keeps what html5ever inferred and nothing reads it yet.
7. **`is_mathml_annotation_xml_integration_point` is always false** on the
   scripted sink. The arena stores no per-element flag for it; the static tier's
   copy is a parse-time fact the tree copy already dropped, so this is not a
   regression, but it is not right either.
8. **Custom-element upgrades run at the pause, not at element creation.** A
   parsed element that could name a custom element is recorded and upgraded at
   the next pause, which makes it upgraded by the time any script can observe
   it. HTML runs the constructor at creation; the difference is observable only
   from another custom element's constructor.
9. **A parse still costs one extra static parse.** `ScriptedDocument::build`
   parses the source once with `StaticDocument::parse` for stylesheet
   resolution, because the resources must be resolved before the runtime exists,
   and once again through the interleaved parser for the live tree. Folding the
   resource resolution onto the live DOM would remove it.
10. **No headed receipt for interleaving.** Ortet's default route is
    script-free, so the Ortet receipt is a control (unchanged), not a proof. Per
    [Ortet O5](2026-09-03_ortet_founding_plan.md) the headed gate for scripted
    behavior stays open.

## Regression manifest

The named suites that must keep passing, and what each one holds:

| Suite | Holds |
|---|---|
| `script-runtime-api --test parser_script_interleaving` (11 cases, Boa + Nova) | the partial tree at a pause, document-order script timing across inline/external/async/defer/module/data-block, `document.write` at the insertion point and across two calls, `currentScript`, the readiness order, parse-time custom-element upgrade, both Shadow DOM regressions, `MutationObserver` over parser insertions, wrapper identity through a collection, and the implied `document.open` |
| `genet-scripted-dom --lib parser::tests` (6 cases) | the tree sink alone: pausing at each script, the tree being *partial* at the pause, `document.write` ordering in the buffer queue, the declarative root at parse time, and both `attach_declarative_shadow` refusals |
| `genet-scripted --lib extraction_tests` (3 cases) | that `ScriptedDocument` still extracts a post-JS article, that a static document is identical under both profiles, and that the document parses and runs interleaved end to end |
| `script-runtime-api --test shadow_dom`, `--test mutation_observer`, `--test selection_range`, `--test dom_node_model` | the arena contracts a new tree-sink writer could break |
| `check-testharness-baselines.ps1`, `check-reftest-baselines.ps1` | `unexpected=0` on the checked slices |
| the Ortet receipt | that the script-free render path is untouched |

## Progress

**2026-09-08 — landed in the engine.** `ScriptedTreeSink` and the pause-at-a-time
`DocumentParser` over the live arena; the script-timing drive loop, readiness
transitions and load sequence in `script-runtime-api::parse`; `document.open` /
`write` / `writeln` / `close`, `currentScript` and a host-backed `readyState`;
the declarative-shadow hooks answered from a per-pause policy table; parse-time
custom-element upgrade; `ScriptedDocument::build` converted; and `innerHTML` on
`<template>` moved onto its contents. +68 subtest passes over eight directories,
30 files `fail -> pass`, 2 `error -> pass`, zero `pass -> fail`. The two Shadow
DOM declarative regressions are fixed at the engine level and unchanged in WPT,
because the runner does not route through the new parse — Mark's call, above.
Receipts under
`Code/testing/genet/wpt-ledger/2026-09-08_parser_script_interleaving/`.
