// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Engine-neutral document resource discovery.
//!
//! A host chooses byte policy through [`ResourceFetcher`]; this component
//! discovers resources, preserves stylesheet order and source identity, and
//! records every unresolved dependency. CSS and layout engines only consume the
//! resulting immutable records.

#![deny(unsafe_code)]

use std::collections::{HashMap, HashSet};

pub use genet_host_api::{ResourceFetcher, ResourceResponse};
use layout_dom_api::{LayoutDom, LocalName, Namespace};

type ByteFetcher<'a> = dyn FnMut(&str) -> Option<Vec<u8>> + 'a;
type ResponseFetcher<'a> = dyn FnMut(&str) -> Option<ResourceResponse> + 'a;

/// The HTML node kind which owns an author stylesheet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StylesheetOwner {
    Inline,
    Linked,
    Imported,
}

/// One leading `@import` rule retained in the resource graph. The resolver
/// keeps it even when the imported response is unavailable, so a CSSOM
/// consumer can distinguish a present rule with no child sheet from no rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedImportRule {
    /// The URL spelling from the import rule.
    pub authored_url: String,
    /// The URL resolved against the importing sheet's final identity.
    pub resolved_url: String,
    /// The supported import media condition, if one was present.
    pub media: Option<String>,
    /// The resource-graph identity of the loaded child sheet, if available.
    pub child_sheet_id: Option<u64>,
}

/// The import rule which introduced an imported stylesheet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StylesheetImportParent {
    pub sheet_id: u64,
    pub import_index: usize,
}

/// One author stylesheet, in document linking order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedStylesheet {
    /// Identity within one resolved resource graph. It connects retained import
    /// rules to their child sheets; document owners remain the stable identity
    /// for direct CSSOM sheets across later resolutions.
    pub sheet_id: u64,
    pub owner: StylesheetOwner,
    /// Opaque identity of the document `<style>` or `<link>` element that
    /// introduced this sheet. Imported sheets inherit their root owner's id;
    /// consumers use it to retain a live CSSOM sheet across re-resolution.
    pub owner_node: Option<u64>,
    /// Identity after redirect handling. `None` for inline sheets without a
    /// known document identity.
    pub source_url: Option<String>,
    /// The resolved URL given to the host before redirect handling. `None` for
    /// inline sheets and host-supplied sheets without a URL.
    pub requested_url: Option<String>,
    /// Response `Content-Type`, retained so consumers can explain why a linked
    /// sheet did or did not enter the cascade.
    pub content_type: Option<String>,
    /// Link `media`, retained for the selected style engine to evaluate.
    pub media: Option<String>,
    /// Leading imports in this sheet, in CSS rule order.
    pub imports: Vec<ResolvedImportRule>,
    /// The parent import for an imported sheet. Direct sheets have no parent.
    pub import_parent: Option<StylesheetImportParent>,
    pub text: String,
    pub document_order: u64,
}

/// Kinds of bytes a document engine may consume.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ResourceKind {
    Image,
    Font,
}

/// A host-fetched dependency, attributed both to its source spelling and to
/// its source-relative resolved identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedResource {
    pub kind: ResourceKind,
    pub authored_url: String,
    /// Canonical URL requested from the host after resolving the author's
    /// spelling against its owning document or stylesheet.
    pub resolved_url: String,
    /// Identity after the host's redirect policy.  It can differ from
    /// `resolved_url` for browser-fetched images and fonts.
    pub final_url: String,
    /// Response media type retained from the host when available.
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

/// The stable identity of one document dependency across successive resource
/// resolutions. The authored spelling plus its source-relative URL identifies
/// one DOM/CSS use, so two linked stylesheets can both use `url(icon.png)`
/// without collapsing into one live resource.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ResourceKey {
    pub kind: ResourceKind,
    pub authored_url: String,
    pub resolved_url: String,
}

impl From<&ResolvedResource> for ResourceKey {
    fn from(resource: &ResolvedResource) -> Self {
        Self {
            kind: resource.kind,
            authored_url: resource.authored_url.clone(),
            resolved_url: resource.resolved_url.clone(),
        }
    }
}

/// The resource portion of one live document reconciliation. Consumers receive
/// the full next ledger as well as this delta, so removal never needs an empty
/// byte sentinel and changed font bytes can replace prior registration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourceDelta {
    pub added: Vec<ResolvedResource>,
    pub updated: Vec<ResolvedResource>,
    pub removed: Vec<ResourceKey>,
}

impl ResourceDelta {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.updated.is_empty() && self.removed.is_empty()
    }
}

/// A dependency which did not become usable bytes. These diagnostics are part
/// of the resolved set so engines never have to represent a failed load as an
/// empty success.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceDiagnostic {
    LinkedStylesheetNoByteAuthority {
        authored_url: String,
        resolved_url: String,
    },
    LinkedStylesheetUnavailable {
        authored_url: String,
        resolved_url: String,
    },
    LinkedStylesheetInvalidUtf8 {
        authored_url: String,
        resolved_url: String,
    },
    LinkedStylesheetUnsupportedContentType {
        authored_url: String,
        resolved_url: String,
        content_type: String,
    },
    LinkedStylesheetBodyLimit {
        authored_url: String,
        resolved_url: String,
        max_bytes: usize,
    },
    ResourceUnavailable {
        kind: ResourceKind,
        authored_url: String,
        resolved_url: String,
    },
    UnsupportedScheme {
        kind: ResourceKind,
        authored_url: String,
        resolved_url: String,
    },
    ImportRuleUnavailable {
        source_url: Option<String>,
        authored_url: String,
        resolved_url: String,
    },
    ImportRuleInvalidUtf8 {
        source_url: Option<String>,
        authored_url: String,
        resolved_url: String,
    },
    ImportRuleUnsupportedContentType {
        source_url: Option<String>,
        authored_url: String,
        resolved_url: String,
        content_type: String,
    },
    ImportRuleCycle {
        source_url: Option<String>,
        resolved_url: String,
    },
    ImportRuleDepthLimit {
        source_url: Option<String>,
        resolved_url: String,
        max_depth: usize,
    },
    ImportRuleBodyLimit {
        source_url: Option<String>,
        authored_url: String,
        resolved_url: String,
        max_bytes: usize,
    },
    /// The selected cascade has no `@layer` or `@supports` import semantics.
    /// Retain a precise blocker instead of fetching and applying it incorrectly.
    ImportRuleUnsupportedCondition {
        source_url: Option<String>,
        authored_url: String,
        condition: String,
    },
    ImportRuleOutOfOrder {
        source_url: Option<String>,
    },
}

/// Host-configurable bounds for one static resource resolution pass. The
/// resolver is deliberately serial, so concurrent fetches are always one;
/// asynchronous hosts enforce their own transport concurrency before exposing
/// [`ResourceFetcher`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceLimits {
    pub max_import_depth: usize,
    pub max_stylesheet_bytes: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_import_depth: 16,
            max_stylesheet_bytes: 2 * 1024 * 1024,
        }
    }
}

/// The host-owned resource view of one parsed HTML document.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResolvedDocumentResources {
    pub document_url: Option<String>,
    pub stylesheets: Vec<ResolvedStylesheet>,
    pub resources: Vec<ResolvedResource>,
    pub diagnostics: Vec<ResourceDiagnostic>,
}

/// A canonical request the host still has to satisfy before an asynchronous
/// client can finish a resource ledger.  The URL is already resolved against
/// the document or stylesheet identity that introduced it.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PendingResourceRequest {
    pub url: String,
}

/// A host attempted to provision a URL which is not pending in this stage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceProvisionError {
    pub request: PendingResourceRequest,
}

/// Immutable response records collected by a host before it crosses the
/// synchronous [`ResourceFetcher`] boundary.  It deliberately holds bytes and
/// metadata only, never a transport client or cache handle.
#[derive(Clone, Debug, Default)]
pub struct ResourceResponseSnapshot {
    responses: HashMap<String, ResourceResponse>,
}

impl ResourceResponseSnapshot {
    pub fn insert(&mut self, requested_url: impl Into<String>, response: ResourceResponse) {
        self.responses.insert(requested_url.into(), response);
    }
}

impl ResourceFetcher for ResourceResponseSnapshot {
    fn fetch(&self, url: &str) -> Option<Vec<u8>> {
        self.responses
            .get(url)
            .map(|response| response.bytes.clone())
    }

    fn fetch_response(&self, url: &str) -> Option<ResourceResponse> {
        self.responses.get(url).cloned()
    }
}

/// A staged, host-driven resource resolution.
///
/// The document component continues to own URL discovery and CSS ordering.
/// An asynchronous host calls [`Self::pending_requests`], fetches those URLs
/// under its own policy, provides immutable [`ResourceResponse`] snapshots,
/// and repeats until the resolver has no work left.  A failed request is
/// explicitly recorded with [`Self::provide_unavailable`] so diagnostics in
/// the final ledger retain the failed dependency.
pub struct StagedResourceResolution<D: LayoutDom> {
    dom: D,
    document_url: Option<String>,
    limits: ResourceLimits,
    responses: ResourceResponseSnapshot,
    unavailable: HashSet<String>,
}

impl<D: LayoutDom> StagedResourceResolution<D> {
    pub fn new(dom: D, document_url: Option<&str>, limits: ResourceLimits) -> Self {
        Self {
            dom,
            document_url: document_url.map(str::to_owned),
            limits,
            responses: ResourceResponseSnapshot::default(),
            unavailable: HashSet::new(),
        }
    }

    /// Return every currently discoverable request once, in discovery order.
    /// Additional calls after a stylesheet response is provided expose its
    /// imports and stylesheet-relative dependencies.
    pub fn pending_requests(&self) -> Vec<PendingResourceRequest> {
        let mut pending = Vec::new();
        let mut seen = HashSet::new();
        let mut fetch = |url: &str| match self.responses.fetch_response(url) {
            Some(response) => Some(response),
            None if self.unavailable.contains(url) => None,
            None => {
                if seen.insert(url.to_owned()) {
                    pending.push(PendingResourceRequest {
                        url: url.to_owned(),
                    });
                }
                None
            },
        };
        let _ = resolve_responses_with_limits(
            &self.dom,
            self.document_url.as_deref(),
            &mut fetch,
            self.limits,
        );
        pending
    }

    /// Store the full immutable response snapshot for a canonical pending URL.
    pub fn provide_response(
        &mut self,
        request: PendingResourceRequest,
        response: ResourceResponse,
    ) -> Result<(), ResourceProvisionError> {
        self.ensure_pending(&request)?;
        self.unavailable.remove(&request.url);
        self.responses.insert(request.url, response);
        Ok(())
    }

    /// Record an attempted request which the host could not provide.
    pub fn provide_unavailable(
        &mut self,
        request: PendingResourceRequest,
    ) -> Result<(), ResourceProvisionError> {
        self.ensure_pending(&request)?;
        self.unavailable.insert(request.url);
        Ok(())
    }

    /// Finish only when the host has accounted for all currently reachable
    /// dependencies.  The returned ledger is the same immutable resource set
    /// used by synchronous `ResourceFetcher` hosts.
    pub fn finish(&self) -> Result<ResolvedDocumentResources, Vec<PendingResourceRequest>> {
        let pending = self.pending_requests();
        if pending.is_empty() {
            Ok(self.resolve())
        } else {
            Err(pending)
        }
    }

    fn resolve(&self) -> ResolvedDocumentResources {
        let mut fetch = |url: &str| self.responses.fetch_response(url);
        resolve_responses_with_limits(
            &self.dom,
            self.document_url.as_deref(),
            &mut fetch,
            self.limits,
        )
    }

    fn ensure_pending(
        &self,
        request: &PendingResourceRequest,
    ) -> Result<(), ResourceProvisionError> {
        self.pending_requests()
            .contains(request)
            .then_some(())
            .ok_or_else(|| ResourceProvisionError {
                request: request.clone(),
            })
    }

    /// Recover the caller's document after finishing its resource ledger.
    pub fn into_document(self) -> D {
        self.dom
    }
}

impl ResolvedDocumentResources {
    /// Resolve all loadable dependencies through the host's byte contract.
    pub fn resolve<D, Fetch>(dom: &D, document_url: Option<&str>, fetcher: &Fetch) -> Self
    where
        D: LayoutDom,
        Fetch: ResourceFetcher + ?Sized,
    {
        let mut fetch = |url: &str| fetcher.fetch_response(url);
        resolve_responses_with_limits(dom, document_url, &mut fetch, ResourceLimits::default())
    }

    /// Resolve resources with explicit host-selected bounds for recursive CSS
    /// imports and individual stylesheet bodies.
    pub fn resolve_with_limits<D, Fetch>(
        dom: &D,
        document_url: Option<&str>,
        fetcher: &Fetch,
        limits: ResourceLimits,
    ) -> Self
    where
        D: LayoutDom,
        Fetch: ResourceFetcher + ?Sized,
    {
        let mut fetch = |url: &str| fetcher.fetch_response(url);
        resolve_responses_with_limits(dom, document_url, &mut fetch, limits)
    }

    /// Discover only inline content. Linked resources remain visible as
    /// diagnostics because this parse has no authority to fetch their bytes.
    pub fn discover<D>(dom: &D, document_url: Option<&str>) -> Self
    where
        D: LayoutDom,
    {
        collect(dom, document_url, None, ResourceLimits::default())
    }

    /// Return the text in retained author-sheet order.
    pub fn stylesheet_text(&self) -> Vec<&str> {
        self.stylesheets
            .iter()
            .map(|sheet| sheet.text.as_str())
            .collect()
    }

    /// Compare this next resolution to a prior ledger. Resource ownership is
    /// keyed by its consumer-visible spelling, source-relative URL, and kind;
    /// changed bytes become a replacement.
    pub fn resource_delta_from(&self, previous: &Self) -> ResourceDelta {
        let old = previous
            .resources
            .iter()
            .map(|resource| (ResourceKey::from(resource), resource))
            .collect::<HashMap<_, _>>();
        let next = self
            .resources
            .iter()
            .map(|resource| (ResourceKey::from(resource), resource))
            .collect::<HashMap<_, _>>();
        let mut delta = ResourceDelta::default();
        for resource in &self.resources {
            let key = ResourceKey::from(resource);
            match old.get(&key) {
                None => delta.added.push(resource.clone()),
                Some(previous) if *previous != resource => delta.updated.push(resource.clone()),
                Some(_) => {},
            }
        }
        for resource in &previous.resources {
            let key = ResourceKey::from(resource);
            if !next.contains_key(&key) {
                delta.removed.push(key);
            }
        }
        delta
    }
}

/// Resolve resources through a closure. This is useful for compatibility
/// adapters which already have a host byte callback without inventing another
/// fetch trait.
pub fn resolve_with<D>(
    dom: &D,
    document_url: Option<&str>,
    fetch: &mut ByteFetcher<'_>,
) -> ResolvedDocumentResources
where
    D: LayoutDom,
{
    let mut responses = |url: &str| fetch(url).map(|bytes| ResourceResponse::new(url, bytes));
    resolve_responses_with_limits(dom, document_url, &mut responses, ResourceLimits::default())
}

/// Response-aware closure entry point for compatibility adapters that already
/// own response metadata and do not need to implement the host trait.
pub fn resolve_responses_with_limits<D>(
    dom: &D,
    document_url: Option<&str>,
    fetch: &mut ResponseFetcher<'_>,
    limits: ResourceLimits,
) -> ResolvedDocumentResources
where
    D: LayoutDom,
{
    collect(dom, document_url, Some(fetch), limits)
}

fn collect<D>(
    dom: &D,
    document_url: Option<&str>,
    mut fetch: Option<&mut ResponseFetcher<'_>>,
    limits: ResourceLimits,
) -> ResolvedDocumentResources
where
    D: LayoutDom,
{
    let mut result = ResolvedDocumentResources {
        document_url: document_url.map(str::to_owned),
        ..Default::default()
    };
    let mut direct_sheets = Vec::new();
    collect_stylesheets(
        dom,
        dom.document(),
        document_url,
        &mut fetch,
        &mut direct_sheets,
        &mut result.diagnostics,
        limits,
    );

    let mut order = StylesheetOrder::default();
    let mut cached_sheets = HashMap::<String, Option<ResourceResponse>>::new();
    for sheet in direct_sheets {
        let mut sheet = sheet;
        sheet.sheet_id = order.allocate_sheet_id();
        expand_stylesheet(
            sheet,
            &mut fetch,
            &mut cached_sheets,
            &mut ImportTrail::default(),
            limits,
            &mut order,
            &mut result,
        );
    }

    let mut cached = HashMap::<String, Option<ResourceResponse>>::new();
    collect_document_resources(
        dom,
        dom.document(),
        document_url,
        &mut fetch,
        &mut cached,
        &mut result,
    );
    let sheets = result.stylesheets.clone();
    for sheet in &sheets {
        collect_stylesheet_resources(
            &sheet.text,
            sheet.source_url.as_deref(),
            &mut fetch,
            &mut cached,
            &mut result,
        );
    }
    result
}

fn collect_stylesheets<D>(
    dom: &D,
    node: D::NodeId,
    document_url: Option<&str>,
    fetch: &mut Option<&mut ResponseFetcher<'_>>,
    sheets: &mut Vec<ResolvedStylesheet>,
    diagnostics: &mut Vec<ResourceDiagnostic>,
    limits: ResourceLimits,
) where
    D: LayoutDom,
{
    if dom
        .element_name(node)
        .is_some_and(|name| name.local.as_ref().eq_ignore_ascii_case("style"))
    {
        let mut text = String::new();
        collect_text(dom, node, &mut text);
        if !text.trim().is_empty() {
            sheets.push(ResolvedStylesheet {
                sheet_id: 0,
                owner: StylesheetOwner::Inline,
                owner_node: Some(dom.opaque_id(node)),
                source_url: document_url.map(str::to_owned),
                requested_url: None,
                content_type: None,
                media: None,
                imports: Vec::new(),
                import_parent: None,
                text,
                document_order: 0,
            });
        }
    }

    if is_stylesheet_link(dom, node) {
        let namespace = Namespace::default();
        let href = LocalName::from("href");
        let media = LocalName::from("media");
        if let Some(authored_url) = dom
            .attribute(node, &namespace, &href)
            .map(str::trim)
            .filter(|href| !href.is_empty())
        {
            let resolved_url = resolve_url(document_url, authored_url);
            let media = dom
                .attribute(node, &namespace, &media)
                .map(str::trim)
                .filter(|media| !media.is_empty())
                .map(str::to_owned);
            match fetch.as_deref_mut() {
                None => diagnostics.push(ResourceDiagnostic::LinkedStylesheetNoByteAuthority {
                    authored_url: authored_url.to_owned(),
                    resolved_url,
                }),
                Some(fetch) => match fetch(&resolved_url) {
                    Some(response) if !is_css_response(response.content_type.as_deref()) => {
                        diagnostics.push(
                            ResourceDiagnostic::LinkedStylesheetUnsupportedContentType {
                                authored_url: authored_url.to_owned(),
                                resolved_url,
                                content_type: response.content_type.unwrap_or_default(),
                            },
                        );
                    },
                    Some(response) if response.bytes.len() > limits.max_stylesheet_bytes => {
                        diagnostics.push(ResourceDiagnostic::LinkedStylesheetBodyLimit {
                            authored_url: authored_url.to_owned(),
                            resolved_url,
                            max_bytes: limits.max_stylesheet_bytes,
                        });
                    },
                    Some(response) => match String::from_utf8(response.bytes) {
                        Ok(text) => {
                            sheets.push(ResolvedStylesheet {
                                sheet_id: 0,
                                owner: StylesheetOwner::Linked,
                                owner_node: Some(dom.opaque_id(node)),
                                source_url: Some(response.final_url),
                                requested_url: Some(resolved_url),
                                content_type: response.content_type,
                                media,
                                imports: Vec::new(),
                                import_parent: None,
                                text,
                                document_order: 0,
                            });
                        },
                        Err(_) => {
                            diagnostics.push(ResourceDiagnostic::LinkedStylesheetInvalidUtf8 {
                                authored_url: authored_url.to_owned(),
                                resolved_url,
                            })
                        },
                    },
                    None => diagnostics.push(ResourceDiagnostic::LinkedStylesheetUnavailable {
                        authored_url: authored_url.to_owned(),
                        resolved_url,
                    }),
                },
            }
        }
    }

    for child in dom.dom_children(node) {
        collect_stylesheets(dom, child, document_url, fetch, sheets, diagnostics, limits);
    }
}

#[derive(Debug)]
struct ImportRule {
    authored_url: String,
    media: Option<String>,
    unsupported_condition: Option<String>,
}

#[derive(Debug)]
struct ImportScan {
    imports: Vec<ImportRule>,
    remaining: String,
    out_of_order: bool,
}

#[derive(Default)]
struct ImportTrail {
    active: Vec<String>,
    depth: usize,
}

#[derive(Default)]
struct StylesheetOrder {
    document_order: u64,
    next_sheet_id: u64,
}

impl StylesheetOrder {
    fn allocate_sheet_id(&mut self) -> u64 {
        let id = self.next_sheet_id;
        self.next_sheet_id = self.next_sheet_id.saturating_add(1);
        id
    }
}

fn expand_stylesheet(
    mut sheet: ResolvedStylesheet,
    fetch: &mut Option<&mut ResponseFetcher<'_>>,
    cached: &mut HashMap<String, Option<ResourceResponse>>,
    trail: &mut ImportTrail,
    limits: ResourceLimits,
    order: &mut StylesheetOrder,
    result: &mut ResolvedDocumentResources,
) {
    let source_url = sheet.source_url.clone();
    let pushed_current = source_url.as_ref().is_some_and(|source| {
        !trail
            .active
            .iter()
            .any(|active_source| active_source == source)
    });
    if pushed_current {
        trail
            .active
            .push(source_url.clone().expect("checked source URL"));
    }

    let scan = scan_leading_imports(&sheet.text);
    sheet.text = scan.remaining;
    sheet.imports = scan
        .imports
        .iter()
        .map(|import| ResolvedImportRule {
            authored_url: import.authored_url.clone(),
            resolved_url: resolve_url(source_url.as_deref(), &import.authored_url),
            media: import.media.clone(),
            child_sheet_id: None,
        })
        .collect();
    if scan.out_of_order {
        result
            .diagnostics
            .push(ResourceDiagnostic::ImportRuleOutOfOrder {
                source_url: source_url.clone(),
            });
    }
    for (import_index, import) in scan.imports.into_iter().enumerate() {
        if let Some(condition) = import.unsupported_condition {
            result
                .diagnostics
                .push(ResourceDiagnostic::ImportRuleUnsupportedCondition {
                    source_url: source_url.clone(),
                    authored_url: import.authored_url,
                    condition,
                });
            continue;
        }
        let resolved_url = resolve_url(source_url.as_deref(), &import.authored_url);
        if trail.depth >= limits.max_import_depth {
            result
                .diagnostics
                .push(ResourceDiagnostic::ImportRuleDepthLimit {
                    source_url: source_url.clone(),
                    resolved_url,
                    max_depth: limits.max_import_depth,
                });
            continue;
        }
        let response = match fetch.as_deref_mut() {
            Some(fetcher) => cached
                .entry(resolved_url.clone())
                .or_insert_with(|| fetcher(&resolved_url))
                .clone(),
            None => {
                result
                    .diagnostics
                    .push(ResourceDiagnostic::ImportRuleUnavailable {
                        source_url: source_url.clone(),
                        authored_url: import.authored_url,
                        resolved_url,
                    });
                continue;
            },
        };
        let Some(response) = response else {
            result
                .diagnostics
                .push(ResourceDiagnostic::ImportRuleUnavailable {
                    source_url: source_url.clone(),
                    authored_url: import.authored_url,
                    resolved_url,
                });
            continue;
        };
        if !is_css_response(response.content_type.as_deref()) {
            result
                .diagnostics
                .push(ResourceDiagnostic::ImportRuleUnsupportedContentType {
                    source_url: source_url.clone(),
                    authored_url: import.authored_url,
                    resolved_url,
                    content_type: response.content_type.clone().unwrap_or_default(),
                });
            continue;
        }
        if response.bytes.len() > limits.max_stylesheet_bytes {
            result
                .diagnostics
                .push(ResourceDiagnostic::ImportRuleBodyLimit {
                    source_url: source_url.clone(),
                    authored_url: import.authored_url,
                    resolved_url,
                    max_bytes: limits.max_stylesheet_bytes,
                });
            continue;
        }
        if trail
            .active
            .iter()
            .any(|active_source| active_source == &response.final_url)
        {
            result
                .diagnostics
                .push(ResourceDiagnostic::ImportRuleCycle {
                    source_url: source_url.clone(),
                    resolved_url,
                });
            continue;
        }
        let text = match String::from_utf8(response.bytes.clone()) {
            Ok(text) => text,
            Err(_) => {
                result
                    .diagnostics
                    .push(ResourceDiagnostic::ImportRuleInvalidUtf8 {
                        source_url: source_url.clone(),
                        authored_url: import.authored_url,
                        resolved_url,
                    });
                continue;
            },
        };
        let text = import
            .media
            .as_deref()
            .map_or(text.clone(), |media| wrap_import_media(&text, media));
        let child_sheet_id = order.allocate_sheet_id();
        sheet.imports[import_index].child_sheet_id = Some(child_sheet_id);
        trail.depth = trail.depth.saturating_add(1);
        expand_stylesheet(
            ResolvedStylesheet {
                sheet_id: child_sheet_id,
                owner: StylesheetOwner::Imported,
                owner_node: sheet.owner_node,
                source_url: Some(response.final_url.clone()),
                requested_url: Some(resolved_url),
                content_type: response.content_type.clone(),
                media: sheet.media.clone(),
                imports: Vec::new(),
                import_parent: Some(StylesheetImportParent {
                    sheet_id: sheet.sheet_id,
                    import_index,
                }),
                text,
                document_order: 0,
            },
            fetch,
            cached,
            trail,
            limits,
            order,
            result,
        );
        trail.depth = trail.depth.saturating_sub(1);
    }
    sheet.document_order = order.document_order;
    order.document_order = order.document_order.saturating_add(1);
    result.stylesheets.push(sheet);
    if pushed_current {
        trail.active.pop();
    }
}

fn is_css_response(content_type: Option<&str>) -> bool {
    content_type.is_none_or(|content_type| {
        content_type
            .split_once(';')
            .map_or(content_type, |(primary, _)| primary)
            .trim()
            .eq_ignore_ascii_case("text/css")
    })
}

fn scan_leading_imports(input: &str) -> ImportScan {
    let mut cursor = 0;
    let mut imports = Vec::new();
    loop {
        skip_css_trivia(input, &mut cursor);
        let Some(after_keyword) = after_import_keyword(input, cursor) else {
            break;
        };
        let Some(end) = find_import_terminator(input, after_keyword) else {
            break;
        };
        let prelude = input[after_keyword..end].trim();
        let Some(import) = parse_import_rule(prelude) else {
            break;
        };
        imports.push(import);
        cursor = end + 1;
    }
    let remaining = input[cursor..].to_owned();
    let out_of_order = contains_import_keyword(&remaining);
    ImportScan {
        imports,
        remaining,
        out_of_order,
    }
}

fn skip_css_trivia(input: &str, cursor: &mut usize) {
    loop {
        while input[*cursor..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
        {
            *cursor += input[*cursor..]
                .chars()
                .next()
                .expect("checked char")
                .len_utf8();
        }
        if !input[*cursor..].starts_with("/*") {
            return;
        }
        let Some(end) = input[*cursor + 2..].find("*/") else {
            *cursor = input.len();
            return;
        };
        *cursor += end + 4;
    }
}

fn after_import_keyword(input: &str, cursor: usize) -> Option<usize> {
    let rest = input.get(cursor..)?;
    let keyword = rest.get(..7)?;
    if !keyword.eq_ignore_ascii_case("@import") {
        return None;
    }
    let after = cursor + 7;
    input
        .get(after..)
        .and_then(|rest| rest.chars().next())
        .filter(|character| {
            character.is_whitespace() || matches!(character, '\'' | '"' | 'u' | 'U')
        })?;
    Some(after)
}

fn find_import_terminator(input: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    let mut parentheses = 0_u32;
    for (offset, character) in input[start..].char_indices() {
        if let Some(open_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == open_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '(' => parentheses = parentheses.saturating_add(1),
            ')' => parentheses = parentheses.saturating_sub(1),
            ';' if parentheses == 0 => return Some(start + offset),
            _ => {},
        }
    }
    None
}

fn parse_import_rule(prelude: &str) -> Option<ImportRule> {
    let (authored_url, rest) = if let Some(after_url) = prelude
        .get(..3)
        .filter(|prefix| prefix.eq_ignore_ascii_case("url"))
        .and_then(|_| prelude.get(3..))
    {
        let after_url = after_url.trim_start();
        let body = after_url.strip_prefix('(')?;
        let close = body.rfind(')')?;
        (
            body[..close].trim().trim_matches(['\'', '"']).to_owned(),
            &body[close + 1..],
        )
    } else {
        let quote = prelude
            .chars()
            .next()
            .filter(|quote| matches!(quote, '\'' | '"'))?;
        let after_quote = &prelude[quote.len_utf8()..];
        let close = after_quote.find(quote)?;
        (
            after_quote[..close].to_owned(),
            &after_quote[close + quote.len_utf8()..],
        )
    };
    if authored_url.is_empty() {
        return None;
    }
    let condition = rest.trim();
    let unsupported_condition = condition
        .split_ascii_whitespace()
        .next()
        .filter(|token| {
            token.eq_ignore_ascii_case("layer")
                || token.starts_with("layer(")
                || token.eq_ignore_ascii_case("supports")
                || token.starts_with("supports(")
        })
        .map(str::to_owned);
    Some(ImportRule {
        authored_url,
        media: unsupported_condition
            .is_none()
            .then(|| condition.to_owned())
            .filter(|media| !media.is_empty()),
        unsupported_condition,
    })
}

fn wrap_import_media(text: &str, media: &str) -> String {
    format!("@media {media} {{\n{text}\n}}")
}

fn contains_import_keyword(input: &str) -> bool {
    let mut cursor = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut block_depth = 0_u32;
    while cursor < input.len() {
        if quote.is_none() && input[cursor..].starts_with("/*") {
            let Some(end) = input[cursor + 2..].find("*/") else {
                return false;
            };
            cursor += end + 4;
            continue;
        }
        let character = input[cursor..].chars().next().expect("checked cursor");
        if let Some(open_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == open_quote {
                quote = None;
            }
        } else {
            match character {
                '\'' | '"' => quote = Some(character),
                '{' => block_depth = block_depth.saturating_add(1),
                '}' => block_depth = block_depth.saturating_sub(1),
                '@' if block_depth == 0 && after_import_keyword(input, cursor).is_some() => {
                    return true;
                },
                _ => {},
            }
        }
        cursor += character.len_utf8();
    }
    false
}

fn collect_document_resources<D>(
    dom: &D,
    node: D::NodeId,
    document_url: Option<&str>,
    fetch: &mut Option<&mut ResponseFetcher<'_>>,
    cached: &mut HashMap<String, Option<ResourceResponse>>,
    result: &mut ResolvedDocumentResources,
) where
    D: LayoutDom,
{
    let namespace = Namespace::default();
    let loading = LocalName::from("loading");
    let attribute = match dom.element_name(node).map(|name| name.local.as_ref()) {
        Some("img" | "embed") => Some("src"),
        Some("object") => Some("data"),
        Some("video") => Some("poster"),
        _ => None,
    };
    let lazy = dom
        .element_name(node)
        .is_some_and(|name| name.local.as_ref().eq_ignore_ascii_case("img"))
        && dom
            .attribute(node, &namespace, &loading)
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("lazy"));
    if !lazy
        && let Some(attribute) = attribute
        && let Some(authored_url) = dom
            .attribute(node, &namespace, &LocalName::from(attribute))
            .map(str::trim)
            .filter(|url| !url.is_empty())
    {
        collect_resource(
            ResourceKind::Image,
            authored_url,
            document_url,
            fetch,
            cached,
            result,
        );
    }
    for child in dom.dom_children(node) {
        collect_document_resources(dom, child, document_url, fetch, cached, result);
    }
}

fn collect_stylesheet_resources(
    css: &str,
    base_url: Option<&str>,
    fetch: &mut Option<&mut ResponseFetcher<'_>>,
    cached: &mut HashMap<String, Option<ResourceResponse>>,
    result: &mut ResolvedDocumentResources,
) {
    let mut cursor = 0;
    let lower = css.to_ascii_lowercase();
    while let Some(found) = lower[cursor..].find("url(") {
        let start = cursor + found + 4;
        let Some(close) = css[start..].find(')') else {
            break;
        };
        let authored_url = css[start..start + close].trim().trim_matches(['\'', '"']);
        if !authored_url.is_empty() {
            collect_resource(
                resource_kind_for_css_url(authored_url),
                authored_url,
                base_url,
                fetch,
                cached,
                result,
            );
        }
        cursor = start + close + 1;
    }
}

fn collect_resource(
    kind: ResourceKind,
    authored_url: &str,
    base_url: Option<&str>,
    fetch: &mut Option<&mut ResponseFetcher<'_>>,
    cached: &mut HashMap<String, Option<ResourceResponse>>,
    result: &mut ResolvedDocumentResources,
) {
    if authored_url.starts_with('#') {
        return;
    }
    let resolved_url = resolve_url(base_url, authored_url);
    if result.resources.iter().any(|resource| {
        resource.kind == kind
            && resource.authored_url == authored_url
            && resource.resolved_url == resolved_url
    }) {
        return;
    }
    let Some(fetch) = fetch.as_deref_mut() else {
        result
            .diagnostics
            .push(ResourceDiagnostic::ResourceUnavailable {
                kind,
                authored_url: authored_url.to_owned(),
                resolved_url,
            });
        return;
    };
    let response = cached
        .entry(resolved_url.clone())
        .or_insert_with(|| fetch(&resolved_url));
    match response {
        Some(response) => result.resources.push(ResolvedResource {
            kind,
            authored_url: authored_url.to_owned(),
            resolved_url,
            final_url: response.final_url.clone(),
            content_type: response.content_type.clone(),
            bytes: response.bytes.clone(),
        }),
        None if explicitly_unsupported_scheme(&resolved_url) => {
            result
                .diagnostics
                .push(ResourceDiagnostic::UnsupportedScheme {
                    kind,
                    authored_url: authored_url.to_owned(),
                    resolved_url,
                });
        },
        None => result
            .diagnostics
            .push(ResourceDiagnostic::ResourceUnavailable {
                kind,
                authored_url: authored_url.to_owned(),
                resolved_url,
            }),
    }
}

fn collect_text<D>(dom: &D, node: D::NodeId, output: &mut String)
where
    D: LayoutDom,
{
    // Static HTML keeps text in text children. The live scripted DOM stores a
    // `textContent` replacement on the target element until its fuller child
    // replacement model lands. Prefer that direct value so a dynamic `<style>`
    // and an edited existing `<style>` never retain stale child text.
    if let Some(text) = dom.text(node) {
        output.push_str(text);
        return;
    }
    for child in dom.dom_children(node) {
        collect_text(dom, child, output);
    }
}

fn is_stylesheet_link<D>(dom: &D, node: D::NodeId) -> bool
where
    D: LayoutDom,
{
    let namespace = Namespace::default();
    let rel = LocalName::from("rel");
    dom.element_name(node)
        .is_some_and(|name| name.local.as_ref().eq_ignore_ascii_case("link"))
        && dom.attribute(node, &namespace, &rel).is_some_and(|tokens| {
            tokens
                .split_ascii_whitespace()
                .any(|token| token.eq_ignore_ascii_case("stylesheet"))
        })
}

fn resource_kind_for_css_url(url: &str) -> ResourceKind {
    if data_url_is_font(url) {
        return ResourceKind::Font;
    }
    let path = url
        .split_once(['?', '#'])
        .map_or(url, |(path, _)| path)
        .to_ascii_lowercase();
    if [".woff", ".woff2", ".ttf", ".otf", ".eot"]
        .iter()
        .any(|extension| path.ends_with(extension))
    {
        ResourceKind::Font
    } else {
        ResourceKind::Image
    }
}

fn data_url_is_font(url: &str) -> bool {
    let Some(metadata) = url.strip_prefix("data:") else {
        return false;
    };
    let media_type = metadata
        .split_once(',')
        .map_or(metadata, |(metadata, _)| metadata)
        .split_once(';')
        .map_or(metadata, |(media_type, _)| media_type);
    matches!(
        media_type.to_ascii_lowercase().as_str(),
        "font/ttf"
            | "font/otf"
            | "font/woff"
            | "font/woff2"
            | "application/font-sfnt"
            | "application/font-woff"
            | "application/font-woff2"
            | "application/x-font-ttf"
            | "application/x-font-woff"
    )
}

fn explicitly_unsupported_scheme(url: &str) -> bool {
    let scheme = url.split_once(':').map(|(scheme, _)| scheme);
    matches!(scheme, Some("about" | "javascript" | "mailto" | "tel"))
}

/// Resolve an authored URL against the identity of the resource containing it.
/// It keeps bare Windows paths local-first while retaining remote root-relative
/// and scheme-relative URL behavior.
pub fn resolve_url(base_url: Option<&str>, authored_url: &str) -> String {
    let Some(base) = base_url else {
        return authored_url.to_owned();
    };
    if has_scheme(authored_url) {
        return normalize_remote_path(authored_url.to_owned());
    }
    if let Some((scheme, authority_end)) = remote_origin(base) {
        if let Some(network_path) = authored_url.strip_prefix("//") {
            return normalize_remote_path(format!("{scheme}://{network_path}"));
        }
        if authored_url.starts_with('/') {
            return normalize_remote_path(format!("{}{}", &base[..authority_end], authored_url));
        }
        let page_end = base.find(['?', '#']).unwrap_or(base.len());
        if authored_url.starts_with('?') || authored_url.starts_with('#') {
            return format!("{}{}", &base[..page_end], authored_url);
        }
        let page = &base[..page_end];
        let path_start = authority_end.min(page.len());
        if let Some(index) = page[path_start..].rfind('/') {
            return normalize_remote_path(format!(
                "{}{}",
                &page[..path_start + index + 1],
                authored_url
            ));
        }
        return normalize_remote_path(format!("{page}/{authored_url}"));
    }
    if authored_url.starts_with('/') || authored_url.starts_with('\\') {
        return authored_url.to_owned();
    }
    let cut = base.rfind(['/', '\\']).map_or(0, |index| index + 1);
    format!("{}{}", &base[..cut], authored_url)
}

/// Remove literal `.` and `..` segments from a remote URL path. The host's
/// byte fetcher receives one canonical source identity, so a stylesheet's
/// `../font.woff2` does not become a distinct, unfetchable spelling. Local
/// paths retain their original spelling because the local-first host accepts
/// normal filesystem path traversal directly.
fn normalize_remote_path(url: String) -> String {
    let Some((_, authority_end)) = remote_origin(&url) else {
        return url;
    };
    let resource_end = url.find(['?', '#']).unwrap_or(url.len());
    let path = &url[authority_end..resource_end];
    if !path.split('/').any(|segment| matches!(segment, "." | "..")) {
        return url;
    }

    let leading_slash = path.starts_with('/');
    let trailing_slash = path.ends_with('/') || path.ends_with("/.") || path.ends_with("/..");
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {},
            ".." => {
                segments.pop();
            },
            segment => segments.push(segment),
        }
    }
    let mut normalized = String::new();
    if leading_slash {
        normalized.push('/');
    }
    normalized.push_str(&segments.join("/"));
    if trailing_slash && !normalized.ends_with('/') {
        normalized.push('/');
    }
    format!(
        "{}{}{}",
        &url[..authority_end],
        normalized,
        &url[resource_end..]
    )
}

fn has_scheme(url: &str) -> bool {
    match url.find(':') {
        Some(index) if index > 0 => url[..index].chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        }),
        _ => false,
    }
}

fn remote_origin(base: &str) -> Option<(&str, usize)> {
    let scheme_end = base.find("://")?;
    let scheme = &base[..scheme_end];
    if scheme.is_empty()
        || !scheme.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
    {
        return None;
    }
    let after_authority = &base[scheme_end + 3..];
    let authority_len = after_authority
        .find(['/', '?', '#'])
        .unwrap_or(after_authority.len());
    Some((scheme, scheme_end + 3 + authority_len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use genet_static_dom::StaticDocument;

    struct Fetch;
    impl ResourceFetcher for Fetch {
        fn fetch(&self, url: &str) -> Option<Vec<u8>> {
            match url {
                "https://example.test/page/site.css" => Some(
                    b".hero { background-image: url(images/hero.png); } @font-face { src: url(fonts/text.woff2); }"
                        .to_vec(),
                ),
                "https://example.test/page/images/hero.png" => Some(vec![1, 2, 3]),
                "https://example.test/page/fonts/text.woff2" => Some(vec![4, 5, 6]),
                "https://example.test/page/logo.png" => Some(vec![7]),
                _ => None,
            }
        }
    }

    #[test]
    fn preserves_interleaved_sheet_order_media_and_source_identity() {
        let document = StaticDocument::parse(
            r#"<style>.first { color: red }</style><link rel="preload stylesheet" href="site.css" media="screen"><style>.last { color: blue }</style>"#,
        );
        let resources = ResolvedDocumentResources::resolve(
            &document,
            Some("https://example.test/page/index.html"),
            &Fetch,
        );
        assert_eq!(resources.stylesheets.len(), 3);
        assert_eq!(resources.stylesheets[0].owner, StylesheetOwner::Inline);
        assert_eq!(resources.stylesheets[1].media.as_deref(), Some("screen"));
        assert_eq!(
            resources.stylesheets[1].source_url.as_deref(),
            Some("https://example.test/page/site.css")
        );
        assert!(resources.stylesheets[2].text.contains("blue"));
        assert!(
            resources
                .resources
                .iter()
                .any(|resource| resource.kind == ResourceKind::Font)
        );
    }

    #[test]
    fn classifies_data_font_face_as_a_font_resource() {
        struct DataFontFetch;
        impl ResourceFetcher for DataFontFetch {
            fn fetch(&self, url: &str) -> Option<Vec<u8>> {
                url.starts_with("data:font/ttf;base64,")
                    .then_some(vec![0, 1, 2])
            }
        }

        let document = StaticDocument::parse(
            r#"<style>@font-face { font-family: receipt; src: url(data:font/ttf;base64,AAEC); }</style>"#,
        );
        let resources = ResolvedDocumentResources::resolve(&document, None, &DataFontFetch);
        assert_eq!(
            resources.resources,
            vec![ResolvedResource {
                kind: ResourceKind::Font,
                authored_url: "data:font/ttf;base64,AAEC".to_owned(),
                resolved_url: "data:font/ttf;base64,AAEC".to_owned(),
                final_url: "data:font/ttf;base64,AAEC".to_owned(),
                content_type: None,
                bytes: vec![0, 1, 2],
            }]
        );
    }

    #[test]
    fn fetch_free_discovery_retains_inline_and_explains_linked_sheet() {
        let document = StaticDocument::parse(
            r#"<style>p { color: red }</style><link rel="stylesheet" href="site.css">"#,
        );
        let resources = ResolvedDocumentResources::discover(
            &document,
            Some("https://example.test/page/index.html"),
        );
        assert_eq!(resources.stylesheets.len(), 1);
        assert!(matches!(
            resources.diagnostics.as_slice(),
            [ResourceDiagnostic::LinkedStylesheetNoByteAuthority { .. }]
        ));
    }

    #[test]
    fn fetch_free_import_is_an_explicit_resource_diagnostic() {
        let document =
            StaticDocument::parse(r#"<style>@import url("later.css"); p { color: red }</style>"#);
        let resources = ResolvedDocumentResources::discover(&document, None);
        assert!(matches!(
            resources.diagnostics.as_slice(),
            [ResourceDiagnostic::ImportRuleUnavailable { .. }]
        ));
    }

    struct ResponseFetch;

    impl ResourceFetcher for ResponseFetch {
        fn fetch(&self, _url: &str) -> Option<Vec<u8>> {
            None
        }

        fn fetch_response(&self, url: &str) -> Option<ResourceResponse> {
            match url {
                "https://example.test/docs/outer.css" => Some(
                    ResourceResponse::new(
                        "https://cdn.example.test/styles/outer.css",
                        br#"@import url("inner.css") screen; .card { color: red; }"#.to_vec(),
                    )
                    .with_content_type("text/css; charset=utf-8"),
                ),
                "https://cdn.example.test/styles/inner.css" => Some(
                    ResourceResponse::new(
                        "https://cdn.example.test/styles/inner.css",
                        br#".card { background-image: url(images/inner.png); color: blue; }"#
                            .to_vec(),
                    )
                    .with_content_type("text/css"),
                ),
                "https://cdn.example.test/styles/images/inner.png" => {
                    Some(ResourceResponse::new(url, vec![1, 2, 3]))
                },
                _ => None,
            }
        }
    }

    #[test]
    fn imported_sheets_precede_their_parent_and_keep_final_identities() {
        let document = StaticDocument::parse(
            r#"<style>@import url("outer.css"); .card { color: green; }</style>"#,
        );
        let resources = ResolvedDocumentResources::resolve(
            &document,
            Some("https://example.test/docs/page.html"),
            &ResponseFetch,
        );
        assert!(resources.diagnostics.is_empty(), "{resources:#?}");
        assert_eq!(resources.stylesheets.len(), 3);
        let inner = &resources.stylesheets[0];
        let outer = &resources.stylesheets[1];
        let root = &resources.stylesheets[2];
        assert_eq!(root.imports.len(), 1);
        assert_eq!(root.imports[0].authored_url, "outer.css");
        assert_eq!(root.imports[0].child_sheet_id, Some(outer.sheet_id));
        assert_eq!(
            outer.import_parent,
            Some(StylesheetImportParent {
                sheet_id: root.sheet_id,
                import_index: 0,
            })
        );
        assert_eq!(outer.imports.len(), 1);
        assert_eq!(outer.imports[0].media.as_deref(), Some("screen"));
        assert_eq!(outer.imports[0].child_sheet_id, Some(inner.sheet_id));
        assert_eq!(
            inner.import_parent,
            Some(StylesheetImportParent {
                sheet_id: outer.sheet_id,
                import_index: 0,
            })
        );
        assert_eq!(resources.stylesheets[0].owner, StylesheetOwner::Imported);
        assert_eq!(
            resources.stylesheets[0].source_url.as_deref(),
            Some("https://cdn.example.test/styles/inner.css")
        );
        assert_eq!(
            resources.stylesheets[0].requested_url.as_deref(),
            Some("https://cdn.example.test/styles/inner.css")
        );
        assert!(resources.stylesheets[0].text.starts_with("@media screen"));
        assert_eq!(
            resources.stylesheets[1].source_url.as_deref(),
            Some("https://cdn.example.test/styles/outer.css")
        );
        assert_eq!(
            resources.stylesheets[1].requested_url.as_deref(),
            Some("https://example.test/docs/outer.css")
        );
        assert_eq!(resources.stylesheets[2].owner, StylesheetOwner::Inline);
        assert!(resources.stylesheets[2].text.contains("green"));
        assert!(resources.resources.iter().any(|resource| {
            resource.resolved_url == "https://cdn.example.test/styles/images/inner.png"
        }));
    }

    #[test]
    fn cyclic_import_is_reported_without_reapplying_a_sheet() {
        let document = StaticDocument::parse(r#"<style>@import "a.css";</style>"#);
        let mut fetch = |url: &str| match url {
            "https://example.test/a.css" => Some(ResourceResponse::new(
                url,
                br#"@import "b.css"; .a { color: red; }"#.to_vec(),
            )),
            "https://example.test/b.css" => Some(ResourceResponse::new(
                url,
                br#"@import "a.css"; .b { color: blue; }"#.to_vec(),
            )),
            _ => None,
        };
        let resources = resolve_responses_with_limits(
            &document,
            Some("https://example.test/page.html"),
            &mut fetch,
            ResourceLimits::default(),
        );
        assert_eq!(resources.stylesheets.len(), 3);
        assert!(matches!(
            resources.diagnostics.as_slice(),
            [ResourceDiagnostic::ImportRuleCycle { .. }]
        ));
    }

    #[test]
    fn non_css_link_response_does_not_enter_the_cascade() {
        let document = StaticDocument::parse(r#"<link rel="stylesheet" href="not-css">"#);
        let mut fetch = |url: &str| {
            Some(
                ResourceResponse::new(url, b"<html>not a stylesheet</html>".to_vec())
                    .with_content_type("text/html"),
            )
        };
        let resources = resolve_responses_with_limits(
            &document,
            Some("https://example.test/page.html"),
            &mut fetch,
            ResourceLimits::default(),
        );
        assert!(resources.stylesheets.is_empty());
        assert!(matches!(
            resources.diagnostics.as_slice(),
            [ResourceDiagnostic::LinkedStylesheetUnsupportedContentType { content_type, .. }]
                if content_type == "text/html"
        ));
    }

    #[test]
    fn late_top_level_import_is_reported_but_css_literals_are_not() {
        let document =
            StaticDocument::parse(r#"<style>.card { color: red; } @import "late.css";</style>"#);
        let resources =
            ResolvedDocumentResources::discover(&document, Some("https://example.test/page.html"));
        assert!(
            matches!(
                resources.diagnostics.as_slice(),
                [ResourceDiagnostic::ImportRuleOutOfOrder { source_url: Some(source_url) }]
                    if source_url == "https://example.test/page.html"
            ),
            "{resources:#?}"
        );

        let document = StaticDocument::parse(
            r#"<style>.card::before { content: "@import late.css"; }</style>"#,
        );
        let resources =
            ResolvedDocumentResources::discover(&document, Some("https://example.test/page.html"));
        assert!(resources.diagnostics.is_empty(), "{resources:#?}");
    }

    #[test]
    fn resolves_link_and_css_urls_against_their_own_sources() {
        assert_eq!(
            resolve_url(Some("https://example.test/docs/page.html"), "css/site.css"),
            "https://example.test/docs/css/site.css"
        );
        assert_eq!(
            resolve_url(
                Some("https://example.test/docs/css/site.css"),
                "images/logo.png"
            ),
            "https://example.test/docs/css/images/logo.png"
        );
    }

    #[test]
    fn normalizes_dot_segments_in_remote_resource_identities() {
        assert_eq!(
            resolve_url(
                Some("https://example.test/docs/styles/site.css"),
                "../fonts/text.woff2"
            ),
            "https://example.test/docs/fonts/text.woff2"
        );
        assert_eq!(
            resolve_url(
                Some("https://example.test/docs/page.html"),
                "https://cdn.example.test/a/../font.woff2"
            ),
            "https://cdn.example.test/font.woff2"
        );
    }

    #[test]
    fn staged_resolution_discovers_redirect_relative_css_imports_images_and_fonts() {
        let document = StaticDocument::parse(
            r#"<link rel="stylesheet" href="styles/root.css"><img src="html.png">"#,
        );
        let mut staged = StagedResourceResolution::new(
            document,
            Some("https://example.test/start/index.html"),
            ResourceLimits::default(),
        );
        let initial = staged.pending_requests();
        assert_eq!(
            initial,
            vec![
                PendingResourceRequest {
                    url: "https://example.test/start/styles/root.css".to_owned(),
                },
                PendingResourceRequest {
                    url: "https://example.test/start/html.png".to_owned(),
                },
            ]
        );
        staged.provide_response(
            initial[0].clone(),
            ResourceResponse::new(
                "https://cdn.example.test/final/css/root.css",
                br#"@import url("nested.css"); .card { background-image: url(hero.png), url(missing.png) }"#.to_vec(),
            )
            .with_content_type("text/css"),
        )
        .expect("root stylesheet was pending");
        staged
            .provide_response(
                initial[1].clone(),
                ResourceResponse::new(initial[1].url.clone(), vec![1, 2, 3]),
            )
            .expect("HTML image was pending");
        let next = staged.pending_requests();
        assert_eq!(
            next,
            vec![
                PendingResourceRequest {
                    url: "https://cdn.example.test/final/css/nested.css".to_owned(),
                },
                PendingResourceRequest {
                    url: "https://cdn.example.test/final/css/hero.png".to_owned(),
                },
                PendingResourceRequest {
                    url: "https://cdn.example.test/final/css/missing.png".to_owned(),
                },
            ]
        );
        staged
            .provide_response(
                next[0].clone(),
                ResourceResponse::new(
                    next[0].url.clone(),
                    br#"@font-face { font-family: proof; src: url(../fonts/proof.woff2) }"#
                        .to_vec(),
                )
                .with_content_type("text/css"),
            )
            .expect("import was pending");
        staged
            .provide_response(
                next[1].clone(),
                ResourceResponse::new("https://assets.example.test/hero.png", vec![4, 5, 6])
                    .with_content_type("image/png"),
            )
            .expect("CSS image was pending");
        staged
            .provide_unavailable(next[2].clone())
            .expect("missing image was pending");
        let final_request = staged.pending_requests();
        assert_eq!(
            final_request,
            vec![PendingResourceRequest {
                url: "https://cdn.example.test/final/fonts/proof.woff2".to_owned(),
            }]
        );
        staged
            .provide_response(
                final_request[0].clone(),
                ResourceResponse::new(
                    "https://assets.example.test/fonts/proof.woff2",
                    vec![7, 8, 9],
                )
                .with_content_type("font/woff2"),
            )
            .expect("font was pending");
        let resources = staged
            .finish()
            .expect("all discovered requests accounted for");
        assert_eq!(resources.stylesheets.len(), 2);
        assert!(resources.resources.iter().any(|resource| {
            resource.kind == ResourceKind::Image
                && resource.resolved_url == "https://example.test/start/html.png"
        }));
        assert!(resources.resources.iter().any(|resource| {
            resource.kind == ResourceKind::Image
                && resource.resolved_url == "https://cdn.example.test/final/css/hero.png"
                && resource.final_url == "https://assets.example.test/hero.png"
                && resource.content_type.as_deref() == Some("image/png")
        }));
        assert!(resources.resources.iter().any(|resource| {
            resource.kind == ResourceKind::Font
                && resource.resolved_url == "https://cdn.example.test/final/fonts/proof.woff2"
                && resource.final_url == "https://assets.example.test/fonts/proof.woff2"
                && resource.content_type.as_deref() == Some("font/woff2")
        }));
        assert!(matches!(
            resources.diagnostics.as_slice(),
            [ResourceDiagnostic::ResourceUnavailable { kind: ResourceKind::Image, resolved_url, .. }]
                if resolved_url == "https://cdn.example.test/final/css/missing.png"
        ));
    }

    #[test]
    fn staged_resolution_rejects_unrequested_provisioning() {
        let document = StaticDocument::parse("<p>no external resources</p>");
        let mut staged = StagedResourceResolution::new(
            document,
            Some("https://example.test/index.html"),
            ResourceLimits::default(),
        );
        let request = PendingResourceRequest {
            url: "https://example.test/injected.png".to_owned(),
        };
        assert_eq!(
            staged.provide_unavailable(request.clone()),
            Err(ResourceProvisionError { request })
        );
    }
}
