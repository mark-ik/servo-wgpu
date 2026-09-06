// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use livery::PropertyValue;
use livery::cascade::{
    CascadeLayer, MatchedDeclaration, Origin, Specificity, cascade, parse_declaration_block,
};
use livery::values::{
    Color, FlexDirection, FlexWrap, FontFamily, FontFeatureSettings, FontSize, FontStyle,
    FontVariantLigatures, FontWeight, Length, LengthPercentage, LineHeight, Margin, Overflow, Size,
    TextWrapMode, TransitionProperty, VerticalAlign, WhiteSpaceCollapse,
};

fn matched(
    css: &str,
    origin: Origin,
    layer: CascadeLayer,
    specificity: u32,
    source_order: u64,
) -> MatchedDeclaration {
    let mut block = parse_declaration_block(css);
    assert!(block.errors.is_empty(), "{css}: {:?}", block.errors);
    assert_eq!(block.declarations.len(), 1, "{css}");
    MatchedDeclaration {
        declaration: block.declarations.remove(0),
        origin,
        layer,
        specificity: Specificity(specificity),
        source_order,
    }
}

#[test]
fn font_feature_longhands_parse_as_independent_inherited_values() {
    let block = parse_declaration_block(
        "font-variant-ligatures: no-common-ligatures discretionary-ligatures; \
         font-feature-settings: 'liga' on, 'dlig' off",
    );
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert_eq!(block.declarations.len(), 2);
    assert!(matches!(
        &block.declarations[0].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::FontVariantLigatures(
            FontVariantLigatures { .. }
        ))
    ));
    assert!(matches!(
        &block.declarations[1].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::FontFeatureSettings(
            FontFeatureSettings::Settings(settings)
        )) if settings.len() == 2
    ));
}

#[test]
fn declaration_parser_expands_the_lane_shorthands_and_recovers() {
    let block = parse_declaration_block(
        "margin: 1px 2px 3px 4px !important;\
         border: 1px solid #abc;\
         white-space: pre;\
         width: florp;\
         future-property: yes",
    );

    assert_eq!(block.declarations.len(), 18);
    assert_eq!(block.errors.len(), 2);
    assert!(block.declarations[..4].iter().all(|decl| decl.important));
    assert_eq!(block.declarations[0].property.metadata().name, "margin-top");
    assert_eq!(
        block.declarations[3].property.metadata().name,
        "margin-left"
    );
}

#[test]
fn white_space_nowrap_expands_to_collapsing_unwrapped_text() {
    let block = parse_declaration_block("white-space: nowrap");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert_eq!(block.declarations.len(), 2);
    assert!(matches!(
        block.declarations[0].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::WhiteSpaceCollapse(
            WhiteSpaceCollapse::Collapse
        ))
    ));
    assert!(matches!(
        block.declarations[1].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::TextWrapMode(TextWrapMode::Nowrap))
    ));
}

fn expanded_css_values(css: &str) -> Vec<(String, String)> {
    parse_declaration_block(css)
        .declarations
        .into_iter()
        .map(|declaration| {
            let name = declaration.property.metadata().name.to_owned();
            let livery::cascade::DeclaredValue::Value(value) = declaration.value else {
                panic!("{css}: shorthand produced a non-value declaration")
            };
            (name, value.to_css_string())
        })
        .collect()
}

#[test]
fn columns_shorthand_is_order_independent_and_resets_omitted_longhands() {
    for (css, expected) in [
        (
            "columns: 2",
            vec![
                ("column-width".into(), "auto".into()),
                ("column-count".into(), "2".into()),
            ],
        ),
        (
            "columns: 20px",
            vec![
                ("column-width".into(), "20px".into()),
                ("column-count".into(), "auto".into()),
            ],
        ),
        (
            "columns: auto 20px",
            vec![
                ("column-width".into(), "20px".into()),
                ("column-count".into(), "auto".into()),
            ],
        ),
        (
            "columns: 2 20px",
            vec![
                ("column-width".into(), "20px".into()),
                ("column-count".into(), "2".into()),
            ],
        ),
        (
            "columns: auto auto",
            vec![
                ("column-width".into(), "auto".into()),
                ("column-count".into(), "auto".into()),
            ],
        ),
    ] {
        assert_eq!(expanded_css_values(css), expected, "{css}");
    }
    assert!(!parse_declaration_block("columns: -1px").errors.is_empty());
    assert!(
        !parse_declaration_block("columns: 2 20px 3")
            .errors
            .is_empty()
    );
    for keyword in ["initial", "inherit", "unset"] {
        let block = parse_declaration_block(&format!("columns: {keyword}"));
        assert!(block.errors.is_empty(), "{keyword}: {:?}", block.errors);
        assert_eq!(block.declarations.len(), 2);
        assert!(block.declarations.iter().all(|declaration| {
            matches!(
                declaration.value,
                livery::cascade::DeclaredValue::Initial
                    | livery::cascade::DeclaredValue::Inherit
                    | livery::cascade::DeclaredValue::Unset
            )
        }));
    }
}

#[test]
fn flex_shorthand_expands_keyword_and_arity_defaults() {
    for (css, expected) in [
        (
            "flex: none",
            vec![
                ("flex-grow", "0"),
                ("flex-shrink", "0"),
                ("flex-basis", "auto"),
            ],
        ),
        (
            "flex: auto",
            vec![
                ("flex-grow", "1"),
                ("flex-shrink", "1"),
                ("flex-basis", "auto"),
            ],
        ),
        (
            "flex: 2",
            vec![
                ("flex-grow", "2"),
                ("flex-shrink", "1"),
                ("flex-basis", "0%"),
            ],
        ),
        (
            "flex: 2 3",
            vec![
                ("flex-grow", "2"),
                ("flex-shrink", "3"),
                ("flex-basis", "0%"),
            ],
        ),
        (
            "flex: 2 20px",
            vec![
                ("flex-grow", "2"),
                ("flex-shrink", "1"),
                ("flex-basis", "20px"),
            ],
        ),
        (
            "flex: 2 3 20px !important",
            vec![
                ("flex-grow", "2"),
                ("flex-shrink", "3"),
                ("flex-basis", "20px"),
            ],
        ),
        (
            "flex: 20px 2 3",
            vec![
                ("flex-grow", "2"),
                ("flex-shrink", "3"),
                ("flex-basis", "20px"),
            ],
        ),
        (
            "flex: 2 20px 3",
            vec![
                ("flex-grow", "2"),
                ("flex-shrink", "3"),
                ("flex-basis", "20px"),
            ],
        ),
        (
            "flex: 0 1 0",
            vec![
                ("flex-grow", "0"),
                ("flex-shrink", "1"),
                ("flex-basis", "0"),
            ],
        ),
        (
            "flex: content",
            vec![
                ("flex-grow", "1"),
                ("flex-shrink", "1"),
                ("flex-basis", "content"),
            ],
        ),
        (
            "flex: fit-content 2 3",
            vec![
                ("flex-grow", "2"),
                ("flex-shrink", "3"),
                ("flex-basis", "fit-content"),
            ],
        ),
    ] {
        let block = parse_declaration_block(css);
        assert!(block.errors.is_empty(), "{css}: {:?}", block.errors);
        assert_eq!(
            block
                .declarations
                .iter()
                .map(|declaration| declaration.important)
                .collect::<Vec<_>>(),
            vec![css.contains("important"); 3]
        );
        assert_eq!(
            expanded_css_values(css),
            expected
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn flex_shorthand_rejects_unmodeled_or_invalid_values() {
    for css in [
        "flex: 1 1 none",
        "flex: 1 1 -1px",
        "flex: 1 1 -2%",
        "flex: 2 3 4",
        "flex: 0 0 4",
        "flex: 0 1 4",
        "flex: 5px 7%",
    ] {
        let block = parse_declaration_block(css);
        assert!(block.declarations.is_empty(), "{css}");
        assert_eq!(block.errors.len(), 1, "{css}: {:?}", block.errors);
    }
}

#[test]
fn flex_shorthands_expand_css_wide_keywords_to_every_longhand() {
    let block = parse_declaration_block("flex: initial; flex-flow: unset");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert_eq!(block.declarations.len(), 5);
    assert!(
        block.declarations[..3].iter().all(|declaration| matches!(
            declaration.value,
            livery::cascade::DeclaredValue::Initial
        ))
    );
    assert!(
        block.declarations[3..]
            .iter()
            .all(|declaration| matches!(declaration.value, livery::cascade::DeclaredValue::Unset))
    );
}

#[test]
fn flex_flow_expands_direction_and_wrap_in_either_order() {
    for css in [
        "flex-flow: column wrap",
        "flex-flow: wrap-reverse column-reverse",
    ] {
        let block = parse_declaration_block(css);
        assert!(block.errors.is_empty(), "{css}: {:?}", block.errors);
        assert_eq!(block.declarations.len(), 2);
    }
    assert_eq!(
        expanded_css_values("flex-flow: wrap column"),
        vec![
            ("flex-direction".to_owned(), "column".to_owned()),
            ("flex-wrap".to_owned(), "wrap".to_owned()),
        ]
    );
    let duplicate = parse_declaration_block("flex-flow: row column");
    assert!(duplicate.declarations.is_empty());
    assert_eq!(duplicate.errors.len(), 1);

    let empty = parse_declaration_block("flex-flow:");
    assert!(empty.declarations.is_empty());
    assert_eq!(empty.errors.len(), 1);
}

#[test]
fn flex_longhand_values_keep_their_typed_models() {
    let block = parse_declaration_block("flex-direction: column; flex-wrap: wrap; flex-grow: 2");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert!(matches!(
        block.declarations[0].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::FlexDirection(FlexDirection::Column))
    ));
    assert!(matches!(
        block.declarations[1].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::FlexWrap(FlexWrap::Wrap))
    ));
    assert!(matches!(
        block.declarations[2].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::FlexFactor(value))
            if value.value() == 2.0
    ));
}

#[test]
fn directional_border_shorthands_expand_to_their_three_longhands() {
    for (shorthand, expected) in [
        (
            "border-left: 100px solid black",
            [
                "border-left-color",
                "border-left-style",
                "border-left-width",
            ],
        ),
        (
            "border-right: 100px solid black",
            [
                "border-right-color",
                "border-right-style",
                "border-right-width",
            ],
        ),
    ] {
        let block = parse_declaration_block(shorthand);
        assert!(block.errors.is_empty(), "{shorthand}: {:?}", block.errors);
        assert_eq!(
            block
                .declarations
                .iter()
                .map(|declaration| declaration.property.metadata().name)
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn border_side_list_shorthands_expand_to_four_longhands() {
    let block = parse_declaration_block(
        "border-style: solid none solid none; border-width: 1px 2px; border-color: red blue",
    );
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert_eq!(block.declarations.len(), 12);
    assert_eq!(
        block
            .declarations
            .iter()
            .take(4)
            .map(|declaration| declaration.property.metadata().name)
            .collect::<Vec<_>>(),
        vec![
            "border-top-style",
            "border-right-style",
            "border-bottom-style",
            "border-left-style",
        ]
    );
}

#[test]
fn background_color_shorthand_accepts_the_bounded_color_form() {
    let block = parse_declaration_block("background: black");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert_eq!(block.declarations.len(), 8);
    assert_eq!(
        block.declarations[0].property.metadata().name,
        "background-color"
    );
    let livery::cascade::DeclaredValue::Value(PropertyValue::Color(color)) =
        &block.declarations[0].value
    else {
        panic!("background-color did not parse as a color");
    };
    assert_eq!(color.to_srgb8(), Some((0, 0, 0, 255)));
}

#[test]
fn background_shorthand_resets_and_expands_the_retained_image_layer() {
    let block = parse_declaration_block(
        "background: url(support/tile.png) right 10px bottom 20% / 40px auto space no-repeat content-box border-box fixed #123456",
    );
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert_eq!(block.declarations.len(), 8);
    let names = block
        .declarations
        .iter()
        .map(|declaration| declaration.property.metadata().name)
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "background-color",
            "background-image",
            "background-position",
            "background-size",
            "background-repeat",
            "background-attachment",
            "background-origin",
            "background-clip",
        ]
    );
    let values = block
        .declarations
        .iter()
        .map(|declaration| match &declaration.value {
            livery::cascade::DeclaredValue::Value(value) => value.to_css_string(),
            _ => panic!("background shorthand emitted a non-value declaration"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        vec![
            "rgb(18, 52, 86)",
            "url(support/tile.png)",
            "calc(100% - 10px) 80%",
            "40px auto",
            "space no-repeat",
            "fixed",
            "content-box",
            "border-box",
        ]
    );
}

#[test]
fn overflow_shorthand_expands_one_or_two_axis_values() {
    let values = |css: &str| {
        let block = parse_declaration_block(css);
        assert!(block.errors.is_empty(), "{css}: {:?}", block.errors);
        block
            .declarations
            .iter()
            .map(|declaration| {
                let livery::cascade::DeclaredValue::Value(PropertyValue::Overflow(value)) =
                    declaration.value
                else {
                    panic!("{css}: expected an overflow declaration");
                };
                (declaration.property.metadata().name, value)
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(
        values("overflow: hidden"),
        vec![
            ("overflow-x", Overflow::Hidden),
            ("overflow-y", Overflow::Hidden)
        ]
    );
    assert_eq!(
        values("overflow: clip auto"),
        vec![
            ("overflow-x", Overflow::Clip),
            ("overflow-y", Overflow::Auto)
        ]
    );
}

#[test]
fn font_shorthand_expands_size_line_height_and_family() {
    let block = parse_declaration_block("font: italic bold 20px/1.5 Ahem");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert_eq!(block.declarations.len(), 7);
    assert!(matches!(
        &block.declarations[0].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::FontStyle(FontStyle::Italic))
    ));
    assert!(matches!(
        &block.declarations[2].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::FontWeight(FontWeight::Bold))
    ));
    assert!(matches!(
        &block.declarations[3].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::FontSize(FontSize::Value(_)))
    ));
    assert!(matches!(
        &block.declarations[4].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::LineHeight(LineHeight::Number(value)))
            if (*value - 1.5).abs() < f32::EPSILON
    ));
    assert!(matches!(
        &block.declarations[5].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::FontFamily(FontFamily::Named(name)))
            if name.as_ref() == "Ahem"
    ));
    assert!(matches!(
        &block.declarations[1].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::FontVariantLigatures(
            FontVariantLigatures { .. }
        ))
    ));
    assert!(matches!(
        &block.declarations[6].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::FontFeatureSettings(
            FontFeatureSettings::Normal
        ))
    ));
}

#[test]
fn font_family_retains_an_ordered_css_family_list() {
    let block = parse_declaration_block("font-family: \"WOFF Test\", \"WOFF Test CFF Fallback\"");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    let livery::cascade::DeclaredValue::Value(value) = &block.declarations[0].value else {
        panic!("expected a concrete font-family value");
    };
    assert!(matches!(
        value,
        PropertyValue::FontFamily(FontFamily::List(source))
            if source.as_ref() == "\"WOFF Test\", \"WOFF Test CFF Fallback\""
    ));
    assert_eq!(
        value.to_css_string(),
        "\"WOFF Test\", \"WOFF Test CFF Fallback\""
    );

    let malformed = parse_declaration_block("font-family: primary,, fallback");
    assert!(malformed.declarations.is_empty());
    assert_eq!(malformed.errors.len(), 1);
}

#[test]
fn vertical_align_accepts_keyword_and_offset_forms() {
    let keyword = parse_declaration_block("vertical-align: text-bottom");
    assert!(keyword.errors.is_empty(), "{:?}", keyword.errors);
    assert!(matches!(
        &keyword.declarations[0].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::VerticalAlign(
            VerticalAlign::TextBottom
        ))
    ));

    let offset = parse_declaration_block("vertical-align: -2px");
    assert!(offset.errors.is_empty(), "{:?}", offset.errors);
    assert!(matches!(
        &offset.declarations[0].value,
        livery::cascade::DeclaredValue::Value(PropertyValue::VerticalAlign(
            VerticalAlign::Length(LengthPercentage::Length(length))
        )) if length.value == -2.0
    ));
}

#[test]
fn transition_shorthand_expands_to_the_opacity_clock_controls() {
    let block = parse_declaration_block("transition: opacity 100ms");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert_eq!(block.declarations.len(), 2);
    assert_eq!(
        block.declarations[0].property.metadata().name,
        "transition-property"
    );
    assert_eq!(
        block.declarations[1].property.metadata().name,
        "transition-duration"
    );
}

#[test]
fn transition_shorthand_accepts_the_bounded_background_color_lane() {
    let block = parse_declaration_block("transition: background-color 100ms");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert!(matches!(
        block
            .declarations
            .first()
            .map(|declaration| &declaration.value),
        Some(livery::cascade::DeclaredValue::Value(
            PropertyValue::TransitionProperty(TransitionProperty::BackgroundColor)
        ))
    ));
}

#[test]
fn transition_shorthand_accepts_the_bounded_border_top_color_lane() {
    let block = parse_declaration_block("transition: border-top-color 100ms");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert!(matches!(
        block
            .declarations
            .first()
            .map(|declaration| &declaration.value),
        Some(livery::cascade::DeclaredValue::Value(
            PropertyValue::TransitionProperty(TransitionProperty::BorderTopColor)
        ))
    ));
}

#[test]
fn transition_shorthand_accepts_the_bounded_border_bottom_color_lane() {
    let block = parse_declaration_block("transition: border-bottom-color 100ms");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert!(matches!(
        block
            .declarations
            .first()
            .map(|declaration| &declaration.value),
        Some(livery::cascade::DeclaredValue::Value(
            PropertyValue::TransitionProperty(TransitionProperty::BorderBottomColor)
        ))
    ));
}

#[test]
fn transition_shorthand_accepts_the_bounded_background_position_lane() {
    let block = parse_declaration_block("transition: background-position 100ms");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert!(matches!(
        block
            .declarations
            .first()
            .map(|declaration| &declaration.value),
        Some(livery::cascade::DeclaredValue::Value(
            PropertyValue::TransitionProperty(TransitionProperty::BackgroundPosition)
        ))
    ));
}

#[test]
fn transition_shorthand_accepts_the_bounded_box_shadow_lane() {
    let block = parse_declaration_block("transition: box-shadow 100ms");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert!(matches!(
        block
            .declarations
            .first()
            .map(|declaration| &declaration.value),
        Some(livery::cascade::DeclaredValue::Value(
            PropertyValue::TransitionProperty(TransitionProperty::BoxShadow)
        ))
    ));
}

#[test]
fn transition_shorthand_accepts_the_bounded_background_image_lane() {
    let block = parse_declaration_block("transition: background-image 100ms");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert!(matches!(
        block
            .declarations
            .first()
            .map(|declaration| &declaration.value),
        Some(livery::cascade::DeclaredValue::Value(
            PropertyValue::TransitionProperty(TransitionProperty::BackgroundImage)
        ))
    ));
}

#[test]
fn transition_shorthand_accepts_the_bounded_border_style_lane() {
    let block = parse_declaration_block("transition: border-top-style 100ms");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert!(matches!(
        block
            .declarations
            .first()
            .map(|declaration| &declaration.value),
        Some(livery::cascade::DeclaredValue::Value(
            PropertyValue::TransitionProperty(TransitionProperty::BorderTopStyle)
        ))
    ));
}

#[test]
fn transition_shorthand_accepts_the_bounded_background_repeat_lane() {
    let block = parse_declaration_block("transition: background-repeat 100ms");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert!(matches!(
        block
            .declarations
            .first()
            .map(|declaration| &declaration.value),
        Some(livery::cascade::DeclaredValue::Value(
            PropertyValue::TransitionProperty(TransitionProperty::BackgroundRepeat)
        ))
    ));
}

#[test]
fn transition_shorthand_merges_the_bounded_two_property_list() {
    let block = parse_declaration_block("transition: opacity 100ms, background-color 100ms");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert!(matches!(
        block
            .declarations
            .first()
            .map(|declaration| &declaration.value),
        Some(livery::cascade::DeclaredValue::Value(
            PropertyValue::TransitionProperty(TransitionProperty::OpacityAndBackgroundColor)
        ))
    ));
}

#[test]
fn transition_shorthand_merges_the_bounded_three_property_list() {
    let block =
        parse_declaration_block("transition: color 100ms, opacity 100ms, background-color 100ms");
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    assert!(matches!(
        block
            .declarations
            .first()
            .map(|declaration| &declaration.value),
        Some(livery::cascade::DeclaredValue::Value(
            PropertyValue::TransitionProperty(
                TransitionProperty::OpacityAndBackgroundColorAndColor
            )
        ))
    ));
}

#[test]
fn transition_shorthand_preserves_a_side_color_list() {
    let block = parse_declaration_block(
        "transition: opacity 100ms, border-left-color 100ms, border-right-color 100ms",
    );
    assert!(block.errors.is_empty(), "{:?}", block.errors);
    let Some(livery::cascade::DeclaredValue::Value(PropertyValue::TransitionProperty(property))) =
        block
            .declarations
            .first()
            .map(|declaration| &declaration.value)
    else {
        panic!("expected transition-property declaration");
    };
    assert_eq!(
        property.to_string(),
        "opacity, border-left-color, border-right-color"
    );
}

#[test]
fn origin_importance_specificity_and_source_order_follow_the_cascade() {
    let declarations = vec![
        matched(
            "color: #111111",
            Origin::User,
            CascadeLayer::Unlayered,
            100,
            0,
        ),
        matched(
            "color: #222222",
            Origin::Author,
            CascadeLayer::Unlayered,
            1,
            1,
        ),
        matched(
            "font-weight: 400",
            Origin::Author,
            CascadeLayer::Unlayered,
            10,
            2,
        ),
        matched(
            "font-weight: 700",
            Origin::Author,
            CascadeLayer::Unlayered,
            10,
            3,
        ),
        matched(
            "background-color: #333333 !important",
            Origin::Author,
            CascadeLayer::Unlayered,
            1000,
            4,
        ),
        matched(
            "background-color: #444444 !important",
            Origin::User,
            CascadeLayer::Unlayered,
            1,
            5,
        ),
    ];

    let values = cascade(None, declarations);
    assert_eq!(values.color, "#222222".parse::<Color>().unwrap());
    assert_eq!(values.font_weight, FontWeight::Number(700));
    assert_eq!(values.background_color, "#444444".parse::<Color>().unwrap());
}

#[test]
fn author_presentational_hints_have_their_own_normal_origin() {
    let values = cascade(
        None,
        vec![
            matched(
                "color: #111111",
                Origin::UserAgent,
                CascadeLayer::Unlayered,
                100,
                0,
            ),
            matched(
                "color: #222222",
                Origin::User,
                CascadeLayer::Unlayered,
                100,
                1,
            ),
            matched(
                "color: #333333",
                Origin::AuthorPresentationalHint,
                CascadeLayer::Unlayered,
                0,
                2,
            ),
            matched(
                "color: #444444",
                Origin::Author,
                CascadeLayer::Unlayered,
                0,
                3,
            ),
        ],
    );
    assert_eq!(values.color, "#444444".parse::<Color>().unwrap());

    let hinted = cascade(
        None,
        vec![
            matched(
                "color: #222222",
                Origin::User,
                CascadeLayer::Unlayered,
                Specificity::INLINE.0,
                0,
            ),
            matched(
                "color: #333333",
                Origin::AuthorPresentationalHint,
                CascadeLayer::Unlayered,
                0,
                1,
            ),
        ],
    );
    assert_eq!(hinted.color, "#333333".parse::<Color>().unwrap());
}
#[test]
fn cascade_layers_reverse_for_important_declarations() {
    let values = cascade(
        None,
        vec![
            matched(
                "color: #111111",
                Origin::Author,
                CascadeLayer::Layer(0),
                1,
                0,
            ),
            matched(
                "color: #222222",
                Origin::Author,
                CascadeLayer::Layer(1),
                1,
                1,
            ),
            matched(
                "color: #333333",
                Origin::Author,
                CascadeLayer::Unlayered,
                1,
                2,
            ),
            matched(
                "background-color: #444444 !important",
                Origin::Author,
                CascadeLayer::Unlayered,
                1,
                3,
            ),
            matched(
                "background-color: #555555 !important",
                Origin::Author,
                CascadeLayer::Layer(1),
                1,
                4,
            ),
            matched(
                "background-color: #666666 !important",
                Origin::Author,
                CascadeLayer::Layer(0),
                1,
                5,
            ),
        ],
    );

    assert_eq!(values.color, "#333333".parse::<Color>().unwrap());
    assert_eq!(values.background_color, "#666666".parse::<Color>().unwrap());
}

#[test]
fn inheritance_and_css_wide_keywords_are_property_aware() {
    let parent = livery::ComputedValues {
        color: "#3568b8".parse().unwrap(),
        width: Size::Value(LengthPercentage::Length(Length::rem(42.0))),
        margin_left: Margin::Value(LengthPercentage::Length(Length::px(12.0))),
        ..Default::default()
    };
    let values = cascade(
        Some(&parent),
        vec![
            matched(
                "color: unset",
                Origin::Author,
                CascadeLayer::Unlayered,
                1,
                0,
            ),
            matched(
                "width: inherit",
                Origin::Author,
                CascadeLayer::Unlayered,
                1,
                1,
            ),
            matched(
                "margin-left: unset",
                Origin::Author,
                CascadeLayer::Unlayered,
                1,
                2,
            ),
        ],
    );

    assert_eq!(values.color, parent.color);
    assert_eq!(values.width, parent.width);
    assert_eq!(values.margin_left, Margin::Value(LengthPercentage::ZERO));
}

#[test]
fn grid_placement_shorthands_expand_to_their_longhands() {
    use livery::values::GridPlacement;
    let placements = |css: &str| {
        let block = parse_declaration_block(css);
        assert!(block.errors.is_empty(), "{css}: {:?}", block.errors);
        block
            .declarations
            .iter()
            .map(|declaration| {
                let livery::cascade::DeclaredValue::Value(PropertyValue::GridPlacement(placement)) =
                    &declaration.value
                else {
                    panic!("{css}: not a grid placement");
                };
                (declaration.property.metadata().name, *placement)
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        placements("grid-column: 1 / 2"),
        vec![
            ("grid-column-start", GridPlacement::Line(1)),
            ("grid-column-end", GridPlacement::Line(2)),
        ]
    );
    // An omitted second value is auto, never a copy: the bounded grammar has
    // no named lines for the copy rule to apply to.
    assert_eq!(
        placements("grid-row: span 2"),
        vec![
            ("grid-row-start", GridPlacement::Span(2)),
            ("grid-row-end", GridPlacement::Auto),
        ]
    );
    assert_eq!(
        placements("grid-area: 1 / 2 / 3 / 4"),
        vec![
            ("grid-row-start", GridPlacement::Line(1)),
            ("grid-column-start", GridPlacement::Line(2)),
            ("grid-row-end", GridPlacement::Line(3)),
            ("grid-column-end", GridPlacement::Line(4)),
        ]
    );
    assert_eq!(
        placements("grid-area: 2"),
        vec![
            ("grid-row-start", GridPlacement::Line(2)),
            ("grid-column-start", GridPlacement::Auto),
            ("grid-row-end", GridPlacement::Auto),
            ("grid-column-end", GridPlacement::Auto),
        ]
    );
    // Malformed forms are declaration errors, not silent drops.
    for css in [
        "grid-column: 1 / 2 / 3",
        "grid-row: florp",
        "grid-area: 1 / 2 / 3 / 4 / 5",
    ] {
        let block = parse_declaration_block(css);
        assert_eq!(block.errors.len(), 1, "{css}");
        assert!(block.declarations.is_empty(), "{css}");
    }
}
