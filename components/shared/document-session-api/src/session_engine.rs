// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Session-engine traits and registry — the third engine kind
//! (2026-07-10 session-engines plan).
//!
//! Document engines (`Engine`) are request/response: bytes in,
//! serializable `EngineDocument` blocks out — the stored/authored
//! lane. Surface engines (`SurfaceEngine`) stream GPU textures from
//! external producers. Session engines sit between: **retained document
//! sessions** that lay content out once and then produce paint frames on
//! demand, with scroll, activation, and (for scripted lanes) a tick +
//! quiescence seam. The genet HTML lanes and the smolweb native lane are
//! session engines.
//!
//! The frame type is generic (`F`) so this crate keeps zero paint
//! dependencies: a netrender host instantiates `F = netrender::Scene`; a
//! different host picks its own frame type. Lane-specific construction seams
//! (resource fetchers, cookie jars, themes) are injected into the concrete
//! `SessionEngine` at registration time, not carried in the spawn request —
//! the request stays plain data.

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::a11y::{
    DocumentA11yActionRequest, DocumentA11yClickTarget, DocumentA11yNodeId, DocumentA11yProjection,
};
use crate::{A11yCapability, DocumentCapabilities, PageCaptureOutput, PageCaptureRequest};

// ── Errors ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionError {
    EngineNotFound(String),
    SpawnFailed(String),
    Unsupported(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EngineNotFound(id) => write!(f, "session engine not registered: {id}"),
            Self::SpawnFailed(reason) => write!(f, "session spawn failed: {reason}"),
            Self::Unsupported(reason) => write!(f, "unsupported: {reason}"),
        }
    }
}

impl std::error::Error for SessionError {}

// ── Spawn request ──────────────────────────────────────────────────────────

/// Plain-data request to open a document session. The body is already
/// fetched when the host has it (mirroring `EngineInput`); a session
/// engine whose lane fetches for itself (subresources, redirects) uses the
/// seams it was constructed with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSpawnRequest {
    pub address: String,
    /// Fetched body, when the host fetched it. `None` asks the engine to
    /// load via its own fetcher seam.
    pub body: Option<String>,
    pub content_type: Option<String>,
    /// Initial viewport, so the first `frame` call needs no resize dance.
    pub viewport: (u32, u32),
    /// Spawn hidden (a background tile): the session may defer work the
    /// visible path would do eagerly.
    pub hidden: bool,
}

impl SessionSpawnRequest {
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            body: None,
            content_type: None,
            viewport: (0, 0),
            hidden: false,
        }
    }

    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    pub fn with_viewport(mut self, width: u32, height: u32) -> Self {
        self.viewport = (width, height);
        self
    }
}

// ── Interaction vocabulary ─────────────────────────────────────────────────

/// A link the session exposes for the host's hit table: url + viewport-space
/// rect (`[x, y, w, h]`, the shape the lanes already emit).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionLink {
    pub url: String,
    pub rect: [f32; 4],
}

/// A structural report of a session's addressed content — title, an outline
/// of role + name, outgoing links, headings. The introspection CONTRACT for
/// [`DocumentSession::inspect`]: pure data, so it lives here rather than in a
/// render crate (genet-render's `content_report` builds one from a LayoutDom
/// and re-exports these types). Hosts that cannot downcast a session to its
/// concrete type (the type may be private to its engine crate) read this
/// instead — turnstone's Inspector pane is the first such consumer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContentReport {
    /// The `<title>` text, if any.
    pub title: Option<String>,
    /// The element outline (painted elements only; metadata tags are skipped),
    /// in document order.
    pub outline: Vec<OutlineEntry>,
    /// Outgoing `<a href>` targets, in document order.
    pub links: Vec<String>,
    /// Heading (`<h1>`..`<h6>`) text, in document order.
    pub headings: Vec<String>,
    /// Derivation evidence when this session presents transformed source bytes.
    pub lineage: Option<ContentLineage>,
}

/// Host-neutral derivation evidence for an inspected document session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentLineage {
    pub tool: String,
    pub version: String,
    pub selector: String,
    pub score: Option<i32>,
    pub block_count: usize,
}

/// The relationship between retained bytes and the representation observed by
/// a document engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentClipArtifactRole {
    /// Bytes returned by the source transport before document execution.
    SourceResponse,
    /// A serialized representation of the state actually observed by the
    /// lowering, such as a post-script DOM snapshot.
    ObservedRepresentation,
}

/// Exact source material offered with a semantic clip. The receiving domain
/// decides whether and where to retain it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentClipArtifact {
    pub role: DocumentClipArtifactRole,
    pub media_type: String,
    pub canonical_uri: String,
    pub bytes: Vec<u8>,
}

/// A host-neutral semantic fragment offered by a retained document session.
///
/// Sessions may expose the whole addressed document when they do not implement
/// range selection. The source address and optional selector keep that
/// distinction explicit for consumers such as Knot clipping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentClip {
    pub source_url: String,
    pub title: Option<String>,
    pub text: String,
    pub selector: Option<String>,
    pub links: Vec<String>,
    /// Source artifacts available from the engine's ordinary retained state.
    /// Engines must not refetch or reconstruct bytes solely for this field.
    pub artifacts: Vec<DocumentClipArtifact>,
}

/// A semantic text occurrence resolved to lane-local pointer coordinates.
///
/// Hosts use this for find-to-select and automation without learning a
/// document engine's DOM ids or text-layout representation. Driving these
/// points through [`DocumentSession::pointer_down`],
/// [`DocumentSession::pointer_move`], and [`DocumentSession::pointer_up`] still
/// exercises the ordinary input path; this is target resolution, not a second
/// selection mutation API.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SessionTextTarget {
    pub anchor: [f32; 2],
    pub focus: [f32; 2],
}

/// Direction for stepping a retained document find model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentFindDirection {
    Previous,
    Next,
}

/// A host-neutral document find query.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentFindQuery {
    pub text: String,
    pub match_case: bool,
}

impl DocumentFindQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            match_case: false,
        }
    }
}

/// How selecting a match is revealed by its owning engine.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum DocumentFindReveal {
    /// Retained document-space vertical offset. The session applies it when the
    /// match becomes current; the value remains observable to automation.
    ScrollY(f32),
    /// A hosted engine owns selection and reveal internally.
    EngineManaged,
}

/// One retained match, in document order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentFindMatch {
    /// Human-readable matched text or accessible element label.
    pub label: String,
    /// Coarse semantic role when the engine exposes one.
    pub role: Option<String>,
    pub reveal: DocumentFindReveal,
}

/// The retained result model shared by retained documents and hosted engines.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentFindState {
    pub query: DocumentFindQuery,
    /// Authoritative number of matches. Engines that can describe individual
    /// matches also populate `matches`; engine-managed pages may expose only
    /// this count and the current ordinal.
    pub count: usize,
    pub matches: Vec<DocumentFindMatch>,
    /// Zero-based index into `matches`.
    pub current: Option<usize>,
    /// Hosted engines may report progressive counts. Static sessions are
    /// complete in their first answer.
    pub complete: bool,
}

impl DocumentFindState {
    pub fn empty(query: DocumentFindQuery) -> Self {
        Self {
            query,
            count: 0,
            matches: Vec::new(),
            current: None,
            complete: true,
        }
    }

    /// Adapt an engine-managed count/current callback into the same retained
    /// model used by static sessions. The engine remains responsible for
    /// revealing the selected occurrence.
    pub fn engine_managed(
        query: DocumentFindQuery,
        count: usize,
        active_match: Option<usize>,
        complete: bool,
    ) -> Self {
        Self {
            query,
            count,
            matches: Vec::new(),
            current: active_match.filter(|index| *index < count),
            complete,
        }
    }

    pub fn current_match(&self) -> Option<&DocumentFindMatch> {
        self.current.and_then(|index| self.matches.get(index))
    }
}

/// The retained page-zoom model shared by retained documents and hosted
/// engines.
///
/// `requested` is the caller's own value, echoed back unchanged: the host
/// persists it per node and steps its own ladder. `applied` is what the engine
/// actually used after clamping and any quantization of its own, and `min` /
/// `max` name the engine's bounds so a host can grey out a step it cannot take.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentZoomState {
    /// The absolute factor the caller asked for (1.0 = 100 %).
    pub requested: f32,
    /// The effective factor this engine is presenting at.
    pub applied: f32,
    pub min: f32,
    pub max: f32,
}

impl DocumentZoomState {
    /// Clamp `requested` into `[min, max]` and report both halves. Engines that
    /// quantize further overwrite `applied` after calling this.
    pub fn clamped(requested: f32, min: f32, max: f32) -> Self {
        // A non-finite request is a caller bug rather than a zoom level, and
        // must never reach layout as a NaN viewport divisor.
        let applied = if requested.is_finite() {
            requested.clamp(min, max)
        } else {
            1.0_f32.clamp(min, max)
        };
        Self {
            requested,
            applied,
            min,
            max,
        }
    }
}

/// One element in the structural outline.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineEntry {
    /// Nesting depth among painted elements (the document root is depth 0).
    pub depth: usize,
    /// A coarse semantic role (`"link"`, `"heading"`, `"paragraph"`, …).
    pub role: &'static str,
    /// The element's accessible name — its direct text content, trimmed.
    pub name: String,
}

/// What a click did, unifying the lanes' divergent returns
/// (`ClickOutcome` / `bool` / `Option<String>`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionClick {
    /// The click resolved to a navigation the HOST performs (a link).
    Navigate(String),
    /// The click resolved to a mutation endpoint. The host must collect and
    /// confirm a body before submitting it.
    Submit(String),
    /// The session consumed the click itself (focus, a scripted handler).
    Handled,
    /// Nothing interactive at that point.
    Miss,
}

/// Modifier state carried with host-neutral session input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionModifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Pointer buttons understood by a retained document session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionPointerButton {
    Primary,
    Secondary,
    Auxiliary,
}

/// Press/release state shared by pointer and keyboard events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionButtonState {
    Pressed,
    Released,
}

/// Cursor requested by the retained content under the pointer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionCursor {
    #[default]
    Default,
    Pointer,
    Text,
}

/// Keyboard keys with document-level meaning. Text-bearing keys retain the
/// produced character string so this vocabulary does not assume ASCII.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionKey {
    Character(String),
    Enter,
    Tab,
    Backspace,
    Delete,
    Escape,
    Space,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Home,
    End,
    PageUp,
    PageDown,
    Unidentified,
}

/// Direction requested by focus traversal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionFocusDirection {
    Forward,
    Backward,
}

/// Platform composition lifecycle translated without exposing a windowing
/// crate to document engines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionIme {
    Enabled,
    Preedit {
        text: String,
        selection: Option<(usize, usize)>,
    },
    Commit(String),
    Disabled,
}

/// Input delivered by a host to one retained session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SessionInput {
    PointerMoved {
        x: f32,
        y: f32,
        modifiers: SessionModifiers,
    },
    PointerButton {
        x: f32,
        y: f32,
        button: SessionPointerButton,
        state: SessionButtonState,
        modifiers: SessionModifiers,
    },
    Key {
        key: SessionKey,
        state: SessionButtonState,
        modifiers: SessionModifiers,
        repeat: bool,
    },
    Text(String),
    Ime(SessionIme),
    Focus(bool),
    FocusMove(SessionFocusDirection),
    Cancel,
}

/// The transport method requested by a form submission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionFormMethod {
    #[default]
    Get,
    Post,
}

/// A form submission assembled by the engine from its retained form state.
/// The host still owns URL resolution, transport, policy, and navigation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionFormSubmission {
    pub action: String,
    pub method: SessionFormMethod,
    pub fields: Vec<(String, String)>,
}

/// Semantic effect of one session input. Navigation remains a host action;
/// sessions name the target or submission and never fetch a replacement page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionEffect {
    Ignored,
    Handled,
    Navigate(String),
    Submit(SessionFormSubmission),
    Cancelled,
}

impl SessionEffect {
    pub fn is_handled(&self) -> bool {
        !matches!(self, Self::Ignored)
    }
}

/// Complete result of one input dispatch, including presentation requests
/// that belong to the host adapter rather than the document engine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInputResult {
    pub effect: SessionEffect,
    pub cursor: Option<SessionCursor>,
    /// `Some(true)` begins primary-pointer capture and `Some(false)` releases it.
    pub capture: Option<bool>,
    /// Whether keyboard defaults such as arrow-key scrolling must yield to an
    /// editable control after this dispatch.
    pub editable: bool,
}

impl SessionInputResult {
    fn new(effect: SessionEffect, editable: bool) -> Self {
        Self {
            effect,
            cursor: None,
            capture: None,
            editable,
        }
    }
}

/// Navigation commands owned by the host around a retained session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionNavigationCommand {
    Address(String),
    Reload,
    Stop,
    Back,
    Forward,
}

/// Keyboard scroll intents, host-neutral. Adapters map these onto their
/// document lane's own scroll vocabulary; this contract does not drag a
/// layout dependency into Inker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionScrollKey {
    LineUp,
    LineDown,
    PageUp,
    PageDown,
    Home,
    End,
}

// ── Traits ─────────────────────────────────────────────────────────────────

/// Work a hosted document still owns after the current turn.
///
/// The delay is relative to the host clock at the point of inspection. A
/// session with no pending work returns [`Self::idle`]. The source flags keep
/// the completion receipt honest while the wake deadline lets a host sleep
/// until a timer is due instead of repainting continuously.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionPendingWork {
    /// Time until the earliest runtime timer is due, if the session exposes
    /// one. A zero delay means the host should wake immediately.
    pub timer: Option<Duration>,
    /// Whether the session still has microtasks to drain on its next pump.
    pub microtasks: bool,
    /// Whether a host-owned asynchronous source may complete the session.
    pub external: bool,
}

impl SessionPendingWork {
    pub const fn idle() -> Self {
        Self {
            timer: None,
            microtasks: false,
            external: false,
        }
    }

    pub const fn timer(delay: Duration) -> Self {
        Self {
            timer: Some(delay),
            microtasks: false,
            external: false,
        }
    }

    pub const fn is_idle(self) -> bool {
        self.timer.is_none() && !self.microtasks && !self.external
    }

    /// Whether script-visible completion has settled. A future timer may still
    /// repaint the page, but it does not keep a completion receipt open; host
    /// external work and microtasks do.
    pub const fn is_settled(self) -> bool {
        !self.microtasks && !self.external
    }

    /// Convert the session-relative delay into the host's next wake deadline.
    pub fn next_wake(self, now: Instant) -> Option<Instant> {
        if self.microtasks {
            Some(now)
        } else {
            self.timer.map(|delay| now + delay)
        }
    }
}

/// Spawns retained document sessions for the engine id it claims. Registered
/// once per host; holds its lane's construction seams (fetcher, cookie jar,
/// theme) so the spawn request stays plain data.
pub trait SessionEngine<F>: Send + Sync {
    /// Stable engine identifier. Must match the `engine_id` of the
    /// `EngineRouteDecision` that selected this engine.
    fn engine_id(&self) -> &str;

    fn spawn(
        &self,
        request: &SessionSpawnRequest,
    ) -> Result<Box<dyn DocumentSession<F>>, SessionError>;

    /// Sessions lay real content out through a real layout engine, so unlike
    /// surface engines they default to [`A11yCapability::Partial`]; a lane
    /// with a full semantic tree overrides to `Full`.
    fn a11y_capability(&self) -> A11yCapability {
        A11yCapability::Partial
    }
}

/// A live document: a retained layout session producing paint frames.
///
/// All methods take `&mut self`; the session is single-owner, driven from the
/// host's content thread — exactly how the lane types are driven today. Not
/// `Send` by default (scripted lanes hold JS engine state).
pub trait DocumentSession<F>: Any {
    /// Document-facing controls this retained session can presently serve.
    ///
    /// A session can expose find while leaving page zoom and raster capture to
    /// its host. Retained navigation is commonly partial because the host owns
    /// lineage, policy, and refetch.
    fn document_capabilities(&self) -> DocumentCapabilities {
        DocumentCapabilities::default()
    }

    /// Lay out (if needed) and paint at the given viewport. Resize is
    /// implicit: a size change re-lays-out, same as the lanes today.
    fn frame(&mut self, width: u32, height: u32) -> F;

    /// Scroll the viewport; `true` if the offset changed.
    fn scroll_by(&mut self, dx: f32, dy: f32) -> bool;

    /// Scroll the scrollable under `(x, y)` (nested scrollers); `true` if an
    /// offset changed. Defaults to viewport scroll for single-scroller lanes.
    fn scroll_at(&mut self, _x: f32, _y: f32, dx: f32, dy: f32) -> bool {
        self.scroll_by(dx, dy)
    }

    fn scroll_for_key(&mut self, key: SessionScrollKey) -> bool;

    /// Jump to an absolute vertical offset (anchor / fragment navigation).
    /// Defaulted no-op for lanes without absolute addressing; lanes that
    /// track their offset override.
    fn scroll_to(&mut self, _y: f32) {}

    fn click_at(&mut self, x: f32, y: f32) -> SessionClick;

    /// Begin a primary-pointer gesture in lane-local coordinates.
    ///
    /// The default preserves the original click-on-press contract for lanes
    /// that have not adopted gesture state. Text-selecting lanes override this
    /// to retain an anchor and defer link activation until pointer-up.
    fn pointer_down(&mut self, x: f32, y: f32) -> SessionClick {
        self.click_at(x, y)
    }

    /// Extend the captured primary-pointer gesture. `true` means visible
    /// session state changed and the host should redraw.
    fn pointer_move(&mut self, _x: f32, _y: f32) -> bool {
        false
    }

    /// Finish the captured primary-pointer gesture.
    ///
    /// Lanes retaining text selection return [`SessionClick::Handled`] for a
    /// non-collapsed selection, or the ordinary click result when the gesture
    /// collapsed.
    fn pointer_up(&mut self, _x: f32, _y: f32) -> SessionClick {
        SessionClick::Miss
    }

    /// Dispatch host-neutral input through the lane's retained interaction
    /// hooks. Existing click/pointer implementations remain the compatibility
    /// floor; richer lanes override the focused keyboard, text, IME, cursor,
    /// and form hooks below.
    fn input(&mut self, input: SessionInput) -> SessionInputResult {
        match input {
            SessionInput::PointerMoved { x, y, .. } => {
                let cursor = self.cursor_at(x, y);
                let effect = if self.pointer_move(x, y) {
                    SessionEffect::Handled
                } else {
                    SessionEffect::Ignored
                };
                let mut result = SessionInputResult::new(effect, self.editable_focus());
                result.cursor = Some(cursor);
                result
            },
            SessionInput::PointerButton {
                x,
                y,
                button: SessionPointerButton::Primary,
                state,
                ..
            } => {
                let cursor = self.cursor_at(x, y);
                let click = match state {
                    SessionButtonState::Pressed => self.pointer_down(x, y),
                    SessionButtonState::Released => self.pointer_up(x, y),
                };
                let effect = self.effect_for_click(click);
                let handled = effect.is_handled();
                let mut result = SessionInputResult::new(effect, self.editable_focus());
                result.cursor = Some(cursor);
                result.capture = Some(matches!(state, SessionButtonState::Pressed) && handled);
                result
            },
            SessionInput::PointerButton { x, y, .. } => {
                let mut result =
                    SessionInputResult::new(SessionEffect::Ignored, self.editable_focus());
                result.cursor = Some(self.cursor_at(x, y));
                result
            },
            SessionInput::Key {
                key,
                state,
                modifiers,
                repeat,
            } => SessionInputResult::new(
                self.key_input(key, state, modifiers, repeat),
                self.editable_focus(),
            ),
            SessionInput::Text(text) => {
                let effect = if self.text_input(&text) {
                    SessionEffect::Handled
                } else {
                    SessionEffect::Ignored
                };
                SessionInputResult::new(effect, self.editable_focus())
            },
            SessionInput::Ime(ime) => {
                let effect = if self.ime_input(ime) {
                    SessionEffect::Handled
                } else {
                    SessionEffect::Ignored
                };
                SessionInputResult::new(effect, self.editable_focus())
            },
            SessionInput::Focus(focused) => {
                self.focus_input(focused);
                SessionInputResult::new(SessionEffect::Handled, self.editable_focus())
            },
            SessionInput::FocusMove(direction) => {
                let effect = if self.focus_move(direction) {
                    SessionEffect::Handled
                } else {
                    SessionEffect::Ignored
                };
                SessionInputResult::new(effect, self.editable_focus())
            },
            SessionInput::Cancel => {
                let effect = if self.cancel_input() {
                    SessionEffect::Cancelled
                } else {
                    SessionEffect::Ignored
                };
                SessionInputResult::new(effect, self.editable_focus())
            },
        }
    }

    fn effect_for_click(&mut self, click: SessionClick) -> SessionEffect {
        match click {
            SessionClick::Navigate(target) => SessionEffect::Navigate(target),
            SessionClick::Submit(action) => SessionEffect::Submit(self.form_submission(&action)),
            SessionClick::Handled => SessionEffect::Handled,
            SessionClick::Miss => SessionEffect::Ignored,
        }
    }

    fn key_input(
        &mut self,
        _key: SessionKey,
        _state: SessionButtonState,
        _modifiers: SessionModifiers,
        _repeat: bool,
    ) -> SessionEffect {
        SessionEffect::Ignored
    }

    fn text_input(&mut self, _text: &str) -> bool {
        false
    }

    fn ime_input(&mut self, ime: SessionIme) -> bool {
        match ime {
            SessionIme::Commit(text) => self.text_input(&text),
            SessionIme::Enabled | SessionIme::Preedit { .. } | SessionIme::Disabled => false,
        }
    }

    fn focus_input(&mut self, _focused: bool) {}

    fn focus_move(&mut self, _direction: SessionFocusDirection) -> bool {
        false
    }

    fn cancel_input(&mut self) -> bool {
        false
    }

    fn editable_focus(&self) -> bool {
        false
    }

    fn cursor_at(&self, _x: f32, _y: f32) -> SessionCursor {
        SessionCursor::Default
    }

    fn form_submission(&mut self, action: &str) -> SessionFormSubmission {
        SessionFormSubmission {
            action: action.to_owned(),
            ..Default::default()
        }
    }

    /// Resolve the first laid-out occurrence of `text` to pointer endpoints.
    /// This remains read-only: callers must drive the normal pointer lifecycle
    /// to create selection state.
    fn text_target(&self, _text: &str) -> Option<SessionTextTarget> {
        None
    }

    /// Replace the retained find model for this session and reveal its first
    /// match. Engines own their document internals; the host receives only the
    /// portable model.
    fn document_find(
        &mut self,
        _query: &DocumentFindQuery,
    ) -> Result<DocumentFindState, SessionError> {
        Err(SessionError::Unsupported(
            "document find is not wired for this session".into(),
        ))
    }

    /// Step the current retained model, wrapping at either end.
    fn document_find_step(
        &mut self,
        _direction: DocumentFindDirection,
    ) -> Result<DocumentFindState, SessionError> {
        Err(SessionError::Unsupported(
            "document find is not wired for this session".into(),
        ))
    }

    fn clear_document_find(&mut self) -> Result<(), SessionError> {
        Ok(())
    }

    /// Present this document at an absolute page-zoom `factor` (1.0 = 100 %).
    ///
    /// This is user-agent document zoom, not CSS `zoom`: the CSS viewport
    /// shrinks as the factor grows, media queries re-evaluate against it, and
    /// the engine scales its rendered output back up. Host chrome is never
    /// scaled. The caller owns its own ladder and persistence — reset is
    /// `factor` 1.0 — while the engine owns only bounds and quantization, which
    /// it reports in the returned [`DocumentZoomState`].
    fn set_page_zoom(&mut self, _factor: f32) -> Result<DocumentZoomState, SessionError> {
        Err(SessionError::Unsupported(
            "page zoom is not wired for this session".into(),
        ))
    }

    /// Capture immediately from this retained session. Retained sessions have
    /// no event queue solely for capture symmetry; a future implementation can
    /// return the same output vocabulary directly.
    fn capture_page(
        &mut self,
        _request: PageCaptureRequest,
    ) -> Result<PageCaptureOutput, SessionError> {
        Err(SessionError::Unsupported(
            "page capture is not wired for this session".into(),
        ))
    }

    /// The link hit-table off the retained layout (no live-DOM query per
    /// click) — the mechanism all three lanes already share.
    fn links(&self) -> Vec<SessionLink>;

    /// Full laid-out content height at this viewport, for hosts that band
    /// scenes. Sessions that scroll internally return the viewport height.
    fn content_height(&mut self, _width: u32, height: u32) -> u32 {
        height
    }

    /// Absolute subresource URLs this retained session wants the host to
    /// fetch. The host owns transport, trust, credentials, and scheduling;
    /// sessions only name resources needed for their current presentation.
    fn subresources(&self) -> Vec<String> {
        Vec::new()
    }

    /// Deliver one host-fetched subresource. `true` means visible session
    /// state changed and the host should redraw.
    fn provide_subresource(&mut self, _url: &str, _bytes: &[u8]) -> bool {
        false
    }

    /// Drive timers / pending script work (scripted lanes). No-op default.
    fn pump(&mut self, _now_ms: f64) {}

    /// Report the work that should wake this session after the current turn.
    ///
    /// Static sessions remain idle. Scripted sessions report their earliest
    /// timer and set `external` while host-owned fetch or Worker work remains.
    fn pending_work(&mut self) -> SessionPendingWork {
        SessionPendingWork::idle()
    }

    /// Whether script-visible completion has settled: no pending microtasks
    /// or host-owned external work. Future timers do not block completion.
    /// This handshake does not track layout dirtiness or establish rendered
    /// quiescence. Static lanes are always settled by default.
    fn settled(&mut self) -> bool {
        self.pending_work().is_settled()
    }

    /// Visibility hint (a hidden tile may skip raster-adjacent work).
    fn set_hidden(&mut self, _hidden: bool) {}

    /// A structural [`ContentReport`] of the addressed content, for hosts that
    /// cannot downcast to the concrete session type (it may be private to its
    /// engine crate — turnstone's one registered lane is exactly that case, which
    /// is why this is a trait method and not an `as_any` detour). `None` for
    /// lanes without a structural read; the host reports the absence honestly
    /// rather than synthesizing one.
    fn inspect(&self) -> Option<ContentReport> {
        None
    }

    /// Capture the current semantic clip. `None` means this document lane
    /// cannot supply clip content; hosts must not synthesize authority or
    /// recover source bytes by downcasting.
    fn clip(&self) -> Option<DocumentClip> {
        None
    }

    /// Current renderer-neutral semantic snapshot of this retained document.
    ///
    /// The projection, when present, is the live truth about semantic
    /// completeness and limitations. It replaces host-side guesses based on
    /// concrete session type. Local IDs remain stable for the same semantic
    /// object across revisions and must be namespaced by a host before entering
    /// a global accessibility tree.
    fn accessibility_projection(&self) -> Option<DocumentA11yProjection> {
        None
    }

    /// Revalidate an advertised clickable node and return its current pointer
    /// target. Hosts use the returned point with their ordinary `click_at`
    /// path, keeping link routing and host navigation custody unchanged.
    ///
    /// A `None` result means the node is stale, no longer clickable, or its
    /// geometry is unavailable. The returned revision must match the current
    /// projection before the host acts on it.
    fn accessibility_click_target(
        &self,
        _target: DocumentA11yNodeId,
    ) -> Option<DocumentA11yClickTarget> {
        None
    }

    /// Revalidate and dispatch a non-pointer accessibility action.
    ///
    /// Engines return `true` only after accepting a request whose revision and
    /// target are current and whose action is advertised by that target.
    /// `Click` intentionally returns `false`: it travels through
    /// [`Self::accessibility_click_target`] and the ordinary pointer path.
    fn dispatch_accessibility_action(&mut self, _request: &DocumentA11yActionRequest) -> bool {
        false
    }

    /// Lane-specific extras (a scripted lane's DOM stats, a static lane's
    /// content report) stay on the concrete type; hosts that need them
    /// downcast through here rather than the trait growing every lane's
    /// diagnostics.
    fn as_any_ref(&self) -> &dyn Any;

    fn as_any(&mut self) -> &mut dyn Any;
}

// ── Registry ───────────────────────────────────────────────────────────────

/// Session engines keyed by engine id, one registry per host frame type.
#[derive(Default)]
pub struct SessionRegistry<F> {
    engines: HashMap<String, Box<dyn SessionEngine<F>>>,
}

impl<F> SessionRegistry<F> {
    pub fn new() -> Self {
        Self {
            engines: HashMap::new(),
        }
    }

    /// Register an engine under its own id. Last registration wins, matching
    /// `EngineRegistry` semantics.
    pub fn register(&mut self, engine: Box<dyn SessionEngine<F>>) {
        self.engines.insert(engine.engine_id().to_string(), engine);
    }

    pub fn contains(&self, engine_id: &str) -> bool {
        self.engines.contains_key(engine_id)
    }

    pub fn get(&self, engine_id: &str) -> Option<&dyn SessionEngine<F>> {
        self.engines.get(engine_id).map(|e| e.as_ref())
    }

    pub fn spawn(
        &self,
        engine_id: &str,
        request: &SessionSpawnRequest,
    ) -> Result<Box<dyn DocumentSession<F>>, SessionError> {
        self.engines
            .get(engine_id)
            .ok_or_else(|| SessionError::EngineNotFound(engine_id.to_string()))?
            .spawn(request)
    }

    pub fn engine_ids(&self) -> impl Iterator<Item = &str> {
        self.engines.keys().map(String::as_str)
    }
}

// ── Kind facade ────────────────────────────────────────────────────────────

/// Which registries hold an engine id. An id may be held by more than one
/// kind (a smolweb format can have both a block engine for cards and a
/// session engine for tiles); the HOST picks by surface context, so this is
/// reported as flags, not a single kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EngineKinds {
    pub document: bool,
    pub session: bool,
    pub surface: bool,
}

impl EngineKinds {
    pub fn any(&self) -> bool {
        self.document || self.session || self.surface
    }
}

/// Non-generic id-to-kind resolution: which registries hold each id is just
/// a map, so hosts resolve kinds without threading the frame type through
/// code that never touches frames. Built after registration from the
/// registries' id sets; host-handled ids (internal pages, ingest markers)
/// are the host's own vocabulary and deliberately absent.
#[derive(Clone, Debug, Default)]
pub struct EngineKindIndex {
    kinds: HashMap<String, EngineKinds>,
}

impl EngineKindIndex {
    pub fn build<'a>(
        document_ids: impl IntoIterator<Item = &'a str>,
        session_ids: impl IntoIterator<Item = &'a str>,
        surface_ids: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let mut kinds: HashMap<String, EngineKinds> = HashMap::new();
        for id in document_ids {
            kinds.entry(id.to_string()).or_default().document = true;
        }
        for id in session_ids {
            kinds.entry(id.to_string()).or_default().session = true;
        }
        for id in surface_ids {
            kinds.entry(id.to_string()).or_default().surface = true;
        }
        Self { kinds }
    }

    pub fn kinds_of(&self, engine_id: &str) -> EngineKinds {
        self.kinds.get(engine_id).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PageCaptureRequestId;

    /// A frame type that just records what was rendered.
    type TextFrame = String;

    struct EchoSession {
        address: String,
        scroll: f32,
        hidden: bool,
    }

    impl DocumentSession<TextFrame> for EchoSession {
        fn frame(&mut self, width: u32, height: u32) -> TextFrame {
            format!("{} @ {width}x{height} scroll={}", self.address, self.scroll)
        }
        fn scroll_by(&mut self, _dx: f32, dy: f32) -> bool {
            self.scroll += dy;
            dy != 0.0
        }
        fn scroll_for_key(&mut self, key: SessionScrollKey) -> bool {
            self.scroll_by(
                0.0,
                if key == SessionScrollKey::PageDown {
                    100.0
                } else {
                    0.0
                },
            )
        }
        fn click_at(&mut self, x: f32, _y: f32) -> SessionClick {
            if x < 10.0 {
                SessionClick::Navigate("gemini://example.test/".into())
            } else {
                SessionClick::Miss
            }
        }
        fn links(&self) -> Vec<SessionLink> {
            vec![SessionLink {
                url: "gemini://example.test/".into(),
                rect: [0.0, 0.0, 10.0, 10.0],
            }]
        }
        fn set_hidden(&mut self, hidden: bool) {
            self.hidden = hidden;
        }
        fn as_any_ref(&self) -> &dyn Any {
            self
        }
        fn as_any(&mut self) -> &mut dyn Any {
            self
        }
    }

    struct EchoSessionEngine;

    impl SessionEngine<TextFrame> for EchoSessionEngine {
        fn engine_id(&self) -> &str {
            "echo.session"
        }
        fn spawn(
            &self,
            request: &SessionSpawnRequest,
        ) -> Result<Box<dyn DocumentSession<TextFrame>>, SessionError> {
            if request.address.is_empty() {
                return Err(SessionError::SpawnFailed("empty address".into()));
            }
            Ok(Box::new(EchoSession {
                address: request.address.clone(),
                scroll: 0.0,
                hidden: request.hidden,
            }))
        }
    }

    #[test]
    fn registry_spawns_and_drives_a_session() {
        let mut registry = SessionRegistry::new();
        registry.register(Box::new(EchoSessionEngine));

        let request = SessionSpawnRequest::new("https://example.test").with_viewport(800, 600);
        let mut session = registry.spawn("echo.session", &request).expect("spawns");

        assert_eq!(
            session.frame(800, 600),
            "https://example.test @ 800x600 scroll=0"
        );
        assert!(session.scroll_by(0.0, 42.0));
        assert!(session.frame(800, 600).ends_with("scroll=42"));
        assert_eq!(
            session.click_at(5.0, 5.0),
            SessionClick::Navigate("gemini://example.test/".into())
        );
        assert_eq!(session.click_at(50.0, 5.0), SessionClick::Miss);
        assert_eq!(session.links().len(), 1);
        // Static-lane defaults: settled immediately, pump is a no-op.
        assert!(session.settled());
        session.pump(16.0);
    }

    #[test]
    fn document_session_capture_defaults_to_unsupported() {
        let mut session = EchoSession {
            address: "https://example.test".into(),
            scroll: 0.0,
            hidden: false,
        };
        assert!(matches!(
            session.capture_page(PageCaptureRequest::viewport(PageCaptureRequestId::new(9))),
            Err(SessionError::Unsupported(_))
        ));
    }

    #[test]
    fn document_session_accessibility_defaults_to_an_honest_absence() {
        let mut session = EchoSession {
            address: "https://example.test".into(),
            scroll: 0.0,
            hidden: false,
        };
        let target = DocumentA11yNodeId::new(7);
        assert_eq!(session.accessibility_projection(), None);
        assert_eq!(session.accessibility_click_target(target), None);
        assert!(
            !session.dispatch_accessibility_action(&DocumentA11yActionRequest {
                revision: 1,
                target,
                action: crate::DocumentA11yAction::Focus,
                data: None,
            })
        );
    }

    #[test]
    fn neutral_pointer_dispatch_preserves_navigation_cursor_and_capture() {
        let mut registry = SessionRegistry::new();
        registry.register(Box::new(EchoSessionEngine));
        let mut session = registry
            .spawn(
                "echo.session",
                &SessionSpawnRequest::new("https://example.test"),
            )
            .expect("spawns");

        let pressed = session.input(SessionInput::PointerButton {
            x: 5.0,
            y: 5.0,
            button: SessionPointerButton::Primary,
            state: SessionButtonState::Pressed,
            modifiers: SessionModifiers::default(),
        });
        assert_eq!(
            pressed.effect,
            SessionEffect::Navigate("gemini://example.test/".into())
        );
        assert_eq!(pressed.cursor, Some(SessionCursor::Default));
        assert_eq!(pressed.capture, Some(true));

        let released = session.input(SessionInput::PointerButton {
            x: 5.0,
            y: 5.0,
            button: SessionPointerButton::Primary,
            state: SessionButtonState::Released,
            modifiers: SessionModifiers::default(),
        });
        assert_eq!(released.effect, SessionEffect::Ignored);
        assert_eq!(released.capture, Some(false));
    }

    #[test]
    fn unknown_engine_is_a_named_error() {
        let registry: SessionRegistry<TextFrame> = SessionRegistry::new();
        let err = match registry.spawn("nope", &SessionSpawnRequest::new("x")) {
            Ok(_) => panic!("unknown engine must not spawn"),
            Err(err) => err,
        };
        assert_eq!(err, SessionError::EngineNotFound("nope".into()));
    }

    #[test]
    fn downcast_reaches_lane_extras() {
        let mut registry = SessionRegistry::new();
        registry.register(Box::new(EchoSessionEngine));
        let mut session = registry
            .spawn("echo.session", &SessionSpawnRequest::new("a"))
            .unwrap();
        session.set_hidden(true);
        assert!(
            session.as_any_ref().downcast_ref::<EchoSession>().is_some(),
            "immutable hosts can reach lane extras without taking session ownership"
        );
        let echo = session
            .as_any()
            .downcast_mut::<EchoSession>()
            .expect("concrete lane type reachable");
        assert!(echo.hidden);
    }

    #[test]
    fn kind_index_reports_flags_not_a_single_kind() {
        let mut sessions: SessionRegistry<TextFrame> = SessionRegistry::new();
        sessions.register(Box::new(EchoSessionEngine));

        // An id may be held by more than one kind (block engine for cards +
        // session engine for tiles); the index reports flags, host picks.
        let index = EngineKindIndex::build(
            ["nematic.gemtext"],
            sessions.engine_ids().chain(["nematic.gemtext"]),
            ["scrying.web"],
        );
        assert!(index.kinds_of("echo.session").session);
        let both = index.kinds_of("nematic.gemtext");
        assert!(both.document && both.session && !both.surface);
        assert!(index.kinds_of("scrying.web").surface);
        assert!(!index.kinds_of("absent").any());
    }

    #[test]
    fn page_zoom_is_a_named_absence_until_a_lane_wires_it() {
        let mut session = EchoSession {
            address: "gemini://example.test/".into(),
            scroll: 0.0,
            hidden: false,
        };
        assert!(matches!(
            session.set_page_zoom(1.25),
            Err(SessionError::Unsupported(_))
        ));
    }

    #[test]
    fn zoom_state_keeps_the_request_and_clamps_only_the_applied_value() {
        let state = DocumentZoomState::clamped(9.0, 0.25, 5.0);
        assert_eq!(state.requested, 9.0);
        assert_eq!(state.applied, 5.0);

        let state = DocumentZoomState::clamped(0.1, 0.25, 5.0);
        assert_eq!(state.requested, 0.1);
        assert_eq!(state.applied, 0.25);

        assert_eq!(DocumentZoomState::clamped(1.5, 0.25, 5.0).applied, 1.5);
        assert_eq!(
            DocumentZoomState::clamped(f32::NAN, 0.25, 5.0).applied,
            1.0,
            "a non-finite request must not reach layout as a viewport divisor"
        );
    }

    #[test]
    fn pending_work_exposes_a_sleepable_timer_deadline() {
        let now = std::time::Instant::now();
        let work = SessionPendingWork::timer(std::time::Duration::from_millis(25));
        let deadline = work.next_wake(now).expect("timer has a wake deadline");
        assert!(deadline >= now + std::time::Duration::from_millis(25));
        assert!(!work.is_idle());
        assert!(
            work.is_settled(),
            "a future timer is wakeable without making a live page perpetually unsettled"
        );
        assert!(
            !SessionPendingWork {
                external: true,
                ..SessionPendingWork::idle()
            }
            .is_settled()
        );
        assert!(SessionPendingWork::idle().next_wake(now).is_none());
        assert_eq!(
            SessionPendingWork {
                microtasks: true,
                ..SessionPendingWork::idle()
            }
            .next_wake(now),
            Some(now)
        );
    }
}
