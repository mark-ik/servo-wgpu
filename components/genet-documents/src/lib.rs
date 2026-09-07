/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Genet's retained document sessions: Livery HTML and scripted HTML as
//! session engines (2026-07-10 session-engines plan). The reader and smolweb
//! lanes and the remote fetch integration are `mere-document-lanes` since the
//! platform boundary plan's P1 split this crate by authority.
//!
//! These types began as pelt's convenience lanes; the formalization promotes
//! them to an engine-grade component. Each lane is a retained layout session
//! producing [`netrender::Scene`] frames on demand, with scroll, activation,
//! and (scripted) a tick + quiescence seam. [`engines`] wraps each lane in
//! `inker::SessionEngine<Scene>` so hosts spawn them through the
//! `SessionRegistry` instead of hand-matching engine ids; pelt consumes this
//! component like any other host.

mod fetch;

// Dependency-free link resolution, shared by `document`, the scripted lane,
// and hosts' chrome (moved with the lanes from pelt).
pub mod href;

#[cfg(feature = "scripted")]
pub use genet_scripted::{
    LiveryScriptedDocument, LiveryScriptedDocument as ScriptedDocument,
    ResourceFetcher as ScriptResourceFetcher, ScriptedEngine,
};

pub mod engines;

#[cfg(feature = "livery")]
pub use engines::{LiveryDocumentSession, LiveryResourcePreparation, LiverySessionEngine};
#[cfg(feature = "scripted")]
pub use engines::{ScriptedDocumentSession, ScriptedSessionEngine};
pub use fetch::{LocalFetcher, LocalFetcherWith, ResourceFetchPolicy};
pub use genet_host_api::ResourceFetcher;
pub use href::resolve_href;
