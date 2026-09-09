// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Style rules joining selectors, media conditions, declarations, and cascade.

use std::{error::Error, fmt};

use crate::ComputedValues;
use crate::cascade::{
    CascadeLayer, DeclarationBlock, MatchedCustomDeclaration, MatchedDeclaration, Origin,
    cascade_with_custom, parse_declaration_block,
};
use crate::custom::CustomProperties;
use crate::media::{Device, MediaParseError, MediaQueryList};
use crate::selector::{Element, SelectorList, SelectorParseError};
use crate::values::{
    ContainerType, FontFeatureSettings, LengthPercentage, RelativeLengthEnvironment, WritingMode,
};

/// A recoverable stylesheet parse diagnostic. Invalid rules are dropped while
/// later rules continue parsing, matching CSS's rule-level recovery model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StylesheetDiagnostic {
    pub prelude: String,
    pub message: String,
}

/// One top-level rule in a sheet's CSSOM-shaped object model (harvest H3,
/// the fork's `stylesheets/` CssRule shape sized to the lane). The
/// flattened style-rule and keyframes views are derived caches; mutation
/// goes through [`Stylesheet::insert_rule`] / [`Stylesheet::delete_rule`]
/// and reindexes them.
#[derive(Clone, Debug, PartialEq)]
pub enum CssRule {
    Style(StyleRule),
    FontFace(FontFaceRule),
    Media(MediaRule),
    Container(ContainerRule),
    Keyframes(Keyframes),
}

/// The rule kinds Livery can project through a CSSOM consumer. Import rules
/// are added by the document-resource graph above this parser, because their
/// child ownership and loading are not stylesheet-parser concerns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CssomRuleKind {
    Style,
    Import,
    FontFace,
    Media,
    Container,
    Keyframes,
    Keyframe,
}

impl CssomRule {
    /// Construct the resource-graph-owned import entry which precedes parsed
    /// rules in a document stylesheet's CSSOM list.
    pub fn import(href: &str, media: Option<&str>) -> Self {
        let escaped_href = href.replace('"', "\\\"");
        let media = media.filter(|media| !media.trim().is_empty());
        Self {
            kind: CssomRuleKind::Import,
            css_text: format!(
                "@import url(\"{escaped_href}\"){};",
                media.map_or_else(String::new, |media| format!(" {media}"))
            ),
            selector_text: None,
            style_text: None,
            condition_text: media.map(str::to_owned),
            name: None,
            key_text: None,
            children: Vec::new(),
        }
    }
}

/// A serializable CSSOM-facing view of one parsed Livery rule. It carries
/// authored source slices for the supported rule types, while the parsed rule
/// objects remain the cascade authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CssomRule {
    pub kind: CssomRuleKind,
    pub css_text: String,
    pub selector_text: Option<String>,
    pub style_text: Option<String>,
    pub condition_text: Option<String>,
    pub name: Option<String>,
    pub key_text: Option<String>,
    pub children: Vec<CssomRule>,
}

impl CssRule {
    pub fn cssom_rule(&self) -> CssomRule {
        match self {
            Self::Style(rule) => rule.cssom_rule(),
            Self::FontFace(rule) => rule.cssom_rule(),
            Self::Media(rule) => {
                let children = rule
                    .rules
                    .iter()
                    .map(StyleRule::cssom_rule)
                    .collect::<Vec<_>>();
                let css_text = format!(
                    "@media {} {{ {} }}",
                    rule.condition,
                    children
                        .iter()
                        .map(|child| child.css_text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                CssomRule {
                    kind: CssomRuleKind::Media,
                    css_text,
                    selector_text: None,
                    style_text: None,
                    condition_text: Some(rule.condition.clone()),
                    name: None,
                    key_text: None,
                    children,
                }
            },
            Self::Container(rule) => {
                let children = rule
                    .rules
                    .iter()
                    .map(StyleRule::cssom_rule)
                    .collect::<Vec<_>>();
                let css_text = format!(
                    "@container {} {{ {} }}",
                    rule.query.css_text(),
                    children
                        .iter()
                        .map(|child| child.css_text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                CssomRule {
                    kind: CssomRuleKind::Container,
                    css_text,
                    selector_text: None,
                    style_text: None,
                    condition_text: Some(rule.query.css_text().to_owned()),
                    name: rule.query.name.clone(),
                    key_text: None,
                    children,
                }
            },
            Self::Keyframes(rule) => rule.cssom_rule(),
        }
    }
}

/// The bounded `@font-face` descriptors consumed by Genet-Livery's host font
/// bridge. Source fetching remains outside the stylesheet parser.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontFaceRule {
    family: Box<str>,
    sources: Box<[Box<str>]>,
    feature_settings: FontFeatureSettings,
    host_loadable: bool,
}

impl FontFaceRule {
    pub fn family(&self) -> &str {
        &self.family
    }

    pub fn sources(&self) -> &[Box<str>] {
        &self.sources
    }

    /// Return this descriptor with each source URL rewritten for a
    /// consuming document. The family, feature settings, and loadability
    /// contract remain unchanged, so callers can keep raw CSSOM text while
    /// using source-relative resource identities for font registration.
    pub fn remap_source_urls(mut self, mut remap: impl FnMut(&str) -> String) -> Self {
        self.sources = self
            .sources
            .iter()
            .map(|source| remap(source).into_boxed_str())
            .collect();
        self
    }

    pub fn feature_settings(&self) -> &FontFeatureSettings {
        &self.feature_settings
    }

    /// Whether Genet-Livery can register this face without ignoring a
    /// descriptor that changes face selection or metrics.
    pub fn is_host_loadable(&self) -> bool {
        self.host_loadable
    }

    fn cssom_rule(&self) -> CssomRule {
        let family = if self.family.contains(char::is_whitespace) {
            format!("\"{}\"", self.family)
        } else {
            self.family.to_string()
        };
        let sources = self
            .sources
            .iter()
            .map(|source| format!("url(\"{source}\")"))
            .collect::<Vec<_>>()
            .join(", ");
        let features = if matches!(&self.feature_settings, FontFeatureSettings::Normal) {
            String::new()
        } else {
            format!(" font-feature-settings: {};", self.feature_settings)
        };
        let style_text = format!("font-family: {family}; src: {sources};{features}");
        CssomRule {
            kind: CssomRuleKind::FontFace,
            css_text: format!("@font-face {{ {style_text} }}"),
            selector_text: None,
            style_text: Some(style_text),
            condition_text: None,
            name: Some(self.family.to_string()),
            key_text: None,
            children: Vec::new(),
        }
    }
}

/// A top-level `@media` group holding its nested style rules. Each nested
/// rule also carries the condition itself, so the flattened cascade view
/// stays self-contained.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaRule {
    condition: String,
    rules: Vec<StyleRule>,
}

impl MediaRule {
    pub fn condition(&self) -> &str {
        &self.condition
    }

    pub fn rules(&self) -> &[StyleRule] {
        &self.rules
    }
}

/// A top-level `@container` group with a parsed boolean size query and nested
/// style rules.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerRule {
    query: ContainerQuery,
    rules: Vec<StyleRule>,
}

impl ContainerRule {
    pub fn query(&self) -> &ContainerQuery {
        &self.query
    }

    pub fn rules(&self) -> &[StyleRule] {
        &self.rules
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContainerAxis {
    Width,
    Height,
    Inline,
    Block,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContainerComparison {
    Less,
    LessEqual,
    Equal,
    GreaterEqual,
    Greater,
}

#[derive(Clone, Debug, PartialEq)]
struct ContainerFeature {
    axis: ContainerAxis,
    comparison: ContainerComparison,
    value: LengthPercentage,
}

#[derive(Clone, Debug, PartialEq)]
enum ContainerCondition {
    Feature(ContainerFeature),
    Not(Box<ContainerCondition>),
    And(Vec<ContainerCondition>),
    Or(Vec<ContainerCondition>),
}

/// Parsed name and boolean size condition for one `@container` group.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerQuery {
    name: Option<String>,
    condition: ContainerCondition,
    css_text: String,
}

impl ContainerQuery {
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The accepted `@container` prelude, retained for CSSOM projection.
    pub fn css_text(&self) -> &str {
        &self.css_text
    }

    pub fn matches(&self, containers: &[ContainerSnapshot], device: &Device) -> bool {
        let required = required_container_axes(&self.condition);
        let Some(container) = containers.iter().find(|container| {
            self.name
                .as_ref()
                .is_none_or(|name| container.names.iter().any(|candidate| candidate == name))
                && container.supports(required)
        }) else {
            return false;
        };
        self.condition.matches(container, device)
    }
}

/// One laid-out query-container ancestor, ordered nearest first for a
/// descendant rule match.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerSnapshot {
    pub names: Vec<String>,
    pub container_type: ContainerType,
    pub writing_mode: WritingMode,
    pub width: f32,
    pub height: f32,
    pub inline_size: f32,
    pub block_size: f32,
}

impl ContainerSnapshot {
    fn supports(&self, axes: u8) -> bool {
        if self.container_type == ContainerType::Size {
            return true;
        }
        if self.container_type != ContainerType::InlineSize {
            return false;
        }
        let inline_physical = if self.writing_mode.is_vertical() {
            axis_bit(ContainerAxis::Height)
        } else {
            axis_bit(ContainerAxis::Width)
        };
        let supported = axis_bit(ContainerAxis::Inline) | inline_physical;
        axes & !supported == 0
    }
}

const fn axis_bit(axis: ContainerAxis) -> u8 {
    1 << axis as u8
}

fn required_container_axes(condition: &ContainerCondition) -> u8 {
    match condition {
        ContainerCondition::Feature(feature) => axis_bit(feature.axis),
        ContainerCondition::Not(condition) => required_container_axes(condition),
        ContainerCondition::And(conditions) | ContainerCondition::Or(conditions) => {
            conditions.iter().fold(0, |axes, condition| {
                axes | required_container_axes(condition)
            })
        },
    }
}

impl ContainerCondition {
    fn matches(&self, container: &ContainerSnapshot, device: &Device) -> bool {
        match self {
            Self::Feature(feature) => feature.matches(container, device),
            Self::Not(condition) => !condition.matches(container, device),
            Self::And(conditions) => conditions
                .iter()
                .all(|condition| condition.matches(container, device)),
            Self::Or(conditions) => conditions
                .iter()
                .any(|condition| condition.matches(container, device)),
        }
    }
}

impl ContainerFeature {
    fn matches(&self, container: &ContainerSnapshot, device: &Device) -> bool {
        let actual = match self.axis {
            ContainerAxis::Width => container.width,
            ContainerAxis::Height => container.height,
            ContainerAxis::Inline => container.inline_size,
            ContainerAxis::Block => container.block_size,
        };
        let expected = self
            .value
            .resolve_relative(RelativeLengthEnvironment::viewport(device.viewport_sizes))
            .resolve_font_relative(16.0, 16.0)
            .to_px(16.0, 16.0, 0.0);
        match self.comparison {
            ContainerComparison::Less => actual < expected,
            ContainerComparison::LessEqual => actual <= expected,
            ContainerComparison::Equal => (actual - expected).abs() < 0.001,
            ContainerComparison::GreaterEqual => actual >= expected,
            ContainerComparison::Greater => actual > expected,
        }
    }
}

/// Why a CSSOM mutation was rejected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuleMutationError {
    /// The index is past the end (insert) or out of range (delete).
    IndexSize,
    /// The rule text did not parse to exactly one supported top-level rule.
    Syntax(String),
}

impl fmt::Display for RuleMutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IndexSize => formatter.write_str("rule index out of range"),
            Self::Syntax(message) => write!(formatter, "invalid rule: {message}"),
        }
    }
}

impl Error for RuleMutationError {}

/// A parsed rule sheet for the bounded Livery lane.
#[derive(Clone, Debug, PartialEq)]
pub struct Stylesheet {
    items: Vec<CssRule>,
    origin: Origin,
    source_order_offset: u64,
    generation: u64,
    rules: Vec<StyleRule>,
    font_faces: Vec<FontFaceRule>,
    keyframes: Vec<Keyframes>,
    diagnostics: Vec<StylesheetDiagnostic>,
}

impl Default for Stylesheet {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            origin: Origin::Author,
            source_order_offset: 0,
            generation: 0,
            rules: Vec::new(),
            font_faces: Vec::new(),
            keyframes: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

impl Stylesheet {
    /// Parse style rules and top-level `@media` groups. Other at-rules are
    /// skipped as unsupported lane input and retained as diagnostics.
    pub fn parse(input: &str, origin: Origin) -> Self {
        Self::parse_with_offset(input, origin, 0)
    }

    /// Apply the `media` attribute carried by an HTML stylesheet link without
    /// rewriting the stylesheet text into a synthetic `@media` block. Nested
    /// CSS media conditions remain independent and both gates must match.
    pub fn with_document_media(mut self, media: Option<&str>) -> Self {
        let Some(media) = media.filter(|media| !media.trim().is_empty()) else {
            return self;
        };
        match media.parse::<MediaQueryList>() {
            Ok(media) => apply_document_media(&mut self.items, &media),
            Err(error) => self.diagnostics.push(StylesheetDiagnostic {
                prelude: media.to_owned(),
                message: format!("invalid stylesheet link media: {error}"),
            }),
        }
        self.reindex();
        self
    }

    /// Parse a sheet whose first rule follows `source_order` rules already
    /// loaded at the same origin.
    pub fn parse_with_offset(input: &str, origin: Origin, source_order: u64) -> Self {
        let clean = without_comments(input);
        let mut sheet = Self {
            origin,
            source_order_offset: source_order,
            ..Self::default()
        };
        parse_rule_list(&clean, origin, source_order, &mut sheet);
        sheet.reindex();
        sheet
    }

    /// The ordered top-level object model.
    pub fn items(&self) -> &[CssRule] {
        &self.items
    }

    /// A monotonic mutation stamp: consumers holding derived state compare
    /// it to know when to rebuild (the StylesheetSet dirty-tracking shape).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Insert one parsed top-level rule at `index`, CSSOM `insertRule`
    /// semantics: later rules shift, the flattened cascade view reindexes,
    /// and the returned value is the index. Rejects malformed input whole.
    pub fn insert_rule(&mut self, rule: &str, index: usize) -> Result<usize, RuleMutationError> {
        if index > self.items.len() {
            return Err(RuleMutationError::IndexSize);
        }
        let clean = without_comments(rule);
        let mut scratch = Self {
            origin: self.origin,
            ..Self::default()
        };
        parse_rule_list(&clean, self.origin, 0, &mut scratch);
        if let Some(diagnostic) = scratch.diagnostics.first() {
            return Err(RuleMutationError::Syntax(diagnostic.message.clone()));
        }
        if scratch.items.len() != 1 {
            return Err(RuleMutationError::Syntax(
                "expected exactly one rule".to_owned(),
            ));
        }
        self.items.insert(index, scratch.items.remove(0));
        self.reindex();
        Ok(index)
    }

    /// Remove the top-level rule at `index`, CSSOM `deleteRule` semantics.
    pub fn delete_rule(&mut self, index: usize) -> Result<(), RuleMutationError> {
        if index >= self.items.len() {
            return Err(RuleMutationError::IndexSize);
        }
        self.items.remove(index);
        self.reindex();
        Ok(())
    }

    /// Rebuild the flattened caches from the object model, renumbering
    /// source order exactly as a fresh parse would.
    fn reindex(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.rules.clear();
        self.font_faces.clear();
        self.keyframes.clear();
        for item in &self.items {
            match item {
                CssRule::Style(rule) => {
                    let mut rule = rule.clone();
                    rule.source_order = self
                        .source_order_offset
                        .saturating_add(self.rules.len() as u64);
                    self.rules.push(rule);
                },
                CssRule::FontFace(font_face) => self.font_faces.push(font_face.clone()),
                CssRule::Media(media) => {
                    for rule in &media.rules {
                        let mut rule = rule.clone();
                        rule.source_order = self
                            .source_order_offset
                            .saturating_add(self.rules.len() as u64);
                        self.rules.push(rule);
                    }
                },
                CssRule::Container(container) => {
                    for rule in &container.rules {
                        let mut rule = rule.clone();
                        rule.source_order = self
                            .source_order_offset
                            .saturating_add(self.rules.len() as u64);
                        self.rules.push(rule);
                    }
                },
                CssRule::Keyframes(keyframes) => self.keyframes.push(keyframes.clone()),
            }
        }
    }

    /// The flattened style rules with source order rebased at `offset`, for
    /// consumers composing several sheets into one cascade list.
    pub fn reindexed_rules(&self, offset: u64) -> Vec<StyleRule> {
        self.rules
            .iter()
            .enumerate()
            .map(|(index, rule)| {
                let mut rule = rule.clone();
                rule.source_order = offset.saturating_add(index as u64);
                rule
            })
            .collect()
    }

    pub fn rules(&self) -> &[StyleRule] {
        &self.rules
    }

    pub fn diagnostics(&self) -> &[StylesheetDiagnostic] {
        &self.diagnostics
    }

    pub fn font_faces(&self) -> &[FontFaceRule] {
        &self.font_faces
    }

    pub fn keyframes(&self) -> &[Keyframes] {
        &self.keyframes
    }

    pub fn into_rules(self) -> Vec<StyleRule> {
        self.rules
    }

    pub fn into_keyframes(self) -> Vec<Keyframes> {
        self.keyframes
    }
}

fn apply_document_media(items: &mut [CssRule], media: &MediaQueryList) {
    for item in items {
        match item {
            CssRule::Style(rule) => rule.document_media = Some(media.clone()),
            CssRule::FontFace(_) => {},
            CssRule::Media(group) => {
                for rule in &mut group.rules {
                    rule.document_media = Some(media.clone());
                }
            },
            CssRule::Container(group) => {
                for rule in &mut group.rules {
                    rule.document_media = Some(media.clone());
                }
            },
            CssRule::Keyframes(_) => {},
        }
    }
}

/// One named keyframe block. The retained animation clock samples direct
/// longhand values from these frames through generated property dispatch.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframes {
    name: Box<str>,
    frames: Vec<Keyframe>,
}

impl Keyframes {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn frames(&self) -> &[Keyframe] {
        &self.frames
    }

    fn cssom_rule(&self) -> CssomRule {
        let children = self
            .frames
            .iter()
            .map(Keyframe::cssom_rule)
            .collect::<Vec<_>>();
        CssomRule {
            kind: CssomRuleKind::Keyframes,
            css_text: format!(
                "@keyframes {} {{ {} }}",
                self.name,
                children
                    .iter()
                    .map(|child| child.css_text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            selector_text: None,
            style_text: None,
            condition_text: None,
            name: Some(self.name.to_string()),
            key_text: None,
            children,
        }
    }
}

/// A keyframe declaration block at a normalized offset in the animation.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    offset: f32,
    declarations: DeclarationBlock,
    key_text: String,
    declaration_text: String,
}

impl Keyframe {
    pub fn offset(&self) -> f32 {
        self.offset
    }

    pub fn declarations(&self) -> &DeclarationBlock {
        &self.declarations
    }

    fn cssom_rule(&self) -> CssomRule {
        CssomRule {
            kind: CssomRuleKind::Keyframe,
            css_text: format!("{} {{ {} }}", self.key_text, self.declaration_text),
            selector_text: None,
            style_text: Some(self.declaration_text.clone()),
            condition_text: None,
            name: None,
            key_text: Some(self.key_text.clone()),
            children: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub enum StyleRuleError {
    Selector(SelectorParseError),
    Media(MediaParseError),
}

impl fmt::Display for StyleRuleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Selector(error) => write!(formatter, "selector: {error}"),
            Self::Media(error) => write!(formatter, "media query: {error}"),
        }
    }
}

impl Error for StyleRuleError {}

/// One style rule in source order. Declaration parse errors remain attached to
/// the rule as diagnostics; valid declarations still participate in cascade.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleRule {
    selectors: SelectorList,
    declarations: DeclarationBlock,
    selector_text: String,
    declaration_text: String,
    media: Option<MediaQueryList>,
    /// A media condition carried by the owning HTML stylesheet link rather
    /// than synthesized into that sheet's source text.
    document_media: Option<MediaQueryList>,
    container: Option<ContainerQuery>,
    origin: Origin,
    layer: CascadeLayer,
    source_order: u64,
    tree_counting: bool,
}

/// Whether any declared value calls a tree-counting function. Their results
/// change for every sibling when a child list changes, and a selector-text
/// scan cannot see them, so this is read off the declaration source.
fn declares_tree_counting(declarations: &str) -> bool {
    let lowered = declarations.to_ascii_lowercase();
    lowered.contains("sibling-index(") || lowered.contains("sibling-count(")
}

impl StyleRule {
    pub fn parse(
        selectors: &str,
        declarations: &str,
        media: Option<&str>,
        origin: Origin,
        layer: CascadeLayer,
        source_order: u64,
    ) -> Result<Self, StyleRuleError> {
        Ok(Self {
            selectors: SelectorList::parse(selectors).map_err(StyleRuleError::Selector)?,
            declarations: parse_declaration_block(declarations),
            selector_text: selectors.trim().to_owned(),
            declaration_text: declarations.trim().to_owned(),
            tree_counting: declares_tree_counting(declarations),
            media: media
                .map(str::parse)
                .transpose()
                .map_err(StyleRuleError::Media)?,
            document_media: None,
            container: None,
            origin,
            layer,
            source_order,
        })
    }

    pub fn declaration_block(&self) -> &DeclarationBlock {
        &self.declarations
    }

    pub fn selector_text(&self) -> &str {
        &self.selector_text
    }

    pub fn declaration_text(&self) -> &str {
        &self.declaration_text
    }

    fn cssom_rule(&self) -> CssomRule {
        CssomRule {
            kind: CssomRuleKind::Style,
            css_text: format!("{} {{ {} }}", self.selector_text, self.declaration_text),
            selector_text: Some(self.selector_text.clone()),
            style_text: Some(self.declaration_text.clone()),
            condition_text: None,
            name: None,
            key_text: None,
            children: Vec::new(),
        }
    }

    /// Whether changing this rule's candidate element can affect a following
    /// sibling. Used only to widen invalidation scope.
    pub fn has_sibling_dependency(&self) -> bool {
        self.selectors.has_sibling_dependency()
    }

    /// Whether child-list changes can alter this rule's result — either
    /// through a structural selector, or through a tree-counting function in
    /// a declared value.
    pub fn has_structural_dependency(&self) -> bool {
        self.selectors.has_structural_dependency() || self.tree_counting
    }

    pub fn has_container_query(&self) -> bool {
        self.container.is_some()
    }

    /// The rule's flattened cascade position, renumbered by the owning
    /// sheet after every parse or mutation.
    pub fn source_order(&self) -> u64 {
        self.source_order
    }

    /// The cascade origin this rule came from. The shadow tree-scope boundary
    /// applies to author rules only: UA rules style every tree.
    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// Whether any selector in this rule can reach out of its own tree scope
    /// (`:host`, `::slotted()`, `::part()`).
    pub fn crosses_tree_scope(&self) -> bool {
        self.selectors.crosses_tree_scope()
    }

    pub fn matched_declarations<E>(&self, element: &E, device: &Device) -> Vec<MatchedDeclaration>
    where
        E: Element<Impl = crate::selector::LiverySelectorImpl>,
    {
        self.matched_declarations_with_containers(element, device, &[])
    }

    pub fn matched_declarations_with_containers<E>(
        &self,
        element: &E,
        device: &Device,
        containers: &[ContainerSnapshot],
    ) -> Vec<MatchedDeclaration>
    where
        E: Element<Impl = crate::selector::LiverySelectorImpl>,
    {
        self.matched_declarations_scoped(element, device, containers, true, None)
    }

    /// [`matched_declarations_with_containers`](Self::matched_declarations_with_containers)
    /// with the shadow tree-scope boundary applied. See
    /// [`SelectorList::matching_specificity_scoped`](crate::selector::SelectorList::matching_specificity_scoped).
    pub fn matched_declarations_scoped<E>(
        &self,
        element: &E,
        device: &Device,
        containers: &[ContainerSnapshot],
        same_scope: bool,
        shadow_host: Option<E>,
    ) -> Vec<MatchedDeclaration>
    where
        E: Element<Impl = crate::selector::LiverySelectorImpl>,
    {
        if self.declarations.declarations.is_empty() {
            return Vec::new();
        }
        if self
            .media
            .as_ref()
            .is_some_and(|condition| !condition.matches(device))
            || self
                .document_media
                .as_ref()
                .is_some_and(|condition| !condition.matches(device))
            || self
                .container
                .as_ref()
                .is_some_and(|query| !query.matches(containers, device))
        {
            return Vec::new();
        }
        let Some(specificity) =
            self.selectors
                .matching_specificity_scoped(element, same_scope, shadow_host)
        else {
            return Vec::new();
        };
        self.declarations
            .declarations
            .iter()
            .enumerate()
            .map(|(index, declaration)| MatchedDeclaration {
                declaration: declaration.clone(),
                origin: self.origin,
                layer: self.layer,
                specificity,
                source_order: self
                    .source_order
                    .saturating_mul(65_536)
                    .saturating_add(index as u64),
            })
            .collect()
    }

    /// The rule's matched `--name` declarations, with the same media and
    /// selector gate as [`Self::matched_declarations`].
    pub fn matched_custom_declarations<E>(
        &self,
        element: &E,
        device: &Device,
    ) -> Vec<MatchedCustomDeclaration>
    where
        E: Element<Impl = crate::selector::LiverySelectorImpl>,
    {
        self.matched_custom_declarations_with_containers(element, device, &[])
    }

    pub fn matched_custom_declarations_with_containers<E>(
        &self,
        element: &E,
        device: &Device,
        containers: &[ContainerSnapshot],
    ) -> Vec<MatchedCustomDeclaration>
    where
        E: Element<Impl = crate::selector::LiverySelectorImpl>,
    {
        self.matched_custom_declarations_scoped(element, device, containers, true, None)
    }

    /// Scoped twin of
    /// [`matched_custom_declarations_with_containers`](Self::matched_custom_declarations_with_containers).
    pub fn matched_custom_declarations_scoped<E>(
        &self,
        element: &E,
        device: &Device,
        containers: &[ContainerSnapshot],
        same_scope: bool,
        shadow_host: Option<E>,
    ) -> Vec<MatchedCustomDeclaration>
    where
        E: Element<Impl = crate::selector::LiverySelectorImpl>,
    {
        if self.declarations.custom.is_empty() {
            return Vec::new();
        }
        if self
            .media
            .as_ref()
            .is_some_and(|condition| !condition.matches(device))
            || self
                .document_media
                .as_ref()
                .is_some_and(|condition| !condition.matches(device))
            || self
                .container
                .as_ref()
                .is_some_and(|query| !query.matches(containers, device))
        {
            return Vec::new();
        }
        let Some(specificity) =
            self.selectors
                .matching_specificity_scoped(element, same_scope, shadow_host)
        else {
            return Vec::new();
        };
        self.declarations
            .custom
            .iter()
            .enumerate()
            .map(|(index, declaration)| MatchedCustomDeclaration {
                declaration: declaration.clone(),
                origin: self.origin,
                layer: self.layer,
                specificity,
                source_order: self
                    .source_order
                    .saturating_mul(65_536)
                    .saturating_add(index as u64),
            })
            .collect()
    }
}

/// Match and cascade a hand-built rule corpus for one element.
pub fn cascade_rules<E>(
    parent: Option<&ComputedValues>,
    element: &E,
    device: &Device,
    rules: &[StyleRule],
) -> ComputedValues
where
    E: Element<Impl = crate::selector::LiverySelectorImpl>,
{
    cascade_rules_with_custom(parent, None, element, device, rules).0
}

/// [`cascade_rules`] with custom properties threaded from the parent and
/// returned alongside the computed style.
pub fn cascade_rules_with_custom<E>(
    parent: Option<&ComputedValues>,
    parent_custom: Option<&CustomProperties>,
    element: &E,
    device: &Device,
    rules: &[StyleRule],
) -> (ComputedValues, CustomProperties)
where
    E: Element<Impl = crate::selector::LiverySelectorImpl>,
{
    cascade_with_custom(
        parent,
        parent_custom,
        rules
            .iter()
            .flat_map(|rule| rule.matched_declarations(element, device)),
        rules
            .iter()
            .flat_map(|rule| rule.matched_custom_declarations(element, device)),
    )
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

fn find_open_brace(input: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    for (offset, ch) in input[start..].char_indices() {
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
            '{' => return Some(start + offset),
            ';' => return None,
            _ => {},
        }
    }
    None
}

fn find_close_brace(input: &str, open: usize) -> Option<usize> {
    let mut depth = 1_u32;
    let mut quote = None;
    let mut escaped = false;
    for (offset, ch) in input[open + 1..].char_indices() {
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
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + 1 + offset);
                }
            },
            _ => {},
        }
    }
    None
}

fn keyframes_name(prelude: &str) -> Option<&str> {
    let mut parts = prelude.split_whitespace();
    let at_rule = parts.next()?;
    if !(at_rule.eq_ignore_ascii_case("@keyframes")
        || at_rule.eq_ignore_ascii_case("@-webkit-keyframes"))
    {
        return None;
    }
    let name = parts.next()?.trim();
    (parts.next().is_none() && !name.is_empty()).then_some(name)
}

fn keyframe_offset(prelude: &str) -> Option<f32> {
    match prelude.trim().to_ascii_lowercase().as_str() {
        "from" => Some(0.0),
        "to" => Some(1.0),
        value => value
            .strip_suffix('%')
            .and_then(|value| value.trim().parse::<f32>().ok())
            .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
            .map(|value| value / 100.0),
    }
}

fn parse_keyframes(name: &str, input: &str, sheet: &mut Stylesheet) -> Option<Keyframes> {
    let mut frames = Vec::new();
    let mut cursor = 0;
    while cursor < input.len() {
        let Some(non_space) = input[cursor..]
            .char_indices()
            .find(|(_, ch)| !ch.is_whitespace())
            .map(|(offset, _)| cursor + offset)
        else {
            break;
        };
        cursor = non_space;
        let Some(open) = find_open_brace(input, cursor) else {
            let tail = input[cursor..].trim();
            if !tail.is_empty() {
                sheet.diagnostics.push(StylesheetDiagnostic {
                    prelude: tail.to_owned(),
                    message: "expected a keyframe block".to_owned(),
                });
            }
            break;
        };
        let prelude = input[cursor..open].trim();
        let Some(close) = find_close_brace(input, open) else {
            sheet.diagnostics.push(StylesheetDiagnostic {
                prelude: prelude.to_owned(),
                message: "unclosed keyframe block".to_owned(),
            });
            break;
        };
        let body = &input[open + 1..close];
        let offsets = prelude
            .split(',')
            .filter_map(keyframe_offset)
            .collect::<Vec<_>>();
        if offsets.is_empty() {
            sheet.diagnostics.push(StylesheetDiagnostic {
                prelude: prelude.to_owned(),
                message: "invalid keyframe selector".to_owned(),
            });
        } else {
            let declarations = parse_declaration_block(body);
            for offset in offsets {
                frames.push(Keyframe {
                    offset,
                    declarations: declarations.clone(),
                    key_text: prelude.to_owned(),
                    declaration_text: body.trim().to_owned(),
                });
            }
        }
        cursor = close + 1;
    }
    if frames.is_empty() {
        return None;
    }
    frames.sort_by(|left, right| left.offset.total_cmp(&right.offset));
    Some(Keyframes {
        name: name.into(),
        frames,
    })
}

fn media_condition(prelude: &str) -> Option<&str> {
    prelude
        .get(..6)
        .filter(|prefix| prefix.eq_ignore_ascii_case("@media"))
        .and_then(|_| prelude.get(6..))
        .filter(|rest| {
            rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with('(')
        })
        .map(str::trim)
}

fn container_prelude(prelude: &str) -> Option<&str> {
    prelude
        .get(..10)
        .filter(|prefix| prefix.eq_ignore_ascii_case("@container"))
        .and_then(|_| prelude.get(10..))
        .filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        .map(str::trim)
}

fn parse_container_query(source: &str) -> Result<ContainerQuery, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err("empty container query".to_owned());
    }
    let (name, condition) = if source.starts_with('(')
        || source
            .get(..3)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("not"))
    {
        (None, source)
    } else {
        let Some(split) = source.find(char::is_whitespace) else {
            return Err("container query needs a condition".to_owned());
        };
        let name = source[..split].trim();
        let parsed = name
            .parse::<crate::values::ContainerName>()
            .map_err(|error| error.to_string())?;
        if !matches!(parsed, crate::values::ContainerName::Names(ref names) if names.len() == 1) {
            return Err("container query name must be one custom identifier".to_owned());
        }
        (Some(name.to_owned()), source[split..].trim())
    };
    Ok(ContainerQuery {
        name,
        condition: parse_container_condition(condition)?,
        css_text: source.to_owned(),
    })
}

fn parse_container_condition(source: &str) -> Result<ContainerCondition, String> {
    let source = source.trim();
    let parts = split_top_level_keyword(source, "or");
    if parts.len() > 1 {
        return parts
            .into_iter()
            .map(parse_container_condition)
            .collect::<Result<Vec<_>, _>>()
            .map(ContainerCondition::Or);
    }
    let parts = split_top_level_keyword(source, "and");
    if parts.len() > 1 {
        return parts
            .into_iter()
            .map(parse_container_condition)
            .collect::<Result<Vec<_>, _>>()
            .map(ContainerCondition::And);
    }
    if source
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("not"))
        && source[3..].chars().next().is_some_and(char::is_whitespace)
    {
        return Ok(ContainerCondition::Not(Box::new(
            parse_container_condition(source[3..].trim())?,
        )));
    }
    let source = strip_outer_parentheses(source).unwrap_or(source);
    let nested_or = split_top_level_keyword(source, "or");
    let nested_and = split_top_level_keyword(source, "and");
    if nested_or.len() > 1 || nested_and.len() > 1 {
        return parse_container_condition(source);
    }
    parse_container_feature(source)
}

fn split_top_level_keyword<'a>(source: &'a str, keyword: &str) -> Vec<&'a str> {
    let mut depth = 0_i32;
    let mut start = 0;
    let mut parts = Vec::new();
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {},
        }
        if depth == 0
            && index > 0
            && bytes[index - 1].is_ascii_whitespace()
            && source[index..]
                .get(..keyword.len())
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(keyword))
            && bytes
                .get(index + keyword.len())
                .is_some_and(u8::is_ascii_whitespace)
        {
            parts.push(source[start..index].trim());
            index += keyword.len();
            start = index;
            continue;
        }
        index += 1;
    }
    if parts.is_empty() {
        vec![source]
    } else {
        parts.push(source[start..].trim());
        parts
    }
}

fn strip_outer_parentheses(source: &str) -> Option<&str> {
    if !source.starts_with('(') || !source.ends_with(')') {
        return None;
    }
    let mut depth = 0_i32;
    for (index, character) in source.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 && index + 1 != source.len() {
                    return None;
                }
            },
            _ => {},
        }
        if depth < 0 {
            return None;
        }
    }
    (depth == 0).then(|| source[1..source.len() - 1].trim())
}

fn parse_container_feature(source: &str) -> Result<ContainerCondition, String> {
    let source = source.trim();
    let tokens = source.split_ascii_whitespace().collect::<Vec<_>>();
    if tokens.len() == 5 {
        let first_value = parse_query_length(tokens[0])?;
        let first_comparison = parse_comparison_token(tokens[1])?;
        let axis = parse_container_axis(tokens[2])?;
        let second_comparison = parse_comparison_token(tokens[3])?;
        let second_value = parse_query_length(tokens[4])?;
        return Ok(ContainerCondition::And(vec![
            ContainerCondition::Feature(ContainerFeature {
                axis,
                comparison: invert_comparison(first_comparison),
                value: first_value,
            }),
            ContainerCondition::Feature(ContainerFeature {
                axis,
                comparison: second_comparison,
                value: second_value,
            }),
        ]));
    }
    if let Some((name, value)) = source.split_once(':') {
        let name = name.trim().to_ascii_lowercase();
        let (comparison, axis_name) = if let Some(axis) = name.strip_prefix("min-") {
            (ContainerComparison::GreaterEqual, axis)
        } else if let Some(axis) = name.strip_prefix("max-") {
            (ContainerComparison::LessEqual, axis)
        } else {
            (ContainerComparison::Equal, name.as_str())
        };
        return Ok(ContainerCondition::Feature(ContainerFeature {
            axis: parse_container_axis(axis_name)?,
            comparison,
            value: parse_query_length(value)?,
        }));
    }
    for token in [">=", "<=", ">", "<", "="] {
        if let Some((left, right)) = split_top_level_operator(source, token) {
            let comparison = parse_comparison_token(token)?;
            if let Ok(axis) = parse_container_axis(left.trim()) {
                return Ok(ContainerCondition::Feature(ContainerFeature {
                    axis,
                    comparison,
                    value: parse_query_length(right)?,
                }));
            }
            return Ok(ContainerCondition::Feature(ContainerFeature {
                axis: parse_container_axis(right.trim())?,
                comparison: invert_comparison(comparison),
                value: parse_query_length(left)?,
            }));
        }
    }
    Err("unsupported container size feature".to_owned())
}

fn split_top_level_operator<'a>(source: &'a str, operator: &str) -> Option<(&'a str, &'a str)> {
    let mut depth = 0_i32;
    for (index, character) in source.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ if depth == 0 && source[index..].starts_with(operator) => {
                return Some((&source[..index], &source[index + operator.len()..]));
            },
            _ => {},
        }
    }
    None
}

fn parse_container_axis(source: &str) -> Result<ContainerAxis, String> {
    match source.trim().to_ascii_lowercase().as_str() {
        "width" => Ok(ContainerAxis::Width),
        "height" => Ok(ContainerAxis::Height),
        "inline-size" => Ok(ContainerAxis::Inline),
        "block-size" => Ok(ContainerAxis::Block),
        _ => Err("unsupported container axis".to_owned()),
    }
}

fn parse_comparison_token(source: &str) -> Result<ContainerComparison, String> {
    match source.trim() {
        "<" => Ok(ContainerComparison::Less),
        "<=" => Ok(ContainerComparison::LessEqual),
        "=" => Ok(ContainerComparison::Equal),
        ">=" => Ok(ContainerComparison::GreaterEqual),
        ">" => Ok(ContainerComparison::Greater),
        _ => Err("invalid container comparison".to_owned()),
    }
}

const fn invert_comparison(comparison: ContainerComparison) -> ContainerComparison {
    match comparison {
        ContainerComparison::Less => ContainerComparison::Greater,
        ContainerComparison::LessEqual => ContainerComparison::GreaterEqual,
        ContainerComparison::Equal => ContainerComparison::Equal,
        ContainerComparison::GreaterEqual => ContainerComparison::LessEqual,
        ContainerComparison::Greater => ContainerComparison::Less,
    }
}

fn parse_query_length(source: &str) -> Result<LengthPercentage, String> {
    let value = source
        .trim()
        .parse::<LengthPercentage>()
        .map_err(|error| error.to_string())?;
    if value.has_percentage() {
        return Err("container query size cannot be a percentage".to_owned());
    }
    Ok(value)
}

/// Parse a top-level rule list into the sheet's object model. Source order
/// is provisional here; [`Stylesheet::reindex`] renumbers the flattened
/// view after every parse or mutation.
fn parse_rule_list(input: &str, origin: Origin, source_order_offset: u64, sheet: &mut Stylesheet) {
    let mut cursor = 0;
    while cursor < input.len() {
        let Some(non_space) = input[cursor..]
            .char_indices()
            .find(|(_, ch)| !ch.is_whitespace())
            .map(|(offset, _)| cursor + offset)
        else {
            break;
        };
        cursor = non_space;

        let Some(open) = find_open_brace(input, cursor) else {
            let tail = input[cursor..].trim();
            if !tail.is_empty() {
                sheet.diagnostics.push(StylesheetDiagnostic {
                    prelude: tail.to_owned(),
                    message: "expected a rule block".to_owned(),
                });
            }
            break;
        };
        let prelude = input[cursor..open].trim();
        let Some(close) = find_close_brace(input, open) else {
            sheet.diagnostics.push(StylesheetDiagnostic {
                prelude: prelude.to_owned(),
                message: "unclosed rule block".to_owned(),
            });
            break;
        };
        let body = &input[open + 1..close];

        if prelude.eq_ignore_ascii_case("@font-face") {
            match parse_font_face(body) {
                Ok(font_face) => sheet.items.push(CssRule::FontFace(font_face)),
                Err(message) => sheet.diagnostics.push(StylesheetDiagnostic {
                    prelude: prelude.to_owned(),
                    message,
                }),
            }
        } else if let Some(name) = keyframes_name(prelude) {
            if let Some(keyframes) = parse_keyframes(name, body, sheet) {
                sheet.items.push(CssRule::Keyframes(keyframes));
            }
        } else if let Some(condition) = media_condition(prelude) {
            if condition.is_empty() {
                sheet.diagnostics.push(StylesheetDiagnostic {
                    prelude: prelude.to_owned(),
                    message: "empty media query".to_owned(),
                });
            } else {
                let rules =
                    parse_media_style_rules(body, origin, condition, source_order_offset, sheet);
                sheet.items.push(CssRule::Media(MediaRule {
                    condition: condition.to_owned(),
                    rules,
                }));
            }
        } else if let Some(condition) = container_prelude(prelude) {
            match parse_container_query(condition) {
                Ok(query) => {
                    let rules = parse_container_style_rules(
                        body,
                        origin,
                        &query,
                        source_order_offset,
                        sheet,
                    );
                    sheet
                        .items
                        .push(CssRule::Container(ContainerRule { query, rules }));
                },
                Err(message) => sheet.diagnostics.push(StylesheetDiagnostic {
                    prelude: prelude.to_owned(),
                    message,
                }),
            }
        } else if prelude.starts_with('@') {
            sheet.diagnostics.push(StylesheetDiagnostic {
                prelude: prelude.to_owned(),
                message: "unsupported at-rule".to_owned(),
            });
        } else {
            let source_order = source_order_offset.saturating_add(sheet.items.len() as u64);
            match StyleRule::parse(
                prelude,
                body,
                None,
                origin,
                CascadeLayer::Unlayered,
                source_order,
            ) {
                Ok(rule) => sheet.items.push(CssRule::Style(rule)),
                Err(error) => sheet.diagnostics.push(StylesheetDiagnostic {
                    prelude: prelude.to_owned(),
                    message: error.to_string(),
                }),
            }
        }
        cursor = close + 1;
    }
}

fn parse_font_face(input: &str) -> Result<FontFaceRule, String> {
    let mut family = None;
    let mut sources = Vec::new();
    let mut feature_settings = FontFeatureSettings::Normal;
    let mut host_loadable = true;
    for declaration in split_descriptor_list(input, ';') {
        let declaration = declaration.trim();
        if declaration.is_empty() {
            continue;
        }
        let parts = split_descriptor_list(declaration, ':');
        let Some(name) = parts.first().map(|part| part.trim().to_ascii_lowercase()) else {
            continue;
        };
        let colon = parts.first().map_or(0, |part| part.len());
        if colon == declaration.len() {
            continue;
        }
        let value = declaration[colon + 1..].trim();
        match name.as_str() {
            "font-family" => {
                if value.is_empty() || value.contains(',') {
                    return Err("@font-face requires one font-family name".to_owned());
                }
                let value = value
                    .strip_prefix('"')
                    .and_then(|value| value.strip_suffix('"'))
                    .or_else(|| {
                        value
                            .strip_prefix('\'')
                            .and_then(|value| value.strip_suffix('\''))
                    })
                    .unwrap_or(value)
                    .trim();
                if value.is_empty() {
                    return Err("@font-face requires a nonempty font-family".to_owned());
                }
                family = Some(Box::<str>::from(value));
            },
            "src" => sources.extend(font_face_urls(value)),
            "font-feature-settings" => {
                feature_settings = value.parse::<FontFeatureSettings>().map_err(|error| {
                    format!("invalid @font-face font-feature-settings: {error}")
                })?;
            },
            _ => host_loadable = false,
        }
    }
    let family = family.ok_or_else(|| "@font-face is missing font-family".to_owned())?;
    if sources.is_empty() {
        return Err("@font-face has no supported url() source".to_owned());
    }
    Ok(FontFaceRule {
        family,
        sources: sources.into_boxed_slice(),
        feature_settings,
        host_loadable,
    })
}

fn font_face_urls(input: &str) -> Vec<Box<str>> {
    let lower = input.to_ascii_lowercase();
    let mut urls: Vec<Box<str>> = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = lower[cursor..].find("url(") {
        let start = cursor + offset + 4;
        let Some(close) = input[start..].find(')') else {
            break;
        };
        let raw = input[start..start + close].trim();
        let url = raw
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                raw.strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(raw)
            .trim();
        if !url.is_empty() && !urls.iter().any(|seen| seen.as_ref() == url) {
            urls.push(Box::<str>::from(url));
        }
        cursor = start + close + 1;
    }
    urls
}

fn split_descriptor_list(input: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in input.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if character == delimiter && depth == 0 => {
                parts.push(&input[start..index]);
                start = index + character.len_utf8();
            },
            _ => {},
        }
    }
    parts.push(&input[start..]);
    parts
}

/// Parse the style rules nested in one `@media` group. Each nested rule
/// carries the condition itself, so the flattened cascade view needs no
/// group context. Nested groups and keyframes stay outside the lane.
fn parse_media_style_rules(
    input: &str,
    origin: Origin,
    condition: &str,
    source_order_offset: u64,
    sheet: &mut Stylesheet,
) -> Vec<StyleRule> {
    let mut rules = Vec::new();
    let mut cursor = 0;
    while cursor < input.len() {
        let Some(non_space) = input[cursor..]
            .char_indices()
            .find(|(_, ch)| !ch.is_whitespace())
            .map(|(offset, _)| cursor + offset)
        else {
            break;
        };
        cursor = non_space;

        let Some(open) = find_open_brace(input, cursor) else {
            let tail = input[cursor..].trim();
            if !tail.is_empty() {
                sheet.diagnostics.push(StylesheetDiagnostic {
                    prelude: tail.to_owned(),
                    message: "expected a rule block".to_owned(),
                });
            }
            break;
        };
        let prelude = input[cursor..open].trim();
        let Some(close) = find_close_brace(input, open) else {
            sheet.diagnostics.push(StylesheetDiagnostic {
                prelude: prelude.to_owned(),
                message: "unclosed rule block".to_owned(),
            });
            break;
        };
        let body = &input[open + 1..close];

        if keyframes_name(prelude).is_some() {
            sheet.diagnostics.push(StylesheetDiagnostic {
                prelude: prelude.to_owned(),
                message: "keyframes inside media groups are outside the first lane".to_owned(),
            });
        } else if media_condition(prelude).is_some() {
            sheet.diagnostics.push(StylesheetDiagnostic {
                prelude: prelude.to_owned(),
                message: "nested media groups are outside the first lane".to_owned(),
            });
        } else if prelude.starts_with('@') {
            sheet.diagnostics.push(StylesheetDiagnostic {
                prelude: prelude.to_owned(),
                message: "unsupported at-rule".to_owned(),
            });
        } else {
            let source_order = source_order_offset.saturating_add(rules.len() as u64);
            match StyleRule::parse(
                prelude,
                body,
                Some(condition),
                origin,
                CascadeLayer::Unlayered,
                source_order,
            ) {
                Ok(rule) => rules.push(rule),
                Err(error) => sheet.diagnostics.push(StylesheetDiagnostic {
                    prelude: prelude.to_owned(),
                    message: error.to_string(),
                }),
            }
        }
        cursor = close + 1;
    }
    rules
}

fn parse_container_style_rules(
    input: &str,
    origin: Origin,
    query: &ContainerQuery,
    source_order_offset: u64,
    sheet: &mut Stylesheet,
) -> Vec<StyleRule> {
    let mut rules = Vec::new();
    let mut cursor = 0;
    while cursor < input.len() {
        let Some(non_space) = input[cursor..]
            .char_indices()
            .find(|(_, character)| !character.is_whitespace())
            .map(|(offset, _)| cursor + offset)
        else {
            break;
        };
        cursor = non_space;
        let Some(open) = find_open_brace(input, cursor) else {
            sheet.diagnostics.push(StylesheetDiagnostic {
                prelude: input[cursor..].trim().to_owned(),
                message: "expected a rule block".to_owned(),
            });
            break;
        };
        let prelude = input[cursor..open].trim();
        let Some(close) = find_close_brace(input, open) else {
            sheet.diagnostics.push(StylesheetDiagnostic {
                prelude: prelude.to_owned(),
                message: "unclosed rule block".to_owned(),
            });
            break;
        };
        let body = &input[open + 1..close];
        if prelude.starts_with('@') {
            sheet.diagnostics.push(StylesheetDiagnostic {
                prelude: prelude.to_owned(),
                message: "nested at-rules inside container groups are outside this lane".to_owned(),
            });
        } else {
            let source_order = source_order_offset.saturating_add(rules.len() as u64);
            match StyleRule::parse(
                prelude,
                body,
                None,
                origin,
                CascadeLayer::Unlayered,
                source_order,
            ) {
                Ok(mut rule) => {
                    rule.container = Some(query.clone());
                    rules.push(rule);
                },
                Err(error) => sheet.diagnostics.push(StylesheetDiagnostic {
                    prelude: prelude.to_owned(),
                    message: error.to_string(),
                }),
            }
        }
        cursor = close + 1;
    }
    rules
}
