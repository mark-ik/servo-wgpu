# G5 S2 template retention edges, 2026-09-08

The store correction adds a directed contents-fragment to inert-owner edge,
keeps the template to contents edge, and prunes both metadata tables with swept
nodes. An owner has no edge back to its fragments. A pinned contents descendant
can therefore retain its owner after the template dies without retaining retired
sibling fragments. Cached owner IDs and contents lookups refuse swept nodes.

The named regression manifest is
`components/genet-scripted-dom/tests/root_closure.rs`; the private-table churn
assertion lives in `shadow.rs::template_collection_tests`. Assertions cover live
owner retention, descendant-only retention, absent DOM parent links, stale lookup
refusal, remint after collection, sibling reclamation, owner-only negative
retention, and shadow-host retention through replacement. Two churn cases run
1,000 cycles, including direct checks that metadata returns to empty.

Native Windows x86_64, Rust 1.97.1, default library features, dev test profile;
engine, renderer and server: none. The before run had 1 passing and 5 failing
root-closure tests. The after run has 7 passing root-closure tests, 38 existing
crate-source tests (including the new bookkeeping test), 6 S1 regressions and
7 original research assertions: 58 pass. Independent review found no blocking
ownership or pruning issue and reran the seven root tests successfully.

The exact external runner manifest, commands, logs and source-overlay hashes
are supplied here. `s2-before-root_closure.rs` preserves the six-case failing
manifest. The lock is byte-identical to
`../2026-09-08_g5_s1/probe-Cargo.lock`, SHA-256
`07bb062b63fe8d172d26c39e87d6db4287f34d338de3c48984bf4ca743abf4ed`.
The runner and original seven-case harness remain in Mere's
`design_docs/mere_docs/testing/receipts/2026-09-08_stack_pillar_probes/arena/`.
Replay uses Genet `ee0b314b3e9` plus the source overlay in `s2-sources.json`;
the before run uses the S1 overlay and saved before manifest. The unit suite
compiles actual original crate source as an external test target. Concurrent
parser/runtime changes are excluded; this is not a whole-workspace gate.

This is store-level S2 evidence. JS wrapper/observer/range roots, per-owning-
document template ownership, adoption, both-engine and headed receipts remain
open. The pre-existing representation still has one inert owner per arena.
