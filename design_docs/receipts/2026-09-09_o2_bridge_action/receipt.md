# O2 bridge-action rejection gate, closed natively with Focus; Invoke residual recorded

This receipt closes the O2 bridge-action rejection gate's outstanding native
leg with a real Windows UIAutomation client, using `Action::Focus` instead of
`Action::Click`/`InvokePattern`, and records the Invoke regression found while
attempting to reproduce the 2026-09-05 acceptance as an explicit residual
rather than a fix.

## Why Focus and not Invoke

The 2026-09-05 receipt (source `28fee14166580c19df464fa26845e2b998d9d75e`)
drove the accepted positive path with a native `InvokePattern.Invoke()` call
on the `Field notes` hyperlink. Reproducing that receipt at the current
source (`f949210ca32`, full `f949210ca324695bde59c80728933429a146492b`) finds
that `AutomationElement.GetSupportedPatterns()` returns an **empty** pattern
list for every hyperlink in `ports/ortet/examples/article.html`
(`article.html`'s nav and in-body `Field notes` links, and `Jump to
propagation`/`Back to the top`), and `TryGetCurrentPattern`/
`GetCurrentPattern` for `InvokePattern` fails for all of them, repeatably
after the window settles. This is not intermittent: it was observed across
the runner's own retry loop and again in this receipt's diagnostics.

`AutomationElement.SetFocus()` is unaffected: it forwards `Action::Focus`
unconditionally through `accesskit_windows::node::PlatformNode_Impl::SetFocus`
-> `do_action`, which builds a raw `ActionRequest` without consulting the
node's advertised action set locally (source:
`accesskit_windows-0.32.1/src/node.rs:1066-1068`, `902-917`). That request
reaches Ortet's `ports/ortet/src/a11y.rs` `Accessibility::route()` exactly
like a Click request would, so the O2 rejection gate's actual subject --
`route()`'s fresh-ID publication policy -- is exercised identically by Focus.
Per Mark's ruling, this receipt does not widen into
`components/genet-livery` or `components/genet-documents` to chase the Invoke
gap; that investigation is out of scope here and recorded as a residual
below.

**Divergence from the brief for this slice:** the brief for this receipt
stated that a synthetic pointer click at the link's screen coordinates
navigates correctly, evidencing a healthy pointer pipeline. This receipt's
own attempt to reproduce that (`support/ci/run_ortet_o2_bridge_action_pointer_diagnostic.ps1`,
`diagnostics-pointer-click/`) does **not** confirm it: a scripted
`SetCursorPos` + `mouse_event` click at the `Field notes` link's UIA
`BoundingRectangle` center, after `SetForegroundWindow` and a neutral
activation click, still timed out without navigating
(`ortet: receipt completion heading "Field notes" was absent before the
40000ms deadline`; `diagnostics-pointer-click/receipt.log.err`,
`driver.log`). `SetForegroundWindow` reported success
(`NativeWindowHandle=11471566`), but Windows' foreground-lock behavior for
background/scripted callers can make that return value unreliable, and a
scripted `mouse_event` from a non-foreground process is a plausible point of
difference from whatever earlier method established the "pointer pipeline
is healthy" claim. This receipt does not resolve that difference -- it is
recorded here as new evidence that qualifies, rather than confirms, the
brief's premise. The empty-`GetSupportedPatterns`/no-`InvokePattern` finding
above is independently confirmed and is not affected by this.

## Cases (Focus-based, real UIAutomation)

Each case builds `ports/ortet` fresh from source `f949210ca32`, launches
`ortet.exe` headed against `article.html`, and drives it with
`System.Windows.Automation`. Artifacts (log, tree/property snapshot,
`result.json`, exit code) are under
`C:/Users/mark_/Code/testing/genet/ortet-o2-bridge-action-20260909/`:

| Case | Directory | What it does | Result |
|---|---|---|---|
| Positive control | `focus-accept/` | A current `SetFocus()` on `Field notes` is dispatched; the focused projection is read back via `AutomationElement.FocusedElement`. | **PASS** -- dispatched; `FocusedElement.Name == "Field notes"` |
| Stale-reject | `focus-stale-reject/` | The same client-cached `Field notes` element is reused for a second `SetFocus()` after its own first dispatch republished (retiring its host id) -- a queued action against a replaced publication. | **PASS** -- refused (see note below) |
| Unadvertised-reject | `focus-unadvertised-reject/` | `SetFocus()` is requested on the non-interactive `Ortet` heading, which never advertises `Action::Focus`. | **PASS** -- refused (see note below) |

All three are asserted, not merely observed: the runner
(`support/ci/run_ortet_o2_bridge_action_focus_receipt.ps1`) throws if a
reject case fails to show a rejection anywhere (host log line or OS-layer
exception -- its deliberate-failing-control requirement) or if the accept
case shows any host rejection line or a `FocusedElement` other than `Field
notes`.

**Where the rejection actually happens.** The brief asked for the two reject
cases to be observed "via the new rejection log line" --
`ortet: receipt bridge-action rejected target=... action=...`, added to
`ports/ortet/src/shell.rs`'s `drain_accessibility_actions`. That line did
**not** appear in either reject case's `receipt.log`/`receipt.log.err`; both
were refused one layer earlier, at the OS/UIAutomation layer itself, before
Ortet's `a11y.rs` `route()` was ever invoked:

- `focus-stale-reject`: the second `SetFocus()` on the retired-id element
  threw `System.Management.Automation.MethodInvocationException: Exception
  calling "SetFocus" with "0" argument(s): ""` -- an empty-message HRESULT
  failure consistent with the element no longer resolving in
  `accesskit_windows`' own tree state. Ortet's fresh-ID publication policy
  (a new host id on every publication, per the comment in
  `run_ortet_o2_bridge_action_receipt.ps1`) means a retired id is gone from
  AccessKit's platform-side tree entirely, not merely stale in Ortet's
  `published` map, so the OS layer refuses the call before a request can
  even be built.
- `focus-unadvertised-reject`: `SetFocus()` on the `Ortet` heading threw
  `"Target element cannot receive focus."`. `accesskit_consumer`'s
  `Node::is_focusable` is exactly `supports_action(Action::Focus) ||
  is_focused_in_tree()` (`accesskit_consumer-0.35.0/src/node.rs:96-98`), so
  UI Automation's own `IsKeyboardFocusable` property is already false for
  this node, and `uiautomationcore.dll`'s `SetFocus` implementation refuses
  before calling into the provider.

Both are genuine native refusals at the real platform boundary -- an actual
assistive-technology client is stopped from ever delivering these two
requests to the document -- which is a stronger guarantee than a host-side
log line would be, not a weaker one. But it means `route()`'s own
generation-mismatch and unadvertised-action branches remain verified only by
`a11y.rs`'s four existing in-process unit tests, not by this native receipt;
this native evidence closes the platform-level behavior the gate is
ultimately for, not the specific Rust branches inside `route()`. Full
per-case logs, snapshots, and `result.json` are in each case directory.

`diagnostics/` holds the per-hyperlink `GetSupportedPatterns`/`InvokePattern`
dump (`hyperlink-patterns.json`, confirming the empty-pattern/no-Invoke
finding above for all four hyperlinks in `article.html`) plus a first
pointer-click attempt; `diagnostics-pointer-click/` holds the retried,
dedicated pointer-click diagnostic described above.

## Environment

- Windows native host, window size 640x400, DPI scale factor 1.0.
- Source revision: `f949210ca324695bde59c80728933429a146492b` (`f949210ca32`),
  branch `slice/o2-bridge-action-20260909` off `genet` main.
- `ortet.exe` SHA-256: see `executable.json` in the artifact directory
  (recomputed by the runner at build time).
- `CARGO_TARGET_DIR=C:/Users/mark_/Code/.targets/s7-ortet-20260909`,
  `cargo build -p ortet --offline`.
- No scripted engine is involved anywhere in this receipt: `article.html` and
  `notes.html` are script-free fixtures, and Ortet's default build is
  script-free. This is a statement about the AccessKit/UIAutomation bridge
  and Livery's document-only pointer/focus targeting, not about Boa or Nova.

## Residual: Invoke does not reproduce

The 2026-09-05 acceptance is not invalidated by this finding -- it recorded
what it observed on `28fee14166580c19df464fa26845e2b998d9d75e` at that time.
It does not reproduce at the current source and environment described above.

Candidate range: 11 commits touched the pointer-target path between the
accepted source `28fee14166580c19df464fa26845e2b998d9d75e` and the current
tip, notably:

- `9a994bb4193` -- Shadow DOM across both DOMs, Livery's flat tree, style
  scoping, and retargeting.
- `f2fb66aa1c9` -- livery: transform property values, hit-testing and paint
  receipts, plus a workspace reflow.
- `cfb7cbc2e55` -- Nested browsing contexts: the context tree, frame loading,
  and composited child documents.

The defining function is `accessible_pointer_target` in
`components/genet-documents/src/engines/livery.rs` (currently at line 650;
line 644 was cited against an earlier working-tree state of this branch) and
its Livery counterpart. None of the above three commits were bisected against
each other in this receipt -- that is deliberately left to the layout lane,
per Mark's ruling not to widen this slice into `genet-livery` or
`genet-documents`.

### Reproduction commands

```
cd C:/Users/mark_/Code/worktrees/genet-s7-ortet-20260909
$env:CARGO_TARGET_DIR = 'C:/Users/mark_/Code/.targets/s7-ortet-20260909'
cargo build -p ortet --offline
# then, from a PowerShell host with System.Windows.Automation available:
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
# launch ortet.exe --url file:///.../ports/ortet/examples/article.html --size 640x400 ...
# find the 'Field notes' hyperlink AutomationElement and inspect:
$link.GetSupportedPatterns()              # -> empty
$link.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)  # -> throws
```

The full driven reproduction, including the three Focus gate cases and the
per-hyperlink empty-pattern dump, is
`support/ci/run_ortet_o2_bridge_action_focus_receipt.ps1`; its exact
invocation is:

```
./support/ci/run_ortet_o2_bridge_action_focus_receipt.ps1 `
  -ArtifactDir C:/Users/mark_/Code/testing/genet/ortet-o2-bridge-action-20260909 `
  -TargetDir C:/Users/mark_/Code/.targets/s7-ortet-20260909
```

The dedicated pointer-click diagnostic is
`support/ci/run_ortet_o2_bridge_action_pointer_diagnostic.ps1`:

```
./support/ci/run_ortet_o2_bridge_action_pointer_diagnostic.ps1 `
  -ArtifactDir C:/Users/mark_/Code/testing/genet/ortet-o2-bridge-action-20260909/diagnostics-pointer-click `
  -Exe C:/Users/mark_/Code/.targets/s7-ortet-20260909/debug/ortet.exe
```

The companion runner `support/ci/run_ortet_o2_bridge_action_receipt.ps1`
(Invoke-based, kept from the prior session) independently reproduces the
pattern gap: its `invoke/driver.log` shows `SetFocus` and re-acquisition
succeeding, then stops before any "issued native Invoke" line, because
`Invoke-Element` throws when the cached pattern lookup fails.

## Status

The O2 rejection gate is closed natively for Focus: the accept, stale-reject,
and unadvertised-reject cases are all observed through the real
UIAutomation bridge, with the two rejections landing at the OS/UIA layer
rather than at Ortet's own rejection log line (see above). The Invoke path
from the 2026-09-05 receipt is an open residual, scoped to
`accessible_pointer_target` in
`components/genet-documents/src/engines/livery.rs` and its Livery
counterpart, left for the layout lane to bisect. Whether a scripted
synthetic pointer click should independently confirm a healthy pointer
pipeline is also open: this receipt's own attempt did not navigate (see
above), which qualifies rather than confirms the brief's premise.

## Correction, 2026-09-10: the Invoke gap is not a regression

The web-platform lanes session bisected the "Invoke does not reproduce"
residual above in isolated worktrees: `28fee141665` (a pre-rebase twin of
`c3da8fba651`, not an ancestor of main) reproduces the symptom identically to
`f949210ca32`, and the three suspected commits are byte-identical on the
pointer-target path. Mechanism: `ortet --size` is in physical pixels and this
display's scale factor is 2.0, not the 1.0 this receipt assumed, so the
640x400 window is a 320x200 CSS viewport. `article.html`'s nav links lay out
at CSS y 233..255, below the fold; `Document::accessible_pointer_target`
(genet-livery `document/scrolling.rs`) returns None at its viewport
intersection, `unrevisioned_accessibility_projection` strips the Click
action for off-screen links, and accesskit_windows therefore exposes no
`InvokePattern`. At 640x1200 both sources expose `InvokePattern` for exactly
the two in-viewport links. This also explains the pointer-click diagnostic:
its coordinates targeted a link that was not on screen.

The residual section above is therefore withdrawn as a code defect and
replaced by a harness requirement: state the receipt size as a CSS viewport
or pin the scale factor, and assert the link's accessibility bounds fall
inside the window before querying patterns. The 2026-09-05 Invoke acceptance
stands. Findings and the three PowerShell probes:
`C:/Users/mark_/Code/scratch/ortet-uia-bisect-20260910/FINDINGS.md`.

