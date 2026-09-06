// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Livery document rendering and host spatial queries.

use std::collections::HashMap;
use std::hash::Hash;

use genet_livery::{
    Device, InteractionStates, LayoutError, LiveryDocument, LiveryLayout, LiveryPaintList,
    StylePlane, StyleSet, TextRange, TextRect, TextSystem, ViewportSizes,
    emit_paint_list_with_text_system_scrolled_with_images, hit_test_with_scroll, layout,
    layout_with_text_system, resolve_styles,
};
use genet_scripted_dom::{NodeId, ScriptedDom};
use layout_dom_api::LayoutDom;
use paint_list_api::{ColorF, DeviceIntSize, LayoutPoint, LayoutRect, LayoutSize, PaintList};

const CARET_WIDTH: f32 = 2.0;
const CARET_COLOR: ColorF = ColorF {
    r: 0.12,
    g: 0.12,
    b: 0.20,
    a: 1.0,
};
const SELECTION_COLOR: ColorF = ColorF {
    r: 0.40,
    g: 0.60,
    b: 0.95,
    a: 0.40,
};
const FOCUS_RING_COLOR: ColorF = ColorF {
    r: 0.42,
    g: 0.62,
    b: 0.98,
    a: 0.95,
};
const FOCUS_RING_WIDTH: f32 = 2.0;

/// Per-element scroll offsets supplied by a host-owned input router.
pub type ScrollOffsets<Id> = HashMap<Id, (f32, f32)>;

/// Which side of a shaped cluster owns a caret at a shared byte boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VisualAffinity {
    #[default]
    Downstream,
    Upstream,
}

/// A caret in rendered byte space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisualCaret {
    pub byte: usize,
    pub affinity: VisualAffinity,
}

/// A directed selection in rendered byte space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisualSelection {
    pub anchor: VisualCaret,
    pub focus: VisualCaret,
}

/// A keyboard movement interpreted against shaped text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualMovement {
    PreviousCluster,
    NextCluster,
    PreviousWord,
    NextWord,
    PreviousLine,
    NextLine,
    LineStart,
    LineEnd,
}

/// What to paint for a focused scripted-DOM element.
pub struct TextCursor {
    pub node: NodeId,
    pub caret: usize,
    pub affinity: VisualAffinity,
    pub selection: Option<(usize, usize)>,
    pub editable: bool,
}

/// Host-neutral metadata for a producer texture emitted by a paint list.
#[derive(Clone, Debug, PartialEq)]
pub struct ExternalTextureDraw {
    pub texture_key: u64,
    pub dest_rect: [f32; 4],
    pub opacity: f32,
    pub scene_op_boundary: usize,
}

/// A rendered document frame and its same-device producer-texture draws.
#[derive(Clone, Debug)]
pub struct RenderedFrame {
    pub scene: netrender::Scene,
    pub external_textures: Vec<ExternalTextureDraw>,
}

/// Lower any owned paint-list vocabulary through the shared NetRender bridge.
pub fn translate_frame<L: PaintList>(list: &L) -> RenderedFrame {
    let translated = paint_list_render::translate_paint_cmd_stream(
        list.viewport(),
        list.commands(),
        list.fonts(),
        list.images(),
    );
    RenderedFrame {
        scene: translated.scene,
        external_textures: translated
            .external_textures
            .into_iter()
            .map(|draw| ExternalTextureDraw {
                texture_key: draw.texture_key,
                dest_rect: draw.placement.dest_rect,
                opacity: draw.placement.opacity,
                scene_op_boundary: draw.scene_op_boundary,
            })
            .collect(),
    }
}

fn overlay_rect(list: &mut LiveryPaintList, rect: TextRect, color: ColorF) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    list.push_overlay_rect(
        LayoutRect::from_origin_and_size(
            LayoutPoint::new(rect.x, rect.y),
            LayoutSize::new(rect.width, rect.height),
        ),
        color,
    );
}

fn ancestor_scroll<D>(
    dom: &D,
    node: D::NodeId,
    scroll_offsets: &ScrollOffsets<D::NodeId>,
) -> (f32, f32)
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let mut total = (0.0, 0.0);
    let mut current = dom.parent(node);
    while let Some(parent) = current {
        if let Some((x, y)) = scroll_offsets.get(&parent) {
            total.0 += x;
            total.1 += y;
        }
        current = dom.parent(parent);
    }
    total
}

fn append_cursor(
    list: &mut LiveryPaintList,
    dom: &ScriptedDom,
    fragments: &LiveryLayout<NodeId>,
    cursor: TextCursor,
    scroll_offsets: &ScrollOffsets<NodeId>,
) {
    let (scroll_x, scroll_y) = ancestor_scroll(dom, cursor.node, scroll_offsets);
    if cursor.editable {
        if let Some((start, end)) = cursor.selection
            && let Some(selection) = fragments.text_selection(TextRange {
                anchor_node: cursor.node,
                anchor_offset: start,
                focus_node: cursor.node,
                focus_offset: end,
            })
        {
            for mut rect in selection.rects {
                rect.x -= scroll_x;
                rect.y -= scroll_y;
                overlay_rect(list, rect, SELECTION_COLOR);
            }
        }
        if let Some(mut rect) = fragments.caret_rect(cursor.node, cursor.caret) {
            rect.x -= scroll_x;
            rect.y -= scroll_y;
            rect.width = CARET_WIDTH;
            overlay_rect(list, rect, CARET_COLOR);
        }
    }

    if let Some(fragment) = fragments.get(cursor.node) {
        let x = fragment.x - scroll_x;
        let y = fragment.y - scroll_y;
        let w = fragment.width;
        let h = fragment.height;
        let t = FOCUS_RING_WIDTH.min(w * 0.5).min(h * 0.5);
        for rect in [
            TextRect {
                x,
                y,
                width: w,
                height: t,
            },
            TextRect {
                x,
                y: y + h - t,
                width: w,
                height: t,
            },
            TextRect {
                x,
                y,
                width: t,
                height: h,
            },
            TextRect {
                x: x + w - t,
                y,
                width: t,
                height: h,
            },
        ] {
            overlay_rect(list, rect, FOCUS_RING_COLOR);
        }
    }
}

fn compute_layout<D>(
    dom: &D,
    stylesheets: &[&str],
    width: u32,
    height: u32,
) -> Result<(StylePlane<D::NodeId>, LiveryLayout<D::NodeId>), LayoutError>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let styles = resolve_styles(
        dom,
        &StyleSet::cambium(stylesheets),
        &Device::screen(width as f32, height as f32),
        &InteractionStates::default(),
    );
    let fragments = layout(dom, &styles, width as f32, height as f32)?;
    Ok((styles, fragments))
}

fn paint_list_from_layout<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    width: u32,
    height: u32,
    scroll_offsets: &ScrollOffsets<D::NodeId>,
) -> LiveryPaintList
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    emit_paint_list_with_text_system_scrolled_with_images(
        dom,
        styles,
        fragments,
        DeviceIntSize::new(width as i32, height as i32),
        0,
        &mut TextSystem::new(),
        scroll_offsets,
        &HashMap::new(),
    )
}

/// Run Livery cascade, Buckram layout, paint emission, and scene translation.
pub fn scene_from_scripted_dom(
    dom: &ScriptedDom,
    stylesheets: &[&str],
    width: u32,
    height: u32,
    cursor: Option<TextCursor>,
    scroll_offsets: &ScrollOffsets<NodeId>,
) -> Result<netrender::Scene, LayoutError> {
    Ok(translate_frame(&paint_list_from_scripted_dom(
        dom,
        stylesheets,
        width,
        height,
        cursor,
        scroll_offsets,
    )?)
    .scene)
}

/// [`scene_from_scripted_dom`] through a caller-owned text system.
///
/// The one-shot path builds a fresh `TextSystem` per call, which is fine
/// wherever fontique can discover system fonts. On `wasm32` there are none:
/// a host that ships its own font registers it once
/// (`TextSystem::register_font_bytes`) on a text system it keeps, and lays
/// out through here — otherwise every text run shapes against an empty
/// collection and paints nothing. Shaping scratch space then also survives
/// between frames, as it does for retained sessions.
pub fn scene_from_scripted_dom_with_text_system(
    dom: &ScriptedDom,
    stylesheets: &[&str],
    width: u32,
    height: u32,
    cursor: Option<TextCursor>,
    scroll_offsets: &ScrollOffsets<NodeId>,
    text: &mut TextSystem,
) -> Result<netrender::Scene, LayoutError> {
    Ok(
        translate_frame(&paint_list_from_scripted_dom_with_text_system(
            dom,
            stylesheets,
            width,
            height,
            cursor,
            scroll_offsets,
            text,
        )?)
        .scene,
    )
}

/// [`paint_list_from_scripted_dom`] through a caller-owned text system; see
/// [`scene_from_scripted_dom_with_text_system`] for when that matters.
pub fn paint_list_from_scripted_dom_with_text_system(
    dom: &ScriptedDom,
    stylesheets: &[&str],
    width: u32,
    height: u32,
    cursor: Option<TextCursor>,
    scroll_offsets: &ScrollOffsets<NodeId>,
    text: &mut TextSystem,
) -> Result<LiveryPaintList, LayoutError> {
    let styles = resolve_styles(
        dom,
        &StyleSet::cambium(stylesheets),
        &Device::screen(width as f32, height as f32),
        &InteractionStates::default(),
    );
    let (styles, fragments) = layout_with_text_system(
        dom,
        &styles,
        width as f32,
        height as f32,
        ViewportSizes::uniform(width as f32, height as f32),
        text,
        &HashMap::new(),
    )?;
    let mut list = emit_paint_list_with_text_system_scrolled_with_images(
        dom,
        &styles,
        &fragments,
        DeviceIntSize::new(width as i32, height as i32),
        0,
        text,
        scroll_offsets,
        &HashMap::new(),
    );
    if let Some(cursor) = cursor {
        append_cursor(&mut list, dom, &fragments, cursor, scroll_offsets);
    }
    Ok(list)
}

/// Render any neutral Genet DOM through the owned Livery and Buckram pair.
pub fn scene_from_layout_dom<D>(
    dom: &D,
    stylesheets: &[&str],
    width: u32,
    height: u32,
    scroll_offsets: &ScrollOffsets<D::NodeId>,
) -> Result<netrender::Scene, LayoutError>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let (styles, fragments) = compute_layout(dom, stylesheets, width, height)?;
    Ok(translate_frame(&paint_list_from_layout(
        dom,
        &styles,
        &fragments,
        width,
        height,
        scroll_offsets,
    ))
    .scene)
}

/// The scripted one-shot path before NetRender lowering.
pub fn paint_list_from_scripted_dom(
    dom: &ScriptedDom,
    stylesheets: &[&str],
    width: u32,
    height: u32,
    cursor: Option<TextCursor>,
    scroll_offsets: &ScrollOffsets<NodeId>,
) -> Result<LiveryPaintList, LayoutError> {
    let (styles, fragments) = compute_layout(dom, stylesheets, width, height)?;
    let mut list = paint_list_from_layout(dom, &styles, &fragments, width, height, scroll_offsets);
    if let Some(cursor) = cursor {
        append_cursor(&mut list, dom, &fragments, cursor, scroll_offsets);
    }
    Ok(list)
}

fn append_session_cursor(
    list: &mut LiveryPaintList,
    session: &LiveryDocument<ScriptedDom>,
    cursor: TextCursor,
) {
    if cursor.editable {
        if let Some((start, end)) = cursor.selection
            && let Some(selection) = session.selection_for_range(TextRange {
                anchor_node: cursor.node,
                anchor_offset: start,
                focus_node: cursor.node,
                focus_offset: end,
            })
        {
            for rect in selection.rects {
                overlay_rect(list, rect, SELECTION_COLOR);
            }
        }
        if let Some(mut rect) = session.caret_rect(cursor.node, cursor.caret) {
            rect.width = CARET_WIDTH;
            overlay_rect(list, rect, CARET_COLOR);
        }
    }
    if let Some([x, y, width, height]) = session.fragment_rect(cursor.node) {
        let t = FOCUS_RING_WIDTH.min(width * 0.5).min(height * 0.5);
        for rect in [
            TextRect {
                x,
                y,
                width,
                height: t,
            },
            TextRect {
                x,
                y: y + height - t,
                width,
                height: t,
            },
            TextRect {
                x,
                y,
                width: t,
                height,
            },
            TextRect {
                x: x + width - t,
                y,
                width: t,
                height,
            },
        ] {
            overlay_rect(list, rect, FOCUS_RING_COLOR);
        }
    }
}

/// Emit a focused scripted session through its retained Livery document.
pub fn paint_list_from_session(
    session: &mut LiveryDocument<ScriptedDom>,
    cursor: Option<TextCursor>,
    width: u32,
    height: u32,
) -> Result<LiveryPaintList, LayoutError> {
    let mut list = session.frame(width, height)?;
    if let Some(cursor) = cursor {
        append_session_cursor(&mut list, session, cursor);
    }
    Ok(list)
}

pub fn scene_from_session(
    session: &mut LiveryDocument<ScriptedDom>,
    cursor: Option<TextCursor>,
    width: u32,
    height: u32,
) -> Result<netrender::Scene, LayoutError> {
    Ok(translate_frame(&paint_list_from_session(session, cursor, width, height)?).scene)
}

/// Translate one retained Livery document frame.
pub fn translated_frame_from_session_dom<D>(
    session: &mut LiveryDocument<D>,
    width: u32,
    height: u32,
) -> Result<RenderedFrame, LayoutError>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    Ok(translate_frame(&session.frame(width, height)?))
}

pub fn scene_from_session_dom<D>(
    session: &mut LiveryDocument<D>,
    width: u32,
    height: u32,
) -> Result<netrender::Scene, LayoutError>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    Ok(translated_frame_from_session_dom(session, width, height)?.scene)
}

/// Livery paint emission already includes document and element scrollbars.
pub fn scene_from_session_dom_with_scrollbars<D>(
    session: &mut LiveryDocument<D>,
    width: u32,
    height: u32,
) -> Result<netrender::Scene, LayoutError>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    scene_from_session_dom(session, width, height)
}

pub fn caret_screen_rect(
    dom: &ScriptedDom,
    stylesheets: &[&str],
    width: u32,
    height: u32,
    node: NodeId,
    caret_byte: usize,
) -> Option<(f32, f32, f32, f32)> {
    LaidOutDocument::compute(dom, stylesheets, width, height)
        .ok()?
        .caret_screen_rect(node, caret_byte)
}

pub fn caret_screen_rect_for_position(
    dom: &ScriptedDom,
    stylesheets: &[&str],
    width: u32,
    height: u32,
    node: NodeId,
    caret: VisualCaret,
) -> Option<(f32, f32, f32, f32)> {
    caret_screen_rect(dom, stylesheets, width, height, node, caret.byte)
}

pub fn soft_wrap_caret_byte(
    dom: &ScriptedDom,
    stylesheets: &[&str],
    width: u32,
    height: u32,
    node: NodeId,
    caret_byte: usize,
    delta: isize,
    goal_x: Option<f32>,
) -> Option<(usize, f32)> {
    LaidOutDocument::compute(dom, stylesheets, width, height)
        .ok()?
        .soft_wrap_caret_byte(node, caret_byte, delta, goal_x)
}

pub fn caret_byte_at(
    dom: &ScriptedDom,
    stylesheets: &[&str],
    width: u32,
    height: u32,
    node: NodeId,
    x: f32,
    y: f32,
) -> Option<usize> {
    caret_position_at(dom, stylesheets, width, height, node, x, y).map(|caret| caret.byte)
}

pub fn caret_position_at(
    dom: &ScriptedDom,
    stylesheets: &[&str],
    width: u32,
    height: u32,
    node: NodeId,
    x: f32,
    y: f32,
) -> Option<VisualCaret> {
    LaidOutDocument::compute(dom, stylesheets, width, height)
        .ok()?
        .caret_position_at(node, x, y)
}

pub fn range_rects_from_scripted_dom(
    dom: &ScriptedDom,
    stylesheets: &[&str],
    width: u32,
    height: u32,
    range: TextRange<NodeId>,
) -> Vec<(f32, f32, f32, f32)> {
    LaidOutDocument::compute(dom, stylesheets, width, height)
        .map(|document| document.range_rects(range))
        .unwrap_or_default()
}

pub fn fragments_from_scripted_dom(
    dom: &ScriptedDom,
    stylesheets: &[&str],
    width: u32,
    height: u32,
) -> Result<LiveryLayout<NodeId>, LayoutError> {
    Ok(LaidOutDocument::compute(dom, stylesheets, width, height)?.into_fragments())
}

pub fn hit_test_node(
    dom: &ScriptedDom,
    stylesheets: &[&str],
    width: u32,
    height: u32,
    x: f32,
    y: f32,
    scroll_offsets: &ScrollOffsets<NodeId>,
) -> Option<NodeId> {
    LaidOutDocument::compute(dom, stylesheets, width, height)
        .ok()?
        .hit_test(x, y, scroll_offsets)
}

/// One Livery cascade and Buckram layout serving many host queries.
pub struct LaidOutDocument<'a> {
    dom: &'a ScriptedDom,
    styles: StylePlane<NodeId>,
    fragments: LiveryLayout<NodeId>,
}

impl<'a> LaidOutDocument<'a> {
    pub fn compute(
        dom: &'a ScriptedDom,
        stylesheets: &[&str],
        width: u32,
        height: u32,
    ) -> Result<Self, LayoutError> {
        let (styles, fragments) = compute_layout(dom, stylesheets, width, height)?;
        Ok(Self {
            dom,
            styles,
            fragments,
        })
    }

    pub fn fragments(&self) -> &LiveryLayout<NodeId> {
        &self.fragments
    }

    pub fn into_fragments(self) -> LiveryLayout<NodeId> {
        self.fragments
    }

    pub fn hit_test(
        &self,
        x: f32,
        y: f32,
        scroll_offsets: &ScrollOffsets<NodeId>,
    ) -> Option<NodeId> {
        hit_test_with_scroll(
            self.dom,
            &self.styles,
            &self.fragments,
            scroll_offsets,
            x,
            y,
        )
    }

    pub fn caret_screen_rect(
        &self,
        node: NodeId,
        caret_byte: usize,
    ) -> Option<(f32, f32, f32, f32)> {
        let rect = self.fragments.caret_rect(node, caret_byte)?;
        Some((rect.x, rect.y, CARET_WIDTH, rect.height))
    }

    pub fn caret_screen_rect_for_position(
        &self,
        node: NodeId,
        caret: VisualCaret,
    ) -> Option<(f32, f32, f32, f32)> {
        self.caret_screen_rect(node, caret.byte)
    }

    pub fn soft_wrap_caret_byte(
        &self,
        node: NodeId,
        caret_byte: usize,
        delta: isize,
        goal_x: Option<f32>,
    ) -> Option<(usize, f32)> {
        let rect = self.fragments.caret_rect(node, caret_byte)?;
        let goal_x = goal_x.unwrap_or(rect.x);
        let y = rect.y + rect.height * (delta.signum() as f32 + 0.5);
        let (source, byte) = self.fragments.text_position_at_point(goal_x, y)?;
        (source == node).then_some((byte, goal_x))
    }

    pub fn caret_byte_at(&self, node: NodeId, x: f32, y: f32) -> Option<usize> {
        self.caret_position_at(node, x, y).map(|caret| caret.byte)
    }

    pub fn caret_position_at(&self, node: NodeId, x: f32, y: f32) -> Option<VisualCaret> {
        let (source, byte) = self.fragments.text_position_at_point(x, y)?;
        (source == node).then_some(VisualCaret {
            byte,
            affinity: VisualAffinity::Downstream,
        })
    }

    pub fn range_rects(&self, range: TextRange<NodeId>) -> Vec<(f32, f32, f32, f32)> {
        self.fragments
            .text_selection(range)
            .map(|selection| {
                selection
                    .rects
                    .into_iter()
                    .map(|rect| (rect.x, rect.y, rect.width, rect.height))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[cfg(feature = "accesskit")]
    pub fn accesskit_tree(&self, focus: Option<NodeId>) -> accesskit::TreeUpdate {
        crate::a11y::accesskit_tree(self.dom, &self.fragments, focus)
    }
}
