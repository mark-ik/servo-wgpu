// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use genet_livery::{
    Device, InteractionStates, LiveryDocument, StyleSet, emit_paint_list, layout, resolve_styles,
};
use genet_static_dom::StaticDocument;
use paint_list_api::{PaintCmd, PaintList};

fn render(html: &str, css: &str, width: u32) -> genet_livery::LiveryPaintList {
    let document = StaticDocument::parse(html);
    let styles = resolve_styles(
        &document,
        &StyleSet::cambium(&[css]),
        &Device::screen(width as f32, 240.0),
        &InteractionStates::default(),
    );
    let fragments = layout(&document, &styles, width as f32, 240.0).expect("layout");
    emit_paint_list(
        &document,
        &styles,
        &fragments,
        paint_list_api::DeviceIntSize::new(width as i32, 240),
        1,
    )
}

fn render_retained(
    html: &str,
    css: &str,
    width: u32,
) -> (genet_livery::LiveryPaintList, genet_livery::LiveryPaintList) {
    let mut document = LiveryDocument::new(
        StaticDocument::parse(html),
        StyleSet::cambium(&[css]),
        Device::screen(width as f32, 240.0),
    );
    let first = document.frame(width, 240).expect("first frame");
    let cached = document.frame(width, 240).expect("cached frame");
    (first, cached)
}

fn glyph_signature(list: &genet_livery::LiveryPaintList) -> Vec<String> {
    list.commands()
        .iter()
        .filter_map(|command| match command {
            PaintCmd::DrawText(run) => Some(run),
            _ => None,
        })
        .flat_map(|run| {
            run.glyphs.iter().map(move |glyph| {
                format!(
                    "font:{:?};size:{:?};color:{:?};glyph:{};point:{:?}",
                    run.font_instance, run.font_size, run.color, glyph.index, glyph.point
                )
            })
        })
        .collect()
}

fn command_signature(list: &genet_livery::LiveryPaintList) -> Vec<String> {
    list.commands()
        .iter()
        .map(|command| format!("{command:?}"))
        .collect()
}

#[test]
fn quoted_inside_string_markers_match_literal_glyphs_and_cached_frames() {
    assert_eq!(
        livery::canonicalize_specified_longhand("list-style-type", r#""\23  ""#),
        Some("\"# \"".to_owned())
    );

    let reference = render(
        "<html><body><div><span>#  </span>item</div></body></html>",
        "* { margin: 0; padding: 0; } span { white-space-collapse: preserve; }",
        320,
    );
    let css = r##"* { margin: 0; padding: 0; }
                  ol, ul { list-style-position: inside; list-style-type: "#  "; }"##;

    for list in ["ol", "ul"] {
        let html = format!("<html><body><{list}><li>item</li></{list}></body></html>");
        assert_eq!(glyph_signature(&render(&html, css, 320)), glyph_signature(&reference));

        let (first, cached) = render_retained(&html, css, 320);
        assert_eq!(glyph_signature(&first), glyph_signature(&reference));
        assert_eq!(command_signature(&cached), command_signature(&first));
    }
}

#[test]
fn decimal_markers_follow_html_ordinals_and_marker_only_block_children() {
    let candidate = r#"<html><body><ol start=" -4legacy"><li>negative four</li><li value="-2legacy">negative two</li><li>negative one<ol><li>inner one</li><li value="4">inner four</li><li>inner five</li></ol></li><li class="hidden" value="500">hidden</li><li>zero</li></ol></body></html>"#;
    let reference = "<html><body><div>-4. negative four</div><div>-2. negative two</div><div>-1. negative one<div>1. inner one</div><div>4. inner four</div><div>5. inner five</div></div><div>0. zero</div></body></html>";
    let candidate_css = "* { margin: 0; padding: 0; line-height: 19px; } ol { list-style-position: inside; } .hidden { display: none; }";
    let reference_css = "* { margin: 0; padding: 0; line-height: 19px; }";

    assert_eq!(
        glyph_signature(&render(candidate, candidate_css, 320)),
        glyph_signature(&render(reference, reference_css, 320))
    );

    let (first, cached) = render_retained(candidate, candidate_css, 320);
    let (reference, _) = render_retained(reference, reference_css, 320);
    assert_eq!(glyph_signature(&first), glyph_signature(&reference));
    assert_eq!(command_signature(&cached), command_signature(&first));
}

#[test]
fn rtl_marker_only_nested_decimal_lists_match_literal_frames() {
    let candidate = r#"<html dir="rtl"><body><ol><li><ol><li><ol><li><span>List item text.</span></li></ol></li></ol></li></ol></body></html>"#;
    let reference = r#"<html><body><div><span class="marker">1. </span><div><span class="marker">1. </span><div>1. <span>List item text.</span></div></div></div></body></html>"#;
    let candidate_css = "ol, li { margin: 0; padding: 0; border: 0; } \
        li, div { color: blue; border: 4px solid silver; \
                  padding: 8px 48px 8px 8px; list-style-position: inside; } \
        span { color: white; }";
    let reference_css = format!(
        "body {{ direction: rtl; }} {candidate_css} \
         span.marker {{ color: blue; white-space-collapse: preserve; }}"
    );

    for width in [320, 247] {
        assert_eq!(
            glyph_signature(&render(candidate, candidate_css, width)),
            glyph_signature(&render(reference, &reference_css, width))
        );

        let (first, cached) = render_retained(candidate, candidate_css, width);
        let (reference, _) = render_retained(reference, &reference_css, width);
        assert_eq!(glyph_signature(&first), glyph_signature(&reference));
        assert_eq!(command_signature(&cached), command_signature(&first));
    }
}
