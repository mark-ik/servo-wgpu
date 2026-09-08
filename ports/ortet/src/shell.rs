// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The window and its event loop.
//!
//! One `winit` window, one `SurfaceHost`, one `DocumentSession<Scene>`. The
//! shell owns exactly three things a session cannot: the window, the fetch
//! seam, and navigation. Following a link is spawning a new session for the
//! resolved address — there is no history, no registry, and no controller
//! between the winit event and the session's own input vocabulary.

use std::sync::Arc;
use std::time::{Duration, Instant};

use document_session_api::session_engine::{
    DocumentSession, SessionButtonState, SessionClick, SessionCursor, SessionEffect, SessionEngine,
    SessionIme, SessionInput, SessionKey, SessionModifiers, SessionPointerButton, SessionScrollKey,
    SessionSpawnRequest,
};
use genet_documents::LiverySessionEngine;
#[cfg(feature = "scripted")]
use genet_documents::ScriptedSessionEngine;
use genet_host_api::navigation::resolve_href;
use genet_winit_host::{AccessKitBridge, BridgeStatus, SurfaceHost, wheel_delta_from_winit};
use netrender::{ColorLoad, ExternalTexturePlacement, NetrenderOptions, Scene};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::a11y::{Accessibility, RoutedAction};
use crate::args::{Action, Config};
use crate::fetch::OrtetFetcher;
use crate::receipt;

/// What a completed run has to say for itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    /// The id claimed by the concrete engine that hosted this run.
    pub engine_id: String,
    /// The command-line backend choice. Boa and Nova intentionally have
    /// different backends even though Boa uses the generic scripted engine id.
    pub backend: String,
    pub metadata: BuildMetadata,
    pub address: String,
    pub frames: u32,
    pub size: (u32, u32),
    pub artifact: Option<std::path::PathBuf>,
    pub digest: Option<u64>,
}

/// Build facts that a host receipt can report without guessing a source
/// revision. `GENET_SOURCE_REVISION` is supplied by a release build when it
/// has one; local builds honestly report it as unknown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildMetadata {
    pub target: String,
    pub features: String,
    pub source_revision: Option<String>,
}

const RECEIPT_SETTLE_TIMEOUT: Duration = Duration::from_secs(5);

pub fn build_metadata() -> BuildMetadata {
    BuildMetadata {
        target: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        features: enabled_features().to_owned(),
        source_revision: option_env!("GENET_SOURCE_REVISION").map(str::to_owned),
    }
}

#[cfg(feature = "scripted-nova")]
const fn enabled_features() -> &'static str {
    "scripted,scripted-nova"
}

#[cfg(all(feature = "scripted", not(feature = "scripted-nova")))]
const fn enabled_features() -> &'static str {
    "scripted"
}

#[cfg(not(feature = "scripted"))]
const fn enabled_features() -> &'static str {
    "none"
}

/// Open the window and run the document until the frame budget or the user
/// closes it.
pub fn run(config: Config, fetcher: OrtetFetcher) -> Result<Outcome, String> {
    let engine = make_engine(config.engine, fetcher)?;
    // Spawn before the window exists so a bad address fails without flashing a
    // window at anyone.
    let session = spawn(engine.as_ref(), &config.address, config.size)?;
    let event_loop =
        EventLoop::new().map_err(|error| format!("could not create the event loop: {error}"))?;
    let mut app = Ortet::new(config, engine, session);
    event_loop
        .run_app(&mut app)
        .map_err(|error| format!("the ortet event loop failed: {error}"))?;
    match app.failure {
        Some(failure) => Err(failure),
        None => Ok(app.outcome()),
    }
}

fn spawn(
    engine: &dyn SessionEngine<Scene>,
    address: &str,
    (width, height): (u32, u32),
) -> Result<Box<dyn DocumentSession<Scene>>, String> {
    let request = SessionSpawnRequest::new(address).with_viewport(width, height);
    engine
        .spawn(&request)
        .map_err(|error| format!("could not open {address}: {error}"))
}

/// Build the selected engine behind the common host contract. This is the O5a
/// seam: engine choice is explicit host policy, while session construction,
/// input, pumping, and frames remain the one `SessionEngine<Scene>` path.
fn make_engine(
    choice: crate::args::EngineChoice,
    fetcher: OrtetFetcher,
) -> Result<Box<dyn SessionEngine<Scene>>, String> {
    match choice {
        crate::args::EngineChoice::Livery => Ok(Box::new(LiverySessionEngine::new(fetcher))),
        crate::args::EngineChoice::Boa => {
            #[cfg(feature = "scripted")]
            {
                Ok(Box::new(ScriptedSessionEngine::<
                    script_engine_boa::BoaEngine,
                    _,
                >::new(
                    document_session_api::engine_ids::ENGINE_GENET_SCRIPTED,
                    fetcher,
                )))
            }
            #[cfg(not(feature = "scripted"))]
            {
                let _ = fetcher;
                Err(
                    "the boa engine is unavailable: rebuild ortet with --features scripted"
                        .to_owned(),
                )
            }
        },
        crate::args::EngineChoice::Nova => {
            #[cfg(all(feature = "scripted-nova", target_pointer_width = "64"))]
            {
                Ok(Box::new(ScriptedSessionEngine::<
                    script_engine_nova::NovaEngine,
                    _,
                >::new(
                    document_session_api::engine_ids::ENGINE_GENET_SCRIPTED_NOVA,
                    fetcher,
                )))
            }
            #[cfg(all(feature = "scripted-nova", not(target_pointer_width = "64")))]
            {
                let _ = fetcher;
                Err("the nova engine is unavailable on 32-bit targets".to_owned())
            }
            #[cfg(not(feature = "scripted-nova"))]
            {
                let _ = fetcher;
                Err(
                    "the nova engine is unavailable: rebuild ortet with --features scripted-nova"
                        .to_owned(),
                )
            }
        },
    }
}

struct Ortet {
    config: Config,
    engine: Box<dyn SessionEngine<Scene>>,
    address: String,
    session: Box<dyn DocumentSession<Scene>>,
    window: Option<Arc<Window>>,
    host: Option<SurfaceHost>,
    a11y_bridge: Option<AccessKitBridge>,
    a11y: Accessibility,
    width: u32,
    height: u32,
    /// Physical device pixels per logical layout pixel.
    scale_factor: f32,
    frames: u32,
    modifiers: SessionModifiers,
    /// Last cursor position in logical pixels; winit's button events carry none.
    cursor: (f32, f32),
    pointer_captured: bool,
    /// Driving steps still to apply. They run once, after the first frame has
    /// established geometry, so a `click` has a laid-out box to hit.
    pending_actions: Vec<Action>,
    start: Instant,
    capture: Option<(std::path::PathBuf, u64)>,
    wake_deadline: Option<Instant>,
    receipt_deadline: Option<Instant>,
    failure: Option<String>,
}

impl Ortet {
    fn new(
        config: Config,
        engine: Box<dyn SessionEngine<Scene>>,
        session: Box<dyn DocumentSession<Scene>>,
    ) -> Self {
        let start = Instant::now();
        let receipt_deadline = config
            .artifact
            .as_ref()
            .map(|_| start + RECEIPT_SETTLE_TIMEOUT);
        Self {
            address: config.address.clone(),
            pending_actions: config.actions.clone(),
            width: config.size.0,
            height: config.size.1,
            config,
            engine,
            session,
            window: None,
            host: None,
            a11y_bridge: None,
            a11y: Accessibility::default(),
            scale_factor: 1.0,
            frames: 0,
            modifiers: SessionModifiers::default(),
            cursor: (0.0, 0.0),
            pointer_captured: false,
            start,
            capture: None,
            wake_deadline: None,
            receipt_deadline,
            failure: None,
        }
    }

    fn outcome(&self) -> Outcome {
        Outcome {
            engine_id: self.engine.engine_id().to_owned(),
            backend: self.config.engine.name().to_owned(),
            metadata: build_metadata(),
            address: self.address.clone(),
            frames: self.frames,
            size: (self.width, self.height),
            artifact: self.capture.as_ref().map(|(path, _)| path.clone()),
            digest: self.capture.as_ref().map(|(_, digest)| *digest),
        }
    }

    fn logical_size(&self) -> (u32, u32) {
        let logical = |extent: u32| {
            if self.scale_factor > 0.0 {
                ((extent as f32 / self.scale_factor).round() as u32).max(1)
            } else {
                extent.max(1)
            }
        };
        (logical(self.width), logical(self.height))
    }

    fn request_redraw(&self) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn receipt_settled(&mut self) -> Result<bool, String> {
        if self.config.artifact.is_none() || self.session.settled() {
            return Ok(true);
        }
        if self
            .receipt_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(format!(
                "semantic completion timed out after {}s",
                RECEIPT_SETTLE_TIMEOUT.as_secs()
            ));
        }
        Ok(false)
    }

    fn schedule_wake(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if self
            .wake_deadline
            .take()
            .is_some_and(|deadline| deadline <= now)
        {
            self.request_redraw();
        }
        let pending = self.session.pending_work();
        if let Some(deadline) = pending.next_wake(now) {
            self.wake_deadline = Some(deadline);
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        } else {
            self.wake_deadline = None;
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }

    fn retitle(&self) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(&format!("ortet — {}", self.address));
        }
    }

    /// Follow a link: resolve it against the current address and replace the
    /// session. Ortet keeps no history, so this is the whole of navigation.
    fn navigate(&mut self, target: &str) {
        let resolved = resolve_href(&self.address, target);
        if resolved == self.address {
            return;
        }
        match spawn(self.engine.as_ref(), &resolved, self.logical_size()) {
            Ok(session) => {
                self.session = session;
                self.a11y.replace_session();
                self.address = resolved;
                self.retitle();
                self.publish_accessibility();
                self.request_redraw();
            },
            Err(error) => eprintln!("[ortet] {error}"),
        }
    }

    fn apply_click(&mut self, click: SessionClick) {
        match click {
            SessionClick::Navigate(target) => self.navigate(&target),
            SessionClick::Submit(action) => {
                // Collecting and confirming a request body is a product flow,
                // and product flows are Mere's. Ortet says so instead of
                // inventing one.
                eprintln!("[ortet] form submission to {action} is not wired in this host");
            },
            SessionClick::Handled | SessionClick::Miss => {},
        }
    }

    fn publish_accessibility(&mut self) {
        let update = self.a11y.publish(self.session.accessibility_projection());
        if let Some(update) = update
            && let Some(bridge) = self.a11y_bridge.as_mut()
        {
            bridge.update(update);
        }
    }

    fn drain_accessibility_actions(&mut self) {
        let requests = self
            .a11y_bridge
            .as_mut()
            .map(AccessKitBridge::drain_actions)
            .unwrap_or_default();
        for request in requests {
            match self.a11y.route(&mut *self.session, &request) {
                RoutedAction::Rejected => {},
                RoutedAction::Dispatched => self.request_redraw(),
                RoutedAction::Click { x, y } => {
                    let _ = self.session.pointer_down(x, y);
                    let click = self.session.pointer_up(x, y);
                    self.apply_click(click);
                },
            }
        }
    }

    /// Run the `--actions` list once, against a document that has already been
    /// laid out at the current viewport.
    fn drive_pending_actions(&mut self) {
        let actions = std::mem::take(&mut self.pending_actions);
        let (width, height) = self.logical_size();
        for action in actions {
            match action {
                Action::Scroll { dx, dy } => {
                    let centre = (width as f32 * 0.5, height as f32 * 0.5);
                    self.session.scroll_at(centre.0, centre.1, dx, dy);
                },
                Action::Click { x, y } => {
                    // Press and release, the same pair a mouse produces: the
                    // Livery lane activates a link on the matching release.
                    let _ = self.session.pointer_down(x, y);
                    let click = self.session.pointer_up(x, y);
                    self.apply_click(click);
                },
            }
        }
    }

    /// Dispatch one neutral input and apply everything the session asked the
    /// host for. Returns `(handled, editable)` so the keyboard path can decide
    /// whether its scroll default still applies.
    fn apply_input(&mut self, input: SessionInput) -> (bool, bool) {
        let result = self.session.input(input);
        if let Some(capture) = result.capture {
            self.pointer_captured = capture;
        }
        if let Some(window) = self.window.as_ref() {
            if let Some(cursor) = result.cursor {
                window.set_cursor(match cursor {
                    SessionCursor::Default => winit::window::CursorIcon::Default,
                    SessionCursor::Pointer => winit::window::CursorIcon::Pointer,
                    SessionCursor::Text => winit::window::CursorIcon::Text,
                });
            }
            window.set_ime_allowed(result.editable);
        }
        let handled = result.effect.is_handled();
        match result.effect {
            SessionEffect::Navigate(target) => self.navigate(&target),
            SessionEffect::Submit(submission) => eprintln!(
                "[ortet] form submission to {} is not wired in this host",
                submission.action
            ),
            SessionEffect::Handled | SessionEffect::Cancelled => self.request_redraw(),
            SessionEffect::Ignored => {},
        }
        (handled, result.editable)
    }

    /// The per-frame shape the host crates document: rasterize the scene into a
    /// texture, acquire the backbuffer, composite, present.
    fn render(&mut self, event_loop: &ActiveEventLoop) {
        let now_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        self.session.pump(now_ms);
        if self.host.is_none() {
            return;
        }
        self.drain_accessibility_actions();
        // The scene is produced before the host is borrowed: driving the
        // pending actions can replace the session, which needs `&mut self`.
        let (width, height) = self.logical_size();
        let mut scene = self.session.frame(width, height);
        if !self.pending_actions.is_empty() {
            self.drive_pending_actions();
            // The actions changed retained state (and may have replaced the
            // session). Present that, not the geometry probe above.
            scene = self.session.frame(width, height);
        }
        self.publish_accessibility();
        let Some(host) = self.host.as_ref() else {
            return;
        };
        let (_scene_texture, view) = host.rasterize_scaled(
            &scene,
            self.width.max(1),
            self.height.max(1),
            // A document with no root background paints over white, as a
            // browser's page canvas does.
            ColorLoad::Clear(wgpu::Color::WHITE),
            self.scale_factor,
        );

        let receipt_ready = match self.receipt_settled() {
            Ok(ready) => ready,
            Err(error) => {
                self.failure = Some(error);
                event_loop.exit();
                return;
            },
        };
        let capture_now = receipt_ready
            && self.config.artifact.is_some()
            && self.capture.is_none()
            && self
                .config
                .frames
                .is_none_or(|limit| self.frames.saturating_add(1) >= limit);
        let captured = if capture_now {
            let path = self
                .config
                .artifact
                .as_deref()
                .expect("capture is gated on an artifact path");
            match receipt::capture(host, &view, self.width, self.height, path) {
                Ok(captured) => Some(captured),
                Err(error) => {
                    self.failure = Some(error);
                    event_loop.exit();
                    return;
                },
            }
        } else {
            None
        };

        let Some(frame) = host.acquire() else { return };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let presented = captured.as_ref().map_or(&view, |captured| &captured.view);
        host.renderer().compose_external_texture(
            presented,
            &target,
            host.format(),
            self.width,
            self.height,
            ExternalTexturePlacement::new([0.0, 0.0, self.width as f32, self.height as f32]),
        );
        host.queue().present(frame);
        self.frames += 1;
        if let Some(captured) = captured {
            self.capture = Some((captured.path, captured.digest));
        }

        if let Some(limit) = self.config.frames {
            if self.frames >= limit && (self.config.artifact.is_none() || self.capture.is_some()) {
                event_loop.exit();
                return;
            }
            // A bounded run owns its redraws until its frame budget and, when
            // requested, semantic receipt completion are both satisfied. The
            // pending-work scheduler supplies timer wakeups between frames.
            if self.frames < limit {
                self.request_redraw();
            }
        }
    }
}

impl ApplicationHandler for Ortet {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title(format!("ortet — {}", self.address))
            .with_visible(false)
            .with_inner_size(winit::dpi::PhysicalSize::new(self.width, self.height));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.failure = Some(format!("could not create the window: {error}"));
                event_loop.exit();
                return;
            },
        };
        let size = window.inner_size();
        self.width = size.width.max(1);
        self.height = size.height.max(1);
        self.scale_factor = window.scale_factor() as f32;
        // The bridge must be installed while the native window is hidden on
        // Windows. Frame once to obtain the session's first real projection;
        // an honest empty document is used only if that engine has none.
        let (logical_width, logical_height) = self.logical_size();
        let _ = self.session.frame(logical_width, logical_height);
        let initial = self
            .a11y
            .publish(self.session.accessibility_projection())
            .expect("accessibility publication always supplies a tree");
        let wake_window = window.clone();
        let mut bridge = AccessKitBridge::new(move || wake_window.request_redraw());
        if let Err(error) = bridge.install(&window, initial) {
            self.failure = Some(format!("could not install accessibility bridge: {error}"));
            event_loop.exit();
            return;
        }
        if bridge.status() == BridgeStatus::Installed {
            eprintln!("[ortet] accessibility bridge installed");
        }
        self.a11y_bridge = Some(bridge);
        // A raw browsing host is exactly the case wgpu's limit bucketing exists
        // for: adapter limits are a fingerprinting surface and the content here
        // is untrusted by construction.
        let options = NetrenderOptions {
            tile_cache_size: Some(64),
            enable_vello: true,
            ..NetrenderOptions::for_untrusted_content()
        };
        match SurfaceHost::boot(window.clone(), self.width, self.height, options) {
            Ok(host) => self.host = Some(host),
            Err(error) => {
                self.failure = Some(error);
                event_loop.exit();
                return;
            },
        }
        self.window = Some(window);
        if let Some(window) = self.window.as_ref() {
            window.set_visible(true);
            window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.schedule_wake(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.as_ref().map(|window| window.id()) != Some(window_id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.width = size.width.max(1);
                self.height = size.height.max(1);
                if let Some(host) = self.host.as_mut() {
                    host.resize(self.width, self.height);
                }
                self.request_redraw();
            },
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale_factor = scale_factor as f32;
                if let Some(window) = self.window.as_ref() {
                    let size = window.inner_size();
                    self.width = size.width.max(1);
                    self.height = size.height.max(1);
                }
                if let Some(host) = self.host.as_mut() {
                    host.resize(self.width, self.height);
                }
                self.request_redraw();
            },
            WindowEvent::MouseWheel { delta, .. } => {
                // The shared wheel default action: `genet-winit-host` owns the
                // translation, and the nested scroller under the pointer takes
                // it before the document viewport does.
                let (dx, dy) = wheel_delta_from_winit(delta);
                let scale = if self.scale_factor > 0.0 {
                    self.scale_factor
                } else {
                    1.0
                };
                let (dx, dy) = (dx / scale, dy / scale);
                if self.session.scroll_at(self.cursor.0, self.cursor.1, dx, dy) {
                    self.request_redraw();
                }
            },
            WindowEvent::ModifiersChanged(modifiers) => {
                let state = modifiers.state();
                self.modifiers = SessionModifiers {
                    shift: state.shift_key(),
                    control: state.control_key(),
                    alt: state.alt_key(),
                    meta: state.super_key(),
                };
            },
            WindowEvent::CursorMoved { position, .. } => {
                let scale = if self.scale_factor > 0.0 {
                    self.scale_factor
                } else {
                    1.0
                };
                self.cursor = (position.x as f32 / scale, position.y as f32 / scale);
                let (x, y) = self.cursor;
                let _ = self.apply_input(SessionInput::PointerMoved {
                    x,
                    y,
                    modifiers: self.modifiers,
                });
            },
            WindowEvent::MouseInput { state, button, .. } => {
                let (x, y) = self.cursor;
                let _ = self.apply_input(SessionInput::PointerButton {
                    x,
                    y,
                    button: pointer_button_from_winit(button),
                    state: button_state_from_winit(state),
                    modifiers: self.modifiers,
                });
            },
            WindowEvent::KeyboardInput { event, .. } => {
                let (handled, editable) = self.apply_input(SessionInput::Key {
                    key: session_key_from_winit(&event.logical_key),
                    state: button_state_from_winit(event.state),
                    modifiers: self.modifiers,
                    repeat: event.repeat,
                });
                if event.state == ElementState::Pressed
                    && !handled
                    && !editable
                    && let Some(key) =
                        scroll_key_from_winit(&event.logical_key, self.modifiers.shift)
                    && self.session.scroll_for_key(key)
                {
                    self.request_redraw();
                }
            },
            WindowEvent::Ime(ime) => {
                let _ = self.apply_input(SessionInput::Ime(ime_from_winit(ime)));
            },
            WindowEvent::Focused(focused) => {
                if let Some(bridge) = self.a11y_bridge.as_mut() {
                    bridge.update_window_focus(focused);
                }
                if !focused && self.pointer_captured {
                    let _ = self.apply_input(SessionInput::Cancel);
                    self.pointer_captured = false;
                }
                let _ = self.apply_input(SessionInput::Focus(focused));
            },
            WindowEvent::RedrawRequested => self.render(event_loop),
            _ => {},
        }
    }
}

fn session_key_from_winit(key: &Key) -> SessionKey {
    match key {
        Key::Character(text) => SessionKey::Character(text.to_string()),
        Key::Named(NamedKey::Enter) => SessionKey::Enter,
        Key::Named(NamedKey::Tab) => SessionKey::Tab,
        Key::Named(NamedKey::Backspace) => SessionKey::Backspace,
        Key::Named(NamedKey::Delete) => SessionKey::Delete,
        Key::Named(NamedKey::Escape) => SessionKey::Escape,
        Key::Named(NamedKey::Space) => SessionKey::Space,
        Key::Named(NamedKey::ArrowLeft) => SessionKey::ArrowLeft,
        Key::Named(NamedKey::ArrowRight) => SessionKey::ArrowRight,
        Key::Named(NamedKey::ArrowUp) => SessionKey::ArrowUp,
        Key::Named(NamedKey::ArrowDown) => SessionKey::ArrowDown,
        Key::Named(NamedKey::Home) => SessionKey::Home,
        Key::Named(NamedKey::End) => SessionKey::End,
        Key::Named(NamedKey::PageUp) => SessionKey::PageUp,
        Key::Named(NamedKey::PageDown) => SessionKey::PageDown,
        _ => SessionKey::Unidentified,
    }
}

/// The keyboard scroll defaults, for keys the session did not consume.
fn scroll_key_from_winit(key: &Key, shift: bool) -> Option<SessionScrollKey> {
    Some(match key {
        Key::Named(NamedKey::ArrowUp) => SessionScrollKey::LineUp,
        Key::Named(NamedKey::ArrowDown) => SessionScrollKey::LineDown,
        Key::Named(NamedKey::PageUp) => SessionScrollKey::PageUp,
        Key::Named(NamedKey::PageDown) => SessionScrollKey::PageDown,
        Key::Named(NamedKey::Home) => SessionScrollKey::Home,
        Key::Named(NamedKey::End) => SessionScrollKey::End,
        Key::Named(NamedKey::Space) => {
            if shift {
                SessionScrollKey::PageUp
            } else {
                SessionScrollKey::PageDown
            }
        },
        _ => return None,
    })
}

fn pointer_button_from_winit(button: MouseButton) -> SessionPointerButton {
    match button {
        MouseButton::Left => SessionPointerButton::Primary,
        MouseButton::Right => SessionPointerButton::Secondary,
        MouseButton::Middle | MouseButton::Back | MouseButton::Forward | MouseButton::Other(_) => {
            SessionPointerButton::Auxiliary
        },
    }
}

fn button_state_from_winit(state: ElementState) -> SessionButtonState {
    match state {
        ElementState::Pressed => SessionButtonState::Pressed,
        ElementState::Released => SessionButtonState::Released,
    }
}

fn ime_from_winit(ime: winit::event::Ime) -> SessionIme {
    match ime {
        winit::event::Ime::Enabled => SessionIme::Enabled,
        winit::event::Ime::Preedit(text, selection) => SessionIme::Preedit { text, selection },
        winit::event::Ime::Commit(text) => SessionIme::Commit(text),
        winit::event::Ime::Disabled => SessionIme::Disabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::EngineChoice;

    #[test]
    fn livery_selection_reports_the_concrete_engine_id() {
        let engine = make_engine(EngineChoice::Livery, OrtetFetcher::local_only())
            .expect("default Livery engine is available");
        assert_eq!(
            engine.engine_id(),
            document_session_api::engine_ids::ENGINE_GENET_LIVERY
        );
    }

    #[cfg(feature = "scripted")]
    #[test]
    fn boa_selection_reports_the_concrete_engine_id() {
        let engine = make_engine(EngineChoice::Boa, OrtetFetcher::local_only())
            .expect("Boa is available when the scripted feature is enabled");
        assert_eq!(
            engine.engine_id(),
            document_session_api::engine_ids::ENGINE_GENET_SCRIPTED
        );
    }

    #[cfg(feature = "scripted-nova")]
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn nova_selection_reports_the_concrete_engine_id() {
        let engine = make_engine(EngineChoice::Nova, OrtetFetcher::local_only())
            .expect("Nova is available on 64-bit targets with the feature enabled");
        assert_eq!(
            engine.engine_id(),
            document_session_api::engine_ids::ENGINE_GENET_SCRIPTED_NOVA
        );
    }

    #[cfg(not(feature = "scripted"))]
    #[test]
    fn unavailable_boa_reports_the_build_feature() {
        let error = match make_engine(EngineChoice::Boa, OrtetFetcher::local_only()) {
            Ok(_) => panic!("default Ortet keeps Boa out of its dependency cone"),
            Err(error) => error,
        };
        assert!(error.contains("--features scripted"), "{error}");
    }

    #[cfg(not(feature = "scripted-nova"))]
    #[test]
    fn unavailable_nova_reports_the_build_feature() {
        let error = match make_engine(EngineChoice::Nova, OrtetFetcher::local_only()) {
            Ok(_) => panic!("default Ortet keeps Nova out of its dependency cone"),
            Err(error) => error,
        };
        assert!(error.contains("--features scripted-nova"), "{error}");
    }
}
