// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use script_engine_api::ScriptEngine;
use script_runtime_api::{NoScriptLoader, Runtime};

fn raw_mutations_require_an_adoption_transaction<E: ScriptEngine>() {
    let mut runtime = Runtime::<E>::new().unwrap();
    runtime
        .set_base_url("https://parent.test/index.html")
        .unwrap();
    runtime.parse_document_interleaved(
        "<body><p id=source>source</p><iframe></iframe></body>",
        &NoScriptLoader,
    );
    runtime.run_event_loop(20).unwrap();
    runtime.eval(r#"
        var child = document.querySelector('iframe').contentWindow;
        var source = document.getElementById('source'), sourceParent = source.parentNode;
        child.document.body.innerHTML = '<b id=local>destination</b>';
        var destination = child.document.body, local = destination.firstChild;
        var beforeSource = document.body.innerHTML, beforeDestination = destination.innerHTML;
        function refused(operation) {
            var error;
            try { operation(); } catch (e) { error=e; }
            if (!error || !/adoption/i.test(String(error)))
                throw new Error('raw cross-store mutation must require adoption transaction');
            if (document.body.innerHTML !== beforeSource || destination.innerHTML !== beforeDestination ||
                source.parentNode !== sourceParent || source.ownerDocument !== document ||
                local.parentNode !== destination)
                throw new Error('refused native mutation changed tree');
        }
        refused(function(){child.__appendChild(destination.__ref,source.__ref);});
        refused(function(){__appendChild(destination.__ref,source.__ref);});
        refused(function(){child.__insertBefore(destination.__ref,source.__ref,local.__ref);});
        refused(function(){child.__insertBefore(destination.__ref,local.__ref,source.__ref);});
        refused(function(){child.__moveBefore(destination.__ref,source.__ref,local.__ref);});
        refused(function(){child.__moveBefore(destination.__ref,local.__ref,source.__ref);});
        refused(function(){child.__removeChild(destination.__ref,source.__ref);});
        if (source.contains(local) || local.contains(source)) throw new Error('disjoint nodes contain each other');
        var forward=source.compareDocumentPosition(local), reverse=local.compareDocumentPosition(source);
        if (!(forward & 1) || !(reverse & 1) || !(forward & 32) || !(reverse & 32) ||
            !((forward & 2) && (reverse & 4) || (forward & 4) && (reverse & 2)))
            throw new Error('disconnected ordering is not stable and reciprocal');
        var equal = child.document.createElement('p'); equal.id='source'; equal.textContent='source';
        if (!source.isEqualNode(equal) || !equal.isEqualNode(source))
            throw new Error('cross-store structural equality failed');
    "#).unwrap();
}

#[test]
fn raw_native_adoption_boundary_on_boa() {
    raw_mutations_require_an_adoption_transaction::<script_engine_boa::BoaEngine>();
}

#[cfg(target_pointer_width = "64")]
#[test]
fn raw_native_adoption_boundary_on_nova() {
    raw_mutations_require_an_adoption_transaction::<script_engine_nova::NovaEngine>();
}
