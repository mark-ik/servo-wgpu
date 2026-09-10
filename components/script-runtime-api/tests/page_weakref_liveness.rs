// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! Page weak reachability must agree with native DOM retention, including when
//! a wrapper's creation realm differs from the node's current storage owner.
use genet_scripted_dom::NodeId;
use layout_dom_api::LayoutDom;
use script_engine_api::{MAIN_REALM, ScriptEngine};
use script_runtime_api::{NoScriptLoader, Runtime};

fn eval_text<E: ScriptEngine>(rt: &mut Runtime<E>, source: &str) -> String {
    let value = rt.eval(source).expect(source);
    rt.value_to_string(&value).unwrap()
}

fn finish_job<E: ScriptEngine>(rt: &mut Runtime<E>) {
    // Enqueue a real Promise job. Merely evaluating another script, or draining
    // an empty queue, is not evidence that the backend ran ClearKeptObjects.
    rt.eval("Promise.resolve().then(function(){globalThis.weakJobTurns++;});")
        .unwrap();
    rt.run_microtasks();
}

fn page_weakref_and_native_liveness_agree<E: ScriptEngine>(adopt: bool) {
    let mut rt = Runtime::<E>::new().unwrap();
    rt.set_base_url("https://parent.test/").unwrap();
    rt.parse_document_interleaved("<body><iframe id=frame></iframe></body>", &NoScriptLoader);
    rt.run_event_loop(20).unwrap();
    rt.eval("var child=document.getElementById('frame').contentWindow; globalThis.weakJobTurns=0;")
        .unwrap();
    rt.eval(
        r#"
        var held=document.createElement('div');
        held.textContent='native payload';
        held.marker='original wrapper';
        var weak;
        var observed=null;
    "#,
    )
    .unwrap();
    let raw = eval_text(&mut rt, "__nodeRawId(held.__ref)");
    let id = NodeId::from_raw(raw.parse().unwrap());
    let owner = if adopt {
        rt.eval("child.document.adoptNode(held);").unwrap();
        rt.host_in_realm(rt.frame_realms(MAIN_REALM)[0].1).unwrap()
    } else {
        rt.host().clone()
    };

    // A Nova host eval is itself a job boundary (GcAgent::run_in_realm
    // performs ClearKeptObjects on return). Check construction/drop/deref within
    // one evaluation; do not assume later host calls are the same JS job.
    rt.eval("weak=new WeakRef(held); held=null; if(weak.deref().marker!=='original wrapper') throw new Error('construction job lost wrapper');")
        .unwrap();

    let mut collected = false;
    for _ in 0..12 {
        finish_job(&mut rt);
        rt.collect_garbage();
        // Inspect only an expando first: a DOM getter must not hide an earlier
        // accounting error by reminting a reflector or throwing on a dead ID.
        let state = eval_text(
            &mut rt,
            "(function(){observed=weak.deref();return observed===undefined?'dead':observed.marker;})()",
        );
        if state == "dead" {
            // Some collectors notify native weak caches one pass later.
            finish_job(&mut rt);
            rt.collect_garbage();
            assert!(
                !owner.borrow().dom.is_live(id),
                "dead page wrapper left native node pinned"
            );
            collected = true;
            break;
        }
        assert_eq!(
            state, "original wrapper",
            "page weak reference returned a replacement wrapper"
        );
        assert!(
            owner.borrow().dom.is_live(id),
            "page WeakRef survived after native node was reaped"
        );
        assert_eq!(eval_text(&mut rt, "observed.textContent"), "native payload");
        assert_eq!(eval_text(&mut rt, "__nodeRawId(observed.__ref)"), raw);
        // The successful deref was promoted to an explicit authored root, so
        // it remains valid across subsequent host jobs and forced collection.
        rt.collect_garbage();
        assert!(
            owner.borrow().dom.is_live(id),
            "promoted page weak reference lost native node"
        );
        assert_eq!(eval_text(&mut rt, "observed.marker"), "original wrapper");
        rt.eval("observed=null;").unwrap();
    }
    assert!(
        collected,
        "weak-only detached node never reclaimed across twelve completed jobs"
    );
    assert!(eval_text(&mut rt, "weakJobTurns").parse::<u32>().unwrap() > 0);
}

macro_rules! backend {
    ($name:ident, $engine:ty) => {
        mod $name {
            #[test]
            fn local_page_weakref_matches_native_liveness() {
                super::page_weakref_and_native_liveness_agree::<$engine>(false);
            }
            #[test]
            fn adopted_page_weakref_matches_native_liveness() {
                super::page_weakref_and_native_liveness_agree::<$engine>(true);
            }
        }
    };
}
backend!(boa, script_engine_boa::BoaEngine);
#[cfg(target_pointer_width = "64")]
backend!(nova, script_engine_nova::NovaEngine);
