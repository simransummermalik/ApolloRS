# ADR-0005: Qualified differential oracles and bounded equivalence claims

- Status: Accepted
- Date: 2026-07-13

## Context

ApolloRS needs an external execution reference, but an emulator is not historical
hardware and may contain defects or share interpretations with ApolloRS.  A
single matching final output is weak evidence.  Conversely, requiring universal
whole-program equivalence before publishing any result is impractical.

## Decision

Use a pinned Virtual AGC/yaAGC-family execution as the initial external oracle
candidate, qualify it against architecture vectors and known artifacts, and
triangulate disputed behavior with documentation and, where feasible, another
independent implementation.

Differential comparison starts at raw instruction/event state and reports the
first unexplained divergence.  Equivalence claims are bounded by revisions,
fixture, input stream, start/end checkpoints, observables, and known exclusions.
The levels E0–E5 in the research plan define claim strength.

The faithful generated path, structured path, and original assembled execution
are compared separately.  One path cannot inherit another's status merely by
using the same Rust semantic primitives.

## Alternatives considered

- **Treat one external emulator as ground truth.** Rejected because oracle defects
  would become ApolloRS defects.
- **Use ApolloRS itself to generate expected tests.** Rejected because agreement
  would be circular.
- **Compare only mission outputs.** Rejected because compensating errors and
  timing divergences remain hidden.
- **Avoid equivalence language entirely.** Rejected because carefully bounded,
  reproducible equivalence is a core research output.

## Consequences

- Oracle versions, configurations, patches, and qualification results become
  first-class artifacts.
- Some behavior may remain unresolved even when one implementation appears
  plausible.
- Reports are more precise but cannot make broad whole-program claims early.
- Divergences and negative results are durable outputs.

## Risks introduced

- Building adapters and extracting comparable traces may require modifying or
  instrumenting the external reference.
- Shared source documents can still produce correlated interpretation errors.
- Narrow fixtures can be overgeneralized by readers.

## Validation

- Seed representative arithmetic, bank, timing, interrupt, and comparator defects
  and require detection/classification.
- Publish oracle qualification scope and failures.
- Require claim metadata and evidence manifests in generated reports.
- Review every public equivalence statement against the verification matrix and
  vertical-slice boundary.

