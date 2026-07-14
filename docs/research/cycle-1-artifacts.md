# First development cycle artifact ledger

Cycle 1 establishes the evidence chain from pinned source to a reproducible
assembly and reviewed architecture plan.  It does not implement the production
translator or claim execution equivalence.

## Foundation artifacts

| ID | Artifact | Acceptance condition | Current status |
|---|---|---|---|
| C1-01 | Historical source checkout and provenance note | URL/commit recorded; checkout clean; no `.agc` modifications | complete |
| C1-02 | SHA-256 source manifest | All 175 `.agc` files covered; deterministic `--check` passes | complete |
| C1-03 | Repository inventory JSON/CSV | Both programs and every module recorded with line count/hash/include state | complete |
| C1-04 | Textual include graph JSON/DOT | Ordered `MAIN.agc` edges; unresolved includes retained | complete |
| C1-05 | Repository-forensics report and subsystem diagram | Scope/evidence levels, initial findings, entry-point map, and completion gate documented | complete |
| C1-06 | Reproducible forensics generator | Standard-library tool regenerates and checks all lexical artifacts | complete |

## Research and architecture artifacts

| ID | Artifact | Acceptance condition | Current status |
|---|---|---|---|
| C1-07 | Research plan | Questions, hypotheses, methodology, references, criteria, limits, reproducibility, contributions | complete |
| C1-08 | Semantic risk register | Risks ranked by severity, likelihood, testability, priority, and control | complete |
| C1-09 | Proposed Rust workspace | Crate ownership, exclusions, dependency direction, implementation gate | complete |
| C1-10 | Gated roadmap | Deliverables and measurable exit gate for Phases 0–9 | complete |
| C1-11 | Verification matrix | Layer, oracle, observable, method, acceptance evidence, and honest status | complete |
| C1-12 | Flagship vertical-slice DoD | P63 boundary, fixture, observables, acceptance, visual demo, non-claims | complete |
| C1-13 | ADR-0001 through ADR-0005 | Choice, alternatives, consequences, risks, and validation in each record | complete |
| C1-14 | Assumption register | Consequential open choices and interim defaults are explicit | complete |
| C1-15 | Paper skeleton | Required sections exist with measurement placeholders and integrity warning | complete |

## Assembler-backed artifacts still required in Cycle 1

| ID | Artifact | Acceptance condition | Current status |
|---|---|---|---|
| C1-16 | Pinned assembler/reference tool manifest | Source revisions, build commands, environment image, licenses, patches, hashes | not started |
| C1-17 | Source compatibility overlay | Both unresolved include tokens represented explicitly; historical files unchanged | not started |
| C1-18 | Comanche 055 assembly bundle | Rope image, listing, symbols, warnings, command/env manifest; two clean runs match | not started |
| C1-19 | Luminary 099 assembly bundle | Same evidence as C1-18, including overlay provenance | not started |
| C1-20 | Assembler-backed memory map | Fixed/erasable banks, fixed-fixed regions, symbols, allocation extents, aliases | not started |
| C1-21 | Symbol/cross-reference graph | Definition/reference edges with file/line/address and unresolved coverage | not started |
| C1-22 | Reviewed subsystem catalog | Every module tagged by source review, with shared/variant relationships | not started |
| C1-23 | Entry-point catalog | Interrupt, interpreter, Executive, Waitlist, DSKY, restart/alarm, guidance/control addresses | not started |
| C1-24 | Source discrepancy report | Include names, assembler diagnostics, overlay effects, upstream evidence, disposition | not started |
| C1-25 | Phase 0 review record | Gate checklist signed/reviewed; approved scope for `agc-word` implementation | not started |

## Cycle completion rule

Cycle 1 is complete when C1-01 through C1-25 are complete or a remaining item is
documented as a reproducible blocker with attempted methods and required external
input.  Production Rust semantics begin only after C1-25 accepts the Phase 0
gate.

