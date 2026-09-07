// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Generates `components/script-runtime-api/dom/html_interfaces_generated.rs`
//! from two vendored WPT inputs: `interfaces/*.idl` (interface shape, parents,
//! `[Exposed]`, `[HTMLConstructor]`, named constructors, `[Reflect*]`
//! attributes) and `html/semantics/interfaces.js` (tag-name mapping, which
//! WebIDL does not carry).
//!
//! Offline and deterministic: same inputs, byte-identical output. The
//! hand-written exceptions live in [`OVERRIDES`] with a reason each.

pub mod idl;
pub mod tags;

use idl::{Attribute, Idl};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The relative path, from the repository root, of the file this crate writes.
pub const OUTPUT_PATH: &str = "components/script-runtime-api/dom/html_interfaces_generated.rs";

// ---------------------------------------------------------------------------
// Reflected-attribute model (mirrors the runtime's `ReflectedAttribute`).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reflected {
    pub idl: String,
    /// s=DOMString, tc=textContent, b=boolean, e=enumerated, l=long,
    /// ul=unsigned long, t=DOMTokenList, u=URL.
    pub kind: &'static str,
    pub attr: Option<String>,
    pub keywords: Vec<&'static str>,
    pub missing: Option<String>,
    pub readonly: bool,
}

fn s(idl: &str) -> Reflected {
    r(idl, "s")
}

fn r(idl: &str, kind: &'static str) -> Reflected {
    Reflected {
        idl: idl.to_string(),
        kind,
        attr: None,
        keywords: Vec::new(),
        missing: None,
        readonly: false,
    }
}

impl Reflected {
    fn attr(mut self, a: &str) -> Self {
        self.attr = Some(a.to_string());
        self
    }
    fn missing(mut self, m: &str) -> Self {
        self.missing = Some(m.to_string());
        self
    }
    fn keywords(mut self, k: &[&'static str]) -> Self {
        self.kind = "e";
        self.keywords = k.to_vec();
        self
    }
}

/// The eight `referrerpolicy` keywords, spelled once.
const REFERRER_POLICY: &[&str] = &[
    "",
    "no-referrer",
    "no-referrer-when-downgrade",
    "same-origin",
    "origin",
    "strict-origin",
    "origin-when-cross-origin",
    "strict-origin-when-cross-origin",
    "unsafe-url",
];
const CROSS_ORIGIN: &[&str] = &["anonymous", "use-credentials"];
const ENCTYPE: &[&str] = &[
    "application/x-www-form-urlencoded",
    "multipart/form-data",
    "text/plain",
];
const ON_OFF: &[&str] = &["on", "off"];

// ---------------------------------------------------------------------------
// Overrides.
// ---------------------------------------------------------------------------

/// What an override does to one `(interface, IDL attribute)` pair.
pub enum Op {
    /// Replace or insert the attribute with this shape.
    Set(fn() -> Reflected),
    /// Drop the generated attribute entirely.
    Drop,
}

pub struct Override {
    pub iface: &'static str,
    pub idl: &'static str,
    pub op: Op,
    /// Why the IDL alone cannot produce this entry.
    pub reason: &'static str,
}

macro_rules! set {
    ($iface:literal, $idl:literal, $reason:literal, $build:expr) => {
        Override {
            iface: $iface,
            idl: $idl,
            op: Op::Set(|| $build),
            reason: $reason,
        }
    };
}

/// Reasons fall into four families:
///
/// * `enum` — HTML's enumerated attributes are plain `DOMString` in WebIDL and
///   carry no `[Reflect]`; the keyword set and missing-value default are prose.
/// * `prose` — the IDL getter is defined by prose, not reflection, but the
///   runtime's approximation of it is a reflection of the content attribute.
/// * `default` — `[Reflect]` without `[ReflectDefault]`, where HTML's prose
///   still gives a non-zero missing-value default.
/// * `union` — the IDL type is a union the reflection kinds do not model.
pub const OVERRIDES: &[Override] = &[
    // enum: keyword sets and missing-value defaults are prose, not IDL.
    set!(
        "HTMLElement",
        "dir",
        "enum",
        r("dir", "s").keywords(&["ltr", "rtl", "auto"])
    ),
    set!(
        "HTMLLinkElement",
        "crossOrigin",
        "enum",
        s("crossOrigin").attr("crossorigin").keywords(CROSS_ORIGIN)
    ),
    set!(
        "HTMLLinkElement",
        "referrerPolicy",
        "enum",
        s("referrerPolicy")
            .attr("referrerpolicy")
            .keywords(REFERRER_POLICY)
    ),
    set!(
        "HTMLImageElement",
        "crossOrigin",
        "enum",
        s("crossOrigin").attr("crossorigin").keywords(CROSS_ORIGIN)
    ),
    set!(
        "HTMLImageElement",
        "referrerPolicy",
        "enum",
        s("referrerPolicy")
            .attr("referrerpolicy")
            .keywords(REFERRER_POLICY)
    ),
    set!(
        "HTMLImageElement",
        "decoding",
        "enum",
        s("decoding").keywords(&["async", "sync", "auto"])
    ),
    set!(
        "HTMLImageElement",
        "loading",
        "enum",
        s("loading").keywords(&["lazy", "eager"])
    ),
    set!(
        "HTMLAnchorElement",
        "referrerPolicy",
        "enum",
        s("referrerPolicy")
            .attr("referrerpolicy")
            .keywords(REFERRER_POLICY)
    ),
    set!(
        "HTMLAreaElement",
        "referrerPolicy",
        "enum",
        s("referrerPolicy")
            .attr("referrerpolicy")
            .keywords(REFERRER_POLICY)
    ),
    set!(
        "HTMLIFrameElement",
        "referrerPolicy",
        "enum",
        s("referrerPolicy")
            .attr("referrerpolicy")
            .keywords(REFERRER_POLICY)
    ),
    set!(
        "HTMLIFrameElement",
        "loading",
        "enum",
        s("loading").keywords(&["lazy", "eager"])
    ),
    set!(
        "HTMLScriptElement",
        "crossOrigin",
        "enum",
        s("crossOrigin").attr("crossorigin").keywords(CROSS_ORIGIN)
    ),
    set!(
        "HTMLScriptElement",
        "referrerPolicy",
        "enum",
        s("referrerPolicy")
            .attr("referrerpolicy")
            .keywords(REFERRER_POLICY)
    ),
    set!(
        "HTMLMediaElement",
        "crossOrigin",
        "enum",
        s("crossOrigin").attr("crossorigin").keywords(CROSS_ORIGIN)
    ),
    set!(
        "HTMLMediaElement",
        "preload",
        "enum",
        s("preload").keywords(&["none", "metadata", "auto"])
    ),
    set!(
        "HTMLTrackElement",
        "kind",
        "enum",
        s("kind").keywords(&[
            "subtitles",
            "captions",
            "descriptions",
            "chapters",
            "metadata"
        ])
    ),
    set!(
        "HTMLTableCellElement",
        "scope",
        "enum",
        s("scope").keywords(&["row", "col", "rowgroup", "colgroup"])
    ),
    set!(
        "HTMLFormElement",
        "autocomplete",
        "enum",
        s("autocomplete").keywords(ON_OFF)
    ),
    set!(
        "HTMLFormElement",
        "enctype",
        "enum",
        s("enctype").keywords(ENCTYPE)
    ),
    set!(
        "HTMLFormElement",
        "encoding",
        "enum",
        s("encoding").attr("enctype").keywords(ENCTYPE)
    ),
    set!(
        "HTMLFormElement",
        "method",
        "enum",
        s("method").keywords(&["get", "post", "dialog"])
    ),
    set!(
        "HTMLInputElement",
        "autocomplete",
        "enum",
        s("autocomplete").keywords(ON_OFF)
    ),
    set!(
        "HTMLInputElement",
        "formEnctype",
        "enum",
        s("formEnctype").attr("formenctype").keywords(ENCTYPE)
    ),
    set!(
        "HTMLButtonElement",
        "formEnctype",
        "enum",
        s("formEnctype").attr("formenctype").keywords(ENCTYPE)
    ),
    set!(
        "HTMLButtonElement",
        "type",
        "enum",
        s("type")
            .keywords(&["submit", "reset", "button"])
            .missing("submit")
    ),
    set!(
        "HTMLSelectElement",
        "autocomplete",
        "enum",
        s("autocomplete").keywords(ON_OFF)
    ),
    set!(
        "HTMLTextAreaElement",
        "autocomplete",
        "enum",
        s("autocomplete").keywords(ON_OFF)
    ),
    // prose: the IDL member is not `[Reflect]`, but the runtime's approximation
    // is a reflection of the named content attribute (or of text content).
    set!("HTMLTitleElement", "text", "prose", r("text", "tc")),
    set!("HTMLAnchorElement", "text", "prose", r("text", "tc")),
    set!("HTMLOptionElement", "text", "prose", r("text", "tc")),
    set!("HTMLScriptElement", "text", "prose", r("text", "tc")),
    set!("HTMLElement", "nonce", "prose", s("nonce")),
    set!(
        "HTMLElement",
        "tabIndex",
        "prose",
        r("tabIndex", "l").missing("-1")
    ),
    set!("HTMLInputElement", "type", "prose", s("type")),
    set!("HTMLInputElement", "value", "prose", s("value")),
    set!("HTMLTextAreaElement", "value", "prose", s("value")),
    set!("HTMLOptionElement", "value", "prose", s("value")),
    set!(
        "HTMLCanvasElement",
        "width",
        "prose",
        r("width", "ul").missing("300")
    ),
    set!(
        "HTMLCanvasElement",
        "height",
        "prose",
        r("height", "ul").missing("150")
    ),
    // union: `hidden` is `(boolean or unrestricted double or DOMString)?`; the
    // runtime models only the boolean form.
    set!("HTMLElement", "hidden", "union", r("hidden", "b")),
    // default: `[Reflect]` with no `[ReflectDefault]`, but HTML's prose gives
    // input's size a missing-value default of 20.
    set!(
        "HTMLInputElement",
        "size",
        "default",
        r("size", "ul").missing("20")
    ),
];

/// Interfaces whose bootstrap glue is keyed by name in `bootstrap.js`.
const MEMBERS: &[(&str, &[&str])] = &[("HTMLCanvasElement", &["canvas_context"])];

/// `dom.idl` / `cssom.idl` interfaces the shape-only pass must not touch, each
/// because a page feature-detects it and takes a different path when it is a
/// bare shape rather than a working implementation.
const SHAPE_ONLY_DENY: &[&str] = &[
    "MutationObserver",
    "MutationRecord",
    "AbortController",
    "AbortSignal",
    "XSLTProcessor",
];

// ---------------------------------------------------------------------------
// Generated records.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GenInterface {
    pub name: String,
    pub parent: String,
    pub tags: Vec<String>,
    pub reflected: Vec<Reflected>,
    pub members: Vec<String>,
    pub exposed: Vec<String>,
    pub html_constructor: bool,
    pub named_constructors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GenShape {
    pub name: String,
    pub parent: String,
    pub exposed: Vec<String>,
    pub constructible: bool,
}

/// Classify one IDL attribute as a reflected attribute, or `None`.
fn classify(a: &Attribute) -> Option<Reflected> {
    let name = |e: Option<&str>| e.map(str::to_string);
    let explicit = name(a.ext.value("Reflect"))
        .or_else(|| name(a.ext.value("ReflectURL")))
        .or_else(|| name(a.ext.value("ReflectSetter")));
    let default = a.ext.value("ReflectDefault").map(str::to_string);
    let kind: &'static str = if a.ext.has("ReflectURL") {
        "u"
    } else if a.ext.has("Reflect") {
        if a.nullable {
            return None;
        }
        match a.ty.as_str() {
            "DOMString" | "USVString" => "s",
            "boolean" => "b",
            "long" => "l",
            "unsigned long" => "ul",
            "DOMTokenList" => "t",
            _ => return None,
        }
    } else if a.ext.has("ReflectSetter") && a.ty == "USVString" {
        // The getter is prose (it resolves against the base URL) — which is
        // exactly what the runtime's `u` kind does.
        "u"
    } else {
        return None;
    };
    let attr = match explicit {
        Some(e) => Some(e),
        None if a.name.to_ascii_lowercase() != a.name => Some(a.name.to_ascii_lowercase()),
        None => None,
    };
    let missing = match kind {
        "l" | "ul" => Some(default.unwrap_or_else(|| "0".to_string())),
        _ => default,
    };
    Some(Reflected {
        idl: a.name.clone(),
        kind,
        attr,
        keywords: Vec::new(),
        missing,
        readonly: a.readonly,
    })
}

fn parse_idl(wpt: &Path, files: &[&str]) -> Result<Idl, String> {
    let mut parsed = Idl::default();
    for f in files {
        let path = wpt.join("interfaces").join(f);
        let src = std::fs::read_to_string(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .replace("\r\n", "\n");
        idl::parse_into(&src, &mut parsed);
    }
    idl::expand_includes(&mut parsed);
    Ok(parsed)
}

fn exposed_of(i: &idl::Interface) -> Vec<String> {
    match i.ext.value("Exposed") {
        Some(v) => v.split(',').map(|s| s.trim().to_string()).collect(),
        None => Vec::new(),
    }
}

/// Build the HTML element table and the shape-only DOM/CSSOM table.
pub fn build(wpt: &Path) -> Result<(Vec<GenInterface>, Vec<GenShape>), String> {
    let html = parse_idl(wpt, &["html.idl"])?;
    let tag_src = std::fs::read_to_string(wpt.join("html/semantics/interfaces.js"))
        .map_err(|e| format!("interfaces.js: {e}"))?;
    let tag_pairs = tags::parse(&tag_src);

    let mut tags_by_iface: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (tag, iface) in &tag_pairs {
        // Only lowercase ASCII tag names key the runtime's per-tag table; the
        // file's `foo-BAR` / `å-bar` rows exercise the unknown-name fallback.
        if tag
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            tags_by_iface
                .entry(iface.clone())
                .or_default()
                .push(tag.clone());
        }
    }

    // Every interface that reaches HTMLElement through `parent` links.
    let mut html_elements: Vec<&idl::Interface> = html
        .interfaces
        .values()
        .filter(|i| {
            let mut cur = Some(*i);
            let mut hops = 0;
            while let Some(c) = cur {
                if c.name == "HTMLElement" {
                    return true;
                }
                hops += 1;
                if hops > 16 {
                    return false;
                }
                cur = c.parent.as_ref().and_then(|p| html.interfaces.get(p));
            }
            false
        })
        .collect();
    html_elements.sort_by_key(|i| i.order);

    let mut out: Vec<GenInterface> = Vec::new();
    for i in html_elements {
        let mut reflected: Vec<Reflected> = Vec::new();
        let mut seen = BTreeSet::new();
        for a in &i.attributes {
            if !seen.insert(a.name.clone()) {
                continue;
            }
            if let Some(refl) = classify(a) {
                reflected.push(refl);
            }
        }
        // Apply the overrides for this interface, in table order.
        for ov in OVERRIDES {
            if ov.iface != i.name {
                continue;
            }
            let at = reflected.iter().position(|r| r.idl == ov.idl);
            match (&ov.op, at) {
                (Op::Drop, Some(k)) => {
                    reflected.remove(k);
                },
                (Op::Drop, None) => {},
                (Op::Set(build), Some(k)) => reflected[k] = build(),
                (Op::Set(build), None) => reflected.push(build()),
            }
        }
        let named_constructors = i.ext.values("LegacyFactoryFunction");
        out.push(GenInterface {
            name: i.name.clone(),
            parent: i.parent.clone().unwrap_or_else(|| "Element".to_string()),
            tags: tags_by_iface.get(&i.name).cloned().unwrap_or_default(),
            reflected,
            members: MEMBERS
                .iter()
                .find(|(n, _)| *n == i.name)
                .map(|(_, m)| m.iter().map(|s| s.to_string()).collect())
                .unwrap_or_default(),
            exposed: exposed_of(i),
            html_constructor: i.html_constructor,
            named_constructors,
        });
    }

    // Parents before children: the bootstrap builds prototype chains in order.
    let known: BTreeSet<String> = out.iter().map(|i| i.name.clone()).collect();
    let mut ordered: Vec<GenInterface> = Vec::new();
    let mut placed: BTreeSet<String> = BTreeSet::new();
    while ordered.len() < out.len() {
        let before = ordered.len();
        for i in &out {
            if placed.contains(&i.name) {
                continue;
            }
            if known.contains(&i.parent) && !placed.contains(&i.parent) {
                continue;
            }
            placed.insert(i.name.clone());
            ordered.push(i.clone());
        }
        if ordered.len() == before {
            return Err("cyclic interface inheritance".to_string());
        }
    }

    // Shape-only DOM / CSSOM interfaces.
    let other = parse_idl(wpt, &["dom.idl", "cssom.idl"])?;
    let mut shapes: Vec<&idl::Interface> = other
        .interfaces
        .values()
        .filter(|i| {
            exposed_of(i).iter().any(|e| e == "Window")
                && !SHAPE_ONLY_DENY.contains(&i.name.as_str())
                && !known.contains(&i.name)
        })
        .collect();
    shapes.sort_by_key(|i| i.order);
    let shape_names: BTreeSet<String> = shapes.iter().map(|i| i.name.clone()).collect();
    let mut shape_out: Vec<GenShape> = shapes
        .iter()
        .map(|i| GenShape {
            name: i.name.clone(),
            parent: i.parent.clone().unwrap_or_default(),
            exposed: exposed_of(i),
            constructible: i.has_constructor,
        })
        .collect();
    // Same parents-first rule, treating an out-of-set parent as already present.
    let mut sorted: Vec<GenShape> = Vec::new();
    let mut done: BTreeSet<String> = BTreeSet::new();
    while sorted.len() < shape_out.len() {
        let before = sorted.len();
        for i in &shape_out {
            if done.contains(&i.name) {
                continue;
            }
            if shape_names.contains(&i.parent) && !done.contains(&i.parent) {
                continue;
            }
            done.insert(i.name.clone());
            sorted.push(i.clone());
        }
        if sorted.len() == before {
            break;
        }
    }
    if sorted.len() == shape_out.len() {
        shape_out = sorted;
    }

    Ok((ordered, shape_out))
}

// ---------------------------------------------------------------------------
// Emission.
// ---------------------------------------------------------------------------

fn rust_str(v: &str) -> String {
    format!("{:?}", v)
}

fn rust_strs(v: &[String]) -> String {
    let inner: Vec<String> = v.iter().map(|s| rust_str(s)).collect();
    format!("&[{}]", inner.join(", "))
}

fn rust_static_strs(v: &[&str]) -> String {
    let inner: Vec<String> = v.iter().map(|s| rust_str(s)).collect();
    format!("&[{}]", inner.join(", "))
}

fn rust_opt(v: Option<&String>) -> String {
    match v {
        Some(v) => format!("Some({})", rust_str(v)),
        None => "None".to_string(),
    }
}

/// Render the generated Rust source. Deterministic for fixed inputs.
pub fn emit(interfaces: &[GenInterface], shapes: &[GenShape]) -> String {
    let mut o = String::new();
    o.push_str(
        "// Copyright 2026 Mark Alan Boykin\n\
         // This Source Code Form is subject to the terms of the Mozilla Public\n\
         // License, v. 2.0. If a copy of the MPL was not distributed with this\n\
         // file, You can obtain one at https://mozilla.org/MPL/2.0/.\n\
         // SPDX-License-Identifier: MPL-2.0\n\n\
         //! @generated by `cargo run -p genet-idl-interface-table`. DO NOT EDIT.\n\
         //!\n\
         //! Sources: `tests/wpt/tests/interfaces/{html,dom,cssom}.idl` and\n\
         //! `tests/wpt/tests/html/semantics/interfaces.js`. Hand-written\n\
         //! exceptions live in the generator's `OVERRIDES` table, each with a\n\
         //! reason; `dom/tests.rs::generated_table_is_current` regenerates this\n\
         //! file and fails on any drift.\n\n\
         use super::html_interfaces::{HtmlInterface, ReflectedAttribute, ShapeInterface};\n\n",
    );
    // `#[rustfmt::skip]` keeps the emitted literal byte-identical to what the
    // drift test regenerates, whatever rustfmt would do to the long arrays.
    o.push_str("#[rustfmt::skip]\npub(crate) const HTML_INTERFACES: &[HtmlInterface] = &[\n");
    for i in interfaces {
        o.push_str("    HtmlInterface {\n");
        o.push_str(&format!("        name: {},\n", rust_str(&i.name)));
        o.push_str(&format!("        parent: {},\n", rust_str(&i.parent)));
        o.push_str(&format!("        tags: {},\n", rust_strs(&i.tags)));
        if i.reflected.is_empty() {
            o.push_str("        reflected: &[],\n");
        } else {
            o.push_str("        reflected: &[\n");
            for a in &i.reflected {
                o.push_str("            ReflectedAttribute {\n");
                o.push_str(&format!("                idl: {},\n", rust_str(&a.idl)));
                o.push_str(&format!("                kind: {},\n", rust_str(a.kind)));
                o.push_str(&format!(
                    "                attr: {},\n",
                    rust_opt(a.attr.as_ref())
                ));
                o.push_str(&format!(
                    "                keywords: {},\n",
                    rust_static_strs(&a.keywords)
                ));
                o.push_str(&format!(
                    "                missing: {},\n",
                    rust_opt(a.missing.as_ref())
                ));
                o.push_str(&format!("                readonly: {},\n", a.readonly));
                o.push_str("            },\n");
            }
            o.push_str("        ],\n");
        }
        o.push_str(&format!("        members: {},\n", rust_strs(&i.members)));
        o.push_str(&format!("        exposed: {},\n", rust_strs(&i.exposed)));
        o.push_str(&format!(
            "        html_constructor: {},\n",
            i.html_constructor
        ));
        o.push_str(&format!(
            "        named_constructors: {},\n",
            rust_strs(&i.named_constructors)
        ));
        o.push_str("    },\n");
    }
    o.push_str("];\n\n");

    o.push_str("#[rustfmt::skip]\npub(crate) const SHAPE_INTERFACES: &[ShapeInterface] = &[\n");
    for i in shapes {
        o.push_str(&format!(
            "    ShapeInterface {{ name: {}, parent: {}, exposed: {}, constructible: {} }},\n",
            rust_str(&i.name),
            rust_str(&i.parent),
            rust_strs(&i.exposed),
            i.constructible
        ));
    }
    o.push_str("];\n");
    o
}

/// Read the vendored WPT inputs under `wpt` and render the table.
pub fn generate(wpt: &Path) -> Result<String, String> {
    let (interfaces, shapes) = build(wpt)?;
    Ok(emit(&interfaces, &shapes))
}
