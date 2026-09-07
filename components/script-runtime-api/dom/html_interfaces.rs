// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Declarative interface metadata consumed by the DOM bootstrap.
//!
//! The data is generated from WPT's vendored WebIDL — see
//! `html_interfaces_generated.rs` and `support/idl-interface-table` — while
//! the shared JS bootstrap keeps the algorithm glue that calls native sinks.
//! This file owns the record shapes and the JS serialization only.

use super::html_interfaces_generated::{HTML_INTERFACES, SHAPE_INTERFACES};

/// One HTML element interface: its prototype chain, the tag names that select
/// it, and the IDL attributes the bootstrap can install by reflection alone.
pub(crate) struct HtmlInterface {
    pub(crate) name: &'static str,
    pub(crate) parent: &'static str,
    pub(crate) tags: &'static [&'static str],
    pub(crate) reflected: &'static [ReflectedAttribute],
    /// Names of the hand-written member groups `bootstrap.js` installs for
    /// this interface (canvas's `getContext`, and the like).
    pub(crate) members: &'static [&'static str],
    pub(crate) exposed: &'static [&'static str],
    /// `[HTMLConstructor]` in IDL. Without it the constructor always throws.
    pub(crate) html_constructor: bool,
    /// `[LegacyFactoryFunction]` names: `Image`, `Audio`, `Option`.
    pub(crate) named_constructors: &'static [&'static str],
}

pub(crate) struct ReflectedAttribute {
    pub(crate) idl: &'static str,
    pub(crate) kind: &'static str,
    pub(crate) attr: Option<&'static str>,
    pub(crate) keywords: &'static [&'static str],
    pub(crate) missing: Option<&'static str>,
    pub(crate) readonly: bool,
}

/// A DOM / CSSOM interface the bootstrap installs shape-only: the interface
/// object, its prototype chain and its class string, with no members. Used to
/// fill gaps, never to replace an interface the bootstrap already defines.
pub(crate) struct ShapeInterface {
    pub(crate) name: &'static str,
    pub(crate) parent: &'static str,
    pub(crate) exposed: &'static [&'static str],
    pub(crate) constructible: bool,
}

/// Counts reported by the plan and asserted by the drift test.
#[cfg(test)]
pub(crate) fn table_size() -> (usize, usize, usize) {
    (
        HTML_INTERFACES.len(),
        HTML_INTERFACES.iter().map(|i| i.reflected.len()).sum(),
        SHAPE_INTERFACES.len(),
    )
}

pub(crate) fn bootstrap_script() -> String {
    let mut out = String::from("globalThis.__genetHtmlInterfaceTable = [");
    for (i, interface) in HTML_INTERFACES.iter().enumerate() {
        if i != 0 {
            out.push(',');
        }
        push_interface(&mut out, interface);
    }
    out.push_str("];\nglobalThis.__genetShapeInterfaceTable = [");
    for (i, shape) in SHAPE_INTERFACES.iter().enumerate() {
        if i != 0 {
            out.push(',');
        }
        out.push_str("{name:");
        push_js_string(&mut out, shape.name);
        out.push_str(",parent:");
        push_js_string(&mut out, shape.parent);
        out.push_str(",exposed:");
        push_string_array(&mut out, shape.exposed);
        out.push_str(",constructible:");
        out.push_str(if shape.constructible { "true" } else { "false" });
        out.push('}');
    }
    out.push_str("];\n");
    out
}

fn push_interface(out: &mut String, interface: &HtmlInterface) {
    out.push_str("{name:");
    push_js_string(out, interface.name);
    out.push_str(",parent:");
    push_js_string(out, interface.parent);
    out.push_str(",tags:");
    push_string_array(out, interface.tags);
    out.push_str(",reflected:[");
    for (i, attr) in interface.reflected.iter().enumerate() {
        if i != 0 {
            out.push(',');
        }
        push_reflected_attribute(out, attr);
    }
    out.push_str("],members:");
    push_string_array(out, interface.members);
    out.push_str(",exposed:");
    push_string_array(out, interface.exposed);
    out.push_str(",htmlConstructor:");
    out.push_str(if interface.html_constructor {
        "true"
    } else {
        "false"
    });
    out.push_str(",namedConstructors:");
    push_string_array(out, interface.named_constructors);
    out.push('}');
}

fn push_reflected_attribute(out: &mut String, attr: &ReflectedAttribute) {
    out.push_str("{idl:");
    push_js_string(out, attr.idl);
    out.push_str(",kind:");
    push_js_string(out, attr.kind);
    out.push_str(",attr:");
    push_optional_js_string(out, attr.attr);
    out.push_str(",keywords:");
    push_string_array(out, attr.keywords);
    out.push_str(",missing:");
    push_optional_js_string(out, attr.missing);
    out.push_str(",readonly:");
    out.push_str(if attr.readonly { "true" } else { "false" });
    out.push('}');
}

fn push_string_array(out: &mut String, values: &[&str]) {
    out.push('[');
    for (i, value) in values.iter().enumerate() {
        if i != 0 {
            out.push(',');
        }
        push_js_string(out, value);
    }
    out.push(']');
}

fn push_optional_js_string(out: &mut String, value: Option<&str>) {
    match value {
        Some(value) => push_js_string(out, value),
        None => out.push_str("null"),
    }
}

fn push_js_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}
