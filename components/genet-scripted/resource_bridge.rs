/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The hosted scripted resource bridge.
//!
//! A Livery scripted document has one host resource route. This adapter keeps
//! that route intact for parser/CSS resources, the JS `fetch()` surface and
//! dedicated workers. Page fetches and hosted worker resources are started
//! off-thread. Their completion wakes the document's host for its next pump.

use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use script_engine_api::ScriptEngine;
use script_runtime_api::{FetchHandler, FetchOutcome, FetchRequest, Runtime, ScriptResourceLoader};

use crate::ResourceFetcher;

#[derive(Clone, Default)]
pub struct ScriptWake {
    state: Arc<Mutex<WakeState>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptWakeEvent {
    Wake {
        source: &'static str,
        generation: u64,
        resource: Option<String>,
    },
    Stale {
        source: &'static str,
        generation: u64,
        active_generation: u64,
        resource: Option<String>,
    },
}

struct WakeState {
    callback: Option<Arc<dyn Fn(ScriptWakeEvent) + Send + Sync>>,
    pending: Option<ScriptWakeEvent>,
    live_generations: BTreeSet<u64>,
}

impl Default for WakeState {
    fn default() -> Self {
        Self {
            callback: None,
            pending: None,
            live_generations: BTreeSet::new(),
        }
    }
}

impl ScriptWake {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install the host's wake sink. If a completion arrived before the
    /// window existed, report it immediately after the sink is attached.
    pub fn set_callback(&self, callback: impl Fn(ScriptWakeEvent) + Send + Sync + 'static) {
        let callback: Arc<dyn Fn(ScriptWakeEvent) + Send + Sync> = Arc::new(callback);
        let should_wake = {
            let mut state = self.state.lock().expect("script wake mutex poisoned");
            state.callback = Some(Arc::clone(&callback));
            state.pending.clone()
        };
        if let Some(event) = should_wake {
            callback(event);
        }
    }

    fn wake(&self, source: &'static str, generation: u64) {
        self.wake_with_resource(source, generation, None);
    }

    fn wake_with_resource(&self, source: &'static str, generation: u64, resource: Option<String>) {
        let callback = {
            let mut state = self.state.lock().expect("script wake mutex poisoned");
            state.pending = Some(ScriptWakeEvent::Wake {
                source,
                generation,
                resource: resource.clone(),
            });
            state.callback.clone()
        };
        if let Some(callback) = callback {
            callback(ScriptWakeEvent::Wake {
                source,
                generation,
                resource,
            });
        }
    }

    pub fn notify_worker_for(&self, generation: u64) {
        let active = self.active_generation();
        if active != generation {
            self.stale("worker", generation, active, None);
            return;
        }
        self.wake("worker", generation);
    }

    pub fn notify_worker_resource(&self, generation: u64, resource: String) {
        self.notify_source("worker", generation, Some(resource));
    }

    fn notify_source(&self, source: &'static str, generation: u64, resource: Option<String>) {
        let active = self.active_generation();
        if active == generation {
            self.wake_with_resource(source, generation, resource);
        } else {
            self.stale(source, generation, active, resource);
        }
    }

    fn stale(
        &self,
        source: &'static str,
        generation: u64,
        active_generation: u64,
        resource: Option<String>,
    ) {
        let callback = self
            .state
            .lock()
            .expect("script wake mutex poisoned")
            .callback
            .clone();
        if let Some(callback) = callback {
            callback(ScriptWakeEvent::Stale {
                source,
                generation,
                active_generation,
                resource,
            });
        }
    }

    fn register_generation(&self, generation: u64) {
        let mut state = self.state.lock().expect("script wake mutex poisoned");
        state.live_generations.insert(generation);
    }

    fn unregister_generation(&self, generation: u64) {
        let dispatch = {
            let mut state = self.state.lock().expect("script wake mutex poisoned");
            let previous = state
                .live_generations
                .iter()
                .next_back()
                .copied()
                .unwrap_or(0);
            state.live_generations.remove(&generation);
            let active = state
                .live_generations
                .iter()
                .next_back()
                .copied()
                .unwrap_or(0);
            if previous == generation && active != 0 {
                let event = ScriptWakeEvent::Wake {
                    source: "generation",
                    generation: active,
                    resource: None,
                };
                state.pending = Some(event.clone());
                state.callback.clone().map(|callback| (callback, event))
            } else {
                None
            }
        };
        if let Some((callback, event)) = dispatch {
            callback(event);
        }
    }

    /// Consume the coalesced notification. This is useful to hosts that use a
    /// channel instead of a direct window callback.
    pub fn take_pending(&self) -> bool {
        let mut state = self.state.lock().expect("script wake mutex poisoned");
        state.pending.take().is_some()
    }

    pub fn active_generation(&self) -> u64 {
        self.state
            .lock()
            .expect("script wake mutex poisoned")
            .live_generations
            .iter()
            .next_back()
            .copied()
            .unwrap_or(0)
    }
}

struct Completion {
    id: u64,
    outcome: FetchOutcome,
}

struct CompletionState {
    closed: bool,
    queue: VecDeque<Completion>,
}

struct BridgeState {
    fetcher: Arc<dyn ResourceFetcher + Send + Sync>,
    completions: Mutex<CompletionState>,
    active: AtomicUsize,
    closed: AtomicBool,
    wake: ScriptWake,
    generation: u64,
}

/// A cloneable resource route installed into both the runtime and Livery.
///
/// The route owns no transport policy. It only calls the `ResourceFetcher`
/// supplied by the host, preserving that fetcher's final URL and content type
/// when turning a response into a JS `Response`.
#[derive(Clone)]
pub struct ScriptResourceBridge {
    state: Arc<BridgeState>,
}

impl ScriptResourceBridge {
    pub fn new<F>(fetcher: F, wake: ScriptWake) -> Self
    where
        F: ResourceFetcher + Send + Sync + 'static,
    {
        Self::new_with_generation(fetcher, wake, 0)
    }

    pub fn new_with_generation<F>(fetcher: F, wake: ScriptWake, generation: u64) -> Self
    where
        F: ResourceFetcher + Send + Sync + 'static,
    {
        wake.register_generation(generation);
        Self {
            state: Arc::new(BridgeState {
                fetcher: Arc::new(fetcher),
                completions: Mutex::new(CompletionState {
                    closed: false,
                    queue: VecDeque::new(),
                }),
                active: AtomicUsize::new(0),
                closed: AtomicBool::new(false),
                wake,
                generation,
            }),
        }
    }

    pub fn wake(&self) -> ScriptWake {
        self.state.wake.clone()
    }

    pub fn generation(&self) -> u64 {
        self.state.generation
    }

    pub fn has_pending(&self) -> bool {
        self.state.active.load(Ordering::Acquire) != 0
            || !self
                .state
                .completions
                .lock()
                .expect("script resource mutex poisoned")
                .queue
                .is_empty()
    }

    /// Drop queued results and reject any future delivery. The worker runtime
    /// is shut down by `Runtime`'s drop; this closes the resource half first so
    /// a late transport completion cannot enter a replacement session.
    pub fn close(&self) {
        if self.state.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        // Retire the generation before waiting for the completion lock. A
        // transport thread that currently owns the lock will then report a
        // stale wake instead of waking the session being closed.
        self.state.wake.unregister_generation(self.state.generation);
        let mut completions = self
            .state
            .completions
            .lock()
            .expect("script resource mutex poisoned");
        completions.closed = true;
        completions.queue.clear();
    }

    pub fn pump<E: ScriptEngine>(&self, runtime: &mut Runtime<E>) -> usize {
        let completions = {
            let mut queue = self
                .state
                .completions
                .lock()
                .expect("script resource mutex poisoned");
            queue.queue.drain(..).collect::<Vec<_>>()
        };
        if self.state.closed.load(Ordering::Acquire) {
            return 0;
        }
        let count = completions.len();
        for completion in completions {
            runtime.settle_fetch(completion.id, completion.outcome);
        }
        count
    }

    fn outcome(&self, request_url: &str) -> FetchOutcome {
        match self.state.fetcher.fetch_response(request_url) {
            Some(response) => {
                let redirected = response.final_url != request_url;
                let mut headers = Vec::new();
                if let Some(content_type) = response.content_type {
                    headers.push(("content-type".to_owned(), content_type));
                }
                FetchOutcome {
                    network_error: false,
                    status: 200,
                    status_text: "OK".to_owned(),
                    response_type: "basic".to_owned(),
                    url: response.final_url,
                    redirected,
                    headers,
                    body: response.bytes,
                }
            },
            None => FetchOutcome::network_error(),
        }
    }
}

impl Drop for ScriptResourceBridge {
    fn drop(&mut self) {
        if Arc::strong_count(&self.state) == 1 {
            self.close();
        }
    }
}

impl ResourceFetcher for ScriptResourceBridge {
    fn fetch(&self, url: &str) -> Option<Vec<u8>> {
        self.state.fetcher.fetch(url)
    }

    fn fetch_response(&self, url: &str) -> Option<genet_host_api::ResourceResponse> {
        self.state.fetcher.fetch_response(url)
    }
}

impl FetchHandler for ScriptResourceBridge {
    fn start(&self, id: u64, request: FetchRequest) -> Option<FetchOutcome> {
        if self.state.closed.load(Ordering::Acquire) {
            return Some(FetchOutcome::network_error());
        }
        self.state.active.fetch_add(1, Ordering::AcqRel);
        let state = Arc::clone(&self.state);
        std::thread::Builder::new()
            .name("genet-script-fetch".to_owned())
            .spawn(move || {
                let outcome = Self {
                    state: Arc::clone(&state),
                }
                .outcome(&request.url);
                let resource = request.url.clone();
                let mut completions = state
                    .completions
                    .lock()
                    .expect("script resource mutex poisoned");
                if completions.closed || state.closed.load(Ordering::Acquire) {
                    drop(completions);
                    state.active.fetch_sub(1, Ordering::AcqRel);
                    state
                        .wake
                        .notify_source("fetch", state.generation, Some(resource));
                    return;
                }
                completions.queue.push_back(Completion { id, outcome });
                state.active.fetch_sub(1, Ordering::AcqRel);
                drop(completions);
                state
                    .wake
                    .notify_source("fetch", state.generation, Some(resource));
            })
            .expect("could not start scripted fetch worker");
        None
    }

    fn fetch(&self, request: FetchRequest) -> FetchOutcome {
        self.outcome(&request.url)
    }
}

impl ScriptResourceLoader for ScriptResourceBridge {
    fn load(&self, url: &str) -> Option<String> {
        let response = self.state.fetcher.fetch_response(url)?;
        String::from_utf8(response.bytes).ok()
    }

    fn start(
        &self,
        _id: u64,
        request: FetchRequest,
        complete: Box<dyn FnOnce(FetchOutcome) + Send>,
    ) -> Option<FetchRequest> {
        if self.state.closed.load(Ordering::Acquire) {
            return Some(request);
        }
        self.state.active.fetch_add(1, Ordering::AcqRel);
        let state = Arc::clone(&self.state);
        std::thread::Builder::new()
            .name("genet-worker-resource".to_owned())
            .spawn(move || {
                let resource = request.url.clone();
                let outcome = Self {
                    state: Arc::clone(&state),
                }
                .outcome(&resource);
                state.active.fetch_sub(1, Ordering::AcqRel);
                if state.closed.load(Ordering::Acquire) {
                    let active = state.wake.active_generation();
                    state
                        .wake
                        .stale("worker", state.generation, active, Some(resource));
                    return;
                }
                complete(outcome);
            })
            .expect("could not start worker resource thread");
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genet_host_api::ResourceResponse;
    use std::sync::atomic::AtomicUsize;

    #[derive(Clone)]
    struct Fixture {
        calls: Arc<AtomicUsize>,
    }

    impl ResourceFetcher for Fixture {
        fn fetch(&self, url: &str) -> Option<Vec<u8>> {
            self.fetch_response(url).map(|r| r.bytes)
        }

        fn fetch_response(&self, _url: &str) -> Option<ResourceResponse> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Some(
                ResourceResponse::new("https://final.test/script.js", b"x".to_vec())
                    .with_content_type("text/javascript"),
            )
        }
    }

    #[test]
    fn async_mapping_preserves_final_url_and_content_type() {
        let bridge = ScriptResourceBridge::new(
            Fixture {
                calls: Arc::new(AtomicUsize::new(0)),
            },
            ScriptWake::new(),
        );
        let outcome = bridge.outcome("https://origin.test/script.js");
        assert_eq!(outcome.url, "https://final.test/script.js");
        assert!(outcome.redirected);
        assert_eq!(
            outcome.headers,
            [("content-type".to_owned(), "text/javascript".to_owned())]
        );
        assert_eq!(outcome.body, b"x");
    }

    #[test]
    fn wake_callback_is_called_for_deferred_completion() {
        let wake = ScriptWake::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        wake.set_callback(move |_| {
            observed.fetch_add(1, Ordering::Relaxed);
        });
        wake.wake("fetch", 0);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(wake.take_pending());
        assert!(!wake.take_pending());
    }

    #[test]
    fn close_discards_queued_completions() {
        let bridge = ScriptResourceBridge::new(
            Fixture {
                calls: Arc::new(AtomicUsize::new(0)),
            },
            ScriptWake::new(),
        );
        bridge
            .state
            .completions
            .lock()
            .unwrap()
            .queue
            .push_back(Completion {
                id: 1,
                outcome: FetchOutcome::network_error(),
            });
        assert!(bridge.has_pending());
        bridge.close();
        assert!(!bridge.has_pending());
    }

    #[test]
    fn failed_replacement_restores_the_previous_generation() {
        let wake = ScriptWake::new();
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        wake.set_callback(move |event| observed.lock().unwrap().push(event));
        let first = ScriptResourceBridge::new_with_generation(
            Fixture {
                calls: Arc::new(AtomicUsize::new(0)),
            },
            wake.clone(),
            1,
        );
        let replacement = ScriptResourceBridge::new_with_generation(
            Fixture {
                calls: Arc::new(AtomicUsize::new(0)),
            },
            wake.clone(),
            2,
        );
        wake.notify_source("fetch", 1, Some("https://old.test/data".to_owned()));
        replacement.close();
        assert_eq!(wake.active_generation(), 1);
        assert!(matches!(
            events.lock().unwrap().last(),
            Some(ScriptWakeEvent::Wake {
                source: "generation",
                generation: 1,
                resource: None,
            })
        ));
        drop(first);
    }

    #[test]
    fn overlapping_failed_spawns_leave_the_latest_live_generation_active() {
        let wake = ScriptWake::new();
        let fixture = || Fixture {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let first = ScriptResourceBridge::new_with_generation(fixture(), wake.clone(), 1);
        let second = ScriptResourceBridge::new_with_generation(fixture(), wake.clone(), 2);
        let third = ScriptResourceBridge::new_with_generation(fixture(), wake.clone(), 3);
        second.close();
        assert_eq!(wake.active_generation(), 3);
        third.close();
        assert_eq!(wake.active_generation(), 1);
        drop(first);
    }
}
