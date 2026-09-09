// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        // Boa and Nova construct their retained contexts on the UI thread.
        // Keep that thread as winit's platform thread while reserving enough
        // stack for the debug host as well as optimized builds.
        println!("cargo:rustc-link-arg-bin=ortet=/STACK:16777216");
    } else if target_os == "windows" && target_env == "gnu" {
        println!("cargo:rustc-link-arg-bin=ortet=-Wl,--stack,16777216");
    }
}
