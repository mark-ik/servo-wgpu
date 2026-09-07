# Generating the scripted tier's interface table from WPT's WebIDL

**Date:** 2026-09-07

**Status:** landed 2026-09-07. `components/script-runtime-api` (the DOM table
and its bootstrap glue) plus a new offline generator at
`support/idl-interface-table`. No renderer, layout or network source touched.

**Parent:** [`2026-09-06_web_platform_wpt_census.md`](2026-09-06_web_platform_wpt_census.md);
measured against the [`2026-09-07_wpt_harness_repair_plan.md`](2026-09-07_wpt_harness_repair_plan.md)
ledger, which is this lane's baseline.

**Receipts:** `Code/testing/genet/wpt-ledger/2026-09-07_idl_interface_table/`.

## Purpose

`components/script-runtime-api/dom/html_interfaces.rs` carried a hand-written
declarative table: 65 HTML interfaces, their parents, their tag names and 277
reflected IDL attributes, all typed out by hand from the HTML standard. Every
row was a transcription that could be wrong, could drift as the standard moved,
and could only grow by more transcription. WPT already ships the machine
extraction of the same standard — `tests/wpt/tests/interfaces/*.idl`, produced
by Reffy from the spec — and it is vendored in this checkout. This lane makes
the table a build product of that vendored IDL, keeps the bootstrap's algorithm
glue where it is, and widens coverage as far as the generated data allows
without implementing new algorithms.

## Findings

Dated 2026-09-07, verified in this checkout at `7f0a3c80d29`.

### F1 — WebIDL alone cannot produce the table; two inputs are needed

WebIDL carries no tag names. `interface HTMLAnchorElement : HTMLElement` does
not say that `<a>` selects it. WPT ships the missing half as data too:
`html/semantics/interfaces.js` is a 151-row `[tag, interface]` array, and it is
the very file `html/semantics/interfaces.html` tests against. The generator
reads both, so the tag map is vendored rather than transcribed, and the file
that asserts the mapping and the file that produces it are the same data.

Three of that file's rows are deliberately not tag mappings: `""` means
`HTMLElement`, `"Unknown"` means `HTMLUnknownElement`, and the non-lowercase
rows (`foo-BAR`, `å-bar`) exercise the unknown-name fallback rather than a tag.

### F2 — the IDL marks reflection explicitly, and richly

The HTML IDL carries `[Reflect]` (254 uses), `[ReflectURL]` (19),
`[ReflectSetter]` (27), `[ReflectDefault=]` (10), `[ReflectRange=]` (4),
`[ReflectNonNegative]`, `[ReflectPositive]` and `[ReflectPositiveWithFallback]`,
plus `Reflect="content-attribute-name"` for the 13 cases where the IDL name and
the content attribute name differ (`relList` → `rel`, `httpEquiv` →
`http-equiv`, `acceptCharset` → `accept-charset`, and kin). Together with the
IDL type that is enough to derive the kind, the content-attribute name, the
missing-value default and the read-only flag mechanically.

`[ReflectSetter]` marks an attribute whose *setter* is the reflection setter
while the getter is prose. For the `USVString` cases (`href` on `<base>`, and
on `<a>`/`<area>` through the `HTMLHyperlinkElementUtils` mixin) that prose is
"resolve against the base URL", which is exactly what the runtime's `u` kind
already does, so those are generated. `[ReflectSetter]` on other types
(`tabIndex`, `autocomplete`, `<img>`'s rendered `width`/`height`) is not.

### F3 — what the IDL does not carry

HTML's **enumerated** attributes are plain `DOMString` in WebIDL with no
`[Reflect]` at all: the keyword set, the missing-value default and the
invalid-value default live in prose. `dir`, `crossOrigin`, `referrerPolicy`,
`decoding`, `loading`, `kind`, `scope`, `autocomplete`, `enctype`, `method`,
`formEnctype` and `<button>`'s `type` are all in this class. So are a handful
of prose-defined getters the runtime approximates by reflection (`text` on
`<title>`/`<a>`/`<option>`/`<script>`, `value` on `<input>`/`<textarea>`/
`<option>`, `<canvas>`'s `width`/`height` defaults). These are the override
list, and they are the reason it is 41 rows rather than zero.

### F4 — mixins carry real reflected attributes

`HTMLElement includes HTMLOrSVGElement` is where `autofocus` lives;
`HTMLAnchorElement includes HTMLHyperlinkElementUtils` is where `href` lives.
A generator that ignores `includes` statements silently loses them, so the
parser expands mixins into every including interface before classification.

### F5 — a getter where there was no property changes assignment semantics

`<script blocking="render">` and kin: `[SameObject, PutForwards=value,
Reflect] readonly attribute DOMTokenList blocking`. Installing that as a
getter-only accessor made `element.blocking = 'render'` throw
`TypeError: cannot set non-writable property`, where previously — with no
property at all — the assignment created an own data property and the test
passed. This turned two `html/dom/render-blocking` files into
`evaluation-threw` errors in the first candidate run. `[PutForwards=value]` is
precisely the fix: assigning to the IDL attribute forwards to the token list's
`value`, i.e. writes the content attribute. Every read-only reflected attribute
the generator produces (10 of them) is a `PutForwards` `DOMTokenList`, so the
`t` kind gained that setter and the regression closed.

Recorded because it generalises: widening an interface's shape is not
automatically safe. A property the page could previously create by assignment
becomes a guarded accessor the moment the table names it.

### F6 — the unknown-element fallback was HTMLElement

`wrapNode` fell back to `HTMLElement.prototype` for any HTML-namespaced tag the
table did not list. Per the standard the fallback is `HTMLUnknownElement`,
except for valid custom element names. Since `HTMLUnknownElement` inherits from
`HTMLElement`, correcting this changes only `instanceof` and the class string,
never a method lookup.

The runtime's `__tagName` uppercases, so a case-preserving
`createElementNS(…, 'foo-BAR')` cannot be told from `foo-bar` at this seam.
Validity is therefore checked on the folded name, which keeps custom elements
working and leaves three `interfaces.html` subtests failing (`foo-BAR: useNS`,
`å-bar: useNS`, `å-BAR: createElement`). That is a `genet-scripted-dom` name
question, not a table question, and it is not opened here.

## The generator

`support/idl-interface-table` is a dependency-free workspace member with a
library and a binary. `cargo run -p genet-idl-interface-table` writes
`components/script-runtime-api/dom/html_interfaces_generated.rs`;
`-- --check` verifies it without writing.

**Parser.** `src/idl.rs` is a hand-written reader for the WebIDL subset the
table needs — about 495 lines including the tokenizer. It handles `interface`,
`partial interface`, `interface mixin`, `X includes Y;`, extended-attribute
lists, and `attribute` members with their types; every other definition and
member is skipped to its terminating `;` by a brace-aware scan. A crate
dependency (weedle2 or similar) was rejected because this repository's
dependency resolution is deliberately conservative — see the census plan's
notes and `Code/CLAUDE.md` — and because the subset is small enough that a
parser is cheaper than a new registry edge. Partials merge onto the base
interface as they are read; mixins are expanded into their including interfaces
in a second pass, so an attribute reaches the classifier exactly once,
in declaration order, base first.

**Classifier.** One function maps an IDL attribute to a reflected attribute or
to nothing: `[ReflectURL]` → the `u` kind; `[Reflect]` by IDL type
(`DOMString`/`USVString` → `s`, `boolean` → `b`, `long` → `l`,
`unsigned long` → `ul`, `DOMTokenList` → `t`); `[ReflectSetter]` on `USVString`
→ `u`; everything else, including nullable and union types, produces nothing.
The content-attribute name is `Reflect="…"` when given and the ASCII-lowercased
IDL name otherwise. `[ReflectDefault=]` supplies the missing-value default, and
the numeric kinds always emit one explicitly so the bootstrap's implicit
fallback can never apply.

**Overrides.** `OVERRIDES` in `src/lib.rs` is a flat table of
`(interface, IDL name, operation, reason)`. An entry replaces the generated row
or inserts one, and its reason is one of four families, stated once at the top
of the table: `enum` (27 rows — the keyword set is prose, per F3), `prose`
(12 rows — the getter is prose but the runtime's approximation is a reflection),
`union` (1 row — `hidden`'s `(boolean or unrestricted double or DOMString)?`,
of which the runtime models the boolean form), and `default` (1 row —
`<input>`'s `size`, `[Reflect]` with no `[ReflectDefault]` where HTML's prose
still gives 20). Nothing else is hand-written: the generated file carries no
hand edits, and the drift test proves it.

**Emission and drift.** The output is a plain Rust data file with a
`@generated` header, deterministic for fixed inputs. Both consts carry
`#[rustfmt::skip]` so that `cargo fmt` cannot rewrite the long tag arrays out
from under the byte-comparison. `dom/tests.rs::generated_table_is_current`
regenerates in-process from the vendored WPT tree and asserts byte equality,
plus floors on the three table sizes, so a stale checked-in file fails the
crate's own test run rather than drifting silently.

**Ordering.** Interfaces are emitted in IDL declaration order, then stably
re-ordered so a parent always precedes its children — the bootstrap builds
prototype chains in table order.

## Before and after

| | before (hand table) | after (generated) |
|---|---|---|
| HTML element interfaces | 65 | 72 |
| Reflected IDL attributes | 277 | 338 |
| Tag names mapped | 74 | 148 |
| DOM/CSSOM shape-only interfaces | 0 | 41 |
| Hand-written rows to maintain | 342 | 41 overrides |
| `html_interfaces.rs` | 1,004 lines of data + glue | 166 lines of glue |

New interfaces: `HTMLUnknownElement`, `HTMLPictureElement`,
`HTMLDirectoryElement`, `HTMLFontElement`, `HTMLFrameElement`,
`HTMLFrameSetElement`, `HTMLSelectedContentElement`.

The 41 shape-only records come from `dom.idl` and `cssom.idl`: the interface
object, its prototype chain and its class string, with no members, and only
where the bootstrap has not already defined the name. Where it has — `Node`,
`Element`, `Document`, `Text`, `Comment`, `DocumentFragment`, `DOMTokenList`,
the `CSS*` family — the pass only adds the class string and leaves the working
implementation alone. Five names are on an explicit deny list
(`MutationObserver`, `MutationRecord`, `AbortController`, `AbortSignal`,
`XSLTProcessor`) because a page that feature-detects them and finds a bare
shape takes a worse path than one that finds nothing.

Three `[LegacyFactoryFunction]` names now exist — `Image`, `Audio`, `Option` —
built from the table's own tag name with the documented argument mapping.

## Phases

### I1 — the generator and the checked-in table

**Done-conditions**

- `cargo run -p genet-idl-interface-table` writes the table from the vendored
  WPT inputs with no network access and no dependencies. **Met.**
- Output is byte-stable across runs and across `cargo fmt`. **Met**, proved by
  `-- --check` after a formatting pass.
- Drift fails loudly: `generated_table_is_current` regenerates and compares.
  **Met.**

### I2 — the runtime consumes the generated table

**Done-conditions**

- `html_interfaces.rs` holds record shapes and JS serialization only; no
  interface data. **Met** (1,004 → 166 lines).
- The bootstrap's algorithm glue is unchanged in kind: `installHtmlInterfaceTable`,
  `installReflectedAttributes` and `makeHtmlInterfaceConstructor` keep their
  shape and gain only what the new columns require — `[Exposed]` filtering,
  `[HTMLConstructor]` (an interface without it always throws), the class
  string, read-only handling, and the named-constructor factories. **Met.**
- `cargo test -p script-runtime-api` green. **Met**: 125 + 16 + 7 + 13 tests,
  0 failed, including the pre-existing `html_interface_table_works` reflection
  test unchanged.

### I3 — widened coverage, measured

**Done-conditions**

- `html/dom` and the idlharness files hold or gain. **Met**: `html/dom` gains
  one file (25 → 26 all-pass) and +1,542 subtest passes; all six idlharness
  files under `html` and `dom` hold at their prior status.
- `dom` and `custom-elements` hold. **Met**: `dom` moves one file
  `error -> fail` and gains +108 subtests; `custom-elements` moves one file
  `error -> fail`, one `fail -> pass`, and gains +50 subtests.
- No unexplained pass-to-fail movement. **Met**: zero `pass -> fail`, zero
  `pass -> error` and zero `fail -> error` across all 16 directories.

## The census diff

Sixteen directories, disk mode, Boa/Livery, `--jobs 8 --timeout 90`, run twice
from the same runner build lineage: `pre/` at `7f0a3c80d29` unmodified, `post/`
with the lane landed. Every `pre/` tally is identical to the corresponding
2026-09-07 harness-repair map, which is the positive control that the two runs
differ only in the engine source.

| Directory | status movements | subtest passes |
|---|---|---|
| `html/dom` | 1 `fail -> pass` | +1,542 |
| `html/semantics/interfaces.html` | — | +298 |
| `dom` | 1 `error -> fail` | +108 |
| `custom-elements` | 1 `error -> fail`, 1 `fail -> pass` | +50 |
| `html/semantics/embedded-content` | 3 `error -> fail`, 3 `fail -> pass` | +23 |
| `html/semantics/forms` | — | +25 |
| `shadow-dom` | — | +3 |
| nine others | — | 0 |
| **total** | **5 `error -> fail`, 5 `fail -> pass`** | **+2,049** |

Named movements:

- `html/dom/elements/name-content-attribute-and-property.html` `fail -> pass`:
  the obsolete-members partial in `html.idl` supplies `name` on interfaces the
  hand table omitted it from.
- `custom-elements/parser/parser-constructs-custom-elements.html` `fail -> pass`
  and `custom-elements/reactions/CSSStyleDeclaration.html` `error -> fail`: the
  `HTMLUnknownElement` fallback and the widened constructor set.
- `dom/events/Body-FrameSet-Event-Handlers.html` `error -> fail`:
  `HTMLFrameSetElement` now exists, so the file evaluates instead of throwing.
- `html/semantics/embedded-content`: two `the-iframe-element` parentage files
  and `delay-load-event-detached.html` move `error -> fail`;
  `resource-selection-invoke-audio-constructor-no-src.html` and two `sandbox_*`
  files move `fail -> pass` on the `Audio` factory function and the `sandbox`
  token list.
- `html/dom/render-blocking/{remove-attr,remove-element}-unblocks-rendering.optional.html`:
  regressed to `evaluation-threw` in the first candidate and are green in the
  landed run — see F5. They are the only pass-to-fail movement this lane
  produced, and it was fixed rather than explained away.

`html/semantics/interfaces.html` moves 0/438 → 298/438 subtests. It stays
`fail` at the file level. Of the 140 residual subtests, 137 are the `useParser`
variant, which needs `DOMParser` — a missing global, not a table gap — and
three are the case-preservation limit recorded in F6.

## Known gaps carried forward

- `[ReflectRange=(a, b)]` is parsed but not applied: `colSpan`, `rowSpan`,
  `span` and `headingOffset` take their `[ReflectDefault]` and are not clamped.
  This matches the pre-lane behaviour exactly; clamping is reflection-algorithm
  work, which this lane deliberately does not do.
- `[ReflectNonNegative]`, `[ReflectPositive]` and
  `[ReflectPositiveWithFallback]` produce nothing, so `maxLength`, `minLength`,
  `<textarea>`'s `cols`/`rows` and `<meter>`'s `double` limits stay absent, as
  before.
- Enumerated attributes still take the limited-enum-with-`""` shape: per
  attribute *invalid*-value defaults remain unimplemented, and the keyword sets
  are override data rather than generated.
- The three case-sensitivity subtests in F6.
- The generator reads `html.idl`, `dom.idl` and `cssom.idl`. `SVG.idl`,
  `uievents.idl` and the rest of the 338 vendored IDL files are untouched;
  extending the shape pass to them is the obvious next slice and is not
  scheduled.

## Progress

- **2026-09-07** — Generator written (`support/idl-interface-table`, 1,323
  lines, no dependencies), table generated, `html_interfaces.rs` reduced to
  glue, bootstrap extended for `[Exposed]`, `[HTMLConstructor]`, class strings,
  read-only/`PutForwards` and named constructors, `wrapNode` corrected to fall
  back to `HTMLUnknownElement`. Drift test and shape test added.
- **2026-09-07** — First candidate run found the `PutForwards` regression (F5);
  fixed, re-run, and the landed run has zero pass-to-fail movement.
- **2026-09-07** — Gates: `cargo test -p script-runtime-api` 161 tests green
  across four targets; `cargo test -p genet-wpt --features netfetch` 63 green,
  3 ignored; clippy clean in every touched file; rustfmt applied. Release
  runner SHA-256
  `20130f96b9495762a0eb66afbc7ad88a078fab20fe3ba87d1db56dbcbf6a1cb2`, genet
  `7f0a3c80d29`, manifest SHA-256 prefix `d5ec5be9bf1a75ed`. Maps under
  `Code/testing/genet/wpt-ledger/2026-09-07_idl_interface_table/`.
