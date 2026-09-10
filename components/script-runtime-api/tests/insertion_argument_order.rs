// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

use script_engine_api::ScriptEngine;
use script_runtime_api::{NoScriptLoader, Runtime};

fn insertion_arguments_precede_hierarchy<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().unwrap();
    rt.parse_document_interleaved("<!doctype html><body><div id=source><b id=moving>kept</b></div><div id=target></div></body>", &NoScriptLoader);
    rt.eval(r#"
        var leaves=[document.createTextNode('text'), document.createComment('comment'),
                    document.implementation.createDocumentType('html','',''),
                    document.createProcessingInstruction('test','data')];
        function expectTypeError(call, label) {
            var caught;
            try { call(); } catch(e) { caught=e; }
            if (!(caught instanceof TypeError)) throw new Error(label+': expected TypeError, got '+caught);
        }
        for (var i=0;i<leaves.length;i++) {
            var leaf=leaves[i];
            expectTypeError(function(){leaf.appendChild(null);}, 'append null '+i);
            expectTypeError(function(){leaf.appendChild();}, 'append omitted '+i);
            expectTypeError(function(){leaf.insertBefore(null,null);}, 'insert null '+i);
            var caught;
            try { leaf.appendChild(document.createTextNode('child')); } catch(e) { caught=e; }
            if (!caught || caught.name!=='HierarchyRequestError') throw new Error('valid node hierarchy '+i);
            if (leaf.parentNode!==null || leaf.childNodes.length!==0) throw new Error('failed leaf insertion mutated tree');
        }
        var ancestor=document.createElement('section'), parent=document.createElement('div');
        var unrelated=document.createElement('b'); ancestor.appendChild(parent);
        var precedenceError;
        try { parent.insertBefore(ancestor,unrelated); } catch(e) { precedenceError=e; }
        if (!precedenceError || precedenceError.name!=='HierarchyRequestError')
            throw new Error('ancestor must precede reference membership');
        var illegal=document.implementation.createHTMLDocument('illegal child');
        precedenceError=null;
        try { parent.insertBefore(illegal,unrelated); } catch(e) { precedenceError=e; }
        if (!precedenceError || precedenceError.name!=='NotFoundError')
            throw new Error('reference membership must precede inserted node kind');
        if(parent.parentNode!==ancestor || ancestor.firstChild!==parent || parent.childNodes.length!==0 || unrelated.parentNode!==null)
            throw new Error('precedence checks mutated nodes');
        var source=document.getElementById('source'), target=document.getElementById('target');
        var moving=document.getElementById('moving'), original=moving.__ref, reads=0;
        Object.defineProperty(moving,'__ref',{configurable:true,get:function(){return ++reads===1?original:undefined;}});
        expectTypeError(function(){target.appendChild(moving);},'changed argument must be revalidated');
        Object.defineProperty(moving,'__ref',{value:original,writable:true,configurable:true});
        if (moving.parentNode!==source || source.firstChild!==moving || target.childNodes.length!==0 || moving.textContent!=='kept')
            throw new Error('argument failure detached or mutated original node');
    "#).unwrap();
}

#[test]
fn boa_insertion_arguments_precede_hierarchy() {
    insertion_arguments_precede_hierarchy::<script_engine_boa::BoaEngine>();
}
#[test]
#[cfg(target_pointer_width = "64")]
fn nova_insertion_arguments_precede_hierarchy() {
    insertion_arguments_precede_hierarchy::<script_engine_nova::NovaEngine>();
}
