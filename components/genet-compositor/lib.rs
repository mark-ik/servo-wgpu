/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! genet's platform presentation layer: the per-OS compositor backends that
//! present a netrender frame (DXGI swapchain on Windows, CALayer on macOS,
//! Wayland subsurfaces with dmabuf on Linux), the same-device interop that
//! shares a wgpu context with a host (`HostWgpuContext`, Dx12 fence sync,
//! Vulkan timelines), and the backend-neutral rendering-context contract that
//! WebGL binds against.
//!
//! Carved out of the former `servo-paint` crate on 2026-09-07: this half was
//! never Servo code and imports nothing from Servo's message traits. The
//! message-driven `Paint` painter that lived beside it was retired with the
//! rest of the Servo constellation cone; the raw host and the WPT reftest lane
//! both render through `genet-render-host` instead.

#![deny(unsafe_code)]

#[allow(unsafe_code)]
pub mod interop;
pub mod rendering_context_core;

#[allow(deprecated)]
pub use crate::compositor::{
    OsCompositorBackend, PaintCompositor, ServoCompositor, StubCompositor, WgpuMasterCaptureBackend,
};
#[cfg(target_vendor = "apple")]
pub use crate::compositor_calayer::{
    BackendError as MacosCALayerBackendError, MacosCALayerBackend,
};
#[cfg(target_os = "windows")]
pub use crate::compositor_dxgi::{BackendError as WindowsDxgiBackendError, WindowsDxgiBackend};
pub use crate::compositor_factory::{
    BoxedFactoryError, default_compositor_for_window, default_compositor_for_window_or_capture,
};
#[cfg(target_os = "linux")]
pub use crate::compositor_wayland::{
    BackendError as WaylandSubsurfaceBackendError, WaylandSubsurfaceBackend,
};
#[cfg(target_os = "windows")]
pub use crate::interop::Dx12FenceSynchronizer;
pub use crate::interop::{HostWgpuContext, InteropBackend, InteropError, SyncMechanism};

mod compositor;
#[cfg(target_vendor = "apple")]
#[allow(unsafe_code)]
mod compositor_calayer;
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod compositor_dxgi;
mod compositor_factory;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod compositor_wayland;
