// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Child browsing contexts on the script-free Livery route: one retained
//! [`genet_livery::LiveryDocument`] per nested context, built through the same
//! resource route as its parent and composited into the parent's replaced box
//! each frame.
//!
//! The recursion lives here rather than in `genet-document-resources` because
//! nesting needs an HTML parser: that crate discovers one level of frames from
//! a DOM it is given and fetches their sources, and this one parses each source
//! into a document and asks the same question again.

use genet_document_resources::{
    FrameSource, ResolvedDocumentResources, ResolvedFrame, ResolvedStylesheet, ResourceKind,
    ResourceLimits, StylesheetOwner,
};
use genet_host_api::ResourceFetcher;

use crate::browsing_context::{
    ActiveDocument, BrowsingContextId, BrowsingContextTree, FrameAttributes,
};

/// How deep a chain of nested browsing contexts this lane builds.
///
/// HTML has no numeric limit; it forbids a *cycle* ("matching nested browsing
/// contexts") and leaves depth to the implementation. Both guards are here:
/// the ancestor-URL check below is the specified one, and this is the
/// belt-and-braces bound for a page that nests distinct URLs without end.
pub(crate) const MAX_FRAME_DEPTH: usize = 8;

/// One child browsing context and the retained document that renders it.
pub(crate) struct ChildFrame {
    /// `LayoutDom::opaque_id` of the `<iframe>` in the *parent* document. The
    /// composite joins scene to context by this value.
    pub(crate) owner_node: u64,
    pub(crate) context: BrowsingContextId,
    /// The child's retained document, or `None` when nothing loaded: an
    /// `about:blank`, a failed `src`, or a `loading="lazy"` frame.
    pub(crate) document: Option<genet_livery::LiveryDocument<genet_scripted_dom::ScriptedDom>>,
    /// This child's own children, keyed the same way.
    pub(crate) children: Vec<ChildFrame>,
    /// The content-box size the child was last laid out at, so a frame whose
    /// used size has not changed does not re-lay-out.
    pub(crate) last_size: (u32, u32),
    /// What the element should report: `load` after a document was created
    /// (including `about:blank` and `srcdoc`), `error` after a `src` that
    /// could not be fetched.
    pub(crate) outcome: FrameOutcome,
}

/// What one `<iframe>` element's load settled to, for a host (or the scripted
/// route) to turn into the element's `load` or `error` event.
///
/// The Livery route runs no script, so it reports rather than dispatches. The
/// report is still part of this lane's contract: an element that loaded an
/// `about:blank` fires `load`, and one whose `src` could not be fetched fires
/// `error`, and both have to be distinguishable before any consumer exists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameLoadReport {
    /// `LayoutDom::opaque_id` of the `<iframe>` in its containing document.
    pub owner_node: u64,
    pub context: BrowsingContextId,
    pub outcome: FrameOutcome,
    /// The child document's URL.
    pub url: String,
    /// The child's sandbox flags after inheritance.
    pub sandbox: crate::browsing_context::SandboxFlags,
}

/// The event an `<iframe>` element fires once its load settles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameOutcome {
    Load,
    Error,
    /// Nothing has been attempted: a `loading="lazy"` frame that has not come
    /// near the viewport. It fires neither event.
    Deferred,
}

/// Build the child contexts for one document.
///
/// `fetcher` is the parent's own resource route; `None` is an asynchronous
/// prepared spawn, whose child documents are built from the sources the
/// staged resolution already fetched but whose *sub*-resources stay inline
/// only. That degradation is named in the plan rather than hidden: a prepared
/// spawn's frames render, and their linked stylesheets do not.
/// The parts of a frame build that do not change as the recursion descends:
/// the resource route, the limits, and the host stylesheets every document in
/// this session gets. Bundled so the recursion carries three moving parts
/// (`tree`, `ancestor_urls`, `depth`) rather than nine arguments.
pub(crate) struct FrameBuild<'a> {
    pub(crate) fetcher: Option<&'a dyn ResourceFetcher>,
    pub(crate) limits: ResourceLimits,
    pub(crate) author_css: &'a [String],
}

pub(crate) fn build_child_frames(
    tree: &mut BrowsingContextTree,
    parent_context: BrowsingContextId,
    parent_url: Option<&str>,
    frames: &[ResolvedFrame],
    build: &FrameBuild<'_>,
    ancestor_urls: &mut Vec<String>,
    depth: usize,
) -> Vec<ChildFrame> {
    frames
        .iter()
        .filter_map(|frame| {
            build_child_frame(
                tree,
                parent_context,
                parent_url,
                frame,
                build,
                ancestor_urls,
                depth,
            )
        })
        .collect()
}

fn build_child_frame(
    tree: &mut BrowsingContextTree,
    parent_context: BrowsingContextId,
    parent_url: Option<&str>,
    frame: &ResolvedFrame,
    build: &FrameBuild<'_>,
    ancestor_urls: &mut Vec<String>,
    depth: usize,
) -> Option<ChildFrame> {
    let context = tree.create_child(
        parent_context,
        frame.owner_node,
        &FrameAttributes {
            name: frame.name.clone(),
            sandbox: frame.sandbox.clone(),
            allow: frame.allow.clone(),
            loading: frame.loading.clone(),
        },
    )?;

    let lazy = frame
        .loading
        .as_deref()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("lazy"));
    if lazy {
        return Some(ChildFrame {
            owner_node: frame.owner_node,
            context,
            document: None,
            children: Vec::new(),
            last_size: (0, 0),
            outcome: FrameOutcome::Deferred,
        });
    }
    if frame.failed {
        return Some(ChildFrame {
            owner_node: frame.owner_node,
            context,
            document: None,
            children: Vec::new(),
            last_size: (0, 0),
            outcome: FrameOutcome::Error,
        });
    }

    // HTML's "matching nested browsing contexts" rule, plus a depth bound. A
    // frame that would re-enter a URL already open in an ancestor loads
    // `about:blank` instead, which is what stops `<iframe src="">` on a page
    // that names itself from recursing forever.
    let recursive = frame.source_kind == FrameSource::Src
        && ancestor_urls
            .iter()
            .any(|ancestor| ancestor == &frame.resolved_url);
    if recursive || depth >= MAX_FRAME_DEPTH || frame.source.is_empty() {
        // Still a real context on the initial `about:blank`, and still a
        // `load` event: the element loaded a document, it was just an empty
        // one.
        return Some(ChildFrame {
            owner_node: frame.owner_node,
            context,
            document: None,
            children: Vec::new(),
            last_size: (0, 0),
            outcome: FrameOutcome::Load,
        });
    }

    // A `srcdoc` document's *base* URL is its container document's, while its
    // own URL is `about:srcdoc`. Resolving its relative links against
    // `about:srcdoc` would break every one of them.
    let base_url = match frame.source_kind {
        FrameSource::SrcDoc => parent_url.map(str::to_owned),
        _ => Some(frame.resolved_url.clone()),
    };

    let dom = genet_scripted_dom::ScriptedDom::from_serialized_document(&frame.source);
    let resources = match build.fetcher {
        Some(fetcher) => ResolvedDocumentResources::resolve_with_limits(
            &dom,
            base_url.as_deref(),
            fetcher,
            build.limits,
        ),
        None => ResolvedDocumentResources::discover(&dom, base_url.as_deref()),
    };

    let origin = tree.mint_origin(&frame.resolved_url);
    if let Some(child) = tree.get_mut(context) {
        // A sandboxed context keeps the opaque origin `create_child` gave it;
        // only an unsandboxed one takes the loaded document's.
        let sandboxed = child
            .sandbox()
            .contains(crate::browsing_context::SandboxFlags::ORIGIN);
        let origin = if sandboxed {
            child.document().origin.clone()
        } else {
            origin
        };
        child.navigate(ActiveDocument {
            url: frame.resolved_url.clone(),
            origin,
            initial_about_blank: false,
        });
    }

    let document = build_document(&resources, dom, build.author_css);
    ancestor_urls.push(frame.resolved_url.clone());
    let children = build_child_frames(
        tree,
        context,
        base_url.as_deref(),
        &resources.frames,
        build,
        ancestor_urls,
        depth + 1,
    );
    ancestor_urls.pop();
    Some(ChildFrame {
        owner_node: frame.owner_node,
        context,
        document: Some(document),
        children,
        last_size: (0, 0),
        outcome: FrameOutcome::Load,
    })
}

/// Assemble one retained Livery document from a resolved resource ledger.
///
/// The same assembly the top-level session does, minus the session wrapper: a
/// child browsing context is a document, not a `DocumentSession`, because the
/// host drives exactly one session and the tree hangs beneath it.
fn build_document(
    resources: &ResolvedDocumentResources,
    dom: genet_scripted_dom::ScriptedDom,
    author_css: &[String],
) -> genet_livery::LiveryDocument<genet_scripted_dom::ScriptedDom> {
    let mut sheets = author_css
        .iter()
        .enumerate()
        .map(|(document_order, text)| ResolvedStylesheet {
            sheet_id: u64::MAX.saturating_sub(document_order as u64),
            owner: StylesheetOwner::Inline,
            owner_node: None,
            source_url: None,
            requested_url: None,
            content_type: None,
            media: None,
            imports: Vec::new(),
            import_parent: None,
            text: text.clone(),
            document_order: document_order as u64,
        })
        .collect::<Vec<_>>();
    sheets.extend(resources.stylesheets.iter().cloned());
    // The child's device is set to the default here and replaced by its used
    // content box on the first composite, which is the only moment the size is
    // known: the parent has to lay out before the frame's used size exists.
    let mut document = genet_livery::LiveryDocument::new(
        dom,
        genet_livery::StyleSet::cambium_resources(&sheets),
        genet_livery::Device::screen(300.0, 150.0),
    );
    for resource in &resources.resources {
        match resource.kind {
            ResourceKind::Image => {
                document.set_image_resource(resource.authored_url.clone(), resource.bytes.clone());
                if resource.resolved_url != resource.authored_url {
                    document
                        .set_image_resource(resource.resolved_url.clone(), resource.bytes.clone());
                }
            },
            ResourceKind::Font => {
                document.set_font_resource(resource.resolved_url.clone(), resource.bytes.clone());
            },
        }
    }
    document
}

impl ChildFrame {
    /// Find this frame or one of its descendants by container node.
    pub(crate) fn find(frames: &[ChildFrame], owner_node: u64) -> Option<&ChildFrame> {
        frames.iter().find_map(|frame| {
            (frame.owner_node == owner_node)
                .then_some(frame)
                .or_else(|| ChildFrame::find(&frame.children, owner_node))
        })
    }

    /// Every frame in this subtree, parent before child, as load reports.
    pub(crate) fn reports(
        frames: &[ChildFrame],
        tree: &BrowsingContextTree,
        out: &mut Vec<FrameLoadReport>,
    ) {
        for frame in frames {
            if let Some(context) = tree.get(frame.context) {
                out.push(FrameLoadReport {
                    owner_node: frame.owner_node,
                    context: frame.context,
                    outcome: frame.outcome,
                    url: context.document().url.clone(),
                    sandbox: context.sandbox(),
                });
            }
            ChildFrame::reports(&frame.children, tree, out);
        }
    }
}
