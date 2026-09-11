// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Box and layout values: sizing, flex and grid vocabulary, aspect
//! ratio, radii, gaps and spacing, ordering, padding, decoration lines,
//! border width, and stacking level.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridAutoFlow {
    Row,
    Column,
    RowDense,
    ColumnDense,
}

impl FromStr for GridAutoFlow {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input.trim().to_ascii_lowercase().as_str() {
            "row" => Ok(Self::Row),
            "column" => Ok(Self::Column),
            "row dense" => Ok(Self::RowDense),
            "column dense" => Ok(Self::ColumnDense),
            _ => Err(ParseError::expected("grid-auto-flow keywords")),
        }
    }
}

impl fmt::Display for GridAutoFlow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Row => "row",
            Self::Column => "column",
            Self::RowDense => "row dense",
            Self::ColumnDense => "column dense",
        })
    }
}

keyword_value! {
    pub enum FontStyle {
        Normal => "normal",
        Italic => "italic",
        Oblique => "oblique",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ListStyleType {
    None,
    Disc,
    Decimal,
    String(std::string::String),
}

impl FromStr for ListStyleType {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        if input.eq_ignore_ascii_case("none") {
            return Ok(Self::None);
        }
        if input.eq_ignore_ascii_case("disc") {
            return Ok(Self::Disc);
        }
        if input.eq_ignore_ascii_case("decimal") {
            return Ok(Self::Decimal);
        }

        let mut buffer = cssparser::ParserInput::new(input);
        let mut parser = cssparser::Parser::new(&mut buffer);
        let value = parser
            .expect_string_cloned()
            .map_err(|_| ParseError::expected("list-style-type keyword or string"))?;
        parser
            .expect_exhausted()
            .map_err(|_| ParseError::expected("one list-style-type value"))?;
        Ok(Self::String(value.as_ref().to_owned()))
    }
}

impl fmt::Display for ListStyleType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("none"),
            Self::Disc => formatter.write_str("disc"),
            Self::Decimal => formatter.write_str("decimal"),
            Self::String(value) => {
                use fmt::Write;
                formatter.write_char('"')?;
                write!(cssparser::CssStringWriter::new(formatter), "{value}")?;
                formatter.write_char('"')
            },
        }
    }
}

keyword_value! {
    pub enum ListStylePosition {
        Inside => "inside",
        Outside => "outside",
    }
}

keyword_value! {
    pub enum Overflow {
        Visible => "visible",
        Hidden => "hidden",
        Clip => "clip",
        Scroll => "scroll",
        Auto => "auto",
    }
}

keyword_value! {
    pub enum Position {
        Static => "static",
        Relative => "relative",
        Absolute => "absolute",
        Sticky => "sticky",
        Fixed => "fixed",
    }
}

keyword_value! {
    pub enum TextWrapMode {
        Wrap => "wrap",
        Nowrap => "nowrap",
    }
}

keyword_value! {
    pub enum WhiteSpaceCollapse {
        Collapse => "collapse",
        Discard => "discard",
        Preserve => "preserve",
        PreserveBreaks => "preserve-breaks",
        PreserveSpaces => "preserve-spaces",
        BreakSpaces => "break-spaces",
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BorderWidth {
    Thin,
    Medium,
    Thick,
    Length(Length),
}

impl BorderWidth {
    /// Interpolate the bounded line-width family used by the border paint
    /// lane. Fixed keyword widths participate in the px family; length
    /// endpoints interpolate only when their units match.
    pub fn interpolate(self, other: Self, progress: f32) -> Self {
        let progress = progress.clamp(0.0, 1.0);
        match (self, other) {
            (Self::Length(from), Self::Length(to)) if from.unit == to.unit => {
                Self::Length(Length {
                    value: from.value + (to.value - from.value) * progress,
                    unit: from.unit,
                })
            },
            (from, to) => match (from.computed_px(), to.computed_px()) {
                (Some(from), Some(to)) => Self::Length(Length::px(from + (to - from) * progress)),
                _ if progress < 0.5 => from,
                _ => to,
            },
        }
    }

    fn computed_px(self) -> Option<f32> {
        match self {
            Self::Thin => Some(1.0),
            Self::Medium => Some(3.0),
            Self::Thick => Some(5.0),
            Self::Length(length) if length.unit == crate::values::LengthUnit::Px => {
                Some(length.value)
            },
            _ => None,
        }
    }
}

impl FromStr for BorderWidth {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input.trim().to_ascii_lowercase().as_str() {
            "thin" => Ok(Self::Thin),
            "medium" => Ok(Self::Medium),
            "thick" => Ok(Self::Thick),
            _ => input.parse::<Length>().map(Self::Length),
        }
    }
}

impl fmt::Display for BorderWidth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Thin => formatter.write_str("thin"),
            Self::Medium => formatter.write_str("medium"),
            Self::Thick => formatter.write_str("thick"),
            Self::Length(length) => length.fmt(formatter),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    Auto,
    None,
    MinContent,
    MaxContent,
    FitContent(LengthPercentage),
    Value(LengthPercentage),
}

impl FromStr for Size {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        if input.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }
        if input.eq_ignore_ascii_case("none") {
            return Ok(Self::None);
        }
        if input.eq_ignore_ascii_case("min-content") {
            return Ok(Self::MinContent);
        }
        if input.eq_ignore_ascii_case("max-content") {
            return Ok(Self::MaxContent);
        }
        if input.len() > 13
            && input[..12].eq_ignore_ascii_case("fit-content(")
            && input.ends_with(')')
        {
            return input[12..input.len() - 1]
                .parse::<LengthPercentage>()
                .map(Self::FitContent);
        }
        input.parse::<LengthPercentage>().map(Self::Value)
    }
}

impl fmt::Display for Size {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => formatter.write_str("auto"),
            Self::None => formatter.write_str("none"),
            Self::MinContent => formatter.write_str("min-content"),
            Self::MaxContent => formatter.write_str("max-content"),
            Self::FitContent(value) => write!(formatter, "fit-content({value})"),
            Self::Value(value) => value.fmt(formatter),
        }
    }
}

/// The `flex-basis` grammar is deliberately separate from generic box sizes.
/// In particular it admits `content` and the intrinsic sizing keywords, while
/// rejecting width-only `none`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlexBasis {
    Auto,
    Content,
    MinContent,
    MaxContent,
    FitContent,
    Value(LengthPercentage),
}

impl FlexBasis {
    fn clamp_definite_nonnegative(self) -> Self {
        let Self::Value(value) = self else {
            return self;
        };
        let value = match value {
            LengthPercentage::Length(length) if length.unit == crate::values::LengthUnit::Px => {
                LengthPercentage::Length(Length::px(length.value.max(0.0)))
            },
            LengthPercentage::Calc(calc)
                if calc.percentage == 0.0
                    && calc.em == 0.0
                    && calc.rem == 0.0
                    && !calc.has_unresolved_relative() =>
            {
                LengthPercentage::Length(Length::px(calc.px.max(0.0)))
            },
            value => value,
        };
        Self::Value(value)
    }

    /// Resolve font-relative terms for the computed-value boundary. A
    /// definite negative result is clamped as required by flex-basis's
    /// non-negative range; percentage-dependent results remain deferred.
    pub fn resolve_font_relative(self, em: f32, rem: f32) -> Self {
        let Self::Value(value) = self else {
            return self;
        };
        Self::Value(value.resolve_font_relative(em, rem)).clamp_definite_nonnegative()
    }

    /// Resolve available environment-relative terms and reapply the computed
    /// non-negative range once the result becomes definite. Font-relative and
    /// percentage-dependent values remain deferred to their own boundaries.
    pub fn resolve_relative(self, environment: RelativeLengthEnvironment) -> Self {
        let Self::Value(value) = self else {
            return self;
        };
        Self::Value(value.resolve_relative(environment)).clamp_definite_nonnegative()
    }

    /// Interpolate numeric bases through their shared length-percentage
    /// representation. Keywords and incompatible value families switch at
    /// the animation midpoint.
    pub fn interpolate(self, other: Self, progress: f32) -> Self {
        match (self, other) {
            (Self::Value(from), Self::Value(to)) => Self::Value(from.interpolate(to, progress)),
            (from, _) if progress.clamp(0.0, 1.0) < 0.5 => from,
            (_, to) => to,
        }
    }

    /// A computed pure percentage serializes without a redundant `calc()`.
    pub fn computed_value(self) -> Self {
        let Self::Value(LengthPercentage::Calc(calc)) = self else {
            return self;
        };
        if calc.px == 0.0
            && calc.em == 0.0
            && calc.rem == 0.0
            && calc.to_string() == format!("calc({}%)", format_number(calc.percentage * 100.0))
        {
            return Self::Value(LengthPercentage::Percentage(calc.percentage));
        }
        self
    }
}

impl FromStr for FlexBasis {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        let keyword = if input.eq_ignore_ascii_case("auto") {
            Some(Self::Auto)
        } else if input.eq_ignore_ascii_case("content") {
            Some(Self::Content)
        } else if input.eq_ignore_ascii_case("min-content") {
            Some(Self::MinContent)
        } else if input.eq_ignore_ascii_case("max-content") {
            Some(Self::MaxContent)
        } else if input.eq_ignore_ascii_case("fit-content") {
            Some(Self::FitContent)
        } else {
            None
        };
        if let Some(keyword) = keyword {
            return Ok(keyword);
        }

        let value = input.parse::<LengthPercentage>()?;
        match value {
            LengthPercentage::Length(length) if length.value < 0.0 => {
                Err(ParseError::expected("a non-negative flex basis"))
            },
            LengthPercentage::Percentage(value) if value < 0.0 => {
                Err(ParseError::expected("a non-negative flex basis"))
            },
            value => Ok(Self::Value(value)),
        }
    }
}

impl fmt::Display for FlexBasis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => formatter.write_str("auto"),
            Self::Content => formatter.write_str("content"),
            Self::MinContent => formatter.write_str("min-content"),
            Self::MaxContent => formatter.write_str("max-content"),
            Self::FitContent => formatter.write_str("fit-content"),
            Self::Value(value) => value.fmt(formatter),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GridTrack {
    Auto,
    MinContent,
    MaxContent,
    Length(Length),
    Percent(f32),
    Fr(f32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum GridTemplate {
    None,
    Tracks(Vec<GridTrack>),
}

impl FromStr for GridTemplate {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        if input.eq_ignore_ascii_case("none") {
            return Ok(Self::None);
        }
        let mut tracks = Vec::new();
        for component in input.split_ascii_whitespace() {
            let track = if component.eq_ignore_ascii_case("auto") {
                GridTrack::Auto
            } else if component.eq_ignore_ascii_case("min-content") {
                GridTrack::MinContent
            } else if component.eq_ignore_ascii_case("max-content") {
                GridTrack::MaxContent
            } else if let Some(value) = component.strip_suffix("fr") {
                GridTrack::Fr(parse_non_negative(value)?)
            } else if let Some(value) = component.strip_suffix('%') {
                GridTrack::Percent(parse_non_negative(value)? / 100.0)
            } else {
                GridTrack::Length(
                    component
                        .parse::<Length>()
                        .map_err(|_| ParseError::expected("grid track sizes"))?,
                )
            };
            tracks.push(track);
        }
        if tracks.is_empty() {
            Err(ParseError::expected("one or more grid tracks"))
        } else {
            Ok(Self::Tracks(tracks))
        }
    }
}

impl fmt::Display for GridTemplate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("none"),
            Self::Tracks(tracks) => {
                for (index, track) in tracks.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(" ")?;
                    }
                    track.fmt(formatter)?;
                }
                Ok(())
            },
        }
    }
}

impl fmt::Display for GridTrack {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => formatter.write_str("auto"),
            Self::MinContent => formatter.write_str("min-content"),
            Self::MaxContent => formatter.write_str("max-content"),
            Self::Length(value) => value.fmt(formatter),
            Self::Percent(value) => write!(formatter, "{}%", format_number(*value * 100.0)),
            Self::Fr(value) => write!(formatter, "{}fr", format_number(*value)),
        }
    }
}

fn parse_non_negative(input: &str) -> Result<f32, ParseError> {
    input
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| ParseError::expected("a non-negative grid track number"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridPlacement {
    Auto,
    Line(i16),
    Span(u16),
}

impl FromStr for GridPlacement {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        if input.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }
        if let Some(span) = input.strip_prefix("span ") {
            return span
                .parse::<u16>()
                .ok()
                .filter(|value| *value > 0)
                .map(Self::Span)
                .ok_or_else(|| ParseError::expected("a positive grid span"));
        }
        input
            .parse::<i16>()
            .map(Self::Line)
            .map_err(|_| ParseError::expected("auto, span, or a grid line number"))
    }
}

impl fmt::Display for GridPlacement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => formatter.write_str("auto"),
            Self::Line(value) => value.fmt(formatter),
            Self::Span(value) => write!(formatter, "span {value}"),
        }
    }
}

/// CSS `aspect-ratio`, represented as width divided by height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AspectRatio {
    Auto,
    Ratio(f32),
    /// HTML dimension attributes contribute `auto <ratio>`. The operands are
    /// retained separately because HTML permits zero in this mapping even
    /// though an authored CSS `<ratio>` requires two positive numbers.
    AutoRatio {
        width: f32,
        height: f32,
    },
}

impl AspectRatio {
    /// Return the usable preferred ratio. Degenerate HTML ratios remain
    /// serializable computed values but do not enter layout arithmetic.
    pub fn preferred_ratio(self) -> Option<f32> {
        let ratio = match self {
            Self::Auto => return None,
            Self::Ratio(ratio) => ratio,
            Self::AutoRatio { width, height } => width / height,
        };
        ratio
            .is_finite()
            .then_some(ratio)
            .filter(|ratio| *ratio > 0.0)
    }

    pub const fn uses_natural_ratio(self) -> bool {
        matches!(self, Self::Auto | Self::AutoRatio { .. })
    }
}
impl FromStr for AspectRatio {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        if input.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }
        let lowercase = input.to_ascii_lowercase();
        let (auto, input) =
            if lowercase.starts_with("auto") && input[4..].starts_with(char::is_whitespace) {
                (true, input[4..].trim())
            } else if lowercase.ends_with("auto")
                && input[..input.len() - 4].ends_with(char::is_whitespace)
            {
                (true, input[..input.len() - 4].trim())
            } else {
                (false, input)
            };
        let (width, height) = input
            .split_once('/')
            .map_or((input, "1"), |(width, height)| {
                (width.trim(), height.trim())
            });
        let width = width
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| ParseError::expected("a positive aspect-ratio"))?;
        let height = height
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| ParseError::expected("a positive aspect-ratio"))?;
        if auto {
            Ok(Self::AutoRatio { width, height })
        } else {
            Ok(Self::Ratio(width / height))
        }
    }
}

impl fmt::Display for AspectRatio {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => formatter.write_str("auto"),
            Self::Ratio(value) => formatter.write_str(&format_number(*value)),
            Self::AutoRatio { width, height } => write!(
                formatter,
                "auto {} / {}",
                format_number(*width),
                format_number(*height)
            ),
        }
    }
}

macro_rules! auto_length_percentage {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub enum $name {
            Auto,
            Value(LengthPercentage),
        }

        impl FromStr for $name {
            type Err = ParseError;

            fn from_str(input: &str) -> Result<Self, Self::Err> {
                if input.trim().eq_ignore_ascii_case("auto") {
                    Ok(Self::Auto)
                } else {
                    input.parse::<LengthPercentage>().map(Self::Value)
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    Self::Auto => formatter.write_str("auto"),
                    Self::Value(value) => value.fmt(formatter),
                }
            }
        }
    };
}

auto_length_percentage!(Inset);
auto_length_percentage!(Margin);

/// A non-negative border corner radius component.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radius(pub LengthPercentage);

impl Radius {
    pub const ZERO: Self = Self(LengthPercentage::ZERO);

    /// Interpolate the bounded radius forms used by the retained paint lane.
    /// Zero and a concrete length/percentage share the same scalar family;
    /// mixed non-zero units stay discrete until the broader value ratchet.
    pub fn interpolate(self, other: Self, progress: f32) -> Self {
        Self(self.0.interpolate(other.0, progress))
    }
}

impl FromStr for Radius {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let value = input.parse::<LengthPercentage>()?;
        let negative = match value {
            LengthPercentage::Zero => false,
            LengthPercentage::Length(length) => length.value < 0.0,
            LengthPercentage::Percentage(value) => value < 0.0,
            LengthPercentage::Calc(calc) => calc.px < 0.0 || calc.em < 0.0 || calc.rem < 0.0,
            LengthPercentage::Math(_) => false,
        };
        if negative {
            return Err(ParseError::expected("a non-negative border radius"));
        }
        Ok(Self(value))
    }
}

impl fmt::Display for Radius {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A non-negative flex/grid gap component.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gap(pub LengthPercentage);

impl Gap {
    pub const ZERO: Self = Self(LengthPercentage::ZERO);
}

impl FromStr for Gap {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let value = input.parse::<LengthPercentage>()?;
        let negative = match value {
            LengthPercentage::Zero => false,
            LengthPercentage::Length(length) => length.value < 0.0,
            LengthPercentage::Percentage(value) => value < 0.0,
            LengthPercentage::Calc(calc) => calc.px < 0.0 || calc.em < 0.0 || calc.rem < 0.0,
            LengthPercentage::Math(_) => false,
        };
        if negative {
            return Err(ParseError::expected("a non-negative gap"));
        }
        Ok(Self(value))
    }
}

impl fmt::Display for Gap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The horizontal and vertical distances in CSS 2.1's separated-border
/// model. Percentages and negative values are invalid for `border-spacing`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableBorderSpacing {
    pub horizontal: Length,
    pub vertical: Length,
}

impl TableBorderSpacing {
    pub const ZERO: Self = Self {
        horizontal: Length::ZERO,
        vertical: Length::ZERO,
    };
}

impl FromStr for TableBorderSpacing {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let values = input.split_ascii_whitespace().collect::<Vec<_>>();
        let (horizontal, vertical) = match values.as_slice() {
            [horizontal] => {
                let horizontal = horizontal.parse::<Length>()?;
                (horizontal, horizontal)
            },
            [horizontal, vertical] => (horizontal.parse::<Length>()?, vertical.parse::<Length>()?),
            _ => return Err(ParseError::expected("one or two non-negative lengths")),
        };
        if horizontal.value < 0.0 || vertical.value < 0.0 {
            return Err(ParseError::expected("one or two non-negative lengths"));
        }
        Ok(Self {
            horizontal,
            vertical,
        })
    }
}

impl fmt::Display for TableBorderSpacing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.horizontal.fmt(formatter)?;
        if self.vertical != self.horizontal {
            write!(formatter, " {}", self.vertical)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlexFactor(f32);

impl FlexFactor {
    pub const ZERO: Self = Self(0.0);
    pub const ONE: Self = Self(1.0);

    pub const fn value(self) -> f32 {
        self.0
    }
}

impl FromStr for FlexFactor {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        input
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(Self)
            .ok_or_else(|| ParseError::expected("a non-negative flex factor"))
    }
}

impl fmt::Display for FlexFactor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&format_number(self.0))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Order(i32);

impl Order {
    pub const ZERO: Self = Self(0);

    pub const fn value(self) -> i32 {
        self.0
    }
}

impl FromStr for Order {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        input
            .trim()
            .parse::<i32>()
            .map(Self)
            .map_err(|_| ParseError::expected("an integer order"))
    }
}

impl fmt::Display for Order {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A CSS spacing value, with `normal` represented explicitly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spacing {
    Normal,
    Length(LengthPercentage),
}

impl FromStr for Spacing {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.trim().eq_ignore_ascii_case("normal") {
            Ok(Self::Normal)
        } else {
            input
                .parse::<LengthPercentage>()
                .map(Self::Length)
                .map_err(|_| ParseError::expected("normal or a length"))
        }
    }
}

impl fmt::Display for Spacing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Normal => formatter.write_str("normal"),
            Self::Length(length) => length.fmt(formatter),
        }
    }
}

pub type TextDecorationColor = crate::values::Color;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Padding(pub LengthPercentage);

impl Padding {
    pub const ZERO: Self = Self(LengthPercentage::ZERO);
}

impl FromStr for Padding {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let value = input.parse::<LengthPercentage>()?;
        let negative = match value {
            LengthPercentage::Zero => false,
            LengthPercentage::Length(length) => length.value < 0.0,
            LengthPercentage::Percentage(value) => value < 0.0,
            LengthPercentage::Calc(_) => false,
            LengthPercentage::Math(_) => false,
        };
        if negative {
            return Err(ParseError::expected("a non-negative padding"));
        }
        Ok(Self(value))
    }
}

impl fmt::Display for Padding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextDecorationLine(u8);

impl TextDecorationLine {
    pub const NONE: Self = Self(0);
    const UNDERLINE: u8 = 1 << 0;
    const OVERLINE: u8 = 1 << 1;
    const LINE_THROUGH: u8 = 1 << 2;
    const BLINK: u8 = 1 << 3;

    pub const fn contains_underline(self) -> bool {
        self.0 & Self::UNDERLINE != 0
    }
}

impl FromStr for TextDecorationLine {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        if input.eq_ignore_ascii_case("none") {
            return Ok(Self::NONE);
        }
        let mut flags = 0;
        for keyword in input.split_ascii_whitespace() {
            let flag = match keyword.to_ascii_lowercase().as_str() {
                "underline" => Self::UNDERLINE,
                "overline" => Self::OVERLINE,
                "line-through" => Self::LINE_THROUGH,
                "blink" => Self::BLINK,
                _ => return Err(ParseError::expected("text-decoration-line keywords")),
            };
            if flags & flag != 0 {
                return Err(ParseError::expected("unique text-decoration-line keywords"));
            }
            flags |= flag;
        }
        if flags == 0 {
            return Err(ParseError::expected("text-decoration-line keywords"));
        }
        Ok(Self(flags))
    }
}

impl fmt::Display for TextDecorationLine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == Self::NONE {
            return formatter.write_str("none");
        }
        let mut first = true;
        for (flag, name) in [
            (Self::UNDERLINE, "underline"),
            (Self::OVERLINE, "overline"),
            (Self::LINE_THROUGH, "line-through"),
            (Self::BLINK, "blink"),
        ] {
            if self.0 & flag == 0 {
                continue;
            }
            if !first {
                formatter.write_str(" ")?;
            }
            formatter.write_str(name)?;
            first = false;
        }
        Ok(())
    }
}

// `Eq` is unavailable now that a z-index can retain a float-bearing math
// program; nothing in the tree needs more than `PartialEq`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZIndex {
    Auto,
    Integer(i32),
    /// The [`Rotate::Deferred`] twin for integer-valued expressions.
    Deferred(MathLengthPercentage),
}

/// Round a resolved number to the integer a `<integer>` property stores.
fn rounded_integer(value: f32) -> Option<i32> {
    let rounded = (value + 0.5).floor();
    (rounded >= i32::MIN as f32 && rounded <= i32::MAX as f32).then_some(rounded as i32)
}

impl ZIndex {
    pub(in crate::values) fn resolve_math(self, environment: RelativeLengthEnvironment) -> Self {
        match self {
            Self::Deferred(math) => {
                let resolved = math.resolve_relative(environment);
                resolved
                    .resolved_px()
                    .and_then(rounded_integer)
                    .map_or(Self::Deferred(resolved), Self::Integer)
            },
            value => value,
        }
    }
}

impl FromStr for ZIndex {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.trim().eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }
        if let Some(integer) = input.trim().parse::<i32>().ok().or_else(|| {
            input
                .contains('(')
                .then(|| crate::values::calc::parse_number(input).ok())
                .flatten()
                .and_then(rounded_integer)
        }) {
            return Ok(Self::Integer(integer));
        }
        input
            .contains('(')
            .then(|| crate::values::calc::parse_number_math(input).ok())
            .flatten()
            .map(Self::Deferred)
            .ok_or_else(|| ParseError::expected("auto or an integer"))
    }
}

impl fmt::Display for ZIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => formatter.write_str("auto"),
            Self::Integer(value) => value.fmt(formatter),
            Self::Deferred(math) => math.fmt(formatter),
        }
    }
}
