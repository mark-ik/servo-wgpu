// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `XMLHttpRequest` over the fetch host seam. Proves the state machine (the
//! readyState sequence and its events), the `open` / `setRequestHeader` rules,
//! the response accessors, abort and timeout, and the synchronous send — all on
//! one host seam, with no second network path. Backend: Boa.

use script_engine_api::ScriptEngine;
use script_engine_boa::BoaEngine;
use script_runtime_api::{FetchHandler, FetchOutcome, FetchRequest, Runtime};

/// Answers every request in place: 200, a couple of headers, and a body naming
/// the method and URL. Records what it saw, so the request rules are checkable.
#[derive(Default)]
struct Echo {
    seen: std::rc::Rc<std::cell::RefCell<Vec<FetchRequest>>>,
}

impl FetchHandler for Echo {
    fn fetch(&self, req: FetchRequest) -> FetchOutcome {
        let body = format!("echo:{}:{}", req.method, req.url).into_bytes();
        let out = FetchOutcome {
            network_error: false,
            status: 200,
            status_text: "OK".to_owned(),
            response_type: "basic".to_owned(),
            url: req.url.clone(),
            redirected: false,
            headers: vec![
                ("content-type".to_owned(), "text/plain".to_owned()),
                ("content-length".to_owned(), body.len().to_string()),
                ("x-echo".to_owned(), "1".to_owned()),
                ("set-cookie".to_owned(), "a=b".to_owned()),
            ],
            body,
        };
        self.seen.borrow_mut().push(req);
        out
    }
}

/// A deferred host that never answers: every fetch stays in flight, so abort and
/// timeout are observable.
struct NeverAnswers;
impl FetchHandler for NeverAnswers {
    fn start(&self, _id: u64, _req: FetchRequest) -> Option<FetchOutcome> {
        None
    }
}

fn read(rt: &mut Runtime<BoaEngine>, expr: &str) -> String {
    let v = rt.eval(expr).expect("eval");
    rt.engine_mut()
        .value_to_string(&v)
        .expect("value_to_string")
}

fn echo_runtime() -> (
    Runtime<BoaEngine>,
    std::rc::Rc<std::cell::RefCell<Vec<FetchRequest>>>,
) {
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut rt = Runtime::<BoaEngine>::new().unwrap();
    rt.set_fetch_handler(Box::new(Echo { seen: seen.clone() }));
    (rt, seen)
}

#[test]
fn the_interfaces_and_constants_exist() {
    let mut rt = Runtime::<BoaEngine>::new().unwrap();
    assert_eq!(
        read(
            &mut rt,
            "[typeof XMLHttpRequest, typeof XMLHttpRequestUpload, typeof XMLHttpRequestEventTarget, typeof ProgressEvent].join(',')"
        ),
        "function,function,function,function"
    );
    assert_eq!(
        read(
            &mut rt,
            "[XMLHttpRequest.UNSENT, XMLHttpRequest.OPENED, XMLHttpRequest.HEADERS_RECEIVED, XMLHttpRequest.LOADING, XMLHttpRequest.DONE].join(',')"
        ),
        "0,1,2,3,4"
    );
    // The constants sit on the prototype too, and a fresh object is UNSENT.
    assert_eq!(
        read(
            &mut rt,
            "var x = new XMLHttpRequest(); [x.DONE, x.readyState, x.status, x.statusText, x.responseText, x.responseType].join('|')"
        ),
        "4|0|0|||"
    );
    // The request and its upload share the XMLHttpRequestEventTarget surface.
    assert_eq!(
        read(
            &mut rt,
            "var y = new XMLHttpRequest(); [y instanceof XMLHttpRequestEventTarget, y.upload instanceof XMLHttpRequestUpload, y instanceof EventTarget].join(',')"
        ),
        "true,true,true"
    );
    // ProgressEvent is an Event with the three progress fields.
    assert_eq!(
        read(
            &mut rt,
            "var e = new ProgressEvent('progress', {lengthComputable:true, loaded:3, total:9}); [e instanceof Event, e.type, e.lengthComputable, e.loaded, e.total].join(',')"
        ),
        "true,progress,true,3,9"
    );
}

#[test]
fn open_enforces_the_method_and_state_rules() {
    let mut rt = Runtime::<BoaEngine>::new().unwrap();
    // CONNECT / TRACE / TRACK are a SecurityError, case-insensitively.
    assert_eq!(
        read(
            &mut rt,
            "var x=new XMLHttpRequest(); var out=[]; ['CONNECT','trace','TrAcK'].forEach(function(m){ try { x.open(m,'/a'); out.push('no-throw'); } catch(e) { out.push(e.name); } }); out.join(',')"
        ),
        "SecurityError,SecurityError,SecurityError"
    );
    // A non-token method is a SyntaxError.
    assert_eq!(
        read(
            &mut rt,
            "var x=new XMLHttpRequest(); try { x.open('G ET','/a'); 'no-throw' } catch(e) { e.name }"
        ),
        "SyntaxError"
    );
    // The six normalizable methods upper-case; others keep their case.
    assert_eq!(
        read(
            &mut rt,
            "var x=new XMLHttpRequest(); x.open('get','/a'); x.send(); x._method + ',' + (function(){var y=new XMLHttpRequest(); y.open('patch','/a'); return y._method;})()"
        ),
        "GET,patch"
    );
    // open() fires readystatechange once when entering OPENED, and not again
    // while already OPENED.
    assert_eq!(
        read(
            &mut rt,
            "var n=0; var x=new XMLHttpRequest(); x.onreadystatechange=function(){n++;}; x.open('GET','/a'); x.open('GET','/b'); [n, x.readyState].join(',')"
        ),
        "1,1"
    );
    // send() before open() is an InvalidStateError.
    assert_eq!(
        read(
            &mut rt,
            "var x=new XMLHttpRequest(); try { x.send(); 'no-throw' } catch(e) { e.name }"
        ),
        "InvalidStateError"
    );
}

#[test]
fn set_request_header_applies_the_forbidden_list_and_combines() {
    let (mut rt, seen) = echo_runtime();
    // Before open(): InvalidStateError.
    assert_eq!(
        read(
            &mut rt,
            "var x=new XMLHttpRequest(); try { x.setRequestHeader('X-A','1'); 'no-throw' } catch(e) { e.name }"
        ),
        "InvalidStateError"
    );
    // An invalid name or value is a SyntaxError.
    assert_eq!(
        read(
            &mut rt,
            "var x=new XMLHttpRequest(); x.open('POST','http://x/h'); var o=[]; try { x.setRequestHeader('X A','1'); o.push('no-throw'); } catch(e) { o.push(e.name); } try { x.setRequestHeader('X-A','a\\nb'); o.push('no-throw'); } catch(e) { o.push(e.name); } o.join(',')"
        ),
        "SyntaxError,SyntaxError"
    );
    // A forbidden header is dropped silently; an allowed one combines with ", ".
    rt.eval(
        r#"var x = new XMLHttpRequest();
           x.open('POST', 'http://x/h');
           x.setRequestHeader('Host', 'evil');
           x.setRequestHeader('Authorization', 'tok');
           x.setRequestHeader('X-Pink', 't1');
           x.setRequestHeader('X-Pink', 't2');
           x.send('body');"#,
    )
    .unwrap();
    rt.run_microtasks();
    let req = &seen.borrow()[0];
    let get = |n: &str| {
        req.headers
            .iter()
            .find(|(k, _)| k == n)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(
        get("host"),
        None,
        "a forbidden header never reaches the host"
    );
    assert_eq!(get("authorization").as_deref(), Some("tok"));
    assert_eq!(get("x-pink").as_deref(), Some("t1, t2"));
    assert_eq!(req.method, "POST");
    assert_eq!(req.body.as_deref(), Some(&b"body"[..]));
}

#[test]
fn an_async_send_walks_the_state_machine_and_fires_the_events() {
    let (mut rt, _seen) = echo_runtime();
    rt.eval(
        r#"var log = [];
           var x = new XMLHttpRequest();
           x.onreadystatechange = function() { log.push('rs' + x.readyState); };
           ['loadstart','progress','load','loadend','error','abort'].forEach(function(t) {
             x.addEventListener(t, function() { log.push(t); });
           });
           x.open('GET', 'http://x/a');
           x.send();"#,
    )
    .unwrap();
    rt.run_microtasks();
    assert_eq!(
        read(&mut rt, "log.join(',')"),
        "rs1,loadstart,rs2,rs3,progress,rs4,load,loadend"
    );
    assert_eq!(
        read(
            &mut rt,
            "x.readyState + '|' + x.status + '|' + x.statusText"
        ),
        "4|200|OK"
    );
    assert_eq!(read(&mut rt, "x.responseText"), "echo:GET:http://x/a");
    assert_eq!(read(&mut rt, "x.response"), "echo:GET:http://x/a");
    assert_eq!(read(&mut rt, "x.responseURL"), "http://x/a");
    assert_eq!(read(&mut rt, "x.responseXML === null"), "true");
    assert_eq!(read(&mut rt, "x.getResponseHeader('X-Echo')"), "1");
    assert_eq!(read(&mut rt, "String(x.getResponseHeader('nope'))"), "null");
    // Sorted, one per line, with the forbidden response header excluded.
    assert_eq!(
        read(&mut rt, "JSON.stringify(x.getAllResponseHeaders())"),
        r#""content-length: 19\r\ncontent-type: text/plain\r\nx-echo: 1\r\n""#
    );
}

#[test]
fn the_response_types_decode_the_same_bytes() {
    let (mut rt, _seen) = echo_runtime();
    // responseText demands responseType "" or "text".
    rt.eval(
        r#"var R = {};
           function go(t, done) {
             var x = new XMLHttpRequest();
             x.open('GET', 'http://x/a');
             x.responseType = t;
             x.onload = function() { done(x); };
             x.send();
             return x;
           }
           go('arraybuffer', function(x) { R.ab = x.response.byteLength; try { x.responseText; R.abText='no-throw'; } catch(e) { R.abText = e.name; } });
           go('blob', function(x) { R.blobSize = x.response.size; R.blobType = x.response.type; });
           go('text', function(x) { R.text = x.responseText; });
           go('document', function(x) { R.doc = String(x.response); });
           var j = new XMLHttpRequest();
           j.open('GET', 'http://x/a'); j.responseType = 'json';
           j.onload = function() { R.json = String(j.response); };
           j.send();
           var bad = new XMLHttpRequest();
           bad.responseType = 'nonsense';
           R.badType = bad.responseType;"#,
    )
    .unwrap();
    rt.run_microtasks();
    assert_eq!(read(&mut rt, "String(R.ab)"), "19");
    assert_eq!(read(&mut rt, "R.abText"), "InvalidStateError");
    assert_eq!(
        read(&mut rt, "R.blobSize + ',' + R.blobType"),
        "19,text/plain"
    );
    assert_eq!(read(&mut rt, "R.text"), "echo:GET:http://x/a");
    assert_eq!(read(&mut rt, "R.doc"), "null");
    assert_eq!(
        read(&mut rt, "R.json"),
        "null",
        "a non-JSON body yields null"
    );
    assert_eq!(
        read(&mut rt, "R.badType"),
        "",
        "an invalid enum value is ignored"
    );
    // responseType cannot change once the response is DONE.
    assert_eq!(
        read(
            &mut rt,
            "var d=new XMLHttpRequest(); d.open('GET','http://x/a'); d.send(); d._state=4; try { d.responseType='blob'; 'no-throw' } catch(e) { e.name }"
        ),
        "InvalidStateError"
    );
}

#[test]
fn override_mime_type_drives_the_decode() {
    let mut rt = Runtime::<BoaEngine>::new().unwrap();
    rt.set_fetch_handler(Box::new(Latin1));
    rt.eval(
        r#"var A = new XMLHttpRequest(); A.open('GET','http://x/a'); A.send();
           var B = new XMLHttpRequest(); B.open('GET','http://x/a');
           B.overrideMimeType('text/plain; charset=windows-1252'); B.send();"#,
    )
    .unwrap();
    rt.run_microtasks();
    // The same byte 0xE9 is U+FFFD as (default) UTF-8 and é as windows-1252.
    assert_eq!(read(&mut rt, "A.responseText.charCodeAt(0)"), "65533");
    assert_eq!(read(&mut rt, "B.responseText.charCodeAt(0)"), "233");
    // overrideMimeType is an InvalidStateError once DONE, and a SyntaxError on junk.
    assert_eq!(
        read(
            &mut rt,
            "try { A.overrideMimeType('text/plain'); 'no-throw' } catch(e) { e.name }"
        ),
        "InvalidStateError"
    );
    assert_eq!(
        read(
            &mut rt,
            "var C=new XMLHttpRequest(); C.open('GET','http://x/a'); try { C.overrideMimeType('nonsense'); 'no-throw' } catch(e) { e.name }"
        ),
        "SyntaxError"
    );
}

/// One byte, 0xE9, with no charset: UTF-8 by default, latin-1 under an override.
struct Latin1;
impl FetchHandler for Latin1 {
    fn fetch(&self, _req: FetchRequest) -> FetchOutcome {
        FetchOutcome {
            network_error: false,
            status: 200,
            status_text: "OK".to_owned(),
            response_type: "basic".to_owned(),
            url: "http://x/a".to_owned(),
            redirected: false,
            headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
            body: vec![0xE9],
        }
    }
}

#[test]
fn abort_runs_the_error_steps_and_resets_to_unsent() {
    let mut rt = Runtime::<BoaEngine>::new().unwrap();
    rt.set_fetch_handler(Box::new(NeverAnswers));
    rt.eval(
        r#"var log = [];
           var x = new XMLHttpRequest();
           x.onreadystatechange = function() { log.push('rs' + x.readyState); };
           ['loadstart','abort','loadend','error','load'].forEach(function(t) {
             x.addEventListener(t, function() { log.push(t); });
           });
           x.open('GET', 'http://x/a');
           x.send();
           x.abort();"#,
    )
    .unwrap();
    rt.run_microtasks();
    assert_eq!(
        read(&mut rt, "log.join(',')"),
        "rs1,loadstart,rs4,abort,loadend"
    );
    // The error steps leave a network error behind, then abort() resets to UNSENT.
    assert_eq!(
        read(
            &mut rt,
            "[x.readyState, x.status, x.responseText].join('|')"
        ),
        "0|0|"
    );
    // Aborting an UNSENT request fires nothing.
    assert_eq!(read(&mut rt, "log.length = 0; x.abort(); log.length"), "0");
}

#[test]
fn a_timeout_fires_the_timeout_event() {
    let mut rt = Runtime::<BoaEngine>::new().unwrap();
    rt.set_fetch_handler(Box::new(NeverAnswers));
    rt.eval(
        r#"var log = [];
           var x = new XMLHttpRequest();
           x.timeout = 10;
           ['timeout','loadend','error','load'].forEach(function(t) {
             x.addEventListener(t, function() { log.push(t); });
           });
           x.open('GET', 'http://x/a');
           x.send();"#,
    )
    .unwrap();
    rt.run_microtasks();
    assert_eq!(
        read(&mut rt, "log.join(',')"),
        "",
        "not yet: the timer has not run"
    );
    rt.run_timers(16, 50.0);
    rt.run_microtasks();
    assert_eq!(read(&mut rt, "log.join(',')"), "timeout,loadend");
    assert_eq!(read(&mut rt, "x.readyState"), "4");
}

#[test]
fn a_network_error_fires_the_error_event() {
    // No handler installed: every fetch is an inline network error.
    let mut rt = Runtime::<BoaEngine>::new().unwrap();
    rt.eval(
        r#"var log = [];
           var x = new XMLHttpRequest();
           ['loadstart','error','loadend','load'].forEach(function(t) {
             x.addEventListener(t, function() { log.push(t); });
           });
           x.open('GET', 'http://x/a');
           x.send();"#,
    )
    .unwrap();
    rt.run_microtasks();
    assert_eq!(read(&mut rt, "log.join(',')"), "loadstart,error,loadend");
    assert_eq!(read(&mut rt, "[x.readyState, x.status].join('|')"), "4|0");
}

#[test]
fn a_synchronous_send_returns_with_the_response_in_hand() {
    let (mut rt, seen) = echo_runtime();
    // No microtask pumping: send() answers in place.
    rt.eval(
        r#"var log = [];
           var x = new XMLHttpRequest();
           x.onreadystatechange = function() { log.push('rs' + x.readyState); };
           ['load','loadend'].forEach(function(t) { x.addEventListener(t, function() { log.push(t); }); });
           x.open('GET', 'http://x/s', false);
           x.send();
           var text = x.responseText;"#,
    )
    .unwrap();
    assert_eq!(read(&mut rt, "text"), "echo:GET:http://x/s");
    assert_eq!(read(&mut rt, "log.join(',')"), "rs1,rs4,load,loadend");
    assert_eq!(read(&mut rt, "x.status"), "200");
    assert_eq!(seen.borrow().len(), 1, "one request, on the one host seam");
    // A synchronous request rejects timeout and responseType.
    assert_eq!(
        read(
            &mut rt,
            "var y=new XMLHttpRequest(); y.open('GET','http://x/s',false); try { y.timeout=1; 'no-throw' } catch(e) { e.name }"
        ),
        "InvalidAccessError"
    );
    assert_eq!(
        read(
            &mut rt,
            "var z=new XMLHttpRequest(); z.open('GET','http://x/s',false); try { z.responseType='blob'; 'no-throw' } catch(e) { e.name }"
        ),
        "InvalidAccessError"
    );
    // ... and open(sync) is rejected when one is already set.
    assert_eq!(
        read(
            &mut rt,
            "var w=new XMLHttpRequest(); w.timeout=5; try { w.open('GET','http://x/s',false); 'no-throw' } catch(e) { e.name }"
        ),
        "InvalidAccessError"
    );
}

#[test]
fn a_synchronous_network_error_throws() {
    // No handler: the blocking sink answers with a network error, which a
    // synchronous send throws rather than reporting through an event.
    let mut rt = Runtime::<BoaEngine>::new().unwrap();
    assert_eq!(
        read(
            &mut rt,
            "var x=new XMLHttpRequest(); x.open('GET','http://x/s',false); try { x.send(); 'no-throw' } catch(e) { e.name }"
        ),
        "NetworkError"
    );
    assert_eq!(read(&mut rt, "x.readyState"), "4");
}

#[test]
fn with_credentials_maps_to_the_request_credentials_mode() {
    let (mut rt, seen) = echo_runtime();
    rt.eval(
        r#"var a = new XMLHttpRequest(); a.open('GET','http://x/1'); a.send();
           var b = new XMLHttpRequest(); b.open('GET','http://x/2'); b.withCredentials = true; b.send();"#,
    )
    .unwrap();
    rt.run_microtasks();
    let s = seen.borrow();
    assert_eq!(s[0].credentials, "same-origin");
    assert_eq!(s[1].credentials, "include");
    drop(s);
    // withCredentials is an InvalidStateError once the request is in flight.
    assert_eq!(
        read(
            &mut rt,
            "try { b.withCredentials = false; 'no-throw' } catch(e) { e.name }"
        ),
        "InvalidStateError"
    );
}

#[test]
fn the_upload_fires_its_own_progress_events() {
    let (mut rt, _seen) = echo_runtime();
    rt.eval(
        r#"var log = [];
           var x = new XMLHttpRequest();
           ['loadstart','progress','load','loadend'].forEach(function(t) {
             x.upload.addEventListener(t, function(e) { log.push('u:' + t + ':' + e.loaded + '/' + e.total); });
           });
           x.open('POST', 'http://x/u');
           x.send('12345');"#,
    )
    .unwrap();
    rt.run_microtasks();
    assert_eq!(
        read(&mut rt, "log.join(',')"),
        "u:loadstart:0/5,u:progress:5/5,u:load:5/5,u:loadend:5/5"
    );
    // A request with no upload listener fires none of them.
    rt.eval(
        r#"var q = 0;
           var y = new XMLHttpRequest();
           y.open('POST', 'http://x/u');
           y.send('12345');
           y.upload.addEventListener('load', function() { q++; });"#,
    )
    .unwrap();
    rt.run_microtasks();
    assert_eq!(read(&mut rt, "String(q)"), "0");
}
