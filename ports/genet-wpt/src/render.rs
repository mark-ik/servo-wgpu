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
use genet_render_host::RenderCore;
use genet_static_dom::StaticDocument;
use layout_dom_api::LayoutDom;
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

/// One child browsing context in the reftest lane: its retained document and
/// its own children.
///
/// The reftest lane builds a bare `LiveryDocument` rather than going through
/// `genet-documents`' session engine, so the frame composite has to be
/// assembled here too. Both routes use the same three engine seams -
/// `LiveryPaintList::frame_slots`, `absorb_frame_commands` and
/// `splice_frame_slots` - which is the point of putting them on the paint list
/// rather than in either host.
struct ChildRender {
    owner_node: u64,
    document: Option<genet_livery::LiveryDocument<StaticDocument>>,
    children: Vec<ChildRender>,
}

/// How deep a chain of nested reftest frames to build. Mirrors the session
/// engine's bound; the ancestor-URL check below is the specified guard.
const MAX_FRAME_DEPTH: usize = 8;

fn build_child_documents(
    frames: &[genet_document_resources::ResolvedFrame],
    resolver: &ResourceResolver,
    parent_url: &str,
    ancestors: &mut Vec<String>,
    depth: usize,
) -> Vec<ChildRender> {
    frames
        .iter()
        .map(|frame| {
            let recursive = ancestors.iter().any(|url| url == &frame.resolved_url);
            if frame.source.is_empty() || recursive || depth >= MAX_FRAME_DEPTH {
                return ChildRender {
                    owner_node: frame.owner_node,
                    document: None,
                    children: Vec::new(),
                };
            }
            // A `srcdoc` child's base URL is its container's; a `src` child's
            // is its own.
            let (base_dir, base_url) =
                if frame.source_kind == genet_document_resources::FrameSource::SrcDoc {
                    (resolver.base_dir.clone(), parent_url.to_owned())
                } else {
                    let resolved = resolver.resolve(&frame.resolved_url);
                    let dir = resolved
                        .as_ref()
                        .and_then(|path| path.parent())
                        .map_or_else(|| resolver.base_dir.clone(), std::path::Path::to_path_buf);
                    (dir, frame.resolved_url.clone())
                };
            let child_resolver = ResourceResolver {
                base_dir,
                tests_root: resolver.tests_root.clone(),
            };
            let document = StaticDocument::parse(&frame.source);
            let resources = document_resources(&document, &child_resolver);
            let sheets = resources
                .stylesheets
                .iter()
                .map(|sheet| sheet.text.clone())
                .collect::<Vec<_>>();
            let sheet_refs = sheets.iter().map(String::as_str).collect::<Vec<_>>();
            let mut child = genet_livery::LiveryDocument::new(
                document,
                LiveryStyleSet::cambium(&sheet_refs),
                LiveryDevice::screen(300.0, 150.0),
            );
            for resource in resources.resources {
                match resource.kind {
                    ResourceKind::Image => {
                        child.set_image_resource(
                            resource.authored_url.clone(),
                            resource.bytes.clone(),
                        );
                        child.set_image_resource(resource.resolved_url, resource.bytes);
                    },
                    ResourceKind::Font => {
                        child.set_font_resource(resource.authored_url, resource.bytes.clone());
                        child.set_font_resource(resource.resolved_url, resource.bytes);
                    },
                }
            }
            ancestors.push(frame.resolved_url.clone());
            let children = build_child_documents(
                &resources.frames,
                &child_resolver,
                &base_url,
                ancestors,
                depth + 1,
            );
            ancestors.pop();
            ChildRender {
                owner_node: frame.owner_node,
                document: Some(child),
                children,
            }
        })
        .collect()
}

/// Render each child at its used content size and splice it into the parent's
/// replaced box, recursing so a grandchild is composited into its own parent
/// before that parent is composited into this one.
fn composite_children(list: &mut genet_livery::LiveryPaintList, children: &mut [ChildRender]) {
    if children.is_empty() || list.frame_slots().is_empty() {
        return;
    }
    let slots = list.frame_slots().to_vec();
    let mut rendered: Vec<(u64, Vec<paint_list_api::PaintCmd>)> = Vec::new();
    for slot in &slots {
        let Some(child) = children
            .iter_mut()
            .find(|child| child.owner_node == slot.owner_node)
        else {
            continue;
        };
        let width = slot.content_rect.width().max(0.0).round() as u32;
        let height = slot.content_rect.height().max(0.0).round() as u32;
        if width == 0 || height == 0 {
            continue;
        }
        let Some(document) = child.document.as_mut() else {
            continue;
        };
        let Ok(mut child_list) = document.frame(width, height) else {
            continue;
        };
        composite_children(&mut child_list, &mut child.children);
        rendered.push((slot.owner_node, list.absorb_frame_commands(&child_list)));
    }
    list.splice_frame_slots(|owner| {
        rendered
            .iter()
            .find(|(node, _)| *node == owner)
            .map(|(_, commands)| commands.clone())
    });
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
        let mut children = build_child_documents(
            &resources.frames,
            &resolver,
            resolver.document_url().as_str(),
            &mut Vec::new(),
            0,
        );
        let mut list = session
            .frame(css_width, css_height)
            .expect("Livery WPT reftest layout");
        composite_children(&mut list, &mut children);
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

/// The reftest lane's child-browsing-context composite, proved without a GPU.
///
/// `render_html` rasterizes, so it needs a device and cannot run in CI. The
/// part this lane added is upstream of the raster — build the children, render
/// them at the frame's used content size, splice — and that half is a pure
/// function of the paint list. Testing it here is what makes "the reftest maps
/// did not move" mean *no reftest exercises a frame* rather than *the
/// composite never ran*.
#[cfg(test)]
mod frame_composite_tests {
    use super::*;
    use paint_list_api::{ClipKind, PaintCmd, PaintList};

    fn fixture_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "genet-wpt-frame-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("sub")).expect("fixture dir");
        dir
    }

    /// The lane's own directory under the fixture root, so `base_dir` and
    /// `tests_root` differ the way they do in a real run.
    fn base_dir(root: &std::path::Path) -> std::path::PathBuf {
        root.join("sub")
    }

    /// Build the parent's paint list and composite its children, the way
    /// `render_html` does between layout and raster.
    fn composited(dir: &std::path::Path, parent: &str) -> genet_livery::LiveryPaintList {
        std::fs::write(base_dir(dir).join("parent.html"), parent).expect("write parent");
        let resolver = ResourceResolver {
            base_dir: base_dir(dir),
            tests_root: dir.to_path_buf(),
        };
        let document = StaticDocument::parse(parent);
        let resources = document_resources(&document, &resolver);
        let mut children = build_child_documents(
            &resources.frames,
            &resolver,
            resolver.document_url().as_str(),
            &mut Vec::new(),
            0,
        );
        let sheets = resources
            .stylesheets
            .iter()
            .map(|sheet| sheet.text.clone())
            .collect::<Vec<_>>();
        let sheet_refs = sheets.iter().map(String::as_str).collect::<Vec<_>>();
        let mut session = genet_livery::LiveryDocument::new(
            document,
            LiveryStyleSet::cambium(&sheet_refs),
            LiveryDevice::screen(400.0, 300.0),
        );
        let mut list = session.frame(400, 300).expect("parent layout");
        composite_children(&mut list, &mut children);
        list
    }

    fn has_rect(list: &genet_livery::LiveryPaintList, color: [f32; 3]) -> bool {
        list.commands().iter().any(|command| {
            matches!(command, PaintCmd::DrawRect(rect)
                if (rect.color.r - color[0]).abs() < 0.01
                    && (rect.color.g - color[1]).abs() < 0.01
                    && (rect.color.b - color[2]).abs() < 0.01)
        })
    }

    /// A `src` child is fetched through the reftest resolver, laid out at the
    /// frame's used content size, and spliced under a clip to that box.
    #[test]
    fn a_src_child_is_composited_under_a_clip_to_the_frames_content_box() {
        let dir = fixture_dir("src");
        std::fs::write(
            base_dir(&dir).join("child.html"),
            r#"<body style="margin:0"><div style="width:40px;height:40px;background:rgb(0,128,0)"></div></body>"#,
        )
        .expect("write child");
        let list = composited(
            &dir,
            r#"<body style="margin:0"><iframe src="child.html" style="width:120px;height:80px;border:0;padding:0;display:block"></iframe></body>"#,
        );
        assert!(
            has_rect(&list, [0.0, 128.0 / 255.0, 0.0]),
            "the child's own paint reaches the parent's list"
        );
        let clipped = list.commands().iter().any(|command| {
            matches!(command, PaintCmd::PushClip(spec)
                if matches!(&spec.kind, ClipKind::Rect(rect)
                    if (rect.width() - 120.0).abs() < 1.0 && (rect.height() - 80.0).abs() < 1.0))
        });
        assert!(clipped, "and it is clipped to the frame's content box");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `srcdoc` child needs no fetch and resolves its relative stylesheet
    /// against the *parent's* base URL.
    #[test]
    fn a_srcdoc_child_uses_the_parents_base_url() {
        let dir = fixture_dir("srcdoc");
        std::fs::write(base_dir(&dir).join("child.css"), "div { background: rgb(0,0,255) }")
            .expect("write css");
        let list = composited(
            &dir,
            r#"<body style="margin:0"><iframe srcdoc="<link rel=stylesheet href=child.css><div style='width:20px;height:20px'></div>" style="width:120px;height:80px;border:0;padding:0;display:block"></iframe></body>"#,
        );
        assert!(
            has_rect(&list, [0.0, 0.0, 1.0]),
            "the srcdoc child's parent-relative stylesheet applied"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A frame whose `src` cannot be read leaves the parent's list intact
    /// rather than failing the render.
    #[test]
    fn a_missing_child_leaves_the_parents_list_alone() {
        let dir = fixture_dir("missing");
        let list = composited(
            &dir,
            r#"<body style="margin:0"><div style="width:10px;height:10px;background:rgb(255,0,0)"></div><iframe src="nope.html" style="width:120px;height:80px;border:0;display:block"></iframe></body>"#,
        );
        assert!(has_rect(&list, [1.0, 0.0, 0.0]), "the parent still paints");
        assert_eq!(list.frame_slots().len(), 0, "the slot was consumed");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A frame that names a document already open in an ancestor stops rather
    /// than recursing.
    #[test]
    fn a_self_referencing_child_stops() {
        let dir = fixture_dir("cycle");
        std::fs::write(
            base_dir(&dir).join("loop.html"),
            r#"<body><iframe src="loop.html" style="width:60px;height:40px"></iframe></body>"#,
        )
        .expect("write loop");
        let list = composited(
            &dir,
            r#"<body style="margin:0"><iframe src="loop.html" style="width:120px;height:80px;border:0;display:block"></iframe></body>"#,
        );
        // The outer child exists; its own self-reference does not recurse.
        assert!(!list.commands().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
