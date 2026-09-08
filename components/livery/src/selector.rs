// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Selector parsing and matching on the standalone `selectors` substrate.

use std::{fmt, hash::Hash};

use cssparser::{CowRcStr, Parser as CssParser, ParserInput, ToCss, match_ignore_ascii_case};
use precomputed_hash::PrecomputedHash;
use selectors::context::{
    MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, QuirksMode, SelectorCaches,
};
use selectors::matching::{MatchingContext, matches_selector};
use selectors::parser::{
    Component, NonTSPseudoClass, ParseRelative, Parser, PseudoElement, Selector, SelectorImpl,
    SelectorList as SubstrateSelectorList, SelectorParseErrorKind,
};

pub use selectors::Element;
pub use selectors::OpaqueElement;
pub use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
pub use selectors::bloom::BloomFilter;
pub use selectors::matching::ElementSelectorFlags;

use crate::cascade::Specificity;

fn stable_hash(value: &str) -> u32 {
    let mut hash = 2_166_136_261_u32;
    for byte in value.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}

/// Owned selector atom with a stable precomputed hash.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Atom {
    text: Box<str>,
    hash: u32,
}

impl Atom {
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl Default for Atom {
    fn default() -> Self {
        Self::from("")
    }
}

impl From<&str> for Atom {
    fn from(value: &str) -> Self {
        Self {
            text: value.into(),
            hash: stable_hash(value),
        }
    }
}

impl AsRef<str> for Atom {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PrecomputedHash for Atom {
    fn precomputed_hash(&self) -> u32 {
        self.hash
    }
}

impl ToCss for Atom {
    fn to_css<W>(&self, destination: &mut W) -> fmt::Result
    where
        W: fmt::Write,
    {
        cssparser::serialize_identifier(self.as_str(), destination)
    }
}

/// Attribute selector value. It serializes as a CSS string, unlike identifier
/// atoms.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AttributeValue(Box<str>);

impl From<&str> for AttributeValue {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl AsRef<str> for AttributeValue {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl ToCss for AttributeValue {
    fn to_css<W>(&self, destination: &mut W) -> fmt::Result
    where
        W: fmt::Write,
    {
        use fmt::Write;
        destination.write_char('"')?;
        write!(cssparser::CssStringWriter::new(destination), "{}", &self.0)?;
        destination.write_char('"')
    }
}

/// Dynamic element states used by the first Cambium selector lane.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StatePseudoClass {
    Hover,
    Active,
    Focus,
    FocusWithin,
    Disabled,
    Checked,
}

impl ToCss for StatePseudoClass {
    fn to_css<W>(&self, destination: &mut W) -> fmt::Result
    where
        W: fmt::Write,
    {
        destination.write_str(match self {
            Self::Hover => ":hover",
            Self::Active => ":active",
            Self::Focus => ":focus",
            Self::FocusWithin => ":focus-within",
            Self::Disabled => ":disabled",
            Self::Checked => ":checked",
        })
    }
}

impl NonTSPseudoClass for StatePseudoClass {
    type Impl = LiverySelectorImpl;

    fn is_active_or_hover(&self) -> bool {
        matches!(self, Self::Active | Self::Hover)
    }

    fn is_user_action_state(&self) -> bool {
        matches!(
            self,
            Self::Active | Self::Hover | Self::Focus | Self::FocusWithin
        )
    }
}

/// Livery does not style pseudo-elements in its first lane.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NoPseudoElement {}

impl ToCss for NoPseudoElement {
    fn to_css<W>(&self, _destination: &mut W) -> fmt::Result
    where
        W: fmt::Write,
    {
        match *self {}
    }
}

impl PseudoElement for NoPseudoElement {
    type Impl = LiverySelectorImpl;
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiverySelectorImpl;

impl SelectorImpl for LiverySelectorImpl {
    type ExtraMatchingData<'a> = std::marker::PhantomData<&'a ()>;
    type AttrValue = AttributeValue;
    type Identifier = Atom;
    type LocalName = Atom;
    type NamespaceUrl = Atom;
    type NamespacePrefix = Atom;
    type BorrowedLocalName = Atom;
    type BorrowedNamespaceUrl = Atom;
    type NonTSPseudoClass = StatePseudoClass;
    type PseudoElement = NoPseudoElement;
}

#[derive(Default)]
struct LiverySelectorParser;

impl<'i> Parser<'i> for LiverySelectorParser {
    type Impl = LiverySelectorImpl;
    type Error = SelectorParseErrorKind<'i>;

    fn parse_is_and_where(&self) -> bool {
        true
    }

    fn parse_nth_child_of(&self) -> bool {
        true
    }

    /// `:host` and `:host(...)`. The selectors crate implements both against
    /// `MatchingContext::shadow_host`; Livery only has to say yes here.
    fn parse_host(&self) -> bool {
        true
    }

    /// `::slotted(...)`. The crate models it as a jump through
    /// `Element::assigned_slot`, which Livery's adapter now answers.
    fn parse_slotted(&self) -> bool {
        true
    }

    /// `::part(...)`, matched against `Element::is_part` / `imported_part`.
    fn parse_part(&self) -> bool {
        true
    }

    fn parse_non_ts_pseudo_class(
        &self,
        location: cssparser::SourceLocation,
        name: CowRcStr<'i>,
    ) -> Result<StatePseudoClass, cssparser::ParseError<'i, Self::Error>> {
        match_ignore_ascii_case! { &name,
            "hover" => Ok(StatePseudoClass::Hover),
            "active" => Ok(StatePseudoClass::Active),
            "focus" => Ok(StatePseudoClass::Focus),
            "focus-within" => Ok(StatePseudoClass::FocusWithin),
            "disabled" => Ok(StatePseudoClass::Disabled),
            "checked" => Ok(StatePseudoClass::Checked),
            _ => Err(location.new_custom_error(
                SelectorParseErrorKind::UnsupportedPseudoClassOrElement(name)
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectorParseError(String);

impl fmt::Display for SelectorParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SelectorParseError {}

/// A parsed selector list. Matching returns the strongest specificity among
/// the selectors in the list that matched the element.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SelectorDependencies {
    sibling: bool,
    structural: bool,
}

/// Parsed selectors plus the small dependency summary the neutral invalidator
/// needs to choose a sound restyle root. This is deliberately conservative:
/// an uncertain selector expands the scope, never narrows it.
/// How far out of its own tree scope one selector in a list can reach.
/// A stylesheet in a shadow tree styles that tree, but three constructs
/// deliberately cross the boundary, and a document sheet must not reach in
/// except through `::part()`. Classified once at parse time so the cascade's
/// per-element loop is a comparison, not a selector inspection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectorReach {
    /// Ordinary: matches only inside the selector's own tree scope.
    Inner,
    /// `:host` / `:host(...)`: matches the scope's own host element, which
    /// lives in the outer tree.
    Host,
    /// `::slotted(...)`: matches a light-DOM node assigned into this tree.
    Slotted,
    /// `::part(...)`: matches an element inside a shadow tree from outside it.
    Part,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectorList {
    selectors: SubstrateSelectorList<LiverySelectorImpl>,
    dependencies: SelectorDependencies,
    /// Per-selector reach, aligned with `selectors.slice()`.
    reach: Vec<SelectorReach>,
}

impl SelectorList {
    pub fn parse(source: &str) -> Result<Self, SelectorParseError> {
        let mut input_buffer = ParserInput::new(source);
        let mut input = CssParser::new(&mut input_buffer);
        SubstrateSelectorList::parse(&LiverySelectorParser, &mut input, ParseRelative::No)
            .map(|selectors| Self {
                reach: selectors.slice().iter().map(selector_reach).collect(),
                selectors,
                dependencies: selector_dependencies(source),
            })
            .map_err(|error| SelectorParseError(format!("{error:?}")))
    }

    /// A changed element may alter the match of one of its following siblings.
    pub fn has_sibling_dependency(&self) -> bool {
        self.dependencies.sibling
    }

    /// Child-list changes may alter `:empty`, positional, or sibling matching.
    pub fn has_structural_dependency(&self) -> bool {
        self.dependencies.structural
    }

    pub fn matching_specificity<E>(&self, element: &E) -> Option<Specificity>
    where
        E: Element<Impl = LiverySelectorImpl>,
    {
        self.matching_specificity_scoped(element, true, None)
    }

    /// Match with a tree-scope boundary.
    ///
    /// `same_scope` says whether this selector list's stylesheet lives in the
    /// same tree scope as `element`. When it does not, only the three
    /// boundary-crossing reaches are offered — that, and nothing about the
    /// selectors themselves, is what makes a shadow tree's styles private and
    /// keeps the document's styles out of it.
    ///
    /// `shadow_host` is the host of the *stylesheet's* scope, which is what
    /// `:host` compares against and what `::part()` climbs from.
    pub fn matching_specificity_scoped<E>(
        &self,
        element: &E,
        same_scope: bool,
        shadow_host: Option<E>,
    ) -> Option<Specificity>
    where
        E: Element<Impl = LiverySelectorImpl>,
    {
        let mut caches = SelectorCaches::default();
        let mut context = MatchingContext::new(
            MatchingMode::Normal,
            None,
            &mut caches,
            QuirksMode::NoQuirks,
            NeedsSelectorFlags::No,
            MatchingForInvalidation::No,
        );
        context.with_shadow_host(shadow_host, |context| {
            self.selectors
                .slice()
                .iter()
                .zip(&self.reach)
                .filter(|(_, reach)| same_scope || **reach != SelectorReach::Inner)
                .filter(|(selector, _)| matches_selector(selector, 0, None, element, context))
                .map(|(selector, _)| Specificity(selector.specificity()))
                .max()
        })
    }

    /// Whether any selector in this list can reach outside its own tree scope.
    /// A rule with none is skipped wholesale across a boundary.
    pub fn crosses_tree_scope(&self) -> bool {
        self.reach
            .iter()
            .any(|reach| *reach != SelectorReach::Inner)
    }
}

/// Classify one selector's reach. `is_slotted` / `is_part` are the crate's own
/// parse-time flags; `:host` is read off the rightmost compound, which is the
/// only place the parser accepts it.
fn selector_reach(selector: &Selector<LiverySelectorImpl>) -> SelectorReach {
    if selector.is_slotted() {
        return SelectorReach::Slotted;
    }
    if selector.is_part() {
        return SelectorReach::Part;
    }
    if selector.iter().any(Component::is_host) {
        return SelectorReach::Host;
    }
    SelectorReach::Inner
}

fn selector_dependencies(input: &str) -> SelectorDependencies {
    let lower = input.to_ascii_lowercase();
    let structural = [
        ":empty",
        ":first-child",
        ":last-child",
        ":only-child",
        ":nth-child",
        ":nth-last-child",
        ":first-of-type",
        ":last-of-type",
        ":only-of-type",
        ":nth-of-type",
        ":nth-last-of-type",
    ]
    .iter()
    .any(|pseudo| lower.contains(pseudo));

    let mut quote = None;
    let mut escaped = false;
    let mut brackets = 0_u32;
    let mut sibling = false;
    for ch in input.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if ch == delimiter {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '[' => brackets = brackets.saturating_add(1),
            ']' => brackets = brackets.saturating_sub(1),
            '+' | '~' if brackets == 0 => sibling = true,
            _ => {},
        }
    }

    SelectorDependencies {
        sibling,
        structural: structural || sibling,
    }
}
