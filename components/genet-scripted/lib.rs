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
mod resource_bridge;

#[cfg(feature = "livery")]
pub use document::LiveryScriptedDocument;
pub use document::{ScriptedDocument, ScriptedEngine, ScrollKey};
#[cfg(feature = "livery")]
pub use livery::{LiveryCssom, ScriptedClick};
pub use resource_bridge::{ScriptResourceBridge, ScriptWake, ScriptWakeEvent};

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

/// Resolve browser URLs and local paths without treating a Windows drive as a URL
/// scheme. Module resolution uses `url::Url::join` separately where normalization
/// is required.
pub fn resolve_href(base: &str, href: &str) -> String {
    if has_scheme(href) || href.starts_with('/') || href.starts_with('\\') {
        return href.to_string();
    }
    let cut = base.rfind(['/', '\\']).map_or(0, |i| i + 1);
    format!("{}{}", &base[..cut], href)
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
