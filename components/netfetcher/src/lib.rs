/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! # netfetcher
//!
//! A portable **WHATWG-Fetch** network engine for the Mere ecosystem: Servo's
//! `net` made portable — the Fetch algorithm (CORS, cookie jar, HTTP cache,
//! redirects, HSTS, mixed-content, CSP hooks, content-encoding) lifted off
//! Servo's `ipc-channel` / resource-thread coupling and exposed as a
//! directly-callable async **library**, plus an HTTP/3 lane.
//!
//! **Layering:** Mere owns networking and drives netfetcher (off the UI thread,
//! in a `FetcherPool` worker); serval and other renderers stay byte-consuming
//! and never link this crate. The JS `fetch()` binding calls it *through the
//! host*, not by linking it. See the plan:
//! `mere/design_docs/archive_docs/2026-06-09_completed_plans/2026-05-25_netfetcher_plan.md`.
//!
//! ## Status — increments 1–5 (2026-05-26)
//!
//! - **1** h1/h2 GET/POST over hyper + rustls, redirects, streaming bodies with
//!   on-the-fly `Content-Encoding` decode.
//! - **2** RFC 6265bis cookie jar; RFC 9111 cache (freshness + revalidation).
//! - **3** cross-origin model: response tainting, CORS (simple + preflight +
//!   header filtering), HSTS, mixed-content auto-upgrade, SameSite; CSP hook.
//! - **4** HTTP/3 via Alt-Svc — a transport-abstracted h3 lane (quinn) with
//!   h1/h2 fallback.
//! - **5** WebSocket (`ws://` / `wss://`).
//!
//! Native-focused; the h3 and WebSocket lanes are native-only (wasm-excluded).
//! Deferred: h3 for requests with bodies, the active/passive mixed-content split,
//! and public-suffix-accurate same-site.
//!
//! ## The authority split (2026-09-02)
//!
//! Genet owns the behaviour web content can observe: everything above the
//! [`transport`] seam. The host owns the wire: transport choice, trust,
//! credentials, caching and persistence, which reach this crate only as the
//! seams on [`FetchContext`]. The default transport (feature `hyper-transport`),
//! the HTTP/3 lane (`h3`) and WebSocket (`websocket`) are on by default and are
//! what a raw host or the WPT harness uses; a build with `default-features =
//! false` carries the Fetch semantics and no transport at all, which is the
//! proof that the two halves are separable.

mod altsvc;
mod cache;
mod context;
mod cookie_jar;
mod cors;
mod data_url;
mod decode;
mod fetch;
// HTTP/3 transport — native-only (QUIC over UDP); excluded from wasm builds.
#[cfg(all(feature = "h3", not(target_arch = "wasm32")))]
mod h3_client;
mod hsts;
mod referrer;
mod request;
mod response;
mod sri;
pub mod transport;
// WebSocket — native-only (tokio + tungstenite); a wasm build binds browser WS.
#[cfg(all(feature = "websocket", not(target_arch = "wasm32")))]
mod websocket;

pub use altsvc::{AltSvcStore, InMemoryAltSvc};
pub use cache::{HttpCache, InMemoryHttpCache, NoHttpCache, StoredResponse};
pub use context::{
    AllowAllCsp, CookieRecord, CookieStore, CspChecker, FetchContext, SameSiteContext,
};
// Re-exported so consumers can name a `CookieRecord`'s `same_site` without taking a
// direct `cookie` crate dep.
pub use cookie::SameSite;
pub use cookie_jar::InMemoryCookieJar;
pub use cors::{InMemoryPreflightCache, PreflightCache};
pub use fetch::fetch;
pub use hsts::{HstsStore, InMemoryHsts};
pub use request::{
    CacheMode, Credentials, Destination, Method, RedirectMode, ReferrerPolicy, Request, RequestMode,
};
pub use response::{BodyStream, Response, ResponseBody, ResponseType};
#[cfg(feature = "hyper-transport")]
pub use transport::hyper::{DefaultTransport, accept_invalid_certs};
pub use transport::{NoTransport, RawResponse, Transport, TransportFuture, WireRequest};
#[cfg(all(feature = "websocket", not(target_arch = "wasm32")))]
pub use websocket::{
    WebSocket, WsClose, WsError, WsIncoming, WsMessage, WsRequest, connect as connect_websocket,
    is_blocked_port, validate_ws_url,
};
