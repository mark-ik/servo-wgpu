// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The browsing-context tree: parent, children, top, the document each
//! context currently holds, and one session history per context.
//!
//! Before this module `genet-documents` held one retained session per
//! document with no parent link, so a nested document had nowhere to be.
//! HTML's navigable tree is the missing shape: every `<iframe>` element in a
//! document has a *child browsing context*, that context has its own active
//! document and its own session history, and `parent` / `top` / `frames` are
//! reads of the tree rather than facts about a window object.
//!
//! Three deliberate boundaries:
//!
//! - **The tree is data, not a session.** It stores each context's identity,
//!   origin, sandbox and history; it does not own the engine session that
//!   renders the document. A host joins the two by [`BrowsingContextId`], so
//!   the Livery route (no script) and the scripted route can share the tree.
//! - **Product history stays in the host.** [`SessionHistory`] is HTML's
//!   per-context session history — what `history.pushState`, `back()` and
//!   `forward()` traverse. Tab and workspace history is Mere's.
//! - **Origin is a value, not a policy.** [`Origin`] computes and compares;
//!   what a caller is *allowed* to do with two origins lives in
//!   the host frame policy.

use std::collections::HashMap;

/// A context's identity within one [`BrowsingContextTree`].
///
/// Dense and reused only after [`BrowsingContextTree::discard`], which is why
/// the tree also carries a generation: a stale id from a discarded context
/// does not silently address its replacement.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BrowsingContextId {
    index: u32,
    generation: u32,
}

impl BrowsingContextId {
    /// The tree-local slot, for a host that keys its own session table by
    /// context. Not stable across [`BrowsingContextTree::discard`].
    pub fn slot(self) -> u32 {
        self.index
    }
}

/// A tuple origin, or an opaque one.
///
/// HTML's origin is either the (scheme, host, port) tuple of a URL or an
/// opaque origin that is same-origin with nothing but itself — including
/// another opaque origin minted at the same instant, which is why the opaque
/// variant carries a serial rather than being a unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Origin {
    Tuple {
        scheme: String,
        host: String,
        port: Option<u16>,
    },
    Opaque(u64),
}

impl Origin {
    /// The origin of `url`, or an opaque origin when the URL has no tuple
    /// origin (`data:`, `about:`, `blob:` without a parsable inner URL, or an
    /// unparsable spelling). `opaque_serial` distinguishes this opaque origin
    /// from every other; callers mint it from the tree's counter.
    pub fn of_url(url: &str, opaque_serial: u64) -> Self {
        let Some((scheme, rest)) = url.split_once(':') else {
            return Origin::Opaque(opaque_serial);
        };
        let scheme = scheme.to_ascii_lowercase();
        // `file:` is treated as opaque, which is what browsers converged on
        // and what the sandboxing tests assume; `about:` and `data:` have no
        // tuple origin at all.
        if !matches!(scheme.as_str(), "http" | "https" | "ws" | "wss" | "ftp") {
            return Origin::Opaque(opaque_serial);
        }
        let Some(rest) = rest.strip_prefix("//") else {
            return Origin::Opaque(opaque_serial);
        };
        let authority = rest
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .rsplit('@')
            .next()
            .unwrap_or_default();
        if authority.is_empty() {
            return Origin::Opaque(opaque_serial);
        }
        // An IPv6 literal keeps its brackets and hides the port colon inside
        // them, so split after the closing bracket rather than on the first
        // colon.
        let (host, port) = match authority.strip_prefix('[') {
            Some(inner) => match inner.split_once(']') {
                Some((host, rest)) => (
                    format!("[{host}]"),
                    rest.strip_prefix(':').and_then(|p| p.parse().ok()),
                ),
                None => (authority.to_owned(), None),
            },
            None => match authority.split_once(':') {
                Some((host, port)) => (host.to_owned(), port.parse().ok()),
                None => (authority.to_owned(), None),
            },
        };
        let port = port.filter(|port| Some(*port) != default_port(&scheme));
        Origin::Tuple {
            scheme,
            host: host.to_ascii_lowercase(),
            port,
        }
    }

    /// HTML's same-origin check. Two opaque origins are same-origin only when
    /// they are *the* same opaque origin.
    pub fn is_same_origin(&self, other: &Self) -> bool {
        self == other
    }

    /// The ASCII serialization used by `postMessage`'s `origin` and by
    /// `targetOrigin` matching. An opaque origin serializes to `"null"`.
    pub fn serialize(&self) -> String {
        match self {
            Origin::Opaque(_) => "null".to_owned(),
            Origin::Tuple { scheme, host, port } => match port {
                Some(port) => format!("{scheme}://{host}:{port}"),
                None => format!("{scheme}://{host}"),
            },
        }
    }

    pub fn is_opaque(&self) -> bool {
        matches!(self, Origin::Opaque(_))
    }
}

fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        "ftp" => Some(21),
        _ => None,
    }
}

/// HTML's sandboxing flag set, as far as one process can hold it.
///
/// Parsed from the `sandbox` content attribute: an absent attribute sets no
/// flags at all, and a present one sets every flag that its token list does
/// **not** unset. That inversion is the attribute's whole shape — `sandbox`
/// with no tokens is the most restrictive value, not the least.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SandboxFlags(u32);

impl SandboxFlags {
    pub const NAVIGATION: Self = Self(1 << 0);
    pub const PLUGINS: Self = Self(1 << 1);
    pub const ORIGIN: Self = Self(1 << 2);
    pub const FORMS: Self = Self(1 << 3);
    pub const POINTER_LOCK: Self = Self(1 << 4);
    pub const SCRIPTS: Self = Self(1 << 5);
    pub const AUTOMATIC_FEATURES: Self = Self(1 << 6);
    pub const TOP_NAVIGATION: Self = Self(1 << 7);
    pub const MODALS: Self = Self(1 << 8);
    pub const ORIENTATION_LOCK: Self = Self(1 << 9);
    pub const PRESENTATION: Self = Self(1 << 10);
    pub const DOWNLOADS: Self = Self(1 << 11);
    pub const TOP_NAVIGATION_BY_USER_ACTIVATION: Self = Self(1 << 12);
    pub const TOP_NAVIGATION_TO_CUSTOM_PROTOCOLS: Self = Self(1 << 13);

    /// No flags: an unsandboxed context.
    pub const NONE: Self = Self(0);

    /// Every flag a bare `sandbox=""` sets.
    pub const FULL: Self = Self(
        Self::NAVIGATION.0
            | Self::PLUGINS.0
            | Self::ORIGIN.0
            | Self::FORMS.0
            | Self::POINTER_LOCK.0
            | Self::SCRIPTS.0
            | Self::AUTOMATIC_FEATURES.0
            | Self::TOP_NAVIGATION.0
            | Self::MODALS.0
            | Self::ORIENTATION_LOCK.0
            | Self::PRESENTATION.0
            | Self::DOWNLOADS.0
            | Self::TOP_NAVIGATION_BY_USER_ACTIVATION.0
            | Self::TOP_NAVIGATION_TO_CUSTOM_PROTOCOLS.0,
    );

    pub fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0 && flag.0 != 0
    }

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Parse a `sandbox` attribute value. `None` is an absent attribute
    /// (no flags); `Some("")` is a present, empty one (every flag).
    pub fn parse(attribute: Option<&str>) -> Self {
        let Some(attribute) = attribute else {
            return Self::NONE;
        };
        let mut flags = Self::FULL;
        for token in attribute.split_ascii_whitespace() {
            let unset = match token.to_ascii_lowercase().as_str() {
                "allow-navigation" => Self::NAVIGATION,
                "allow-plugins" => Self::PLUGINS,
                "allow-same-origin" => Self::ORIGIN,
                "allow-forms" => Self::FORMS,
                "allow-pointer-lock" => Self::POINTER_LOCK,
                "allow-scripts" => Self::SCRIPTS,
                "allow-popups" => Self::AUTOMATIC_FEATURES,
                "allow-top-navigation" => Self::TOP_NAVIGATION
                    .union(Self::TOP_NAVIGATION_BY_USER_ACTIVATION)
                    .union(Self::TOP_NAVIGATION_TO_CUSTOM_PROTOCOLS),
                "allow-top-navigation-by-user-activation" => {
                    Self::TOP_NAVIGATION_BY_USER_ACTIVATION
                },
                "allow-top-navigation-to-custom-protocols" => {
                    Self::TOP_NAVIGATION_TO_CUSTOM_PROTOCOLS
                },
                "allow-modals" => Self::MODALS,
                "allow-orientation-lock" => Self::ORIENTATION_LOCK,
                "allow-presentation" => Self::PRESENTATION,
                "allow-downloads" => Self::DOWNLOADS,
                _ => Self::NONE,
            };
            flags = Self(flags.0 & !unset.0);
        }
        flags
    }
}

/// The `loading` content attribute on an embedded-content element.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FrameLoading {
    #[default]
    Eager,
    Lazy,
}

impl FrameLoading {
    /// `loading` is an enumerated attribute whose invalid *and* missing value
    /// default is `eager`, so anything that is not an ASCII case-insensitive
    /// `lazy` is eager.
    pub fn parse(attribute: Option<&str>) -> Self {
        match attribute {
            Some(value) if value.trim().eq_ignore_ascii_case("lazy") => FrameLoading::Lazy,
            _ => FrameLoading::Eager,
        }
    }
}

/// One entry in a context's session history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryEntry {
    pub url: String,
    /// The serialized `pushState` / `replaceState` state, in the same JSON
    /// encoding of the structured-clone record the Worker lane uses as its
    /// cross-agent wire. `None` is the null state a plain navigation leaves.
    pub state: Option<String>,
    pub title: Option<String>,
    /// Which of the context's documents this entry belongs to. HTML decides
    /// whether a traversal keeps the document by comparing *document*
    /// identity, not URLs: `pushState` can move the URL anywhere within the
    /// same document, and two entries can share a URL across a reload. The
    /// context stamps this, and only a real navigation advances it.
    pub document: u64,
}

impl HistoryEntry {
    pub fn for_url(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            state: None,
            title: None,
            document: 0,
        }
    }

    /// The same entry, stamped as belonging to document `serial`.
    #[must_use]
    pub fn in_document(mut self, serial: u64) -> Self {
        self.document = serial;
        self
    }
}

/// HTML's per-context session history.
///
/// One list plus a current index. `push` truncates everything after the
/// current entry — the forward list a new navigation discards — which is the
/// only reason this is not a plain `Vec` with an append.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionHistory {
    entries: Vec<HistoryEntry>,
    current: usize,
}

impl SessionHistory {
    pub fn new(entry: HistoryEntry) -> Self {
        Self {
            entries: vec![entry],
            current: 0,
        }
    }

    /// `history.length`.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// The index `history.go(0)` is already at.
    pub fn index(&self) -> usize {
        self.current
    }

    pub fn current(&self) -> &HistoryEntry {
        &self.entries[self.current]
    }

    pub fn current_mut(&mut self) -> &mut HistoryEntry {
        &mut self.entries[self.current]
    }

    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// `history.pushState` and a same-context navigation: append after the
    /// current entry, discarding the forward list.
    pub fn push(&mut self, entry: HistoryEntry) {
        self.entries.truncate(self.current + 1);
        self.entries.push(entry);
        self.current = self.entries.len() - 1;
    }

    /// `history.replaceState` and a `location.replace()` navigation.
    pub fn replace(&mut self, entry: HistoryEntry) {
        self.entries[self.current] = entry;
    }

    /// `history.go(delta)`. Out-of-range traversal is a no-op that returns
    /// `None`, which is what HTML specifies rather than a clamp.
    pub fn go(&mut self, delta: i64) -> Option<&HistoryEntry> {
        let target = i64::try_from(self.current).ok()?.checked_add(delta)?;
        let target = usize::try_from(target).ok()?;
        if target >= self.entries.len() {
            return None;
        }
        self.current = target;
        Some(&self.entries[target])
    }

    pub fn can_go(&self, delta: i64) -> bool {
        i64::try_from(self.current)
            .ok()
            .and_then(|current| current.checked_add(delta))
            .and_then(|target| usize::try_from(target).ok())
            .is_some_and(|target| target < self.entries.len())
    }
}

/// What a context's active document is, as far as the tree needs to know.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveDocument {
    /// The document's URL, as `document.URL` reports it.
    pub url: String,
    pub origin: Origin,
    /// True while this is the *initial* `about:blank` a freshly created child
    /// context holds before its first real navigation. HTML gives that
    /// document its container's origin and lets the first real load *replace*
    /// its history entry rather than push one, and both rules key off this
    /// bit rather than off the URL — a document can navigate to `about:blank`
    /// deliberately, and that one is not initial.
    pub initial_about_blank: bool,
}

impl ActiveDocument {
    pub fn initial_about_blank(origin: Origin) -> Self {
        Self {
            url: "about:blank".to_owned(),
            origin,
            initial_about_blank: true,
        }
    }
}

/// One browsing context in the tree.
#[derive(Clone, Debug)]
pub struct BrowsingContext {
    id: BrowsingContextId,
    parent: Option<BrowsingContextId>,
    children: Vec<BrowsingContextId>,
    /// The `opaque_id` of the embedder element in the *parent* document, or
    /// `None` for the top-level context.
    container: Option<u64>,
    /// The container's `name` content attribute — what `window.frames['x']`
    /// and a `target` resolve against.
    name: String,
    document: ActiveDocument,
    history: SessionHistory,
    sandbox: SandboxFlags,
    /// The `allow` attribute's raw value, stored rather than interpreted: the
    /// Permissions Policy container-policy algorithm is a named residual.
    allow: String,
    loading: FrameLoading,
    /// Which document this context is showing, counted rather than named.
    /// Every real navigation advances it; a fragment navigation, a
    /// `pushState` and a `replaceState` do not, which is exactly the
    /// distinction a traversal needs.
    document_serial: u64,
}

impl BrowsingContext {
    pub fn id(&self) -> BrowsingContextId {
        self.id
    }

    pub fn parent(&self) -> Option<BrowsingContextId> {
        self.parent
    }

    pub fn children(&self) -> &[BrowsingContextId] {
        &self.children
    }

    pub fn container(&self) -> Option<u64> {
        self.container
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    pub fn document(&self) -> &ActiveDocument {
        &self.document
    }

    pub fn history(&self) -> &SessionHistory {
        &self.history
    }

    pub fn history_mut(&mut self) -> &mut SessionHistory {
        &mut self.history
    }

    pub fn sandbox(&self) -> SandboxFlags {
        self.sandbox
    }

    pub fn allow(&self) -> &str {
        &self.allow
    }

    pub fn loading(&self) -> FrameLoading {
        self.loading
    }

    /// Replace the active document, joining the session history the way HTML
    /// does: the first navigation *out of* an initial `about:blank` replaces
    /// its entry, and every later one pushes.
    pub fn navigate(&mut self, document: ActiveDocument) {
        self.navigate_with(document, false);
    }

    /// [`navigate`](Self::navigate) with an explicit replacement flag, which is
    /// what `location.replace()` and a `replace`-flavoured traversal need. An
    /// initial `about:blank` replaces regardless: HTML's history-handling
    /// behaviour for the first navigation out of one is "replace" whatever the
    /// caller asked for.
    /// Which document this context is showing. Stamp a `pushState` entry with
    /// it, and a traversal to an entry carrying it keeps the document.
    pub fn document_serial(&self) -> u64 {
        self.document_serial
    }

    pub fn navigate_with(&mut self, document: ActiveDocument, replace: bool) {
        self.document_serial += 1;
        let entry = HistoryEntry::for_url(document.url.clone()).in_document(self.document_serial);
        if replace || self.document.initial_about_blank {
            self.history.replace(entry);
        } else {
            self.history.push(entry);
        }
        self.document = document;
    }

    /// Adopt `url` without touching the session history: a fragment navigation
    /// and a `replaceState` both move the document's URL under a history entry
    /// the caller has already positioned.
    pub fn set_document_url(&mut self, url: impl Into<String>) {
        let url = url.into();
        self.document.initial_about_blank = false;
        self.document.url = url;
    }
}

/// The tree of browsing contexts rooted at one top-level context.
#[derive(Clone, Debug)]
pub struct BrowsingContextTree {
    slots: Vec<Slot>,
    free: Vec<u32>,
    top: BrowsingContextId,
    /// Serial for the next opaque origin this tree mints.
    next_opaque: u64,
}

#[derive(Clone, Debug)]
struct Slot {
    generation: u32,
    context: Option<BrowsingContext>,
}

impl BrowsingContextTree {
    /// A tree with one top-level context holding `url`.
    pub fn new(url: impl Into<String>) -> Self {
        let url = url.into();
        let mut tree = Self {
            slots: Vec::new(),
            free: Vec::new(),
            top: BrowsingContextId {
                index: 0,
                generation: 0,
            },
            next_opaque: 0,
        };
        let origin = tree.mint_origin(&url);
        let top = tree.insert(BrowsingContext {
            id: BrowsingContextId {
                index: 0,
                generation: 0,
            },
            parent: None,
            children: Vec::new(),
            container: None,
            name: String::new(),
            document: ActiveDocument {
                url: url.clone(),
                origin,
                initial_about_blank: false,
            },
            history: SessionHistory::new(HistoryEntry::for_url(url)),
            sandbox: SandboxFlags::NONE,
            allow: String::new(),
            loading: FrameLoading::Eager,
            document_serial: 0,
        });
        tree.top = top;
        tree
    }

    /// The origin of `url` in this tree, minting a fresh opaque origin when
    /// the URL has no tuple origin.
    pub fn mint_origin(&mut self, url: &str) -> Origin {
        let serial = self.next_opaque;
        let origin = Origin::of_url(url, serial);
        if origin.is_opaque() {
            self.next_opaque += 1;
        }
        origin
    }

    /// A fresh opaque origin belonging to nothing else — what a
    /// `sandbox` without `allow-same-origin` gives a child.
    pub fn mint_opaque_origin(&mut self) -> Origin {
        let origin = Origin::Opaque(self.next_opaque);
        self.next_opaque += 1;
        origin
    }

    pub fn top(&self) -> BrowsingContextId {
        self.top
    }

    pub fn get(&self, id: BrowsingContextId) -> Option<&BrowsingContext> {
        let slot = self.slots.get(usize::try_from(id.index).ok()?)?;
        (slot.generation == id.generation)
            .then_some(slot.context.as_ref())
            .flatten()
    }

    pub fn get_mut(&mut self, id: BrowsingContextId) -> Option<&mut BrowsingContext> {
        let slot = self.slots.get_mut(usize::try_from(id.index).ok()?)?;
        (slot.generation == id.generation)
            .then_some(slot.context.as_mut())
            .flatten()
    }

    /// Every live context, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = &BrowsingContext> {
        self.slots.iter().filter_map(|slot| slot.context.as_ref())
    }

    pub fn len(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.context.is_some())
            .count()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Create a child context for the `<iframe>` whose parent-document
    /// `opaque_id` is `container`.
    ///
    /// The child starts on the *initial* `about:blank`: HTML creates the
    /// nested context and its initial document synchronously when the element
    /// is inserted, before anything is fetched, and gives that document its
    /// container document's origin unless sandboxing takes it away. Nothing
    /// here fetches; [`BrowsingContext::navigate`] joins the real load later.
    pub fn create_child(
        &mut self,
        parent: BrowsingContextId,
        container: u64,
        attributes: &FrameAttributes,
    ) -> Option<BrowsingContextId> {
        let parent_context = self.get(parent)?;
        let inherited_sandbox = parent_context.sandbox;
        let parent_origin = parent_context.document.origin.clone();
        // A child inherits its parent's flags and adds its own container's;
        // an `allow-same-origin` token cannot restore what an ancestor took.
        let sandbox = inherited_sandbox.union(SandboxFlags::parse(attributes.sandbox.as_deref()));
        let origin = if sandbox.contains(SandboxFlags::ORIGIN) {
            self.mint_opaque_origin()
        } else {
            parent_origin
        };
        let id = self.insert(BrowsingContext {
            id: BrowsingContextId {
                index: 0,
                generation: 0,
            },
            parent: Some(parent),
            children: Vec::new(),
            container: Some(container),
            name: attributes.name.clone().unwrap_or_default(),
            document: ActiveDocument::initial_about_blank(origin),
            history: SessionHistory::new(HistoryEntry::for_url("about:blank")),
            sandbox,
            allow: attributes.allow.clone().unwrap_or_default(),
            loading: FrameLoading::parse(attributes.loading.as_deref()),
            document_serial: 0,
        });
        self.get_mut(parent)?.children.push(id);
        Some(id)
    }

    /// Discard a context and every descendant, returning them in
    /// child-before-parent order so a caller can retire each one's runtime
    /// and scene resources before its parent's.
    pub fn discard(&mut self, id: BrowsingContextId) -> Vec<BrowsingContextId> {
        let mut order = Vec::new();
        self.collect_descendants(id, &mut order);
        if let Some(parent) = self.get(id).and_then(|context| context.parent)
            && let Some(parent) = self.get_mut(parent)
        {
            parent.children.retain(|child| *child != id);
        }
        for discarded in &order {
            let Ok(index) = usize::try_from(discarded.index) else {
                continue;
            };
            let slot = &mut self.slots[index];
            slot.context = None;
            slot.generation = slot.generation.wrapping_add(1);
            self.free.push(discarded.index);
        }
        order
    }

    fn collect_descendants(&self, id: BrowsingContextId, out: &mut Vec<BrowsingContextId>) {
        let Some(context) = self.get(id) else {
            return;
        };
        for child in context.children.clone() {
            self.collect_descendants(child, out);
        }
        out.push(id);
    }

    /// The child context whose container element is `container`, if any.
    pub fn context_for_container(&self, container: u64) -> Option<BrowsingContextId> {
        self.iter()
            .find(|context| context.container == Some(container))
            .map(BrowsingContext::id)
    }

    /// `window.parent`: the parent context, or the context itself when it is
    /// top-level — HTML's `parent` never yields null for a live context.
    pub fn parent_of(&self, id: BrowsingContextId) -> Option<BrowsingContextId> {
        Some(self.get(id)?.parent.unwrap_or(id))
    }

    /// `window.top`: the root of this context's tree.
    pub fn top_of(&self, id: BrowsingContextId) -> Option<BrowsingContextId> {
        let mut current = self.get(id)?;
        while let Some(parent) = current.parent {
            current = self.get(parent)?;
        }
        Some(current.id)
    }

    /// `window.frames[i]` / `window.length`: the child contexts in document
    /// order.
    pub fn frames_of(&self, id: BrowsingContextId) -> &[BrowsingContextId] {
        self.get(id).map_or(&[], BrowsingContext::children)
    }

    /// `window.frames['name']`: the first child whose container `name`
    /// matches, searched in document order.
    pub fn named_frame(&self, id: BrowsingContextId, name: &str) -> Option<BrowsingContextId> {
        self.frames_of(id)
            .iter()
            .copied()
            .find(|child| self.get(*child).is_some_and(|child| child.name == name))
    }

    /// Whether `a` may synchronously reach `b`'s document.
    pub fn is_same_origin(&self, a: BrowsingContextId, b: BrowsingContextId) -> bool {
        match (self.get(a), self.get(b)) {
            (Some(a), Some(b)) => a.document.origin.is_same_origin(&b.document.origin),
            _ => false,
        }
    }

    /// A map from container `opaque_id` to context, for a renderer that walks
    /// the parent DOM and needs each iframe's child in one pass.
    pub fn containers(&self) -> HashMap<u64, BrowsingContextId> {
        self.iter()
            .filter_map(|context| Some((context.container?, context.id)))
            .collect()
    }

    fn insert(&mut self, mut context: BrowsingContext) -> BrowsingContextId {
        let index = match self.free.pop() {
            Some(index) => index,
            None => {
                self.slots.push(Slot {
                    generation: 0,
                    context: None,
                });
                u32::try_from(self.slots.len() - 1).expect("browsing-context slot fits in u32")
            },
        };
        let slot = &mut self.slots[index as usize];
        let id = BrowsingContextId {
            index,
            generation: slot.generation,
        };
        context.id = id;
        slot.context = Some(context);
        id
    }
}

/// The container attributes a child context is created from.
///
/// Collected by the loading pass from the `<iframe>` element, so the tree
/// never has to know how to read a DOM.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FrameAttributes {
    pub name: Option<String>,
    pub sandbox: Option<String>,
    pub allow: Option<String>,
    pub loading: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tuple_origin_drops_its_default_port_and_folds_its_host() {
        assert_eq!(
            Origin::of_url("HTTPS://Example.COM:443/a?b#c", 0).serialize(),
            "https://example.com"
        );
        assert_eq!(
            Origin::of_url("http://example.com:8080/", 0).serialize(),
            "http://example.com:8080"
        );
        assert_eq!(
            Origin::of_url("http://user:pw@example.com/", 0).serialize(),
            "http://example.com"
        );
        assert_eq!(
            Origin::of_url("https://[::1]:8443/", 0).serialize(),
            "https://[::1]:8443"
        );
    }

    #[test]
    fn about_data_and_file_urls_have_opaque_origins_that_match_nothing() {
        for url in ["about:blank", "data:text/html,x", "file:///c/page.html"] {
            let first = Origin::of_url(url, 1);
            let second = Origin::of_url(url, 2);
            assert!(first.is_opaque(), "{url} should be opaque");
            assert_eq!(first.serialize(), "null");
            assert!(
                !first.is_same_origin(&second),
                "{url}: two opaque origins must not match"
            );
            assert!(first.is_same_origin(&first.clone()));
        }
    }

    #[test]
    fn an_absent_sandbox_sets_nothing_and_a_bare_one_sets_everything() {
        assert!(SandboxFlags::parse(None).is_empty());
        let bare = SandboxFlags::parse(Some(""));
        assert!(bare.contains(SandboxFlags::SCRIPTS));
        assert!(bare.contains(SandboxFlags::ORIGIN));
        assert!(bare.contains(SandboxFlags::FORMS));
    }

    #[test]
    fn a_sandbox_token_unsets_exactly_its_flag() {
        let flags = SandboxFlags::parse(Some("allow-scripts  ALLOW-Forms"));
        assert!(!flags.contains(SandboxFlags::SCRIPTS));
        assert!(!flags.contains(SandboxFlags::FORMS));
        assert!(flags.contains(SandboxFlags::ORIGIN));
        assert!(flags.contains(SandboxFlags::TOP_NAVIGATION));
        // An unknown token is ignored rather than clearing the set.
        let unknown = SandboxFlags::parse(Some("allow-everything"));
        assert_eq!(unknown, SandboxFlags::FULL);
    }

    #[test]
    fn allow_top_navigation_also_unsets_its_two_narrower_flags() {
        let flags = SandboxFlags::parse(Some("allow-top-navigation"));
        assert!(!flags.contains(SandboxFlags::TOP_NAVIGATION));
        assert!(!flags.contains(SandboxFlags::TOP_NAVIGATION_BY_USER_ACTIVATION));
        assert!(!flags.contains(SandboxFlags::TOP_NAVIGATION_TO_CUSTOM_PROTOCOLS));
    }

    #[test]
    fn loading_is_eager_unless_the_value_is_exactly_lazy() {
        assert_eq!(FrameLoading::parse(None), FrameLoading::Eager);
        assert_eq!(FrameLoading::parse(Some(" LAZY ")), FrameLoading::Lazy);
        assert_eq!(FrameLoading::parse(Some("nonsense")), FrameLoading::Eager);
    }

    fn frame(sandbox: Option<&str>) -> FrameAttributes {
        FrameAttributes {
            sandbox: sandbox.map(str::to_owned),
            ..Default::default()
        }
    }

    #[test]
    fn a_child_starts_on_the_initial_about_blank_with_its_parents_origin() {
        let mut tree = BrowsingContextTree::new("https://example.com/parent.html");
        let child = tree
            .create_child(tree.top(), 7, &frame(None))
            .expect("child context");
        let child = tree.get(child).expect("live child");
        assert_eq!(child.document().url, "about:blank");
        assert!(child.document().initial_about_blank);
        assert_eq!(child.document().origin.serialize(), "https://example.com");
        assert_eq!(child.history().len(), 1);
    }

    #[test]
    fn a_sandbox_without_allow_same_origin_gives_the_child_an_opaque_origin() {
        let mut tree = BrowsingContextTree::new("https://example.com/parent.html");
        let sandboxed = tree
            .create_child(tree.top(), 1, &frame(Some("allow-scripts")))
            .expect("child");
        let same = tree
            .create_child(tree.top(), 2, &frame(Some("allow-same-origin")))
            .expect("child");
        assert!(tree.get(sandboxed).unwrap().document().origin.is_opaque());
        assert!(!tree.is_same_origin(tree.top(), sandboxed));
        assert!(tree.is_same_origin(tree.top(), same));
    }

    #[test]
    fn sandbox_flags_are_inherited_and_a_child_cannot_take_them_back() {
        let mut tree = BrowsingContextTree::new("https://example.com/");
        let outer = tree
            .create_child(tree.top(), 1, &frame(Some("allow-same-origin")))
            .expect("outer");
        let inner = tree
            .create_child(outer, 2, &frame(Some("allow-scripts allow-same-origin")))
            .expect("inner");
        // The outer frame did not allow scripts, so the inner one cannot.
        assert!(
            tree.get(inner)
                .unwrap()
                .sandbox()
                .contains(SandboxFlags::SCRIPTS)
        );
        assert!(
            !tree
                .get(outer)
                .unwrap()
                .sandbox()
                .contains(SandboxFlags::ORIGIN)
        );
    }

    #[test]
    fn the_first_load_replaces_the_initial_about_blank_entry_and_the_next_pushes() {
        let mut tree = BrowsingContextTree::new("https://example.com/");
        let child = tree
            .create_child(tree.top(), 1, &frame(None))
            .expect("child");
        let origin = tree.mint_origin("https://example.com/a.html");
        let context = tree.get_mut(child).expect("child");
        context.navigate(ActiveDocument {
            url: "https://example.com/a.html".into(),
            origin: origin.clone(),
            initial_about_blank: false,
        });
        assert_eq!(context.history().len(), 1);
        assert_eq!(
            context.history().current().url,
            "https://example.com/a.html"
        );
        context.navigate(ActiveDocument {
            url: "https://example.com/b.html".into(),
            origin,
            initial_about_blank: false,
        });
        assert_eq!(context.history().len(), 2);
        assert_eq!(context.history().index(), 1);
    }

    #[test]
    fn push_state_discards_the_forward_list_and_go_refuses_to_leave_the_range() {
        let mut history = SessionHistory::new(HistoryEntry::for_url("https://example.com/a"));
        history.push(HistoryEntry::for_url("https://example.com/b"));
        history.push(HistoryEntry::for_url("https://example.com/c"));
        assert_eq!(history.len(), 3);
        assert_eq!(
            history.go(-2).map(|entry| entry.url.as_str()),
            Some("https://example.com/a")
        );
        assert!(history.go(-1).is_none());
        assert_eq!(history.index(), 0);
        assert!(history.can_go(2));
        history.push(HistoryEntry {
            url: "https://example.com/a".into(),
            state: Some("{\"n\":1}".into()),
            title: None,
            document: 0,
        });
        // The forward entries b and c are gone.
        assert_eq!(history.len(), 2);
        assert_eq!(history.index(), 1);
        assert_eq!(history.current().state.as_deref(), Some("{\"n\":1}"));
        history.replace(HistoryEntry {
            url: "https://example.com/a".into(),
            state: Some("{\"n\":2}".into()),
            title: None,
            document: 0,
        });
        assert_eq!(history.len(), 2);
        assert_eq!(history.current().state.as_deref(), Some("{\"n\":2}"));
    }

    #[test]
    fn parent_top_and_frames_read_the_tree() {
        let mut tree = BrowsingContextTree::new("https://example.com/");
        let top = tree.top();
        let a = tree
            .create_child(
                top,
                1,
                &FrameAttributes {
                    name: Some("alpha".into()),
                    ..Default::default()
                },
            )
            .expect("a");
        let b = tree.create_child(top, 2, &frame(None)).expect("b");
        let nested = tree.create_child(a, 3, &frame(None)).expect("nested");

        assert_eq!(tree.frames_of(top), &[a, b]);
        assert_eq!(tree.frames_of(a), &[nested]);
        assert_eq!(tree.parent_of(nested), Some(a));
        assert_eq!(tree.parent_of(top), Some(top));
        assert_eq!(tree.top_of(nested), Some(top));
        assert_eq!(tree.named_frame(top, "alpha"), Some(a));
        assert_eq!(tree.named_frame(top, "missing"), None);
        assert_eq!(tree.context_for_container(3), Some(nested));
    }

    #[test]
    fn discarding_a_context_retires_its_descendants_child_first() {
        let mut tree = BrowsingContextTree::new("https://example.com/");
        let top = tree.top();
        let a = tree.create_child(top, 1, &frame(None)).expect("a");
        let nested = tree.create_child(a, 2, &frame(None)).expect("nested");
        let deep = tree.create_child(nested, 3, &frame(None)).expect("deep");

        let discarded = tree.discard(a);
        assert_eq!(discarded, vec![deep, nested, a]);
        assert!(tree.get(a).is_none());
        assert!(tree.get(nested).is_none());
        assert!(tree.frames_of(top).is_empty());
        assert_eq!(tree.len(), 1);

        // A reused slot does not answer to the discarded id.
        let fresh = tree.create_child(top, 9, &frame(None)).expect("fresh");
        assert_ne!(fresh, a);
        assert!(tree.get(a).is_none());
        assert_eq!(tree.context_for_container(9), Some(fresh));
    }
}
