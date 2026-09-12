// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Native WebGL contexts backed by Ortet's existing render device.
//!
//! A [`WebGlHost`] is document-host state, rather than a global GPU helper:
//! every context it mints uses the exact `wgpu::Device` and `Queue` from the
//! current `RenderCore`.  The registry retains only live default-framebuffer
//! contexts.  A handler has a stable key for its lifetime; resize replaces the
//! backing texture in-place and drop removes the key.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use script_runtime_api::WebGlHandler;
use webgl_wgpu::{
    BufferTarget, BufferUsage, PrimitiveMode, ShaderStage, WebGlBufferId, WebGlCanvasDescriptor,
    WebGlContext, WebGlError, WebGlProgramId, WebGlShaderId, WebGlTextureId, WebGlUniformLocation,
};

const GL_ARRAY_BUFFER: u32 = 0x8892;
const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
const GL_DITHER: u32 = 0x0BD0;
const GL_ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
const GL_FRAGMENT_SHADER: u32 = 0x8B30;
const GL_INVALID_ENUM: u32 = 0x0500;
const GL_INVALID_FRAMEBUFFER_OPERATION: u32 = 0x0506;
const GL_INVALID_OPERATION: u32 = 0x0502;
const GL_INVALID_VALUE: u32 = 0x0501;
const GL_NO_ERROR: u32 = 0;
const GL_SCISSOR_TEST: u32 = 0x0C11;
const GL_TRIANGLES: u32 = 0x0004;
const GL_VERTEX_SHADER: u32 = 0x8B31;
const GL_CONTEXT_LOST_WEBGL: u32 = 0x9242;

type Context = Arc<Mutex<WebGlContext>>;

#[derive(Clone)]
struct RegisteredContext {
    context: Context,
    content_generation: Arc<AtomicU64>,
}

type Registry = Arc<Mutex<HashMap<u64, RegisteredContext>>>;

/// Same-device WebGL factory and live default-framebuffer registry.
#[derive(Clone)]
pub struct WebGlHost {
    device: wgpu::Device,
    queue: wgpu::Queue,
    next_key: Arc<AtomicU64>,
    contexts: Registry,
    staged_images: Arc<Mutex<HashSet<netrender::ImageKey>>>,
}

impl WebGlHost {
    /// Build from the `RenderCore` device before a scripted document is
    /// spawned.  This must not create a second wgpu device.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self {
            device,
            queue,
            next_key: Arc::new(AtomicU64::new(1)),
            contexts: Arc::new(Mutex::new(HashMap::new())),
            staged_images: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Fresh document-local script capability. Each factory invocation mints
    /// one context and registers its default framebuffer under a stable key.
    pub fn scripted_document_options(&self) -> genet_scripted::ScriptedDocumentOptions {
        let host = self.clone();
        genet_scripted::ScriptedDocumentOptions {
            webgl: Some(Box::new(move |width, height| {
                Box::new(host.make_handler(width, height))
            })),
        }
    }

    fn make_handler(&self, width: u32, height: u32) -> OrtetWebGl {
        let key = self.next_key.fetch_add(1, Ordering::Relaxed);
        let context = Arc::new(Mutex::new(
            WebGlContext::from_wgpu_handles(
                self.device.clone(),
                self.queue.clone(),
                WebGlCanvasDescriptor::new(width.max(1), height.max(1)),
            )
            .expect("same-device WebGL canvas"),
        ));
        let content_generation = Arc::new(AtomicU64::new(1));
        self.contexts.lock().expect("WebGL registry lock").insert(
            key,
            RegisteredContext {
                context: context.clone(),
                content_generation: content_generation.clone(),
            },
        );
        OrtetWebGl::new(context, key, self.contexts.clone(), content_generation)
    }

    /// Stage every trusted texture used by this frame as a normal Vello image.
    ///
    /// This runs even for an empty draw list so a dropped, hidden, or navigated
    /// canvas cannot leave its previous GPU image registered for a later key.
    pub fn sync_external_images(
        &self,
        renderer: &netrender::Renderer,
        draws: &[document_session_api::SessionExternalTextureDraw],
    ) {
        let live: Vec<(netrender::ImageKey, RegisteredContext)> = {
            let registry = self.contexts.lock().expect("WebGL registry lock");
            draws
                .iter()
                .filter_map(|draw| {
                    registry
                        .get(&draw.texture_key)
                        .cloned()
                        .map(|context| (netrender::external_image_key(draw.texture_key), context))
                })
                .collect()
        };
        let desired: HashSet<_> = live.iter().map(|(key, _)| *key).collect();
        for (image_key, registered) in &live {
            let context = registered.context.lock().expect("WebGL context lock");
            let drawing_buffer = context.texture();
            let source_view = drawing_buffer.create_view();
            renderer.stage_external_image(
                *image_key,
                &source_view,
                [drawing_buffer.size.0, drawing_buffer.size.1],
                netrender::SourceAlpha::Premultiplied,
                registered.content_generation.load(Ordering::Relaxed),
            );
        }
        let mut staged = self.staged_images.lock().expect("staged WebGL images lock");
        for stale in staged.difference(&desired) {
            renderer.unregister_external_image(*stale);
        }
        *staged = desired;
    }

    #[cfg(test)]
    fn live_contexts(&self) -> usize {
        self.contexts.lock().expect("WebGL registry lock").len()
    }
}

struct OrtetWebGl {
    context: Context,
    external_key: u64,
    registry: Registry,
    content_generation: Arc<AtomicU64>,
    /// Next id to hand out across the JS seam for `create_*`. Distinct from
    /// the WebGl*Id namespaces because webgl-wgpu's ids start at 1 per
    /// resource kind; we keep one shared counter to avoid id-collisions
    /// across kinds on the JS side.
    next_id: RefCell<u64>,
    buffers: RefCell<HashMap<u64, WebGlBufferId>>,
    shaders: RefCell<HashMap<u64, WebGlShaderId>>,
    programs: RefCell<HashMap<u64, WebGlProgramId>>,
    textures: RefCell<HashMap<u64, WebGlTextureId>>,
    /// `i32` index into this list is the opaque uniform-location id the JS
    /// wrapper holds onto. `getUniformLocation` returns the index; setters
    /// look it up by index. `-1` is reserved for "not found", per WebGL.
    uniform_locations: RefCell<Vec<WebGlUniformLocation>>,
    /// Current `gl.clearColor` state. webgl-wgpu's `clear` takes the color
    /// directly each call (no bound-color state), so we hold it here and
    /// pass it through on `clear`. WebGL's default is transparent black.
    clear_color: RefCell<[f32; 4]>,
    /// `gl.enable` / `gl.disable` state, keyed by GLenum cap. Source of
    /// truth for `isEnabled`. Only `SCISSOR_TEST` currently forwards to the
    /// backend (webgl-wgpu's scissor toggle); the rest are recorded so the
    /// round-trip is correct even though the draw effect isn't wired yet
    /// (BLEND / CULL_FACE aren't exposed; DEPTH_TEST needs a depth canvas).
    enabled_caps: RefCell<std::collections::HashSet<u32>>,
    /// Errors discovered while decoding a raw WebGL enum before webgl-wgpu
    /// receives a method call.
    pending_error: RefCell<Option<WebGlError>>,
}

impl OrtetWebGl {
    fn new(
        context: Context,
        external_key: u64,
        registry: Registry,
        content_generation: Arc<AtomicU64>,
    ) -> Self {
        // WebGL's only default-enabled capability is DITHER.
        let mut caps = std::collections::HashSet::new();
        caps.insert(GL_DITHER);
        Self {
            context,
            external_key,
            registry,
            content_generation,
            next_id: RefCell::new(1),
            buffers: RefCell::new(HashMap::new()),
            shaders: RefCell::new(HashMap::new()),
            programs: RefCell::new(HashMap::new()),
            textures: RefCell::new(HashMap::new()),
            uniform_locations: RefCell::new(Vec::new()),
            clear_color: RefCell::new([0.0, 0.0, 0.0, 0.0]),
            enabled_caps: RefCell::new(caps),
            pending_error: RefCell::new(None),
        }
    }

    fn alloc_id(&self) -> u64 {
        let mut n = self.next_id.borrow_mut();
        let id = *n;
        *n += 1;
        id
    }

    fn buffer_target(target: u32) -> Option<BufferTarget> {
        match target {
            GL_ARRAY_BUFFER => Some(BufferTarget::ArrayBuffer),
            GL_ELEMENT_ARRAY_BUFFER => Some(BufferTarget::ElementArrayBuffer),
            _ => None,
        }
    }

    fn record_error(&self, error: WebGlError) {
        let mut pending = self.pending_error.borrow_mut();
        if pending.is_none() {
            *pending = Some(error);
        }
    }

    fn note_content_change(&self) {
        self.content_generation.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for OrtetWebGl {
    fn drop(&mut self) {
        self.registry
            .lock()
            .expect("WebGL registry lock")
            .remove(&self.external_key);
    }
}

impl WebGlHandler for OrtetWebGl {
    fn external_texture_key(&self) -> Option<u64> {
        Some(self.external_key)
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.context
            .lock()
            .expect("WebGL context lock")
            .resize(width.max(1), height.max(1))
            .expect("WebGL drawing buffer resizes");
        self.note_content_change();
    }

    fn clear_color(&mut self, r: f32, g: f32, b: f32, a: f32) {
        // webgl-wgpu's clear takes the color directly each call (no
        // bound-color state), so hold it and pass it through on `clear`.
        *self.clear_color.borrow_mut() = [r, g, b, a];
    }

    fn clear(&mut self, mask: u32) {
        if mask & GL_COLOR_BUFFER_BIT != 0 {
            let c = *self.clear_color.borrow();
            self.context
                .lock()
                .expect("webgl context lock")
                .clear(wgpu::Color {
                    r: c[0] as f64,
                    g: c[1] as f64,
                    b: c[2] as f64,
                    a: c[3] as f64,
                });
            self.note_content_change();
        }
    }

    fn enable(&mut self, cap: u32) {
        self.enabled_caps.borrow_mut().insert(cap);
        if cap == GL_SCISSOR_TEST {
            self.context
                .lock()
                .expect("webgl context lock")
                .set_scissor_test_enabled(true);
        }
        // DEPTH_TEST / BLEND / CULL_FACE recorded only — see struct doc.
    }

    fn disable(&mut self, cap: u32) {
        self.enabled_caps.borrow_mut().remove(&cap);
        if cap == GL_SCISSOR_TEST {
            self.context
                .lock()
                .expect("webgl context lock")
                .set_scissor_test_enabled(false);
        }
    }

    fn is_enabled(&mut self, cap: u32) -> bool {
        self.enabled_caps.borrow().contains(&cap)
    }

    fn color_mask(&mut self, r: bool, g: bool, b: bool, a: bool) {
        self.context
            .lock()
            .expect("webgl context lock")
            .set_color_mask(r, g, b, a);
    }

    fn viewport(&mut self, x: i32, y: i32, width: u32, height: u32) {
        if x < 0 || y < 0 {
            self.record_error(WebGlError::InvalidValue);
            return;
        }
        self.context
            .lock()
            .expect("WebGL context lock")
            .viewport(x as u32, y as u32, width, height);
    }

    fn create_buffer(&mut self) -> u64 {
        let id = self.alloc_id();
        let buffer = self
            .context
            .lock()
            .expect("webgl context lock")
            .create_buffer();
        self.buffers.borrow_mut().insert(id, buffer);
        id
    }
    fn bind_buffer(&mut self, target: u32, buffer: Option<u64>) {
        let Some(target) = Self::buffer_target(target) else {
            self.record_error(WebGlError::InvalidEnum);
            return;
        };
        let resolved = buffer.and_then(|id| self.buffers.borrow().get(&id).copied());
        self.context
            .lock()
            .expect("webgl context lock")
            .bind_buffer(target, resolved);
    }
    fn buffer_data_f32(&mut self, target: u32, data: &[f32], _usage: u32) {
        let Some(target) = Self::buffer_target(target) else {
            self.record_error(WebGlError::InvalidEnum);
            return;
        };
        self.context
            .lock()
            .expect("WebGL context lock")
            .buffer_data_f32(target, data, BufferUsage::StaticDraw);
    }

    fn create_shader(&mut self, stage: u32) -> u64 {
        let stage = match stage {
            GL_VERTEX_SHADER => ShaderStage::Vertex,
            GL_FRAGMENT_SHADER => ShaderStage::Fragment,
            _ => {
                self.record_error(WebGlError::InvalidEnum);
                return 0;
            },
        };
        let id = self.alloc_id();
        let shader = self
            .context
            .lock()
            .expect("webgl context lock")
            .create_shader(stage);
        self.shaders.borrow_mut().insert(id, shader);
        id
    }
    fn shader_source(&mut self, shader: u64, source: &str) {
        let Some(&shader_id) = self.shaders.borrow().get(&shader) else {
            return;
        };
        self.context
            .lock()
            .expect("webgl context lock")
            .shader_source(shader_id, source);
    }
    fn compile_shader(&mut self, shader: u64) {
        let Some(&shader_id) = self.shaders.borrow().get(&shader) else {
            return;
        };
        self.context
            .lock()
            .expect("webgl context lock")
            .compile_shader(shader_id);
    }
    fn get_shader_compile_status(&mut self, shader: u64) -> bool {
        let Some(&shader_id) = self.shaders.borrow().get(&shader) else {
            return false;
        };
        self.context
            .lock()
            .expect("WebGL context lock")
            .get_shader_compile_status(shader_id)
    }
    fn get_shader_info_log(&mut self, shader: u64) -> String {
        let Some(&shader_id) = self.shaders.borrow().get(&shader) else {
            return String::new();
        };
        self.context
            .lock()
            .expect("WebGL context lock")
            .get_shader_info_log(shader_id)
            .unwrap_or_default()
    }

    fn create_program(&mut self) -> u64 {
        let id = self.alloc_id();
        let program = self
            .context
            .lock()
            .expect("webgl context lock")
            .create_program();
        self.programs.borrow_mut().insert(id, program);
        id
    }
    fn attach_shader(&mut self, program: u64, shader: u64) {
        let Some(&program_id) = self.programs.borrow().get(&program) else {
            return;
        };
        let Some(&shader_id) = self.shaders.borrow().get(&shader) else {
            return;
        };
        self.context
            .lock()
            .expect("WebGL context lock")
            .attach_shader(program_id, shader_id);
    }
    fn link_program(&mut self, program: u64) {
        let Some(&program_id) = self.programs.borrow().get(&program) else {
            return;
        };
        self.context
            .lock()
            .expect("webgl context lock")
            .link_program(program_id);
    }
    fn get_program_link_status(&mut self, program: u64) -> bool {
        let Some(&program_id) = self.programs.borrow().get(&program) else {
            return false;
        };
        self.context
            .lock()
            .expect("WebGL context lock")
            .get_program_link_status(program_id)
    }
    fn get_program_info_log(&mut self, program: u64) -> String {
        let Some(&program_id) = self.programs.borrow().get(&program) else {
            return String::new();
        };
        self.context
            .lock()
            .expect("WebGL context lock")
            .get_program_info_log(program_id)
            .unwrap_or_default()
    }
    fn use_program(&mut self, program: Option<u64>) {
        let resolved = program.and_then(|id| self.programs.borrow().get(&id).copied());
        self.context
            .lock()
            .expect("webgl context lock")
            .use_program(resolved);
    }

    fn get_attrib_location(&mut self, program: u64, name: &str) -> i32 {
        let Some(&program_id) = self.programs.borrow().get(&program) else {
            return -1;
        };
        self.context
            .lock()
            .expect("WebGL context lock")
            .get_attrib_location(program_id, name)
    }
    fn get_uniform_location(&mut self, program: u64, name: &str) -> i32 {
        let Some(&program_id) = self.programs.borrow().get(&program) else {
            return -1;
        };
        let Some(loc) = self
            .context
            .lock()
            .expect("WebGL context lock")
            .get_uniform_location(program_id, name)
        else {
            return -1;
        };
        let mut locs = self.uniform_locations.borrow_mut();
        let index = locs.len() as i32;
        locs.push(loc);
        index
    }

    fn enable_vertex_attrib_array(&mut self, index: u32) {
        self.context
            .lock()
            .expect("webgl context lock")
            .enable_vertex_attrib_array(index);
    }
    fn vertex_attrib_pointer_f32(
        &mut self,
        index: u32,
        size: u32,
        normalized: bool,
        stride: u32,
        offset: u32,
    ) {
        self.context
            .lock()
            .expect("webgl context lock")
            .vertex_attrib_pointer_f32(index, size, normalized, stride as u64, offset as u64);
    }

    fn uniform4f(&mut self, location: i32, x: f32, y: f32, z: f32, w: f32) {
        if location < 0 {
            return;
        }
        let loc = match self.uniform_locations.borrow().get(location as usize) {
            Some(loc) => *loc,
            None => return,
        };
        self.context
            .lock()
            .expect("webgl context lock")
            .uniform4f(loc, x, y, z, w);
    }
    fn uniform_matrix4fv(&mut self, location: i32, _transpose: bool, value: &[f32]) {
        if location < 0 || value.len() < 16 {
            return;
        }
        let loc = match self.uniform_locations.borrow().get(location as usize) {
            Some(loc) => *loc,
            None => return,
        };
        let mut m = [0.0f32; 16];
        m.copy_from_slice(&value[..16]);
        self.context
            .lock()
            .expect("webgl context lock")
            .uniform_matrix4fv(loc, &m);
    }
    fn uniform1i(&mut self, location: i32, value: i32) {
        if location < 0 {
            return;
        }
        let loc = match self.uniform_locations.borrow().get(location as usize) {
            Some(loc) => *loc,
            None => return,
        };
        self.context
            .lock()
            .expect("webgl context lock")
            .uniform1i(loc, value);
    }

    fn create_texture(&mut self) -> u64 {
        let id = self.alloc_id();
        let texture = self
            .context
            .lock()
            .expect("webgl context lock")
            .create_texture();
        self.textures.borrow_mut().insert(id, texture);
        id
    }
    fn bind_texture_2d(&mut self, texture: Option<u64>) {
        let resolved = texture.and_then(|id| self.textures.borrow().get(&id).copied());
        self.context
            .lock()
            .expect("webgl context lock")
            .bind_texture_2d(resolved);
    }
    fn active_texture(&mut self, unit: u32) {
        self.context
            .lock()
            .expect("webgl context lock")
            .active_texture(unit);
    }
    fn tex_image_2d_rgba8(&mut self, width: u32, height: u32, pixels: &[u8]) {
        self.context
            .lock()
            .expect("WebGL context lock")
            .tex_image_2d_rgba8(width, height, pixels);
    }

    fn draw_arrays(&mut self, mode: u32, first: i32, count: i32) {
        let topology = match mode {
            GL_TRIANGLES => PrimitiveMode::Triangles,
            _ => {
                self.record_error(WebGlError::InvalidEnum);
                return;
            },
        };
        self.context
            .lock()
            .expect("WebGL context lock")
            .draw_arrays(topology, first as u32, count as u32);
        self.note_content_change();
    }

    fn get_error(&mut self) -> u32 {
        let error = self
            .pending_error
            .borrow_mut()
            .take()
            .unwrap_or_else(|| self.context.lock().expect("webgl context lock").get_error());
        match error {
            WebGlError::NoError => GL_NO_ERROR,
            WebGlError::InvalidEnum => GL_INVALID_ENUM,
            WebGlError::InvalidValue => GL_INVALID_VALUE,
            WebGlError::InvalidOperation => GL_INVALID_OPERATION,
            WebGlError::InvalidFramebufferOperation => GL_INVALID_FRAMEBUFFER_OPERATION,
            WebGlError::ContextLostWebgl => GL_CONTEXT_LOST_WEBGL,
        }
    }

    fn read_pixels_rgba8(&mut self, x: i32, y: i32, w: u32, h: u32) -> Vec<u8> {
        if x < 0 || y < 0 {
            self.record_error(WebGlError::InvalidValue);
            return Vec::new();
        }
        self.context
            .lock()
            .expect("WebGL context lock")
            .read_pixels(x as u32, y as u32, w, h)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genet_render_host::RenderCore;
    use netrender::NetrenderOptions;

    fn core_and_host() -> Option<(RenderCore, WebGlHost)> {
        let core = match RenderCore::boot(NetrenderOptions {
            tile_cache_size: Some(4),
            enable_vello: true,
            ..Default::default()
        }) {
            Ok(core) => core,
            Err(error) => {
                eprintln!("skipping Ortet WebGL GPU test: {error}");
                return None;
            },
        };
        let host = WebGlHost::new(core.device().clone(), core.queue().clone());
        Some((core, host))
    }

    fn host() -> Option<WebGlHost> {
        core_and_host().map(|(_, host)| host)
    }

    #[test]
    fn registry_tracks_two_contexts_across_resize_and_drop() {
        let Some(host) = host() else { return };
        let mut first = host.make_handler(4, 4);
        let second = host.make_handler(8, 8);
        let first_key = first.external_texture_key().expect("first key");
        let second_key = second.external_texture_key().expect("second key");
        assert_ne!(first_key, second_key);
        assert_eq!(host.live_contexts(), 2);

        let before = {
            let context = first.context.lock().expect("WebGL context lock");
            (context.texture().size, context.texture().generation)
        };
        first.resize(6, 5);
        let after = {
            let context = first.context.lock().expect("WebGL context lock");
            (context.texture().size, context.texture().generation)
        };
        assert_eq!(first.external_texture_key(), Some(first_key));
        assert_eq!(before.0, (4, 4));
        assert_eq!(after.0, (6, 5));
        assert!(after.1 > before.1);
        drop(second);
        assert_eq!(host.live_contexts(), 1);
        drop(first);
        assert_eq!(host.live_contexts(), 0);
        let third = host.make_handler(2, 2);
        assert_ne!(third.external_texture_key(), Some(first_key));
        drop(third);
    }

    #[test]
    fn document_factory_registers_and_drops_its_context() {
        let Some(host) = host() else { return };
        let mut options = host.scripted_document_options();
        let handler = options.webgl.as_mut().expect("WebGL capability")(3, 2);
        assert_eq!(host.live_contexts(), 1);
        drop(handler);
        assert_eq!(host.live_contexts(), 0);
    }

    #[test]
    fn sync_stages_only_trusted_contexts_refreshes_and_retires() {
        let Some((core, host)) = core_and_host() else {
            return;
        };
        let mut context = host.make_handler(4, 3);
        let key = context.external_texture_key().expect("context key");
        let image_key = netrender::external_image_key(key);
        let draws = [
            document_session_api::SessionExternalTextureDraw {
                texture_key: key,
                dest_rect: [0.0, 0.0, 4.0, 3.0],
                opacity: 1.0,
                scene_op_boundary: 0,
            },
            document_session_api::SessionExternalTextureDraw {
                texture_key: 999,
                dest_rect: [4.0, 0.0, 8.0, 3.0],
                opacity: 1.0,
                scene_op_boundary: 1,
            },
        ];

        host.sync_external_images(core.renderer(), &draws);
        assert_eq!(
            *host.staged_images.lock().expect("staged WebGL images lock"),
            HashSet::from([image_key]),
            "an untrusted/unknown source key must not stage an image"
        );

        context.clear_color(0.0, 1.0, 0.0, 1.0);
        context.clear(GL_COLOR_BUFFER_BIT);
        context.resize(6, 5);
        host.sync_external_images(core.renderer(), &draws);
        assert_eq!(
            *host.staged_images.lock().expect("staged WebGL images lock"),
            HashSet::from([image_key]),
            "a producer resize refreshes the stable staged image key"
        );

        host.sync_external_images(core.renderer(), &[]);
        assert!(
            host.staged_images
                .lock()
                .expect("staged WebGL images lock")
                .is_empty()
        );
        assert!(
            !core.renderer().unregister_external_image(image_key),
            "the empty frame already retired the source"
        );
    }

    #[test]
    fn production_handler_draws_a_triangle() {
        let Some(host) = host() else { return };
        let mut gl = host.make_handler(32, 32);
        gl.viewport(0, 0, 32, 32);
        gl.clear_color(0.0, 0.0, 0.0, 1.0);
        gl.clear(GL_COLOR_BUFFER_BIT);

        let vertex = gl.create_shader(GL_VERTEX_SHADER);
        gl.shader_source(
            vertex,
            "attribute vec2 a_position; void main() { gl_Position = vec4(a_position, 0.0, 1.0); }",
        );
        gl.compile_shader(vertex);
        assert!(gl.get_shader_compile_status(vertex));
        let fragment = gl.create_shader(GL_FRAGMENT_SHADER);
        gl.shader_source(
            fragment,
            "precision mediump float; uniform vec4 u_color; void main() { gl_FragColor = u_color; }",
        );
        gl.compile_shader(fragment);
        assert!(gl.get_shader_compile_status(fragment));
        let program = gl.create_program();
        gl.attach_shader(program, vertex);
        gl.attach_shader(program, fragment);
        gl.link_program(program);
        assert!(gl.get_program_link_status(program));
        gl.use_program(Some(program));
        let color = gl.get_uniform_location(program, "u_color");
        assert!(color >= 0);
        gl.uniform4f(color, 0.0, 1.0, 0.0, 1.0);
        let position = gl.get_attrib_location(program, "a_position");
        assert_eq!(position, 0);
        let buffer = gl.create_buffer();
        gl.bind_buffer(GL_ARRAY_BUFFER, Some(buffer));
        gl.buffer_data_f32(GL_ARRAY_BUFFER, &[-0.8, -0.8, 0.8, -0.8, 0.0, 0.8], 0x88E4);
        gl.enable_vertex_attrib_array(position as u32);
        gl.vertex_attrib_pointer_f32(position as u32, 2, false, 0, 0);
        gl.draw_arrays(GL_TRIANGLES, 0, 3);
        assert_eq!(gl.get_error(), GL_NO_ERROR);
        assert_eq!(gl.read_pixels_rgba8(16, 16, 1, 1), vec![0, 255, 0, 255]);
    }
}
