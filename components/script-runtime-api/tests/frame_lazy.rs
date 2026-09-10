// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::cell::Cell;
use std::rc::Rc;

use script_engine_api::ScriptEngine;
use script_runtime_api::{NoScriptLoader, Runtime, ScriptResourceLoader};

struct CountLoads(Rc<Cell<usize>>);
impl ScriptResourceLoader for CountLoads {
    fn load(&self, _: &str) -> Option<String> {
        self.0.set(self.0.get() + 1);
        Some("<script>parent.childRan=true;</script>".into())
    }
}

fn lazy_frames_keep_initial_context_without_loading<E: ScriptEngine>() {
    let mut runtime = Runtime::<E>::new().expect("runtime");
    runtime
        .set_base_url("https://parent.test/index.html")
        .unwrap();
    let loads = Rc::new(Cell::new(0));
    runtime.set_script_resource_loader(Box::new(CountLoads(loads.clone())));
    runtime
        .eval(
            "var parentLoads=0, childLoads=0; addEventListener('load',function(){parentLoads++;});",
        )
        .unwrap();
    runtime.parse_document_interleaved(
        r#"<body><iframe id=external loading=" LAZY " src="https://other.test/child.html"></iframe>
        <iframe id=inline loading=lazy srcdoc="&lt;script&gt;parent.childRan=true;&lt;/script&gt;"></iframe></body>"#,
        &NoScriptLoader,
    );
    runtime.eval(r#"
        var initial = document.getElementById('external').contentWindow;
        if (!initial || initial.document.body.innerHTML !== '' || initial.parent !== window)
            throw new Error('lazy frame must expose its same-origin initial context');
        initial.addEventListener('load',function(){childLoads++;});
        document.getElementById('inline').onload=function(){childLoads++;};
        if (parentLoads !== 1 || document.readyState !== 'complete')
            throw new Error('lazy frame delayed parent load');
        var inert = new DOMParser().parseFromString('<iframe loading=lazy src="/hidden.html" style="display:none"></iframe>', 'text/html');
        var adopted = inert.querySelector('iframe');
        adopted.onload=function(){childLoads++;};
        document.body.appendChild(adopted);
        if (!adopted.contentWindow) throw new Error('adopted lazy frame missing context');
    "#).unwrap();
    runtime.run_event_loop(20).unwrap();
    let result = runtime.eval("childLoads === 0 && parentLoads === 1 && globalThis.childRan === undefined && document.getElementById('external').contentWindow === initial").unwrap();
    assert_eq!(runtime.value_to_string(&result).unwrap(), "true");
    assert_eq!(loads.get(), 0, "lazy documents must not acquire source");
}

#[test]
fn lazy_frames_on_boa() {
    lazy_frames_keep_initial_context_without_loading::<script_engine_boa::BoaEngine>();
}

#[cfg(target_pointer_width = "64")]
#[test]
fn lazy_frames_on_nova() {
    lazy_frames_keep_initial_context_without_loading::<script_engine_nova::NovaEngine>();
}
