// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;

use std::any::Any;

use document_session_api::session_engine::SessionRegistry;
use document_session_api::session_engine::{
    DocumentClip, DocumentClipArtifact, DocumentClipArtifactRole, DocumentFindDirection,
    DocumentFindMatch, DocumentFindQuery, DocumentFindReveal, DocumentFindState, DocumentSession,
    DocumentZoomState, SessionButtonState, SessionClick, SessionCursor, SessionEffect,
    SessionEngine, SessionError, SessionFocusDirection, SessionFormMethod, SessionFormSubmission,
    SessionIme, SessionKey, SessionLink, SessionModifiers, SessionScrollKey, SessionSpawnRequest,
    SessionTextTarget,
};
#[cfg(feature = "livery")]
use document_session_api::{
    DocumentA11yAction, DocumentA11yActionData, DocumentA11yActionRequest, DocumentA11yNodeId,
};
use document_session_api::{DocumentCapabilities, DocumentCapabilityStatus};
#[cfg(feature = "livery")]
use genet_document_resources::{ResourceKind, ResourceLimits, StylesheetOwner};
use genet_host_api::ResourceFetcher;
#[cfg(feature = "livery")]
use genet_host_api::ResourceResponse;
#[cfg(feature = "livery")]
use layout_dom_api::LayoutDom;
use layout_dom_api::{LayoutDomMut, LocalName, Namespace, NodeKind, QualName};
use netrender::Scene;
#[cfg(feature = "livery")]
use std::sync::{Arc, Mutex};

/// Byte source for spawn-with-body tests; never fetches.
#[derive(Clone)]
struct NoFetch;
impl ResourceFetcher for NoFetch {
    fn fetch(&self, _url: &str) -> Option<Vec<u8>> {
        None
    }
}
#[cfg(feature = "livery")]
struct ImageFetch {
    bytes: Vec<u8>,
    requests: Arc<Mutex<Vec<String>>>,
}

#[cfg(feature = "livery")]
impl ResourceFetcher for ImageFetch {
    fn fetch(&self, url: &str) -> Option<Vec<u8>> {
        self.requests.lock().unwrap().push(url.to_owned());
        Some(self.bytes.clone())
    }
}

#[cfg(feature = "livery")]
fn livery_node_with_id(
    session: &LiveryDocumentSession,
    wanted_id: &str,
) -> genet_scripted_dom::NodeId {
    fn find(
        dom: &genet_scripted_dom::ScriptedDom,
        node: genet_scripted_dom::NodeId,
        wanted_id: &str,
    ) -> Option<genet_scripted_dom::NodeId> {
        if dom.attribute(node, &Namespace::default(), &LocalName::from("id")) == Some(wanted_id) {
            return Some(node);
        }
        dom.dom_children(node)
            .find_map(|child| find(dom, child, wanted_id))
    }

    let dom = session.document().dom();
    find(dom, dom.document(), wanted_id)
        .unwrap_or_else(|| panic!("expected retained node #{wanted_id}"))
}

#[cfg(feature = "livery")]
struct LinkedResourceFetch {
    image: Vec<u8>,
    font: Vec<u8>,
    requests: Arc<Mutex<Vec<String>>>,
}

#[cfg(feature = "livery")]
impl ResourceFetcher for LinkedResourceFetch {
    fn fetch(&self, url: &str) -> Option<Vec<u8>> {
        self.requests.lock().unwrap().push(url.to_owned());
        match url {
            "https://example.test/docs/styles/site.css" => Some(
                br#".card { display: block; width: 80px; height: 40px; background-image: url(images/hero.png); }
@font-face { font-family: linked; src: url(../fonts/text.woff2); }"#
                    .to_vec(),
            ),
            "https://example.test/docs/styles/images/hero.png" => Some(self.image.clone()),
            "https://example.test/docs/fonts/text.woff2" => Some(self.font.clone()),
            _ => None,
        }
    }
}

#[cfg(feature = "livery")]
struct ImportedSheetFetch;

#[cfg(feature = "livery")]
impl ResourceFetcher for ImportedSheetFetch {
    fn fetch(&self, _url: &str) -> Option<Vec<u8>> {
        None
    }

    fn fetch_response(&self, url: &str) -> Option<genet_host_api::ResourceResponse> {
        match url {
            "https://example.test/docs/styles/root.css" => Some(
                genet_host_api::ResourceResponse::new(
                    "https://cdn.example.test/styles/root.css",
                    br#"@import "palette.css"; .card { color: rgb(255, 0, 0); }"#.to_vec(),
                )
                .with_content_type("text/css"),
            ),
            "https://cdn.example.test/styles/palette.css" => Some(
                genet_host_api::ResourceResponse::new(
                    url,
                    br#".card { color: rgb(0, 0, 255); }"#.to_vec(),
                )
                .with_content_type("text/css; charset=utf-8"),
            ),
            _ => None,
        }
    }
}

#[cfg(feature = "livery")]
struct RedirectedDocumentFetch;

#[cfg(feature = "livery")]
impl ResourceFetcher for RedirectedDocumentFetch {
    fn fetch(&self, _url: &str) -> Option<Vec<u8>> {
        None
    }

    fn fetch_response(&self, url: &str) -> Option<genet_host_api::ResourceResponse> {
        match url {
            "https://example.test/start" => Some(
                genet_host_api::ResourceResponse::new(
                    "https://cdn.example.test/final/index.html",
                    br#"<link rel="stylesheet" href="site.css"><main class="card"><h1>final base</h1></main>"#
                        .to_vec(),
                )
                .with_content_type("text/html"),
            ),
            "https://cdn.example.test/final/site.css" => Some(
                genet_host_api::ResourceResponse::new(
                    url,
                    br#".card { color: rgb(0, 128, 0); }"#.to_vec(),
                )
                .with_content_type("text/css"),
            ),
            _ => None,
        }
    }
}

#[cfg(feature = "scripted")]
#[test]
fn scripted_session_selects_and_clips_the_live_dom() {
    let engine =
        ScriptedSessionEngine::<script_engine_boa::BoaEngine, _>::new("genet.scripted", NoFetch);
    let request = SessionSpawnRequest::new("https://example.test/report")
        .with_body(
            "<html><head><title>Live Page</title></head><body style=\"margin:0\">\
             <p style=\"margin:0\">before <a id=\"choice\" href=\"/chosen\"></a> and \
             <a href=\"/also\">second link</a> after <a href=\"/outside\">outside</a></p>\
             <script>document.getElementById('choice').appendChild(\
             document.createTextNode('selected link'));</script>\
             </body></html>",
        )
        .with_viewport(640, 200);
    let mut session = engine.spawn(&request).expect("scripted lane spawns");
    let unselected = session.frame(640, 200);
    let unselected_rects = unselected
        .ops
        .iter()
        .filter(|op| matches!(op, netrender::SceneOp::Rect(_)))
        .count();
    let report = session.inspect().expect("live DOM is inspectable");
    assert_eq!(report.title.as_deref(), Some("Live Page"));

    let target = session
        .text_target("selected link")
        .expect("post-script first link resolves to pointer endpoints");
    let second = session
        .text_target("second link")
        .expect("second link resolves to pointer endpoints");
    assert_eq!(
        session.pointer_down(target.anchor[0], target.anchor[1]),
        SessionClick::Handled
    );
    assert!(
        session.pointer_move(second.focus[0], second.focus[1]),
        "the live range extends through ordinary pointer input"
    );
    assert_eq!(
        session.pointer_up(second.focus[0], second.focus[1]),
        SessionClick::Handled
    );
    let selected = session.frame(640, 200);
    let selected_rects = selected
        .ops
        .iter()
        .filter(|op| matches!(op, netrender::SceneOp::Rect(_)))
        .count();
    assert!(
        selected_rects > unselected_rects,
        "the retained live range paints selection geometry"
    );

    let clip = session.clip().expect("live selection supplies a clip");
    assert_eq!(clip.source_url, "https://example.test/report");
    assert_eq!(clip.text, "selected link and second link");
    assert_eq!(clip.links, vec!["/chosen", "/also"]);
    let selector: serde_json::Value =
        serde_json::from_str(clip.selector.as_deref().expect("range selector"))
            .expect("selector is typed JSON");
    assert_eq!(selector["type"], "dom-range");
    assert_eq!(selector["version"], 1);
    assert_eq!(selector["quote"], "selected link and second link");
    assert!(selector["anchor"]["path"].is_array());
    assert!(selector["focus"]["path"].is_array());
}

#[cfg(feature = "scripted")]
#[test]
fn scripted_session_returns_only_uncancelled_external_navigation() {
    let engine =
        ScriptedSessionEngine::<script_engine_boa::BoaEngine, _>::new("genet.scripted", NoFetch);
    let request = SessionSpawnRequest::new("https://example.test/start")
        .with_body(
            r#"<body style="margin:0">
                <style>a { display:block; width:180px; padding:20px; }</style>
                <p>Selection start</p>
                <a href="next.html"><span>Open next</span></a>
                <a id="blocked" href="blocked.html">Stay here</a>
                <a id="changed" href="old.html">Change target</a>
                <script>
                  document.addEventListener('click', function (event) {
                    if (event.target.id === 'blocked') event.preventDefault();
                    if (event.target.id === 'changed') {
                      event.target.setAttribute('href', 'changed.html');
                    }
                  });
                </script>
            </body>"#,
        )
        .with_viewport(640, 200);
    let mut session = engine.spawn(&request).expect("scripted lane spawns");
    let _scene = session.frame(640, 200);
    session.pump(1.0);

    let right_padding = |target: SessionTextTarget| {
        (
            target.focus[0] + 8.0,
            (target.anchor[1] + target.focus[1]) * 0.5,
        )
    };
    let next_text = session
        .text_target("Open next")
        .expect("next link geometry");
    let next_padding = right_padding(next_text);
    assert_eq!(
        session.pointer_down(next_padding.0, next_padding.1),
        SessionClick::Handled,
        "an anchor-padding press starts capture without navigating"
    );
    assert_eq!(
        session.pointer_up(
            next_text.focus[0] - 0.5,
            (next_text.anchor[1] + next_text.focus[1]) * 0.5,
        ),
        SessionClick::Navigate("next.html".to_owned()),
        "release on the same anchor's inline child keeps one activation target"
    );

    assert_eq!(
        session.pointer_down(next_padding.0, next_padding.1),
        SessionClick::Handled
    );
    assert_eq!(
        session.pointer_up(500.0, 190.0),
        SessionClick::Handled,
        "release outside becomes a selection instead of navigation"
    );
    assert_eq!(
        session.pointer_up(next_padding.0, next_padding.1),
        SessionClick::Miss,
        "a mismatched release clears the retained press"
    );

    let blocked = right_padding(
        session
            .text_target("Stay here")
            .expect("cancelled link geometry"),
    );
    assert_eq!(
        session.pointer_down(blocked.0, blocked.1),
        SessionClick::Handled
    );
    assert_eq!(
        session.pointer_up(blocked.0, blocked.1),
        SessionClick::Handled,
        "preventDefault cancels navigation through the ordinary release path"
    );

    let changed = right_padding(
        session
            .text_target("Change target")
            .expect("mutating link geometry"),
    );
    assert_eq!(
        session.pointer_down(changed.0, changed.1),
        SessionClick::Handled
    );
    assert_eq!(
        session.pointer_up(changed.0, changed.1),
        SessionClick::Navigate("changed.html".to_owned()),
        "the default action reads href after uncancelled listeners run"
    );

    assert_eq!(
        session.pointer_down(blocked.0, blocked.1),
        SessionClick::Handled
    );
    let cancelled = session.input(document_session_api::SessionInput::Cancel);
    assert_eq!(cancelled.effect, SessionEffect::Cancelled);
    assert_eq!(
        session.pointer_up(blocked.0, blocked.1),
        SessionClick::Miss,
        "cancelled capture cannot activate on a later release"
    );

    assert_eq!(
        session.pointer_down(changed.0, changed.1),
        SessionClick::Handled
    );
    let blurred = session.input(document_session_api::SessionInput::Focus(false));
    assert_eq!(blurred.effect, SessionEffect::Handled);
    assert_eq!(
        session.pointer_up(changed.0, changed.1),
        SessionClick::Miss,
        "focus loss clears a captured press before its later release"
    );

    let drag_start = session
        .text_target("Selection start")
        .expect("selection start geometry");
    assert_eq!(
        session.pointer_down(drag_start.anchor[0], drag_start.anchor[1]),
        SessionClick::Handled
    );
    assert!(session.pointer_move(next_text.focus[0], next_text.focus[1]));
    assert_eq!(
        session.pointer_up(next_text.focus[0], next_text.focus[1]),
        SessionClick::Handled,
        "a drag selection ending on a link wins over navigation"
    );
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_routes_retained_structural_and_text_paint() {
    let mut registry: SessionRegistry<Scene> = SessionRegistry::new();
    registry.register(Box::new(LiverySessionEngine::new(NoFetch)));
    assert!(registry.contains(document_session_api::engine_ids::ENGINE_GENET_LIVERY));

    let request = SessionSpawnRequest::new("https://example.test/")
        .with_body(
            r#"<html><head><style>.card { background-color: navy; color: white; width: 120px; }</style></head><body><div class="card">Livery <span>session</span></div></body></html>"#,
        )
        .with_viewport(320, 240);
    let mut session = registry
        .spawn(
            document_session_api::engine_ids::ENGINE_GENET_LIVERY,
            &request,
        )
        .expect("registered Livery lane spawns from body");

    let first = session.frame(320, 240);
    assert!(
        first
            .ops
            .iter()
            .any(|operation| matches!(operation, netrender::SceneOp::Rect(_)))
    );
    assert!(
        first
            .ops
            .iter()
            .any(|operation| matches!(operation, netrender::SceneOp::GlyphRun(_)))
    );
    let concrete = session
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("session keeps its concrete Livery owner");
    let generation = concrete.document().generation();
    let shape_count = concrete.document().text_system().shape_count();
    assert_eq!(concrete.last_error(), None);

    let _cached = session.frame(320, 240);
    let concrete = session
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("session keeps its concrete Livery owner");
    assert_eq!(concrete.document().generation(), generation);
    assert_eq!(concrete.document().text_system().shape_count(), shape_count);
    assert!(!session.scroll_by(0.0, 100.0));
    assert_eq!(session.click_at(20.0, 20.0), SessionClick::Miss);
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_activates_the_first_matching_text_directive() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new(
        "https://example.test/article#:~:text=missing&text=prefix-,target,end,-suffix",
    )
    .with_body(
        r#"<html><head><style>body { margin: 0; }</style></head><body>
            <div style="height: 900px"></div><p>prefix target end suffix</p>
            </body></html>"#,
    )
    .with_viewport(320, 160);
    let mut session = engine.spawn(&request).expect("livery session spawns");

    let scene = session.frame(320, 160);
    let concrete = session
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("retained static session");
    let selection = concrete
        .document()
        .text_selection()
        .expect("the second directive matched in source order");
    assert_eq!(selection.text, "target end");
    assert!(
        concrete.document().scroll().1 > 0.0,
        "activation reveals the retained match"
    );
    assert!(
        scene
            .ops
            .iter()
            .any(|operation| matches!(operation, netrender::SceneOp::Rect(_))),
        "the selection is emitted as scene indication geometry"
    );
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_falls_back_to_the_ordinary_element_fragment() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request =
        SessionSpawnRequest::new("https://example.test/article#fallback:~:text=not-present")
            .with_body(
                r#"<html><head><style>body { margin: 0; }</style></head><body>
            <div style="height: 900px"></div><p id="fallback">ordinary fallback</p>
            </body></html>"#,
            )
            .with_viewport(320, 160);
    let mut session = engine.spawn(&request).expect("livery session spawns");

    let _scene = session.frame(320, 160);
    let concrete = session
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("retained static session");
    assert!(concrete.document().text_selection().is_none());
    assert!(
        concrete.document().scroll().1 > 0.0,
        "an unmatched text directive falls through to #fallback"
    );
}

/// Page zoom is a document scale the host requests and the engine bounds:
/// the requested factor comes back untouched for the host to persist, while
/// `applied` is what this lane could honour.
#[cfg(feature = "livery")]
#[test]
fn livery_page_zoom_reports_requested_and_clamped_applied() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("https://example.test/")
        .with_body("<html><body><p>zoom</p></body></html>")
        .with_viewport(320, 240);
    let mut session = engine.spawn(&request).expect("livery lane spawns");

    let state = session
        .set_page_zoom(1.25)
        .expect("livery honours page zoom");
    assert_eq!(state.requested, 1.25);
    assert_eq!(state.applied, 1.25);
    assert_eq!((state.min, state.max), (0.25, 5.0));
    assert_eq!(
        session
            .as_any_ref()
            .downcast_ref::<LiveryDocumentSession>()
            .expect("livery session remains observable")
            .page_zoom(),
        1.25
    );

    let state = session
        .set_page_zoom(12.0)
        .expect("an out-of-range request");
    assert_eq!(state.requested, 12.0, "the request stays the caller's");
    assert_eq!(state.applied, 5.0, "the engine owns its bounds");

    let state = session
        .set_page_zoom(0.05)
        .expect("an out-of-range request");
    assert_eq!(state.requested, 0.05);
    assert_eq!(state.applied, 0.25);

    let state = session.set_page_zoom(1.0).expect("reset is factor 1.0");
    assert_eq!(state.applied, 1.0);
}

/// Zoom is a user-agent document scale, not a CSS `zoom`: the CSS viewport
/// shrinks by the factor, so a `max-width` media query flips at a wider
/// presentation viewport than its own boundary.
#[cfg(feature = "livery")]
#[test]
fn livery_page_zoom_shrinks_the_css_viewport_media_queries_see() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("https://example.test/")
        .with_body(
            r#"<html><head><style>
                html, body { margin: 0; padding: 0; }
                .grow { height: 100px; }
                @media (max-width: 500px) { .grow { height: 900px; } }
            </style></head><body><div class="grow"></div></body></html>"#,
        )
        .with_viewport(600, 300);
    let mut session = engine.spawn(&request).expect("livery lane spawns");

    let _scene = session.frame(600, 300);
    assert!(
        session.content_height(600, 300) < 500,
        "a 600px presentation viewport is a 600px CSS viewport at 100 %"
    );

    session
        .set_page_zoom(1.25)
        .expect("livery honours page zoom");
    let _scene = session.frame(600, 300);
    let content = session.content_height(600, 300);
    assert!(
        content > 1000,
        "the 480px CSS viewport matches the query and the 900px block \
         scales back up: {content}"
    );
}

/// The boundary space is stable: link rects leave in presentation space and
/// pointer coordinates arrive in it, so one visual point keeps resolving to
/// the same node as the document scales under it.
#[cfg(feature = "livery")]
#[test]
fn livery_page_zoom_keeps_hit_testing_at_the_same_visual_point() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("https://example.test/")
        .with_body(
            r#"<html><head><style>
                html, body { margin: 0; padding: 0; }
                a { display: block; height: 20px; width: 100px; }
            </style></head><body><a href="/next">next</a></body></html>"#,
        )
        .with_viewport(320, 240);
    let mut session = engine.spawn(&request).expect("livery lane spawns");
    let _scene = session.frame(320, 240);

    let rect = session
        .links()
        .into_iter()
        .next()
        .expect("retained link")
        .rect;
    assert!((rect[2] - 100.0).abs() < 1.0, "{rect:?}");
    assert_eq!(
        session.click_at(rect[0] + 10.0, rect[1] + 5.0),
        SessionClick::Navigate("/next".to_owned())
    );
    // Activation restyles the anchor, so the retained layout the hit table
    // reads is only current again after the next frame.
    let _scene = session.frame(320, 240);
    assert_eq!(session.click_at(110.0, 5.0), SessionClick::Miss);

    session
        .set_page_zoom(1.25)
        .expect("livery honours page zoom");
    let _scene = session.frame(320, 240);

    let zoomed = session
        .links()
        .into_iter()
        .next()
        .expect("retained link")
        .rect;
    assert!(
        (zoomed[2] - 125.0).abs() < 1.0,
        "the reported rect is presentation space: {zoomed:?}"
    );
    assert_eq!(
        session.click_at(rect[0] + 10.0, rect[1] + 5.0),
        SessionClick::Navigate("/next".to_owned()),
        "the same visual point still hits the link"
    );
    let _scene = session.frame(320, 240);
    assert_eq!(
        session.click_at(110.0, 5.0),
        SessionClick::Navigate("/next".to_owned()),
        "and the point the grown link now covers hits it too"
    );
}

/// A find reveal is a document offset the host applies in presentation
/// space, so it scales with zoom and survives the round trip back into the
/// retained CSS-space scroll.
#[cfg(feature = "livery")]
#[test]
fn livery_page_zoom_keeps_find_reveal_in_presentation_space() {
    fn reveal(state: &DocumentFindState) -> f32 {
        match state.current_match().expect("a current match").reveal {
            DocumentFindReveal::ScrollY(y) => y,
            DocumentFindReveal::EngineManaged => panic!("the livery lane reveals by offset"),
        }
    }

    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("https://example.test/find")
        .with_body(
            r#"<html><head><style>html, body { margin: 0; padding: 0; }</style></head>
            <body><div style="height: 1200px"></div><p>Last finding.</p>
            <div style="height: 1200px"></div></body></html>"#,
        )
        .with_viewport(480, 240);
    let mut session = engine.spawn(&request).expect("livery lane spawns");
    let _scene = session.frame(480, 240);

    let query = DocumentFindQuery::new("finding");
    let at_100 = reveal(&session.document_find(&query).expect("retained find"));
    assert!(at_100 > 1000.0, "the match sits below the fold: {at_100}");

    session
        .set_page_zoom(1.25)
        .expect("livery honours page zoom");
    let _scene = session.frame(480, 240);
    let at_125 = reveal(&session.document_find(&query).expect("retained find"));
    assert!(
        (at_125 - at_100 * 1.25).abs() < 1.0,
        "the reveal offset scales with the document: {at_100} {at_125}"
    );

    let concrete = session
        .as_any()
        .downcast_ref::<LiveryDocumentSession>()
        .expect("session keeps its concrete Livery owner");
    let scrolled = concrete.document().scroll().1;
    assert!(
        (at_125 / 1.25 - 24.0 - scrolled).abs() < 1.0,
        "the same offset converts back into the retained CSS scroll: \
         {at_125} {scrolled}"
    );
}

/// The livery lane's structural report through the trait — the same
/// contract the static lane serves, so a viewer override to livery keeps
/// the Inspector/a11y read instead of degrading to "none for this lane".
#[cfg(feature = "livery")]
#[test]
fn livery_session_reports_structure_through_the_trait() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("https://example.test/")
        .with_body(
            "<html><head><title>The Page</title></head>\
             <body><h1>Heading</h1><a href=\"/next\">next</a></body></html>",
        )
        .with_viewport(640, 480);
    let mut session = engine.spawn(&request).expect("livery lane spawns");
    let report = session
        .inspect()
        .expect("the livery lane has a structural read");
    assert_eq!(report.title.as_deref(), Some("The Page"));
    assert_eq!(report.headings, vec!["Heading"]);
    assert_eq!(report.links, vec!["/next"]);

    let _scene = session.frame(640, 480);
    let link = session.links().into_iter().next().expect("retained link");
    let pointer = |state| document_session_api::SessionInput::PointerButton {
        x: link.rect[0] + 2.0,
        y: link.rect[1] + 2.0,
        button: document_session_api::SessionPointerButton::Primary,
        state,
        modifiers: SessionModifiers::default(),
    };
    let pressed = session.input(pointer(SessionButtonState::Pressed));
    assert_eq!(pressed.effect, SessionEffect::Handled);
    assert_eq!(pressed.capture, Some(true));
    let released = session.input(pointer(SessionButtonState::Released));
    assert_eq!(released.effect, SessionEffect::Navigate("/next".to_owned()));
    assert_eq!(released.capture, Some(false));
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_retains_structural_find_and_wraps_reveal() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("https://example.test/find")
        .with_body(
            "<html><body><h1>Finding heading</h1><p>First finding.</p>\
             <div style=\"height: 1200px\"></div><p>Last finding.</p></body></html>",
        )
        .with_viewport(480, 240);
    let mut session = engine.spawn(&request).expect("livery lane spawns");
    let _ = session.frame(480, 240);

    let capabilities = session.document_capabilities();
    assert_eq!(
        capabilities.find_in_page,
        DocumentCapabilityStatus::Supported
    );
    assert_eq!(capabilities.page_zoom, DocumentCapabilityStatus::Supported);
    assert!(matches!(
        capabilities.page_capture,
        DocumentCapabilityStatus::Unsupported { .. }
    ));
    assert!(matches!(
        capabilities.navigation,
        DocumentCapabilityStatus::Partial { .. }
    ));

    let state = session
        .document_find(&DocumentFindQuery::new("finding"))
        .expect("livery supplies retained find");
    assert_eq!(state.matches.len(), 3);
    assert_eq!(state.count, 3);
    assert_eq!(state.current, Some(0));
    assert_eq!(
        state.current_match().and_then(|item| item.role.as_deref()),
        Some("heading")
    );
    assert_eq!(
        state.current_match().map(|item| item.label.as_str()),
        Some("Finding heading")
    );

    let state = session
        .document_find_step(DocumentFindDirection::Previous)
        .expect("previous wraps");
    assert_eq!(state.current, Some(2));
    assert_eq!(
        state.current_match().map(|item| item.label.as_str()),
        Some("Last finding.")
    );
    let concrete = session
        .as_any_ref()
        .downcast_ref::<LiveryDocumentSession>()
        .expect("livery session remains concrete");
    assert!(
        concrete.document().scroll().1 > 0.0,
        "wrapped match is revealed"
    );
    assert!(concrete.document().text_selection().is_some());

    let state = session
        .document_find_step(DocumentFindDirection::Next)
        .expect("next wraps");
    assert_eq!(state.current, Some(0));

    let changed = session
        .document_find(&DocumentFindQuery::new("LAST"))
        .expect("query replacement recomputes the model");
    assert_eq!(changed.matches.len(), 1);
    assert_eq!(changed.current, Some(0));
    assert_eq!(changed.matches[0].role.as_deref(), Some("paragraph"));
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_clip_retains_the_source_response() {
    let engine = LiverySessionEngine::new(NoFetch);
    let body = "<html><head><title>The Page</title></head><body><main>\
                <h1>Heading</h1><p>A useful finding.</p></main></body></html>";
    let request = SessionSpawnRequest::new("https://example.test/report")
        .with_body(body)
        .with_content_type("text/html; charset=utf-8");
    let session = engine.spawn(&request).expect("livery lane spawns");
    let clip = session.clip().expect("the livery lane can supply a clip");

    assert_eq!(clip.artifacts.len(), 1);
    assert_eq!(
        clip.artifacts[0].role,
        DocumentClipArtifactRole::SourceResponse
    );
    assert_eq!(clip.artifacts[0].media_type, "text/html; charset=utf-8");
    assert_eq!(
        clip.artifacts[0].canonical_uri,
        "https://example.test/report"
    );
    assert_eq!(clip.artifacts[0].bytes, body.as_bytes());
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_pointer_selection_scopes_clip_and_selector() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("https://example.test/report")
        .with_body(
            "<html><head><title>The Page</title><style>\
             html, body, p { margin: 0; padding: 0; }\
             </style></head><body><p>before \
             <a href=\"/chosen\">selected link</a> after \
             <a href=\"/outside\">outside</a></p></body></html>",
        )
        .with_viewport(640, 200);
    let mut session = engine.spawn(&request).expect("spawns");
    let unselected = session.frame(640, 200);
    let unselected_rects = unselected
        .ops
        .iter()
        .filter(|op| matches!(op, netrender::SceneOp::Rect(_)))
        .count();
    let target = session
        .text_target("selected link")
        .expect("Livery source ranges resolve to pointer endpoints");

    assert_eq!(
        session.pointer_down(target.anchor[0], target.anchor[1]),
        SessionClick::Handled
    );
    assert!(
        session.pointer_move(target.focus[0], target.focus[1]),
        "the range extends through ordinary pointer input"
    );
    assert_eq!(
        session.pointer_up(target.focus[0], target.focus[1]),
        SessionClick::Handled
    );

    let selected = session.frame(640, 200);
    let selected_rects = selected
        .ops
        .iter()
        .filter(|op| matches!(op, netrender::SceneOp::Rect(_)))
        .count();
    assert!(
        selected_rects > unselected_rects,
        "the retained Livery range paints selection geometry"
    );

    let clip = session.clip().expect("selection supplies a clip");
    assert_eq!(clip.source_url, "https://example.test/report");
    assert_eq!(clip.text, "selected link");
    assert_eq!(clip.links, vec!["/chosen"]);
    let selector: serde_json::Value =
        serde_json::from_str(clip.selector.as_deref().expect("range selector"))
            .expect("selector is typed JSON");
    assert_eq!(selector["type"], "dom-range");
    assert_eq!(selector["version"], 1);
    assert_eq!(selector["quote"], "selected link");
    assert!(selector["anchor"]["path"].is_array());
    assert!(selector["focus"]["path"].is_array());
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_routes_scroll_focus_and_links() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("https://example.test/")
        .with_body(
            r##"<html><head><style>
                html, body { margin: 0; padding: 0; }
                .link, .target { display: block; width: 100px; height: 20px; }
                .spacer { height: 500px; }
            </style></head><body>
                <a class="link" href="#target">top</a>
                <div class="spacer"></div>
                <div id="target" class="target">target</div>
            </body></html>"##,
        )
        .with_viewport(320, 240);
    let mut session = engine.spawn(&request).expect("spawns");
    let _scene = session.frame(320, 240);

    assert!(session.content_height(320, 240) > 240);
    let link = session.links().into_iter().next().expect("retained link");
    let click = session.click_at(link.rect[0] + 5.0, link.rect[1] + 5.0);
    assert_eq!(click, SessionClick::Handled);
    assert!(session.scroll_for_key(SessionScrollKey::Home));
    assert!(session.scroll_by(0.0, 100.0));
    assert!(session.scroll_for_key(SessionScrollKey::Home));
    assert!(session.scroll_at(10.0, 10.0, 0.0, 100.0));
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_replaces_accessible_native_text_values() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("fixtures/form/index.html")
        .with_body(
            r#"<html><body><form action="result.html" method="get">
                <input id="query" name="query" type=" EMAIL " value="cedar">
                <textarea id="note" name="note">old note</textarea>
            </form></body></html>"#,
        )
        .with_viewport(400, 240);
    let mut boxed = engine.spawn(&request).expect("form session spawns");
    let session = boxed
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("Livery engine retains its concrete session");
    let query = livery_node_with_id(session, "query");
    let note = livery_node_with_id(session, "note");
    let query_id = session.document().dom().opaque_id(query);
    let note_id = session.document().dom().opaque_id(note);

    assert!(session.replace_accessible_text_value(query_id, "birch"));
    assert_eq!(session.attribute(query, "value"), Some("birch"));
    assert_eq!(
        session.editor.as_ref().map(|editor| {
            (
                editor.node,
                editor.kind,
                editor.value.as_str(),
                editor.caret,
            )
        }),
        Some((query, EditableKind::Input, "birch", "birch".len()))
    );
    assert_eq!(
        session.form_submission("fallback.html").fields,
        [
            ("query".to_owned(), "birch".to_owned()),
            ("note".to_owned(), "old note".to_owned()),
        ]
    );

    assert!(session.replace_accessible_text_value(note_id, "new note"));
    assert_eq!(session.text_content(note), "new note");
    assert_eq!(
        session.editor.as_ref().map(|editor| {
            (
                editor.node,
                editor.kind,
                editor.value.as_str(),
                editor.caret,
            )
        }),
        Some((note, EditableKind::Textarea, "new note", "new note".len()))
    );
    let submission = session.form_submission("fallback.html");
    assert_eq!(submission.action, "result.html");
    assert_eq!(submission.method, SessionFormMethod::Get);
    assert_eq!(
        submission.fields,
        [
            ("query".to_owned(), "birch".to_owned()),
            ("note".to_owned(), "new note".to_owned()),
        ]
    );
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_rejects_inaccessible_native_text_value_replacements_and_text_edits() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("fixtures/form/index.html")
        .with_body(
            r#"<html><body><form action="result.html">
                <input id="disabled" disabled value="locked">
                <textarea id="readonly" readonly>fixed</textarea>
                <input id="aria-disabled" aria-disabled=" TRUE " value="aria locked">
                <textarea id="aria-readonly" aria-readonly="true">aria fixed</textarea>
                <input id="checkbox" type="checkbox" value="on">
                <input id="password" type="password" value="secret">
                <button id="disabled-submit" disabled tabindex="0" type="submit">send</button>
            </form></body></html>"#,
        )
        .with_viewport(400, 240);
    let mut boxed = engine.spawn(&request).expect("control session spawns");
    let session = boxed
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("Livery engine retains its concrete session");
    let disabled = livery_node_with_id(session, "disabled");
    let readonly = livery_node_with_id(session, "readonly");
    let aria_disabled = livery_node_with_id(session, "aria-disabled");
    let aria_readonly = livery_node_with_id(session, "aria-readonly");
    let checkbox = livery_node_with_id(session, "checkbox");
    let password = livery_node_with_id(session, "password");
    let disabled_submit = livery_node_with_id(session, "disabled-submit");
    let disabled_id = session.document().dom().opaque_id(disabled);
    let readonly_id = session.document().dom().opaque_id(readonly);
    let aria_disabled_id = session.document().dom().opaque_id(aria_disabled);
    let aria_readonly_id = session.document().dom().opaque_id(aria_readonly);
    let checkbox_id = session.document().dom().opaque_id(checkbox);
    let password_id = session.document().dom().opaque_id(password);

    assert!(!session.replace_accessible_text_value(disabled_id, "changed"));
    assert!(!session.replace_accessible_text_value(readonly_id, "changed"));
    assert!(!session.replace_accessible_text_value(aria_disabled_id, "changed"));
    assert!(!session.replace_accessible_text_value(aria_readonly_id, "changed"));
    assert!(!session.replace_accessible_text_value(checkbox_id, "changed"));
    assert!(!session.replace_accessible_text_value(password_id, "changed"));
    assert!(
        !session.replace_accessible_text_value(u64::MAX, "changed"),
        "malformed or foreign local IDs are inert"
    );
    assert_eq!(session.attribute(disabled, "value"), Some("locked"));
    assert_eq!(session.text_content(readonly), "fixed");
    assert_eq!(
        session.attribute(aria_disabled, "value"),
        Some("aria locked")
    );
    assert_eq!(session.text_content(aria_readonly), "aria fixed");
    assert_eq!(session.attribute(checkbox, "value"), Some("on"));
    assert_eq!(session.attribute(password, "value"), Some("secret"));
    assert!(
        session.editor.is_none(),
        "rejected controls do not take edit focus"
    );

    for node in [disabled, readonly, aria_disabled, aria_readonly] {
        // Clicking can invalidate retained layout through focus styling. Render
        // the next frame before resolving the next control's hit geometry.
        let _scene = session.frame(400, 240);
        let [x, y, width, height] = session.document().fragment_rect(node).unwrap_or_else(|| {
            panic!(
                "native control {:?} has retained geometry",
                session.attribute(node, "id")
            )
        });
        let _ = session.click_at(x + width * 0.5, y + height * 0.5);
        assert!(
            !session.text_input("changed"),
            "disabled and readonly controls reject ordinary text input"
        );
        assert_eq!(
            session.key_input(
                SessionKey::Enter,
                SessionButtonState::Pressed,
                SessionModifiers::default(),
                false,
            ),
            SessionEffect::Ignored,
            "disabled and readonly controls do not retain a form submit target"
        );
    }
    assert_eq!(session.attribute(disabled, "value"), Some("locked"));
    assert_eq!(session.text_content(readonly), "fixed");
    assert_eq!(
        session.attribute(aria_disabled, "value"),
        Some("aria locked")
    );
    assert_eq!(session.text_content(aria_readonly), "aria fixed");
    let _scene = session.frame(400, 240);
    let [x, y, width, height] = session
        .document()
        .fragment_rect(disabled_submit)
        .expect("disabled submit has retained geometry");
    assert!(
        !matches!(
            session.click_at(x + width * 0.5, y + height * 0.5),
            SessionClick::Submit(_)
        ),
        "a disabled submit control is inert under physical click"
    );
    assert_eq!(
        session.key_input(
            SessionKey::Enter,
            SessionButtonState::Pressed,
            SessionModifiers::default(),
            false,
        ),
        SessionEffect::Ignored,
        "a disabled submit control cannot retain an Enter submit target"
    );
    let mut focusable = Vec::new();
    session.collect_focusable(session.document().dom().document(), &mut focusable);
    assert!(
        !focusable.contains(&disabled_submit),
        "a disabled tabindex submit control stays out of sequential focus"
    );
    session.focused_node = None;
    session.active_form = None;
    let _scene = session.frame(400, 240);
    assert!(session.focus_move(SessionFocusDirection::Forward));
    assert_ne!(session.focused_node, Some(disabled_submit));
    assert_eq!(
        session.key_input(
            SessionKey::Enter,
            SessionButtonState::Pressed,
            SessionModifiers::default(),
            false,
        ),
        SessionEffect::Ignored,
        "Tab cannot leave a disabled submit control armed for Enter"
    );
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_projects_accessible_pointer_targets_in_css_space() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("https://example.test/")
        .with_body(
            r#"<html><head><style>
                html, body { margin: 0; padding: 0; }
                #scroller { width: 100px; height: 40px; overflow-y: auto; }
                #before { height: 30px; }
                #target, #label { display: block; width: 100px; height: 40px; }
                #tail { height: 120px; }
            </style></head><body><div id="scroller"><div id="before"></div>
            <a id="target" href="/next"><span id="label">Open</span></a>
            <div id="tail"></div></div></body></html>"#,
        )
        .with_viewport(320, 160);
    let mut boxed = engine.spawn(&request).expect("Livery session spawns");
    boxed
        .set_page_zoom(1.25)
        .expect("Livery accepts the focused page zoom");
    let _scene = boxed.frame(320, 160);
    let session = boxed
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("Livery engine retains its concrete session");
    let scroller = livery_node_with_id(session, "scroller");
    let target = livery_node_with_id(session, "target");
    let target_id = session.document().dom().opaque_id(target);
    assert!(session.doc.scroll_at(5.0, 5.0, 0.0, 45.0));

    let css = session
        .document()
        .accessible_pointer_target(target)
        .expect("partly visible link has a CSS-space target");
    assert_eq!(
        session.accessible_pointer_target(target_id),
        Some(css),
        "the concrete session preserves Livery's CSS coordinate ownership"
    );
    assert_eq!(
        session.click_at(css.0 * 1.25, css.1 * 1.25),
        SessionClick::Navigate("/next".to_owned()),
        "the host-side CSS-to-presentation transform reaches normal Livery input"
    );
    assert!(
        session
            .document()
            .element_scroll()
            .get(&scroller)
            .is_some_and(|&(_, y)| y > 0.0),
        "activation leaves the nested retained scroll state in Livery"
    );
    assert_eq!(
        session.accessible_pointer_target(u64::MAX),
        None,
        "foreign local ids are inert"
    );
}

#[cfg(feature = "livery")]
#[test]
fn livery_accessibility_projection_revisions_follow_semantics_and_geometry() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("https://example.test/")
        .with_body(
            r#"<html><body>
                <a id="next" href="/next">Next page</a>
                <input id="query" aria-label="Search" value="cedar">
            </body></html>"#,
        )
        .with_viewport(400, 240);
    let mut boxed = engine.spawn(&request).expect("Livery session spawns");
    let _scene = boxed.frame(400, 240);
    let session = boxed
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("Livery engine retains its concrete session");

    let first = session
        .accessibility_projection()
        .expect("retained layout supplies an accessibility projection");
    let next = livery_node_with_id(session, "next");
    let next_id = DocumentA11yNodeId::new(session.document().dom().opaque_id(next));
    let next_node = first.node(next_id).expect("link is projected");
    assert_eq!(next_node.name.as_deref(), Some("Next page"));
    assert!(next_node.actions.contains(&DocumentA11yAction::Click));

    let unchanged = session
        .accessibility_projection()
        .expect("unchanged projection remains available");
    assert_eq!(unchanged.revision(), first.revision());
    assert_eq!(unchanged.nodes(), first.nodes());

    session
        .set_page_zoom(1.25)
        .expect("Livery accepts page zoom");
    let zoomed = session
        .accessibility_projection()
        .expect("zoomed projection remains available");
    assert!(
        zoomed.revision() > first.revision(),
        "presentation geometry changes advance the projection revision"
    );
    let zoomed_node = zoomed.node(next_id).expect("link remains projected");
    assert_ne!(zoomed_node.bounds, next_node.bounds);
}

#[cfg(feature = "livery")]
#[test]
fn livery_accessibility_projection_uses_zoomed_bounds_and_click_targets() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("https://example.test/")
        .with_body(
            r#"<html><head><style>
                html, body { margin: 0; padding: 0; }
                #target { display: block; width: 100px; height: 30px; }
            </style></head><body><a id="target" href="/next">Open</a></body></html>"#,
        )
        .with_viewport(320, 160);
    let mut boxed = engine.spawn(&request).expect("Livery session spawns");
    boxed.set_page_zoom(1.25).expect("page zoom is supported");
    let _scene = boxed.frame(320, 160);
    let session = boxed
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("Livery engine retains its concrete session");
    let target = livery_node_with_id(session, "target");
    let target_id = DocumentA11yNodeId::new(session.document().dom().opaque_id(target));
    let projection = session
        .accessibility_projection()
        .expect("retained layout supplies an accessibility projection");
    let bounds = projection
        .node(target_id)
        .and_then(|node| node.bounds)
        .expect("link bounds are projected");
    let css_bounds = session
        .document()
        .fragment_rect(target)
        .expect("link has retained CSS geometry");
    assert_eq!(bounds.x, css_bounds[0] * 1.25);
    assert_eq!(bounds.y, css_bounds[1] * 1.25);
    assert_eq!(bounds.width, css_bounds[2] * 1.25);
    assert_eq!(bounds.height, css_bounds[3] * 1.25);

    let css_point = session
        .document()
        .accessible_pointer_target(target)
        .expect("link has a current CSS click target");
    let click = session
        .accessibility_click_target(target_id)
        .expect("projected link has a current click target");
    assert_eq!(click.revision, projection.revision());
    assert_eq!(click.point.x, css_point.0 * 1.25);
    assert_eq!(click.point.y, css_point.1 * 1.25);
}

#[cfg(feature = "livery")]
#[test]
fn livery_accessibility_actions_reject_stale_revisions() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("fixtures/form/index.html")
        .with_body(
            r#"<html><head><style>
                html, body { margin: 0; padding: 0; }
                #scroller { width: 100px; height: 40px; overflow-y: auto; }
                #before { height: 30px; }
                #tail { height: 120px; }
            </style></head><body><input id="query" aria-label="Search" value="cedar">
                <div id="scroller"><div id="before"></div><div id="tail">tail</div></div>
            </body></html>"#,
        )
        .with_viewport(400, 160);
    let mut boxed = engine.spawn(&request).expect("Livery session spawns");
    let _scene = boxed.frame(400, 160);
    let session = boxed
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("Livery engine retains its concrete session");
    let input = livery_node_with_id(session, "query");
    let input_id = DocumentA11yNodeId::new(session.document().dom().opaque_id(input));
    let first = session
        .accessibility_projection()
        .expect("retained layout supplies an accessibility projection");
    let set_value = DocumentA11yActionRequest {
        revision: first.revision(),
        target: input_id,
        action: DocumentA11yAction::SetValue,
        data: Some(DocumentA11yActionData::Value("birch".to_owned())),
    };
    assert!(session.dispatch_accessibility_action(&set_value));
    assert!(!session.dispatch_accessibility_action(&set_value));
    assert_eq!(session.attribute(input, "value"), Some("birch"));

    let tail = livery_node_with_id(session, "tail");
    let target = DocumentA11yNodeId::new(session.document().dom().opaque_id(tail));
    let scroller = livery_node_with_id(session, "scroller");
    let _ = session.doc.scroll_at(5.0, 5.0, 0.0, 20.0);
    let scrolled = session
        .accessibility_projection()
        .expect("scrolled projection remains available");
    let scroll_revision = scrolled.revision();
    let scroll_into_view = DocumentA11yActionRequest {
        revision: scroll_revision,
        target,
        action: DocumentA11yAction::ScrollIntoView,
        data: None,
    };
    assert!(
        scrolled
            .node(target)
            .is_some_and(|node| node.actions.contains(&DocumentA11yAction::ScrollIntoView))
    );
    let _ = session.doc.scroll_at(5.0, 5.0, 0.0, 30.0);
    assert!(
        session
            .doc
            .element_scroll()
            .get(&scroller)
            .is_some_and(|&(_, y)| y > 0.0)
    );
    assert!(!session.dispatch_accessibility_action(&scroll_into_view));

    let current = session
        .accessibility_projection()
        .expect("changed scroll publishes a current projection");
    assert!(
        session.dispatch_accessibility_action(&DocumentA11yActionRequest {
            revision: current.revision(),
            target,
            action: DocumentA11yAction::ScrollIntoView,
            data: None,
        })
    );
    let revealed = session
        .accessibility_projection()
        .expect("revealed target republishes its actions");
    assert!(
        revealed
            .node(target)
            .is_some_and(|node| node.actions.contains(&DocumentA11yAction::Click)),
        "a clip-aware Livery pointer target restores Click at the engine boundary"
    );
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_drag_selects_a_textarea_without_creating_a_page_clip() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("fixtures/form/index.html")
        .with_body(
            r#"<html><head><style>
                html, body { margin: 0; padding: 0; }
                textarea { display: block; width: 220px; min-height: 40px; }
            </style></head><body><p>page selection</p><form>
                <textarea id="note" name="note">cedar</textarea>
            </form></body></html>"#,
        )
        .with_viewport(400, 160);
    let mut boxed = engine.spawn(&request).expect("form session spawns");
    let session = boxed
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("Livery engine retains its concrete session");
    let initial = session.frame(400, 160);
    let page = session
        .text_target("page selection")
        .expect("ordinary page text has retained pointer endpoints");
    assert_eq!(
        session.pointer_down(page.anchor[0], page.anchor[1]),
        SessionClick::Handled
    );
    assert!(session.pointer_move(page.focus[0], page.focus[1]));
    assert_eq!(
        session.pointer_up(page.focus[0], page.focus[1]),
        SessionClick::Handled
    );
    assert!(
        session.clip().is_some(),
        "the page selection supplies a clip"
    );
    let target = session
        .text_target("cedar")
        .expect("the textarea text has retained pointer endpoints");

    assert_eq!(
        session.pointer_down(target.focus[0], target.focus[1]),
        SessionClick::Handled
    );
    assert!(session.pointer_move(target.anchor[0], target.anchor[1]));
    assert_eq!(session.pointer_up(390.0, 150.0), SessionClick::Handled);
    let selection = session
        .editor
        .as_ref()
        .and_then(|editor| editor.selection)
        .expect("the reverse drag forms a directed editor-local range");
    assert!(selection.anchor > selection.focus);
    assert!(session.document().text_selection().is_none());
    assert!(
        session.clip().is_none(),
        "editor selection never becomes a page clip"
    );
    let selected = session.frame(400, 160);
    assert!(
        selected
            .ops
            .iter()
            .filter(|operation| matches!(operation, netrender::SceneOp::Rect(_)))
            .count()
            > initial
                .ops
                .iter()
                .filter(|operation| matches!(operation, netrender::SceneOp::Rect(_)))
                .count(),
        "the local range produces retained selection overlay geometry"
    );

    assert!(session.ime_input(SessionIme::Commit("oak".to_owned())));
    let note = livery_node_with_id(session, "note");
    assert_eq!(session.text_content(note), "oak");
    assert_eq!(
        session.form_submission("fallback.html").fields,
        [("note".to_owned(), "oak".to_owned())]
    );
    assert!(
        session
            .editor
            .as_ref()
            .is_some_and(|editor| editor.selection.is_none() && editor.caret == 3)
    );
    assert!(
        matches!(
            session.pointer_down(target.anchor[0], target.anchor[1]),
            SessionClick::Handled | SessionClick::Miss
        ),
        "a stale pre-mutation text point is inert rather than querying dead text"
    );
    assert!(session.document().text_selection().is_none());
    assert!(session.clip().is_none());
    assert!(matches!(
        session.pointer_up(target.anchor[0], target.anchor[1]),
        SessionClick::Handled | SessionClick::Miss
    ));
    let _scene = session.frame(400, 160);
    let caret_target = session
        .text_target("oak")
        .expect("the replacement keeps retained pointer endpoints");
    assert_eq!(
        session.pointer_down(caret_target.anchor[0], caret_target.anchor[1]),
        SessionClick::Handled
    );
    assert_eq!(
        session.pointer_up(caret_target.anchor[0], caret_target.anchor[1]),
        SessionClick::Handled
    );
    assert!(
        session
            .editor
            .as_ref()
            .is_some_and(|editor| editor.selection.is_none() && editor.caret == 0),
        "a collapsed editor gesture preserves its local caret"
    );

    let full = session
        .text_target("oak")
        .expect("the replacement keeps retained pointer endpoints");
    assert_eq!(
        session.pointer_down(full.focus[0], full.focus[1]),
        SessionClick::Handled
    );
    assert!(session.pointer_move(full.anchor[0], full.anchor[1]));
    assert_eq!(
        session.pointer_up(full.anchor[0], full.anchor[1]),
        SessionClick::Handled
    );
    assert!(session.delete_backward());
    assert_eq!(session.text_content(note), "");
    let empty = session.frame(400, 160);
    let empty_rects = empty
        .ops
        .iter()
        .filter(|operation| matches!(operation, netrender::SceneOp::Rect(_)))
        .count();
    session.focus_input(false);
    let unfocused = session.frame(400, 160);
    assert!(
        empty_rects
            > unfocused
                .ops
                .iter()
                .filter(|operation| matches!(operation, netrender::SceneOp::Rect(_)))
                .count(),
        "an empty focused textarea retains a caret overlay"
    );
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_edits_and_submits_a_retained_get_form() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("fixtures/form/index.html")
        .with_body(
            r#"<html><head><style>
                html, body { margin: 0; padding: 0; }
                textarea, button { display: block; width: 220px; min-height: 40px; margin: 8px; }
            </style></head><body><form action="result.html" method="get">
                <textarea id="note" name="note">cedar</textarea>
                <button id="submit" type="submit">send</button>
            </form></body></html>"#,
        )
        .with_viewport(400, 240);
    let mut session = engine.spawn(&request).expect("form session spawns");
    let initial = session.frame(400, 240);
    let initial_glyphs = initial
        .ops
        .iter()
        .filter_map(|operation| match operation {
            netrender::SceneOp::GlyphRun(run) => Some(run.glyphs.len()),
            _ => None,
        })
        .sum::<usize>();
    let target = session
        .text_target("cedar")
        .expect("the textarea value has a retained text target");
    let point = (target.anchor[0] + 2.0, target.anchor[1]);
    let pointer = |state| document_session_api::SessionInput::PointerButton {
        x: point.0,
        y: point.1,
        button: document_session_api::SessionPointerButton::Primary,
        state,
        modifiers: SessionModifiers::default(),
    };
    assert!(
        session
            .input(pointer(SessionButtonState::Pressed))
            .effect
            .is_handled()
    );
    let released = session.input(pointer(SessionButtonState::Released));
    assert!(released.editable);
    assert_eq!(released.cursor, Some(SessionCursor::Text));

    // The pointer target identifies the start caret. Select the end explicitly
    // because this test exercises appending and submission, not hit-test bias.
    assert_eq!(
        session
            .input(document_session_api::SessionInput::Key {
                key: SessionKey::End,
                state: SessionButtonState::Pressed,
                modifiers: SessionModifiers::default(),
                repeat: false,
            })
            .effect,
        SessionEffect::Handled,
    );

    let edited = session.input(document_session_api::SessionInput::Text(
        " and ash".to_owned(),
    ));
    assert_eq!(edited.effect, SessionEffect::Handled);
    let edited_scene = session.frame(400, 240);
    let edited_glyphs = edited_scene
        .ops
        .iter()
        .filter_map(|operation| match operation {
            netrender::SceneOp::GlyphRun(run) => Some(run.glyphs.len()),
            _ => None,
        })
        .sum::<usize>();
    assert!(
        edited_glyphs > initial_glyphs,
        "the textarea edit reaches paint"
    );
    assert!(
        session
            .inspect()
            .expect("form remains inspectable")
            .outline
            .iter()
            .any(|entry| entry.role == "textbox" && entry.name == "cedar and ash")
    );

    let tabbed = session.input(document_session_api::SessionInput::Key {
        key: SessionKey::Tab,
        state: SessionButtonState::Pressed,
        modifiers: SessionModifiers::default(),
        repeat: false,
    });
    assert_eq!(tabbed.effect, SessionEffect::Handled);
    assert!(!tabbed.editable, "Tab moves focus to the submit button");
    let submitted = session.input(document_session_api::SessionInput::Key {
        key: SessionKey::Enter,
        state: SessionButtonState::Pressed,
        modifiers: SessionModifiers::default(),
        repeat: false,
    });
    let SessionEffect::Submit(submission) = submitted.effect else {
        panic!("Enter on the focused submit button must submit: {submitted:?}");
    };
    assert_eq!(submission.action, "result.html");
    assert_eq!(submission.method, SessionFormMethod::Get);
    assert_eq!(
        submission.fields,
        [("note".to_owned(), "cedar and ash".to_owned())]
    );
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_fetches_remote_image_resources_through_the_host() {
    let image = image::RgbaImage::from_pixel(2, 3, image::Rgba([0, 0, 255, 255]));
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode test PNG");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let engine = LiverySessionEngine::new(ImageFetch {
        bytes,
        requests: requests.clone(),
    });
    let request = SessionSpawnRequest::new("https://example.test/docs/index.html")
        .with_body(
            r#"<html><head><style>
                .card { display: block; width: 80px; height: 40px;
                        background-repeat: no-repeat;
                        background-image: url(hero.png); }
            </style></head><body><div class="card"></div></body></html>"#,
        )
        .with_viewport(320, 240);
    let mut session = engine.spawn(&request).expect("Livery lane spawns");

    let scene = session.frame(320, 240);
    assert!(
        scene
            .ops
            .iter()
            .any(|operation| matches!(operation, netrender::SceneOp::Image(_))),
        "host-fetched image reaches the scene"
    );
    assert_eq!(
        requests.lock().unwrap().as_slice(),
        ["https://example.test/docs/hero.png"]
    );
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_uses_linked_sheet_identity_and_sheet_relative_resources() {
    let image = image::RgbaImage::from_pixel(2, 3, image::Rgba([0, 0, 255, 255]));
    let mut image_bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut image_bytes),
            image::ImageFormat::Png,
        )
        .expect("encode test PNG");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let engine = LiverySessionEngine::new(LinkedResourceFetch {
        image: image_bytes,
        // The retained text system accepts host-provided bytes; rendering
        // the fake fixture font is outside this source-attribution test.
        font: b"not-a-real-font".to_vec(),
        requests: requests.clone(),
    });
    let request = SessionSpawnRequest::new("https://example.test/docs/index.html")
        .with_body(
            r#"<html><head><link rel="stylesheet" href="styles/site.css" media="screen"></head>
<body><div class="card">linked resource</div></body></html>"#,
        )
        .with_viewport(320, 240);
    let mut session = engine.spawn(&request).expect("linked Livery route spawns");
    let _ = session.frame(320, 240);
    let concrete = session
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("session keeps its resource ledger");
    assert_eq!(concrete.resources.stylesheets.len(), 1);
    let sheet = &concrete.resources.stylesheets[0];
    assert_eq!(sheet.media.as_deref(), Some("screen"));
    assert_eq!(
        sheet.source_url.as_deref(),
        Some("https://example.test/docs/styles/site.css")
    );
    assert!(concrete.resources.resources.iter().any(|resource| {
        resource.kind == ResourceKind::Image
            && resource.resolved_url == "https://example.test/docs/styles/images/hero.png"
    }));
    assert!(concrete.resources.resources.iter().any(|resource| {
        resource.kind == ResourceKind::Font
            && resource.resolved_url == "https://example.test/docs/fonts/text.woff2"
    }));
    assert!(concrete.resource_diagnostics().is_empty());
    assert_eq!(
        requests.lock().unwrap().as_slice(),
        [
            "https://example.test/docs/styles/site.css",
            "https://example.test/docs/styles/images/hero.png",
            "https://example.test/docs/fonts/text.woff2",
        ]
    );
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_applies_imports_before_their_redirected_parent_sheet() {
    let engine = LiverySessionEngine::new(ImportedSheetFetch);
    let request = SessionSpawnRequest::new("https://example.test/docs/index.html")
        .with_body(
            r#"<html><head><link rel="stylesheet" href="styles/root.css"></head>
<body><p class="card">cascade</p></body></html>"#,
        )
        .with_viewport(320, 240);
    let mut session = engine
        .spawn(&request)
        .expect("imported Livery route spawns");
    let scene = session.frame(320, 240);
    assert!(
        scene.ops.iter().any(|operation| {
            matches!(operation, netrender::SceneOp::GlyphRun(run) if run.color == [1.0, 0.0, 0.0, 1.0])
        }),
        "the parent sheet follows the imported sheet in the author cascade"
    );
    let concrete = session
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("session keeps its resource ledger");
    assert!(concrete.resource_diagnostics().is_empty());
    assert_eq!(concrete.resources.stylesheets.len(), 2);
    assert_eq!(
        concrete.resources.stylesheets[0].owner,
        StylesheetOwner::Imported
    );
    assert_eq!(
        concrete.resources.stylesheets[1].source_url.as_deref(),
        Some("https://cdn.example.test/styles/root.css")
    );
    assert_eq!(
        concrete.resources.stylesheets[1].requested_url.as_deref(),
        Some("https://example.test/docs/styles/root.css")
    );
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_applies_host_selected_import_limits() {
    let engine =
        LiverySessionEngine::new(ImportedSheetFetch).with_resource_limits(ResourceLimits {
            max_import_depth: 0,
            max_stylesheet_bytes: 2 * 1024 * 1024,
        });
    let request = SessionSpawnRequest::new("https://example.test/docs/index.html").with_body(
        r#"<html><head><link rel="stylesheet" href="styles/root.css"></head>
<body><p class="card">bounded</p></body></html>"#,
    );
    let session = engine.spawn(&request).expect("bounded Livery route spawns");
    let concrete = session
        .as_any_ref()
        .downcast_ref::<LiveryDocumentSession>()
        .expect("session keeps its resource ledger");
    assert_eq!(concrete.resource_set().stylesheets.len(), 1);
    assert!(matches!(
        concrete.resource_diagnostics(),
        [genet_document_resources::ResourceDiagnostic::ImportRuleDepthLimit { max_depth: 0, .. }]
    ));
}

#[cfg(feature = "livery")]
#[test]
fn livery_session_resolves_links_against_a_redirected_document_identity() {
    let engine = LiverySessionEngine::new(RedirectedDocumentFetch);
    let request = SessionSpawnRequest::new("https://example.test/start").with_viewport(320, 240);
    let mut session = engine.spawn(&request).expect("redirected document spawns");
    let scene = session.frame(320, 240);
    assert!(
        scene.ops.iter().any(|operation| {
            matches!(operation, netrender::SceneOp::GlyphRun(run) if run.color == [0.0, 128.0 / 255.0, 0.0, 1.0])
        }),
        "the final document identity supplies the linked stylesheet base"
    );
    let clip = session.clip().expect("redirected document supplies a clip");
    assert_eq!(
        clip.artifacts[0].canonical_uri,
        "https://cdn.example.test/final/index.html"
    );
    assert_eq!(clip.artifacts[0].media_type, "text/html");
    let concrete = session
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("session keeps its resource ledger");
    assert_eq!(
        concrete.resources.document_url.as_deref(),
        Some("https://cdn.example.test/final/index.html")
    );
    assert_eq!(
        concrete.resources.stylesheets[0].source_url.as_deref(),
        Some("https://cdn.example.test/final/site.css")
    );
}

#[cfg(feature = "livery")]
#[test]
fn prepared_livery_resources_cannot_cross_navigation_boundaries() {
    let engine = LiverySessionEngine::new(NoFetch);
    let first = SessionSpawnRequest::new("https://example.test/first");
    let preparation = LiveryResourcePreparation::new(
        &first,
        ResourceResponse::new(
            "https://cdn.example.test/final/first.html",
            b"<p>first</p>".to_vec(),
        )
        .with_content_type("text/html"),
        ResourceLimits::default(),
    );
    let second = SessionSpawnRequest::new("https://example.test/second");
    let error = match engine.spawn_prepared(&second, preparation) {
        Err(error) => error,
        Ok(_) => panic!("a preparation is pinned to its navigation request"),
    };
    assert!(error.to_string().contains("different navigation"));
}

#[cfg(feature = "livery")]
#[test]
fn prepared_livery_session_uses_redirect_final_identity_and_requested_fragment() {
    let engine = LiverySessionEngine::new(NoFetch);
    let request = SessionSpawnRequest::new("https://example.test/start#proof");
    let preparation = LiveryResourcePreparation::new(
        &request,
        ResourceResponse::new(
            "https://cdn.example.test/final/index.html",
            b"<h1 id=proof>proof</h1>".to_vec(),
        )
        .with_content_type("text/html"),
        ResourceLimits::default(),
    );
    let mut session = engine
        .spawn_prepared(&request, preparation)
        .expect("prepared session spawns");
    let clip = session.clip().expect("prepared session supplies a clip");
    assert_eq!(
        clip.artifacts[0].canonical_uri,
        "https://cdn.example.test/final/index.html"
    );
    let concrete = session
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("prepared session retains the Livery identity");
    assert_eq!(
        concrete.address(),
        "https://cdn.example.test/final/index.html#proof"
    );
}

// ---------------------------------------------------------------------------
// Child browsing contexts on the Livery route (the iframes plan, phases 1-3)
// ---------------------------------------------------------------------------

/// A fetcher over a small in-memory site, so a frame's `src` travels the same
/// route as the parent's stylesheets and images.
#[cfg(feature = "livery")]
#[derive(Clone)]
struct SiteFetch;

#[cfg(feature = "livery")]
impl ResourceFetcher for SiteFetch {
    fn fetch(&self, url: &str) -> Option<Vec<u8>> {
        match url {
            "https://example.test/child.html" => Some(
                br#"<body style="margin:0"><div id="mark" style="width:40px;height:40px;background:rgb(0,128,0)"></div></body>"#
                    .to_vec(),
            ),
            "https://example.test/outer.html" => Some(
                br#"<body style="margin:0"><iframe src="child.html" style="width:120px;height:80px;border:0;padding:0;display:block"></iframe></body>"#
                    .to_vec(),
            ),
            "https://example.test/self.html" => {
                Some(br#"<body><iframe src="self.html"></iframe></body>"#.to_vec())
            },
            "https://example.test/child.css" => Some(b"div { background: rgb(0,0,255) }".to_vec()),
            _ => None,
        }
    }
}

#[cfg(feature = "livery")]
fn spawn_site(address: &str, body: Option<&str>) -> Box<dyn DocumentSession<Scene>> {
    let engine = LiverySessionEngine::new(SiteFetch);
    let mut request = SessionSpawnRequest::new(address).with_viewport(400, 300);
    if let Some(body) = body {
        request = request.with_body(body);
    }
    engine.spawn(&request).expect("livery session spawns")
}

#[cfg(feature = "livery")]
fn livery(session: &mut Box<dyn DocumentSession<Scene>>) -> &mut LiveryDocumentSession {
    session
        .as_any()
        .downcast_mut::<LiveryDocumentSession>()
        .expect("a Livery session")
}

/// A frame's `src` is fetched through the parent's own route, becomes a child
/// browsing context in the tree, and reports `load`.
#[cfg(feature = "livery")]
#[test]
fn a_frame_src_becomes_a_child_context_through_the_parents_resource_route() {
    let mut session = spawn_site(
        "https://example.test/index.html",
        Some(
            r#"<body style="margin:0"><iframe src="child.html" style="width:120px;height:80px;border:0"></iframe></body>"#,
        ),
    );
    let session = livery(&mut session);
    let reports = session.frame_load_reports();
    assert_eq!(reports.len(), 1, "one child context: {reports:?}");
    assert_eq!(
        reports[0].outcome,
        crate::engines::frames::FrameOutcome::Load
    );
    assert_eq!(reports[0].url, "https://example.test/child.html");

    let tree = session.browsing_contexts();
    let top = tree.top();
    assert_eq!(tree.frames_of(top).len(), 1);
    let child = tree.frames_of(top)[0];
    assert_eq!(tree.parent_of(child), Some(top));
    assert_eq!(tree.top_of(child), Some(top));
    // Same-origin, so the scripted route would get full access.
    assert!(tree.is_same_origin(top, child));
    assert_eq!(tree.get(child).unwrap().history().len(), 1);
}

/// The child's scene is composited into the parent's replaced box: its content
/// appears in the parent's frame, offset by the frame's position.
#[cfg(feature = "livery")]
#[test]
fn the_childs_scene_is_composited_into_the_parents_replaced_box() {
    let mut session = spawn_site(
        "https://example.test/index.html",
        Some(
            r#"<body style="margin:0"><div style="height:30px"></div><iframe src="child.html" style="width:120px;height:80px;border:0;padding:0;display:block"></iframe></body>"#,
        ),
    );
    let session = livery(&mut session);
    let scene = session.frame(400, 300);
    let green = scene
        .ops
        .iter()
        .filter_map(|op| scene_rect_color_and_bounds(&scene, op))
        .find(|(color, _)| *color == [0.0, 128.0 / 255.0, 0.0, 1.0]);
    let (_, bounds) = green.expect("the child's green square reaches the parent's scene");
    // The child's div is at its own (0, 0); the frame starts 30px down.
    assert!(
        (bounds[1] - 30.0).abs() < 1.5,
        "the child is translated to the frame's content origin: {bounds:?}"
    );
    assert!(bounds[0].abs() < 1.5, "and to its x: {bounds:?}");
}

/// A child larger than its frame is cut off at the frame's edge.
#[cfg(feature = "livery")]
#[test]
fn the_child_is_clipped_to_the_frames_content_box() {
    let mut session = spawn_site(
        "https://example.test/index.html",
        Some(
            r#"<body style="margin:0"><iframe src="child.html" style="width:20px;height:20px;border:0;padding:0;display:block"></iframe></body>"#,
        ),
    );
    let session = livery(&mut session);
    let scene = session.frame(400, 300);
    assert!(
        scene
            .ops
            .iter()
            .filter_map(|op| scene_rect_color_and_bounds(&scene, op))
            .any(|(color, _)| color == [0.0, 128.0 / 255.0, 0.0, 1.0]),
        "the child still paints"
    );
    assert!(
        scene_has_layer_clip_of_size(&scene, 20.0, 20.0),
        "a layer clipped to the frame's 20x20 content box wraps the child"
    );
}

/// A `srcdoc` frame needs no fetch and resolves its own relative URLs against
/// the *parent's* base URL, not `about:srcdoc`.
#[cfg(feature = "livery")]
#[test]
fn a_srcdoc_frame_loads_without_a_fetch_and_keeps_the_parents_base_url() {
    let mut session = spawn_site(
        "https://example.test/index.html",
        Some(
            r#"<iframe srcdoc="<link rel=stylesheet href=child.css><div style='width:20px;height:20px'></div>"></iframe>"#,
        ),
    );
    let session = livery(&mut session);
    let reports = session.frame_load_reports();
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].outcome,
        crate::engines::frames::FrameOutcome::Load
    );
    assert_eq!(reports[0].url, "about:srcdoc");
    // `child.css` resolved against the parent, so the child's div is blue.
    let scene = session.frame(400, 300);
    assert!(
        scene
            .ops
            .iter()
            .filter_map(|op| scene_rect_color_and_bounds(&scene, op))
            .any(|(color, _)| color == [0.0, 0.0, 1.0, 1.0]),
        "the srcdoc child's parent-relative stylesheet applied"
    );
}

/// A `src` that cannot be fetched reports `error` and leaves the box empty
/// rather than failing the parent's frame.
#[cfg(feature = "livery")]
#[test]
fn an_unfetchable_frame_reports_error_and_leaves_an_empty_box() {
    let mut session = spawn_site(
        "https://example.test/index.html",
        Some(r#"<iframe src="missing.html"></iframe>"#),
    );
    let session = livery(&mut session);
    let reports = session.frame_load_reports();
    assert_eq!(
        reports[0].outcome,
        crate::engines::frames::FrameOutcome::Error
    );
    let scene = session.frame(400, 300);
    assert!(!scene.ops.is_empty(), "the parent still renders");
}

/// An empty `src` leaves the initial `about:blank`, which is a `load`.
#[cfg(feature = "livery")]
#[test]
fn an_about_blank_frame_is_a_context_that_loaded() {
    let mut session = spawn_site("https://example.test/index.html", Some("<iframe></iframe>"));
    let session = livery(&mut session);
    let reports = session.frame_load_reports();
    assert_eq!(
        reports[0].outcome,
        crate::engines::frames::FrameOutcome::Load
    );
    assert_eq!(reports[0].url, "about:blank");
    // The initial about:blank inherits the container's origin.
    let tree = session.browsing_contexts();
    assert!(tree.is_same_origin(tree.top(), reports[0].context));
}

/// A nested frame is a grandchild context, and its scene reaches the top
/// document through two composites.
#[cfg(feature = "livery")]
#[test]
fn a_nested_frame_is_a_grandchild_context_composited_through_both_levels() {
    let mut session = spawn_site(
        "https://example.test/index.html",
        Some(
            r#"<body style="margin:0"><iframe src="outer.html" style="width:200px;height:150px;border:0;padding:0;display:block"></iframe></body>"#,
        ),
    );
    let session = livery(&mut session);
    let reports = session.frame_load_reports();
    assert_eq!(reports.len(), 2, "parent then child: {reports:?}");
    assert_eq!(reports[0].url, "https://example.test/outer.html");
    assert_eq!(reports[1].url, "https://example.test/child.html");
    let tree = session.browsing_contexts();
    assert_eq!(tree.top_of(reports[1].context), Some(tree.top()));
    assert_eq!(tree.parent_of(reports[1].context), Some(reports[0].context));

    let scene = session.frame(400, 300);
    assert!(
        scene
            .ops
            .iter()
            .filter_map(|op| scene_rect_color_and_bounds(&scene, op))
            .any(|(color, _)| color == [0.0, 128.0 / 255.0, 0.0, 1.0]),
        "the grandchild's content reaches the top document"
    );
}

/// A frame whose `src` is already open in an ancestor loads `about:blank`
/// instead of recursing.
#[cfg(feature = "livery")]
#[test]
fn a_self_referencing_frame_stops_rather_than_recursing() {
    let mut session = spawn_site("https://example.test/self.html", None);
    let session = livery(&mut session);
    let reports = session.frame_load_reports();
    assert_eq!(reports.len(), 1, "the recursion stops at one: {reports:?}");
    assert_eq!(
        reports[0].outcome,
        crate::engines::frames::FrameOutcome::Load
    );
    let _ = session.frame(400, 300);
}

/// Sandbox flags are stored on the context, and a sandbox without
/// `allow-same-origin` takes the child's origin away.
#[cfg(feature = "livery")]
#[test]
fn a_sandboxed_frame_loses_its_origin_and_its_script_permission() {
    let mut session = spawn_site(
        "https://example.test/index.html",
        Some(r#"<iframe src="child.html" sandbox="allow-forms"></iframe>"#),
    );
    let session = livery(&mut session);
    let reports = session.frame_load_reports();
    let tree = session.browsing_contexts();
    assert!(!tree.is_same_origin(tree.top(), reports[0].context));
    assert!(
        tree.get(reports[0].context)
            .unwrap()
            .document()
            .origin
            .is_opaque()
    );
    assert!(!crate::frame_policy::scripts_allowed(reports[0].sandbox));
    assert!(crate::frame_policy::forms_allowed(reports[0].sandbox));
}

/// A `loading="lazy"` frame is a real context that has not loaded, so it
/// fires neither event.
#[cfg(feature = "livery")]
#[test]
fn a_lazy_frame_is_a_context_that_has_not_loaded() {
    let mut session = spawn_site(
        "https://example.test/index.html",
        Some(r#"<iframe src="child.html" loading="lazy"></iframe>"#),
    );
    let session = livery(&mut session);
    let reports = session.frame_load_reports();
    assert_eq!(
        reports[0].outcome,
        crate::engines::frames::FrameOutcome::Deferred
    );
    let tree = session.browsing_contexts();
    assert_eq!(
        tree.frames_of(tree.top()).len(),
        1,
        "the context exists even though the load is deferred"
    );
}

/// Hit testing descends into a child context and answers with the node *and*
/// the context that node belongs to.
#[cfg(feature = "livery")]
#[test]
fn hit_testing_descends_into_the_child_context() {
    let mut session = spawn_site(
        "https://example.test/index.html",
        Some(
            r#"<body style="margin:0"><iframe src="child.html" style="width:120px;height:80px;border:0;padding:0;display:block"></iframe></body>"#,
        ),
    );
    let session = livery(&mut session);
    let _ = session.frame(400, 300);
    let top = session.browsing_contexts().top();
    let child_context = session.browsing_contexts().frames_of(top)[0];
    let (context, _node) = session
        .hit_test_frames(10.0, 10.0)
        .expect("a point inside the frame hits something");
    assert_eq!(
        context, child_context,
        "the point lands in the child browsing context, not the parent"
    );
    // A point outside the frame stays in the top-level context.
    if let Some((outer, _)) = session.hit_test_frames(300.0, 200.0) {
        assert_eq!(outer, top);
    }
}

#[cfg(feature = "livery")]
fn scene_rect_color_and_bounds(
    scene: &Scene,
    op: &netrender::SceneOp,
) -> Option<([f32; 4], [f32; 4])> {
    let netrender::SceneOp::Rect(rect) = op else {
        return None;
    };
    let (tx, ty) = scene_translation(scene, rect.transform_id);
    Some((
        rect.color,
        [rect.x0 + tx, rect.y0 + ty, rect.x1 + tx, rect.y1 + ty],
    ))
}

/// Whether some layer in the scene is clipped to a `width` x `height` rect.
///
/// The paint-list translator lowers `PushClip` to a netrender **layer** with a
/// `SceneClip::Rect`, not to a per-primitive `clip_rect`, so the frame's clip
/// is proved by finding that layer.
#[cfg(feature = "livery")]
fn scene_has_layer_clip_of_size(scene: &Scene, width: f32, height: f32) -> bool {
    scene.ops.iter().any(|op| match op {
        netrender::SceneOp::PushLayer(layer) => match &layer.clip {
            netrender::SceneClip::Rect { rect, .. } => {
                (rect[2] - rect[0] - width).abs() < 1.5 && (rect[3] - rect[1] - height).abs() < 1.5
            },
            _ => false,
        },
        _ => false,
    })
}

#[cfg(feature = "livery")]
fn scene_translation(scene: &Scene, transform_id: u32) -> (f32, f32) {
    scene
        .transforms
        .get(transform_id as usize)
        .map_or((0.0, 0.0), |transform| (transform.m[12], transform.m[13]))
}
