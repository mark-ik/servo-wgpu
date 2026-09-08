// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Hit testing: stacking-aware candidate collection over the retained
//! fragment tree, in paint order, honouring ancestor clips and scroll.

use super::*;

/// Return the topmost pointer-events-enabled element whose layout fragment
/// contains a scene point. The walk mirrors the lane's DOM paint order for the
/// bounded stacking subset: numeric z-index first, then source order within a
/// stacking context. Descendants remain inside their nearest positioned
/// context, so a child can paint above its context's background without
/// escaping an ancestor that is below a sibling context.
pub fn hit_test<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    x: f32,
    y: f32,
) -> Option<D::NodeId>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    hit_test_with_scroll(dom, styles, fragments, &HashMap::new(), x, y)
}

/// Return a numeric stacking level only where CSS lets `z-index` establish a
/// context: positioned boxes and direct flex/grid items. A static ordinary
/// block keeps normal paint order even when it carries a numeric declaration.
pub(crate) fn z_index_stacking_level<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    id: D::NodeId,
) -> Option<i32>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let style = styles.get(id)?;
    let livery::values::ZIndex::Integer(level) = style.z_index else {
        return None;
    };
    let is_flex_or_grid_item = dom
        .parent(id)
        .and_then(|parent| styles.get(parent))
        .is_some_and(|parent| matches!(parent.display, CssDisplay::Flex | CssDisplay::Grid));
    (style.position != CssPosition::Static || is_flex_or_grid_item).then_some(level)
}

/// The hit-order twin of paint's bounded stacking-context admission. A
/// polygon clip is an atomic paint scope, so descendants must keep its level
/// rather than competing with the clipped box's siblings.
fn hit_stacking_level<D>(dom: &D, styles: &StylePlane<D::NodeId>, id: D::NodeId) -> Option<i32>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    z_index_stacking_level(dom, styles, id).or_else(|| {
        styles
            .get(id)
            .filter(|style| !style.clip_path.is_none())
            .map(|_| 0)
    })
}

/// Return direct DOM children in CSS paint order for the admitted item
/// containers. Flex and grid order by their computed `order` value while the
/// stable sort preserves document order for equal values and anonymous text.
pub(crate) fn order_modified_children<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    parent: D::NodeId,
) -> Vec<D::NodeId>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let mut children = dom.flat_children(parent).collect::<Vec<_>>();
    let is_flex_or_grid = styles
        .get(parent)
        .is_some_and(|style| matches!(style.display, CssDisplay::Flex | CssDisplay::Grid));
    if is_flex_or_grid {
        children.sort_by_key(|child| styles.get(*child).map_or(0, |style| style.order.value()));
    }
    children
}

/// Direct children in the admitted paint order. Flex/grid `order` remains the
/// first ordering step; positioned and flex/grid-item stacking levels then
/// divide that sequence, with equal levels retaining its stable source order.
///
/// This is local to one stacking context. A child of a positioned
/// `z-index: 4` box may have `z-index: 2`, but it still belongs to the outer
/// level 4 context rather than competing with an unrelated level 2 sibling.
pub(in crate::layout) fn stacking_paint_children<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    parent: D::NodeId,
) -> Vec<D::NodeId>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let mut children = order_modified_children(dom, styles, parent);
    children.sort_by_key(|child| hit_stacking_level(dom, styles, *child).unwrap_or_default());
    children
}

/// Hit-test a retained fragment plane after applying per-element scroll
/// offsets to descendants. The ordinary [`hit_test`] path keeps the map empty;
/// retained sessions use this variant for wheel-scrolled containers.
pub fn hit_test_with_scroll<D>(
    dom: &D,
    styles: &StylePlane<D::NodeId>,
    fragments: &LiveryLayout<D::NodeId>,
    scroll_offsets: &HashMap<D::NodeId, (f32, f32)>,
    x: f32,
    y: f32,
) -> Option<D::NodeId>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let mut state = HitTestState {
        dom,
        styles,
        fragments,
        scroll_offsets,
        x,
        y,
        clips: Vec::new(),
        order: 0,
        candidates: Vec::new(),
    };
    collect_hit_candidates(&mut state, dom.document(), (0.0, 0.0), None);
    state
        .candidates
        .into_iter()
        .max_by_key(|candidate| (candidate.level, candidate.order))
        .map(|candidate| candidate.id)
}

pub(in crate::layout) struct HitCandidate<Id> {
    pub(in crate::layout) id: Id,
    pub(in crate::layout) level: i32,
    pub(in crate::layout) order: u64,
}

enum HitClip {
    Rect(f32, f32, f32, f32),
    Polygon(Vec<(f32, f32)>),
}

impl HitClip {
    fn contains(&self, x: f32, y: f32) -> bool {
        match self {
            Self::Rect(left, top, right, bottom) => {
                x >= *left && x <= *right && y >= *top && y <= *bottom
            },
            Self::Polygon(points) => point_in_polygon(points, x, y),
        }
    }
}

fn point_in_polygon(points: &[(f32, f32)], x: f32, y: f32) -> bool {
    let Some(&last) = points.last() else {
        return false;
    };
    let mut previous = last;
    // CSS path fills use the nonzero winding rule. Matching it here matters
    // for self-intersecting and double-wound polygons, where an even-odd
    // toggle would make an invisible region steal a click.
    let mut winding = 0_i32;
    for &(current_x, current_y) in points {
        let (previous_x, previous_y) = previous;
        if point_on_segment(previous_x, previous_y, current_x, current_y, x, y) {
            return true;
        }
        let side = (current_x - previous_x) * (y - previous_y)
            - (x - previous_x) * (current_y - previous_y);
        if previous_y <= y {
            if current_y > y && side > 0.0 {
                winding += 1;
            }
        } else if current_y <= y && side < 0.0 {
            winding -= 1;
        }
        previous = (current_x, current_y);
    }
    winding != 0
}

fn point_on_segment(ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) -> bool {
    let cross = (x - ax) * (by - ay) - (y - ay) * (bx - ax);
    cross.abs() <= f32::EPSILON
        && x >= ax.min(bx)
        && x <= ax.max(bx)
        && y >= ay.min(by)
        && y <= ay.max(by)
}

pub(in crate::layout) struct HitTestState<'a, D>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    pub(in crate::layout) dom: &'a D,
    pub(in crate::layout) styles: &'a StylePlane<D::NodeId>,
    pub(in crate::layout) fragments: &'a LiveryLayout<D::NodeId>,
    pub(in crate::layout) scroll_offsets: &'a HashMap<D::NodeId, (f32, f32)>,
    pub(in crate::layout) x: f32,
    pub(in crate::layout) y: f32,
    clips: Vec<HitClip>,
    pub(in crate::layout) order: u64,
    pub(in crate::layout) candidates: Vec<HitCandidate<D::NodeId>>,
}

pub(in crate::layout) fn collect_hit_candidates<D>(
    state: &mut HitTestState<'_, D>,
    id: D::NodeId,
    ancestor_scroll: (f32, f32),
    ancestor_stacking_level: Option<i32>,
) where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let style = state.styles.get(id);
    let stacking_level =
        ancestor_stacking_level.or_else(|| hit_stacking_level(state.dom, state.styles, id));
    // K4e4: the hit target is the node's outermost box - a table element's
    // wrapper - so the caption area belongs to the table when nothing deeper
    // claims it, and the caption element wins inside its own rectangle by
    // paint order.
    let fragment = state.fragments.get(id);
    let visible_fragment = fragment.map(|fragment| Fragment {
        x: fragment.x - ancestor_scroll.0,
        y: fragment.y - ancestor_scroll.1,
        ..fragment.physical_rect()
    });
    let own_clip = style.zip(visible_fragment).and_then(|(style, fragment)| {
        style
            .clip_path
            .polygon_points(fragment.width, fragment.height)
            .map(|points| {
                HitClip::Polygon(
                    points
                        .into_iter()
                        .map(|(x, y)| (fragment.x + x, fragment.y + y))
                        .collect(),
                )
            })
    });
    let inside_clips = state
        .clips
        .iter()
        .all(|clip| clip.contains(state.x, state.y));
    let inside_own_clip = own_clip
        .as_ref()
        .is_none_or(|clip| clip.contains(state.x, state.y));
    if state.dom.kind(id) == NodeKind::Element
        && let (Some(style), Some(fragment)) = (style, visible_fragment)
        && style.display != CssDisplay::None
        && style.visibility == livery::values::Visibility::Visible
        && style.pointer_events == livery::values::PointerEvents::Auto
        && inside_clips
        && inside_own_clip
        && state.x >= fragment.x
        && state.x <= fragment.x + fragment.width
        && state.y >= fragment.y
        && state.y <= fragment.y + fragment.height
    {
        let level = stacking_level.unwrap_or_default();
        state.candidates.push(HitCandidate {
            id,
            level,
            order: state.order,
        });
    }
    state.order = state.order.saturating_add(1);

    let overflow_clip = style
        .zip(visible_fragment)
        .filter(|(style, _)| {
            style.overflow_x != CssOverflow::Visible || style.overflow_y != CssOverflow::Visible
        })
        .map(|(_, fragment)| {
            HitClip::Rect(
                fragment.x,
                fragment.y,
                fragment.x + fragment.width,
                fragment.y + fragment.height,
            )
        });
    let mut pushed_clips = 0;
    if let Some(clip) = own_clip {
        state.clips.push(clip);
        pushed_clips += 1;
    }
    if let Some(clip) = overflow_clip {
        state.clips.push(clip);
        pushed_clips += 1;
    }
    let children = stacking_paint_children(state.dom, state.styles, id);
    let next_scroll = state
        .scroll_offsets
        .get(&id)
        .copied()
        .map_or(ancestor_scroll, |offset| {
            (ancestor_scroll.0 + offset.0, ancestor_scroll.1 + offset.1)
        });
    for child in children {
        collect_hit_candidates(state, child, next_scroll, stacking_level);
    }
    for _ in 0..pushed_clips {
        state.clips.pop();
    }
}
