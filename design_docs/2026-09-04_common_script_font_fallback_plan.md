# Common-script font fallback

**Status:** in progress (2026-09-05). The original Common-to-`Zyyy` diagnosis
was corrected against the patched Parley source; T0 landed as a controlled
Windows diagnostic. The bounded Windows T1 repair and Ortet readback are accepted;
upstream disposition, other platforms, and consumer revision adoption remain
open. Founded when Isometry's side panel drew its disclosure
markers as tofu and the workaround was to retreat to ASCII.

**Related:** `components/genet-livery/src/text.rs` (the stack's whole font
story: it builds the parley `FontContext` and hands parley the resolved
family); `isometry/design_docs/2026-09-03_side_panel_diet_plan.md` (the
consumer that hit it, and the ASCII retreat recorded there).

## 1. Why

A cross-platform stack cannot draw only the characters its default UI font
happens to carry. Isometry put `▾` and `▸` on a disclosure trigger and got two
tofu boxes; it now ships `[-]` and `[+]`. That is a real product cost paid to
work around a stack defect, and the next non-ASCII glyph anyone reaches for
pays it again.

The controlled test separates primary coverage from fallback: its authored
primary carries Latin `f` but omits both disclosure markers. That makes it
possible to test the fallback route without depending on a default UI font's
coverage.

## 2. The diagnosis, verified

**Common-script characters do reach font fallback, but under an effective real
script chosen from their item rather than under `Zyyy`.** That means this plan
does not establish a universal Common-script fallback defect. The pre-T1
route at `505b5be26b4` was:

1. `▾` U+25BE is Geometric Shapes, whose Unicode script property is
   **Common (`Zyyy`)**. So are the arrows, box drawing, dingbats, most
   punctuation above Latin-1, and the general symbol blocks.
2. Before it builds an item, patched Parley finds its first real script, or
   defaults an all-Common item to Latin (`support/patches/parley/src/shape/mod.rs`,
   `shape_text` item initialization).
   In the character loop it replaces every non-real script, including Common,
   with that item script. `shape_item` therefore sends Latin (or
   the surrounding real script) through `script_to_fontique`, then sets that
   key when `FontSelector` is constructed. It does **not** pass `Zyyy` unchanged.
3. Upstream Fontique 0.10's fallback was **script-keyed**. Its DirectWrite
   backend supplied a *sample string* for the script instead of the actual
   character: `fontique-0.10.0/src/backend/dwrite.rs`,
   `let text = key.script().sample()?;`.
4. The absent Common sample is therefore not exercised by normal Parley item
   shaping. It would matter only if a caller handed fontique `Zyyy` directly.
5. A controlled primary face that lacks the target is needed to tell whether
   Fontique supplies a candidate and whether Parley's coverage pass accepts it.
   Those are observations distinct from the final shaped glyph: T0 records the
   actual fallback key, every queried candidate, its coverage status, the
   selected candidate, and paint glyph ids. It does not infer a platform cause
   from any one of those layers.

**macOS needs the same measurement, not a transferred conclusion.** Its
`backend/coretext.rs:68-78` also uses `script.sample()?`, but Parley supplies
the effective real script above.

**Linux is a different path and is not yet assessed.**
`backend/fontconfig.rs:735` builds a fontconfig `Pattern` from lang and script
rather than a sample, so it may or may not resolve Common. T3 settles it
rather than assuming; the workspace has both a Fedora Wayland and a Mint X11
machine to answer it on.

The Windows repair now passes the actual cluster to
[`IDWriteFontFallback::MapCharacters`](https://learn.microsoft.com/en-us/windows/win32/api/dwrite_2/nf-dwrite_2-idwritefontfallback-mapcharacters)
after authored candidates fail. The API's mapped length and scale are part of
its contract; the bounded family-only path accepts complete ranges at scale 1.

## 3. What is measured, and what remains open

The original Isometry tofu report is a real consumer symptom. Before T1, T0 reproduced
U+25BE glyph zero on this Windows host with a primary face known to omit it.
The cluster-correlated negative trace records the primary and two platform
candidates as `Discard`, retaining the primary as the selected fallback. This
is evidence about this host and target only. It is not evidence that every
Common character, consumer, or desktop platform fails.

After T1, the actual-text system query selects a covering candidate and paints
U+25BE glyph 1325. The authored-face control continues to select its covering
second face, and covered Latin stays in the primary. Ortet shows both U+25BE
and U+25B8 through the normal system route. Partial/scaled platform mappings,
other operating systems, and downstream product adoption remain separate work.

## 4. Gates

**T0 — The instrument (landed 2026-09-05).** A `genet-livery` test shapes a Common-script target
through the real `LiveryDocument`/`TextSystem`/Parley path with an authored
primary face proven to lack that target, plus a Latin control covered by the
same primary. It records source script (`Common`), Parley's effective fallback
script (observed from Fontique's fallback key), queried candidate identities and
coverage statuses, selected candidate, selected paint faces, and target/control
glyph ids. It asserts only controlled fixture coverage and ordered authored
fallback; it prints the installed-platform outcome without treating it as a
portable assertion.
**Done when:** the diagnostic runs on Windows and reports the effective script,
the requested primary and selected paint faces, and coverage outcome. A green target is
evidence about this host's query and shaping layers. T1 adds a regression
assertion for system coverage of both disclosure markers on this Windows host.

**T1 — Windows codepoint-aware fallback repair (accepted 2026-09-05).** Fontique's DirectWrite
backend already called `MapCharacters`, but supplied `FallbackKey::script()`'s
sample and cached the returned family by script/locale before Parley saw the
actual cluster. On this host, the sample route left U+25BE as `Discard`; an
actual U+25BE query returns a candidate that is `Complete` and paints glyph
1325. The repair uses an actual-text Fontique query only after authored primary
candidates fail. It preserves effective script, locale, requested attributes,
and authored/explicit fallback precedence.

The family-only path accepts DirectWrite only when its result covers the full
UTF-16 cluster at scale 1.0. Partial ranges and scaled results retain the old
script route because neither semantic can be represented by a family id. The
Windows-specific text state is invalidated before a same-key script query;
non-Windows continues using its existing cache. The focused receipt proves
U+25BE system completion, authored Segoe UI Symbol precedence, Latin primary
completion with one candidate, and alternating U+25BE/U+25B8 queries.
**Done when:** focused regressions pass and a headed capture shows the result
in a real app. Both are met by the final repair and Ortet receipt below; this
does not claim whole-workspace or full-WPT validation.

**T2 — Upstream disposition (open).** The responsible boundary spans Fontique's
script-sample query and Parley's actual cluster. Genet carries the paired patch
locally; no upstream submission or acceptance is claimed. Prepare the bounded
query change with its reproducer for upstream review, including the unresolved
partial-range and scale semantics before proposing broader support.
**Done when:** upstream disposition and the retained or retired local patch
are recorded.

**T3 — Linux.** Run T0's effective-script, candidate-status, selected-face,
and final-glyph trace on both Fedora Wayland and Mint X11.
**Done when:** both receipts distinguish query coverage from final paint glyph
and the plan records the outcomes.

## 5. Stop rules

- No consumer is asked to avoid a character as the fix. Isometry's ASCII
  retreat is a workaround this plan exists to retire, not a precedent.
- The stack-side repair does not become a private font stack that diverges
  from what CSS asked for; a family the author named still wins.
- Once ownership evidence warrants it, upstream investigation may run alongside
  a local probe; a verified local repair is not gated on upstream release.

## Findings

### 2026-09-05 — Windows T1 and Ortet readback accepted

Implementation source `ad734ac89b0` passed four focused Livery tests, the
Fontique same-key query restoration test, and the DirectWrite attribute test.
`cargo check --locked --offline -j 1 -p parley --no-default-features --features libm`
also passed after the final feature-gating change. The normal/build dependency
tree excludes `font-diagnostic`. The 600-package Ortet dependency witness
passed with the paired patch and no forbidden Mere dependency.

The windowed fixture is `ports/ortet/examples/font_fallback.html`. Its primary
is the existing WPT Lato face; only the third row explicitly names Segoe UI
Symbol. Ortet presented three frames at 1200x1200 physical pixels on Windows
11 build 26220, using Rust 1.97.1, `-C debuginfo=0`, and isolated offline builds.
The baseline was `d8ca6805fdf`; the final headed source was `8952a18ff17`.
The latter contains the final repair unchanged, plus the same fixture.

```text
cargo build --locked --offline -j 1 -p ortet
ortet --url ports/ortet/examples/font_fallback.html --size 1200x1200 \
  --frames 3 --artifact <receipt.png>
before: frame digest 0xde8a5c242ef83301, two missing-glyph boxes
after:  frame digest 0x23fc2d2218d4e40f, both disclosure triangles
```

Artifacts are retained under `Code/scratch/genet-font-repair-20260905/`:
`before.png`, `after.png`, both binaries, build/run logs, platform/compiler
details, and `image-comparison.json`. Of 4,444 changed pixels, 4,443 are in the
target-symbol row; one label pixel changes its blue channel by one. The Latin
control and explicit-symbol glyph bands are pixel-identical. This is a bounded
host readback receipt, not a full-WPT or cross-platform conformance claim.

| Frozen artifact | SHA-256 |
|---|---|
| `Cargo.before.lock` | `c2f5fd457ee09be046baa75f84530a646d761ecd33e2578f65f56f794e800e9c` |
| `Cargo.after.lock` | `0fdf7926973c4498ba0f7608dda2b03b5e10acb728804f82c18c37e77efae126` |
| `before-ortet.exe` | `c7c869bbc424ba9fe1a4104594442f670b83f98f46a0176315f9e99d2ad13687` |
| `after-ortet.exe` | `6b3a4136c9717361b7fc50264969683f4d09ed3a2d0976ee1de0948199c44023` |

Delivery is a separate boundary. A standalone consumer manifest with only a
path dependency on this Parley resolves exactly one Fontique, at the sibling
vendored path, without Genet's root patch table. Its metadata is archived as
`consumer-metadata.json`. Mere, Isometry, and Turnstone currently pin Parley at
`115d348dedd`; those manifests were not updated here. Product adoption requires
an intentional revision refresh and consumer verification before retiring the
Isometry ASCII workaround. Upstream submission and the other-platform gates
remain open.

### 2026-09-05 — Windows T0 observes `Latn` candidates and an authored control

`support/patches/parley` carries an opt-in `font-diagnostic` feature. A
thread-local RAII capture enables recording only for one `LiveryDocument` /
`TextSystem` / Parley run; `take()` disables it before returning events and an
unconsumed capture disables and discards events on drop. Each event carries its
source grapheme text, Fontique's actual fallback key, the ordered `QueryFont`
family/index candidates, Parley's `NoCharmap`,
`Discard`, `Keep`, or `Complete` coverage result, and the selected candidate
index. The feature is a `genet-livery` dev-dependency only, so normal Livery
production builds do not enable it.

The Windows-gated test selects the U+25BE and U+0066 events by their recorded
cluster text, and requires one paint run for each fixture colour. It uses WPT `Lato-Medium-Liga.ttf` as an authored primary
(SHA-256 `23FFAFCF7950D019BA65FECB27645AD53522F95756CEB4F6A4FB2096C3231D63`).
Its cmap maps U+25BE to zero and U+0066 to glyph 4. The negative session names
only that face. On this host it observed `Latn`, a primary `Discard`, then two
platform candidates also `Discard`, selected candidate 0, and painted U+25BE
as glyph zero. The events are selected by their recorded `▾` cluster, so this
is a covering-candidate absence receipt for that query rather than an inference from a
combined paint list.

The positive control resolves `seguisym.ttf` from `WINDIR` or `SystemRoot` and
loads it as a second **authored** face under `@font-face` (not as platform fallback; SHA-256
`A4A35DCC62CD30E1A6C97B695ECEF83E59D2B149E3AF834F6A49F11851D56B37` on this
host). It observed primary `Discard`, authored secondary `Complete`, selected
candidate 1, and painted U+25BE glyph 1325 from that secondary font resource.
The U+0066 control selected and painted from the primary, glyph 4. This verifies ordered authored fallback without
claiming that another Windows installation has the same platform fallback.

The condensed receipt was:

```text
negative_fallback_key=[76, 97, 116, 110]
negative_candidates=[primary Discard, platform candidate Discard, platform candidate Discard]
negative_selected=Some(0) negative_target_glyphs=[0]
authored_fallback_key=[76, 97, 116, 110]
authored_candidates=[primary Discard, secondary Complete]
authored_selected=Some(1) target=U+25BE glyphs=[1325] control=U+0066 glyphs=[4]
```

The command was run from the detached sparse worktree with its standalone
dependency resolution, so it is a clean-source receipt rather than a claim
about the primary checkout's local Cargo overrides:

The final three-test run used source `b9ed7e36430`; its code was integrated
unchanged at `caa562d1e3a`. The generated lock is retained locally as
`Code/scratch/genet-plan-review-20260905/Cargo.font.lock`, SHA-256
`C2F5FD457EE09BE046BAA75F84530A646D761ECD33E2578F65F56F794E800E9C`.

```text
CARGO_TARGET_DIR=C:\Users\mark_\Code\target-font-20260905
RUSTFLAGS="-C debuginfo=0"
cargo test --offline -j 1 -p genet-livery --test k5d_font_feature_resolution \
  common_script_fallback_diagnostic_records_effective_script_and_selected_faces \
  -- --exact --nocapture
```

### 2026-09-04 — the chain, and what it corrects

The original five-link conclusion was based on the upstream conversion helper
without checking the patched item's script normalization. That conclusion is
withdrawn by the 2026-09-05 finding below. It remains true that Latin-1 can
render through the primary face, so a normal-system-font green result has no
diagnostic force.

One hypothesis was raised and killed on the way, worth recording so nobody
spends the same hour: `genet-livery`'s `font_family` (`text.rs:3722`) maps
only `system-ui` and the user-agent default to a `GenericFamily`, and passes
everything else — including the CSS generic keyword `sans-serif` — through as
`FontFamily::Source`, a literal family name. That looks like the bug and is
not: parley's `resolve/mod.rs:217-230` runs `FontFamilyName::parse_css_list`
over the source string, which recognises the generic keywords and expands them
through `generic_families`. The generic resolves correctly. The defect is
downstream of family resolution, in what happens when the resolved family has
no glyph.

## Progress

- **2026-09-04.** Scoped after Isometry's disclosure markers rendered as tofu.
  The then-recorded universal `Zyyy` diagnosis is withdrawn: it omitted the
  patched Parley item's real-script normalization.
- **2026-09-05.** Corrected the premise before implementation: patched Parley
  normalizes Common to an item real script, with Latin as the all-Common
  default. T0 changed from a required failing assertion to a controlled
  diagnostic. The completed Windows trace observed `Latn`, a primary discard,
  two platform-candidate discards, and final U+25BE glyph zero; the authored
  secondary control then painted U+25BE glyph 1325 while primary Latin stayed
  glyph 4. Windows is the only measured platform in this pass; macOS and both
  named Linux environments retain explicit evidence requirements.
- **2026-09-05.** Accepted the bounded Windows T1 repair after four Livery
  tests, two targeted Fontique unit tests, the `libm` build, dependency witness,
  and Ortet before/after readback. The paired Parley/Fontique source dependency
  also resolves from a standalone consumer. Product pins, upstream disposition,
  and other-platform measurements remain open.
