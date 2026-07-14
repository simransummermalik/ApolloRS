# ApolloRS

ApolloRS is a research-grade reconstruction, execution, and verification
framework for Apollo 11 Guidance Computer software in Rust.  The project is in
Phase 0: repository forensics and research design.  No emulator, translation,
or behavioral-equivalence claim exists yet.

The governing principle is simple: preserve the historical source, build an
executable model of the AGC architecture, and require differential evidence
before describing any Rust reconstruction as equivalent.

## Current baseline

- Historical corpus: `chrislgarry/Apollo-11`
- Pinned upstream commit: `247dd7d0d1b0e7f9f270750ec08983e0a72e73e1`
- Programs: Comanche 055 (Command Module) and Luminary 099 (Lunar Module)
- Initial inventory: 175 `.agc` files and 130,186 physical lines
- Production implementation status: not started; blocked by the Phase 0 gate

The upstream checkout is held under `historical/Apollo-11/` and is treated as
read-only.  Its provenance and handling rules are recorded in
`historical/README.md` and ADR-0001.

## Phase 0 deliverables

- `docs/research/repository-forensics.md` — forensic method and initial findings
- `docs/research/research-plan.md` — questions, hypotheses, and evaluation plan
- `docs/research/semantic-risk-register.md` — ranked mistranslation risks
- `docs/research/roadmap.md` — gated milestone sequence
- `docs/architecture/workspace.md` — proposed Rust workspace and dependencies
- `docs/validation/verification-matrix.md` — evidence required at each layer
- `docs/validation/vertical-slice-dod.md` — first flagship definition of done
- `docs/adr/0001-*.md` through `0005-*.md` — initial architecture decisions
- `docs/research/assumptions.md` — decisions that still require confirmation
- `docs/research/cycle-1-artifacts.md` — first-cycle artifact ledger

Machine-readable forensic outputs live in `artifacts/forensics/`.  Regenerate
and verify them with:

```sh
python3 tools/forensics/inventory.py \
  --source historical/Apollo-11 \
  --output artifacts/forensics

python3 tools/forensics/inventory.py \
  --source historical/Apollo-11 \
  --output artifacts/forensics \
  --check
```

These outputs are lexical inventories and textual include graphs.  They are not
assembler symbol tables, call graphs, memory maps, or evidence of execution.

## Evidence vocabulary

ApolloRS uses four deliberately separate terms:

1. **Historical emulation**: execution of assembled AGC code on a modeled AGC.
2. **Mechanical translation**: source or IR transformed into Rust without a
   readability claim.
3. **Idiomatic reconstruction**: a manually structured Rust implementation.
4. **Behaviorally verified equivalence**: agreement for named observables,
   initial conditions, inputs, oracle, and a bounded execution interval.

Only the fourth term is an equivalence claim, and every such claim must identify
its exact boundary and supporting artifacts.

