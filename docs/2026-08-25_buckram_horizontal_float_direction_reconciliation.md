# Buckram floats and shapes: horizontal direction reconciliation

**Date:** 2026-08-25

**Status:** Row 12 in progress. Horizontal direction changes and the exposed
negative end-margin BFC case are complete on source commits `17297c75153` and
`bb596757b5f`.

**Parent:** [Buckram and Livery lane program](2026-08-21_buckram_livery_lane_program_plan.md),
row 12. This continues the [relative float-state reconciliation](2026-08-25_buckram_relative_float_state_reconciliation.md).

## Ruling

An ordinary block may continue its parent's block formatting context across an
LTR/RTL boundary. Buckram owns that coordinate conversion:

- translate exclusions into the descendant content box;
- mirror margin and line areas across the descendant content inline size;
- swap the rounded area's inline-start and inline-end corner radii;
- flip the logical `at_inline_start` fact while retaining physical
  `FloatSide`; and
- apply the inverse mirror before descendant-created floats return to the
  parent context.

This admission is horizontal only. Orthogonal and vertical writing modes still
need an axis transform and remain outside the claim.

The new admission exposed a real BFC avoidance defect in
`new-fc-beside-float-with-margin-rtl.html`. An authored negative inline-end
margin contracts the BFC's outer fit and may keep its start-aligned border box
beside an inline-end float. It must not force the BFC below that float. Buckram
now applies that rule in both horizontal directions and rejects it when an
inline-start float also constrains the band. No direction-specific admission
guard was added.

## Exact WPT receipts

The baseline maps come from relative float-state source commit `ca57c615a45`.
The branch was rebased over six Fleece-only commits before the final runner was
built. The pre-rebase and post-rebase candidate maps were status-identical for
shape-box, CSS Shapes, CSS2 floats, and CSS2 floats-clear.

- final candidate runner: `candidate-bb596757b5f-genet-wpt.exe`
- final runner SHA-256:
  `E2FCB94EA203D2ECB4147FC4CBAB439F3660E8848CF30E476A80D9F600448905`
- manifest SHA-256:
  `D5EC5BE9BF1A75ED00D7E7AB28AFE8A694A55E11682BA74305874D70B18DD422`

| Exact subset | Baseline | Candidate | Status movement |
|---|---:|---:|---:|
| `css/css-shapes/shape-outside/shape-box` | 19 pass / 23 fail / 0 skip | 27 pass / 15 fail / 0 skip | 8 fail-to-pass, 0 loss |
| `css/css-shapes` | 36 pass / 187 fail / 148 skip | 44 pass / 179 fail / 148 skip | 10 fail-to-pass, 2 assigned false-pass losses |
| `css/CSS2/floats` | 62 pass / 39 fail / 43 skip | 63 pass / 38 fail / 43 skip | 1 fail-to-pass, 0 loss |
| `css/CSS2/floats-clear` | 98 pass / 113 fail / 38 skip | 98 pass / 113 fail / 38 skip | status-identical |
| `css/css-position` | 45 pass / 73 fail / 226 skip | 45 pass / 73 fail / 226 skip | status-identical |
| full `css` | 7,852 pass / 11,876 fail / 16,583 skip | 7,860 pass / 11,868 fail / 16,583 skip | 11 fail-to-pass, 3 assigned false-pass losses |

The eight shape-box gains are the horizontal LTR/RTL cases for rectangular and
rounded border, content, margin, and padding reference boxes. The three other
full-CSS gains are:

- `shape-outside-circle-047.html`;
- `shape-outside-ellipse-045.html`; and
- `zero-width-floats.html`.

The final comparison covers 36,311 identities. Exactly 14 statuses changed.
The three pass-to-fail results are assigned rather than hidden:

- `shape-outside-ellipse-042.html` and `shape-outside-ellipse-043.html` use
  basic `ellipse()` values. Livery's current `ShapeOutside` model admits only
  `none` and box values. Their baseline matches depended on the old direction
  fallback dropping the surrounding float context, so they were false passes
  outside the implemented shape claim.
- `text-indent-rtl-002.xht` exposes the existing text-direction gap. Livery
  does not yet supply CSS paragraph direction as Parley's bidi base level, and
  `text-indent-rtl-001.xht` already failed at baseline. The old 002 match was a
  fallback coincidence, not a float result.

The exact RTL BFC control passed on current main, failed on the first horizontal
candidate, and passes on `bb596757b5f`. That live sequence forced the Buckram
avoidance repair before landing.

| Final artifact | SHA-256 |
|---|---|
| shape box map | `A8DAC80686E341B3B49D76EAC684677F35F9AEFF47113C5336FCA621FF342290` |
| CSS Shapes map | `E5F3A4AD7760138F402F012A5F55092CFE25B0456AAEDC22041043B8C79BB3BB` |
| CSS2 floats map | `1E78208E2568C3A20C30ECE40E5C08FEB10FD9FC30BD11EFF956462ADB456F4E` |
| CSS2 floats-clear map | `B5E4F9D441C08C046B14B041F7199A8D9F2E9A68E28710B76B2B878BEA6F9E2A` |
| css-position map | `D96098B7292D0F3A1927A7377B7A26D3848C6C0C463A2EEF655F78531F39C582` |
| full CSS map | `CFB3127DC0AD914A480C91819088A5C77DF8E0F667D1D1F7D70BACC31E7718FD` |

All maps, control runners, and frozen candidates are under
`testing/genet/wpt-ledger/2026-08-25_horizontal_float_direction`.

## Native wall

- Buckram is 254 / 254 on the final source.
- Genet-Livery is 213 / 213 on the final source.
- The live Livery receipt proves rounded shape constraints across an LTR/RTL
  boundary with six atomic inline boxes and zero CSS-facing Taffy block runs.
- Buckram's reciprocal BFC tests cover RTL/physical-left and
  LTR/physical-right negative end margins, plus the two-sided-float guard.
- Buckram strict Clippy passes for library and tests with warnings denied.
- Changed-file rustfmt and `git diff --check` pass.
- The final release runner built `--locked --offline -j 1`. The rebase first
  exposed a stale ignored lock, which was regenerated offline before the
  accepted locked build.

## Remaining row 12 work

The exact shape-box map has 15 red files:

- 2 horizontal padding-box cases have correct layout but a localized
  border/overlap paint difference;
- 2 rounded margin-box cases use floats split from an inline formatting
  context;
- 8 rounded border-box cases use vertical or sideways writing modes and need
  the deferred axis transform; and
- 3 rectangular box cases use `shape-margin` and need an expanded contour.

Basic shapes beyond the box model and shape images also remain open. The next
Row 12 slice must select one of those owned seams and retain the same exact-map
stop rule.

## Done condition

This slice is complete because horizontal direction changes preserve and
return rectangular and rounded float geometry, the exposed BFC avoidance bug
is repaired in both directions, native gates are green, and every full-CSS
status change is either a measured gain or an assigned false pass outside the
implemented float-shape claim. Row 12 remains in progress for the 15 classified
shape-box residuals and the wider shape families.

## Progress — 2026-09-11: RTL root body geometry

`9ad96bd6f4f` removes the UA `body { inline-size: 100% }` declaration while
retaining its default 8px margins. Automatic body sizing now resolves inside
those margins in LTR, RTL, and vertical roots; an authored `inline-size: 100%`
continues to overflow them. The implementation also adds Buckram's physical
margin invariant and updates the affected automatic-flex expectation.

Focused native checks on the committed source passed: Buckram's RTL physical
margin invariant (1), Livery's root-body geometry cases (5), and the related
automatic-flex regression (1). This is a root-block sizing correction, not a
new float or shape receipt; the Row 12 map and its remaining work are unchanged.
