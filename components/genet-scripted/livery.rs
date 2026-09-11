/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Livery's scripted CSSOM adapter.
//!
//! The JS runtime owns the live mutable DOM. Livery owns retained author rule
//! objects beside it, and resolves a style plane on demand for
//! `getComputedStyle`. This is intentionally a style/session bridge rather than
//! a second DOM copy: script mutations are visible to the next read immediately.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use genet_document_resources::{
    ResolvedDocumentResources, ResourceDelta, ResourceFetcher, ResourceLimits,
};
use genet_livery::{
    CssomRuleKind, Device, IncrementalStyle, InlineShorthandExpansion, InteractionStates,
    LayoutError, LiveryLayout, LiveryPaintList, RestyleStats, RuleMutationError, StylePlane,
    StyleSet, TextDirective, TextRange, TextSelection, TextSystem, ViewportSizes,
    canonicalize_specified_value, classify_specified_shorthand, content_box_size,
    emit_paint_list_with_text_system_scrolled_with_images,
    emit_paint_list_with_text_system_scrolled_with_images_and_external_textures, hit_test,
    is_implemented_shorthand, layout, layout_with_text_system, reconstruct_specified_shorthand,
    resolve_container_query_styles, resolve_container_query_styles_with_images, resolve_styles,
    specified_shorthand_longhands, used_value_context as layout_used_value_context,
};
use genet_scripted_dom::{NodeId, ScriptedDom};
use layout_dom_api::{
    AttributeView, LayoutDom, LayoutDomMut, LocalName, Namespace, NodeKind, QualName, QuirksMode,
};
use livery::media::MediaQueryList;
use paint_list_api::{ColorF, DeviceIntSize, LayoutPoint, LayoutRect, LayoutSize};
use script_engine_api::ScriptEngine;
use script_runtime_api::{
    ComputedStyleHandler, HostState, InlineStyleHandler, InlineStyleValueResult, MediaQueryHandler,
    Runtime, SelectionHandler, SharedHost, StyleSheetHandler, StyleSheetImportOwner,
    StyleSheetImportRule, StyleSheetMutationError, StyleSheetRule, StyleSheetRuleKind,
};

/// Author markup may name a texture key, but only a key minted by this
/// runtime's live WebGL contexts is eligible for paint emission.
fn trusted_canvas_textures(host: &HostState) -> HashMap<NodeId, u64> {
    let allowed: HashSet<u64> = host
        .webgl_contexts
        .iter()
        .filter_map(|context| context.external_texture_key())
        .collect();
    if allowed.is_empty() {
        return HashMap::new();
    }
    let mut textures = HashMap::new();
    let mut nodes = vec![host.dom.document()];
    while let Some(node) = nodes.pop() {
        if host.dom.element_name(node).is_some_and(|name| {
            name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "canvas"
        }) {
            if let Some(key) = host
                .dom
                .attribute(
                    node,
                    &Namespace::default(),
                    &LocalName::from("data-genet-external-texture-key"),
                )
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|key| allowed.contains(key))
            {
                textures.insert(node, key);
            }
        }
        nodes.extend(host.dom.dom_children(node));
    }
    textures
}

struct LiveryState {
    styles: StyleSet,
    document_sheets: Vec<usize>,
    device: Device,
    interactions: InteractionStates<NodeId>,
    session: IncrementalStyle<NodeId>,
    mutation_cursor: u64,
    render_cursor: u64,
    render_generation: u64,
    cached_frame: Option<((u32, u32), LiveryPaintList)>,
    external_textures: HashMap<NodeId, u64>,
    frame: Option<LiveFrame>,
    text: TextSystem,
    image_sources: HashMap<String, Vec<u8>>,
    font_sources: HashMap<String, Vec<u8>>,
    scroll: (f32, f32),
    selection_anchor: Option<(NodeId, usize)>,
    selection_range: Option<TextRange<NodeId>>,
    live_sheets: Option<LiveStylesheetSource>,
    /// Set whenever `device` moves, cleared by
    /// [`LiveryCssom::take_device_changed`]. A live `MediaQueryList` has to be
    /// re-evaluated and its `change` event fired, and only the owner of the
    /// `Runtime` can do that — a handler called from inside a native cannot
    /// re-enter the engine.
    device_changed: bool,
}

const SELECTION_COLOR: ColorF = ColorF {
    r: 0.40,
    g: 0.60,
    b: 0.95,
    a: 0.40,
};

/// The default action produced by a click through the live scripted layout.
///
/// Script listeners run before this result is chosen. A listener that calls
/// `preventDefault()` therefore leaves the click handled without asking the
/// embedding host to replace the addressed document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScriptedClick {
    Miss,
    Handled,
    Navigate(String),
}

/// Geometry retained beside a live runtime-owned DOM. It is deliberately not a
/// second DOM or a `LiveryDocument`: the runtime remains the mutable owner.
struct LiveFrame {
    viewport: (u32, u32),
    styles: StylePlane<NodeId>,
    fragments: LiveryLayout<NodeId>,
    content_extent: (f32, f32),
}

struct LiveStylesheetSource {
    document_url: String,
    fetcher: Rc<dyn ResourceFetcher>,
    limits: ResourceLimits,
    resources: ResolvedDocumentResources,
    mutation_cursor: u64,
    resource_sink: Option<Rc<RefCell<dyn LiveResourceSink>>>,
}

/// Host callback for resource bytes that change while a scripted document is
/// live. The callback receives the complete next ledger plus an explicit delta,
/// so it can remove stale images and rebuild font registration without CSSOM
/// fetching behind the host boundary.
pub trait LiveResourceSink {
    fn replace_resources(&mut self, resources: &ResolvedDocumentResources, delta: &ResourceDelta);
}

impl LiveryState {
    fn sheet_key_at(&self, list_index: usize) -> Option<String> {
        let author_index = *self.document_sheets.get(list_index)?;
        self.styles.cssom_key(author_index).map(str::to_owned)
    }

    fn author_index_for_key(&self, key: &str) -> Option<usize> {
        self.styles.author_index_by_cssom_key(key)
    }

    /// Anything that changes the retained cascade or device must retire the
    /// last paint list. DOM batches are tracked separately by `render_cursor`.
    fn invalidate_frame(&mut self) {
        self.cached_frame = None;
        self.frame = None;
    }

    /// Move the device and record that media queries must be re-evaluated.
    fn device_moved(&mut self) {
        self.device_changed = true;
        self.invalidate_frame();
    }
}

fn resource_ledgers(
    resources: &ResolvedDocumentResources,
) -> (HashMap<String, Vec<u8>>, HashMap<String, Vec<u8>>) {
    let mut images = HashMap::new();
    let mut fonts = HashMap::new();
    for resource in &resources.resources {
        match resource.kind {
            genet_document_resources::ResourceKind::Image => {
                images.insert(resource.authored_url.clone(), resource.bytes.clone());
                if resource.resolved_url != resource.authored_url {
                    images.insert(resource.resolved_url.clone(), resource.bytes.clone());
                }
            },
            genet_document_resources::ResourceKind::Font => {
                fonts.insert(resource.resolved_url.clone(), resource.bytes.clone());
            },
        }
    }
    (images, fonts)
}

fn replace_resource_ledger(state: &mut LiveryState, resources: &ResolvedDocumentResources) {
    let (images, fonts) = resource_ledgers(resources);
    let images_changed = state.image_sources != images;
    let fonts_changed = state.font_sources != fonts;
    if !images_changed && !fonts_changed {
        return;
    }
    state.image_sources = images;
    if fonts_changed {
        state.font_sources = fonts;
        state.text = TextSystem::new();
        for bytes in state.font_sources.values().cloned() {
            state.text.register_font_bytes(bytes);
        }
    }
    state.invalidate_frame();
}

fn synchronize_live_styles(host: &HostState, state: &mut LiveryState) {
    if state.live_sheets.is_none() {
        return;
    }
    let (base, pending) = host.dom.pending_mutations();
    let end = base.saturating_add(pending.len() as u64);
    let (resources, changed) = {
        let live = state
            .live_sheets
            .as_mut()
            .expect("checked live stylesheet source");
        if live.mutation_cursor >= base && live.mutation_cursor == end {
            return;
        }
        let resources = ResolvedDocumentResources::resolve_with_limits(
            &host.dom,
            Some(&live.document_url),
            live.fetcher.as_ref(),
            live.limits,
        );
        let changed = resources != live.resources;
        if changed {
            let resource_delta = resources.resource_delta_from(&live.resources);
            if !resource_delta.is_empty()
                && let Some(sink) = &live.resource_sink
            {
                sink.borrow_mut()
                    .replace_resources(&resources, &resource_delta);
            }
            live.resources = resources.clone();
        }
        live.mutation_cursor = end;
        (resources, changed)
    };
    if changed {
        replace_resource_ledger(state, &resources);
        state.styles.replace_author_sheets(&resources.stylesheets);
        state.document_sheets = state.styles.document_sheet_indexes();
        state.session.invalidate();
        state.invalidate_frame();
    }
}

/// A fetcher that serves nothing. A live installation over it resolves exactly
/// the inline content `ResolvedDocumentResources::discover` finds, while
/// keeping the live source that makes the resolution repeat as the DOM grows.
struct NoResourceFetch;

impl ResourceFetcher for NoResourceFetch {
    fn fetch(&self, _url: &str) -> Option<Vec<u8>> {
        None
    }
}

/// A retained Livery stylesheet session installed on one scripted runtime.
///
/// Keep this handle when the host needs to update the media device. The runtime
/// itself retains the handler state for JS reads and mutations.
#[derive(Clone)]
pub struct LiveryCssom {
    state: Rc<RefCell<LiveryState>>,
}

impl LiveryCssom {
    /// Install Livery as the runtime's `document.styleSheets` and
    /// `getComputedStyle` provider. `author_sheets` are ordered exactly as they
    /// appear in the document.
    pub fn install<E: ScriptEngine>(
        runtime: &mut Runtime<E>,
        author_sheets: &[&str],
        device: Device,
    ) -> Self {
        let styles = StyleSet::cambium(author_sheets);
        let state = Rc::new(RefCell::new(LiveryState {
            document_sheets: styles.document_sheet_indexes(),
            styles,
            device,
            interactions: InteractionStates::default(),
            session: IncrementalStyle::new(),
            mutation_cursor: 0,
            render_cursor: 0,
            render_generation: 0,
            cached_frame: None,
            external_textures: HashMap::new(),
            frame: None,
            text: TextSystem::new(),
            image_sources: HashMap::new(),
            font_sources: HashMap::new(),
            scroll: (0.0, 0.0),
            selection_anchor: None,
            selection_range: None,
            live_sheets: None,
            device_changed: false,
        }));
        Self::install_state(runtime, state)
    }

    /// Install over a document the parser has not built yet — the two-phase
    /// form HTML's parsing model needs.
    ///
    /// [`install`](Self::install) freezes the author sheets it is handed, which
    /// over an arena the parser has not filled is none at all. This one keeps a
    /// live source with a fetcher that serves nothing, so the sheets are
    /// re-resolved from the arena at every read: a `<style>` or a `<link>`
    /// enters the cascade at the point the parser inserts it, and a script that
    /// reads computed style mid-parse sees the sheets parsed so far — exactly
    /// what a browser shows it. Inline-only, so it resolves what
    /// `ResolvedDocumentResources::discover` would, but never goes stale.
    pub fn install_over_parse<E: ScriptEngine>(
        runtime: &mut Runtime<E>,
        document_url: impl Into<String>,
        device: Device,
    ) -> Self {
        Self::install_live_with_optional_sink(
            runtime,
            NoResourceFetch,
            document_url,
            ResourceLimits::default(),
            device,
            None,
        )
    }

    /// Install a live resource-backed stylesheet set. The shared resolver runs
    /// again after DOM mutations and reconciles retained direct-sheet CSSOM
    /// objects by their owner node, while imported sheets remain cascade-only
    /// children of their parent source.
    pub fn install_live<E, Fetch>(
        runtime: &mut Runtime<E>,
        fetcher: Fetch,
        document_url: impl Into<String>,
        limits: ResourceLimits,
        device: Device,
    ) -> Self
    where
        E: ScriptEngine,
        Fetch: ResourceFetcher + 'static,
    {
        Self::install_live_with_optional_sink(runtime, fetcher, document_url, limits, device, None)
    }

    /// Install a live resource-backed stylesheet set and deliver initial and
    /// later image/font ledger changes to one host-owned consumer.
    pub fn install_live_with_resource_sink<E, Fetch, Sink>(
        runtime: &mut Runtime<E>,
        fetcher: Fetch,
        document_url: impl Into<String>,
        limits: ResourceLimits,
        device: Device,
        sink: Sink,
    ) -> Self
    where
        E: ScriptEngine,
        Fetch: ResourceFetcher + 'static,
        Sink: LiveResourceSink + 'static,
    {
        Self::install_live_with_optional_sink(
            runtime,
            fetcher,
            document_url,
            limits,
            device,
            Some(Rc::new(RefCell::new(sink))),
        )
    }

    fn install_live_with_optional_sink<E, Fetch>(
        runtime: &mut Runtime<E>,
        fetcher: Fetch,
        document_url: impl Into<String>,
        limits: ResourceLimits,
        device: Device,
        resource_sink: Option<Rc<RefCell<dyn LiveResourceSink>>>,
    ) -> Self
    where
        E: ScriptEngine,
        Fetch: ResourceFetcher + 'static,
    {
        let fetcher: Rc<dyn ResourceFetcher> = Rc::new(fetcher);
        let cssom = Self::install_live_host_with_optional_sink(
            runtime.host(),
            fetcher.clone(),
            document_url,
            limits,
            device,
            resource_sink,
        );
        Self::install_child_providers(runtime, fetcher, limits);
        cssom
    }

    /// Bind a child realm's CSSOM directly to its live host arena.
    pub fn install_live_for_host<Fetch: ResourceFetcher + 'static>(
        host: &SharedHost,
        fetcher: Fetch,
        document_url: impl Into<String>,
        limits: ResourceLimits,
        device: Device,
    ) -> Self {
        Self::install_live_host_with_optional_sink(
            host,
            Rc::new(fetcher),
            document_url,
            limits,
            device,
            None,
        )
    }

    fn install_live_host_with_optional_sink(
        host: &SharedHost,
        fetcher: Rc<dyn ResourceFetcher>,
        document_url: impl Into<String>,
        limits: ResourceLimits,
        device: Device,
        resource_sink: Option<Rc<RefCell<dyn LiveResourceSink>>>,
    ) -> Self {
        let document_url = document_url.into();
        let (resources, mutation_cursor) = {
            let host = host.borrow();
            let resources = ResolvedDocumentResources::resolve_with_limits(
                &host.dom,
                Some(&document_url),
                fetcher.as_ref(),
                limits,
            );
            let (base, pending) = host.dom.pending_mutations();
            (resources, base.saturating_add(pending.len() as u64))
        };
        let styles = StyleSet::cambium_resources(&resources.stylesheets);
        if let Some(sink) = &resource_sink {
            let initial = resources.resource_delta_from(&ResolvedDocumentResources::default());
            if !initial.is_empty() {
                sink.borrow_mut().replace_resources(&resources, &initial);
            }
        }
        let mut initial_state = LiveryState {
            document_sheets: styles.document_sheet_indexes(),
            styles,
            device,
            interactions: InteractionStates::default(),
            session: IncrementalStyle::new(),
            mutation_cursor,
            render_cursor: mutation_cursor,
            render_generation: 0,
            cached_frame: None,
            external_textures: HashMap::new(),
            frame: None,
            text: TextSystem::new(),
            image_sources: HashMap::new(),
            font_sources: HashMap::new(),
            scroll: (0.0, 0.0),
            selection_anchor: None,
            selection_range: None,
            live_sheets: Some(LiveStylesheetSource {
                document_url,
                fetcher,
                limits,
                resources: resources.clone(),
                mutation_cursor,
                resource_sink,
            }),
            device_changed: false,
        };
        replace_resource_ledger(&mut initial_state, &resources);
        let state = Rc::new(RefCell::new(initial_state));
        Self::install_state_for_host(host, state)
    }

    fn install_state<E: ScriptEngine>(
        runtime: &mut Runtime<E>,
        state: Rc<RefCell<LiveryState>>,
    ) -> Self {
        let cssom = Self::install_state_for_host(runtime.host(), state);
        Self::install_child_providers(runtime, Rc::new(NoResourceFetch), ResourceLimits::default());
        cssom
    }

    /// A child is a separate document arena. Bind its own live stylesheet
    /// source before authored child script, including hosts using this CSSOM
    /// adapter directly rather than LiveryScriptedDocument's paint registry.
    fn install_child_providers<E: ScriptEngine>(
        runtime: &mut Runtime<E>,
        fetcher: Rc<dyn ResourceFetcher>,
        limits: ResourceLimits,
    ) {
        let initialize = Rc::new(move |_: script_engine_api::RealmId, host: &SharedHost| {
            let (url, viewport) = {
                let h = host.borrow();
                (
                    h.base_url.clone().unwrap_or_else(|| "about:blank".into()),
                    h.viewport_size,
                )
            };
            Self::install_live_host_with_optional_sink(
                host,
                fetcher.clone(),
                url,
                limits,
                Device::screen(viewport.0.max(1.0), viewport.1.max(1.0)),
                None,
            );
        });
        let future = initialize.clone();
        runtime.set_child_host_initializer(move |realm, host| future(realm, host));
        // A host can install CSSOM after parsing a document which already has
        // children. Bind those arenas as well as future inserted frames.
        let mut parents = vec![script_engine_api::MAIN_REALM];
        while let Some(parent) = parents.pop() {
            let style = runtime
                .host_in_realm(parent)
                .ok()
                .and_then(|host| host.borrow().computed_style.clone());
            for (owner, realm) in runtime.frame_realms(parent) {
                if let Ok(host) = runtime.host_in_realm(realm) {
                    // Eager frame creation can precede installing this adapter.
                    // Resolve the existing child's viewport against its parent's
                    // newly installed provider before constructing its device.
                    let dimension = |name: &str, fallback: f32| {
                        style
                            .as_ref()
                            .and_then(|handler| handler.computed_value(owner, name))
                            .and_then(|value| {
                                value
                                    .trim()
                                    .strip_suffix("px")
                                    .and_then(|value| value.parse::<f32>().ok())
                            })
                            .filter(|value| value.is_finite() && *value >= 0.0)
                            .unwrap_or(fallback)
                    };
                    let viewport = host.borrow().viewport_size;
                    host.borrow_mut().viewport_size = (
                        dimension("width", viewport.0),
                        dimension("height", viewport.1),
                    );
                    initialize(realm, &host);
                }
                parents.push(realm);
            }
        }
    }

    fn install_state_for_host(shared: &SharedHost, state: Rc<RefCell<LiveryState>>) -> Self {
        let host = Rc::downgrade(shared);
        let mut target = shared.borrow_mut();
        target.computed_style = Some(Rc::new(LiveryComputedStyle {
            host: host.clone(),
            state: state.clone(),
        }));
        target.media_query = Some(Rc::new(LiveryMediaQueries {
            state: state.clone(),
        }));
        target.inline_style = Some(Rc::new(LiveryInlineStyle));
        target.selection = Some(Rc::new(LiverySelection {
            host: host.clone(),
            state: state.clone(),
        }));
        target.stylesheets = Some(Rc::new(LiveryStyleSheets {
            state: state.clone(),
            host,
        }));
        Self { state }
    }

    /// Update the device used by media queries and computed-value resolution.
    pub fn set_viewport_size(&self, width: f32, height: f32) {
        let mut state = self.state.borrow_mut();
        state.device.set_viewport_size(width, height);
        state.device_moved();
    }

    /// Whether the device has moved since this was last asked, clearing the
    /// flag. The owner of the `Runtime` calls
    /// [`Runtime::notify_media_features_changed`] when it answers `true`, which
    /// is what makes `matchMedia(q).onchange` fire on this route.
    pub fn take_device_changed(&self) -> bool {
        let mut state = self.state.borrow_mut();
        std::mem::take(&mut state.device_changed)
    }

    /// Supply distinct small, large, and dynamic viewport sizes.
    pub fn set_viewport_sizes(&self, sizes: ViewportSizes) {
        let mut state = self.state.borrow_mut();
        state.device.set_viewport_sizes(sizes);
        state.device_moved();
    }

    /// The retained generation stamp for one author sheet.
    pub fn generation(&self, sheet: usize) -> Option<u64> {
        let state = self.state.borrow();
        let sheet = *state.document_sheets.get(sheet)?;
        state
            .styles
            .author_sheets()
            .get(sheet)
            .map(|sheet| sheet.generation())
    }

    /// The shared resource ledger for a live CSSOM installation. Static
    /// `install` sessions have no resource owner and return `None`.
    pub fn resource_set(&self) -> Option<ResolvedDocumentResources> {
        self.state
            .borrow()
            .live_sheets
            .as_ref()
            .map(|live| live.resources.clone())
    }

    /// Work performed by the latest scripted computed-style read.
    pub fn last_restyle_stats(&self) -> RestyleStats {
        self.state.borrow().session.last_stats()
    }

    /// Build one retained Livery paint frame from the runtime's live DOM.
    ///
    /// The scripted runtime remains the DOM owner. This method observes its
    /// exact pending `DomMutation` suffix, brings the retained Livery style
    /// session to that suffix, then lays out and paints before draining it.
    /// Therefore a CSSOM read and the following paint frame cannot use
    /// different collapsed-border winner generations. The current frame path
    /// deliberately performs a complete layout when any batch is present.
    pub fn frame<E: ScriptEngine>(
        &self,
        runtime: &mut Runtime<E>,
        width: u32,
        height: u32,
    ) -> Result<LiveryPaintList, LayoutError> {
        self.frame_host(runtime.host(), width, height)
    }

    /// Paint the same arena served by the realm's DOM and CSSOM callbacks.
    pub fn frame_host(
        &self,
        shared: &SharedHost,
        width: u32,
        height: u32,
    ) -> Result<LiveryPaintList, LayoutError> {
        let viewport = (width.max(1), height.max(1));
        let mut host = shared.borrow_mut();
        let (base, pending) = host.dom.pending_mutations();
        let end = base.saturating_add(pending.len() as u64);
        let mut state = self.state.borrow_mut();
        synchronize_live_styles(&host, &mut state);
        let external_textures = trusted_canvas_textures(&host);
        if state.external_textures != external_textures {
            state.external_textures = external_textures;
            state.invalidate_frame();
        }

        if (state.device.viewport_width, state.device.viewport_height)
            != (viewport.0 as f32, viewport.1 as f32)
        {
            state
                .device
                .set_viewport_size(viewport.0 as f32, viewport.1 as f32);
            state.device_moved();
        }
        if state.mutation_cursor < base || state.mutation_cursor > end {
            // Another selected layout consumer drained a batch first. Its
            // exact records are no longer available, so restyle conservatively
            // instead of retaining a paint list from the old DOM generation.
            state.session.invalidate();
            state.mutation_cursor = base;
            state.invalidate_frame();
        }
        let start = state.mutation_cursor.saturating_sub(base) as usize;
        {
            let LiveryState {
                styles,
                device,
                interactions,
                session,
                ..
            } = &mut *state;
            session.update(&host.dom, styles, device, interactions, &pending[start..]);
        }
        state.mutation_cursor = end;

        if state.render_cursor == end
            && let Some((cached_viewport, frame)) = &state.cached_frame
            && *cached_viewport == viewport
        {
            let mut displayed = frame.clone().translated(-state.scroll.0, -state.scroll.1);
            push_selection_overlay(&mut displayed, &state);
            return Ok(displayed);
        }

        let styles = {
            let LiveryState {
                styles,
                device,
                interactions,
                session,
                image_sources,
                ..
            } = &mut *state;
            resolve_container_query_styles_with_images(
                &host.dom,
                session.styles(),
                styles,
                device,
                interactions,
                image_sources,
            )?
        };
        let (styles, fragments) = {
            let viewport_sizes = state.device.viewport_sizes;
            let LiveryState {
                text,
                image_sources,
                ..
            } = &mut *state;
            layout_with_text_system(
                &host.dom,
                &styles,
                viewport.0 as f32,
                viewport.1 as f32,
                viewport_sizes,
                text,
                image_sources,
            )?
        };
        let content_extent = document_content_extent(&host.dom, &fragments);
        state.scroll.0 = state
            .scroll
            .0
            .clamp(0.0, (content_extent.0 - viewport.0 as f32).max(0.0));
        state.scroll.1 = state
            .scroll
            .1
            .clamp(0.0, (content_extent.1 - viewport.1 as f32).max(0.0));
        state.render_generation = state.render_generation.saturating_add(1);
        let generation = state.render_generation;
        let frame = {
            let external_textures = state.external_textures.clone();
            let LiveryState {
                text,
                image_sources,
                ..
            } = &mut *state;
            emit_paint_list_with_text_system_scrolled_with_images_and_external_textures(
                &host.dom,
                &styles,
                &fragments,
                DeviceIntSize::new(viewport.0 as i32, viewport.1 as i32),
                generation,
                text,
                &HashMap::new(),
                image_sources,
                &external_textures,
            )
        };
        state.render_cursor = end;
        state.cached_frame = Some((viewport, frame.clone()));
        state.frame = Some(LiveFrame {
            viewport,
            styles,
            fragments,
            content_extent,
        });
        let mut displayed = frame.translated(-state.scroll.0, -state.scroll.1);
        push_selection_overlay(&mut displayed, &state);
        drop(state);

        let mut drained = Vec::new();
        host.dom.drain_mutations(&mut drained);
        Ok(displayed)
    }

    /// The runtime-visible viewport offset retained by this Livery session.
    pub fn scroll(&self) -> (f32, f32) {
        self.state.borrow().scroll
    }

    /// The most recently rendered viewport. It is `(0, 0)` before the first
    /// frame so keyboard scrolling remains a no-op until geometry exists.
    pub fn viewport(&self) -> (u32, u32) {
        self.state
            .borrow()
            .frame
            .as_ref()
            .map_or((0, 0), |frame| frame.viewport)
    }

    /// Apply a wheel delta to the document viewport. Nested overflow routing
    /// remains in `LiveryDocument`; this scripted bridge has one live viewport
    /// and never creates a second retained DOM.
    pub fn scroll_by(&self, dx: f32, dy: f32) -> bool {
        let mut state = self.state.borrow_mut();
        let Some(frame) = state.frame.as_ref() else {
            return false;
        };
        let next = (
            (state.scroll.0 + dx).clamp(
                0.0,
                (frame.content_extent.0 - frame.viewport.0 as f32).max(0.0),
            ),
            (state.scroll.1 + dy).clamp(
                0.0,
                (frame.content_extent.1 - frame.viewport.1 as f32).max(0.0),
            ),
        );
        let moved = next != state.scroll;
        state.scroll = next;
        moved
    }

    /// Reconcile a script-requested viewport position against the current
    /// Livery frame. Before the first frame, the request is retained and is
    /// clamped once geometry exists.
    pub fn scroll_to(&self, x: f32, y: f32) {
        let mut state = self.state.borrow_mut();
        let Some(frame) = state.frame.as_ref() else {
            state.scroll = (x.max(0.0), y.max(0.0));
            return;
        };
        state.scroll = (
            x.clamp(
                0.0,
                (frame.content_extent.0 - frame.viewport.0 as f32).max(0.0),
            ),
            y.clamp(
                0.0,
                (frame.content_extent.1 - frame.viewport.1 as f32).max(0.0),
            ),
        );
    }

    /// Scroll a live element's retained fragment into view. Returns false if
    /// no Livery frame has established the target's geometry yet.
    pub fn scroll_to_id(&self, id: NodeId) -> bool {
        let y = {
            let state = self.state.borrow();
            state
                .frame
                .as_ref()
                .and_then(|frame| frame.fragments.get(id).map(|rect| rect.y))
        };
        let Some(y) = y else {
            return false;
        };
        self.scroll_to(0.0, y);
        true
    }

    /// Resolve the first pending URL Text Directive against the current retained
    /// live DOM, retain its selection indication, and reveal its first rect.
    /// Returns true when the match was already in view as well as when scrolling
    /// changed, so navigation can distinguish success from element fallback.
    pub fn activate_text_directives(&self, directives: &[TextDirective]) -> bool {
        let target = {
            let state = self.state.borrow();
            let frame = match state.frame.as_ref() {
                Some(frame) => frame,
                None => return false,
            };
            let range = match directives
                .iter()
                .find_map(|directive| frame.fragments.text_range_for_text_directive(directive))
            {
                Some(range) => range,
                None => return false,
            };
            let selection = match frame.fragments.text_selection(range) {
                Some(selection) => selection,
                None => return false,
            };
            let first = match selection.rects.first() {
                Some(first) => first,
                None => return false,
            };
            (
                range,
                (first.y - 24.0).max(0.0),
                (frame.content_extent.1 - frame.viewport.1 as f32).max(0.0),
            )
        };
        let mut state = self.state.borrow_mut();
        state.selection_anchor = None;
        state.selection_range = Some(target.0);
        state.scroll = (0.0, target.1.clamp(0.0, target.2));
        true
    }

    /// The node's retained outer fragment in viewport coordinates.
    pub fn fragment_rect(&self, id: NodeId) -> Option<[f32; 4]> {
        let state = self.state.borrow();
        let fragment = state.frame.as_ref()?.fragments.get(id)?;
        Some([
            fragment.x - state.scroll.0,
            fragment.y - state.scroll.1,
            fragment.width,
            fragment.height,
        ])
    }

    /// Hit-test the most recently rendered live frame without dispatching an
    /// event. Test drivers use this to resolve typed pointer and wheel targets;
    /// product clicks continue through [`Self::click_at`].
    pub fn hit_test<E: ScriptEngine>(
        &self,
        runtime: &Runtime<E>,
        x: f32,
        y: f32,
    ) -> Option<NodeId> {
        let host = runtime.host().borrow();
        let state = self.state.borrow();
        let frame = state.frame.as_ref()?;
        hit_test(
            &host.dom,
            &frame.styles,
            &frame.fragments,
            x + state.scroll.0,
            y + state.scroll.1,
        )
        .and_then(|node| pointer_event_target(&host.dom, node))
    }

    /// Resolve the stable activation owner for a press/release gesture.
    ///
    /// A hit inside an anchor can move between an inline child and the anchor's
    /// own padding while remaining the same activation. Non-link targets retain
    /// their ordinary pointer-event element identity.
    pub fn activation_target_at<E: ScriptEngine>(
        &self,
        runtime: &Runtime<E>,
        x: f32,
        y: f32,
    ) -> Option<NodeId> {
        let target = self.hit_test(runtime, x, y)?;
        let host = runtime.host().borrow();
        Some(link_target(&host.dom, target).unwrap_or(target))
    }

    /// Begin a primary-pointer selection against the live shaped frame.
    pub fn begin_text_selection(&self, x: f32, y: f32) -> bool {
        let mut state = self.state.borrow_mut();
        state.selection_range = None;
        state.selection_anchor = state.frame.as_ref().and_then(|frame| {
            frame
                .fragments
                .text_position_at_point(x + state.scroll.0, y + state.scroll.1)
        });
        state.selection_anchor.is_some()
    }

    /// Extend the current primary-pointer selection.
    pub fn extend_text_selection(&self, x: f32, y: f32) -> bool {
        let mut state = self.state.borrow_mut();
        let Some(anchor) = state.selection_anchor else {
            return false;
        };
        let Some(focus) = state.frame.as_ref().and_then(|frame| {
            frame
                .fragments
                .text_position_at_point(x + state.scroll.0, y + state.scroll.1)
        }) else {
            return false;
        };
        let next = TextRange {
            anchor_node: anchor.0,
            anchor_offset: anchor.1,
            focus_node: focus.0,
            focus_offset: focus.1,
        };
        if state.selection_range == Some(next) {
            return false;
        }
        state.selection_range = Some(next);
        true
    }

    /// Finish a pointer selection. A collapsed gesture remains an ordinary
    /// click and therefore clears the pending range.
    pub fn finish_text_selection(&self, x: f32, y: f32) -> bool {
        self.extend_text_selection(x, y);
        let mut state = self.state.borrow_mut();
        state.selection_anchor = None;
        if live_text_selection(&state).is_some() {
            true
        } else {
            state.selection_range = None;
            false
        }
    }

    /// Drop an unfinished or retained selection when the host cancels input.
    pub fn cancel_text_selection(&self) -> bool {
        let mut state = self.state.borrow_mut();
        let had_selection =
            state.selection_anchor.take().is_some() || state.selection_range.is_some();
        state.selection_range = None;
        had_selection
    }

    /// Recompute live selection text and viewport geometry from the retained
    /// source range.
    pub fn text_selection(&self) -> Option<TextSelection<NodeId>> {
        live_text_selection(&self.state.borrow())
    }

    /// Resolve the first shaped occurrence of `text` to pointer endpoints.
    pub fn text_target(&self, text: &str) -> Option<([f32; 2], [f32; 2])> {
        let state = self.state.borrow();
        let frame = state.frame.as_ref()?;
        let range = frame.fragments.text_range_for_text(text)?;
        let anchor = frame
            .fragments
            .caret_rect(range.anchor_node, range.anchor_offset)?;
        let focus = frame
            .fragments
            .caret_rect(range.focus_node, range.focus_offset)?;
        Some((
            [
                anchor.x - state.scroll.0,
                anchor.y - state.scroll.1 + anchor.height * 0.5,
            ],
            [
                focus.x - state.scroll.0,
                focus.y - state.scroll.1 + focus.height * 0.5,
            ],
        ))
    }

    /// Compatibility boolean for callers that do not own navigation. Hosts
    /// should consume [`Self::click_at_result`] so external links cross a typed
    /// document-replacement seam.
    pub fn click_at<E: ScriptEngine>(&self, runtime: &mut Runtime<E>, x: f32, y: f32) -> bool {
        !matches!(self.click_at_result(runtime, x, y), ScriptedClick::Miss)
    }

    /// Dispatch a click at the Livery hit-tested live node. The runtime owns
    /// event propagation; Livery supplies the geometry and returns the default
    /// action only after a listener has had a chance to prevent it.
    pub fn click_at_result<E: ScriptEngine>(
        &self,
        runtime: &mut Runtime<E>,
        x: f32,
        y: f32,
    ) -> ScriptedClick {
        let target = {
            let host = runtime.host().borrow();
            let state = self.state.borrow();
            let Some(frame) = state.frame.as_ref() else {
                return ScriptedClick::Miss;
            };
            hit_test(
                &host.dom,
                &frame.styles,
                &frame.fragments,
                x + state.scroll.0,
                y + state.scroll.1,
            )
        };
        let target = target.and_then(|node| {
            let host = runtime.host().borrow();
            pointer_event_target(&host.dom, node)
        });
        let Some(target) = target else {
            return ScriptedClick::Miss;
        };
        let proceed = runtime
            .dispatch_event(target.raw(), "click")
            .unwrap_or(true);
        if !proceed {
            return ScriptedClick::Handled;
        }
        let href = {
            let host = runtime.host().borrow();
            link_href(&host.dom, target)
        };
        let Some(href) = href else {
            return ScriptedClick::Handled;
        };
        if let Some(fragment) = href.strip_prefix('#') {
            if !fragment.is_empty()
                && let Some(target) = {
                    let host = runtime.host().borrow();
                    find_element_id(&host.dom, host.dom.document(), fragment)
                }
            {
                let _ = self.scroll_to_id(target);
            }
            ScriptedClick::Handled
        } else {
            ScriptedClick::Navigate(href)
        }
    }
}

/// The selection seam over the live Livery frame.
///
/// Script owns the selection: the bootstrap's `Selection` holds the one live
/// `Range`. `state.selection_range` is that range *projected* into rendered
/// byte space, and it is written only from here — the same field a pointer
/// gesture writes, so the painted selection and the scripted one are never two
/// authorities. `range_rects` reads the same range-rect primitive the overlay
/// paints from, so geometry and paint cannot disagree either.
struct LiverySelection {
    host: Weak<RefCell<HostState>>,
    state: Rc<RefCell<LiveryState>>,
}

/// UTF-16 code units (what script counts) to a byte offset (what shaped text is
/// indexed by).
fn utf16_to_byte(text: &str, offset: usize) -> usize {
    let mut units = 0usize;
    for (index, ch) in text.char_indices() {
        if units >= offset {
            return index;
        }
        units += ch.len_utf16();
    }
    text.len()
}

fn first_text(dom: &ScriptedDom, node: NodeId) -> Option<NodeId> {
    if matches!(dom.kind(node), NodeKind::Text) {
        return Some(node);
    }
    dom.dom_children(node)
        .find_map(|child| first_text(dom, child))
}

fn last_text(dom: &ScriptedDom, node: NodeId) -> Option<NodeId> {
    if matches!(dom.kind(node), NodeKind::Text) {
        return Some(node);
    }
    let children: Vec<NodeId> = dom.dom_children(node).collect();
    children
        .into_iter()
        .rev()
        .find_map(|child| last_text(dom, child))
}

/// A DOM boundary point as a shaped-text position. A container boundary has no
/// shaped text of its own, so it resolves to the nearest text node on the side
/// the boundary faces.
fn resolve_text_point(
    dom: &ScriptedDom,
    node: NodeId,
    offset: usize,
    start_side: bool,
) -> Option<(NodeId, usize)> {
    if matches!(dom.kind(node), NodeKind::Text) {
        return Some((node, utf16_to_byte(dom.text(node).unwrap_or(""), offset)));
    }
    let children: Vec<NodeId> = dom.dom_children(node).collect();
    let forward = |children: &[NodeId]| {
        children
            .iter()
            .skip(offset)
            .find_map(|&child| first_text(dom, child))
            .map(|text| (text, 0usize))
    };
    let backward = |children: &[NodeId]| {
        children
            .iter()
            .take(offset)
            .rev()
            .find_map(|&child| last_text(dom, child))
            .map(|text| (text, dom.text(text).unwrap_or("").len()))
    };
    if start_side {
        forward(&children).or_else(|| backward(&children))
    } else {
        backward(&children).or_else(|| forward(&children))
    }
}

impl LiverySelection {
    fn to_text_range(
        &self,
        start: u64,
        start_offset: u32,
        end: u64,
        end_offset: u32,
    ) -> Option<TextRange<NodeId>> {
        let host = self.host.upgrade()?;
        let host = host.borrow();
        let start = NodeId::from_raw(start);
        let end = NodeId::from_raw(end);
        if !host.dom.is_live(start) || !host.dom.is_live(end) {
            return None;
        }
        let (anchor_node, anchor_offset) =
            resolve_text_point(&host.dom, start, start_offset as usize, true)?;
        let (focus_node, focus_offset) =
            resolve_text_point(&host.dom, end, end_offset as usize, false)?;
        Some(TextRange {
            anchor_node,
            anchor_offset,
            focus_node,
            focus_offset,
        })
    }
}

impl SelectionHandler for LiverySelection {
    fn range_rects(
        &self,
        start: u64,
        start_offset: u32,
        end: u64,
        end_offset: u32,
    ) -> Vec<[f32; 4]> {
        let Some(range) = self.to_text_range(start, start_offset, end, end_offset) else {
            return Vec::new();
        };
        let state = self.state.borrow();
        let Some(selection) = state
            .frame
            .as_ref()
            .and_then(|frame| frame.fragments.text_selection(range))
        else {
            return Vec::new();
        };
        selection
            .rects
            .into_iter()
            .map(|rect| {
                [
                    rect.x - state.scroll.0,
                    rect.y - state.scroll.1,
                    rect.width,
                    rect.height,
                ]
            })
            .collect()
    }

    fn set_visual_selection(&self, range: Option<(u64, u32, u64, u32)>) {
        let resolved = range.and_then(|(start, start_offset, end, end_offset)| {
            self.to_text_range(start, start_offset, end, end_offset)
        });
        let mut state = self.state.borrow_mut();
        state.selection_anchor = None;
        state.selection_range = resolved;
    }
}

fn live_text_selection(state: &LiveryState) -> Option<TextSelection<NodeId>> {
    let mut selection = state
        .frame
        .as_ref()?
        .fragments
        .text_selection(state.selection_range?)?;
    for rect in &mut selection.rects {
        rect.x -= state.scroll.0;
        rect.y -= state.scroll.1;
    }
    Some(selection)
}

fn push_selection_overlay(displayed: &mut LiveryPaintList, state: &LiveryState) {
    if let Some(selection) = live_text_selection(state) {
        for rect in selection.rects {
            if rect.width > 0.0 && rect.height > 0.0 {
                displayed.push_overlay_rect(
                    LayoutRect::from_origin_and_size(
                        LayoutPoint::new(rect.x, rect.y),
                        LayoutSize::new(rect.width, rect.height),
                    ),
                    SELECTION_COLOR,
                );
            }
        }
    }
}
fn document_content_extent<D: LayoutDom>(dom: &D, fragments: &LiveryLayout<D::NodeId>) -> (f32, f32)
where
    D::NodeId: Copy + Eq + std::hash::Hash,
{
    fn visit<D: LayoutDom>(
        dom: &D,
        fragments: &LiveryLayout<D::NodeId>,
        node: D::NodeId,
        extent: &mut (f32, f32),
    ) where
        D::NodeId: Copy + Eq + std::hash::Hash,
    {
        if let Some(fragment) = fragments.get(node) {
            extent.0 = extent.0.max(fragment.x + fragment.width);
            extent.1 = extent.1.max(fragment.y + fragment.height);
        }
        for child in dom.dom_children(node) {
            visit(dom, fragments, child, extent);
        }
    }

    let mut extent = (0.0, 0.0);
    visit(dom, fragments, dom.document(), &mut extent);
    extent
}

fn link_target<D: LayoutDom>(dom: &D, mut node: D::NodeId) -> Option<D::NodeId> {
    loop {
        if dom.kind(node) == NodeKind::Element
            && dom
                .element_name(node)
                .is_some_and(|name| name.local.as_ref().eq_ignore_ascii_case("a"))
            && dom
                .attribute(node, &Namespace::default(), &LocalName::from("href"))
                .is_some()
        {
            return Some(node);
        }
        node = dom.parent(node)?;
    }
}

fn link_href<D: LayoutDom>(dom: &D, node: D::NodeId) -> Option<String> {
    let link = link_target(dom, node)?;
    dom.attribute(link, &Namespace::default(), &LocalName::from("href"))
        .map(str::to_owned)
}
/// A layout hit can land on a text fragment, but DOM pointer listeners are
/// attached to elements. Walk that text node to the nearest element before
/// dispatching through the scripted runtime.
fn pointer_event_target<D: LayoutDom>(dom: &D, mut node: D::NodeId) -> Option<D::NodeId> {
    loop {
        if dom.kind(node) == NodeKind::Element {
            return Some(node);
        }
        node = dom.parent(node)?;
    }
}

fn find_element_id<D: LayoutDom>(dom: &D, node: D::NodeId, id: &str) -> Option<D::NodeId> {
    if dom.kind(node) == NodeKind::Element
        && dom.attribute(node, &Namespace::default(), &LocalName::from("id")) == Some(id)
    {
        return Some(node);
    }
    dom.dom_children(node)
        .find_map(|child| find_element_id(dom, child, id))
}

struct LiveryInlineStyle;

impl InlineStyleHandler for LiveryInlineStyle {
    fn canonicalize(&self, property: &str, value: &str) -> InlineStyleValueResult {
        if is_implemented_shorthand(property) {
            return match classify_specified_shorthand(property, value) {
                InlineShorthandExpansion::Expanded(components) => InlineStyleValueResult::Expanded(
                    components
                        .into_iter()
                        .map(|component| (component.name.to_string(), component.value))
                        .collect(),
                ),
                InlineShorthandExpansion::Deferred => InlineStyleValueResult::SupportedDeferred,
                InlineShorthandExpansion::Invalid => InlineStyleValueResult::Invalid,
            };
        }
        if let Some(value) = canonicalize_specified_value(property, value) {
            InlineStyleValueResult::Canonical(value)
        } else if genet_livery::PropertyId::from_css_name(&property.to_ascii_lowercase()).is_some()
            && !value.to_ascii_lowercase().contains("var(")
        {
            InlineStyleValueResult::Invalid
        } else {
            InlineStyleValueResult::PassThrough
        }
    }

    fn shorthand_components(&self, property: &str) -> Vec<String> {
        specified_shorthand_longhands(property)
            .unwrap_or_default()
            .iter()
            .map(|name| (*name).to_string())
            .collect()
    }

    fn shorthands(&self) -> Vec<String> {
        vec!["flex".to_string(), "flex-flow".to_string()]
    }

    fn reconstruct_shorthand(
        &self,
        property: &str,
        components: &[(String, String)],
    ) -> Option<String> {
        reconstruct_specified_shorthand(property, components)
    }
}

/// The host side of `script_runtime_api`'s media-query seam on the live route:
/// evaluates a `matchMedia` string against Livery's own retained `Device`,
/// which is the device the frame was laid out with. This is the seam the
/// retired `MediaQueryBridge` filled over `genet_layout::IncrementalLayout`;
/// without it `matchMedia(q).matches` was `false` for every query.
///
/// `livery::media` has no serializer for a parsed query, so the normalized form
/// is the trimmed input — enough for `MediaQueryList.media` to be non-empty and
/// stable, and honest about being no more than that.
struct LiveryMediaQueries {
    state: Rc<RefCell<LiveryState>>,
}

impl MediaQueryHandler for LiveryMediaQueries {
    fn evaluate(&self, query: &str) -> (String, bool) {
        let serialized = query.trim().to_owned();
        let matches = serialized
            .parse::<MediaQueryList>()
            .is_ok_and(|list| list.matches(&self.state.borrow().device));
        (serialized, matches)
    }
}

struct LiveryComputedStyle {
    host: Weak<RefCell<HostState>>,
    state: Rc<RefCell<LiveryState>>,
}

fn needs_used_values(property: &str) -> bool {
    matches!(
        property.to_ascii_lowercase().as_str(),
        "width" | "height" | "margin-top" | "margin-right" | "margin-bottom" | "margin-left"
    )
}

impl ComputedStyleHandler for LiveryComputedStyle {
    fn computed_value(&self, node: u64, property: &str) -> Option<String> {
        let host = self.host.upgrade()?;
        let host = host.borrow();
        let (base, pending) = host.dom.pending_mutations();
        let end = base.saturating_add(pending.len() as u64);
        let mut state = self.state.borrow_mut();
        synchronize_live_styles(&host, &mut state);
        if state.mutation_cursor < base || state.mutation_cursor > end {
            state.session.invalidate();
            state.mutation_cursor = base;
        }
        let start = state.mutation_cursor.saturating_sub(base) as usize;
        let LiveryState {
            styles,
            device,
            interactions,
            session,
            ..
        } = &mut *state;
        session.update(&host.dom, styles, device, interactions, &pending[start..]);
        let node = NodeId::from_raw(node);
        let container_resolved = resolve_container_query_styles(
            &host.dom,
            session.styles(),
            styles,
            device,
            interactions,
        )
        .ok();
        let computed_styles = container_resolved
            .as_ref()
            .unwrap_or_else(|| session.styles());
        let used = needs_used_values(property)
            .then(|| {
                layout_used_value_context(
                    &host.dom,
                    computed_styles,
                    device.viewport_width,
                    device.viewport_height,
                    node,
                )
                .ok()
                .flatten()
            })
            .flatten();
        let value = computed_styles.computed_style_with_used_values(node, property, used);
        state.mutation_cursor = end;
        value
    }

    fn computed_value_in_context(&self, context: u64, node: u64, property: &str) -> Option<String> {
        let host = self.host.upgrade()?;
        let host = host.borrow();
        let (base, pending) = host.dom.pending_mutations();
        let end = base.saturating_add(pending.len() as u64);
        let mut state = self.state.borrow_mut();
        synchronize_live_styles(&host, &mut state);
        if state.mutation_cursor < base || state.mutation_cursor > end {
            state.session.invalidate();
            state.mutation_cursor = base;
        }
        let start = state.mutation_cursor.saturating_sub(base) as usize;
        let LiveryState {
            styles,
            device,
            interactions,
            session,
            mutation_cursor,
            ..
        } = &mut *state;
        session.update(&host.dom, styles, device, interactions, &pending[start..]);
        *mutation_cursor = end;

        let primary = resolve_container_query_styles(
            &host.dom,
            session.styles(),
            styles,
            device,
            interactions,
        )
        .ok();
        let primary = primary.as_ref().unwrap_or_else(|| session.styles());
        let context = NodeId::from_raw(context);
        let fragments = layout(
            &host.dom,
            primary,
            device.viewport_width,
            device.viewport_height,
        )
        .ok()?;
        let frame_style = primary.get(context)?;
        let frame_fragment = fragments.get(context)?;
        let (width, height) = content_box_size(frame_style, frame_fragment);

        let node = NodeId::from_raw(node);
        let document = owning_document(&host.dom, node)?;
        if document == host.dom.document() {
            let used = needs_used_values(property)
                .then(|| {
                    layout_used_value_context(
                        &host.dom,
                        primary,
                        device.viewport_width,
                        device.viewport_height,
                        node,
                    )
                    .ok()
                    .flatten()
                })
                .flatten();
            return primary.computed_style_with_used_values(node, property, used);
        }
        let scoped = ScopedDom {
            dom: &host.dom,
            document,
        };
        let sheets = inline_stylesheets(&scoped);
        let sheet_refs = sheets.iter().map(String::as_str).collect::<Vec<_>>();
        let child_styles = StyleSet::cambium(&sheet_refs);
        let child_device = Device::screen(width, height);
        let child_interactions = InteractionStates::default();
        let child_plane =
            resolve_styles(&scoped, &child_styles, &child_device, &child_interactions);
        let child_plane = resolve_container_query_styles(
            &scoped,
            &child_plane,
            &child_styles,
            &child_device,
            &child_interactions,
        )
        .ok()?;
        let used = needs_used_values(property)
            .then(|| {
                layout_used_value_context(&scoped, &child_plane, width, height, node)
                    .ok()
                    .flatten()
            })
            .flatten();
        child_plane.computed_style_with_used_values(node, property, used)
    }
}

struct ScopedDom<'a> {
    dom: &'a ScriptedDom,
    document: NodeId,
}

impl LayoutDom for ScopedDom<'_> {
    type NodeId = NodeId;

    fn document(&self) -> Self::NodeId {
        self.document
    }

    fn is_live(&self, id: Self::NodeId) -> bool {
        self.dom.is_live(id)
    }

    fn quirks_mode(&self) -> QuirksMode {
        self.dom.quirks_mode()
    }

    fn parent(&self, id: Self::NodeId) -> Option<Self::NodeId> {
        self.dom.parent(id)
    }

    fn prev_sibling(&self, id: Self::NodeId) -> Option<Self::NodeId> {
        self.dom.prev_sibling(id)
    }

    fn next_sibling(&self, id: Self::NodeId) -> Option<Self::NodeId> {
        self.dom.next_sibling(id)
    }

    fn dom_children(&self, id: Self::NodeId) -> impl Iterator<Item = Self::NodeId> + '_ {
        self.dom.dom_children(id)
    }

    fn kind(&self, id: Self::NodeId) -> NodeKind {
        self.dom.kind(id)
    }

    fn opaque_id(&self, id: Self::NodeId) -> u64 {
        self.dom.opaque_id(id)
    }

    fn element_name(&self, id: Self::NodeId) -> Option<&QualName> {
        self.dom.element_name(id)
    }

    fn attribute(
        &self,
        id: Self::NodeId,
        namespace: &Namespace,
        local: &LocalName,
    ) -> Option<&str> {
        self.dom.attribute(id, namespace, local)
    }

    fn attributes(&self, id: Self::NodeId) -> impl Iterator<Item = AttributeView<'_>> + '_ {
        self.dom.attributes(id)
    }

    fn text(&self, id: Self::NodeId) -> Option<&str> {
        self.dom.text(id)
    }
}

fn owning_document(dom: &ScriptedDom, mut node: NodeId) -> Option<NodeId> {
    loop {
        if dom.kind(node) == NodeKind::Document {
            return Some(node);
        }
        node = dom.parent(node)?;
    }
}

fn inline_stylesheets(dom: &impl LayoutDom) -> Vec<String> {
    fn text_content<D: LayoutDom>(dom: &D, node: D::NodeId, output: &mut String) {
        if dom.kind(node) == NodeKind::Text {
            output.push_str(dom.text(node).unwrap_or(""));
        }
        for child in dom.dom_children(node) {
            text_content(dom, child, output);
        }
    }

    fn collect<D: LayoutDom>(dom: &D, node: D::NodeId, output: &mut Vec<String>) {
        if dom
            .element_name(node)
            .is_some_and(|name| name.local.as_ref() == "style")
        {
            let mut sheet = String::new();
            text_content(dom, node, &mut sheet);
            output.push(sheet);
        }
        for child in dom.dom_children(node) {
            collect(dom, child, output);
        }
    }

    let mut sheets = Vec::new();
    collect(dom, dom.document(), &mut sheets);
    sheets
}

struct LiveryStyleSheets {
    state: Rc<RefCell<LiveryState>>,
    host: Weak<RefCell<HostState>>,
}

impl LiveryStyleSheets {
    fn synchronize(&self) {
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let host = host.borrow();
        synchronize_live_styles(&host, &mut self.state.borrow_mut());
    }

    fn author_index(&self, sheet: usize) -> Option<usize> {
        self.state.borrow().document_sheets.get(sheet).copied()
    }

    fn author_index_by_key(&self, key: &str) -> Option<usize> {
        self.state.borrow().author_index_for_key(key)
    }
}

impl StyleSheetHandler for LiveryStyleSheets {
    fn sheet_count(&self) -> usize {
        self.synchronize();
        self.state.borrow().document_sheets.len()
    }

    fn rule_count(&self, sheet: usize) -> Option<usize> {
        self.synchronize();
        let sheet = self.author_index(sheet)?;
        self.state.borrow().styles.cssom_rule_count(sheet)
    }

    fn insert_rule(
        &self,
        sheet: usize,
        rule: &str,
        index: usize,
    ) -> Result<usize, StyleSheetMutationError> {
        self.synchronize();
        let sheet = self
            .author_index(sheet)
            .ok_or(StyleSheetMutationError::IndexSize)?;
        let mut state = self.state.borrow_mut();
        let result = state.styles.insert_cssom_rule(sheet, rule, index);
        if result.is_ok() {
            state.session.invalidate();
            state.invalidate_frame();
        }
        result.map_err(mutation_error)
    }

    fn delete_rule(&self, sheet: usize, index: usize) -> Result<(), StyleSheetMutationError> {
        self.synchronize();
        let sheet = self
            .author_index(sheet)
            .ok_or(StyleSheetMutationError::IndexSize)?;
        let mut state = self.state.borrow_mut();
        let result = state.styles.delete_cssom_rule(sheet, index);
        if result.is_ok() {
            state.session.invalidate();
            state.invalidate_frame();
        }
        result.map_err(mutation_error)
    }

    fn sheet_key(&self, sheet: usize) -> Option<String> {
        self.synchronize();
        self.state.borrow().sheet_key_at(sheet)
    }

    fn rule_count_by_key(&self, key: &str) -> Option<usize> {
        self.synchronize();
        let sheet = self.author_index_by_key(key)?;
        self.state.borrow().styles.cssom_rule_count(sheet)
    }

    fn insert_rule_by_key(
        &self,
        key: &str,
        rule: &str,
        index: usize,
    ) -> Result<usize, StyleSheetMutationError> {
        self.synchronize();
        let sheet = self
            .author_index_by_key(key)
            .ok_or(StyleSheetMutationError::IndexSize)?;
        let mut state = self.state.borrow_mut();
        let result = state.styles.insert_cssom_rule(sheet, rule, index);
        if result.is_ok() {
            state.session.invalidate();
            state.invalidate_frame();
        }
        result.map_err(mutation_error)
    }

    fn delete_rule_by_key(&self, key: &str, index: usize) -> Result<(), StyleSheetMutationError> {
        self.synchronize();
        let sheet = self
            .author_index_by_key(key)
            .ok_or(StyleSheetMutationError::IndexSize)?;
        let mut state = self.state.borrow_mut();
        let result = state.styles.delete_cssom_rule(sheet, index);
        if result.is_ok() {
            state.session.invalidate();
            state.invalidate_frame();
        }
        result.map_err(mutation_error)
    }

    fn owner_node_by_key(&self, key: &str) -> Option<u64> {
        self.synchronize();
        let sheet = self.author_index_by_key(key)?;
        self.state
            .borrow()
            .styles
            .author_sheets()
            .get(sheet)
            .and_then(|sheet| {
                (sheet.owner() != genet_document_resources::StylesheetOwner::Imported)
                    .then(|| sheet.owner_node())
                    .flatten()
            })
    }

    fn import_rule_by_key(&self, key: &str, index: usize) -> Option<StyleSheetImportRule> {
        self.synchronize();
        let sheet = self.author_index_by_key(key)?;
        self.state
            .borrow()
            .styles
            .cssom_import_rule(sheet, index)
            .map(|rule| StyleSheetImportRule {
                href: rule.href,
                media: rule.media,
                child_sheet_key: rule.child_sheet_key,
            })
    }

    fn import_owner_by_key(&self, key: &str) -> Option<StyleSheetImportOwner> {
        self.synchronize();
        let sheet = self.author_index_by_key(key)?;
        self.state
            .borrow()
            .styles
            .cssom_import_owner(sheet)
            .map(|owner| StyleSheetImportOwner {
                parent_sheet_key: owner.parent_sheet_key,
                import_index: owner.import_index,
            })
    }

    fn rule_by_key(&self, key: &str, path: &[usize]) -> Option<StyleSheetRule> {
        self.synchronize();
        let sheet = self.author_index_by_key(key)?;
        let rule = self.state.borrow().styles.cssom_rule(sheet, path)?;
        let kind = match rule.kind {
            CssomRuleKind::Style => StyleSheetRuleKind::Style,
            CssomRuleKind::Import => StyleSheetRuleKind::Import,
            CssomRuleKind::FontFace => StyleSheetRuleKind::FontFace,
            CssomRuleKind::Media => StyleSheetRuleKind::Media,
            CssomRuleKind::Container => StyleSheetRuleKind::Container,
            CssomRuleKind::Keyframes => StyleSheetRuleKind::Keyframes,
            CssomRuleKind::Keyframe => StyleSheetRuleKind::Keyframe,
        };
        Some(StyleSheetRule {
            kind,
            css_text: rule.css_text,
            selector_text: rule.selector_text,
            style_text: rule.style_text,
            condition_text: rule.condition_text,
            name: rule.name,
            key_text: rule.key_text,
            child_count: rule.children.len(),
        })
    }
}

fn mutation_error(error: RuleMutationError) -> StyleSheetMutationError {
    match error {
        RuleMutationError::IndexSize => StyleSheetMutationError::IndexSize,
        RuleMutationError::Syntax(message) => StyleSheetMutationError::Syntax(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genet_livery::ViewportSize;
    use genet_static_dom::StaticDocument;
    use layout_dom_api::LayoutDomMut;
    use paint_list_api::{ColorF, PaintCmd, PaintList};
    use script_engine_boa::BoaEngine;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct LiveSheetFetch;

    impl ResourceFetcher for LiveSheetFetch {
        fn fetch(&self, url: &str) -> Option<Vec<u8>> {
            match url {
                "https://example.test/docs/theme.css" => Some(b".card { color: green; }".to_vec()),
                "https://example.test/docs/replacement.css" => {
                    Some(b".card { color: orange; }".to_vec())
                },
                "https://example.test/docs/parent.css" => {
                    Some(b"@import url('child.css') screen; .host { display: block; }".to_vec())
                },
                "https://example.test/docs/child.css" => Some(b".card { color: blue; }".to_vec()),
                _ => None,
            }
        }
    }

    struct LiveAssetFetch {
        image: Rc<RefCell<Vec<u8>>>,
        font: Rc<RefCell<Vec<u8>>>,
    }

    impl ResourceFetcher for LiveAssetFetch {
        fn fetch(&self, url: &str) -> Option<Vec<u8>> {
            match url {
                "https://example.test/docs/poster.png" => Some(self.image.borrow().clone()),
                "https://example.test/docs/reading.woff2" => Some(self.font.borrow().clone()),
                _ => None,
            }
        }
    }

    struct RecordingResourceSink(Rc<RefCell<Vec<ResourceDelta>>>);

    impl LiveResourceSink for RecordingResourceSink {
        fn replace_resources(
            &mut self,
            _resources: &ResolvedDocumentResources,
            delta: &ResourceDelta,
        ) {
            self.0.borrow_mut().push(delta.clone());
        }
    }

    fn border_rectangles(frame: &LiveryPaintList, color: ColorF) -> Vec<[f32; 4]> {
        frame
            .commands()
            .iter()
            .filter_map(|command| match command {
                PaintCmd::DrawRect(rect) if rect.color == color => Some([
                    rect.placement.bounds.min.x,
                    rect.placement.bounds.min.y,
                    rect.placement.bounds.max.x,
                    rect.placement.bounds.max.y,
                ]),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn boa_livery_frame_consumes_the_same_collapsed_border_batch_as_cssom() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><body><table id='table'><tbody><tr id='row'>\
             <td id='left'>left</td><td id='right'>right</td>\
             </tr></tbody></table></body></html>",
        ));
        let cssom = LiveryCssom::install(
            &mut runtime,
            &[
                "body { margin: 0; } table { border-collapse: collapse; color: black; } \
                 td { width: 60px; height: 30px; border: 4px solid currentcolor; }",
            ],
            Device::screen(220.0, 100.0),
        );

        let initial = cssom
            .frame(&mut runtime, 220, 100)
            .expect("initial collapsed frame");
        let black = border_rectangles(&initial, ColorF::BLACK);
        assert!(!black.is_empty(), "initial frame paints collapsed borders");
        assert!(
            runtime.host().borrow().dom.pending_mutations().1.is_empty(),
            "the completed frame drains its initial DOM batch"
        );
        let cached = cssom.frame(&mut runtime, 220, 100).expect("cached frame");
        assert_eq!(cached.generation_id(), initial.generation_id());

        runtime
            .eval(
                "var table = document.getElementById('table'); \
                 table.style.color = 'red'; \
                 console.log(getComputedStyle(table).color);",
            )
            .expect("scripted color mutation and CSSOM read");
        assert!(
            !runtime.host().borrow().dom.pending_mutations().1.is_empty(),
            "CSSOM observes the batch without stealing it from the frame"
        );
        let recolored = cssom
            .frame(&mut runtime, 220, 100)
            .expect("recolored collapsed frame");
        let red = border_rectangles(&recolored, ColorF::new(1.0, 0.0, 0.0, 1.0));
        assert_eq!(red, black, "color-only mutation preserves geometry");
        assert!(
            runtime.host().borrow().dom.pending_mutations().1.is_empty(),
            "the same batch is drained only after its paint frame"
        );

        runtime
            .eval("document.getElementById('left').style.border = '12px solid red';")
            .expect("scripted winner-width mutation");
        let wider = cssom
            .frame(&mut runtime, 220, 100)
            .expect("wider collapsed frame");
        let wider_red = border_rectangles(&wider, ColorF::new(1.0, 0.0, 0.0, 1.0));
        assert_ne!(wider_red, red, "winner-width mutation rebuilds geometry");

        runtime
            .eval(
                "document.getElementById('row')\
                    .removeChild(document.getElementById('right'));",
            )
            .expect("scripted cell removal");
        let one_cell = cssom
            .frame(&mut runtime, 220, 100)
            .expect("cell removal frame");
        assert_ne!(
            border_rectangles(&one_cell, ColorF::new(1.0, 0.0, 0.0, 1.0)),
            wider_red,
            "removed cells cannot retain prior winner geometry"
        );
        assert_eq!(runtime.host().borrow().console, vec!["rgb(255, 0, 0)"]);
    }

    #[test]
    fn live_cssom_replaces_image_and_font_resources_after_a_dom_reconciliation() {
        let image = Rc::new(RefCell::new(vec![1]));
        let font = Rc::new(RefCell::new(vec![2]));
        let deltas = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><head><style>.card { background-image: url(reading.woff2); }</style></head>\
             <body><img src='poster.png'><div class='card'></div></body></html>",
        ));
        let cssom = LiveryCssom::install_live_with_resource_sink(
            &mut runtime,
            LiveAssetFetch {
                image: image.clone(),
                font: font.clone(),
            },
            "https://example.test/docs/index.html",
            ResourceLimits::default(),
            Device::screen(800.0, 600.0),
            RecordingResourceSink(deltas.clone()),
        );
        assert_eq!(deltas.borrow().len(), 1, "initial ledger is delivered");
        assert_eq!(deltas.borrow()[0].added.len(), 2);

        *image.borrow_mut() = vec![3];
        *font.borrow_mut() = vec![4];
        runtime
            .eval("document.body.setAttribute('data-resource-revision', '2'); document.styleSheets.length;")
            .expect("trigger live resource reconciliation");

        {
            let observed = deltas.borrow();
            assert_eq!(observed.len(), 2);
            assert_eq!(observed[1].updated.len(), 2);
            assert!(observed[1].removed.is_empty());
        }
        let resources = cssom.resource_set().expect("updated resource ledger");
        assert!(
            resources
                .resources
                .iter()
                .any(|resource| resource.bytes == vec![3])
        );
        assert!(
            resources
                .resources
                .iter()
                .any(|resource| resource.bytes == vec![4])
        );

        runtime
            .eval(
                "document.body.removeChild(document.querySelector('img'));\
                 document.head.removeChild(document.querySelector('style'));\
                 document.styleSheets.length;",
            )
            .expect("trigger live resource removal");
        let observed = deltas.borrow();
        assert_eq!(observed.len(), 3);
        assert_eq!(observed[2].removed.len(), 2);
        assert!(
            cssom
                .resource_set()
                .expect("resource removal ledger")
                .resources
                .is_empty()
        );
    }

    #[test]
    fn boa_live_cssom_reconciles_inserted_removed_and_media_gated_sheets() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><head><style id='base'>.card { color: red; }</style></head>\
             <body><div id='card' class='card'></div></body></html>",
        ));
        let cssom = LiveryCssom::install_live(
            &mut runtime,
            LiveSheetFetch,
            "https://example.test/docs/index.html",
            ResourceLimits::default(),
            Device::screen(800.0, 600.0),
        );

        runtime
            .eval(
                "var card = document.getElementById('card');\
                 var head = document.head;\
                 var base = document.styleSheets[0];\
                 console.log(document.styleSheets.length + '|' + base.ownerNode.id + '|' + getComputedStyle(card).color);\
                 base.insertRule('.card { color: purple; }', base.cssRules.length);\
                 console.log(base.cssRules.length + '|' + getComputedStyle(card).color);\
                 var dynamic = document.createElement('style');\
                 dynamic.id = 'dynamic'; dynamic.textContent = '.card { color: blue; }'; head.appendChild(dynamic);\
                 console.log(document.styleSheets.length + '|' + String(document.styleSheets[0] === base));\
                 console.log(document.styleSheets[1].ownerNode.id + '|' + getComputedStyle(card).color);\
                 var link = document.createElement('link');\
                 link.id = 'theme'; link.setAttribute('rel', 'stylesheet'); link.setAttribute('href', 'theme.css'); link.setAttribute('media', 'screen'); head.appendChild(link);\
                 console.log(document.styleSheets.length + '|' + document.styleSheets[2].ownerNode.id + '|' + getComputedStyle(card).color);\
                 var theme = document.styleSheets[2];\
                 link.setAttribute('media', 'print');\
                 console.log(document.styleSheets.length + '|' + getComputedStyle(card).color);\
                 head.removeChild(dynamic);\
                 console.log(document.styleSheets.length + '|' + String(document.styleSheets[0] === base) + '|' + getComputedStyle(card).color);\
                 link.setAttribute('media', 'screen');\
                 console.log(String(document.styleSheets[1] === theme) + '|' + getComputedStyle(card).color);\
                 link.setAttribute('href', 'replacement.css');\
                 console.log(String(document.styleSheets[1] === theme) + '|' + getComputedStyle(card).color);\
                 link.setAttribute('href', 'missing.css');\
                 console.log(document.styleSheets.length + '|' + String(theme.ownerNode === null) + '|' + getComputedStyle(card).color);\
                 head.removeChild(link);\
                 console.log(document.styleSheets.length + '|' + String(document.styleSheets[0] === base) + '|' + getComputedStyle(card).color);",
            )
            .expect("live Livery CSSOM script");

        assert_eq!(
            runtime.host().borrow().console,
            vec![
                "1|base|rgb(255, 0, 0)",
                "2|rgb(128, 0, 128)",
                "2|true",
                "dynamic|rgb(0, 0, 255)",
                "3|theme|rgb(0, 128, 0)",
                "3|rgb(0, 0, 255)",
                "2|true|rgb(128, 0, 128)",
                "true|rgb(0, 128, 0)",
                "true|rgb(255, 165, 0)",
                "1|true|rgb(128, 0, 128)",
                "1|true|rgb(128, 0, 128)",
            ],
        );
        let resources = cssom.resource_set().expect("live resource ledger");
        assert_eq!(resources.stylesheets.len(), 1);
        assert!(resources.stylesheets[0].owner_node.is_some());
    }

    #[test]
    fn boa_live_cssom_exposes_imported_sheet_owner_graph() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><head><link id='theme' rel='stylesheet' href='parent.css'></head>\
             <body><div id='card' class='card'></div></body></html>",
        ));
        let _cssom = LiveryCssom::install_live(
            &mut runtime,
            LiveSheetFetch,
            "https://example.test/docs/index.html",
            ResourceLimits::default(),
            Device::screen(800.0, 600.0),
        );

        runtime
            .eval(
                "var card = document.getElementById('card');\
                 var parentSheet = document.styleSheets[0];\
                 var rule = parentSheet.cssRules.item(0);\
                 var child = rule.styleSheet;\
                 console.log(document.styleSheets.length + '|' + parentSheet.ownerNode.id + '|' + parentSheet.cssRules.length + '|' + String(child.ownerNode === null));\
                 console.log(rule.href + '|' + rule.media.mediaText + '|' + String(rule.parentStyleSheet === parentSheet) + '|' + String(child.ownerRule === rule) + '|' + String(rule instanceof CSSImportRule) + '|' + child.cssRules.length + '|' + getComputedStyle(card).color);\
                 console.log(child.insertRule('.card { color: green; }', child.cssRules.length) + '|' + getComputedStyle(card).color);",
            )
            .expect("import ownership CSSOM script");

        assert_eq!(
            runtime.host().borrow().console,
            vec![
                "1|theme|2|true",
                "child.css|screen|true|true|true|1|rgb(0, 0, 255)",
                "1|rgb(0, 128, 0)",
            ],
        );
    }

    #[test]
    fn boa_cssom_projects_every_livery_rule_object() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse("<html><body></body></html>"));
        LiveryCssom::install(
            &mut runtime,
            &[".root { color:red; }\
               @media screen { .media { color:blue; } }\
               @container (width > 10px) { .container { color:green; } }\
               @keyframes pulse { from { opacity:0; } to { opacity:1; } }"],
            Device::screen(800.0, 600.0),
        );

        runtime
            .eval(
                "var rules = document.styleSheets[0].cssRules;\
                 var style = rules[0], media = rules[1], container = rules[2], frames = rules[3];\
                 console.log(rules.length + '|' + String(style instanceof CSSStyleRule) + '|' + style.type + '|' + style.selectorText + '|' + style.style.color + '|' + style.cssText);\
                 console.log(String(media instanceof CSSMediaRule) + '|' + media.media.mediaText + '|' + media.cssRules.length + '|' + String(media.cssRules[0].parentRule === media) + '|' + media.cssRules[0].selectorText);\
                 console.log(String(container instanceof CSSContainerRule) + '|' + container.conditionText + '|' + container.cssRules.length + '|' + container.cssRules[0].style.color);\
                 console.log(String(frames instanceof CSSKeyframesRule) + '|' + frames.name + '|' + frames.cssRules.length + '|' + String(frames.cssRules[0] instanceof CSSKeyframeRule) + '|' + frames.cssRules[0].keyText + '|' + frames.cssRules[0].style.opacity);",
            )
            .expect("full Livery rule-object script");

        assert_eq!(
            runtime.host().borrow().console,
            vec![
                "4|true|1|.root|red|.root { color:red; }",
                "true|screen|1|true|.media",
                "true|(width > 10px)|1|green",
                "true|pulse|2|true|from|0",
            ],
        );
    }

    #[test]
    fn boa_reaches_livery_stylesheets_mutation_and_computed_values() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><body><div id='card' class='card'></div></body></html>",
        ));
        let cssom = LiveryCssom::install(
            &mut runtime,
            &[".card { --accent: #ff0000; color: var(--accent); }"],
            Device::screen(800.0, 600.0),
        );
        let initial_generation = cssom.generation(0).expect("author sheet");

        runtime
            .eval(
                "var card = document.getElementById('card');\
                 var sheet = document.styleSheets[0];\
                 console.log(document.styleSheets.length + '|' + sheet.cssRules.length + '|' +\
                   getComputedStyle(card).color + '|' +\
                   getComputedStyle(card).getPropertyValue('--accent'));\
                 console.log(sheet.insertRule('.card { --accent: #0000ff; }', 1));\
                 console.log(sheet.cssRules.length + '|' + getComputedStyle(card).color + '|' +\
                   getComputedStyle(card).getPropertyValue('--accent'));\
                 try { sheet.insertRule('.bad {}', 9); } catch (e) { console.log(e.name); }\
                 try { sheet.insertRule('not a rule', 2); } catch (e) { console.log(e.name); }\
                 console.log(sheet.cssRules.length + '|' + getComputedStyle(card).color);\
                 sheet.deleteRule(1);\
                 console.log(sheet.cssRules.length + '|' + getComputedStyle(card).color);",
            )
            .expect("Livery CSSOM script");

        assert_eq!(
            runtime.host().borrow().console,
            vec![
                "1|1|rgb(255, 0, 0)|#ff0000",
                "1",
                "2|rgb(0, 0, 255)|#0000ff",
                "IndexSizeError",
                "SyntaxError",
                "2|rgb(0, 0, 255)",
                "1|rgb(255, 0, 0)",
            ],
        );
        assert_eq!(cssom.generation(0), Some(initial_generation + 2));
    }

    #[test]
    fn boa_cssom_flex_shorthand_receipts_cover_specified_and_computed_reads() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><body><div id='card' class='card'></div></body></html>",
        ));
        LiveryCssom::install(
            &mut runtime,
            &[".card { flex: 2 3 10px; flex-flow: column wrap; }"],
            Device::screen(800.0, 600.0),
        );

        runtime
            .eval(
                "var card = document.getElementById('card');\
                 var s = card.style;\
                 s.flex = '1';\
                 console.log(s.flex + '|' + s.flexGrow + '|' + s.flexShrink + '|' +\
                   s.flexBasis + '|' + s.length);\
                 s.flex = 'none 1';\
                 console.log(s.flex);\
                 s.flexFlow = 'nowrap column';\
                 console.log(s.flexFlow + '|' + s.flexDirection + '|' + s.flexWrap + '|' +\
                   s.length);\
                 s.flexFlow = 'nowrap row nowrap';\
                 console.log(s.flexFlow);\
                 console.log(s.removeProperty('flex') + '|' + s.flex + '|' +\
                   s.flexFlow + '|' + s.length);\
                 s.flex = '1';\
                 console.log(String(CSS.supports('flex', '1')) + '|' +\
                   String(CSS.supports('flex', 'var(--flex)')) + '|' +\
                   String(CSS.supports('flex', 'none 1')) + '|' +\
                   String(CSS.supports('flex-flow', 'column wrap')) + '|' +\
                   String(CSS.supports('flex-flow', 'var(--flow)')) + '|' +\
                   String(CSS.supports('flex-flow', 'column wrap column')));\
                 console.log(getComputedStyle(card).flex + '|' +\
                   getComputedStyle(card).flexFlow);",
            )
            .expect("Livery flex CSSOM script");

        assert_eq!(
            runtime.host().borrow().console,
            vec![
                "1 1 0%|1|1|0%|3",
                "1 1 0%",
                "column|column|nowrap|5",
                "column",
                "1 1 0%||column|2",
                "true|true|false|true|true|false",
                "1 1 0%|column",
            ],
        );
    }

    #[test]
    fn boa_cssom_flex_basis_accepts_intrinsic_keywords_in_scripted_reads() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><body><div id='card' class='card'></div></body></html>",
        ));
        LiveryCssom::install(
            &mut runtime,
            &[".card { flex-basis: content; }"],
            Device::screen(800.0, 600.0),
        );

        runtime
            .eval(
                "var card = document.getElementById('card');\
                 console.log(String(CSS.supports('flex-basis', 'content')) + '|' +\
                   String(CSS.supports('flex-basis', 'fit-content')));\
                 console.log(getComputedStyle(card).flexBasis);\
                 card.style.flexBasis = 'fit-content';\
                 console.log(card.style.flexBasis + '|' + getComputedStyle(card).flexBasis);",
            )
            .expect("Livery flex-basis CSSOM script");

        assert_eq!(
            runtime.host().borrow().console,
            vec!["true|true", "content", "fit-content|fit-content"],
        );
    }

    #[test]
    fn boa_canonicalizes_nested_calc_through_livery_inline_cssom() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><body><div id='target'></div></body></html>",
        ));
        LiveryCssom::install(&mut runtime, &[], Device::screen(800.0, 600.0));

        runtime
            .eval(
                "var s = document.getElementById('target').style;\
                 var values = [\
                   'calc(20px + calc(80px))',\
                   'calc(calc(100px))',\
                   'calc(calc(2) * calc(50px))',\
                   'calc(calc(150px*2/3))',\
                   'calc(calc(2 * calc(calc(3)) + 4) * 10px)',\
                   'calc(50px + calc(40%))'\
                 ];\
                 for (var i = 0; i < values.length; i++) {\
                   s.left = values[i]; console.log(s.left);\
                 }\
                 s.border = 'calc(calc(10px)) solid pink';\
                 console.log(s.border);",
            )
            .expect("Livery inline CSSOM script");

        assert_eq!(
            runtime.host().borrow().console,
            vec![
                "calc(100px)",
                "calc(100px)",
                "calc(100px)",
                "calc(100px)",
                "calc(100px)",
                "calc(40% + 50px)",
                "calc(10px) solid pink",
            ]
        );
    }

    #[test]
    fn boa_resolves_nested_calc_widths_through_livery_layout() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><body><div id='parent'>\
             <div id='div1'></div><div id='div2'></div>\
             <div id='div3'></div><div id='div4'></div>\
             </div></body></html>",
        ));
        LiveryCssom::install(
            &mut runtime,
            &["#parent { width: 200px; }\
                 #div1 { width: calc(calc(50px)); }\
                 #div2 { width: calc(calc(60%) - 20px); }\
                 #div3 { width: calc(calc(3 * 25%)); }\
                 #div4 { --width: calc(10% + 30px); width: calc(2 * var(--width)); }"],
            Device::screen(800.0, 600.0),
        );

        runtime
            .eval(
                "console.log(getComputedStyle(div1).width);\
                 console.log(getComputedStyle(div2).width);\
                 console.log(getComputedStyle(div3).width);\
                 console.log(getComputedStyle(div4).width);",
            )
            .expect("Livery used width script");

        assert_eq!(
            runtime.host().borrow().console,
            vec!["50px", "100px", "150px", "100px"]
        );
    }

    #[test]
    fn boa_reads_advanced_math_through_livery_used_values() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><body><div id='parent'><div id='target'></div></div></body></html>",
        ));
        LiveryCssom::install(
            &mut runtime,
            &["#parent { width: 75px; }"],
            Device::screen(800.0, 600.0),
        );

        runtime
            .eval(
                "target.style = '';\
                 target.style.marginLeft = 'round(10%, 5px)';\
                 console.log(getComputedStyle(target).marginLeft);\
                 target.style.marginLeft = 'mod(-18px, 100% / 10)';\
                 console.log(getComputedStyle(target).marginLeft);\
                 target.style.marginLeft = 'calc(10px * exp(log(2)))';\
                 console.log(getComputedStyle(target).marginLeft);\
                 target.style.scale = 'sin(30deg)';\
                 console.log(getComputedStyle(target).scale);\
                 target.style.rotate = 'atan2(1px, -1px)';\
                 console.log(getComputedStyle(target).rotate);",
            )
            .expect("advanced CSS math script");

        assert_eq!(
            runtime.host().borrow().console,
            vec!["10px", "4.5px", "20px", "0.5", "2.3561945rad"]
        );
    }

    #[test]
    fn boa_restyles_viewport_units_when_the_livery_device_changes() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><body><div id='target'></div></body></html>",
        ));
        let cssom = LiveryCssom::install(
            &mut runtime,
            &["#target { width: 10vw; }"],
            Device::screen(800.0, 600.0),
        );

        runtime
            .eval("console.log(getComputedStyle(target).width);")
            .expect("initial viewport width");
        cssom.set_viewport_size(400.0, 300.0);
        runtime
            .eval("console.log(getComputedStyle(target).width);")
            .expect("resized viewport width");

        assert_eq!(runtime.host().borrow().console, vec!["80px", "40px"]);
        assert!(cssom.last_restyle_stats().device_invalidated);
    }

    #[test]
    fn boa_scopes_iframe_documents_to_the_laid_out_frame_viewport() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><body><iframe id='frame'></iframe></body></html>",
        ));
        LiveryCssom::install(
            &mut runtime,
            &["iframe { display: inline-block; width: 200px; height: 100px; }"],
            Device::screen(800.0, 600.0),
        );

        runtime
            .eval(
                "var doc = frame.contentDocument;\
                 doc.body.innerHTML = '<style>* { margin: 0; } body { height: 100%; } div { height: calc(1dvw + 1dvh); }</style><div></div>';\
                 console.log(doc.body.innerHTML);\
                 console.log(getComputedStyle(frame).width + 'x' + getComputedStyle(frame).height);\
                 console.log(frame.contentWindow.getComputedStyle(doc.querySelector('div')).height);",
            )
            .expect("iframe style script");

        assert_eq!(
            runtime.host().borrow().console,
            vec![
                "<style>* { margin: 0; } body { height: 100%; } div { height: calc(1dvw + 1dvh); }</style><div></div>",
                "200pxx100px",
                "3px",
            ]
        );
    }

    #[test]
    fn boa_mutates_named_container_queries_through_cssom() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><body>\
             <div class='panel wide'><div id='wide' class='card'></div></div>\
             <div class='panel narrow'><div id='narrow' class='card'></div></div>\
             </body></html>",
        ));
        LiveryCssom::install(
            &mut runtime,
            &[
                ".panel { container-type: size; container-name: sidebar; height: 100px; }\
                 .wide { width: 320px; } .narrow { width: 200px; }\
                 .card { color: red; }",
            ],
            Device::screen(800.0, 600.0),
        );

        runtime
            .eval(
                "var sheet = document.styleSheets[0];\
                 console.log(getComputedStyle(wide).color + '|' + getComputedStyle(narrow).color);\
                 console.log(sheet.insertRule(\
                   '@container sidebar (width >= 300px) { .card { color: green; } }',\
                   sheet.cssRules.length));\
                 console.log(getComputedStyle(wide).color + '|' + getComputedStyle(narrow).color);\
                 sheet.deleteRule(sheet.cssRules.length - 1);\
                 console.log(getComputedStyle(wide).color + '|' + getComputedStyle(narrow).color);",
            )
            .expect("container query CSSOM script");

        assert_eq!(
            runtime.host().borrow().console,
            vec![
                "rgb(255, 0, 0)|rgb(255, 0, 0)",
                "4",
                "rgb(0, 128, 0)|rgb(255, 0, 0)",
                "rgb(255, 0, 0)|rgb(255, 0, 0)",
            ]
        );
    }

    #[test]
    fn boa_resolves_tiered_viewports_comparison_math_and_container_axes() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><body>\
             <div class='inline outer'><div class='size outer'>\
             <div class='inline inner'><div id='target'></div></div>\
             </div></div></body></html>",
        ));
        let mut device = Device::screen(450.0, 250.0);
        device.set_viewport_sizes(ViewportSizes {
            small: ViewportSize::new(300.0, 200.0),
            large: ViewportSize::new(600.0, 400.0),
            dynamic: ViewportSize::new(450.0, 250.0),
        });
        let cssom = LiveryCssom::install(
            &mut runtime,
            &[".inline { container-type: inline-size; }\
               .size { container-type: size; }\
               .inline.outer { width: 500px; }\
               .size.outer { height: 400px; }\
               .inline.inner { width: 300px; }\
               #target { width: max(10cqi, 5cqb); height: min(10cqi, 10cqb);\
                         margin-left: 10svw; margin-right: 10lvw;\
                         top: 10dvh; }"],
            device,
        );

        runtime
            .eval(
                "console.log(getComputedStyle(target).width);\
                 console.log(getComputedStyle(target).height);\
                 console.log(getComputedStyle(target).marginLeft);\
                 console.log(getComputedStyle(target).marginRight);\
                 console.log(getComputedStyle(target).top);",
            )
            .expect("tiered and container unit reads");
        assert_eq!(
            runtime.host().borrow().console,
            vec!["30px", "30px", "30px", "60px", "25px"]
        );

        cssom.set_viewport_sizes(ViewportSizes {
            small: ViewportSize::new(200.0, 100.0),
            large: ViewportSize::new(800.0, 500.0),
            dynamic: ViewportSize::new(500.0, 300.0),
        });
        runtime
            .eval("console.log(getComputedStyle(target).marginLeft);")
            .expect("updated small viewport tier");
        assert!(cssom.last_restyle_stats().device_invalidated);
        runtime
            .eval(
                "console.log(getComputedStyle(target).marginRight);\
                 console.log(getComputedStyle(target).top);",
            )
            .expect("updated large and dynamic viewport tiers");
        assert_eq!(
            runtime.host().borrow().console,
            vec![
                "30px", "30px", "30px", "60px", "25px", "20px", "80px", "30px"
            ]
        );
    }

    #[test]
    fn scripted_attribute_change_restyles_only_its_livery_subtree() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><body>\
             <main id='branch' class='branch'><span id='leaf' class='leaf'></span></main>\
             <aside><span class='unrelated'></span></aside>\
             </body></html>",
        ));
        let cssom = LiveryCssom::install(
            &mut runtime,
            &[".branch.on .leaf { color: #0000ff; }"],
            Device::screen(800.0, 600.0),
        );

        runtime
            .eval(
                "var leaf = document.getElementById('leaf');\
                 console.log(getComputedStyle(leaf).color);\
                 document.getElementById('branch').className = 'branch on';\
                 console.log(getComputedStyle(leaf).color);",
            )
            .expect("scoped Livery restyle");

        assert_eq!(
            runtime.host().borrow().console,
            vec!["rgb(0, 0, 0)", "rgb(0, 0, 255)"]
        );
        let stats = cssom.last_restyle_stats();
        assert_eq!(stats.snapshots, 1);
        assert_eq!(stats.hints, 1);
        assert_eq!(stats.restyled_elements, 2);
        assert!(stats.restyled_elements < stats.total_elements);
        assert!(!stats.full_document);
    }

    #[test]
    fn scripted_style_read_recovers_when_layout_drained_an_unseen_batch() {
        let mut runtime = Runtime::<BoaEngine>::new().expect("runtime");
        runtime.load_dom(&StaticDocument::parse(
            "<html><body><main id='branch'><span id='leaf'></span></main></body></html>",
        ));
        let cssom = LiveryCssom::install(
            &mut runtime,
            &[".on span { color: #0000ff; }"],
            Device::screen(800.0, 600.0),
        );

        runtime
            .eval("console.log(getComputedStyle(document.getElementById('leaf')).color);")
            .expect("initial style read");
        runtime
            .eval("document.getElementById('branch').className = 'on';")
            .expect("mutation");
        let mut drained = Vec::new();
        runtime
            .host()
            .borrow_mut()
            .dom
            .drain_mutations(&mut drained);
        assert!(!drained.is_empty());
        runtime
            .eval("console.log(getComputedStyle(document.getElementById('leaf')).color);")
            .expect("style read after layout drain");

        assert_eq!(
            runtime.host().borrow().console,
            vec!["rgb(0, 0, 0)", "rgb(0, 0, 255)"]
        );
        assert!(cssom.last_restyle_stats().full_document);
    }
}
