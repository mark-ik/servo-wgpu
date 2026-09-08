# iframes and nested browsing contexts

**Date:** 2026-09-08

**Status:** landed 2026-09-08 for the browsing-context tree, loading, the
script-free composite and the Window's own view of its children; the second
`Runtime` per context and the cross-origin `WindowProxy` are **named residuals
and a decision for Mark**, recorded in "Residuals" and "What is Mark's" below.

**Parent:** [Deferred web platform lanes: scoping](2026-09-07_deferred_web_platform_lanes_scoping.md),
section "iframes and nested browsing contexts". That document's five-part
ordering is this plan's phase order, and its shared-acceptance section is the
receipt shape used here.

**Related:** [Ortet](2026-09-03_ortet_founding_plan.md) (the headed host and the
digests below), [Dedicated Worker](2026-09-07_worker_plan.md) (the second
`Runtime` and the structured-clone wire this lane's script phase reasons
against), [parser/script interleaving](2026-09-08_parser_script_interleaving_plan.md)
(a child document loads through the same interleaved parse), and
[reflector identity](2026-09-07_reflector_identity_plan.md).

**Genet commit at lane start:** `72fccdb65fd`.

---

## What this lane is, and what it is not

`<iframe>` was a replaced box with width and height hints in Livery, a pair of
stubs in the bootstrap, and nothing at all in `genet-documents`, which held one
retained session per document with no parent link. A nested document had
nowhere to *be*.

This lane gives it somewhere: a browsing-context tree, a loading path through
the parent's own resource route, and a composite that puts the child's scene in
the parent's replaced box. Mark's ruling shaped the boundary:

> One script `Runtime` per browsing context; Ortet is extended to composite
> child frames, **through the script-free Livery route only**, so the Ortet
> receipt is a static parent with a static child; script-driven frames are
> proved in the WPT lane, and a headed script-driven receipt stays a named
> residual until the scripted Ortet route exists.

So the rendering half landed on the Livery route and is proved headed; the
script half landed as far as one `Runtime` honestly reaches, and the rest is
named rather than faked. §4 says exactly where that line falls and why it is an
architectural decision rather than an unfinished task.

---

## 1. The browsing-context tree

`components/genet-documents/src/browsing_context.rs` (new). Pure data with no
engine dependency, so both routes can share it.

### Shape

| Type | What it holds |
|---|---|
| `BrowsingContextTree` | A slot arena of contexts plus the top-level id and the counter that mints opaque origins. |
| `BrowsingContextId` | `{ index, generation }`. The generation is why `discard` can reuse a slot without a stale id silently addressing its replacement. |
| `BrowsingContext` | parent, ordered children, container `opaque_id`, name, active document, session history, sandbox flags, `allow`, `loading`. |
| `ActiveDocument` | URL, origin, and **`initial_about_blank`** — the bit, not the URL, that decides origin inheritance and whether the first real load replaces or pushes. |
| `SessionHistory` | HTML's per-context history: entries plus a current index, with `push` truncating the forward list. |
| `Origin` | A tuple origin or an opaque one carrying a serial. |
| `SandboxFlags` | The fourteen flags, parsed from the attribute's *inverted* token list. |

### Three decisions worth their own line

**The tree is data, not a session.** It stores identity and policy; it does not
own the engine session that renders the document. A host joins the two by
`BrowsingContextId`. That is what lets the script-free Livery route and the
scripted route share one tree without either importing the other.

**An opaque origin carries a serial.** HTML's opaque origin is same-origin with
*nothing but itself*, including another opaque origin minted a moment later. A
unit variant would make two `about:blank` documents same-origin, which is
exactly the check a sandbox test looks for. `Origin::Opaque(u64)` with a
tree-owned counter makes the wrong answer unrepresentable.

**`initial_about_blank` is a bit, not a URL test.** A document can navigate to
`about:blank` deliberately, and that one is *not* the initial one: it does not
inherit its container's origin and its successor pushes rather than replaces.
Keying either rule off the URL string gets both wrong.

**Sandbox parsing is inverted, and inheritance is a union.** `sandbox` with no
tokens is the *most* restrictive value; each token *unsets* a flag. A child
inherits its parent's flags and adds its container's, so an `allow-scripts`
token cannot take back what an ancestor already took — the union direction is
the whole rule and is asserted directly.

Ten unit tests cover the origin serializer (default ports, IPv6 literals,
userinfo, case folding), opaque non-matching, the sandbox inversion and
inheritance, `loading`, the replace-then-push history join, `pushState`
truncating the forward list, `go` refusing to leave the range,
`parent`/`top`/`frames`/named lookup, and child-first discard with slot reuse.

### Cross-context policy

`components/genet-documents/src/frame_policy.rs` (new) holds the *decisions*
taken from those values, so a caller cannot re-derive "may I touch this" three
different ways: `DocumentAccess::between`, the `CROSS_ORIGIN_WINDOW_PROPERTIES`
list as data, `target_origin_matches`, the three sandbox gates, and
`CrossOriginIsolation` for COOP/COEP.

---

## 2. Loading

`components/genet-document-resources/lib.rs` gained `ResolvedFrame` and a
`frames: Vec<ResolvedFrame>` on `ResolvedDocumentResources`, discovered by the
same walk that finds images — so **a child document is fetched through exactly
the resource route its parent used**, with the same limits, the same cache and
the same URL resolution, rather than through a second path that could diverge.

| Case | Behaviour |
|---|---|
| `srcdoc` | Wins over `src`. Document URL `about:srcdoc`, **base URL the parent's** — resolving its relative links against `about:srcdoc` would break every one of them. No fetch. |
| `src` | Resolved against the document URL, fetched through the shared closure, `final_url` taken from the response so a redirect is recorded. |
| absent or empty `src` | The initial `about:blank` stands. A `load`, not an error. |
| `src` that misses | `failed`, and the element reports `error`. |
| `loading="lazy"` | Discovered, context created, **not fetched**: the element and its context exist, only the load is deferred. |
| inside `<template>` | Absent, and nothing checks for a template — a template's contents have no parent by construction, so no walk of the document reaches them. |

Nesting is resolved one level up, in `components/genet-documents/src/engines/frames.rs`,
because recursion needs an HTML parser and `genet-document-resources` takes a
`LayoutDom` it is given rather than parsing one. Two guards: HTML's
"matching nested browsing contexts" rule (a `src` already open in an ancestor
loads `about:blank` instead) and a depth bound of 8 for a page that nests
distinct URLs without end.

**One named degradation.** `LiverySessionEngine::spawn_prepared` — the
asynchronous host's staged path — has no fetcher at the point the children are
built. Its frames render from the sources the staged resolution already
fetched, but their *own* linked stylesheets and images stay undiscovered. The
staged resolution has no frame stage yet; that is the residual, and it is a
parameter (`Option<&dyn ResourceFetcher>`) rather than a silent difference.

---

## 3. Rendering: the composite

### The decision — a paint-list splice, not a producer texture

`paint_list_api` names "embedded iframe output" as an `ExternalTextureItem`
candidate, and the scoping document asked this lane to use the producer-texture
path "if it fits, or say why a scene-level composite is better". It does not
fit, for three reasons, and the reasons are about where the composite happens
rather than about effort:

1. **A child browsing context produces a paint list, not a texture.** Taking
   the texture route means rasterizing every child to an offscreen target every
   frame, which needs a device — so the reftest lane's software comparison and
   Ortet's own capture would see an empty box.
2. **External-texture draws are composited after the scene.** `RenderedFrame`
   carries them beside the scene and `genet-render-host` composes them in a
   separate pass, which puts them outside the scene's clip and transform stack:
   the parent's `overflow`, `border-radius`, ancestor transforms and stacking
   order would not apply to the child.
3. **Livery already records the right position.** `HostLeafSlot` marks a custom
   leaf's place *while its ancestors' clips, transforms and stacking context are
   live*, and `splice_host_leaf_slots` fills it. A child's paint commands are
   exactly such a stream.

So `<iframe>` records a **`FrameSlot`** in the same two paint-walk call sites,
and the host splices the child's commands into it. The texture route stays
right for a child produced on another thread or device, and nothing here
forecloses it: `FrameSlot` carries the destination rectangle either way.

`FrameSlot` is a **separate list** from `HostLeafSlot` rather than another
`custom-leaf` key, because the two key spaces are unrelated — a custom leaf's
key is an author attribute and a frame's is the node's `opaque_id` — and
sharing one `u64` space would let a page collide with a frame by writing an
integer into an attribute.

### What the splice does

```
PushClip(content_box)  PushTransform(content_box.origin)  <child commands>  PopTransform  PopClip
```

The clip is what makes the frame a *viewport* rather than a hole. The recorded
rectangle is the element's **content box**, not the fragment's border box: it is
both the child's viewport and the clip, and the user-agent sheet's 2px inset
border makes the two differ by default. `content_box_rect` is the origin half of
the existing `content_box_size`.

### The one thing that does not merge for free

Font keys are **content-hashed** (`text::content_key`), so the same face has the
same key in every list and merging is a dedupe. Image keys are **per-list
ordinals** (`image_key_for` allocates `images.len() + 1`), so the child's key 1
and the parent's key 1 are different pictures. `absorb_frame_commands` re-keys
every child image and rewrites every command that names one — `DrawImage`,
`DrawRepeatingImage`, a nine-patch `DrawBorder`, and a `PushLayer` image mask.
Splicing without that step does not fail loudly: it draws *the parent's* image
inside the frame.

### `<iframe>` became a real replaced element

Three copies of `is_replaced_element` (box tree, layout, text) gained `iframe`,
box generation skips a frame's fallback children (a frame's content is its
nested context; generating boxes for the fallback would both paint it and stop
the element being a measured leaf), and sizing takes a new early branch:

> A frame has a **default object size** (300x150) and **no natural size or
> ratio at all**. That distinction is the whole reason it cannot go through the
> natural-size path: with a ratio, `width: 600px; height: auto` would be
> 600x300; with only a default object size it is 600x150, because the two axes
> never speak to each other.

### Scroll, hit testing, and the slot indices

- The child's own scroll is a translation its own paint list already carries.
- `LiveryDocumentSession::hit_test_frames` descends, returning the innermost
  context **and** the node within *that* context's document, so a caller never
  has to guess which document a returned node belongs to.
- Every recorded slot is an index into `commands`, so `translated`,
  `scaled_to` and both splice methods shift the outstanding slots. A slot
  recorded before a page-zoom wrap that did not move would put the child one
  command too early.

### Two hosts, one seam

The reftest lane builds a bare `LiveryDocument` rather than going through the
session engine, so `ports/genet-wpt/src/render.rs` assembles the same composite
independently. Both routes use the same three engine seams — `frame_slots`,
`absorb_frame_commands`, `splice_frame_slots` — which is the point of putting
them on the paint list rather than in either host.

---

## 4. Script, and the line this lane did not cross

### What landed

| Surface | Before | After |
|---|---|---|
| `window.frames` | `globalThis` | `globalThis` — this was already right; `frames` **is** the window. |
| `window.length` | `undefined` | The number of child browsing contexts, live, `[Replaceable]`. |
| `window[i]` | absent | The i-th child's `contentWindow`, refreshed with the named properties and **taken back** when a frame leaves the document. |
| `window.frameElement` | absent | `null` — correct for a top-level context, which every document this runtime hosts is. |
| `postMessage` `targetOrigin` | ignored | Enforced: `*` and `/` always deliver, anything else is compared as a serialized origin and a mismatch **discards silently**, after the clone. |

The count is taken from the document rather than from a registry, and that is
load-bearing: a nested browsing context exists only for an `<iframe>` in the
document tree, and a `<template>`'s contents have no parent, so a templated
frame is correctly absent with nothing checking for a template.

Twenty tests, ten bodies on both backends. Both-engine coverage matters more
than usual here because `length` and the indices are properties **of the global
object**, and Boa's globals are writable while Nova's are not.

### The residual, and why it is a decision rather than a task

Mark's ruling is **one script `Runtime` per browsing context**, and `Runtime`
already supports that: it owns its engine and its own `ScriptedDom`, and
`Runtime::new_worker` proves a second one can be constructed with a different
global shape. Building a second `Runtime` for a child document is not the hard
part.

The hard part is what `contentWindow` then *is*. HTML requires a same-origin
parent to reach a child's real object graph synchronously —
`iframe.contentWindow.document.getElementById(x)` returns a node with the same
identity the child's own script sees. Two `Runtime`s are two **engine
instances**, and this stack's cross-instance boundary is string-marshalled:
`CallCx` carries strings, which is why the Worker lane's cross-agent wire is a
JSON encoding of the clone record rather than a shared object graph. A
marshalled `contentWindow` can carry `postMessage` and the cross-origin member
list faithfully; it cannot carry same-origin object identity.

The alternative is **one engine instance with two realms**, which is how a
browser does it — and `ScriptEngine` has no realm concept today. Whether to add
one is a change to the engine-neutral trait and to both backends.

So the honest statement is: the per-context `Runtime` is buildable, and the
`WindowProxy` it would hang off cannot be correct without a realm decision.
This lane therefore **did not dress up `contentWindow`**. It stays the existing
stub. The `Node-baseURI` lesson is the reason: a `contentWindow` that answers
`parent`, `top` and `frameElement` plausibly while its `document` is a
different, empty document would convert honest failures into false passes
across `html/browsers/the-window-object`, and the score would improve while the
engine got no closer.

`document.domain` is a separate residual, as the scoping document already
named.

---

## 5. Security policy, as far as one process allows

**Enforced**, because each is a check this engine performs before it acts:
same-origin document access (`DocumentAccess::between`), `postMessage` origin
checks, the sandbox flags that gate our own behaviour, and origin inheritance —
a sandbox without `allow-same-origin` gives the child a fresh opaque origin that
matches nothing, asserted end to end through a real session.

**Recorded, not enforced.** COOP and COEP parse into `CrossOriginIsolation` and
each carries a named residual from `CrossOriginIsolation::residual`. Their
observable effect is that a cross-origin document lands in a **different agent
cluster** — a separate process with separate memory, so `SharedArrayBuffer`,
high-resolution timers and Spectre-adjacent reads cannot cross. Parsing them and
recording what they ask for is honest; claiming to enforce them in one process
would not be. `self.crossOriginIsolated` is deliberately not reported as true.

The cross-origin `WindowProxy` member list exists as data
(`CROSS_ORIGIN_WINDOW_PROPERTIES`) and is tested, but nothing consults it yet:
there is no cross-origin `WindowProxy` to guard until §4's decision is taken.
That is stated here rather than left for someone to discover.

---

## Receipts

### Runners and configuration

Both built with `CARGO_TARGET_DIR=C:/t/laneI-target`,
`cargo build --release -p genet-wpt --features netfetch`.

| Runner | SHA-256 |
|---|---|
| `pre` — built from `72fccdb65fd`, **before the first edit** | `da25e8bd6be1c9c881e4187ab705649fea50f884a595203ddfa1d65680c5f708` |
| `post` — the lane's tree | `d14c8c9886f146a2c9ea3c8904ff95278c86a68ec66cc240271fae5a6309b60b` |

`Cargo.lock` as generated by the `pre` build:
`15cff0a0d52248d2c14a747084818169ace33c4ad7f3d2cfcee1ae58bf25788e`.
Disk mode, `--engine boa --renderer livery`, `--jobs 8 --timeout 240` for the
testharness lane; the reftest lane is sequential with no job flag. Raw maps and
drivers under `Code/testing/genet/wpt-ledger/2026-09-08_iframes/`.

### Testharness lane, `pre` -> `post`

Files as all-pass / with-failures / errored / no-results / skipped, then
subtests passed of total.

| Directory | Files, pre | Files, post | Subtests, pre | Subtests, post |
|---|---|---|---|---|
| `html/semantics/embedded-content/the-iframe-element` | 8 / 136 / 0 / 0 / 20 | 8 / 136 / 0 / 0 / 20 | 17 / 217 | 17 / 217 |
| `html/browsers/windows` | 0 / 38 / 0 / 13 / 9 | 0 / 38 / 0 / 13 / 9 | 2 / 109 | **4** / 109 |
| `html/browsers/the-window-object` | 4 / 74 / 0 / 16 / 2 | 4 / 74 / 0 / 16 / 2 | 62 / 609 | **69** / 609 |
| `html/browsers/origin` | 2 / 42 / 0 / 71 / 1 | 2 / 42 / 0 / 71 / 1 | 2 / 237 | 2 / 237 |
| `html/browsers/sandboxing` | 0 / 19 / 0 / 0 / 1 | 0 / 19 / 0 / 0 / 1 | 0 / 28 | 0 / 28 |
| `webmessaging` | 65 / 89 / 0 / 6 / 0 | **67** / 87 / 0 / 6 / 0 | 158 / 276 | **160** / 276 |
| `html/dom` | 37 / 196 / 0 / 14 / 140 | 37 / 196 / 0 / 14 / 140 | 42,331 / 60,042 | 42,331 / 60,042 |
| `dom` | 223 / 366 / 8 / 50 / 51 | 223 / **367** / 8 / **49** / 51 | 46,370 / 57,192 | **46,382** / 57,193 |

**+23 subtest passes, two files `fail -> pass`, zero `pass -> fail`.**

- `webmessaging/with-ports/015.html` and `webmessaging/without-ports/015.html`
  `fail -> pass`: both are the `targetOrigin` tests.
- `dom/nodes/MutationObserver-cross-realm-callback-report-exception.html`
  `no-results -> fail`: a cross-realm test that reached `frames[0]`, found
  `undefined`, and died before its first subtest. It now gets far enough to
  report one honest failure. Forward movement in reporting, not a regression.

**`the-iframe-element` did not move, and that is the expected result, not a
disappointment.** The testharness runner drives the *scripted* route, and this
lane's rendering work is on the Livery route; the directory's failures are
`contentDocument` / `contentWindow` demands, which §4 deliberately did not
fake.

### Reftest lane, `pre` -> `post`

| Directory | Pre | Post |
|---|---|---|
| `html/semantics/embedded-content` | 6 passed (6 verified), 8 failed, 784 skipped, 0 errored (of 798) | identical |
| `html/rendering/replaced-elements` | 26 passed (26 verified), 23 failed, 51 skipped, 0 errored (of 100) | identical |

**Zero movement, and the instrument was proved before that was believed.** Four
`frame_composite_tests` in `ports/genet-wpt/src/render.rs` drive the reftest
lane's own `build_child_documents` / `composite_children` over real fixture
files and assert the child's paint reaches the parent's list under a clip to the
frame's content box. The composite runs; these directories simply do not
exercise it. Of `html/semantics/embedded-content`'s 798 files, 750 are
non-reftest and 34 need script; **two** iframe reftests run at all —
`iframe-with-base.html` (passing before and after) and
`iframe_sandbox_iframe_pdf_viewer.html` (failing before and after, a PDF
viewer). A reftest whose reference also renders through this engine can pass
with both sides blank, so `iframe-with-base.html` passing is not evidence
either way.

### Ortet

`cargo run -p ortet -- --url <page> --frames 3 --artifact <png>`,
engine `genet.livery`, backend `livery`, 3 frames at 960x640.

| Page | Digest |
|---|---|
| `ports/ortet/examples/article.html` | `0x6377ba8a6bf4dbc9` — **unchanged**, and byte-identical to the same page rendered by an Ortet built from `72fccdb65fd` in a separate worktree. |
| `ports/ortet/examples/frames.html` (new) | `0x97bdd4bd9e03ec02` |

Both digests are **stable across three consecutive runs**, which had to be
established rather than assumed: see the determinism finding below.

The new page is a static parent with six static children, no script anywhere.
Examining the whole captured frame: the header and both caption rows render;
row one is a `src` child (clipped at the frame's right and bottom edges, with a
partial glyph row visible at the cut), a `srcdoc` child styled by a stylesheet
that resolved against the *parent's* base URL, and a two-level nest whose inner
frame's own 1px border is visible inside the outer child; row two is a `src`
that misses (an empty box, the parent unaffected), a frame with no `src` holding
the initial `about:blank`, and a sandboxed frame, which renders normally because
sandboxing takes the origin and the script permission, not the pixels.

New example files: `frames.html`, `frames-child.html`, `frames-nested.html`,
`frames-child.css`. The captured PNGs are archived beside the maps as
`ortet-article.png` and `ortet-frames.png`.

**The first version of this page produced a different digest on every run**, and
the composite was not the cause. Three runs gave three digests; two of the
captures differ by **one pixel**, at a glyph edge in a 9px caption in the
parent's own text, outside every frame. The control settles it: the same page
with all six `<iframe>`s replaced by same-sized `<div>`s is equally unstable,
while `article.html` is byte-stable across three runs. Genet's glyph
rasterization is not deterministic at very small sizes on this device. The page
was rebuilt at 13px body and 11px captions and is now stable; the small-size
nondeterminism is recorded as a finding and is not this lane's to fix. **An
Ortet receipt taken from a page with sub-10px text is not a receipt** — take the
digest three times before writing it down.

Ortet itself needed **no code change**. It already selects
`LiverySessionEngine` and spawns through the synchronous route, so extending the
engine extended the host.

### Gates

| Gate | Result |
|---|---|
| `cargo test -p genet-livery` | green (32 suites) |
| `cargo test -p genet-document-resources` | green, 18 tests, 6 new |
| `cargo test -p genet-documents --features livery` | 58 pass, 13 new; 2 **pre-existing** failures (below) |
| `cargo test -p script-runtime-api` (both engines) | green, 20 new tests in `tests/browsing_contexts.rs` |
| `cargo test -p genet-wpt --features netfetch` | green, 4 new composite tests |
| Table drift (`generated_table_is_current`) | green |
| `cargo check --workspace --features genet-wpt/netfetch` | green |
| `cargo clippy` on the touched packages | no new warnings; every remaining one is pre-existing |
| `cargo fmt` on the touched packages | clean |
| `check-testharness-baselines.ps1` (14 slices) | `unexpected=0` after three forward repins |
| `check-reftest-baselines.ps1` (2 slices) | `unexpected=0`: `css/mediaqueries` 16 passed / 40 failed, `css/css-position` 45 passed / 73 failed |

**Repins, forward only, by subset and name.** Three baselines, six distinct
entries, and a diff against the `72fccdb65fd` copy of each file confirming that
**nothing else moved**.

`dom_boa.json` and `dom_nodes_boa.json`, the same four entries in each:

| Test | Before | After |
|---|---|---|
| `dom/nodes/MutationObserver-cross-realm-callback-report-exception.html` | `no-results` | `fail` 0/1 |
| `dom/nodes/Node-appendChild.html` | `fail` 5/11 | `fail` 6/11 |
| `dom/nodes/Node-removeChild.html` | `fail` 18/28 | `fail` 27/28 |
| `dom/nodes/insertion-removing-steps/insertion-removing-steps-iframe.window.html` | `fail` 1/4 | `fail` 3/4 |

`html_webappapis_timers_boa.json`, two entries:

| Test | Before | After |
|---|---|---|
| `html/webappapis/timers/setinterval-cross-realm-callback-report-exception.html` | `no-results` | `fail` 0/1 |
| `html/webappapis/timers/settimeout-cross-realm-callback-report-exception.html` | `no-results` | `fail` 0/1 |

Every one is `window.length` / `window[i]` reaching a test that previously found
`undefined` where a child context should be. The four `no-results -> fail`
entries are all *cross-realm* tests: each reaches `frames[0]` in its setup, got
`undefined`, and died before its first subtest. They now report one honest
failure each — which is exactly the shape §4 refuses to paper over, and the
reason `contentWindow` was left alone. The `dom` pair accounts for the whole of
that directory's +12 subtest delta.

---

## Named regression manifest

Everything below was measured on this machine, with this lane's own runners,
and must hold on any re-run of this receipt.

1. **Ortet's article digest is `0x6377ba8a6bf4dbc9`.** The static reference page
   must not move. Verified against an Ortet built from `72fccdb65fd`, and
   re-verified three times on the final tree.
1b. **Ortet's frames digest is `0x97bdd4bd9e03ec02`**, and it is *stable*: three
   consecutive runs agree. A run that disagrees is either a composite change or
   the small-text rasterization nondeterminism above — render the page with the
   frames replaced by plain boxes to tell them apart.
2. **Zero `pass -> fail`** across the eight testharness directories and the two
   reftest directories.
3. **The two reftest maps are byte-identical** between `pre` and `post`.
4. **`webmessaging` holds at 67 all-pass / 160 of 276.** A regression in
   `targetOrigin` shows here first.
5. **`dom` holds at 223 all-pass / 46,382 of 57,193**, and the `dom`,
   `dom/nodes` and `html/webappapis/timers` baselines stay at `unexpected=0`.
6. **The four `frame_composite_tests` stay green.** They are the reftest lane's
   positive control; without them "the reftest maps did not move" means
   nothing.
7. **The twenty `browsing_contexts` tests stay green on both backends.** A
   global-object property that installs on one backend and silently does
   nothing on the other is the specific failure they exist against.
8. **`the-iframe-element` at 8 all-pass / 17 of 217** is the *floor* this lane
   leaves; it is where the per-context `Runtime` decision will show up.

### Two pre-existing failures, not this lane's

`genet-documents`' `livery_session_edits_and_submits_a_retained_get_form` and
`livery_session_rejects_inaccessible_native_text_value_replacements_and_text_edits`
fail on this machine **at `72fccdb65fd`**, verified by running them in a clean
detached worktree at that commit before this lane's first edit reached them.
Both are retained-text-geometry assertions and are most likely font-environment
dependent. They are recorded here so the next lane does not re-diagnose them.

---

## Findings

- **The producer-texture path and the paint-list splice differ in *where* the
  composite happens, not in effort.** An external texture is composed after the
  scene, so it is outside the scene's clip and transform stack and invisible to
  a software rasterizer. Anything that must be clipped by CSS, travel under a
  parent transform, or appear in a captured frame has to be *in* the paint list.
  `paint_list_api` naming iframes as a texture candidate is about a child
  produced on another device, not about this one.
- **Two resource key spaces, two different merge rules, and only one of them
  fails loudly.** Content-hashed font keys merge by dedupe; ordinal image keys
  must be re-keyed. Splicing a child without re-keying draws the *parent's*
  image inside the frame — a plausible wrong picture, not an error. Check how a
  key is minted before merging two lists that carry them.
- **A recorded index into a command stream is invalidated by every later
  insertion.** `HostLeafSlot` got away with it because nothing wrapped the list
  while a slot was outstanding. `translated` and `scaled_to` both insert at
  index 0, so the moment a second slot kind existed the bug was one page-zoom
  away. A slot is a position, and positions move.
- **A default object size is not an intrinsic size.** Giving `<iframe>` a
  300x150 "natural size" would give it a 2:1 natural ratio and make
  `width: 600px; height: auto` render 600x300. The two axes of a frame never
  speak to each other; that needed its own branch before every ratio rule, not
  a value threaded into them.
- **The frame count is a read of the document, and that is what makes
  `<template>` free.** A templated `<iframe>` has no browsing context. Because a
  template's contents have no parent, no walk of the document reaches one, so
  `window.length` is correct with nothing checking for a template — the Shadow
  DOM lane's encapsulation-by-shape rule paying a dividend in a lane that has
  nothing to do with shadow trees.
- **A cross-instance boundary that marshals strings cannot carry object
  identity, and same-origin `contentWindow` is an object-identity requirement.**
  This is the same wall the Worker lane hit and answered with a JSON clone wire.
  A worker can live with it because the specification's boundary is a message
  queue; a same-origin iframe cannot, because the specification's boundary is a
  shared object graph. The choice is realms in the engine or a permanently
  partial `contentWindow`, and it is Mark's.
- **Proving the instrument turned an uninformative zero into a fact.** The
  reftest maps did not move. Without the four composite tests that would have
  read as "the composite may not run at all"; with them it reads as "two iframe
  reftests exist in these directories and neither discriminates". The positive
  control cost four tests and is now a permanent guard.
- **A digest is not a receipt until it repeats.** This lane's first Ortet frame
  digest was different on every run. The instinct is to suspect the new code;
  the control said otherwise — the same page with the frames replaced by plain
  boxes was equally unstable, and the reference page was byte-stable. The cause
  is one pixel at a glyph edge in 9px text, in the *parent*, outside every
  frame. Rebuilding the page at 13px made it stable. Two lessons: take a digest
  three times before recording it, and when a receipt moves, render the same
  page **without** the feature before believing the feature moved it.
- **A monospace inline run that begins a wrapped line loses its glyphs, and it
  is not this lane's bug.** Found while examining the Ortet receipt: a `<code>`
  element wrapping to the start of a line paints its background and no text. The
  same fixture rendered by an Ortet built from `72fccdb65fd` produces the
  **identical digest** (`0x6f5ce0603f76aaa5`), so it is pre-existing in Livery's
  text lane. Recorded here; not fixed, because it is next door.

---

## Residuals

Named, in the order they would be taken up.

1. **A second `Runtime` per browsing context, and what `contentWindow` becomes.**
   §4. Blocked on a realm decision, not on effort. Everything below it waits.
2. **The cross-origin `WindowProxy`.** The member list is data and is tested;
   nothing consults it because there is no cross-origin proxy to guard.
   `SecurityError` on a denied member, `Location`'s two permitted members, and
   proxy identity across navigation all belong to (1).
3. **`document.domain`.** Carried forward from the scoping document.
4. **A headed script-driven receipt.** Mark's ruling: it stays a residual until
   the scripted Ortet route (O5) exists.
5. **`spawn_prepared` has no frame stage.** An asynchronous prepared spawn's
   children render without their own linked resources. §2.
6. **`loading="lazy"` never loads.** The context exists and the load is
   deferred forever: there is no intersection observation to trigger it.
7. **The `load` / `error` events are reported, not dispatched.**
   `frame_load_reports` is the contract; the Livery route has no script to
   dispatch to, and the scripted route needs (1).
8. **COOP, COEP and agent-cluster separation.** §5. Recorded with residuals;
   unenforceable in one process.
9. **Permissions Policy.** `allow` is stored verbatim; the container-policy
   algorithm is not implemented.
10. **Navigating a child.** `BrowsingContext::navigate` and the session history
    exist and are tested, but nothing in the Livery route calls them after the
    initial load — a child cannot follow a link yet.
11. **Focus, and input routing into a child.** Hit testing descends;
    keyboard focus and input dispatch do not.

---

## What is Mark's

**The realm decision (§4).** Three options, with what each costs:

- **Realms in `ScriptEngine`.** A realm concept on the engine-neutral trait plus
  an implementation on both backends. Buys a correct same-origin
  `contentWindow` with real object identity, which is what most of
  `the-iframe-element` and `html/browsers/the-window-object` actually test.
  Largest change, and it lands in the layer the whole scripted tier sits on.
- **A marshalled `WindowProxy` over two `Runtime`s.** Reuses the Worker lane's
  wire. Buys `postMessage`, the cross-origin member list, `parent`/`top`/
  `frames`, and cross-origin `SecurityError` behaviour — genuinely most of the
  *security* surface. Cannot buy same-origin synchronous DOM access, ever, and
  a partial `contentWindow` risks converting honest failures into false passes
  unless the same-origin path throws rather than answering plausibly.
- **Neither, for now.** The Livery composite and the Window's own view stand;
  the iframe script surface stays where this lane left it.

I did not choose. The second option is tempting because it is reachable, and it
is exactly the kind of choice that reads as progress on the score while leaving
the engine further from the specification than the number suggests.

---

## Progress

**2026-09-08.** Landed the browsing-context tree, cross-context policy, loading
through the shared resource route, the paint-list composite on both hosts, the
Window's view of its children, and the `postMessage` origin check. +23 subtest
passes over eight directories, two files `fail -> pass`, zero `pass -> fail`,
three baselines repinned forward with six named entries. Reftest lane byte-identical
with a positive control proving the composite runs. Ortet's article digest
unchanged at `0x6377ba8a6bf4dbc9`; new iframe receipt `0x97bdd4bd9e03ec02`.
The per-context `Runtime` and the `WindowProxy` are a decision for Mark, above.
