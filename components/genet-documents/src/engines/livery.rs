// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The Livery lane as an inker session engine: the clean-room static
//! HTML route, including its retained editable controls.

use std::any::Any;
use std::cell::{Cell, RefCell};

use document_session_api::session_engine::{
    DocumentClip, DocumentClipArtifact, DocumentClipArtifactRole, DocumentFindDirection,
    DocumentFindMatch, DocumentFindQuery, DocumentFindReveal, DocumentFindState, DocumentSession,
    DocumentZoomState, SessionButtonState, SessionClick, SessionCursor, SessionEffect,
    SessionEngine, SessionError, SessionFocusDirection, SessionFormMethod, SessionFormSubmission,
    SessionIme, SessionKey, SessionLink, SessionModifiers, SessionScrollKey, SessionSpawnRequest,
    SessionTextTarget,
};
use document_session_api::{
    DocumentA11yAction, DocumentA11yActionRequest, DocumentA11yClickTarget, DocumentA11yNodeId,
    DocumentA11yProjection, DocumentCapabilities, DocumentCapabilityStatus,
};
use genet_document_resources::{
    PendingResourceRequest, ResolvedDocumentResources, ResolvedStylesheet, ResourceDelta,
    ResourceKind, ResourceLimits, ResourceProvisionError, StagedResourceResolution,
    StylesheetOwner,
};
use genet_host_api::ResourceFetcher;
use genet_host_api::ResourceResponse;
use layout_dom_api::{LayoutDom, LayoutDomMut, LocalName, Namespace, NodeKind, QualName};
use netrender::Scene;
use unicode_segmentation::UnicodeSegmentation;

use super::*;

/// Session engine for the owned Livery CSS and Buckram layout path.
#[cfg(feature = "livery")]
pub struct LiverySessionEngine<Fetch> {
    fetcher: Fetch,
    author_css: Vec<String>,
    resource_limits: ResourceLimits,
}

/// One browser-host preparation for a Livery session.  It owns the exact DOM
/// which will become the session, so URL discovery and layout cannot diverge
/// through separate HTML parsers.
#[cfg(feature = "livery")]
pub struct LiveryResourcePreparation {
    requested_address: String,
    source_response: ResourceResponse,
    staged: StagedResourceResolution<genet_scripted_dom::ScriptedDom>,
}

#[cfg(feature = "livery")]
impl LiveryResourcePreparation {
    pub fn new(
        request: &SessionSpawnRequest,
        source_response: ResourceResponse,
        limits: ResourceLimits,
    ) -> Self {
        let base_resource = source_response
            .final_url
            .split_once('#')
            .map_or(source_response.final_url.as_str(), |(resource, _)| resource)
            .to_owned();
        let source = String::from_utf8_lossy(&source_response.bytes).into_owned();
        let dom = genet_scripted_dom::ScriptedDom::from_serialized_document(&source);
        Self {
            requested_address: request.address.clone(),
            source_response,
            staged: StagedResourceResolution::new(dom, Some(&base_resource), limits),
        }
    }

    pub fn pending_requests(&self) -> Vec<PendingResourceRequest> {
        self.staged.pending_requests()
    }

    pub fn provide_response(
        &mut self,
        request: PendingResourceRequest,
        response: ResourceResponse,
    ) -> Result<(), ResourceProvisionError> {
        self.staged.provide_response(request, response)
    }

    pub fn provide_unavailable(
        &mut self,
        request: PendingResourceRequest,
    ) -> Result<(), ResourceProvisionError> {
        self.staged.provide_unavailable(request)
    }

    fn finish(
        self,
    ) -> Result<
        (
            genet_scripted_dom::ScriptedDom,
            ResourceResponse,
            ResolvedDocumentResources,
        ),
        Vec<PendingResourceRequest>,
    > {
        let resources = self.staged.finish()?;
        Ok((self.staged.into_document(), self.source_response, resources))
    }
}

#[cfg(feature = "livery")]
impl<Fetch> LiverySessionEngine<Fetch> {
    pub fn new(fetcher: Fetch) -> Self {
        Self {
            fetcher,
            author_css: Vec::new(),
            resource_limits: ResourceLimits::default(),
        }
    }

    /// Add host-supplied author sheets before the document's own inline
    /// sheets. This keeps lane policy configurable at registration time.
    pub fn with_author_css(
        fetcher: Fetch,
        sheets: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            fetcher,
            author_css: sheets.into_iter().map(Into::into).collect(),
            resource_limits: ResourceLimits::default(),
        }
    }

    /// Select bounded recursive stylesheet resolution for sessions made by
    /// this host engine.
    pub fn with_resource_limits(mut self, resource_limits: ResourceLimits) -> Self {
        self.resource_limits = resource_limits;
        self
    }
}

#[cfg(feature = "livery")]
impl<Fetch: ResourceFetcher + Send + Sync> SessionEngine<Scene> for LiverySessionEngine<Fetch> {
    fn engine_id(&self) -> &str {
        document_session_api::engine_ids::ENGINE_GENET_LIVERY
    }

    fn spawn(
        &self,
        request: &SessionSpawnRequest,
    ) -> Result<Box<dyn DocumentSession<Scene>>, SessionError> {
        let navigation = genet_livery::NavigationFragment::parse(&request.address);
        let requested_resource = navigation.resource_url.as_str();
        let source_response = match &request.body {
            Some(body) => ResourceResponse {
                final_url: request.address.clone(),
                content_type: request
                    .content_type
                    .clone()
                    .or_else(|| Some("text/html".to_string())),
                bytes: body.as_bytes().to_vec(),
            },
            None => self
                .fetcher
                .fetch_response(requested_resource)
                .ok_or_else(|| {
                    SessionError::SpawnFailed(format!("could not load {requested_resource}"))
                })?,
        };
        let base_resource = source_response
            .final_url
            .split_once('#')
            .map_or(source_response.final_url.as_str(), |(resource, _)| resource)
            .to_owned();
        let source = String::from_utf8_lossy(&source_response.bytes).into_owned();
        // Script-free HTML still uses the mutable DOM backing. Form controls
        // need one retained value plane even when JavaScript is disabled;
        // ScriptedDom supplies LayoutDomMut without constructing a JS runtime.
        let dom = genet_scripted_dom::ScriptedDom::from_serialized_document(&source);
        self.spawn_livery_document(request, dom, base_resource, source_response, navigation)
    }
}

#[cfg(feature = "livery")]
impl<Fetch: ResourceFetcher + Send + Sync> LiverySessionEngine<Fetch> {
    fn spawn_livery_document(
        &self,
        request: &SessionSpawnRequest,
        dom: genet_scripted_dom::ScriptedDom,
        base_resource: String,
        source_response: ResourceResponse,
        navigation: genet_livery::NavigationFragment,
    ) -> Result<Box<dyn DocumentSession<Scene>>, SessionError> {
        let resources = ResolvedDocumentResources::resolve_with_limits(
            &dom,
            Some(&base_resource),
            &self.fetcher,
            self.resource_limits,
        );
        self.spawn_livery_document_with_resources(
            request,
            dom,
            source_response,
            navigation,
            resources,
        )
    }
}

#[cfg(feature = "livery")]
impl<Fetch> LiverySessionEngine<Fetch> {
    /// Complete a browser-host preparation.  The opaque preparation owns both
    /// the source snapshot and the DOM which produced its ledger, making a
    /// source/ledger mismatch unrepresentable at this boundary.
    pub fn spawn_prepared(
        &self,
        request: &SessionSpawnRequest,
        preparation: LiveryResourcePreparation,
    ) -> Result<Box<dyn DocumentSession<Scene>>, SessionError> {
        if preparation.requested_address != request.address {
            return Err(SessionError::SpawnFailed(
                "prepared Livery resources belong to a different navigation".to_owned(),
            ));
        }
        let (dom, source_response, resources) = preparation.finish().map_err(|pending| {
            SessionError::SpawnFailed(format!(
                "prepared Livery resources still pending: {pending:?}"
            ))
        })?;
        let navigation = genet_livery::NavigationFragment::parse(&redirected_navigation_address(
            &request.address,
            &source_response.final_url,
        ));
        self.spawn_livery_document_with_resources(
            request,
            dom,
            source_response,
            navigation,
            resources,
        )
    }

    fn spawn_livery_document_with_resources(
        &self,
        request: &SessionSpawnRequest,
        dom: genet_scripted_dom::ScriptedDom,
        source_response: ResourceResponse,
        navigation: genet_livery::NavigationFragment,
        resources: ResolvedDocumentResources,
    ) -> Result<Box<dyn DocumentSession<Scene>>, SessionError> {
        let mut sheets = self
            .author_css
            .iter()
            .enumerate()
            .map(|(document_order, text)| ResolvedStylesheet {
                // Resource-resolved sheet ids begin at zero. Keep injected
                // host sheets in a disjoint range when the vectors join.
                sheet_id: u64::MAX.saturating_sub(document_order as u64),
                owner: StylesheetOwner::Inline,
                owner_node: None,
                source_url: None,
                requested_url: None,
                content_type: None,
                media: None,
                imports: Vec::new(),
                import_parent: None,
                text: text.clone(),
                document_order: document_order as u64,
            })
            .collect::<Vec<_>>();
        sheets.extend(resources.stylesheets.iter().cloned());
        let (width, height) = request.viewport;
        let mut doc = genet_livery::LiveryDocument::new(
            dom,
            genet_livery::StyleSet::cambium_resources(&sheets),
            genet_livery::Device::screen(width as f32, height as f32),
        );
        for resource in &resources.resources {
            match resource.kind {
                ResourceKind::Image => {
                    doc.set_image_resource(resource.authored_url.clone(), resource.bytes.clone());
                    if resource.resolved_url != resource.authored_url {
                        doc.set_image_resource(
                            resource.resolved_url.clone(),
                            resource.bytes.clone(),
                        );
                    }
                },
                ResourceKind::Font => {
                    doc.set_font_resource(resource.resolved_url.clone(), resource.bytes.clone());
                },
            }
        }
        Ok(Box::new(LiveryDocumentSession {
            doc,
            address: navigation.script_visible_url.clone(),
            focused_node: None,
            editor: None,
            editor_drag: None,
            active_form: None,
            pressed_submit: None,
            last_error: None,
            resources,
            source_response,
            find_state: DocumentFindState::empty(DocumentFindQuery::default()),
            find_ranges: Vec::new(),
            pending_fragment: (!navigation.text_directives.is_empty()
                || navigation.element_fragment.is_some())
            .then_some(navigation),
            zoom: DocumentZoomState::clamped(1.0, LIVERY_PAGE_ZOOM_MIN, LIVERY_PAGE_ZOOM_MAX),
            a11y_revision: Cell::new(0),
            a11y_cache: RefCell::new(None),
        }))
    }
}

#[cfg(feature = "livery")]
fn redirected_navigation_address(requested: &str, final_url: &str) -> String {
    let final_base = final_url
        .split_once('#')
        .map_or(final_url, |(base, _)| base);
    requested.split_once('#').map_or_else(
        || final_base.to_owned(),
        |(_, fragment)| format!("{final_base}#{fragment}"),
    )
}

/// Retained Livery document session. The document owns the resolved style and
/// fragment planes, so this adapter only translates the session contract.
#[cfg(feature = "livery")]
pub struct LiveryDocumentSession {
    pub(crate) doc: genet_livery::LiveryDocument<genet_scripted_dom::ScriptedDom>,
    address: String,
    pub(crate) focused_node: Option<genet_scripted_dom::NodeId>,
    pub(crate) editor: Option<EditableControl>,
    editor_drag: Option<EditableDrag>,
    pub(crate) active_form: Option<genet_scripted_dom::NodeId>,
    pressed_submit: Option<genet_scripted_dom::NodeId>,
    last_error: Option<String>,
    pub(crate) resources: ResolvedDocumentResources,
    source_response: ResourceResponse,
    find_state: DocumentFindState,
    find_ranges: Vec<genet_livery::TextRange<genet_scripted_dom::NodeId>>,
    pending_fragment: Option<genet_livery::NavigationFragment>,
    zoom: DocumentZoomState,
    a11y_revision: Cell<u64>,
    a11y_cache: RefCell<Option<DocumentA11yProjection>>,
}

/// Livery's page-zoom bounds — engine policy, matching the range a Chromium
/// user agent enforces. The requested factor stays the caller's, and this lane
/// quantizes nothing further: the host's ladder is the only stepping.
#[cfg(feature = "livery")]
const LIVERY_PAGE_ZOOM_MIN: f32 = 0.25;
#[cfg(feature = "livery")]
const LIVERY_PAGE_ZOOM_MAX: f32 = 5.0;

#[cfg(feature = "livery")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditableKind {
    Input,
    Textarea,
}

#[cfg(feature = "livery")]
pub(crate) struct EditableControl {
    pub(crate) node: genet_scripted_dom::NodeId,
    pub(crate) kind: EditableKind,
    pub(crate) value: String,
    pub(crate) caret: usize,
    pub(crate) selection: Option<EditableSelection>,
    pub(crate) composition: Option<String>,
}

/// A directed range in one shaped source belonging to the active native
/// editor. This stays separate from Livery's document selection so editor
/// typing cannot become a page clip.
#[cfg(feature = "livery")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EditableSelection {
    pub(crate) source: genet_scripted_dom::NodeId,
    pub(crate) anchor: usize,
    pub(crate) focus: usize,
}

#[cfg(feature = "livery")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EditableDrag {
    source: genet_scripted_dom::NodeId,
    anchor: usize,
}

#[cfg(feature = "livery")]
impl LiveryDocumentSession {
    pub fn document(&self) -> &genet_livery::LiveryDocument<genet_scripted_dom::ScriptedDom> {
        &self.doc
    }

    /// Redirect-final document identity, retaining an explicitly requested
    /// fragment when this session was prepared by an asynchronous host.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Replace one native text control from its document-local accessibility
    /// identity.
    ///
    /// `local_node_id` is the un-namespaced AccessKit node id emitted by
    /// this session's retained Livery tree. It is deliberately not a general
    /// DOM mutation surface: only live, enabled, writable text-like inputs and
    /// textareas accept a replacement. Pelt owns namespacing that local id into
    /// its composite tree and must reject stale session identities before
    /// calling here.
    pub fn replace_accessible_text_value(&mut self, local_node_id: u64, value: &str) -> bool {
        let Ok(raw_node_id) = usize::try_from(local_node_id) else {
            return false;
        };
        let node = genet_scripted_dom::NodeId::from_raw(raw_node_id);
        if !self.doc.dom().is_live(node) {
            return false;
        }
        let Some(kind) = self.accessible_text_kind(node) else {
            return false;
        };

        // Reuse the one retained editing plane so a following IME or key event
        // starts from the accessibility replacement, while `apply_editor`
        // makes the DOM and form submission plane observe the same value.
        self.activate_editable(node, kind);
        let editor = self
            .editor
            .as_mut()
            .expect("an accepted native text control activates its retained editor");
        editor.value.clear();
        editor.value.push_str(value);
        editor.caret = editor.value.len();
        editor.selection = None;
        editor.composition = None;
        self.apply_editor();
        true
    }

    /// Reveal a retained Livery node through its active nested scrollports.
    pub fn scroll_accessible_node_into_view(&mut self, local_node_id: u64) -> bool {
        let Ok(raw_node_id) = usize::try_from(local_node_id) else {
            return false;
        };
        let node = genet_scripted_dom::NodeId::from_raw(raw_node_id);
        self.doc.dom().is_live(node) && self.doc.scroll_accessible_node_into_view(node)
    }

    /// Resolve an accessible node to a visible retained pointer target in CSS
    /// space.
    ///
    /// Pelt owns its composite-tree identity and session-generation checks.
    /// Livery owns clipping and hit testing, so a failed query is inert rather
    /// than a request for the host to guess coordinates from accessibility
    /// bounds. The host owns the CSS-to-presentation transform before it sends
    /// ordinary press and release input back through this session.
    pub fn accessible_pointer_target(&self, local_node_id: u64) -> Option<(f32, f32)> {
        let raw_node_id = usize::try_from(local_node_id).ok()?;
        let node = genet_scripted_dom::NodeId::from_raw(raw_node_id);
        self.doc
            .dom()
            .is_live(node)
            .then_some(node)
            .and_then(|node| self.doc.accessible_pointer_target(node))
    }

    /// The applied page-zoom factor in presentation pixels per CSS pixel.
    ///
    /// A host composing Livery's retained semantics into a larger tree uses
    /// this with its own content-hole transform. The session continues to own
    /// zoom bounds and all document-local coordinate conversion.
    pub fn page_zoom(&self) -> f32 {
        self.zoom()
    }

    /// The effective page zoom: presentation pixels per CSS pixel.
    ///
    /// The retained [`genet_livery::LiveryDocument`] works only in CSS pixels.
    /// Everything crossing the session boundary — pointer coordinates in,
    /// link rects, text targets, find reveals, and content heights out — is in
    /// the host's presentation space, so this factor converts between them and
    /// nothing below this type ever sees it.
    fn zoom(&self) -> f32 {
        self.zoom.applied
    }

    /// Presentation-space point → CSS space.
    fn to_css_point(&self, x: f32, y: f32) -> (f32, f32) {
        let zoom = self.zoom();
        (x / zoom, y / zoom)
    }

    /// Presentation-space length or offset → CSS space.
    fn to_css_length(&self, length: f32) -> f32 {
        length / self.zoom()
    }

    /// CSS-space length or offset → presentation space.
    fn to_presentation_length(&self, length: f32) -> f32 {
        length * self.zoom()
    }

    /// The CSS viewport this presentation viewport covers. Layout and media
    /// queries run against this shrunk box, which is what makes zoom a
    /// user-agent document scale rather than a CSS `zoom` on the root.
    fn css_viewport(&self, width: u32, height: u32) -> (u32, u32) {
        let zoom = self.zoom();
        (
            ((width as f32 / zoom).round() as u32).max(1),
            ((height as f32 / zoom).round() as u32).max(1),
        )
    }

    /// Build the neutral projection from the last completed Livery layout,
    /// then move its CSS geometry into this session's presentation viewport.
    /// The projection helper owns semantic traversal; this adapter owns page
    /// scroll and page-zoom transforms at the session boundary.
    fn unrevisioned_accessibility_projection(&self) -> Option<DocumentA11yProjection> {
        let fragments = self.doc.retained_layout()?;
        let projection = genet_render::document_a11y_projection_with_scroll(
            self.doc.dom(),
            fragments,
            self.focused_node,
            0,
            self.doc.element_scroll(),
        );
        let (scroll_x, scroll_y) = self.doc.scroll();
        let zoom = self.zoom();
        let nodes = projection
            .nodes()
            .iter()
            .cloned()
            .map(|mut node| {
                let pointer_target = node
                    .actions
                    .iter()
                    .any(|action| {
                        matches!(
                            action,
                            DocumentA11yAction::Click | DocumentA11yAction::ScrollIntoView
                        )
                    })
                    .then(|| {
                        usize::try_from(node.id.get())
                            .ok()
                            .map(genet_scripted_dom::NodeId::from_raw)
                            .filter(|node| self.doc.dom().is_live(*node))
                            .and_then(|node| self.doc.accessible_pointer_target(node))
                    })
                    .flatten();
                if node.state.disabled || node.state.hidden || pointer_target.is_none() {
                    node.actions
                        .retain(|action| *action != DocumentA11yAction::Click);
                } else if node.actions.contains(&DocumentA11yAction::ScrollIntoView)
                    && !node.actions.contains(&DocumentA11yAction::Click)
                {
                    // The canonical DOM walk stays conservative below an
                    // active scrollport. Once Livery can supply a current,
                    // clip-aware pointer target, the session may advertise
                    // Click without making Pelt understand Livery geometry.
                    node.actions.push(DocumentA11yAction::Click);
                }
                if let Some(bounds) = node.bounds.as_mut() {
                    bounds.x = (bounds.x - scroll_x) * zoom;
                    bounds.y = (bounds.y - scroll_y) * zoom;
                    bounds.width *= zoom;
                    bounds.height *= zoom;
                }
                node
            })
            .collect();
        Some(DocumentA11yProjection::new(
            0,
            projection.support().clone(),
            projection.root(),
            nodes,
        ))
    }

    fn current_accessibility_projection(&self) -> Option<DocumentA11yProjection> {
        let fresh = self.unrevisioned_accessibility_projection()?;
        let unchanged = self.a11y_cache.borrow().as_ref().is_some_and(|cached| {
            cached.root() == fresh.root()
                && cached.support() == fresh.support()
                && cached.nodes() == fresh.nodes()
        });
        if unchanged {
            return self.a11y_cache.borrow().clone();
        }
        let revision = self.a11y_revision.get().saturating_add(1).max(1);
        let current = DocumentA11yProjection::new(
            revision,
            fresh.support().clone(),
            fresh.root(),
            fresh.nodes().to_vec(),
        );
        self.a11y_revision.set(revision);
        *self.a11y_cache.borrow_mut() = Some(current.clone());
        Some(current)
    }

    pub(crate) fn attribute(&self, node: genet_scripted_dom::NodeId, name: &str) -> Option<&str> {
        self.doc
            .dom()
            .attribute(node, &Namespace::default(), &LocalName::from(name))
    }

    fn tag(&self, node: genet_scripted_dom::NodeId) -> Option<&str> {
        self.doc
            .dom()
            .element_name(node)
            .map(|name| name.local.as_ref())
    }

    fn aria_true(&self, node: genet_scripted_dom::NodeId, name: &str) -> bool {
        self.attribute(node, name)
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
    }

    fn ancestor_matching(
        &self,
        mut node: genet_scripted_dom::NodeId,
        predicate: impl Fn(genet_scripted_dom::NodeId, &str) -> bool,
    ) -> Option<genet_scripted_dom::NodeId> {
        loop {
            if let Some(tag) = self.tag(node)
                && predicate(node, tag)
            {
                return Some(node);
            }
            node = self.doc.dom().parent(node)?;
        }
    }

    fn form_ancestor(
        &self,
        node: genet_scripted_dom::NodeId,
    ) -> Option<genet_scripted_dom::NodeId> {
        self.ancestor_matching(node, |_node, tag| tag.eq_ignore_ascii_case("form"))
    }

    fn editable_kind(&self, node: genet_scripted_dom::NodeId) -> Option<EditableKind> {
        match self.tag(node)? {
            tag if tag.eq_ignore_ascii_case("textarea") => Some(EditableKind::Textarea),
            tag if tag.eq_ignore_ascii_case("input") => {
                let kind = self.attribute(node, "type").unwrap_or("text");
                (!matches!(
                    kind.to_ascii_lowercase().as_str(),
                    "button"
                        | "checkbox"
                        | "file"
                        | "hidden"
                        | "image"
                        | "radio"
                        | "reset"
                        | "submit"
                ))
                .then_some(EditableKind::Input)
            },
            _ => None,
        }
    }

    /// The small native-control set on which an accessibility `SetValue` is
    /// meaningful in this script-free lane. Other input kinds either have a
    /// different value model or need type-specific validation that this
    /// session does not own yet.
    fn accessible_text_kind(&self, node: genet_scripted_dom::NodeId) -> Option<EditableKind> {
        if !self.editable_is_writable(node) {
            return None;
        }
        match self.tag(node)? {
            tag if tag.eq_ignore_ascii_case("textarea") => Some(EditableKind::Textarea),
            tag if tag.eq_ignore_ascii_case("input") => {
                let input_type = self
                    .attribute(node, "type")
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("text");
                matches!(
                    input_type.to_ascii_lowercase().as_str(),
                    "text" | "search" | "email" | "tel" | "url"
                )
                .then_some(EditableKind::Input)
            },
            _ => None,
        }
    }

    fn editable_ancestor(
        &self,
        node: genet_scripted_dom::NodeId,
    ) -> Option<(genet_scripted_dom::NodeId, EditableKind)> {
        let node = self.ancestor_matching(node, |node, _tag| self.editable_kind(node).is_some())?;
        Some((node, self.editable_kind(node)?))
    }

    fn editable_is_writable(&self, node: genet_scripted_dom::NodeId) -> bool {
        !self.is_disabled_control(node)
            && self.attribute(node, "readonly").is_none()
            && !self.aria_true(node, "aria-readonly")
    }

    fn is_disabled_control(&self, node: genet_scripted_dom::NodeId) -> bool {
        self.attribute(node, "disabled").is_some() || self.aria_true(node, "aria-disabled")
    }

    fn is_submit_control(&self, node: genet_scripted_dom::NodeId) -> bool {
        if self.is_disabled_control(node) {
            return false;
        }
        match self.tag(node) {
            Some(tag) if tag.eq_ignore_ascii_case("button") => {
                !self.attribute(node, "type").is_some_and(|kind| {
                    matches!(kind.to_ascii_lowercase().as_str(), "button" | "reset")
                })
            },
            Some(tag) if tag.eq_ignore_ascii_case("input") => {
                self.attribute(node, "type").is_some_and(|kind| {
                    matches!(kind.to_ascii_lowercase().as_str(), "submit" | "image")
                })
            },
            _ => false,
        }
    }

    fn submit_ancestor(
        &self,
        node: genet_scripted_dom::NodeId,
    ) -> Option<genet_scripted_dom::NodeId> {
        self.ancestor_matching(node, |node, _tag| self.is_submit_control(node))
    }

    pub(crate) fn text_content(&self, node: genet_scripted_dom::NodeId) -> String {
        fn append(
            dom: &genet_scripted_dom::ScriptedDom,
            node: genet_scripted_dom::NodeId,
            out: &mut String,
        ) {
            if dom.kind(node) == NodeKind::Text
                && let Some(text) = dom.text(node)
            {
                out.push_str(text);
            }
            for child in dom.dom_children(node) {
                append(dom, child, out);
            }
        }

        let mut value = String::new();
        append(self.doc.dom(), node, &mut value);
        value
    }

    fn activate_editable(&mut self, node: genet_scripted_dom::NodeId, kind: EditableKind) {
        self.editor_drag = None;
        if self.is_disabled_control(node) {
            self.focused_node = None;
            self.active_form = None;
            self.editor = None;
            return;
        }
        self.focused_node = Some(node);
        if !self.editable_is_writable(node) {
            self.active_form = None;
            self.editor = None;
            return;
        }
        let value = match kind {
            EditableKind::Input => self.attribute(node, "value").unwrap_or("").to_owned(),
            EditableKind::Textarea => self.text_content(node),
        };
        let caret = value.len();
        self.active_form = self.form_ancestor(node);
        self.editor = Some(EditableControl {
            node,
            kind,
            value,
            caret,
            selection: None,
            composition: None,
        });
    }

    fn editor_text_source(
        &self,
        node: genet_scripted_dom::NodeId,
    ) -> Option<genet_scripted_dom::NodeId> {
        if self.doc.dom().kind(node) == NodeKind::Text {
            return self.doc.dom().text(node).is_some().then_some(node);
        }
        self.doc
            .dom()
            .dom_children(node)
            .find_map(|child| self.editor_text_source(child))
    }

    fn editor_owns_text_source(
        &self,
        editor: genet_scripted_dom::NodeId,
        source: genet_scripted_dom::NodeId,
    ) -> bool {
        self.editable_ancestor(source)
            .is_some_and(|(owner, _)| owner == editor)
    }

    fn begin_editor_selection(&mut self, x: f32, y: f32) -> bool {
        let Some((source, anchor)) = self.doc.text_position_at_point(x, y) else {
            return false;
        };
        let Some(editor_node) = self.editor.as_ref().map(|editor| editor.node) else {
            return false;
        };
        if !self.editor_owns_text_source(editor_node, source) {
            return false;
        }
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        editor.caret = anchor;
        editor.selection = None;
        self.editor_drag = Some(EditableDrag { source, anchor });
        true
    }

    fn extend_editor_selection(&mut self, x: f32, y: f32) -> bool {
        let Some(drag) = self.editor_drag else {
            return false;
        };
        let Some((source, focus)) = self.doc.text_position_at_point(x, y) else {
            return false;
        };
        let Some(editor_node) = self.editor.as_ref().map(|editor| editor.node) else {
            return false;
        };
        if source != drag.source || !self.editor_owns_text_source(editor_node, source) {
            return false;
        }
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        editor.caret = focus;
        editor.selection = (drag.anchor != focus).then_some(EditableSelection {
            source,
            anchor: drag.anchor,
            focus,
        });
        true
    }

    /// The click path in CSS space, shared by the boundary's `click_at` and the
    /// collapsed-gesture tail of `pointer_up`, which has already converted.
    fn click_at_css(&mut self, x: f32, y: f32) -> SessionClick {
        let hit = self.doc.hit_test(x, y);
        let submit = hit.and_then(|node| self.submit_ancestor(node));
        self.activate_hit(x, y);
        match self.doc.click_at(x, y) {
            genet_livery::ClickOutcome::None => SessionClick::Miss,
            genet_livery::ClickOutcome::Focused | genet_livery::ClickOutcome::Scrolled => {
                submit.map_or(SessionClick::Handled, |submit| self.submit_click(submit))
            },
            genet_livery::ClickOutcome::Navigate(href) => SessionClick::Navigate(href),
        }
    }

    fn activate_hit(&mut self, x: f32, y: f32) {
        let Some(hit) = self.doc.hit_test(x, y) else {
            self.focused_node = None;
            self.editor = None;
            self.active_form = None;
            return;
        };
        if let Some((node, kind)) = self.editable_ancestor(hit) {
            self.activate_editable(node, kind);
            return;
        }
        self.focused_node = self.ancestor_matching(hit, |node, tag| {
            !self.is_disabled_control(node)
                && (tag.eq_ignore_ascii_case("button")
                    || tag.eq_ignore_ascii_case("select")
                    || tag.eq_ignore_ascii_case("a") && self.attribute(node, "href").is_some()
                    || self.attribute(node, "tabindex").is_some())
        });
        self.editor = None;
        self.active_form = self.focused_node.and_then(|node| self.form_ancestor(node));
    }

    fn submission_for_form(
        &self,
        form: genet_scripted_dom::NodeId,
        fallback_action: &str,
    ) -> SessionFormSubmission {
        let action = self
            .attribute(form, "action")
            .filter(|action| !action.is_empty())
            .unwrap_or(fallback_action)
            .to_owned();
        let method = if self
            .attribute(form, "method")
            .is_some_and(|method| method.eq_ignore_ascii_case("post"))
        {
            SessionFormMethod::Post
        } else {
            SessionFormMethod::Get
        };
        SessionFormSubmission {
            action,
            method,
            fields: self.doc.dom().form_values(form),
        }
    }

    fn submit_click(&mut self, submit: genet_scripted_dom::NodeId) -> SessionClick {
        let Some(form) = self.form_ancestor(submit) else {
            return SessionClick::Handled;
        };
        self.active_form = Some(form);
        let action = self
            .attribute(form, "action")
            .unwrap_or(&self.address)
            .to_owned();
        SessionClick::Submit(action)
    }

    fn apply_editor(&mut self) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let (node, kind, value) = (editor.node, editor.kind, editor.value.clone());
        let _ = self.doc.mutate_dom(|dom| match kind {
            EditableKind::Input => dom.set_attribute(
                node,
                QualName::new(None, Namespace::default(), LocalName::from("value")),
                &value,
            ),
            EditableKind::Textarea => dom.set_text_content(node, &value),
        });
    }

    fn insert_text(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        if let Some(selection) = editor.selection.take() {
            let start = selection.anchor.min(selection.focus);
            let end = selection.anchor.max(selection.focus);
            editor.value.replace_range(start..end, text);
            editor.caret = start + text.len();
        } else {
            editor.value.insert_str(editor.caret, text);
            editor.caret += text.len();
        }
        editor.composition = None;
        self.apply_editor();
        true
    }

    pub(crate) fn delete_backward(&mut self) -> bool {
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        if let Some(selection) = editor.selection.take() {
            let start = selection.anchor.min(selection.focus);
            let end = selection.anchor.max(selection.focus);
            editor.value.replace_range(start..end, "");
            editor.caret = start;
            self.apply_editor();
            return true;
        }
        if editor.caret == 0 {
            return true;
        }
        let previous = editor.value[..editor.caret]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(index, _)| index);
        editor.value.replace_range(previous..editor.caret, "");
        editor.caret = previous;
        self.apply_editor();
        true
    }

    fn delete_forward(&mut self) -> bool {
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        if let Some(selection) = editor.selection.take() {
            let start = selection.anchor.min(selection.focus);
            let end = selection.anchor.max(selection.focus);
            editor.value.replace_range(start..end, "");
            editor.caret = start;
            self.apply_editor();
            return true;
        }
        if editor.caret == editor.value.len() {
            return true;
        }
        let next = editor.value[editor.caret..]
            .grapheme_indices(true)
            .nth(1)
            .map_or(editor.value.len(), |(offset, _)| editor.caret + offset);
        editor.value.replace_range(editor.caret..next, "");
        self.apply_editor();
        true
    }

    fn move_caret(&mut self, direction: i8) -> bool {
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        editor.selection = None;
        editor.caret = if direction < 0 {
            editor.value[..editor.caret]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(index, _)| index)
        } else if editor.caret == editor.value.len() {
            editor.caret
        } else {
            editor.value[editor.caret..]
                .grapheme_indices(true)
                .nth(1)
                .map_or(editor.value.len(), |(offset, _)| editor.caret + offset)
        };
        true
    }

    pub(crate) fn collect_focusable(
        &self,
        node: genet_scripted_dom::NodeId,
        out: &mut Vec<genet_scripted_dom::NodeId>,
    ) {
        if let Some(tag) = self.tag(node)
            && !self.is_disabled_control(node)
            && (matches!(
                tag.to_ascii_lowercase().as_str(),
                "button" | "input" | "select" | "textarea"
            ) || tag.eq_ignore_ascii_case("a") && self.attribute(node, "href").is_some()
                || self.attribute(node, "tabindex").is_some())
        {
            out.push(node);
        }
        for child in self.doc.dom().dom_children(node) {
            self.collect_focusable(child, out);
        }
    }

    /// The immutable host-owned inputs used to construct this Livery session.
    /// Hosts can inspect this ledger for a product receipt without gaining a
    /// path to mutate resources or fetch behind the host boundary.
    pub fn resource_set(&self) -> &ResolvedDocumentResources {
        &self.resources
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Replace only the host-fetched image and font ledger. The retained
    /// stylesheet graph must be unchanged; a stylesheet change belongs to the
    /// CSSOM/live-style reconciliation path, not an asset update hidden under
    /// a static document session.
    pub fn replace_resource_bytes(
        &mut self,
        next: ResolvedDocumentResources,
    ) -> Result<ResourceDelta, String> {
        if next.stylesheets != self.resources.stylesheets {
            return Err("resource replacement cannot change the stylesheet graph".to_owned());
        }
        let delta = next.resource_delta_from(&self.resources);
        if delta.is_empty() {
            self.resources = next;
            return Ok(delta);
        }
        let images = next
            .resources
            .iter()
            .filter(|resource| resource.kind == ResourceKind::Image)
            .flat_map(|resource| {
                let mut keys = vec![(resource.authored_url.clone(), resource.bytes.clone())];
                if resource.resolved_url != resource.authored_url {
                    keys.push((resource.resolved_url.clone(), resource.bytes.clone()));
                }
                keys
            });
        let fonts = next
            .resources
            .iter()
            .filter(|resource| resource.kind == ResourceKind::Font)
            .map(|resource| (resource.resolved_url.clone(), resource.bytes.clone()));
        self.doc.replace_image_resources(images);
        self.doc.replace_font_resources(fonts);
        self.resources = next;
        Ok(delta)
    }

    /// Missing or deferred dependencies observed while assembling this
    /// document. This is the Livery ledger for the selected product route.
    pub fn resource_diagnostics(&self) -> &[genet_document_resources::ResourceDiagnostic] {
        &self.resources.diagnostics
    }

    fn find_occurrences(text: &str, query: &DocumentFindQuery) -> Vec<std::ops::Range<usize>> {
        if query.text.is_empty() {
            return Vec::new();
        }
        if query.match_case {
            return text
                .match_indices(&query.text)
                .map(|(start, matched)| start..start + matched.len())
                .collect();
        }
        if text.is_ascii() && query.text.is_ascii() {
            let haystack = text.to_ascii_lowercase();
            let needle = query.text.to_ascii_lowercase();
            return haystack
                .match_indices(&needle)
                .map(|(start, matched)| start..start + matched.len())
                .collect();
        }

        let needle = query.text.to_lowercase();
        let width = query.text.chars().count();
        let boundaries: Vec<_> = text
            .char_indices()
            .map(|(index, _)| index)
            .chain(std::iter::once(text.len()))
            .collect();
        (0..boundaries.len().saturating_sub(1))
            .filter_map(|start_index| {
                let end_index = start_index.checked_add(width)?;
                let end = *boundaries.get(end_index)?;
                let start = boundaries[start_index];
                (text[start..end].to_lowercase() == needle).then_some(start..end)
            })
            .collect()
    }

    fn find_structural_context(
        &self,
        source: genet_scripted_dom::NodeId,
        matched: &str,
    ) -> (Option<String>, String) {
        let mut node = self.doc.dom().parent(source);
        let mut fallback = None;
        while let Some(candidate) = node {
            if let Some(tag) = self.tag(candidate) {
                let role = match tag {
                    "a" => "link",
                    "button" => "button",
                    "input" | "textarea" => "textbox",
                    "p" => "paragraph",
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
                    "li" => "listitem",
                    "img" => "image",
                    "label" => "label",
                    "nav" => "navigation",
                    "header" => "banner",
                    "footer" => "contentinfo",
                    "main" => "main",
                    "section" | "article" => "region",
                    _ => "group",
                };
                let label = self.text_content(candidate).trim().to_string();
                let context = (
                    Some(role.to_string()),
                    if label.is_empty() {
                        matched.to_string()
                    } else {
                        label
                    },
                );
                if role != "group" {
                    return context;
                }
                fallback.get_or_insert(context);
            }
            node = self.doc.dom().parent(candidate);
        }
        fallback.unwrap_or_else(|| (None, matched.to_string()))
    }

    fn find_matches(
        &self,
        query: &DocumentFindQuery,
    ) -> (
        Vec<DocumentFindMatch>,
        Vec<genet_livery::TextRange<genet_scripted_dom::NodeId>>,
    ) {
        fn collect_text_nodes(
            dom: &genet_scripted_dom::ScriptedDom,
            node: genet_scripted_dom::NodeId,
            out: &mut Vec<genet_scripted_dom::NodeId>,
        ) {
            if dom.kind(node) == NodeKind::Text {
                out.push(node);
            }
            for child in dom.dom_children(node) {
                collect_text_nodes(dom, child, out);
            }
        }

        let mut sources = Vec::new();
        collect_text_nodes(self.doc.dom(), self.doc.dom().document(), &mut sources);
        let mut matches = Vec::new();
        let mut ranges = Vec::new();
        for source in sources {
            let Some(text) = self.doc.dom().text(source) else {
                continue;
            };
            for range in Self::find_occurrences(text, query) {
                let Some(caret) = self.doc.caret_rect(source, range.start) else {
                    continue;
                };
                let matched = &text[range.clone()];
                let (role, label) = self.find_structural_context(source, matched);
                matches.push(DocumentFindMatch {
                    label,
                    role,
                    reveal: DocumentFindReveal::ScrollY(
                        self.to_presentation_length((self.doc.scroll().1 + caret.y).max(0.0)),
                    ),
                });
                ranges.push(genet_livery::TextRange {
                    anchor_node: source,
                    anchor_offset: range.start,
                    focus_node: source,
                    focus_offset: range.end,
                });
            }
        }
        (matches, ranges)
    }

    fn reveal_find_current(&mut self) {
        let Some(index) = self.find_state.current else {
            self.doc.select_text_range(None);
            return;
        };
        let Some(range) = self.find_ranges.get(index).copied() else {
            return;
        };
        if let Some(DocumentFindMatch {
            reveal: DocumentFindReveal::ScrollY(y),
            ..
        }) = self.find_state.matches.get(index)
        {
            let y = self.to_css_length(*y);
            self.doc.scroll_to((y - 24.0).max(0.0));
        }
        self.doc.select_text_range(Some(range));
    }
}

#[cfg(feature = "livery")]
impl DocumentSession<Scene> for LiveryDocumentSession {
    fn document_capabilities(&self) -> DocumentCapabilities {
        DocumentCapabilities {
            find_in_page: DocumentCapabilityStatus::Supported,
            page_zoom: DocumentCapabilityStatus::Supported,
            page_capture: DocumentCapabilityStatus::unsupported(
                "Livery sessions do not capture rendered pages",
            ),
            navigation: DocumentCapabilityStatus::Partial {
                detail: "the host owns document lineage, policy, and refetch".into(),
            },
        }
    }

    fn frame(&mut self, width: u32, height: u32) -> Scene {
        let (css_width, css_height) = self.css_viewport(width, height);
        let mut list = match self.doc.frame(css_width, css_height) {
            Ok(list) => {
                self.last_error = None;
                list
            },
            Err(error) => {
                self.last_error = Some(error.to_string());
                return Scene::new(width, height);
            },
        };
        if let Some(navigation) = self.pending_fragment.take() {
            let text_activated = self
                .doc
                .activate_text_directives(&navigation.text_directives);
            if !text_activated && let Some(fragment) = navigation.element_fragment.as_deref() {
                self.doc.scroll_to_element_fragment(fragment);
            }
            // The activation only changes retained selection/scroll state. It
            // deliberately reuses this session's already-loaded source.
            list = match self.doc.frame(css_width, css_height) {
                Ok(list) => list,
                Err(error) => {
                    self.last_error = Some(error.to_string());
                    return Scene::new(width, height);
                },
            };
        }
        let list = list.scaled_to(self.zoom(), width, height);
        let mut scene = paint_list_render::translate_paint_list(&list);
        if let Some(selection) = self.doc.text_selection() {
            for rect in selection.rects {
                // The overlay is host-side paint, so it is placed in the
                // presentation viewport the scaled document now fills.
                let x0 = self.to_presentation_length(rect.x).max(0.0);
                let y0 = self.to_presentation_length(rect.y).max(0.0);
                let x1 = self
                    .to_presentation_length(rect.x + rect.width)
                    .min(width as f32);
                let y1 = self
                    .to_presentation_length(rect.y + rect.height)
                    .min(height as f32);
                if x0 < x1 && y0 < y1 {
                    scene.push_rect(x0, y0, x1, y1, [0.18, 0.46, 0.95, 0.34]);
                }
            }
        }
        if let Some(editor) = self.editor.as_ref() {
            let selection = editor.selection.and_then(|selection| {
                self.doc.selection_for_range(genet_livery::TextRange {
                    anchor_node: selection.source,
                    anchor_offset: selection.anchor,
                    focus_node: selection.source,
                    focus_offset: selection.focus,
                })
            });
            if let Some(selection) = selection {
                for rect in selection.rects {
                    let x0 = self.to_presentation_length(rect.x).max(0.0);
                    let y0 = self.to_presentation_length(rect.y).max(0.0);
                    let x1 = self
                        .to_presentation_length(rect.x + rect.width)
                        .min(width as f32);
                    let y1 = self
                        .to_presentation_length(rect.y + rect.height)
                        .min(height as f32);
                    if x0 < x1 && y0 < y1 {
                        scene.push_rect(x0, y0, x1, y1, [0.18, 0.46, 0.95, 0.34]);
                    }
                }
            } else {
                let caret = self
                    .editor_text_source(editor.node)
                    .and_then(|source| self.doc.caret_rect(source, editor.caret))
                    .or_else(|| {
                        (editor.kind == EditableKind::Textarea && editor.value.is_empty())
                            .then(|| self.doc.fragment_rect(editor.node))
                            .flatten()
                            .map(|[x, y, _width, height]| genet_livery::TextRect {
                                x: x + 1.0,
                                y: y + 1.0,
                                width: 1.0,
                                height: (height - 2.0).clamp(1.0, 16.0),
                            })
                    });
                if let Some(caret) = caret {
                    let x0 = self.to_presentation_length(caret.x).max(0.0);
                    let y0 = self.to_presentation_length(caret.y).max(0.0);
                    let x1 = self
                        .to_presentation_length(caret.x + caret.width)
                        .min(width as f32);
                    let y1 = self
                        .to_presentation_length(caret.y + caret.height)
                        .min(height as f32);
                    if x0 < x1 && y0 < y1 {
                        scene.push_rect(x0, y0, x1, y1, [0.18, 0.46, 0.95, 0.85]);
                    }
                }
            }
        }
        scene
    }

    fn scroll_by(&mut self, dx: f32, dy: f32) -> bool {
        let (dx, dy) = self.to_css_point(dx, dy);
        self.doc.scroll_by(dx, dy)
    }

    fn scroll_at(&mut self, x: f32, y: f32, dx: f32, dy: f32) -> bool {
        let (x, y) = self.to_css_point(x, y);
        let (dx, dy) = self.to_css_point(dx, dy);
        self.doc.scroll_at(x, y, dx, dy)
    }

    fn scroll_for_key(&mut self, key: SessionScrollKey) -> bool {
        match key {
            SessionScrollKey::LineUp => self.doc.scroll_line(-1),
            SessionScrollKey::LineDown => self.doc.scroll_line(1),
            SessionScrollKey::PageUp => self.doc.scroll_page(-1),
            SessionScrollKey::PageDown => self.doc.scroll_page(1),
            SessionScrollKey::Home => {
                let before = self.doc.scroll();
                self.doc.scroll_to(0.0);
                before != self.doc.scroll()
            },
            SessionScrollKey::End => {
                let before = self.doc.scroll();
                self.doc.scroll_to(f32::MAX);
                before != self.doc.scroll()
            },
        }
    }

    fn scroll_to(&mut self, y: f32) {
        let y = self.to_css_length(y);
        self.doc.scroll_to(y);
    }

    fn click_at(&mut self, x: f32, y: f32) -> SessionClick {
        let (x, y) = self.to_css_point(x, y);
        self.click_at_css(x, y)
    }

    fn pointer_down(&mut self, x: f32, y: f32) -> SessionClick {
        let (x, y) = self.to_css_point(x, y);
        self.editor_drag = None;
        self.pressed_submit = self
            .doc
            .hit_test(x, y)
            .and_then(|node| self.submit_ancestor(node));
        self.activate_hit(x, y);
        if self.editor.is_some() {
            self.doc.select_text_range(None);
            if self.begin_editor_selection(x, y) {
                return SessionClick::Handled;
            }
            let _ = self.doc.click_at(x, y);
            return SessionClick::Handled;
        }
        if self.doc.begin_text_selection(x, y) {
            SessionClick::Handled
        } else if self.editor.is_some() {
            let _ = self.doc.click_at(x, y);
            SessionClick::Handled
        } else if self.pressed_submit.is_some() || self.focused_node.is_some() {
            // Links and buttons activate on the matching release. Keeping the
            // press handled starts the host's logical pointer capture without
            // replacing the session before its release arrives.
            SessionClick::Handled
        } else {
            SessionClick::Miss
        }
    }

    fn pointer_move(&mut self, x: f32, y: f32) -> bool {
        let (x, y) = self.to_css_point(x, y);
        if self.editor_drag.is_some() {
            return self.extend_editor_selection(x, y);
        }
        self.doc.extend_text_selection(x, y)
    }

    fn pointer_up(&mut self, x: f32, y: f32) -> SessionClick {
        let (x, y) = self.to_css_point(x, y);
        if self.editor_drag.is_some() {
            let _ = self.extend_editor_selection(x, y);
            self.editor_drag = None;
            self.pressed_submit = None;
            return SessionClick::Handled;
        }
        if self.doc.finish_text_selection(x, y) {
            self.pressed_submit = None;
            SessionClick::Handled
        } else {
            let released_submit = self
                .doc
                .hit_test(x, y)
                .and_then(|node| self.submit_ancestor(node));
            let pressed_submit = self.pressed_submit.take();
            if let Some(submit) = released_submit
                && Some(submit) == pressed_submit
            {
                let _ = self.doc.click_at(x, y);
                self.activate_hit(x, y);
                self.submit_click(submit)
            } else {
                self.click_at_css(x, y)
            }
        }
    }

    fn key_input(
        &mut self,
        key: SessionKey,
        state: SessionButtonState,
        modifiers: SessionModifiers,
        _repeat: bool,
    ) -> SessionEffect {
        if state == SessionButtonState::Released {
            return SessionEffect::Ignored;
        }
        match key {
            SessionKey::Character(text)
                if !modifiers.control && !modifiers.meta && !modifiers.alt =>
            {
                if self.insert_text(&text) {
                    SessionEffect::Handled
                } else {
                    SessionEffect::Ignored
                }
            },
            SessionKey::Space if self.insert_text(" ") => SessionEffect::Handled,
            SessionKey::Backspace if self.delete_backward() => SessionEffect::Handled,
            SessionKey::Delete if self.delete_forward() => SessionEffect::Handled,
            SessionKey::ArrowLeft if self.move_caret(-1) => SessionEffect::Handled,
            SessionKey::ArrowRight if self.move_caret(1) => SessionEffect::Handled,
            SessionKey::Home if let Some(editor) = self.editor.as_mut() => {
                editor.caret = 0;
                editor.selection = None;
                SessionEffect::Handled
            },
            SessionKey::End if let Some(editor) = self.editor.as_mut() => {
                editor.caret = editor.value.len();
                editor.selection = None;
                SessionEffect::Handled
            },
            SessionKey::Enter => {
                if self
                    .editor
                    .as_ref()
                    .is_some_and(|editor| editor.kind == EditableKind::Textarea)
                {
                    if self.insert_text("\n") {
                        SessionEffect::Handled
                    } else {
                        SessionEffect::Ignored
                    }
                } else if let Some(form) = self.active_form {
                    SessionEffect::Submit(self.submission_for_form(form, &self.address))
                } else {
                    SessionEffect::Ignored
                }
            },
            SessionKey::Tab => {
                let direction = if modifiers.shift {
                    SessionFocusDirection::Backward
                } else {
                    SessionFocusDirection::Forward
                };
                if self.focus_move(direction) {
                    SessionEffect::Handled
                } else {
                    SessionEffect::Ignored
                }
            },
            SessionKey::Escape => {
                if self.cancel_input() {
                    SessionEffect::Cancelled
                } else {
                    SessionEffect::Ignored
                }
            },
            _ => SessionEffect::Ignored,
        }
    }

    fn text_input(&mut self, text: &str) -> bool {
        self.insert_text(text)
    }

    fn ime_input(&mut self, ime: SessionIme) -> bool {
        match ime {
            SessionIme::Enabled => self.editor.is_some(),
            SessionIme::Preedit { text, .. } => {
                let Some(editor) = self.editor.as_mut() else {
                    return false;
                };
                editor.composition = Some(text);
                true
            },
            SessionIme::Commit(text) => self.insert_text(&text),
            SessionIme::Disabled => {
                let Some(editor) = self.editor.as_mut() else {
                    return false;
                };
                editor.composition = None;
                true
            },
        }
    }

    fn focus_input(&mut self, focused: bool) {
        if !focused {
            self.editor_drag = None;
            self.focused_node = None;
            self.editor = None;
            self.active_form = None;
        }
    }

    fn focus_move(&mut self, direction: SessionFocusDirection) -> bool {
        let mut focusable = Vec::new();
        self.collect_focusable(self.doc.dom().document(), &mut focusable);
        if focusable.is_empty() {
            return false;
        }
        let current = self
            .focused_node
            .and_then(|focused| focusable.iter().position(|node| *node == focused));
        let next = match direction {
            SessionFocusDirection::Forward => {
                current.map_or(0, |index| (index + 1) % focusable.len())
            },
            SessionFocusDirection::Backward => current.map_or(focusable.len() - 1, |index| {
                index.checked_sub(1).unwrap_or(focusable.len() - 1)
            }),
        };
        let node = focusable[next];
        if self.is_disabled_control(node) {
            self.focused_node = None;
            self.editor = None;
            self.active_form = None;
            return false;
        }
        let Some([x, y, width, height]) = self.doc.fragment_rect(node) else {
            return false;
        };
        let _ = self.doc.click_at(x + width * 0.5, y + height * 0.5);
        if let Some(kind) = self.editable_kind(node) {
            self.activate_editable(node, kind);
        } else {
            self.focused_node = Some(node);
            self.editor = None;
            self.active_form = self.form_ancestor(node);
        }
        true
    }

    fn cancel_input(&mut self) -> bool {
        self.editor_drag = None;
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        editor.composition.take().is_some()
    }

    fn editable_focus(&self) -> bool {
        self.editor.is_some()
    }

    fn cursor_at(&self, x: f32, y: f32) -> SessionCursor {
        let (x, y) = self.to_css_point(x, y);
        let Some(hit) = self.doc.hit_test(x, y) else {
            return SessionCursor::Default;
        };
        if self.editable_ancestor(hit).is_some() {
            SessionCursor::Text
        } else if self.submit_ancestor(hit).is_some()
            || self
                .ancestor_matching(hit, |node, tag| {
                    tag.eq_ignore_ascii_case("a") && self.attribute(node, "href").is_some()
                })
                .is_some()
        {
            SessionCursor::Pointer
        } else {
            SessionCursor::Default
        }
    }

    fn form_submission(&mut self, action: &str) -> SessionFormSubmission {
        self.active_form.map_or_else(
            || SessionFormSubmission {
                action: action.to_owned(),
                ..Default::default()
            },
            |form| self.submission_for_form(form, action),
        )
    }

    fn text_target(&self, text: &str) -> Option<SessionTextTarget> {
        let (anchor, focus) = self.doc.text_target(text)?;
        // These are pointer endpoints callers drive back through `pointer_*`,
        // so they leave in the same presentation space those methods take.
        Some(SessionTextTarget {
            anchor: anchor.map(|value| self.to_presentation_length(value)),
            focus: focus.map(|value| self.to_presentation_length(value)),
        })
    }

    fn document_find(
        &mut self,
        query: &DocumentFindQuery,
    ) -> Result<DocumentFindState, SessionError> {
        if query.text.is_empty() {
            self.clear_document_find()?;
            self.find_state.query = query.clone();
            return Ok(self.find_state.clone());
        }
        let (matches, ranges) = self.find_matches(query);
        self.find_ranges = ranges;
        self.find_state = DocumentFindState {
            query: query.clone(),
            current: (!matches.is_empty()).then_some(0),
            count: matches.len(),
            matches,
            complete: true,
        };
        self.reveal_find_current();
        Ok(self.find_state.clone())
    }

    fn document_find_step(
        &mut self,
        direction: DocumentFindDirection,
    ) -> Result<DocumentFindState, SessionError> {
        let count = self.find_state.count;
        if count == 0 {
            return Ok(self.find_state.clone());
        }
        let current = self.find_state.current.unwrap_or(0);
        self.find_state.current = Some(match direction {
            DocumentFindDirection::Previous => (current + count - 1) % count,
            DocumentFindDirection::Next => (current + 1) % count,
        });
        self.reveal_find_current();
        Ok(self.find_state.clone())
    }

    fn clear_document_find(&mut self) -> Result<(), SessionError> {
        self.find_ranges.clear();
        self.find_state = DocumentFindState::empty(DocumentFindQuery::default());
        self.doc.select_text_range(None);
        Ok(())
    }

    fn set_page_zoom(&mut self, factor: f32) -> Result<DocumentZoomState, SessionError> {
        self.zoom = DocumentZoomState::clamped(factor, LIVERY_PAGE_ZOOM_MIN, LIVERY_PAGE_ZOOM_MAX);
        // The retained scroll offset stays a CSS-space value, so the document
        // keeps the same content near the top of the viewport across the
        // reflow the next frame performs at the new CSS viewport.
        Ok(self.zoom)
    }

    fn links(&self) -> Vec<SessionLink> {
        self.doc
            .links()
            .into_iter()
            .map(|link| SessionLink {
                url: link.url,
                rect: link.rect.map(|value| self.to_presentation_length(value)),
            })
            .collect()
    }

    fn content_height(&mut self, width: u32, height: u32) -> u32 {
        let (_, css_height) = self.css_viewport(width, height);
        let content = self.doc.content_height(css_height) as f32;
        self.to_presentation_length(content).ceil() as u32
    }

    fn pump(&mut self, now_ms: f64) {
        self.doc.pump(now_ms);
    }

    fn settled(&mut self) -> bool {
        self.doc.settled()
    }

    /// The structural report through the trait, off the Livery document's own
    /// DOM — the same read the static lane serves, so a viewer-pinned livery
    /// session inspects (and a11y-projects) instead of answering "none for
    /// this lane".
    fn inspect(&self) -> Option<document_session_api::ContentReport> {
        Some(content_report(self.doc.dom()))
    }

    fn accessibility_projection(&self) -> Option<DocumentA11yProjection> {
        self.current_accessibility_projection()
    }

    fn accessibility_click_target(
        &self,
        target: DocumentA11yNodeId,
    ) -> Option<DocumentA11yClickTarget> {
        let projection = self.current_accessibility_projection()?;
        let node = projection.node(target)?;
        if !node.actions.contains(&DocumentA11yAction::Click) {
            return None;
        }
        let raw_node_id = usize::try_from(target.get()).ok()?;
        let dom_node = genet_scripted_dom::NodeId::from_raw(raw_node_id);
        let (x, y) = self.doc.accessible_pointer_target(dom_node)?;
        Some(DocumentA11yClickTarget {
            revision: projection.revision(),
            point: document_session_api::DocumentA11yPoint {
                x: x * self.zoom(),
                y: y * self.zoom(),
            },
        })
    }

    fn dispatch_accessibility_action(&mut self, request: &DocumentA11yActionRequest) -> bool {
        let Some(projection) = self.current_accessibility_projection() else {
            return false;
        };
        let Some(node) = projection.node(request.target) else {
            return false;
        };
        if projection.revision() != request.revision || !node.actions.contains(&request.action) {
            return false;
        }
        let Ok(raw_node_id) = usize::try_from(request.target.get()) else {
            return false;
        };
        let dom_node = genet_scripted_dom::NodeId::from_raw(raw_node_id);
        if !self.doc.dom().is_live(dom_node) {
            return false;
        }
        match request.action {
            DocumentA11yAction::Click => false,
            DocumentA11yAction::Focus => {
                let Some([x, y, width, height]) = self.doc.fragment_rect(dom_node) else {
                    return false;
                };
                self.activate_hit(x + width * 0.5, y + height * 0.5);
                self.focused_node == Some(dom_node)
                    || self
                        .editor
                        .as_ref()
                        .is_some_and(|editor| editor.node == dom_node)
            },
            DocumentA11yAction::SetValue => {
                let Some(document_session_api::DocumentA11yActionData::Value(value)) =
                    request.data.as_ref()
                else {
                    return false;
                };
                self.replace_accessible_text_value(request.target.get(), value)
            },
            DocumentA11yAction::ScrollIntoView => {
                self.scroll_accessible_node_into_view(request.target.get())
            },
            DocumentA11yAction::Increment | DocumentA11yAction::Decrement => false,
        }
    }

    fn clip(&self) -> Option<DocumentClip> {
        let selection = self.doc.text_selection();
        let mut clip = match selection {
            Some(selection) => {
                let links = self.doc.links_for_selection(&selection);
                semantic_clip_from_selection_with_links(
                    &self.address,
                    self.doc.dom(),
                    ClipSelection {
                        range: ClipRange {
                            anchor_node: selection.range.anchor_node,
                            anchor_offset: selection.range.anchor_offset,
                            focus_node: selection.range.focus_node,
                            focus_offset: selection.range.focus_offset,
                        },
                        text: selection.text,
                    },
                    links,
                )
            },
            None => semantic_clip_from_dom(&self.address, self.doc.dom()),
        }?;
        clip.artifacts.push(DocumentClipArtifact {
            role: DocumentClipArtifactRole::SourceResponse,
            media_type: self
                .source_response
                .content_type
                .as_deref()
                .unwrap_or("text/html")
                .to_string(),
            canonical_uri: self.source_response.final_url.clone(),
            bytes: self.source_response.bytes.clone(),
        });
        Some(clip)
    }

    fn as_any_ref(&self) -> &dyn Any {
        self
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}
