# Documentation Policy

> **Canonical core v1 (2026-08-24).** Everything from "## Core principles" down
> to the end of §10 is the shared core, copied verbatim into every repository
> under `Code` that keeps a `design_docs/`. It is not owned by any one repo:
> change it in all of them or not at all, so that `diff` between any two copies
> shows only local addenda. Repo-specific rules belong under
> **Local addendum** at the foot of this file, never inside the core.

## Core principles

### 1. Control doc growth

Add to an existing doc unless the material is substantial (>500 words), covers
a distinct topic, and is unrelated to any current document. Keep the total doc
count low. Do not create a file for a one-time analysis.

### 2. Eliminate redundancy

Audit before commits and after substantial changes. Newer documents are
generally more authoritative. If two docs disagree, reconcile them — do not let
drift accumulate. Material shared across several repos lives once, in a named
home, and is cited by path from the others; never copied.

### 3. No legacy friction

When a path changes, optimize for clean fit with the new path. Do not preserve
obsolete parallel systems or migration shims unless they are needed for real
user data. Tests track current semantics only.

### 4. Location and archival

- **Active docs** live directly in `design_docs/`. Flat is fine, and is the
  right default. When one domain accumulates enough material to justify it,
  promote that domain to an area root, `design_docs/<area>_docs/`.
- **Area roots**, once a repo has them, take a consistent set of category
  subdirectories. Use only the ones a given area needs:

  | Category | Holds |
  |---|---|
  | `research/` | briefs, surveys, reports, critiques, design probes |
  | `technical_architecture/` | component definitions, boundaries, interfaces, decisions |
  | `implementation_strategy/` | dated plans, development approaches, roadmaps |
  | `design/` | UI/UX, interaction design, accessibility |
  | `testing/` | test plans, harness docs, manual checklists |

- **Docs live with the repo that owns the subject, at that repo's doc root.**
  Do not scatter `design_docs/` into member crates of a workspace: a doc in a
  member crate is invisible to the canonical index, which is a violation of §6
  rather than a matter of taste.
- **Archive**: `design_docs/archive_docs/<YYYY-MM-DD>/` for retired plans and
  superseded notes. Check for an existing checkpoint folder before creating a
  new one. Move rather than delete; delete only with rationale and
  confirmation.

### 5. Cross-referencing

- Within a repo: relative links.
- Across repos: cite by path (`isometry/design_docs/...`), since relative links
  do not cross repository boundaries reliably and rot silently when the
  neighbour moves or is archived.
- Crates: link to crates.io when referring to a public API
  (`https://crates.io/crates/<name>`).
- When a doc moves, repair the links that pointed at it in the same session.

### 6. DOC_README authority

`design_docs/DOC_README.md` is the sole canonical index. It must contain:

- AI-assistant working principles for this project
- An index of all active docs with one-line descriptions
- Pointers to `DOC_POLICY.md` and `PROJECT_DESCRIPTION.md`

Any doc added, moved, or removed requires a `DOC_README.md` update in the same
session. If any other index disagrees with `DOC_README.md`, `DOC_README.md`
wins.

### 7. PROJECT_DESCRIPTION.md ownership

`design_docs/PROJECT_DESCRIPTION.md` — inside the doc root, not at the
repository root — is reserved for the maintainer. Do not edit it without
explicit instruction. Treat it as authoritative and surface contradictions for
discussion rather than resolving them silently.

The root `README.md` is derived from `PROJECT_DESCRIPTION.md` and the current
authoritative docs. Speculative features without plans appear only in
`PROJECT_DESCRIPTION.md`.

### 8. Plan documents

Work that changes code — not doc-only work — gets a dated plan named
`<YYYY-MM-DD>_<keyword>_plan.md`, in `design_docs/` or, where the repo has area
roots, in `<area>_docs/implementation_strategy/`. Each plan carries:

- A dated **Status** line, kept current: plan, in progress, landed, superseded
  by X.
- **Phases** organised by feature target and validation criteria, each with
  **done-conditions**. Never calendar labels — no "Day 1", no "Week 2" — and
  never time estimates.
- A **Findings** section for facts verified during the work, dated, with code
  references.
- A **Progress** log, dated, appended as phases land.

Code samples in a plan state whether they are illustrative or compile-ready.

Update the plan every two prompts on the project, or every two completed tasks.
Re-read it before resuming work rather than working from memory of it. On
completion, extract any deferred or still-open points into a new or existing
plan *before* moving it to `archive_docs/<date>/`.

### 9. Implementation feedback loop

Every implementation pass is also a design probe. After each pass, disseminate
structural learnings to the relevant plans and docs in the same session.
Surface architectural problems explicitly in the plan even when the fix is
deferred.

### 10. Workflow rule for AI assistants

Read `DOC_README.md` first, then this policy, before starting work. Any durable
working principle learned during a session is promoted into `DOC_README.md`'s
working-principles section in that same session.

## Local addendum — genet

`design_docs/` was founded 2026-08-24, when the canonical core was distributed
across the workspace and the component documents describing inker, nematic and
verso-tile were repatriated here from mere, where they had been orphaned since
the code moved.

### Two doc homes, and the boundary between them

**This repository currently has two documentation directories.** That is a
known duplication, decided deliberately by Mark on 2026-08-24, and recorded
here rather than left to be discovered:

| Directory | What is in it | Governed by this policy? |
|---|---|---|
| `design_docs/` | the component areas below, and everything written from 2026-08-24 onward | **yes** |
| `docs/` | ~166 dated notes, plans, audits and studies on the engine itself — layout, styling, scripting, WPT conformance, the servo-stack lift | **not yet** |

The boundary is **date and governance, not subject matter**. `docs/` is the
older flat corpus; it predates any policy here and has no separate index.
Since 2026-09-05, `design_docs/DOC_README.md` links selected current execution
entry points there; it does not inventory the whole corpus or bring it under
this policy. `design_docs/` is the policy-governed tree.

**New documents go in `design_docs/`.** Do not add to `docs/` — that only
deepens the split.

**One recorded exception.** Four `2026-08-25_buckram_*_reconciliation.md`
files entered `docs/` in `e4d14718` the day after this rule was written. They
stay where they are, recorded here rather than moved, until the migration
below takes the whole corpus; nothing after them has gone to `docs/`.

**Migrating `docs/` is open work and is not scheduled.** It is the obvious
end state and it is genuinely expensive: 166 files, plus **53 references to
`genet/docs/` across the workspace from 34 files outside this repository**
(counts re-taken 2026-09-02; they were 163, 49 and 32 at founding),
each of which would need repair under core §5. It was deferred rather than
attempted because doing it badly is worse than the current split. Until it
happens, core §6's "sole canonical index" claim covers `design_docs/`.
The selected `docs/` entry points are a navigation aid, not a claim that its
166-file inventory or migration is complete.

### Area roots

```
genet/design_docs/
├── DOC_README.md              ← canonical index (§6)
├── DOC_POLICY.md              ← this file
└── archive_docs/              ← superseded documents, dated on retirement
```

`inker_docs/`, `nematic_docs/` and `verso_docs/` moved to mere on 2026-09-03
with the code they mirror, under the platform boundary plan. No topic area
root remains; genet's dated plans sit directly under `design_docs/`.

### What belongs here rather than in smolweb

genet is the **implementation** side of the smolweb split. The rule was decided
by Mark on 2026-08-03 and lives at
`smolweb/design_docs/technical_architecture/2026-08-03_smolweb_home_decision.md`:
spec-accurate wire and grammar implementations belong to the smolweb workspace;
enrichment, lowering, rendering, theming and trust chrome are ours. A doc about
how a browser *uses* a protocol belongs here. A doc about *what the protocol
is* belongs there, cited by path.

### PROJECT_DESCRIPTION.md

genet has no `design_docs/PROJECT_DESCRIPTION.md`. The root `README.md` carries
the description today, so core §7's derivation rule is inert here rather than
violated.
