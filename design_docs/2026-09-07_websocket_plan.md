# WebSocket: a browser API over netfetcher's transport

**Date:** 2026-09-07

**Status:** landed 2026-09-07.

**Parent:**
[`2026-09-07_deferred_web_platform_lanes_scoping.md`](2026-09-07_deferred_web_platform_lanes_scoping.md),
whose WebSocket section named what the transport lacks and what policy the
connection path must enforce, and
[`2026-09-06_web_platform_wpt_census.md`](2026-09-06_web_platform_wpt_census.md),
which recorded `websockets` at 0 of 1,392 subtests.

**Seam it copies:** [`2026-09-07_xhr_plan.md`](2026-09-07_xhr_plan.md) — native
sinks, completions delivered through the drive loop, a per-test handler over the
shared `genet-wpt` tokio worker.

## Purpose

`WebSocket` was undefined in the runtime, so every file in the WPT `websockets`
directory failed at its first statement. `components/netfetcher/src/websocket.rs`
had a working `ws://` / `wss://` transport, but one shaped for a library caller,
not for a browser API: `connect` took only a URL and threw the handshake response
away; `send` and `recv` collapsed every failure to `bool` / `Option`;
`WsMessage::Close` dropped the code and reason; nothing counted buffered bytes.

WebSocket is also not a second shape of `fetch()`. The Fetch algorithm does not
wrap it, so none of the protections a subresource fetch inherits — scheme rules,
blocked ports, HSTS, mixed content, redirect refusal, the CSP hook — apply
unless this path enforces them itself. That is the reason the lane has two
halves: a transport contract that carries what a browser needs and enforces
what a browser must, and a host object that is nothing but the script-visible
state machine over it.

## Phases

### W1 — the transport contract

`components/netfetcher/src/websocket.rs`, keeping the public API free of
tungstenite types.

- `WsRequest`: URL, requested subprotocols, initiator `Origin`, a
  secure-context flag, a credentials flag, extra headers.
- `WsError`: `BadScheme` / `Fragment` / `BlockedPort` / `MixedContent` /
  `CspBlocked` / `Redirect` / `Handshake` / `ProtocolMismatch` / `Closed` /
  `Io`, with `Display` and `std::error::Error`.
- `WsClose { code, reason, clean }` and `WsIncoming::{Message, Close}`;
  `WsMessage` loses its `Close` variant, which was never a message.
- `WebSocket::{protocol, extensions, buffered_amount}`; `send` returns
  `Result<(), WsError>` and moves the payload's byte count through the queue.
- Cookies through `FetchContext::cookies` — the same jar the fetch path uses —
  in both directions: a `Cookie` header on the handshake, `Set-Cookie` from its
  response recorded back.
- Policy in `connect`, against the same caller-owned `FetchContext`: `ws` /
  `wss` only with `http` / `https` normalized and a fragment rejected
  (`validate_ws_url`); Fetch's bad-port list; HSTS upgrade before the
  mixed-content test; `ws:` from a secure context blocked outright (a WebSocket
  is never optionally-blockable, so there is no auto-upgrade to fall back on);
  the CSP `connect-src` hook, keyed on the `http`/`https` spelling of the URL;
  and a 3xx handshake answer typed as `WsError::Redirect` rather than followed.

**Done-conditions**

- `cargo test -p netfetcher` green, with a test per policy rule and a live
  echo-server round trip. **Met** (15 `websocket::tests`, 85 lib tests total).
- No tungstenite type in the public API. **Met** (`WsRequest` / `WsError` /
  `WsClose` / `WsIncoming` / `WsMessage` are the whole surface).

### W2 — the host seam

`WebSocketHandler` in `components/script-runtime-api/websocket.rs`, beside
`FetchHandler`, with `connect` / `send_text` / `send_binary` / `close`; a new
`HostState::websocket` field; and six completion entry points on `Runtime`
(`ws_open`, `ws_message_text`, `ws_message_binary`, `ws_flushed`, `ws_error`,
`ws_close`) plus `pending_websockets` and `fail_all_websockets` for the drive
loop. The implementation is `NetWebSocketHandler` in `ports/genet-wpt/src/net.rs`,
on the existing tokio worker: one task per socket, owning the connection and
`select!`ing between its command channel and its incoming frames, reporting
through the same per-test channel the deferred fetches use.

**Done-conditions**

- No network stack enters `script-runtime-api`: only the trait does. **Met.**
- The drive loop treats a live socket as outstanding network work and sweeps it
  at the deadline. **Met** (`drive_wall`).
- `cargo test -p genet-wpt --features netfetch` green. **Met** (65 tests).

### W3 — the host object

`WebSocket` and `CloseEvent` in the new `websocket.rs` bootstrap: the
constructor's URL and subprotocol validation, the four ready states and their
constants on both the constructor and the prototype, `send` for string /
`ArrayBuffer` / `ArrayBufferView` / `Blob`, `close` with code and reason
validation, `bufferedAmount`, `binaryType`, `extensions`, `protocol`, `url`,
the four events, and the spec's error-then-close ordering.

**Done-conditions**

- `cargo test -p script-runtime-api` green with new tests on **both** engines.
  **Met** (`tests/websocket_binding.rs`, 12 bodies × Boa and Nova = 24 tests).

### W4 — measurement

A `pre` runner built from `HEAD` before the first edit and a `post` runner built
from a clean worktree carrying only this lane's files; `websockets` in disk mode
both sides; `fetch` and `xhr` in disk mode as the hold; and the first server-mode
`websockets` run, against a spawned `wpt serve`.

**Done-conditions**

- `websockets` moves substantially from 0 subtests. **Met** (1,090 disk,
  1,144 server).
- `fetch` and `xhr` hold. **Met** (byte-identical maps).
- Every pass-to-fail movement explained or fixed. **Met** (there are none).

## Scope and provenance

| Item | Value |
|---|---|
| genet commit at start | `c9254000b0e`, reached through the merge `54f5d163555` |
| WPT tree | `tests/wpt/tests` as vendored at that commit |
| `MANIFEST.json` SHA-256 | `d5ec5be9bf1a75ed00d7e7ab28afe8a694a55e11682ba74305874d70b18dd422` |
| `pre` runner | `genet-wpt` release, `--features netfetch`, built from `HEAD` before the first edit, SHA-256 `44aab8f6e87e58491d6c6383af7943459ee81227586e0a72a06f1364b988c695` |
| `post` runner | the same, built from a detached worktree at `HEAD` carrying only this lane's nine files, SHA-256 `8522f3b6c3ef76055dc2474ca4b217a8cecae81e9565134db722ac1215908818` |
| Raw results | `Code/testing/genet/wpt-ledger/2026-09-07_websocket/` (outside Git) |
| Disk lane | `testharness`, engine Boa, renderer Livery, `--jobs 8 --timeout 300` |
| Server lane | the same, `--spawn-server --drive-deadline 5 --jobs 4 --timeout 120` |

The `post` runner was built from a **worktree**, not the working tree: another
lane was concurrently editing `components/script-runtime-api/dom/` and
`worker.rs`, and a runner built over those edits would have carried movement
this lane did not cause. The worktree holds `HEAD` plus exactly the nine files
below, so the `pre`/`post` pair differs by this lane and nothing else.

Files this lane owns and changed:
`components/netfetcher/src/{websocket.rs,lib.rs}`,
`components/script-runtime-api/{websocket.rs,lib.rs,tests/websocket_binding.rs}`,
`ports/genet-wpt/{Cargo.toml,src/net.rs,src/harness.rs,src/testharness.rs}`.

## Policy enforced on the connection path

Everything below is in `netfetcher::websocket::connect`, tested in
`components/netfetcher/src/websocket.rs`'s test module, and reachable from
script only as the spec's single `error` event.

| Rule | Behaviour | Test |
|---|---|---|
| Scheme | `ws` / `wss` pass; `http` / `https` normalize to them; anything else is rejected | `scheme_and_fragment_rules` |
| Fragment | any fragment, including a bare `#`, is rejected | `scheme_and_fragment_rules` |
| Blocked ports | Fetch's bad-port list, checked on the effective port (default included) | `blocked_ports_are_the_fetch_list`, `a_blocked_port_is_refused_before_any_socket` |
| HSTS | a known-secure host upgrades `ws` to `wss` **before** the mixed-content test | `hsts_upgrades_ws_to_wss_before_the_mixed_content_test` |
| Mixed content | `ws:` from a secure context is blocked, never auto-upgraded | `ws_from_a_secure_context_is_mixed_content` |
| CSP | the `connect-src` hook is consulted with the `http(s)` spelling of the URL | `csp_can_refuse_the_connection` |
| Redirects | a 3xx handshake answer is `WsError::Redirect` and is never followed | `a_redirect_answer_is_typed_as_a_redirect_and_never_followed` |
| Credentials | cookies attach from, and `Set-Cookie` records into, the shared jar | `cookies_ride_the_handshake_through_the_shared_jar`, `an_uncredentialed_connection_sends_no_cookie` |
| Origin | the initiator origin travels as `Origin` | `the_origin_header_travels` |
| Subprotocols | a server selection that was not offered is `ProtocolMismatch` | `selects_an_offered_subprotocol`, `a_server_selected_protocol_that_was_not_offered_fails` |

The script-visible failure detail stays what the specification allows: `error`,
then `close` with `wasClean` false, code 1006 and an empty reason. `WsError`
exists for the host's logs and for these tests, and is dropped at the seam.

## Findings

Dated 2026-09-07, verified in this checkout.

### F1 — the `__ws` prefix was already taken

`components/script-runtime-api/worker.rs` uses `__wsInit` / `__wsPump` for
*worker scope*, which predates this lane. The WebSocket sinks and completions
are therefore `__websocket_*` and `__websocket*`. A one-word collision in a flat
global namespace is the kind of thing a bootstrap-per-feature design makes easy
to hit; the prefix is now unambiguous on both sides.

### F2 — one URL parser, reached from the bootstrap

The constructor does no URL parsing of its own. `__resolve_url` and
`__url_parse` / `__url_with`, which the fetch bootstrap already installs, give
it WHATWG resolution against the document base, the scheme, and the fragment;
the WebSocket rules are then four lines on top. This is why `Create-http-urls`
and `Create-non-absolute-url` pass in disk mode with no server at all: they are
URL questions, not network ones. It is also the reason the WebSocket surface is
installed *after* the fetch surface — it depends on those sinks and on `Blob`.

### F3 — `MessageEvent.origin` is the socket's origin, not the page's

The first test written here asserted the page origin (`http://page.example`) and
failed. HTML initializes a WebSocket message event's `origin` to the
serialization of the **WebSocket URL's** origin, so it carries the `ws` scheme:
`ws://a.example`. The implementation was right and the expectation was wrong;
the test now records why.

### F4 — the disk-mode ceiling is `.sub.js`, not the API

137 `websockets` files still report `error/evaluation-threw` in disk mode, the
same count as on the baseline. Their shared `constants.sub.js` is a template
(`ws://{{host}}:{{ports[ws][0]}}/echo`), and the disk loader serves it
unsubstituted, so the very first `new WebSocket(...)` throws a `SyntaxError` on
an unparseable port — correctly. Nothing about the API moves them; the server
lane is their measurement, and it takes them to 0 errors.

### F5 — a 512-**byte** cut inside a character panicked three files

`looks_like_xml` in `ports/genet-wpt/src/harness.rs` sliced its probe at
`min(512)` bytes. `websockets/constructor/016.html` carries a U+FFFD straddling
that offset, so all three of its variants panicked the worker rather than
reporting. Backed off to a char boundary, with a regression test. This is a
runner defect the baseline shares; it is fixed here because it is this
directory's own residual and the file is already in this lane.

### F6 — a dedicated Worker has no WebSocket

All 214 `no-results` files in the server-mode map are `.any.worker.html`
variants. The Worker lane gives the worker thread its own `Runtime` and routes
its resource loads back to the page; nothing routes a socket. The bootstrap is
installed in worker scope, so `WebSocket` exists there and fails every
connection, and the harness reports no subtests. Wiring a relay means touching
`worker.rs`, which belongs to that lane, so it is named here rather than done.

### F7 — the shape-only IDL table was left alone, deliberately

`tests/wpt/tests/interfaces/websockets.idl` could join `dom.idl` / `cssom.idl` /
`selection-api.idl` as a shape source in `support/idl-interface-table`. It was
not, for two reasons. The gain is nil: the shape pass defers to an
implementation when it finds the name taken, and this bootstrap already stamps
both class strings and builds both prototype chains. The cost is real:
regenerating `components/script-runtime-api/dom/html_interfaces_generated.rs`
rewrites a file another lane was editing at the time, and a wholesale rewrite
could clobber their regeneration. **This is Mark's call to reverse**: if the
generated table should own the declarations anyway, the change is one string in
`generate_interfaces` plus a regeneration, and is safe once `dom/` is quiet.

## Results

### `websockets`, disk mode

| | `pre` (this lane's runner, `HEAD`) | `post` | Delta |
|---|---:|---:|---:|
| Files (variants) | 729 | 729 | 0 |
| All-pass | 0 | **76** | **+76** |
| With failures | 565 | 492 | -73 |
| Error | 140 | 137 | -3 |
| No results | 24 | 24 | 0 |
| Subtests passed / total | 0 / 1,874 | **1,090 / 1,877** | +1,090 / +3 |

34 distinct test files, 76 variants. The 2026-09-06 census recorded this
directory at 0 of 1,392 subtests over 375 enumerated files; this runner
enumerates 729 variants of the same tree, so the census figure is the historical
record and the `pre` map is this lane's baseline.

### `websockets`, server mode — the first one

No baseline exists: no census has run `websockets` against a live `wpt serve`,
which is the only place a socket can connect.

| | Server mode |
|---|---:|
| All-pass | **254** of 729 |
| With failures | 261 |
| Error | **0** |
| No results | 214 (every one a `.any.worker.html` variant, F6) |
| Subtests passed / total | **1,144 / 1,586** |

161 distinct non-worker files pass, across `binary`, `closing-handshake`,
`constructor`, `cookies`, `opening-handshake`, `security` and every
`interfaces/WebSocket/*` group — `bufferedAmount` 10, `send` 11, `events` 12,
`close` 6, `readyState` 8, `url` 7, `constants` 6, `extensions` 1, `protocol` 1.
69 `?wss` variants pass, so the TLS lane is exercised too.

### `fetch` and `xhr`, disk mode — the hold

| | `fetch` pre | `fetch` post | `xhr` pre | `xhr` post |
|---|---:|---:|---:|---:|
| All-pass | 87 | 87 | 95 | 95 |
| With failures | 507 | 507 | 300 | 300 |
| Error | 27 | 27 | 5 | 5 |
| Subtests passed / total | 2,262 / 7,574 | 2,262 / 7,574 | 373 / 1,578 | 373 / 1,578 |

Byte-identical status maps: `diff_census.py` reports "no movement" for both.

### Explaining every movement

`diff_census.py`, `post` against this lane's own `pre`:

| Count | Directory | Transition | Cause |
|---:|---|---|---|
| 76 | websockets | `fail` -> `pass` | the file's subtests ran and passed once `WebSocket` existed |
| 3 | websockets | `error/panic` -> `fail` | the char-boundary fix (F5): `constructor/016.html` now reports instead of panicking |

**Pass-to-fail movements: none**, at file level or subtest level, in any of the
three directories. No file's subtest pass count went down.

## Regression manifest

The exact files this lane requires to keep passing. Disk mode, `websockets`,
all variants (`?default`, `?wss`, `?wpt_flags=h2` where the file declares them):

```
websockets/Create-asciiSep-protocol-string.any.html
websockets/Create-asciiSep-protocol-string.any.worker.html
websockets/Create-http-urls.any.html
websockets/Create-invalid-urls.any.html
websockets/Create-invalid-urls.any.worker.html
websockets/Create-non-absolute-url.any.html
websockets/Create-nonAscii-protocol-string.any.html
websockets/Create-nonAscii-protocol-string.any.worker.html
websockets/Create-protocol-with-space.any.html
websockets/Create-protocol-with-space.any.worker.html
websockets/Create-protocols-repeated-case-insensitive.any.html
websockets/Create-protocols-repeated-case-insensitive.any.worker.html
websockets/Create-protocols-repeated.any.html
websockets/Create-protocols-repeated.any.worker.html
websockets/Create-url-with-space.any.html
websockets/Create-url-with-space.any.worker.html
websockets/Create-url-with-windows-1252-encoding.html
websockets/constructor/001.html
websockets/constructor/004.html
websockets/constructor/007.html
websockets/constructor/008.html
websockets/constructor/021.html
websockets/interfaces/CloseEvent/constructor.html
websockets/interfaces/CloseEvent/historical.html
websockets/interfaces/WebSocket/bufferedAmount/bufferedAmount-defineProperty-getter.html
websockets/interfaces/WebSocket/bufferedAmount/bufferedAmount-defineProperty-setter.html
websockets/interfaces/WebSocket/constants/005.html
websockets/interfaces/WebSocket/constants/006.html
websockets/interfaces/WebSocket/events/020.html
websockets/interfaces/WebSocket/readyState/004.html
websockets/interfaces/WebSocket/readyState/005.html
websockets/interfaces/WebSocket/url/005.html
websockets/interfaces/WebSocket/url/006.html
websockets/opening-handshake/received-301-code.html
```

Plus, in the repository's own suites:

- `components/netfetcher/src/websocket.rs` — the 15 transport tests, one per
  policy rule in the table above plus the echo round trip.
- `components/script-runtime-api/tests/websocket_binding.rs` — 12 bodies on
  both Boa and Nova.
- `ports/genet-wpt/src/harness.rs::tests::the_xml_probe_survives_a_multibyte_char_at_the_cut`.

The server-mode set (254 variants / 161 files) is not frozen as a manifest: it
depends on a live `wpt serve` and on the 5s drive deadline, both of which are
run-time conditions rather than properties of this tree. Its map is the receipt.

## Gates

| Gate | Result |
|---|---|
| `cargo test -p netfetcher` | green: 85 lib (15 new `websocket::tests`), 0 failed |
| `cargo test -p script-runtime-api` | green: 125 lib + 24 new `websocket_binding` (12 bodies × 2 engines) + 136 other integration tests, 0 failed |
| `cargo test -p genet-wpt --features netfetch` | green: 65 passed (1 new), 3 ignored, plus `fetch_netfetcher` 1 passed |
| `cargo check --workspace --features genet-wpt/netfetch` | clean |
| `cargo clippy` on the touched files | clean. The remaining `genet-wpt` warnings near the change (`items_after_test_module`, an unfulfilled `dead_code` expectation on the WebGL entry point) are pre-existing at `HEAD` |
| `rustfmt` | applied per file, not per crate, so another lane's in-progress files were not reformatted |
| `websockets` disk census | 0 -> 76 all-pass, subtests 0/1,874 -> 1,090/1,877 |
| `websockets` server census | 254 all-pass, 0 errored, subtests 1,144/1,586 |
| `fetch` / `xhr` disk census | hold: byte-identical maps |

## Progress

- **2026-09-07** — plan written; `pre` runner built from `HEAD` before the first
  edit and its baseline taken: `websockets` 0 all-pass / 565 fail / 140 error /
  24 no-results of 729 variants, subtests 0/1,874.
- **2026-09-07** — W1-W4 landed. `websockets` 0 -> 76 all-pass and 0 -> 1,090
  subtests in disk mode, 254 all-pass and 1,144/1,586 subtests in the first
  server-mode run; `fetch` and `xhr` byte-identical. No pass-to-fail movement.

## Open, for whichever lane owns it

- **Worker-hosted WebSocket** is the whole remaining server-mode `no-results`
  class: 214 `.any.worker.html` variants (F6). It needs a relay from the worker
  thread to the page's `WebSocketHandler`, in `worker.rs`.
- **`permessage-deflate`** is reported, not negotiated: `extensions` carries
  whatever the server chose, and the transport does not offer the extension.
- **`WebSocketStream`** (the `websockets/stream/` subdirectory) is untouched.
- **`send(Blob)` is synchronous** because the bootstrap's `Blob` is already
  resident bytes. A streaming `Blob` would have to queue and report
  `bufferedAmount` asynchronously.
- **`bufferedAmount` under backpressure** is honest but coarse: the count drops
  when the transport's sink accepts the frame, which for a fast local socket is
  immediately. `send-many-64K-messages-with-backpressure` needs a sink that
  reports real queue depth.
- **The generated IDL table** could declare `WebSocket` and `CloseEvent` from
  `websockets.idl` (F7). Deferred on a concurrency argument, not a technical
  one.
- **The server lane's 5s drive deadline** is what the harness-repair plan chose;
  a longer one may convert some of the 261 server-mode failures. Not measured
  here.
