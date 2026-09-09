# G5 S3 handle-boundary research receipt

This receipt records a model probe compiled against committed Genet revision `ec5421b75914529e017e084eb327df4dc13b76c7`. The runner uses actual `genet-scripted-dom` nodes, but the 24/40 packed handle and registry comparison live only in the runner. Genet does not implement this handle yet.

## Results

- Native debug: 3/3 tests passed. Two actual arenas produced distinct debug-fenced raw roots; packed correlation was stable.
- Native release: two actual arenas produced the same raw root `0`. The modeled packed handles were distinct and the foreign handle was refused.
- wasm32 release: the runner and actual path dependency compiled. It was not executed in a wasm runtime.
- Checked model limits: 24-bit arena and 40-bit local maxima accept; the next values refuse.
- One release sample over 1,000,000 operations measured 2.2331 ms arithmetic extraction and 213.4494 ms lookup in a 100,000-entry HashMap. This prices only the extra boundary operation and is not an acceptance threshold.

Commands and exit codes are in `run.json`; raw output is in `debug.log`, `release.log`, and `wasm-check.log`. `sources.json` pins the inspected production sources. The runner source and lockfile are retained; Cargo caches and target artifacts are excluded.

## Evidence boundary

The probe supports a 24/40 packed `u64` as a plausible end-state representation. It does not establish a production contract. Current runtime code still narrows `ReflectorData` through `as usize` in about 70 inbound paths, and an additive packed API without consumer migration would not fix that boundary. The current `NodeId(usize)` also cannot refuse a foreign raw `NodeId` at capture on release, where the debug arena tag is absent; `remint_node_id` converts captured `u64` to `usize`, which panics on overflow rather than truncating today. In the model, a same-arena local value above wasm32 usize would therefore panic; a future packed arena field also cannot pass through that remint unchanged. A production slice must migrate the coherent reflector consumer through the roughly 70 runtime conversions and then adopt an all-target semantic node handle. Acceptance must execute values above 2^53 through enabled engines and string bridges, and reject foreign/dead handles on native release and wasm.

Same-`ScriptedDom` secondary-document adoption can preserve the packed identity because the storage arena stays fixed. Cross-`ScriptedDom` adoption remains explicitly unsupported until a coordinated transfer or multi-arena resolver can preserve wrapper identity.


## Replay and configuration

Native Windows x86_64, Rust 1.97.1, actual scripted-dom default features;
engine, renderer and server: none. Native tests use the dev test profile;
release uses the optimized profile. The wasm target is
`wasm32-unknown-unknown`, checked only. From the supplied `runner` directory,
the three commands in `run.json` use the supplied lock. Its relative source
paths require a sibling `genet-sparse` checkout at the recorded revision.
The pinned source hashes were independently checked against that commit's
Git blobs. Caches and target directories are host plumbing, not prerequisites
for source reconstruction.

The named regression manifest is `runner/src/main.rs`. The current model
uses caller-chosen distinct arena tokens; it does not implement a production
non-reusing token allocator, raw-NodeId custody checks, or typed overflow refusal
at every consumer. The discarded S4 sketch is not a deliverable. S4 stays open
for a coherent representation and consumer migration, with actual engine
high-bit tests and wasm runtime refusal before promotion.
