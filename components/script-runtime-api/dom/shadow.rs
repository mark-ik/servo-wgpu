// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Shadow-tree and `<template>` command sinks.
//!
//! The arena owns the shadow root, its init dictionary and the slot assignment
//! table; these natives are the narrow window the bootstrap reads and writes
//! them through. Mode policy (`element.shadowRoot` is `null` for a closed root)
//! lives in the bootstrap, not here: layout, serialization and the reflector
//! policy all need the root regardless of mode, so the arena answers honestly
//! and the DOM surface withholds.

use genet_scripted_dom::{
    AttachShadowError, ShadowRootInit, ShadowRootMode, SlotAssignmentMode, may_host_shadow_tree,
};

use super::*;

/// Ids as the bootstrap passes and receives them: a comma-separated list of raw
/// arena indices. The count/item pair used elsewhere would need one `RefCell`
/// borrow per element; these lists are short and read whole.
fn parse_raw_ids(list: &str) -> Vec<NodeId> {
    list.split(',')
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<usize>().ok())
        .map(NodeId::from_raw)
        .collect()
}

fn join_raw_ids(ids: &[NodeId]) -> String {
    ids.iter()
        .map(|id| id.raw().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// `__mayHostShadow(localName)` → `"1"` if a shadow root may be attached to an
/// element with that local name.
pub(crate) struct MayHostShadow;
impl<E: ScriptEngine> NativeFn<E> for MayHostShadow {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let arg = cx.arg(0);
        let local = cx.value_to_string(&arg)?;
        cx.make_string(if may_host_shadow_tree(&local) {
            "1"
        } else {
            "0"
        })
    }
}

/// `__attachShadow(host, mode, delegatesFocus, clonable, serializable,
/// slotAssignment)` → the shadow root's reflector, or the name of the
/// `DOMException` the DOM requires, as a string.
pub(crate) struct AttachShadow;
impl<E: ScriptEngine> NativeFn<E> for AttachShadow {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let host = cx.arg(0);
        let Some(raw) = cx.reflector_data(&host) else {
            return cx.make_string("NotSupportedError");
        };
        let mode_v = cx.arg(1);
        let mode = cx.value_to_string(&mode_v)?;
        let Some(mode) = ShadowRootMode::parse(&mode) else {
            return cx.make_string("TypeError");
        };
        let flag = |cx: &mut E::CallCx<'_>, index: usize| -> Result<bool, E::Error> {
            let value = cx.arg(index);
            Ok(cx.value_to_string(&value)? == "1")
        };
        let delegates_focus = flag(cx, 2)?;
        let clonable = flag(cx, 3)?;
        let serializable = flag(cx, 4)?;
        let assignment_v = cx.arg(5);
        let assignment = cx.value_to_string(&assignment_v)?;
        let init = ShadowRootInit {
            mode,
            delegates_focus,
            clonable,
            serializable,
            slot_assignment: SlotAssignmentMode::parse(&assignment)
                .unwrap_or(SlotAssignmentMode::Named),
        };
        let host_id = NodeId::from_raw(raw as usize);
        let outcome = with_dom::<E, _>(cx, |dom| {
            let local = dom
                .element_name(host_id)
                .map(|name| name.local.as_ref().to_owned());
            match local {
                Some(local) if may_host_shadow_tree(&local) => dom.attach_shadow(host_id, init),
                _ => Err(AttachShadowError::NotSupported),
            }
        });
        match outcome {
            Some(Ok(root)) => reflect_pinned::<E>(cx, root.raw() as u64),
            Some(Err(AttachShadowError::NotSupported)) => cx.make_string("NotSupportedError"),
            Some(Err(AttachShadowError::AlreadyAttached)) => cx.make_string("NotSupportedError"),
            None => cx.make_string("NotSupportedError"),
        }
    }
}

/// `__shadowRoot(host)` → the shadow root's reflector regardless of mode, or
/// null. The bootstrap decides what `element.shadowRoot` may reveal.
pub(crate) struct ShadowRootOf;
impl<E: ScriptEngine> NativeFn<E> for ShadowRootOf {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let host = cx.arg(0);
        let Some(raw) = cx.reflector_data(&host) else {
            return Ok(cx.make_null());
        };
        let found = with_dom::<E, _>(cx, |dom| dom.shadow_root_of(NodeId::from_raw(raw as usize)))
            .flatten();
        match found {
            Some(root) => reflect_pinned::<E>(cx, root.raw() as u64),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__shadowHost(root)` → the host element's reflector, or null.
pub(crate) struct ShadowHostOf;
impl<E: ScriptEngine> NativeFn<E> for ShadowHostOf {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let root = cx.arg(0);
        let Some(raw) = cx.reflector_data(&root) else {
            return Ok(cx.make_null());
        };
        let found = with_dom::<E, _>(cx, |dom| dom.shadow_host_of(NodeId::from_raw(raw as usize)))
            .flatten();
        match found {
            Some(host) => reflect_pinned::<E>(cx, host.raw() as u64),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__shadowInit(root)` → `"mode,delegatesFocus,clonable,serializable,slotAssignment"`,
/// or `""` when `root` is not a shadow root. One native for the whole
/// dictionary: every field is a fixed read and script asks for them together.
pub(crate) struct ShadowInitOf;
impl<E: ScriptEngine> NativeFn<E> for ShadowInitOf {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let root = cx.arg(0);
        let Some(raw) = cx.reflector_data(&root) else {
            return cx.make_string("");
        };
        let init =
            with_dom::<E, _>(cx, |dom| dom.shadow_init(NodeId::from_raw(raw as usize))).flatten();
        let text = match init {
            Some(init) => format!(
                "{},{},{},{},{}",
                init.mode.as_str(),
                u8::from(init.delegates_focus),
                u8::from(init.clonable),
                u8::from(init.serializable),
                init.slot_assignment.as_str(),
            ),
            None => String::new(),
        };
        cx.make_string(&text)
    }
}

/// `__assignedSlot(node)` → the slot's reflector, or null.
pub(crate) struct AssignedSlotOf;
impl<E: ScriptEngine> NativeFn<E> for AssignedSlotOf {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let node = cx.arg(0);
        let Some(raw) = cx.reflector_data(&node) else {
            return Ok(cx.make_null());
        };
        let found = with_dom::<E, _>(cx, |dom| {
            dom.assigned_slot_of(NodeId::from_raw(raw as usize))
        })
        .flatten();
        match found {
            Some(slot) => reflect_pinned::<E>(cx, slot.raw() as u64),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__assignedNodes(slot)` → the assigned node ids, comma-separated.
pub(crate) struct AssignedNodesOf;
impl<E: ScriptEngine> NativeFn<E> for AssignedNodesOf {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let slot = cx.arg(0);
        let Some(raw) = cx.reflector_data(&slot) else {
            return cx.make_string("");
        };
        let ids = with_dom::<E, _>(cx, |dom| {
            dom.assigned_nodes_of(NodeId::from_raw(raw as usize))
        })
        .unwrap_or_default();
        let text = join_raw_ids(&ids);
        cx.make_string(&text)
    }
}

/// `__slotAssign(slot, "id,id")` — `HTMLSlotElement.assign()`.
pub(crate) struct SlotAssign;
impl<E: ScriptEngine> NativeFn<E> for SlotAssign {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let slot = cx.arg(0);
        let Some(raw) = cx.reflector_data(&slot) else {
            return Ok(cx.undefined());
        };
        let list_v = cx.arg(1);
        let list = cx.value_to_string(&list_v)?;
        let nodes = parse_raw_ids(&list);
        with_dom::<E, _>(cx, |dom| {
            dom.set_manual_assignment(NodeId::from_raw(raw as usize), nodes)
        });
        Ok(cx.undefined())
    }
}

/// `__takeSlotChanges()` → the ids of slots whose assignment moved since the
/// last call, comma-separated. The bootstrap fires one `slotchange` each.
pub(crate) struct TakeSlotChanges;
impl<E: ScriptEngine> NativeFn<E> for TakeSlotChanges {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let ids = with_dom::<E, _>(cx, |dom| dom.take_slot_changes()).unwrap_or_default();
        let text = join_raw_ids(&ids);
        cx.make_string(&text)
    }
}

/// `__templateContent(template)` → the contents fragment's reflector, minting
/// the fragment (and the shared inert owner document behind it) on first ask.
pub(crate) struct TemplateContent;
impl<E: ScriptEngine> NativeFn<E> for TemplateContent {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let template = cx.arg(0);
        let Some(raw) = cx.reflector_data(&template) else {
            return Ok(cx.make_null());
        };
        let id = NodeId::from_raw(raw as usize);
        let fragment = with_dom::<E, _>(cx, |dom| {
            let is_template = dom
                .element_name(id)
                .is_some_and(|name| name.local.as_ref() == "template");
            is_template.then(|| dom.ensure_template_contents(id))
        })
        .flatten();
        match fragment {
            Some(fragment) => reflect_pinned::<E>(cx, fragment.raw() as u64),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__templateOwnerDocument()` → the shared inert template-contents owner
/// document, minting it if no template has asked yet. `content.ownerDocument`.
pub(crate) struct TemplateOwnerDocument;
impl<E: ScriptEngine> NativeFn<E> for TemplateOwnerDocument {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        match with_dom::<E, _>(cx, |dom| dom.template_owner_document()) {
            Some(id) => reflect_pinned::<E>(cx, id.raw() as u64),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__realizeDeclarativeShadow(node)` → the number of shadow roots the
/// declarative pass built under `node`. `setHTMLUnsafe` calls it; `innerHTML`
/// must not, which is why it is a separate step rather than part of parsing.
pub(crate) struct RealizeDeclarativeShadow;
impl<E: ScriptEngine> NativeFn<E> for RealizeDeclarativeShadow {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let node = cx.arg(0);
        let Some(raw) = cx.reflector_data(&node) else {
            return cx.make_string("0");
        };
        let count = with_dom::<E, _>(cx, |dom| {
            dom.realize_declarative_shadow_roots(NodeId::from_raw(raw as usize))
                .len()
        })
        .unwrap_or(0);
        cx.make_string(&count.to_string())
    }
}

/// `__getHTML(node, includeSerializable)` → the node's `innerHTML`, with each
/// serializable shadow root written back as a `<template shadowrootmode>` when
/// asked. The inverse of the declarative post-parse pass, and the only
/// serializer that crosses a shadow boundary.
pub(crate) struct GetHtmlWithShadow;
impl<E: ScriptEngine> NativeFn<E> for GetHtmlWithShadow {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let node = cx.arg(0);
        let Some(raw) = cx.reflector_data(&node) else {
            return cx.make_string("");
        };
        let include_v = cx.arg(1);
        let include = cx.value_to_string(&include_v)? == "1";
        let html = with_dom::<E, _>(cx, |dom| {
            let id = NodeId::from_raw(raw as usize);
            if include {
                dom.inner_html_with_shadow_roots(id)
            } else {
                dom.inner_html(id)
            }
        })
        .unwrap_or_default();
        cx.make_string(&html)
    }
}

pub(crate) fn install<E: ScriptEngine>(
    engine: &mut crate::Surface<'_, '_, E>,
) -> Result<(), crate::SurfaceError<E::Error>> {
    engine.set_function::<MayHostShadow>("__mayHostShadow", 1)?;
    engine.set_function::<AttachShadow>("__attachShadow", 6)?;
    engine.set_function::<ShadowRootOf>("__shadowRoot", 1)?;
    engine.set_function::<ShadowHostOf>("__shadowHost", 1)?;
    engine.set_function::<ShadowInitOf>("__shadowInit", 1)?;
    engine.set_function::<AssignedSlotOf>("__assignedSlot", 1)?;
    engine.set_function::<AssignedNodesOf>("__assignedNodes", 1)?;
    engine.set_function::<SlotAssign>("__slotAssign", 2)?;
    engine.set_function::<TakeSlotChanges>("__takeSlotChanges", 0)?;
    engine.set_function::<TemplateContent>("__templateContent", 1)?;
    engine.set_function::<TemplateOwnerDocument>("__templateOwnerDocument", 0)?;
    engine.set_function::<RealizeDeclarativeShadow>("__realizeDeclarativeShadow", 1)?;
    engine.set_function::<GetHtmlWithShadow>("__getHTML", 2)?;
    Ok(())
}
