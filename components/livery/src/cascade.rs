// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! DOM-neutral declaration parsing and cascade ordering.

use std::{cmp::Ordering, fmt};

use crate::custom::{
    CustomDeclaration, CustomDeclaredValue, CustomProperties, contains_var, substitute,
};
use crate::media::{Device, SystemPalette};
use crate::values::{
    AnimationDelay, AnimationName, BackgroundAttachment, BackgroundBox, BackgroundImage,
    BackgroundPosition, BackgroundRepeat, BackgroundSize, BorderStyle, BorderWidth, BoxShadow,
    ColorScheme, ColumnCount, ColumnWidth, ComputedColor, Duration, FlexBasis, FlexDirection,
    FlexFactor, FlexWrap, FontFamily, FontFeatureSettings, FontSize, FontStyle,
    FontVariantLigatures, FontWeight, Inset, LineHeight, Margin, Padding, Radius, SystemColor,
    TimingFunction, TransitionProperty, UsedColorContext,
};
use crate::{ComputedValues, PropertyId, PropertyValue, ShorthandId};

/// The host facts needed while one element's color-bearing values become
/// computed values. The media preference remains a host input; an element's
/// `color-scheme` selects its own used scheme from it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorComputeContext {
    preferred_scheme: ColorScheme,
    palette: SystemPalette,
}

impl ColorComputeContext {
    pub fn from_device(device: &Device) -> Self {
        Self {
            preferred_scheme: device.preferred_color_scheme(),
            palette: device.system_palette,
        }
    }

    pub fn new(preferred_scheme: ColorScheme, palette: SystemPalette) -> Self {
        Self {
            preferred_scheme,
            palette,
        }
    }

    /// The host preference remains distinct from every element's used scheme.
    /// Consumers use this only to select that element's already-cascaded
    /// `color-scheme` list.
    pub fn preferred_scheme(self) -> ColorScheme {
        self.preferred_scheme
    }

    /// The host-owned palette that contextual color consumers must use.
    pub fn palette(self) -> SystemPalette {
        self.palette
    }
}

impl Default for ColorComputeContext {
    fn default() -> Self {
        Self::new(ColorScheme::Light, SystemPalette::default())
    }
}

/// A declaration whose value contains `var()` and therefore cannot parse
/// until the element's custom properties are known (harvest H1). A pending
/// shorthand stores one copy per expanded longhand, the fork's
/// `WithVariables` shape.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingSubstitution {
    pub raw: String,
    pub from_shorthand: Option<ShorthandId>,
}

/// A parsed longhand value, including the CSS-wide keywords supported by the
/// first lane.
#[derive(Clone, Debug, PartialEq)]
pub enum DeclaredValue {
    Value(PropertyValue),
    Initial,
    Inherit,
    Unset,
    /// Deferred until `var()` substitution at computed-value time.
    Pending(PendingSubstitution),
}

impl DeclaredValue {
    fn parse(property: PropertyId, input: &str) -> Result<Self, crate::values::ParseError> {
        match input.trim().to_ascii_lowercase().as_str() {
            "initial" => Ok(Self::Initial),
            "inherit" => Ok(Self::Inherit),
            "unset" => Ok(Self::Unset),
            _ => PropertyValue::parse(property, input).map(Self::Value),
        }
    }
}

/// One parsed longhand declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub property: PropertyId,
    pub value: DeclaredValue,
    pub important: bool,
}

/// Why an authored declaration was ignored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeclarationErrorKind {
    UnknownProperty,
    /// The name is in the catalog's imported property space (harvest H0)
    /// but livery does not implement it yet. Distinguishable from a typo
    /// so diagnostics can say what was ignored and why.
    KnownUnimplemented,
    InvalidValue,
    MalformedDeclaration,
}

/// A non-fatal declaration parse diagnostic. CSS drops the declaration and
/// continues parsing the block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclarationError {
    pub name: String,
    pub value: String,
    pub kind: DeclarationErrorKind,
}

/// Parsed declarations plus the declarations CSS error recovery discarded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeclarationBlock {
    pub declarations: Vec<Declaration>,
    /// `--name` declarations, case-sensitive, in source order.
    pub custom: Vec<CustomDeclaration>,
    pub errors: Vec<DeclarationError>,
}

fn without_comments(css: &str) -> String {
    let mut clean = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    let mut in_comment = false;
    while let Some(ch) = chars.next() {
        if in_comment {
            if ch == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_comment = false;
            }
        } else if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            in_comment = true;
        } else {
            clean.push(ch);
        }
    }
    clean
}

fn split_top_level(input: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in input.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if ch == delimiter && depth == 0 => {
                parts.push(&input[start..index]);
                start = index + ch.len_utf8();
            },
            _ => {},
        }
    }
    parts.push(&input[start..]);
    parts
}

pub(crate) fn contains_top_level_declaration_delimiter(input: &str) -> bool {
    split_top_level(&without_comments(input), ';').len() > 1
}

fn split_components(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = None;
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in input.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                start.get_or_insert(index);
                quote = Some(ch);
            },
            '(' => {
                start.get_or_insert(index);
                depth += 1;
            },
            ')' => depth = depth.saturating_sub(1),
            _ if ch.is_ascii_whitespace() && depth == 0 => {
                if let Some(part_start) = start.take() {
                    parts.push(&input[part_start..index]);
                }
            },
            _ => {
                start.get_or_insert(index);
            },
        }
    }
    if let Some(part_start) = start {
        parts.push(&input[part_start..]);
    }
    parts
}

fn strip_important(value: &str) -> (&str, bool) {
    let trimmed = value.trim();
    let Some(bang) = trimmed.rfind('!') else {
        return (trimmed, false);
    };
    if trimmed[bang + 1..].trim().eq_ignore_ascii_case("important") {
        (trimmed[..bang].trim_end(), true)
    } else {
        (trimmed, false)
    }
}

fn push_longhand(block: &mut DeclarationBlock, name: &str, value: &str, important: bool) -> bool {
    let Some(property) = PropertyId::from_css_name(name) else {
        return false;
    };
    if contains_var(value) {
        block.declarations.push(Declaration {
            property,
            value: DeclaredValue::Pending(PendingSubstitution {
                raw: value.to_owned(),
                from_shorthand: None,
            }),
            important,
        });
        return true;
    }
    match DeclaredValue::parse(property, value) {
        Ok(value) => block.declarations.push(Declaration {
            property,
            value,
            important,
        }),
        Err(_) => block.errors.push(DeclarationError {
            name: name.to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        }),
    }
    true
}

fn box_sides<T: Clone>(values: &[T]) -> Option<[T; 4]> {
    match values {
        [all] => Some([all.clone(), all.clone(), all.clone(), all.clone()]),
        [vertical, horizontal] => Some([
            vertical.clone(),
            horizontal.clone(),
            vertical.clone(),
            horizontal.clone(),
        ]),
        [top, horizontal, bottom] => Some([
            top.clone(),
            horizontal.clone(),
            bottom.clone(),
            horizontal.clone(),
        ]),
        [top, right, bottom, left] => {
            Some([top.clone(), right.clone(), bottom.clone(), left.clone()])
        },
        _ => None,
    }
}

fn expand_box_shorthand(
    block: &mut DeclarationBlock,
    shorthand: ShorthandId,
    value: &str,
    important: bool,
) -> bool {
    let parsed = match shorthand {
        ShorthandId::Margin => split_components(value)
            .into_iter()
            .map(str::parse::<Margin>)
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .and_then(|values| box_sides(&values))
            .map(|values| values.map(|value| DeclaredValue::Value(PropertyValue::Margin(value)))),
        ShorthandId::Inset => split_components(value)
            .into_iter()
            .map(str::parse::<Inset>)
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .and_then(|values| box_sides(&values))
            .map(|values| values.map(|value| DeclaredValue::Value(PropertyValue::Inset(value)))),
        ShorthandId::Padding => split_components(value)
            .into_iter()
            .map(str::parse::<Padding>)
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .and_then(|values| box_sides(&values))
            .map(|values| values.map(|value| DeclaredValue::Value(PropertyValue::Padding(value)))),
        ShorthandId::BorderRadius => split_components(value)
            .into_iter()
            .map(str::parse::<Radius>)
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .and_then(|values| box_sides(&values))
            .map(|values| values.map(|value| DeclaredValue::Value(PropertyValue::Radius(value)))),
        ShorthandId::Gap => split_components(value)
            .into_iter()
            .map(str::parse::<crate::values::Gap>)
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .and_then(|values| box_sides(&values))
            .map(|values| values.map(|value| DeclaredValue::Value(PropertyValue::Gap(value)))),
        ShorthandId::Overflow => match split_components(value).as_slice() {
            [only] => only.parse::<crate::values::Overflow>().ok().map(|value| {
                std::array::from_fn(|_| DeclaredValue::Value(PropertyValue::Overflow(value)))
            }),
            [horizontal, vertical] => horizontal
                .parse::<crate::values::Overflow>()
                .ok()
                .zip(vertical.parse::<crate::values::Overflow>().ok())
                .map(|(horizontal, vertical)| {
                    [horizontal, vertical, horizontal, vertical]
                        .map(|value| DeclaredValue::Value(PropertyValue::Overflow(value)))
                }),
            _ => None,
        },
        ShorthandId::BorderColor => split_components(value)
            .into_iter()
            .map(str::parse::<ComputedColor>)
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .and_then(|values| box_sides(&values))
            .map(|values| values.map(|value| DeclaredValue::Value(PropertyValue::Color(value)))),
        ShorthandId::BorderStyle => split_components(value)
            .into_iter()
            .map(str::parse::<BorderStyle>)
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .and_then(|values| box_sides(&values))
            .map(|values| {
                values.map(|value| DeclaredValue::Value(PropertyValue::BorderStyle(value)))
            }),
        ShorthandId::BorderWidth => split_components(value)
            .into_iter()
            .map(str::parse::<BorderWidth>)
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .and_then(|values| box_sides(&values))
            .map(|values| {
                values.map(|value| DeclaredValue::Value(PropertyValue::BorderWidth(value)))
            }),
        _ => return false,
    };
    let Some(values) = parsed else {
        block.errors.push(DeclarationError {
            name: shorthand.metadata().name.to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return true;
    };
    for (&property, value) in shorthand.metadata().longhands.iter().zip(values) {
        block.declarations.push(Declaration {
            property,
            value,
            important,
        });
    }
    true
}

/// Expand `grid-row`, `grid-column`, and `grid-area` into their placement
/// longhands (css-grid section 3.4). Livery's `<grid-line>` grammar has no
/// named lines, so the spec's copy-the-custom-ident rule for omitted values
/// never applies and every omitted position is `auto`.
fn expand_grid_placement(
    block: &mut DeclarationBlock,
    shorthand: ShorthandId,
    value: &str,
    important: bool,
) {
    let longhands = shorthand.metadata().longhands;
    let parts = split_top_level(value, '/');
    let parsed = if parts.is_empty() || parts.len() > longhands.len() {
        None
    } else {
        parts
            .iter()
            .map(|part| part.trim().parse::<crate::values::GridPlacement>())
            .collect::<Result<Vec<_>, _>>()
            .ok()
    };
    let Some(mut placements) = parsed else {
        block.errors.push(DeclarationError {
            name: shorthand.metadata().name.to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    };
    placements.resize(longhands.len(), crate::values::GridPlacement::Auto);
    for (&property, placement) in longhands.iter().zip(placements) {
        block.declarations.push(Declaration {
            property,
            value: DeclaredValue::Value(PropertyValue::GridPlacement(placement)),
            important,
        });
    }
}

/// Expand `grid-template` and the template form of `grid`:
/// `none | <'grid-template-rows'> / <'grid-template-columns'>`. The `grid`
/// shorthand additionally resets `grid-auto-flow` to its initial. The
/// auto-flow forms of `grid` are outside the bounded grammar and reject.
fn expand_grid_template(
    block: &mut DeclarationBlock,
    shorthand: ShorthandId,
    value: &str,
    important: bool,
) {
    use crate::values::{GridAutoFlow, GridTemplate};
    let parts = split_top_level(value, '/');
    let templates = match parts.as_slice() {
        [only] if only.trim().eq_ignore_ascii_case("none") => {
            Some((GridTemplate::None, GridTemplate::None))
        },
        [rows, columns] => rows
            .trim()
            .parse::<GridTemplate>()
            .ok()
            .zip(columns.trim().parse::<GridTemplate>().ok()),
        _ => None,
    };
    let Some((rows, columns)) = templates else {
        block.errors.push(DeclarationError {
            name: shorthand.metadata().name.to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    };
    let mut values = vec![
        (
            PropertyId::GridTemplateRows,
            PropertyValue::GridTemplate(rows),
        ),
        (
            PropertyId::GridTemplateColumns,
            PropertyValue::GridTemplate(columns),
        ),
    ];
    if shorthand == ShorthandId::Grid {
        values.push((
            PropertyId::GridAutoFlow,
            PropertyValue::GridAutoFlow(GridAutoFlow::Row),
        ));
    }
    for (property, value) in values {
        block.declarations.push(Declaration {
            property,
            value: DeclaredValue::Value(value),
            important,
        });
    }
}

/// Expand `place-items`: `<align-items> <justify-items>?`, the second
/// defaulting to the first.
fn expand_place_items(block: &mut DeclarationBlock, value: &str, important: bool) {
    use crate::values::Alignment;
    let parts = split_components(value);
    let parsed = match parts.as_slice() {
        [only] => only.parse::<Alignment>().ok().map(|both| (both, both)),
        [align, justify] => align
            .parse::<Alignment>()
            .ok()
            .zip(justify.parse::<Alignment>().ok()),
        _ => None,
    };
    // `auto` is a self-alignment value; the items properties reject it.
    let Some((align, justify)) =
        parsed.filter(|(a, j)| *a != Alignment::Auto && *j != Alignment::Auto)
    else {
        block.errors.push(DeclarationError {
            name: "place-items".to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    };
    for (property, alignment) in [
        (PropertyId::AlignItems, align),
        (PropertyId::JustifyItems, justify),
    ] {
        block.declarations.push(Declaration {
            property,
            value: DeclaredValue::Value(PropertyValue::Alignment(alignment)),
            important,
        });
    }
}

fn expand_transition(block: &mut DeclarationBlock, value: &str, important: bool) {
    let mut property = None;
    let mut duration = None;
    for item in split_top_level(value, ',') {
        let Some((item_property, item_duration)) = parse_transition_item(item) else {
            block.errors.push(DeclarationError {
                name: "transition".to_owned(),
                value: value.to_owned(),
                kind: DeclarationErrorKind::InvalidValue,
            });
            return;
        };
        if duration
            .is_some_and(|current: Duration| current.milliseconds() != item_duration.milliseconds())
        {
            block.errors.push(DeclarationError {
                name: "transition".to_owned(),
                value: value.to_owned(),
                kind: DeclarationErrorKind::InvalidValue,
            });
            return;
        }
        duration = Some(item_duration);
        property = Some(merge_transition_properties(property, item_property));
    }
    let Some(duration) = duration else {
        block.errors.push(DeclarationError {
            name: "transition".to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    };
    block.declarations.push(Declaration {
        property: PropertyId::TransitionProperty,
        value: DeclaredValue::Value(PropertyValue::TransitionProperty(
            property.unwrap_or(TransitionProperty::All),
        )),
        important,
    });
    block.declarations.push(Declaration {
        property: PropertyId::TransitionDuration,
        value: DeclaredValue::Value(PropertyValue::Duration(duration)),
        important,
    });
}

fn parse_transition_item(input: &str) -> Option<(TransitionProperty, Duration)> {
    let mut property = None;
    let mut duration = None;
    for component in split_components(input) {
        if property.is_none()
            && let Ok(parsed) = component.parse::<TransitionProperty>()
        {
            property = Some(parsed);
        } else if duration.is_none()
            && let Ok(parsed) = component.parse::<Duration>()
        {
            duration = Some(parsed);
        } else {
            return None;
        }
    }
    Some((property.unwrap_or(TransitionProperty::All), duration?))
}

fn merge_transition_properties(
    current: Option<TransitionProperty>,
    next: TransitionProperty,
) -> TransitionProperty {
    let Some(current) = current else {
        return next;
    };
    current.merge(next)
}

fn expand_animation(block: &mut DeclarationBlock, value: &str, important: bool) {
    let mut name = None;
    let mut duration = None;
    let mut timing = None;
    let mut delay = None;
    for component in split_components(value) {
        if duration.is_none()
            && let Ok(parsed) = component.parse::<Duration>()
        {
            duration = Some(parsed);
        } else if delay.is_none()
            && let Ok(parsed) = component.parse::<AnimationDelay>()
        {
            delay = Some(parsed);
        } else if timing.is_none()
            && let Ok(parsed) = component.parse::<TimingFunction>()
        {
            timing = Some(parsed);
        } else if name.is_none()
            && let Ok(parsed) = component.parse::<AnimationName>()
        {
            name = Some(parsed);
        } else {
            block.errors.push(DeclarationError {
                name: "animation".to_owned(),
                value: value.to_owned(),
                kind: DeclarationErrorKind::InvalidValue,
            });
            return;
        }
    }
    let Some(duration) = duration else {
        block.errors.push(DeclarationError {
            name: "animation".to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    };
    let push = |block: &mut DeclarationBlock, property, value| {
        block.declarations.push(Declaration {
            property,
            value: DeclaredValue::Value(value),
            important,
        });
    };
    push(
        block,
        PropertyId::AnimationName,
        PropertyValue::AnimationName(name.unwrap_or(AnimationName::None)),
    );
    push(
        block,
        PropertyId::AnimationDuration,
        PropertyValue::Duration(duration),
    );
    push(
        block,
        PropertyId::AnimationTimingFunction,
        PropertyValue::TimingFunction(timing.unwrap_or(TimingFunction::Linear)),
    );
    push(
        block,
        PropertyId::AnimationDelay,
        PropertyValue::AnimationDelay(delay.unwrap_or(AnimationDelay::ZERO)),
    );
}

/// Expand the bounded `flex` grammar. The omitted values follow CSS
/// Flexbox's shorthand defaults: a lone factor uses shrink `1` and basis
/// `0%`, two factors use `0%` for basis, and a basis-only form uses `1 1`.
fn expand_flex(block: &mut DeclarationBlock, value: &str, important: bool) {
    let parts = split_components(value);
    let parsed = match parts.as_slice() {
        [keyword] if keyword.eq_ignore_ascii_case("none") => {
            Some((FlexFactor::ZERO, FlexFactor::ZERO, FlexBasis::Auto))
        },
        parts => parse_flex_components(parts),
    };
    let Some((grow, shrink, basis)) = parsed else {
        block.errors.push(DeclarationError {
            name: "flex".to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    };
    for (property, value) in [
        (PropertyId::FlexGrow, PropertyValue::FlexFactor(grow)),
        (PropertyId::FlexShrink, PropertyValue::FlexFactor(shrink)),
        (PropertyId::FlexBasis, PropertyValue::FlexBasis(basis)),
    ] {
        block.declarations.push(Declaration {
            property,
            value: DeclaredValue::Value(value),
            important,
        });
    }
}

fn parse_flex_components(parts: &[&str]) -> Option<(FlexFactor, FlexFactor, FlexBasis)> {
    if !(1..=3).contains(&parts.len()) {
        return None;
    }

    let factors = parts
        .iter()
        .map(|part| part.parse::<FlexFactor>())
        .collect::<Result<Vec<_>, _>>();
    if let Ok(factors) = factors {
        return match factors.as_slice() {
            [grow] => Some((*grow, FlexFactor::ONE, flex_zero_basis())),
            [grow, shrink] => Some((*grow, *shrink, flex_zero_basis())),
            [grow, shrink, _] => parse_flex_basis(parts[2]).map(|basis| (*grow, *shrink, basis)),
            _ => None,
        };
    }

    // The basis may appear before, between, or after the factor group. Try
    // the conventional trailing position first. Three entirely numeric tokens
    // were handled above so an interior zero cannot rescue an invalid unitless
    // basis such as `flex: 0 1 4`.
    for basis_index in (0..parts.len()).rev() {
        let Some(basis) = parse_flex_basis(parts[basis_index]) else {
            continue;
        };
        let Ok(factors) = parts
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != basis_index)
            .map(|(_, part)| part.parse::<FlexFactor>())
            .collect::<Result<Vec<_>, _>>()
        else {
            continue;
        };
        match factors.as_slice() {
            [] => return Some((FlexFactor::ONE, FlexFactor::ONE, basis)),
            [grow] => return Some((*grow, FlexFactor::ONE, basis)),
            [grow, shrink] => return Some((*grow, *shrink, basis)),
            _ => {},
        }
    }
    None
}

fn parse_flex_basis(input: &str) -> Option<FlexBasis> {
    input.parse().ok()
}

fn flex_zero_basis() -> FlexBasis {
    "0%".parse()
        .expect("the flex shorthand's zero basis is valid")
}

/// Expand `flex-flow`'s order-independent direction and wrapping keywords.
/// Each component may occur at most once; omitted components use the
/// longhands' initial values.
fn expand_flex_flow(block: &mut DeclarationBlock, value: &str, important: bool) {
    if value.trim().is_empty() {
        block.errors.push(DeclarationError {
            name: "flex-flow".to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    }
    let mut direction = None;
    let mut wrap = None;
    for component in split_components(value) {
        if direction.is_none() {
            direction = component.parse::<FlexDirection>().ok();
            if direction.is_some() {
                continue;
            }
        }
        if wrap.is_none() {
            wrap = component.parse::<FlexWrap>().ok();
            if wrap.is_some() {
                continue;
            }
        }
        block.errors.push(DeclarationError {
            name: "flex-flow".to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    }
    for (property, value) in [
        (
            PropertyId::FlexDirection,
            PropertyValue::FlexDirection(direction.unwrap_or(FlexDirection::Row)),
        ),
        (
            PropertyId::FlexWrap,
            PropertyValue::FlexWrap(wrap.unwrap_or(FlexWrap::NoWrap)),
        ),
    ] {
        block.declarations.push(Declaration {
            property,
            value: DeclaredValue::Value(value),
            important,
        });
    }
}

/// Expand the order-independent `columns` shorthand.  A single component
/// selects its matching longhand and resets the other to its initial `auto`;
/// the deliberately ambiguous single `auto` form resets both longhands.
fn expand_columns(block: &mut DeclarationBlock, value: &str, important: bool) {
    let components = split_components(value);
    if components.is_empty() || components.len() > 2 {
        block.errors.push(DeclarationError {
            name: "columns".to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    }

    let mut width = None;
    let mut count = None;
    let mut auto_count = 0_u8;
    for component in components {
        if component.eq_ignore_ascii_case("auto") {
            auto_count += 1;
            continue;
        } else if component.parse::<ColumnWidth>().is_ok() && width.is_none() {
            width = Some(component);
            continue;
        } else if component.parse::<ColumnCount>().is_ok() && count.is_none() {
            count = Some(component);
            continue;
        }
        block.errors.push(DeclarationError {
            name: "columns".to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    }

    if auto_count > 2 || (auto_count == 2 && (width.is_some() || count.is_some())) {
        block.errors.push(DeclarationError {
            name: "columns".to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    }
    if auto_count > 0 {
        if width.is_none() {
            width = Some("auto");
        } else if count.is_none() {
            count = Some("auto");
        }
    }

    let (width, count) = match (width, count) {
        (Some(width), Some(count)) => (width, count),
        (Some(width), None) => (width, "auto"),
        (None, Some(count)) => ("auto", count),
        (None, None) => unreachable!("non-empty columns shorthand has a component"),
    };
    push_longhand(block, "column-width", width, important);
    push_longhand(block, "column-count", count, important);
}

fn expand_border(
    block: &mut DeclarationBlock,
    shorthand: ShorthandId,
    value: &str,
    important: bool,
) {
    let mut width = None;
    let mut style = None;
    let mut color = None;
    for component in split_components(value) {
        if width.is_none() {
            width = component.parse::<BorderWidth>().ok();
            if width.is_some() {
                continue;
            }
        }
        if style.is_none() {
            style = component.parse::<BorderStyle>().ok();
            if style.is_some() {
                continue;
            }
        }
        if color.is_none() {
            color = component.parse::<ComputedColor>().ok();
            if color.is_some() {
                continue;
            }
        }
        block.errors.push(DeclarationError {
            name: shorthand.metadata().name.to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    }
    let width = width.unwrap_or(BorderWidth::Medium);
    let style = style.unwrap_or(BorderStyle::None);
    let color = color.unwrap_or(ComputedColor::CURRENT_COLOR);
    for &property in shorthand.metadata().longhands {
        let value = match property.metadata().value_type {
            crate::ValueType::BorderWidth => PropertyValue::BorderWidth(width),
            crate::ValueType::BorderStyle => PropertyValue::BorderStyle(style),
            crate::ValueType::Color => PropertyValue::Color(color.clone()),
            _ => unreachable!("validated border longhand family"),
        };
        block.declarations.push(Declaration {
            property,
            value: DeclaredValue::Value(value),
            important,
        });
    }
}

struct BackgroundLayer {
    color: ComputedColor,
    image: BackgroundImage,
    position: BackgroundPosition,
    size: BackgroundSize,
    repeat: BackgroundRepeat,
    attachment: BackgroundAttachment,
    origin: BackgroundBox,
    clip: BackgroundBox,
}

fn background_components(value: &str) -> Vec<&str> {
    let mut components = Vec::new();
    let mut start = None;
    let mut depth = 0_u32;
    let mut quote = None;
    for (index, ch) in value.char_indices() {
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                start.get_or_insert(index);
                quote = Some(ch);
            },
            '(' => {
                start.get_or_insert(index);
                depth += 1;
            },
            ')' => depth = depth.saturating_sub(1),
            '/' if depth == 0 => {
                if let Some(component_start) = start.take() {
                    components.push(&value[component_start..index]);
                }
                components.push("/");
            },
            _ if ch.is_ascii_whitespace() && depth == 0 => {
                if let Some(component_start) = start.take() {
                    components.push(&value[component_start..index]);
                }
            },
            _ => {
                start.get_or_insert(index);
            },
        }
    }
    if let Some(component_start) = start {
        components.push(&value[component_start..]);
    }
    components
}

fn is_background_image_component(component: &str) -> bool {
    let lower = component.to_ascii_lowercase();
    lower == "none" || lower.starts_with("url(") || lower.contains("-gradient(")
}

fn is_background_position_component(component: &str) -> bool {
    matches!(
        component.to_ascii_lowercase().as_str(),
        "left" | "right" | "top" | "bottom" | "center"
    ) || component.parse::<crate::values::LengthPercentage>().is_ok()
}

fn is_background_size_component(component: &str) -> bool {
    component
        .parse::<crate::values::BackgroundSizeComponent>()
        .is_ok()
}

fn parse_background_layer(value: &str) -> Option<BackgroundLayer> {
    if split_top_level(value, ',').len() != 1 {
        return None;
    }
    let components = background_components(value.trim());
    if components.is_empty() {
        return None;
    }
    let mut color = None;
    let mut image = None;
    let mut position = None;
    let mut size = None;
    let mut repeat = None;
    let mut attachment = None;
    let mut boxes = Vec::new();
    let mut index = 0;
    while index < components.len() {
        let component = components[index];
        if component == "/" {
            return None;
        }
        if image.is_none() && is_background_image_component(component) {
            image = Some(component.parse::<BackgroundImage>().ok()?);
            index += 1;
            continue;
        }
        if repeat.is_none() && component.parse::<BackgroundRepeat>().is_ok() {
            let mut end = index + 1;
            if end < components.len()
                && component.parse::<crate::values::RepeatStyle>().is_ok()
                && components[end]
                    .parse::<crate::values::RepeatStyle>()
                    .is_ok()
            {
                end += 1;
            }
            repeat = Some(components[index..end].join(" ").parse().ok()?);
            index = end;
            continue;
        }
        if attachment.is_none()
            && let Ok(parsed) = component.parse::<BackgroundAttachment>()
        {
            attachment = Some(parsed);
            index += 1;
            continue;
        }
        if boxes.len() < 2
            && let Ok(parsed) = component.parse::<BackgroundBox>()
        {
            boxes.push(parsed);
            index += 1;
            continue;
        }
        if position.is_none() && is_background_position_component(component) {
            let mut end = index + 1;
            while end < components.len()
                && end - index < 4
                && is_background_position_component(components[end])
            {
                end += 1;
            }
            let parsed = loop {
                if let Ok(parsed) = components[index..end]
                    .join(" ")
                    .parse::<BackgroundPosition>()
                {
                    break Some((parsed, end));
                }
                if end - index == 1 {
                    break None;
                }
                end -= 1;
            };
            let (parsed, run_end) = parsed?;
            position = Some(parsed);
            index = run_end;
            if index < components.len() && components[index] == "/" {
                let start = index + 1;
                let mut end = start;
                while end < components.len()
                    && end - start < 2
                    && is_background_size_component(components[end])
                {
                    end += 1;
                }
                let candidate = if end == start {
                    components.get(start).copied()?.to_owned()
                } else {
                    components[start..end].join(" ")
                };
                size = Some(candidate.parse::<BackgroundSize>().ok()?);
                index = if end == start { start + 1 } else { end };
            }
            continue;
        }
        if color.is_none()
            && let Ok(parsed) = component.parse::<ComputedColor>()
        {
            color = Some(parsed);
            index += 1;
            continue;
        }
        return None;
    }
    Some(BackgroundLayer {
        color: color.unwrap_or(ComputedColor::TRANSPARENT),
        image: image.unwrap_or(BackgroundImage::None),
        position: position.unwrap_or(BackgroundPosition::ZERO),
        size: size.unwrap_or(BackgroundSize::AUTO),
        repeat: repeat.unwrap_or(BackgroundRepeat::REPEAT),
        attachment: attachment.unwrap_or(BackgroundAttachment::Scroll),
        origin: boxes.first().copied().unwrap_or(BackgroundBox::PaddingBox),
        clip: boxes
            .get(1)
            .or(boxes.first())
            .copied()
            .unwrap_or(BackgroundBox::BorderBox),
    })
}

fn expand_background(block: &mut DeclarationBlock, value: &str, important: bool) {
    let Some(layer) = parse_background_layer(value) else {
        block.errors.push(DeclarationError {
            name: "background".to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    };
    for (property, value) in [
        (
            PropertyId::BackgroundColor,
            PropertyValue::Color(layer.color),
        ),
        (
            PropertyId::BackgroundImage,
            PropertyValue::BackgroundImage(layer.image),
        ),
        (
            PropertyId::BackgroundPosition,
            PropertyValue::BackgroundPosition(layer.position),
        ),
        (
            PropertyId::BackgroundSize,
            PropertyValue::BackgroundSize(layer.size),
        ),
        (
            PropertyId::BackgroundRepeat,
            PropertyValue::BackgroundRepeat(layer.repeat),
        ),
        (
            PropertyId::BackgroundAttachment,
            PropertyValue::BackgroundAttachment(layer.attachment),
        ),
        (
            PropertyId::BackgroundOrigin,
            PropertyValue::BackgroundBox(layer.origin),
        ),
        (
            PropertyId::BackgroundClip,
            PropertyValue::BackgroundBox(layer.clip),
        ),
    ] {
        block.declarations.push(Declaration {
            property,
            value: DeclaredValue::Value(value),
            important,
        });
    }
}

fn expand_white_space(block: &mut DeclarationBlock, value: &str, important: bool) {
    let (collapse, wrap) = match value.trim().to_ascii_lowercase().as_str() {
        "normal" => ("collapse", "wrap"),
        "nowrap" => ("collapse", "nowrap"),
        "pre" => ("preserve", "nowrap"),
        "pre-wrap" => ("preserve", "wrap"),
        "pre-line" => ("preserve-breaks", "wrap"),
        _ => {
            block.errors.push(DeclarationError {
                name: "white-space".to_owned(),
                value: value.to_owned(),
                kind: DeclarationErrorKind::InvalidValue,
            });
            return;
        },
    };
    push_longhand(block, "white-space-collapse", collapse, important);
    push_longhand(block, "text-wrap-mode", wrap, important);
}

fn expand_font(block: &mut DeclarationBlock, value: &str, important: bool) {
    let components = split_components(value);
    let mut style = FontStyle::Normal;
    let mut weight = FontWeight::Normal;
    let mut size = None;
    let mut line_height = LineHeight::Normal;
    let mut family_start = None;
    let mut index = 0;

    while index < components.len() {
        let component = components[index];
        if let Some((size_value, line_value)) = component.split_once('/') {
            let Ok(parsed_size) = size_value.parse::<FontSize>() else {
                break;
            };
            let Ok(parsed_line_height) = line_value.parse::<LineHeight>() else {
                break;
            };
            size = Some(parsed_size);
            line_height = parsed_line_height;
            family_start = Some(index + 1);
            break;
        }
        if component == "/" {
            break;
        }
        if let Ok(parsed_size) = component.parse::<FontSize>() {
            size = Some(parsed_size);
            if components.get(index + 1) == Some(&"/") {
                let Some(line_value) = components.get(index + 2) else {
                    break;
                };
                let Ok(parsed_line_height) = line_value.parse::<LineHeight>() else {
                    break;
                };
                line_height = parsed_line_height;
                family_start = Some(index + 3);
            } else {
                family_start = Some(index + 1);
            }
            break;
        }
        if let Ok(parsed_style) = component.parse::<FontStyle>() {
            style = parsed_style;
        } else if let Ok(parsed_weight) = component.parse::<FontWeight>() {
            weight = parsed_weight;
        } else {
            break;
        }
        index += 1;
    }

    let Some(size) = size else {
        block.errors.push(DeclarationError {
            name: "font".to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    };
    let Some(family_start) = family_start else {
        block.errors.push(DeclarationError {
            name: "font".to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    };
    let family_value = components[family_start..].join(" ");
    let Ok(family) = family_value.parse::<FontFamily>() else {
        block.errors.push(DeclarationError {
            name: "font".to_owned(),
            value: value.to_owned(),
            kind: DeclarationErrorKind::InvalidValue,
        });
        return;
    };
    for (property, value) in [
        (PropertyId::FontStyle, PropertyValue::FontStyle(style)),
        (
            PropertyId::FontVariantLigatures,
            PropertyValue::FontVariantLigatures(FontVariantLigatures::NORMAL),
        ),
        (PropertyId::FontWeight, PropertyValue::FontWeight(weight)),
        (PropertyId::FontSize, PropertyValue::FontSize(size)),
        (
            PropertyId::LineHeight,
            PropertyValue::LineHeight(line_height),
        ),
        (PropertyId::FontFamily, PropertyValue::FontFamily(family)),
        (
            PropertyId::FontFeatureSettings,
            PropertyValue::FontFeatureSettings(FontFeatureSettings::Normal),
        ),
    ] {
        block.declarations.push(Declaration {
            property,
            value: DeclaredValue::Value(value),
            important,
        });
    }
}

fn expand_css_wide_shorthand(
    block: &mut DeclarationBlock,
    shorthand: ShorthandId,
    keyword: &str,
    important: bool,
) {
    for &property in shorthand.metadata().longhands {
        block.declarations.push(Declaration {
            property,
            value: DeclaredValue::parse(property, keyword).expect("CSS-wide keyword"),
            important,
        });
    }
}

/// Parse a style-rule declaration block. Invalid declarations are retained as
/// diagnostics while valid declarations continue through CSS error recovery.
pub fn parse_declaration_block(input: &str) -> DeclarationBlock {
    let clean = without_comments(input);
    let mut block = DeclarationBlock::default();
    for raw in split_top_level(&clean, ';') {
        let declaration = raw.trim();
        if declaration.is_empty() {
            continue;
        }
        let Some(colon) = split_top_level(declaration, ':')
            .first()
            .map(|head| head.len())
        else {
            continue;
        };
        if colon == declaration.len() {
            block.errors.push(DeclarationError {
                name: declaration.to_owned(),
                value: String::new(),
                kind: DeclarationErrorKind::MalformedDeclaration,
            });
            continue;
        }
        let raw_name = declaration[..colon].trim();
        let (value, important) = strip_important(&declaration[colon + 1..]);
        if let Some(custom_tail) = raw_name.strip_prefix("--") {
            if custom_tail.is_empty() {
                block.errors.push(DeclarationError {
                    name: raw_name.to_owned(),
                    value: value.to_owned(),
                    kind: DeclarationErrorKind::MalformedDeclaration,
                });
                continue;
            }
            // Custom property names stay case-sensitive; CSS-wide keywords
            // in the value position keep their usual meaning.
            let declared = match value.trim().to_ascii_lowercase().as_str() {
                "initial" => CustomDeclaredValue::Initial,
                "inherit" => CustomDeclaredValue::Inherit,
                "unset" => CustomDeclaredValue::Unset,
                _ => CustomDeclaredValue::Value(value.trim().to_owned()),
            };
            block.custom.push(CustomDeclaration {
                name: raw_name.to_owned(),
                value: declared,
                important,
            });
            continue;
        }
        let name = raw_name.to_ascii_lowercase();
        if push_longhand(&mut block, &name, value, important) {
            continue;
        }
        if let Some(shorthand) = ShorthandId::from_css_name(&name)
            && contains_var(value)
        {
            // The fork's WithVariables shape: every expanded longhand
            // carries the raw shorthand value and re-expands after
            // substitution at computed-value time.
            for longhand in shorthand.metadata().longhands {
                block.declarations.push(Declaration {
                    property: *longhand,
                    value: DeclaredValue::Pending(PendingSubstitution {
                        raw: value.to_owned(),
                        from_shorthand: Some(shorthand),
                    }),
                    important,
                });
            }
            continue;
        }
        let Some(shorthand) = ShorthandId::from_css_name(&name) else {
            let kind = if crate::unimplemented_longhand(&name).is_some()
                || crate::unimplemented_shorthand(&name).is_some()
            {
                DeclarationErrorKind::KnownUnimplemented
            } else {
                DeclarationErrorKind::UnknownProperty
            };
            block.errors.push(DeclarationError {
                name,
                value: value.to_owned(),
                kind,
            });
            continue;
        };
        if matches!(
            value.to_ascii_lowercase().as_str(),
            "initial" | "inherit" | "unset"
        ) {
            expand_css_wide_shorthand(&mut block, shorthand, value, important);
        } else if expand_box_shorthand(&mut block, shorthand, value, important) {
        } else if shorthand == ShorthandId::Background {
            expand_background(&mut block, value, important);
        } else if shorthand == ShorthandId::Transition {
            expand_transition(&mut block, value, important);
        } else if shorthand == ShorthandId::Animation {
            expand_animation(&mut block, value, important);
        } else if matches!(
            shorthand,
            ShorthandId::Border
                | ShorthandId::BorderTop
                | ShorthandId::BorderRight
                | ShorthandId::BorderBottom
                | ShorthandId::BorderLeft
        ) {
            expand_border(&mut block, shorthand, value, important);
        } else if matches!(
            shorthand,
            ShorthandId::GridRow | ShorthandId::GridColumn | ShorthandId::GridArea
        ) {
            expand_grid_placement(&mut block, shorthand, value, important);
        } else if matches!(shorthand, ShorthandId::Grid | ShorthandId::GridTemplate) {
            expand_grid_template(&mut block, shorthand, value, important);
        } else if shorthand == ShorthandId::PlaceItems {
            expand_place_items(&mut block, value, important);
        } else if shorthand == ShorthandId::WhiteSpace {
            expand_white_space(&mut block, value, important);
        } else if shorthand == ShorthandId::Font {
            expand_font(&mut block, value, important);
        } else if shorthand == ShorthandId::Flex {
            expand_flex(&mut block, value, important);
        } else if shorthand == ShorthandId::FlexFlow {
            expand_flex_flow(&mut block, value, important);
        } else if shorthand == ShorthandId::Columns {
            expand_columns(&mut block, value, important);
        }
    }
    block
}

/// Stylesheet origin.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Origin {
    UserAgent,
    User,
    /// HTML presentational hints. These are author-adjacent declarations with
    /// their own Cascade Level 5 origin: normal author declarations override
    /// them, while hints still override normal user and UA declarations.
    AuthorPresentationalHint,
    Author,
}

/// Layer position inside one origin. Layer numbers increase in declaration
/// order. Unlayered normal declarations outrank layered normal declarations;
/// important declarations reverse that order.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CascadeLayer {
    Layer(u32),
    Unlayered,
}

/// Packed selector specificity. The selectors crate supplies this encoding.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Specificity(pub u32);

impl Specificity {
    pub const INLINE: Self = Self(u32::MAX);
}

/// One declaration whose selector and media condition already matched.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchedDeclaration {
    pub declaration: Declaration,
    pub origin: Origin,
    pub layer: CascadeLayer,
    pub specificity: Specificity,
    pub source_order: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Priority {
    cascade_level: u8,
    layer: u32,
    specificity: Specificity,
    source_order: u64,
}

impl Priority {
    fn new(declaration: &MatchedDeclaration) -> Self {
        Self::from_parts(
            declaration.declaration.important,
            declaration.origin,
            declaration.layer,
            declaration.specificity,
            declaration.source_order,
        )
    }

    fn from_parts(
        important: bool,
        origin: Origin,
        layer: CascadeLayer,
        specificity: Specificity,
        source_order: u64,
    ) -> Self {
        let cascade_level = match (important, origin) {
            (false, Origin::UserAgent) => 0,
            (false, Origin::User) => 1,
            (false, Origin::AuthorPresentationalHint) => 2,
            (false, Origin::Author) => 3,
            // Presentational hints are injected by the host contract without
            // `!important`; keep this rank defensive if an invalid caller
            // constructs one directly.
            (true, Origin::AuthorPresentationalHint) => 3,
            (true, Origin::Author) => 4,
            (true, Origin::User) => 5,
            (true, Origin::UserAgent) => 6,
        };
        let layer = match (important, layer) {
            (false, CascadeLayer::Layer(order)) => order,
            (false, CascadeLayer::Unlayered) => u32::MAX,
            (true, CascadeLayer::Unlayered) => 0,
            (true, CascadeLayer::Layer(order)) => u32::MAX - order,
        };
        Self {
            cascade_level,
            layer,
            specificity,
            source_order,
        }
    }
}

impl Ord for Priority {
    fn cmp(&self, other: &Self) -> Ordering {
        (
            self.cascade_level,
            self.layer,
            self.specificity,
            self.source_order,
        )
            .cmp(&(
                other.cascade_level,
                other.layer,
                other.specificity,
                other.source_order,
            ))
    }
}

impl PartialOrd for Priority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One matched `--name` declaration with its cascade coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchedCustomDeclaration {
    pub declaration: CustomDeclaration,
    pub origin: Origin,
    pub layer: CascadeLayer,
    pub specificity: Specificity,
    pub source_order: u64,
}

/// Resolve a set of already-matched declarations into one concrete style.
/// Declarations that used `var()` resolve against an empty custom map here;
/// use [`cascade_with_custom`] to thread real custom properties.
pub fn cascade(
    parent: Option<&ComputedValues>,
    declarations: impl IntoIterator<Item = MatchedDeclaration>,
) -> ComputedValues {
    cascade_with_custom(parent, None, declarations, std::iter::empty()).0
}

/// Cascade one element using an explicit host preference and system palette.
pub fn cascade_with_color_context(
    parent: Option<&ComputedValues>,
    declarations: impl IntoIterator<Item = MatchedDeclaration>,
    color_context: ColorComputeContext,
) -> ComputedValues {
    cascade_with_custom_context(
        parent,
        None,
        declarations,
        std::iter::empty(),
        color_context,
    )
    .0
}

/// Resolve matched longhand and custom declarations into one concrete
/// style plus the element's computed custom-property map. The map starts
/// from the parent's (custom properties inherit wholesale), applies this
/// element's winners with the same priority rules as longhands, and then
/// substitutes every pending `var()` declaration; a substitution or
/// reparse failure is invalid at computed-value time and behaves as
/// `unset`, per css-variables-1.
pub fn cascade_with_custom(
    parent: Option<&ComputedValues>,
    parent_custom: Option<&CustomProperties>,
    declarations: impl IntoIterator<Item = MatchedDeclaration>,
    custom_declarations: impl IntoIterator<Item = MatchedCustomDeclaration>,
) -> (ComputedValues, CustomProperties) {
    cascade_with_custom_context(
        parent,
        parent_custom,
        declarations,
        custom_declarations,
        ColorComputeContext::default(),
    )
}

/// Cascade one element with the host color facts needed at computed-value
/// time. Callers that recurse must pass their already-computed parent style.
pub fn cascade_with_custom_context(
    parent: Option<&ComputedValues>,
    parent_custom: Option<&CustomProperties>,
    declarations: impl IntoIterator<Item = MatchedDeclaration>,
    custom_declarations: impl IntoIterator<Item = MatchedCustomDeclaration>,
    color_context: ColorComputeContext,
) -> (ComputedValues, CustomProperties) {
    let mut custom_winners: std::collections::BTreeMap<String, (Priority, CustomDeclaredValue)> =
        std::collections::BTreeMap::new();
    for matched in custom_declarations {
        let priority = Priority::from_parts(
            matched.declaration.important,
            matched.origin,
            matched.layer,
            matched.specificity,
            matched.source_order,
        );
        let entry = custom_winners.entry(matched.declaration.name);
        match entry {
            std::collections::btree_map::Entry::Vacant(vacant) => {
                vacant.insert((priority, matched.declaration.value));
            },
            std::collections::btree_map::Entry::Occupied(mut occupied) => {
                if priority >= occupied.get().0 {
                    occupied.insert((priority, matched.declaration.value));
                }
            },
        }
    }
    let custom = crate::custom::resolve_custom_map(
        parent_custom,
        custom_winners
            .into_iter()
            .map(|(name, (_, value))| (name, value)),
    );

    let mut winners = (0..PropertyId::ALL.len())
        .map(|_| None)
        .collect::<Vec<Option<(Priority, DeclaredValue)>>>();
    for matched in declarations {
        let index = matched.declaration.property as usize;
        let priority = Priority::new(&matched);
        let replace = winners[index]
            .as_ref()
            .is_none_or(|(current, _)| priority >= *current);
        if replace {
            winners[index] = Some((priority, matched.declaration.value));
        }
    }

    let initial = ComputedValues::default();
    let mut result = parent.map(ComputedValues::for_child).unwrap_or_default();
    for (index, winner) in winners.into_iter().enumerate() {
        let Some((_, value)) = winner else {
            continue;
        };
        let property = PropertyId::ALL[index];
        let value = match value {
            DeclaredValue::Pending(pending) => resolve_pending(&pending, property, &custom),
            other => other,
        };
        match value {
            DeclaredValue::Value(value) => {
                result
                    .set(property, value)
                    .unwrap_or_else(|_| panic!("generated value type mismatch for {property:?}"));
            },
            DeclaredValue::Initial => result.copy_property_from(property, &initial),
            DeclaredValue::Inherit => {
                result.copy_property_from(property, parent.unwrap_or(&initial));
            },
            DeclaredValue::Unset => {
                if property.metadata().inherited {
                    result.copy_property_from(property, parent.unwrap_or(&initial));
                } else {
                    result.copy_property_from(property, &initial);
                }
            },
            DeclaredValue::Pending(_) => unreachable!("pending values resolve above"),
        }
    }
    resolve_computed_colors(&mut result, parent, color_context);
    (result, custom)
}

fn resolve_computed_colors(
    result: &mut ComputedValues,
    parent: Option<&ComputedValues>,
    color_context: ColorComputeContext,
) {
    let scheme = result
        .color_scheme
        .used_scheme(color_context.preferred_scheme);
    let initial_foreground = color_context.palette.get(scheme, SystemColor::CanvasText);
    let inherited_foreground = parent
        .map(|style| {
            style.color.resolve_used(UsedColorContext::with_palette(
                initial_foreground,
                color_context.palette,
                scheme,
            ))
        })
        .unwrap_or(initial_foreground);
    let used = UsedColorContext::with_palette(inherited_foreground, color_context.palette, scheme);

    // `color` is special: its `currentcolor` resolves against the inherited
    // foreground before this new foreground becomes available to descendants.
    result.color = ComputedColor::Absolute(result.color.resolve_used(used));

    for &property in PropertyId::ALL {
        if property == PropertyId::Color {
            continue;
        }
        let value = resolve_property_system_colors(result.get(property), used);
        result
            .set(property, value)
            .expect("generated property read and write types agree");
    }
}

fn resolve_property_system_colors(
    value: PropertyValue,
    context: UsedColorContext,
) -> PropertyValue {
    match value {
        PropertyValue::Color(color) => PropertyValue::Color(color.resolve_system_colors(context)),
        PropertyValue::BackgroundImage(BackgroundImage::LinearGradient { from, to }) => {
            PropertyValue::BackgroundImage(BackgroundImage::LinearGradient {
                from: from.resolve_system_colors(context),
                to: to.resolve_system_colors(context),
            })
        },
        PropertyValue::BoxShadow(BoxShadow::Value(mut shadow)) => {
            shadow.color = shadow.color.resolve_system_colors(context);
            PropertyValue::BoxShadow(BoxShadow::Value(shadow))
        },
        value => value,
    }
}

/// Substitute and parse one pending declaration. Any failure is invalid at
/// computed-value time, which css-variables-1 defines as `unset`.
fn resolve_pending(
    pending: &PendingSubstitution,
    property: PropertyId,
    custom: &CustomProperties,
) -> DeclaredValue {
    let Ok(substituted) = substitute(&pending.raw, custom) else {
        return DeclaredValue::Unset;
    };
    match pending.from_shorthand {
        None => DeclaredValue::parse(property, &substituted).unwrap_or(DeclaredValue::Unset),
        Some(shorthand) => {
            let reparsed =
                parse_declaration_block(&format!("{}: {}", shorthand.metadata().name, substituted));
            match reparsed
                .declarations
                .into_iter()
                .find(|declaration| declaration.property == property)
            {
                Some(Declaration {
                    value: DeclaredValue::Pending(_),
                    ..
                })
                | None => DeclaredValue::Unset,
                Some(declaration) => declaration.value,
            }
        },
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UserAgent => "user-agent",
            Self::User => "user",
            Self::AuthorPresentationalHint => "author-presentational-hint",
            Self::Author => "author",
        })
    }
}
