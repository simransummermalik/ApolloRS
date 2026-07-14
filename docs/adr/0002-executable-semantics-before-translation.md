# ADR-0002: Executable architecture semantics before translation

- Status: Accepted
- Date: 2026-07-13

## Context

A line-by-line Rust translation can compile while misrepresenting AGC words,
banks, timing, interrupts, channels, scheduler behavior, and interpretive
execution.  Without an executable architecture model, translated code and its
tests are likely to share assumptions and defects.

## Decision

Implement and validate the AGC numerical, memory, ISA, CPU, runtime, channel,
interrupt, and interpretive semantics before production transpiler or broad
mission-reconstruction work.  The first original-program proof point is loading
and executing assembled historical code on this model.

Transpilation is an outer-layer consumer of the validated semantic model and
typed source IR.  It may not define semantics needed to make its own output pass.

## Alternatives considered

- **Translate mission source first and patch until outputs look plausible.**
  Rejected because failures cannot be localized and final outputs can conceal
  compensating divergences.
- **Wrap an external emulator and translate only high-level routines.** Useful
  as an oracle adapter, but rejected as the ApolloRS semantic core because it
  does not provide an independently testable reconstruction.
- **Implement parser and IR before CPU semantics.** Some lexical tooling may
  proceed, but production translation remains gated because semantic analysis
  needs an authoritative architecture model.

## Consequences

- Early progress emphasizes small exact models and tests, not visible mission
  coverage.
- Crate dependencies point from translation/validation toward the semantic core.
- Original assembled execution is available as a baseline before generated Rust.
- Unsupported semantics stop execution explicitly.

## Risks introduced

- The architecture model can itself become a second emulator with hidden errors.
- The project may overbuild hardware details not needed for the first slice.
- Visible demonstration work arrives later.

## Validation

- Qualify the model instruction-by-instruction against independent references.
- Use architecture-derived vectors and defect seeding to reduce correlated
  agreement with the external oracle.
- Tie each modeled feature to vertical-slice coverage or a documented broader
  research requirement.
- Enforce workspace dependency checks so transpiler/guidance crates cannot be
  semantic dependencies of the CPU/runtime.

