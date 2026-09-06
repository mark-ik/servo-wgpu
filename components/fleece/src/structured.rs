/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::collections::{HashMap, HashSet, VecDeque};

use layout_dom_api::{LayoutDom, NodeKind};

use crate::{attr, local_name};

/// One value harvested from page-carried JSON-LD syntax or HTML Microdata.
///
/// Fleece keeps the JSON-shaped variants local so JSON-LD syntax harvesting
/// does not add a parser dependency to the render-free extraction cone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuredValue {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<StructuredValue>),
    Object(Vec<(String, StructuredValue)>),
    /// A nested HTML Microdata item, including its own declared types and ID.
    Item(Box<StructuredData>),
    /// A nested Microdata item that would revisit an ancestor item.
    Cycle,
}

/// One HTML JSON-LD script retained before any JSON-LD semantic processing.
///
/// `dom_text` is the exact descendant text supplied by the parsed DOM. It is
/// not the original response bytes, which remain acquisition-owned. The
/// document order is the script element's zero-based DOM preorder position,
/// including non-element nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedJsonLdBlock {
    pub document_order: u64,
    /// The exact DOM `id` attribute value, including an empty or duplicate value.
    pub element_id: Option<String>,
    /// The exact DOM `type` attribute value that matched `application/ld+json`.
    pub declared_type: String,
    pub dom_text: String,
    pub parse: JsonLdParseStatus,
}

/// Syntax outcome for one retained JSON-LD script.
///
/// A parsed value is still JSON syntax, not expanded JSON-LD or RDF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonLdParseStatus {
    Parsed(StructuredValue),
    InvalidJson,
}

impl StructuredValue {
    /// Return an object member by name.
    pub fn get(&self, name: &str) -> Option<&StructuredValue> {
        let Self::Object(entries) = self else {
            return None;
        };
        entries
            .iter()
            .find_map(|(key, value)| (key == name).then_some(value))
    }

    /// Return this value as a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }
}

/// A JSON-LD object or HTML Microdata item harvested without semantic processing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredData {
    /// Raw declared `@type` or `itemtype` values, in source order.
    pub types: Vec<String>,
    /// Raw string `@id` or `itemid`, if one was declared.
    pub id: Option<String>,
    /// The harvested value, retaining fields fleece does not interpret.
    pub value: StructuredValue,
    /// The page syntax that supplied the value.
    pub source: StructuredDataSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredDataSource {
    JsonLd,
    Microdata,
}

/// Retain every HTML `application/ld+json` script in DOM order.
pub fn extract_json_ld_blocks<D: LayoutDom>(dom: &D) -> Vec<EmbeddedJsonLdBlock> {
    extract_page_carried_data(dom).0
}

/// Harvest JSON-LD and microdata without interpreting consumer policy.
pub fn extract_structured_data<D: LayoutDom>(dom: &D) -> Vec<StructuredData> {
    extract_page_carried_data(dom).1
}

pub(crate) fn extract_page_carried_data<D: LayoutDom>(
    dom: &D,
) -> (Vec<EmbeddedJsonLdBlock>, Vec<StructuredData>) {
    let index = DomIndex::build(dom);
    let mut blocks = Vec::new();
    let mut projection = Vec::new();
    walk_structured_data(dom, dom.document(), &index, &mut blocks, &mut projection);
    (blocks, projection)
}

fn walk_structured_data<D: LayoutDom>(
    dom: &D,
    id: D::NodeId,
    index: &DomIndex<D::NodeId>,
    blocks: &mut Vec<EmbeddedJsonLdBlock>,
    out: &mut Vec<StructuredData>,
) {
    if let Some(declared_type) = json_ld_script_type(dom, id) {
        let raw = raw_text_of(dom, id);
        let parse = match parse_json_ld_dom_text(&raw) {
            Some(value) => {
                collect_json_ld_root(value.clone(), out);
                JsonLdParseStatus::Parsed(value)
            },
            None => JsonLdParseStatus::InvalidJson,
        };
        blocks.push(EmbeddedJsonLdBlock {
            document_order: u64::try_from(
                *index
                    .order
                    .get(&id)
                    .expect("walked node must have a DOM preorder position"),
            )
            .expect("DOM preorder position must fit u64"),
            element_id: attr(dom, id, "id"),
            declared_type,
            dom_text: raw,
            parse,
        });
    }
    if is_html_element(dom, id)
        && attr(dom, id, "itemscope").is_some()
        && attr(dom, id, "itemprop").is_none()
    {
        out.push(microdata_item(dom, id, index, &mut HashSet::new()));
    }
    for child in dom.dom_children(id) {
        walk_structured_data(dom, child, index, blocks, out);
    }
}

fn json_ld_script_type<D: LayoutDom>(dom: &D, id: D::NodeId) -> Option<String> {
    if !is_html_element(dom, id) || local_name(dom, id) != Some("script") {
        return None;
    }
    attr(dom, id, "type").filter(|value| is_json_ld_media_type(value))
}

pub(crate) fn is_json_ld_media_type(value: &str) -> bool {
    value
        .split(';')
        .next()
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/ld+json"))
}

pub(crate) fn parse_json_ld_dom_text(source: &str) -> Option<StructuredValue> {
    JsonParser::new(source).parse()
}

fn collect_json_ld_root(value: StructuredValue, out: &mut Vec<StructuredData>) {
    match value {
        StructuredValue::Array(values) => {
            for value in values {
                collect_json_ld_root(value, out);
            }
        },
        StructuredValue::Object(entries) => {
            collect_json_ld_object(StructuredValue::Object(entries), out);
        },
        _ => {},
    }
}

fn collect_json_ld_object(value: StructuredValue, out: &mut Vec<StructuredData>) {
    out.push(StructuredData {
        types: json_ld_types(&value),
        id: json_ld_id(&value),
        value: value.clone(),
        source: StructuredDataSource::JsonLd,
    });

    let StructuredValue::Object(entries) = value else {
        return;
    };
    for (name, graph) in entries {
        if name != "@graph" {
            continue;
        }
        match graph {
            StructuredValue::Object(_) => collect_json_ld_object(graph, out),
            StructuredValue::Array(members) => {
                for member in members {
                    if matches!(member, StructuredValue::Object(_)) {
                        collect_json_ld_object(member, out);
                    }
                }
            },
            _ => {},
        }
    }
}

fn json_ld_types(value: &StructuredValue) -> Vec<String> {
    let StructuredValue::Object(entries) = value else {
        return Vec::new();
    };
    let mut types = Vec::new();
    for (name, value) in entries {
        if name != "@type" {
            continue;
        }
        match value {
            StructuredValue::String(value) => types.push(value.clone()),
            StructuredValue::Array(values) => {
                types.extend(
                    values
                        .iter()
                        .filter_map(StructuredValue::as_str)
                        .map(str::to_string),
                );
            },
            _ => {},
        }
    }
    types
}

fn json_ld_id(value: &StructuredValue) -> Option<String> {
    let StructuredValue::Object(entries) = value else {
        return None;
    };
    entries.iter().find_map(|(name, value)| {
        (name == "@id")
            .then(|| value.as_str().map(str::to_string))
            .flatten()
    })
}

fn microdata_item<D: LayoutDom>(
    dom: &D,
    root: D::NodeId,
    index: &DomIndex<D::NodeId>,
    item_stack: &mut HashSet<D::NodeId>,
) -> StructuredData {
    let inserted = item_stack.insert(root);
    debug_assert!(inserted);
    let mut fields = Vec::new();
    for property in item_property_elements(dom, root, index) {
        let names = item_property_names(dom, property);
        let value = if is_html_element(dom, property) && attr(dom, property, "itemscope").is_some()
        {
            if item_stack.contains(&property) {
                StructuredValue::Cycle
            } else {
                StructuredValue::Item(Box::new(microdata_item(dom, property, index, item_stack)))
            }
        } else {
            StructuredValue::String(microdata_property_text(dom, property))
        };
        for name in names {
            fields.push((name, value.clone()));
        }
    }
    let removed = item_stack.remove(&root);
    debug_assert!(removed);
    StructuredData {
        types: attr(dom, root, "itemtype")
            .map(|types| html_tokens(&types).map(str::to_string).collect())
            .unwrap_or_default(),
        id: attr(dom, root, "itemid"),
        value: StructuredValue::Object(fields),
        source: StructuredDataSource::Microdata,
    }
}

fn item_property_elements<D: LayoutDom>(
    dom: &D,
    root: D::NodeId,
    index: &DomIndex<D::NodeId>,
) -> Vec<D::NodeId> {
    let mut memory = HashSet::new();
    memory.insert(root);
    let mut pending = VecDeque::new();
    pending.extend(element_children(dom, root));
    if let Some(itemref) = attr(dom, root, "itemref") {
        for reference in html_tokens(&itemref) {
            if let Some(target) = index.first_id.get(reference) {
                pending.push_back(*target);
            }
        }
    }

    let mut results = Vec::new();
    while let Some(current) = pending.pop_front() {
        if !memory.insert(current) {
            continue;
        }
        if !is_html_element(dom, current) || attr(dom, current, "itemscope").is_none() {
            pending.extend(element_children(dom, current));
        }
        if !item_property_names(dom, current).is_empty() {
            results.push(current);
        }
    }
    results.sort_by_key(|id| index.order.get(id).copied().unwrap_or(usize::MAX));
    results
}

fn item_property_names<D: LayoutDom>(dom: &D, id: D::NodeId) -> Vec<String> {
    if !is_html_element(dom, id) {
        return Vec::new();
    }
    let Some(properties) = attr(dom, id, "itemprop") else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    html_tokens(&properties)
        .filter(|property| seen.insert(*property))
        .map(str::to_string)
        .collect()
}

fn html_tokens(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(['\t', '\n', '\u{000c}', '\r', ' '])
        .filter(|token| !token.is_empty())
}

fn is_html_element<D: LayoutDom>(dom: &D, id: D::NodeId) -> bool {
    dom.element_name(id)
        .is_some_and(|name| name.ns.as_ref() == "http://www.w3.org/1999/xhtml")
}

fn microdata_property_text<D: LayoutDom>(dom: &D, id: D::NodeId) -> String {
    match local_name(dom, id) {
        Some("meta") => attr(dom, id, "content").unwrap_or_default(),
        Some("a" | "area" | "link") => attr(dom, id, "href").unwrap_or_default(),
        Some("audio" | "embed" | "iframe" | "img" | "source" | "track" | "video") => {
            attr(dom, id, "src").unwrap_or_default()
        },
        Some("object") => attr(dom, id, "data").unwrap_or_default(),
        Some("data" | "meter") => attr(dom, id, "value").unwrap_or_default(),
        Some("time") => attr(dom, id, "datetime").unwrap_or_else(|| text_content(dom, id)),
        _ => text_content(dom, id),
    }
}

fn raw_text_of<D: LayoutDom>(dom: &D, id: D::NodeId) -> String {
    text_content(dom, id)
}

fn text_content<D: LayoutDom>(dom: &D, id: D::NodeId) -> String {
    let mut out = String::new();
    collect_text_content(dom, id, &mut out);
    out
}

fn collect_text_content<D: LayoutDom>(dom: &D, id: D::NodeId, out: &mut String) {
    if dom.kind(id) == NodeKind::Text {
        if let Some(text) = dom.text(id) {
            out.push_str(text);
        }
    }
    for child in dom.dom_children(id) {
        collect_text_content(dom, child, out);
    }
}

fn element_children<D: LayoutDom>(dom: &D, id: D::NodeId) -> Vec<D::NodeId> {
    dom.dom_children(id)
        .filter(|child| local_name(dom, *child).is_some())
        .collect()
}

struct DomIndex<N> {
    order: HashMap<N, usize>,
    first_id: HashMap<String, N>,
}

impl<N: Copy + Eq + std::hash::Hash> DomIndex<N> {
    fn build<D: LayoutDom<NodeId = N>>(dom: &D) -> Self {
        fn visit<D: LayoutDom>(dom: &D, id: D::NodeId, index: &mut DomIndex<D::NodeId>) {
            let position = index.order.len();
            index.order.insert(id, position);
            if local_name(dom, id).is_some()
                && let Some(value) = attr(dom, id, "id")
            {
                index.first_id.entry(value).or_insert(id);
            }
            for child in dom.dom_children(id) {
                visit(dom, child, index);
            }
        }

        let mut index = Self {
            order: HashMap::new(),
            first_id: HashMap::new(),
        };
        visit(dom, dom.document(), &mut index);
        index
    }
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> JsonParser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            bytes: source.as_bytes(),
            cursor: 0,
        }
    }

    fn parse(mut self) -> Option<StructuredValue> {
        self.skip_ws();
        let value = self.value()?;
        self.skip_ws();
        (self.cursor == self.bytes.len()).then_some(value)
    }

    fn value(&mut self) -> Option<StructuredValue> {
        self.skip_ws();
        match self.peek()? {
            b'n' => self.literal(b"null", StructuredValue::Null),
            b't' => self.literal(b"true", StructuredValue::Bool(true)),
            b'f' => self.literal(b"false", StructuredValue::Bool(false)),
            b'"' => self.string().map(StructuredValue::String),
            b'[' => self.array(),
            b'{' => self.object(),
            b'-' | b'0'..=b'9' => self.number().map(StructuredValue::Number),
            _ => None,
        }
    }

    fn literal(&mut self, literal: &[u8], value: StructuredValue) -> Option<StructuredValue> {
        let end = self.cursor.checked_add(literal.len())?;
        (self.bytes.get(self.cursor..end)? == literal).then(|| {
            self.cursor = end;
            value
        })
    }

    fn array(&mut self) -> Option<StructuredValue> {
        self.take(b'[')?;
        let mut values = Vec::new();
        self.skip_ws();
        if self.take(b']').is_some() {
            return Some(StructuredValue::Array(values));
        }
        loop {
            values.push(self.value()?);
            self.skip_ws();
            if self.take(b']').is_some() {
                break;
            }
            self.take(b',')?;
        }
        Some(StructuredValue::Array(values))
    }

    fn object(&mut self) -> Option<StructuredValue> {
        self.take(b'{')?;
        let mut entries = Vec::new();
        self.skip_ws();
        if self.take(b'}').is_some() {
            return Some(StructuredValue::Object(entries));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.take(b':')?;
            entries.push((key, self.value()?));
            self.skip_ws();
            if self.take(b'}').is_some() {
                break;
            }
            self.take(b',')?;
        }
        Some(StructuredValue::Object(entries))
    }

    fn string(&mut self) -> Option<String> {
        self.take(b'"')?;
        let mut out = String::new();
        while let Some(byte) = self.peek() {
            self.cursor += 1;
            match byte {
                b'"' => return Some(out),
                b'\\' => {
                    let escaped = self.peek()?;
                    self.cursor += 1;
                    match escaped {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return None,
                    }
                },
                0x00..=0x1f => return None,
                ascii if ascii.is_ascii() => out.push(ascii as char),
                _ => {
                    self.cursor -= 1;
                    let tail = std::str::from_utf8(&self.bytes[self.cursor..]).ok()?;
                    let character = tail.chars().next()?;
                    self.cursor += character.len_utf8();
                    out.push(character);
                },
            }
        }
        None
    }

    fn unicode_escape(&mut self) -> Option<char> {
        let first = self.hex_quad()?;
        if (0xd800..=0xdbff).contains(&first) {
            self.take(b'\\')?;
            self.take(b'u')?;
            let second = self.hex_quad()?;
            if !(0xdc00..=0xdfff).contains(&second) {
                return None;
            }
            let scalar =
                0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00);
            char::from_u32(scalar)
        } else {
            char::from_u32(u32::from(first))
        }
    }

    fn hex_quad(&mut self) -> Option<u16> {
        let mut value = 0u16;
        for _ in 0..4 {
            let digit = (self.peek()? as char).to_digit(16)? as u16;
            self.cursor += 1;
            value = value.checked_mul(16)?.checked_add(digit)?;
        }
        Some(value)
    }

    fn number(&mut self) -> Option<String> {
        let start = self.cursor;
        if self.peek() == Some(b'-') {
            self.cursor += 1;
        }
        match self.peek()? {
            b'0' => self.cursor += 1,
            b'1'..=b'9' => {
                self.cursor += 1;
                while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.cursor += 1;
                }
            },
            _ => return None,
        }
        if self.peek() == Some(b'.') {
            self.cursor += 1;
            let fraction = self.cursor;
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.cursor += 1;
            }
            if self.cursor == fraction {
                return None;
            }
        }
        if self.peek().is_some_and(|byte| matches!(byte, b'e' | b'E')) {
            self.cursor += 1;
            if self.peek().is_some_and(|byte| matches!(byte, b'+' | b'-')) {
                self.cursor += 1;
            }
            let exponent = self.cursor;
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.cursor += 1;
            }
            if self.cursor == exponent {
                return None;
            }
        }
        std::str::from_utf8(&self.bytes[start..self.cursor])
            .ok()
            .map(str::to_string)
    }

    fn skip_ws(&mut self) {
        while self
            .peek()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.cursor += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.cursor).copied()
    }

    fn take(&mut self, expected: u8) -> Option<()> {
        (self.peek()? == expected).then(|| self.cursor += 1)
    }
}

#[cfg(test)]
mod tests {
    use super::JsonParser;

    #[test]
    fn json_parser_accepts_only_json_whitespace() {
        assert!(
            JsonParser::new("{\t\"@type\"\r:\n\"Thing\" }")
                .parse()
                .is_some()
        );
        assert!(
            JsonParser::new("{\"@type\"\u{000c}:\"Thing\"}")
                .parse()
                .is_none()
        );
    }
}
