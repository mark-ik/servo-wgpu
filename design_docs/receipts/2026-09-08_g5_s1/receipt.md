# G5 S1 replacement retention, 2026-09-08

Production change: text and fragment replacement orphan former children;
pin-aware collection decides reclamation. `LayoutDomMut::remove` retains its
existing destructive semantics. The new named regression manifest is
`components/genet-scripted-dom/tests/replacement_retention.rs`.

Validated on native Windows x86_64, Rust 1.97.1, default library features,
dev test profile, no JS engine, renderer or server:

- 6 production regression tests pass: empty/nonempty text and fragment
  replacement with observers off/on; a retained descendant's readback,
  parent links, mutation records, reattachment and unpin/reclamation; 1,000
  alternating replacement cycles for each observer setting return to baseline.
- 37 existing crate source tests pass, compiled as a separate external test
  target from the original `lib.rs` and its modules.
- 7 original research assertions pass.

`s1-run.json`, `s1-sources.json` and `s1.log` carry the command, exit 0,
overlay hashes and output. `probe-Cargo.toml` and `probe-Cargo.lock` are the
exact external runner's manifest and lock. The runner and original seven
assertions remain at
`mere/design_docs/mere_docs/testing/receipts/2026-09-08_stack_pillar_probes/arena/`.

The source baseline is Genet `ee0b314b3e9`, with the two files enumerated in
`s1-sources.json` copied from the production change. Concurrent parser/runtime
checkout work was excluded. The external runner's `scripted_dom_unit_suite`
compiles actual crate source as a test target; this is not a full dirty-workspace
Cargo gate. Its private offline Cargo cache and target are execution plumbing,
not artifacts supplied here. Standard package tests can be run on a clean
integrated checkout; exact external replay uses the saved manifest/lock and
the baseline plus recorded source overlay.

The complete G5 JS-root, owner-document, adoption, all-target handle and O5
headed gates remain open. This slice does not repair foreign-handle aliasing.
