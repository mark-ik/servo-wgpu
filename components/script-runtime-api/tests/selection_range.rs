// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `Range`, `StaticRange` and `Selection` over the scripted arena: boundary
//! validation and ordering, the content operations, live range updating from
//! the bootstrap's mutation funnel, the Selection API's single live range, and
//! the geometry seam. Each body runs against both backends.

use genet_static_dom::StaticDocument;
use script_engine_api::ScriptEngine;
use script_runtime_api::{Runtime, SelectionHandler};

fn read<E: ScriptEngine>(rt: &mut Runtime<E>, expr: &str) -> String {
    let v = rt.eval(expr).expect("eval");
    rt.value_to_string(&v).expect("stringify")
}

fn runtime<E: ScriptEngine>() -> Runtime<E> {
    let mut rt = Runtime::<E>::new().expect("runtime");
    rt.load_dom(&StaticDocument::parse(
        "<html><body><div id='host'><span id='a'>one</span><span id='b'>two</span></div></body></html>",
    ));
    rt.eval("function thrown(fn){ try { fn(); return 'no-throw'; } catch(e){ return e.name; } }")
        .expect("helper");
    rt
}

/// The interface objects, their class strings and the constructors.
fn range_interfaces<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    assert_eq!(read(&mut rt, "typeof Range"), "function");
    assert_eq!(read(&mut rt, "typeof StaticRange"), "function");
    assert_eq!(read(&mut rt, "typeof AbstractRange"), "function");
    assert_eq!(read(&mut rt, "typeof Selection"), "function");
    assert_eq!(
        read(&mut rt, "Object.prototype.toString.call(new Range())"),
        "[object Range]"
    );
    assert_eq!(
        read(&mut rt, "Object.prototype.toString.call(getSelection())"),
        "[object Selection]"
    );
    assert_eq!(
        read(&mut rt, "new Range() instanceof AbstractRange"),
        "true"
    );
    assert_eq!(
        read(&mut rt, "thrown(function(){ new AbstractRange(); })"),
        "TypeError"
    );
    assert_eq!(
        read(&mut rt, "thrown(function(){ new Selection(); })"),
        "TypeError"
    );
    // A fresh range and `document.createRange()` are both collapsed at (document, 0).
    assert_eq!(
        read(
            &mut rt,
            "var r = document.createRange(); [r.collapsed, r.startContainer === document, r.startOffset].join(',')"
        ),
        "true,true,0"
    );
    assert_eq!(
        read(
            &mut rt,
            "[Range.START_TO_START, Range.START_TO_END, Range.END_TO_END, Range.END_TO_START].join(',')"
        ),
        "0,1,2,3"
    );
    // StaticRange is inert: it takes an init dictionary and never updates.
    assert_eq!(
        read(
            &mut rt,
            "var t = document.getElementById('a').firstChild;\
             var s = new StaticRange({ startContainer: t, startOffset: 1, endContainer: t, endOffset: 2 });\
             [s.startOffset, s.endOffset, s.collapsed, s instanceof AbstractRange].join(',')"
        ),
        "1,2,false,true"
    );
}

/// `ProcessingInstruction`, the node type both directories' shared `common.js`
/// builds before any Range or Selection test runs.
fn processing_instruction<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    assert_eq!(read(&mut rt, "typeof ProcessingInstruction"), "function");
    rt.eval("var pi = document.createProcessingInstruction('somePI', 'chirp');")
        .expect("create");
    assert_eq!(
        read(
            &mut rt,
            "[pi.nodeType, pi.nodeName, pi.target, pi.data, pi.length].join(',')"
        ),
        "7,somePI,somePI,chirp,5"
    );
    assert_eq!(read(&mut rt, "pi instanceof CharacterData"), "true");
    assert_eq!(
        read(&mut rt, "Object.prototype.toString.call(pi)"),
        "[object ProcessingInstruction]"
    );
    assert_eq!(
        read(
            &mut rt,
            "thrown(function(){ document.createProcessingInstruction('a', 'x?>y'); })"
        ),
        "InvalidCharacterError"
    );
    // A PI carries a node name but is not an element: `documentElement` must
    // not mistake one for `<html>`.
    rt.eval("document.getElementById('host').appendChild(pi);")
        .expect("append");
    assert_eq!(read(&mut rt, "document.documentElement.nodeName"), "HTML");
    // A range over a PI's data behaves like any other character data.
    assert_eq!(
        read(
            &mut rt,
            "var r = document.createRange(); r.setStart(pi, 1); r.setEnd(pi, 4);             [r.toString(), r.cloneContents().childNodes.length].join('|')"
        ),
        "|1"
    );
    assert_eq!(read(&mut rt, "r.extractContents().firstChild.data"), "hir");
    assert_eq!(read(&mut rt, "pi.data"), "cp");
}

/// Boundary validation, the collapse-on-cross rule and the comparisons.
fn boundaries_and_comparison<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    rt.eval(
        "var host = document.getElementById('host');\
         var a = document.getElementById('a').firstChild;\
         var b = document.getElementById('b').firstChild;\
         var r = document.createRange();",
    )
    .expect("setup");
    assert_eq!(
        read(&mut rt, "thrown(function(){ r.setStart(a, 99); })"),
        "IndexSizeError"
    );
    // Setting the start past the end drags the end with it.
    rt.eval("r.setStart(a, 1); r.setEnd(a, 2); r.setStart(b, 1);")
        .expect("set");
    assert_eq!(
        read(
            &mut rt,
            "[r.startContainer === b, r.endContainer === b, r.collapsed].join(',')"
        ),
        "true,true,true"
    );
    rt.eval("r.setStart(a, 0); r.setEnd(b, 3);").expect("set");
    assert_eq!(read(&mut rt, "r.commonAncestorContainer === host"), "true");
    assert_eq!(read(&mut rt, "r.toString()"), "onetwo");
    // comparePoint / isPointInRange / intersectsNode.
    assert_eq!(read(&mut rt, "r.comparePoint(a, 0)"), "0");
    // (host, 0) is before span#a, which contains the start: the point is before
    // the range even though the range's common ancestor is host.
    assert_eq!(read(&mut rt, "r.comparePoint(host, 0)"), "-1");
    assert_eq!(read(&mut rt, "r.comparePoint(host, 2)"), "1");
    assert_eq!(read(&mut rt, "r.isPointInRange(b, 3)"), "true");
    assert_eq!(
        read(&mut rt, "r.intersectsNode(document.getElementById('b'))"),
        "true"
    );
    assert_eq!(
        read(&mut rt, "r.intersectsNode(document.createElement('p'))"),
        "false"
    );
    // compareBoundaryPoints against a strictly inner range.
    rt.eval("var inner = document.createRange(); inner.setStart(a, 1); inner.setEnd(b, 1);")
        .expect("inner");
    assert_eq!(
        read(
            &mut rt,
            "r.compareBoundaryPoints(Range.START_TO_START, inner)"
        ),
        "-1"
    );
    assert_eq!(
        read(&mut rt, "r.compareBoundaryPoints(Range.END_TO_END, inner)"),
        "1"
    );
    assert_eq!(
        read(
            &mut rt,
            "thrown(function(){ r.compareBoundaryPoints(9, inner); })"
        ),
        "NotSupportedError"
    );
    // selectNode / selectNodeContents.
    rt.eval("r.selectNode(document.getElementById('a'));")
        .expect("selectNode");
    assert_eq!(
        read(
            &mut rt,
            "[r.startContainer === host, r.startOffset, r.endOffset].join(',')"
        ),
        "true,0,1"
    );
    rt.eval("r.selectNodeContents(host);")
        .expect("selectNodeContents");
    assert_eq!(
        read(&mut rt, "[r.startOffset, r.endOffset].join(',')"),
        "0,2"
    );
    // cloneRange is a separate live object.
    assert_eq!(
        read(
            &mut rt,
            "var c = r.cloneRange(); c.collapse(true); [r.collapsed, c.collapsed].join(',')"
        ),
        "false,true"
    );
}

/// deleteContents / extractContents / cloneContents / insertNode /
/// surroundContents over a partially contained text pair.
fn content_operations<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    rt.eval(
        "var host = document.getElementById('host');\
         var r = document.createRange();\
         r.setStart(document.getElementById('a').firstChild, 1);\
         r.setEnd(document.getElementById('b').firstChild, 1);\
         var f = r.cloneContents();",
    )
    .expect("clone");
    // The clone carries the partial head, the partial tail, and nothing else.
    assert_eq!(read(&mut rt, "f.childNodes.length"), "2");
    assert_eq!(read(&mut rt, "f.textContent"), "net");
    assert_eq!(read(&mut rt, "host.textContent"), "onetwo");
    // Extracting takes the same content out of the document.
    assert_eq!(read(&mut rt, "r.extractContents().textContent"), "net");
    assert_eq!(read(&mut rt, "host.textContent"), "owo");
    assert_eq!(read(&mut rt, "r.collapsed"), "true");
    // deleteContents on a single text node is a replace-data.
    rt.eval(
        "var t = document.createTextNode('abcdef'); host.appendChild(t);\
         var d = document.createRange(); d.setStart(t, 1); d.setEnd(t, 4); d.deleteContents();",
    )
    .expect("delete");
    assert_eq!(read(&mut rt, "t.data"), "aef");
    // insertNode splits the start text node and lands between the halves.
    rt.eval(
        "var i = document.createRange(); i.setStart(t, 1); i.collapse(true);\
             i.insertNode(document.createElement('br'));",
    )
    .expect("insertNode");
    assert_eq!(read(&mut rt, "t.data"), "a");
    assert_eq!(read(&mut rt, "t.nextSibling.nodeName"), "BR");
    // surroundContents wraps the selected text.
    rt.eval(
        "var w = document.createElement('div'); w.innerHTML = 'hello';\
         host.appendChild(w);\
         var s = document.createRange(); s.setStart(w.firstChild, 1); s.setEnd(w.firstChild, 4);\
         s.surroundContents(document.createElement('em'));",
    )
    .expect("surround");
    assert_eq!(read(&mut rt, "w.innerHTML"), "h<em>ell</em>o");
    // A range partially containing an element cannot be surrounded.
    rt.eval(
        "var p = document.createRange(); p.setStart(w.firstChild, 0);\
             p.setEnd(w.childNodes[1].firstChild, 1);",
    )
    .expect("partial");
    assert_eq!(
        read(
            &mut rt,
            "thrown(function(){ p.surroundContents(document.createElement('b')); })"
        ),
        "InvalidStateError"
    );
    // The partial-containment check is step 1, so a fully contained range is
    // needed to reach the new-parent type check.
    rt.eval("var q = document.createRange(); q.selectNodeContents(w);")
        .expect("full");
    assert_eq!(
        read(
            &mut rt,
            "thrown(function(){ q.surroundContents(document.createDocumentFragment()); })"
        ),
        "InvalidNodeTypeError"
    );
}

/// The live range steps, driven from the bootstrap's own mutation funnel.
fn live_range_updating<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    rt.eval(
        "var host = document.getElementById('host');\
         var r = document.createRange(); r.setStart(host, 1); r.setEnd(host, 2);",
    )
    .expect("setup");
    // Inserting before the range's start pushes both offsets along.
    rt.eval("host.insertBefore(document.createElement('i'), host.firstChild);")
        .expect("insert");
    assert_eq!(
        read(&mut rt, "[r.startOffset, r.endOffset].join(',')"),
        "2,3"
    );
    // Removing that node pulls them back.
    rt.eval("host.removeChild(host.firstChild);")
        .expect("remove");
    assert_eq!(
        read(&mut rt, "[r.startOffset, r.endOffset].join(',')"),
        "1,2"
    );
    // A boundary inside a removed subtree collapses to the removal point.
    rt.eval(
        "var inner = document.createRange();\
         inner.selectNodeContents(document.getElementById('b').firstChild);\
         var b = document.getElementById('b'); host.removeChild(b);",
    )
    .expect("remove subtree");
    assert_eq!(
        read(
            &mut rt,
            "[inner.startContainer === host, inner.startOffset, inner.collapsed].join(',')"
        ),
        "true,1,true"
    );
    // replace-data steps: an offset inside the replaced run clamps to its start,
    // one after it shifts by the length delta.
    rt.eval(
        "var t = document.getElementById('a').firstChild;\
         t.data = 'abcdef';\
         var x = document.createRange(); x.setStart(t, 2); x.setEnd(t, 5);\
         t.replaceData(1, 2, 'ZZZZ');",
    )
    .expect("replace data");
    assert_eq!(read(&mut rt, "t.data"), "aZZZZdef");
    assert_eq!(
        read(&mut rt, "[x.startOffset, x.endOffset].join(',')"),
        "1,7"
    );
    // splitText moves a boundary past the split onto the new node.
    rt.eval(
        "var y = document.createRange(); y.setStart(t, 6); y.setEnd(t, 7);\
             var tail = t.splitText(4);",
    )
    .expect("split");
    assert_eq!(read(&mut rt, "[t.data, tail.data].join('|')"), "aZZZ|Zdef");
    assert_eq!(
        read(
            &mut rt,
            "[y.startContainer === tail, y.startOffset, y.endOffset].join(',')"
        ),
        "true,2,3"
    );
    // `textContent` is a replace-all: every boundary under it lands at offset 0.
    rt.eval(
        "var z = document.createRange(); z.selectNodeContents(host); host.textContent = 'gone';",
    )
    .expect("replace all");
    assert_eq!(
        read(&mut rt, "[z.startOffset, z.endOffset].join(',')"),
        "0,0"
    );
}

/// The Selection API over its single live range, and the `selectionchange`
/// event on the document.
fn selection_api<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    rt.eval(
        "var host = document.getElementById('host');\
         var a = document.getElementById('a').firstChild;\
         var b = document.getElementById('b').firstChild;\
         var sel = getSelection(); var fired = 0;\
         document.addEventListener('selectionchange', function(){ fired++; });",
    )
    .expect("setup");
    assert_eq!(
        read(&mut rt, "getSelection() === document.getSelection()"),
        "true"
    );
    assert_eq!(
        read(
            &mut rt,
            "[sel.rangeCount, sel.type, sel.direction, sel.isCollapsed, String(sel.anchorNode)].join(',')"
        ),
        "0,None,none,true,null"
    );
    assert_eq!(
        read(&mut rt, "thrown(function(){ sel.getRangeAt(0); })"),
        "IndexSizeError"
    );
    assert_eq!(
        read(&mut rt, "thrown(function(){ sel.collapseToStart(); })"),
        "InvalidStateError"
    );
    // setBaseAndExtent forward, then backward.
    rt.eval("sel.setBaseAndExtent(a, 1, b, 2);").expect("sbae");
    assert_eq!(
        read(
            &mut rt,
            "[sel.rangeCount, sel.type, sel.direction, sel.anchorOffset, sel.focusOffset].join(',')"
        ),
        "1,Range,forward,1,2"
    );
    assert_eq!(read(&mut rt, "sel.toString()"), "netw");
    rt.eval("sel.setBaseAndExtent(b, 2, a, 1);")
        .expect("sbae back");
    assert_eq!(
        read(
            &mut rt,
            "[sel.direction, sel.anchorNode === b, sel.focusNode === a].join(',')"
        ),
        "backward,true,true"
    );
    // The range handed out is the selection's own live object.
    assert_eq!(
        read(&mut rt, "sel.getRangeAt(0) === sel.getRangeAt(0)"),
        "true"
    );
    assert_eq!(read(&mut rt, "sel.getRangeAt(0).toString()"), "netw");
    // extend past the anchor flips the direction.
    rt.eval("sel.setBaseAndExtent(a, 1, a, 2); sel.extend(a, 0);")
        .expect("extend");
    assert_eq!(
        read(
            &mut rt,
            "[sel.direction, sel.anchorOffset, sel.focusOffset].join(',')"
        ),
        "backward,1,0"
    );
    // containsNode, collapse, selectAllChildren, removeAllRanges.
    rt.eval("sel.selectAllChildren(host);")
        .expect("selectAllChildren");
    assert_eq!(
        read(&mut rt, "sel.containsNode(document.getElementById('a'))"),
        "true"
    );
    assert_eq!(
        read(&mut rt, "sel.containsNode(document.createElement('p'))"),
        "false"
    );
    rt.eval("sel.collapse(a, 2);").expect("collapse");
    assert_eq!(
        read(
            &mut rt,
            "[sel.type, sel.isCollapsed, sel.anchorOffset].join(',')"
        ),
        "Caret,true,2"
    );
    rt.eval("sel.removeAllRanges();").expect("removeAllRanges");
    assert_eq!(
        read(&mut rt, "[sel.rangeCount, sel.type].join(',')"),
        "0,None"
    );
    // addRange takes an outside range; a second one is ignored per spec.
    rt.eval(
        "var r = document.createRange(); r.setStart(a, 0); r.setEnd(a, 3); sel.addRange(r);\
             var r2 = document.createRange(); r2.selectNodeContents(b); sel.addRange(r2);",
    )
    .expect("addRange");
    assert_eq!(
        read(
            &mut rt,
            "[sel.rangeCount, sel.getRangeAt(0) === r].join(',')"
        ),
        "1,true"
    );
    assert_eq!(
        read(&mut rt, "thrown(function(){ sel.removeRange(r2); })"),
        "NotFoundError"
    );
    // deleteFromDocument goes through the live range.
    rt.eval("sel.deleteFromDocument();")
        .expect("deleteFromDocument");
    assert_eq!(
        read(&mut rt, "document.getElementById('a').textContent"),
        ""
    );
    // selectionchange is a task, not synchronous; it has landed by now.
    rt.run_microtasks();
    assert_eq!(read(&mut rt, "fired > 0"), "true");
}

/// The geometry seam: empty without a handler, and the handler's rectangles
/// with one.
struct Rects;
impl SelectionHandler for Rects {
    fn range_rects(&self, _start: u64, _so: u32, _end: u64, _eo: u32) -> Vec<[f32; 4]> {
        vec![[1.0, 2.0, 3.0, 4.0], [10.0, 2.0, 5.0, 4.0]]
    }
    fn set_visual_selection(&self, _range: Option<(u64, u32, u64, u32)>) {}
}

fn range_geometry<E: ScriptEngine>() {
    let mut rt = runtime::<E>();
    rt.eval(
        "var r = document.createRange();\
         r.setStart(document.getElementById('a').firstChild, 0);\
         r.setEnd(document.getElementById('a').firstChild, 3);",
    )
    .expect("setup");
    // No layout bound: an empty list and a zero rect, per the spec's answer for
    // a range with no boxes.
    assert_eq!(read(&mut rt, "r.getClientRects().length"), "0");
    assert_eq!(
        read(
            &mut rt,
            "var b = r.getBoundingClientRect(); [b.x, b.y, b.width, b.height].join(',')"
        ),
        "0,0,0,0"
    );
    rt.set_selection_handler(Box::new(Rects));
    assert_eq!(read(&mut rt, "r.getClientRects().length"), "2");
    assert_eq!(
        read(
            &mut rt,
            "var b2 = r.getBoundingClientRect(); [b2.x, b2.y, b2.width, b2.height].join(',')"
        ),
        "1,2,14,4"
    );
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
    range_interfaces => (range_interfaces_on_boa, range_interfaces_on_nova),
    processing_instruction => (processing_instruction_on_boa, processing_instruction_on_nova),
    boundaries_and_comparison => (boundaries_on_boa, boundaries_on_nova),
    content_operations => (content_operations_on_boa, content_operations_on_nova),
    live_range_updating => (live_range_updating_on_boa, live_range_updating_on_nova),
    selection_api => (selection_api_on_boa, selection_api_on_nova),
    range_geometry => (range_geometry_on_boa, range_geometry_on_nova),
}
