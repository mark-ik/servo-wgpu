/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Per-document capabilities installed before authored scripts execute.

use script_runtime_api::WebGlFactory;

/// Host capabilities for one live scripted document.
///
/// Each navigation receives a fresh value. Fetching and realm routing remain
/// owned by the document's `ScriptResourceBridge`.
#[derive(Default)]
pub struct ScriptedDocumentOptions {
    pub webgl: Option<WebGlFactory>,
}
