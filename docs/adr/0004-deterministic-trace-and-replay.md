# ADR-0004: Deterministic, versioned trace and replay as architecture

- Status: Accepted
- Date: 2026-07-13

## Context

Final state cannot identify the first semantic divergence, and a debugger-only
log is too unstable for research evidence.  Timing, bank, interrupt, channel,
scheduler, and interpreter behavior require a common observable timeline.
Instrumentation must not alter execution.

## Decision

Make a canonical event trace and timestamped input replay protocol part of the
architecture from the first CPU step.

The trace schema is versioned and records raw state transitions, cycle/time,
logical and physical addresses, banks, interrupt lifecycle, channel operations,
scheduler/interpreter boundaries, and source provenance.  Implementations emit
events through a one-way observer interface.  The UI and reports consume traces
or read-only snapshots; they are not semantic dependencies.

Canonical serialization is deterministic.  Normalization for comparison is
explicit, versioned, and forbidden from collapsing signed zeros, bank identity,
or event order without a claim-specific justification.

## Alternatives considered

- **Add logging after the emulator works.** Rejected because observability and
  deterministic input boundaries affect architecture and testability.
- **Compare only registers after each instruction.** Insufficient for memory,
  channels, interrupts, schedulers, and interpreter boundaries.
- **Use free-form text logs.** Rejected because parsing, schema evolution, and
  canonical comparison become unreliable.
- **Expose mutable callbacks for UI integrations.** Rejected because observers
  could change semantic state or order.

## Consequences

- Trace volume and schema evolution need active management.
- Every runtime subsystem must define observable events.
- Headless validation and visual debugging share one evidence source.
- First-divergence reports can become permanent regression fixtures.

## Risks introduced

- Overly detailed traces may make tests and storage expensive.
- Missing events can create false equivalence.
- Schema changes can invalidate golden artifacts.
- Event emission could affect performance or, if poorly designed, behavior.

## Validation

- Repeat each golden scenario and require identical canonical digests.
- Test serializer round trips and adversarial event differences.
- Run tracing off/on and require identical terminal semantic state; where a
  canonical semantic event stream is mandatory, compare through a null sink.
- Version migrations preserve raw events or explicitly invalidate old evidence.
- Audit mandatory observables before each equivalence claim.

