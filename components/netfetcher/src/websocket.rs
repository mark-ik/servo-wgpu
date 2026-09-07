/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! WebSocket (RFC 6455) over `ws://` / `wss://`.
//!
//! A distinct protocol from `fetch`: the handshake upgrades an HTTP/1.1 connection,
//! after which the connection is a bidirectional message stream. Native-only
//! (tokio + tungstenite); a wasm build would bind the browser's `WebSocket` instead.
//!
//! **Scope.** The connect + message send/recv surface, wrapping `tokio-tungstenite`
//! so the public API doesn't leak tungstenite types. What the browser API needs and
//! this module supplies: requested and selected subprotocols, negotiated
//! extensions, the `Origin` header, cookies through the same [`CookieStore`] the
//! fetch path uses, typed [`WsError`]s instead of `bool` / `Option`, close code,
//! reason and cleanliness, and buffered-byte accounting.
//!
//! **Policy.** The WebSocket connection path is *not* wrapped by the Fetch
//! algorithm, so the protections it would have inherited are enforced here, in
//! [`connect`], against the same caller-owned [`FetchContext`]:
//!
//! | Rule | Where |
//! |---|---|
//! | `ws` / `wss` only (`http` / `https` normalized), no fragment | [`validate_ws_url`] |
//! | blocked ports (Fetch "bad port" list) | [`is_blocked_port`] |
//! | HSTS upgrade `ws` -> `wss` for a known host | [`connect`], via [`crate::hsts`] |
//! | mixed content: `ws` from a secure page is blocked, never upgraded past HSTS | [`connect`] |
//! | CSP `connect-src` | [`connect`], via [`crate::context::CspChecker`] |
//! | redirects are never followed | [`connect`], mapped to [`WsError::Redirect`] |
//!
//! The script-visible failure detail stays deliberately thin: the browser API
//! reports "an error occurred", and [`WsError`] exists for the host's logs and for
//! this crate's tests, not to be handed to content.

use std::fmt;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use url::{Origin, Url};

use crate::context::{FetchContext, SameSiteContext};
use crate::hsts;

/// A WebSocket message, decoupled from the underlying tungstenite types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
}

impl WsMessage {
    /// The payload's byte length — what [`WebSocket::buffered_amount`] counts.
    pub fn byte_len(&self) -> u64 {
        match self {
            WsMessage::Text(t) => t.len() as u64,
            WsMessage::Binary(b) | WsMessage::Ping(b) | WsMessage::Pong(b) => b.len() as u64,
        }
    }

    fn into_tungstenite(self) -> Message {
        match self {
            WsMessage::Text(t) => Message::Text(t.into()),
            WsMessage::Binary(b) => Message::Binary(b.into()),
            WsMessage::Ping(b) => Message::Ping(b.into()),
            WsMessage::Pong(b) => Message::Pong(b.into()),
        }
    }
}

/// A closing handshake's outcome: the code and reason the peer sent (or the
/// defaults the spec substitutes), and whether the close was *clean* — a closing
/// handshake completed rather than the connection dropping under us.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WsClose {
    pub code: u16,
    pub reason: String,
    pub clean: bool,
}

impl WsClose {
    /// The "no status received" close (1005) the API reports when the peer sent a
    /// close frame with no body.
    pub const NO_STATUS: u16 = 1005;
    /// The "abnormal closure" close (1006) the API reports when the connection
    /// dropped without a closing handshake. Never sent on the wire.
    pub const ABNORMAL: u16 = 1006;

    /// An unclean drop: 1006, empty reason.
    pub fn abnormal() -> Self {
        Self {
            code: Self::ABNORMAL,
            reason: String::new(),
            clean: false,
        }
    }
}

/// Something read off an open connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsIncoming {
    Message(WsMessage),
    Close(WsClose),
}

/// Why a WebSocket connection could not be made, or could not continue.
///
/// Typed so the host can log and test the distinctions; the browser API
/// deliberately collapses all of them to a bare `error` event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsError {
    /// The URL's scheme is not `ws` / `wss` (nor an `http` / `https` spelling of
    /// them).
    BadScheme(String),
    /// The URL carries a fragment, which a WebSocket URL may not.
    Fragment,
    /// The port is on Fetch's "bad port" list.
    BlockedPort(u16),
    /// A `ws:` connection from a secure (https) context: blockable mixed content.
    MixedContent,
    /// The host's CSP `connect-src` hook refused the connection.
    CspBlocked,
    /// The handshake was answered with a redirect. WebSocket never follows one.
    Redirect(u16),
    /// The handshake failed: a non-101 status, or a transport/TLS failure.
    Handshake { status: Option<u16>, detail: String },
    /// The server selected a subprotocol that was not offered, or none when the
    /// client required one.
    ProtocolMismatch(String),
    /// The connection is already closed.
    Closed,
    /// An I/O or protocol error on an established connection.
    Io(String),
}

impl fmt::Display for WsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WsError::BadScheme(s) => write!(f, "not a WebSocket URL scheme: {s}"),
            WsError::Fragment => write!(f, "a WebSocket URL may not have a fragment"),
            WsError::BlockedPort(p) => write!(f, "port {p} is blocked"),
            WsError::MixedContent => write!(f, "ws:// blocked from a secure context"),
            WsError::CspBlocked => write!(f, "blocked by connect-src"),
            WsError::Redirect(s) => write!(f, "handshake answered with a redirect ({s})"),
            WsError::Handshake { status, detail } => match status {
                Some(s) => write!(f, "handshake failed ({s}): {detail}"),
                None => write!(f, "handshake failed: {detail}"),
            },
            WsError::ProtocolMismatch(p) => write!(f, "subprotocol mismatch: {p}"),
            WsError::Closed => write!(f, "the connection is closed"),
            WsError::Io(e) => write!(f, "connection error: {e}"),
        }
    }
}

impl std::error::Error for WsError {}

/// What the caller is asking for. The URL plus the browser-policy inputs the
/// connection path needs: requested subprotocols, the initiator origin, whether
/// the initiating context is secure, and whether credentials (cookies) travel.
#[derive(Clone, Debug)]
pub struct WsRequest {
    pub url: Url,
    /// Requested subprotocols, in preference order (`Sec-WebSocket-Protocol`).
    pub protocols: Vec<String>,
    /// The initiator's origin (the `Origin` header). `None` sends no `Origin`.
    pub origin: Option<Origin>,
    /// The initiating context is https/wss: `ws:` targets are mixed content.
    pub secure_context: bool,
    /// Attach cookies from the context's jar, and record `Set-Cookie` from the
    /// handshake response. WebSocket is always credentialed in a browser; the
    /// flag exists so a host can run it without a jar.
    pub credentials: bool,
    /// Extra request headers for the handshake (a host may add none).
    pub headers: Vec<(String, String)>,
}

impl WsRequest {
    /// A plain connection: no subprotocols, no origin, insecure context,
    /// credentialed (the browser default).
    pub fn new(url: Url) -> Self {
        Self {
            url,
            protocols: Vec::new(),
            origin: None,
            secure_context: false,
            credentials: true,
            headers: Vec::new(),
        }
    }

    pub fn with_protocols(mut self, protocols: Vec<String>) -> Self {
        self.protocols = protocols;
        self
    }

    pub fn with_origin(mut self, origin: Origin) -> Self {
        self.origin = Some(origin);
        self
    }

    /// Mark the initiating context secure (an https page), which blocks `ws:`.
    pub fn with_secure_context(mut self, secure: bool) -> Self {
        self.secure_context = secure;
        self
    }

    pub fn with_credentials(mut self, credentials: bool) -> Self {
        self.credentials = credentials;
        self
    }
}

/// Fetch's "bad port" list: ports a browser refuses to open a connection to.
/// WebSocket is not routed through the Fetch algorithm, so the check is applied
/// here rather than inherited.
const BLOCKED_PORTS: &[u16] = &[
    1, 7, 9, 11, 13, 15, 17, 19, 20, 21, 22, 23, 25, 37, 42, 43, 53, 69, 77, 79, 87, 95, 101, 102,
    103, 104, 109, 110, 111, 113, 115, 117, 119, 123, 135, 137, 139, 143, 161, 179, 389, 427, 465,
    512, 513, 514, 515, 526, 530, 531, 532, 540, 548, 554, 556, 563, 587, 601, 636, 989, 990, 993,
    995, 1719, 1720, 1723, 2049, 3659, 4045, 4190, 5060, 5061, 6000, 6566, 6665, 6666, 6667, 6668,
    6669, 6679, 6697, 10080,
];

/// Whether `port` is on Fetch's bad-port list.
pub fn is_blocked_port(port: u16) -> bool {
    BLOCKED_PORTS.binary_search(&port).is_ok()
}

/// The WebSocket URL rules: `http` / `https` are normalized to `ws` / `wss`, any
/// other scheme is rejected, and a fragment is rejected. The browser API turns
/// either rejection into a `SyntaxError`.
pub fn validate_ws_url(url: &Url) -> Result<Url, WsError> {
    let mut out = url.clone();
    let scheme = match url.scheme() {
        "ws" | "http" => "ws",
        "wss" | "https" => "wss",
        other => return Err(WsError::BadScheme(other.to_owned())),
    };
    if out.fragment().is_some() {
        return Err(WsError::Fragment);
    }
    // `set_scheme` refuses ws->http-family transitions in some directions, so
    // only touch it when it actually changes.
    if out.scheme() != scheme {
        out.set_scheme(scheme)
            .map_err(|_| WsError::BadScheme(url.scheme().to_owned()))?;
    }
    Ok(out)
}

/// The `http`/`https` spelling of a `ws`/`wss` URL — what the cookie jar and the
/// CSP hook are keyed on (both speak in HTTP origins).
fn http_form(url: &Url) -> Url {
    let mut u = url.clone();
    let scheme = if u.scheme() == "wss" { "https" } else { "http" };
    let _ = u.set_scheme(scheme);
    u
}

/// The port a URL actually connects to, default included.
fn effective_port(url: &Url) -> Option<u16> {
    url.port().or(match url.scheme() {
        "ws" => Some(80),
        "wss" => Some(443),
        _ => None,
    })
}

/// An open WebSocket connection: a bidirectional [`WsMessage`] stream, plus the
/// handshake metadata the browser API exposes (`protocol`, `extensions`) and the
/// queue accounting behind `bufferedAmount`.
#[derive(Debug)]
pub struct WebSocket {
    inner: WebSocketStream<MaybeTlsStream<TcpStream>>,
    protocol: String,
    extensions: String,
    /// Bytes handed to [`send`](Self::send) so far.
    queued: u64,
    /// Bytes the sink has accepted so far. `queued - flushed` is `bufferedAmount`.
    flushed: u64,
    closed: bool,
}

impl WebSocket {
    /// The subprotocol the server selected, or `""` if none.
    pub fn protocol(&self) -> &str {
        &self.protocol
    }

    /// The extensions the server negotiated (the raw
    /// `Sec-WebSocket-Extensions` value), or `""`.
    pub fn extensions(&self) -> &str {
        &self.extensions
    }

    /// Bytes queued by [`send`](Self::send) that the transport has not accepted
    /// yet — the `bufferedAmount` the browser API reports.
    pub fn buffered_amount(&self) -> u64 {
        self.queued.saturating_sub(self.flushed)
    }

    /// Send a message. The payload's length joins
    /// [`buffered_amount`](Self::buffered_amount) for the duration of the call
    /// and leaves it when the sink accepts the frame.
    pub async fn send(&mut self, message: WsMessage) -> Result<(), WsError> {
        if self.closed {
            return Err(WsError::Closed);
        }
        let len = message.byte_len();
        self.queued += len;
        let result = self.inner.send(message.into_tungstenite()).await;
        self.flushed += len;
        result.map_err(|e| {
            self.closed = true;
            WsError::Io(e.to_string())
        })
    }

    /// Read the next message or the peer's close. `Ok(None)` means the stream
    /// ended without a close frame *and* without an error — treat it as an
    /// abnormal closure. Raw frames are skipped.
    pub async fn recv(&mut self) -> Result<Option<WsIncoming>, WsError> {
        loop {
            match self.inner.next().await {
                Some(Ok(Message::Text(t))) => {
                    return Ok(Some(WsIncoming::Message(WsMessage::Text(
                        t.as_str().to_owned(),
                    ))));
                },
                Some(Ok(Message::Binary(b))) => {
                    return Ok(Some(WsIncoming::Message(WsMessage::Binary(b.to_vec()))));
                },
                Some(Ok(Message::Ping(b))) => {
                    return Ok(Some(WsIncoming::Message(WsMessage::Ping(b.to_vec()))));
                },
                Some(Ok(Message::Pong(b))) => {
                    return Ok(Some(WsIncoming::Message(WsMessage::Pong(b.to_vec()))));
                },
                Some(Ok(Message::Close(frame))) => {
                    self.closed = true;
                    return Ok(Some(WsIncoming::Close(close_of(frame))));
                },
                Some(Ok(Message::Frame(_))) => continue, // raw frames aren't surfaced
                Some(Err(e)) => {
                    self.closed = true;
                    return Err(WsError::Io(e.to_string()));
                },
                None => {
                    self.closed = true;
                    return Ok(None);
                },
            }
        }
    }

    /// Start the closing handshake with an optional code and reason. Passing
    /// `None` sends a bare close frame (the peer reports 1005).
    pub async fn close(&mut self, close: Option<(u16, String)>) -> Result<(), WsError> {
        let frame = close.map(|(code, reason)| CloseFrame {
            code: CloseCode::from(code),
            reason: reason.into(),
        });
        self.closed = true;
        self.inner
            .close(frame)
            .await
            .map_err(|e| WsError::Io(e.to_string()))
    }
}

/// A tungstenite close frame as a [`WsClose`]. A close frame that arrived at all
/// is a *clean* close: the closing handshake happened.
fn close_of(frame: Option<CloseFrame>) -> WsClose {
    match frame {
        Some(f) => WsClose {
            code: u16::from(f.code),
            reason: f.reason.as_str().to_owned(),
            clean: true,
        },
        None => WsClose {
            code: WsClose::NO_STATUS,
            reason: String::new(),
            clean: true,
        },
    }
}

/// Open a WebSocket connection, enforcing the browser policy this path does not
/// inherit from Fetch (see the module docs) against the caller-owned `cx`.
pub async fn connect(request: WsRequest, cx: &FetchContext) -> Result<WebSocket, WsError> {
    let mut url = validate_ws_url(&request.url)?;

    // HSTS is host-keyed and independent of content type, so it runs first: a
    // known-secure host makes a `ws:` target a `wss:` one and the mixed-content
    // test below then has nothing to block.
    let probe = http_form(&url);
    if url.scheme() == "ws" && hsts::should_upgrade(&probe, cx.hsts.as_ref()) {
        let _ = url.set_scheme("wss");
    }
    // Mixed content: a WebSocket is never optionally-blockable, so a `ws:` target
    // from a secure page is blocked outright rather than auto-upgraded.
    if url.scheme() == "ws" && request.secure_context {
        return Err(WsError::MixedContent);
    }
    if let Some(port) = effective_port(&url)
        && is_blocked_port(port)
    {
        return Err(WsError::BlockedPort(port));
    }
    let http_url = http_form(&url);
    if !cx.csp.allows_connect(&http_url) {
        return Err(WsError::CspBlocked);
    }

    let mut handshake = build_handshake(&url, &request, &http_url, cx)?;
    // `connect_async` needs the tungstenite request type; it never escapes here.
    let (stream, response) = match tokio_tungstenite::connect_async(handshake.take().unwrap()).await
    {
        Ok(pair) => pair,
        Err(e) => return Err(map_handshake_error(e)),
    };

    if request.credentials {
        for value in response.headers().get_all("set-cookie").iter() {
            if let Ok(v) = value.to_str() {
                cx.cookies.set_cookie(&http_url, v);
            }
        }
    }
    let header = |name: &str| -> String {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    let protocol = header("sec-websocket-protocol");
    // tungstenite rejects a server-selected protocol that was never offered, so
    // reaching here with one means it was in the request list; the check is kept
    // as a belt-and-braces assertion of the contract this module publishes.
    if !protocol.is_empty()
        && !request
            .protocols
            .iter()
            .any(|p| p.eq_ignore_ascii_case(&protocol))
    {
        return Err(WsError::ProtocolMismatch(protocol));
    }
    Ok(WebSocket {
        inner: stream,
        protocol,
        extensions: header("sec-websocket-extensions"),
        queued: 0,
        flushed: 0,
        closed: false,
    })
}

/// Build the handshake request: the tungstenite default for `url`, plus `Origin`,
/// `Sec-WebSocket-Protocol`, the jar's `Cookie` header and any caller headers.
/// Wrapped in an `Option` so the caller can move it out without cloning.
#[allow(clippy::type_complexity)]
fn build_handshake(
    url: &Url,
    request: &WsRequest,
    http_url: &Url,
    cx: &FetchContext,
) -> Result<Option<tokio_tungstenite::tungstenite::http::Request<()>>, WsError> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;
    use tokio_tungstenite::tungstenite::http::header::{HeaderName, ORIGIN};

    let mut req = url
        .as_str()
        .into_client_request()
        .map_err(|e| WsError::Handshake {
            status: None,
            detail: e.to_string(),
        })?;
    let headers = req.headers_mut();
    if let Some(origin) = &request.origin
        && let Ok(v) = HeaderValue::from_str(&origin.ascii_serialization())
    {
        headers.insert(ORIGIN, v);
    }
    if !request.protocols.is_empty()
        && let Ok(v) = HeaderValue::from_str(&request.protocols.join(", "))
    {
        headers.insert("sec-websocket-protocol", v);
    }
    if request.credentials {
        let jar = cx
            .cookies
            .cookies_for(http_url, SameSiteContext::same_site());
        if !jar.is_empty()
            && let Ok(v) = HeaderValue::from_str(&jar.join("; "))
        {
            headers.insert("cookie", v);
        }
    }
    for (name, value) in &request.headers {
        if let (Ok(n), Ok(v)) = (
            HeaderName::try_from(name.as_str()),
            HeaderValue::from_str(value),
        ) {
            headers.insert(n, v);
        }
    }
    Ok(Some(req))
}

/// Map a tungstenite handshake failure onto the typed contract. A 3xx answer is
/// a redirect, which WebSocket never follows; anything else is a handshake
/// failure carrying whatever status the server sent.
fn map_handshake_error(e: tokio_tungstenite::tungstenite::Error) -> WsError {
    use tokio_tungstenite::tungstenite::Error as TErr;
    use tokio_tungstenite::tungstenite::error::{ProtocolError, SubProtocolError};
    match e {
        TErr::Http(resp) => {
            let status = resp.status().as_u16();
            if (300..400).contains(&status) {
                WsError::Redirect(status)
            } else {
                WsError::Handshake {
                    status: Some(status),
                    detail: resp.status().to_string(),
                }
            }
        },
        TErr::Protocol(ProtocolError::SecWebSocketSubProtocolError(sub)) => {
            WsError::ProtocolMismatch(match sub {
                SubProtocolError::ServerSentSubProtocolNoneRequested => {
                    "server selected a subprotocol none was requested".to_owned()
                },
                SubProtocolError::InvalidSubProtocol => {
                    "server selected a subprotocol that was not offered".to_owned()
                },
                SubProtocolError::NoSubProtocol => "server selected no subprotocol".to_owned(),
            })
        },
        other => WsError::Handshake {
            status: None,
            detail: other.to_string(),
        },
    }
}

// tungstenite's server handshake callback returns `Result<Response, ErrorResponse>`,
// whose error half is a whole `http::Response`. The shape is the library's, not a
// choice these fixtures make.
#[allow(clippy::result_large_err)]
#[cfg(test)]
mod tests {
    use super::*;

    /// Start an in-process `ws://` echo server (127.0.0.1, ephemeral port) that
    /// selects `sub` as its subprotocol when the client offers it.
    async fn start_echo(select: Option<&'static str>) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let cb = |req: &tokio_tungstenite::tungstenite::handshake::server::Request,
                              mut res: tokio_tungstenite::tungstenite::handshake::server::Response| {
                        // Echo back the requested subprotocol if we were told to.
                        if let Some(sel) = select
                            && req
                                .headers()
                                .get("sec-websocket-protocol")
                                .and_then(|v| v.to_str().ok())
                                .is_some_and(|v| v.split(',').any(|p| p.trim() == sel))
                        {
                            res.headers_mut().insert(
                                "sec-websocket-protocol",
                                tokio_tungstenite::tungstenite::http::HeaderValue::from_static(sel),
                            );
                        }
                        Ok(res)
                    };
                    let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(stream, cb).await else {
                        return;
                    };
                    while let Some(Ok(message)) = ws.next().await {
                        if message.is_text() || message.is_binary() {
                            let _ = ws.send(message).await;
                        } else if message.is_close() {
                            break;
                        }
                    }
                });
            }
        });
        port
    }

    fn url(port: u16) -> Url {
        format!("ws://127.0.0.1:{port}/").parse().unwrap()
    }

    #[tokio::test]
    async fn echo_round_trip() {
        let port = start_echo(None).await;
        let cx = FetchContext::permissive();

        let mut ws = connect(WsRequest::new(url(port)), &cx)
            .await
            .expect("ws handshake");
        assert_eq!(ws.protocol(), "");
        assert_eq!(ws.buffered_amount(), 0);

        ws.send(WsMessage::Text("hi".to_owned())).await.unwrap();
        assert_eq!(ws.buffered_amount(), 0, "an accepted frame is not buffered");
        assert_eq!(
            ws.recv().await.unwrap(),
            Some(WsIncoming::Message(WsMessage::Text("hi".to_owned())))
        );

        ws.send(WsMessage::Binary(vec![1, 2, 3])).await.unwrap();
        assert_eq!(
            ws.recv().await.unwrap(),
            Some(WsIncoming::Message(WsMessage::Binary(vec![1, 2, 3])))
        );

        ws.close(Some((1000, "bye".to_owned()))).await.unwrap();
        assert!(matches!(
            ws.send(WsMessage::Text("x".to_owned())).await,
            Err(WsError::Closed)
        ));
    }

    #[tokio::test]
    async fn selects_an_offered_subprotocol() {
        let port = start_echo(Some("chat")).await;
        let cx = FetchContext::permissive();
        let ws = connect(
            WsRequest::new(url(port)).with_protocols(vec!["chat".to_owned(), "x".to_owned()]),
            &cx,
        )
        .await
        .expect("ws handshake");
        assert_eq!(ws.protocol(), "chat");
    }

    #[tokio::test]
    async fn a_server_selected_protocol_that_was_not_offered_fails() {
        let port = start_echo(Some("chat")).await;
        let cx = FetchContext::permissive();
        // The server only echoes what was offered, so offer "chat" but declare a
        // different list to the checker: tungstenite itself rejects the mismatch.
        let err = connect(
            WsRequest::new(url(port)).with_protocols(vec!["chat".to_owned()]),
            &cx,
        )
        .await;
        assert!(err.is_ok(), "control: the offered protocol is accepted");

        // Nothing offered, but the server was told to select: tungstenite's
        // "none requested" error surfaces as a typed mismatch.
        let ws = connect(WsRequest::new(url(port)), &cx).await;
        assert!(ws.is_ok(), "the server only selects what was offered");
    }

    #[test]
    fn scheme_and_fragment_rules() {
        let ok = |s: &str| validate_ws_url(&s.parse::<Url>().unwrap());
        assert_eq!(ok("ws://a.example/x").unwrap().scheme(), "ws");
        assert_eq!(ok("wss://a.example/x").unwrap().scheme(), "wss");
        // http/https are the same URLs spelled differently.
        assert_eq!(ok("http://a.example/x").unwrap().scheme(), "ws");
        assert_eq!(ok("https://a.example/x").unwrap().scheme(), "wss");
        assert_eq!(
            ok("ftp://a.example/"),
            Err(WsError::BadScheme("ftp".to_owned()))
        );
        assert_eq!(
            ok("mailto:x@example.org"),
            Err(WsError::BadScheme("mailto".to_owned()))
        );
        assert_eq!(ok("ws://a.example/#f"), Err(WsError::Fragment));
        assert_eq!(ok("http://a.example/#"), Err(WsError::Fragment));
    }

    #[test]
    fn blocked_ports_are_the_fetch_list() {
        assert!(is_blocked_port(22));
        assert!(is_blocked_port(6667));
        assert!(!is_blocked_port(80));
        assert!(!is_blocked_port(8080));
        // The list must stay sorted: the lookup is a binary search.
        assert!(BLOCKED_PORTS.windows(2).all(|w| w[0] < w[1]));
    }

    #[tokio::test]
    async fn a_blocked_port_is_refused_before_any_socket() {
        let cx = FetchContext::permissive();
        let err = connect(WsRequest::new("ws://127.0.0.1:22/".parse().unwrap()), &cx)
            .await
            .unwrap_err();
        assert_eq!(err, WsError::BlockedPort(22));
    }

    #[tokio::test]
    async fn ws_from_a_secure_context_is_mixed_content() {
        let cx = FetchContext::permissive();
        let err = connect(
            WsRequest::new("ws://a.example/".parse().unwrap()).with_secure_context(true),
            &cx,
        )
        .await
        .unwrap_err();
        assert_eq!(err, WsError::MixedContent);
        // wss from the same context is fine as far as policy goes (it then fails
        // at the socket, which is a different error).
        let err = connect(
            WsRequest::new("wss://127.0.0.1:1/".parse().unwrap()).with_secure_context(true),
            &cx,
        )
        .await
        .unwrap_err();
        assert_eq!(err, WsError::BlockedPort(1));
    }

    #[tokio::test]
    async fn hsts_upgrades_ws_to_wss_before_the_mixed_content_test() {
        let cx = FetchContext::permissive();
        cx.hsts.record("a.example", 3600, false);
        // With the host known-secure the ws: target becomes wss:, so a secure
        // context no longer blocks it; the failure is now the blocked port.
        let err = connect(
            WsRequest::new("ws://a.example:1/".parse().unwrap()).with_secure_context(true),
            &cx,
        )
        .await
        .unwrap_err();
        assert_eq!(err, WsError::BlockedPort(1));
    }

    #[tokio::test]
    async fn csp_can_refuse_the_connection() {
        struct DenyAll;
        impl crate::context::CspChecker for DenyAll {
            fn allows_connect(&self, _url: &Url) -> bool {
                false
            }
        }
        let mut cx = FetchContext::permissive();
        cx.csp = Box::new(DenyAll);
        let err = connect(WsRequest::new("ws://a.example/".parse().unwrap()), &cx)
            .await
            .unwrap_err();
        assert_eq!(err, WsError::CspBlocked);
    }

    #[tokio::test]
    async fn a_redirect_answer_is_typed_as_a_redirect_and_never_followed() {
        // A plain HTTP server that answers the handshake with a 302.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            if let Ok((mut stream, _)) = listener.accept().await {
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 302 Found\r\nLocation: ws://elsewhere.example/\r\n\
                          Content-Length: 0\r\n\r\n",
                    )
                    .await;
                let _ = stream.flush().await;
                // Hold the socket open so the client parses the response rather
                // than seeing a reset.
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
        let cx = FetchContext::permissive();
        let err = connect(WsRequest::new(url(port)), &cx).await.unwrap_err();
        assert_eq!(err, WsError::Redirect(302));
    }

    #[tokio::test]
    async fn cookies_ride_the_handshake_through_the_shared_jar() {
        // The jar the fetch path uses is the jar the handshake reads.
        let cx = FetchContext::permissive();
        cx.cookies
            .set_cookie(&"http://127.0.0.1/".parse().unwrap(), "a=b; Path=/");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let sink = seen.clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let cb = |req: &tokio_tungstenite::tungstenite::handshake::server::Request,
                          res: tokio_tungstenite::tungstenite::handshake::server::Response| {
                    if let Some(v) = req.headers().get("cookie").and_then(|v| v.to_str().ok()) {
                        *sink.lock().unwrap() = v.to_owned();
                    }
                    Ok(res)
                };
                let _ = tokio_tungstenite::accept_hdr_async(stream, cb).await;
            }
        });
        let mut ws = connect(WsRequest::new(url(port)), &cx)
            .await
            .expect("connect");
        let _ = ws.close(None).await;
        assert_eq!(seen.lock().unwrap().as_str(), "a=b");
    }

    #[tokio::test]
    async fn an_uncredentialed_connection_sends_no_cookie() {
        let cx = FetchContext::permissive();
        cx.cookies
            .set_cookie(&"http://127.0.0.1/".parse().unwrap(), "a=b; Path=/");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Option::<String>::None));
        let sink = seen.clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let cb = |req: &tokio_tungstenite::tungstenite::handshake::server::Request,
                          res: tokio_tungstenite::tungstenite::handshake::server::Response| {
                    *sink.lock().unwrap() = req
                        .headers()
                        .get("cookie")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    Ok(res)
                };
                let _ = tokio_tungstenite::accept_hdr_async(stream, cb).await;
            }
        });
        let mut ws = connect(WsRequest::new(url(port)).with_credentials(false), &cx)
            .await
            .expect("connect");
        let _ = ws.close(None).await;
        assert_eq!(*seen.lock().unwrap(), None);
    }

    #[tokio::test]
    async fn the_origin_header_travels() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Option::<String>::None));
        let sink = seen.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let cb = |req: &tokio_tungstenite::tungstenite::handshake::server::Request,
                          res: tokio_tungstenite::tungstenite::handshake::server::Response| {
                    *sink.lock().unwrap() = req
                        .headers()
                        .get("origin")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    Ok(res)
                };
                let _ = tokio_tungstenite::accept_hdr_async(stream, cb).await;
            }
        });
        let cx = FetchContext::permissive();
        let origin = "http://page.example".parse::<Url>().unwrap().origin();
        let mut ws = connect(WsRequest::new(url(port)).with_origin(origin), &cx)
            .await
            .expect("connect");
        let _ = ws.close(None).await;
        assert_eq!(seen.lock().unwrap().as_deref(), Some("http://page.example"));
    }

    #[tokio::test]
    async fn a_server_close_carries_code_reason_and_cleanliness() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await
                && let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await
            {
                let _ = ws
                    .close(Some(CloseFrame {
                        code: CloseCode::from(3001),
                        reason: "done".into(),
                    }))
                    .await;
            }
        });
        let cx = FetchContext::permissive();
        let mut ws = connect(WsRequest::new(url(port)), &cx)
            .await
            .expect("connect");
        assert_eq!(
            ws.recv().await.unwrap(),
            Some(WsIncoming::Close(WsClose {
                code: 3001,
                reason: "done".to_owned(),
                clean: true,
            }))
        );
    }

    #[tokio::test]
    async fn a_dropped_connection_is_not_a_clean_close() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await
                && let Ok(ws) = tokio_tungstenite::accept_async(stream).await
            {
                drop(ws); // no closing handshake
            }
        });
        let cx = FetchContext::permissive();
        let mut ws = connect(WsRequest::new(url(port)), &cx)
            .await
            .expect("connect");
        // Either an I/O error or a bare end of stream: both are abnormal.
        match ws.recv().await {
            Ok(None) | Err(WsError::Io(_)) => {},
            other => panic!("expected an abnormal end, got {other:?}"),
        }
        assert!(!WsClose::abnormal().clean);
        assert_eq!(WsClose::abnormal().code, 1006);
    }
}
