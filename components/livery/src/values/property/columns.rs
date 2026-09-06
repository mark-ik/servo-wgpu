// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::{fmt, str::FromStr};

use cssparser::{Parser, ParserInput, Token};

use super::super::{LengthPercentage, ParseError, ZIndex};

fn parser_contains_percentage(parser: &mut Parser<'_, '_>) -> bool {
    loop {
        let token = match parser.next_including_whitespace() {
            Ok(token) => token,
            Err(_) => return false,
        };
        match token {
            Token::Percentage { .. } => return true,
            Token::Function(..)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                if parser
                    .parse_nested_block(|nested| {
                        Ok::<bool, cssparser::ParseError<'_, ()>>(parser_contains_percentage(
                            nested,
                        ))
                    })
                    .unwrap_or(false)
                {
                    return true;
                }
            },
            _ => {},
        }
    }
}

fn contains_percentage_token(input: &str) -> bool {
    let mut input_buffer = ParserInput::new(input);
    let mut parser = Parser::new(&mut input_buffer);
    parser_contains_percentage(&mut parser)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColumnCount {
    Auto,
    Count(u32),
}

impl FromStr for ColumnCount {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        if input.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }
        let count = input
            .parse::<u32>()
            .ok()
            .or_else(|| {
                input.parse::<ZIndex>().ok().and_then(|value| match value {
                    ZIndex::Integer(value) if input.contains('(') => Some(value.max(1) as u32),
                    _ => None,
                })
            })
            .filter(|count| *count > 0)
            .ok_or_else(|| ParseError::expected("auto or a positive integer"))?;
        Ok(Self::Count(count))
    }
}

impl fmt::Display for ColumnCount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => formatter.write_str("auto"),
            Self::Count(count) => count.fmt(formatter),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColumnWidth {
    Auto,
    Length(LengthPercentage),
}

impl FromStr for ColumnWidth {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        if input.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }
        let length = input
            .parse::<LengthPercentage>()
            .map_err(|_| ParseError::expected("auto or a non-negative length"))?;
        if contains_percentage_token(input)
            || length.has_percentage()
            || matches!(length, LengthPercentage::Length(length) if length.value < 0.0)
        {
            return Err(ParseError::expected("auto or a non-negative length"));
        }
        Ok(Self::Length(length))
    }
}

impl fmt::Display for ColumnWidth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => formatter.write_str("auto"),
            Self::Length(length) => length.fmt(formatter),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColumnFill {
    Auto,
    Balance,
    BalanceAll,
}

impl FromStr for ColumnFill {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "balance" => Ok(Self::Balance),
            "balance-all" => Ok(Self::BalanceAll),
            _ => Err(ParseError::expected("auto, balance, or balance-all")),
        }
    }
}

impl fmt::Display for ColumnFill {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Auto => "auto",
            Self::Balance => "balance",
            Self::BalanceAll => "balance-all",
        })
    }
}

macro_rules! break_value {
    ($name:ident, [$($variant:ident => $keyword:literal),+ $(,)?]) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum $name { $($variant),+ }

        impl FromStr for $name {
            type Err = ParseError;
            fn from_str(input: &str) -> Result<Self, Self::Err> {
                match input.trim().to_ascii_lowercase().as_str() {
                    $($keyword => Ok(Self::$variant),)+
                    _ => Err(ParseError::expected("a supported break value")),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(match self { $(Self::$variant => $keyword,)+ })
            }
        }
    };
}

break_value!(BreakBefore, [
    Auto => "auto", Avoid => "avoid", Always => "always",
    AvoidPage => "avoid-page", Page => "page", Left => "left", Right => "right",
    Recto => "recto", Verso => "verso", AvoidColumn => "avoid-column",
    Column => "column", AvoidRegion => "avoid-region", Region => "region",
]);

break_value!(BreakAfter, [
    Auto => "auto", Avoid => "avoid", Always => "always",
    AvoidPage => "avoid-page", Page => "page", Left => "left", Right => "right",
    Recto => "recto", Verso => "verso", AvoidColumn => "avoid-column",
    Column => "column", AvoidRegion => "avoid-region", Region => "region",
]);

break_value!(BreakInside, [
    Auto => "auto", Avoid => "avoid", AvoidPage => "avoid-page",
    AvoidColumn => "avoid-column", AvoidRegion => "avoid-region",
]);

macro_rules! positive_integer {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct $name(pub u32);

        impl FromStr for $name {
            type Err = ParseError;
            fn from_str(input: &str) -> Result<Self, Self::Err> {
                input
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .or_else(|| {
                        input
                            .trim()
                            .parse::<ZIndex>()
                            .ok()
                            .and_then(|value| match value {
                                ZIndex::Integer(value) if input.contains('(') => {
                                    Some(value.max(1) as u32)
                                },
                                _ => None,
                            })
                    })
                    .filter(|value| *value > 0)
                    .map(Self)
                    .ok_or_else(|| ParseError::expected("a positive integer"))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

positive_integer!(Orphans);
positive_integer!(Widows);
