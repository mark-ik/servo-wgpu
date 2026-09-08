// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Retained Livery document ownership.

use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

use layout_dom_api::{DomMutation, LayoutDom, LayoutDomMut, LocalName, Namespace, NodeKind};
use livery::cascade::DeclaredValue;
use livery::media::{Device, SystemPalette};
use livery::{AnimationClass, PropertyId};
use livery::{
    PropertyValue,
    selector::StatePseudoClass,
    stylesheet::Keyframes,
    values::{AnimationName, ColorScheme, Opacity, Overflow, TimingFunction, TransitionProperty},
};
use paint_list_api::DeviceIntSize;

use crate::{
    IncrementalStyle, InteractionStates, LayoutError, LiveryLayout, LiveryPaintList, RestyleStats,
    StylePlane, StyleSet, TextDirective, TextRange, TextRect, TextSelection, TextSystem,
    emit_paint_list_with_text_system_scrolled_with_images, hit_test_with_scroll,
    layout::{
        RetainedRootFormatting, StickyScrollport, layout_retained_formatting_root,
        layout_with_text_system, resolve_container_query_styles,
        resolve_container_query_styles_with_images, retained_table_owner,
    },
    resolve_styles,
};

mod animation;
mod frame;
mod resources;
mod scrolling;
mod selection;
#[cfg(test)]
use selection::find_id;
#[cfg(test)]
mod tests;

/// What a Livery click resolved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClickOutcome {
    None,
    Focused,
    Scrolled,
    Navigate(String),
}

/// A link rectangle retained from the last layout pass.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkTarget {
    pub url: String,
    pub rect: [f32; 4],
}

/// The event class that invalidated retained layout geometry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutDamageKind {
    Dom,
    Stylesheet,
    Device,
    Resource,
    Interaction,
    Viewport,
}

/// Conservative formatting-context candidates for the next retained-layout
/// replacement. K5h records this separately from style invalidation so a
/// caller can inspect exactly why a frame was rebuilt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutDamage<Id> {
    pub kind: LayoutDamageKind,
    pub roots: Vec<Id>,
    pub full_document: bool,
}

struct LayoutState<Id> {
    viewport: (u32, u32),
    styles: StylePlane<Id>,
    fragments: LiveryLayout<Id>,
    content_width: f32,
    content_height: f32,
}

fn nodes_in_subtree<D>(dom: &D, root: D::NodeId) -> HashSet<D::NodeId>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    fn visit<D>(dom: &D, node: D::NodeId, nodes: &mut HashSet<D::NodeId>)
    where
        D: LayoutDom,
        D::NodeId: Copy + Eq + Hash,
    {
        nodes.insert(node);
        for child in dom.flat_children(node) {
            visit(dom, child, nodes);
        }
    }

    let mut nodes = HashSet::new();
    visit(dom, root, &mut nodes);
    nodes
}

fn text_sources_in_dom_order<D>(dom: &D) -> Vec<D::NodeId>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    fn visit<D>(dom: &D, node: D::NodeId, sources: &mut Vec<D::NodeId>)
    where
        D: LayoutDom,
        D::NodeId: Copy + Eq + Hash,
    {
        if dom.kind(node) == NodeKind::Text {
            sources.push(node);
        }
        for child in dom.dom_children(node) {
            visit(dom, child, sources);
        }
    }

    let mut sources = Vec::new();
    visit(dom, dom.document(), &mut sources);
    sources
}

/// One retained transition on the generic clock (harvest H2): any
/// transitionable longhand, sampled through the generated
/// `PropertyValue::interpolate` dispatch.
#[derive(Clone)]
struct PropertyTransition<Id> {
    node: Id,
    property: PropertyId,
    from: PropertyValue,
    to: PropertyValue,
    start_ms: f64,
    duration_ms: f64,
    automatic: bool,
}

#[derive(Clone)]
struct KeyframeAnimation<Id> {
    node: Id,
    name: Box<str>,
    start_ms: f64,
    duration_ms: f64,
    delay_ms: f64,
    timing: TimingFunction,
}

/// A retained DOM plus the Livery state that should survive between frames.
///
/// Equal-size frames reuse the complete paint list. Resizes recascade media
/// queries, relayout, and repaint while retaining Parley's font database,
/// shaping scratch space, and shared font-resource allocations.
pub struct LiveryDocument<D>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    dom: D,
    style_set: StyleSet,
    device: Device,
    interactions: InteractionStates<D::NodeId>,
    style_session: IncrementalStyle<D::NodeId>,
    text: TextSystem,
    generation: u64,
    layout_generation: u64,
    #[cfg(test)]
    retained_root_relayout_generation: u64,
    cached: Option<((u32, u32), LiveryPaintList)>,
    layout: Option<LayoutState<D::NodeId>>,
    identity_source: Option<LayoutState<D::NodeId>>,
    last_layout_damage: Option<LayoutDamage<D::NodeId>>,
    layout_dirty: bool,
    viewport: (u32, u32),
    scroll: (f32, f32),
    focused_chain: Vec<D::NodeId>,
    clock_ms: f64,
    transitions: Vec<PropertyTransition<D::NodeId>>,
    keyframe_animation: Option<KeyframeAnimation<D::NodeId>>,
    nested_scroll: HashMap<D::NodeId, (f32, f32)>,
    image_sources: HashMap<String, Vec<u8>>,
    font_sources: HashMap<String, Vec<u8>>,
    selection_anchor: Option<(D::NodeId, usize)>,
    selection_range: Option<TextRange<D::NodeId>>,
}

impl<D> LiveryDocument<D>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    pub fn new(dom: D, style_set: StyleSet, device: Device) -> Self {
        let viewport = (
            device.viewport_width.max(0.0) as u32,
            device.viewport_height.max(0.0) as u32,
        );
        Self {
            dom,
            style_set,
            device,
            interactions: InteractionStates::default(),
            style_session: IncrementalStyle::new(),
            text: TextSystem::new(),
            generation: 0,
            layout_generation: 0,
            #[cfg(test)]
            retained_root_relayout_generation: 0,
            cached: None,
            layout: None,
            identity_source: None,
            last_layout_damage: None,
            layout_dirty: true,
            viewport,
            scroll: (0.0, 0.0),
            focused_chain: Vec::new(),
            clock_ms: 0.0,
            transitions: Vec::new(),
            keyframe_animation: None,
            nested_scroll: HashMap::new(),
            image_sources: HashMap::new(),
            font_sources: HashMap::new(),
            selection_anchor: None,
            selection_range: None,
        }
    }

    pub fn dom(&self) -> &D {
        &self.dom
    }

    pub fn text_system(&self) -> &TextSystem {
        &self.text
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The number of completed geometry passes. Paint-only and scroll-only
    /// frames advance [`Self::generation`] but not this counter.
    pub fn layout_generation(&self) -> u64 {
        self.layout_generation
    }

    pub fn interactions_mut(&mut self) -> &mut InteractionStates<D::NodeId> {
        self.record_full_layout_damage(LayoutDamageKind::Interaction);
        self.retain_layout_identity();
        &mut self.interactions
    }

    /// Apply one exact DOM-mutation batch before the next frame.
    ///
    /// A retained frame is otherwise eligible for paint-list reuse. A table
    /// mutation must not reuse its old candidates, winner grid, metrics, or
    /// collapsed-border paint, so any nonempty batch discards every derived
    /// frame artifact and updates the retained style plane first. The next
    /// [`Self::frame`] then rebuilds layout and paint from that same style
    /// generation. The current correctness path deliberately rebuilds
    /// geometry; K5g reconciles only compatible identities from the retained
    /// prior generation, and callers must not infer incremental table geometry
    /// from `RestyleStats`.
    pub fn apply_dom_mutations(&mut self, mutations: &[DomMutation<D::NodeId>]) -> RestyleStats {
        if mutations.is_empty() {
            return self.style_session.last_stats();
        }

        self.record_dom_layout_damage(mutations);
        self.mark_layout_dirty();
        self.transitions.clear();
        self.keyframe_animation = None;
        self.style_session.update(
            &self.dom,
            &self.style_set,
            &self.device,
            &self.interactions,
            mutations,
        )
    }

    /// Mutate the owned DOM and atomically hand its exact recorded batch to
    /// [`Self::apply_dom_mutations`]. This is the retained-document entry point
    /// for script-hosted tables: a caller cannot accidentally paint a cached
    /// frame after changing table structure or a participating border style.
    pub fn mutate_dom<R>(&mut self, mutate: impl FnOnce(&mut D) -> R) -> (R, RestyleStats)
    where
        D: LayoutDomMut,
    {
        let result = mutate(&mut self.dom);
        let mut mutations = Vec::new();
        self.dom.drain_mutations(&mut mutations);
        let stats = self.apply_dom_mutations(&mutations);
        (result, stats)
    }

    /// Set the host preference that media queries and an element's supported
    /// `color-scheme` list consult. A real change invalidates style, layout,
    /// and paint together; a repeated setting is intentionally a no-op.
    pub fn set_preferred_color_scheme(&mut self, scheme: ColorScheme) -> bool {
        if self.device.preferred_color_scheme() == scheme {
            return false;
        }
        self.device.set_preferred_color_scheme(scheme);
        self.invalidate();
        true
    }

    /// Replace the host system palette. Palette values are consumed while
    /// styles compute, so this has the same full retained invalidation as a
    /// changed media preference.
    pub fn set_system_palette(&mut self, palette: SystemPalette) -> bool {
        if self.device.system_palette == palette {
            return false;
        }
        self.device.set_system_palette(palette);
        self.invalidate();
        true
    }

    pub fn last_restyle_stats(&self) -> RestyleStats {
        self.style_session.last_stats()
    }

    /// The table dispatch record from the most recent completed layout.
    ///
    /// A retained document has no layout before its first frame, and an
    /// invalidation deliberately clears the prior record with the rest of its
    /// derived state. Hosts can therefore observe a ledger only for the exact
    /// frame they rendered.
    pub fn table_shadow_ledger(&self) -> Option<&crate::table_shadow::TableShadowLedger> {
        self.layout
            .as_ref()
            .map(|layout| layout.fragments.table_shadow_ledger())
    }

    /// CSSOM `insertRule` on one retained author sheet (harvest H3). The
    /// next frame restyles, relays out, and repaints.
    pub fn insert_author_rule(
        &mut self,
        sheet: usize,
        rule: &str,
        index: usize,
    ) -> Result<usize, livery::stylesheet::RuleMutationError> {
        let inserted = self.style_set.insert_author_rule(sheet, rule, index)?;
        self.rebuild_font_resources();
        self.record_full_layout_damage(LayoutDamageKind::Stylesheet);
        self.retain_layout_identity();
        Ok(inserted)
    }

    /// CSSOM `deleteRule` on one retained author sheet (harvest H3).
    pub fn delete_author_rule(
        &mut self,
        sheet: usize,
        index: usize,
    ) -> Result<(), livery::stylesheet::RuleMutationError> {
        self.style_set.delete_author_rule(sheet, index)?;
        self.rebuild_font_resources();
        self.record_full_layout_damage(LayoutDamageKind::Stylesheet);
        self.retain_layout_identity();
        Ok(())
    }

    /// The retained style set, for CSSOM reads (rule counts, object model).
    pub fn style_set(&self) -> &StyleSet {
        &self.style_set
    }

    /// getComputedStyle backing (harvest H3): serialize one longhand or
    /// custom property of one element from the retained style plane. With
    /// no retained layout yet, styles resolve on demand at the current
    /// device. Unknown names and unstyled nodes return None.
    pub fn computed_style(&self, node: D::NodeId, property: &str) -> Option<String> {
        let resolved;
        let container_resolved;
        let plane = match self.layout.as_ref() {
            Some(layout) => &layout.styles,
            None => {
                resolved =
                    resolve_styles(&self.dom, &self.style_set, &self.device, &self.interactions);
                container_resolved = resolve_container_query_styles(
                    &self.dom,
                    &resolved,
                    &self.style_set,
                    &self.device,
                    &self.interactions,
                )
                .ok();
                container_resolved.as_ref().unwrap_or(&resolved)
            },
        };
        plane.computed_style(node, property)
    }

    /// The last completed retained layout in document coordinates.
    ///
    /// This only borrows geometry published by a successful [`Self::frame`]
    /// call. It never computes a frame and does not apply viewport or nested
    /// scroll offsets, so consumers which publish screen-space bounds retain
    /// ownership of those transforms.
    pub fn retained_layout(&self) -> Option<&LiveryLayout<D::NodeId>> {
        self.layout.as_ref().map(|layout| &layout.fragments)
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Option<D::NodeId> {
        let layout = self.layout.as_ref()?;
        let active = self.sticky_layout(layout);
        hit_test_with_scroll(
            &self.dom,
            &layout.styles,
            &active,
            &self.nested_scroll,
            x + self.scroll.0,
            y + self.scroll.1,
        )
    }

    /// The outermost retained fragment for a node in viewport coordinates.
    pub fn fragment_rect(&self, node: D::NodeId) -> Option<[f32; 4]> {
        let fragment = self.layout.as_ref()?.fragments.get(node)?;
        let (nested_x, nested_y) = self.ancestor_scroll(node);
        Some([
            fragment.x - self.scroll.0 - nested_x,
            fragment.y - self.scroll.1 - nested_y,
            fragment.width,
            fragment.height,
        ])
    }

    /// A replaced element's retained **content box** in viewport coordinates.
    ///
    /// [`Self::fragment_rect`] is the border box. A child browsing context
    /// lives in the content box, so hit testing into a frame needs this and
    /// not that: the difference is the frame's own border and padding, which
    /// the user-agent sheet makes non-zero by default.
    pub fn content_rect(&self, node: D::NodeId) -> Option<[f32; 4]> {
        let layout = self.layout.as_ref()?;
        let fragment = layout.fragments.get(node)?;
        let style = layout.styles.get(node)?;
        let (x, y, width, height) = crate::layout::content_box_rect(style, fragment);
        let (nested_x, nested_y) = self.ancestor_scroll(node);
        Some([
            x - self.scroll.0 - nested_x,
            y - self.scroll.1 - nested_y,
            width,
            height,
        ])
    }

    fn focus(&mut self, id: D::NodeId) -> bool {
        for old in self.focused_chain.drain(..) {
            self.interactions.set(old, StatePseudoClass::Focus, false);
            self.interactions
                .set(old, StatePseudoClass::FocusWithin, false);
        }
        self.interactions.set(id, StatePseudoClass::Focus, true);
        let mut chain = vec![id];
        let mut parent = self.dom.parent(id);
        while let Some(ancestor) = parent {
            if self.dom.kind(ancestor) == NodeKind::Element {
                self.interactions
                    .set(ancestor, StatePseudoClass::FocusWithin, true);
                chain.push(ancestor);
            }
            parent = self.dom.parent(ancestor);
        }
        self.focused_chain = chain;
        self.record_full_layout_damage(LayoutDamageKind::Interaction);
        self.retain_layout_identity();
        true
    }

    fn focusable_ancestor(&self, mut id: D::NodeId) -> Option<D::NodeId> {
        loop {
            if self.is_focusable(id) {
                return Some(id);
            }
            id = self.dom.parent(id)?;
        }
    }

    fn is_focusable(&self, id: D::NodeId) -> bool {
        if self.dom.kind(id) != NodeKind::Element {
            return false;
        }
        let Some(name) = self.dom.element_name(id) else {
            return false;
        };
        let local = name.local.as_ref();
        local.eq_ignore_ascii_case("a") && self.attribute(id, "href").is_some()
            || matches!(
                local.to_ascii_lowercase().as_str(),
                "button" | "input" | "select" | "textarea"
            )
            || self.attribute(id, "tabindex").is_some()
    }

    fn node_contains(&self, node: D::NodeId, ancestor: D::NodeId) -> bool {
        let mut current = Some(node);
        while let Some(candidate) = current {
            if candidate == ancestor {
                return true;
            }
            current = self.dom.parent(candidate);
        }
        false
    }

    fn attribute(&self, id: D::NodeId, local: &str) -> Option<&str> {
        self.dom
            .attribute(id, &Namespace::from(""), &LocalName::from(local))
    }

    pub fn into_dom(self) -> D {
        self.dom
    }
}
