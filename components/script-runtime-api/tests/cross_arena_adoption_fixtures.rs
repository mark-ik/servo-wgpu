// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Two-engine acceptance fixtures for ordinary same-origin cross-arena DOM
//! adoption: wrapper/document identity, events, ranges, observers, reactions,
//! and component reclamation. Native node identity is full-width and stable
//! across arenas. Only live iframe relocation remains explicitly ignored;
//! it requires a browsing-context ownership transaction beyond DOM adoption.
//! See `design_docs/2026-09-08_realms_plan.md` for the current lane boundary.

use genet_scripted_dom::NodeId;
use layout_dom_api::LayoutDom;
use script_engine_api::ScriptEngine;
use script_runtime_api::{NoScriptLoader, Runtime};

fn runtime<E: ScriptEngine>() -> Runtime<E> {
    let mut runtime = Runtime::<E>::new().expect("runtime");
    runtime
        .set_base_url("https://parent.test/page.html")
        .expect("base URL");
    runtime.parse_document_interleaved(
        "<html><head></head><body><div id='hostb'></div></body></html>",
        &NoScriptLoader,
    );
    runtime
}

fn run<E: ScriptEngine>(runtime: &mut Runtime<E>, source: &str) {
    runtime.eval(source).expect("script");
}

fn read<E: ScriptEngine>(runtime: &mut Runtime<E>, expression: &str) -> String {
    let value = runtime.eval(expression).expect("eval");
    runtime.value_to_string(&value).expect("stringify")
}

fn drain<E: ScriptEngine>(runtime: &mut Runtime<E>) {
    runtime.run_event_loop(100).expect("frame tasks");
}

/// Every case starts here: a document B (the parent realm) and a document A
/// living in a **same-origin** iframe (`srcdoc`, not a foreign `src` â€” a
/// cross-origin frame's `contentDocument` is null by design, per
/// `frame_realms.rs`'s `cross_origin_window_rejects_non_whitelisted_reads`,
/// and that is a different, already-covered refusal from the one this file
/// tests). Same-origin is enough: each iframe document still gets its own
/// `ScriptedDom` arena (one `HostState` per `RealmId`), which is the whole
/// point â€” the arena boundary does not coincide with the origin boundary.
/// Wired up and drained so `frame.contentWindow` and `frame.contentDocument`
/// are live. Setup alone must never be the thing that fails â€” only the
/// cross-arena action that follows it.
fn two_documents<E: ScriptEngine>() -> Runtime<E> {
    let mut runtime = runtime::<E>();
    run(
        &mut runtime,
        "var frame = document.createElement('iframe'); \
         frame.srcdoc = '<html><body><div id=\"target\"><span id=\"leaf\">leaf</span></div></body></html>'; \
         document.body.appendChild(frame);",
    );
    drain(&mut runtime);
    runtime
}

// ---------------------------------------------------------------------------
// WPT-derived focused local reproductions (the plan's regression table).
// ---------------------------------------------------------------------------

/// WPT `dom/nodes/Node-appendChild.html`, "Adopting an orphan": a node
/// detached in its creation document, appended into a foreign document, keeps
/// its JS wrapper identity and gains the foreign document as `ownerDocument`.
fn adopting_an_orphan_preserves_identity_and_owner_document<E: ScriptEngine>() {
    let mut runtime = two_documents::<E>();
    run(
        &mut runtime,
        "var childDoc = frame.contentDocument; \
         globalThis.orphan = childDoc.createElement('p'); \
         globalThis.orphanOwnerBefore = orphan.ownerDocument === childDoc;",
    );
    assert_eq!(read(&mut runtime, "String(orphanOwnerBefore)"), "true");
    run(
        &mut runtime,
        "var appended = document.getElementById('hostb').appendChild(orphan);",
    );
    assert_eq!(
        read(&mut runtime, "String(appended === orphan)"),
        "true",
        "appendChild must return the same wrapper it was given"
    );
    assert_eq!(
        read(&mut runtime, "String(orphan.ownerDocument === document)"),
        "true",
        "adopting steps must retarget ownerDocument to the destination document"
    );
    assert_eq!(
        read(
            &mut runtime,
            "String(orphan.parentNode === document.getElementById('hostb'))"
        ),
        "true"
    );
}

/// Same case, but proves descendant and attribute ownership move too â€” G5's
/// "Adopting steps additionally require descendant and attribute document
/// ownership" and `docs/2026-06-11_gc_arena_dom_plan.md` G5's Mutation clause.
fn adoption_updates_descendant_and_attribute_owner_documents<E: ScriptEngine>() {
    let mut runtime = two_documents::<E>();
    run(
        &mut runtime,
        "var childDoc = frame.contentDocument; \
         globalThis.subtree = childDoc.getElementById('target'); \
         subtree.setAttribute('data-tag', 'from-a');",
    );
    run(
        &mut runtime,
        "document.getElementById('hostb').appendChild(subtree);",
    );
    assert_eq!(
        read(&mut runtime, "String(subtree.ownerDocument === document)"),
        "true"
    );
    assert_eq!(
        read(
            &mut runtime,
            "String(subtree.querySelector('#leaf').ownerDocument === document)"
        ),
        "true",
        "descendants must be re-owned, not only the adopted root"
    );
    assert_eq!(
        read(
            &mut runtime,
            "String(subtree.getAttributeNode('data-tag').ownerDocument === document)"
        ),
        "true",
        "attribute nodes carry document ownership independently per the adopting steps"
    );
}

/// WPT `dom/nodes/Node-isConnected.html`, "Test with iframes": a node that was
/// connected in its origin document, once appended under the destination
/// document's connected tree, reports connected there â€” and disconnected in
/// its old tree if it was the last reference.
fn isconnected_reflects_iframe_adoption<E: ScriptEngine>() {
    let mut runtime = two_documents::<E>();
    run(
        &mut runtime,
        "var childDoc = frame.contentDocument; \
         globalThis.moved = childDoc.getElementById('target'); \
         globalThis.wasConnectedInA = moved.isConnected;",
    );
    assert_eq!(read(&mut runtime, "String(wasConnectedInA)"), "true");
    run(
        &mut runtime,
        "document.getElementById('hostb').appendChild(moved);",
    );
    assert_eq!(
        read(&mut runtime, "String(moved.isConnected)"),
        "true",
        "moved node must be connected through the destination document's tree"
    );
    assert_eq!(
        read(
            &mut runtime,
            "String(frame.contentDocument.getElementById('target'))"
        ),
        "null",
        "the source document must no longer contain the moved subtree"
    );
}

/// WPT `html/browsers/the-window-object/.../window_length.html`, "Child
/// browsing context has a child browsing context": moving an `<iframe>`
/// element (itself hosting a nested browsing context) between two documents
/// must re-parent its browsing context, not merely its DOM node, so the
/// destination window's `.length`/`frames` reflect the moved nested context.
fn moving_an_iframe_element_relocates_its_browsing_context<E: ScriptEngine>() {
    let mut runtime = two_documents::<E>();
    run(
        &mut runtime,
        "var childDoc = frame.contentDocument; \
         var nested = childDoc.createElement('iframe'); \
         nested.srcdoc = '<body></body>'; \
         childDoc.body.appendChild(nested);",
    );
    drain(&mut runtime);
    run(
        &mut runtime,
        "globalThis.nestedFrame = nested; \
         document.getElementById('hostb').appendChild(nested);",
    );
    drain(&mut runtime);
    assert_eq!(
        read(&mut runtime, "String(window.length)"),
        "2",
        "the top window now has both the original child iframe and the relocated nested one"
    );
    assert_eq!(
        read(
            &mut runtime,
            "String(nestedFrame.contentWindow.top === window)"
        ),
        "true",
        "the relocated nested browsing context's top must be the new document's window"
    );
}

/// WPT `html/dom/partial-updates/tentative/template-for-html-setters.html`,
/// "Setter createContextualFragment should not patch existing target in
/// head": a `Range` created against one document must not let
/// `createContextualFragment` reach into another document's existing nodes
/// when the range's owner document is adopted or reused across arenas.
fn contextual_fragment_does_not_patch_existing_target_in_foreign_head<E: ScriptEngine>() {
    let mut runtime = two_documents::<E>();
    run(
        &mut runtime,
        "var marker = document.createElement('title'); \
         marker.textContent = 'original'; \
         document.head.appendChild(marker); \
         var childDoc = frame.contentDocument; \
         globalThis.foreignRange = childDoc.createRange(); \
         foreignRange.selectNodeContents(childDoc.body); \
         var fragment = foreignRange.createContextualFragment('<title>from-b</title>'); \
         document.getElementById('hostb').appendChild(fragment);",
    );
    assert_eq!(
        read(
            &mut runtime,
            "document.head.querySelectorAll('title').length + ''"
        ),
        "1",
        "a contextual fragment parsed against a foreign range must not mutate the \
         destination document's existing head content"
    );
    assert_eq!(read(&mut runtime, "marker.textContent"), "original");
}

// ---------------------------------------------------------------------------
// Deliverable-mandated coverage beyond the WPT table.
// ---------------------------------------------------------------------------

/// Event propagation on an adopted node must end at the *current* root
/// document's `defaultView` â€” the realms plan's "Runtime coordination
/// inventory" finding that `dispatchEvent`'s creation-realm `globalThis.window`
/// becomes wrong after adoption.
fn dispatch_after_adoption_ends_at_destination_defaultview<E: ScriptEngine>() {
    let mut runtime = two_documents::<E>();
    run(
        &mut runtime,
        "var childDoc = frame.contentDocument; \
         globalThis.moved = childDoc.getElementById('target'); \
         globalThis.sourceHeard = false, globalThis.destHeard = false; \
         frame.contentWindow.addEventListener('bubbles-test', function(){ sourceHeard = true; }); \
         window.addEventListener('bubbles-test', function(){ destHeard = true; });",
    );
    run(
        &mut runtime,
        "document.getElementById('hostb').appendChild(moved);",
    );
    run(
        &mut runtime,
        "moved.dispatchEvent(new Event('bubbles-test', { bubbles: true }));",
    );
    assert_eq!(
        read(&mut runtime, "String(destHeard)"),
        "true",
        "the event must bubble to the destination document's defaultView"
    );
    assert_eq!(
        read(&mut runtime, "String(sourceHeard)"),
        "false",
        "the creation realm's window must not still be on the propagation path"
    );
}

/// Live `Range` endpoints and `rangeIndex` bucketing (`dom/bootstrap.js`'s
/// per-realm `rangeIndex`) must follow an adopted node into its new arena.
fn live_range_endpoints_follow_adopted_node<E: ScriptEngine>() {
    let mut runtime = two_documents::<E>();
    run(
        &mut runtime,
        r#"
        var childDoc = frame.contentDocument;
        var movedTarget = childDoc.getElementById('target');
        var range = childDoc.createRange();
        range.selectNode(movedTarget.firstChild);
        document.getElementById('hostb').appendChild(movedTarget);
        if (!range.collapsed || range.startContainer !== childDoc.body || range.startOffset !== 0 ||
            range.endContainer !== childDoc.body || range.endOffset !== 0)
            throw new Error('connected removal did not relocate source boundaries');
        movedTarget.remove();
        var detachedRange = document.createRange();
        var originalLeaf = movedTarget.firstChild;
        detachedRange.selectNode(originalLeaf);
        childDoc.adoptNode(movedTarget);
        if (detachedRange.startContainer !== movedTarget || detachedRange.endContainer !== movedTarget ||
            detachedRange.startOffset !== 0 || detachedRange.endOffset !== 1 || movedTarget.firstChild !== originalLeaf)
            throw new Error('detached adoption changed boundary identity');
        originalLeaf.textContent = 'changed';
        if (detachedRange.toString() !== 'changed' || movedTarget.ownerDocument !== childDoc)
            throw new Error('detached adopted range lost destination mutation');
    "#,
    );
}

/// `MutationObserver` registrations in *both* the source and destination
/// documents must be delivered for one adoption: a childList removal record
/// in the source, and a childList insertion record in the destination.
fn mutation_observer_delivers_in_both_arenas<E: ScriptEngine>() {
    let mut runtime = two_documents::<E>();
    run(
        &mut runtime,
        "var childDoc = frame.contentDocument; \
         globalThis.sourceRecords = [], globalThis.destRecords = []; \
         var sourceObserver = new childDoc.defaultView.MutationObserver(function(r){ sourceRecords.push.apply(sourceRecords, r); }); \
         sourceObserver.observe(childDoc.body, { childList: true }); \
         var destObserver = new MutationObserver(function(r){ destRecords.push.apply(destRecords, r); }); \
         var hostb = document.getElementById('hostb'); \
         destObserver.observe(hostb, { childList: true }); \
         globalThis.moved = childDoc.getElementById('target'); \
         hostb.appendChild(moved);",
    );
    drain(&mut runtime);
    assert_eq!(
        read(
            &mut runtime,
            "String(sourceRecords.length === 1 && sourceRecords[0].removedNodes[0] === moved)"
        ),
        "true",
        "the source document's observer must see the removal"
    );
    assert_eq!(
        read(
            &mut runtime,
            "String(destRecords.length === 1 && destRecords[0].addedNodes[0] === moved)"
        ),
        "true",
        "the destination document's observer must see the insertion"
    );
}

/// Custom-element reaction ordering across an adoption: disconnect in the
/// source, `adoptedCallback`, then connect in the destination â€” DOM's
/// adopting steps order, exercised through `appendChild`'s implicit adopt.
fn custom_element_reactions_run_in_adoption_order<E: ScriptEngine>() {
    let mut runtime = two_documents::<E>();
    run(
        &mut runtime,
        "globalThis.order = []; \
         function define(win, doc){ \
           win.customElements.define('x-lane', class extends win.HTMLElement { \
             connectedCallback(){ order.push('connected:' + (this.ownerDocument === doc)); } \
             disconnectedCallback(){ order.push('disconnected'); } \
             adoptedCallback(oldDoc,newDoc){ order.push('adopted:' + (oldDoc === doc && newDoc === document)); } \
           }); \
         } \
         define(frame.contentWindow, frame.contentDocument); \
         var childDoc = frame.contentDocument; \
         globalThis.el = childDoc.createElement('x-lane'); \
         childDoc.body.appendChild(el);",
    );
    drain(&mut runtime);
    run(
        &mut runtime,
        "order.length = 0; document.getElementById('hostb').appendChild(el);",
    );
    drain(&mut runtime);
    assert_eq!(
        read(&mut runtime, "order.join(',')"),
        "disconnected,adopted:true,connected:false",
        "reactions must fire in adopting-steps order, and connectedCallback's \
         the callback captures the source document, so its owner comparison becomes false"
    );
    assert_eq!(
        read(&mut runtime, "String(el.ownerDocument === document)"),
        "true"
    );
}

/// Release the retained wrapper, force GC, and check reclamation through the
/// same host-side accounting the G5 receipts use (`Pins::len`/`is_empty`,
/// `Runtime::collect_garbage`'s `(unpinned, collected)` counts) in *both*
/// arenas: no dangling pin, no leftover death record in either realm's
/// `HostState`.
fn adopted_wrapper_reclaimed_with_no_dangling_pin_in_either_arena<E: ScriptEngine>() {
    let mut runtime = two_documents::<E>();
    run(
        &mut runtime,
        r#"
        var childDoc = frame.contentDocument;
        var held = childDoc.getElementById('target');
        var componentIds = [__nodeRawId(held.__ref), __nodeRawId(held.firstChild.__ref)];
        document.getElementById('hostb').appendChild(held);
        held.remove();
    "#,
    );
    let ids: Vec<NodeId> = read(&mut runtime, "componentIds.join(',')")
        .split(',')
        .map(|raw| NodeId::from_raw(raw.parse().unwrap()))
        .collect();
    let realm = runtime.frame_realms(0)[0].1;
    let source = runtime.host_in_realm(realm).unwrap();
    for id in &ids {
        assert!(
            !source.borrow().pins.is_pinned(*id),
            "adopted component left a source pin"
        );
        assert!(
            !source.borrow().dom.is_live(*id),
            "adopted component stayed in source storage"
        );
    }
    for _ in 0..3 {
        runtime.collect_garbage();
    }
    for id in &ids {
        assert!(
            runtime.host().borrow().dom.is_live(*id),
            "held wrapper lost detached component"
        );
        assert!(
            runtime.host().borrow().pins.is_pinned(*id),
            "held component lost destination pin"
        );
    }
    run(
        &mut runtime,
        "held = null; Promise.resolve().then(function() {});",
    );
    runtime.run_microtasks();
    for _ in 0..8 {
        runtime.collect_garbage();
        if ids
            .iter()
            .all(|id| !runtime.host().borrow().dom.is_live(*id))
        {
            break;
        }
    }
    for id in ids {
        assert!(
            !runtime.host().borrow().dom.is_live(id),
            "released detached component was not reclaimed"
        );
        assert!(
            !runtime.host().borrow().pins.is_pinned(id),
            "component left destination pin"
        );
        assert!(
            !source.borrow().pins.is_pinned(id),
            "component left source pin"
        );
    }
}

// ---------------------------------------------------------------------------
// Control: proves the fixture itself distinguishes storage transfer from
// adoption. Runs unignored today and must currently PASS, because today's
// single-document-model `adoptNode`/`appendChild` *is* exactly "JS-side
// bookkeeping updated, arena storage never moved" when the node did not
// originate in the destination document's own arena but the two happen to
// share one arena (the non-iframe, same-realm case) â€” see below. If a future
// change makes this assertion fail, that is a signal the control itself needs
// updating alongside the router, not that adoption regressed.
// ---------------------------------------------------------------------------

/// A same-arena, single-document control: build a node whose bookkeeping is
/// forced stale by hand (simulating "storage moved, ownerDocument accounting
/// did not run") and confirm the same assertion style used above actually
/// trips on it. This is the fixture's self-check, not a DOM API exercise.
///
/// The stale state is forged with `Object.defineProperty`, which installs an
/// own data property on `moved` that shadows `Node.prototype`'s
/// `ownerDocument` accessor â€” simulating a transfer that moved the node's
/// storage without running the adopting steps' owner-document propagation.
/// (Note for anyone editing the JS below: keep it free of `//` line comments.
/// Rust's `\`-continued string literals strip the newline, so a `//` comment
/// here would swallow every line after it as well.)
fn storage_transfer_without_owner_document_update_trips_the_fixture<E: ScriptEngine>() {
    let mut runtime = runtime::<E>();
    run(
        &mut runtime,
        "var moved = document.createElement('p'); \
         document.getElementById('hostb').appendChild(moved); \
         Object.defineProperty(moved, 'ownerDocument', { value: undefined, configurable: true });",
    );
    let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_eq!(
            read(&mut runtime, "String(moved.ownerDocument === document)"),
            "true",
            "storage transfer alone (bookkeeping left stale) must not read as adoption"
        );
    }))
    .is_err();
    assert!(
        ok,
        "the shared ownerDocument assertion must trip on a node whose storage moved \
         without the adopting steps' bookkeeping â€” otherwise the regression suite above \
         could pass on storage transfer alone and this control exists to prevent that"
    );
}

macro_rules! both_engines {
    ($($body:ident => ($boa:ident, $nova:ident)),* $(,)?) => {
        $(
            #[test]
            fn $boa() { $body::<script_engine_boa::BoaEngine>(); }

            #[cfg(target_pointer_width = "64")]
            #[test]
            fn $nova() { $body::<script_engine_nova::NovaEngine>(); }
        )*
    };
}

both_engines! {
    adopting_an_orphan_preserves_identity_and_owner_document =>
        (wpt_adopting_orphan_on_boa, wpt_adopting_orphan_on_nova),
    adoption_updates_descendant_and_attribute_owner_documents =>
        (descendant_attribute_owner_on_boa, descendant_attribute_owner_on_nova),
    isconnected_reflects_iframe_adoption =>
        (wpt_isconnected_iframes_on_boa, wpt_isconnected_iframes_on_nova),
    contextual_fragment_does_not_patch_existing_target_in_foreign_head =>
        (wpt_contextual_fragment_head_on_boa, wpt_contextual_fragment_head_on_nova),
    dispatch_after_adoption_ends_at_destination_defaultview =>
        (dispatch_defaultview_on_boa, dispatch_defaultview_on_nova),
    live_range_endpoints_follow_adopted_node =>
        (range_endpoints_on_boa, range_endpoints_on_nova),
    mutation_observer_delivers_in_both_arenas =>
        (mutation_observer_both_arenas_on_boa, mutation_observer_both_arenas_on_nova),
    custom_element_reactions_run_in_adoption_order =>
        (custom_element_order_on_boa, custom_element_order_on_nova),
    adopted_wrapper_reclaimed_with_no_dangling_pin_in_either_arena =>
        (reclamation_no_dangling_pin_on_boa, reclamation_no_dangling_pin_on_nova),
}

#[test]
#[ignore = "live iframe adoption requires relocating browsing-context parentage, loading and host registration; ordinary DOM subtree lane refuses it"]
fn wpt_window_length_nested_context_on_boa() {
    moving_an_iframe_element_relocates_its_browsing_context::<script_engine_boa::BoaEngine>();
}
#[cfg(target_pointer_width = "64")]
#[test]
#[ignore = "live iframe adoption requires relocating browsing-context parentage, loading and host registration; ordinary DOM subtree lane refuses it"]
fn wpt_window_length_nested_context_on_nova() {
    moving_an_iframe_element_relocates_its_browsing_context::<script_engine_nova::NovaEngine>();
}

macro_rules! control_both_engines {
    ($($body:ident => ($boa:ident, $nova:ident)),* $(,)?) => {
        $(
            #[test]
            fn $boa() { $body::<script_engine_boa::BoaEngine>(); }

            #[cfg(target_pointer_width = "64")]
            #[test]
            fn $nova() { $body::<script_engine_nova::NovaEngine>(); }
        )*
    };
}

control_both_engines! {
    storage_transfer_without_owner_document_update_trips_the_fixture =>
        (control_storage_transfer_on_boa, control_storage_transfer_on_nova),
}
