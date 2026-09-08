# ortet

The raw Genet host. One window, one document, no chrome.

A genet is a whole clonal colony; an ortet is the original individual it
descends from. This is the reference individual of the engine: the one headed
port that proves Genet runs with **no Mere crate in its dependency cone**.
`support/ci/check_dependency_cones.py` (`assert_ortet_cone`) witnesses that on
every CI run, with synthetic Cargo-shaped positive controls over exact moved
names and the `cambium`/`pelt` prefixes, plus a clean allowed graph. The old
live controls over `document-canvas` and `cambium-genet-winit-host` retired
when those crates moved to Mere on 2026-09-03.

See `design_docs/2026-09-03_ortet_founding_plan.md`.

## What it is not

No tabs, no tiles, no reader lane, no smolweb, no profiles, no settings, no
persistence, no history, no trust store. Every one of those is a product
decision, and product decisions live in Mere. Ortet drives the session itself:
it holds one `LiverySessionEngine`, spawns one `DocumentSession<Scene>` for the
address it was given, and maps winit events onto the session's own input
vocabulary. Following a link is spawning a new session for the new address.

## Usage

```
ortet --url <address> [options]

  --url <address>      A file path, a file:// URL, or an http(s) URL.
  --size <WxH>         Window size in physical pixels (default 960x640).
  --frames <N>         Present exactly N frames, then exit.
  --artifact <path>    Write the captured frame as a PNG and print its digest.
  --actions <list>     Drive the document once, after its first laid-out frame.
  --help               Print the usage and exit.
```

### `--actions`

The bounded-run driving list. A person's hands are the real check for a headed
host; `--actions` is what lets a *machine* produce a receipt for the same
gestures. It is deliberately two verbs and no more — this is not a scripting
language, and it should not grow into one.

Steps are separated by `;`, each `<name>:<a>,<b>`:

| step             | effect                                                     |
| ---------------- | ---------------------------------------------------------- |
| `scroll:<dx>,<dy>` | `scroll_at` the viewport centre, so a nested scroller under that point takes it first |
| `click:<x>,<y>`  | a pointer press and release at a logical-pixel point, which activates a link the way a mouse does |

The list runs **once**, after the first frame has laid the document out, so a
click has real geometry to hit. Whatever the actions changed is what the next
frame — and therefore the captured receipt — shows.

### `--expect-heading`

An optional completion condition for a bounded receipt. On its final frame,
Ortet reads the selected session's engine-owned `ContentReport` and requires an
exact matching heading. It is intended for a scripted fixture whose final DOM
state must be correlated with the PNG capture; it is not general page
automation. The option requires `--frames` so a failed asynchronous completion
has a finite, reproducible timeout.

## Accessibility

On native platforms Ortet installs `genet-winit-host`'s AccessKit bridge before
showing its window and publishes the Livery session's current semantic tree.
There is no accessibility command-line action syntax: platform assistive
technology sends normal AccessKit actions, which Ortet routes through the
session's existing focus, scroll, and pointer paths.

For the article fixture, probe the heading named `Ortet` and the hyperlink
named `Field notes`. The document root is preserved as the session supplies it;
Ortet does not rename it from the heading. Ortet logs
`[ortet] accessibility bridge installed` when the native adapter is live.

Windows receipt, 2026-09-05: source
`28fee14166580c19df464fa26845e2b998d9d75e` built
`debug/ortet.exe` with SHA-256
`71E3CC1F4750191128D785F6035265A80C2A1EE05905DC2881E92812D2494B1D`.
The UIAutomation probe found the Document, `Ortet`, and `Field notes`; focused
the link; re-acquired the rotated tree; invoked the link; and observed the
`Field notes` heading in `notes.html`. The probe and its logs are in
`C:/Users/mark_/Code/scratch/genet-k6-ortet-20260905/`.

## Receipts

`--frames N --artifact out.png` presents N frames, composes the last one into a
texture ortet owns, reads it back, writes the PNG, prints the frame digest, and
exits non-zero on a blank frame. That is what CI and a plan can cite; a person
looking at the window is confirmation, not the gate.

```
cargo run -p ortet -- --url ports/ortet/examples/article.html \
    --frames 3 --artifact /tmp/ortet_article.png
cargo run -p ortet -- --url ports/ortet/examples/article.html \
    --frames 3 --artifact /tmp/ortet_scrolled.png --actions 'scroll:0,240'
```

The digest is FNV-1a over the frame's RGBA bytes (`RgbaFrame::digest`), so two
runs of the same address on the same machine agree, and a run whose content
moved does not. Every run also prints `settled at <address>`, which is the
address it *ended* on: it differs from the one it started on exactly when a link
was followed, so a navigation receipt needs nothing else to be legible.

Note that an in-page `#fragment` link never reaches the host. `genet-livery`
scrolls to the element itself and reports `ClickOutcome::Scrolled`
(`components/genet-livery/src/document/selection.rs:253`), which arrives here as
`SessionClick::Handled`. Only a cross-document href becomes
`SessionClick::Navigate` and replaces the session. Both are visible in the
receipts: the fragment run's digest moves while its address does not, and the
cross-document run's does both.

## The fixture

`examples/article.html` is a script-free article with a linked stylesheet, an
image, an in-page `#` link and a link to a second local document
(`examples/notes.html`). One receipt over it exercises fetch, layout, paint and
navigation at once. `examples/mark.png` is generated, not borrowed — a 48x48
gradient written byte by byte.
