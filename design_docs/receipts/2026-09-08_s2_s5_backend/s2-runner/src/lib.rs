use genet_static_dom::StaticDocument;
use script_engine_api::ScriptEngine;
use script_engine_boa::BoaEngine;
use script_runtime_api::Runtime;

fn read(rt: &mut Runtime<BoaEngine>, expression: &str) -> String {
    let value = rt.eval(expression).expect("eval");
    rt.value_to_string(&value).expect("string")
}

fn runtime() -> Runtime<BoaEngine> {
    let mut rt = Runtime::<BoaEngine>::new().expect("runtime");
    rt.load_dom(&StaticDocument::parse(
        "<html><body><div id='text-host'><span id='text-child' data-proof='text'>text leaf</span></div><div id='html-host'><span id='html-child' data-proof='html'>html leaf</span></div></body></html>",
    ));
    rt
}

#[test]
fn js_references_survive_replacement_then_release_collects() {
    let mut rt = runtime();
    rt.eval(
        "var textChild = document.getElementById('text-child'); \
         var htmlChild = document.getElementById('html-child'); \
         var textHost = document.getElementById('text-host'); \
         var htmlHost = document.getElementById('html-host'); \
         textHost.textContent = 'replacement'; \
         htmlHost.innerHTML = '<i>replacement</i>';",
    )
    .expect("replace with observers off");

    // Backend-visible semantic reads prove these are live JS reflectors, not
    // raw NodeIds held by the test harness.
    assert_eq!(read(&mut rt, "textChild.textContent"), "text leaf");
    assert_eq!(read(&mut rt, "htmlChild.textContent"), "html leaf");
    assert_eq!(rt.collect_garbage().0, 0, "live JS reflectors stay pinned");
    assert_eq!(read(&mut rt, "String(textChild.isConnected) + ',' + String(htmlChild.isConnected)"), "false,false");

    // Reattachment runs through the public DOM surface after both replacement
    // forms. This is the positive control that the retained descendants still
    // denote usable nodes.
    rt.eval("textHost.appendChild(textChild); htmlHost.appendChild(htmlChild);")
        .expect("reattach");
    assert_eq!(read(&mut rt, "textHost.lastChild.textContent + ',' + htmlHost.lastChild.textContent"), "text leaf,html leaf");

    // Detach, release both last JS references, and force the real backend GC.
    rt.eval(
        "textHost.removeChild(textChild); htmlHost.removeChild(htmlChild); \
         textChild = null; htmlChild = null;",
    )
    .expect("detach and release");
    let (unpinned, collected) = rt.collect_garbage();
    assert!(unpinned >= 2, "both released reflectors must unpin; got {unpinned}");
    assert!(collected >= 2, "both detached descendants must collect; got {collected}");
}
