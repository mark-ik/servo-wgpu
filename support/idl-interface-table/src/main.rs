// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! `cargo run -p genet-idl-interface-table [-- --check]`
//!
//! Writes (or, with `--check`, verifies) the generated interface table.

use std::path::PathBuf;

fn main() {
    let repo: PathBuf = std::env::var("GENET_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("repository root")
        });
    let wpt = repo.join("tests/wpt/tests");
    let generated = match genet_idl_interface_table::generate(&wpt) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("generate: {e}");
            std::process::exit(2);
        },
    };
    let out = repo.join(genet_idl_interface_table::OUTPUT_PATH);
    if std::env::args().any(|a| a == "--check") {
        let current = std::fs::read_to_string(&out).unwrap_or_default();
        if current.replace("\r\n", "\n") != generated {
            eprintln!("{} is out of date; rerun the generator", out.display());
            std::process::exit(1);
        }
        println!("{} is current", out.display());
        return;
    }
    std::fs::write(&out, generated).expect("write generated table");
    println!("wrote {}", out.display());
}
