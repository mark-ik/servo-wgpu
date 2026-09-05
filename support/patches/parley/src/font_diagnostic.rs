// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Opt-in, thread-local receipts for Parley's Fontique queries.

use std::{cell::RefCell, marker::PhantomData, rc::Rc, string::String, thread_local, vec::Vec};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontDiagnosticCandidate {
    pub family: String,
    pub index: u32,
    pub status: FontDiagnosticCandidateStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontDiagnosticCandidateStatus {
    NoCharmap,
    Complete,
    Keep,
    Discard,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontDiagnosticEvent {
    /// Source characters in the queried grapheme cluster.
    pub cluster: String,
    pub fallback_script: [u8; 4],
    pub candidates: Vec<FontDiagnosticCandidate>,
    pub selected_candidate: Option<usize>,
}

#[derive(Default)]
struct State {
    enabled: bool,
    events: Vec<FontDiagnosticEvent>,
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State { enabled: false, events: Vec::new() }) };
}

/// Enables recording on this thread until [`take`](Self::take) or drop.
///
/// Starting a capture clears stale events. Dropping an unconsumed capture
/// disables recording and discards its events, including during unwinding.
#[must_use = "capture diagnostics with take() or let the scope drop"]
pub struct FontDiagnosticCapture {
    active: bool,
    // Capture teardown must run on the thread whose thread-local state it owns.
    _not_send_or_sync: PhantomData<Rc<()>>,
}

/// Starts one scoped capture of Fontique queries on the current thread.
pub fn begin_font_diagnostic_capture() -> FontDiagnosticCapture {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        assert!(!state.enabled, "font diagnostic capture is already active");
        state.events.clear();
        state.enabled = true;
    });
    FontDiagnosticCapture {
        active: true,
        _not_send_or_sync: PhantomData,
    }
}

impl FontDiagnosticCapture {
    /// Stops recording and returns this scope's events.
    pub fn take(mut self) -> Vec<FontDiagnosticEvent> {
        self.active = false;
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.enabled = false;
            core::mem::take(&mut state.events)
        })
    }
}

impl Drop for FontDiagnosticCapture {
    fn drop(&mut self) {
        if self.active {
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                state.enabled = false;
                state.events.clear();
            });
        }
    }
}

pub(crate) fn record(event: FontDiagnosticEvent) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.enabled {
            state.events.push(event);
        }
    });
}
