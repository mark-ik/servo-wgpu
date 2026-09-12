// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Paint-list emission for Livery's bounded structural lane.

use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

use layout_dom_api::{LayoutDom, LocalName, Namespace, NodeKind};
use livery::{
    ComputedValues,
    values::{
        BackgroundAttachment, BackgroundBox, BackgroundImage, BackgroundSize,
        BackgroundSizeComponent, BorderCollapse, BorderStyle as CssBorderStyle,
        BoxShadow as CssBoxShadow, ComputedColor, Display, EmptyCells, FontSize, Length,
        LengthPercentage, LengthUnit, Matrix2D, Overflow as CssOverflow, Position, Radius,
        RepeatStyle, Visibility,
    },
};
use paint_list_api::{
    AlphaType, BorderDetails, BorderItem, BorderRadius, BorderSide, BorderStyle, BoxShadowClipMode,
    ClipKind, ClipSpec, ColorF, CommonPlacement, DashPattern, DeviceIntSize, EngineId, ExtendMode,
    ExternalTextureItem, FontResource, GradientStop, IdNamespace, ImageItem, ImageKey,
    ImageRendering, ImageResource, LayerSpec, LayoutPoint, LayoutRect, LayoutSideOffsets,
    LayoutSize, LayoutTransform, LayoutVector2D, LinearGradientItem, LinearGradientPayload,
    NormalBorder, PaintCmd, PaintList, PathCommand, PathData, RectItem, ShadowItem, StrokeCap,
    StrokeItem, StrokeJoin, TransformKind, TransformSpec,
};
use serde::{Deserialize, Serialize};

use crate::{
    LiveryLayout, StylePlane,
    layout::{
        Fragment, TablePaintModel, border_width_px, length_percentage_px, order_modified_children,
        z_index_stacking_level,
    },
    text::{TextFrame, TextSystem},
};
use buckram::{
    BoxId, DisplayInside, FragmentId, GridEdgeOrientation, PhysicalSize, TableBorderStyle,
    TableFragmentRole,
};

/// Genet paint output produced by the Livery CSS/layout path.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveryPaintList {
    viewport: DeviceIntSize,
    generation: u64,
    commands: Vec<PaintCmd>,
    fonts: Vec<FontResource>,
    images: Vec<ImageResource>,
    #[serde(skip)]
    image_keys: HashMap<String, ImageKey>,
    #[serde(skip)]
    image_sources: HashMap<String, Vec<u8>>,
    #[serde(skip)]
    host_leaf_slots: Vec<HostLeafSlot>,
    #[serde(skip)]
    frame_slots: Vec<FrameSlot>,
}

/// A child browsing context's place in the parent document's paint order.
///
/// Recorded while the parent's clips, transforms and stacking context are
/// still live, exactly as [`HostLeafSlot`] is, so a spliced child scene is
/// clipped by the parent's `overflow` and travels under the parent's
/// transforms without either side rewriting a primitive.
///
/// It is a separate list from [`HostLeafSlot`] rather than another
/// `custom-leaf` key because the two key spaces are unrelated: a custom leaf's
/// key is an author attribute, and a frame's is the DOM node's `opaque_id`.
/// Sharing one `u64` space would let a page collide with a frame by writing an
/// integer into an attribute.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameSlot {
    /// `LayoutDom::opaque_id` of the `<iframe>` element.
    pub owner_node: u64,
    /// The element's content box, in the parent document's coordinates. This
    /// is the child's viewport: its origin is where the child's `(0, 0)` goes
    /// and its size is what the child lays out at.
    pub content_rect: LayoutRect,
    command_index: usize,
}

/// A custom leaf's content position in the CSS paint order.
///
/// Livery records this while the DOM paint stack is live. The retained host
/// later replaces the slot with its command stream or fragment, so CSS clips,
/// transforms, and stacking contexts apply to host-painted content too.
#[derive(Clone, Copy, Debug)]
struct HostLeafSlot {
    key: u64,
    command_index: usize,
    origin: LayoutPoint,
}

impl LiveryPaintList {
    pub fn new(viewport: DeviceIntSize, generation: u64) -> Self {
        Self::with_image_sources(viewport, generation, &HashMap::new())
    }

    /// Append a host-owned overlay rectangle after document content.
    ///
    /// Focus, caret, selection, and inspection overlays belong to the render
    /// driver rather than CSS paint emission, but they still travel through the
    /// same engine-neutral paint-list boundary.
    pub fn push_overlay_rect(&mut self, rect: LayoutRect, color: ColorF) {
        self.commands.push(PaintCmd::DrawRect(RectItem {
            placement: CommonPlacement::new(rect),
            color,
        }));
    }

    /// Fill custom-leaf paint slots recorded during the CSS paint walk.
    ///
    /// A leaf's commands are inserted while its ancestors' clips, transforms,
    /// and stacking context remain active. `fragment` takes precedence because
    /// a renderer-owned lowering is the retained form of that same leaf.
    pub fn splice_host_leaf_slots<F, G>(&mut self, mut commands: F, mut fragment: G)
    where
        F: FnMut(u64) -> Option<Vec<PaintCmd>>,
        G: FnMut(u64) -> Option<u64>,
    {
        let slots = std::mem::take(&mut self.host_leaf_slots);
        let mut inserted = 0usize;
        for slot in slots {
            let replacement = if let Some(id) = fragment(slot.key) {
                vec![PaintCmd::PlaceRetainedFragment(
                    paint_list_api::RetainedFragmentRef {
                        id,
                        origin: slot.origin,
                    },
                )]
            } else if let Some(items) = commands(slot.key) {
                if items.is_empty() {
                    continue;
                }
                let mut replacement = Vec::with_capacity(items.len() + 2);
                replacement.push(PaintCmd::PushTransform(TransformSpec {
                    origin: slot.origin,
                    transform: LayoutTransform::identity(),
                    kind: TransformKind::Standard,
                }));
                replacement.extend(items);
                replacement.push(PaintCmd::PopTransform);
                replacement
            } else {
                continue;
            };
            let index = slot.command_index + inserted;
            inserted += replacement.len();
            let added = replacement.len();
            self.commands.splice(index..index, replacement);
            self.shift_slots_after(index, added);
        }
    }

    /// Every child browsing context this frame painted, in paint order.
    ///
    /// A caller reads this *after* the parent frame is laid out and before it
    /// calls [`Self::splice_frame_slots`]: the child's viewport is the
    /// parent's used content box, so the parent must be laid out first.
    pub fn frame_slots(&self) -> &[FrameSlot] {
        &self.frame_slots
    }

    /// Take a child browsing context's commands into this list's resource
    /// tables, returning them with every resource key rewritten to this
    /// list's.
    ///
    /// The two resource kinds behave differently and both had to be checked:
    ///
    /// - **Font keys are content-hashed** (`text::content_key`), so the same
    ///   face has the same key in every list. Merging is a dedupe by key, and
    ///   a face the parent already uses costs nothing.
    /// - **Image keys are per-list ordinals** (`image_key_for` allocates
    ///   `images.len() + 1`), so the child's key 1 and the parent's key 1 are
    ///   different pictures. Every child image is re-keyed here, and every
    ///   command that names one is rewritten. Splicing without this does not
    ///   fail loudly: it draws the *parent's* image inside the frame.
    pub fn absorb_frame_commands(&mut self, child: &LiveryPaintList) -> Vec<PaintCmd> {
        for font in &child.fonts {
            if !self.fonts.iter().any(|existing| existing.key == font.key) {
                self.fonts.push(font.clone());
            }
        }
        let mut remap = HashMap::new();
        for image in &child.images {
            let key = ImageKey::new(IdNamespace(0), self.images.len() as u32 + 1);
            self.images.push(ImageResource {
                key,
                width: image.width,
                height: image.height,
                data: image.data.clone(),
            });
            remap.insert(image.key, key);
        }
        let mut commands = child.commands.clone();
        if !remap.is_empty() {
            for command in &mut commands {
                rewrite_image_keys(command, &remap);
            }
        }
        commands
    }

    /// Fill child-browsing-context slots with each child's own paint commands.
    ///
    /// The child's commands are in *its* document coordinates, so they are
    /// wrapped in a clip to the content box and a translation to its origin.
    /// The clip is what makes the child a viewport rather than a hole: a child
    /// document taller than the frame is cut off at the frame's edge, and its
    /// own scroll offset is a translation the child's paint list already
    /// carries.
    pub fn splice_frame_slots<F>(&mut self, mut child: F)
    where
        F: FnMut(u64) -> Option<Vec<PaintCmd>>,
    {
        let slots = std::mem::take(&mut self.frame_slots);
        let mut inserted = 0usize;
        for slot in slots {
            let Some(commands) = child(slot.owner_node) else {
                continue;
            };
            if commands.is_empty() || slot.content_rect.is_empty() {
                continue;
            }
            let mut replacement = Vec::with_capacity(commands.len() + 4);
            replacement.push(PaintCmd::PushClip(ClipSpec {
                kind: ClipKind::Rect(slot.content_rect),
            }));
            replacement.push(PaintCmd::PushTransform(TransformSpec {
                origin: slot.content_rect.min,
                transform: LayoutTransform::identity(),
                kind: TransformKind::Standard,
            }));
            replacement.extend(commands);
            replacement.push(PaintCmd::PopTransform);
            replacement.push(PaintCmd::PopClip);
            let index = slot.command_index + inserted;
            inserted += replacement.len();
            let added = replacement.len();
            self.commands.splice(index..index, replacement);
            self.shift_slots_after(index, added);
        }
    }

    /// Shift recorded slot positions after a command insertion at `index`.
    ///
    /// Every recorded position is an index into `commands`, so any insertion
    /// before it invalidates it. The two splice methods and the two whole-list
    /// wrappers ([`Self::translated`], [`Self::scaled_to`]) are the only
    /// insertions that can happen while a slot is still outstanding.
    fn shift_slots_after(&mut self, index: usize, by: usize) {
        for slot in &mut self.frame_slots {
            if slot.command_index >= index {
                slot.command_index += by;
            }
        }
        for slot in &mut self.host_leaf_slots {
            if slot.command_index >= index {
                slot.command_index += by;
            }
        }
    }

    /// Splice host-painted commands at a retained document position.
    ///
    /// Custom widget interiors are host content rather than CSS paint. Their
    /// command stream stays local to the leaf and is translated as one stack
    /// frame so neither the host nor Livery rewrites individual primitives.
    pub fn push_host_commands_at(&mut self, origin: LayoutPoint, commands: &[PaintCmd]) {
        if commands.is_empty() {
            return;
        }
        self.commands.push(PaintCmd::PushTransform(TransformSpec {
            origin,
            transform: LayoutTransform::identity(),
            kind: TransformKind::Standard,
        }));
        self.commands.extend_from_slice(commands);
        self.commands.push(PaintCmd::PopTransform);
    }

    /// Place one renderer-retained host fragment at a document position.
    pub fn push_host_fragment_at(&mut self, origin: LayoutPoint, id: u64) {
        self.commands.push(PaintCmd::PlaceRetainedFragment(
            paint_list_api::RetainedFragmentRef { id, origin },
        ));
    }

    fn with_image_sources(
        viewport: DeviceIntSize,
        generation: u64,
        image_sources: &HashMap<String, Vec<u8>>,
    ) -> Self {
        Self {
            viewport,
            generation,
            commands: Vec::new(),
            fonts: Vec::new(),
            images: Vec::new(),
            image_keys: HashMap::new(),
            image_sources: image_sources.clone(),
            host_leaf_slots: Vec::new(),
            frame_slots: Vec::new(),
        }
    }

    fn image_key_for(&mut self, url: &str) -> Option<ImageKey> {
        if let Some(key) = self.image_keys.get(url) {
            return Some(*key);
        }
        let bytes = if let Ok(data_url) = data_url::DataUrl::process(url) {
            data_url.decode_to_vec().ok()?.0
        } else {
            self.image_sources.get(url)?.clone()
        };
        let rgba = image::load_from_memory(&bytes).ok()?.to_rgba8();
        let (width, height) = rgba.dimensions();
        let key = ImageKey::new(IdNamespace(0), self.images.len() as u32 + 1);
        self.images.push(ImageResource {
            key,
            width,
            height,
            data: rgba.into_raw(),
        });
        self.image_keys.insert(url.to_owned(), key);
        Some(key)
    }

    fn image_size(&self, key: ImageKey) -> Option<(f32, f32)> {
        self.images
            .iter()
            .find(|image| image.key == key)
            .map(|image| (image.width as f32, image.height as f32))
    }

    /// Translate an already-emitted frame into the current viewport scroll
    /// coordinate system without rebuilding paint commands.
    pub fn translated(mut self, x: f32, y: f32) -> Self {
        if x == 0.0 && y == 0.0 {
            return self;
        }
        let transform = TransformSpec {
            origin: LayoutPoint::new(0.0, 0.0),
            transform: LayoutTransform::translation(x, y, 0.0),
            kind: TransformKind::Standard,
        };
        self.commands.insert(0, PaintCmd::PushTransform(transform));
        self.commands.push(PaintCmd::PopTransform);
        self.shift_slots_after(0, 1);
        self
    }

    /// Scale an already-emitted frame out of the CSS viewport it was laid out
    /// at and into the larger presentation viewport the host asked for.
    ///
    /// This is the render half of user-agent page zoom: layout ran at
    /// `width / factor` by `height / factor`, so every painted primitive —
    /// including a scroll translation already pushed by [`Self::translated`] —
    /// is composed under one root scale rather than rewritten. The presentation
    /// size is taken rather than derived so the frame keeps the host's exact
    /// pixel size across the rounding that CSS division introduced.
    pub fn scaled_to(mut self, factor: f32, width: u32, height: u32) -> Self {
        self.viewport = DeviceIntSize::new(width as i32, height as i32);
        if factor == 1.0 {
            return self;
        }
        let transform = TransformSpec {
            origin: LayoutPoint::new(0.0, 0.0),
            transform: LayoutTransform::scale(factor, factor, 1.0),
            kind: TransformKind::Standard,
        };
        self.commands.insert(0, PaintCmd::PushTransform(transform));
        self.commands.push(PaintCmd::PopTransform);
        self.shift_slots_after(0, 1);
        self
    }
}

impl PaintList for LiveryPaintList {
    fn engine_id(&self) -> EngineId {
        EngineId::GENET
    }

    fn viewport(&self) -> DeviceIntSize {
        self.viewport
    }

    fn generation_id(&self) -> u64 {
        self.generation
    }

    fn commands(&self) -> &[PaintCmd] {
        &self.commands
    }

    fn fonts(&self) -> &[FontResource] {
        &self.fonts
    }

    fn images(&self) -> &[ImageResource] {
        &self.images
    }
}

/// One-shot convenience path. Retained sessions should use
/// [`emit_paint_list_with_text_system`] so font discovery, shaping scratch
/// space, and font resources survive between frames.
pub fn emit_paint_list<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    viewport: DeviceIntSize,
    generation: u64,
) -> LiveryPaintList
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    emit_paint_list_with_text_system(
        dom,
        styles,
        fragments,
        viewport,
        generation,
        &mut TextSystem::new(),
    )
}

/// Emit structural boxes and shared inline formatting through a retained text
/// system. `generation` is supplied by the document/session owner.
pub fn emit_paint_list_with_text_system<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    viewport: DeviceIntSize,
    generation: u64,
    text: &mut TextSystem,
) -> LiveryPaintList
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    emit_paint_list_with_text_system_scrolled(
        dom,
        styles,
        fragments,
        viewport,
        generation,
        text,
        &HashMap::new(),
    )
}

/// Emit a retained frame with per-element scroll offsets applied to descendant
/// paint. The public convenience path keeps this map empty; retained sessions
/// supply their wheel-owned offsets here.
pub(crate) fn emit_paint_list_with_text_system_scrolled<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    viewport: DeviceIntSize,
    generation: u64,
    text: &mut TextSystem,
    scroll_offsets: &HashMap<D::NodeId, (f32, f32)>,
) -> LiveryPaintList
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    emit_paint_list_with_text_system_scrolled_with_images(
        dom,
        styles,
        fragments,
        viewport,
        generation,
        text,
        scroll_offsets,
        &HashMap::new(),
    )
}

#[allow(clippy::too_many_arguments)]
/// Emit a retained Livery frame with caller-owned shaped text, nested scroll
/// offsets, and image bytes. This is public for a live runtime that owns the
/// DOM outside `LiveryDocument`.
pub fn emit_paint_list_with_text_system_scrolled_with_images<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    viewport: DeviceIntSize,
    generation: u64,
    text: &mut TextSystem,
    scroll_offsets: &HashMap<D::NodeId, (f32, f32)>,
    image_sources: &HashMap<String, Vec<u8>>,
) -> LiveryPaintList
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    emit_paint_list_with_text_system_scrolled_with_images_and_external_textures(
        dom,
        styles,
        fragments,
        viewport,
        generation,
        text,
        scroll_offsets,
        image_sources,
        &HashMap::new(),
    )
}

/// Emit a retained Livery frame and trusted WebGL canvas draws. The map is
/// supplied by the runtime owner, keyed by the canvas DOM node; authored DOM
/// attributes alone never authorize a compositor texture import.
#[allow(clippy::too_many_arguments)]
pub fn emit_paint_list_with_text_system_scrolled_with_images_and_external_textures<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    viewport: DeviceIntSize,
    generation: u64,
    text: &mut TextSystem,
    scroll_offsets: &HashMap<D::NodeId, (f32, f32)>,
    image_sources: &HashMap<String, Vec<u8>>,
    external_textures: &HashMap<D::NodeId, u64>,
) -> LiveryPaintList
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    // C3: consumers receive a numeric view made with the exact retained
    // element scheme and host palette. Keep `styles` contextual for cascade
    // and CSSOM, but never let paint reach the old light-palette fallback.
    let styles = styles.with_used_colors();
    let mut list = LiveryPaintList::with_image_sources(viewport, generation, image_sources);
    let mut text_frame = fragments
        .text_frame()
        .cloned()
        .unwrap_or_else(|| text.begin_frame());
    let mut text_state = PaintText {
        system: text,
        frame: &mut text_frame,
    };
    let canvas_background_source = emit_canvas_background(dom, &styles, fragments, &mut list);
    emit_node(
        dom,
        &styles,
        fragments,
        dom.document(),
        None,
        &mut text_state,
        &mut list,
        scroll_offsets,
        canvas_background_source,
        external_textures,
    );
    list.fonts = text.fonts_for(&text_frame);
    list
}

struct PaintText<'a, Id> {
    system: &'a mut TextSystem,
    frame: &'a mut TextFrame<Id>,
}

#[allow(clippy::too_many_arguments)]
fn emit_node<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    id: D::NodeId,
    inherited: Option<&ComputedValues>,
    text: &mut PaintText<'_, D::NodeId>,
    list: &mut LiveryPaintList,
    scroll_offsets: &HashMap<D::NodeId, (f32, f32)>,
    canvas_background_source: Option<D::NodeId>,
    external_textures: &HashMap<D::NodeId, u64>,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    // K4e4: `transform` and `opacity` anchor to `get`, the node's outermost
    // box. For a table element that is the wrapper, which carries both under
    // CSS Tables 3 section 3.6.1, so the layer and the coordinate space wrap
    // the captions along with the grid.
    let transform = styles
        .get(id)
        .filter(|style| style.display != Display::None && style.visibility == Visibility::Visible)
        .and_then(|style| {
            fragments
                .get(id)
                .and_then(|fragment| transform_spec(style, fragment))
        });
    if let Some(transform) = &transform {
        list.commands
            .push(PaintCmd::PushTransform(transform.clone()));
    }
    let opacity = styles
        .get(id)
        .filter(|style| style.display != Display::None && style.visibility == Visibility::Visible)
        .map(|style| style.opacity.value())
        .filter(|opacity| *opacity < 1.0);
    if let Some(opacity) = opacity {
        list.commands.push(PaintCmd::PushLayer(LayerSpec {
            opacity,
            ..LayerSpec::default()
        }));
    }
    let Some((inherited, clips_descendants)) = begin_node(
        dom,
        styles,
        fragments,
        id,
        PaintScope {
            inherited,
            stacking_roots: None,
            inline_owner: None,
            canvas_background_source,
            external_textures,
        },
        text,
        list,
    ) else {
        return;
    };
    let scroll_transform = scroll_offsets.get(&id).copied().and_then(scroll_spec);
    if let Some(transform) = &scroll_transform {
        list.commands
            .push(PaintCmd::PushTransform(transform.clone()));
    }
    let table = fragments.table_paint_for_node(id);
    if let Some(table) = table {
        emit_table_backgrounds(dom, styles, fragments, table, list);
    }
    let mut deferred_collapsed = table
        .filter(|table| table.is_collapsed())
        .map(DeferredCollapsedBorders::new);
    record_host_leaf_slot(dom, fragments, id, list);
    record_frame_slot(dom, styles, fragments, id, list);
    emit_children_in_stacking_order(
        dom,
        styles,
        fragments,
        id,
        inherited,
        text,
        list,
        scroll_offsets,
        canvas_background_source,
        external_textures,
        deferred_collapsed.as_mut(),
    );
    if let Some(deferred) = deferred_collapsed.as_mut() {
        deferred.flush(styles, fragments, list);
    }
    if scroll_transform.is_some() {
        list.commands.push(PaintCmd::PopTransform);
    }
    for _ in 0..clips_descendants {
        list.commands.push(PaintCmd::PopClip);
    }
    if opacity.is_some() {
        list.commands.push(PaintCmd::PopLayer);
    }
    if transform.is_some() {
        list.commands.push(PaintCmd::PopTransform);
    }
}

fn custom_leaf_key<D>(dom: &D, id: D::NodeId) -> Option<u64>
where
    D: LayoutDom,
{
    if dom.kind(id) != NodeKind::Element
        || !matches!(
            dom.element_name(id)?.local.as_ref(),
            "custom-leaf" | "chisel-leaf"
        )
    {
        return None;
    }
    dom.attribute(id, &Namespace::default(), &LocalName::from("key"))?
        .parse()
        .ok()
}

/// Mark this replaced leaf's place in the current CSS paint phase.
///
/// A positioned leaf arrives through [`emit_node`], while an ordinary block
/// leaf arrives through [`emit_normal_node`]. Both paths must record the same
/// marker: leaving it only in the stacking-context path silently drops the
/// normal-flow leaves used by graph canvases and grid cells.
fn record_host_leaf_slot<D>(
    dom: &D,
    fragments: &LiveryLayout<D::NodeId>,
    id: D::NodeId,
    list: &mut LiveryPaintList,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    if let Some(key) = custom_leaf_key(dom, id)
        && let Some(fragment) = fragments.get(id)
    {
        list.host_leaf_slots.push(HostLeafSlot {
            key,
            command_index: list.commands.len(),
            origin: LayoutPoint::new(fragment.x, fragment.y),
        });
    }
}

/// Point every image key in one command at its absorbed replacement.
fn rewrite_image_keys(command: &mut PaintCmd, remap: &HashMap<ImageKey, ImageKey>) {
    let swap = |key: &mut ImageKey| {
        if let Some(replacement) = remap.get(key) {
            *key = *replacement;
        }
    };
    match command {
        PaintCmd::DrawImage(item) => swap(&mut item.image_key),
        PaintCmd::DrawRepeatingImage(item) => swap(&mut item.image_key),
        PaintCmd::DrawBorder(item) => {
            if let paint_list_api::BorderDetails::NinePatch(nine) = &mut item.details
                && let paint_list_api::NinePatchSource::Image(key, _) = &mut nine.source
            {
                swap(key);
            }
        },
        PaintCmd::PushLayer(spec) => {
            if let Some(mask) = spec.mask.as_mut()
                && let Some(key) = mask.image_mask.as_mut()
            {
                swap(key);
            }
        },
        _ => {},
    }
}

/// Mark an `<iframe>`'s child browsing context in the current paint phase.
///
/// Recorded from the same two call sites as [`record_host_leaf_slot`], for the
/// same reason: a positioned frame arrives through `emit_node` and a
/// normal-flow one through `emit_normal_node`, and a marker left in only one
/// of them silently drops half the frames on a page.
///
/// The recorded rectangle is the element's **content box**, which is both the
/// child's viewport and the clip the composite applies. A frame with no
/// computed style, no fragment, or an empty content box records nothing, so a
/// `display: none` frame costs the composite nothing.
fn record_frame_slot<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    id: D::NodeId,
    list: &mut LiveryPaintList,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    if dom.kind(id) != NodeKind::Element
        || !dom
            .element_name(id)
            .is_some_and(|name| name.local.as_ref().eq_ignore_ascii_case("iframe"))
    {
        return;
    }
    let (Some(style), Some(fragment)) = (styles.get(id), fragments.get(id)) else {
        return;
    };
    let (x, y, width, height) = crate::layout::content_box_rect(style, fragment);
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    list.frame_slots.push(FrameSlot {
        owner_node: dom.opaque_id(id),
        content_rect: LayoutRect::new(
            LayoutPoint::new(x, y),
            LayoutPoint::new(x + width, y + height),
        ),
        command_index: list.commands.len(),
    });
}

/// Flex and grid items are blockified for layout and paint even when their
/// computed outside display was inline. Keep decoration selection aligned with
/// the generated box tree, rather than treating a canvas flex item as an IFC
/// fragment that has no line-owned decoration record.
fn is_blockified_item<Id>(fragments: &LiveryLayout<Id>, id: Id) -> bool
where
    Id: Copy + Eq + Hash,
{
    let boxes = fragments.boxes();
    boxes
        .principal_box(id)
        .and_then(|box_id| boxes[box_id].parent())
        .is_some_and(|parent| {
            matches!(
                boxes[parent].display.inside,
                Some(DisplayInside::Flex | DisplayInside::Grid)
            )
        })
}

fn begin_node<'a, D>(
    dom: &D,
    styles: &'a StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    id: D::NodeId,
    scope: PaintScope<'a, D::NodeId>,
    text: &mut PaintText<'_, D::NodeId>,
    list: &mut LiveryPaintList,
) -> Option<(Option<&'a ComputedValues>, usize)>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let mut clips_descendants = 0;
    let inherited = match dom.kind(id) {
        NodeKind::Element => {
            let style = styles.get(id)?;
            if style.display == Display::None || style.visibility != Visibility::Visible {
                return None;
            }
            // `clip-path` applies to this box's own decoration and content as
            // well as every descendant. Keep it outside the overflow/table
            // clips below so the normal reverse-pop closes the narrow scopes
            // first.
            if let Some(fragment) = fragments
                .get(id)
                .filter(|fragment| paintable_fragment(fragment))
                && let Some(clip) = polygon_clip(style, fragment)
            {
                list.commands.push(PaintCmd::PushClip(clip));
                clips_descendants += 1;
            }
            text.system
                .prepare_inline_children(text.frame, dom, styles, fragments, id, style);
            let background_propagated = scope.canvas_background_source == Some(id);
            let paints_as_inline = matches!(style.display, Display::Inline | Display::InlineBlock)
                && !is_blockified_item(fragments, id);
            if paints_as_inline {
                if !background_propagated && !fragments.table_paint_manages_node(id) {
                    emit_inline_element_decoration(text.frame, fragments, id, style, list);
                }
                emit_inline_replaced_image(dom, text.frame, fragments, id, list);
            } else if let Some(fragment) = fragments
                // K4e4: decorations address the principal box. For a table
                // element that is the grid, which owns background and borders
                // under CSS 2.1 section 17.4 - painting them on the wrapper
                // would wrongly cover the captions.
                .principal_fragment(id)
                .filter(|fragment| paintable_fragment(fragment))
                && !fragments.table_paint_manages_node(id)
            {
                emit_shadow(list, style, fragment);
                if !background_propagated {
                    emit_background(list, style, fragment);
                }
                emit_replaced_image(dom, list, id, style, fragment);
                if !fragments.table_paint_uses_collapsed_borders(id) {
                    emit_border(list, style, fragment);
                }
            }
            emit_canvas_external_texture(dom, fragments, id, style, scope.external_textures, list);
            // The overflow clip stays on the outer box: CSS Tables 3 section
            // 3.6.1 puts `overflow` on the table wrapper box, so a clipping
            // table clips at the box that contains its captions.
            if !paints_as_inline
                && let Some(fragment) = fragments.get(id)
                && let Some(clip) = descendant_clip(style, fragment, list.viewport)
            {
                list.commands.push(PaintCmd::PushClip(clip));
                clips_descendants += 1;
            }
            // CSS Tables 3 clips a cell that crosses a collapsed track at the
            // accepted, post-collapse cell edge. The table layout model marks
            // precisely those cells; a generic overflow rule cannot infer it.
            if fragments.table_cell_requires_clip(id)
                && let Some(fragment) = fragments.principal_fragment(id)
                && paintable_fragment(fragment)
            {
                list.commands.push(PaintCmd::PushClip(ClipSpec {
                    kind: ClipKind::Rect(bounds(fragment)),
                }));
                clips_descendants += 1;
            }
            if style.display == Display::ListItem {
                // A generated marker is shaped against its Buckram pseudo box,
                // then attributed to the owning list-item DOM node. Normal text
                // drains at its text node, but a marker has no DOM text node of
                // its own. Drain this prepared group before child content so an
                // inside marker keeps the pseudo's inline-before-item order.
                text.frame.drain(
                    id,
                    scope.inline_owner,
                    scope.stacking_roots,
                    &mut list.commands,
                );
            }
            Some(style)
        },
        NodeKind::Text => {
            if let (Some(style), Some(value)) = (scope.inherited, dom.text(id)) {
                let first_command = list.commands.len();
                let drained = text.frame.drain(
                    id,
                    scope.inline_owner,
                    scope.stacking_roots,
                    &mut list.commands,
                );
                if !drained
                    && let Some(fragment) = fragments.get(id)
                    && paintable_fragment(fragment)
                {
                    text.system
                        .emit_single(text.frame, value, style, fragment, &mut list.commands);
                }
                if style.background_clip == BackgroundBox::Text
                    && matches!(style.background_image, BackgroundImage::None)
                {
                    let color = resolve_color(&style.background_color);
                    if color.a > 0.0 {
                        for command in &mut list.commands[first_command..] {
                            if let PaintCmd::DrawText(run) = command {
                                run.color = color;
                            }
                        }
                    }
                }
            }
            scope.inherited
        },
        _ => scope.inherited,
    };
    Some((inherited, clips_descendants))
}

fn emit_canvas_external_texture<D>(
    dom: &D,
    fragments: &LiveryLayout<D::NodeId>,
    id: D::NodeId,
    style: &ComputedValues,
    trusted: &HashMap<D::NodeId, u64>,
    list: &mut LiveryPaintList,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    if dom.kind(id) != NodeKind::Element
        || !dom
            .element_name(id)
            .is_some_and(|name| name.local.as_ref().eq_ignore_ascii_case("canvas"))
    {
        return;
    }
    let Some(&texture_key) = trusted.get(&id) else {
        return;
    };
    let marker = dom
        .attribute(
            id,
            &Namespace::default(),
            &LocalName::from("data-genet-external-texture-key"),
        )
        .and_then(|value| value.parse::<u64>().ok());
    if marker != Some(texture_key) {
        return;
    }
    let Some(fragment) = fragments
        .get(id)
        .filter(|fragment| paintable_fragment(fragment))
    else {
        return;
    };
    let (x, y, width, height) = crate::layout::content_box_rect(style, fragment);
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    list.commands
        .push(PaintCmd::DrawExternalTexture(ExternalTextureItem {
            // Canvas pixels paint in the replaced element's content box.
            // Border and padding remain ordinary Livery commands around it.
            placement: CommonPlacement::new(LayoutRect::new(
                LayoutPoint::new(x, y),
                LayoutPoint::new(x + width, y + height),
            )),
            texture_key,
            // `emit_node` already brackets this element in the matching
            // `PushLayer`; adding the same opacity to the source would apply
            // element opacity twice.
            opacity: 1.0,
            content_generation: None,
        }));
}

fn emit_table_backgrounds<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    table: &TablePaintModel,
    list: &mut LiveryPaintList,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    for phase in 0..6 {
        for table_fragment in table.fragments() {
            let belongs_to_phase = match phase {
                0 => table.is_collapsed() && matches!(table_fragment.role, TableFragmentRole::Grid),
                1 => matches!(table_fragment.role, TableFragmentRole::ColumnGroup),
                2 => matches!(table_fragment.role, TableFragmentRole::Column),
                3 => matches!(table_fragment.role, TableFragmentRole::RowGroup(_)),
                4 => matches!(table_fragment.role, TableFragmentRole::Row),
                5 => matches!(table_fragment.role, TableFragmentRole::Cell),
                _ => false,
            };
            if !belongs_to_phase {
                continue;
            }
            let Some(box_id) = table_fragment.box_id else {
                continue;
            };
            let Some(node) = fragments.boxes().origin_node(box_id) else {
                continue;
            };
            let Some(style) = styles.get(node) else {
                continue;
            };
            if style.display == Display::None || style.visibility != Visibility::Visible {
                continue;
            }
            let Some(fragment) = fragments.fragments().fragments_for_box(box_id).next() else {
                continue;
            };
            if !paintable_fragment(fragment) {
                continue;
            }
            let empty_hidden = matches!(table_fragment.role, TableFragmentRole::Cell)
                && style.border_collapse == BorderCollapse::Separate
                && style.empty_cells == EmptyCells::Hide
                && !table_cell_has_visible_content(dom, styles, node, true);
            if empty_hidden {
                continue;
            }
            if matches!(table_fragment.role, TableFragmentRole::Grid) {
                emit_shadow(list, style, fragment);
            }
            emit_background(list, style, fragment);
            // In the separated model `empty-cells: hide` hides the complete
            // cell box. Structural tracks have background layers but no
            // independently painted borders; the table and cell boxes own
            // their borders.
            if table.is_separated() && matches!(table_fragment.role, TableFragmentRole::Cell) {
                emit_border(list, style, fragment);
            }
        }
    }
}

/// Emit K4g5's already-resolved atomic border winners. Logical strip geometry
/// reaches this boundary unchanged; `FlowAxes` performs the one physical
/// conversion against the table fragment, and the paint list keeps fractional
/// CSS pixels rather than device-rounding them.
fn emit_collapsed_table_borders<Id>(
    styles: &StylePlane<Id>,
    fragments: &LiveryLayout<Id>,
    table: &TablePaintModel,
    list: &mut LiveryPaintList,
) where
    Id: Copy + Eq + Hash,
{
    let Some(segments) = table.collapsed_segments() else {
        return;
    };
    let table_box = table
        .collapsed_table()
        .expect("a collapsed table paint model owns its grid box");
    let grid_fragment = fragments
        .fragments()
        .fragments_for_box(table_box)
        .next()
        .expect("a painted collapsed table owns its grid fragment");
    let table_fragment = grid_fragment.id();
    let grid_rect = grid_fragment.physical_rect();
    let flow = grid_fragment.flow();
    let grid_size = PhysicalSize {
        width: grid_rect.width,
        height: grid_rect.height,
    };

    for segment in segments {
        debug_assert_eq!(segment.table, table_box);
        let source = fragments
            .boxes()
            .origin_node(segment.winner)
            .or_else(|| fragments.boxes().origin_node(table_box))
            .expect("a collapsed-border winner or its table originates at a source node");
        let context = styles
            .used_color_context(source)
            .expect("a collapsed-border winner has a retained used color context");
        let color = resolve_color(&ComputedColor::Absolute(
            segment.color.resolve_used(context),
        ));
        let local = flow.physical_rect(segment.rect, grid_size);
        let rect = LayoutRect::new(
            LayoutPoint::new(grid_rect.x + local.x, grid_rect.y + local.y),
            LayoutPoint::new(
                grid_rect.x + local.x + local.width,
                grid_rect.y + local.y + local.height,
            ),
        );
        let segment = FinalCollapsedBorderPaintSegment {
            winner: segment.winner,
            table_fragment,
            rect,
            orientation: segment.edge.orientation,
            style: segment.style,
            color,
        };
        emit_collapsed_border_style(list, &segment);
    }
}

/// One final paint-owned segment. Its `winner` and `table_fragment` make the
/// command's source explicit without teaching the neutral command stream CSS
/// table ownership.
#[derive(Clone, Debug)]
struct FinalCollapsedBorderPaintSegment {
    winner: BoxId,
    table_fragment: FragmentId,
    rect: LayoutRect,
    orientation: GridEdgeOrientation,
    style: TableBorderStyle,
    color: ColorF,
}

fn emit_collapsed_border_style(
    list: &mut LiveryPaintList,
    segment: &FinalCollapsedBorderPaintSegment,
) {
    // Retain the source pair through the exact lowerer that emits commands.
    let _provenance = (segment.winner, segment.table_fragment);
    emit_collapsed_border_style_at(
        list,
        segment.rect,
        segment.orientation,
        segment.style,
        segment.color,
    );
}

fn emit_collapsed_border_style_at(
    list: &mut LiveryPaintList,
    rect: LayoutRect,
    orientation: GridEdgeOrientation,
    style: TableBorderStyle,
    color: ColorF,
) {
    match style {
        TableBorderStyle::Solid => push_rect(list, rect, color),
        TableBorderStyle::Double => {
            let (first, second) = split_cross_axis(rect, orientation, 3.0);
            push_rect(list, first, color);
            push_rect(list, second, color);
        },
        TableBorderStyle::Dashed => {
            push_stroke(
                list,
                rect,
                orientation,
                color,
                StrokeCap::Butt,
                Some(DashPattern {
                    intervals: vec![
                        cross_axis_width(rect, orientation) * 3.0,
                        cross_axis_width(rect, orientation),
                    ],
                    offset: 0.0,
                }),
            );
        },
        TableBorderStyle::Dotted => {
            let width = cross_axis_width(rect, orientation);
            push_stroke(
                list,
                rect,
                orientation,
                color,
                StrokeCap::Round,
                Some(DashPattern {
                    // A near-zero dash plus a round cap makes each dash a
                    // circle without requiring a new neutral primitive.
                    intervals: vec![f32::EPSILON, (width * 2.0).max(f32::EPSILON)],
                    offset: 0.0,
                }),
            );
        },
        TableBorderStyle::Ridge => {
            let (first, second) = split_cross_axis(rect, orientation, 2.0);
            push_rect(list, first, lighten(color));
            push_rect(list, second, darken(color));
        },
        TableBorderStyle::Groove => {
            let (first, second) = split_cross_axis(rect, orientation, 2.0);
            push_rect(list, first, darken(color));
            push_rect(list, second, lighten(color));
        },
        TableBorderStyle::Hidden
        | TableBorderStyle::None
        | TableBorderStyle::Inset
        | TableBorderStyle::Outset => {
            unreachable!("K4g5 removes suppressed styles and maps inset/outset before paint")
        },
    }
}

fn split_cross_axis(
    rect: LayoutRect,
    orientation: GridEdgeOrientation,
    parts: f32,
) -> (LayoutRect, LayoutRect) {
    match orientation {
        GridEdgeOrientation::InlineRunning => {
            let size = (rect.max.y - rect.min.y) / parts;
            (
                LayoutRect::new(rect.min, LayoutPoint::new(rect.max.x, rect.min.y + size)),
                LayoutRect::new(LayoutPoint::new(rect.min.x, rect.max.y - size), rect.max),
            )
        },
        GridEdgeOrientation::BlockRunning => {
            let size = (rect.max.x - rect.min.x) / parts;
            (
                LayoutRect::new(rect.min, LayoutPoint::new(rect.min.x + size, rect.max.y)),
                LayoutRect::new(LayoutPoint::new(rect.max.x - size, rect.min.y), rect.max),
            )
        },
    }
}

fn push_stroke(
    list: &mut LiveryPaintList,
    rect: LayoutRect,
    orientation: GridEdgeOrientation,
    color: ColorF,
    cap: StrokeCap,
    dash: Option<DashPattern>,
) {
    let (start, end) = match orientation {
        GridEdgeOrientation::InlineRunning => (
            LayoutPoint::new(rect.min.x, (rect.min.y + rect.max.y) * 0.5),
            LayoutPoint::new(rect.max.x, (rect.min.y + rect.max.y) * 0.5),
        ),
        GridEdgeOrientation::BlockRunning => (
            LayoutPoint::new((rect.min.x + rect.max.x) * 0.5, rect.min.y),
            LayoutPoint::new((rect.min.x + rect.max.x) * 0.5, rect.max.y),
        ),
    };
    list.commands.push(PaintCmd::DrawStroke(StrokeItem {
        placement: CommonPlacement::new(rect),
        path: PathData {
            commands: vec![PathCommand::MoveTo(start), PathCommand::LineTo(end)],
        },
        color,
        width: cross_axis_width(rect, orientation),
        cap,
        join: StrokeJoin::Miter,
        dash,
    }));
}

fn cross_axis_width(rect: LayoutRect, orientation: GridEdgeOrientation) -> f32 {
    match orientation {
        GridEdgeOrientation::InlineRunning => rect.max.y - rect.min.y,
        GridEdgeOrientation::BlockRunning => rect.max.x - rect.min.x,
    }
}

fn push_rect(list: &mut LiveryPaintList, rect: LayoutRect, color: ColorF) {
    list.commands.push(PaintCmd::DrawRect(RectItem {
        placement: CommonPlacement::new(rect),
        color,
    }));
}

fn lighten(color: ColorF) -> ColorF {
    ColorF::new(
        (color.r + (1.0 - color.r) * 0.33).min(1.0),
        (color.g + (1.0 - color.g) * 0.33).min(1.0),
        (color.b + (1.0 - color.b) * 0.33).min(1.0),
        color.a,
    )
}

fn darken(color: ColorF) -> ColorF {
    ColorF::new(color.r * 0.67, color.g * 0.67, color.b * 0.67, color.a)
}

#[cfg(test)]
mod collapsed_border_style_tests {
    use super::*;

    fn rect() -> LayoutRect {
        LayoutRect::new(LayoutPoint::new(10.0, 20.0), LayoutPoint::new(70.0, 26.0))
    }

    fn commands_for(style: TableBorderStyle) -> Vec<PaintCmd> {
        let mut list = LiveryPaintList::new(DeviceIntSize::new(80, 40), 1);
        emit_collapsed_border_style_at(
            &mut list,
            rect(),
            GridEdgeOrientation::InlineRunning,
            style,
            ColorF::new(0.3, 0.4, 0.5, 1.0),
        );
        list.commands
    }

    #[test]
    fn collapsed_border_styles_lower_to_rectangles_or_styled_strokes() {
        assert!(matches!(
            commands_for(TableBorderStyle::Solid).as_slice(),
            [PaintCmd::DrawRect(_)]
        ));
        assert!(matches!(
            commands_for(TableBorderStyle::Double).as_slice(),
            [PaintCmd::DrawRect(_), PaintCmd::DrawRect(_)]
        ));
        assert!(matches!(
            commands_for(TableBorderStyle::Ridge).as_slice(),
            [PaintCmd::DrawRect(_), PaintCmd::DrawRect(_)]
        ));
        assert!(matches!(
            commands_for(TableBorderStyle::Groove).as_slice(),
            [PaintCmd::DrawRect(_), PaintCmd::DrawRect(_)]
        ));

        let dashed = commands_for(TableBorderStyle::Dashed);
        let [PaintCmd::DrawStroke(dashed)] = dashed.as_slice() else {
            panic!("dashed collapsed borders use the neutral stroke primitive");
        };
        assert_eq!(dashed.cap, StrokeCap::Butt);
        assert_eq!(dashed.width, 6.0);
        assert_eq!(
            dashed.dash.as_ref().map(|dash| dash.intervals.as_slice()),
            Some(&[18.0, 6.0][..])
        );

        let dotted = commands_for(TableBorderStyle::Dotted);
        let [PaintCmd::DrawStroke(dotted)] = dotted.as_slice() else {
            panic!("dotted collapsed borders use round-capped neutral strokes");
        };
        assert_eq!(dotted.cap, StrokeCap::Round);
        assert_eq!(dotted.width, 6.0);
        assert!(dotted.dash.is_some());
    }
}

#[cfg(test)]
mod collapsed_border_paint_tests {
    use super::*;
    use crate::{Device, InteractionStates, LiveryDocument, StyleSet, layout, resolve_styles};
    use genet_static_dom::StaticDocument;

    fn render(html: &str, css: &str) -> LiveryPaintList {
        let document = StaticDocument::parse(html);
        let styles = resolve_styles(
            &document,
            &StyleSet::cambium(&[css]),
            &Device::screen(320.0, 240.0),
            &InteractionStates::default(),
        );
        let fragments = layout(&document, &styles, 320.0, 240.0).expect("collapsed table layout");
        emit_paint_list(
            &document,
            &styles,
            &fragments,
            DeviceIntSize::new(320, 240),
            1,
        )
    }

    /// Reproduce the retained route used by the WPT renderer. The stateless
    /// helper above is still useful for small paint units, but WPT documents
    /// arrive as full HTML trees and shape their preceding text first.
    fn render_wpt(html: &str) -> LiveryPaintList {
        let document = StaticDocument::parse(html);
        let mut session = LiveryDocument::new(
            document,
            StyleSet::cambium(&[]),
            Device::screen(800.0, 600.0),
        );
        session.frame(800, 600).expect("collapsed table WPT layout")
    }

    /// The winning cell's `currentcolor` proves that paint resolves the
    /// retained C3 context from the winner source, rather than table color.
    #[test]
    fn collapsed_table_paints_atomic_winners_once_without_generic_cell_borders() {
        let list = render(
            "<table><tr><td></td><td></td></tr><tr><td></td><td></td></tr></table>",
            "table { display: table; table-layout: fixed; width: 100px; border-collapse: collapse; \
                      color: #0000ff; } \
             tr { display: table-row; height: 20px; } \
             td { display: table-cell; padding: 0; border: 3px solid currentcolor; color: #ff0000; }",
        );
        let red = ColorF::new(1.0, 0.0, 0.0, 1.0);
        let winner_rects = list
            .commands()
            .iter()
            .filter_map(|command| match command {
                PaintCmd::DrawRect(rect) if rect.color == red => Some(rect.placement.bounds),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            winner_rects.len(),
            12,
            "two rows and columns have twelve atomic collapsed-border edges"
        );
        assert!(winner_rects.iter().all(|rect| {
            ((rect.max.x - rect.min.x) - 3.0).abs() < 0.01
                || ((rect.max.y - rect.min.y) - 3.0).abs() < 0.01
        }));
        assert!(
            !list
                .commands()
                .iter()
                .any(|command| matches!(command, PaintCmd::DrawBorder(_))),
            "collapsed cells and the table must not fall through to generic borders"
        );
    }

    #[test]
    fn hidden_collapsed_winner_suppresses_its_atomic_edge() {
        let list = render(
            "<table><tr><td></td></tr></table>",
            "table { display: table; table-layout: fixed; width: 100px; border-collapse: collapse; } \
             tr { display: table-row; height: 20px; } \
             td { display: table-cell; padding: 0; border: 3px solid #ff0000; border-top: hidden; }",
        );
        let red = ColorF::new(1.0, 0.0, 0.0, 1.0);
        let winner_rects = list
            .commands()
            .iter()
            .filter(|command| matches!(command, PaintCmd::DrawRect(rect) if rect.color == red))
            .count();

        assert_eq!(
            winner_rects, 3,
            "the hidden top winner paints no replacement"
        );
        assert!(
            !list
                .commands()
                .iter()
                .any(|command| matches!(command, PaintCmd::DrawBorder(_))),
            "the hidden winner does not reactivate generic cell border paint"
        );
    }

    /// The inner foreground overflows its 50px parent in
    /// `collapsed-border-paint-phase-001`. The right collapsed edge remains
    /// beneath that foreground instead of escaping as a visible strip.
    #[test]
    fn collapsed_outer_edge_stays_inside_the_cell_foreground_coverage() {
        let list = render_wpt(
            r#"<!DOCTYPE html>
<p>Test passes if there is a filled green square and <strong>no red</strong>.</p>
<table style="border-collapse: collapse; border-spacing: 0;">
  <td style="border-right: 50px solid red; padding: 0;">
    <div style="width: 50px; line-height: 0;">
      <div style="display: inline-block; width: 100px; height: 100px; background: green;"></div>
    </div>
  </td>
</table>"#,
        );
        let red = ColorF::new(1.0, 0.0, 0.0, 1.0);
        let green = ColorF::new(0.0, 128.0 / 255.0, 0.0, 1.0);
        let mut red_rects = Vec::new();
        let mut green_index = None;
        let mut last_red_index = None;
        for (index, command) in list.commands().iter().enumerate() {
            let PaintCmd::DrawRect(rect) = command else {
                continue;
            };
            if rect.color == red {
                red_rects.push(rect.placement.bounds);
                last_red_index = Some(index);
            }
            if rect.color == green {
                green_index = Some(index);
            }
        }
        let green_index = green_index.expect("the cell foreground paints");
        let last_red_index = last_red_index.expect("the collapsed edge paints");
        assert!(
            last_red_index < green_index,
            "collapsed borders stay in the table background phase: {last_red_index} >= {green_index}"
        );
        let green_rect = match &list.commands()[green_index] {
            PaintCmd::DrawRect(rect) => rect.placement.bounds,
            _ => unreachable!("green index names a rectangle"),
        };
        assert!(
            red_rects.iter().all(|rect| {
                rect.min.x >= green_rect.min.x
                    && rect.max.x <= green_rect.max.x
                    && rect.min.y >= green_rect.min.y
                    && rect.max.y <= green_rect.max.y
            }),
            "the phase-001 foreground covers every collapsed outer strip: {red_rects:?} vs {green_rect:?}"
        );
    }

    /// `collapsed-border-paint-phase-002` has block foreground content. Its
    /// collapsed outer edge paints after that content, so the winning border
    /// covers the whole overflowing red rectangle.
    #[test]
    fn collapsed_outer_edge_covers_block_foreground_in_the_later_table_phase() {
        let list = render_wpt(
            r#"<!DOCTYPE html>
<p>Test passes if there is a filled green square and <strong>no red</strong>.</p>
<table style="border-collapse: collapse; border-spacing: 0;">
  <td style="border-right: solid 100px green; height: 100px; padding: 0;">
    <div style="width: 0;">
      <div style="width: 100px; height: 100px; background: red;"></div>
    </div>
  </td>
</table>"#,
        );
        let red = ColorF::new(1.0, 0.0, 0.0, 1.0);
        let green = ColorF::new(0.0, 128.0 / 255.0, 0.0, 1.0);
        let mut red_index = None;
        let mut green_rects = Vec::new();
        let mut last_green_index = None;
        for (index, command) in list.commands().iter().enumerate() {
            let PaintCmd::DrawRect(rect) = command else {
                continue;
            };
            if rect.color == red {
                red_index = Some(index);
            }
            if rect.color == green {
                green_rects.push(rect.placement.bounds);
                last_green_index = Some(index);
            }
        }
        let red_index = red_index.expect("the overflowing block foreground paints");
        let last_green_index = last_green_index.expect("the collapsed edge paints");
        assert!(
            red_index < last_green_index,
            "the phase-002 border follows block foreground content: {red_index} >= {last_green_index}"
        );
        let red_rect = match &list.commands()[red_index] {
            PaintCmd::DrawRect(rect) => rect.placement.bounds,
            _ => unreachable!("red index names a rectangle"),
        };
        assert!(
            green_rects.iter().any(|rect| {
                rect.min.x <= red_rect.min.x
                    && rect.max.x >= red_rect.max.x
                    && rect.min.y <= red_rect.min.y
                    && rect.max.y >= red_rect.max.y
            }),
            "the phase-002 collapsed edge covers the foreground: {green_rects:?} vs {red_rect:?}"
        );
    }
}

#[cfg(test)]
mod frame_slot_tests {
    use super::*;
    use crate::{Device, LiveryDocument, StyleSet};
    use genet_static_dom::StaticDocument;

    fn render(html: &str) -> LiveryPaintList {
        let document = StaticDocument::parse(html);
        let mut session = LiveryDocument::new(
            document,
            StyleSet::cambium(&[]),
            Device::screen(800.0, 600.0),
        );
        session.frame(800, 600).expect("frame layout")
    }

    /// The user-agent default: 300x150 content, inside the 2px inset border
    /// the user-agent sheet gives an `<iframe>`. That the *content* box is
    /// recorded (not the 304x154 border box) is the whole point — the child's
    /// viewport is its content box.
    #[test]
    fn a_default_frame_records_a_three_hundred_by_one_fifty_content_box() {
        let list = render(r#"<!DOCTYPE html><iframe></iframe>"#);
        let slots = list.frame_slots();
        assert_eq!(slots.len(), 1, "one slot per frame: {slots:?}");
        let rect = slots[0].content_rect;
        assert!(
            (rect.width() - 300.0).abs() < 0.5 && (rect.height() - 150.0).abs() < 0.5,
            "default object size reaches the slot: {rect:?}"
        );
    }

    /// The two axes are independent because a frame has no natural ratio: a
    /// specified width does not transfer to the height.
    #[test]
    fn a_specified_width_does_not_transfer_to_the_frames_height() {
        let list = render(
            r#"<!DOCTYPE html><iframe style="width: 600px; border: 0; padding: 0"></iframe>"#,
        );
        let rect = list.frame_slots()[0].content_rect;
        assert!((rect.width() - 600.0).abs() < 0.5, "{rect:?}");
        assert!(
            (rect.height() - 150.0).abs() < 0.5,
            "no natural ratio, so the height stays at the default: {rect:?}"
        );
    }

    /// Borders and padding are removed, and the recorded origin is the content
    /// box's, not the fragment's.
    #[test]
    fn the_slot_is_inset_by_the_frames_own_border_and_padding() {
        let list = render(
            r#"<!DOCTYPE html><body style="margin: 0">
               <iframe style="width: 100px; height: 80px; border: 5px solid black;
                              padding: 10px; box-sizing: border-box"></iframe></body>"#,
        );
        let rect = list.frame_slots()[0].content_rect;
        assert!(
            (rect.width() - 70.0).abs() < 0.5,
            "100 - 2*(5+10): {rect:?}"
        );
        assert!(
            (rect.height() - 50.0).abs() < 0.5,
            "80 - 2*(5+10): {rect:?}"
        );
        assert!(
            rect.min.x >= 15.0 && rect.min.y >= 15.0,
            "the origin moves inside the border and padding: {rect:?}"
        );
    }

    /// A `display: none` frame has no fragment, so it records no slot and the
    /// composite costs nothing.
    #[test]
    fn a_display_none_frame_records_no_slot() {
        let list = render(r#"<!DOCTYPE html><iframe style="display: none"></iframe>"#);
        assert!(list.frame_slots().is_empty());
    }

    /// Fallback children never become boxes, so nothing of them paints.
    #[test]
    fn frame_fallback_content_generates_no_boxes() {
        let list = render(
            r#"<!DOCTYPE html><iframe><span style="background: rgb(255, 0, 0)">fallback</span></iframe>"#,
        );
        assert_eq!(list.frame_slots().len(), 1);
        let red = ColorF::new(1.0, 0.0, 0.0, 1.0);
        assert!(
            !list.commands().iter().any(|command| matches!(
                command,
                PaintCmd::DrawRect(rect) if rect.color == red
            )),
            "the fallback subtree paints nothing"
        );
    }

    /// The splice puts the child's commands inside a clip to the content box
    /// and under a translation to its origin, and leaves the parent's own
    /// commands in order around them.
    #[test]
    fn splicing_a_child_clips_and_translates_it_into_the_content_box() {
        let mut list = render(
            r#"<!DOCTYPE html><body style="margin: 0">
               <iframe style="width: 200px; height: 100px; border: 0; padding: 0"></iframe>
               <div style="background: rgb(0, 0, 255); width: 10px; height: 10px"></div></body>"#,
        );
        let owner = list.frame_slots()[0].owner_node;
        let child_marker = ColorF::new(0.0, 1.0, 0.0, 1.0);
        let before = list.commands().len();
        list.splice_frame_slots(|node| {
            (node == owner).then(|| {
                vec![PaintCmd::DrawRect(RectItem {
                    placement: CommonPlacement::new(LayoutRect::new(
                        LayoutPoint::new(0.0, 0.0),
                        LayoutPoint::new(400.0, 400.0),
                    )),
                    color: child_marker,
                })]
            })
        });
        assert_eq!(
            list.commands().len(),
            before + 5,
            "clip, transform, one draw, pop, pop"
        );
        let index = list
            .commands()
            .iter()
            .position(
                |command| matches!(command, PaintCmd::DrawRect(rect) if rect.color == child_marker),
            )
            .expect("the child's command is spliced in");
        let clip = match &list.commands()[index - 2] {
            PaintCmd::PushClip(spec) => &spec.kind,
            other => panic!("expected a clip before the child: {other:?}"),
        };
        match clip {
            ClipKind::Rect(rect) => {
                assert!((rect.width() - 200.0).abs() < 0.5, "{rect:?}");
                assert!((rect.height() - 100.0).abs() < 0.5, "{rect:?}");
            },
            other => panic!("the frame clip is a plain rectangle: {other:?}"),
        }
        assert!(matches!(
            &list.commands()[index - 1],
            PaintCmd::PushTransform(_)
        ));
        assert!(matches!(
            &list.commands()[index + 1],
            PaintCmd::PopTransform
        ));
        assert!(matches!(&list.commands()[index + 2], PaintCmd::PopClip));
        // A second splice finds no slots left, so nothing doubles.
        let after = list.commands().len();
        list.splice_frame_slots(|_| Some(vec![PaintCmd::PopClip]));
        assert_eq!(list.commands().len(), after);
    }

    /// A frame with no child scene leaves the parent's command stream
    /// untouched, so an unfetched or lazy frame is a hole, not a panic.
    #[test]
    fn a_frame_with_no_child_scene_splices_nothing() {
        let mut list = render(r#"<!DOCTYPE html><iframe></iframe>"#);
        let before = list.commands().len();
        list.splice_frame_slots(|_| None);
        assert_eq!(list.commands().len(), before);
    }

    /// `translated` and `scaled_to` each insert one command at the head, so a
    /// slot recorded before them must move with it or the child lands in the
    /// wrong place.
    #[test]
    fn wrapping_the_list_keeps_the_slot_pointing_at_the_same_command() {
        let list = render(r#"<!DOCTYPE html><p>x</p><iframe></iframe>"#);
        let original = list.frame_slots()[0].command_index;
        let mut wrapped = list.translated(0.0, -40.0).scaled_to(2.0, 1600, 1200);
        assert_eq!(wrapped.frame_slots()[0].command_index, original + 2);
        let marker = ColorF::new(0.0, 1.0, 0.0, 1.0);
        wrapped.splice_frame_slots(|_| {
            Some(vec![PaintCmd::DrawRect(RectItem {
                placement: CommonPlacement::new(LayoutRect::new(
                    LayoutPoint::new(0.0, 0.0),
                    LayoutPoint::new(1.0, 1.0),
                )),
                color: marker,
            })])
        });
        let index = wrapped
            .commands()
            .iter()
            .position(|command| matches!(command, PaintCmd::DrawRect(rect) if rect.color == marker))
            .expect("the child still splices in");
        // The two wrappers' `PushTransform`s are commands 0 and 1; the child
        // must land after both of them, inside the scroll and zoom frames.
        assert!(index > 2, "the child stays inside both wrappers: {index}");
    }
}

#[cfg(test)]
mod positioned_paint_tests {
    use super::*;
    use crate::{Device, InteractionStates, StyleSet, layout, resolve_styles};
    use genet_static_dom::StaticDocument;

    fn render(html: &str, css: &str) -> LiveryPaintList {
        let document = StaticDocument::parse(html);
        let styles = resolve_styles(
            &document,
            &StyleSet::cambium(&[css]),
            &Device::screen(320.0, 240.0),
            &InteractionStates::default(),
        );
        let fragments = layout(&document, &styles, 320.0, 240.0).expect("positioned layout");
        emit_paint_list(
            &document,
            &styles,
            &fragments,
            DeviceIntSize::new(320, 240),
            1,
        )
    }

    fn first_rect(list: &LiveryPaintList, color: ColorF) -> usize {
        list.commands()
            .iter()
            .position(|command| matches!(command, PaintCmd::DrawRect(rect) if rect.color == color))
            .expect("the fixture color paints")
    }

    #[test]
    fn custom_leaf_stays_below_a_later_stacking_overlay_and_inside_its_clip() {
        let document = StaticDocument::parse(
            "<div id=canvas><custom-leaf key=7></custom-leaf><div id=card></div></div>",
        );
        let styles = resolve_styles(
            &document,
            &StyleSet::cambium(&["html, body, div { margin: 0; padding: 0; } \
                 #canvas { position: relative; width: 80px; height: 80px; overflow: hidden; } \
                 custom-leaf { display: block; width: 120px; height: 120px; } \
                 #card { position: absolute; left: 10px; top: 10px; width: 50px; height: 50px; \
                         z-index: 4; background: #ff0000; }"]),
            &Device::screen(320.0, 240.0),
            &InteractionStates::default(),
        );
        let fragments = layout(&document, &styles, 320.0, 240.0).expect("leaf fixture layout");
        let mut list = emit_paint_list(
            &document,
            &styles,
            &fragments,
            DeviceIntSize::new(320, 240),
            1,
        );
        let blue = ColorF::new(0.0, 0.0, 1.0, 1.0);
        list.splice_host_leaf_slots(
            |key| {
                assert_eq!(key, 7);
                Some(vec![PaintCmd::DrawRect(RectItem {
                    placement: CommonPlacement::new(LayoutRect::new(
                        LayoutPoint::new(0.0, 0.0),
                        LayoutPoint::new(120.0, 120.0),
                    )),
                    color: blue,
                })])
            },
            |_| None,
        );

        let leaf = first_rect(&list, blue);
        let card = first_rect(&list, ColorF::new(1.0, 0.0, 0.0, 1.0));
        let clip_start = list.commands()[..leaf]
            .iter()
            .rposition(|command| matches!(command, PaintCmd::PushClip(_)))
            .expect("the canvas clip encloses its custom leaf");
        let clip_end = list.commands()[leaf + 1..]
            .iter()
            .position(|command| matches!(command, PaintCmd::PopClip))
            .map(|index| leaf + index + 1)
            .expect("the canvas clip closes after its custom leaf");

        assert!(
            clip_start < leaf && leaf < clip_end,
            "the leaf stays in the canvas overflow stack"
        );
        assert!(
            leaf < card,
            "a custom leaf paints in its DOM stacking phase before the later card overlay"
        );
    }

    #[test]
    fn absolute_card_subtree_translates_its_shaped_text_with_its_fragment() {
        let list = render(
            "<div id=canvas><div id=layer><div id=card><div id=editor>Card controls</div></div></div></div>",
            "html, body, div { margin: 0; padding: 0; } \
             #canvas { position: relative; width: 520px; height: 260px; } \
             #layer { position: absolute; left: 100px; top: 60px; width: 150px; height: 160px; } \
             #card { position: absolute; left: 0; top: 0; width: 100%; height: 100%; \
                     box-sizing: border-box; overflow: hidden; padding: 10px; } \
             #editor { display: flex; flex-wrap: wrap; }",
        );
        let run = list
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCmd::DrawText(run) if !run.glyphs.is_empty() => Some(run),
                _ => None,
            })
            .expect("the card editor's text paints");
        let glyph = &run.glyphs[0];

        assert!(
            glyph.point.x >= 110.0 && glyph.point.y >= 70.0,
            "the shaped text follows the absolute card's 100px, 60px translation: {:?}",
            glyph.point
        );
    }

    #[test]
    fn relative_subtree_translates_its_shaped_text_with_its_fragment() {
        let list = render(
            "<div id=relative>Moved text</div>",
            "html, body, div { margin: 0; padding: 0; } \
             #relative { position: relative; left: 80px; top: 50px; width: 120px; }",
        );
        let run = list
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCmd::DrawText(run) if !run.glyphs.is_empty() => Some(run),
                _ => None,
            })
            .expect("the relative box's text paints");
        let glyph = &run.glyphs[0];

        assert!(
            glyph.point.x >= 80.0 && glyph.point.y >= 50.0,
            "the shaped text follows the relative box's 80px, 50px translation: {:?}",
            glyph.point
        );
    }

    #[test]
    fn positioned_numeric_z_indices_wrap_the_normal_paint_phase() {
        let list = render(
            "<div id=host><div id=behind></div><div id=normal></div><div id=front></div></div>",
            "html, body, div { margin: 0; padding: 0; } \
             #host { position: relative; width: 100px; height: 100px; } \
             #behind, #front { position: absolute; left: 0; top: 0; width: 80px; height: 80px; } \
             #behind { z-index: -1; background: #f00; } \
             #normal { width: 80px; height: 80px; background: #00f; } \
             #front { z-index: 1; background: #0f0; }",
        );
        let behind = first_rect(&list, ColorF::new(1.0, 0.0, 0.0, 1.0));
        let normal = first_rect(&list, ColorF::new(0.0, 0.0, 1.0, 1.0));
        let front = first_rect(&list, ColorF::new(0.0, 1.0, 0.0, 1.0));

        assert!(
            behind < normal && normal < front,
            "negative positioned content paints before normal flow and positive content after it: {behind}, {normal}, {front}"
        );
    }

    #[test]
    fn static_grid_item_z_indices_wrap_the_normal_paint_phase() {
        let list = render(
            "<div id=grid><div id=front></div><div id=normal></div><div id=behind></div></div>",
            "html, body, div { margin: 0; padding: 0; } \
             #grid { display: grid; width: 80px; height: 80px; \
                     grid-template-columns: 80px; grid-template-rows: 80px; } \
             #behind, #normal, #front { grid-area: 1 / 1 / 2 / 2; width: 80px; height: 80px; } \
             #behind { z-index: -1; background: #f00; } \
             #normal { background: #00f; } \
             #front { z-index: 1; background: #0f0; }",
        );
        let behind = first_rect(&list, ColorF::new(1.0, 0.0, 0.0, 1.0));
        let normal = first_rect(&list, ColorF::new(0.0, 0.0, 1.0, 1.0));
        let front = first_rect(&list, ColorF::new(0.0, 1.0, 0.0, 1.0));

        assert!(
            behind < normal && normal < front,
            "negative static grid items paint before normal items and positive items after them: {behind}, {normal}, {front}"
        );
    }

    #[test]
    fn static_flex_item_z_indices_wrap_the_normal_paint_phase() {
        let list = render(
            "<div id=flex><div id=front></div><div id=normal></div><div id=behind></div></div>",
            "html, body, div { margin: 0; padding: 0; } \
             #flex { display: flex; flex-direction: column; width: 80px; height: 80px; } \
             #behind, #normal, #front { width: 80px; height: 80px; flex-shrink: 0; margin-bottom: -80px; } \
             #behind { z-index: -1; background: #f00; } \
             #normal { background: #00f; } \
             #front { z-index: 1; background: #0f0; }",
        );
        let behind = first_rect(&list, ColorF::new(1.0, 0.0, 0.0, 1.0));
        let normal = first_rect(&list, ColorF::new(0.0, 0.0, 1.0, 1.0));
        let front = first_rect(&list, ColorF::new(0.0, 1.0, 0.0, 1.0));

        assert!(
            behind < normal && normal < front,
            "negative static flex items paint before normal items and positive items after them: {behind}, {normal}, {front}"
        );
    }

    #[test]
    fn grid_items_paint_in_order_modified_document_order() {
        let list = render(
            "<div id=grid><div id=later></div><div id=earlier></div></div>",
            "html, body, div { margin: 0; padding: 0; } \
             #grid { display: grid; width: 80px; height: 80px; \
                     grid-template-columns: 80px; grid-template-rows: 80px; } \
             #later, #earlier { grid-area: 1 / 1 / 2 / 2; width: 80px; height: 80px; } \
             #later { order: 1; background: #0f0; } \
             #earlier { order: -1; background: #f00; }",
        );
        let earlier = first_rect(&list, ColorF::new(1.0, 0.0, 0.0, 1.0));
        let later = first_rect(&list, ColorF::new(0.0, 1.0, 0.0, 1.0));

        assert!(
            earlier < later,
            "grid item order changes normal-phase paint order: {earlier}, {later}"
        );
    }

    #[test]
    fn flex_items_paint_in_order_modified_document_order() {
        let list = render(
            "<div id=flex><div id=later></div><div id=earlier></div></div>",
            "html, body, div { margin: 0; padding: 0; } \
             #flex { display: flex; flex-direction: column; width: 80px; height: 80px; } \
             #later, #earlier { width: 80px; height: 80px; flex-shrink: 0; margin-bottom: -80px; } \
             #later { order: 1; background: #0f0; } \
             #earlier { order: -1; background: #f00; }",
        );
        let earlier = first_rect(&list, ColorF::new(1.0, 0.0, 0.0, 1.0));
        let later = first_rect(&list, ColorF::new(0.0, 1.0, 0.0, 1.0));

        assert!(
            earlier < later,
            "flex item order changes normal-phase paint order: {earlier}, {later}"
        );
    }

    #[test]
    fn positioned_stacking_item_keeps_its_overflow_clip() {
        let list = render(
            "<div id=clip><div id=overlay></div></div>",
            "html, body, div { margin: 0; padding: 0; } \
             #clip { position: relative; width: 50px; height: 50px; overflow: hidden; } \
             #overlay { position: absolute; left: 0; top: 0; width: 100px; height: 100px; \
                        z-index: 1; background: #f00; }",
        );
        let overlay = first_rect(&list, ColorF::new(1.0, 0.0, 0.0, 1.0));
        let push = list.commands()[..overlay]
            .iter()
            .rposition(|command| matches!(command, PaintCmd::PushClip(_)))
            .expect("the overflow clip encloses the positioned paint");
        let pop = list.commands()[overlay + 1..]
            .iter()
            .position(|command| matches!(command, PaintCmd::PopClip))
            .map(|index| overlay + index + 1)
            .expect("the positioned paint closes the overflow clip");

        assert!(push < overlay && overlay < pop);
    }

    /// A pane of absolutely positioned children under one `overflow: hidden`
    /// ancestor opens a fixed number of clip scopes, not one per child.
    ///
    /// The regression: every flattened stacking item re-pushed its whole
    /// ancestor clip stack, so a 24x24 isometric board emitted ~700 clip
    /// scopes for a single `overflow: hidden` pane. A clip scope is a
    /// compositing layer downstream and the frame came back empty. The
    /// sibling panel painted before the pane must stay outside them.
    #[test]
    fn one_overflow_scope_wraps_a_run_of_flattened_items() {
        fn pane_of(tiles: usize) -> LiveryPaintList {
            let mut html = String::from("<div id=app><div id=side></div><div id=pane>");
            for index in 0..tiles {
                html.push_str(&format!(
                    "<div class=tile style=\"z-index: {index}\"></div>"
                ));
            }
            html.push_str("</div></div>");
            render(
                &html,
                "html, body, div { margin: 0; padding: 0; }                  #app { display: flex; width: 320px; height: 240px; }                  #side { width: 80px; height: 100%; background: #00f; }                  #pane { position: relative; flex: 1; overflow: hidden; background: #0f0; }                  .tile { position: absolute; left: 4px; top: 4px; width: 8px; height: 8px;                          background: #f00; }",
            )
        }
        fn clips(list: &LiveryPaintList) -> (usize, usize) {
            list.commands()
                .iter()
                .fold((0, 0), |(push, pop), command| match command {
                    PaintCmd::PushClip(_) => (push + 1, pop),
                    PaintCmd::PopClip => (push, pop + 1),
                    _ => (push, pop),
                })
        }

        let few = pane_of(4);
        let many = pane_of(64);
        assert_eq!(
            clips(&few).0,
            clips(&few).1,
            "the clip stack stays balanced"
        );
        assert_eq!(
            clips(&many).0,
            clips(&many).1,
            "the clip stack stays balanced"
        );
        assert_eq!(
            clips(&many).0,
            clips(&few).0,
            "clip scopes count runs of flattened items, not the items in them"
        );

        // The run's scope is the pane's padding box, and it holds every tile.
        let push = many
            .commands()
            .iter()
            .rposition(|command| matches!(command, PaintCmd::PushClip(_)))
            .expect("the flattened run opens a scope");
        let pop = many.commands()[push..]
            .iter()
            .position(|command| matches!(command, PaintCmd::PopClip))
            .map(|index| push + index)
            .expect("the flattened run closes its scope");
        let PaintCmd::PushClip(ClipSpec {
            kind: ClipKind::Rect(rect),
        }) = &many.commands()[push]
        else {
            panic!("an `overflow: hidden` pane clips to a rectangle");
        };
        assert_eq!(
            (rect.min.x, rect.min.y, rect.max.x, rect.max.y),
            (80.0, 0.0, 320.0, 240.0),
            "the clip is the pane's padding box in viewport coordinates"
        );
        let red = ColorF::new(1.0, 0.0, 0.0, 1.0);
        let tiles = many.commands()[push..pop]
            .iter()
            .filter(|command| matches!(command, PaintCmd::DrawRect(rect) if rect.color == red))
            .count();
        assert_eq!(tiles, 64, "every tile paints inside the one scope");
        assert!(
            first_rect(&many, ColorF::new(0.0, 0.0, 1.0, 1.0)) < push,
            "the sibling panel paints before the run opens its clip"
        );
    }
}

/// A blank `<td>` is not inferred from its rectangle. Text, replacement
/// content, and visible descendant decoration make it non-empty; whitespace
/// and empty inline wrappers do not.
fn table_cell_has_visible_content<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    id: D::NodeId,
    is_root: bool,
) -> bool
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    match dom.kind(id) {
        // A CDATA section is character data like a text node.
        NodeKind::Text | NodeKind::CdataSection => {
            dom.text(id).is_some_and(|text| !text.trim().is_empty())
        },
        NodeKind::Element => {
            let Some(style) = styles.get(id) else {
                return false;
            };
            if style.display == Display::None || style.visibility != Visibility::Visible {
                return false;
            }
            let replaced = dom.element_name(id).is_some_and(|name| {
                matches!(
                    name.local.as_ref(),
                    "audio"
                        | "canvas"
                        | "embed"
                        | "iframe"
                        | "img"
                        | "input"
                        | "object"
                        | "svg"
                        | "video"
                )
            });
            if replaced || (!is_root && has_visible_box_decoration(style)) {
                return true;
            }
            dom.flat_children(id)
                .any(|child| table_cell_has_visible_content(dom, styles, child, false))
        },
        NodeKind::Document
        | NodeKind::DocumentFragment
        | NodeKind::ShadowRoot
        | NodeKind::Comment => dom
            .flat_children(id)
            .any(|child| table_cell_has_visible_content(dom, styles, child, false)),
        NodeKind::Doctype | NodeKind::ProcessingInstruction => false,
    }
}

fn has_visible_box_decoration(style: &ComputedValues) -> bool {
    if has_background(style) || matches!(style.box_shadow, CssBoxShadow::Value(_)) {
        return true;
    }
    let em = used_font_size(style);
    [
        (style.border_top_style, style.border_top_width),
        (style.border_right_style, style.border_right_width),
        (style.border_bottom_style, style.border_bottom_width),
        (style.border_left_style, style.border_left_width),
    ]
    .into_iter()
    .any(|(border_style, width)| border_width_px(border_style, width, em) > 0.0)
}

struct StackingItem<Id> {
    id: Id,
    level: i32,
    // Flattening moves the subtree outside these ancestors' normal paint
    // walk, so their overflow clips must travel with it.
    ancestor_clips: Vec<ClipSpec>,
}

#[derive(Clone, Copy)]
struct PaintScope<'a, Id> {
    inherited: Option<&'a ComputedValues>,
    stacking_roots: Option<&'a HashSet<Id>>,
    inline_owner: Option<Id>,
    canvas_background_source: Option<Id>,
    external_textures: &'a HashMap<Id, u64>,
}

/// A collapsed table's border phase sits between block descendant backgrounds
/// and inline foreground. The structural table fragments have no DOM walk of
/// their own, so the phase stays explicit instead of becoming an incidental
/// child traversal order.
struct DeferredCollapsedBorders<'a> {
    table: &'a TablePaintModel,
    emitted: bool,
}

impl<'a> DeferredCollapsedBorders<'a> {
    fn new(table: &'a TablePaintModel) -> Self {
        Self {
            table,
            emitted: false,
        }
    }

    fn flush<Id>(
        &mut self,
        styles: &StylePlane<Id>,
        fragments: &LiveryLayout<Id>,
        list: &mut LiveryPaintList,
    ) where
        Id: Copy + Eq + Hash,
    {
        if !self.emitted {
            emit_collapsed_table_borders(styles, fragments, self.table, list);
            self.emitted = true;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_children_in_stacking_order<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    parent: D::NodeId,
    inherited: Option<&ComputedValues>,
    text: &mut PaintText<'_, D::NodeId>,
    list: &mut LiveryPaintList,
    scroll_offsets: &HashMap<D::NodeId, (f32, f32)>,
    canvas_background_source: Option<D::NodeId>,
    external_textures: &HashMap<D::NodeId, u64>,
    mut deferred_collapsed: Option<&mut DeferredCollapsedBorders<'_>>,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let mut items = Vec::new();
    collect_stacking_items(
        dom,
        styles,
        fragments,
        parent,
        list.viewport,
        &mut Vec::new(),
        &mut items,
    );
    items.sort_by_key(|item| item.level);
    let roots = items.iter().map(|item| item.id).collect::<HashSet<_>>();

    if !items.is_empty()
        && let Some(deferred) = deferred_collapsed.as_deref_mut()
    {
        deferred.flush(styles, fragments, list);
    }
    emit_stacking_items(
        dom,
        styles,
        fragments,
        items.iter().filter(|item| item.level < 0),
        text,
        list,
        scroll_offsets,
        canvas_background_source,
        external_textures,
    );

    emit_normal_children(
        dom,
        styles,
        fragments,
        parent,
        PaintScope {
            inherited,
            stacking_roots: Some(&roots),
            inline_owner: styles
                .get(parent)
                .filter(|style| style.display == Display::Inline)
                .map(|_| parent),
            canvas_background_source,
            external_textures,
        },
        text,
        list,
        scroll_offsets,
        deferred_collapsed,
    );

    emit_stacking_items(
        dom,
        styles,
        fragments,
        items.iter().filter(|item| item.level >= 0),
        text,
        list,
        scroll_offsets,
        canvas_background_source,
        external_textures,
    );
}

fn collect_stacking_items<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    parent: D::NodeId,
    viewport: DeviceIntSize,
    ancestor_clips: &mut Vec<ClipSpec>,
    items: &mut Vec<StackingItem<D::NodeId>>,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    for child in order_modified_children(dom, styles, parent) {
        // A numeric positioned or flex/grid item starts a local context. Its
        // descendants are collected when that context is emitted, keeping it
        // atomic here.
        if let Some(level) = stacking_level(dom, styles, child) {
            items.push(StackingItem {
                id: child,
                level,
                ancestor_clips: ancestor_clips.clone(),
            });
            continue;
        }

        let added_clip = match dom.kind(child) {
            NodeKind::Element => {
                let Some(style) = styles.get(child) else {
                    continue;
                };
                if style.display == Display::None {
                    continue;
                }
                if matches!(style.display, Display::Inline | Display::InlineBlock) {
                    None
                } else {
                    fragments
                        .get(child)
                        .and_then(|fragment| descendant_clip(style, fragment, viewport))
                }
            },
            NodeKind::Text => continue,
            _ => None,
        };
        let pushed_clip = added_clip.is_some();
        if let Some(clip) = added_clip {
            ancestor_clips.push(clip);
        }
        collect_stacking_items(
            dom,
            styles,
            fragments,
            child,
            viewport,
            ancestor_clips,
            items,
        );
        if pushed_clip {
            ancestor_clips.pop();
        }
    }
}

/// Emit one z-order phase of flattened stacking items, holding each run of
/// items that flattened out of the same ancestors inside a single clip scope.
///
/// A clip scope is a compositing group in every renderer this list reaches, so
/// re-pushing an identical stack per item is not free bookkeeping: a board of a
/// few hundred `position: absolute` children under one `overflow: hidden`
/// ancestor emitted one clip layer each, and a compositor whose per-frame
/// allocation is sized against the layer count drew nothing at all. Coalescing
/// is sound because nothing paints between two adjacent items of a phase, and
/// a clip is an intersection: one scope around the run means what a scope
/// around each member meant.
#[allow(clippy::too_many_arguments)]
fn emit_stacking_items<'items, D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    items: impl Iterator<Item = &'items StackingItem<D::NodeId>>,
    text: &mut PaintText<'_, D::NodeId>,
    list: &mut LiveryPaintList,
    scroll_offsets: &HashMap<D::NodeId, (f32, f32)>,
    canvas_background_source: Option<D::NodeId>,
    external_textures: &HashMap<D::NodeId, u64>,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash + 'items,
{
    let mut open: Option<&'items [ClipSpec]> = None;
    for item in items {
        let reuse = open.is_some_and(|current| clip_stacks_match(current, &item.ancestor_clips));
        if !reuse {
            if let Some(current) = open {
                for _ in current {
                    list.commands.push(PaintCmd::PopClip);
                }
            }
            for clip in &item.ancestor_clips {
                list.commands.push(PaintCmd::PushClip(clip.clone()));
            }
            open = Some(&item.ancestor_clips);
        }
        emit_node(
            dom,
            styles,
            fragments,
            item.id,
            None,
            text,
            list,
            scroll_offsets,
            canvas_background_source,
            external_textures,
        );
    }
    if let Some(current) = open {
        for _ in current {
            list.commands.push(PaintCmd::PopClip);
        }
    }
}

/// Whether two flattened items carry the same ancestor clip stack, and may
/// therefore share one scope. Deliberately conservative: only rectangular
/// clips compare, so a rounded or path clip simply opens its own scope rather
/// than risking a wrong match on geometry this does not compare.
fn clip_stacks_match(left: &[ClipSpec], right: &[ClipSpec]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| match (&left.kind, &right.kind) {
                (ClipKind::Rect(left), ClipKind::Rect(right)) => left == right,
                _ => false,
            })
}

#[allow(clippy::too_many_arguments)]
fn emit_normal_node<'a, D>(
    dom: &D,
    styles: &'a StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    id: D::NodeId,
    scope: PaintScope<'a, D::NodeId>,
    text: &mut PaintText<'_, D::NodeId>,
    list: &mut LiveryPaintList,
    scroll_offsets: &HashMap<D::NodeId, (f32, f32)>,
    mut deferred_collapsed: Option<&mut DeferredCollapsedBorders<'_>>,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    if scope
        .stacking_roots
        .is_some_and(|roots| roots.contains(&id))
    {
        return;
    }
    let Some((inherited, clips_descendants)) =
        begin_node(dom, styles, fragments, id, scope, text, list)
    else {
        return;
    };
    let scroll_transform = scroll_offsets.get(&id).copied().and_then(scroll_spec);
    if let Some(transform) = &scroll_transform {
        list.commands
            .push(PaintCmd::PushTransform(transform.clone()));
    }
    record_host_leaf_slot(dom, fragments, id, list);
    record_frame_slot(dom, styles, fragments, id, list);
    if let Some(table) = fragments.table_paint_for_node(id) {
        if let Some(deferred) = deferred_collapsed.as_deref_mut() {
            deferred.flush(styles, fragments, list);
        }
        emit_table_backgrounds(dom, styles, fragments, table, list);
        let mut nested_deferred = table
            .is_collapsed()
            .then(|| DeferredCollapsedBorders::new(table));
        emit_normal_children(
            dom,
            styles,
            fragments,
            id,
            PaintScope { inherited, ..scope },
            text,
            list,
            scroll_offsets,
            nested_deferred.as_mut(),
        );
        if let Some(deferred) = nested_deferred.as_mut() {
            deferred.flush(styles, fragments, list);
        }
    } else {
        emit_normal_children(
            dom,
            styles,
            fragments,
            id,
            PaintScope { inherited, ..scope },
            text,
            list,
            scroll_offsets,
            deferred_collapsed,
        );
    }
    if scroll_transform.is_some() {
        list.commands.push(PaintCmd::PopTransform);
    }
    for _ in 0..clips_descendants {
        list.commands.push(PaintCmd::PopClip);
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_normal_children<'a, D>(
    dom: &D,
    styles: &'a StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    parent: D::NodeId,
    scope: PaintScope<'a, D::NodeId>,
    text: &mut PaintText<'_, D::NodeId>,
    list: &mut LiveryPaintList,
    scroll_offsets: &HashMap<D::NodeId, (f32, f32)>,
    mut deferred_collapsed: Option<&mut DeferredCollapsedBorders<'_>>,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let child_ids = order_modified_children(dom, styles, parent);

    let mut inline_group = Vec::new();
    for (index, child) in child_ids.iter().copied().enumerate() {
        if scope
            .stacking_roots
            .is_some_and(|roots| roots.contains(&child))
        {
            continue;
        }
        if is_inline_node(dom, styles, child) {
            inline_group.push(child);
            continue;
        }
        emit_inline_group(
            dom,
            styles,
            fragments,
            &inline_group,
            scope,
            text,
            list,
            scroll_offsets,
            deferred_collapsed.as_deref_mut(),
        );
        inline_group.clear();
        if positioned_inline_overlay_is_covered(dom, styles, fragments, &child_ids, index, child) {
            continue;
        }
        emit_normal_node(
            dom,
            styles,
            fragments,
            child,
            scope,
            text,
            list,
            scroll_offsets,
            deferred_collapsed.as_deref_mut(),
        );
    }
    emit_inline_group(
        dom,
        styles,
        fragments,
        &inline_group,
        scope,
        text,
        list,
        scroll_offsets,
        deferred_collapsed,
    );
}

fn positioned_inline_overlay_is_covered<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    siblings: &[D::NodeId],
    index: usize,
    child: D::NodeId,
) -> bool
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    // The retained text backend blends glyph edges.  When a later positioned
    // inline occupies the same rectangle, keeping the earlier text underneath
    // leaks its color through those anti-aliased edge pixels.  CSS paints the
    // later overlay as the visible result, so drop the covered earlier run.
    let Some(style) = styles.get(child) else {
        return false;
    };
    if !matches!(style.position, Position::Absolute | Position::Fixed) {
        return false;
    }
    // This shortcut exists only for overlapping positioned inline text runs.
    // A table is a structural paint root: its text and column backgrounds must
    // remain available for normal stacking and glyph-edge compositing.
    if fragments.table_paint_for_node(child).is_some() {
        return false;
    }
    if subtree_has_visible_box_decoration(dom, styles, child) {
        return false;
    }
    let Some(fragment) = fragments.get(child) else {
        return false;
    };
    siblings[index.saturating_add(1)..].iter().any(|later| {
        let Some(later_style) = styles.get(*later) else {
            return false;
        };
        if later_style.display == Display::None
            || !matches!(later_style.position, Position::Absolute | Position::Fixed)
            || !has_text_descendant(dom, *later)
        {
            return false;
        }
        let Some(later_fragment) = fragments.get(*later) else {
            return false;
        };
        same_fragment(fragment, later_fragment)
    })
}

fn subtree_has_visible_box_decoration<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    id: D::NodeId,
) -> bool
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    if dom.kind(id) == NodeKind::Element {
        let Some(style) = styles.get(id) else {
            return false;
        };
        if style.display == Display::None || style.visibility != Visibility::Visible {
            return false;
        }
        if has_visible_box_decoration(style) {
            return true;
        }
    }
    dom.flat_children(id)
        .any(|child| subtree_has_visible_box_decoration(dom, styles, child))
}

fn has_text_descendant<D>(dom: &D, id: D::NodeId) -> bool
where
    D: LayoutDom,
    D::NodeId: Copy,
{
    dom.kind(id) == NodeKind::Text && dom.text(id).is_some_and(|text| !text.trim().is_empty())
        || dom
            .flat_children(id)
            .any(|child| has_text_descendant(dom, child))
}

fn same_fragment(left: &Fragment, right: &Fragment) -> bool {
    (left.x - right.x).abs() <= 0.5
        && (left.y - right.y).abs() <= 0.5
        && (left.width - right.width).abs() <= 0.5
        && (left.height - right.height).abs() <= 0.5
}

fn stacking_level<D>(dom: &D, styles: &StylePlane<D::NodeId>, id: D::NodeId) -> Option<i32>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let style = styles.get(id)?;
    if let Some(level) = z_index_stacking_level(dom, styles, id) {
        return Some(level);
    }
    (style.opacity.value() < 1.0
        || establishes_transform_context(style)
        || !style.clip_path.is_none())
    .then_some(0)
}

fn establishes_transform_context(style: &ComputedValues) -> bool {
    style.display != Display::Inline
        && (!style.transform.is_none()
            || style.rotate.radians().is_some()
            || style.scale.factor().is_some())
}

fn scroll_spec(offset: (f32, f32)) -> Option<TransformSpec> {
    if offset.0 == 0.0 && offset.1 == 0.0 {
        return None;
    }
    Some(TransformSpec {
        origin: LayoutPoint::new(0.0, 0.0),
        transform: LayoutTransform::translation(-offset.0, -offset.1, 0.0),
        kind: TransformKind::Standard,
    })
}

fn transform_spec(style: &ComputedValues, fragment: &Fragment) -> Option<TransformSpec> {
    if !establishes_transform_context(style) {
        return None;
    }
    let em = used_font_size(style);
    let mut matrix = Matrix2D::IDENTITY;
    if let Some(angle) = style.rotate.radians() {
        let (sin, cos) = angle.sin_cos();
        matrix = matrix.multiply(Matrix2D::new(cos, sin, -sin, cos, 0.0, 0.0));
    }
    if let Some(factor) = style.scale.factor() {
        matrix = matrix.multiply(Matrix2D::new(factor, 0.0, 0.0, factor, 0.0, 0.0));
    }
    if let Some(transform) = style
        .transform
        .to_matrix(em, (fragment.width, fragment.height))
    {
        matrix = matrix.multiply(transform);
    }
    let authored = LayoutTransform::new(
        matrix.a, matrix.b, 0.0, 0.0, matrix.c, matrix.d, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, matrix.e,
        matrix.f, 0.0, 1.0,
    );

    let origin = LayoutPoint::new(
        fragment.x + fragment.width / 2.0,
        fragment.y + fragment.height / 2.0,
    );
    let transform = LayoutTransform::translation(-origin.x, -origin.y, 0.0).then(&authored);
    Some(TransformSpec {
        origin,
        transform,
        kind: TransformKind::Standard,
    })
}

fn descendant_clip(
    style: &ComputedValues,
    fragment: &Fragment,
    viewport: DeviceIntSize,
) -> Option<ClipSpec> {
    let clips_x = clips_overflow(style.overflow_x);
    let clips_y = clips_overflow(style.overflow_y);
    if !clips_x && !clips_y {
        return None;
    }
    let em = used_font_size(style);
    let left = border_width_px(style.border_left_style, style.border_left_width, em);
    let right = border_width_px(style.border_right_style, style.border_right_width, em);
    let top = border_width_px(style.border_top_style, style.border_top_width, em);
    let bottom = border_width_px(style.border_bottom_style, style.border_bottom_width, em);
    let min_x = if clips_x { fragment.x + left } else { 0.0 };
    let max_x = if clips_x {
        (fragment.x + fragment.width - right).max(min_x)
    } else {
        viewport.width as f32
    };
    let min_y = if clips_y { fragment.y + top } else { 0.0 };
    let max_y = if clips_y {
        (fragment.y + fragment.height - bottom).max(min_y)
    } else {
        viewport.height as f32
    };
    Some(ClipSpec {
        kind: ClipKind::Rect(LayoutRect::new(
            LayoutPoint::new(min_x, min_y),
            LayoutPoint::new(max_x, max_y),
        )),
    })
}

fn polygon_clip(style: &ComputedValues, fragment: &Fragment) -> Option<ClipSpec> {
    let points = style
        .clip_path
        .polygon_points(fragment.width, fragment.height)?;
    let (first, rest) = points.split_first()?;
    let mut commands = Vec::with_capacity(points.len() + 1);
    commands.push(PathCommand::MoveTo(LayoutPoint::new(
        fragment.x + first.0,
        fragment.y + first.1,
    )));
    commands.extend(
        rest.iter()
            .map(|(x, y)| PathCommand::LineTo(LayoutPoint::new(fragment.x + x, fragment.y + y))),
    );
    commands.push(PathCommand::Close);
    Some(ClipSpec {
        kind: ClipKind::Path(PathData { commands }),
    })
}

fn clips_overflow(overflow: CssOverflow) -> bool {
    overflow != CssOverflow::Visible
}

#[allow(clippy::too_many_arguments)]
fn emit_inline_group<'a, D>(
    dom: &D,
    styles: &'a StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    roots: &[D::NodeId],
    scope: PaintScope<'a, D::NodeId>,
    text: &mut PaintText<'_, D::NodeId>,
    list: &mut LiveryPaintList,
    scroll_offsets: &HashMap<D::NodeId, (f32, f32)>,
    mut deferred_collapsed: Option<&mut DeferredCollapsedBorders<'_>>,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    if roots.is_empty() {
        return;
    }
    if inline_group_enters_foreground(dom, fragments, roots)
        && let Some(deferred) = deferred_collapsed.as_deref_mut()
    {
        deferred.flush(styles, fragments, list);
    }
    if let Some(first_line) = roots
        .iter()
        .filter_map(|root| text.frame.first_inline_line(*root))
        .min_by(|left, right| left.total_cmp(right))
    {
        for root in roots {
            emit_inline_descendant_decorations(
                dom,
                styles,
                fragments,
                *root,
                scope.stacking_roots,
                first_line,
                text.frame,
                list,
            );
        }
    }
    for root in roots {
        emit_normal_node(
            dom,
            styles,
            fragments,
            *root,
            scope,
            text,
            list,
            scroll_offsets,
            deferred_collapsed.as_deref_mut(),
        );
    }
}

/// Table fixup retains row and cell boxes in the structural paint model, even
/// where the DOM-side cascade has not supplied their table display keyword.
/// They can therefore arrive in an inline group. Those structural nodes and
/// their formatting whitespace must not advance the collapsed-border phase;
/// the first real inline foreground descendant does.
fn inline_group_enters_foreground<D>(
    dom: &D,
    fragments: &LiveryLayout<D::NodeId>,
    roots: &[D::NodeId],
) -> bool
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    roots.iter().copied().any(|root| {
        if fragments.table_paint_manages_node(root) {
            return false;
        }
        match dom.kind(root) {
            NodeKind::Text => dom.text(root).is_some_and(|text| !text.trim().is_empty()),
            NodeKind::Element => true,
            _ => false,
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn emit_inline_descendant_decorations<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    id: D::NodeId,
    stacking_roots: Option<&HashSet<D::NodeId>>,
    first_line: f32,
    frame: &mut TextFrame<D::NodeId>,
    list: &mut LiveryPaintList,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    if stacking_roots.is_some_and(|roots| roots.contains(&id))
        || frame
            .first_inline_line(id)
            .is_some_and(|line| (line - first_line).abs() > 0.5)
    {
        return;
    }
    let NodeKind::Element = dom.kind(id) else {
        return;
    };
    let Some(style) = styles.get(id) else {
        return;
    };
    if style.display == Display::None {
        return;
    }
    emit_inline_element_decoration(frame, fragments, id, style, list);
    if style.display == Display::Inline {
        for child in dom.flat_children(id) {
            if is_inline_node(dom, styles, child) {
                emit_inline_descendant_decorations(
                    dom,
                    styles,
                    fragments,
                    child,
                    stacking_roots,
                    first_line,
                    frame,
                    list,
                );
            }
        }
    }
}

fn emit_inline_element_decoration<Id>(
    frame: &mut TextFrame<Id>,
    fragments: &LiveryLayout<Id>,
    id: Id,
    style: &ComputedValues,
    list: &mut LiveryPaintList,
) where
    Id: Copy + Eq + Hash,
{
    let paintable = frame
        .inline_fragments(id)
        .map(|inline_fragments| {
            inline_fragments
                .iter()
                .copied()
                .filter(paintable_fragment)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !paintable.is_empty() {
        if !frame.mark_decoration_painted(id) {
            return;
        }
        let last = paintable.len().saturating_sub(1);
        for (index, fragment) in paintable.iter().enumerate() {
            emit_shadow(list, style, fragment);
            emit_background(list, style, fragment);
            emit_inline_border(list, style, fragment, index == 0, index == last);
        }
    } else if style.display == Display::InlineBlock
        && let Some(fragment) = fragments
            .get(id)
            .filter(|fragment| paintable_fragment(fragment))
        && frame.mark_decoration_painted(id)
    {
        emit_shadow(list, style, fragment);
        emit_background(list, style, fragment);
        emit_border(list, style, fragment);
    }
}

fn emit_inline_replaced_image<D>(
    dom: &D,
    frame: &TextFrame<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    id: D::NodeId,
    list: &mut LiveryPaintList,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let Some(url) = replaced_image_url(dom, id) else {
        return;
    };
    let Some(image_key) = list.image_key_for(&url) else {
        return;
    };
    let paintable = frame
        .inline_fragments(id)
        .map(|fragments| fragments.to_vec())
        .or_else(|| {
            fragments
                .get(id)
                .map(|fragment| vec![fragment.physical_rect()])
        })
        .unwrap_or_default();
    for fragment in paintable
        .iter()
        .filter(|fragment| paintable_fragment(fragment))
    {
        list.commands.push(PaintCmd::DrawImage(ImageItem {
            placement: CommonPlacement::new(bounds(fragment)),
            image_key,
            image_rendering: ImageRendering::Auto,
            alpha_type: AlphaType::Alpha,
            color: ColorF::WHITE,
        }));
    }
}

fn emit_replaced_image<D>(
    dom: &D,
    list: &mut LiveryPaintList,
    id: D::NodeId,
    style: &ComputedValues,
    fragment: &Fragment,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let Some(url) = replaced_image_url(dom, id) else {
        return;
    };
    let Some(image_key) = list.image_key_for(&url) else {
        return;
    };
    let radius = border_radius(style, fragment);
    if !radius.is_zero() {
        list.commands.push(PaintCmd::PushClip(ClipSpec {
            kind: ClipKind::RoundedRect {
                rect: bounds(fragment),
                radius,
                clip_out: false,
            },
        }));
    }
    list.commands.push(PaintCmd::DrawImage(ImageItem {
        placement: CommonPlacement::new(bounds(fragment)),
        image_key,
        image_rendering: ImageRendering::Auto,
        alpha_type: AlphaType::Alpha,
        color: ColorF::WHITE,
    }));
    if !radius.is_zero() {
        list.commands.push(PaintCmd::PopClip);
    }
}

fn replaced_image_url<D>(dom: &D, id: D::NodeId) -> Option<String>
where
    D: LayoutDom,
    D::NodeId: Copy,
{
    if dom.kind(id) != NodeKind::Element
        || !dom
            .element_name(id)
            .is_some_and(|name| name.local.as_ref().eq_ignore_ascii_case("img"))
    {
        return None;
    }
    dom.attributes(id).find_map(|attribute| {
        (attribute.name.ns.as_ref().is_empty()
            && attribute.name.local.as_ref().eq_ignore_ascii_case("src"))
        .then(|| attribute.value.to_owned())
    })
}

fn is_inline_node<D>(dom: &D, styles: &StylePlane<D::NodeId>, id: D::NodeId) -> bool
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    match dom.kind(id) {
        NodeKind::Text => true,
        NodeKind::Element => styles.get(id).is_some_and(|style| {
            matches!(style.display, Display::Inline | Display::InlineBlock)
                && !matches!(style.position, Position::Absolute | Position::Fixed)
                && !(style.display == Display::Inline
                    && dom.flat_children(id).any(|child| {
                        !is_inline_node(dom, styles, child)
                            && !styles
                                .get(child)
                                .is_some_and(|child_style| child_style.display == Display::None)
                    }))
        }),
        _ => false,
    }
}

fn paintable_fragment(fragment: &Fragment) -> bool {
    fragment.width.is_finite()
        && fragment.height.is_finite()
        && fragment.width > 0.0
        && fragment.height > 0.0
}

pub(crate) fn bounds(fragment: &Fragment) -> LayoutRect {
    LayoutRect::new(
        LayoutPoint::new(fragment.x, fragment.y),
        LayoutPoint::new(fragment.x + fragment.width, fragment.y + fragment.height),
    )
}

/// Paint the CSS canvas background before the document tree and return the
/// element whose used background becomes transparent.
///
/// CSS Backgrounds 3 section 2.11 selects the document element, except that
/// an HTML document with a transparent, image-free `html` background takes the
/// first `body` child's background instead. The painting area is the viewport;
/// image sizing and positioning still use the root element's box.
fn emit_canvas_background<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    list: &mut LiveryPaintList,
) -> Option<D::NodeId>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let mut elements = dom
        .dom_children(dom.document())
        .filter(|child| dom.kind(*child) == NodeKind::Element);
    let root = elements.next()?;
    if elements.next().is_some() {
        return None;
    }

    let root_style = styles.get(root)?;
    // CSS Backgrounds 3 section 2.11: if the element selected as the
    // canvas-background source has `display: none`, the canvas background is
    // transparent. There is no generated box whose absence may be replaced by
    // the viewport here.
    if root_style.display == Display::None {
        return None;
    }
    let root_is_html = dom
        .element_name(root)
        .is_some_and(|name| name.local.as_ref().eq_ignore_ascii_case("html"));
    let (source, source_style) = if has_background(root_style) {
        (root, root_style)
    } else if root_is_html {
        let body = dom.dom_children(root).find(|child| {
            dom.kind(*child) == NodeKind::Element
                && dom
                    .element_name(*child)
                    .is_some_and(|name| name.local.as_ref().eq_ignore_ascii_case("body"))
        })?;
        let body_style = styles.get(body)?;
        if root_style.contain.is_active()
            || body_style.contain.is_active()
            || !generates_visible_box(body_style)
            || !has_background(body_style)
        {
            return None;
        }
        (body, body_style)
    } else {
        return None;
    };

    let canvas = Fragment {
        x: 0.0,
        y: 0.0,
        width: list.viewport.width.max(0) as f32,
        height: list.viewport.height.max(0) as f32,
    };
    let positioning = fragments
        .get(root)
        .map(|fragment| fragment.physical_rect())
        .unwrap_or(canvas);
    let canvas_rect = bounds(&canvas);
    let positioning = bounds(&positioning);
    let positioning = if source_style.background_attachment == BackgroundAttachment::Fixed {
        canvas_rect
    } else {
        background_box_rect(source_style, positioning, source_style.background_origin)
    };
    let color = resolve_color(&source_style.background_color);
    if color.a > 0.0 {
        list.commands.push(PaintCmd::DrawRect(RectItem {
            placement: CommonPlacement::new(bounds(&canvas)),
            color,
        }));
    }
    emit_background_image_in(list, source_style, positioning, canvas_rect);
    Some(source)
}

fn generates_visible_box(style: &ComputedValues) -> bool {
    !matches!(style.display, Display::None | Display::Contents)
        && style.visibility == Visibility::Visible
}

fn has_background(style: &ComputedValues) -> bool {
    !matches!(style.background_image, BackgroundImage::None)
        || resolve_color(&style.background_color).a > 0.0
}

fn emit_background(list: &mut LiveryPaintList, style: &ComputedValues, fragment: &Fragment) {
    if style.background_clip == BackgroundBox::Text {
        return;
    }
    let color = resolve_color(&style.background_color);
    let radius = border_radius(style, fragment);
    let has_image = !matches!(&style.background_image, BackgroundImage::None);
    if color.a <= 0.0 && !has_image {
        return;
    }
    if !radius.is_zero() {
        list.commands.push(PaintCmd::PushClip(ClipSpec {
            kind: ClipKind::RoundedRect {
                rect: bounds(fragment),
                radius,
                clip_out: false,
            },
        }));
    }
    let painting_rect = background_box_rect(style, bounds(fragment), style.background_clip);
    if style.background_clip != BackgroundBox::BorderBox {
        list.commands.push(PaintCmd::PushClip(ClipSpec {
            kind: ClipKind::Rect(painting_rect),
        }));
    }
    if color.a > 0.0 && !painting_rect.is_empty() {
        list.commands.push(PaintCmd::DrawRect(RectItem {
            placement: CommonPlacement::new(painting_rect),
            color,
        }));
    }
    emit_background_image_in(
        list,
        style,
        if style.background_attachment == BackgroundAttachment::Fixed {
            LayoutRect::new(
                LayoutPoint::zero(),
                LayoutPoint::new(list.viewport.width as f32, list.viewport.height as f32),
            )
        } else {
            background_box_rect(style, bounds(fragment), style.background_origin)
        },
        painting_rect,
    );
    if style.background_clip != BackgroundBox::BorderBox {
        list.commands.push(PaintCmd::PopClip);
    }
    if !radius.is_zero() {
        list.commands.push(PaintCmd::PopClip);
    }
}

fn emit_background_image_in(
    list: &mut LiveryPaintList,
    style: &ComputedValues,
    positioning_rect: LayoutRect,
    painting_rect: LayoutRect,
) {
    match &style.background_image {
        BackgroundImage::None => {},
        BackgroundImage::LinearGradient { from, to } => {
            let em = used_font_size(style);
            let (mut tile_width, mut tile_height) =
                used_gradient_size(style.background_size, positioning_rect.size(), em);
            if tile_width <= 0.0 || tile_height <= 0.0 {
                return;
            }
            if style.background_repeat.x == RepeatStyle::Round {
                tile_width = rounded_tile_size(positioning_rect.width(), tile_width);
            }
            if style.background_repeat.y == RepeatStyle::Round {
                tile_height = rounded_tile_size(positioning_rect.height(), tile_height);
            }
            let offset_x = resolve_length_percentage(
                style.background_position.x,
                positioning_rect.width() - tile_width,
                em,
            );
            let offset_y = resolve_length_percentage(
                style.background_position.y,
                positioning_rect.height() - tile_height,
                em,
            );
            let xs = background_axis_tiles(
                positioning_rect.min.x,
                positioning_rect.width(),
                painting_rect.min.x,
                painting_rect.max.x,
                tile_width,
                offset_x,
                style.background_repeat.x,
            );
            let ys = background_axis_tiles(
                positioning_rect.min.y,
                positioning_rect.height(),
                painting_rect.min.y,
                painting_rect.max.y,
                tile_height,
                offset_y,
                style.background_repeat.y,
            );
            list.commands.push(PaintCmd::PushClip(ClipSpec {
                kind: ClipKind::Rect(painting_rect),
            }));
            for x in xs {
                for &y in &ys {
                    let tile = LayoutRect::new(
                        LayoutPoint::new(x, y),
                        LayoutPoint::new(x + tile_width, y + tile_height),
                    );
                    list.commands
                        .push(PaintCmd::DrawLinearGradient(LinearGradientItem {
                            placement: CommonPlacement::new(tile),
                            gradient: LinearGradientPayload {
                                start_point: LayoutPoint::new(x + tile_width * 0.5, y),
                                end_point: LayoutPoint::new(x + tile_width * 0.5, y + tile_height),
                                extend_mode: ExtendMode::Clamp,
                                stops: vec![
                                    GradientStop {
                                        offset: 0.0,
                                        color: resolve_color(from),
                                    },
                                    GradientStop {
                                        offset: 1.0,
                                        color: resolve_color(to),
                                    },
                                ],
                            },
                            tile_size: tile.size(),
                            tile_spacing: LayoutSize::zero(),
                        }));
                }
            }
            list.commands.push(PaintCmd::PopClip);
        },
        BackgroundImage::Url(url) => {
            let Some(image_key) = list.image_key_for(url) else {
                return;
            };
            let Some((image_width, image_height)) = list.image_size(image_key) else {
                return;
            };
            let em = used_font_size(style);
            let (mut tile_width, mut tile_height) = used_background_size(
                style.background_size,
                positioning_rect.size(),
                image_width,
                image_height,
                em,
            );
            if tile_width <= 0.0 || tile_height <= 0.0 {
                return;
            }
            if style.background_repeat.x == RepeatStyle::Round {
                tile_width = rounded_tile_size(positioning_rect.width(), tile_width);
            }
            if style.background_repeat.y == RepeatStyle::Round {
                tile_height = rounded_tile_size(positioning_rect.height(), tile_height);
            }
            let offset_x = resolve_length_percentage(
                style.background_position.x,
                positioning_rect.size().width - tile_width,
                em,
            );
            let offset_y = resolve_length_percentage(
                style.background_position.y,
                positioning_rect.size().height - tile_height,
                em,
            );
            let xs = background_axis_tiles(
                positioning_rect.min.x,
                positioning_rect.width(),
                painting_rect.min.x,
                painting_rect.max.x,
                tile_width,
                offset_x,
                style.background_repeat.x,
            );
            let ys = background_axis_tiles(
                positioning_rect.min.y,
                positioning_rect.height(),
                painting_rect.min.y,
                painting_rect.max.y,
                tile_height,
                offset_y,
                style.background_repeat.y,
            );
            list.commands.push(PaintCmd::PushClip(ClipSpec {
                kind: ClipKind::Rect(painting_rect),
            }));
            for x in xs {
                for &y in &ys {
                    let placement = LayoutRect::new(
                        LayoutPoint::new(x, y),
                        LayoutPoint::new(x + tile_width, y + tile_height),
                    );
                    list.commands.push(PaintCmd::DrawImage(ImageItem {
                        placement: CommonPlacement::new(placement),
                        image_key,
                        image_rendering: ImageRendering::Auto,
                        alpha_type: AlphaType::Alpha,
                        color: ColorF::WHITE,
                    }));
                }
            }
            list.commands.push(PaintCmd::PopClip);
        },
    }
}

fn background_box_rect(
    style: &ComputedValues,
    border_rect: LayoutRect,
    background_box: BackgroundBox,
) -> LayoutRect {
    if background_box == BackgroundBox::BorderBox {
        return border_rect;
    }
    let em = used_font_size(style);
    let border_left = border_width_px(style.border_left_style, style.border_left_width, em);
    let border_right = border_width_px(style.border_right_style, style.border_right_width, em);
    let border_top = border_width_px(style.border_top_style, style.border_top_width, em);
    let border_bottom = border_width_px(style.border_bottom_style, style.border_bottom_width, em);
    let mut left = border_left;
    let mut right = border_right;
    let mut top = border_top;
    let mut bottom = border_bottom;
    if background_box == BackgroundBox::ContentBox {
        let basis = border_rect.width();
        left += length_percentage_px(style.padding_left.0, em, basis);
        right += length_percentage_px(style.padding_right.0, em, basis);
        top += length_percentage_px(style.padding_top.0, em, basis);
        bottom += length_percentage_px(style.padding_bottom.0, em, basis);
    }
    inset_rect(border_rect, left, top, right, bottom)
}

fn inset_rect(rect: LayoutRect, left: f32, top: f32, right: f32, bottom: f32) -> LayoutRect {
    let min = LayoutPoint::new(rect.min.x + left.max(0.0), rect.min.y + top.max(0.0));
    let max = LayoutPoint::new(
        (rect.max.x - right.max(0.0)).max(min.x),
        (rect.max.y - bottom.max(0.0)).max(min.y),
    );
    LayoutRect::new(min, max)
}

fn used_background_size(
    size: BackgroundSize,
    area: LayoutSize,
    intrinsic_width: f32,
    intrinsic_height: f32,
    em: f32,
) -> (f32, f32) {
    let intrinsic_width = intrinsic_width.max(f32::EPSILON);
    let intrinsic_height = intrinsic_height.max(f32::EPSILON);
    match size {
        BackgroundSize::Cover | BackgroundSize::Contain => {
            let x = area.width / intrinsic_width;
            let y = area.height / intrinsic_height;
            let scale = if size == BackgroundSize::Cover {
                x.max(y)
            } else {
                x.min(y)
            };
            (intrinsic_width * scale, intrinsic_height * scale)
        },
        BackgroundSize::Explicit { width, height } => {
            let width = used_background_size_component(width, area.width, em);
            let height = used_background_size_component(height, area.height, em);
            match (width, height) {
                (Some(width), Some(height)) => (width, height),
                (Some(width), None) => (width, intrinsic_height * width / intrinsic_width),
                (None, Some(height)) => (intrinsic_width * height / intrinsic_height, height),
                (None, None) => (intrinsic_width, intrinsic_height),
            }
        },
    }
}

fn used_gradient_size(size: BackgroundSize, area: LayoutSize, em: f32) -> (f32, f32) {
    match size {
        BackgroundSize::Cover | BackgroundSize::Contain => (area.width, area.height),
        BackgroundSize::Explicit { width, height } => (
            used_background_size_component(width, area.width, em).unwrap_or(area.width),
            used_background_size_component(height, area.height, em).unwrap_or(area.height),
        ),
    }
}

fn used_background_size_component(
    component: BackgroundSizeComponent,
    basis: f32,
    em: f32,
) -> Option<f32> {
    match component {
        BackgroundSizeComponent::Auto => None,
        BackgroundSizeComponent::Value(value) => {
            Some(resolve_length_percentage(value, basis, em).max(0.0))
        },
    }
}

fn rounded_tile_size(area: f32, tile: f32) -> f32 {
    let count = (area / tile).round().max(1.0);
    area / count
}

fn background_axis_tiles(
    positioning_min: f32,
    positioning_size: f32,
    painting_min: f32,
    painting_max: f32,
    tile: f32,
    offset: f32,
    repeat: RepeatStyle,
) -> Vec<f32> {
    let positioned = positioning_min + offset;
    match repeat {
        RepeatStyle::NoRepeat => vec![positioned],
        RepeatStyle::Repeat | RepeatStyle::Round => {
            let first = tile_origin(positioned, painting_min, tile, true);
            let count = tile_count(first, painting_max, tile, true);
            (0..count)
                .map(|index| first + index as f32 * tile)
                .collect()
        },
        RepeatStyle::Space => {
            let count = (positioning_size / tile).floor() as usize;
            if count < 2 {
                return vec![positioned];
            }
            let spacing = (positioning_size - count as f32 * tile) / (count - 1) as f32;
            (0..count)
                .map(|index| positioning_min + index as f32 * (tile + spacing))
                .collect()
        },
    }
}

fn resolve_length_percentage(value: LengthPercentage, basis: f32, em: f32) -> f32 {
    match value {
        LengthPercentage::Zero => 0.0,
        LengthPercentage::Length(length) => length.unit.to_px(length.value, em, 16.0),
        LengthPercentage::Percentage(value) => basis * value,
        LengthPercentage::Calc(calc) => {
            calc.percentage * basis + calc.px + calc.em * em + calc.rem * 16.0
        },
        LengthPercentage::Math(math) => LengthPercentage::Math(math).to_px(em, 16.0, basis),
    }
}

fn tile_origin(origin: f32, painting_min: f32, tile: f32, repeated: bool) -> f32 {
    if repeated && tile > 0.0 {
        origin - ((origin - painting_min) / tile).ceil() * tile
    } else {
        origin
    }
}

fn tile_count(first: f32, max: f32, tile: f32, repeated: bool) -> usize {
    if !repeated || tile <= 0.0 {
        return 1;
    }
    ((max - first) / tile).ceil().max(0.0) as usize
}

fn emit_shadow(list: &mut LiveryPaintList, style: &ComputedValues, fragment: &Fragment) {
    let CssBoxShadow::Value(shadow) = &style.box_shadow else {
        return;
    };
    let em = used_font_size(style);
    let length = |value: Length| value.unit.to_px(value.value, em, 16.0);
    list.commands.push(PaintCmd::DrawShadow(ShadowItem {
        placement: CommonPlacement::new(bounds(fragment)),
        box_bounds: bounds(fragment),
        offset: LayoutVector2D::new(length(shadow.offset_x), length(shadow.offset_y)),
        color: resolve_color(&shadow.color),
        blur_radius: length(shadow.blur_radius).max(0.0),
        spread_radius: length(shadow.spread_radius),
        border_radius: border_radius(style, fragment),
        clip_mode: if shadow.inset {
            BoxShadowClipMode::Inset
        } else {
            BoxShadowClipMode::Outset
        },
    }));
}

fn emit_border(list: &mut LiveryPaintList, style: &ComputedValues, fragment: &Fragment) {
    emit_inline_border(list, style, fragment, true, true);
}

fn emit_inline_border(
    list: &mut LiveryPaintList,
    style: &ComputedValues,
    fragment: &Fragment,
    paint_left: bool,
    paint_right: bool,
) {
    let em = used_font_size(style);
    let widths = LayoutSideOffsets::new(
        border_width_px(style.border_top_style, style.border_top_width, em),
        if paint_right {
            border_width_px(style.border_right_style, style.border_right_width, em)
        } else {
            0.0
        },
        border_width_px(style.border_bottom_style, style.border_bottom_width, em),
        if paint_left {
            border_width_px(style.border_left_style, style.border_left_width, em)
        } else {
            0.0
        },
    );
    if widths.top == 0.0 && widths.right == 0.0 && widths.bottom == 0.0 && widths.left == 0.0 {
        return;
    }
    list.commands.push(PaintCmd::DrawBorder(BorderItem {
        placement: CommonPlacement::new(bounds(fragment)),
        widths,
        details: BorderDetails::Normal(NormalBorder {
            left: border_side(style.border_left_style, &style.border_left_color),
            right: border_side(style.border_right_style, &style.border_right_color),
            top: border_side(style.border_top_style, &style.border_top_color),
            bottom: border_side(style.border_bottom_style, &style.border_bottom_color),
            radius: border_radius(style, fragment),
            do_aa: true,
        }),
    }));
}

fn border_radius(style: &ComputedValues, fragment: &Fragment) -> BorderRadius {
    let em = used_font_size(style);
    let corner = |x: Radius, y: Radius| {
        LayoutSize::new(
            super::layout::length_percentage_px(x.0, em, fragment.width),
            super::layout::length_percentage_px(y.0, em, fragment.height),
        )
    };
    BorderRadius {
        top_left: corner(style.border_top_left_radius, style.border_top_left_radius),
        top_right: corner(style.border_top_right_radius, style.border_top_right_radius),
        bottom_left: corner(
            style.border_bottom_left_radius,
            style.border_bottom_left_radius,
        ),
        bottom_right: corner(
            style.border_bottom_right_radius,
            style.border_bottom_right_radius,
        ),
    }
}

fn border_side(style: CssBorderStyle, color: &ComputedColor) -> BorderSide {
    BorderSide {
        color: resolve_color(color),
        style: match style {
            CssBorderStyle::None => BorderStyle::None,
            CssBorderStyle::Hidden => BorderStyle::Hidden,
            CssBorderStyle::Dotted => BorderStyle::Dotted,
            CssBorderStyle::Dashed => BorderStyle::Dashed,
            CssBorderStyle::Solid => BorderStyle::Solid,
            CssBorderStyle::Double => BorderStyle::Double,
            CssBorderStyle::Groove => BorderStyle::Groove,
            CssBorderStyle::Ridge => BorderStyle::Ridge,
            CssBorderStyle::Inset => BorderStyle::Inset,
            CssBorderStyle::Outset => BorderStyle::Outset,
        },
    }
}

pub(crate) fn resolve_color(color: &ComputedColor) -> ColorF {
    let (red, green, blue, alpha) = color
        .to_srgb()
        .expect("C3 resolves every paint color before emission");
    ColorF::new(
        red.clamp(0.0, 1.0),
        green.clamp(0.0, 1.0),
        blue.clamp(0.0, 1.0),
        alpha.clamp(0.0, 1.0),
    )
}

pub(crate) fn used_font_size(style: &ComputedValues) -> f32 {
    match style.font_size {
        FontSize::Value(LengthPercentage::Length(Length {
            value,
            unit: LengthUnit::Px,
        })) => value,
        _ => 16.0,
    }
}
