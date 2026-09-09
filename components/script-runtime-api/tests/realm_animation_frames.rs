// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use script_engine_api::ScriptEngine;
use script_runtime_api::{HostState, Runtime};

fn child_animation_frames_share_agent_drive<E: ScriptEngine>() {
    let mut runtime = Runtime::<E>::new().expect("runtime");
    let realm = runtime
        .create_child_realm(HostState::default())
        .expect("child realm");
    let order = runtime.eval("globalThis.order = []; order").unwrap();
    runtime
        .engine_mut()
        .set_global_in_realm(realm, "order", &order)
        .unwrap();
    runtime.eval_in_realm(realm, "requestAnimationFrame(function(now){order.push('child:'+now+':'+(this===window));Promise.resolve().then(function(){order.push('child-microtask');});requestAnimationFrame(function(now){order.push('next:'+now);});}); var cancelled=requestAnimationFrame(function(){order.push('cancelled');});cancelAnimationFrame(cancelled);").unwrap();
    assert!(
        runtime.has_animation_frame_callbacks(),
        "child alone keeps host rendering awake"
    );
    runtime.eval("requestAnimationFrame(function(now){order.push('parent:'+now);Promise.resolve().then(function(){order.push('parent-microtask');});});").unwrap();
    assert_eq!(runtime.run_animation_frame_callbacks(16.0).unwrap(), 2);
    let value = runtime.eval("order.join(',')").unwrap();
    assert_eq!(
        runtime.value_to_string(&value).unwrap(),
        "parent:16,parent-microtask,child:16:true,child-microtask"
    );
    assert!(
        runtime.has_animation_frame_callbacks(),
        "child's reentrant request is the next frame"
    );
    assert_eq!(runtime.run_animation_frame_callbacks(32.0).unwrap(), 1);
    let value = runtime.eval("order[order.length-1]").unwrap();
    assert_eq!(runtime.value_to_string(&value).unwrap(), "next:32");
    assert!(!runtime.has_animation_frame_callbacks());
    let realms: Vec<_> = runtime
        .scheduler_trace()
        .iter()
        .filter(|event| event.boundary == "animation_frame_realm")
        .map(|event| event.detail.clone().unwrap())
        .collect();
    assert_eq!(
        realms,
        vec![
            "realm=0;handle=1".to_owned(),
            format!("realm={realm};handle=1"),
            format!("realm={realm};handle=3")
        ]
    );
}

#[test]
fn child_animation_frames_share_agent_drive_on_boa() {
    child_animation_frames_share_agent_drive::<script_engine_boa::BoaEngine>();
}
#[cfg(target_pointer_width = "64")]
#[test]
fn child_animation_frames_share_agent_drive_on_nova() {
    child_animation_frames_share_agent_drive::<script_engine_nova::NovaEngine>();
}

fn child_animation_callbacks_stay_private<E: ScriptEngine>() {
    let mut runtime = Runtime::<E>::new().expect("runtime");
    runtime.eval(r#"
        globalThis.stolenChild = null;
        var originalPush = Array.prototype.push, originalMap = Array.prototype.map, originalApply = Reflect.apply;
        Array.prototype.push = function(value) {
            if (value && value.callbacks && value.global) stolenChild = value.global;
            return originalApply(originalPush, this, arguments);
        };
        Array.prototype.map = function() {
            for (var i=0; i<this.length; i++) {
                var value=this[i];
                if (value && value.callbacks && value.global) stolenChild=value.global;
            }
            return originalApply(originalMap, this, arguments);
        };
    "#).unwrap();
    let realm = runtime
        .create_child_realm(HostState::default())
        .expect("child realm");
    runtime
        .eval_in_realm(
            realm,
            "globalThis.childRan=0; requestAnimationFrame(function(){childRan++;});",
        )
        .unwrap();
    assert!(runtime.has_animation_frame_callbacks());
    assert_eq!(runtime.run_animation_frame_callbacks(16.0).unwrap(), 1);
    let value = runtime.eval("stolenChild === null").unwrap();
    assert_eq!(
        runtime.value_to_string(&value).unwrap(),
        "true",
        "foreign callback records never reach authored prototype methods"
    );
    let value = runtime.eval_in_realm(realm, "childRan").unwrap();
    assert_eq!(runtime.value_to_string(&value).unwrap(), "1");
}

#[test]
fn child_animation_callbacks_stay_private_on_boa() {
    child_animation_callbacks_stay_private::<script_engine_boa::BoaEngine>();
}
#[cfg(target_pointer_width = "64")]
#[test]
fn child_animation_callbacks_stay_private_on_nova() {
    child_animation_callbacks_stay_private::<script_engine_nova::NovaEngine>();
}
