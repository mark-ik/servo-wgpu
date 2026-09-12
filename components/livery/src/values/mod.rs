// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Typed values used by Livery's first property lane.

use std::{error::Error, fmt, str::FromStr};

mod calc;
mod color;
mod color_scheme;
mod length;
mod logical;
mod property;
mod transform_matrix;

pub use color::{
    Color, ColorSpace, ComputedColor, HueInterpolation, SpecifiedColor, SystemColor,
    UsedColorContext,
};
pub use color_scheme::{ColorScheme, ColorSchemeList};
pub use length::{
    CalcLengthPercentage, ContainerAxisSize, Length, LengthPercentage, LengthUnit,
    MathLengthPercentage, RelativeLengthEnvironment, TreeCounts,
};
pub use logical::{LogicalAxis, LogicalSide, PhysicalAxis, PhysicalSide};
pub use property::{
    Alignment, AnimationDelay, AnimationName, AspectRatio, BackgroundAttachment, BackgroundBox,
    BackgroundImage, BackgroundPosition, BackgroundRepeat, BackgroundSize, BackgroundSizeComponent,
    BorderCollapse, BorderStyle, BorderWidth, BoxShadow, BoxShadowValue, BoxSizing, BreakAfter,
    BreakBefore, BreakInside, CaptionSide, Clear, ClipPath, ColumnCount, ColumnFill, ColumnWidth,
    Contain, ContainIntrinsicSize, ContainerName, ContainerType, Direction, Display, Duration,
    EmptyCells, FlexBasis, FlexDirection, FlexFactor, FlexWrap, Float, FontFamily,
    FontFeatureSetting, FontFeatureSettings, FontSize, FontStyle, FontVariantLigatures, FontWeight,
    Gap, GridAutoFlow, GridPlacement, GridTemplate, GridTrack, HangingPunctuation, Hyphens, Inset,
    LineBreak, LineHeight, ListStylePosition, ListStyleType, Margin, Opacity, Order, Orphans,
    Overflow, OverflowWrap, Padding, PointerEvents, Position, Radius, RepeatStyle, Rotate, Scale,
    ShapeOutside, Size, Spacing, TabSize, TableBorderSpacing, TableLayout, TextAlign,
    TextAlignLast, TextDecorationColor, TextDecorationLine, TextIndent, TextJustify, TextTransform,
    TextTransformCase, TextWrapMode, TimingFunction, Transform, TransformFunction, TransformOrigin,
    TransitionProperty, VerticalAlign, Visibility, WhiteSpaceCollapse, Widows, WordBreak,
    WritingMode, ZIndex,
};
pub use transform_matrix::Matrix2D;

/// A rejected CSS value from Livery's bounded first-lane grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParseError {
    expected: &'static str,
}

impl ParseError {
    pub(crate) const fn expected(expected: &'static str) -> Self {
        Self { expected }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "expected {}", self.expected)
    }
}

impl Error for ParseError {}

/// Common parse/serialize contract for Livery value types.
pub trait CssValue: Sized + fmt::Display + FromStr<Err = ParseError> {
    fn parse_css(input: &str) -> Result<Self, ParseError> {
        input.parse()
    }

    fn to_css_string(&self) -> String {
        self.to_string()
    }
}

impl<T> CssValue for T where T: Sized + fmt::Display + FromStr<Err = ParseError> {}

/// Resolve viewport-relative lengths at the specified-to-computed boundary.
///
/// Every generated value family implements this trait so adding a new family
/// requires an explicit decision about whether it contains viewport units.
pub trait ResolveViewport: Clone {
    fn resolve_viewport(&self, viewport_width: f32, viewport_height: f32) -> Self {
        self.resolve_relative_lengths(RelativeLengthEnvironment::uniform_viewport(
            viewport_width,
            viewport_height,
        ))
    }

    fn resolve_relative_lengths(&self, _environment: RelativeLengthEnvironment) -> Self {
        self.clone()
    }
}

macro_rules! unchanged_viewport_resolution {
    ($($name:ident),+ $(,)?) => {
        $(
            impl ResolveViewport for $name {}
        )+
    };
}

unchanged_viewport_resolution!(
    Alignment,
    AnimationDelay,
    AnimationName,
    AspectRatio,
    BackgroundAttachment,
    BackgroundBox,
    BackgroundImage,
    BackgroundRepeat,
    BorderCollapse,
    BorderStyle,
    BoxSizing,
    CaptionSide,
    Clear,
    ColorSchemeList,
    ColumnCount,
    ColumnFill,
    Contain,
    ContainerName,
    ContainerType,
    ComputedColor,
    BreakAfter,
    BreakBefore,
    BreakInside,
    Direction,
    Display,
    Duration,
    EmptyCells,
    FlexDirection,
    FlexFactor,
    FlexWrap,
    Float,
    FontFamily,
    FontFeatureSettings,
    FontStyle,
    FontVariantLigatures,
    FontWeight,
    GridAutoFlow,
    GridPlacement,
    HangingPunctuation,
    Hyphens,
    Orphans,
    LineBreak,
    ListStylePosition,
    ListStyleType,
    Opacity,
    Order,
    Overflow,
    OverflowWrap,
    PointerEvents,
    Position,
    ShapeOutside,
    TableLayout,
    TextAlign,
    TextAlignLast,
    TextDecorationLine,
    TextJustify,
    TextTransform,
    TextWrapMode,
    TimingFunction,
    TransitionProperty,
    Visibility,
    WhiteSpaceCollapse,
    WordBreak,
    WritingMode,
    Widows,
);

impl ResolveViewport for ColumnWidth {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        match self {
            Self::Length(length) => Self::Length(length.resolve_relative(environment)),
            Self::Auto => Self::Auto,
        }
    }
}

// The three bounded scalar families carry no lengths, but they can retain a
// math program whose tree-counting leaves resolve from the same environment.
impl ResolveViewport for Rotate {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        self.resolve_math(environment)
    }
}

impl ResolveViewport for Scale {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        self.resolve_math(environment)
    }
}

impl ResolveViewport for ClipPath {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        match self {
            Self::None => Self::None,
            Self::Polygon(points) => Self::Polygon(
                points
                    .iter()
                    .map(|(x, y)| {
                        (
                            x.resolve_relative(environment),
                            y.resolve_relative(environment),
                        )
                    })
                    .collect(),
            ),
        }
    }
}

impl ResolveViewport for ZIndex {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        self.resolve_math(environment)
    }
}

impl ResolveViewport for GridTemplate {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        match self {
            Self::None => Self::None,
            Self::Tracks(tracks) => Self::Tracks(
                tracks
                    .iter()
                    .map(|track| match track {
                        GridTrack::Length(value) => {
                            GridTrack::Length(value.resolve_relative(environment))
                        },
                        _ => *track,
                    })
                    .collect(),
            ),
        }
    }
}

impl ResolveViewport for BackgroundPosition {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        Self {
            x: self.x.resolve_relative(environment),
            y: self.y.resolve_relative(environment),
        }
    }
}

impl ResolveViewport for BackgroundSize {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        self.resolve_relative(environment)
    }
}

impl ResolveViewport for ContainIntrinsicSize {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        match *self {
            Self::None => Self::None,
            Self::Lengths { width, height } => Self::Lengths {
                width: width.resolve_relative(environment),
                height: height.resolve_relative(environment),
            },
        }
    }
}

impl ResolveViewport for BorderWidth {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        match *self {
            Self::Length(length) => Self::Length(length.resolve_relative(environment)),
            value => value,
        }
    }
}

impl ResolveViewport for BoxShadow {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        match self {
            Self::None => Self::None,
            Self::Value(value) => Self::Value(BoxShadowValue {
                offset_x: value.offset_x.resolve_relative(environment),
                offset_y: value.offset_y.resolve_relative(environment),
                blur_radius: value.blur_radius.resolve_relative(environment),
                spread_radius: value.spread_radius.resolve_relative(environment),
                color: value.color.clone(),
                inset: value.inset,
            }),
        }
    }
}

impl ResolveViewport for FontSize {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        match *self {
            Self::Value(value) => Self::Value(value.resolve_relative(environment)),
            value => value,
        }
    }
}

impl ResolveViewport for Gap {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        Self(self.0.resolve_relative(environment))
    }
}

impl ResolveViewport for TableBorderSpacing {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        Self {
            horizontal: self.horizontal.resolve_relative(environment),
            vertical: self.vertical.resolve_relative(environment),
        }
    }
}

impl ResolveViewport for Inset {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        match *self {
            Self::Value(value) => Self::Value(value.resolve_relative(environment)),
            value => value,
        }
    }
}

impl ResolveViewport for LineHeight {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        match *self {
            Self::Value(value) => Self::Value(value.resolve_relative(environment)),
            value => value,
        }
    }
}

impl ResolveViewport for Margin {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        match *self {
            Self::Value(value) => Self::Value(value.resolve_relative(environment)),
            value => value,
        }
    }
}

impl ResolveViewport for Padding {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        Self(self.0.resolve_relative(environment))
    }
}

impl ResolveViewport for Radius {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        Self(self.0.resolve_relative(environment))
    }
}

impl ResolveViewport for Size {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        match *self {
            Self::FitContent(value) => Self::FitContent(value.resolve_relative(environment)),
            Self::Value(value) => Self::Value(value.resolve_relative(environment)),
            value => value,
        }
    }
}

impl ResolveViewport for FlexBasis {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        self.resolve_relative(environment)
    }
}

impl ResolveViewport for TextIndent {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        Self {
            length: self.length.resolve_relative(environment),
            ..*self
        }
    }
}

impl ResolveViewport for TabSize {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        match *self {
            Self::Length(length) => Self::Length(length.resolve_relative(environment)),
            value => value,
        }
    }
}

impl ResolveViewport for Spacing {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        match *self {
            Self::Length(value) => Self::Length(value.resolve_relative(environment)),
            value => value,
        }
    }
}

impl ResolveViewport for Transform {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        let Self::Functions(functions) = self else {
            return Self::None;
        };
        Self::Functions(
            functions
                .iter()
                .copied()
                .map(|function| match function {
                    TransformFunction::Translate(x, y) => TransformFunction::Translate(
                        x.resolve_relative(environment),
                        y.resolve_relative(environment),
                    ),
                    function => function,
                })
                .collect(),
        )
    }
}

impl ResolveViewport for TransformOrigin {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        Self {
            x: self.x.resolve_relative(environment),
            y: self.y.resolve_relative(environment),
            z: self.z.resolve_relative(environment),
        }
    }
}

impl ResolveViewport for VerticalAlign {
    fn resolve_relative_lengths(&self, environment: RelativeLengthEnvironment) -> Self {
        match *self {
            Self::Length(value) => Self::Length(value.resolve_relative(environment)),
            value => value,
        }
    }
}

/// Computed-value interpolation (harvest H2), the general machinery the
/// retained transition clock dispatches through. The default is the
/// discrete midpoint flip of css-transitions; families with a defined
/// interpolation override it. Every generated `PropertyValue` variant type
/// must have an impl below, so adding a value family without deciding its
/// interpolation is a compile error.
pub trait Interpolate: Clone {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        if progress < 0.5 {
            self.clone()
        } else {
            other.clone()
        }
    }
}

macro_rules! discrete_interpolation {
    ($($name:ident),+ $(,)?) => {
        $(impl Interpolate for $name {})+
    };
}

discrete_interpolation!(
    Alignment,
    AnimationDelay,
    AnimationName,
    AspectRatio,
    BreakAfter,
    BreakBefore,
    BreakInside,
    BackgroundAttachment,
    BackgroundBox,
    BorderCollapse,
    BoxSizing,
    CaptionSide,
    ClipPath,
    Clear,
    ColorSchemeList,
    ColumnCount,
    ColumnFill,
    ColumnWidth,
    Contain,
    ContainIntrinsicSize,
    ContainerName,
    ContainerType,
    Direction,
    Display,
    Duration,
    EmptyCells,
    FlexDirection,
    FlexFactor,
    FlexWrap,
    Float,
    FontFamily,
    FontFeatureSettings,
    FontSize,
    FontStyle,
    FontVariantLigatures,
    FontWeight,
    Gap,
    GridAutoFlow,
    GridPlacement,
    GridTemplate,
    HangingPunctuation,
    Hyphens,
    Orphans,
    Inset,
    LineBreak,
    LineHeight,
    ListStylePosition,
    ListStyleType,
    Margin,
    Order,
    Overflow,
    OverflowWrap,
    Padding,
    PointerEvents,
    Position,
    ShapeOutside,
    Size,
    Spacing,
    TabSize,
    TableBorderSpacing,
    TableLayout,
    TextAlign,
    TextAlignLast,
    TextDecorationLine,
    TextIndent,
    TextJustify,
    TextTransform,
    TextWrapMode,
    TimingFunction,
    TransitionProperty,
    VerticalAlign,
    Visibility,
    WhiteSpaceCollapse,
    WordBreak,
    WritingMode,
    Widows,
    ZIndex,
);

impl Interpolate for FlexBasis {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        (*self).interpolate(*other, progress)
    }
}

impl Interpolate for Color {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        Self::interpolate(*self, *other, progress)
    }
}

impl Interpolate for Opacity {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        Self::from_value(self.value() + (other.value() - self.value()) * progress)
    }
}

impl Interpolate for Rotate {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        match (*self, *other) {
            (Self::Angle(from), Self::Angle(to)) => {
                Self::Angle(from + (to - from) * progress.clamp(0.0, 1.0))
            },
            _ if progress < 0.5 => *self,
            _ => *other,
        }
    }
}

impl Interpolate for Scale {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        match (*self, *other) {
            (Self::Uniform(from), Self::Uniform(to)) => {
                Self::Uniform(from + (to - from) * progress.clamp(0.0, 1.0))
            },
            _ if progress < 0.5 => *self,
            _ => *other,
        }
    }
}

impl Interpolate for BackgroundImage {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        self.interpolate(other, progress)
    }
}

impl Interpolate for BackgroundPosition {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        Self::interpolate(*self, *other, progress)
    }
}

impl Interpolate for BackgroundRepeat {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        Self::interpolate(*self, *other, progress)
    }
}

impl Interpolate for BackgroundSize {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        Self::interpolate(*self, *other, progress)
    }
}

impl Interpolate for BorderStyle {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        Self::interpolate(*self, *other, progress)
    }
}

impl Interpolate for BorderWidth {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        Self::interpolate(*self, *other, progress)
    }
}

impl Interpolate for Radius {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        Self::interpolate(*self, *other, progress)
    }
}

impl Interpolate for BoxShadow {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        self.interpolate(other, progress)
    }
}

impl Interpolate for ComputedColor {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        self.interpolate(other, progress)
    }
}

impl Interpolate for Transform {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        self.interpolate(other, progress)
    }
}

impl Interpolate for TransformOrigin {
    fn interpolate_value(&self, other: &Self, progress: f32) -> Self {
        self.interpolate(*other, progress)
    }
}

/// Crate-internal alias for the specified-value boundary in lib.rs.
pub(crate) fn format_number_public(value: f32) -> String {
    format_number(value)
}

pub(crate) fn format_number(value: f32) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    value.to_string()
}

macro_rules! keyword_value {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($variant:ident => $css:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum $name {
            $($variant),+
        }

        impl std::str::FromStr for $name {
            type Err = super::ParseError;

            fn from_str(input: &str) -> Result<Self, Self::Err> {
                match input.trim().to_ascii_lowercase().as_str() {
                    $($css => Ok(Self::$variant),)+
                    _ => Err(super::ParseError::expected(stringify!($name))),
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(match self {
                    $(Self::$variant => $css,)+
                })
            }
        }
    };
}

pub(crate) use keyword_value;
