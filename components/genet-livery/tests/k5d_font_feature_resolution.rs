// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use genet_livery::{Device, LiveryDocument, StyleSet};
use genet_static_dom::StaticDocument;
use layout_dom_api::{LayoutDom, LocalName, Namespace, NodeKind};
use paint_list_api::{ColorF, PaintCmd, PaintList};
#[cfg(windows)]
use parley::FontDiagnosticCandidateStatus;
#[cfg(windows)]
use read_fonts::{FontRef, TableProvider};

fn find(
    dom: &StaticDocument,
    node: <StaticDocument as LayoutDom>::NodeId,
    needle: &str,
) -> Option<<StaticDocument as LayoutDom>::NodeId> {
    if dom.kind(node) == NodeKind::Element
        && dom.attribute(node, &Namespace::from(""), &LocalName::from("id")) == Some(needle)
    {
        return Some(node);
    }
    dom.dom_children(node)
        .find_map(|child| find(dom, child, needle))
}

fn glyph_count(frame: &genet_livery::LiveryPaintList, color: ColorF) -> usize {
    frame
        .commands()
        .iter()
        .filter_map(|command| match command {
            PaintCmd::DrawText(run) if run.color == color => Some(run.glyphs.len()),
            _ => None,
        })
        .sum()
}

#[cfg(windows)]
fn text_run<'a>(
    frame: &'a genet_livery::LiveryPaintList,
    color: ColorF,
) -> &'a paint_list_api::TextRunItem {
    let runs = frame
        .commands()
        .iter()
        .filter_map(|command| match command {
            PaintCmd::DrawText(run) if run.color == color => Some(run),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(runs.len(), 1, "one fixture run for {color:?}");
    runs[0]
}

#[cfg(windows)]
fn segoe_ui_symbol_bytes() -> Vec<u8> {
    let windows = std::env::var_os("WINDIR")
        .or_else(|| std::env::var_os("SystemRoot"))
        .expect("Windows directory is available through WINDIR or SystemRoot");
    std::fs::read(
        std::path::PathBuf::from(windows)
            .join("Fonts")
            .join("seguisym.ttf"),
    )
    .expect("Windows Segoe UI Symbol is the controlled authored secondary face")
}

/// T0 receipt: an all-Common item reaches Parley with Latin as its effective
/// fallback script. Lato is an authored primary known to omit U+25BE, while
/// its Latin control is covered. The Windows-only secondary is loaded as an
/// authored `@font-face`, so the positive route does not rely on system fallback.
#[cfg(windows)]
#[test]
fn common_script_fallback_diagnostic_records_effective_script_and_selected_faces() {
    const PRIMARY: &[u8] = include_bytes!("../../../tests/wpt/tests/fonts/Lato-Medium-Liga.ttf");
    let secondary = segoe_ui_symbol_bytes();
    const TARGET: char = '\u{25be}';
    const CONTROL: char = 'f';
    let primary = FontRef::new(PRIMARY).expect("Lato fixture parses");
    let primary_charmap = primary.cmap().expect("Lato fixture has a charmap");
    assert_eq!(
        primary_charmap
            .map_codepoint(TARGET)
            .map(|glyph| glyph.to_u32())
            .unwrap_or(0),
        0,
        "the controlled primary must omit the Common target"
    );
    assert_ne!(
        primary_charmap
            .map_codepoint(CONTROL)
            .map(|glyph| glyph.to_u32())
            .unwrap_or(0),
        0,
        "the controlled primary must cover the Latin control"
    );
    let secondary_font = FontRef::new(&secondary).expect("secondary fixture parses");
    assert_ne!(
        secondary_font
            .cmap()
            .expect("secondary fixture has a charmap")
            .map_codepoint(TARGET)
            .map(|glyph| glyph.to_u32())
            .unwrap_or(0),
        0,
        "the controlled second authored face must cover the Common target"
    );

    let negative_css = "@font-face { font-family: t0-primary; src: url(/fonts/Lato-Medium-Liga.ttf); } \
                        #target, #control { display: block; font-family: t0-primary; font-size: 32px; } \
                        #target { color: #010101; } #control { color: #020202; }";
    let mut negative_session = LiveryDocument::new(
        StaticDocument::parse(
            "<html><body><span id=target>&#x25be;</span><span id=control>f</span></body></html>",
        ),
        StyleSet::cambium(&[negative_css]),
        Device::screen(320.0, 160.0),
    );
    negative_session.set_font_resource("/fonts/Lato-Medium-Liga.ttf", PRIMARY.to_vec());
    let negative_capture = parley::begin_font_diagnostic_capture();
    let negative_frame = negative_session.frame(320, 160).expect("T0 negative frame");
    let negative_target_glyphs = text_run(
        &negative_frame,
        ColorF::new(1.0 / 255.0, 1.0 / 255.0, 1.0 / 255.0, 1.0),
    )
    .glyphs
    .iter()
    .map(|glyph| glyph.index)
    .collect::<Vec<_>>();
    let negative_events = negative_capture.take();
    let negative_target_event = negative_events
        .iter()
        .find(|event| event.cluster == TARGET.to_string())
        .expect("the primary-missing target reaches the actual query");
    assert_eq!(negative_target_event.fallback_script, *b"Latn");

    let css = "@font-face { font-family: t0-primary; src: url(/fonts/Lato-Medium-Liga.ttf); } \
               @font-face { font-family: t0-secondary; src: url(/fonts/seguisym.ttf); } \
               #target, #control { display: block; font-family: t0-primary, t0-secondary; font-size: 32px; } \
               #target { color: #010101; } #control { color: #020202; }";
    let mut session = LiveryDocument::new(
        StaticDocument::parse(
            "<html><body><span id=target>&#x25be;</span><span id=control>f</span></body></html>",
        ),
        StyleSet::cambium(&[css]),
        Device::screen(320.0, 160.0),
    );
    session.set_font_resource("/fonts/Lato-Medium-Liga.ttf", PRIMARY.to_vec());
    session.set_font_resource("/fonts/seguisym.ttf", secondary.clone());
    let capture = parley::begin_font_diagnostic_capture();
    let frame = session.frame(320, 160).expect("T0 frame");
    let target = text_run(
        &frame,
        ColorF::new(1.0 / 255.0, 1.0 / 255.0, 1.0 / 255.0, 1.0),
    );
    let control = text_run(
        &frame,
        ColorF::new(2.0 / 255.0, 2.0 / 255.0, 2.0 / 255.0, 1.0),
    );
    let target_glyphs = target
        .glyphs
        .iter()
        .map(|glyph| glyph.index)
        .collect::<Vec<_>>();
    let control_glyphs = control
        .glyphs
        .iter()
        .map(|glyph| glyph.index)
        .collect::<Vec<_>>();
    assert!(!target_glyphs.is_empty(), "target reaches the paint path");
    assert!(
        target_glyphs.iter().all(|glyph| *glyph != 0),
        "authored secondary paints the target"
    );
    assert!(
        !control_glyphs.is_empty(),
        "Latin control reaches the paint path"
    );
    assert!(
        control_glyphs.iter().all(|glyph| *glyph != 0),
        "Latin control paints"
    );

    let events = capture.take();
    let target_event = events
        .iter()
        .find(|event| {
            event.cluster == TARGET.to_string()
                && event.candidates.len() >= 2
                && event.candidates[0].status == FontDiagnosticCandidateStatus::Discard
                && event.candidates[1].status == FontDiagnosticCandidateStatus::Complete
        })
        .expect("actual query records primary miss followed by authored fallback coverage");
    assert_eq!(target_event.fallback_script, *b"Latn");
    assert_eq!(target_event.selected_candidate, Some(1));
    let control_event = events
        .iter()
        .find(|event| {
            event.cluster == CONTROL.to_string()
                && event.candidates.first().is_some_and(|candidate| {
                    candidate.status == FontDiagnosticCandidateStatus::Complete
                })
        })
        .expect("actual query records the Latin control");
    assert_eq!(control_event.selected_candidate, Some(0));
    assert_eq!(
        control_event.candidates.len(),
        1,
        "a complete authored primary avoids the text fallback query"
    );

    let target_font = frame
        .fonts()
        .iter()
        .find(|font| font.key == target.font_instance)
        .expect("target font resource");
    let control_font = frame
        .fonts()
        .iter()
        .find(|font| font.key == control.font_instance)
        .expect("control font resource");
    assert_eq!(target_font.data.as_slice(), secondary.as_slice());
    assert_eq!(control_font.data.as_slice(), PRIMARY);
    eprintln!(
        "T0 Common-script diagnostic: source_script=Common negative_fallback_key={:?} \
         negative_candidates={:?} negative_selected={:?} negative_target_glyphs={negative_target_glyphs:?} \
         authored_fallback_key={:?} authored_candidates={:?} authored_selected={:?} target_face=secondary control_face=primary \
         target=U+25BE glyphs={target_glyphs:?} control=U+0066 glyphs={control_glyphs:?}",
        negative_target_event.fallback_script,
        negative_target_event.candidates,
        negative_target_event.selected_candidate,
        target_event.fallback_script,
        target_event.candidates,
        target_event.selected_candidate,
    );
}

/// Windows T1 regression: one item rebuilds fallback state for each
/// primary-missing cluster, so a prior Common symbol cannot supply the next.
#[cfg(windows)]
#[test]
fn common_symbols_use_their_own_system_fallback_queries() {
    const PRIMARY: &[u8] = include_bytes!("../../../tests/wpt/tests/fonts/Lato-Medium-Liga.ttf");
    const FIRST: char = '\u{25be}';
    const SECOND: char = '\u{25b8}';
    let primary = FontRef::new(PRIMARY).expect("Lato fixture parses");
    let charmap = primary.cmap().expect("Lato fixture has a charmap");
    for symbol in [FIRST, SECOND] {
        assert_eq!(
            charmap
                .map_codepoint(symbol)
                .map(|glyph| glyph.to_u32())
                .unwrap_or(0),
            0,
            "controlled primary omits {symbol:?}"
        );
    }
    let css = "@font-face { font-family: t1-primary; src: url(/fonts/Lato-Medium-Liga.ttf); } \
               #sequence { display: block; font-family: t1-primary; font-size: 32px; color: #030303; }";
    let mut session = LiveryDocument::new(
        StaticDocument::parse(
            "<html><body><span id=sequence>&#x25be;&#x25b8;</span></body></html>",
        ),
        StyleSet::cambium(&[css]),
        Device::screen(320.0, 160.0),
    );
    session.set_font_resource("/fonts/Lato-Medium-Liga.ttf", PRIMARY.to_vec());
    let capture = parley::begin_font_diagnostic_capture();
    let frame = session
        .frame(320, 160)
        .expect("T1 alternating-symbol frame");
    let events = capture.take();
    for symbol in [FIRST, SECOND] {
        let event = events
            .iter()
            .find(|event| event.cluster == symbol.to_string())
            .expect("symbol reaches its own fallback query");
        assert_eq!(event.fallback_script, *b"Latn");
        assert_eq!(
            event.candidates[0].status,
            FontDiagnosticCandidateStatus::Discard
        );
        assert!(
            event
                .candidates
                .iter()
                .skip(1)
                .any(|candidate| { candidate.status == FontDiagnosticCandidateStatus::Complete }),
            "system fallback covers {symbol:?}: {event:?}"
        );
    }
    let glyphs = text_run(
        &frame,
        ColorF::new(3.0 / 255.0, 3.0 / 255.0, 3.0 / 255.0, 1.0),
    )
    .glyphs
    .iter()
    .map(|glyph| glyph.index)
    .collect::<Vec<_>>();
    assert_eq!(glyphs.len(), 2, "one item paints both symbols");
    assert!(glyphs.iter().all(|glyph| *glyph != 0), "both symbols paint");
}

#[test]
fn font_feature_precedence_reaches_parley_with_authored_face_aliases() {
    let html = "<html><body>\
        <span class=face-off>fi</span>\
        <span class=variant-on>fi</span>\
        <span class=variant-off>fi</span>\
        <span class=spacing-off>fi</span>\
        <span class=explicit-on>fi</span>\
        <span class=dlig-face>st</span>\
        <span class=dlig-spacing>st</span>\
        <span class=dlig-explicit>st</span>\
        </body></html>";
    let css = "
        @font-face { font-family: face-off; src: url(/fonts/Lato-Medium-Liga.ttf);
                     font-feature-settings: 'liga' off; }
        @font-face { font-family: face-on; src: url(/fonts/Lato-Medium-Liga.ttf);
                     font-feature-settings: 'liga' on; }
        @font-face { font-family: dlig-on; src: url(/fonts/Lato-Medium-Liga.ttf);
                     font-feature-settings: 'dlig' on; }
        span { display: block; font-size: 32px; }
        .face-off { color: #010101; font-family: face-off; }
        .variant-on { color: #020202; font-family: face-off;
                      font-variant-ligatures: common-ligatures; }
        .variant-off { color: #030303; font-family: face-on;
                       font-variant-ligatures: no-common-ligatures; }
        .spacing-off { color: #040404; font-family: face-on; letter-spacing: 0.1em; }
        .explicit-on { color: #050505; font-family: face-on; letter-spacing: 0.1em;
                       font-feature-settings: 'liga' on; }
        .dlig-face { color: #060606; font-family: dlig-on; }
        .dlig-spacing { color: #070707; font-family: dlig-on; letter-spacing: 0.1em; }
        .dlig-explicit { color: #080808; font-family: dlig-on; letter-spacing: 0.1em;
                         font-feature-settings: 'dlig' on; }
    ";
    let mut session = LiveryDocument::new(
        StaticDocument::parse(html),
        StyleSet::cambium(&[css]),
        Device::screen(320.0, 400.0),
    );
    session.set_font_resource(
        "/fonts/Lato-Medium-Liga.ttf",
        include_bytes!("../../../tests/wpt/tests/fonts/Lato-Medium-Liga.ttf").to_vec(),
    );
    let frame = session.frame(320, 400).expect("font feature frame");
    let color = |channel: u8| {
        let channel = f32::from(channel) / 255.0;
        ColorF::new(channel, channel, channel, 1.0)
    };

    assert_eq!(
        glyph_count(&frame, color(1)),
        2,
        "face descriptor disables liga"
    );
    assert_eq!(
        glyph_count(&frame, color(2)),
        1,
        "variant overrides the face"
    );
    assert_eq!(glyph_count(&frame, color(3)), 2, "variant can disable liga");
    assert_eq!(
        glyph_count(&frame, color(4)),
        2,
        "letter spacing disables liga"
    );
    assert_eq!(glyph_count(&frame, color(5)), 1, "explicit liga wins last");
    assert_eq!(
        glyph_count(&frame, color(6)),
        1,
        "face descriptor enables dlig"
    );
    assert_eq!(
        glyph_count(&frame, color(7)),
        2,
        "letter spacing disables dlig"
    );
    assert_eq!(glyph_count(&frame, color(8)), 1, "explicit dlig wins last");
}

#[test]
fn join_controls_and_presentation_ligatures_keep_the_face_line_metrics() {
    let html = "<html><body><div id=plain>fi</div><div id=join>f&zwnj;i</div>\
                <div id=presentation>&#xfb01;</div><div id=spacing-plain class=spaced>st</div>\
                <div id=spacing-join class=spaced>s&zwnj;t</div></body></html>";
    let css = "@font-face { font-family: face; src: url(/fonts/Lato-Medium-Liga.ttf); } \
               div { position: absolute; top: 0; left: 0; \
                     font-family: face; font-size: 32px; } \
               .spaced { letter-spacing: 0.1em; font-feature-settings: 'dlig' off; }";
    let mut session = LiveryDocument::new(
        StaticDocument::parse(html),
        StyleSet::cambium(&[css]),
        Device::screen(320.0, 240.0),
    );
    session.set_font_resource(
        "/fonts/Lato-Medium-Liga.ttf",
        include_bytes!("../../../tests/wpt/tests/fonts/Lato-Medium-Liga.ttf").to_vec(),
    );
    session.frame(320, 240).expect("line metric frame");
    let height = |id| {
        let node = find(session.dom(), session.dom().document(), id).expect("fixture node");
        session.fragment_rect(node).expect("fixture fragment")[3]
    };

    assert_eq!(
        height("join"),
        height("plain"),
        "ZWNJ stays in the selected face"
    );
    assert_eq!(
        height("presentation"),
        height("plain"),
        "the presentation ligature keeps the selected face metrics"
    );
    let width = |id| {
        let node = find(session.dom(), session.dom().document(), id).expect("fixture node");
        session.fragment_rect(node).expect("fixture fragment")[2]
    };
    assert_eq!(
        width("spacing-join"),
        width("spacing-plain"),
        "ZWNJ does not receive letter spacing"
    );
}
