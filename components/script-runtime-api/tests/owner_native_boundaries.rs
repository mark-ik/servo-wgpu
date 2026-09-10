// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

use genet_scripted_dom::NodeId;
use script_engine_api::{MAIN_REALM, ScriptEngine};
use script_runtime_api::{NoScriptLoader, Runtime};

fn runtime<E: ScriptEngine>() -> Runtime<E> {
    let mut rt = Runtime::<E>::new().unwrap();
    rt.set_base_url("https://parent.test/index.html").unwrap();
    rt.parse_document_interleaved(
        "<body><p id=keep>caller</p><iframe id=frame></iframe></body>",
        &NoScriptLoader,
    );
    rt.run_event_loop(20).unwrap();
    rt.eval("var child=document.getElementById('frame').contentWindow;")
        .unwrap();
    rt
}

fn borrowed_document_stream_refuses_without_mutation<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    rt.eval(r#"
        child.document.body.innerHTML='<p id=childKeep>child</p>';
        var before=document.documentElement.outerHTML;
        var childBefore=child.document.documentElement.outerHTML;
        for (var op of ['open','write','close']) {
            var refused=false;
            try { Document.prototype[op].call(child.document, '<p>unexpected</p>'); }
            catch(e) { refused=String(e).indexOf('owning document realm')>=0; }
            if (!refused) throw new Error('foreign stream did not refuse '+op);
            if (document.documentElement.outerHTML!==before || child.document.documentElement.outerHTML!==childBefore)
                throw new Error('foreign stream mutated a document '+op);
        }
        var frame=child.document.createElement('iframe');
        child.document.body.appendChild(frame);
        var getWindow=Object.getOwnPropertyDescriptor(HTMLIFrameElement.prototype,'contentWindow').get;
        var refused=false;
        try { getWindow.call(frame); } catch(e) { refused=String(e).indexOf('owning document realm')>=0; }
        if(!refused) throw new Error('borrowed frame getter did not refuse');
    "#).unwrap();
}

fn adopted_scroll_targets_storage_owner<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    rt.eval("var node=document.createElement('div'); child.document.body.appendChild(node); node.scrollIntoView();").unwrap();
    let raw = rt.eval("String(__nodeRawId(node.__ref))").unwrap();
    let node = NodeId::from_raw(rt.value_to_string(&raw).unwrap().parse().unwrap());
    let realm = rt.frame_realms(MAIN_REALM)[0].1;
    assert_eq!(rt.host().borrow().scroll_into_view, None);
    assert_eq!(
        rt.host_in_realm(realm).unwrap().borrow().scroll_into_view,
        Some(node)
    );
}

fn public_child_bootstrap_resolves_its_registered_host<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().unwrap();
    let realm = rt
        .create_child_realm(script_runtime_api::HostState::default())
        .unwrap();
    let value = rt.eval_in_realm(realm,
        "var node=document.createElement('div'); document.appendChild(node); document.firstChild===node && node.ownerDocument===document"
    ).unwrap();
    assert_eq!(rt.value_to_string(&value).unwrap(), "true");
}

macro_rules! backend {
    ($module:ident,$engine:ty) => {
        mod $module {
            #[test]
            fn public_child_bootstrap() {
                super::public_child_bootstrap_resolves_its_registered_host::<$engine>();
            }
            #[test]
            fn borrowed_document_stream() {
                super::borrowed_document_stream_refuses_without_mutation::<$engine>();
            }
            #[test]
            fn adopted_scroll_owner() {
                super::adopted_scroll_targets_storage_owner::<$engine>();
            }
        }
    };
}
backend!(boa, script_engine_boa::BoaEngine);
#[cfg(target_pointer_width = "64")]
backend!(nova, script_engine_nova::NovaEngine);
