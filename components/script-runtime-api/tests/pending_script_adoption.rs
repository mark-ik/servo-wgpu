// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

use script_engine_api::ScriptEngine;
use script_runtime_api::{ParserScriptLoader, Runtime};
use std::cell::RefCell;

#[derive(Default)]
struct Loader {
    requests: RefCell<Vec<String>>,
}
impl ParserScriptLoader for Loader {
    fn load(&self, src: &str, _: Option<&str>, _: Option<&str>) -> Option<String> {
        self.requests.borrow_mut().push(src.to_owned());
        Some(if src.ends_with("move.js") {
            "var child = document.getElementById('frame').contentWindow; var movedScript = document.getElementById('later'); child.document.body.appendChild(movedScript);".to_owned()
        } else {
            "window.pendingAdoptedScriptRan = true;".to_owned()
        })
    }
}

fn queued_script_is_not_executed_after_adoption<E: ScriptEngine>(attributes: &str) {
    let mut rt = Runtime::<E>::new().unwrap();
    rt.set_base_url("https://parent.test/index.html").unwrap();
    let loader = Loader::default();
    rt.parse_document_interleaved(&format!(
        "<body><iframe id=frame></iframe><script async src=move.js></script><script id=later {attributes} src=later.js></script></body>"
    ), &loader);
    rt.run_event_loop(20).unwrap();
    rt.eval(r#"
        if (movedScript.ownerDocument !== child.document || movedScript.parentNode !== child.document.body)
            throw new Error('queued script was not adopted');
        if (window.pendingAdoptedScriptRan !== undefined || child.pendingAdoptedScriptRan !== undefined)
            throw new Error('prepared script executed after its document changed');
        document.body.appendChild(movedScript);
        if (window.pendingAdoptedScriptRan !== undefined)
            throw new Error('already-started queued script reran after returning');
    "#).unwrap();
    assert_eq!(
        loader.requests.borrow().len(),
        1,
        "moved queued script reached execution loader"
    );
}

macro_rules! backend {
    ($module:ident, $engine:ty) => {
        mod $module {
            #[test]
            fn queued_async() {
                super::queued_script_is_not_executed_after_adoption::<$engine>("async");
            }
            #[test]
            fn queued_deferred() {
                super::queued_script_is_not_executed_after_adoption::<$engine>("defer");
            }
            #[test]
            fn queued_module() {
                super::queued_script_is_not_executed_after_adoption::<$engine>("type=module");
            }
        }
    };
}
backend!(boa, script_engine_boa::BoaEngine);
#[cfg(target_pointer_width = "64")]
backend!(nova, script_engine_nova::NovaEngine);
