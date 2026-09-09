use genet_scripted::ScriptedDocument;
use script_engine_boa::BoaEngine;
use script_runtime_api::ScriptResourceLoader;
use std::time::{Duration, Instant};

struct Scripts;
impl ScriptResourceLoader for Scripts {
    fn load(&self, url: &str) -> Option<String> {
        eprintln!("worker resource request: {url}");
        (url == "https://probe.test/wake.js").then(||
            "onmessage = function(e) { postMessage('wake:' + e.data); };".to_owned())
    }
}

#[test]
fn document_pump_forwards_worker_service_and_waits_for_acknowledged_idle() {
    let mut doc = ScriptedDocument::<BoaEngine>::parse("<html><body>before</body></html>").unwrap();
    doc.set_script_resource_loader(Box::new(Scripts));
    // Evaluated separately, so script marker text cannot contaminate the DOM.
    doc.evaluate("var w = new Worker('https://probe.test/wake.js'); w.onmessage = function(e) { document.body.textContent = e.data; Promise.resolve().then(function(){ document.body.setAttribute('data-reaction','done'); }); }; w.onerror = function(e) { document.body.textContent = 'worker-error:' + e.message; }; w.postMessage('one');").unwrap();
    assert_eq!(doc.dom_snapshot(), "<html><head></head><body>before</body></html>");
    assert!(doc.has_pending_work(), "live worker is outstanding without a timer");
    let expected="<html><head></head><body data-reaction=\"done\">wake:one</body></html>";
    let started=Instant::now();
    let mut pumps=0;
    while started.elapsed()<Duration::from_secs(30) && doc.dom_snapshot()!=expected {
        doc.pump(started.elapsed().as_secs_f64()*1000.0);
        pumps+=1;
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(doc.dom_snapshot(),expected,"worker reply and its Promise reaction");
    let idle_started=Instant::now();
    while idle_started.elapsed()<Duration::from_secs(30) && doc.has_pending_work() {
        doc.pump(started.elapsed().as_secs_f64()*1000.0);
        pumps+=1;
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!doc.has_pending_work(),"worker idle acknowledgement");
    println!("delivery+idle pumps={pumps}, elapsed_ms={}",started.elapsed().as_millis());
}
