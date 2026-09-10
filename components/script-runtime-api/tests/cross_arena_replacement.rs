// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

use script_engine_api::ScriptEngine;
use script_runtime_api::{NoScriptLoader, Runtime};

fn check<E: ScriptEngine>(source: &str) {
    let mut rt = Runtime::<E>::new().unwrap();
    rt.set_base_url("https://parent.test/").unwrap();
    rt.parse_document_interleaved("<body><iframe id=frame></iframe></body>", &NoScriptLoader);
    rt.run_event_loop(20).unwrap();
    rt.eval("var child = document.getElementById('frame').contentWindow;")
        .unwrap();
    rt.eval(source).expect(source);
}

fn replace_child<E: ScriptEngine>() {
    check::<E>(
        r#"
        var target = child.document.createElement('div'); child.document.body.appendChild(target);
        var old = child.document.createElement('i'); target.appendChild(old);
        var replacement = document.createElement('b'); document.body.appendChild(replacement);
        var observer = new MutationObserver(function(){}); observer.observe(target,{childList:true});
        if (Node.prototype.replaceChild.call(target,replacement,old) !== old)
            throw new Error('replace return');
        var records = observer.takeRecords();
        if (target.firstChild !== replacement || replacement.ownerDocument !== child.document || old.parentNode !== null)
            throw new Error('replace ownership');
        if (records.length !== 1 || records[0].target !== target || records[0].addedNodes[0] !== replacement ||
            records[0].removedNodes[0] !== old) throw new Error('replace coalescing');
    "#,
    );
}

fn replace_children_fragment<E: ScriptEngine>() {
    check::<E>(
        r#"
        var target = child.document.createElement('div'); child.document.body.appendChild(target);
        var old = child.document.createElement('i'); target.appendChild(old);
        var fragment = document.createDocumentFragment();
        var first = document.createElement('b'), second = document.createElement('em');
        fragment.append(first,second);
        var sourceObserver = new MutationObserver(function(){});
        sourceObserver.observe(fragment,{childList:true});
        var observer = new MutationObserver(function(){}); observer.observe(target,{childList:true});
        Element.prototype.replaceChildren.call(target,fragment);
        var records = observer.takeRecords(), removed = sourceObserver.takeRecords();
        if (target.firstChild !== first || target.lastChild !== second || fragment.firstChild !== null ||
            fragment.ownerDocument !== child.document || first.ownerDocument !== child.document)
            throw new Error('fragment identity/ownership');
        if (records.length !== 1 || records[0].addedNodes.length !== 2 || records[0].removedNodes[0] !== old)
            throw new Error('replaceChildren coalescing');
        if (removed.length !== 1 || removed[0].removedNodes.length !== 2 || removed[0].target !== fragment)
            throw new Error('fragment removal coalescing');
    "#,
    );
}

fn unsupported_replacement_is_atomic<E: ScriptEngine>() {
    check::<E>(
        r#"
        var target = child.document.createElement('div'); child.document.body.appendChild(target);
        var old = child.document.createElement('i'); target.appendChild(old);
        var first = document.createElement('b'), unsupported = document.createElement('iframe');
        document.body.appendChild(first); document.body.appendChild(unsupported);
        function refused(f) { try {f();} catch(e) {return;} throw new Error('unsupported transfer admitted'); }
        refused(function(){Element.prototype.replaceChildren.call(target,first,unsupported);});
        if (first.parentNode !== document.body || unsupported.parentNode !== document.body || target.firstChild !== old)
            throw new Error('multiargument refusal mutated source or destination');
        refused(function(){Node.prototype.replaceChild.call(target,unsupported,old);});
        if (unsupported.parentNode !== document.body || target.firstChild !== old)
            throw new Error('replace refusal mutated source or destination');
    "#,
    );
}

fn invalid_destination_preserves_both_trees<E: ScriptEngine>() {
    check::<E>(
        r#"
        var originalRoot = child.document.documentElement;
        var source = document.createElement('b'); document.body.appendChild(source);
        var text = document.createTextNode('source'); document.body.appendChild(text);
        function rejected(name, f) {
            try { f(); } catch (e) {
                if (e.name !== name) throw new Error('wrong refusal: '+e.name);
                if (source.parentNode !== document.body || text.parentNode !== document.body ||
                    child.document.documentElement !== originalRoot) throw new Error('refusal changed tree');
                return;
            }
            throw new Error('invalid destination admitted');
        }
        rejected('HierarchyRequestError', function(){child.document.appendChild(source);});
        rejected('HierarchyRequestError', function(){child.document.replaceChild(text,originalRoot);});
        rejected('HierarchyRequestError', function(){text.appendChild(source);});
        rejected('HierarchyRequestError', function(){child.document.body.appendChild(document);});
        rejected('NotFoundError', function(){child.document.body.insertBefore(source,document.body.firstChild);});
        var fragment = document.createDocumentFragment();
        var one = document.createElement('i'), two = document.createElement('em'); fragment.append(one,two);
        rejected('HierarchyRequestError', function(){child.document.replaceChild(fragment,originalRoot);});
        if (fragment.firstChild !== one || fragment.lastChild !== two) throw new Error('invalid fragment was drained');
        rejected('HierarchyRequestError', function(){child.document.replaceChildren(source,text);});
        var doctype = document.implementation.createDocumentType('html','','');
        document.insertBefore(doctype,document.documentElement);
        rejected('HierarchyRequestError', function(){child.document.appendChild(doctype);});
        if (doctype.parentNode !== document) throw new Error('invalid doctype move detached source');
        // Replacing the sole element with a sole element remains valid.
        child.document.replaceChild(source,originalRoot);
        if (child.document.documentElement !== source || source.ownerDocument !== child.document)
            throw new Error('valid document replacement was rejected');
    "#,
    );
}

macro_rules! backend {
    ($module:ident, $engine:ty) => {
        mod $module {
            #[test]
            fn invalid_destination() {
                super::invalid_destination_preserves_both_trees::<$engine>();
            }
            #[test]
            fn replace_child() {
                super::replace_child::<$engine>();
            }
            #[test]
            fn fragment_replace_children() {
                super::replace_children_fragment::<$engine>();
            }
            #[test]
            fn atomic_refusal() {
                super::unsupported_replacement_is_atomic::<$engine>();
            }
        }
    };
}
backend!(boa, script_engine_boa::BoaEngine);
#[cfg(target_pointer_width = "64")]
backend!(nova, script_engine_nova::NovaEngine);
