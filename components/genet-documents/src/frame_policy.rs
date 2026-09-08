// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Cross-context policy: what one browsing context may do to another, and the
//! two isolation headers, parsed and recorded.
//!
//! The split from [`crate::browsing_context`] is deliberate. That module holds
//! origins and sandbox flags as *values*; this one holds the decisions taken
//! from them, so a caller cannot accidentally re-derive "may I touch this"
//! three different ways.
//!
//! ## What one process can and cannot enforce
//!
//! Genet runs every browsing context in one process today. Same-origin
//! document access, `postMessage` origin checks, the WindowProxy cross-origin
//! member list and the sandbox flags that gate *our own* behaviour (scripts,
//! forms, navigation) are all enforceable here, because every one of them is a
//! check this engine performs before it acts.
//!
//! Cross-Origin-Opener-Policy and Cross-Origin-Embedder-Policy are not. Their
//! observable effect is that a cross-origin document lands in a **different
//! agent cluster** — a separate process, with separate memory — so that
//! `SharedArrayBuffer`, high-resolution timers and Spectre-adjacent reads
//! cannot cross. Parsing them and recording the resulting
//! [`CrossOriginIsolation`] is honest; claiming to enforce them in one process
//! would not be. The residuals are named in the plan and in
//! [`CrossOriginIsolation::residual`].

use crate::browsing_context::{Origin, SandboxFlags};

/// A parsed `Cross-Origin-Opener-Policy` header value.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OpenerPolicy {
    #[default]
    UnsafeNone,
    SameOriginAllowPopups,
    SameOrigin,
}

impl OpenerPolicy {
    /// Parse the header value. An absent or unrecognised value is
    /// `unsafe-none`, the header's own default.
    pub fn parse(header: Option<&str>) -> Self {
        let Some(header) = header else {
            return OpenerPolicy::UnsafeNone;
        };
        // Structured-header token plus optional parameters; only the token
        // decides the policy here, and the `report-to` parameter is dropped.
        let token = header
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        match token.as_str() {
            "same-origin" => OpenerPolicy::SameOrigin,
            "same-origin-allow-popups" => OpenerPolicy::SameOriginAllowPopups,
            _ => OpenerPolicy::UnsafeNone,
        }
    }
}

/// A parsed `Cross-Origin-Embedder-Policy` header value.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EmbedderPolicy {
    #[default]
    UnsafeNone,
    Credentialless,
    RequireCorp,
}

impl EmbedderPolicy {
    pub fn parse(header: Option<&str>) -> Self {
        let Some(header) = header else {
            return EmbedderPolicy::UnsafeNone;
        };
        let token = header
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        match token.as_str() {
            "require-corp" => EmbedderPolicy::RequireCorp,
            "credentialless" => EmbedderPolicy::Credentialless,
            _ => EmbedderPolicy::UnsafeNone,
        }
    }
}

/// The isolation a document's two headers ask for, and what this engine
/// actually does about it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CrossOriginIsolation {
    pub opener: OpenerPolicy,
    pub embedder: EmbedderPolicy,
}

impl CrossOriginIsolation {
    pub fn from_headers(opener: Option<&str>, embedder: Option<&str>) -> Self {
        Self {
            opener: OpenerPolicy::parse(opener),
            embedder: EmbedderPolicy::parse(embedder),
        }
    }

    /// Whether the pair *asks* for cross-origin isolation. This is the
    /// question `self.crossOriginIsolated` answers in a browser that can
    /// deliver it; genet records it and does not report it as true, because a
    /// single process cannot back the guarantee.
    pub fn requests_isolation(self) -> bool {
        self.opener == OpenerPolicy::SameOrigin
            && matches!(
                self.embedder,
                EmbedderPolicy::RequireCorp | EmbedderPolicy::Credentialless
            )
    }

    /// The named residual for this document, or `None` when the headers ask
    /// for nothing a single process withholds.
    pub fn residual(self) -> Option<&'static str> {
        if self.requests_isolation() {
            return Some(
                "cross-origin isolation requested: agent-cluster separation, \
                 SharedArrayBuffer gating and crossOriginIsolated need a second \
                 process, which genet does not have",
            );
        }
        if self.opener != OpenerPolicy::UnsafeNone {
            return Some(
                "Cross-Origin-Opener-Policy recorded but not enforced: opener \
                 severance is a browsing-context-group split across processes",
            );
        }
        if self.embedder != EmbedderPolicy::UnsafeNone {
            return Some(
                "Cross-Origin-Embedder-Policy recorded but not enforced: \
                 subresource CORP checking is a fetch-side gate this lane did \
                 not land",
            );
        }
        None
    }
}

/// What a context may reach in another context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentAccess {
    /// Same origin: the full object graph, including `contentDocument`.
    Full,
    /// Cross origin: only the WindowProxy members HTML permits.
    CrossOriginOnly,
}

impl DocumentAccess {
    pub fn between(accessor: &Origin, target: &Origin) -> Self {
        if accessor.is_same_origin(target) {
            DocumentAccess::Full
        } else {
            DocumentAccess::CrossOriginOnly
        }
    }

    pub fn is_full(self) -> bool {
        self == DocumentAccess::Full
    }
}

/// The `WindowProxy` members a cross-origin accessor may still touch.
///
/// HTML calls these the *cross-origin properties*. Everything else throws a
/// `SecurityError`, which is why this list is data rather than a set of
/// `if`s scattered through the bootstrap: one place to read, one place to
/// test, and the same list on both engines.
pub const CROSS_ORIGIN_WINDOW_PROPERTIES: &[&str] = &[
    "blur",
    "close",
    "closed",
    "focus",
    "frames",
    "length",
    "location",
    "opener",
    "parent",
    "postMessage",
    "self",
    "top",
    "window",
];

/// The `Location` members a cross-origin accessor may still touch.
pub const CROSS_ORIGIN_LOCATION_PROPERTIES: &[&str] = &["href", "replace"];

/// Whether a cross-origin accessor may read `name` off a WindowProxy.
///
/// `Symbol.toStringTag`, `Symbol.hasInstance` and `Symbol.isConcatSpreadable`
/// are also permitted by the specification; they arrive here as their
/// `"Symbol(...)"` spellings from the bootstrap, which is the only form a
/// string-marshalled boundary can carry.
pub fn is_cross_origin_window_property(name: &str) -> bool {
    CROSS_ORIGIN_WINDOW_PROPERTIES.contains(&name)
        || matches!(
            name,
            "Symbol(Symbol.toStringTag)"
                | "Symbol(Symbol.hasInstance)"
                | "Symbol(Symbol.isConcatSpreadable)"
        )
        || name.strip_prefix("__index__").is_some_and(|index| {
            // Indexed access into `frames` is permitted cross-origin.
            index.chars().all(|c| c.is_ascii_digit()) && !index.is_empty()
        })
}

/// Whether `postMessage`'s `targetOrigin` permits delivery to `target`.
///
/// `"*"` always matches, `"/"` means the sender's own origin, and any other
/// value is compared as a serialized origin.
pub fn target_origin_matches(target_origin: &str, sender: &Origin, target: &Origin) -> bool {
    match target_origin {
        "*" => true,
        "/" => sender.is_same_origin(target),
        explicit => Origin::of_url(explicit, u64::MAX).serialize() == target.serialize(),
    }
}

/// Whether a sandboxed context may run script.
pub fn scripts_allowed(sandbox: SandboxFlags) -> bool {
    !sandbox.contains(SandboxFlags::SCRIPTS)
}

/// Whether a sandboxed context may submit a form.
pub fn forms_allowed(sandbox: SandboxFlags) -> bool {
    !sandbox.contains(SandboxFlags::FORMS)
}

/// Whether a sandboxed context may navigate itself.
pub fn navigation_allowed(sandbox: SandboxFlags) -> bool {
    !sandbox.contains(SandboxFlags::NAVIGATION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_isolation_headers_parse_their_token_and_ignore_parameters() {
        assert_eq!(
            OpenerPolicy::parse(Some("same-origin; report-to=\"a\"")),
            OpenerPolicy::SameOrigin
        );
        assert_eq!(
            OpenerPolicy::parse(Some("  Same-Origin-Allow-Popups ")),
            OpenerPolicy::SameOriginAllowPopups
        );
        assert_eq!(
            OpenerPolicy::parse(Some("nonsense")),
            OpenerPolicy::UnsafeNone
        );
        assert_eq!(OpenerPolicy::parse(None), OpenerPolicy::UnsafeNone);
        assert_eq!(
            EmbedderPolicy::parse(Some("require-corp")),
            EmbedderPolicy::RequireCorp
        );
        assert_eq!(
            EmbedderPolicy::parse(Some("credentialless")),
            EmbedderPolicy::Credentialless
        );
        assert_eq!(EmbedderPolicy::parse(None), EmbedderPolicy::UnsafeNone);
    }

    #[test]
    fn requested_isolation_is_recorded_with_a_named_residual_rather_than_claimed() {
        let isolated =
            CrossOriginIsolation::from_headers(Some("same-origin"), Some("require-corp"));
        assert!(isolated.requests_isolation());
        assert!(isolated.residual().is_some_and(|r| r.contains("second")));
        let none = CrossOriginIsolation::default();
        assert!(!none.requests_isolation());
        assert!(none.residual().is_none());
        let coop_only = CrossOriginIsolation::from_headers(Some("same-origin"), None);
        assert!(!coop_only.requests_isolation());
        assert!(coop_only.residual().is_some());
    }

    #[test]
    fn document_access_is_full_only_between_same_origins() {
        let a = Origin::of_url("https://example.com/x", 0);
        let b = Origin::of_url("https://example.com:443/y", 0);
        let c = Origin::of_url("https://other.example/", 0);
        assert!(DocumentAccess::between(&a, &b).is_full());
        assert!(!DocumentAccess::between(&a, &c).is_full());
        assert!(!DocumentAccess::between(&Origin::Opaque(1), &Origin::Opaque(2)).is_full());
        assert!(DocumentAccess::between(&Origin::Opaque(1), &Origin::Opaque(1)).is_full());
    }

    #[test]
    fn the_cross_origin_member_list_admits_only_what_html_names() {
        for allowed in [
            "postMessage",
            "location",
            "top",
            "parent",
            "frames",
            "closed",
        ] {
            assert!(is_cross_origin_window_property(allowed), "{allowed}");
        }
        for denied in [
            "document",
            "alert",
            "name",
            "innerWidth",
            "getComputedStyle",
        ] {
            assert!(!is_cross_origin_window_property(denied), "{denied}");
        }
        assert!(is_cross_origin_window_property("__index__0"));
        assert!(!is_cross_origin_window_property("__index__"));
        assert!(is_cross_origin_window_property(
            "Symbol(Symbol.toStringTag)"
        ));
    }

    #[test]
    fn target_origin_star_slash_and_explicit_each_match_what_they_should() {
        let sender = Origin::of_url("https://a.example/", 0);
        let same = Origin::of_url("https://a.example/other", 0);
        let other = Origin::of_url("https://b.example/", 0);
        assert!(target_origin_matches("*", &sender, &other));
        assert!(target_origin_matches("/", &sender, &same));
        assert!(!target_origin_matches("/", &sender, &other));
        assert!(target_origin_matches("https://b.example", &sender, &other));
        assert!(target_origin_matches(
            "https://b.example/ignored",
            &sender,
            &other
        ));
        assert!(!target_origin_matches("https://c.example", &sender, &other));
        // An opaque target serializes to "null" and matches only "*".
        let opaque = Origin::Opaque(3);
        assert!(target_origin_matches("*", &sender, &opaque));
        assert!(!target_origin_matches(
            "https://a.example",
            &sender,
            &opaque
        ));
    }

    #[test]
    fn the_sandbox_gates_read_the_flag_they_name() {
        let full = SandboxFlags::parse(Some(""));
        assert!(!scripts_allowed(full));
        assert!(!forms_allowed(full));
        assert!(!navigation_allowed(full));
        let permissive = SandboxFlags::parse(Some("allow-scripts allow-forms allow-navigation"));
        assert!(scripts_allowed(permissive));
        assert!(forms_allowed(permissive));
        assert!(navigation_allowed(permissive));
        assert!(scripts_allowed(SandboxFlags::NONE));
    }
}
