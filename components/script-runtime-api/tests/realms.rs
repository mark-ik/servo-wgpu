// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use script_engine_api::{MAIN_REALM, ScriptEngine};
use script_runtime_api::{FetchHandler, FetchOutcome, FetchRequest, HostState, Runtime};

fn child_host() -> HostState {
    let mut host = HostState::default();
    host.dom = genet_scripted_dom::ScriptedDom::from_serialized_document(
        "<body><p id=child>child</p></body>",
    );
    host
}

fn read<E: ScriptEngine>(rt: &mut Runtime<E>, realm: u32, source: &str) -> String {
    let value = rt.eval_in_realm(realm, source).expect("realm eval");
    rt.value_to_string(&value).expect("value")
}

fn separate_documents<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    let realm = rt.create_child_realm(child_host()).expect("child");
    assert_eq!(
        read(&mut rt, MAIN_REALM, "typeof __agentTimers"),
        "undefined"
    );
    assert_eq!(read(&mut rt, realm, "typeof __agentTimers"), "undefined");
    assert_eq!(
        read(
            &mut rt,
            MAIN_REALM,
            "document.getElementById('child') === null"
        ),
        "true"
    );
    assert_eq!(
        read(
            &mut rt,
            realm,
            "document.getElementById('child').textContent"
        ),
        "child"
    );
    assert_eq!(
        read(
            &mut rt,
            realm,
            "document.getElementById('child') instanceof HTMLParagraphElement"
        ),
        "true"
    );
    let global = rt.engine_mut().realm_global(realm).expect("global");
    rt.engine_mut()
        .set_global("child", &global)
        .expect("set child");
    assert_eq!(
        read(
            &mut rt,
            MAIN_REALM,
            "child.Object !== Object && child.document !== document"
        ),
        "true"
    );
    rt.eval_in_realm(
        realm,
        "globalThis.held = document.getElementById('child'); held.expando = 37;",
    )
    .unwrap();
    assert_eq!(
        read(
            &mut rt,
            MAIN_REALM,
            "child.document.getElementById('child') === child.held"
        ),
        "true"
    );
    rt.collect_garbage();
    assert_eq!(
        read(
            &mut rt,
            MAIN_REALM,
            "child.document.getElementById('child').expando"
        ),
        "37"
    );
}

fn shared_timer_order<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    let realm = rt.create_child_realm(child_host()).expect("child");
    let log = rt.eval("globalThis.order=[]; order").unwrap();
    rt.engine_mut()
        .set_global_in_realm(realm, "order", &log)
        .unwrap();
    rt.eval("setTimeout(function(){order.push('parent');},0)")
        .unwrap();
    rt.eval_in_realm(realm, "setTimeout(function(){order.push(document.getElementById('child').textContent);Promise.resolve().then(function(){order.push('microtask');});},0)").unwrap();
    rt.eval("setTimeout(function(){order.push('last');},0)")
        .unwrap();
    assert_eq!(rt.run_timers(8, 0.0), 3);
    assert_eq!(
        read(&mut rt, MAIN_REALM, "order.join(',')"),
        "parent,child,microtask,last"
    );
    let realm_marks: Vec<_> = rt
        .scheduler_trace()
        .iter()
        .filter(|e| e.boundary == "timer_realm")
        .map(|e| e.detail.clone().unwrap())
        .collect();
    assert_eq!(
        realm_marks,
        vec![
            "realm=0;sequence=1".to_string(),
            format!("realm={realm};sequence=2"),
            "realm=0;sequence=3".to_string()
        ]
    );
}

struct Deferred;
impl FetchHandler for Deferred {
    fn start(&self, _: u64, _: FetchRequest) -> Option<FetchOutcome> {
        None
    }
    fn fetch(&self, _: FetchRequest) -> FetchOutcome {
        FetchOutcome::network_error()
    }
}
fn child_fetch_completion<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.set_fetch_handler(Box::new(Deferred));
    let realm = rt.create_child_realm(child_host()).expect("child");
    rt.eval_in_realm(
        realm,
        "fetch('https://example.test/').catch(function(){console.log('child failure');})",
    )
    .unwrap();
    assert_eq!(rt.pending_fetches(), 1);
    assert_eq!(rt.pending_fetches_in_realm(MAIN_REALM).unwrap(), 0);
    let child_id: u64 = read(&mut rt, realm, "Object.keys(__pending)[0]")
        .parse()
        .unwrap();
    rt.eval(
        "fetch('https://example.test/parent').catch(function(){console.log('parent failure');})",
    )
    .unwrap();
    assert_eq!(rt.pending_fetches(), 2);
    rt.fail_fetch(child_id, "test complete");
    assert_eq!(rt.pending_fetches(), 1);
    assert!(rt.host().borrow().console.is_empty());
    rt.fail_all_pending("parent complete");
    assert_eq!(rt.pending_fetches(), 0);
    assert_eq!(
        rt.host_in_realm(realm).unwrap().borrow().console,
        vec!["child failure"]
    );
}

macro_rules! tests {
    ($engine:ty, $module:ident) => {
        mod $module {
            #[test]
            fn separate_documents() {
                super::separate_documents::<$engine>();
            }
            #[test]
            fn shared_timer_order() {
                super::shared_timer_order::<$engine>();
            }
            #[test]
            fn child_fetch_completion() {
                super::child_fetch_completion::<$engine>();
            }
        }
    };
}
tests!(script_engine_boa::BoaEngine, boa);
#[cfg(target_pointer_width = "64")]
tests!(script_engine_nova::NovaEngine, nova);
