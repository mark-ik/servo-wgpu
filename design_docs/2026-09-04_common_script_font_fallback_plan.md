# Common-script font fallback

**Status:** in progress (2026-09-05). The original Common-to-`Zyyy` diagnosis
was corrected against the patched Parley source; T0 landed as a controlled
Windows diagnostic. T1 through T3 remain open. Founded when Isometry's side panel drew its disclosure
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

The framing that came out of that pass — "our font fallback covers Latin-1 but
not Geometric Shapes" — is not what is happening, and the true statement is
much broader. Latin-1 renders because the *primary* family carries it and no
fallback is needed. What is actually true is below.

## 2. The diagnosis, verified

**Common-script characters do reach font fallback, but under an effective real
script chosen from their item rather than under `Zyyy`.** That means this plan
does not yet establish a universal Common-script fallback defect. The chain,
each link read in source:

1. `▾` U+25BE is Geometric Shapes, whose Unicode script property is
   **Common (`Zyyy`)**. So are the arrows, box drawing, dingbats, most
   punctuation above Latin-1, and the general symbol blocks.
2. Before it builds an item, patched Parley finds its first real script, or
   defaults an all-Common item to Latin (`support/patches/parley/src/shape/mod.rs`,
   `shape_text` item initialization).
   In the character loop it replaces every non-real script, including Common,
   with that item script (`:122-124`). `shape_item` therefore sends Latin (or
   the surrounding real script) through `script_to_fontique`, then sets that
   key when `FontSelector` is constructed. It does **not** pass `Zyyy` unchanged.
3. fontique's fallback remains **script-keyed, not codepoint-keyed**. The
   DirectWrite backend does not ask DirectWrite which font has the character.
   It asks for a *sample string* for the script and requests the default
   family for that sample: `fontique-0.10.0/src/backend/dwrite.rs:115-123`,
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

The irony worth recording: the DirectWrite backend already holds an
`IDWriteFontFallback` (`dwrite.rs:137`) — the very interface whose
`MapCharacters` answers "which font covers this text" — and the script path
never uses it for characters. CoreText has the same shape available in
`CTFontCreateForString`.

## 3. What is measured, and what remains open

The original Isometry tofu report is a real consumer symptom. T0 now reproduces
U+25BE glyph zero on this Windows host with a primary face known to omit it.
The cluster-correlated negative trace records the primary and two platform
candidates as `Discard`, retaining the primary as the selected fallback. This
is evidence about this host and target only. It is not evidence that every
Common character, consumer, or desktop platform fails.

The ordered authored-face control does select a covering second face and paints
a nonzero U+25BE glyph while the primary retains Latin. This proves the CSS
family ordering and Parley candidate loop can carry a deliberate fallback. It
does not repair the installed-platform route. The next repair decision must
compare the script-keyed candidate set with codepoint-aware platform fallback.

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
evidence about this host's query and shaping layers, not a false test. T1's
next probe can proceed; a repair requires evidence of the owning layer.

**T1 — The owning-layer repair.** T0 currently sees every candidate discard
for U+25BE under the `Latn` fallback key. The next bounded probe must compare
that candidate set with the platform's codepoint-aware result and correlate any
covering candidate's `QueryFont`/`FontData` identity with the shaper. If the
mismatch is local, repair candidate-to-`FontData` or shaper propagation; if it
is upstream, record that ownership before changing Fontique. A deliberate
authored fallback remains a separate product choice, not a substitute for
locating this seam.
**Done when:** the owning layer is evidenced, a covering selected face produces
a nonzero glyph through the normal path, the primary retains covered Latin,
and a headed capture shows the result in a real app.

**T2 — Upstream investigation, conditional on ownership.** If T1 locates the
fault in Fontique or Parley, propose the smallest upstream change with T0's
trace and an isolated reproducer. Fontique's platform backends are script-keyed
and might eventually use `IDWriteFontFallback::MapCharacters` or
`CTFontCreateForString`, but that is not presumed to be this defect.
**Done when:** an upstream disposition exists if upstream owns the cause; if
the cause is local, the plan records why no upstream change is required.

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
