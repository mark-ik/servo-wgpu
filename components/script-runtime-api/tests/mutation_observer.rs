// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `MutationObserver` as the second consumer of the arena's mutation point:
//! registration and option validation, the record shapes, subtree and
//! attribute-filter scoping, `takeRecords` / `disconnect`, transient registered
//! observers for a removed subtree, and delivery as one microtask. Each body
//! runs against both backends.

use genet_static_dom::StaticDocument;
use script_engine_api::ScriptEngine;
use script_runtime_api::Runtime;

fn read<E: ScriptEngine>(rt: &mut Runtime<E>, expr: &str) -> String {
    let v = rt.eval(expr).expect("eval");
    rt.value_to_string(&v).expect("stringify")
}

fn runtime<E: ScriptEngine>() -> Runtime<E> {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.load_dom(&StaticDocument::parse(
        "<html><body><div id='host'><span id='a'>one</span></div></body></html>",
    ));
    rt
}

/// The interface object, its class strings, and the option validation.
fn observer_interface_and_options<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    assert_eq!(read(&mut rt, "typeof MutationObserver"), "function");
    assert_eq!(read(&mut rt, "typeof MutationRecord"), "function");
    assert_eq!(
        read(
            &mut rt,
            "Object.prototype.toString.call(new MutationObserver(function(){}))"
        ),
        "[object MutationObserver]"
    );
    rt.eval(
        "function thrown(fn){ try { fn(); return 'no-throw'; } catch(e){ return e.name; } }\
             var o = new MutationObserver(function(){}); var t = document.getElementById('host');",
    )
    .expect("setup");
    // Callback is required and must be callable.
    assert_eq!(
        read(&mut rt, "thrown(function(){ new MutationObserver(); })"),
        "TypeError"
    );
    // No qualifier at all, and each qualifier without its flag.
    assert_eq!(
        read(&mut rt, "thrown(function(){ o.observe(t, {}); })"),
        "TypeError"
    );
    assert_eq!(
        read(
            &mut rt,
            "thrown(function(){ o.observe(t, { attributes: false, attributeOldValue: true }); })"
        ),
        "TypeError"
    );
    assert_eq!(
        read(
            &mut rt,
            "thrown(function(){ o.observe(t, { attributes: false, attributeFilter: ['id'] }); })"
        ),
        "TypeError"
    );
    assert_eq!(
        read(
            &mut rt,
            "thrown(function(){ o.observe(t, { characterData: false, characterDataOldValue: true }); })"
        ),
        "TypeError"
    );
    // A qualifier alone implies its flag, so these are accepted.
    assert_eq!(
        read(
            &mut rt,
            "thrown(function(){ o.observe(t, { attributeOldValue: true }); \
             o.observe(t, { characterDataOldValue: true }); o.disconnect(); })"
        ),
        "no-throw"
    );
    assert_eq!(
        read(
            &mut rt,
            "thrown(function(){ o.observe(null, { childList: true }); })"
        ),
        "TypeError"
    );
}

/// `childList` records: added, removed, and the siblings either side.
fn child_list_records<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    rt.eval(
        "var seen = []; \
         var host = document.getElementById('host'); \
         var o = new MutationObserver(function(records){ \
           for (var i = 0; i < records.length; i++) seen.push(records[i]); \
         }); \
         o.observe(host, { childList: true }); \
         var b = document.createElement('b'); host.appendChild(b); \
         var c = document.createElement('c'); host.insertBefore(c, b); \
         host.removeChild(b);",
    )
    .expect("mutate");
    // Nothing is delivered synchronously.
    assert_eq!(read(&mut rt, "String(seen.length)"), "0");
    rt.run_microtasks();
    assert_eq!(read(&mut rt, "String(seen.length)"), "3");
    assert_eq!(
        read(
            &mut rt,
            "seen[0].type + ',' + seen[0].addedNodes.length + ',' + seen[0].addedNodes[0].localName \
             + ',' + String(seen[0].target === host) + ',' + seen[0].previousSibling.localName \
             + ',' + String(seen[0].nextSibling)"
        ),
        "childList,1,b,true,span,null"
    );
    assert_eq!(
        read(
            &mut rt,
            "seen[1].addedNodes[0].localName + ',' + seen[1].previousSibling.localName + ',' + seen[1].nextSibling.localName"
        ),
        "c,span,b"
    );
    assert_eq!(
        read(
            &mut rt,
            "seen[2].removedNodes.length + ',' + seen[2].removedNodes[0].localName \
             + ',' + seen[2].previousSibling.localName + ',' + String(seen[2].nextSibling) \
             + ',' + seen[2].addedNodes.length"
        ),
        "1,b,c,null,0"
    );
}

/// `attributes` records: old value, the filter, and the namespace field.
fn attribute_records<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    rt.eval(
        "var all = [], filtered = []; \
         var a = document.getElementById('a'); \
         var o1 = new MutationObserver(function(r){ all = all.concat(r); }); \
         var o2 = new MutationObserver(function(r){ filtered = filtered.concat(r); }); \
         o1.observe(a, { attributes: true, attributeOldValue: true }); \
         o2.observe(a, { attributeFilter: ['class'] }); \
         a.setAttribute('class', 'first'); \
         a.setAttribute('class', 'second'); \
         a.setAttribute('title', 'ignored-by-o2'); \
         a.removeAttribute('class');",
    )
    .expect("mutate");
    rt.run_microtasks();
    assert_eq!(read(&mut rt, "String(all.length)"), "4");
    assert_eq!(
        read(
            &mut rt,
            "all[0].type + ',' + all[0].attributeName + ',' + String(all[0].attributeNamespace) + ',' + String(all[0].oldValue)"
        ),
        "attributes,class,null,null"
    );
    assert_eq!(read(&mut rt, "String(all[1].oldValue)"), "first");
    assert_eq!(read(&mut rt, "String(all[3].oldValue)"), "second");
    // The filter drops `title`, and no old value was asked for.
    assert_eq!(
        read(
            &mut rt,
            "filtered.length + ',' + filtered[0].attributeName + ',' + String(filtered[0].oldValue)"
        ),
        "3,class,null"
    );
}

/// `characterData` records, including through the `textContent` sink.
fn character_data_records<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    rt.eval(
        "var seen = []; \
         var text = document.getElementById('a').firstChild; \
         var o = new MutationObserver(function(r){ seen = seen.concat(r); }); \
         o.observe(text, { characterData: true, characterDataOldValue: true }); \
         text.data = 'two'; \
         text.textContent = 'three';",
    )
    .expect("mutate");
    rt.run_microtasks();
    assert_eq!(
        read(
            &mut rt,
            "seen.length + ',' + seen[0].type + ',' + seen[0].oldValue + ',' + seen[1].oldValue"
        ),
        "2,characterData,one,two"
    );
}

/// `subtree` reaches descendants; without it a descendant mutation is not ours.
/// `innerHTML` and `textContent` arrive as childList records too.
fn subtree_and_native_sinks<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    rt.eval(
        "var deep = [], shallow = []; \
         var body = document.body, host = document.getElementById('host'); \
         var o1 = new MutationObserver(function(r){ deep = deep.concat(r); }); \
         var o2 = new MutationObserver(function(r){ shallow = shallow.concat(r); }); \
         o1.observe(body, { childList: true, attributes: true, subtree: true }); \
         o2.observe(body, { childList: true, attributes: true }); \
         host.setAttribute('data-x', '1'); \
         host.innerHTML = '<i></i><u></u>';",
    )
    .expect("mutate");
    rt.run_microtasks();
    // One attribute record plus one childList record for the innerHTML replacement.
    assert_eq!(read(&mut rt, "String(deep.length)"), "2");
    assert_eq!(read(&mut rt, "String(shallow.length)"), "0");
    assert_eq!(
        read(
            &mut rt,
            "deep[1].type + ',' + deep[1].addedNodes.length + ',' + deep[1].removedNodes.length \
             + ',' + deep[1].removedNodes[0].localName"
        ),
        "childList,2,1,span"
    );
    // textContent on a container is a childList replacement as well.
    rt.eval("deep = []; host.textContent = 'flat';")
        .expect("text");
    rt.run_microtasks();
    assert_eq!(
        read(
            &mut rt,
            "deep.length + ',' + deep[0].addedNodes.length + ',' + deep[0].removedNodes.length"
        ),
        "1,1,2"
    );
}

/// `takeRecords` drains without delivering; `disconnect` empties the queue and
/// stops further records.
fn take_records_and_disconnect<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    rt.eval(
        "var delivered = 0; \
         var host = document.getElementById('host'); \
         var o = new MutationObserver(function(r){ delivered += r.length; }); \
         o.observe(host, { childList: true }); \
         host.appendChild(document.createElement('b')); \
         var taken = o.takeRecords();",
    )
    .expect("mutate");
    rt.run_microtasks();
    assert_eq!(read(&mut rt, "taken.length + ',' + delivered"), "1,0");
    assert_eq!(read(&mut rt, "String(o.takeRecords().length)"), "0");

    rt.eval(
        "host.appendChild(document.createElement('c')); \
         o.disconnect(); \
         host.appendChild(document.createElement('d'));",
    )
    .expect("after disconnect");
    rt.run_microtasks();
    assert_eq!(
        read(&mut rt, "delivered + ',' + o.takeRecords().length"),
        "0,0"
    );

    // Re-observing does not resurrect what happened while disconnected.
    rt.eval("o.observe(host, { childList: true }); host.appendChild(document.createElement('e'));")
        .expect("re-observe");
    rt.run_microtasks();
    assert_eq!(read(&mut rt, "String(delivered)"), "1");
}

/// A removed subtree keeps its former ancestors' subtree observers until the
/// next delivery, and loses them after it.
fn transient_registered_observers<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    rt.eval(
        "var seen = []; \
         var host = document.getElementById('host'); \
         var span = document.getElementById('a'); \
         var o = new MutationObserver(function(r){ seen = seen.concat(r); }); \
         o.observe(host, { childList: true, attributes: true, subtree: true }); \
         host.removeChild(span); \
         span.setAttribute('after-removal', '1');",
    )
    .expect("mutate");
    rt.run_microtasks();
    // The removal and the mutation inside the removed subtree both arrive.
    assert_eq!(
        read(
            &mut rt,
            "seen.length + ',' + seen[0].type + ',' + seen[1].type"
        ),
        "2,childList,attributes"
    );
    // Delivery ends the transient registration.
    rt.eval("seen = []; span.setAttribute('later', '1');")
        .expect("later");
    rt.run_microtasks();
    assert_eq!(read(&mut rt, "String(seen.length)"), "0");
}

/// Delivery is one microtask, batched per observer, observers in the order
/// their first record was queued.
fn delivery_is_one_microtask<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    rt.eval(
        "var order = []; \
         var host = document.getElementById('host'); \
         var span = document.getElementById('a'); \
         var first = new MutationObserver(function(r){ order.push('first:' + r.length); }); \
         var second = new MutationObserver(function(r){ order.push('second:' + r.length); }); \
         second.observe(span, { attributes: true }); \
         first.observe(host, { childList: true }); \
         host.appendChild(document.createElement('b')); \
         span.setAttribute('x', '1'); \
         host.appendChild(document.createElement('c')); \
         Promise.resolve().then(function(){ order.push('promise'); });",
    )
    .expect("mutate");
    rt.run_microtasks();
    // `first` queued a record before `second` did, and both batch into one
    // callback each. Both run before the unrelated promise job, which was
    // queued after the mutations: the notify microtask is scheduled at mutation
    // time, not at the checkpoint.
    assert_eq!(read(&mut rt, "order.join('|')"), "first:2|second:1|promise");
}

macro_rules! both_engines {
    ($($body:ident => ($boa:ident, $nova:ident)),* $(,)?) => {
        $(
            #[test]
            fn $boa() { $body::<script_engine_boa::BoaEngine>(); }

            #[cfg(not(target_arch = "wasm32"))]
            #[test]
            fn $nova() { $body::<script_engine_nova::NovaEngine>(); }
        )*
    };
}

both_engines! {
    observer_interface_and_options => (observer_interface_on_boa, observer_interface_on_nova),
    child_list_records => (child_list_records_on_boa, child_list_records_on_nova),
    attribute_records => (attribute_records_on_boa, attribute_records_on_nova),
    character_data_records => (character_data_records_on_boa, character_data_records_on_nova),
    subtree_and_native_sinks => (subtree_and_native_sinks_on_boa, subtree_and_native_sinks_on_nova),
    take_records_and_disconnect => (take_records_on_boa, take_records_on_nova),
    transient_registered_observers => (transient_observers_on_boa, transient_observers_on_nova),
    delivery_is_one_microtask => (delivery_microtask_on_boa, delivery_microtask_on_nova),
}
