// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The selection seam: range geometry out of the host's layout, and the visual
//! selection's projection back into it.
//!
//! Script owns the selection. `Selection`'s single live `Range` in the
//! bootstrap is the one source of truth; what Livery paints is a projection
//! pushed through [`SelectionHandler::set_visual_selection`] every time that
//! range moves, and a host pointer gesture reaches script through the same seam
//! in the other direction (`__selectionFromHost`). Nothing here holds selection
//! state of its own.

use super::*;

/// The host's layout seam for range geometry and the visual selection. Mirrors
/// [`ComputedStyleHandler`]: the runtime never links a layout engine, so a host
/// that has one implements this. Boundary nodes are reflector raw ids and the
/// offsets are UTF-16 code units, exactly as script states them; the host owns
/// the conversion to whatever its shaped text is indexed by.
pub trait SelectionHandler {
    /// Viewport rectangles (`[x, y, width, height]`) covering the range. An
    /// empty list when the document has no box tree or the range covers no
    /// shaped text, which is what the spec asks for in that case.
    fn range_rects(
        &self,
        start: u64,
        start_offset: u32,
        end: u64,
        end_offset: u32,
    ) -> Vec<[f32; 4]>;

    /// Project the script-owned selection onto the visual one. `None` clears it.
    fn set_visual_selection(&self, range: Option<(u64, u32, u64, u32)>);
}

fn handler<E: ScriptEngine>(cx: &mut E::CallCx<'_>) -> Option<Rc<dyn SelectionHandler>> {
    let host = adoption::host_for_call::<E>(cx)?;
    let handler = host.borrow().selection.clone();
    handler
}

// Authored calls can pass independently valid nodes from different documents.
// A layout handler must never receive a boundary owned by another arena.
fn same_boundary_owner(start: &adoption::OwnedNode, end: &adoption::OwnedNode) -> bool {
    Rc::ptr_eq(start.host(), end.host())
}

fn offset_arg<E: ScriptEngine>(cx: &mut E::CallCx<'_>, index: usize) -> Result<u32, E::Error> {
    let value = cx.arg(index);
    let text = cx.value_to_string(&value)?;
    Ok(text.parse::<f64>().unwrap_or(0.0).max(0.0) as u32)
}

/// `__rangeRects(startRef, startOffset, endRef, endOffset)` -> newline-separated
/// `x,y,width,height`, empty when there is no geometry.
pub(crate) struct RangeRects;
impl<E: ScriptEngine> NativeFn<E> for RangeRects {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let start_value = cx.arg(0);
        let end_value = cx.arg(2);
        let (Some(start), Some(end)) = (cx.owned_node(&start_value)?, cx.owned_node(&end_value)?)
        else {
            return cx.make_string("");
        };
        if !same_boundary_owner(&start, &end) {
            return cx.make_string("");
        }
        let start_offset = offset_arg::<E>(cx, 1)?;
        let end_offset = offset_arg::<E>(cx, 3)?;
        let rects = handler::<E>(cx)
            .map(|h| h.range_rects(start.raw(), start_offset, end.raw(), end_offset))
            .unwrap_or_default();
        let encoded = rects
            .iter()
            .map(|r| format!("{},{},{},{}", r[0], r[1], r[2], r[3]))
            .collect::<Vec<_>>()
            .join("\n");
        cx.make_string(&encoded)
    }
}

/// `__selectionVisual(startRef, startOffset, endRef, endOffset)` — push the
/// script-owned selection to the host. No arguments clears it.
pub(crate) struct VisualSelection;
impl<E: ScriptEngine> NativeFn<E> for VisualSelection {
    fn call(cx: &mut E::CallCx<'_>) -> Result<E::Value, E::Error> {
        let start_value = cx.arg(0);
        let end_value = cx.arg(2);
        let points = match (cx.owned_node(&start_value)?, cx.owned_node(&end_value)?) {
            (Some(start), Some(end)) if same_boundary_owner(&start, &end) => {
                let start_offset = offset_arg::<E>(cx, 1)?;
                let end_offset = offset_arg::<E>(cx, 3)?;
                Some((start.raw(), start_offset, end.raw(), end_offset))
            },
            _ => None,
        };
        if let Some(handler) = handler::<E>(cx) {
            handler.set_visual_selection(points);
        }
        Ok(cx.undefined())
    }
}
