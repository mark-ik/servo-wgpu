# Cross-arena DOM adoption regression fixtures, 2026-09-09

Support lane for the "ownership router" continuation named in the realms
plan's "Next runtime boundary" and "Runtime coordination inventory,
2026-09-09". Tests only: no adoption/router production code was touched.
Branch `slice/adoption-fixtures-20260909`, worktree
`C:/Users/mark_/Code/worktrees/genet-s4-handle-20260909`, base `f949210ca32`.

## Pointer, not an edit, to the realms plan

`design_docs/2026-09-08_realms_plan.md` on this branch/`main` is 441 lines and
has no "Cross-arena adoption foundation", "Next runtime boundary" or "Runtime
coordination inventory" section — those sections (referenced in this lane's
brief) exist only in the uncommitted working tree at `C:/Users/mark_/Code/repos/genet`
(same file, 800 lines there), where the ownership-router continuation is
active right now. Per this repo's `DOC_POLICY.md` core and the live edits in
progress on that file in the other session, this receipt does **not** add a
dated line to the realms plan — that plan is actively owned by the
continuation lane. This receipt is the pointer instead. Once the continuation
commits its plan update, a line naming this fixture file belongs there.

## Manifest

`components/script-runtime-api/tests/cross_arena_adoption_fixtures.rs`, twinned on Boa
and Nova via the repo's `both_engines!` macro convention (same shape as
`mutation_observer.rs`). Two documents — a same-origin iframe (`srcdoc`, not a
foreign `src`: cross-origin `contentDocument` is null by design, a different
and already-covered refusal) and its parent — each get their own `ScriptedDom`
arena (`HostState::dom`, one per `RealmId`), which is the real cross-arena
boundary regardless of origin.

**20 regression cases (10 x 2 engines), all `#[ignore]`d** with the shared
reason `"realms plan 'Next runtime boundary' (design_docs/2026-09-08_realms_plan.md):
cross-arena adoption router not yet implemented on main"`:

| Case | Source |
|---|---|
| `wpt_adopting_orphan_*` | WPT `dom/nodes/Node-appendChild.html`, "Adopting an orphan" |
| `descendant_attribute_owner_*` | Adopting steps: descendant + attribute `ownerDocument` |
| `wpt_isconnected_iframes_*` | WPT `dom/nodes/Node-isConnected.html`, "Test with iframes" |
| `wpt_window_length_nested_context_*` | WPT `window_length.html`, "Child browsing context has a child browsing context" |
| `wpt_contextual_fragment_head_*` | WPT `template-for-html-setters.html`, "Setter createContextualFragment should not patch existing target in head" |
| `dispatch_defaultview_*` | Event propagation ends at the destination document's `defaultView` |
| `range_endpoints_*` | Live `Range` endpoints / `rangeIndex` follow the adopted node |
| `mutation_observer_both_arenas_*` | `MutationObserver` delivery in source and destination |
| `custom_element_order_*` | Custom-element reaction ordering across adoption |
| `reclamation_no_dangling_pin_*` | Release + GC: no dangling pin in either arena, via `Pins::len`/`Runtime::collect_garbage` |

**2 control cases (unignored, run by default):** `control_storage_transfer_on_{boa,nova}`
— a same-arena node whose `ownerDocument` is forced stale via
`Object.defineProperty` (simulating "storage moved, bookkeeping did not run")
and asserts the shared checking style actually trips on it, so the ten
regressions above cannot be satisfied by storage transfer alone.

## Current outcome (native Windows x86_64, debug/test profile, `CARGO_TARGET_DIR=C:/Users/mark_/Code/.targets/s4-handle-20260909`, `cargo --offline`)

- **Compiles clean.** `cargo test -p script-runtime-api --offline --test cross_arena_adoption --no-run` — 0 errors.
- **All 20 regression cases currently REFUSE**, run explicitly with `-- --ignored`. Every one panics identically at `components/genet-scripted-dom/lib.rs:373`, the G0 document-fence debug assertion:
  `NodeId from a different document (id tag N, this doc M)` — the exact
  untagged-`NodeId`/foreign-handle refusal the reflector-identity and G5 plans
  name (`#[cfg(all(debug_assertions, target_pointer_width = "64"))]`). No
  setup-phase panic in any case: `two_documents()` always completes; every
  panic is in the test body's cross-arena action. 94.9s wall for 20 cases.
- **Both control cases PASS** (`control_storage_transfer_on_boa`,
  `control_storage_transfer_on_nova`), unignored, 25.1s wall. This is the
  fixture's self-check that it distinguishes storage transfer from adoption.
- **Real bug found and fixed while building this fixture:** the first draft
  used a cross-origin iframe `src`, which correctly hit the *existing*,
  already-covered origin-security refusal (`contentDocument === null`, per
  `frame_realms.rs`'s `cross_origin_window_rejects_non_whitelisted_reads`)
  instead of the arena boundary this file exists to test. Switched to a
  same-origin `srcdoc` iframe, which still gets its own arena. A second bug —
  a `//` JS comment embedded in a `\`-continued Rust string literal, which
  silently swallowed the `Object.defineProperty` call the first control case
  needed — was caught by the control case itself failing for the wrong reason,
  fixed, and re-verified.

## Existing-suite verification

- `cargo test -p script-runtime-api --offline --test cross_arena_adoption` (this file alone): 2 passed (controls), 20 ignored, 0 failed.
- `cargo test -p genet-scripted-dom --offline`: **51/51 passed, 0 failed** (38 lib + 6 `replacement_retention.rs` + 7 `root_closure.rs`).
- `cargo test -p script-runtime-api --offline` (full lib suite, 142 tests):
  **130 passed, 12 failed.** All 12 failures are in `dom/tests.rs` and
  `lib.rs` — files this lane did not touch — and are unrelated to this
  fixture: `custom_elements_adoption_{boa,nova}`, `dom_node_events_{boa,nova}`,
  `iframe_initial_document_{boa,nova}`, `imported_stylesheet_cssom_relationships_{boa,nova}`,
  `opaque_root_policy_cost_is_bounded_{boa,nova}` (a perf-threshold assertion;
  its own printed numbers — 30–79ms per tick — suggest contention from the
  other cargo builds running concurrently on this machine during this session,
  named below), and `post_message_trace_ndjson_{boa,nova}`. These were not
  reproduced against a clean baseline (no time budget for a second full
  70+ minute build); recorded as observed, not attributed with certainty, and
  flagged rather than silently absorbed into this lane's own gate.
- `cargo test -p genet-scripted --offline` (78 tests): **77 passed, 1 failed**
  — `document::tests::livery_render_tests::child_realm_mutations_render_on_boa`,
  in `components/genet-scripted/document.rs`, not touched by this lane.

**Environment note:** this build ran concurrently with at least one other
active cargo process in a sibling worktree (`genet-s7-ortet-20260909`, seen
mid-run via `ps`) and with the continuation lane's own uncommitted work in
`C:/Users/mark_/Code/repos/genet`. Package-cache lock contention ("Blocking
waiting for file lock on package cache") was observed and waited out per
instruction; no other session's cargo process was touched or killed. One
transient `error: crate 'nova_vm' required to be available in rlib format`
occurred on one `cargo test -p script-runtime-api --offline` invocation and
cleared on retry without any source change — recorded as a build-system
transient, not a code defect.

## Spots that need a one-line update once the continuation lands

- The ten regression cases lose their `#[ignore]` once cross-arena
  `appendChild`/`adoptNode` routes through the "current owner" the plan
  describes rather than the raw `NodeId` tag check.
- If the continuation's permanent u64 `NodeId` and
  `ScriptedDom::transfer_detached_subtree_to` land with different names or
  signatures than described in the (uncommitted) plan text this lane read for
  context, nothing in `cross_arena_adoption.rs` calls either directly — the
  fixture is written entirely against the authored JS surface
  (`appendChild`, `adoptNode`, `Range`, `MutationObserver`, custom elements),
  so no Rust-level rename is needed on this file.
- Once `design_docs/2026-09-08_realms_plan.md` gains the sections this
  receipt currently points at instead of quoting, add a line there citing
  `components/script-runtime-api/tests/cross_arena_adoption_fixtures.rs` by path.
