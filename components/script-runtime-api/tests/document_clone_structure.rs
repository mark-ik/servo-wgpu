// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

use script_engine_api::ScriptEngine;
use script_runtime_api::{NoScriptLoader, Runtime};

fn documents_clone_into_empty_owned_trees<E: ScriptEngine>() {
    let mut rt = Runtime::<E>::new().unwrap();
    rt.parse_document_interleaved("<!doctype html><!--before--><html><head><link rel=stylesheet href='https://example.test/style.css'></head><body><p id=source>kept</p></body></html>", &NoScriptLoader);
    rt.eval(r#"
        var serializer=new XMLSerializer(), before=serializer.serializeToString(document);
        var shallow=document.cloneNode(false);
        if (shallow===document || shallow.nodeType!==9 || shallow.ownerDocument!==null ||
            shallow.defaultView!==null || shallow.childNodes.length!==0 || shallow.documentElement!==null)
            throw new Error('shallow document clone was populated');
        var deep=document.cloneNode(true);
        if (deep.childNodes.length!==document.childNodes.length || deep.defaultView!==null ||
            serializer.serializeToString(deep)!==before)
            throw new Error('deep document children differ');
        function compare(source, copy, owner) {
            if (source===copy || source.nodeType!==copy.nodeType || source.nodeName!==copy.nodeName)
                throw new Error('clone identity/type');
            if (copy.nodeType!==9 && copy.ownerDocument!==owner) throw new Error('descendant owner');
            if (source.childNodes.length!==copy.childNodes.length) throw new Error('child count');
            for(var i=0;i<source.childNodes.length;i++) compare(source.childNodes[i],copy.childNodes[i],owner);
        }
        compare(document,deep,deep);
        if(deep.getElementById('source').textContent!=='kept') throw new Error('content lost');
        if(serializer.serializeToString(document)!==before) throw new Error('source mutated');
        var inert=document.implementation.createHTMLDocument('inert');
        inert.head.innerHTML='<link rel=stylesheet href="https://example.test/inert.css">';
        var inertClone=inert.cloneNode(true);
        compare(inert,inertClone,inertClone);
        if(inertClone.documentElement.outerHTML!==inert.documentElement.outerHTML) throw new Error('inert clone changed');
        var xml=document.implementation.createDocument('urn:test','x:Root',null);
        var xmlClone=xml.cloneNode(true), xmlShallow=xml.cloneNode(false);
        compare(xml,xmlClone,xmlClone);
        if(xmlShallow.childNodes.length!==0 || xmlShallow.createElement('MiXeD').localName!=='MiXeD' ||
            xmlClone.documentElement.namespaceURI!=='urn:test') throw new Error('XML document mode lost');
        var written=document.cloneNode(true);
        written.write('<div><template shadowrootmode=open>test</template></div>');
        if(!written.body.firstChild.shadowRoot || written.body.firstChild.shadowRoot.textContent!=='test')
            throw new Error('cloned HTML document declarative shadow parsing lost');
    "#).unwrap();
}

#[test]
fn boa_document_clone_structure() {
    documents_clone_into_empty_owned_trees::<script_engine_boa::BoaEngine>();
}
#[test]
#[cfg(target_pointer_width = "64")]
fn nova_document_clone_structure() {
    documents_clone_into_empty_owned_trees::<script_engine_nova::NovaEngine>();
}
