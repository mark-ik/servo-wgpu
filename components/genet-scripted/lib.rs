/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Host-neutral scripted document/runtime tier.
//!
//! A JS script, handed a **reflector** (a value carrying a `NodeId`), can mutate
//! the corresponding `genet-scripted-dom` node through a native callback: the
//! callback recovers the `NodeId` from the reflector and calls [`LayoutDomMut`].
//! This closes the JS→DOM half of the live-scripting loop. Livery observes the
//! runtime-owned mutation stream through its retained CSSOM session.
//!
//! Built on the engine-neutral `script-engine-api` contract (`NativeFn` +
//! `CallCx` + host data), implemented by `script-engine-nova`. The host DOM
//! reaches the callback through Nova host-defined data, not a `thread_local`. See
//! `docs/2026-05-26_pluggable_engines_testharness_plan.md`.
//!
//! Nova is available on 64-bit targets, including wasm64. Hosts can pair the same
//! document/runtime surface with Boa on wasm32.

#![cfg_attr(target_arch = "wasm32", allow(unused_crate_dependencies))]

use genet_scripted_dom::{NodeId, ScriptedDom};
use script_engine_api::ScriptEngine;

mod capture;
mod document;
#[cfg(feature = "livery")]
mod livery;

#[cfg(feature = "livery")]
pub use document::LiveryScriptedDocument;
pub use document::{ScriptedDocument, ScriptedEngine, ScrollKey};
#[cfg(feature = "livery")]
pub use livery::{LiveryCssom, ScriptedClick};

/// Byte-loading seam supplied by a shell or worker host. Networking and filesystem
/// policy stay above the scripted document owner. This is the shared host contract;
/// the scripted lane no longer carries its own duplicate fetch trait.
pub use genet_host_api::ResourceFetcher;

/// Structural defaults shared by every host of [`ScriptedDocument`].
pub const STRUCTURAL_SHEET: &[&str] = &[
    "html, body, div, p, h1, h2, h3, h4, h5, h6, ul, ol, li, dl, dt, dd, \
     section, article, header, footer, nav, main, aside, figure, figcaption, \
     blockquote, pre, table, thead, tbody, tr, hr, form, fieldset { display: block; }",
    "head, style, script, title, meta, link, base { display: none; }",
    "body { padding: 8px; }",
];

/// Resolve a URL or local path against `base`, without treating a Windows drive
/// as a URL scheme.
///
/// When `base` is a real URL this is the URL standard's relative resolution,
/// through [`url::Url::join`], which is the only thing that answers `./dep.js`,
/// `../dep.js`, the empty string, `#frag` and `?q=1` correctly. It used to be
/// plain prefix concatenation, so a module's `import './dep.js'` resolved to
/// `http://x/./dep.js` and never matched the resource route's key — the bug the
/// module tests in `document.rs` had recorded as their reason for being ignored.
///
/// A scheme-less base (a bare relative path, a Windows path) is not a URL and
/// cannot go through `Url`, so it keeps the prefix form, extended with the same
/// five relative cases and RFC 3986 dot-segment removal.
pub fn resolve_href(base: &str, href: &str) -> String {
    if is_windows_drive(href) || href.starts_with('\\') || has_scheme(href) {
        return href.to_string();
    }
    if has_scheme(base) && !is_windows_drive(base) {
        if let Ok(joined) = url::Url::parse(base).and_then(|b| b.join(href)) {
            return joined.to_string();
        }
    }
    resolve_pathlike(base, href)
}

/// `C:\...` / `c:/...`: a one-letter "scheme" is a drive letter, not a scheme.
fn is_windows_drive(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// The URL standard's relative cases over a base that is not a URL: an empty
/// reference is the base minus its fragment, a fragment-only or query-only
/// reference replaces that component, an absolute path replaces the whole path,
/// and anything else joins at the base's last separator with `.` and `..`
/// removed.
fn resolve_pathlike(base: &str, href: &str) -> String {
    let base_no_frag = base.split('#').next().unwrap_or("");
    if href.is_empty() {
        return base_no_frag.to_string();
    }
    if href.starts_with('#') {
        return format!("{base_no_frag}{href}");
    }
    let base_path = base_no_frag.split('?').next().unwrap_or("");
    if href.starts_with('?') {
        return format!("{base_path}{href}");
    }
    let (href_path, href_rest) = match href.find(['?', '#']) {
        Some(i) => (&href[..i], &href[i..]),
        None => (href, ""),
    };
    let joined = if href_path.starts_with('/') {
        href_path.to_string()
    } else {
        let cut = base_path.rfind(['/', '\\']).map_or(0, |i| i + 1);
        format!("{}{}", &base_path[..cut], href_path)
    };
    format!("{}{href_rest}", remove_dot_segments(&joined))
}

/// RFC 3986 "remove dot segments", over the `/` separator only: a Windows path
/// has no `.`/`..` convention a browser href would use.
fn remove_dot_segments(path: &str) -> String {
    if !path.contains("./") && !path.ends_with('.') {
        return path.to_string();
    }
    let leading = path.starts_with('/');
    let trailing = path.ends_with('/') || path.ends_with("/.") || path.ends_with("/..");
    let mut out: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "." => {},
            ".." => {
                if out.len() > usize::from(leading) {
                    out.pop();
                }
            },
            other => out.push(other),
        }
    }
    let mut joined = out.join("/");
    if trailing && !joined.ends_with('/') {
        joined.push('/');
    }
    joined
}

fn has_scheme(url: &str) -> bool {
    match url.find(':') {
        Some(i) if i > 0 => url[..i]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')),
        _ => false,
    }
}

/// The reflector-pin table (G1 reflector liveness) now lives next to the
/// collector it feeds, in `genet-scripted-dom` as [`Pins`] (keyed on `NodeId`).
/// Re-exported here as `ReflectorPins` — the *host's* word for it — so callers
/// and the engine-coupled helpers below keep a stable name. The host `pin`s a
/// node while script can reach it and `retire`s the ids the engine reports dead;
/// [`collect_dom`] then treats the pinned set as extra mark roots.
pub use genet_scripted_dom::Pins as ReflectorPins;

/// Pump the engine's microtasks, then retire into `pins` any reflectors it
/// reported dead. The host calls this at task boundaries (the
/// [`pump_microtasks`](ScriptEngine::pump_microtasks) cadence). On a fallback
/// backend the drain is empty, so this is pump + a no-op retire (epoch-pin
/// mode); on a death-reporting backend it unpins the freshly collected nodes,
/// the signal G3's collector acts on. The engine reports deaths as
/// [`ReflectorData`] (`u64`); each *is* a `NodeId`'s raw value (the bridge packs
/// `id.raw()`), so they map back through `NodeId::from_raw`. Returns the number
/// of nodes unpinned.
pub fn pump_and_retire<E: ScriptEngine>(engine: &mut E, pins: &mut ReflectorPins) -> usize {
    engine.pump_microtasks();
    let dead = engine.drain_dead_reflectors();
    pins.retire_dead(dead.into_iter().map(|data| NodeId::from_raw(data as usize)))
}

/// Run a mark-sweep collection over `dom`, treating the currently-pinned ids as
/// extra roots (G3). The pin set keeps any orphaned subtree a live reflector can
/// still reach; everything else detached is reaped. Returns the number of nodes
/// pruned.
pub fn collect_dom(dom: &mut ScriptedDom, pins: &ReflectorPins) -> usize {
    dom.collect(pins.iter())
}

/// The full scripted-tier GC tick: pump microtasks, retire the reflectors the
/// engine reported dead (unpinning their nodes), then collect — so a node that
/// JS just dropped its last reference to is reaped in the same step. This is the
/// post-unpin cadence; the host also calls [`collect_dom`] at the
/// `drain_mutations` boundary and on an idle tick. Returns
/// `(reflectors_unpinned, nodes_collected)`.
pub fn pump_retire_collect<E: ScriptEngine>(
    engine: &mut E,
    pins: &mut ReflectorPins,
    dom: &mut ScriptedDom,
) -> (usize, usize) {
    let unpinned = pump_and_retire(engine, pins);
    let collected = collect_dom(dom, pins);
    (unpinned, collected)
}

#[cfg(all(feature = "scripted-nova", target_pointer_width = "64"))]
mod native {
    use std::cell::RefCell;
    use std::rc::Rc;

    use genet_scripted_dom::{NodeId, ScriptedDom};
    use layout_dom_api::LayoutDomMut;
    use script_engine_api::{CallCx, NativeFn, ScriptEngine, ScriptEngineLive};
    use script_engine_nova::NovaEngine;

    /// The host DOM stashed in engine host data, recovered inside the callback.
    type HostDom = RefCell<ScriptedDom>;

    /// `setText(node, text)` — recover the `NodeId` off the reflector argument, read
    /// the text, and set it on the host DOM. Host state arrives through host-defined
    /// data (`CallCx::host_data`), not a `thread_local`.
    struct SetText;

    impl NativeFn<NovaEngine> for SetText {
        fn call(
            cx: &mut <NovaEngine as ScriptEngine>::CallCx<'_>,
        ) -> Result<<NovaEngine as ScriptEngine>::Value, <NovaEngine as ScriptEngine>::Error>
        {
            let node = cx.arg(0);
            let text = cx.arg(1);
            let Some(id) = cx.reflector_data(&node) else {
                return Ok(cx.undefined());
            };
            let text = cx.value_to_string(&text)?;
            if let Some(data) = cx.host_data() {
                if let Some(dom) = data.downcast_ref::<HostDom>() {
                    dom.borrow_mut()
                        .set_text(NodeId::from_raw(id as usize), &text);
                }
            }
            Ok(cx.undefined())
        }
    }

    /// Run `source` against an engine wired so JS can mutate `dom` through the `node`
    /// reflector (which reflects `reflect`).
    pub fn run_script(dom: Rc<RefCell<ScriptedDom>>, reflect: NodeId, source: &str) {
        let mut engine = NovaEngine::new().expect("NovaEngine");
        engine.set_host_data(dom);
        engine
            .set_function::<SetText>("setText", 2)
            .expect("install setText");

        let reflector = engine
            .make_reflector(reflect.raw() as u64)
            .expect("reflector");
        engine.set_global("node", &reflector).expect("install node");

        let _ = engine.eval(source);
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use layout_dom_api::{DomMutation, LayoutDom, LocalName, Namespace, QualName};

        #[test]
        fn js_mutates_dom_through_reflector() {
            let dom = Rc::new(RefCell::new(ScriptedDom::new()));
            let div = {
                let mut d = dom.borrow_mut();
                let root = d.document();
                let div = d.create_element(QualName::new(
                    None,
                    Namespace::from(""),
                    LocalName::from("div"),
                ));
                d.append_child(root, div);
                let mut drained = Vec::new();
                d.drain_mutations(&mut drained); // clear the append
                div
            };

            // JS reaches the host DOM node via its reflector and mutates it.
            run_script(dom.clone(), div, "setText(node, 'hello from JS')");

            let mut d = dom.borrow_mut();
            assert_eq!(d.text(div), Some("hello from JS"));
            let mut muts = Vec::new();
            d.drain_mutations(&mut muts);
            assert!(matches!(
                muts.as_slice(),
                [DomMutation::CharacterDataChanged { .. }]
            ));
        }
    }
}

#[cfg(all(feature = "scripted-nova", target_pointer_width = "64"))]
pub use native::run_script;

#[cfg(test)]
mod pin_tests {
    use super::*;

    // The pure pin-set unit tests live with the type in genet-scripted-dom; this
    // guards the host helper `collect_dom` that feeds the pins into `collect`.
    #[test]
    fn collect_dom_uses_pins_as_roots() {
        use layout_dom_api::{LayoutDom, LayoutDomMut, LocalName, Namespace, QualName};
        let qual = |s: &str| QualName::new(None, Namespace::from(""), LocalName::from(s));

        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let orphan = dom.create_element(qual("o"));
        dom.append_child(root, orphan);
        dom.remove_child(orphan); // detach it from the document

        // A live reflector pins the orphan: collect_dom spares it.
        let mut pins = ReflectorPins::new();
        pins.pin(orphan);
        assert_eq!(collect_dom(&mut dom, &pins), 0);
        assert!(dom.is_live(orphan));

        // JS drops the reflector → unpin → the orphan is reaped.
        pins.unpin(orphan);
        assert_eq!(collect_dom(&mut dom, &pins), 1);
        assert!(!dom.is_live(orphan));
    }
}

#[cfg(all(test, feature = "scripted-nova", target_pointer_width = "64"))]
mod drain_tests {
    use super::*;
    use script_engine_api::ScriptEngineLive;
    use script_engine_nova::NovaEngine;

    /// Only *canonical* reflectors (minted through `reflector_for` and weakly
    /// cached) are death-tracked by `drain_dead_reflectors`. A one-off
    /// `make_reflector` value is not in the canonical cache, so the drain never
    /// reports it and `pump_and_retire` leaves its pin intact until teardown.
    /// (The real canonical-reflector reclamation is exercised end-to-end in
    /// each backend crate's `reflector_for_reports_death_after_gc` — Nova, Boa,
    /// and piccolo all report deaths now; this guards the host pin-table seam.)
    #[test]
    fn non_canonical_reflector_pin_survives_until_teardown() {
        let mut engine = NovaEngine::new().unwrap();
        let mut pins = ReflectorPins::new();

        // Mint a non-canonical reflector for node 0x42 and pin it.
        let reflector = engine.make_reflector(0x42).unwrap();
        pins.pin(NodeId::from_raw(0x42));
        // Drop the only host handle to the reflector.
        drop(reflector);

        // Pump + drain: 0x42 is not in the canonical cache, so the drain reports
        // no death and the pin survives.
        let unpinned = pump_and_retire(&mut engine, &mut pins);
        assert_eq!(unpinned, 0);
        assert!(pins.is_pinned(NodeId::from_raw(0x42)));

        // Teardown clears it.
        pins.clear();
        assert!(pins.is_empty());
    }
}

/// [`resolve_href`], one case per relative form the URL standard defines.
/// Every one of these is a specifier a module `import` can carry, and the
/// prefix-concatenation version answered four of them wrongly.
#[cfg(test)]
mod href_tests {
    use super::resolve_href;

    #[test]
    fn single_dot_is_collapsed() {
        assert_eq!(
            resolve_href("http://x/main.js", "./dep.js"),
            "http://x/dep.js"
        );
        assert_eq!(
            resolve_href("http://x/a/b/main.js", "./dep.js"),
            "http://x/a/b/dep.js"
        );
        // Directory-relative `.` on a scheme-less base takes the same route.
        assert_eq!(resolve_href("a/b/main.js", "./dep.js"), "a/b/dep.js");
    }

    #[test]
    fn double_dot_walks_up() {
        assert_eq!(
            resolve_href("http://x/a/b/main.js", "../dep.js"),
            "http://x/a/dep.js"
        );
        assert_eq!(
            resolve_href("http://x/a/b/main.js", "../../dep.js"),
            "http://x/dep.js"
        );
        // Past the root the URL standard clamps rather than escaping the host.
        assert_eq!(
            resolve_href("http://x/a/main.js", "../../../dep.js"),
            "http://x/dep.js"
        );
        assert_eq!(resolve_href("a/b/main.js", "../dep.js"), "a/dep.js");
    }

    #[test]
    fn empty_specifier_is_the_base_without_its_fragment() {
        assert_eq!(resolve_href("http://x/a/main.js", ""), "http://x/a/main.js");
        assert_eq!(
            resolve_href("http://x/a/main.js?v=1#top", ""),
            "http://x/a/main.js?v=1"
        );
        assert_eq!(resolve_href("a/main.js#top", ""), "a/main.js");
    }

    #[test]
    fn fragment_only_keeps_the_path_and_query() {
        assert_eq!(
            resolve_href("http://x/a/main.js?v=1#old", "#new"),
            "http://x/a/main.js?v=1#new"
        );
        assert_eq!(
            resolve_href("a/main.js?v=1#old", "#new"),
            "a/main.js?v=1#new"
        );
    }

    #[test]
    fn query_only_replaces_the_query_and_drops_the_fragment() {
        assert_eq!(
            resolve_href("http://x/a/main.js?v=1#top", "?v=2"),
            "http://x/a/main.js?v=2"
        );
        assert_eq!(resolve_href("a/main.js?v=1#top", "?v=2"), "a/main.js?v=2");
    }

    #[test]
    fn absolute_and_opaque_references_are_unchanged_relative_forms() {
        // An absolute path against a URL base gains the base's origin, which is
        // what the resource route is keyed by.
        assert_eq!(
            resolve_href("http://x/a/main.js", "/dep.js"),
            "http://x/dep.js"
        );
        // Against a scheme-less base there is no origin to gain.
        assert_eq!(resolve_href("a/b/main.js", "/dep.js"), "/dep.js");
        // A specifier with its own scheme wins outright.
        assert_eq!(
            resolve_href("http://x/main.js", "https://y/dep.js"),
            "https://y/dep.js"
        );
        assert_eq!(
            resolve_href("http://x/main.js", "data:text/javascript,0"),
            "data:text/javascript,0"
        );
    }

    #[test]
    fn a_windows_drive_is_not_a_scheme() {
        assert_eq!(
            resolve_href("docs/a.html", "C:\\pages\\root.html"),
            "C:\\pages\\root.html"
        );
        // As a base it must not be handed to `Url`, which would read `c:` as an
        // opaque scheme and produce nonsense.
        assert_eq!(
            resolve_href("C:\\pages\\index.html", "dep.js"),
            "C:\\pages\\dep.js"
        );
    }

    #[test]
    fn ordinary_relative_joins_are_unchanged() {
        assert_eq!(resolve_href("docs/a.html", "b.html"), "docs/b.html");
        assert_eq!(resolve_href("a.html", "sub/c.html"), "sub/c.html");
        assert_eq!(
            resolve_href("file:///x/a.html", "b.html"),
            "file:///x/b.html"
        );
        assert_eq!(
            resolve_href("http://x/index.html", "main.js"),
            "http://x/main.js"
        );
    }
}
