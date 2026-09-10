// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `MutationObserver`'s two native sinks: the observing switch and the drain of
//! the arena's observer record.
//!
//! The registry, the option validation, the interested-observer walk and the
//! "notify mutation observers" microtask all live in the JS bootstrap; Rust only
//! carries facts the arena knows and JS cannot re-derive after the fact (the
//! sibling either side of a removal, an attribute's or text node's old value,
//! and the target's ancestor chain at mutation time).

use super::*;
use genet_scripted_dom::ObservedMutation;

/// Escape a field for the tab/newline-delimited record protocol. Same shape as
/// the inline-style protocol next door: text values may contain either.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

fn ids(list: &[NodeId], dom: &ScriptedDom) -> String {
    list.iter()
        .filter(|&&id| dom.is_live(id))
        .map(|id| id.raw().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn id_or_blank(id: Option<NodeId>, dom: &ScriptedDom) -> String {
    match id {
        Some(id) if dom.is_live(id) => id.raw().to_string(),
        _ => String::new(),
    }
}

fn old_field(old: Option<&String>) -> String {
    match old {
        Some(value) => format!("1\t{}", escape(value)),
        None => "0\t".to_owned(),
    }
}

/// Encode the drained batch. One record per line, fields tab-separated:
///
/// - `C` target prev next added removed ancestors
/// - `A` target localName namespace hasOld old ancestors
/// - `D` target hasOld old ancestors
pub(super) fn encode(records: &[ObservedMutation], dom: &ScriptedDom) -> String {
    let mut out = String::new();
    for record in records {
        let line = match record {
            ObservedMutation::ChildList {
                target,
                added,
                removed,
                previous_sibling,
                next_sibling,
                ancestors,
            } => {
                if !dom.is_live(*target) {
                    continue;
                }
                format!(
                    "C\t{}\t{}\t{}\t{}\t{}\t{}",
                    target.raw(),
                    id_or_blank(*previous_sibling, dom),
                    id_or_blank(*next_sibling, dom),
                    ids(added, dom),
                    ids(removed, dom),
                    ids(ancestors, dom),
                )
            },
            ObservedMutation::Attributes {
                target,
                name,
                old_value,
                ancestors,
            } => {
                if !dom.is_live(*target) {
                    continue;
                }
                format!(
                    "A\t{}\t{}\t{}\t{}\t{}",
                    target.raw(),
                    escape(&name.local),
                    escape(&name.ns),
                    old_field(old_value.as_ref()),
                    ids(ancestors, dom),
                )
            },
            ObservedMutation::CharacterData {
                target,
                old_value,
                ancestors,
            } => {
                if !dom.is_live(*target) {
                    continue;
                }
                format!(
                    "D\t{}\t{}\t{}",
                    target.raw(),
                    old_field(old_value.as_ref()),
                    ids(ancestors, dom),
                )
            },
        };
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&line);
    }
    out
}

/// `__moObserving(flag)` — turn the arena's observer record on or off. The
/// bootstrap flips it on with the first registration and off when the last one
/// goes, so a document nobody observes keeps its old cost exactly.
pub(crate) struct SetObserving;
impl<E: ScriptEngine> NativeFn<E> for SetObserving {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let arg = cx.arg(0);
        let on = cx.value_to_string(&arg)? == "1";
        with_dom::<E, _>(cx, |dom| dom.set_observing(on));
        Ok(cx.undefined())
    }
}

/// `__moGroup(flag)` — open or close a coalescing group, so a compound DOM
/// operation built from several arena mutations (`replaceChild`) reaches script
/// as the one record the spec specifies.
pub(crate) struct SetGroup;
impl<E: ScriptEngine> NativeFn<E> for SetGroup {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let arg = cx.arg(0);
        let on = cx.value_to_string(&arg)? == "1";
        with_dom::<E, _>(cx, |dom| dom.set_observer_group(on));
        Ok(cx.undefined())
    }
}

/// `__moTake()` — drain the arena's observer record into the line protocol
/// above. Empty string when nothing is pending.
pub(crate) struct TakeRecords;
impl<E: ScriptEngine> NativeFn<E> for TakeRecords {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let encoded = with_dom::<E, _>(cx, |dom| {
            let records = dom.take_observed();
            encode(&records, dom)
        })
        .unwrap_or_default();
        cx.make_string(&encoded)
    }
}
