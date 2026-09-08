// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The scripted lane as an inker session engine: a page's DOM after
//! its scripts ran, wrapped for the session registry.

use std::any::Any;
use std::time::Duration;

use document_session_api::DocumentCapabilities;
use document_session_api::session_engine::{
    DocumentClip, DocumentSession, SessionClick, SessionEngine, SessionError, SessionLink,
    SessionPendingWork, SessionScrollKey, SessionSpawnRequest, SessionTextTarget,
};
use netrender::Scene;

use super::*;

/// Map the host-neutral scroll-key vocabulary onto the owned scripted lane.
#[cfg(feature = "scripted")]
pub(crate) fn scripted_scroll_key(key: SessionScrollKey) -> genet_scripted::ScrollKey {
    match key {
        SessionScrollKey::LineUp => genet_scripted::ScrollKey::Up,
        SessionScrollKey::LineDown => genet_scripted::ScrollKey::Down,
        SessionScrollKey::PageUp => genet_scripted::ScrollKey::PageUp,
        SessionScrollKey::PageDown => genet_scripted::ScrollKey::PageDown,
        SessionScrollKey::Home => genet_scripted::ScrollKey::Home,
        SessionScrollKey::End => genet_scripted::ScrollKey::End,
    }
}

/// Session engine for the scripted lane, generic over the JS engine `E` (the
/// per-engine monomorphization genet-scripted already uses: the host
/// registers `ScriptedSessionEngine::<BoaEngine, _>` under `genet.scripted`
/// and, on 64-bit targets with the `scripted-nova` feature,
/// `ScriptedSessionEngine::<NovaEngine, _>` under `genet.scripted.nova`).
/// Holds the shell's fetcher for external `<script src>` resolution.
#[cfg(feature = "scripted")]
pub struct ScriptedSessionEngine<E, Fetch> {
    engine_id: String,
    fetcher: Fetch,
    _engine: std::marker::PhantomData<fn() -> E>,
}

#[cfg(feature = "scripted")]
impl<E, Fetch> ScriptedSessionEngine<E, Fetch> {
    pub fn new(engine_id: impl Into<String>, fetcher: Fetch) -> Self {
        Self {
            engine_id: engine_id.into(),
            fetcher,
            _engine: std::marker::PhantomData,
        }
    }
}

#[cfg(feature = "scripted")]
impl<E, Fetch> SessionEngine<Scene> for ScriptedSessionEngine<E, Fetch>
where
    E: script_engine_api::ScriptEngine + 'static,
    Fetch: genet_scripted::ResourceFetcher + Clone + Send + Sync + 'static,
{
    fn engine_id(&self) -> &str {
        &self.engine_id
    }

    fn spawn(
        &self,
        request: &SessionSpawnRequest,
    ) -> Result<Box<dyn DocumentSession<Scene>>, SessionError> {
        let navigation = genet_livery::NavigationFragment::parse(&request.address);
        let doc = match &request.body {
            Some(body) => genet_scripted::LiveryScriptedDocument::<E>::from_body(
                body,
                self.fetcher.clone(),
                &request.address,
            ),
            None => genet_scripted::LiveryScriptedDocument::<E>::load(
                self.fetcher.clone(),
                &request.address,
            ),
        }
        .map_err(SessionError::SpawnFailed)?;
        let mut session = ScriptedDocumentSession {
            doc,
            address: navigation.script_visible_url,
            pressed_target: None,
            pointer_active: false,
        };
        if request.hidden {
            session.doc.set_hidden(true);
        }
        Ok(Box::new(session))
    }
}

/// The scripted document as a session. Public so a host with richer
/// construction seams (per-spawn fetchers, cookie jars) builds the document
/// itself and wraps it; the engine above is the simple-seam path.
#[cfg(feature = "scripted")]
pub struct ScriptedDocumentSession<E: script_engine_api::ScriptEngine> {
    doc: genet_scripted::LiveryScriptedDocument<E>,
    address: String,
    pressed_target: Option<genet_scripted_dom::NodeId>,
    pointer_active: bool,
}

#[cfg(feature = "scripted")]
impl<E: script_engine_api::ScriptEngine + 'static> ScriptedDocumentSession<E> {
    pub fn new(doc: genet_scripted::LiveryScriptedDocument<E>) -> Self {
        Self::new_at(doc, "about:blank")
    }

    pub fn new_at(
        doc: genet_scripted::LiveryScriptedDocument<E>,
        address: impl Into<String>,
    ) -> Self {
        Self {
            doc,
            address: address.into(),
            pressed_target: None,
            pointer_active: false,
        }
    }
}

#[cfg(feature = "scripted")]
impl<E: script_engine_api::ScriptEngine + 'static> DocumentSession<Scene>
    for ScriptedDocumentSession<E>
{
    fn document_capabilities(&self) -> DocumentCapabilities {
        retained_document_capabilities("scripted sessions do not expose document find")
    }

    fn frame(&mut self, width: u32, height: u32) -> Scene {
        self.doc.frame(width, height)
    }
    fn scroll_by(&mut self, dx: f32, dy: f32) -> bool {
        self.doc.scroll_by(dx, dy)
    }
    fn scroll_for_key(&mut self, key: SessionScrollKey) -> bool {
        self.doc.scroll_for_key(scripted_scroll_key(key))
    }
    fn click_at(&mut self, x: f32, y: f32) -> SessionClick {
        match self.doc.click_at_result(x, y) {
            genet_scripted::ScriptedClick::Miss => SessionClick::Miss,
            genet_scripted::ScriptedClick::Handled => SessionClick::Handled,
            genet_scripted::ScriptedClick::Navigate(target) => SessionClick::Navigate(target),
        }
    }
    fn pointer_down(&mut self, x: f32, y: f32) -> SessionClick {
        let pressed_target = self.doc.click_target_at(x, y);
        self.pressed_target = pressed_target;
        self.pointer_active = self.doc.begin_text_selection(x, y) || pressed_target.is_some();
        if self.pointer_active {
            SessionClick::Handled
        } else {
            SessionClick::Miss
        }
    }
    fn pointer_move(&mut self, x: f32, y: f32) -> bool {
        self.doc.extend_text_selection(x, y)
    }
    fn pointer_up(&mut self, x: f32, y: f32) -> SessionClick {
        if !std::mem::replace(&mut self.pointer_active, false) {
            self.pressed_target = None;
            return SessionClick::Miss;
        }
        let pressed_target = self.pressed_target.take();
        if self.doc.finish_text_selection(x, y) {
            SessionClick::Handled
        } else if pressed_target.is_some() && self.doc.click_target_at(x, y) == pressed_target {
            self.click_at(x, y)
        } else {
            SessionClick::Miss
        }
    }
    fn focus_input(&mut self, focused: bool) {
        if !focused && self.pointer_active {
            self.pointer_active = false;
            self.pressed_target = None;
            let _ = self.doc.cancel_text_selection();
        }
    }
    fn cancel_input(&mut self) -> bool {
        let had_pointer = std::mem::replace(&mut self.pointer_active, false)
            || self.pressed_target.take().is_some();
        self.pressed_target = None;
        self.doc.cancel_text_selection() || had_pointer
    }
    fn text_target(&self, text: &str) -> Option<SessionTextTarget> {
        let (anchor, focus) = self.doc.text_target(text)?;
        Some(SessionTextTarget { anchor, focus })
    }
    fn links(&self) -> Vec<SessionLink> {
        self.doc
            .links()
            .into_iter()
            .map(|(url, rect)| SessionLink { url, rect })
            .collect()
    }
    fn pump(&mut self, now_ms: f64) {
        let _ = self.doc.pump(now_ms);
    }
    fn pending_work(&mut self) -> SessionPendingWork {
        self.doc
            .next_timer_delay()
            .and_then(|delay| delay.is_finite().then_some(delay.max(0.0)))
            .map(|delay| SessionPendingWork::timer(Duration::from_secs_f64(delay / 1000.0)))
            .unwrap_or_else(SessionPendingWork::idle)
    }
    fn settled(&mut self) -> bool {
        self.pending_work().is_idle()
    }
    fn set_hidden(&mut self, hidden: bool) {
        self.doc.set_hidden(hidden);
    }
    fn inspect(&self) -> Option<document_session_api::ContentReport> {
        Some(self.doc.with_dom(content_report))
    }
    fn clip(&self) -> Option<DocumentClip> {
        let selection = self.doc.text_selection();
        self.doc.with_dom(|dom| match selection {
            Some(selection) => {
                let selected_links = links_for_source_nodes(dom, &selection.source_nodes);
                semantic_clip_from_selection_with_links(
                    &self.address,
                    dom,
                    ClipSelection {
                        range: ClipRange {
                            anchor_node: selection.range.anchor_node,
                            anchor_offset: selection.range.anchor_offset,
                            focus_node: selection.range.focus_node,
                            focus_offset: selection.range.focus_offset,
                        },
                        text: selection.text,
                    },
                    selected_links,
                )
            },
            None => semantic_clip_from_dom(&self.address, dom),
        })
    }
    /// Observation extras (extract, dom_snapshot, dispatch_event, dom stats)
    /// stay on the concrete type until the observation contract lands
    /// (session-engines plan phase 3 rescope); hosts reach them here.
    fn as_any_ref(&self) -> &dyn Any {
        self
    }
    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(feature = "scripted")]
impl<E: script_engine_api::ScriptEngine> ScriptedDocumentSession<E> {
    /// The concrete document, for observation downcasts (phase 3 rescope:
    /// extract / dom_snapshot / dispatch_event stay concrete until the
    /// observation contract lands).
    pub fn document_mut(&mut self) -> &mut genet_scripted::LiveryScriptedDocument<E> {
        &mut self.doc
    }
}
