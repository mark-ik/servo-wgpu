# Genet Fontique patch

This is a vendored copy of `fontique 0.10.0`. Parley's sibling dependency
points at it directly, so downstream Genet consumers do not depend on a root
`[patch.crates-io]` entry to obtain the matching query API.

Source: crates.io `fontique-0.10.0.crate`, SHA-256
`274fa4f0f0a926ae182c7c076c078cce8a38471d15e61a102a02cac984be9813`;
upstream source revision `1df9544bf0bd675d304001c0d0b35df2d220cd14`
(recorded in `.cargo_vcs_info.json`, which identifies upstream rather than this
patch).

## Windows codepoint fallback

Fontique's DirectWrite backend used `IDWriteFontFallback::MapCharacters`, but
called it with a representative script sample and cached the returned family
by script and locale. Common-script punctuation normalized to `Latn` could
therefore receive a Latin-sample family that did not cover the actual symbol.

The added text query is called only after Parley's authored candidates fail.
It passes the complete cluster, locale, and matching attributes to
`MapCharacters`; explicit Fontique fallback families remain first. A result is
accepted only when one mapped font covers the full UTF-16 range at scale 1.0.
Partial ranges and scaled results use the existing script fallback because the
family-only query API cannot carry their remaining-range or scale semantics.

Non-Windows targets retain Fontique's existing script-cache route. The text
route clears its cache identity, so a following same-key script query cannot
reuse text-specific families.
