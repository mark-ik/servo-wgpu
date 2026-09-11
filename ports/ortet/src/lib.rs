// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! **Ortet — the raw Genet host.**
//!
//! A genet is a whole clonal colony; an ortet is the original individual it
//! descends from. This is the reference individual of the engine: the one
//! headed port that proves Genet runs with no Mere crate anywhere in its
//! dependency cone. `support/ci/check_dependency_cones.py` witnesses that on
//! every CI run.
//!
//! It is a window, a document, and nothing else. No tabs, no tiles, no reader
//! lane, no smolweb, no profiles, no settings, no persistence. All of those are
//! product decisions, and product decisions live in Mere.
//!
//! Four small pieces:
//!
//! - [`args`] — the command line, and the address normalization the fetch lanes
//!   and `resolve_href` both rely on.
//! - [`fetch`] — the scheme split: local schemes to `genet-documents`'
//!   `LocalFetcher`, http(s) to netfetcher, everything else an honest miss.
//! - [`shell`] — the winit window, the per-frame shape, and navigation.
//! - [`receipt`] — one composed frame read back to a PNG with its digest.
//!
//! The library half exists so the argument parser and the scheme split can be
//! unit-tested without a GPU; `main.rs` is a thin wrapper over [`shell::run`].

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod a11y;
pub mod args;
#[cfg(not(target_arch = "wasm32"))]
pub mod fetch;
#[cfg(not(target_arch = "wasm32"))]
pub mod receipt;
#[cfg(not(target_arch = "wasm32"))]
pub mod shell;
#[cfg(target_arch = "wasm32")]
pub mod web;
#[cfg(all(not(target_arch = "wasm32"), feature = "scripted"))]
pub mod webgl;
