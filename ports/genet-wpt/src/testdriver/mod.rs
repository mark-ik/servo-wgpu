//! The testdriver input path: WebDriver action sequences interpreted into
//! per-tick input events, delivered to the retained session as DOM events.
//!
//! `input_events` and `webdriver_actions` were absorbed from Servo's
//! `embedder_traits` on 2026-09-07 when the constellation cone was retired.
//! genet-wpt is their only consumer; a host-facing input contract, if one is
//! wanted later, should grow in `genet-host-api`, not here.

use euclid::Point2D;
use paint_types::units::{CSSPixel, DevicePoint, LayoutPoint};
use serde::{Deserialize, Serialize};

pub mod input_events;
pub mod webdriver_actions;

/// A point in a `WebView`, either in device pixels or page (CSS) pixels.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub enum WebViewPoint {
    Device(DevicePoint),
    Page(Point2D<f32, CSSPixel>),
}

impl From<DevicePoint> for WebViewPoint {
    fn from(point: DevicePoint) -> Self {
        Self::Device(point)
    }
}

impl From<LayoutPoint> for WebViewPoint {
    fn from(point: LayoutPoint) -> Self {
        Self::Page(Point2D::new(point.x, point.y))
    }
}
