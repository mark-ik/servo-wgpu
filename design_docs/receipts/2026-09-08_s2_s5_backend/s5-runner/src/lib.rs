use std::time::{Duration, Instant};

use script_engine_api::ScriptEngine;
use script_engine_boa::BoaEngine;
use script_runtime_api::{Runtime, ScriptResourceLoader};

struct Scripts;

impl ScriptResourceLoader for Scripts {
    fn load(&self, url: &str) -> Option<String> {
        (url == "wake.js").then(|| {
            "onmessage = function(e) { postMessage('wake:' + e.data); };".to_owned()
        })
    }
}

fn read(rt: &mut Runtime<BoaEngine>, expr: &str) -> String {
    let value = rt.eval(expr).expect("eval");
    rt.value_to_string(&value).expect("string")
}

fn start(rt: &mut Runtime<BoaEngine>) {
    rt.set_script_resource_loader(Box::new(Scripts));
    rt.eval(
        "var got = ''; var w = new Worker('wake.js'); \
         w.onmessage = function(e) { got = e.data; }; w.postMessage('one');",
    )
    .expect("worker starts");
}

#[test]
fn worker_liveness_is_not_a_timer_and_pump_is_the_wake_service() {
    let mut rt = Runtime::<BoaEngine>::new().expect("runtime");
    start(&mut rt);

    // Negative control: timer and microtask progress alone never service the
    // worker link, even though the runtime truthfully reports external work.
    assert!(rt.has_worker_work());
    for now_ms in [0.0, 1.0, 2.0] {
        assert_eq!(rt.run_timers(64, now_ms), 0);
        rt.run_microtasks();
    }
    assert_eq!(read(&mut rt, "got"), "");
    assert!(rt.has_worker_work());

    // Positive: the actual runtime worker service delivers the external wake.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && read(&mut rt, "got") != "wake:one" {
        rt.run_microtasks();
        rt.run_timers(64, 0.0);
        rt.pump_workers();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(read(&mut rt, "got"), "wake:one");

    // A quiescence observation is separate from the reply: keep servicing
    // until the worker's acknowledged-idle report crosses the link.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && rt.has_worker_work() {
        rt.pump_workers();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!rt.has_worker_work(), "worker must report idle before quiescence");
}
