// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Visual transform values: opacity, clip paths, rotate and scale, the
//! transform list and its functions, and box shadows.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Opacity(f32);

impl Opacity {
    pub const ONE: Self = Self(1.0);

    pub const fn from_value(value: f32) -> Self {
        Self(value.clamp(0.0, 1.0))
    }

    pub const fn value(self) -> f32 {
        self.0
    }
}

impl FromStr for Opacity {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        let value = if let Some(percentage) = input.strip_suffix('%') {
            percentage.trim().parse::<f32>().map(|value| value / 100.0)
        } else {
            input.parse::<f32>()
        }
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| ParseError::expected("a finite opacity number or percentage"))?;
        Ok(Self(value.clamp(0.0, 1.0)))
    }
}

impl fmt::Display for Opacity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&format_number(self.0))
    }
}

/// The bounded 2D individual `rotate` property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rotate {
    None,
    Angle(f32),
    /// An angle expression whose bases are not all constant, retained until
    /// cascade supplies the element's tree context.
    Deferred(MathLengthPercentage),
}

impl Rotate {
    pub const fn radians(self) -> Option<f32> {
        match self {
            Self::None | Self::Deferred(_) => None,
            Self::Angle(value) => Some(value),
        }
    }

    pub(in crate::values) fn resolve_math(self, environment: RelativeLengthEnvironment) -> Self {
        match self {
            Self::Deferred(math) => {
                let resolved = math.resolve_relative(environment);
                resolved
                    .resolved_px()
                    .map_or(Self::Deferred(resolved), Self::Angle)
            },
            value => value,
        }
    }
}

impl FromStr for Rotate {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        if input.eq_ignore_ascii_case("none") {
            return Ok(Self::None);
        }
        parse_angle(input)
            .or_else(|_| crate::values::calc::parse_angle(input))
            .map(Self::Angle)
            .or_else(|error| {
                crate::values::calc::parse_angle_math(input)
                    .map(Self::Deferred)
                    .map_err(|_| error)
            })
    }
}

impl fmt::Display for Rotate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("none"),
            Self::Angle(value) => write!(formatter, "{}rad", format_number(*value)),
            Self::Deferred(math) => math.fmt(formatter),
        }
    }
}

/// The bounded uniform individual `scale` property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scale {
    None,
    Uniform(f32),
    /// The [`Rotate::Deferred`] twin for number-valued expressions.
    Deferred(MathLengthPercentage),
}

impl Scale {
    pub const fn factor(self) -> Option<f32> {
        match self {
            Self::None | Self::Deferred(_) => None,
            Self::Uniform(value) => Some(value),
        }
    }

    pub(in crate::values) fn resolve_math(self, environment: RelativeLengthEnvironment) -> Self {
        match self {
            Self::Deferred(math) => {
                let resolved = math.resolve_relative(environment);
                resolved
                    .resolved_px()
                    .map_or(Self::Deferred(resolved), Self::Uniform)
            },
            value => value,
        }
    }
}

impl FromStr for Scale {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        if input.eq_ignore_ascii_case("none") {
            return Ok(Self::None);
        }
        let value = input
            .strip_suffix('%')
            .and_then(|value| value.trim().parse::<f32>().ok())
            .map(|value| value / 100.0)
            .or_else(|| input.parse::<f32>().ok())
            .filter(|value| value.is_finite())
            .map(Ok)
            .unwrap_or_else(|| crate::values::calc::parse_number(input));
        match value {
            Ok(value) => Ok(Self::Uniform(value)),
            Err(error) => crate::values::calc::parse_number_math(input)
                .map(Self::Deferred)
                .map_err(|_| error),
        }
    }
}

impl fmt::Display for Scale {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("none"),
            Self::Uniform(value) => formatter.write_str(&format_number(*value)),
            Self::Deferred(math) => math.fmt(formatter),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Transform {
    None,
    Functions(Vec<TransformFunction>),
}

/// The bounded `clip-path` subset used by Cambium's tile geometry.
///
/// A polygon coordinate is a regular Livery length-percentage. The cascade
/// resolves its environment-relative terms before paint supplies the element
/// box as the percentage basis.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipPath {
    None,
    Polygon(Vec<(LengthPercentage, LengthPercentage)>),
}

impl ClipPath {
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Resolve the polygon against its border-box dimensions for the shared
    /// paint and hit-testing paths. `None` deliberately has no geometry.
    pub fn polygon_points(&self, width: f32, height: f32) -> Option<Vec<(f32, f32)>> {
        let Self::Polygon(points) = self else {
            return None;
        };
        Some(
            points
                .iter()
                .map(|(x, y)| (x.to_px(16.0, 16.0, width), y.to_px(16.0, 16.0, height)))
                .collect(),
        )
    }
}

impl FromStr for ClipPath {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        if input.eq_ignore_ascii_case("none") {
            return Ok(Self::None);
        }
        let open = input
            .find('(')
            .ok_or_else(|| ParseError::expected("none or polygon()"))?;
        if !input[..open].trim().eq_ignore_ascii_case("polygon") || !input.ends_with(')') {
            return Err(ParseError::expected("none or polygon()"));
        }
        let arguments = &input[open + 1..input.len() - 1];
        let mut points = Vec::new();
        for point in polygon_arguments(arguments)? {
            let coordinates = shadow_components(point);
            let [x, y] = coordinates.as_slice() else {
                return Err(ParseError::expected("two polygon coordinates"));
            };
            points.push((x.parse()?, y.parse()?));
        }
        if points.len() < 3 {
            return Err(ParseError::expected("at least three polygon points"));
        }
        Ok(Self::Polygon(points))
    }
}

fn polygon_arguments(input: &str) -> Result<Vec<&str>, ParseError> {
    let mut arguments = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    for (index, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            ')' => return Err(ParseError::expected("closed polygon coordinates")),
            ',' if depth == 0 => {
                let argument = input[start..index].trim();
                if argument.is_empty() {
                    return Err(ParseError::expected("a polygon coordinate pair"));
                }
                arguments.push(argument);
                start = index + 1;
            },
            _ => {},
        }
    }
    if depth != 0 {
        return Err(ParseError::expected("closed polygon coordinates"));
    }
    let argument = input[start..].trim();
    if argument.is_empty() {
        return Err(ParseError::expected("a polygon coordinate pair"));
    }
    arguments.push(argument);
    Ok(arguments)
}

impl fmt::Display for ClipPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("none"),
            Self::Polygon(points) => {
                formatter.write_str("polygon(")?;
                for (index, (x, y)) in points.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{x} {y}")?;
                }
                formatter.write_str(")")
            },
        }
    }
}

/// A bounded single-layer CSS box shadow.
#[derive(Clone, Debug, PartialEq)]
pub enum BoxShadow {
    None,
    Value(BoxShadowValue),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoxShadowValue {
    pub inset: bool,
    pub offset_x: Length,
    pub offset_y: Length,
    pub blur_radius: Length,
    pub spread_radius: Length,
    pub color: ComputedColor,
}

impl BoxShadow {
    /// Interpolate the bounded single-shadow form used by the retained paint
    /// lane. Matching length units and inset mode are required; `none`, mixed
    /// units, and mode changes stay discrete until the shadow-list ratchet.
    pub fn interpolate(&self, other: &Self, progress: f32) -> Self {
        let progress = progress.clamp(0.0, 1.0);
        let value = match (self, other) {
            (Self::Value(from), Self::Value(to)) if from.inset == to.inset => {
                interpolate_box_shadow_value(from, to, progress).map(Self::Value)
            },
            _ => None,
        };
        value.unwrap_or_else(|| {
            if progress < 0.5 {
                self.clone()
            } else {
                other.clone()
            }
        })
    }

    /// Interpolate a matching shadow after resolving the two color endpoints
    /// under their respective used-value contexts.
    pub fn interpolate_used(
        &self,
        other: &Self,
        from_context: UsedColorContext,
        to_context: UsedColorContext,
        progress: f32,
    ) -> Self {
        let progress = progress.clamp(0.0, 1.0);
        let value = match (self, other) {
            (Self::Value(from), Self::Value(to)) if from.inset == to.inset => {
                interpolate_box_shadow_value(from, to, progress).map(|mut value| {
                    value.color =
                        from.color
                            .interpolate_used(&to.color, from_context, to_context, progress);
                    Self::Value(value)
                })
            },
            _ => None,
        };
        value.unwrap_or_else(|| {
            if progress < 0.5 {
                self.clone()
            } else {
                other.clone()
            }
        })
    }
}

fn interpolate_box_shadow_value(
    from: &BoxShadowValue,
    to: &BoxShadowValue,
    progress: f32,
) -> Option<BoxShadowValue> {
    Some(BoxShadowValue {
        inset: from.inset,
        offset_x: interpolate_length(from.offset_x, to.offset_x, progress)?,
        offset_y: interpolate_length(from.offset_y, to.offset_y, progress)?,
        blur_radius: interpolate_length(from.blur_radius, to.blur_radius, progress)?,
        spread_radius: interpolate_length(from.spread_radius, to.spread_radius, progress)?,
        color: from.color.interpolate(&to.color, progress),
    })
}

impl FromStr for BoxShadow {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        if input.eq_ignore_ascii_case("none") {
            return Ok(Self::None);
        }
        let mut inset = false;
        let mut color = None;
        let mut lengths = Vec::new();
        for component in shadow_components(input) {
            if component.eq_ignore_ascii_case("inset") {
                if inset {
                    return Err(ParseError::expected("one inset box-shadow keyword"));
                }
                inset = true;
            } else if let Ok(value) = component.parse::<ComputedColor>() {
                if color.replace(value).is_some() {
                    return Err(ParseError::expected("one box-shadow color"));
                }
            } else if let Ok(value) = component.parse::<Length>() {
                lengths.push(value);
            } else {
                return Err(ParseError::expected("a bounded box-shadow component"));
            }
        }
        if !(2..=4).contains(&lengths.len()) {
            return Err(ParseError::expected("two through four box-shadow lengths"));
        }
        Ok(Self::Value(BoxShadowValue {
            inset,
            offset_x: lengths[0],
            offset_y: lengths[1],
            blur_radius: lengths.get(2).copied().unwrap_or(Length::ZERO),
            spread_radius: lengths.get(3).copied().unwrap_or(Length::ZERO),
            color: color.unwrap_or(ComputedColor::CURRENT_COLOR),
        }))
    }
}

impl fmt::Display for BoxShadow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("none"),
            Self::Value(value) => {
                write!(
                    formatter,
                    "{} {} {} {} {}",
                    value.offset_x,
                    value.offset_y,
                    value.blur_radius,
                    value.spread_radius,
                    value.color
                )?;
                if value.inset {
                    formatter.write_str(" inset")?;
                }
                Ok(())
            },
        }
    }
}

pub(crate) fn shadow_components(input: &str) -> Vec<&str> {
    let mut components = Vec::new();
    let mut start = None;
    let mut depth = 0_u32;
    for (index, ch) in input.char_indices() {
        match ch {
            '(' => {
                start.get_or_insert(index);
                depth += 1;
            },
            ')' => depth = depth.saturating_sub(1),
            _ if ch.is_ascii_whitespace() && depth == 0 => {
                if let Some(offset) = start.take() {
                    components.push(&input[offset..index]);
                }
            },
            _ => {
                start.get_or_insert(index);
            },
        }
    }
    if let Some(offset) = start {
        components.push(&input[offset..]);
    }
    components
}

impl Transform {
    pub fn functions(&self) -> Option<&[TransformFunction]> {
        match self {
            Self::None => None,
            Self::Functions(functions) => Some(functions),
        }
    }

    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Interpolate matching transform primitives directly, then normalize any
    /// mismatched suffix (including `none`) through a decomposed 2D matrix.
    pub fn interpolate(&self, other: &Self, progress: f32) -> Self {
        let progress = progress.clamp(0.0, 1.0);
        if matches!((self, other), (Self::None, Self::None)) {
            return Self::None;
        }

        let from = self.functions().unwrap_or(&[]);
        let to = other.functions().unwrap_or(&[]);
        let mut functions = Vec::new();
        let mut prefix = 0;
        while let (Some(from), Some(to)) = (from.get(prefix), to.get(prefix)) {
            let Some(value) = interpolate_transform_function(*from, *to, progress) else {
                break;
            };
            functions.push(value);
            prefix += 1;
        }
        if prefix == from.len() && prefix == to.len() {
            return Self::Functions(functions);
        }

        let from_matrix = Matrix2D::from_absolute_functions(&from[prefix..], 16.0);
        let to_matrix = Matrix2D::from_absolute_functions(&to[prefix..], 16.0);
        match from_matrix
            .zip(to_matrix)
            .and_then(|(from, to)| from.interpolate(to, progress))
        {
            Some(matrix) => {
                functions.push(TransformFunction::Matrix(matrix));
                Self::Functions(functions)
            },
            None if progress < 0.5 => self.clone(),
            None => other.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransformFunction {
    Translate(LengthPercentage, LengthPercentage),
    Scale(f32, f32),
    Rotate(f32),
    Skew(f32, f32),
    Matrix(Matrix2D),
}

fn interpolate_transform_function(
    from: TransformFunction,
    to: TransformFunction,
    progress: f32,
) -> Option<TransformFunction> {
    let scalar = |from: f32, to: f32| from + (to - from) * progress;
    Some(match (from, to) {
        (
            TransformFunction::Translate(from_x, from_y),
            TransformFunction::Translate(to_x, to_y),
        ) => TransformFunction::Translate(
            from_x.interpolate(to_x, progress),
            from_y.interpolate(to_y, progress),
        ),
        (TransformFunction::Scale(from_x, from_y), TransformFunction::Scale(to_x, to_y)) => {
            TransformFunction::Scale(scalar(from_x, to_x), scalar(from_y, to_y))
        },
        (TransformFunction::Rotate(from), TransformFunction::Rotate(to)) => {
            TransformFunction::Rotate(scalar(from, to))
        },
        (TransformFunction::Skew(from_x, from_y), TransformFunction::Skew(to_x, to_y)) => {
            TransformFunction::Skew(scalar(from_x, to_x), scalar(from_y, to_y))
        },
        (TransformFunction::Matrix(from), TransformFunction::Matrix(to)) => {
            TransformFunction::Matrix(from.interpolate(to, progress)?)
        },
        _ => return None,
    })
}

fn interpolate_length(from: Length, to: Length, progress: f32) -> Option<Length> {
    (from.unit == to.unit).then_some(Length {
        value: from.value + (to.value - from.value) * progress,
        unit: from.unit,
    })
}

impl FromStr for Transform {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let mut input = input.trim();
        if input.eq_ignore_ascii_case("none") {
            return Ok(Self::None);
        }

        let mut functions = Vec::new();
        while !input.is_empty() {
            let open = input
                .find('(')
                .ok_or_else(|| ParseError::expected("a supported 2D transform function"))?;
            let name = input[..open].trim().to_ascii_lowercase();
            if name.is_empty() || name.split_ascii_whitespace().count() != 1 {
                return Err(ParseError::expected("a supported 2D transform function"));
            }
            let tail = &input[open + 1..];
            let close = tail
                .find(')')
                .ok_or_else(|| ParseError::expected("a closed 2D transform function"))?;
            let arguments = tail[..close]
                .split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>();
            functions.push(parse_transform_function(&name, &arguments)?);
            input = tail[close + 1..].trim_start();
        }
        if functions.is_empty() {
            Err(ParseError::expected("none or a 2D transform list"))
        } else {
            Ok(Self::Functions(functions))
        }
    }
}

fn parse_transform_function(
    name: &str,
    arguments: &[&str],
) -> Result<TransformFunction, ParseError> {
    let length_percentage = |value: &str| value.parse::<LengthPercentage>();
    let number = |value: &str| {
        value
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| ParseError::expected("a finite transform number"))
    };
    match (name, arguments) {
        ("translate", [x]) => Ok(TransformFunction::Translate(
            length_percentage(x)?,
            LengthPercentage::ZERO,
        )),
        ("translate", [x, y]) => Ok(TransformFunction::Translate(
            length_percentage(x)?,
            length_percentage(y)?,
        )),
        ("translatex", [x]) => Ok(TransformFunction::Translate(
            length_percentage(x)?,
            LengthPercentage::ZERO,
        )),
        ("translatey", [y]) => Ok(TransformFunction::Translate(
            LengthPercentage::ZERO,
            length_percentage(y)?,
        )),
        ("scale", [both]) => {
            let both = number(both)?;
            Ok(TransformFunction::Scale(both, both))
        },
        ("scale", [x, y]) => Ok(TransformFunction::Scale(number(x)?, number(y)?)),
        ("scalex", [x]) => Ok(TransformFunction::Scale(number(x)?, 1.0)),
        ("scaley", [y]) => Ok(TransformFunction::Scale(1.0, number(y)?)),
        ("rotate", [angle]) => Ok(TransformFunction::Rotate(parse_angle(angle)?)),
        ("skew", [x]) => Ok(TransformFunction::Skew(parse_angle(x)?, 0.0)),
        ("skew", [x, y]) => Ok(TransformFunction::Skew(parse_angle(x)?, parse_angle(y)?)),
        ("skewx", [x]) => Ok(TransformFunction::Skew(parse_angle(x)?, 0.0)),
        ("skewy", [y]) => Ok(TransformFunction::Skew(0.0, parse_angle(y)?)),
        ("matrix", [a, b, c, d, e, f]) => Ok(TransformFunction::Matrix(Matrix2D::new(
            number(a)?,
            number(b)?,
            number(c)?,
            number(d)?,
            number(e)?,
            number(f)?,
        ))),
        _ => Err(ParseError::expected(
            "translate, scale, rotate, skew, or matrix",
        )),
    }
}

fn parse_angle(input: &str) -> Result<f32, ParseError> {
    let lower = input.trim().to_ascii_lowercase();
    let (number, factor) = if let Some(value) = lower.strip_suffix("deg") {
        (value, std::f32::consts::PI / 180.0)
    } else if let Some(value) = lower.strip_suffix("grad") {
        (value, std::f32::consts::PI / 200.0)
    } else if let Some(value) = lower.strip_suffix("rad") {
        (value, 1.0)
    } else if let Some(value) = lower.strip_suffix("turn") {
        (value, std::f32::consts::TAU)
    } else if lower == "0" || lower == "+0" || lower == "-0" {
        ("0", 1.0)
    } else {
        return Err(ParseError::expected("a deg, rad, or turn angle"));
    };
    number
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .map(|value| value * factor)
        .ok_or_else(|| ParseError::expected("a finite angle"))
}

impl fmt::Display for Transform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("none"),
            Self::Functions(functions) => {
                for (index, function) in functions.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(" ")?;
                    }
                    function.fmt(formatter)?;
                }
                Ok(())
            },
        }
    }
}

impl fmt::Display for TransformFunction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Translate(x, y) => write!(formatter, "translate({x}, {y})"),
            Self::Scale(x, y) => write!(
                formatter,
                "scale({}, {})",
                format_number(*x),
                format_number(*y)
            ),
            Self::Rotate(radians) => write!(formatter, "rotate({}rad)", format_number(*radians)),
            Self::Skew(x, y) => write!(
                formatter,
                "skew({}rad, {}rad)",
                format_number(*x),
                format_number(*y)
            ),
            Self::Matrix(matrix) => matrix.fmt(formatter),
        }
    }
}
