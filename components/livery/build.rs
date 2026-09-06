// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::PathBuf,
};

use serde::Deserialize;

#[derive(Deserialize)]
struct Database {
    schema: u32,
    lane: String,
    engine_owner: String,
    consumer: String,
    status: String,
    sources: BTreeMap<String, Source>,
    shorthands: BTreeMap<String, Shorthand>,
    property: Vec<Property>,
    /// Harvest H0: the servo-lane property space livery does not implement
    /// yet, imported as data by tools/import-stylo-db. Known to the
    /// catalog and rejected with a known-unimplemented diagnostic.
    #[serde(default)]
    unimplemented: Vec<Unimplemented>,
    #[serde(default)]
    unimplemented_shorthand: Vec<UnimplementedShorthandEntry>,
}

#[derive(Deserialize)]
struct Source {
    url: String,
}

#[derive(Deserialize)]
struct Shorthand {
    css_name: Option<String>,
    longhands: Vec<String>,
    grammar: String,
    seed_values: Vec<String>,
    source: String,
}

#[derive(Deserialize)]
struct Property {
    name: String,
    value_type: String,
    inherited: bool,
    initial: String,
    grammar: String,
    seed_values: Vec<String>,
    animation: String,
    source: String,
    /// The structural subset of the specification that is actually built,
    /// when it is not the whole property. A property carrying this is
    /// **partial**: the parser accepts the value and the engine honours the
    /// named subset only.
    ///
    /// This exists because a catalog that can only say implemented or
    /// unimplemented turns a property-name count into a conformance claim,
    /// which it is not. Recognising a value is not implementing its
    /// semantics.
    #[serde(default)]
    partial: Option<String>,
    /// Legacy names that parse as this property (`word-wrap` for
    /// `overflow-wrap`). Aliases are parse-time spellings only.
    #[serde(default)]
    aliases: Vec<String>,
    /// Stylo's logical-property override group. A logical property and all
    /// physical targets it can resolve to share this identifier.
    #[serde(default)]
    logical_group: Option<String>,
    #[serde(default)]
    logical_axis: Option<String>,
    #[serde(default)]
    physical_axis: Option<String>,
    #[serde(default)]
    logical_side: Option<String>,
    #[serde(default)]
    physical_side: Option<String>,
}

#[derive(Deserialize)]
struct Unimplemented {
    name: String,
    group: String,
    inherited: bool,
    animation: String,
    #[serde(default)]
    logical: bool,
    #[serde(default)]
    logical_group: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    spec: String,
}

#[derive(Deserialize)]
struct UnimplementedShorthandEntry {
    name: String,
    sub_properties: Vec<String>,
    #[serde(default)]
    aliases: Vec<String>,
    spec: String,
}

fn rust_name(css_name: &str) -> String {
    css_name
        .split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|ch| ch.to_ascii_uppercase())
                .into_iter()
                .chain(chars)
                .collect::<String>()
        })
        .collect()
}

fn literal(value: &str) -> String {
    format!("{value:?}")
}

fn string_slice(values: &[String]) -> String {
    let values = values
        .iter()
        .map(|value| literal(value))
        .collect::<Vec<_>>()
        .join(", ");
    format!("&[{values}]")
}

fn rust_field(css_name: &str) -> String {
    css_name.replace('-', "_")
}

fn logical_axis_variant(value: &str) -> &'static str {
    match value {
        "inline" => "crate::values::LogicalAxis::Inline",
        "block" => "crate::values::LogicalAxis::Block",
        _ => panic!("unsupported logical axis {value}"),
    }
}

fn physical_axis_variant(value: &str) -> &'static str {
    match value {
        "horizontal" => "crate::values::PhysicalAxis::Horizontal",
        "vertical" => "crate::values::PhysicalAxis::Vertical",
        _ => panic!("unsupported physical axis {value}"),
    }
}

fn logical_side_variant(value: &str) -> &'static str {
    match value {
        "block-start" => "crate::values::LogicalSide::BlockStart",
        "block-end" => "crate::values::LogicalSide::BlockEnd",
        "inline-start" => "crate::values::LogicalSide::InlineStart",
        "inline-end" => "crate::values::LogicalSide::InlineEnd",
        _ => panic!("unsupported logical side {value}"),
    }
}

fn physical_side_variant(value: &str) -> &'static str {
    match value {
        "top" => "crate::values::PhysicalSide::Top",
        "right" => "crate::values::PhysicalSide::Right",
        "bottom" => "crate::values::PhysicalSide::Bottom",
        "left" => "crate::values::PhysicalSide::Left",
        _ => panic!("unsupported physical side {value}"),
    }
}

fn option_variant(value: Option<&String>, variant: fn(&str) -> &'static str) -> String {
    value.map_or_else(
        || "None".to_owned(),
        |value| format!("Some({})", variant(value)),
    )
}

fn value_type_path(value_type: &str) -> &'static str {
    match value_type {
        "alignment" => "crate::values::Alignment",
        "animation-delay" => "crate::values::AnimationDelay",
        "animation-name" => "crate::values::AnimationName",
        "timing-function" => "crate::values::TimingFunction",
        "aspect-ratio" => "crate::values::AspectRatio",
        "background-attachment" => "crate::values::BackgroundAttachment",
        "background-box" => "crate::values::BackgroundBox",
        "background-image" => "crate::values::BackgroundImage",
        "background-position" => "crate::values::BackgroundPosition",
        "background-repeat" => "crate::values::BackgroundRepeat",
        "background-size" => "crate::values::BackgroundSize",
        "border-collapse" => "crate::values::BorderCollapse",
        "border-style" => "crate::values::BorderStyle",
        "border-width" => "crate::values::BorderWidth",
        "box-shadow" => "crate::values::BoxShadow",
        "box-sizing" => "crate::values::BoxSizing",
        "break-after" => "crate::values::BreakAfter",
        "break-before" => "crate::values::BreakBefore",
        "break-inside" => "crate::values::BreakInside",
        "caption-side" => "crate::values::CaptionSide",
        "clip-path" => "crate::values::ClipPath",
        "column-count" => "crate::values::ColumnCount",
        "column-fill" => "crate::values::ColumnFill",
        "column-width" => "crate::values::ColumnWidth",
        "table-layout" => "crate::values::TableLayout",
        "table-border-spacing" => "crate::values::TableBorderSpacing",
        "clear" => "crate::values::Clear",
        "color-scheme" => "crate::values::ColorSchemeList",
        "container-name" => "crate::values::ContainerName",
        "container-type" => "crate::values::ContainerType",
        "color" => "crate::values::ComputedColor",
        "contain" => "crate::values::Contain",
        "contain-intrinsic-size" => "crate::values::ContainIntrinsicSize",
        "direction" => "crate::values::Direction",
        "display" => "crate::values::Display",
        "duration" => "crate::values::Duration",
        "empty-cells" => "crate::values::EmptyCells",
        "font-family" => "crate::values::FontFamily",
        "font-feature-settings" => "crate::values::FontFeatureSettings",
        "font-size" => "crate::values::FontSize",
        "font-style" => "crate::values::FontStyle",
        "font-variant-ligatures" => "crate::values::FontVariantLigatures",
        "font-weight" => "crate::values::FontWeight",
        "flex-direction" => "crate::values::FlexDirection",
        "flex-basis" => "crate::values::FlexBasis",
        "flex-factor" => "crate::values::FlexFactor",
        "flex-wrap" => "crate::values::FlexWrap",
        "float" => "crate::values::Float",
        "gap" => "crate::values::Gap",
        "grid-auto-flow" => "crate::values::GridAutoFlow",
        "grid-placement" => "crate::values::GridPlacement",
        "grid-template" => "crate::values::GridTemplate",
        "hanging-punctuation" => "crate::values::HangingPunctuation",
        "inset" => "crate::values::Inset",
        "line-height" => "crate::values::LineHeight",
        "list-style-type" => "crate::values::ListStyleType",
        "margin" => "crate::values::Margin",
        "opacity" => "crate::values::Opacity",
        "order" => "crate::values::Order",
        "orphans" => "crate::values::Orphans",
        "overflow" => "crate::values::Overflow",
        "padding" => "crate::values::Padding",
        "pointer-events" => "crate::values::PointerEvents",
        "position" => "crate::values::Position",
        "radius" => "crate::values::Radius",
        "rotate" => "crate::values::Rotate",
        "scale" => "crate::values::Scale",
        "shape-outside" => "crate::values::ShapeOutside",
        "size" => "crate::values::Size",
        "spacing" => "crate::values::Spacing",
        "text-align" => "crate::values::TextAlign",
        "text-align-last" => "crate::values::TextAlignLast",
        "text-decoration-color" => "crate::values::Color",
        "text-decoration-line" => "crate::values::TextDecorationLine",
        "text-indent" => "crate::values::TextIndent",
        "text-justify" => "crate::values::TextJustify",
        "text-transform" => "crate::values::TextTransform",
        "text-wrap-mode" => "crate::values::TextWrapMode",
        "overflow-wrap" => "crate::values::OverflowWrap",
        "word-break" => "crate::values::WordBreak",
        "line-break" => "crate::values::LineBreak",
        "hyphens" => "crate::values::Hyphens",
        "tab-size" => "crate::values::TabSize",
        "transform" => "crate::values::Transform",
        "transition-property" => "crate::values::TransitionProperty",
        "visibility" => "crate::values::Visibility",
        "widows" => "crate::values::Widows",
        "vertical-align" => "crate::values::VerticalAlign",
        "white-space-collapse" => "crate::values::WhiteSpaceCollapse",
        "writing-mode" => "crate::values::WritingMode",
        "z-index" => "crate::values::ZIndex",
        _ => panic!("unsupported value_type {value_type}"),
    }
}

fn value_type_is_copy(value_type: &str) -> bool {
    !matches!(
        value_type,
        "animation-name"
            | "background-image"
            | "box-shadow"
            | "color-scheme"
            | "clip-path"
            | "color"
            | "container-name"
            | "font-family"
            | "font-feature-settings"
            | "grid-template"
            | "transform"
    )
}

fn initial_expression(property: &Property) -> &'static str {
    match (property.value_type.as_str(), property.initial.as_str()) {
        ("alignment", "auto") => "crate::values::Alignment::Auto",
        ("alignment", "normal") => "crate::values::Alignment::Normal",
        ("alignment", "start") => "crate::values::Alignment::Start",
        ("alignment", "stretch") => "crate::values::Alignment::Stretch",
        ("animation-delay", "0s") => "crate::values::AnimationDelay::ZERO",
        ("animation-name", "none") => "crate::values::AnimationName::None",
        ("timing-function", "linear") => "crate::values::TimingFunction::Linear",
        ("aspect-ratio", "auto") => "crate::values::AspectRatio::Auto",
        ("background-attachment", "scroll") => "crate::values::BackgroundAttachment::Scroll",
        ("background-box", "border-box") => "crate::values::BackgroundBox::BorderBox",
        ("background-box", "padding-box") => "crate::values::BackgroundBox::PaddingBox",
        ("background-image", "none") => "crate::values::BackgroundImage::None",
        ("background-position", "0% 0%") => "crate::values::BackgroundPosition::ZERO",
        ("background-repeat", "repeat") => "crate::values::BackgroundRepeat::REPEAT",
        ("background-size", "auto") => "crate::values::BackgroundSize::AUTO",
        ("border-collapse", "separate") => "crate::values::BorderCollapse::Separate",
        ("border-style", "none") => "crate::values::BorderStyle::None",
        ("border-width", "medium") => "crate::values::BorderWidth::Medium",
        ("box-shadow", "none") => "crate::values::BoxShadow::None",
        ("box-sizing", "content-box") => "crate::values::BoxSizing::ContentBox",
        ("break-after", "auto") => "crate::values::BreakAfter::Auto",
        ("break-before", "auto") => "crate::values::BreakBefore::Auto",
        ("break-inside", "auto") => "crate::values::BreakInside::Auto",
        ("caption-side", "top") => "crate::values::CaptionSide::Top",
        ("clip-path", "none") => "crate::values::ClipPath::None",
        ("column-count", "auto") => "crate::values::ColumnCount::Auto",
        ("column-fill", "balance") => "crate::values::ColumnFill::Balance",
        ("column-width", "auto") => "crate::values::ColumnWidth::Auto",
        ("table-layout", "auto") => "crate::values::TableLayout::Auto",
        ("table-border-spacing", "0") => "crate::values::TableBorderSpacing::ZERO",
        ("clear", "none") => "crate::values::Clear::None",
        ("color-scheme", "normal") => "crate::values::ColorSchemeList::NORMAL",
        ("container-name", "none") => "crate::values::ContainerName::None",
        ("container-type", "normal") => "crate::values::ContainerType::Normal",
        ("color", "transparent") => "crate::values::ComputedColor::TRANSPARENT",
        ("color", "currentcolor") => "crate::values::ComputedColor::CURRENT_COLOR",
        ("color", "CanvasText") => "crate::values::ComputedColor::CANVAS_TEXT",
        ("contain", "none") => "crate::values::Contain::NONE",
        ("contain-intrinsic-size", "none") => "crate::values::ContainIntrinsicSize::NONE",
        ("direction", "ltr") => "crate::values::Direction::Ltr",
        ("display", "inline") => "crate::values::Display::Inline",
        ("duration", "0s") => "crate::values::Duration::ZERO",
        ("empty-cells", "show") => "crate::values::EmptyCells::Show",
        ("font-family", "depends-on-user-agent") => "crate::values::FontFamily::UserAgentDefault",
        ("font-feature-settings", "normal") => "crate::values::FontFeatureSettings::Normal",
        ("font-size", "medium") => "crate::values::FontSize::Medium",
        ("font-style", "normal") => "crate::values::FontStyle::Normal",
        ("font-variant-ligatures", "normal") => "crate::values::FontVariantLigatures::NORMAL",
        ("font-weight", "normal") => "crate::values::FontWeight::Normal",
        ("flex-direction", "row") => "crate::values::FlexDirection::Row",
        ("flex-basis", "auto") => "crate::values::FlexBasis::Auto",
        ("flex-factor", "0") => "crate::values::FlexFactor::ZERO",
        ("flex-factor", "1") => "crate::values::FlexFactor::ONE",
        ("flex-wrap", "nowrap") => "crate::values::FlexWrap::NoWrap",
        ("float", "none") => "crate::values::Float::None",
        ("gap", "0") => "crate::values::Gap::ZERO",
        ("grid-auto-flow", "row") => "crate::values::GridAutoFlow::Row",
        ("grid-placement", "auto") => "crate::values::GridPlacement::Auto",
        ("grid-template", "none") => "crate::values::GridTemplate::None",
        ("hanging-punctuation", "none") => "crate::values::HangingPunctuation::NONE",
        ("inset", "auto") => "crate::values::Inset::Auto",
        ("line-height", "normal") => "crate::values::LineHeight::Normal",
        ("list-style-type", "disc") => "crate::values::ListStyleType::Disc",
        ("margin", "0") => "crate::values::Margin::Value(crate::values::LengthPercentage::ZERO)",
        ("opacity", "1") => "crate::values::Opacity::ONE",
        ("order", "0") => "crate::values::Order::ZERO",
        ("orphans", "2") => "crate::values::Orphans(2)",
        ("overflow", "visible") => "crate::values::Overflow::Visible",
        ("padding", "0") => "crate::values::Padding::ZERO",
        ("pointer-events", "auto") => "crate::values::PointerEvents::Auto",
        ("position", "static") => "crate::values::Position::Static",
        ("radius", "0") => "crate::values::Radius::ZERO",
        ("rotate", "none") => "crate::values::Rotate::None",
        ("scale", "none") => "crate::values::Scale::None",
        ("shape-outside", "none") => "crate::values::ShapeOutside::None",
        ("size", "auto") => "crate::values::Size::Auto",
        ("size", "none") => "crate::values::Size::None",
        ("spacing", "normal") => "crate::values::Spacing::Normal",
        ("text-align", "start") => "crate::values::TextAlign::Start",
        ("text-align-last", "auto") => "crate::values::TextAlignLast::Auto",
        ("text-decoration-color", "currentcolor") => "crate::values::ComputedColor::CURRENT_COLOR",
        ("text-decoration-line", "none") => "crate::values::TextDecorationLine::NONE",
        ("text-indent", "0") => "crate::values::TextIndent::ZERO",
        ("text-justify", "auto") => "crate::values::TextJustify::Auto",
        ("text-transform", "none") => "crate::values::TextTransform::NONE",
        ("text-wrap-mode", "wrap") => "crate::values::TextWrapMode::Wrap",
        ("overflow-wrap", "normal") => "crate::values::OverflowWrap::Normal",
        ("word-break", "normal") => "crate::values::WordBreak::Normal",
        ("line-break", "auto") => "crate::values::LineBreak::Auto",
        ("hyphens", "manual") => "crate::values::Hyphens::Manual",
        ("tab-size", "8") => "crate::values::TabSize::DEFAULT",
        ("transform", "none") => "crate::values::Transform::None",
        ("transition-property", "all") => "crate::values::TransitionProperty::All",
        ("visibility", "visible") => "crate::values::Visibility::Visible",
        ("widows", "2") => "crate::values::Widows(2)",
        ("vertical-align", "baseline") => "crate::values::VerticalAlign::Baseline",
        ("white-space-collapse", "collapse") => "crate::values::WhiteSpaceCollapse::Collapse",
        ("writing-mode", "horizontal-tb") => "crate::values::WritingMode::HorizontalTb",
        ("z-index", "auto") => "crate::values::ZIndex::Auto",
        _ => panic!(
            "unsupported initial value {} for {} ({})",
            property.initial, property.name, property.value_type
        ),
    }
}

fn validate(db: &Database) {
    assert_eq!(db.schema, 1, "unsupported properties.toml schema");
    assert!(
        db.property.len() >= 87,
        "the native Cambium lane must retain at least the 87-property receipt"
    );

    let mut names = BTreeSet::new();
    for property in &db.property {
        assert!(
            names.insert(property.name.as_str()),
            "duplicate property {}",
            property.name
        );
        assert!(
            !property.initial.is_empty(),
            "{} has no initial value",
            property.name
        );
        value_type_path(&property.value_type);
        initial_expression(property);
        assert!(
            !property.grammar.is_empty(),
            "{} has no grammar",
            property.name
        );
        assert!(
            !property.seed_values.is_empty(),
            "{} has no seed values",
            property.name
        );
        assert!(
            matches!(
                property.animation.as_str(),
                "by-computed-value" | "discrete" | "none"
            ),
            "{} has unsupported animation class {}",
            property.name,
            property.animation
        );
        assert!(
            db.sources.contains_key(&property.source),
            "{} cites missing source {}",
            property.name,
            property.source
        );
        let logical_mappings = usize::from(property.logical_axis.is_some())
            + usize::from(property.logical_side.is_some());
        let physical_mappings = usize::from(property.physical_axis.is_some())
            + usize::from(property.physical_side.is_some());
        assert!(
            logical_mappings <= 1,
            "{} has multiple logical mappings",
            property.name
        );
        assert!(
            physical_mappings <= 1,
            "{} has multiple physical mappings",
            property.name
        );
        assert!(
            logical_mappings == 0 || physical_mappings == 0,
            "{} is both a logical and physical mapping",
            property.name
        );
        assert_eq!(
            property.logical_group.is_some(),
            logical_mappings + physical_mappings > 0,
            "{} must pair logical_group with exactly one mapping",
            property.name
        );
        if let Some(value) = property.logical_axis.as_deref() {
            logical_axis_variant(value);
        }
        if let Some(value) = property.physical_axis.as_deref() {
            physical_axis_variant(value);
        }
        if let Some(value) = property.logical_side.as_deref() {
            logical_side_variant(value);
        }
        if let Some(value) = property.physical_side.as_deref() {
            physical_side_variant(value);
        }
    }

    for property in &db.property {
        let Some(group) = property.logical_group.as_deref() else {
            continue;
        };
        if property.logical_axis.is_some() {
            for target in ["horizontal", "vertical"] {
                assert!(
                    db.property.iter().any(|candidate| {
                        candidate.logical_group.as_deref() == Some(group)
                            && candidate.physical_axis.as_deref() == Some(target)
                    }),
                    "{} logical group {group} has no {target} target",
                    property.name
                );
            }
        }
        if property.logical_side.is_some() {
            for target in ["top", "right", "bottom", "left"] {
                assert!(
                    db.property.iter().any(|candidate| {
                        candidate.logical_group.as_deref() == Some(group)
                            && candidate.physical_side.as_deref() == Some(target)
                    }),
                    "{} logical group {group} has no {target} target",
                    property.name
                );
            }
        }
    }

    for (key, shorthand) in &db.shorthands {
        let css_name = shorthand
            .css_name
            .clone()
            .unwrap_or_else(|| key.replace('_', "-"));
        assert!(
            !shorthand.longhands.is_empty(),
            "{css_name} has no longhands"
        );
        assert!(!shorthand.grammar.is_empty(), "{css_name} has no grammar");
        assert!(
            !shorthand.seed_values.is_empty(),
            "{css_name} has no seed values"
        );
        assert!(
            db.sources.contains_key(&shorthand.source),
            "{css_name} cites missing source {}",
            shorthand.source
        );
        for longhand in &shorthand.longhands {
            assert!(
                names.contains(longhand.as_str()),
                "{css_name} expands to missing longhand {longhand}"
            );
        }
    }

    let shorthand_names: BTreeSet<String> = db
        .shorthands
        .iter()
        .map(|(key, shorthand)| {
            shorthand
                .css_name
                .clone()
                .unwrap_or_else(|| key.replace('_', "-"))
        })
        .collect();
    let mut known = BTreeSet::new();
    for property in &db.property {
        for alias in &property.aliases {
            assert!(
                !names.contains(alias.as_str()) && !shorthand_names.contains(alias),
                "alias {alias} of {} collides with a catalog name",
                property.name
            );
            assert!(
                known.insert(alias.clone()),
                "duplicate alias {alias} on {}",
                property.name
            );
        }
    }
    for entry in &db.unimplemented {
        assert!(
            !names.contains(entry.name.as_str()) && !shorthand_names.contains(&entry.name),
            "{} is implemented; remove its [[unimplemented]] entry",
            entry.name
        );
        assert!(
            known.insert(entry.name.clone()),
            "duplicate unimplemented entry {}",
            entry.name
        );
        assert!(!entry.group.is_empty(), "{} has no group", entry.name);
        assert!(!entry.spec.is_empty(), "{} has no spec", entry.name);
        assert!(
            matches!(
                entry.animation.as_str(),
                "by-computed-value" | "discrete" | "none"
            ),
            "{} has unsupported animation class {}",
            entry.name,
            entry.animation
        );
    }
    for entry in &db.unimplemented_shorthand {
        assert!(
            !names.contains(entry.name.as_str()) && !shorthand_names.contains(&entry.name),
            "{} is implemented; remove its [[unimplemented_shorthand]] entry",
            entry.name
        );
        assert!(
            known.insert(entry.name.clone()),
            "duplicate unimplemented entry {}",
            entry.name
        );
        assert!(
            !entry.sub_properties.is_empty(),
            "{} has no sub_properties",
            entry.name
        );
        assert!(!entry.spec.is_empty(), "{} has no spec", entry.name);
    }
    for (aliases, owner) in db
        .unimplemented
        .iter()
        .map(|entry| (&entry.aliases, &entry.name))
        .chain(
            db.unimplemented_shorthand
                .iter()
                .map(|entry| (&entry.aliases, &entry.name)),
        )
    {
        for alias in aliases {
            assert!(
                !names.contains(alias.as_str())
                    && !shorthand_names.contains(alias)
                    && !known.contains(alias),
                "alias {alias} of {owner} collides with a catalog name"
            );
        }
    }
}

fn generate(db: &Database) -> String {
    let mut out = String::from(
        "// @generated by components/livery/build.rs from properties.toml.\n\
         // Do not edit this output directly.\n\n",
    );

    out.push_str(&format!(
        "pub const DATABASE_SCHEMA: u32 = {};\n",
        db.schema
    ));
    out.push_str(&format!("pub const LANE: &str = {};\n", literal(&db.lane)));
    out.push_str(&format!(
        "pub const ENGINE_OWNER: &str = {};\n",
        literal(&db.engine_owner)
    ));
    out.push_str(&format!(
        "pub const CONSUMER: &str = {};\n",
        literal(&db.consumer)
    ));
    out.push_str(&format!(
        "pub const CATALOG_STATUS: &str = {};\n\n",
        literal(&db.status)
    ));

    out.push_str(
        "#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]\n\
         pub enum AnimationClass {\n    ByComputedValue,\n    Discrete,\n    None,\n}\n\n",
    );
    let value_types = db
        .property
        .iter()
        .map(|property| property.value_type.as_str())
        .collect::<BTreeSet<_>>();
    out.push_str(
        "#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]\n\
         pub enum ValueType {\n",
    );
    for value_type in &value_types {
        out.push_str(&format!("    {},\n", rust_name(value_type)));
    }
    out.push_str("}\n\n");
    out.push_str(
        "#[derive(Clone, Copy, Debug, Eq, PartialEq)]\n\
         pub struct PropertyMetadata {\n\
         \x20   pub id: PropertyId,\n\
         \x20   pub name: &'static str,\n\
         \x20   pub value_type: ValueType,\n\
         \x20   pub inherited: bool,\n\
         \x20   pub initial: &'static str,\n\
         \x20   /// The built subset when the property is partial, `None`\n\
         \x20   /// when the whole specification is built.\n\
         \x20   pub partial: Option<&'static str>,\n\
         \x20   pub grammar: &'static str,\n\
         \x20   pub seed_values: &'static [&'static str],\n\
         \x20   pub animation: AnimationClass,\n\
         \x20   /// Stylo-derived logical override group, when this property\n\
         \x20   /// participates in flow-relative projection.\n\
         \x20   pub logical_group: Option<&'static str>,\n\
         \x20   pub logical_axis: Option<crate::values::LogicalAxis>,\n\
         \x20   pub physical_axis: Option<crate::values::PhysicalAxis>,\n\
         \x20   pub logical_side: Option<crate::values::LogicalSide>,\n\
         \x20   pub physical_side: Option<crate::values::PhysicalSide>,\n\
         \x20   pub source_url: &'static str,\n\
         }\n\n",
    );
    out.push_str(
        "#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]\n\
         #[repr(u8)]\n\
         pub enum PropertyId {\n",
    );
    for property in &db.property {
        out.push_str(&format!("    {},\n", rust_name(&property.name)));
    }
    out.push_str("}\n\nimpl PropertyId {\n    pub const ALL: &'static [Self] = &[\n");
    for property in &db.property {
        out.push_str(&format!("        Self::{},\n", rust_name(&property.name)));
    }
    out.push_str(
        "    ];\n\n    pub fn from_css_name(name: &str) -> Option<Self> {\n        match name {\n",
    );
    for property in &db.property {
        out.push_str(&format!(
            "            {} => Some(Self::{}),\n",
            literal(&property.name),
            rust_name(&property.name)
        ));
        for alias in &property.aliases {
            out.push_str(&format!(
                "            {} => Some(Self::{}),\n",
                literal(alias),
                rust_name(&property.name)
            ));
        }
    }
    out.push_str("            _ => None,\n        }\n    }\n\n    pub const fn metadata(self) -> PropertyMetadata {\n        match self {\n");
    for property in &db.property {
        let animation = match property.animation.as_str() {
            "by-computed-value" => "AnimationClass::ByComputedValue",
            "discrete" => "AnimationClass::Discrete",
            "none" => "AnimationClass::None",
            _ => unreachable!("validated animation class"),
        };
        let source = &db.sources[&property.source].url;
        out.push_str(&format!(
            "            Self::{variant} => PropertyMetadata {{ id: Self::{variant}, name: {name}, value_type: ValueType::{value_type}, inherited: {inherited}, initial: {initial}, partial: {partial}, grammar: {grammar}, seed_values: {seed_values}, animation: {animation}, logical_group: {logical_group}, logical_axis: {logical_axis}, physical_axis: {physical_axis}, logical_side: {logical_side}, physical_side: {physical_side}, source_url: {source} }},\n",
            variant = rust_name(&property.name),
            name = literal(&property.name),
            value_type = rust_name(&property.value_type),
            inherited = property.inherited,
            initial = literal(&property.initial),
            partial = match &property.partial {
                Some(note) => format!("Some({})", literal(note)),
                None => "None".to_owned(),
            },
            grammar = literal(&property.grammar),
            seed_values = string_slice(&property.seed_values),
            logical_group = property
                .logical_group
                .as_deref()
                .map_or_else(|| "None".to_owned(), |group| format!("Some({})", literal(group))),
            logical_axis = option_variant(property.logical_axis.as_ref(), logical_axis_variant),
            physical_axis = option_variant(property.physical_axis.as_ref(), physical_axis_variant),
            logical_side = option_variant(property.logical_side.as_ref(), logical_side_variant),
            physical_side = option_variant(property.physical_side.as_ref(), physical_side_variant),
            source = literal(source),
        ));
    }
    out.push_str(
        "        }\n    }\n\n\
         \x20   /// Resolve a generated logical longhand to the physical longhand\n\
         \x20   /// selected by the element's computed writing mode and direction.\n\
         \x20   pub fn to_physical(\n\
         \x20       self,\n\
         \x20       writing_mode: crate::values::WritingMode,\n\
         \x20       direction: crate::values::Direction,\n\
         \x20   ) -> Self {\n\
         \x20       let metadata = self.metadata();\n\
         \x20       let Some(group) = metadata.logical_group else { return self };\n\
         \x20       if let Some(axis) = metadata.logical_axis {\n\
         \x20           let target = axis.to_physical(writing_mode);\n\
         \x20           return Self::ALL.iter().copied().find(|candidate| {\n\
         \x20               let candidate = candidate.metadata();\n\
         \x20               candidate.logical_group == Some(group)\n\
         \x20                   && candidate.physical_axis == Some(target)\n\
         \x20           }).expect(\"validated logical axis target\");\n\
         \x20       }\n\
         \x20       if let Some(side) = metadata.logical_side {\n\
         \x20           let target = side.to_physical(writing_mode, direction);\n\
         \x20           return Self::ALL.iter().copied().find(|candidate| {\n\
         \x20               let candidate = candidate.metadata();\n\
         \x20               candidate.logical_group == Some(group)\n\
         \x20                   && candidate.physical_side == Some(target)\n\
         \x20           }).expect(\"validated logical side target\");\n\
         \x20       }\n\
         \x20       self\n\
         \x20   }\n\n\
         \x20   pub const fn is_logical(self) -> bool {\n\
         \x20       let metadata = self.metadata();\n\
         \x20       metadata.logical_axis.is_some() || metadata.logical_side.is_some()\n\
         \x20   }\n\
         }\n\n",
    );

    out.push_str("#[derive(Clone, Debug, PartialEq)]\npub enum PropertyValue {\n");
    for value_type in &value_types {
        out.push_str(&format!(
            "    {}({}),\n",
            rust_name(value_type),
            value_type_path(value_type)
        ));
    }
    out.push_str("}\n\nimpl PropertyValue {\n    pub fn parse(property: PropertyId, input: &str) -> Result<Self, crate::values::ParseError> {\n        match property {\n");
    for property in &db.property {
        out.push_str(&format!(
            "            PropertyId::{property} => input.parse::<{value_path}>().map(Self::{value_variant}),\n",
            property = rust_name(&property.name),
            value_path = value_type_path(&property.value_type),
            value_variant = rust_name(&property.value_type),
        ));
    }
    out.push_str("        }\n    }\n\n    pub const fn value_type(&self) -> ValueType {\n        match self {\n");
    for value_type in &value_types {
        let variant = rust_name(value_type);
        out.push_str(&format!(
            "            Self::{variant}(..) => ValueType::{variant},\n"
        ));
    }
    out.push_str(
        "        }\n    }\n\n    pub fn to_css_string(&self) -> String {\n        match self {\n",
    );
    for value_type in &value_types {
        out.push_str(&format!(
            "            Self::{}(value) => value.to_string(),\n",
            rust_name(value_type)
        ));
    }
    out.push_str(
        "        }\n    }\n\n    /// Resolve viewport-relative lengths at the specified-to-computed boundary.\n    pub fn resolve_viewport(&self, viewport_width: f32, viewport_height: f32) -> Self {\n        match self {\n",
    );
    for value_type in &value_types {
        let variant = rust_name(value_type);
        out.push_str(&format!(
            "            Self::{variant}(value) => Self::{variant}(crate::values::ResolveViewport::resolve_viewport(value, viewport_width, viewport_height)),\n"
        ));
    }
    out.push_str(
        "        }\n    }\n\n    /// Resolve viewport and deferred container-relative lengths from an explicit environment.\n    pub fn resolve_relative_lengths(&self, environment: crate::values::RelativeLengthEnvironment) -> Self {\n        match self {\n",
    );
    for value_type in &value_types {
        let variant = rust_name(value_type);
        out.push_str(&format!(
            "            Self::{variant}(value) => Self::{variant}(crate::values::ResolveViewport::resolve_relative_lengths(value, environment)),\n"
        ));
    }
    out.push_str("        }\n    }\n}\n\n");

    out.push_str("#[derive(Clone, Debug, PartialEq)]\npub struct ComputedValues {\n");
    for property in &db.property {
        out.push_str(&format!(
            "    pub {}: {},\n",
            rust_field(&property.name),
            value_type_path(&property.value_type)
        ));
    }
    out.push_str(
        "}\n\nimpl Default for ComputedValues {\n    fn default() -> Self {\n        Self {\n",
    );
    for property in &db.property {
        out.push_str(&format!(
            "            {}: {},\n",
            rust_field(&property.name),
            initial_expression(property)
        ));
    }
    out.push_str("        }\n    }\n}\n\nimpl ComputedValues {\n    /// Construct a child style with inherited properties copied from `parent`\n    /// and non-inherited properties reset to their CSS initial values.\n    pub fn for_child(parent: &Self) -> Self {\n        let initial = Self::default();\n        Self {\n");
    for property in &db.property {
        let field = rust_field(&property.name);
        if property.inherited {
            if value_type_is_copy(&property.value_type) {
                out.push_str(&format!("            {field}: parent.{field},\n"));
            } else {
                out.push_str(&format!("            {field}: parent.{field}.clone(),\n"));
            }
        } else {
            out.push_str(&format!("            {field}: initial.{field},\n"));
        }
    }
    out.push_str("        }\n    }\n\n    /// Assign one generated property, returning the value unchanged on a type mismatch.\n    pub fn set(&mut self, property: PropertyId, value: PropertyValue) -> Result<(), PropertyValue> {\n        match (property, value) {\n");
    for property in &db.property {
        let property_variant = rust_name(&property.name);
        let value_variant = rust_name(&property.value_type);
        let field = rust_field(&property.name);
        out.push_str(&format!(
            "            (PropertyId::{property_variant}, PropertyValue::{value_variant}(value)) => {{ self.{field} = value; Ok(()) }},\n"
        ));
    }
    out.push_str("            (_, value) => Err(value),\n        }\n    }\n\n    /// Copy one property from another computed style.\n    pub fn copy_property_from(&mut self, property: PropertyId, source: &Self) {\n        match property {\n");
    for property in &db.property {
        let property_variant = rust_name(&property.name);
        let field = rust_field(&property.name);
        if value_type_is_copy(&property.value_type) {
            out.push_str(&format!(
                "            PropertyId::{property_variant} => self.{field} = source.{field},\n"
            ));
        } else {
            out.push_str(&format!(
                "            PropertyId::{property_variant} => self.{field} = source.{field}.clone(),\n"
            ));
        }
    }
    out.push_str("        }\n    }\n}\n\n");

    out.push_str(
        "#[derive(Clone, Copy, Debug, Eq, PartialEq)]\n\
         pub struct ShorthandMetadata {\n\
         \x20   pub id: ShorthandId,\n\
         \x20   pub name: &'static str,\n\
         \x20   pub longhands: &'static [PropertyId],\n\
         \x20   pub grammar: &'static str,\n\
         \x20   pub seed_values: &'static [&'static str],\n\
         \x20   pub source_url: &'static str,\n\
         }\n\n",
    );
    out.push_str(
        "#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]\n\
         pub enum ShorthandId {\n",
    );
    for (key, shorthand) in &db.shorthands {
        let name = shorthand
            .css_name
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| key.replace('_', "-"));
        out.push_str(&format!("    {},\n", rust_name(&name)));
    }
    out.push_str("}\n\nimpl ShorthandId {\n    pub const ALL: &'static [Self] = &[\n");
    for (key, shorthand) in &db.shorthands {
        let name = shorthand
            .css_name
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| key.replace('_', "-"));
        out.push_str(&format!("        Self::{},\n", rust_name(&name)));
    }
    out.push_str(
        "    ];\n\n    pub fn from_css_name(name: &str) -> Option<Self> {\n        match name {\n",
    );
    for (key, shorthand) in &db.shorthands {
        let name = shorthand
            .css_name
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| key.replace('_', "-"));
        out.push_str(&format!(
            "            {} => Some(Self::{}),\n",
            literal(&name),
            rust_name(&name)
        ));
    }
    out.push_str("            _ => None,\n        }\n    }\n\n    pub const fn metadata(self) -> ShorthandMetadata {\n        match self {\n");
    for (key, shorthand) in &db.shorthands {
        let name = shorthand
            .css_name
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| key.replace('_', "-"));
        let longhands = shorthand
            .longhands
            .iter()
            .map(|name| format!("PropertyId::{}", rust_name(name)))
            .collect::<Vec<_>>()
            .join(", ");
        let source = &db.sources[&shorthand.source].url;
        out.push_str(&format!(
            "            Self::{variant} => ShorthandMetadata {{ id: Self::{variant}, name: {name}, longhands: &[{longhands}], grammar: {grammar}, seed_values: {seed_values}, source_url: {source} }},\n",
            variant = rust_name(&name),
            name = literal(&name),
            grammar = literal(&shorthand.grammar),
            seed_values = string_slice(&shorthand.seed_values),
            source = literal(source),
        ));
    }
    out.push_str("        }\n    }\n}\n\n");

    // Harvest H2: general interpolation and tagged property reads.
    out.push_str(
        "impl PropertyValue {\n\
         \x20   /// Interpolate two values of one property (harvest H2). Same-family\n\
         \x20   /// pairs delegate to the family's `Interpolate` impl; a family without\n\
         \x20   /// defined interpolation, or a cross-family pair, flips at the midpoint\n\
         \x20   /// per css-transitions discrete animation.\n\
         \x20   pub fn interpolate(&self, other: &Self, progress: f32) -> Self {\n\
         \x20       match (self, other) {\n",
    );
    for value_type in &value_types {
        let variant = rust_name(value_type);
        out.push_str(&format!(
            "            (Self::{variant}(a), Self::{variant}(b)) => Self::{variant}(crate::values::Interpolate::interpolate_value(a, b, progress)),\n"
        ));
    }
    out.push_str(
        "            _ => {\n\
         \x20               if progress < 0.5 {\n\
         \x20                   self.clone()\n\
         \x20               } else {\n\
         \x20                   other.clone()\n\
         \x20               }\n\
         \x20           },\n\
         \x20       }\n\
         \x20   }\n\
         }\n\n",
    );
    out.push_str(
        "impl ComputedValues {\n\
         \x20   /// Read one generated property as a tagged value.\n\
         \x20   pub fn get(&self, property: PropertyId) -> PropertyValue {\n\
         \x20       match property {\n",
    );
    for property in &db.property {
        let read = if value_type_is_copy(&property.value_type) {
            ""
        } else {
            ".clone()"
        };
        out.push_str(&format!(
            "            PropertyId::{variant} => PropertyValue::{value_variant}(self.{field}{read}),\n",
            variant = rust_name(&property.name),
            value_variant = rust_name(&property.value_type),
            field = rust_field(&property.name),
        ));
    }
    out.push_str("        }\n    }\n}\n\n");

    // Harvest H0: the known-unimplemented property space, as metadata.
    out.push_str(
        "#[derive(Clone, Copy, Debug, Eq, PartialEq)]\n\
         pub enum UnimplementedAnimation {\n\
         \x20   ByComputedValue,\n\
         \x20   Discrete,\n\
         \x20   NotAnimatable,\n\
         }\n\n\
         #[derive(Clone, Copy, Debug, Eq, PartialEq)]\n\
         pub struct UnimplementedLonghand {\n\
         \x20   pub name: &'static str,\n\
         \x20   /// The fork's style struct: the future ComputedValues grouping seam.\n\
         \x20   pub group: &'static str,\n\
         \x20   pub inherited: bool,\n\
         \x20   pub animation: UnimplementedAnimation,\n\
         \x20   pub logical: bool,\n\
         \x20   pub logical_group: Option<&'static str>,\n\
         \x20   pub spec_url: &'static str,\n\
         }\n\n\
         #[derive(Clone, Copy, Debug, Eq, PartialEq)]\n\
         pub struct UnimplementedShorthand {\n\
         \x20   pub name: &'static str,\n\
         \x20   pub sub_properties: &'static [&'static str],\n\
         \x20   pub spec_url: &'static str,\n\
         }\n\n",
    );
    out.push_str("pub const UNIMPLEMENTED_LONGHANDS: &[UnimplementedLonghand] = &[\n");
    for entry in &db.unimplemented {
        let animation = match entry.animation.as_str() {
            "by-computed-value" => "UnimplementedAnimation::ByComputedValue",
            "discrete" => "UnimplementedAnimation::Discrete",
            "none" => "UnimplementedAnimation::NotAnimatable",
            _ => unreachable!("validated animation class"),
        };
        out.push_str(&format!(
            "    UnimplementedLonghand {{ name: {name}, group: {group}, inherited: {inherited}, animation: {animation}, logical: {logical}, logical_group: {logical_group}, spec_url: {spec} }},\n",
            name = literal(&entry.name),
            group = literal(&entry.group),
            inherited = entry.inherited,
            logical = entry.logical,
            logical_group = entry.logical_group.as_deref().map_or_else(
                || "None".to_owned(),
                |group| format!("Some({})", literal(group)),
            ),
            spec = literal(&entry.spec),
        ));
    }
    out.push_str("];\n\n");
    out.push_str("pub const UNIMPLEMENTED_SHORTHANDS: &[UnimplementedShorthand] = &[\n");
    for entry in &db.unimplemented_shorthand {
        out.push_str(&format!(
            "    UnimplementedShorthand {{ name: {name}, sub_properties: {subs}, spec_url: {spec} }},\n",
            name = literal(&entry.name),
            subs = string_slice(&entry.sub_properties),
            spec = literal(&entry.spec),
        ));
    }
    out.push_str("];\n\n");
    out.push_str(
        "/// Look up a known-but-unimplemented longhand by canonical name or alias.\n\
         pub fn unimplemented_longhand(name: &str) -> Option<&'static UnimplementedLonghand> {\n\
         \x20   let index = match name {\n",
    );
    for (index, entry) in db.unimplemented.iter().enumerate() {
        out.push_str(&format!(
            "        {} => {index}usize,\n",
            literal(&entry.name)
        ));
        for alias in &entry.aliases {
            out.push_str(&format!("        {} => {index}usize,\n", literal(alias)));
        }
    }
    out.push_str(
        "        _ => return None,\n    };\n    Some(&UNIMPLEMENTED_LONGHANDS[index])\n}\n\n",
    );
    out.push_str(
        "/// Look up a known-but-unimplemented shorthand by canonical name or alias.\n\
         pub fn unimplemented_shorthand(name: &str) -> Option<&'static UnimplementedShorthand> {\n\
         \x20   let index = match name {\n",
    );
    for (index, entry) in db.unimplemented_shorthand.iter().enumerate() {
        out.push_str(&format!(
            "        {} => {index}usize,\n",
            literal(&entry.name)
        ));
        for alias in &entry.aliases {
            out.push_str(&format!("        {} => {index}usize,\n", literal(alias)));
        }
    }
    out.push_str(
        "        _ => return None,\n    };\n    Some(&UNIMPLEMENTED_SHORTHANDS[index])\n}\n",
    );
    out
}

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let database_path = manifest_dir.join("properties.toml");
    println!("cargo:rerun-if-changed={}", database_path.display());

    let source = fs::read_to_string(&database_path).expect("read properties.toml");
    let database: Database = toml::from_str(&source).expect("parse properties.toml");
    validate(&database);

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(out_dir.join("properties.rs"), generate(&database))
        .expect("write generated properties.rs");
}
