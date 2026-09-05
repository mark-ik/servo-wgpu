# Ortet founding plan

**Status:** in progress, updated 2026-09-05. O0, O1 and O4 landed on
2026-09-03; the crates.io claim stays pending. O2 and O3 are open. The
`fleece` carve-out is reconciled with the boundary plan's §9.1 (see Findings);
the witness still names fleece on every run.

Ortet is the raw genet host: the one headed port that proves the engine runs
without Mere. The platform boundary plan
(`mere/design_docs/mere_docs/implementation_strategy/2026-09-02_platform_boundary_and_repository_topology_plan.md`,
§2 and §9.3 ruling 4) leaves Genet "WPT, conformance harnesses, and one small
host that proves Genet works without Mere". Pelt cannot be that host: its
desktop port enables the reader and netfetch lanes by default, and `pelt-core`
depends on `workbench` and `inker`, both Mere authority. Under the "consumers
first" order Mark ruled on 2026-09-03, Pelt moves to mere before Cambium does,
and from that commit genet's only host is `genet-wpt`, which is headless. Ortet
takes the role a smaller host was always going to take.

The name is the botanical one. A genet is a whole clonal colony; an ortet is the
original individual it descends from; a ramet is any member. The raw host is
the reference individual of the engine. Mark ruled "ortet is fine" on
2026-09-03; the name is free on crates.io and is claimed with a real publish
when the crate has one worth publishing (naming ledger rule).

## What ortet is, and is not

- A `ports/ortet` binary over engine crates only: `genet-winit-host` (window,
  wheel translation, AccessKit bridge) and through it `genet-render-host`
  (wgpu + netrender boot, rasterize, acquire, compose), `genet-documents` with
  its `livery` feature (`LiverySessionEngine`), `document-session-api`
  (`SessionEngine`, `DocumentSession`, `SessionSpawnRequest`), `netrender`
  (`Scene`), `genet-host-api` (`ResourceFetcher`, `ResourceResponse`) and
  `netfetcher` with its default transport for http(s).
- It drives the session itself. Pelt routes through inker's `SessionRegistry`
  and `PeltController`; ortet holds one `LiverySessionEngine`, spawns one
  `DocumentSession<Scene>` for the address it was given, and maps winit events
  onto the session's semantic input directly. Following a link is spawning a
  new session for the new address.
- It has no chrome. One window, one document, no tabs, no tiles, no reader
  lane, no smolweb, no settings, no persistence. Anything of that kind is Mere.
- **Its dependency cone contains no Mere crate**, and the cone witness says
  so on every CI run: none of `inker`, `workbench`, `cambium*`, `mere-*`,
  `nematic`, `errand`, `document-canvas`, `pelt*`, `tabard`,
  `knot-editor-host`. `fleece` is not Mere: the boundary plan's §9.1
  reclassed it independent, an engine-side lower library, and this list
  first said otherwise from memory; the witness names it separately so the
  carve-out is visible and cannot widen.
- Its receipts are self-driven. `ortet --url <file or http(s)> --frames N
  --artifact out.png` presents N frames, reads the last one back through
  `genet-render-host`, writes the PNG, prints the frame digest and exits
  non-zero on a blank frame. That is what CI and a plan can cite; a person
  looking at the window is confirmation, not the gate.

## Phases

### O0. Found the crate

- `ports/ortet/Cargo.toml` (workspace-inherited version, license, edition;
  `publish` as the workspace sets it; a description that says what it is),
  `ports/ortet/src/main.rs`, `ports/ortet/README.md`, workspace member entry
  beside `ports/pelt`; every new source carries the house header (copyright
  line, Exhibit A, SPDX).
- `support/ci/check_dependency_cones.py` gains `assert_ortet_cone`: resolve
  ortet's cone from `cargo metadata` and fail on any crate in the forbidden
  set above. The current positive control runs the same `resolved_cone`
  traversal over a synthetic Cargo-shaped graph: intermediate packages reach
  both an exact forbidden name and a forbidden prefix, two same-named package
  IDs have distinct outgoing paths, non-normal edges stay excluded, and an
  allowed graph stays clean. `assert_ports_depend_inward` asserts Ortet's
  manifest and prevents any component from depending on a port.

**Done when:** `cargo check -p ortet` is green, the witness passes with the
positive control, and `python support/ci/check_dependency_cones.py` is what
CI already runs.

### O1. The desktop viewer

- Boot `SurfaceHost` on a winit window sized from `--size` (default 960x640);
  `NetrenderOptions::for_untrusted_content()`, since a raw browsing host is
  exactly the case that option exists for.
- Fetcher: `LocalFetcher` for `file:` and bare paths, falling back to a
  netfetcher-backed `ResourceFetcher` for http(s) over the default transport,
  on a background tokio runtime the host owns. Nothing else: no trust store,
  no scheme sniffing, no smolweb.
- Per frame: `session.frame(w, h)` gives the `Scene`; rasterize, acquire,
  compose, present, as the host crates document. `pump` and `settled` drive
  redraw scheduling.
- Input: pointer down/move/up, wheel via the host's translation into
  `scroll_at`, keyboard through `key_input` and `scroll_for_key`, text and IME
  input, focus in and out, resize. A `SessionClick` whose effect navigates
  spawns a new session for the new address; the window title follows.
- The receipt path above, with the PNG written through the `png` crate the
  workspace already carries and the digest from `RgbaFrame::digest`.
- A fixture under `ports/ortet/examples/`: one script-free article with a
  linked stylesheet, an image and an in-page link, so one receipt exercises
  fetch, layout, paint and navigation. Reuse Pelt's `p5-resources` fixture
  only if it needs no Pelt code.

**Done when:** the article receipt produces a non-blank frame with a digest
that is identical across two runs on the same machine; a headed run opens
the fixture, scrolls, and follows its in-page link and one external link to a
second document; `cargo test -p ortet` (unit tests for the argument parser and
the fetcher's scheme split) is green; and the O0 witness still passes.

### O2. Accessibility

- Wire `AccessKitBridge` the way Pelt's workspace viewer does, over the
  session's accessibility projection from `document-session-api`, with the
  host correlating raw `A11yActionRequest` values to its published projection
  mapping before routing them back into the session. The bridge request itself
  carries only its target, action, and action data.
- Treat a session replacement as an accessibility identity boundary. The host
  assigns a monotonically increasing session generation and namespaces every
  document-local node ID by that generation before it enters the bridge.
  Reused local IDs in a new document therefore cannot name nodes from the old
  tree.
- When the host publishes a tree, it records each bridge-visible target's
  session generation, document-local target, projection revision, and
  advertised actions. A drained request must match that published mapping; a
  request queued under document A must never be correlated with document B's
  current observation. The host rejects an absent or generation-mismatched
  mapping; the session then revalidates the current revision, target, and
  advertised action. Pointer actions use the same current click-target
  revalidation before they enter ordinary input routing.

**Done when:** the bridge reports a tree whose root carries the article's
heading; a queued action from document A is rejected after navigation to
document B even when B reuses A's local node ID; an action whose revision or
advertised action changed is rejected; and one current focus or scroll action
is accepted by a live session and visibly changes its projection or scroll
state.

### O3. The web target

The boundary plan's P0-P5 landed 2026-09-02 through 2026-09-04, so they are no
longer a deferral condition. The prior canvas host,
`cambium-genet-web-host`, now belongs to Mere; its wasm receipt proves that
Mere adapter, not an Ortet web host. Genet does retain the needed lower seam:
`genet-render-host::RenderCore::create_surface` documents
`wgpu::SurfaceTarget::Canvas(HtmlCanvasElement)`, and its manifest has neither
winit nor AccessKit. The missing feature cone is Ortet-owned: this workspace
has `ports/ortet` only, its manifest directly selects `genet-winit-host` and
`winit`, and it contains neither a browser entrypoint nor a wasm-specific
manifest. O3 starts by adding that narrow target without importing the former
Cambium host.

Ortet's web target is the same `RenderCore` over an `HtmlCanvasElement`, driven
by DOM events, with no Cambium. Its receipt must include a real Chromium
non-blank canvas readback; a screenshot alone is insufficient.

**Done when:** `wasm32-unknown-unknown` builds, the article renders in Chromium
with a non-blank canvas readback, and the witness holds for the web target's
cone too.

### O4. Take over Pelt's place

When Pelt moves to mere (boundary plan P3): ortet becomes the workspace
`default-members` entry, the witness drops the Pelt manifest assertion and
keeps ortet's, the boundary plan's ruling 4 records the smaller host as the
answer, and the naming ledger's claim is made with a real publish if the crate
is worth one by then.

**Done when:** genet's root `cargo build` builds ortet, no dependency or
manifest assertion points at Pelt, and the boundary plan and naming ledger both
say so. The retained forbidden `pelt` prefix is intentional.

The witness's former live positive control was the one thing Pelt's departure took with it.
`pelt-desktop`'s cone exercised both halves of `is_ortet_forbidden` at once — an
exact name (`inker`) and a prefix (`cambium`, `mere-`) — and no single remaining
member reaches both. Neither `cambium` nor `cambium-genet-winit-host` reaches
`inker` at all, checked before choosing. Those two controls were retired with
their crates on 2026-09-03. The current gate uses a synthetic Cargo resolve
graph that exercises the actual traversal through intermediate packages to an
exact forbidden name and a forbidden prefix, including two versions of a
same-named package with distinct outgoing paths. It also proves dev and build
edges are excluded and retains an allowed graph control. Predicate assertions
remain a separate guard on the name list; they are not presented as a live
graph control.

## Findings

- 2026-09-03: `genet-winit-host` and `genet-render-host` already split the
  window-specific from the target-neutral present mechanics, and both
  document the per-frame shape a host follows; ortet adds no rendering code.
- 2026-09-03: `LiverySessionEngine<Fetch>` needs only a `ResourceFetcher`;
  `genet-documents` no longer depends on inker, netfetcher, errand, nematic or
  document-canvas after the boundary plan's P1, so the engine half is the
  whole of what ortet needs from the documents crate.
- 2026-09-03: Pelt's `static_viewer.rs` (2,622 lines) is the nearest prior
  art, but 140 references to the engine and host crates are wrapped in inker
  routing, Pelt profiles and seven product receipts. Ortet is written fresh
  against the two host crates and the session traits, not extracted from it.

- 2026-09-03 (**needs a ruling**): the forbidden set above names `fleece`, but
  ortet cannot avoid it and no amount of care in ortet will change that.
  `genet-documents` reaches `fleece::extract_main_text` unconditionally at
  `components/genet-documents/src/engines/clip.rs:114`, so anything holding a
  `LiverySessionEngine` holds fleece. This is not an oversight in the manifest:
  §9.1 of the boundary plan already **reclassed fleece as independent** — "it
  may stay in genet as a lower library or leave for its own repository, but it
  does not go to Mere" — with `genet-scripted` and `genet-documents` as its
  engine-side consumers and a CI-witnessed cone of `layout_dom_api` +
  `unicode-segmentation`. The two documents disagree, and this plan's list is
  the later one. Resolved for now by naming fleece separately in
  `assert_ortet_cone` (`ORTET_RECLASSED`), which prints it on every run and
  fails if the carve-out ever widens, rather than dropping it from the list
  silently. Three ways to settle it: strike fleece from this plan's list and
  cite §9.1; move the clip lane behind a `genet-documents` feature ortet does
  not enable; or reclass fleece back and split it out of `genet-documents`.
  The first is the smallest and matches the boundary plan; it is Mark's call.
  **Resolved 2026-09-03, the first way:** §9.1 was ruled with the rest of the
  inventory and is the authority; this plan's list was written from memory and
  is corrected above. The witness keeps naming fleece on every run.

- 2026-09-03: nothing else in the forbidden set is reachable. `assert_ortet_cone`
  walks 592 packages from ortet over normal (non-dev, non-build) resolve edges
  and finds none of `inker`, `workbench`, `cambium*`, `mere-*`, `nematic`,
  `errand`, `document-canvas`, `pelt*`, `tabard`, `knot-editor-host`. The
  positive control over `pelt-desktop`'s cone reports eleven of them, `inker`
  included, so a clean result is not a broken walk.

- 2026-09-03: **an in-page `#fragment` link never reaches the host.**
  `genet-livery` scrolls to the element itself and returns
  `ClickOutcome::Scrolled` (`components/genet-livery/src/document/selection.rs:253`),
  which the session adapter turns into `SessionClick::Handled`. Only a
  cross-document href becomes `SessionClick::Navigate`. O1's plan text ("a
  `SessionClick` whose effect navigates spawns a new session") is therefore
  right about the mechanism but describes only half the fixture's link
  behaviour; both halves are receipted below.

- 2026-09-03: an address the user typed as a filesystem path has to be
  normalized to an absolute `file://` URL before it reaches the engine.
  `genet_host_api::navigation::resolve_href` joins a `#fragment` onto the
  *document* only for a base with a scheme; against a bare path it joins onto
  the directory (`docs/a.html` + `#x` -> `docs/#x`). `args::address_from_argument`
  does the normalization, and its scheme test has a two-character floor so a
  Windows drive letter (`C:\pages\a.html`) stays a path rather than a `c:` URL.

- 2026-09-03: `NetrenderOptions::for_untrusted_content()` sets only
  `apply_limit_buckets`; it leaves `tile_cache_size` and `enable_vello` at their
  defaults, and `render_vello` needs both. The host must spread it:
  `NetrenderOptions { tile_cache_size: Some(64), enable_vello: true,
  ..NetrenderOptions::for_untrusted_content() }`.

- 2026-09-03 (observation, not ortet's to fix): in the scrolled receipt the
  fixture's `float: right` figure overlaps the body text it should displace,
  and its `figcaption` paints over the following paragraph. The float lands in
  the right place at the top of the document, so this is a Buckram float/line-box
  interaction, not a host bug. Recorded here because ortet is now a cheap way to
  see such things; chasing it belongs to the layout lane.

- 2026-09-03 (observation, not ortet's to fix; layout lane): **the fixture's
  `header` lays out wider than its containing block.** In the article and
  notes receipts at 480x320 logical the standfirst paragraph inside `header`
  wraps at roughly 470 logical px and its lines are clipped at the frame
  edge, and the header's 3px bottom border runs to the frame edge too, while
  the `body`'s own paragraphs wrap correctly at the 400px content box (body
  padding 40px each side). A control run with the paragraph `max-width`
  removed from the stylesheet produced the identical digest
  (`0x6377ba8a6bf4dbc9`), so the fixture's `max-width` is not the cause. The
  shell's sizing was checked and is right: the session is framed at the
  logical size and the scene rasterized with the device scale
  (`shell.rs` `render`). One thing to check first in the engine: whether the
  UA sheet blockifies `header` (and `nav`, `figure`, `figcaption`), since an
  inline `header` would explain both the wide lines and the border.

- 2026-09-05 (source-only follow-up): `components/genet-livery/src/lib.rs`
  `CAMBIUM_UA_DEFAULTS` already blockifies `header` and `nav` (around line
  120), so that part of the hypothesis is excluded. The UA defaults omit
  `figure` and `figcaption`; floated `figure` blockification and the effective
  `figcaption` display still need runtime probes. The header-width observation
  remains unresolved, and this source check does not establish the earlier
  overlap observation's root cause.

- 2026-09-05: the cone witness used package names as traversal identities.
  Cargo permits multiple package IDs with one name, and either version can have
  different dependencies, so the old walk could omit a forbidden outgoing path.
  It now visits package IDs and reports names only after traversal. Its
  synthetic Cargo-shaped resolve graph has two `relay` versions that separately
  reach `inker` and `mere-test` through intermediate packages; it also proves
  dev/build poison is excluded and an allowed `genet-livery` graph stays clean.
  This replaces the retired `document-canvas` and
  `cambium-genet-winit-host` live controls. The live Ortet witness still proves
  its actual normal-edge cone reaches `genet-documents` and `netrender`.

- 2026-09-05: P0-P5 of the platform-boundary plan are landed, so O3 is not
  waiting on that migration. `genet-render-host` already owns the target-neutral
  canvas seam, but Ortet has only the native `genet-winit-host` feature cone.
  The work still missing is a small Ortet browser entrypoint and wasm manifest,
  plus the Chromium readback receipt; the old Cambium web-host receipt belongs
  to Mere and cannot close this gate.

- 2026-09-05: O2's reusable seams already exist in
  `components/genet-documents/src/engines/livery.rs`: the session publishes a
  revisioned `DocumentA11yProjection`, returns revision-bound click targets,
  and rejects stale or unadvertised actions in
  `dispatch_accessibility_action`. The concrete next O2 slice is host-owned:
  add `AccessKitBridge` to `ports/ortet/src/shell.rs`, namespace projection
  node IDs by an Ortet session generation, install/update the bridge from the
  current projection, and drain requests back through the session's revision
  and action checks. `genet-winit-host/src/a11y.rs` supplies the platform
  adapter, but does not supply Ortet's session-generation custody.

- 2026-09-05: O3 has a reusable target-neutral seam in
  `components/genet-render-host/src/lib.rs::RenderCore::create_surface`,
  including `wgpu::SurfaceTarget::Canvas`, while `ports/ortet` still has only
  the native `winit` entrypoint and no wasm target module or manifest. The next
  O3 slice is a target-gated Ortet web entrypoint that boots
  `RenderCore::boot_async`, creates the canvas surface, drives the existing
  session/frame path, and adds a Chromium non-blank readback receipt. The
  native `SurfaceHost` and AccessKit adapter must stay out of that wasm path.

- 2026-09-05: the registry branch of the global boundary witness had gone
  stale after the 2026-09-03 moves. The Mere workspace inventory and Genet's
  movement comments identify the moved families as Pelt (`pelt`, `pelt-core`,
  `pelt-desktop`), Tabard, the Cambium family (`cambium*`, `meristem`,
  `sprigging`), Workbench, `mere-surface-api`, and the engine-management names
  already listed in the witness. The registry predicate now names the
  unprefixed members and uses only the moved `cambium` and `pelt` prefixes.
  `document-session-api`, `fleece`, and `netrender` retain their documented
  engine or independent classifications; the synthetic negative control keeps
  them accepted. `inker` is among the newly forbidden moved names.

- 2026-09-05: loading the pre-change classifier from `e78eaa86a4d` and feeding it
  explicit registry fixtures for `inker`, `cambium`, `workbench`, `nematic`,
  `sprigging`, `meristem`, and `pelt-desktop` returned an empty result. The
  updated classifier catches all seven; this is a separate regression receipt,
  rather than a self-test that reimplements the old rule.

## Progress

- 2026-09-03: plan written; O0 and O1 dispatched.

- 2026-09-03: **O0 landed.** `ports/ortet` founded: `Cargo.toml` (workspace
  version/license/edition/publish, its own description), `README.md`,
  `src/lib.rs`, `src/args.rs`, `src/fetch.rs`, `src/main.rs`; workspace member
  entry beside `ports/pelt`; every source carries the house header (the
  relicense audit's "without Exhibit A" stayed at 6, all in other lanes, while
  owned sources went 881 -> 885). `support/ci/check_dependency_cones.py` gained
  `assert_ortet_cone`, wired into `main()` on the resolve-graph metadata the
  Mere-source witness already fetches, so the script runs `cargo metadata`
  no more often than before.

  Receipt — `python support/ci/check_dependency_cones.py`:

  ```
  ortet cone: 592 packages, none forbidden; positive control over pelt-desktop
  reports ['cambium', 'cambium-genet-winit-host', 'cambium-rootstock',
  'cambium-winit', 'cambium-winit-a11y', 'document-canvas', 'inker',
  'mere-document-lanes', 'mere-surface-api', 'pelt-core', 'workbench']
  ortet cone note: ['fleece'] present - reclassed independent by the boundary
  plan 9.1, reached through genet-documents' clip lane
  dependency-cone witnesses passed
  ```

  Done-conditions: `cargo check -p ortet` green; the witness passes with its
  positive control; the script is the one CI already runs. All met, with the
  `fleece` carve-out recorded in Findings.

- 2026-09-03: **O1 landed.** `src/shell.rs` (the winit window, the per-frame
  shape, pointer / wheel / keyboard / text / IME / focus / resize routing, and
  navigation as re-spawning the session) and `src/receipt.rs` (compose into an
  owned texture, read back, write the PNG through `png`, digest through
  `RgbaFrame::digest`, non-zero exit on a blank frame). Fixture under
  `ports/ortet/examples/`: `article.html` (script-free, a linked `article.css`,
  a generated 48x48 `mark.png`, an in-page `#propagation` link and a
  cross-document link) plus `notes.html`. The swatch is written byte by byte,
  not borrowed.

  A headed run driven by a person is not something CI or an agent can produce,
  so the plan's headed done-condition is met instead by `--actions`, a
  deliberately two-verb driving list (`scroll:<dx>,<dy>`, `click:<x>,<y>`)
  applied once after the first laid-out frame. It is documented in the README
  and is not to grow into a scripting language.

  Receipts, all at 960x640 on a 2x display (so a 480x320 logical viewport):

  | run | actions | frame digest | settled at |
  | --- | --- | --- | --- |
  | article, run 1 | none | `0x6377ba8a6bf4dbc9` | `article.html` |
  | article, run 2 | none | `0x6377ba8a6bf4dbc9` | `article.html` |
  | scrolled | `scroll:0,240` | `0x43a2675a3e2a6712` | `article.html` |
  | in-page link | `click:100,185` | `0x48442e53798e22f3` | `article.html` |
  | cross-document link | `click:210,185` | `0xb4499d1e2aea318b` | `notes.html` |

  The two unactioned runs agree to the digest *and* byte-for-byte in the PNG
  (sha256 `da19d04a3cbd5403e12435a8f677ce31…`), so the receipt is reproducible on
  one machine. Both driven runs move the digest, and the frames show it: the
  scrolled frame opens on "What the word carries", the fragment frame on the
  "Propagation" heading, the cross-document frame on "Field notes". The address
  moves only for the cross-document run, which is the Findings entry above made
  visible. Nothing is blank; a blank frame would have exited non-zero.

  Done-conditions: identical digest across two runs on one machine, yes;
  non-blank, yes; the headed scroll + in-page link + second document, met
  through `--actions` receipts rather than a person's hands; `cargo test -p
  ortet` green (10 unit tests over the argument parser and the fetcher's scheme
  split, including a positive control that the local lane really reads a file,
  so its "unsupported scheme" misses are not an instrument that answers `None`
  to everything); `cargo check -p ortet` green with no new warnings;
  `cargo clippy -p ortet` reports nothing in ortet's own sources (the workspace
  lints it does report are pre-existing elsewhere). The O0 witness still passes.

- 2026-09-03: **O4 landed, except the publish claim.** Pelt left genet the same
  day (`ports/pelt`, `ports/tabard`, `components/inker/knot-editor-host` and
  `components/mere-document-lanes`, 129 tracked files), so ortet takes the
  places Pelt held:

  - `default-members = ["ports/ortet"]` in the root manifest, and the six
    workspace entries the four crates held (three members plus
    `knot-editor-host`, `mere-document-lanes`, `pelt-core` and `pelt-desktop`
    in `[workspace.dependencies]`) are gone with them.
  - `assert_ports_depend_inward` asserts ortet's single manifest at
    `ports/ortet/Cargo.toml` where it asserted Pelt's two. The rest of that
    function — no `components/` crate may name a `ports/` path, and
    `genet-host-api` has exactly one manifest — is unchanged.
  - The cone witness's positive control split in two, for the reason recorded
    under O4 above. Checked before choosing: `cargo tree -p cambium` and
    `cargo tree -p cambium-genet-winit-host` both reach `inker` zero times, so
    neither candidate the move suggested could carry the `inker` half alone.

  Receipt — `python support/ci/check_dependency_cones.py`:

  ```
  ortet cone: 592 packages, none forbidden; positive controls: document-canvas
  reports ['inker']; cambium-genet-winit-host reports ['cambium',
  'cambium-rootstock', 'cambium-winit', 'cambium-winit-a11y',
  'mere-surface-api', 'workbench']
  ortet cone note: ['fleece'] present - reclassed independent by the boundary
  plan 9.1, reached through genet-documents' clip lane
  dependency-cone witnesses passed
  ```

  The cone is still 592 packages, unchanged by Pelt's departure, which is the
  point: ortet never reached any of it.

  Other receipts: root `cargo build` (default member, so this is the ortet
  binary) finished green; `cargo check --workspace` green with 0 errors and 24
  warnings across eight crates, every one pre-existing and none in ortet;
  `cargo check -p ortet` green; `cargo test -p ortet` 10 passed;
  `cargo check -p netfetcher --no-default-features` green. The relicense audit
  went 887 -> 843 owned sources, exactly the 44 sources in the four removed
  directories, with "without Exhibit A" unmoved at 6 (all in other lanes).

  Still pending: the crates.io claim. Ortet's dependencies are `publish =
  false` host crates, so there is nothing publishable yet; the naming ledger
  entry waits on that, not on this commit.

- 2026-09-05: **O0/O4 witness hardening landed.**
  `support/ci/check_dependency_cones.py` now traverses Cargo resolve package
  IDs, never package names, and runs a synthetic positive/negative control
  before it reads the workspace graph. The positive graph reaches both `inker`
  and `mere-test` through separate versions of `relay`; dev-only
  `pelt-dev-only` and build-only `cambium-build-only` stay outside the cone.
  The allowed graph reaches only `safe-middle` and `genet-livery`. This is the
  current graph-control receipt after the historical Pelt,
  `document-canvas`, and `cambium-genet-winit-host` controls departed with the
  boundary migration. O2's done-condition now names generation-scoped IDs,
  host correlation of raw bridge requests to their published mapping, queued
  A-to-B rejection, revision/action revalidation, and one live accepted action.
  O3 now records the landed boundary, retained `RenderCore` canvas seam, and
  missing Ortet-owned web feature cone.

  Receipt from the detached sparse worktree after adding `tests/unit` (the
  workspace's `tests/unit/*` member glob): `python -m py_compile
  support/ci/check_dependency_cones.py` passed, then `python
  support/ci/check_dependency_cones.py` passed with:

  ```text
  resolved-cone synthetic controls: exact and prefix forbidden paths found;
  same-name package ids both traversed; dev/build edges excluded; allowed graph clean
  ortet cone: 600 packages, none forbidden; historical live positive control: none
  (retired with P3); predicate control: forbids all 26 exact names, forbids
  cambium-/mere-/pelt-anything, admits genet-livery
  dependency-cone witnesses passed
  ```

  The final check at source `dccb680f94f` produced the output above. The
  600-package result is the refreshed sparse-review metadata witness,
  compared with the prior 597-package artifact; the three newly visible
  allowed names are `chacha20`, `jni-sys-macros`, and `objc2-core-video`.
  This verifies the witness from the sparse review checkout, which lacks the
  primary checkout's local `.cargo/config` overrides. It is a graph receipt,
  not a frozen-WPT or headed-host receipt.

- 2026-09-05: registry-route controls now exercise explicit synthetic
  packages for `inker`, `cambium`, `workbench`, `nematic`, `sprigging`,
  `meristem`, and `pelt-desktop`; the old predicate misses all seven, while
  the updated witness catches them. The negative control keeps `fleece`,
  `document-session-api`, and `netrender` clean. The isolated review artifacts
  are `Code/scratch/genet-plan-review-20260905/metadata.json` and
  `Code/scratch/genet-plan-review-20260905/cone-regression.json`; the ignored
  generated `Cargo.lock` SHA-256 is
  `95C994BF5F2D12348F0B19394692537B087B1F6200DB6BFA8DB14232B012D538`.
  No unlocked primary resolution was run; locked primary resolution was
  blocked by the local override graph and failed read-only.

- 2026-09-05: final integration at `caa562d1e3a` passed the synthetic and
  live dependency witness again: 600 packages, none forbidden, 26 exact moved
  names guarded. The font diagnostic adds dev-dependency edges, so the generated
  lock changed to SHA-256
  `C2F5FD457EE09BE046BAA75F84530A646D761ECD33E2578F65F56F794E800E9C`;
  this matches the font lane's retained `Cargo.font.lock`. The earlier lock
  remains archived as `Cargo.cone.lock` beside it. Normal Ortet reachability
  and the seven registry regression results are unchanged.

- 2026-09-05: read-only O2/O3 source review identified the host-side
  accessibility custody and target-gated canvas entrypoint as the next slices;
  no implementation or runtime receipt was claimed.
