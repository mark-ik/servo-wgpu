# Servo constellation cone retirement

**Date:** 2026-09-07

**Status:** landed 2026-09-07, uncommitted. Every gate green; the directory
deletion was run by Mark.

**Parent:** [Web platform WPT census](2026-09-06_web_platform_wpt_census.md),
whose dependency-graph pass found the cone.

## Ruling

`cargo metadata` at `2c47cff7627` showed 26 of 77 workspace members
unreachable from both Ortet and genet-wpt, and a further sixteen reachable
only through the WPT reftest lane. The reachable set was Servo's message
plumbing: `servo-paint`'s `Paint` painter, `paint-api`, and the
embedder, constellation, net, devtools, storage, canvas and profile trait
crates with their base, config, url, pixels, geometry, hyper_serde, wakelock
and servo_tracing dependencies. Ortet never used any of it; genet-render
lowers paint lists straight into netrender.

Two things inside that cone were live and not Servo's:

1. **The platform compositor half of `servo-paint`**: the DXGI, CALayer and
   Wayland presentation backends, `HostWgpuContext` and the Dx12 and Vulkan
   interop, plus the `RenderingContextCore` and `WgpuCapability` contract
   WebGL binds against. Mere consumes exactly this half, by git pin. None of
   it imported the Servo traits.
2. **The testdriver input path**: `embedder_traits::webdriver_actions` and
   `input_events`, which genet-wpt uses to turn WebDriver action sequences
   into DOM events. genet-wpt was the only consumer.

Everything else had no consumer once the reftest lane rendered through
`genet-render-host`.

Kept deliberately: the servo-media family (woodshed's redshank playback
spike took a path dependency on 2026-09-04), WebGL wgpu and ESSL,
script-engine-piccolo (the game wing), malloc_size_of and allocator (used by
paint-types and xpath).

## Work

1. Carve `components/genet-compositor` out of `servo-paint`: the compositor
   backends, interop, and `rendering_context_core` with its `RefreshDriver`
   hook dropped (no caller). Public names are unchanged so Mere's next bump
   is a rename of the dependency, not an API change.
2. Port `genet-wpt`'s reftest renderer onto `RenderCore`: translate the
   envelope with `paint_list_render`, materialize box-shadow masks on the
   netrender renderer, rasterize with a transparent clear, read back through
   `read_rgba8_texture`. Delete the `cfg(any())` incumbent path and the
   envelope-era URL scanners it alone used.
3. Absorb `webdriver_actions` and `input_events` into
   `ports/genet-wpt/src/testdriver/`, pruned to what the interpreter and the
   harness use; `WebViewPoint` is defined there over paint-types units.
4. Remove the cone from the workspace: aliases, the `paint` feature gates in
   constellation-traits and paint, canvas-traits' unused webxr-api
   dependency, and the unit-test glob narrowed to the one surviving test
   crate.
5. Delete the directories. Blocked for the assistant by the auto-mode
   classifier; the command is recorded under Progress for Mark to run.

## Done-conditions

- `cargo check --workspace --features genet-wpt/netfetch` is green with the
  cone directories absent.
- `cargo test -p genet-wpt` is green.
- Strict Clippy on `genet-compositor` and `genet-wpt` reports nothing in the
  changed files.
- Both checked reftest baselines (`css/mediaqueries`, `css/css-position`)
  report `unexpected=0` under the ported renderer, which proves the
  `RenderCore` route is pixel-identical to the retired painter route for
  those slices.
- Ortet builds and its self-driven frame receipt still captures.
- Mere's next bump renames its `paint` git dependency to
  `genet-compositor`; recorded here, not done in this lane.

## Findings

- 2026-09-07: `servo-paint` was two crates under one name. The painter half
  was 607 lines and pulled about 25,000 lines of trait crates behind it; the
  compositor half is about 2,200 lines with no Servo imports.
- 2026-09-07: the reftest lane rendered through `render_with_compositor` and
  the `WgpuMasterCaptureBackend`; Ortet renders through `rasterize_scaled`
  and reads back an owned RGBA8 target. The two are the same netrender
  scene path with different presentation; the baseline guard is the proof.
- 2026-09-07: the `crates.io` `servo-webgpu` and siblings are Servo's own
  publications, not ours; nothing in this cut touches a published name.
- 2026-09-07: `engine-observables-api` has zero dependents in the workspace.
  It is the intended seam for probe, diagnostics and the accessibility
  bridge, and nothing publishes through it yet. Not this lane's work;
  recorded so it is not lost.

## Progress

- 2026-09-07: steps 1 through 4 landed in the working tree. Workspace check
  green, `genet-wpt` unit tests 60 passed / 3 ignored. Deletion command for
  step 5, from the genet root:

  ```
  git rm -r -q components/webgpu components/shared/webgpu components/webxr components/shared/webxr components/profile tests/unit/profile components/deny_public_fields tests/unit/deny_public_fields components/paint components/shared/paint components/shared/embedder components/shared/constellation components/shared/net components/shared/devtools components/shared/storage components/shared/canvas components/shared/profile components/shared/base components/config components/url components/pixels components/geometry components/hyper_serde components/wakelock components/servo_tracing components/default-resources
  ```

- 2026-09-07: gates run on the ported route. Release runner SHA-256
  `04cf50caf8ef2d7b0e82eea86bc60879e3fa243a4bff70955ac92afcf170339f`.
  `css/mediaqueries` 16 pass / 40 fail / 37 skip and `css/css-position`
  45 pass / 73 fail / 226 skip, both `unexpected=0`, identical to the checked
  baselines. Strict Clippy: two inherited findings in the compositor and one
  in the absorbed input events fixed; the `harness.rs:58` unfulfilled
  expectation predates this lane (the netfetch feature constructs those
  variants). Ortet presented three frames and captured its receipt, digest
  `0x6377ba8a6bf4dbc9`. Remaining: step 5, the deletion.
- 2026-09-07: Mark ran both deletion batches (the second with `-f`, over the
  pre-split feature-gate edits) and the workspace check finished green in
  41.59s with 27 directories gone. Lane complete apart from the commit.

## Follow-through: build warnings and engine sources (2026-09-07)

Mark asked for the warning cascade the workspace check printed to go, and
for the engine choice to be one fork at a time. Both landed in the same
working tree:

- **Engine sources.** `script-engine-boa` names the boa fork (`mark-ik/boa`,
  branch `genet`) and `script-engine-nova` names the vano fork
  (`merely-made/vano`, branch `genet-embedder`) by git directly, instead of
  crates-io names patched to git at the root. A crates-io name plus a root
  patch cannot be redirected a second time from `.cargo/config.toml`, which
  is why the machine-local `paths` overrides existed and why cargo warned on
  every command. The config now carries `[patch]` tables for the three fork
  git sources (boa_engine and boa_gc, nova_vm, piccolo), the same mechanism
  netrender uses; metadata shows all three resolving from disk, boa_gc
  unified on the path copy.
- **One engine per host.** `genet-scripted` used Boa only in tests; it is a
  dev-dependency now. A host names script-engine-boa or script-engine-nova
  itself. genet-wpt keeps both because it is the oracle. Ortet has no engine
  in its closure at all.
- **Dead code.** The capture-file replay half in `genet-scripted` (no caller
  since the incumbent route left; the recorder and its tests stay), four
  never-read SPIR-V type ids and two `kind` fields in webgl-essl, an unused
  texture accessor in webgl-wgpu, an unused import in the GStreamer Unix
  render shim, and the netfetch-only lint expectation in genet-wpt's harness.
- **Manifest.** The `num-bigint-dig` dev profile override (its `rsa` consumer
  left with net-traits), the unused `vello` workspace dependency and git
  patch, and the crates-io `paint_list_api` patch that only sprigging needed
  before it moved to mere.

Receipt: `cargo check --workspace --features genet-wpt/netfetch` prints no
warning located under this repository and no resolver warning; the only
remaining warnings are three inside `crates/vano`'s `nova_vm`, which is the
fork's own tree. `cargo test -p genet-scripted -p script-runtime-api` is
green (25, 122, 16 and 7 tests). Mere's root `[patch.crates-io]` entries for
boa_engine and boa_gc become unused once it bumps past this commit; harmless,
and removable then.

