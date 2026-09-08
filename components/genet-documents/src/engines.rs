/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The lanes as inker session engines: `SessionEngine<Scene>` /
//! `DocumentSession<Scene>` impls wrapping [`LoadedDocument`], the scripted
//! document.
//!
//! Construction seams (fetchers, themes, cookie jars) live on the engine at
//! registration time; the spawn request stays plain data (session-engines
//! plan, review-resolved 2026-07-10).

#[cfg(feature = "scripted")]
use document_session_api::{DocumentCapabilities, DocumentCapabilityStatus};

#[cfg(feature = "scripted")]
fn retained_document_capabilities(find_reason: impl Into<String>) -> DocumentCapabilities {
    DocumentCapabilities {
        find_in_page: DocumentCapabilityStatus::unsupported(find_reason),
        page_zoom: DocumentCapabilityStatus::unsupported(
            "scripted and smolweb sessions do not expose page zoom",
        ),
        page_capture: DocumentCapabilityStatus::unsupported(
            "retained sessions do not capture rendered pages",
        ),
        navigation: DocumentCapabilityStatus::Partial {
            detail: "the host owns document lineage, policy, and refetch".into(),
        },
    }
}

#[cfg(any(feature = "livery", feature = "scripted"))]
mod clip;
#[cfg(feature = "livery")]
mod frames;
#[cfg(feature = "livery")]
mod livery;
#[cfg(feature = "scripted")]
mod scripted;
#[cfg(test)]
mod tests;

#[cfg(feature = "scripted")]
pub(crate) use clip::links_for_source_nodes;
#[cfg(any(feature = "livery", feature = "scripted"))]
pub(crate) use clip::{
    ClipRange, ClipSelection, content_report, semantic_clip_from_dom,
    semantic_clip_from_selection_with_links,
};
#[cfg(all(test, feature = "livery"))]
pub(crate) use livery::EditableKind;
#[cfg(feature = "livery")]
pub use livery::{LiveryDocumentSession, LiveryResourcePreparation, LiverySessionEngine};
#[cfg(feature = "scripted")]
pub use scripted::{ScriptedDocumentSession, ScriptedSessionEngine};
