// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The raw `wasm32` Ortet host.
//!
//! [`start`] receives a real browser canvas, boots the target-neutral
//! `RenderCore`, and connects DOM input to the same retained Livery session
//! path that the native host uses. It intentionally has no winit, AccessKit,
//! Cambium, or Mere dependency: browser ownership stops at the canvas and DOM
//! events. Fetches in an older navigation are ignored when a newer generation
//! wins; aborting their browser requests is a separate follow-up.

use std::cell::RefCell;
use std::rc::Rc;

use document_session_api::session_engine::{
    DocumentSession, SessionButtonState, SessionEffect, SessionEngine, SessionInput, SessionKey,
    SessionModifiers, SessionPointerButton, SessionScrollKey, SessionSpawnRequest,
};
use genet_document_resources::ResourceLimits;
use genet_documents::{LiveryResourcePreparation, LiverySessionEngine, LocalFetcher};
use genet_host_api::navigation::resolve_href;
use genet_host_api::{ResourceFetchPolicy, ResourceResponse};
use genet_render_host::{RenderCore, WindowSurface};
use netrender::{ColorLoad, ExternalTexturePlacement, NetrenderOptions, Scene};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Event, EventTarget, HtmlCanvasElement, KeyboardEvent, PointerEvent, Response, WheelEvent,
};

/// Browser-host bounds for one navigation.  The defaults retain the shared
/// host response cap and make the aggregate/resource caps explicit here.
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub struct BrowserResourceLimits {
    max_response_bytes: u32,
    max_total_bytes: u32,
    max_resources: u32,
}

impl Default for BrowserResourceLimits {
    fn default() -> Self {
        let policy = ResourceFetchPolicy::default();
        Self {
            max_response_bytes: policy.max_response_bytes.min(u32::MAX as usize) as u32,
            max_total_bytes: policy
                .max_response_bytes
                .saturating_mul(4)
                .min(u32::MAX as usize) as u32,
            max_resources: 64,
        }
    }
}

#[wasm_bindgen]
impl BrowserResourceLimits {
    #[wasm_bindgen(constructor)]
    pub fn new(max_response_bytes: u32, max_total_bytes: u32, max_resources: u32) -> Self {
        Self {
            max_response_bytes,
            max_total_bytes,
            max_resources,
        }
    }
}

/// Mount Ortet on `canvas` and open `address`. Browser fetch and asynchronous
/// scheduling remain in this host; the document engine receives only immutable
/// response snapshots through its existing synchronous resource seam.
#[wasm_bindgen]
pub async fn start(canvas: HtmlCanvasElement, address: String) -> Result<(), JsValue> {
    start_with_limits(canvas, address, BrowserResourceLimits::default()).await
}

/// Mount Ortet with host-selected acquisition bounds.
#[wasm_bindgen]
pub async fn start_with_limits(
    canvas: HtmlCanvasElement,
    address: String,
    limits: BrowserResourceLimits,
) -> Result<(), JsValue> {
    let (layout_width, layout_height, scale, physical_width, physical_height) =
        canvas_size(&canvas)?;
    canvas.set_width(physical_width);
    canvas.set_height(physical_height);
    canvas.set_tab_index(0);
    let loaded = load_session(&address, layout_width, layout_height, limits).await?;
    let options = NetrenderOptions {
        tile_cache_size: Some(64),
        enable_vello: true,
        ..NetrenderOptions::for_untrusted_content()
    };
    let core = RenderCore::boot_async(options)
        .await
        .map_err(|error| JsValue::from_str(&format!("could not boot renderer: {error}")))?;
    let surface = core
        .create_surface(
            wgpu::SurfaceTarget::Canvas(canvas.clone()),
            physical_width,
            physical_height,
        )
        .map_err(|error| JsValue::from_str(&format!("could not create canvas surface: {error}")))?;

    let host = Rc::new(RefCell::new(WebOrtet {
        canvas: canvas.clone(),
        core,
        surface,
        address: loaded.navigation_address,
        session: loaded.session,
        layout_width,
        layout_height,
        scale,
        physical_width,
        physical_height,
        cursor: (0.0, 0.0),
        modifiers: SessionModifiers::default(),
        pointer_capture_id: None,
        browser_limits: limits,
        navigation_generation: 0,
        pending_navigation: None,
    }));
    host.borrow_mut().render();
    install_dom_events(&host)?;
    Ok(())
}

struct WebOrtet {
    canvas: HtmlCanvasElement,
    core: RenderCore,
    surface: WindowSurface,
    address: String,
    session: Box<dyn DocumentSession<Scene>>,
    layout_width: u32,
    layout_height: u32,
    scale: f32,
    physical_width: u32,
    physical_height: u32,
    cursor: (f32, f32),
    modifiers: SessionModifiers,
    pointer_capture_id: Option<i32>,
    browser_limits: BrowserResourceLimits,
    navigation_generation: u64,
    pending_navigation: Option<String>,
}

struct LoadedSession {
    session: Box<dyn DocumentSession<Scene>>,
    navigation_address: String,
}

impl WebOrtet {
    fn render(&mut self) {
        let scene = self.session.frame(self.layout_width, self.layout_height);
        let (_texture, view) = self.core.rasterize_scaled(
            &scene,
            self.physical_width,
            self.physical_height,
            ColorLoad::Clear(wgpu::Color::WHITE),
            self.scale,
        );
        let Some(frame) = self.surface.acquire(&self.core) else {
            return;
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.core.renderer().compose_external_texture(
            &view,
            &target,
            self.surface.format(),
            self.physical_width,
            self.physical_height,
            ExternalTexturePlacement::new([
                0.0,
                0.0,
                self.physical_width as f32,
                self.physical_height as f32,
            ]),
        );
        self.core.queue().present(frame);
    }

    fn resize(&mut self) {
        let Ok((layout_width, layout_height, scale, physical_width, physical_height)) =
            canvas_size(&self.canvas)
        else {
            return;
        };
        if (physical_width, physical_height) != (self.physical_width, self.physical_height) {
            self.canvas.set_width(physical_width);
            self.canvas.set_height(physical_height);
            self.surface
                .resize(&self.core, physical_width, physical_height);
        }
        self.layout_width = layout_width;
        self.layout_height = layout_height;
        self.scale = scale;
        self.physical_width = physical_width;
        self.physical_height = physical_height;
        self.render();
    }

    fn dispatch_input(&mut self, input: SessionInput) -> Option<bool> {
        let result = self.session.input(input);
        let capture = result.capture;
        match result.effect {
            SessionEffect::Navigate(target) => {
                self.pending_navigation = Some(resolve_href(&self.address, &target));
            },
            SessionEffect::Submit(submission) => {
                web_sys::console::warn_1(
                    &format!(
                        "[ortet] form submission to {} is not wired in this host",
                        submission.action
                    )
                    .into(),
                );
            },
            SessionEffect::Handled | SessionEffect::Cancelled => self.render(),
            SessionEffect::Ignored => {},
        }
        capture
    }

    fn capture_pointer(&mut self, pointer_id: i32) {
        if self.canvas.set_pointer_capture(pointer_id).is_ok() {
            self.pointer_capture_id = Some(pointer_id);
        }
    }

    fn release_pointer(&mut self, pointer_id: i32) {
        if self.pointer_capture_id == Some(pointer_id) {
            let _ = self.canvas.release_pointer_capture(pointer_id);
            self.pointer_capture_id = None;
        }
    }

    fn release_captured_pointer(&mut self) {
        if let Some(pointer_id) = self.pointer_capture_id {
            self.release_pointer(pointer_id);
        }
    }

    fn pointer_position(&self, event: &PointerEvent) -> (f32, f32) {
        let rect = self.canvas.get_bounding_client_rect();
        let width = rect.width().max(1.0);
        let height = rect.height().max(1.0);
        (
            ((f64::from(event.client_x()) - rect.left()) * f64::from(self.layout_width) / width)
                .clamp(0.0, f64::from(self.layout_width)) as f32,
            ((f64::from(event.client_y()) - rect.top()) * f64::from(self.layout_height) / height)
                .clamp(0.0, f64::from(self.layout_height)) as f32,
        )
    }
}

fn launch_pending_navigation(host: &Rc<RefCell<WebOrtet>>) {
    let Some((generation, address, width, height, limits)) = (|| {
        let mut host = host.borrow_mut();
        let address = host.pending_navigation.take()?;
        if address == host.address {
            return None;
        }
        host.navigation_generation = host.navigation_generation.wrapping_add(1);
        Some((
            host.navigation_generation,
            address,
            host.layout_width,
            host.layout_height,
            host.browser_limits,
        ))
    })() else {
        return;
    };
    let host = host.clone();
    spawn_local(async move {
        match load_session(&address, width, height, limits).await {
            Ok(loaded) => {
                let mut host = host.borrow_mut();
                if host.navigation_generation != generation {
                    return;
                }
                host.session = loaded.session;
                host.address = loaded.navigation_address;
                host.render();
            },
            Err(error) => {
                if host.borrow().navigation_generation == generation {
                    web_sys::console::warn_1(
                        &format!("[ortet] could not open {address}: {error:?}").into(),
                    );
                }
            },
        }
    });
}

async fn load_session(
    address: &str,
    width: u32,
    height: u32,
    limits: BrowserResourceLimits,
) -> Result<LoadedSession, JsValue> {
    let request = SessionSpawnRequest::new(address).with_viewport(width, height);
    let engine = LiverySessionEngine::new(LocalFetcher);
    if !is_remote_http(address) {
        let session = engine
            .spawn(&request)
            .map_err(|error| JsValue::from_str(&format!("could not open {address}: {error}")))?;
        return Ok(LoadedSession {
            session,
            navigation_address: address.to_owned(),
        });
    }
    let mut total_bytes = 0usize;
    if limits.max_resources == 0 {
        return Err(JsValue::from_str("browser resource count limit exceeded"));
    }
    let source_response = fetch_browser_response(address, &limits, &mut total_bytes).await?;
    let navigation_address = redirected_navigation_address(address, &source_response.final_url);
    let mut staged =
        LiveryResourcePreparation::new(&request, source_response, ResourceLimits::default());
    let mut resources = 1usize;
    loop {
        let pending = staged.pending_requests();
        if pending.is_empty() {
            break;
        }
        for request in pending {
            resources = resources.saturating_add(1);
            if resources > limits.max_resources as usize {
                return Err(JsValue::from_str("browser resource count limit exceeded"));
            }
            match fetch_browser_response(&request.url, &limits, &mut total_bytes).await {
                Ok(response) => staged
                    .provide_response(request, response)
                    .map_err(|error| JsValue::from_str(&format!("invalid provision: {error:?}")))?,
                Err(_) => {
                    staged.provide_unavailable(request).map_err(|error| {
                        JsValue::from_str(&format!("invalid provision: {error:?}"))
                    })?;
                },
            }
        }
    }
    let session = engine
        .spawn_prepared(&request, staged)
        .map_err(|error| JsValue::from_str(&format!("could not open {address}: {error}")))?;
    Ok(LoadedSession {
        session,
        navigation_address,
    })
}

async fn fetch_browser_response(
    url: &str,
    limits: &BrowserResourceLimits,
    total_bytes: &mut usize,
) -> Result<ResourceResponse, JsValue> {
    let window =
        web_sys::window().ok_or_else(|| JsValue::from_str("browser window is unavailable"))?;
    let response = JsFuture::from(window.fetch_with_str(url))
        .await?
        .dyn_into::<Response>()?;
    if !response.ok() {
        return Err(JsValue::from_str(&format!(
            "HTTP status {}",
            response.status()
        )));
    }
    if response
        .headers()
        .get("content-length")?
        .and_then(|length| length.parse::<usize>().ok())
        .is_some_and(|length| length > limits.max_response_bytes as usize)
    {
        return Err(JsValue::from_str("browser response byte limit exceeded"));
    }
    let final_url = if response.url().is_empty() {
        url.to_owned()
    } else {
        response.url()
    };
    let content_type = response.headers().get("content-type")?;
    let body = response
        .body()
        .ok_or_else(|| JsValue::from_str("browser response body is unavailable"))?;
    let reader = body
        .get_reader()
        .dyn_into::<web_sys::ReadableStreamDefaultReader>()?;
    let mut bytes = Vec::new();
    loop {
        let read = JsFuture::from(reader.read()).await?;
        if js_sys::Reflect::get(&read, &JsValue::from_str("done"))?
            .as_bool()
            .unwrap_or(false)
        {
            break;
        }
        let chunk =
            js_sys::Uint8Array::new(&js_sys::Reflect::get(&read, &JsValue::from_str("value"))?);
        let length = chunk.length() as usize;
        if bytes.len().saturating_add(length) > limits.max_response_bytes as usize
            || total_bytes.saturating_add(length) > limits.max_total_bytes as usize
        {
            let _ = reader.cancel();
            return Err(JsValue::from_str("browser response byte limit exceeded"));
        }
        bytes.extend_from_slice(&chunk.to_vec());
        *total_bytes += length;
    }
    Ok(ResourceResponse {
        final_url,
        content_type,
        bytes,
    })
}

fn is_remote_http(address: &str) -> bool {
    address.split_once(':').is_some_and(|(scheme, _)| {
        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
    })
}

/// Keep an explicitly requested fragment while replacing the transport base
/// with the redirect-final document identity.
fn redirected_navigation_address(requested: &str, final_url: &str) -> String {
    let final_base = final_url
        .split_once('#')
        .map_or(final_url, |(base, _)| base);
    requested.split_once('#').map_or_else(
        || final_base.to_owned(),
        |(_, fragment)| format!("{final_base}#{fragment}"),
    )
}

#[cfg(test)]
mod tests {
    use super::{is_remote_http, redirected_navigation_address};

    #[test]
    fn redirected_navigation_uses_final_base_and_requested_fragment() {
        assert_eq!(
            redirected_navigation_address(
                "HTTPS://example.test/start#proof",
                "https://cdn.example.test/final/index.html",
            ),
            "https://cdn.example.test/final/index.html#proof"
        );
        assert!(is_remote_http("HTTPS://example.test/start"));
    }
}

fn install_dom_events(host: &Rc<RefCell<WebOrtet>>) -> Result<(), JsValue> {
    let canvas = host.borrow().canvas.clone();
    let target: &EventTarget = canvas.unchecked_ref();

    listen(target, "pointermove", {
        let host = host.clone();
        move |event| {
            let Some(event) = event.dyn_ref::<PointerEvent>() else {
                return;
            };
            {
                let mut state = host.borrow_mut();
                state.cursor = state.pointer_position(event);
                let (x, y) = state.cursor;
                state.modifiers = modifiers_from_pointer(event);
                let modifiers = state.modifiers;
                state.dispatch_input(SessionInput::PointerMoved { x, y, modifiers });
            }
            launch_pending_navigation(&host);
        }
    })?;
    listen(target, "pointerdown", {
        let host = host.clone();
        move |event| {
            let Some(event) = event.dyn_ref::<PointerEvent>() else {
                return;
            };
            let _ = host.borrow().canvas.focus();
            {
                let mut state = host.borrow_mut();
                state.cursor = state.pointer_position(event);
                let (x, y) = state.cursor;
                state.modifiers = modifiers_from_pointer(event);
                let modifiers = state.modifiers;
                if state.dispatch_input(SessionInput::PointerButton {
                    x,
                    y,
                    button: pointer_button(event.button()),
                    state: SessionButtonState::Pressed,
                    modifiers,
                }) == Some(true)
                {
                    state.capture_pointer(event.pointer_id());
                }
            }
            launch_pending_navigation(&host);
        }
    })?;
    listen(target, "pointerup", {
        let host = host.clone();
        move |event| {
            let Some(event) = event.dyn_ref::<PointerEvent>() else {
                return;
            };
            {
                let mut state = host.borrow_mut();
                state.cursor = state.pointer_position(event);
                let (x, y) = state.cursor;
                state.modifiers = modifiers_from_pointer(event);
                let modifiers = state.modifiers;
                state.dispatch_input(SessionInput::PointerButton {
                    x,
                    y,
                    button: pointer_button(event.button()),
                    state: SessionButtonState::Released,
                    modifiers,
                });
                state.release_pointer(event.pointer_id());
            }
            launch_pending_navigation(&host);
        }
    })?;
    listen(target, "pointercancel", {
        let host = host.clone();
        move |event| {
            let Some(event) = event.dyn_ref::<PointerEvent>() else {
                return;
            };
            let mut host = host.borrow_mut();
            host.dispatch_input(SessionInput::Cancel);
            host.release_pointer(event.pointer_id());
        }
    })?;
    listen(target, "wheel", {
        let host = host.clone();
        move |event| {
            let Some(event) = event.dyn_ref::<WheelEvent>() else {
                return;
            };
            event.prevent_default();
            let mut host = host.borrow_mut();
            let scale = if host.scale > 0.0 { host.scale } else { 1.0 };
            let (x, y) = host.cursor;
            if host.session.scroll_at(
                x,
                y,
                event.delta_x() as f32 / scale,
                event.delta_y() as f32 / scale,
            ) {
                host.render();
            }
        }
    })?;
    listen(target, "keydown", {
        let host = host.clone();
        move |event| {
            let Some(event) = event.dyn_ref::<KeyboardEvent>() else {
                return;
            };
            {
                let mut state = host.borrow_mut();
                state.modifiers = modifiers_from_key(event);
                let modifiers = state.modifiers;
                let key = session_key(event.key());
                let scroll = scroll_key(&key, modifiers.shift);
                state.dispatch_input(SessionInput::Key {
                    key,
                    state: SessionButtonState::Pressed,
                    modifiers,
                    repeat: event.repeat(),
                });
                if let Some(scroll) = scroll
                    && state.session.scroll_for_key(scroll)
                {
                    event.prevent_default();
                    state.render();
                }
            }
            launch_pending_navigation(&host);
        }
    })?;
    listen(target, "keyup", {
        let host = host.clone();
        move |event| {
            let Some(event) = event.dyn_ref::<KeyboardEvent>() else {
                return;
            };
            {
                let mut state = host.borrow_mut();
                state.modifiers = modifiers_from_key(event);
                let modifiers = state.modifiers;
                state.dispatch_input(SessionInput::Key {
                    key: session_key(event.key()),
                    state: SessionButtonState::Released,
                    modifiers,
                    repeat: false,
                });
            }
            launch_pending_navigation(&host);
        }
    })?;
    listen(target, "focus", {
        let host = host.clone();
        move |_| {
            host.borrow_mut().dispatch_input(SessionInput::Focus(true));
        }
    })?;
    listen(target, "blur", {
        let host = host.clone();
        move |_| {
            let mut host = host.borrow_mut();
            if host.pointer_capture_id.is_some() {
                host.dispatch_input(SessionInput::Cancel);
            }
            host.release_captured_pointer();
            host.dispatch_input(SessionInput::Focus(false));
        }
    })?;
    let window =
        web_sys::window().ok_or_else(|| JsValue::from_str("browser window is unavailable"))?;
    let window_target: &EventTarget = window.unchecked_ref();
    listen(window_target, "resize", {
        let host = host.clone();
        move |_| host.borrow_mut().resize()
    })?;
    Ok(())
}

fn listen(
    target: &EventTarget,
    event: &str,
    handler: impl FnMut(Event) + 'static,
) -> Result<(), JsValue> {
    let closure = Closure::wrap(Box::new(handler) as Box<dyn FnMut(Event)>);
    target.add_event_listener_with_callback(event, closure.as_ref().unchecked_ref())?;
    // The browser owns the listener for the lifetime of its canvas. Holding the
    // closure in JS also retains the host state captured by each handler.
    closure.forget();
    Ok(())
}

fn canvas_size(canvas: &HtmlCanvasElement) -> Result<(u32, u32, f32, u32, u32), JsValue> {
    let window =
        web_sys::window().ok_or_else(|| JsValue::from_str("browser window is unavailable"))?;
    let rect = canvas.get_bounding_client_rect();
    let layout_width = rect.width().round().max(1.0) as u32;
    let layout_height = rect.height().round().max(1.0) as u32;
    let scale = window.device_pixel_ratio().max(1.0) as f32;
    Ok((
        layout_width,
        layout_height,
        scale,
        (layout_width as f32 * scale).round().max(1.0) as u32,
        (layout_height as f32 * scale).round().max(1.0) as u32,
    ))
}

fn modifiers_from_pointer(event: &PointerEvent) -> SessionModifiers {
    SessionModifiers {
        shift: event.shift_key(),
        control: event.ctrl_key(),
        alt: event.alt_key(),
        meta: event.meta_key(),
    }
}

fn modifiers_from_key(event: &KeyboardEvent) -> SessionModifiers {
    SessionModifiers {
        shift: event.shift_key(),
        control: event.ctrl_key(),
        alt: event.alt_key(),
        meta: event.meta_key(),
    }
}

fn pointer_button(button: i16) -> SessionPointerButton {
    match button {
        0 => SessionPointerButton::Primary,
        1 | 3 | 4 => SessionPointerButton::Auxiliary,
        2 => SessionPointerButton::Secondary,
        _ => SessionPointerButton::Auxiliary,
    }
}

fn session_key(key: String) -> SessionKey {
    match key.as_str() {
        "Enter" => SessionKey::Enter,
        "Tab" => SessionKey::Tab,
        "Backspace" => SessionKey::Backspace,
        "Delete" => SessionKey::Delete,
        "Escape" => SessionKey::Escape,
        " " => SessionKey::Space,
        "ArrowLeft" => SessionKey::ArrowLeft,
        "ArrowRight" => SessionKey::ArrowRight,
        "ArrowUp" => SessionKey::ArrowUp,
        "ArrowDown" => SessionKey::ArrowDown,
        "Home" => SessionKey::Home,
        "End" => SessionKey::End,
        "PageUp" => SessionKey::PageUp,
        "PageDown" => SessionKey::PageDown,
        _ if key.chars().count() == 1 => SessionKey::Character(key),
        _ => SessionKey::Unidentified,
    }
}

fn scroll_key(key: &SessionKey, shift: bool) -> Option<SessionScrollKey> {
    Some(match key {
        SessionKey::ArrowUp => SessionScrollKey::LineUp,
        SessionKey::ArrowDown => SessionScrollKey::LineDown,
        SessionKey::PageUp => SessionScrollKey::PageUp,
        SessionKey::PageDown => SessionScrollKey::PageDown,
        SessionKey::Home => SessionScrollKey::Home,
        SessionKey::End => SessionScrollKey::End,
        SessionKey::Space if shift => SessionScrollKey::PageUp,
        SessionKey::Space => SessionScrollKey::PageDown,
        _ => return None,
    })
}
