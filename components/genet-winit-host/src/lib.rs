/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Shared genet-on-winit host plumbing.
//!
//! What is specific to winit lives here: the single-window [`SurfaceHost`]
//! convenience, wheel translation, and the AccessKit bridge. The present
//! mechanics under them — booting wgpu + a netrender [`Renderer`], configuring
//! a surface, rasterizing a [`Scene`], acquiring a backbuffer — are not about
//! windowing at all and now live in `genet-render-host`, which a browser
//! canvas host reaches without pulling winit or accesskit into its build.
//! [`RenderCore`] and [`WindowSurface`] are re-exported here so existing
//! callers are unaffected.
//!
//! Each host keeps its own scene composition and input routing. Cambium
//! keyboard translation lives in `cambium-winit`; this engine host does not
//! depend on the GUI layer.
//!
//! Per-frame shape a host follows:
//!
//! ```text
//! let (_tex, view) = host.rasterize(&scene, w, h, clear);   // one per layer
//! let Some(frame)  = host.acquire() else { return };         // skip if outdated
//! let target = frame.texture.create_view(&Default::default());
//! host.renderer().compose_external_texture(&view, &target, host.format(), w, h, placement);
//! host.queue().present(frame);
//! ```

mod a11y;
pub use a11y::{A11yActionRequest, AccessKitBridge, BridgeStatus};

use std::sync::Arc;

use netrender::{ColorLoad, NetrenderOptions, Renderer, Scene};
use winit::event::MouseScrollDelta;
use winit::window::Window;

/// The target-neutral present core. Re-exported so hosts that already speak
/// `genet_winit_host::RenderCore` keep working; new target-neutral code should
/// depend on `genet-render-host` directly.
pub use genet_render_host::{RenderCore, WindowSurface};

/// A [`RenderCore`] + its one [`WindowSurface`]: the single-window present stack,
/// kept as a convenience for hosts that only ever have one window (the standalone
/// orrery host). A multi-window host holds a shared `RenderCore` + a `WindowSurface`
/// per window directly. The per-frame shape is unchanged — `rasterize` each scene,
/// `acquire` the backbuffer, composite, present.
pub struct SurfaceHost {
    core: Arc<RenderCore>,
    surface: WindowSurface,
}

impl SurfaceHost {
    /// Attach a native surface to an already-booted shared render core.
    ///
    /// Hosts that need document-local same-device capabilities before parsing
    /// can boot the core first, build those capabilities, then create the
    /// visible surface without allocating a second wgpu device.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_shared_core(
        window: Arc<Window>,
        width: u32,
        height: u32,
        core: Arc<RenderCore>,
    ) -> Result<Self, String> {
        let surface = core.create_surface(window, width, height)?;
        Ok(Self { core, surface })
    }

    /// Boot the core + create this window's surface (native blocking).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn boot(
        window: Arc<Window>,
        width: u32,
        height: u32,
        options: NetrenderOptions,
    ) -> Result<Self, String> {
        Self::boot_with_transparency(window, width, height, options, false)
    }

    /// Boot against a native window that needs compositor-visible alpha.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn boot_with_transparency(
        window: Arc<Window>,
        width: u32,
        height: u32,
        options: NetrenderOptions,
        transparent: bool,
    ) -> Result<Self, String> {
        let core = Arc::new(RenderCore::boot(options)?);
        let surface = core.create_surface_with_transparency(window, width, height, transparent)?;
        Ok(Self { core, surface })
    }

    /// Async boot (the only path on wasm; works everywhere).
    pub async fn boot_async(
        window: Arc<Window>,
        width: u32,
        height: u32,
        options: NetrenderOptions,
    ) -> Result<Self, String> {
        let core = Arc::new(RenderCore::boot_async(options).await?);
        let surface = core.create_surface(window, width, height)?;
        Ok(Self { core, surface })
    }

    /// The shared render core (device + renderer).
    pub fn core(&self) -> &RenderCore {
        self.core.as_ref()
    }

    /// Clone the render core for a second native surface. A multi-window host
    /// owns this shared core plus one [`WindowSurface`] per window, preserving
    /// one wgpu device across every presentation target.
    pub fn shared_core(&self) -> Arc<RenderCore> {
        Arc::clone(&self.core)
    }

    /// The netrender renderer — call `compose_external_texture` (and friends) on it.
    pub fn renderer(&self) -> &Renderer {
        self.core.renderer()
    }

    /// The surface's texture format (pass to `compose_external_texture`).
    pub fn format(&self) -> wgpu::TextureFormat {
        self.surface.format()
    }

    /// The wgpu device backing the renderer.
    pub fn device(&self) -> &wgpu::Device {
        self.core.device()
    }

    /// The wgpu queue backing the renderer (e.g. for external-texture import).
    pub fn queue(&self) -> &wgpu::Queue {
        self.core.queue()
    }

    /// Reconfigure the surface for a new size (clamped to ≥ 1).
    pub fn resize(&mut self, width: u32, height: u32) {
        self.surface.resize(&self.core, width, height);
    }

    /// Rasterize `scene` into a fresh `(w, h)` texture cleared to `clear`.
    pub fn rasterize(
        &self,
        scene: &Scene,
        w: u32,
        h: u32,
        clear: ColorLoad,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        self.core.rasterize(scene, w, h, clear)
    }

    /// Rasterize a logical-coordinate scene into a physical `(w, h)` texture at
    /// `scale` device pixels per logical pixel. The same scale must be used for
    /// layout and hit testing by the caller; this wrapper keeps the window host
    /// on the crisp renderer path instead of upscaling a low-resolution texture.
    pub fn rasterize_scaled(
        &self,
        scene: &Scene,
        w: u32,
        h: u32,
        clear: ColorLoad,
        scale: f32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        self.core.rasterize_scaled(scene, w, h, clear, scale)
    }

    /// Rasterize with caller-owned same-device textures at their emitted scene
    /// operation boundaries.
    pub fn rasterize_scaled_with_external_textures(
        &self,
        scene: &Scene,
        w: u32,
        h: u32,
        clear: ColorLoad,
        scale: f32,
        external_textures: &[netrender::ExternalTextureComposite<'_>],
    ) -> (wgpu::Texture, wgpu::TextureView) {
        self.core.rasterize_scaled_with_external_textures(
            scene,
            w,
            h,
            clear,
            scale,
            external_textures,
        )
    }

    /// Acquire the surface backbuffer for this frame (`None` to skip on outdated).
    pub fn acquire(&self) -> Option<wgpu::SurfaceTexture> {
        self.surface.acquire(&self.core)
    }
}

/// Device px per wheel "line" step, for `MouseScrollDelta::LineDelta` events
/// (mouse wheels report lines, trackpads report pixels). One notch ≈ a few lines.
pub const WHEEL_LINE_PX: f32 = 48.0;

/// Map a winit wheel event to a device-px delta to **add** to a document's
/// viewport scroll (`viewport.scroll += delta`). A line step scales by
/// [`WHEEL_LINE_PX`]; a pixel step (trackpad) passes through. The sign is flipped
/// from winit's "positive = content moves up / away", so rolling the wheel down
/// advances the document toward its end (a larger offset). The shared wheel default
/// action (scope doc rule 5): every winit host maps the wheel through this one
/// helper, not several hand-rolled copies.
///
/// A host whose scroll model is in **logical** pixels wants
/// [`wheel_delta_from_winit_logical`] instead: winit's `PixelDelta` is physical,
/// so on a 2x display this one returns twice the distance such a host means.
pub fn wheel_delta_from_winit(delta: MouseScrollDelta) -> (f32, f32) {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => (-x * WHEEL_LINE_PX, -y * WHEEL_LINE_PX),
        MouseScrollDelta::PixelDelta(p) => (-(p.x as f32), -(p.y as f32)),
    }
}

/// [`wheel_delta_from_winit`] for a host whose scroll model is in **logical**
/// pixels, such as `cambium_rootstock::Host::wheel`.
///
/// Winit reports a trackpad's `PixelDelta` in *physical* device pixels — the
/// same frame `CursorMoved` uses, which every such host already divides by the
/// scale factor. Left unscaled it made a trackpad scroll `scale_factor` times
/// as far as the pointer moved, so on a 2x display the page ran away under the
/// finger. It is divided here.
///
/// `LineDelta` is untouched: [`WHEEL_LINE_PX`] is already a logical figure —
/// a line is a line whatever the display's density — so scaling it would make
/// a mouse wheel travel *half* as far on a 2x display, trading one wrong feel
/// for another.
///
/// A `scale_factor` that is not positive (a platform that never reported one)
/// is read as 1.0 rather than producing an infinity.
pub fn wheel_delta_from_winit_logical(delta: MouseScrollDelta, scale_factor: f64) -> (f32, f32) {
    match delta {
        MouseScrollDelta::LineDelta(..) => wheel_delta_from_winit(delta),
        MouseScrollDelta::PixelDelta(p) => {
            let scale = if scale_factor > 0.0 {
                scale_factor
            } else {
                1.0
            };
            ((-(p.x / scale)) as f32, (-(p.y / scale)) as f32)
        },
    }
}

#[cfg(test)]
mod tests {
    use winit::dpi::PhysicalPosition;

    use super::*;

    /// A line step scales to `WHEEL_LINE_PX` with the sign flipped: rolling the
    /// wheel down (winit y < 0) advances the document (positive dy), up reverses it.
    #[test]
    fn wheel_line_delta_maps_to_document_scroll() {
        let (dx, down) = wheel_delta_from_winit(MouseScrollDelta::LineDelta(0.0, -1.0));
        assert_eq!(dx, 0.0);
        assert!(
            (down - WHEEL_LINE_PX).abs() < 0.01,
            "one line down = +{WHEEL_LINE_PX}px, got {down}"
        );
        let (_, up) = wheel_delta_from_winit(MouseScrollDelta::LineDelta(0.0, 1.0));
        assert!(
            (up + WHEEL_LINE_PX).abs() < 0.01,
            "one line up = -{WHEEL_LINE_PX}px, got {up}"
        );
    }

    /// Pixel deltas (trackpads) pass through unscaled, sign-flipped.
    #[test]
    fn wheel_pixel_delta_passes_through() {
        let got = wheel_delta_from_winit(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
            3.0, -10.0,
        )));
        assert_eq!(got, (-3.0, 10.0));
    }

    /// The logical variant halves a physical pixel delta at scale 2, so a
    /// trackpad moves the document exactly as far as it moves the pointer.
    #[test]
    fn wheel_pixel_delta_scales_to_logical_pixels() {
        let physical = MouseScrollDelta::PixelDelta(PhysicalPosition::new(3.0, -10.0));
        assert_eq!(
            wheel_delta_from_winit_logical(physical, 1.0),
            wheel_delta_from_winit(physical),
            "scale 1 must agree with the device-px mapping"
        );
        assert_eq!(
            wheel_delta_from_winit_logical(physical, 2.0),
            (-1.5, 5.0),
            "scale 2 halves the distance"
        );
        // A platform that never reported a scale factor must not produce an
        // infinite scroll.
        assert_eq!(
            wheel_delta_from_winit_logical(physical, 0.0),
            (-3.0, 10.0),
            "a non-positive scale factor reads as 1.0"
        );
    }

    /// A line step is already logical, so the scale factor must not touch it:
    /// halving it would make a mouse wheel crawl on a 2x display.
    #[test]
    fn wheel_line_delta_is_scale_independent() {
        let lines = MouseScrollDelta::LineDelta(0.0, -1.0);
        assert_eq!(
            wheel_delta_from_winit_logical(lines, 2.0),
            wheel_delta_from_winit(lines)
        );
        assert_eq!(
            wheel_delta_from_winit_logical(lines, 2.0),
            (0.0, WHEEL_LINE_PX)
        );
    }
}
