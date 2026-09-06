// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Generated CSS property and cascade engine.
//!
//! The generated catalog is the executable contract for Livery's first lane:
//! Cambium structural UI. Value parsing and cascade behavior grow against this
//! bounded property set.

#![forbid(unsafe_code)]

pub mod cascade;
pub mod custom;
pub mod media;
pub mod selector;
pub mod stylesheet;
pub mod values;

include!(concat!(env!("OUT_DIR"), "/properties.rs"));

/// Canonicalize one implemented longhand's specified value.
///
/// `None` means Livery cannot safely classify the value yet. Callers at a
/// shared CSSOM boundary must preserve it rather than treating a bounded
/// grammar as proof that full-web CSS is invalid.
pub fn canonicalize_specified_longhand(name: &str, value: &str) -> Option<String> {
    if custom::contains_var(value) {
        return None;
    }
    let property = PropertyId::from_css_name(&name.to_ascii_lowercase())?;
    match value.trim().to_ascii_lowercase().as_str() {
        "initial" => Some("initial".to_string()),
        "inherit" => Some("inherit".to_string()),
        "unset" => Some("unset".to_string()),
        _ => {
            // Color longhands serialize their *specified* form, which keeps
            // more of the authored shape than the computed Color does:
            // keywords stay keywords, and color-mix() and relative colors
            // serialize as themselves (csswg-drafts #7302).
            if property.metadata().value_type == ValueType::Color {
                return value
                    .parse::<values::SpecifiedColor>()
                    .ok()
                    .map(|specified| specified.to_string());
            }
            // Opacity clamps at computed-value time, not at parse:
            // `opacity: 3` is valid and its specified value serializes as
            // `3`. The Opacity type stores the clamped computed form, so the
            // raw number is reconstructed here at the specified boundary.
            if property.metadata().value_type == ValueType::Opacity
                && value.trim().parse::<values::Opacity>().is_ok()
                && let Some(raw) = specified_opacity(value.trim())
            {
                return Some(raw);
            }
            PropertyValue::parse(property, value)
                .ok()
                .map(|parsed| parsed.to_css_string())
        },
    }
}

/// The specified serialization of a plain opacity value: the authored
/// number, unclamped, with percentages resolved to their number form.
fn specified_opacity(value: &str) -> Option<String> {
    let raw = if let Some(percentage) = value.strip_suffix('%') {
        percentage.trim().parse::<f32>().ok()? / 100.0
    } else {
        value.parse::<f32>().ok()?
    };
    raw.is_finite().then(|| values::format_number_public(raw))
}

/// Canonicalize one specified CSSOM value covered by Livery's retained value
/// model.
///
/// Longhands use the generated property catalog. The border shorthand has one
/// additional reconstruction path so harvested `calc()` widths serialize
/// through the same value grammar while the authored style and color tokens
/// retain their CSSOM spelling.
pub fn canonicalize_specified_value(name: &str, value: &str) -> Option<String> {
    canonicalize_specified_longhand(name, value).or_else(|| {
        if name.eq_ignore_ascii_case("border") {
            canonicalize_border(value)
        } else {
            canonicalize_specified_shorthand(name, value)
        }
    })
}

/// One longhand component produced from a supported inline-style shorthand.
///
/// The declaration host stores these components rather than retaining the
/// authored shorthand, matching the cascade's shorthand expansion boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InlineShorthandComponent {
    pub name: &'static str,
    pub value: String,
}

/// How Livery classified an inline-style shorthand assignment.
///
/// A deferred value is syntactically owned by a supported shorthand but
/// contains `var()` and cannot expand before custom-property substitution.
/// Keeping it distinct from `Invalid` lets CSSOM report support without
/// inventing component values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InlineShorthandExpansion {
    Expanded(Vec<InlineShorthandComponent>),
    Deferred,
    Invalid,
}

/// Return the ordered component longhands for a shorthand supported at the
/// inline CSSOM seam.
pub fn specified_shorthand_longhands(name: &str) -> Option<Vec<&'static str>> {
    let shorthand = ShorthandId::from_css_name(&name.to_ascii_lowercase())?;
    matches!(
        shorthand,
        ShorthandId::Columns | ShorthandId::Flex | ShorthandId::FlexFlow
    )
    .then(|| {
        shorthand
            .metadata()
            .longhands
            .iter()
            .map(|property| property.metadata().name)
            .collect()
    })
}

/// Return whether `name` is one of the shorthands whose bounded parser can
/// classify at the CSSOM specified-value seam.
pub fn is_implemented_shorthand(name: &str) -> bool {
    specified_shorthand_longhands(name).is_some()
}

/// Expand a supported shorthand through Livery's declaration parser.
///
/// This preserves the cascade's grammar, defaults, CSS-wide keyword handling,
/// and canonical component serializations. It deliberately exposes variables
/// as deferred rather than pretending their eventual longhands are known.
pub fn classify_specified_shorthand(name: &str, value: &str) -> InlineShorthandExpansion {
    let name = name.to_ascii_lowercase();
    let Some(longhand_names) = specified_shorthand_longhands(&name) else {
        return InlineShorthandExpansion::Invalid;
    };
    if value.trim().is_empty() || cascade::contains_top_level_declaration_delimiter(value) {
        return InlineShorthandExpansion::Invalid;
    }

    let css_wide = value.trim().to_ascii_lowercase();
    if matches!(css_wide.as_str(), "initial" | "inherit" | "unset") {
        return InlineShorthandExpansion::Expanded(
            longhand_names
                .iter()
                .map(|&longhand_name| InlineShorthandComponent {
                    name: longhand_name,
                    value: css_wide.clone(),
                })
                .collect(),
        );
    }

    let block = cascade::parse_declaration_block(&format!("{name}: {value}"));
    if !block.errors.is_empty()
        || !block.custom.is_empty()
        || block.declarations.len() != longhand_names.len()
        || block
            .declarations
            .iter()
            .any(|declaration| declaration.important)
    {
        return InlineShorthandExpansion::Invalid;
    }

    let mut components = Vec::with_capacity(longhand_names.len());
    for (longhand_name, declaration) in longhand_names.iter().zip(&block.declarations) {
        if declaration.property.metadata().name != *longhand_name {
            return InlineShorthandExpansion::Invalid;
        }
        let value = match &declaration.value {
            cascade::DeclaredValue::Value(value) => value.to_css_string(),
            cascade::DeclaredValue::Initial => "initial".to_string(),
            cascade::DeclaredValue::Inherit => "inherit".to_string(),
            cascade::DeclaredValue::Unset => "unset".to_string(),
            cascade::DeclaredValue::Pending(_) => return InlineShorthandExpansion::Deferred,
        };
        components.push(InlineShorthandComponent {
            name: longhand_name,
            value,
        });
    }
    InlineShorthandExpansion::Expanded(components)
}

/// Expand a valid non-variable shorthand into its canonical component values.
///
/// Use [`classify_specified_shorthand`] when the caller must distinguish a
/// deferred `var()` assignment from invalid syntax.
pub fn expand_specified_shorthand(name: &str, value: &str) -> Option<Vec<(String, String)>> {
    let InlineShorthandExpansion::Expanded(components) = classify_specified_shorthand(name, value)
    else {
        return None;
    };
    Some(
        components
            .into_iter()
            .map(|component| (component.name.to_string(), component.value))
            .collect(),
    )
}

/// Reconstruct the canonical shorthand value from every ordered longhand
/// component currently stored by an inline style declaration.
///
/// Missing, duplicated, variable-bearing, and mixed CSS-wide components have
/// no valid shorthand serialization and return `None`.
pub fn reconstruct_specified_shorthand(
    name: &str,
    components: &[(String, String)],
) -> Option<String> {
    let name = name.to_ascii_lowercase();
    let longhand_names = specified_shorthand_longhands(&name)?;
    if components.len() != longhand_names.len() {
        return None;
    }

    let mut values = Vec::with_capacity(longhand_names.len());
    for longhand_name in longhand_names {
        let mut matching = components
            .iter()
            .filter(|(component_name, _)| component_name.eq_ignore_ascii_case(longhand_name));
        let (_, value) = matching.next()?;
        if matching.next().is_some() {
            return None;
        }
        values.push(canonicalize_specified_longhand(longhand_name, value)?);
    }

    let css_wide = values
        .first()
        .filter(|value| matches!(value.as_str(), "initial" | "inherit" | "unset"));
    if let Some(keyword) = css_wide {
        return values
            .iter()
            .all(|value| value == keyword)
            .then(|| keyword.clone());
    }
    if values
        .iter()
        .any(|value| matches!(value.as_str(), "initial" | "inherit" | "unset"))
    {
        return None;
    }

    match name.as_str() {
        "columns" => match (values[0].as_str(), values[1].as_str()) {
            ("auto", "auto") => Some("auto".to_owned()),
            ("auto", count) => Some(count.to_owned()),
            (width, "auto") => Some(width.to_owned()),
            (width, count) => Some(format!("{width} {count}")),
        },
        "flex" => Some(values.join(" ")),
        "flex-flow" => {
            let direction = &values[0];
            let wrap = &values[1];
            match (direction.as_str(), wrap.as_str()) {
                ("row", "nowrap") => Some(direction.clone()),
                ("row", _) => Some(wrap.clone()),
                (_, "nowrap") => Some(direction.clone()),
                _ => Some(format!("{direction} {wrap}")),
            }
        },
        _ => None,
    }
}

/// Canonicalize the implemented flex shorthands using the same declaration
/// parser that drives the retained cascade. `None` means either that the
/// shorthand is outside this bounded helper or that the value is invalid.
/// Values containing `var()` remain unknown to this parser and therefore pass
/// through at the shared CSSOM boundary.
pub fn canonicalize_specified_shorthand(name: &str, value: &str) -> Option<String> {
    let components = expand_specified_shorthand(name, value)?;
    reconstruct_specified_shorthand(name, &components)
}

fn canonicalize_border(value: &str) -> Option<String> {
    use values::{BorderStyle, BorderWidth, Color, LengthPercentage};

    if custom::contains_var(value) {
        return None;
    }
    let components = top_level_components(value)?;
    let mut width = false;
    let mut style = false;
    let mut color = false;
    let mut canonical = Vec::with_capacity(components.len());
    for component in components {
        if !width {
            if let Ok(parsed) = component.parse::<BorderWidth>() {
                width = true;
                canonical.push(parsed.to_string());
                continue;
            }
            if let Ok(parsed) = component.parse::<LengthPercentage>()
                && !parsed.has_percentage()
            {
                width = true;
                canonical.push(parsed.to_string());
                continue;
            }
        }
        if !style && component.parse::<BorderStyle>().is_ok() {
            style = true;
            canonical.push(component.to_string());
            continue;
        }
        if !color && component.parse::<Color>().is_ok() {
            color = true;
            canonical.push(component.to_string());
            continue;
        }
        return None;
    }
    (!canonical.is_empty()).then(|| canonical.join(" "))
}

fn top_level_components(value: &str) -> Option<Vec<&str>> {
    let mut components = Vec::new();
    let mut start = None;
    let mut depth = 0_u32;
    for (index, character) in value.char_indices() {
        match character {
            '(' => {
                depth = depth.checked_add(1)?;
                start.get_or_insert(index);
            },
            ')' => {
                depth = depth.checked_sub(1)?;
            },
            _ if character.is_ascii_whitespace() && depth == 0 => {
                if let Some(component_start) = start.take() {
                    components.push(&value[component_start..index]);
                }
            },
            _ => {
                start.get_or_insert(index);
            },
        }
    }
    if depth != 0 {
        return None;
    }
    if let Some(component_start) = start {
        components.push(&value[component_start..]);
    }
    Some(components)
}
