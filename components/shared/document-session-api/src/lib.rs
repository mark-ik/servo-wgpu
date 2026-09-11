// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! # document-session-api
//!
//! The contracts an engine implements to be hosted: the retained session
//! traits (`SessionEngine`, `DocumentSession`) and their input, clip, find
//! and report vocabulary; the accessibility projection an engine produces;
//! the document-capability declaration; page capture; and the engine-id
//! namespace with the genet render-rung ladder.
//!
//! This is the engine-facing half of `inker`, split out under the platform
//! boundary plan (mere `design_docs/mere_docs/implementation_strategy/`
//! `2026-09-02_platform_boundary_and_repository_topology_plan.md`, P1) so
//! that Genet's engines depend on a crate Genet owns, and `inker`, the
//! controller that selects and orchestrates engines, can move to Mere and
//! depend downward on this. `inker` re-exports everything here, so its paths
//! still resolve for its consumers. Frame-type generic and paint-free.

/// Accessibility capability contract (R0 invariant: every surface declares
/// what it can expose to the a11y tree; degradation is declared, never silent).
pub mod a11y;

/// Shared retained/hosted document-control capability vocabulary.
pub mod capabilities;

/// The engine-id namespace and the genet render-rung ladder.
pub mod engine_ids;

/// Shared page-capture request/result vocabulary.
pub mod page_capture;

/// Session-engine traits and registry: retained document sessions producing
/// paint frames (the genet HTML lanes, smolweb native).
pub mod session_engine;

pub use a11y::{
    A11yCapability, DocumentA11yAction, DocumentA11yActionData, DocumentA11yActionRequest,
    DocumentA11yBounds, DocumentA11yClickTarget, DocumentA11yHasPopup, DocumentA11yLive,
    DocumentA11yNode, DocumentA11yNodeId, DocumentA11yOrientation, DocumentA11yPoint,
    DocumentA11yProjection, DocumentA11yRole, DocumentA11yState, DocumentA11ySupport,
    DocumentA11ySupportError, DocumentA11yToggled,
};
pub use capabilities::{
    CapabilityStatus, DocumentCapabilities, DocumentCapabilityStatus, WebFeatureStatus,
};
pub use engine_ids::*;
pub use page_capture::{
    CssExtent, CssPoint, PageCaptureImageArtifact, PageCaptureOutput, PageCaptureRequest,
    PageCaptureRequestId, PageCaptureScope, PageCaptureViewportFacts,
};
pub use session_engine::{
    ContentLineage, ContentReport, DocumentClip, DocumentClipArtifact, DocumentClipArtifactRole,
    DocumentFindDirection, DocumentFindMatch, DocumentFindQuery, DocumentFindReveal,
    DocumentFindState, DocumentSession, DocumentZoomState, EngineKindIndex, EngineKinds,
    OutlineEntry, SessionButtonState, SessionClick, SessionCursor, SessionEffect, SessionEngine,
    SessionError, SessionExternalTextureDraw, SessionFocusDirection, SessionFormMethod,
    SessionFormSubmission, SessionIme, SessionInput, SessionInputResult, SessionKey, SessionLink,
    SessionModifiers, SessionNavigationCommand, SessionPendingWork, SessionPointerButton,
    SessionRegistry, SessionScrollKey, SessionSpawnRequest, SessionTextTarget,
};
