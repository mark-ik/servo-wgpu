// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! A small WebIDL reader covering the subset genet's interface table needs:
//! interface / partial interface / interface mixin declarations, `includes`
//! statements, extended-attribute lists, and `attribute` members. Everything
//! else in a definition body is skipped to its terminating `;`.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    Ident(String),
    Str(String),
    Punct(char),
}

fn tokenize(src: &str) -> Vec<Tok> {
    let b: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && i + 1 < b.len() && b[i + 1] == '/' {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && i + 1 < b.len() && b[i + 1] == '*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(b.len());
        } else if c == '"' {
            i += 1;
            let mut s = String::new();
            while i < b.len() && b[i] != '"' {
                s.push(b[i]);
                i += 1;
            }
            i += 1;
            out.push(Tok::Str(s));
        } else if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
            let mut s = String::new();
            while i < b.len()
                && (b[i].is_ascii_alphanumeric() || b[i] == '_' || b[i] == '-' || b[i] == '.')
            {
                s.push(b[i]);
                i += 1;
            }
            out.push(Tok::Ident(s));
        } else {
            out.push(Tok::Punct(c));
            i += 1;
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct ExtAttrs(pub Vec<(String, Option<String>)>);

impl ExtAttrs {
    pub fn has(&self, name: &str) -> bool {
        self.0.iter().any(|(n, _)| n == name)
    }
    pub fn value(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_deref())
    }
    pub fn values(&self, name: &str) -> Vec<String> {
        self.0
            .iter()
            .filter(|(n, _)| n == name)
            .filter_map(|(_, v)| v.clone())
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct Attribute {
    pub name: String,
    /// Normalized IDL type spelling: `DOMString`, `USVString`, `boolean`,
    /// `long`, `unsigned long`, `double`, `DOMTokenList`, `union`, ...
    pub ty: String,
    pub nullable: bool,
    pub readonly: bool,
    pub ext: ExtAttrs,
}

#[derive(Debug, Clone, Default)]
pub struct Interface {
    pub name: String,
    pub parent: Option<String>,
    pub ext: ExtAttrs,
    pub attributes: Vec<Attribute>,
    pub has_constructor: bool,
    pub html_constructor: bool,
    /// Declaration index in the source file, for stable emission order.
    pub order: usize,
}

#[derive(Debug, Default)]
pub struct Idl {
    pub interfaces: BTreeMap<String, Interface>,
    pub mixins: BTreeMap<String, Interface>,
    pub includes: Vec<(String, String)>,
}

struct Parser {
    t: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn is_punct(&self, c: char) -> bool {
        matches!(self.peek(), Some(Tok::Punct(p)) if *p == c)
    }
    fn is_ident(&self, s: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(n)) if n == s)
    }
    fn ident(&mut self) -> Option<String> {
        if let Some(Tok::Ident(n)) = self.peek().cloned() {
            self.i += 1;
            Some(n)
        } else {
            None
        }
    }
    fn eat(&mut self, c: char) -> bool {
        if self.is_punct(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    /// Skip a balanced `open`..`close` run, returning the tokens inside it.
    fn balanced(&mut self, open: char, close: char) -> Vec<Tok> {
        let mut depth = 0usize;
        let mut out = Vec::new();
        while self.i < self.t.len() {
            let tok = self.t[self.i].clone();
            self.i += 1;
            if let Tok::Punct(p) = tok {
                if p == open {
                    depth += 1;
                    if depth == 1 {
                        continue;
                    }
                } else if p == close {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            if depth > 0 {
                out.push(tok);
            }
        }
        out
    }
    /// Skip forward to the next `;` at the current nesting level.
    fn skip_to_semi(&mut self) {
        let mut depth = 0i32;
        while self.i < self.t.len() {
            match &self.t[self.i] {
                Tok::Punct('{') | Tok::Punct('(') | Tok::Punct('[') => depth += 1,
                Tok::Punct('}') | Tok::Punct(')') | Tok::Punct(']') => depth -= 1,
                Tok::Punct(';') if depth <= 0 => {
                    self.i += 1;
                    return;
                },
                _ => {},
            }
            self.i += 1;
        }
    }
}

fn parse_ext_attrs(toks: &[Tok]) -> ExtAttrs {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        // One entry: Name [= Value | = ( ... ) | ( args )].
        let name = match &toks[i] {
            Tok::Ident(n) => n.clone(),
            _ => {
                i += 1;
                continue;
            },
        };
        i += 1;
        let mut value = None;
        if matches!(toks.get(i), Some(Tok::Punct('='))) {
            i += 1;
            match toks.get(i) {
                Some(Tok::Ident(v)) => {
                    value = Some(v.clone());
                    i += 1;
                    // `LegacyFactoryFunction=Image(...)`: drop the argument list.
                    if matches!(toks.get(i), Some(Tok::Punct('('))) {
                        let mut d = 0i32;
                        while i < toks.len() {
                            match &toks[i] {
                                Tok::Punct('(') => d += 1,
                                Tok::Punct(')') => {
                                    d -= 1;
                                    if d == 0 {
                                        i += 1;
                                        break;
                                    }
                                },
                                _ => {},
                            }
                            i += 1;
                        }
                    }
                },
                Some(Tok::Str(v)) => {
                    value = Some(v.clone());
                    i += 1;
                },
                Some(Tok::Punct('(')) => {
                    // `Exposed=(Window,Worker)` / `ReflectRange=(0, 8)`.
                    let mut parts = Vec::new();
                    let mut d = 0i32;
                    while i < toks.len() {
                        match &toks[i] {
                            Tok::Punct('(') => d += 1,
                            Tok::Punct(')') => {
                                d -= 1;
                                if d == 0 {
                                    i += 1;
                                    break;
                                }
                            },
                            Tok::Ident(v) => parts.push(v.clone()),
                            _ => {},
                        }
                        i += 1;
                    }
                    value = Some(parts.join(","));
                },
                _ => {},
            }
        } else if matches!(toks.get(i), Some(Tok::Punct('('))) {
            let mut d = 0i32;
            while i < toks.len() {
                match &toks[i] {
                    Tok::Punct('(') => d += 1,
                    Tok::Punct(')') => {
                        d -= 1;
                        if d == 0 {
                            i += 1;
                            break;
                        }
                    },
                    _ => {},
                }
                i += 1;
            }
        }
        out.push((name, value));
        // Skip to the next top-level comma.
        let mut d = 0i32;
        while i < toks.len() {
            match &toks[i] {
                Tok::Punct('(') | Tok::Punct('[') => d += 1,
                Tok::Punct(')') | Tok::Punct(']') => d -= 1,
                Tok::Punct(',') if d == 0 => {
                    i += 1;
                    break;
                },
                _ => {},
            }
            i += 1;
        }
    }
    ExtAttrs(out)
}

const TYPE_WORDS: &[&str] = &[
    "unsigned",
    "unrestricted",
    "long",
    "short",
    "double",
    "float",
];

impl Parser {
    /// Parse an interface body, appending members to `iface`.
    fn body(&mut self, iface: &mut Interface) {
        // Consume the opening `{`.
        if !self.eat('{') {
            return;
        }
        loop {
            if self.i >= self.t.len() || self.eat('}') {
                break;
            }
            let mut ext = ExtAttrs::default();
            if self.is_punct('[') {
                let inner = self.balanced('[', ']');
                ext = parse_ext_attrs(&inner);
            }
            if self.is_ident("constructor") {
                iface.has_constructor = true;
                if ext.has("HTMLConstructor") {
                    iface.html_constructor = true;
                }
                self.skip_to_semi();
                continue;
            }
            // `static`, `stringifier`, `inherit`, `readonly` may precede `attribute`.
            let mut readonly = false;
            loop {
                if self.is_ident("readonly") {
                    readonly = true;
                    self.i += 1;
                } else if self.is_ident("static")
                    || self.is_ident("stringifier")
                    || self.is_ident("inherit")
                {
                    self.i += 1;
                } else {
                    break;
                }
            }
            if self.is_ident("attribute") {
                self.i += 1;
                if let Some(attr) = self.attribute(readonly, ext) {
                    iface.attributes.push(attr);
                }
                continue;
            }
            if self.i < self.t.len() {
                self.skip_to_semi();
            }
        }
        // Trailing `;` after the closing brace.
        self.eat(';');
    }

    fn attribute(&mut self, readonly: bool, ext: ExtAttrs) -> Option<Attribute> {
        // A member-level extended attribute may sit on the type
        // (`attribute [LegacyNullToEmptyString] DOMString text`).
        if self.is_punct('[') {
            self.balanced('[', ']');
        }
        let ty = if self.is_punct('(') {
            self.balanced('(', ')');
            "union".to_string()
        } else {
            let mut words = Vec::new();
            while let Some(Tok::Ident(n)) = self.peek().cloned() {
                if words.is_empty() {
                    words.push(n);
                    self.i += 1;
                    if !TYPE_WORDS.contains(&words[0].as_str()) {
                        break;
                    }
                } else if TYPE_WORDS.contains(&n.as_str()) {
                    words.push(n);
                    self.i += 1;
                } else {
                    break;
                }
            }
            if words.is_empty() {
                self.skip_to_semi();
                return None;
            }
            words.join(" ")
        };
        let nullable = self.eat('?');
        // A sequence/record/promise type parameter list.
        if self.is_punct('<') {
            self.balanced('<', '>');
        }
        let name = self.ident();
        self.skip_to_semi();
        name.map(|name| Attribute {
            name,
            ty,
            nullable,
            readonly,
            ext,
        })
    }
}

/// Parse one IDL file into `idl`, merging partials onto the base interface.
pub fn parse_into(src: &str, idl: &mut Idl) {
    let mut p = Parser {
        t: tokenize(src),
        i: 0,
    };
    let mut order = idl.interfaces.len() + idl.mixins.len();
    loop {
        if p.i >= p.t.len() {
            break;
        }
        let mut ext = ExtAttrs::default();
        if p.is_punct('[') {
            let inner = p.balanced('[', ']');
            ext = parse_ext_attrs(&inner);
        }
        let partial = if p.is_ident("partial") {
            p.i += 1;
            true
        } else {
            false
        };
        if p.is_ident("interface") {
            p.i += 1;
            let mixin = if p.is_ident("mixin") {
                p.i += 1;
                true
            } else {
                false
            };
            let Some(name) = p.ident() else { continue };
            let parent = if p.eat(':') { p.ident() } else { None };
            let mut fresh = Interface {
                name: name.clone(),
                parent,
                ext,
                order,
                ..Default::default()
            };
            p.body(&mut fresh);
            order += 1;
            let table = if mixin {
                &mut idl.mixins
            } else {
                &mut idl.interfaces
            };
            match table.get_mut(&name) {
                Some(existing) => {
                    existing.attributes.extend(fresh.attributes);
                    existing.has_constructor |= fresh.has_constructor;
                    existing.html_constructor |= fresh.html_constructor;
                    if existing.parent.is_none() {
                        existing.parent = fresh.parent;
                    }
                    if !partial && !fresh.ext.0.is_empty() {
                        existing.ext = fresh.ext;
                    }
                },
                None => {
                    table.insert(name, fresh);
                },
            }
            continue;
        }
        // `X includes Y;`
        if let Some(Tok::Ident(lhs)) = p.peek().cloned() {
            if matches!(p.t.get(p.i + 1), Some(Tok::Ident(k)) if k == "includes") {
                if let Some(Tok::Ident(rhs)) = p.t.get(p.i + 2).cloned() {
                    idl.includes.push((lhs, rhs));
                }
                p.i += 3;
                p.eat(';');
                continue;
            }
        }
        p.skip_to_semi();
    }
}

/// Append each mixin's attributes to every interface that includes it.
pub fn expand_includes(idl: &mut Idl) {
    let includes = idl.includes.clone();
    for (target, mixin) in includes {
        let Some(m) = idl.mixins.get(&mixin).cloned() else {
            continue;
        };
        if let Some(t) = idl.interfaces.get_mut(&target) {
            t.attributes.extend(m.attributes);
        }
    }
}
