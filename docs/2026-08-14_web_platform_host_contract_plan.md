# Web platform host contract plan

**Date:** 2026-08-14  
**Status:** active; S0 and S1 landed; S2 is implemented locally through the
policy and Weld adapter boundary, with rendered prompts and headed proof open
**Owners (reconciled 2026-09-05):** Genet's `document-session-api` owns
retained document-session, accessibility, capability and capture contracts.
Mere's `inker` owns surface orchestration and the `WebSurface` vocabulary;
hosts apply graph, profile, policy, and presentation effects.

**Current ownership note, 2026-09-05.** The S0-S5 status and test counts below
are their dated implementation records, last updated here in August. Before
resuming an open S gate, refresh the corresponding Mere/Turnstone consumer
and its current receipt. The September platform split moved Inker, its
surface adapters and application policy out of Genet; it did not move the
engine-facing session API out. Inker re-exports that API, so an `inker::`
import alone no longer identifies the implementation owner. Source anchors:
`components/shared/document-session-api/src/lib.rs` in this repository and
`mere/crates/inker/inker/src/{lib,surface_engine}.rs`. The split is recorded in
`mere/design_docs/mere_docs/implementation_strategy/2026-09-02_platform_boundary_and_repository_topology_plan.md`.

## Thesis

The contract between a browsing engine and its host uses web-platform terms,
not CEF, WebView2, WebKit, or Servo terms. Weld, Scry, Graft, Genet's rungs,
and Smol are adapters or native implementations of that contract.

This is not a promise that every engine implements every feature. Each engine
reports support against the same operation or event and names any degradation.
A backend-specific protocol may implement a standard operation, but it does
not become the shared API. In particular, Chrome DevTools Protocol is a Weld
adapter detail; the shared automation vocabulary follows WebDriver and
WebDriver BiDi.

Related plans:

- [W3C mechanism adoption](./2026-07-05_w3c_mechanism_adoption_plan.md)
- [shared web-platform API middle](./2026-05-25_web_platform_api_shared_middle_plan.md)
- [Turnstone engine adoption](../../turnstone/design_docs/2026-08-03_turnstone_engine_adoption_plan.md)

## Standards vocabulary

The contract uses five scopes. They must not collapse into one `WebSurface`
bag.

| Scope | Standard source | Durable identity | Host treatment |
|---|---|---|---|
| Resource | WHATWG URL, Fetch, HTTP | serialized URL plus response provenance | addressed content; eligible for a graph node |
| Navigable | WHATWG HTML navigation and session history | host/engine browsing-context id | live presentation of a sequence of resources |
| Origin | URL/HTML origin model | tuple or opaque origin | permission and security-policy lookup key |
| Profile | cookie storage model and user-agent policy | persona/session profile id | registry-backed state; never a content node |
| Representation | HTML printing, CSS Paged Media, WebDriver capture/print | source resource + parameters + digest | attached artifact; promoted to a node only by explicit import |

Normative anchors:

- URL and origin: <https://url.spec.whatwg.org/>
- navigables and session history: <https://html.spec.whatwg.org/multipage/document-sequences.html>
- navigation and auxiliary contexts: <https://html.spec.whatwg.org/multipage/nav-history-apis.html>
- downloads: <https://html.spec.whatwg.org/multipage/links.html#downloading-resources>
- printing: <https://html.spec.whatwg.org/multipage/timers-and-user-prompts.html#printing>
- paged layout: <https://www.w3.org/TR/css-page-3/>
- permissions: <https://www.w3.org/TR/permissions/> and
  <https://www.w3.org/TR/permissions-policy/>
- HTTP authentication: <https://www.rfc-editor.org/rfc/rfc9110.html#section-11>
- cookies: <https://datatracker.ietf.org/doc/draft-ietf-httpbis-rfc6265bis/>
- pointer input: <https://www.w3.org/TR/pointerevents/>
- drag and drop: <https://html.spec.whatwg.org/multipage/dnd.html>
- automation, script results, screenshots, and print output:
  <https://www.w3.org/TR/webdriver/> and <https://www.w3.org/TR/webdriver-bidi/>

## Product projection

### Addressed content

- A committed top-level navigation changes the current graph member's address
  and appends to that member's navigation lineage. A redirect does the same.
- An auxiliary navigable request (`window.open`, `target=_blank`) is a request
  for another addressed presentation. Turnstone chooses a tile, pane, window,
  or denial according to user activation and policy. The engine does not choose
  the GUI container.
- A completed download retains its source URL and response metadata. Turnstone
  may admit that addressed resource as a node and attach the local bytes as a
  representation. The destination path is not the resource identity.
- A dropped URL enters the ordinary open-address path. A dropped file enters
  the ordinary import/sniff path. Drag and drop does not get a second graph
  ingestion model.

### Associated state

- Cookies are profile-scoped HTTP state. Inspection and deletion are profile
  registry operations, optionally projected as facets associated with an
  origin or node. Cookie values are not nodes.
- Permission decisions are keyed by origin, descriptor, and profile. A prompt
  is a pending user-agent decision; the retained grant/denial lives in the
  permission registry.
- HTTP authentication is a challenge/credential exchange scoped to an RFC 9110
  protection space. Credentials belong in a credential provider, not graph
  facets. A node may retain that a challenge occurred, never the secret.
- Cursor shape, load progress, context-menu target, focus, and IME geometry are
  ephemeral presentation state.

### Representations

- `window.print()` and a host Print action run the HTML printing steps. Native
  printer UI is a host opportunity; it creates no graph node.
- Save as PDF produces a paged representation of the current resource. It is
  attached to that resource with media type, digest, creation parameters, and
  provenance. **Import PDF** is the explicit action that promotes the PDF into
  an independently addressed graph member.
- A screenshot is a visual representation/receipt associated with a resource,
  navigable state, viewport, and capture parameters. It is not automatically
  content.

## Shared command and event families

The public contract will be grouped by standard behavior rather than one
backend-shaped trait:

1. `NavigableControl`: navigate, reload, stop, traverse; emits started,
   committed, completed, failed, title, and auxiliary-navigable events.
2. `InputControl`: Pointer Events-shaped pointer contacts, UI Events-shaped
   keyboard input, wheel input, IME, and HTML `DataTransfer` drag/drop.
3. `ProfileControl`: cookies and retained site state. Profile operations are
   asynchronous and keyed explicitly; no process-global jar is implied.
4. `UserAgentPolicy`: permission requests and HTTP authentication challenges,
   each with a request id, origin/protection-space data, and an explicit answer.
5. `RepresentationControl`: print opportunity, paged/PDF output, and screenshot
   requests, all asynchronous and correlated by request id.
6. `AutomationControl`: WebDriver/BiDi-shaped realm targeting, typed remote
   values, exception details, log events, and screenshot/print commands. CDP
   is one adapter underneath this family, never a capability exposed to all
   engines.

Each producer has one ordered event stream. Convenience pollers must not drain
and discard events of other kinds. Commands that complete asynchronously carry
correlation ids through that stream.

## Capability reporting

Capability status attaches to an operation, not a marketing feature name. For
example:

- `representation.print.opportunity`: native UI, host UI required, or unsupported
- `representation.pdf`: supported/partial/unsupported with paged-media limits
- `automation.evaluate`: realm targeting, typed result, exception details
- `input.pointer`: mouse/pen/touch device kinds and supported sensor fields
- `profile.cookies`: read/write/delete/change observation and attribute fidelity

`Partial` names the missing semantic field or lifecycle. A handler existing is
not sufficient evidence for `Supported`; a headed or protocol receipt must
exercise both request and answer paths.

## GUI consequences for Turnstone

- The Apparatus pane shows node/resource facts and associated origin/profile
  state. It does not mint settings, cookies, or permissions as nodes.
- Pending permission and authentication requests appear as transient prompts
  anchored to the requesting tile and origin. Durable decisions remain visible
  through the Apparatus facet projection.
- Downloads appear as activity tied to their source node. Completion offers
  Reveal, Attach, or Import; only Import creates a new member when appropriate.
- Auxiliary navigables enter the ordinary placement policy. User activation,
  opener/noopener, sandbox, and requested disposition are data supplied to that
  policy.
- Print, Save PDF, and Capture Snapshot are node actions. Their outputs attach
  to the node's representation list; Import is separate.

## Execution gates

### S0. Event integrity and terms

- Replace lossy specialized pollers with one ordered `WebSurfaceEvent` stream.
- Document the five scopes above in `inker` and keep backend diagnostics
  explicitly outside standard events.
- Consume committed navigation, title, and cursor events in Turnstone.

Done when an in-page Weld navigation updates the same graph member's address
and lineage, the title follows, hovering a link changes the host cursor, and a
mixed event fixture proves no event is discarded.

**Landed 2026-08-14.** `WebSurface::poll_web_event` is the host's single event
drain; the former specialized pollers that could discard a different event
kind are gone. A mixed navigation/message fixture preserves producer order.
Turnstone consumes committed URL, title, auxiliary-navigable, failure, crash,
and cursor callbacks. Its Windows Weld scenario moves over Example Domain's
link, requires a `surface-cursor` receipt, clicks it, and requires the IANA URL
plus `content-navigated` on the same member. The fresh-profile run ended
`RESULT ok`; the full Weld-feature library suite passed 247 tests with 4
explicitly ignored endpoint receipts.

### S1. Pointer and drag input

- Replace the current pen-shaped `PointerEvent` with Pointer Events fields:
  pointer id/type/primary, phase, buttons, contact geometry, pressure,
  tangential pressure, tilt, twist, altitude/azimuth where supplied.
- Lower winit touch contacts through that type. Weld uses CEF touch input;
  Scry maps onto its producer; Genet rungs dispatch the same DOM event model.
- Add a MIME-labelled `DataTransfer` item list and one cross-engine drag
  lifecycle. Files and URLs rejoin ordinary import/open actions.

Done when mouse, pen, two simultaneous touch contacts, host-to-page file drop,
and page-to-host link drag have equivalent receipts on every engine that claims
support.

**Landed 2026-08-14 at the reported-support boundary.** `inker` now carries
Pointer Events ids, device kinds, primary state, phases, DOM button masks,
contact geometry, pressure and pen orientation fields. Its HTML drag contract
carries the inherited mouse-button state plus a MIME-labelled `DataTransfer`
list of string and file items. Turnstone lowers winit mouse and independently
captured touch contacts through that contract. File hover and drop use separate
UI turns so Chromium can answer cancellable `dragover` before `drop`.

The fresh-profile Windows Weld receipt ended `RESULT ok`: two simultaneous
touch ids reached the DOM, a host file became one DOM `File` named
`s1_drop.txt` with `text/plain`, and a page link drag returned
`text/uri-list` to Turnstone. Scry maps the Pointer Events subset its producer
accepts and reports generic drag data unsupported. Graft exposes the shared
drag seam but reports it unsupported until a concrete host implements it.
Turnstone reports Weld pen, contact geometry, twist, and page-to-host native OS
drag completion as `Partial`; winit does not supply the first three fields or
a source-drag loop. Those boundaries are not promoted by CEF accepting the
lower-level values. The final Weld-feature Turnstone suite passed 251 tests
with 4 explicit endpoint receipts ignored; Inker passed 95, Scry 13, Graft 2,
the Weld engine 2, and Welding 48.

### S2. Requests and policy

- Add typed permission descriptors/states and answerable request ids.
- Add RFC 9110 authentication challenges and credential-provider answers.
- Store decisions in profile/origin registries; project summaries as facets.

Done when grant, denial, dismissal, restart retention, private-profile
isolation, and an unanswered-request timeout are tested without storing a
credential in graph data.

**Implemented locally 2026-08-15; the gate remains open at headed/UI proof.**
`inker` now carries surface-scoped `UserAgentRequestId`s, typed Permissions API
descriptors and states, explicit grant/deny/dismiss answers, and RFC 9110
protection-space challenges. Credential answers deliberately implement neither
serialization nor ordinary secret-bearing debug output. `WebSurface` has
answer methods for both request families. The Weld adapter forwards those
answers; Turnstone enables CEF's held permission and authentication callbacks,
maps their backend ids and challenge fields, and calls CEF's grant/deny,
credential, or cancel methods. Scry's informational system-handled auth event
remains a diagnostic rather than pretending to be an answerable request.

Turnstone owns a default-profile registry at
`<data_root>/profiles/default/web-policy.json`, keyed by canonical origin plus
permission descriptor. Private registries are isolated and memory-only.
Credentials live in a process-memory provider keyed by protection space and
never enter that JSON. Pending request events include the surface member and
request id; public shell answer methods use the same correlation pair. The
timeout is configurable with `TURNSTONE_WEB_REQUEST_TIMEOUT_MS` and dismisses
permissions or cancels authentication without retaining a decision. Node
facets receive only the `web.user-agent-policy` summary: descriptor/state and
non-secret challenge metadata.

Focused receipts:

- `cargo test -p inker -p weld-engine --offline`: Inker 95 passed and
  Weld-engine 2 passed.
- `cargo test --lib web_policy --offline` in Turnstone: 6 passed, covering
  grant, denial, dismissal, restart retention, private-profile isolation,
  timeout, credential-provider reuse, and an assertion that the facet contains
  neither username nor password.
- `cargo check --lib --features weld --offline` in Turnstone passed against the
  local Weld/Welding adapters.

This is not yet a completed S2 claim. Turnstone publishes correlated pending
events and answer methods, but does not render the anchored prompt UI. A headed
CEF run has not yet proved page-observed grant, denial, dismissal, Basic-auth
success/cancel, and timeout. Weld therefore reports permissions and
authentication as `Partial`, not `Supported`.

### S3. Addressed downloads and representations

- Model download lifecycle with source URL, response metadata, suggested name,
  local representation, progress, cancellation, and failure.
- Model print/PDF/screenshot as representation requests and results.
- Add Turnstone Attach/Import choices and provenance facets.

Done when one downloaded resource imports under its network address, one PDF
attaches without minting a node, explicit PDF import mints one, native print
spools on supported hosts, and screenshots retain viewport/source provenance.

### S4. Standard automation

- Add WebDriver BiDi-shaped browsing-context and realm ids, async evaluate,
  typed remote values, exception details, logs, screenshots, and print.
- Adapt Weld through CDP where CEF requires it; implement Genet directly; mark
  unsupported pieces on Smol/Scry/Graft rather than leaking CDP.

Done when the same host script obtains title, structured object result,
exception detail, screenshot, and navigation events from two different engine
families without branching on the backend.

### S5. Capability matrix and conformance

- Generate the picker/Apparatus matrix from operation-level capability data.
- Back standard claims with WPT/WebDriver tests where applicable and headed
  host receipts where platform UI is involved.

Done when every selectable engine/rung has a truthful matrix and no capability
is inferred from another engine or from a registered-but-unobserved callback.

## Stop rules

- Do not add a shared `cdp_*`, `cef_*`, `webview2_*`, or `webkit_*` type.
- Do not create graph nodes for profile state or transient presentation state.
- Do not call a callback registration `Supported` without an exercised answer
  path.
- Do not make a representation independently addressed without explicit import
  or an existing canonical address.
- Do not build separate ingestion paths for downloads and drag/drop; return to
  the normal address/import pipeline.
