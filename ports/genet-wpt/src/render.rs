/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! HTML -> image, for reftest pixel comparison (phase 2).
//!
//! Replicates the public path the `html_to_pixels_e2e` test drives:
//! parse -> cascade -> layout -> emit paint list -> netrender -> readback,
//! through the same `genet-render-host` core the raw host presents with. The
//! wgpu boot + netrender instance are created once ([`Renderer::boot`]) and
//! reused across every test in a subset.
//!
//! The Livery route keeps the producer bounded: linked stylesheets and local
//! image bytes are supplied by the host, while remote fetch remains outside
//! the route.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;

use genet_document_resources::{ResolvedDocumentResources, ResourceKind, resolve_with};
use genet_livery::{
    Device as LiveryDevice, LiveryDocument, StyleSet as LiveryStyleSet,
    table_shadow::TableShadowLedger,
};
use genet_static_dom::StaticDocument;
use layout_dom_api::LayoutDom;
use genet_render_host::RenderCore;
use netrender::{ColorLoad, NetrenderOptions};
use paint_list_api::{
    ColorF, CommonPlacement, DeviceIntSize, IdNamespace, ImageKey, LayoutPoint, LayoutRect,
    LayoutTransform, PaintCmd, PaintEnvelope, RectItem, TransformKind, TransformSpec,
};
use paint_list_render::translate_envelope_with_external_textures;
use paint_types::PipelineId;

pub type Image = image::ImageBuffer<image::Rgba<u8>, Vec<u8>>;

#[derive(Clone)]
struct ResourceResolver {
    base_dir: std::path::PathBuf,
    tests_root: std::path::PathBuf,
}

impl ResourceResolver {
    fn resolve(&self, authored: &str) -> Option<std::path::PathBuf> {
        let authored = authored.split(['#', '?']).next()?.trim();
        if authored.is_empty() || authored.starts_with("data:") {
            return None;
        }
        for scheme in ["http://", "https://"] {
            if let Some(rest) = authored.strip_prefix(scheme) {
                let (_, path) = rest.split_once('/')?;
                return Some(self.tests_root.join(path));
            }
        }
        if let Some(rest) = authored.strip_prefix('/') {
            return Some(self.tests_root.join(rest));
        }
        Some(self.base_dir.join(authored))
    }

    fn load(&self, authored: &str) -> Option<Vec<u8>> {
        std::fs::read(self.resolve(authored)?).ok()
    }

    fn document_url(&self) -> String {
        let relative = self
            .base_dir
            .strip_prefix(&self.tests_root)
            .unwrap_or(self.base_dir.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        format!(
            "http://web-platform.test/{}/__genet_wpt_document__.html",
            relative.trim_matches('/')
        )
    }
}

fn document_resources<D: LayoutDom>(
    dom: &D,
    resolver: &ResourceResolver,
) -> ResolvedDocumentResources {
    let document_url = resolver.document_url();
    let mut fetch = |url: &str| resolver.load(url);
    resolve_with(dom, Some(&document_url), &mut fetch)
}

/// The visual result and the exact table dispatch record that produced it.
///
/// Reftest comparisons consume only [`Self::image`]. The ledger is an
/// optional accounting surface for the table lane: it distinguishes a painted
/// result supplied by Buckram from one that remained on an explicit fallback.
pub struct LiveryRender {
    pub image: Image,
    pub table_ledger: TableShadowLedger,
}

/// The two coordinate spaces a reftest render needs.
///
/// Layout remains in CSS pixels. The device target and the final paint-stream
/// transform use `device_scale`, so a scale-two run exercises the same page
/// geometry at twice the raster resolution rather than laying out a wider page.
#[derive(Clone, Copy, Debug)]
pub struct RenderViewport {
    css_width: u32,
    css_height: u32,
    device_scale: f32,
    device_width: u32,
    device_height: u32,
}

impl RenderViewport {
    pub fn new(css_width: u32, css_height: u32, device_scale: f32) -> Result<Self, String> {
        if css_width == 0 || css_height == 0 {
            return Err("reftest CSS viewport must be non-zero".to_owned());
        }
        if !device_scale.is_finite() || device_scale <= 0.0 {
            return Err(format!(
                "invalid device scale {device_scale}; expected a finite number greater than zero"
            ));
        }
        let device_width = scaled_dimension(css_width, device_scale)?;
        let device_height = scaled_dimension(css_height, device_scale)?;
        Ok(Self {
            css_width,
            css_height,
            device_scale,
            device_width,
            device_height,
        })
    }

    pub fn css_size(self) -> (u32, u32) {
        (self.css_width, self.css_height)
    }

    pub fn device_size(self) -> (u32, u32) {
        (self.device_width, self.device_height)
    }

    pub fn device_scale(self) -> f32 {
        self.device_scale
    }
}

fn scaled_dimension(css: u32, device_scale: f32) -> Result<u32, String> {
    let scaled = (css as f64 * device_scale as f64).round();
    if !scaled.is_finite() || scaled < 1.0 || scaled > i32::MAX as f64 {
        return Err(format!(
            "device scale {device_scale} makes CSS dimension {css} unrepresentable"
        ));
    }
    Ok(scaled as u32)
}

/// A booted renderer reused across a subset's tests.
pub struct Renderer {
    core: RenderCore,
    next_pipeline_index: Cell<u32>,
}

impl Renderer {
    /// Boot wgpu + netrender once. Returns an error string if the GPU is
    /// unavailable (the runner can then report reftests as unrunnable
    /// rather than crash).
    pub fn boot() -> Result<Self, String> {
        let core = RenderCore::boot(NetrenderOptions {
            tile_cache_size: Some(64),
            enable_vello: true,
            ..Default::default()
        })?;
        Ok(Self {
            core,
            next_pipeline_index: Cell::new(1),
        })
    }

    /// Lower one device-space envelope to a netrender scene, materialize its
    /// blurred box-shadow masks, rasterize into a fresh RGBA8 target and read
    /// it back. Reftest envelopes carry no producer textures, so external
    /// texture draws are not composited here.
    fn rasterize(&self, envelope: &PaintEnvelope, viewport: RenderViewport) -> Image {
        let (device_width, device_height) = viewport.device_size();
        let translated = translate_envelope_with_external_textures(envelope);
        debug_assert!(
            translated.external_textures.is_empty(),
            "reftest envelopes carry no producer textures"
        );
        for mask in &translated.box_shadow_masks {
            self.core.renderer().build_box_shadow_mask(
                mask.key,
                mask.dim,
                mask.bounds,
                mask.corner_radius,
                mask.blur_radius_px,
                mask.invert,
            );
        }
        let (texture, _view) = self.core.rasterize_scaled(
            &translated.scene,
            device_width,
            device_height,
            ColorLoad::Clear(wgpu::Color::TRANSPARENT),
            1.0,
        );
        let frame = self
            .core
            .read_rgba8_texture(&texture, device_width, device_height)
            .expect("reftest readback");
        Image::from_raw(frame.width, frame.height, frame.rgba).expect("reftest frame dimensions")
    }

    fn next_pipeline_id(&self) -> PipelineId {
        let index = self.next_pipeline_index.get();
        self.next_pipeline_index.set(index.saturating_add(1));
        PipelineId(1, index)
    }

    /// Render `html` to an image in `viewport`, resolving the page's inline +
    /// linked CSS and local images relative to `base_dir` (and `tests_root`
    /// for `/`-absolute URLs).
    /// Render through the clean-room Livery lane. This first WPT bridge is
    /// intentionally bounded: it extracts inline and local linked stylesheets,
    /// supplies host-resolved local image bytes, and lets Livery handle its own
    /// declarations and data-URI image subset.
    pub fn render_html(
        &self,
        html: &str,
        base_dir: &Path,
        tests_root: &Path,
        viewport: RenderViewport,
        is_xml: bool,
    ) -> LiveryRender {
        let pipeline_id = self.next_pipeline_id();
        let (css_width, css_height) = viewport.css_size();
        let document = if is_xml {
            StaticDocument::parse_xml(html)
        } else {
            StaticDocument::parse(html)
        };
        let resolver = ResourceResolver {
            base_dir: base_dir.to_path_buf(),
            tests_root: tests_root.to_path_buf(),
        };
        let resources = document_resources(&document, &resolver);
        let sheets = resources
            .stylesheets
            .iter()
            .map(|sheet| sheet.text.clone())
            .collect::<Vec<_>>();
        let sheet_refs = sheets.iter().map(String::as_str).collect::<Vec<_>>();
        let mut session = LiveryDocument::new(
            document,
            LiveryStyleSet::cambium(&sheet_refs),
            LiveryDevice::screen(css_width as f32, css_height as f32),
        );
        for resource in resources.resources {
            match resource.kind {
                ResourceKind::Image => {
                    session
                        .set_image_resource(resource.authored_url.clone(), resource.bytes.clone());
                    session.set_image_resource(resource.resolved_url, resource.bytes);
                },
                ResourceKind::Font => {
                    session.set_font_resource(resource.authored_url, resource.bytes.clone());
                    session.set_font_resource(resource.resolved_url, resource.bytes);
                },
            }
        }
        let list = session
            .frame(css_width, css_height)
            .expect("Livery WPT reftest layout");
        let table_ledger = session.table_shadow_ledger().cloned().unwrap_or_default();
        let envelope = isolate_image_keys(
            with_reftest_backdrop(PaintEnvelope::from_list(&list), css_width, css_height),
            pipeline_id,
        );
        let envelope = scale_envelope_for_device(envelope, viewport);
        let image = self.rasterize(&envelope, viewport);
        LiveryRender {
            image,
            table_ledger,
        }
    }
}

/// Give every frame's image resources a distinct namespace before handing the
/// list to the long-lived NetRender instance. Producers intentionally restart
/// their per-list key counters at one; NetRender retains atlas entries after a
/// pipeline exits and rejects a reused key whose dimensions differ.
fn isolate_image_keys(mut envelope: PaintEnvelope, pipeline_id: PipelineId) -> PaintEnvelope {
    if envelope.images.is_empty() {
        return envelope;
    }

    let namespace = IdNamespace(0x4000_0000 | pipeline_id.1);
    let mut remap = HashMap::with_capacity(envelope.images.len());
    for image in &mut envelope.images {
        let old = image.key;
        let new = ImageKey::new(namespace, old.1);
        image.key = new;
        remap.insert(old, new);
    }

    for command in &mut envelope.commands {
        match command {
            PaintCmd::DrawImage(item) => remap_key(&mut item.image_key, &remap),
            PaintCmd::DrawRepeatingImage(item) => remap_key(&mut item.image_key, &remap),
            PaintCmd::PushLayer(layer) => {
                if let Some(mask) = &mut layer.mask {
                    if let Some(key) = &mut mask.image_mask {
                        remap_key(key, &remap);
                    }
                }
            },
            _ => {},
        }
    }
    envelope
}

/// WPT screenshots composite the document canvas over an opaque white
/// browser backdrop. The CSS canvas background remains engine-owned and paints
/// over this command; a transparent canvas exposes white instead of NetRender's
/// implementation clear color.
fn with_reftest_backdrop(mut envelope: PaintEnvelope, width: u32, height: u32) -> PaintEnvelope {
    envelope.commands.insert(
        0,
        PaintCmd::DrawRect(RectItem {
            placement: CommonPlacement::new(LayoutRect::new(
                LayoutPoint::new(0.0, 0.0),
                LayoutPoint::new(width as f32, height as f32),
            )),
            color: ColorF::WHITE,
        }),
    );
    envelope
}

/// Adapt a CSS-space paint envelope to the physical target selected by the
/// WPT runner. The NetRender provider consumes the envelope's viewport and
/// coordinates, so the scale is an explicit root transform.
fn scale_envelope_for_device(
    mut envelope: PaintEnvelope,
    viewport: RenderViewport,
) -> PaintEnvelope {
    let (device_width, device_height) = viewport.device_size();
    envelope.viewport = DeviceIntSize::new(device_width as i32, device_height as i32);
    if viewport.device_scale() == 1.0 {
        return envelope;
    }

    let scale = viewport.device_scale();
    envelope.commands.insert(
        0,
        PaintCmd::PushTransform(TransformSpec {
            origin: LayoutPoint::new(0.0, 0.0),
            transform: LayoutTransform::new(
                scale, 0.0, 0.0, 0.0, 0.0, scale, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ),
            kind: TransformKind::Standard,
        }),
    );
    envelope.commands.push(PaintCmd::PopTransform);
    envelope
}

fn remap_key(key: &mut ImageKey, remap: &HashMap<ImageKey, ImageKey>) {
    if let Some(&new) = remap.get(key) {
        *key = new;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RenderViewport, isolate_image_keys, scale_envelope_for_device, with_reftest_backdrop,
    };
    use paint_list_api::{
        AlphaType, ColorF, CommonPlacement, DeviceIntSize, EngineId, IdNamespace, ImageItem,
        ImageKey, ImageRendering, ImageResource, LayoutPoint, LayoutRect, PaintCmd, PaintEnvelope,
        RectItem,
    };
    use paint_types::PipelineId;

    #[test]
    fn image_keys_are_namespaced_per_pipeline() {
        let old_key = ImageKey::new(IdNamespace(1), 1);
        let envelope = PaintEnvelope {
            engine: EngineId::GENET,
            viewport: DeviceIntSize::new(1, 1),
            generation: 0,
            commands: vec![PaintCmd::DrawImage(ImageItem {
                placement: CommonPlacement::new(LayoutRect::new(
                    LayoutPoint::new(0.0, 0.0),
                    LayoutPoint::new(1.0, 1.0),
                )),
                image_key: old_key,
                image_rendering: ImageRendering::Auto,
                alpha_type: AlphaType::Alpha,
                color: ColorF::WHITE,
            })],
            fonts: Vec::new(),
            images: vec![ImageResource {
                key: old_key,
                width: 1,
                height: 1,
                data: vec![255, 255, 255, 255],
            }],
        };

        let rekeyed = isolate_image_keys(envelope, PipelineId(1, 7));
        let new_key = ImageKey::new(IdNamespace(0x4000_0007), 1);
        assert_eq!(rekeyed.images[0].key, new_key);
        let PaintCmd::DrawImage(item) = &rekeyed.commands[0] else {
            panic!("expected image command");
        };
        assert_eq!(item.image_key, new_key);
    }

    #[test]
    fn reftest_backdrop_is_white_and_precedes_document_paint() {
        let envelope = with_reftest_backdrop(
            PaintEnvelope {
                engine: EngineId::GENET,
                viewport: DeviceIntSize::new(20, 10),
                generation: 0,
                commands: Vec::new(),
                fonts: Vec::new(),
                images: Vec::new(),
            },
            20,
            10,
        );
        let PaintCmd::DrawRect(rect) = &envelope.commands[0] else {
            panic!("backdrop is a rectangle");
        };
        assert_eq!(rect.color, ColorF::WHITE);
        assert_eq!(rect.placement.bounds.max, LayoutPoint::new(20.0, 10.0));
    }

    #[test]
    fn device_scale_keeps_css_layout_and_scales_the_provider_target() {
        let viewport = RenderViewport::new(800, 600, 2.0).unwrap();
        assert_eq!(viewport.css_size(), (800, 600));
        assert_eq!(viewport.device_size(), (1600, 1200));

        let envelope = scale_envelope_for_device(
            PaintEnvelope {
                engine: EngineId::GENET,
                viewport: DeviceIntSize::new(800, 600),
                generation: 0,
                commands: vec![PaintCmd::DrawRect(RectItem {
                    placement: CommonPlacement::new(LayoutRect::new(
                        LayoutPoint::new(0.0, 0.0),
                        LayoutPoint::new(10.0, 10.0),
                    )),
                    color: ColorF::WHITE,
                })],
                fonts: Vec::new(),
                images: Vec::new(),
            },
            viewport,
        );
        assert_eq!(envelope.viewport, DeviceIntSize::new(1600, 1200));
        let [
            PaintCmd::PushTransform(spec),
            PaintCmd::DrawRect(_),
            PaintCmd::PopTransform,
        ] = envelope.commands.as_slice()
        else {
            panic!("device scale wraps the complete command stream");
        };
        assert_eq!(spec.transform.m11, 2.0);
        assert_eq!(spec.transform.m22, 2.0);
        assert_eq!(viewport.css_size(), (800, 600));
        assert_eq!(viewport.device_size(), (1600, 1200));
    }

    #[test]
    fn device_scale_rejects_non_positive_or_unrepresentable_targets() {
        assert!(RenderViewport::new(800, 600, 0.0).is_err());
        assert!(RenderViewport::new(800, 600, f32::NAN).is_err());
        assert!(RenderViewport::new(800, 600, f32::MAX).is_err());
    }
}
