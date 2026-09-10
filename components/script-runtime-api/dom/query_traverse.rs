// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! DOM query (selectors / collections), tree traversal, mutation, and
//! node-info command sinks.

use super::*;

/// `__removeAttribute(element, name)`.
pub(crate) struct RemoveAttribute;
impl<E: ScriptEngine> NativeFn<E> for RemoveAttribute {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let el = cx.arg(0);
        let Some(id) = cx.owned_node(&el)? else {
            return Ok(cx.undefined());
        };
        let name_v = cx.arg(1);
        let name = cx.value_to_string(&name_v)?;
        id.with_dom(|dom| {
            let node = id.id();
            // `removeAttribute` matches on the **qualified** name, so a namespaced
            // attribute is reachable by `prefix:local` too.
            if let Some(qual) = dom.attribute_qual_name(node, &name) {
                dom.remove_attribute(node, qual);
            }
        });
        Ok(cx.undefined())
    }
}

/// `__matches(element, selector)` → `"true"`/`"false"`.
pub(crate) struct Matches;
impl<E: ScriptEngine> NativeFn<E> for Matches {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let el = cx.arg(0);
        let Some(id) = cx.owned_node(&el)? else {
            return cx.make_string("false");
        };
        let sel_v = cx.arg(1);
        let sel = cx.value_to_string(&sel_v)?;
        let matched = id.with_dom(|dom| crate::selector::parse(&sel).matches(dom, id.id()));
        cx.make_string(if matched { "true" } else { "false" })
    }
}

/// `__querySelector(scope, selector)` → the first matching descendant's reflector,
/// or `null`. `scope` is an element or the document.
pub(crate) struct QuerySelector;
impl<E: ScriptEngine> NativeFn<E> for QuerySelector {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let scope = cx.arg(0);
        let Some(id) = cx.owned_node(&scope)? else {
            return Ok(cx.make_null());
        };
        let sel_v = cx.arg(1);
        let sel = cx.value_to_string(&sel_v)?;
        match id.with_dom(|dom| crate::selector::parse(&sel).query_first(dom, id.id())) {
            Some(node) => reflect_pinned::<E>(cx, node.raw() as u64),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__querySelectorAllCount(scope, selector)` → match count (as a string). Paired
/// with `__querySelectorAllItem`, the count/item pattern used elsewhere.
pub(crate) struct QuerySelectorAllCount;
impl<E: ScriptEngine> NativeFn<E> for QuerySelectorAllCount {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let scope = cx.arg(0);
        let Some(id) = cx.owned_node(&scope)? else {
            return cx.make_string("0");
        };
        let sel_v = cx.arg(1);
        let sel = cx.value_to_string(&sel_v)?;
        let n = id.with_dom(|dom| crate::selector::parse(&sel).query_all(dom, id.id()).len());
        cx.make_string(&n.to_string())
    }
}

/// `__querySelectorAllItem(scope, selector, i)` → the i-th match's reflector, or
/// `undefined`.
pub(crate) struct QuerySelectorAllItem;
impl<E: ScriptEngine> NativeFn<E> for QuerySelectorAllItem {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let scope = cx.arg(0);
        let Some(id) = cx.owned_node(&scope)? else {
            return Ok(cx.undefined());
        };
        let sel_v = cx.arg(1);
        let sel = cx.value_to_string(&sel_v)?;
        let i_v = cx.arg(2);
        let i = cx
            .value_to_string(&i_v)?
            .parse::<usize>()
            .unwrap_or(usize::MAX);
        match id.with_dom(|dom| {
            crate::selector::parse(&sel)
                .query_all(dom, id.id())
                .get(i)
                .copied()
        }) {
            Some(node) => reflect_pinned::<E>(cx, node.raw() as u64),
            None => Ok(cx.undefined()),
        }
    }
}

/// `__firstChild(node)` → first child reflector, or `undefined`.
pub(crate) struct FirstChild;
impl<E: ScriptEngine> NativeFn<E> for FirstChild {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        child_at::<E>(cx, |dom, node| dom.dom_children(node).next())
    }
}

/// `__lastChild(node)` → last child reflector, or `undefined`.
pub(crate) struct LastChild;
impl<E: ScriptEngine> NativeFn<E> for LastChild {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        child_at::<E>(cx, |dom, node| dom.dom_children(node).last())
    }
}

/// `__nextSibling(node)` → next sibling reflector, or `undefined`.
pub(crate) struct NextSibling;
impl<E: ScriptEngine> NativeFn<E> for NextSibling {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        child_at::<E>(cx, |dom, node| dom.next_sibling(node))
    }
}

/// `__prevSibling(node)` → previous sibling reflector, or `undefined`.
pub(crate) struct PrevSibling;
impl<E: ScriptEngine> NativeFn<E> for PrevSibling {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        child_at::<E>(cx, |dom, node| dom.prev_sibling(node))
    }
}

/// Shared helper for the single-node traversal sinks: recover the arg-0 node, run
/// `pick` against the DOM, reflect the result (or `undefined`).
fn child_at<E: ScriptEngine>(
    cx: &mut E::CallCx<'_>,
    pick: impl FnOnce(&ScriptedDom, NodeId) -> Option<NodeId>,
) -> Result<E::Value, E::Error> {
    let node = cx.arg(0);
    let Some(id) = cx.owned_node(&node)? else {
        return Ok(cx.undefined());
    };
    match id.with_dom(|dom| pick(dom, id.id())) {
        Some(n) => reflect_pinned::<E>(cx, n.raw() as u64),
        None => Ok(cx.undefined()),
    }
}

/// `__childNodesCount(node)` → child count (string). With `__childNodesItem`, backs
/// `childNodes` (and, JS-filtered, `children`).
pub(crate) struct ChildNodesCount;
impl<E: ScriptEngine> NativeFn<E> for ChildNodesCount {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let node = cx.arg(0);
        let Some(id) = cx.owned_node(&node)? else {
            return cx.make_string("0");
        };
        let n = id.with_dom(|dom| dom.dom_children(id.id()).count());
        cx.make_string(&n.to_string())
    }
}

/// `__childNodesItem(node, i)` → the i-th child's reflector, or `undefined`.
pub(crate) struct ChildNodesItem;
impl<E: ScriptEngine> NativeFn<E> for ChildNodesItem {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let node = cx.arg(0);
        let Some(id) = cx.owned_node(&node)? else {
            return Ok(cx.undefined());
        };
        let i_v = cx.arg(1);
        let i = cx
            .value_to_string(&i_v)?
            .parse::<usize>()
            .unwrap_or(usize::MAX);
        match id.with_dom(|dom| dom.dom_children(id.id()).nth(i)) {
            Some(n) => reflect_pinned::<E>(cx, n.raw() as u64),
            None => Ok(cx.undefined()),
        }
    }
}

/// `__nodeName(node)`: element → its case-preserved qualified name; text →
/// `#text`; comment → `#comment`; document → `#document`; else the kind's
/// conventional name. The HTML uppercasing is applied by the bootstrap, which
/// is the only tier that knows the node's current node document.
pub(crate) struct NodeName;
impl<E: ScriptEngine> NativeFn<E> for NodeName {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let node = cx.arg(0);
        let Some(id) = cx.owned_node(&node)? else {
            return cx.make_string("");
        };
        let name = with_dom::<E, _>(cx, |dom| {
            let n = id.id();
            match dom.kind(n) {
                NodeKind::Element => dom.element_name(n).map(qualified_of).unwrap_or_default(),
                NodeKind::Text => "#text".to_string(),
                NodeKind::Comment => "#comment".to_string(),
                NodeKind::Document => "#document".to_string(),
                NodeKind::CdataSection => "#cdata-section".to_string(),
                NodeKind::Doctype => dom.text(n).unwrap_or("html").to_string(),
                // A PI's node name is its target.
                NodeKind::ProcessingInstruction => dom
                    .element_name(n)
                    .map(|q| q.local.as_ref().to_string())
                    .unwrap_or_default(),
                // A ShadowRoot is a DocumentFragment, and DOM gives it the same
                // nodeName.
                NodeKind::DocumentFragment | NodeKind::ShadowRoot => {
                    "#document-fragment".to_string()
                },
            }
        })
        .unwrap_or_default();
        cx.make_string(&name)
    }
}

/// `__nodeValue(node)`: text/comment → its data; otherwise `null`.
pub(crate) struct NodeValue;
impl<E: ScriptEngine> NativeFn<E> for NodeValue {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let node = cx.arg(0);
        let Some(id) = cx.owned_node(&node)? else {
            return Ok(cx.make_null());
        };
        let value = with_dom::<E, _>(cx, |dom| {
            let n = id.id();
            match dom.kind(n) {
                NodeKind::Text
                | NodeKind::Comment
                | NodeKind::CdataSection
                | NodeKind::ProcessingInstruction => Some(dom.text(n).unwrap_or("").to_string()),
                _ => None,
            }
        })
        .flatten();
        match value {
            Some(s) => cx.make_string(&s),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__removeChild(parent, child)` — detach `child` (the JS side has already checked
/// it is a child of `parent`).
pub(crate) struct RemoveChild;
impl<E: ScriptEngine> NativeFn<E> for RemoveChild {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let parent = cx.arg(0);
        let child = cx.arg(1);
        if let (Some(p), Some(c)) = (cx.owned_node(&parent)?, cx.owned_node(&child)?) {
            super::adoption::require_same_owner(cx, &[&p, &c])?;
            // Orphan (keep alive + re-insertable), not drop — DOM `removeChild`.
            c.with_dom(|dom| dom.remove_child(c.id()));
        }
        Ok(cx.undefined())
    }
}

/// `__insertBefore(parent, node, ref)` — insert `node` before `ref` (a reflector),
/// or append when `ref` is not a reflector (undefined/null).
pub(crate) struct InsertBefore;
impl<E: ScriptEngine> NativeFn<E> for InsertBefore {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let parent = cx.arg(0);
        let node = cx.arg(1);
        let reference = cx.arg(2);
        if let (Some(p), Some(n)) = (cx.owned_node(&parent)?, cx.owned_node(&node)?) {
            let r = cx.owned_node(&reference)?;
            let mut ids = vec![&p, &n];
            if let Some(reference) = r.as_ref() {
                ids.push(reference);
            }
            super::adoption::require_same_owner(cx, &ids)?;
            let reference = r.as_ref().map(super::adoption::OwnedNode::id);
            p.with_dom(|dom| dom.insert_before(p.id(), n.id(), reference));
            super::root_connected_subtree::<E>(cx, n.id());
        }
        Ok(cx.undefined())
    }
}

/// `__moveBefore(parent, node, ref)` — atomically move the in-tree `node` before
/// `ref` (append when `ref` is not a reflector), preserving subtree state: one
/// `DomMutation::Moved`, never a `Removed` + `Inserted` pair. The unchecked
/// primitive under `Node.prototype.moveBefore`; the spec's pre-move validity
/// (same root, no cycles, NotFoundError reference) throws on the JS side, per
/// this file's split. (moveBefore plan S3.)
pub(crate) struct MoveBefore;
impl<E: ScriptEngine> NativeFn<E> for MoveBefore {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let parent = cx.arg(0);
        let node = cx.arg(1);
        let reference = cx.arg(2);
        if let (Some(p), Some(n)) = (cx.owned_node(&parent)?, cx.owned_node(&node)?) {
            let r = cx.owned_node(&reference)?;
            let mut ids = vec![&p, &n];
            if let Some(reference) = r.as_ref() {
                ids.push(reference);
            }
            super::adoption::require_same_owner(cx, &ids)?;
            let reference = r.as_ref().map(super::adoption::OwnedNode::id);
            p.with_dom(|dom| dom.move_before(p.id(), n.id(), reference));
            super::root_connected_subtree::<E>(cx, n.id());
        }
        Ok(cx.undefined())
    }
}

/// `__localName(element)` → the element's local name (as stored, lowercase for
/// HTML), or `null`.
pub(crate) struct LocalNameOf;
impl<E: ScriptEngine> NativeFn<E> for LocalNameOf {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let el = cx.arg(0);
        let Some(id) = cx.owned_node(&el)? else {
            return Ok(cx.make_null());
        };
        let name = id.with_dom(|dom| {
            dom.element_name(id.id())
                .map(|q| q.local.as_ref().to_string())
        });
        match name {
            Some(s) => cx.make_string(&s),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__namespaceURI(element)` → the element's namespace, or `null` when empty.
pub(crate) struct NamespaceUri;
impl<E: ScriptEngine> NativeFn<E> for NamespaceUri {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let el = cx.arg(0);
        let Some(id) = cx.owned_node(&el)? else {
            return Ok(cx.make_null());
        };
        let ns = id.with_dom(|dom| dom.element_name(id.id()).map(|q| q.ns.as_ref().to_string()));
        match ns {
            Some(s) if !s.is_empty() => cx.make_string(&s),
            _ => Ok(cx.make_null()),
        }
    }
}

/// `__prefix(element)` → the element's namespace prefix, or `null`.
pub(crate) struct PrefixOf;
impl<E: ScriptEngine> NativeFn<E> for PrefixOf {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let el = cx.arg(0);
        let Some(id) = cx.owned_node(&el)? else {
            return Ok(cx.make_null());
        };
        let prefix = with_dom::<E, _>(cx, |dom| {
            dom.element_name(id.id())
                .and_then(|q| q.prefix.as_ref().map(|p| p.as_ref().to_string()))
        })
        .flatten();
        match prefix {
            Some(s) => cx.make_string(&s),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__createElementNS(ns, qualifiedName)` → a reflector for the new element. The
/// qualified name is split on `:` into prefix + local. (Strict name validation /
/// `InvalidCharacterError` is deferred; a malformed name still creates an element.)
pub(crate) struct CreateElementNS;
impl<E: ScriptEngine> NativeFn<E> for CreateElementNS {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let ns_v = cx.arg(0);
        let qname_v = cx.arg(1);
        let ns = cx.value_to_string(&ns_v)?;
        let qname = cx.value_to_string(&qname_v)?;
        let (prefix, local) = match qname.split_once(':') {
            Some((p, l)) => (Some(Prefix::from(p)), l.to_string()),
            None => (None, qname),
        };
        let qual = QualName::new(
            prefix,
            Namespace::from(ns.as_str()),
            LocalName::from(local.as_str()),
        );
        match with_dom::<E, _>(cx, |dom| dom.create_element(qual)) {
            Some(node) => reflect_pinned::<E>(cx, node.raw() as u64),
            None => Ok(cx.undefined()),
        }
    }
}

/// `__attributeNames(element)` → the element's attribute local names, space-joined
/// (attribute names contain no spaces). Backs `dataset` ownKeys / enumeration.
pub(crate) struct AttributeNames;
impl<E: ScriptEngine> NativeFn<E> for AttributeNames {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let el = cx.arg(0);
        let Some(id) = cx.owned_node(&el)? else {
            return cx.make_string("");
        };
        let names = id.with_dom(|dom| {
            dom.attributes(id.id())
                .map(|a| a.name.local.as_ref().to_string())
                .collect::<Vec<_>>()
                .join(" ")
        });
        cx.make_string(&names)
    }
}

/// `__attributeRecords(element)` → one record per attribute, `namespace`,
/// `prefix` and `local name` separated by U+001F, records separated by U+001E.
/// Neither separator can occur in a name, so the JS side splits without escaping;
/// values are read back with `__getAttributeNS`, keeping arbitrary text out of
/// this record. Backs the live `NamedNodeMap`.
pub(crate) struct AttributeRecords;
impl<E: ScriptEngine> NativeFn<E> for AttributeRecords {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let el = cx.arg(0);
        let Some(id) = cx.owned_node(&el)? else {
            return cx.make_string("");
        };
        let records = id.with_dom(|dom| {
            dom.attribute_names(id.id())
                .into_iter()
                .map(|(ns, prefix, local)| format!("{ns}\u{1f}{prefix}\u{1f}{local}"))
                .collect::<Vec<_>>()
                .join("\u{1e}")
        });
        cx.make_string(&records)
    }
}

/// `__setAttributeNS(element, ns, qualifiedName, value)` — the namespace-aware
/// attribute write. An empty `ns` is the null namespace.
pub(crate) struct SetAttributeNS;
impl<E: ScriptEngine> NativeFn<E> for SetAttributeNS {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let el = cx.arg(0);
        let Some(id) = cx.owned_node(&el)? else {
            return Ok(cx.undefined());
        };
        let ns_v = cx.arg(1);
        let qname_v = cx.arg(2);
        let value_v = cx.arg(3);
        let ns = cx.value_to_string(&ns_v)?;
        let qname = cx.value_to_string(&qname_v)?;
        let value = cx.value_to_string(&value_v)?;
        id.with_dom(|dom| dom.set_attribute(id.id(), ns_attr_qual(&ns, &qname), &value));
        Ok(cx.undefined())
    }
}

/// `__getAttributeNS(element, ns, localName)` → the value, or `null`.
pub(crate) struct GetAttributeNS;
impl<E: ScriptEngine> NativeFn<E> for GetAttributeNS {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let el = cx.arg(0);
        let Some(id) = cx.owned_node(&el)? else {
            return Ok(cx.make_null());
        };
        let ns_v = cx.arg(1);
        let local_v = cx.arg(2);
        let ns = cx.value_to_string(&ns_v)?;
        let local = cx.value_to_string(&local_v)?;
        let value = id.with_dom(|dom| {
            dom.attribute(
                id.id(),
                &Namespace::from(ns.as_str()),
                &LocalName::from(local.as_str()),
            )
            .map(str::to_string)
        });
        match value {
            Some(s) => cx.make_string(&s),
            None => Ok(cx.make_null()),
        }
    }
}

/// `__removeAttributeNS(element, ns, localName)`.
pub(crate) struct RemoveAttributeNS;
impl<E: ScriptEngine> NativeFn<E> for RemoveAttributeNS {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let el = cx.arg(0);
        let Some(id) = cx.owned_node(&el)? else {
            return Ok(cx.undefined());
        };
        let ns_v = cx.arg(1);
        let local_v = cx.arg(2);
        let ns = cx.value_to_string(&ns_v)?;
        let local = cx.value_to_string(&local_v)?;
        id.with_dom(|dom| {
            dom.remove_attribute(
                id.id(),
                QualName::new(
                    None,
                    Namespace::from(ns.as_str()),
                    LocalName::from(local.as_str()),
                ),
            )
        });
        Ok(cx.undefined())
    }
}
