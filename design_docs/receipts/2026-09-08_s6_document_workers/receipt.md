# S6a scripted-document worker service, 2026-09-08

Both scripted-document routes now forward worker service after timers and a
microtask checkpoint, then checkpoint message-handler reactions before capture
and collection. Their non-frozen pending status includes outstanding workers.
A public setter transfers an owned ScriptResourceLoader into the runtime;
the borrowed parser ResourceFetcher does not survive construction. Hosts install
the retained route before the first worker-service pump.

The named production regression manifest is
`components/genet-scripted/tests/worker_service.rs`, byte-identical to supplied
`runner/src/lib.rs`. Two real ScriptedDocument<BoaEngine> tests pass:

- An initially unchanged document receives exactly `wake:one` plus the Promise
  reaction attribute, using a real worker and retained loader. Freeze refuses
  drive without delivering the reply; resume restores demand. Pending becomes
  false only after the worker's idle acknowledgement.
- An unavailable worker route delivers an error to the page and quiesces.

Script code is evaluated separately from markup, so its marker strings cannot
satisfy serialized-DOM assertions. Delivery and idle have independent bounded
30-second wall-clock waits. This is an eventual-service gate, not a latency
benchmark. Earlier five-second attempts were too short under the concurrent
build load and did not establish a semantic failure. Final test body: 11.90s;
worker delivery and idle observation: 80 pumps in 8,420ms in that run.

The negative control retains only the new loader setter and restores pre-change
pump/pending behavior. The previous one-case manifest fails at the assertion
that a worker without a timer is outstanding. Its exact manifest, log and run
metadata and baseline-relative `s6-preforward.patch` are supplied. S5 separately proves timer/microtask-only non-delivery.

Native Windows x86_64, Rust 1.97.1, Boa pinned at
`8ecd311be103b989341a4f8a0e60394d1dec3aa5`; genet-scripted default features off,
dev test profile, renderer none, synchronous injected script loader, no HTTP
server. Source is Genet `c52ee06f53a` plus `s6-final.patch`; concurrent parser
and media-query changes are excluded. Apply the patch and place the saved
runner beside `source-s6` to replay offline/locked with the supplied lock.
The recorded source hash is the tested patch; `post-test-doc-comment.patch`
only clarifies pending-work documentation and was reviewed after the test.

The Livery source path mirrors the change but was not compiled or rendered in
this receipt. Nova, a ready/deadline/external drive report, asynchronous host
wake registration, worker teardown across navigation, and headed Ortet O5
remain open. Outstanding worker work is not a ready-now predicate.
