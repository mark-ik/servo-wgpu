// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Nova's half of the realm contract's named regression set. Every case here is
//! a twin of one in `script-engine-boa`'s `tests` module, asserted through the
//! engine-neutral trait rather than through Nova types, so the pair is a real
//! both-engine gate: the contract's point is that a child browsing context
//! behaves the same on either backend.
//!
//! See `design_docs/2026-09-08_realms_plan.md`.
#![cfg(target_pointer_width = "64")]

use std::cell::RefCell;
use std::rc::Rc;

use script_engine_api::{
    CallCx, MAIN_REALM, NativeFn, RealmError, RealmId, ScriptEngine, ScriptEngineLive,
};
use script_engine_nova::{NovaCallCx, NovaEngine, NovaValue};

fn text(engine: &mut NovaEngine, value: &NovaValue) -> String {
    engine.value_to_string(value).expect("value_to_string")
}

/// A realm has its own global object: a binding made in one is invisible in the
/// other, in both directions.
#[test]
fn realms_have_separate_globals() {
    let mut engine = NovaEngine::new().unwrap();
    assert!(engine.supports_realms());
    let child = engine.create_realm().unwrap();
    assert_ne!(child, MAIN_REALM);

    engine.eval("globalThis.here = 'parent'").unwrap();
    engine
        .eval_in_realm(child, "globalThis.here = 'child'")
        .unwrap();

    let a = engine.eval("here").unwrap();
    assert_eq!(text(&mut engine, &a), "parent");
    let b = engine.eval_in_realm(child, "here").unwrap();
    assert_eq!(text(&mut engine, &b), "child");
    let missing = engine.eval("typeof globalThis.childOnly").unwrap();
    assert_eq!(text(&mut engine, &missing), "undefined");
}

/// A realm has its own intrinsics: its `Object` is not the parent's, which is
/// what makes `instanceof` cross-realm-false and is the reason a realm — not
/// merely a fresh global — is the right unit for a browsing context.
#[test]
fn realms_have_separate_intrinsics() {
    let mut engine = NovaEngine::new().unwrap();
    let child = engine.create_realm().unwrap();
    let child_object = engine.eval_in_realm(child, "Object").unwrap();
    engine.set_global("childObject", &child_object).unwrap();
    let same = engine.eval("childObject === Object").unwrap();
    assert_eq!(text(&mut engine, &same), "false");
}

/// The whole point of one agent: a value made in the child realm is an
/// **ordinary reference** in the parent, with identity preserved across the
/// boundary in both directions. No clone, no wire, no marshalling.
#[test]
fn objects_cross_realms_with_identity() {
    let mut engine = NovaEngine::new().unwrap();
    let child = engine.create_realm().unwrap();
    engine
        .eval_in_realm(child, "globalThis.thing = { tag: 'child-object' }")
        .unwrap();
    let handle = engine.eval_in_realm(child, "thing").unwrap();

    engine.set_global("fromChild", &handle).unwrap();
    let tag = engine.eval("fromChild.tag").unwrap();
    assert_eq!(text(&mut engine, &tag), "child-object");
    // Mutating in the parent is visible in the child, which is only true of a
    // shared reference.
    engine.eval("fromChild.tag = 'touched-by-parent'").unwrap();
    let seen = engine.eval_in_realm(child, "thing.tag").unwrap();
    assert_eq!(text(&mut engine, &seen), "touched-by-parent");
    engine.set_global_in_realm(child, "back", &handle).unwrap();
    let identical = engine.eval_in_realm(child, "back === thing").unwrap();
    assert_eq!(text(&mut engine, &identical), "true");
}

/// `realm_global` hands back the child's actual global — the primitive
/// `contentWindow` is built from.
#[test]
fn realm_global_is_the_childs_own_global() {
    let mut engine = NovaEngine::new().unwrap();
    let child = engine.create_realm().unwrap();
    engine
        .eval_in_realm(child, "globalThis.marker = 41")
        .unwrap();
    let global = engine.realm_global(child).unwrap();
    engine.set_global("contentWindow", &global).unwrap();
    engine.eval("contentWindow.marker += 1").unwrap();
    let seen = engine.eval_in_realm(child, "marker").unwrap();
    assert_eq!(text(&mut engine, &seen), "42");
    let child_global_this = engine.eval_in_realm(child, "globalThis").unwrap();
    engine
        .set_global("childGlobalThis", &child_global_this)
        .unwrap();
    let same = engine.eval("contentWindow === childGlobalThis").unwrap();
    assert_eq!(text(&mut engine, &same), "true");
}

type Seen = RefCell<Vec<RealmId>>;

struct WhereAmI;
impl NativeFn<NovaEngine> for WhereAmI {
    fn call(cx: &mut NovaCallCx<'_>) -> Result<NovaValue, String> {
        let realm = cx.current_realm();
        let hits = cx
            .host_data()
            .and_then(|d| d.downcast::<Seen>().ok())
            .map(|seen| {
                seen.borrow_mut().push(realm);
                seen.borrow().len()
            })
            .unwrap_or(0);
        cx.make_string(&format!("realm={realm} hits={hits}"))
    }
}

/// A native function installed per realm reports that realm from inside the
/// call, and reaches that realm's own host data. This is the per-realm host
/// surface in miniature: one `NativeFn` impl, two realms, two answers, and the
/// callback never learns that realms exist.
#[test]
fn native_fn_sees_its_own_realm_and_host_data() {
    let mut engine = NovaEngine::new().unwrap();
    let parent_state: Rc<Seen> = Rc::new(RefCell::new(Vec::new()));
    let child_state: Rc<Seen> = Rc::new(RefCell::new(Vec::new()));
    engine.set_host_data(parent_state.clone());
    engine.set_function::<WhereAmI>("whereAmI", 0).unwrap();

    let child = engine.create_realm().unwrap();
    engine
        .set_host_data_in_realm(child, child_state.clone())
        .unwrap();
    engine
        .set_function_in_realm::<WhereAmI>(child, "whereAmI", 0)
        .unwrap();

    let a = engine.eval("whereAmI()").unwrap();
    assert_eq!(text(&mut engine, &a), "realm=0 hits=1");
    let b = engine.eval_in_realm(child, "whereAmI()").unwrap();
    assert_eq!(text(&mut engine, &b), format!("realm={child} hits=1"));
    assert_eq!(*parent_state.borrow(), vec![MAIN_REALM]);
    assert_eq!(*child_state.borrow(), vec![child]);
}

/// A reflector handed into the child realm is recoverable there: the reflector
/// bridge is agent-wide, so a node crossing realms is still the same node to the
/// host.
#[test]
fn reflectors_cross_realms() {
    let mut engine = NovaEngine::new().unwrap();
    let child = engine.create_realm().unwrap();
    let reflector = ScriptEngineLive::make_reflector(&mut engine, 0x99).unwrap();
    engine
        .set_global_in_realm(child, "node", &reflector)
        .unwrap();
    let back = engine.eval_in_realm(child, "node").unwrap();
    assert_eq!(engine.reflector_data(&back), Some(0x99));
}

/// Refusals are stated, not approximated.
#[test]
fn realm_refusals_are_exact() {
    let mut engine = NovaEngine::new().unwrap();
    // `NovaValue` is not `Debug` (it is a rooted heap handle), so the Ok side
    // cannot be unwrapped for a message; match on the error instead.
    match engine.eval_in_realm(4242, "1") {
        Err(e) => assert_eq!(e, RealmError::NoSuchRealm(4242)),
        Ok(_) => panic!("eval_in_realm on an unknown id must refuse"),
    }
    assert!(matches!(
        engine.discard_realm(MAIN_REALM),
        Err(RealmError::Refused(_))
    ));
    let child = engine.create_realm().unwrap();
    assert_eq!(engine.discard_realm(child), Ok(()));
    assert_eq!(
        engine.discard_realm(child).unwrap_err(),
        RealmError::NoSuchRealm(child)
    );
}

#[test]
fn equal_raw_reflector_ids_are_isolated_and_rooted_per_realm() {
    struct Reflect;
    impl<E: ScriptEngine> NativeFn<E> for Reflect {
        fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
            cx.reflector_for(7)
        }
    }
    let mut engine = NovaEngine::new().unwrap();
    let child = engine.create_realm().unwrap();
    engine.set_function::<Reflect>("reflect", 0).unwrap();
    engine
        .set_function_in_realm::<Reflect>(child, "reflect", 0)
        .unwrap();
    engine
        .eval("globalThis.node = reflect(); node.marker = 'parent'")
        .unwrap();
    engine
        .eval_in_realm(child, "globalThis.node = reflect(); node.marker = 'child'")
        .unwrap();
    let child_node = engine.eval_in_realm(child, "node").unwrap();
    engine.set_global("childNode", &child_node).unwrap();
    let result = engine.eval("node !== childNode && reflect() === node && node.marker === 'parent' && childNode.marker === 'child'").unwrap();
    assert_eq!(engine.value_to_string(&result).unwrap(), "true");
    engine.root_reflectors(&[7]);
    engine.root_reflectors_in_realm(child, &[7]).unwrap();
    engine.unroot_reflectors(&[7]);
    assert_eq!(engine.rooted_reflector_count(), 0);
    assert_eq!(engine.rooted_reflector_count_in_realm(child).unwrap(), 1);
    engine.force_gc();
    assert!(
        engine
            .drain_dead_reflectors_in_realm(child)
            .unwrap()
            .is_empty()
    );
    assert_eq!(engine.minted_reflectors_in_realm(child).unwrap(), vec![7]);
    engine.unroot_reflectors_in_realm(child, &[7]).unwrap();
    assert_eq!(engine.rooted_reflector_count_in_realm(child).unwrap(), 0);
    assert_eq!(
        engine.minted_reflectors_in_realm(99999),
        Err(RealmError::NoSuchRealm(99999))
    );
}

#[test]
fn child_host_promises_settle_without_parent_token_collisions() {
    use std::{cell::RefCell, rc::Rc};
    struct Deferred;
    impl<E: ScriptEngine> NativeFn<E> for Deferred {
        fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
            let (value, token) = cx.new_host_promise()?;
            let data = cx.host_data().unwrap();
            data.downcast_ref::<RefCell<Vec<u64>>>()
                .unwrap()
                .borrow_mut()
                .push(token);
            Ok(value)
        }
    }
    let mut engine = NovaEngine::new().unwrap();
    let child = engine.create_realm().unwrap();
    let tokens = Rc::new(RefCell::new(Vec::<u64>::new()));
    engine.set_host_data(tokens.clone());
    engine
        .set_host_data_in_realm(child, tokens.clone())
        .unwrap();
    engine.set_function::<Deferred>("deferred", 0).unwrap();
    engine
        .set_function_in_realm::<Deferred>(child, "deferred", 0)
        .unwrap();
    engine
        .eval("globalThis.answer = 'pending'; deferred().then(v => answer = v)")
        .unwrap();
    engine
        .eval_in_realm(
            child,
            "globalThis.answer = 'pending'; deferred().then(v => answer = v)",
        )
        .unwrap();
    let ids = tokens.borrow().clone();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    let value = engine.eval("('child settled')").unwrap();
    assert_eq!(engine.value_to_string(&value).unwrap(), "child settled");
    engine.settle_host_promise(ids[1], Ok(&value)).unwrap();
    engine.pump_microtasks();
    let value = engine.eval("answer").unwrap();
    assert_eq!(engine.value_to_string(&value).unwrap(), "pending");
    let value = engine.eval_in_realm(child, "answer").unwrap();
    assert_eq!(engine.value_to_string(&value).unwrap(), "child settled");
    let value = engine.eval("('parent settled')").unwrap();
    assert_eq!(engine.value_to_string(&value).unwrap(), "parent settled");
    engine.settle_host_promise(ids[0], Ok(&value)).unwrap();
    engine.pump_microtasks();
    let value = engine.eval("answer").unwrap();
    assert_eq!(engine.value_to_string(&value).unwrap(), "parent settled");
}

#[test]
fn uninitialized_child_never_inherits_parent_host_data() {
    use std::rc::Rc;
    struct Check;
    impl<E: ScriptEngine> NativeFn<E> for Check {
        fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
            let found = cx.host_data().is_some();
            cx.make_string(if found { "present" } else { "absent" })
        }
    }
    let mut engine = NovaEngine::new().unwrap();
    engine.set_host_data(Rc::new(17_u32));
    let child = engine.create_realm().unwrap();
    engine
        .set_function_in_realm::<Check>(child, "check", 0)
        .unwrap();
    let value = engine.eval_in_realm(child, "check()").unwrap();
    assert_eq!(engine.value_to_string(&value).unwrap(), "absent");
}

#[test]
fn discarding_realm_does_not_revoke_retained_functions() {
    struct Marker;
    impl<E: ScriptEngine> NativeFn<E> for Marker {
        fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
            let id = cx.current_realm();
            cx.make_string(&id.to_string())
        }
    }
    let mut engine = NovaEngine::new().unwrap();
    let child = engine.create_realm().unwrap();
    engine
        .set_function_in_realm::<Marker>(child, "marker", 0)
        .unwrap();
    let function = engine.eval_in_realm(child, "() => marker()").unwrap();
    engine.set_global("retained", &function).unwrap();
    engine.discard_realm(child).unwrap();
    engine.force_gc();
    let value = engine.eval("retained()").unwrap();
    assert_eq!(engine.value_to_string(&value).unwrap(), child.to_string());
}

#[test]
fn snapshot_refuses_multiple_realms() {
    let mut engine = NovaEngine::new().unwrap();
    engine.create_realm().unwrap();
    assert_eq!(
        engine.snapshot_clone().err().as_deref(),
        Some("cannot snapshot clone NovaEngine with multiple realms")
    );
}

#[test]
fn native_callback_creates_and_initializes_child_synchronously() {
    use std::{cell::RefCell, rc::Rc};
    struct Who;
    impl<E: ScriptEngine> NativeFn<E> for Who {
        fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
            let realm = cx.current_realm();
            cx.make_string(&realm.to_string())
        }
    }
    struct Spawn;
    impl<E: ScriptEngine> NativeFn<E> for Spawn {
        fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
            let parent = cx.current_realm();
            let data = cx.host_data().unwrap();
            let records = data.downcast_ref::<RefCell<Vec<RealmId>>>().unwrap();
            let mut global = None;
            let id = E::create_realm_from_call(cx, data.clone(), |child| {
                E::set_function_from_call::<Who>(child, "who", 0)?;
                E::eval_from_call(
                    child,
                    "globalThis.childRealm = who(); globalThis.obj = { answer: 42 }",
                )?;
                global = Some(E::eval_from_call(child, "globalThis")?);
                Ok(())
            })
            .unwrap();
            records.borrow_mut().push(id);
            assert_eq!(cx.current_realm(), parent);
            Ok(global.unwrap())
        }
    }
    let mut engine = NovaEngine::new().unwrap();
    let records = Rc::new(RefCell::new(Vec::<RealmId>::new()));
    engine.set_host_data(records.clone());
    engine.set_function::<Spawn>("spawn", 0).unwrap();
    engine
        .eval("globalThis.child = spawn(); child.obj.answer += 1")
        .unwrap();
    let id = records.borrow()[0];
    let value = engine
        .eval_in_realm(id, "obj.answer === 43 && childRealm === who()")
        .unwrap();
    assert_eq!(engine.value_to_string(&value).unwrap(), "true");
    let value = engine.eval("typeof who").unwrap();
    assert_eq!(engine.value_to_string(&value).unwrap(), "undefined");
    let global = engine.realm_global(id).unwrap();
    engine.set_global("sameChild", &global).unwrap();
    let value = engine.eval("sameChild === child").unwrap();
    assert_eq!(engine.value_to_string(&value).unwrap(), "true");
}

#[test]
fn direct_native_child_method_observes_caller_and_receiver() {
    struct Method;
    impl<E: ScriptEngine> NativeFn<E> for Method {
        fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
            let callee = cx.current_realm();
            let caller = cx.caller_realm();
            let receiver = cx.this_value();
            E::set_global_from_call(cx, "received", &receiver).unwrap();
            let function = E::eval_from_call(cx, "(function(value) { return value; })").unwrap();
            let undefined = cx.undefined();
            let same = E::call_from_call(cx, &function, &undefined, &[receiver]).unwrap();
            E::set_global_from_call(cx, "receivedCall", &same).unwrap();
            E::eval_in_realm_from_call(cx, caller, "globalThis.calledBack = 'parent'").unwrap();
            assert_eq!(cx.current_realm(), callee);
            let _ = E::eval_in_realm_from_call(cx, caller, "throw new Error('restore')")
                .err()
                .expect("target throw");
            assert_eq!(cx.current_realm(), callee);
            cx.make_string(&format!("{callee}:{caller}"))
        }
    }
    let mut engine = NovaEngine::new().unwrap();
    let child = engine.create_realm().unwrap();
    engine
        .set_function_in_realm::<Method>(child, "method", 0)
        .unwrap();
    let global = engine.realm_global(child).unwrap();
    engine.set_global("child", &global).unwrap();
    let value = engine.eval("child.method()").unwrap();
    assert_eq!(
        engine.value_to_string(&value).unwrap(),
        format!("{child}:0")
    );
    let value = engine.eval("child.received === child && child.receivedCall === child && calledBack === 'parent' && typeof child.calledBack === 'undefined'").unwrap();
    assert_eq!(engine.value_to_string(&value).unwrap(), "true");
}

#[test]
fn host_installed_globals_are_writable_configurable_and_deletable() {
    struct Install;
    impl<E: ScriptEngine> NativeFn<E> for Install {
        fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
            let value = cx.arg(0);
            E::set_global_from_call(cx, "callbackValue", &value)
                .map_err(|error| cx.error(&error.to_string()))?;
            Ok(cx.undefined())
        }
    }
    let mut engine = NovaEngine::new().unwrap();
    let child = engine.create_realm().unwrap();
    let value = engine.eval("42").unwrap();
    engine.set_global("installedValue", &value).unwrap();
    engine
        .set_global_in_realm(child, "installedValue", &value)
        .unwrap();
    engine.set_function::<Install>("installValue", 1).unwrap();
    engine
        .set_function_in_realm::<Install>(child, "installValue", 1)
        .unwrap();
    let script = "installValue(42); ['installedValue','callbackValue'].every(function(name){var d=Object.getOwnPropertyDescriptor(globalThis,name); var flags=d.writable && d.enumerable && d.configurable; globalThis[name]=99; return flags && globalThis[name]===99 && delete globalThis[name] && !(name in globalThis);})";
    let value = engine.eval(script).unwrap();
    assert_eq!(engine.value_to_string(&value).unwrap(), "true");
    let value = engine.eval_in_realm(child, script).unwrap();
    assert_eq!(engine.value_to_string(&value).unwrap(), "true");
}
