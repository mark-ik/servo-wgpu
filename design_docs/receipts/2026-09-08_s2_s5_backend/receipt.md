# S2 and S5 backend receipts, 2026-09-08

This directory preserves two focused Boa-backed external probes. It contains
the raw logs, exact runner manifests, locks and source, plus replay patches.
It does not contain a source-tree copy, a shared Cargo cache or a build target.

## S2: retained descendants through replacement

`s2-backend.log` records one passing test in 2.63 seconds. The probe loads the
actual scripted DOM through `Runtime<BoaEngine>`, retains JS descendants across
`textContent` and `innerHTML` replacement with observers off, forces collection,
checks semantic readback and reattachment, then releases and confirms at least
two reflectors unpinned and at least two nodes collected. `s2-backend-run.json`
is the invocation receipt.

Replay source is Genet `ec5421b7591` plus `s2-dom-overlay.patch`. The resulting
two source files are pinned in `s2-sources.json`. Run the untouched
`s2-runner` beside the replay source so its relative `../source-s2` paths resolve,
using its recorded `Cargo.lock`; its Boa patch was the clean local checkout at
`C:/Users/mark_/Code/crates/boa` revision `8ecd311be103b989341a4f8a0e60394d1dec3aa5`.

## S5: runtime worker wake and acknowledged idle

`runtime-wake.log` records one passing test. It starts an actual
`Runtime<BoaEngine>` worker, establishes that timer and microtask turns alone
do not deliver a reply, then calls `pump_workers()` and waits for the worker's
idle acknowledgement before `has_worker_work()` becomes false.
`runtime-wake-run.json` records the invocation and exact runtime/worker source
hashes. It deliberately records that the source included the pre-existing arena
retention candidate. That overlay is unrelated to worker servicing and remains
separate in `s5-arena-overlay.patch` and `s5-sources.json`. The former is the
original recorded semantic patch. `s5-arena-overlay-replay.patch` is a binary
overlay which also preserves the recorded Windows source bytes and is the replay
artifact named by `s5-sources.json`.

Replay source is Genet `ee0b314b3e9ac4a2fadb07fb7816990fa3f2b71d` plus the
recorded S5 overlay. Run the untouched `s5-runner` beside that replay source so
its relative `../source` paths resolve. Both final commands were
`cargo test --offline --locked --manifest-path <runner>/Cargo.toml -- --nocapture`.
The private Cargo home was populated only for the earlier dependency resolution;
the recorded test runs were offline and locked.

S6 forwarding is intentionally outside this receipt bundle.

Both runs used Boa on native Windows x86_64 with Rust 1.97.1, default runtime
features and no renderer. S5 uses only its dedicated server-style script loader;
neither receipt covers Nova, wasm, a headed host or browser navigation.
