# Parser / script interleaving

**Status: landed 2026-09-08, both parts.** Genet commits: part one from
`9a994bb4193`, part two from `d0b56fcfb26`. Mark's ruling for the lane:
*genet's scripted document must interleave parsing and script execution per the
HTML parsing model rather than parsing the whole document first.*

Part one (below) converted the engine and `ScriptedDocument::build`. **Part two
— [its own section at the foot of this document](#part-two--the-runner-the-livery-route-and-the-named-residuals) —**
routed the WPT runner and the Livery-backed document through the same parse and
closed the three named residuals that were left. The gate part one could not
reach is now met: both Shadow DOM declarative files recover.

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

---

# Part two — the runner, the Livery route, and the named residuals

**Status: landed 2026-09-08.** Genet commit at start: `d0b56fcfb26` (part one).
Everything above this line is part one and is left as it was written, including
its "gate this lane could not reach", which this part closes.

## What part two is for

Part one left the engine correct and the *consumers* on the old route. Three of
them, and each hid the engine behind a different wall:

| Consumer | What it still did | Why it mattered |
|---|---|---|
| `ports/genet-wpt`'s `run_test_with_webgl_and_style` | `parse_doc` -> `collect_scripts` -> one joined blob | every WPT file measured the runner, not the engine |
| `LiveryScriptedDocument::build` | parse-then-run, because `LiveryCssom::install_live` needed the finished DOM | the product's own scripted route was not the tested one |
| `document.write` after a parse | re-materialized the accumulated source; a written `<script>` was inert | two named residuals, plus a whole WPT battery |

## Phases and done-conditions

### Phase 6 — the runner parses interleaved

**Done-conditions, all met.**

1. `run_test_with_webgl_and_style` calls the runtime's interleaved parse through
   a `ParserScriptLoader` over the existing `ScriptSrcLoader`. **Met** —
   `HarnessParserLoader` serves `testdriver-vendor.js` from genet's automation
   backend, serves nothing for `testharness.js` / the report hook, and delegates
   everything else to `ScriptSrcLoader::load_script`.
2. The harness bridge is installed **before** the first parsed script runs.
   **Met** — `load_testharness` is a prelude in `run_with`, before the parse,
   which is where a browser has it (the test loads it from `<head>`).
3. **Exactly one** of the runner and the parse dispatches `load`. **Met** —
   `Runtime::begin_parsed_testharness` clears results, calls
   `parse_document_interleaved_with(.., dispatch_load = false)`, then dispatches
   `load` itself. The runner keeps the completion handshake it already owned.
4. Disk-mode scoring semantics otherwise unchanged; the virtual clock and the
   deadline still govern; `--in-process` and the worker path both go through the
   same routine. **Met** — everything downstream of `run_parsed_with` is the
   original `run_loaded_with` body, and `NovaHarnessTemplate` takes the same
   fork.

**The one exception, deliberately kept.** `xml5ever` has no pause-at-a-script
driver, so an XML-syntax document (`looks_like_xml`: the `svg/` corpus and
`html/the-xhtml-syntax`) keeps the parse-then-run route. The fork is the same
test that already chose the parser.

### Phase 7 — the Livery-backed document parses interleaved

**Done-conditions, all met.**

1. `LiveryCssom` has a two-phase form. **Met** — `LiveryCssom::install_over_parse`
   installs a *live* stylesheet source with a fetcher that serves nothing, so it
   resolves exactly what `ResolvedDocumentResources::discover` finds but never
   goes stale: `synchronize_live_styles` re-resolves from the arena at every
   read (`getComputedStyle`, `document.styleSheets`, and each frame).
2. Stylesheets and style elements attach at the point they are parsed. **Met** —
   `livery_stylesheets_attach_where_they_are_parsed_on_boa` asserts it
   positionally: the script *before* a `<style>` reports 0 sheets and the
   unstyled colour, the script *after* it reports 1 and `rgb(0, 128, 0)`.
3. `LiveryScriptedDocument::build` uses the interleaved parse. **Met** — the
   static pre-parse, `load_dom`, `collect_scripts` and the two-pass script loop
   are gone; the CSSOM is installed over the empty arena before the parse, and
   the capture recorder is opened after it, over the same tree the old route
   handed it.
4. The WPT rendering session uses the same install. **Met** —
   `RenderSession::new` is built before the parse and still holds the whole
   cascade afterwards.

### Phase 8 — the three named residuals

**Done-conditions, all met.**

1. **A `<script>` written after parsing runs, per the script element's insertion
   steps** — and runs synchronously, inside the `document.write` call that wrote
   it. **Met** — `a_written_script_runs`, both engines.
2. **The post-parse open stream appends rather than re-materializing.** **Met** —
   `the_open_stream_appends`: a node from an earlier write keeps its identity
   across a later one, and a tag split across two writes is one element.
3. **Parse-time custom-element upgrades run at creation rather than at the next
   pause.** **Met** — `a_parsed_element_upgrades_at_creation` reads `1,2,3` from
   three constructors, not `3,3,3`.

## Design

### One tokenizer, and it does not live in the driver

Part one's driver owned the `DocumentParser` as a local. Part two moves it into
`MarkupState`, beside `readyState` and `currentScript`, because
**`document.write` has to be able to feed it from inside a native**. HTML has
the parser process the inserted characters *during* the `write()` call, and the
whole `document-write/0xx` battery is written in that shape:

```js
document.write("PASS");
assert_equals(document.body.textContent, "PASS");
```

Queuing the source until the calling script returns — part one's model, which
was invisible while the runner never parsed with scripts — fails every one of
those. So:

- `DocWrite` pushes the text at the insertion point and tokenizes it **now**.
- It tokenizes *only* the written characters (`DocumentParser::pump_written`),
  not on into the document's remaining source; otherwise a second write in the
  same script would land after markup that follows the first one.
- `write_at_insertion_point` hands the tokenizer's unread input back to the
  parser's own `pending` queue first, which is what makes the insertion point a
  *point* rather than a queue position.
- If the written source reaches a `<script>`, the native **stalls** the pause in
  `MarkupState::stalled` instead of consuming it. The driver's `stream_resume`
  takes a stalled pause before asking the tokenizer for another, so script
  *timing* — `async`, `defer`, modules, external sources — stays in
  `script-runtime-api::parse`, where it belongs.

The post-parse `document.open` stream is the *same* type and the same
tokenizer; only who runs the scripts differs. With no document parser active,
the bootstrap's own `document.write` runs the loop:

```js
__docWrite(text);
var source;
while ((source = __docPumpStream()) !== null) indirectEval(source);
```

(Illustrative; the shipped form is in `bootstrap.js` with an iteration guard.)
A native cannot re-enter the engine; JavaScript can. Putting the loop in the
bootstrap is what makes a written `<script>` run *and* keeps `document.write`
synchronous.

### Upgrading at creation, without an engine call in the sink

The tree sink cannot call the engine, so the constructor cannot run exactly
where HTML runs it. What the driver *can* do is regain control more often.
`ParserPolicy` gains `upgrade_at_creation`, set by the driver's per-pause
refresh whenever the registry holds any definition at all. While it is set:

- `DocumentParser` hands the tokenizer one **tag at a time** (cut just past the
  next `>`) instead of the whole remaining buffer, and takes back whatever the
  tokenizer is still holding so the switch takes effect immediately;
- `resume` returns a new `ParsePause::Created` as soon as a candidate exists;
- the driver upgrades and resumes.

A document that never defines a custom element pays nothing: the flag is false,
the parser feeds whole buffers, and `Created` is never returned.

### The two-phase CSSOM

`LiveryCssom::install` freezes the author sheets it is handed. Over an arena the
parser has not filled that is none at all, which would have taken `css/cssom`
and `css/css-values/tree-counting` to the floor. `install_over_parse` keeps a
live source instead — `synchronize_live_styles` already re-resolved from the DOM
on every read for the product route, so the two-phase form was one no-op fetcher
away. It is strictly more than the old static install could do: a `<style>` a
*script* adds now enters the cascade too.

## Findings

- **A queued `document.write` is invisible until the runner parses.** Part one's
  "queue the text, apply it when the script returns" passed 22 runtime cases and
  a whole WPT directory, because nothing in either route ever read the DOM back
  inside the writing script. The first honest consumer — the runner — turned 21
  `document-write/0xx` files from `pass` to `fail` in one step. A model no
  consumer exercises is not validated by the tests that do not exercise it.
- **The `.window.html` wrapper had no `<div id=log>`, and only interleaving
  could tell.** WPT's generated wrapper puts one between the harness scripts and
  the test script; genet's synthesized copy omitted it. Under parse-then-run
  `load_dom` produced a body regardless, so `document.body.append(...)` worked.
  Under interleaving the test script runs while everything is still in `head`
  and `document.body` is null — which is what a browser would do to that
  wrapper, too. Two `pass -> fail` files, both fixed by making the wrapper match
  the generator it claims to imitate.
- **The tree builder holds ids the arena is free to reclaim.** Two files call
  `document.documentElement.innerHTML = ...` from a script the parser is
  running. `set_inner_html` frees the replaced subtree (unless a
  `MutationObserver` is watching), and the tree builder's open-element stack
  still pointed at it: `NodeId refers to a live node`, a panic, on the very
  first file that tried. The arena now carries a `parsing` flag with exactly the
  shape `observing` already had — while set, a replaced subtree is orphaned
  rather than freed, and ordinary collection reclaims it once the parse ends.
  The fence caught this the first time it could have gone wrong silently, which
  is twice now for this lane.
- **A whole test module is compiled out by a feature that does not exist.**
  `genet-scripted`'s 1,800-line `#[cfg(all(test, feature = "render"))] mod tests`
  never builds: the crate has no `render` feature, and its own `Cargo.toml`
  lints section says so. Its Livery receipts — including the one that would have
  proved parse-time CSSOM visibility — have not run for some time. The new
  two-phase receipt was therefore written into `livery_text_fragment_tests`,
  which is gated on `feature = "livery"` and does run. Reviving the dead module
  is its own lane; it is recorded as a residual below, not fixed here.
- **`type` classification had two spec bugs that only a running script shows.**
  `type="MODULE"` must match ASCII case-insensitively, and a present-but-empty
  `type` is classic outright — `language` is consulted only when there is no
  `type` at all. Both were in part one's `classify`; neither could be observed
  until the runner ran the scripts the parser found.
- **`is_mathml_annotation_xml_integration_point` needed no arena field after
  all.** Part one's residual assumed the flag had nowhere to live. It arrives on
  `ElementFlags` at `create_element` and is only asked about during the parse,
  so a `HashSet` on the sink — the shape `already_started` already had — is the
  whole fix. Three `html/syntax` foreign-content files were failing for it.

## Before / after — the named subset

Disk mode, `--engine boa --renderer livery --jobs 8 --timeout 240`, both runners
built in `C:/t/laneP2-target`.

| Directory | files all-pass | errored | subtests passed |
|---|---|---|---|
| `html/syntax` | 21 → **22** | 1 → **0** | 2,717 / 8,237 → **2,810 / 8,475** |
| `html/semantics/scripting-1` | 54 → **71** | 103 → **0** | 1,364 / 2,849 → **1,391 / 2,989** |
| `html/webappapis/dynamic-markup-insertion` | 30 → **63** | 3 → **0** | 45 / 338 → **78 / 340** |
| `html/dom/documents` | 13 → 13 | 1 → **0** | 89 / 240 → **96 / 258** |
| `dom` | 222 → **223** | 23 → **8** | 46,370 / 57,171 → 46,370 / 57,192 |
| `custom-elements` | 9 → **10** | 22 → **0** | 2,150 / 3,839 → **2,158 / 3,929** |
| `shadow-dom` | 45 → **50** | 13 → **0** | 1,515 / 8,804 → **1,521 / 8,825** |
| `html/webappapis/scripting` | 15 → **16** | 19 → **1** | 80 / 266 → **83 / 293** |
| `css/cssom` | 20 → **22** | 19 → **0** | 811 / 1,500 → **815 / 1,513** |
| `css/css-values/tree-counting` | 1 → 1 | 1 → **0** | 19 / 82 → 19 / 82 |

Aggregate: **+181 subtest passes**, 55 files `fail -> pass`, 8 `error -> pass`,
117 `error -> fail`, 71 `error -> no-results`, and **2 `pass -> fail`**, both
named below.

**The gate part one could not reach is met.**
`shadow-dom/declarative/declarative-with-disabled-shadow.html` moves
`fail -> pass` and `innerhtml-on-ordinary-template.html` moves `error -> pass`.

### The subset's explained movements

- **`errored` goes to zero in eight of ten directories.** A file whose first
  script threw used to take the whole joined blob with it, because every script
  in the document was one `eval`. Under interleaving each script is its own
  evaluation, so a throw costs that script and nothing after it. This is the
  single largest effect in the census and it is a *harness* effect, not an
  engine one: 117 `error -> fail` and 71 `error -> no-results` in the subset are
  the same files reporting honestly instead of aborting.
- **`dynamic-markup-insertion` 30 → 63 all-pass.** The `document-write/0xx`
  battery now runs during the parse, at the insertion point, and its writes are
  visible to the writing script.
- **`scripting-1` 54 → 71, errored 103 → 0.** The `execution-timing/*` family
  measures exactly what this lane implements and now reaches it.
- **`css/cssom` 20 → 22 and `tree-counting` held.** The two-phase CSSOM: the
  session is now built before the parse and still holds the whole cascade, and a
  `<style>` a script adds enters it too.
- **`css/css-values/tree-counting` unchanged at 19 / 82.** The control for
  Phase 7 — a one-shot install over an empty arena would have floored it.

## Before / after — the full disk census

The 2026-09-07 harness-repair census tooling, adapted to this lane's own
`pre_census` / `post_census` pair (`run_census.sh`, `diff_census.py`). 80
directories named, **79 mapped** (`html/semantics/the-root-element` is not in
this vendored tree, as in every previous census). `--jobs 8 --timeout 90`.

| Aggregate transition | Files |
|---|---|
| `error/evaluation-threw -> no-results/no-subtests` | 994 |
| `error/evaluation-threw -> fail` | 509 |
| `fail -> pass` | 68 |
| `no-results/no-subtests -> fail` | 49 |
| `error/evaluation-threw -> pass` | 22 |
| **`pass -> fail`** | **19** |
| `error/evaluation-threw -> no-results/server-side-handler` | 15 |
| `fail -> no-results/no-subtests` | 6 |
| `error/server-side-handler -> fail` | 3 |
| `error/panic -> pass` | 2 |
| `no-results/no-subtests -> pass` | 2 |
| `fail -> error/evaluation-threw` | 1 |
| `error/panic -> fail` | 1 |
| `fail -> error/hang-killed` | 1 |

Census subtest-pass delta: **−55** as measured, **+439** with the two timing
artifacts below removed. Both artifacts are identified, not assumed.

### Attribution — harness change or engine change

**Harness (the interleaved route itself), the dominant effect.**

- **1,503 `error -> {no-results, fail}`.** One `eval` per script instead of one
  per document. A file that threw in its first line used to be `error` with no
  subtests; it now runs the rest of its scripts and reports.
- **68 `fail -> pass` + 22 `error -> pass`.** Files whose subject *is* the
  parsing model: `document.write`, `currentScript`, `readyState`, parse-time
  custom elements, declarative shadow roots.
- **16 `pass -> fail` in `html/dom/render-blocking/`.** Every one calls
  `generateParserDelay()`, which is `document.write('<script src=…trickle…>')`.
  Under parse-then-run that write happened *after* the parse, so the implied
  `document.open` **wiped the document** — and the file's assertion is
  `assert_false(!!document.getElementById("last"))`, which an empty document
  satisfies. They passed because the document had been destroyed. Under
  interleaving the write goes to the insertion point, the document survives, and
  the assertion fails honestly. This is the "a subtest that compares two absent
  things passes" principle, in its most literal form yet.
- **2 `error/panic -> pass`** and **1 `error/panic -> fail`**: files that
  panicked in the pre runner and no longer do.
- **1 `fail -> error/hang-killed`, and it is a clock, not a defect.**
  `shadow-dom/declarative/gethtml.html` takes **84.5 s on `pre` and 90.3 s on
  `post`**, timed directly, and the census timeout is 90 s. Both runners score
  it 376 / 6,904 when allowed to finish. Its 376 subtests are the whole of
  `shadow-dom`'s −370 in the census column; the same directory in the
  `--timeout 240` subset run is **+6**.
- **±261 swings across `encoding/textdecoder-fatal-single-byte.any.worker.html`
  variants, net −118, every file `fail -> fail`.** Worker variants truncated by
  the drive deadline stop at a different subtest each run. This directory was
  **+242** on an earlier post run of the same tree; it is nondeterminism in a
  deadline-truncated worker test, not a movement.

**Engine (the parse and the script model).**

- **+93 `html/syntax`**, from
  `is_mathml_annotation_xml_integration_point` now being answered (three
  foreign-content files) and from the whole directory running.
- **`svg`, net −1 with one `pass -> fail`.** Foreign-namespace `<script>`
  elements do not pause html5ever's tokenizer; they are collected at creation
  and run at the next pause. That recovers `script-common.html`,
  `scripted/async-02`, `defer-02`, `module-02`, `async-03` and the
  `svg/animations/*` cluster. The one remaining loss,
  `svg/scripted/script-invalid-script-type.html`, needs a *re-prepare* when a
  script element's `type` becomes valid after parsing — a DOM insertion-step
  behaviour genet does not have; it is a residual below.
- **2 `pass -> fail` in `execution-timing/`** (`112`, `120`) — the `async`
  timing simplification and a script created without a browsing context. Both
  named residuals, below.
- **5 `fail -> no-results` in `protocol-handler-*.https.html` and 1
  `fail -> error` in `fetch/api/basic/stream-safe-creation.any.html`.** Zero
  subtest passes on both sides; the file's throw moved earlier, so it enumerates
  nothing rather than enumerating and failing. No capability changed.
- **`dom/ranges/Range-selectNode.html`, 280/288 → 276/284.** The file generates
  its subtests by walking `document` from a script. Under interleaving that
  script sees the tree as far as its own position, so four trailing nodes are
  not yet there: four fewer generated subtests and four fewer passes, the same
  pass rate. Repinned with this reason.

## Gates and receipts

Runner digests (SHA-256), both built in `C:/t/laneP2-target` with
`cargo build --release -p genet-wpt --features netfetch`:

| Runner | Digest |
|---|---|
| `pre` (built from `d0b56fcfb26`, before the first edit) | `4818b5673fc294b2e5e0e68428812c4ee240108352ac2e94f7eaa8152838059c` |
| `post` | `50d4f8be4696046cbd186ee15e3b6480edf45b352636e4ea4d5c22a23550b2f5` |

Maps, logs, run scripts and diffs under
`Code/testing/genet/wpt-ledger/2026-09-08_interleaving_part_two/`
(`pre/`, `post/`, `pre_census/`, `post_census/`).

- `cargo test` green for every crate touched, on **both** engines:
  `genet-scripted-dom` (37), `script-runtime-api` (142 unit + 13 integration
  targets, the interleaving suite now **34** cases across Boa and Nova),
  `genet-scripted` (27 default / 29 with `scripted-nova`), `genet-wpt` (63).
- `cargo clippy --all-targets` reports **nothing** in any of the four crates
  this lane touched. One pre-existing `clippy::modulo_one` **deny** in
  `harness.rs` (`GC_TURN_INTERVAL` is 1) is silenced with a reasoned `allow`
  rather than by changing the tunable.
- `cargo fmt` applied; it also picked up pre-existing drift in
  `ports/genet-wpt/src/render.rs` and `testdriver/input_events.rs` (an import
  order and two blank lines).
- `cargo check --workspace --features genet-wpt/netfetch` clean.
- **All fourteen checked testharness baselines at `unexpected=0`**, after
  forward-only repins of **six** subsets: `dom`, `dom/nodes`,
  `html/webappapis/timers`, `css/css-position`, `css/css-animations`,
  `css/css-values/tree-counting`. Every movement in them is a gain
  (`error -> fail`, `error -> pass`, `fail 0/1 -> fail 1/7`) or a neutral
  re-shape (`error -> no-results`), except `Range-selectNode.html`, whose four
  fewer subtests are explained above.
- **Both reftest guards at `unexpected=0`**: `css/mediaqueries` (16 passed, 40
  failed) and `css/css-position` (45 passed, 73 failed).
- **Ortet receipt unchanged.** `cargo run -p ortet -- --url
  ports/ortet/examples/article.html --frames 3`: engine `genet.livery`, backend
  livery, 3 frames at 960x640, digest `0x6377ba8a6bf4dbc9` — identical to part
  one's. The whole frame was examined, not only the changed feature: heading,
  italic lede, link run with its separator, the rule, section heading, body text
  and the gradient swatch all render as before. Ortet's route is script-free, so
  this is the control that the new parse costs it nothing — and it is now a
  *stronger* control than in part one, because the Livery-backed scripted
  document moved onto the interleaved parse in this part.

## Residuals

Part one's residuals 1, 2, 3, 4 and 7 are **closed** by this part. What is
still open:

1. **`document.open(url, name, features)` throws `NotSupportedError`.** That
   form is `window.open`, and the scripted tier has no browsing context.
   (Part one residual 5, unchanged.)
2. **Quirks mode is recorded but dropped.** (Part one residual 6, unchanged.)
3. **`async` external scripts run at the pause.** HTML runs them "as soon as
   available" as a task; the resource route is synchronous, so genet runs them
   where they are found. `execution-timing/112` — removing `async` at runtime
   from a script that also has `defer` — measures the difference and fails.
4. **A script element is not re-prepared when its `type` becomes valid.**
   `svg/scripted/script-invalid-script-type.html` sets a valid `type` and
   appends a text node after parsing, which HTML says re-prepares and runs the
   script. The DOM insertion-step machinery that would do this does not exist.
5. **A script created without a browsing context.** `execution-timing/120`.
6. **A foreign-namespace `<script>` runs at the next pause, not at its own end
   tag.** html5ever pauses only for HTML-namespace scripts, so the driver runs
   the collected foreign scripts at the next pause and again after the parse.
   That is before the next HTML script — the ordering the SVG corpus depends on
   — but not exactly where HTML puts it.
7. **The XML corpus still parses then runs.** `xml5ever` has no
   pause-at-a-script driver.
8. **`genet-scripted`'s main test module does not compile.**
   `#[cfg(all(test, feature = "render"))] mod tests` is gated on a feature the
   crate does not have, so ~1,800 lines of Livery and layout receipts have not
   run. Reviving it is its own lane; the code under it has drifted.
9. **No headed receipt for interleaving.** Ortet's default route is
   script-free. (Part one residual 10, unchanged.)
10. **A parse still costs one extra static parse — on paper.**
    `ScriptedDocument::build`'s second parse is inside `#[cfg(feature =
    "render")]`, which is the same non-existent feature as residual 8, so it is
    currently dead code rather than a cost. `LiveryScriptedDocument::build` no
    longer has one at all.

## Regression manifest

The named suites that must keep passing, and what each one holds. Part one's
manifest still applies; this part adds and extends:

| Suite | Holds |
|---|---|
| `script-runtime-api --test parser_script_interleaving` (**17 bodies, 34 cases** across Boa and Nova) | part one's eleven, plus: a written `<script>` runs synchronously inside `document.write`; the open stream appends (node identity across two writes, a tag split across two writes); a parsed element upgrades at creation (`1,2,3`, not `3,3,3`); a during-parse write is visible to its own script and still lands at the insertion point; an SVG script runs before the next HTML script; a script in template contents does not run |
| `genet-scripted --lib livery_text_fragment_tests::livery_stylesheets_attach_where_they_are_parsed_on_boa` | the two-phase CSSOM, positionally: 0 sheets before the `<style>`, 1 and the styled colour after it |
| `genet-scripted-dom --lib parser::tests` | the tree sink and the pause driver, including the `Created` pause arm |
| `check-testharness-baselines.ps1` (14 slices), `check-reftest-baselines.ps1` (2) | `unexpected=0` |
| the Ortet receipt, digest `0x6377ba8a6bf4dbc9` | the script-free render path is untouched |
| `shadow-dom/declarative/declarative-with-disabled-shadow.html`, `innerhtml-on-ordinary-template.html` | the two files this lane exists to recover, now `pass` in WPT and not only at the engine level |

## Progress

**2026-09-08 — part two landed.** The WPT runner parses interleaved with
`testharness.js` as a prelude and a single `load` dispatch;
`LiveryScriptedDocument::build` and the runner's rendering session both use a
two-phase `LiveryCssom` that resolves author sheets from the live arena as the
parser fills it; `document.write` tokenizes its own source inside the call, so
written markup is visible to the writing script and a written `<script>` runs;
the post-parse `document.open` stream appends through one live tokenizer;
parse-time custom-element upgrades run at creation; foreign-namespace scripts
run; scripts in template contents do not; `is_mathml_annotation_xml_integration_point`
is answered; `type="MODULE"` and `type=""` classify correctly; and the arena
carries a `parsing` flag so an `innerHTML` from a parser-run script cannot free
the tree builder's own handles. **+181 subtest passes over the named subset**
with two explained `pass -> fail`; **+439 across the 79-directory disk census**
once two identified timing artifacts are removed, with all 19 census
`pass -> fail` attributed. Six testharness baselines repinned forward-only; all
fourteen at `unexpected=0`; both reftest guards at `unexpected=0`; the Ortet
digest unchanged. Receipts under
`Code/testing/genet/wpt-ledger/2026-09-08_interleaving_part_two/`.

## Findings — 2026-09-08, `genet-scripted` test-surface cleanup

Part two's residual 8 ("`genet-scripted`'s main test module does not
compile") and residual 10 (the second-parse cost being dead code under the
same phantom feature) are **closed**. A separate lane revived the crate's
`#[cfg(all(test, feature = "render"))] mod tests` in `document.rs` and the
matching `#[cfg(feature = "render")]` production code in `document.rs` and
`capture.rs` — the `render` feature this crate never declared. Scope was
`genet-scripted`'s `Cargo.toml`, `document.rs`, `capture.rs`, `livery.rs`, and
their tests only.

- **`ScriptedDocument` is not dead — only its render half was.** Only the
  fields/methods explicitly gated `#[cfg(feature = "render")]` (a
  `ComputedStyleBridge`/`MediaQueryBridge` pair, `frame`,
  `frame_with_external_textures`, `links`, the text-selection trio,
  `dom()`, `scroll_by`, `scroll_for_key`, `click_at`, `scroll()`,
  `last_layout_batch_stats`, plus the render-only struct fields backing
  them) were unreachable. `ScriptedDocument::parse`/`load`/`from_body`,
  `evaluate`, `dispatch_event`, `pump`, `extract`, `console`, `dom_snapshot`,
  and the WebGL-factory constructors were never gated at all — `Runtime::
  set_webgl_factory` is called unconditionally in `build`. So `ScriptedDocument`
  is a real, live, intentionally headless route (script execution + DOM
  mutation + render-free extraction — see its module doc, which undersold this
  as "GPU-free and testable" when it is actually the only mode this type has
  now). Also, `genet_layout`/`genet_render` were never even declared as
  dependencies in `Cargo.toml` — the render-gated code could not have compiled
  even if the feature had existed.
- **Two Livery receipts were trapped inside the dead module and never ran.**
  `livery_scripted_document_owns_live_cssom_resources_and_frame_on_boa` and
  `livery_scripted_button_hit_dispatches_to_its_listener_on_boa` already
  targeted `LiveryScriptedDocument` (the live route) but sat inside
  `#[cfg(all(test, feature = "render"))] mod tests`, so the `render` gate
  silently swallowed them too. Relocated verbatim into a new, ungated
  `livery_render_tests` submodule; both pass.
- **`capture.rs`'s render-gated code was a "shadow layout" for the retired
  route by name** (`shadow_layout: IncrementalLayout<NodeId>`,
  `fragment_digest` over a `genet_layout::FragmentPlane`) — exactly the
  "shadow layout" / "fragment planes" the task's delete criteria named.
  Deleted outright: `shadow_layout` field, `new_shadow_layout`,
  `stylesheet_refs`, `fragment_digest`, both hash helpers, and
  `RecordedLayoutBatch::capture` / `From<Applied> for RecordedApplied`.
  `record_pending` now always writes `layout: None`; `RecordedLayoutBatch`'s
  struct/enum shapes are kept in the wire format for backward-compatible
  deserialization of old capture files, just never constructed.
- **Two headless script-timing tests were finally run for the first time and
  turned out wrong, not my port.** Once un-gated (both are plain
  `ScriptedDocument` tests, untouched otherwise):
  - `async_runs_after_parser_blocking` expects `["inline", "async"]`, but
    `script-runtime-api`'s own `ScriptTiming::Async` doc says a synchronous
    fetcher makes `async` run "at the pause" like `Blocking` — the observed
    `["async", "inline"]` matches the documented design. The test's
    expectation was never correct.
  - `module_imports_dependency` and `module_import_diamond_loads_shared_once`
    fail to fetch: `DocumentScriptLoader::resolve` (in `document.rs`) routes a
    module's relative `import` specifier through `crate::resolve_href`
    (`lib.rs`), which is plain string-prefix concatenation and never collapses
    a leading `./` — `resolve_href("http://x/main.js", "./dep.js")` yields
    `"http://x/./dep.js"`, which does not string-match a fixture's
    `"http://x/dep.js"` key. `lib.rs`'s own doc comment on `resolve_href`
    claims "Module resolution uses `url::Url::join` separately where
    normalization is required" — that path is not actually wired up.
  - All three left `#[ignore]`d with the reason inline, per instruction not to
    weaken assertions to force a pass; fixing `resolve_href` or the module
    resolver is out of this cleanup's scope (and touches `lib.rs` /
    `script-runtime-api`, neither owned here).
- **A genuine capability gap, not a bug: Livery has no `MediaQueryHandler`.**
  `LiveryCssom::install_live_with_optional_sink` installs a
  `ComputedStyleHandler` (`LiveryComputedStyle`) but never
  `set_media_query_handler`, unlike the retired `MediaQueryBridge`. Ported
  `match_media_evaluates_against_the_frame` onto `LiveryScriptedDocument`
  anyway, `#[ignore]`d with that reason, so the gap has a compiling receipt
  instead of silently missing coverage. `from_body_wires_document_cookie` and
  the WebGL-factory test needed no such treatment — both are `ScriptedDocument`
  (headless) capabilities, ungated, and ported unchanged.
- **`LiveryScriptedDocument::click_at`'s bool means something different from
  the retired `ScriptedDocument::click_at`'s.** The old one returned "did the
  default action move the scroll"; Livery's returns "did the click hit
  anything at all" (`ScriptedClick::Miss` vs. not) — a `preventDefault`
  listener still counts as a hit. `prevent_default_blocks_anchor_nav` was
  ported to assert directly on `scroll()` rather than reusing the return value
  as a "scrolled" stand-in, with a comment recording the semantic difference.

**Disposition.** 40 render-gated generic test scenarios (many run on both Boa
and, where `scripted-nova` was already covering them, Nova): **34 ported**
(25 stayed on headless `ScriptedDocument` essentially unchanged — three of
those newly `#[ignore]`d for the findings above — and 9 moved onto
`LiveryScriptedDocument`, one of those `#[ignore]`d for the `MediaQueryHandler`
gap), **2 relocated** unchanged out of the dead module (the trapped Livery
receipts), **5 deleted** as specifically testing the retired genet-layout
route: `dom_and_layout_stats_surface` (`last_layout_batch_stats` /
`LayoutApplyKind` — incremental-layout receipts),
`canvas_external_texture_metadata_reaches_the_frame` (the
`frame_with_external_textures`/`RenderedFrame` external-texture side channel,
which has no Livery equivalent), and `transition_interpolates_via_get_computed_style`,
`transition_events_dispatch_to_listeners`, `animation_events_dispatch_to_listeners`
(all three hand-drive a `genet_layout::IncrementalLayout` directly —
`tick_animations`/`take_transition_events`/`has_active_animations` — with no
Livery transition/animation machinery to port onto at all). The
`unexpected_cfgs` allowance and every `#[cfg(feature = "render")]` gate are
gone from `Cargo.toml`, `document.rs`, and `capture.rs`. `cargo test -p
genet-scripted` is green (60 passed, 4 ignored, each with a reason);
`cargo clippy -p genet-scripted --all-targets` adds no new warnings.
