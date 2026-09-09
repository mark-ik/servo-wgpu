// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Clone records cross the engine's actual realm boundary without a JSON wire.
//! Serialization runs in the sender; reconstruction uses recipient intrinsics.

use script_engine_api::ScriptEngine;
use script_runtime_api::{HostState, Runtime};

fn with_child<E: ScriptEngine>() -> Runtime<E> {
    let mut runtime = Runtime::<E>::new().expect("runtime");
    let child = runtime
        .create_child_realm(HostState::default())
        .expect("child surface");
    let global = runtime
        .engine_mut()
        .realm_global(child)
        .expect("child global");
    runtime
        .engine_mut()
        .set_global("child", &global)
        .expect("share global");
    runtime
}

fn check<E: ScriptEngine>(runtime: &mut Runtime<E>, source: &str) {
    let result = runtime.eval(source).expect("clone regression");
    assert_eq!(runtime.value_to_string(&result).expect("result"), "true");
}

fn cycles_and_recipient_intrinsics<E: ScriptEngine>() {
    let mut runtime = with_child::<E>();
    check(
        &mut runtime,
        r#"
        var shared = { value: 7 };
        var input = { first: shared, second: shared, array: [shared], map: new Map() };
        input.self = input;
        input.map.set(shared, input);
        var record = __scSerializeRecord(input);
        var copy = child.__scDeserializeRecord(record);
        var checks = {
            distinct: copy !== input, cycle: copy.self === copy, alias: copy.first === copy.second,
            arrayAlias: copy.array[0] === copy.first, mapAlias: copy.map.get(copy.first) === copy,
            objectPrototype: Object.getPrototypeOf(copy) === child.Object.prototype,
            nestedPrototype: Object.getPrototypeOf(copy.first) === child.Object.prototype,
            arrayPrototype: Object.getPrototypeOf(copy.array) === child.Array.prototype,
            mapPrototype: Object.getPrototypeOf(copy.map) === child.Map.prototype,
            foreignObjectBrand: !(copy instanceof Object), foreignArrayBrand: !(copy.array instanceof Array)
        };
        var failed = Object.keys(checks).filter(function(key) { return !checks[key]; });
        if (failed.length) throw new Error(failed.join(','));
        true
    "#,
    );
}

fn buffers_preserve_aliases_and_bytes<E: ScriptEngine>() {
    let mut runtime = with_child::<E>();
    check(
        &mut runtime,
        r#"
        var buffer = new ArrayBuffer(8);
        var bytes = new Uint8Array(buffer);
        bytes.set([3, 5, 8, 13, 21, 34, 55, 89]);
        var input = { buffer: buffer, bytes: bytes, slice: new Uint8Array(buffer, 2, 3), view: new DataView(buffer, 1, 4) };
        var copy = child.__scDeserializeRecord(__scSerializeRecord(input));
        copy.bytes[0] = 99;
        var checks = {
            distinct: copy.buffer !== buffer, independent: bytes[0] === 3,
            byteAlias: copy.bytes.buffer === copy.buffer, sliceAlias: copy.slice.buffer === copy.buffer,
            viewAlias: copy.view.buffer === copy.buffer, sliceOffset: copy.slice.byteOffset === 2,
            sliceLength: copy.slice.length === 3, sliceByte: copy.slice[2] === 21,
            viewOffset: copy.view.byteOffset === 1, viewLength: copy.view.byteLength === 4,
            viewByte: copy.view.getUint8(0) === 5,
            bufferPrototype: Object.getPrototypeOf(copy.buffer) === child.ArrayBuffer.prototype,
            bytesPrototype: Object.getPrototypeOf(copy.bytes) === child.Uint8Array.prototype,
            viewPrototype: Object.getPrototypeOf(copy.view) === child.DataView.prototype
        };
        var failed = Object.keys(checks).filter(function(key) { return !checks[key]; });
        if (failed.length) throw new Error(failed.join(','));
        true
    "#,
    );
}

fn sender_rejects_platform_objects<E: ScriptEngine>() {
    let mut runtime = with_child::<E>();
    check(
        &mut runtime,
        r#"
        var errors = [];
        try { child.__scDeserializeRecord(__scSerializeRecord(document.createElement('div'))); }
        catch (error) { errors.push(error.name); }
        try { __scDeserializeRecord(child.__scSerializeRecord(child.document.createElement('span'))); }
        catch (error) { errors.push(error.name); }
        errors.join(',') === 'DataCloneError,DataCloneError'
    "#,
    );
}

fn records_are_detached_and_worker_wire_stays_compatible<E: ScriptEngine>() {
    let mut runtime = with_child::<E>();
    check(
        &mut runtime,
        r#"
        var input = { value: 1, bytes: new Uint8Array([2, 3]) };
        input.self = input;
        var record = __scSerializeRecord(input);
        input.value = 9;
        input.bytes[0] = 8;
        var copy = child.__scDeserializeRecord(record);
        var wireCopy = child.__scDeserialize(__scSerialize(input));
        copy.value === 1 && copy.bytes[0] === 2 && copy.self === copy &&
        wireCopy.value === 9 && wireCopy.bytes[0] === 8 && wireCopy.self === wireCopy &&
        Object.getPrototypeOf(wireCopy) === child.Object.prototype
    "#,
    );
}

fn exposed_records_carry_no_source_prototypes<E: ScriptEngine>() {
    let mut runtime = with_child::<E>();
    check(
        &mut runtime,
        r#"
        var channel = new MessageChannel();
        var input = { array: [1, { value: 2 }], bytes: new Uint8Array([3, 4]), port: channel.port1 };
        input.self = input;
        var record = __scSerializeRecord(input, [channel.port1]);
        child.inspect = child.Function('record', `
            function inspect(value) {
                if (value === null || typeof value !== 'object') return typeof value !== 'function';
                if (Object.getPrototypeOf(value) !== null || !Object.isFrozen(value)) return false;
                if (value.constructor !== undefined || value.__proto__ !== undefined) return false;
                var names = Reflect.ownKeys(value);
                for (var i = 0; i < names.length; i++) {
                    var descriptor = Object.getOwnPropertyDescriptor(value, names[i]);
                    if (!Object.prototype.hasOwnProperty.call(descriptor, 'value') || !inspect(descriptor.value)) return false;
                }
                return true;
            }
            return inspect(record) && Array.isArray(record.h);
        `);
        child.inspect(record) && JSON.parse(JSON.stringify(record)).h.length > 0
    "#,
    );
}

fn record_hardening_uses_captured_intrinsics<E: ScriptEngine>() {
    let mut runtime = with_child::<E>();
    check(
        &mut runtime,
        r#"
        var originalCreate = Object.create, originalSet = Object.setPrototypeOf;
        var originalFreeze = Object.freeze, originalKeys = Reflect.ownKeys;
        var calls = 0;
        Object.create = Object.setPrototypeOf = Object.freeze = Reflect.ownKeys = function() { calls++; throw Error('replaced intrinsic'); };
        var record;
        try { record = __scSerializeRecord({ value: [7] }); }
        finally {
            Object.create = originalCreate; Object.setPrototypeOf = originalSet;
            Object.freeze = originalFreeze; Reflect.ownKeys = originalKeys;
        }
        calls === 0 && Object.getPrototypeOf(record) === null &&
        Object.getPrototypeOf(record.h) === null && child.__scDeserializeRecord(record).value[0] === 7
    "#,
    );
}

fn decoder_rejects_accessors_and_defines_own_properties<E: ScriptEngine>() {
    let mut runtime = with_child::<E>();
    check(
        &mut runtime,
        r#"
        var getterCalls = 0, errorName = '';
        var malicious = { h: [] };
        Object.defineProperty(malicious, 'r', { get: function() { getterCalls++; return ['s', 'bad']; } });
        try { child.__scDeserializeRecord(malicious); } catch (error) { errorName = error.name; }
        child.setterCalls = 0;
        child.Object.defineProperty(child.Object.prototype, 'trap', {
            configurable: true, set: child.Function('value', 'globalThis.setterCalls++;')
        });
        var input = { trap: 9 };
        Object.defineProperty(input, '__proto__', { enumerable: true, value: { value: 4 } });
        var copy = child.__scDeserializeRecord(__scSerializeRecord(input));
        delete child.Object.prototype.trap;
        getterCalls === 0 && errorName === 'DataCloneError' && child.setterCalls === 0 &&
        copy.trap === 9 && Object.prototype.hasOwnProperty.call(copy, '__proto__') &&
        Object.getPrototypeOf(copy) === child.Object.prototype && copy.__proto__.value === 4
    "#,
    );
}

macro_rules! backend {
    ($module:ident, $engine:ty) => {
        mod $module {
            #[test]
            fn cycles_and_recipient_intrinsics() {
                super::cycles_and_recipient_intrinsics::<$engine>();
            }
            #[test]
            fn buffers_preserve_aliases_and_bytes() {
                super::buffers_preserve_aliases_and_bytes::<$engine>();
            }
            #[test]
            fn sender_rejects_platform_objects() {
                super::sender_rejects_platform_objects::<$engine>();
            }
            #[test]
            fn records_are_detached_and_worker_wire_stays_compatible() {
                super::records_are_detached_and_worker_wire_stays_compatible::<$engine>();
            }
            #[test]
            fn exposed_records_carry_no_source_prototypes() {
                super::exposed_records_carry_no_source_prototypes::<$engine>();
            }
            #[test]
            fn record_hardening_uses_captured_intrinsics() {
                super::record_hardening_uses_captured_intrinsics::<$engine>();
            }
            #[test]
            fn decoder_rejects_accessors_and_defines_own_properties() {
                super::decoder_rejects_accessors_and_defines_own_properties::<$engine>();
            }
        }
    };
}

backend!(boa, script_engine_boa::BoaEngine);
#[cfg(target_pointer_width = "64")]
backend!(nova, script_engine_nova::NovaEngine);
