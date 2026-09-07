// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! WPT's tag-name -> interface map, read from
//! `html/semantics/interfaces.js`. WebIDL carries no tag names, so this is the
//! second vendored input the table is generated from. An entry's second field
//! is the interface's middle word: `"Anchor"` means `HTMLAnchorElement`, `""`
//! means `HTMLElement`, `"Unknown"` means `HTMLUnknownElement`.

/// `(tag, interface name)` pairs in source order.
pub fn parse(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if !line.starts_with("[\"") {
            continue;
        }
        let mut fields = Vec::new();
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '"' {
                continue;
            }
            let mut s = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => {
                        // Only `\uXXXX` appears in this file.
                        if chars.peek() == Some(&'u') {
                            chars.next();
                            let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                            if let Some(ch) =
                                u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                            {
                                s.push(ch);
                            }
                        }
                    },
                    c => s.push(c),
                }
            }
            fields.push(s);
            if fields.len() == 2 {
                break;
            }
        }
        if fields.len() == 2 {
            let iface = format!("HTML{}Element", fields[1]);
            out.push((fields[0].clone(), iface));
        }
    }
    out
}
