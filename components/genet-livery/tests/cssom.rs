// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Harvest H3: CSSOM-shaped mutation and getComputedStyle serialization
//! against the retained Livery style plane, the bounded corpus the
//! genet-scripted bridge will drive.

use genet_livery::{LiveryDocument, StyleSet};
use genet_static_dom::StaticDocument;
use layout_dom_api::LayoutDom;
use livery::media::Device;
use livery::stylesheet::RuleMutationError;

fn retained(
    author: &str,
) -> (
    LiveryDocument<StaticDocument>,
    <StaticDocument as LayoutDom>::NodeId,
) {
    let document =
        StaticDocument::parse(r#"<html><body><div class="card">card</div></body></html>"#);
    let styles = StyleSet::cambium(&[author]);
    let card = document
        .first_with_class(document.document(), "card")
        .expect("card node");
    (
        LiveryDocument::new(document, styles, Device::screen(200.0, 100.0)),
        card,
    )
}

#[test]
fn computed_style_serializes_longhands_and_custom_properties() {
    let (mut retained, card) = retained(
        ".card { --accent: #ff0000; color: var(--accent); width: 50px; margin-top: 1em; }",
    );
    retained.frame(200, 100).unwrap();

    assert_eq!(
        retained.computed_style(card, "color").as_deref(),
        Some("rgb(255, 0, 0)")
    );
    assert_eq!(
        retained.computed_style(card, "--accent").as_deref(),
        Some("#ff0000")
    );
    assert_eq!(
        retained.computed_style(card, "width").as_deref(),
        Some("50px")
    );
    assert_eq!(retained.computed_style(card, "--missing"), None);
    assert_eq!(retained.computed_style(card, "not-a-property"), None);
}

#[test]
fn computed_style_resolves_without_a_prior_frame() {
    let (retained, card) = retained(".card { color: #00ff00; }");
    assert_eq!(
        retained.computed_style(card, "color").as_deref(),
        Some("rgb(0, 255, 0)")
    );
}

#[test]
fn computed_style_serializes_border_lengths_and_box_shorthands() {
    let (retained, card) = retained(
        ".card { color: rgb(1, 2, 3); border-width: 0; border-style: inset; border-color: currentcolor; }",
    );
    assert_eq!(
        retained.computed_style(card, "border-top-width").as_deref(),
        Some("0px")
    );
    assert_eq!(
        retained.computed_style(card, "border-style").as_deref(),
        Some("inset")
    );
    assert_eq!(
        retained.computed_style(card, "border-color").as_deref(),
        Some("rgb(1, 2, 3)")
    );
}

#[test]
fn computed_style_serializes_flex_shorthands_from_longhands() {
    let (style_set, card) = retained(".card { flex: 2 3 10px; flex-flow: column wrap; }");
    assert_eq!(
        style_set.computed_style(card, "flex").as_deref(),
        Some("2 3 10px")
    );
    assert_eq!(
        style_set.computed_style(card, "flex-flow").as_deref(),
        Some("column wrap")
    );

    let (style_set, card) = retained(".card { flex: 1; flex-flow: row wrap; }");
    assert_eq!(
        style_set.computed_style(card, "flex").as_deref(),
        Some("1 1 0%")
    );
    assert_eq!(
        style_set.computed_style(card, "flex-flow").as_deref(),
        Some("wrap")
    );

    let (style_set, card) = retained(".card { flex: 0 0 0; }");
    assert_eq!(
        style_set.computed_style(card, "flex").as_deref(),
        Some("0 0 0px")
    );

    let (style_set, card) = retained(".card { font-size: 40px; flex-basis: content; }");
    assert_eq!(
        style_set.computed_style(card, "flex-basis").as_deref(),
        Some("content")
    );

    let (style_set, card) = retained(".card { flex-basis: fit-content; }");
    assert_eq!(
        style_set.computed_style(card, "flex-basis").as_deref(),
        Some("fit-content")
    );

    let (style_set, card) = retained(".card { font-size: 40px; flex-basis: calc(10px + 0.5em); }");
    assert_eq!(
        style_set.computed_style(card, "flex-basis").as_deref(),
        Some("30px")
    );

    let (style_set, card) = retained(".card { font-size: 40px; flex-basis: calc(10px - 0.5em); }");
    assert_eq!(
        style_set.computed_style(card, "flex-basis").as_deref(),
        Some("0px")
    );

    let (style_set, card) = retained(".card { flex-basis: calc(10% + 0px); }");
    assert_eq!(
        style_set.computed_style(card, "flex-basis").as_deref(),
        Some("10%")
    );

    let (style_set, card) = retained(".card { flex-basis: calc(10%); }");
    assert_eq!(
        style_set.computed_style(card, "flex-basis").as_deref(),
        Some("10%")
    );
}

#[test]
fn computed_style_serializes_k6a_columns_and_fragmentation_values() {
    let (style_set, card) = retained(
        ".card { columns: auto 20px; column-fill: balance-all; break-before: column; break-inside: avoid-column; orphans: 3; widows: 4; }",
    );
    assert_eq!(
        style_set.computed_style(card, "column-count").as_deref(),
        Some("auto")
    );
    assert_eq!(
        style_set.computed_style(card, "column-fill").as_deref(),
        Some("balance-all")
    );
    assert_eq!(
        style_set.computed_style(card, "break-before").as_deref(),
        Some("column")
    );
    assert_eq!(
        style_set.computed_style(card, "break-inside").as_deref(),
        Some("avoid-column")
    );
    assert_eq!(
        style_set.computed_style(card, "orphans").as_deref(),
        Some("3")
    );
    assert_eq!(
        style_set.computed_style(card, "widows").as_deref(),
        Some("4")
    );

    let (relative, card) = retained(".card { font-size: 10px; column-width: 2em; }");
    assert_eq!(
        relative.computed_style(card, "column-width").as_deref(),
        Some("20px")
    );

    let (calc, card) = retained(
        ".card { font-size: 40px; column-width: calc(10px + 0.5em); orphans: calc(1 + 234); widows: calc(1 + 234); }",
    );
    assert_eq!(
        calc.computed_style(card, "column-width").as_deref(),
        Some("30px")
    );
    assert_eq!(calc.computed_style(card, "orphans").as_deref(), Some("235"));
    assert_eq!(calc.computed_style(card, "widows").as_deref(), Some("235"));

    let (clamped, card) = retained(".card { font-size: 40px; column-width: calc(10px - 0.5em); }");
    assert_eq!(
        clamped.computed_style(card, "column-width").as_deref(),
        Some("0px")
    );

    let (zero, card) = retained(".card { column-width: 0; }");
    assert_eq!(
        zero.computed_style(card, "column-width").as_deref(),
        Some("0px")
    );
}

#[test]
fn inherited_column_width_keeps_the_parent_font_basis() {
    let document = StaticDocument::parse(
        "<html><body><div class='parent'><div class='child'>child</div></div></body></html>",
    );
    let styles = StyleSet::cambium(&[".parent { font-size: 40px; column-width: 2em; } \
         .child { font-size: 10px; column-width: inherit; }"]);
    let child = document
        .first_with_class(document.document(), "child")
        .expect("child node");
    let parent = document
        .first_with_class(document.document(), "parent")
        .expect("parent node");
    let retained = LiveryDocument::new(document, styles, Device::screen(200.0, 100.0));
    assert_eq!(
        retained.computed_style(parent, "column-width").as_deref(),
        Some("80px")
    );
    assert_eq!(
        retained.computed_style(child, "column-width").as_deref(),
        Some("80px")
    );
}

#[test]
fn computed_transform_serializes_as_a_resolved_2d_matrix() {
    let (retained, card) =
        retained(".card { font-size: 10px; transform: translate(2em, 4px) skewX(45deg); }");
    assert_eq!(
        retained.computed_style(card, "transform").as_deref(),
        Some("matrix(1, 0, 1, 1, 20, 4)")
    );
}

#[test]
fn computed_transform_resolves_percentages_against_a_definite_box() {
    let (retained, card) =
        retained(".card { width: 100px; height: 50px; transform: translate(25%, 50%); }");
    assert_eq!(
        retained.computed_style(card, "transform").as_deref(),
        Some("matrix(1, 0, 0, 1, 25, 25)")
    );
}

#[test]
fn insert_and_delete_author_rules_restyle_the_retained_document() {
    let (mut retained, card) = retained(".card { color: #111111; }");
    retained.frame(200, 100).unwrap();
    assert_eq!(
        retained.computed_style(card, "color").as_deref(),
        Some("rgb(17, 17, 17)")
    );

    // A later same-specificity rule wins the cascade tie.
    let index = retained
        .insert_author_rule(0, ".card { color: #222222; }", 1)
        .expect("insert");
    assert_eq!(index, 1);
    retained.frame(200, 100).unwrap();
    assert_eq!(
        retained.computed_style(card, "color").as_deref(),
        Some("rgb(34, 34, 34)")
    );

    retained.delete_author_rule(0, 1).expect("delete");
    retained.frame(200, 100).unwrap();
    assert_eq!(
        retained.computed_style(card, "color").as_deref(),
        Some("rgb(17, 17, 17)")
    );
}

#[test]
fn retained_k6a_mutation_matches_a_fresh_final_document() {
    let (mut document, card) = retained(".card { columns: 2; }");
    document.frame(200, 100).expect("initial K6a frame");
    document
        .insert_author_rule(0, ".card { columns: 20px; }", 1)
        .expect("insert K6a rule");
    document.frame(200, 100).expect("mutated K6a frame");

    let (mut fresh, fresh_card) = retained(".card { columns: 20px; }");
    fresh.frame(200, 100).expect("fresh K6a frame");
    for property in ["column-width", "column-count"] {
        assert_eq!(
            document.computed_style(card, property),
            fresh.computed_style(fresh_card, property),
            "{property} differs after retained mutation"
        );
    }
}

#[test]
fn inserted_media_rules_respect_the_device() {
    let (mut retained, card) = retained(".card { color: #111111; }");
    retained
        .insert_author_rule(
            0,
            "@media (min-width: 500px) { .card { color: #333333; } }",
            1,
        )
        .expect("insert non-matching media");
    retained.frame(200, 100).unwrap();
    assert_eq!(
        retained.computed_style(card, "color").as_deref(),
        Some("rgb(17, 17, 17)")
    );

    retained
        .insert_author_rule(
            0,
            "@media (min-width: 100px) { .card { color: #444444; } }",
            2,
        )
        .expect("insert matching media");
    retained.frame(200, 100).unwrap();
    assert_eq!(
        retained.computed_style(card, "color").as_deref(),
        Some("rgb(68, 68, 68)")
    );
}

#[test]
fn rule_mutation_errors_surface_and_leave_the_document_intact() {
    let (mut retained, card) = retained(".card { color: #111111; }");
    retained.frame(200, 100).unwrap();

    assert_eq!(
        retained.insert_author_rule(3, ".card { color: red; }", 0),
        Err(RuleMutationError::IndexSize)
    );
    assert_eq!(
        retained.insert_author_rule(0, ".card { color: red; }", 9),
        Err(RuleMutationError::IndexSize)
    );
    assert!(matches!(
        retained.insert_author_rule(0, "not a rule", 0),
        Err(RuleMutationError::Syntax(_))
    ));
    assert_eq!(
        retained.computed_style(card, "color").as_deref(),
        Some("rgb(17, 17, 17)")
    );
}
