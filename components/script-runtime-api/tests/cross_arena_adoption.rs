// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use genet_scripted_dom::NodeId;
use layout_dom_api::LayoutDom;
use script_engine_api::{MAIN_REALM, ScriptEngine};
use script_runtime_api::{NoScriptLoader, Runtime};

fn runtime<E: ScriptEngine>() -> Runtime<E> {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_base_url("https://parent.test/index.html").unwrap();
    rt.parse_document_interleaved("<body><iframe id=frame></iframe></body>", &NoScriptLoader);
    rt.run_event_loop(20).unwrap();
    rt.eval("var child = document.getElementById('frame').contentWindow;")
        .unwrap();
    rt
}

fn run<E: ScriptEngine>(rt: &mut Runtime<E>, source: &str) {
    if let Err(error) = rt.eval(source) {
        panic!("{source}: {}", rt.describe_error(&error));
    }
}

fn automatic_insertion_preserves_wrapper_and_routes_events<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    run(
        &mut rt,
        r#"
        var node = document.createElement('section');
        node.id = 'adopted';
        node.innerHTML = '<span id=leaf>text</span>';
        var leaf = node.firstChild, prototype = Object.getPrototypeOf(node);
        var token = {}; node.expando = token;
        var localEvents=0, sourceEvents=0, targetEvents=0;
        node.addEventListener('adoption-probe', function(e) {
            if (this !== node || e.target !== leaf) throw new Error('listener identity');
            localEvents++;
        });
        window.addEventListener('adoption-probe', function(){sourceEvents++;});
        child.addEventListener('adoption-probe', function(){targetEvents++;});
        if (child.document.body.appendChild(node) !== node) throw new Error('append return');
        if (child.document.getElementById('adopted') !== node ||
            child.document.getElementById('leaf') !== leaf || document.getElementById('adopted') !== null)
            throw new Error('lookup identity or source ownership');
        if (node.ownerDocument !== child.document || leaf.ownerDocument !== child.document ||
            node.parentNode !== child.document.body || Object.getPrototypeOf(node) !== prototype ||
            !(node instanceof HTMLElement) || node instanceof child.HTMLElement || node.expando !== token)
            throw new Error('wrapper identity/prototype/owner');
        leaf.dispatchEvent(new Event('adoption-probe', {bubbles:true}));
        if (localEvents !== 1 || sourceEvents !== 0 || targetEvents !== 1)
            throw new Error('event bubbled to wrong realm');
        child.Element.prototype.setAttribute.call(node, 'data-from', 'destination');
        if (node.getAttribute('data-from') !== 'destination') throw new Error('source getter routing');
    "#,
    );
}

fn explicit_adoption_keeps_detached_identity<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    run(
        &mut rt,
        r#"
        var node = document.createElement('div');
        node.innerHTML = '<b>kept</b>';
        var nested = node.firstChild;
        if (child.document.adoptNode(node) !== node || node.parentNode !== null || node.isConnected ||
            node.ownerDocument !== child.document || nested.ownerDocument !== child.document)
            throw new Error('explicit detached adoption');
        if (node.firstChild !== nested || nested.textContent !== 'kept') throw new Error('subtree identity');
        child.document.body.appendChild(node);
        if (child.document.body.firstChild !== node) throw new Error('destination query reminted wrapper');
        node.remove();
        if (document.adoptNode(node) !== node || nested.ownerDocument !== document)
            throw new Error('return adoption');
        document.body.appendChild(node);
        if (node.parentNode !== document.body || nested.textContent !== 'kept')
            throw new Error('round trip lost subtree');
    "#,
    );
}

fn source_observer_follows_destination_mutations<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    run(
        &mut rt,
        r#"
        var node = document.createElement('div'), records = [];
        var observer = new MutationObserver(function(batch) {
            for (var i=0;i<batch.length;i++) records.push(batch[i]);
        });
        observer.observe(node, {attributes:true, attributeOldValue:true});
        child.document.body.appendChild(node);
        child.Element.prototype.setAttribute.call(node, 'data-value', 'one');
        child.Element.prototype.setAttribute.call(node, 'data-value', 'two');
    "#,
    );
    rt.run_microtasks();
    run(
        &mut rt,
        r#"
        if (records.length !== 2 || records[0].target !== node || records[1].target !== node ||
            records[0].oldValue !== null || records[1].oldValue !== 'one' ||
            !(records[0] instanceof MutationRecord)) throw new Error('observer lost adopted target');
        observer.disconnect();
        child.Element.prototype.setAttribute.call(node, 'data-value', 'three');
    "#,
    );
    rt.run_microtasks();
    run(
        &mut rt,
        "if (records.length !== 2) throw new Error('disconnect left destination registration');",
    );
}

fn source_range_tracks_destination_edits<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    run(
        &mut rt,
        r#"
        var node = document.createElement('div');
        node.textContent = 'abcd';
        var text = node.firstChild, range = document.createRange();
        range.setStart(text, 1); range.setEnd(text, 3);
        child.document.adoptNode(node);
        if (range.startContainer !== text || range.endContainer !== text || range.toString() !== 'bc')
            throw new Error('detached adoption changed range');
        child.document.body.appendChild(node);
        child.CharacterData.prototype.insertData.call(text, 0, 'X');
        if (range.startContainer !== text || range.startOffset !== 2 || range.endOffset !== 4 ||
            range.toString() !== 'bc') throw new Error('foreign mutation did not update live range');
        child.document.body.removeChild(node);
        if (!range.collapsed || range.startContainer !== child.document.body || range.startOffset !== 0)
            throw new Error('removal failed to relocate source range');
    "#,
    );
}

fn collection_tracks_owner_separately_from_wrapper_realm<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    run(
        &mut rt,
        "var retained = document.createElement('div'); retained.textContent='alive'; child.document.adoptNode(retained);",
    );
    let id = {
        let value = rt.eval("__nodeRawId(retained.__ref)").unwrap();
        NodeId::from_raw(rt.value_to_string(&value).unwrap().parse::<u64>().unwrap())
    };
    let realm = rt.frame_realms(MAIN_REALM)[0].1;
    let owner = rt.host_in_realm(realm).unwrap();
    assert!(
        !rt.host().borrow().dom.is_live(id),
        "node stayed in source store"
    );
    for _ in 0..3 {
        rt.collect_garbage();
    }
    assert!(
        owner.borrow().dom.is_live(id),
        "source wrapper did not pin current owner"
    );
    run(
        &mut rt,
        "if (retained.textContent !== 'alive' || retained.ownerDocument !== child.document) throw new Error('retained adopted node lost'); retained = null;",
    );
    for _ in 0..8 {
        rt.collect_garbage();
        if !owner.borrow().dom.is_live(id) {
            break;
        }
    }
    assert!(
        !owner.borrow().dom.is_live(id),
        "released wrapper left adopted detached node pinned"
    );
}

fn opaque_frame_access_stays_refused<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    run(
        &mut rt,
        "var foreign = document.createElement('iframe'); foreign.setAttribute('sandbox',''); foreign.srcdoc='<p>private</p>'; document.body.appendChild(foreign);",
    );
    rt.run_event_loop(20).unwrap();
    run(
        &mut rt,
        r#"
        var source = document.createElement('p'); source.textContent = 'source';
        document.body.appendChild(source);
        function denied(operation) {
            try { operation(); } catch (e) {
                if (e.name === 'SecurityError') return;
                throw e;
            }
            throw new Error('opaque-origin access admitted');
        }
        denied(function(){foreign.contentWindow.document.adoptNode(source);});
        denied(function(){foreign.contentWindow.__appendChild(source.__ref, source.__ref);});
        if (foreign.contentDocument !== null || source.parentNode !== document.body || source.textContent !== 'source')
            throw new Error('refusal changed source document');
    "#,
    );
}

fn mixed_creation_realms_share_detached_component_liveness<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    run(
        &mut rt,
        r#"
        var root = document.createElement('section');
        var sibling = document.createElement('b');
        root.marker = 'original-root'; sibling.marker = 'original-sibling';
        root.appendChild(sibling);
        child.document.adoptNode(root);
        var held = child.document.createElement('span');
        root.appendChild(held);
        var rootId = __nodeRawId(root.__ref);
        if (held.parentNode.marker !== 'original-root') throw new Error('before GC: root marker lost');
        if (held.parentNode.firstChild.marker !== 'original-sibling') throw new Error('before GC: sibling marker lost');
        if (!(held.parentNode instanceof HTMLElement)) throw new Error('before GC: root creation prototype lost');
        if (!(held instanceof child.HTMLElement)) throw new Error('before GC: held creation prototype lost');
        root = null; sibling = null;
    "#,
    );
    let root_id = {
        let value = rt.eval("rootId").unwrap();
        NodeId::from_raw(rt.value_to_string(&value).unwrap().parse::<u64>().unwrap())
    };
    let owner = rt.host_in_realm(rt.frame_realms(MAIN_REALM)[0].1).unwrap();
    // The mixed component must be grouped by its physical parent links, even
    // though its reflectors belong to different creation-realm inventories.
    let held_id = {
        let value = rt.eval("__nodeRawId(held.__ref)").unwrap();
        NodeId::from_raw(rt.value_to_string(&value).unwrap().parse::<u64>().unwrap())
    };
    assert_eq!(owner.borrow_mut().tree_root_of(held_id), Some(root_id));
    for _ in 0..3 {
        rt.collect_garbage();
    }
    run(
        &mut rt,
        r#"
        if (held.parentNode.marker !== 'original-root') throw new Error('after GC: root marker lost');
        if (held.parentNode.firstChild.marker !== 'original-sibling') throw new Error('after GC: sibling marker lost');
        if (!(held.parentNode instanceof HTMLElement)) throw new Error('after GC: root creation prototype lost');
        if (!(held instanceof child.HTMLElement)) throw new Error('after GC: held creation prototype lost');
        held = null;
    "#,
    );
    for _ in 0..8 {
        rt.collect_garbage();
        if !owner.borrow().dom.is_live(root_id) {
            break;
        }
    }
    assert!(
        !owner.borrow().dom.is_live(root_id),
        "mixed detached component remained rooted after release"
    );
}

fn fresh_classic_script_executes_in_destination_once<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    run(
        &mut rt,
        r#"
        var script = document.createElement('script');
        var identity = {}; script.expando = identity;
        script.textContent = "window.adoptionScriptRuns = (window.adoptionScriptRuns || 0) + 1;" +
            "window.adoptionScriptDocument = document; window.adoptionCurrentScript = document.currentScript;";
        // Deliberately borrow the source realm method: preparation must follow
        // the destination document even when the method closure belongs here.
        Node.prototype.appendChild.call(child.document.body, script);
        if (window.adoptionScriptRuns !== undefined || child.adoptionScriptRuns !== 1 ||
            child.adoptionScriptDocument !== child.document || child.adoptionCurrentScript !== script ||
            script.expando !== identity || script.ownerDocument !== child.document ||
            child.document.currentScript !== null)
            throw new Error('fresh script preparation used creation realm');
        document.body.appendChild(script);
        child.document.body.appendChild(script);
        if (window.adoptionScriptRuns !== undefined || child.adoptionScriptRuns !== 1 ||
            script.expando !== identity || child.adoptionCurrentScript !== script)
            throw new Error('already-started script ran again after round trip');
    "#,
    );
}

fn mutated_range_retains_then_releases_detached_text<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().unwrap();
    run(
        &mut rt,
        r#"
        var text = document.createTextNode('abcd');
        var heldRange = document.createRange();
        heldRange.setStart(text,1); heldRange.setEnd(text,3);
        text.insertData(0,'X');
        if (heldRange.startOffset !== 2 || heldRange.endOffset !== 4 || heldRange.toString() !== 'bc')
            throw new Error('mutation changed boundary semantics');
        var textId = __nodeRawId(text.__ref);
        text = null;
    "#,
    );
    let value = rt.eval("textId").unwrap();
    let id = NodeId::from_raw(rt.value_to_string(&value).unwrap().parse::<u64>().unwrap());
    drop(value);
    for _ in 0..3 {
        rt.collect_garbage();
    }
    assert!(
        rt.host().borrow().dom.is_live(id),
        "held Range lost its detached boundary node"
    );
    run(
        &mut rt,
        "if (heldRange.toString() !== 'bc') throw new Error('held boundary changed after collection'); heldRange = null;",
    );
    // WeakRef.deref keeps its target alive until a real job boundary. Boa's
    // empty drain does not finish a job, so enqueue one before testing release.
    run(&mut rt, "Promise.resolve().then(function() {});");
    rt.run_microtasks();
    for _ in 0..8 {
        rt.collect_garbage();
        if !rt.host().borrow().dom.is_live(id) {
            break;
        }
    }
    assert!(
        !rt.host().borrow().dom.is_live(id),
        "weak range index retained a mutated Range after release"
    );
}

/// The owner-resolved accessor's own contract: after adoption every native sink
/// reached from the *creation* realm answers from the arena that now stores the
/// node, and the creation arena no longer holds it. Before the accessor this was
/// a convention each sink had to remember.
fn native_reads_follow_the_owning_arena<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    run(
        &mut rt,
        r#"
        var node = document.createElement('section');
        node.id = 'moved';
        node.appendChild(document.createTextNode('body text'));
        child.document.body.appendChild(node);
        node.setAttribute('data-probe', 'set-from-source-realm');
        node.textContent = 'rewritten from the source realm';
        node.innerHTML = '<i id=deep>inner</i>';
    "#,
    );
    let id = {
        let host = rt.host().borrow();
        assert!(
            find_id(&host.dom, LayoutDom::document(&host.dom), "moved").is_none(),
            "the creation arena still holds the adopted node"
        );
        None::<NodeId>
    };
    assert!(id.is_none());
    let child = rt.frame_realms(MAIN_REALM)[0].1;
    let host = rt.host_in_realm(child).expect("child host");
    let host = host.borrow();
    let node = find_id(&host.dom, LayoutDom::document(&host.dom), "moved")
        .expect("the destination arena stores the adopted node");
    assert_eq!(
        host.dom
            .attribute(
                node,
                &layout_dom_api::Namespace::from(""),
                &layout_dom_api::LocalName::from("data-probe")
            )
            .map(str::to_string),
        Some("set-from-source-realm".to_string()),
        "the attribute sink wrote the owning arena"
    );
    assert!(
        find_id(&host.dom, node, "deep").is_some(),
        "the innerHTML sink parsed into the owning arena"
    );
}

fn find_id(dom: &genet_scripted_dom::ScriptedDom, node: NodeId, id: &str) -> Option<NodeId> {
    let matched = dom
        .attribute(
            node,
            &layout_dom_api::Namespace::from(""),
            &layout_dom_api::LocalName::from("id"),
        )
        .is_some_and(|value| value == id);
    if matched {
        return Some(node);
    }
    dom.dom_children(node)
        .collect::<Vec<_>>()
        .into_iter()
        .find_map(|child| find_id(dom, child, id))
}

macro_rules! backend {
    ($module:ident, $engine:ty) => {
        mod $module {
            #[test]
            fn mutated_range_collection() {
                super::mutated_range_retains_then_releases_detached_text::<$engine>();
            }
            #[test]
            fn adopted_classic_script() {
                super::fresh_classic_script_executes_in_destination_once::<$engine>();
            }
            #[test]
            fn mixed_component_collection() {
                super::mixed_creation_realms_share_detached_component_liveness::<$engine>();
            }
            #[test]
            fn automatic_insertion() {
                super::automatic_insertion_preserves_wrapper_and_routes_events::<$engine>();
            }
            #[test]
            fn explicit_adoption() {
                super::explicit_adoption_keeps_detached_identity::<$engine>();
            }
            #[test]
            fn mutation_observer() {
                super::source_observer_follows_destination_mutations::<$engine>();
            }
            #[test]
            fn live_range() {
                super::source_range_tracks_destination_edits::<$engine>();
            }
            #[test]
            fn collection() {
                super::collection_tracks_owner_separately_from_wrapper_realm::<$engine>();
            }
            #[test]
            fn owner_resolved_native_reads() {
                super::native_reads_follow_the_owning_arena::<$engine>();
            }
            #[test]
            fn opaque_origin() {
                super::opaque_frame_access_stays_refused::<$engine>();
            }
        }
    };
}
backend!(boa, script_engine_boa::BoaEngine);
#[cfg(target_pointer_width = "64")]
backend!(nova, script_engine_nova::NovaEngine);
