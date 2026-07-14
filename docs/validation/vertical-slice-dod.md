# Definition of done: first flagship vertical slice

## Slice identity

**ID:** `VS-LM-P63-001`

**Program:** Luminary 099 at the pinned historical commit.

**Purpose:** demonstrate the complete ApolloRS method on a landing-related path:
historical source assembly, original-code execution, typed semantic execution,
faithful or reconstructed Rust, synchronized trace comparison, provenance, and
a debugging visualization.

**Provisional execution boundary:** enter label `P63LM` in
`THE_LUNAR_LANDING.agc`, include the P63 ignition/landing setup and transitions
needed to reach `LUNLAND`, and complete the first declared landing-guidance job
cycle ending at the `ENDLLJOB` transfer in
`LUNAR_LANDING_GUIDANCE_EQUATIONS.agc`.

The exact start address, terminal event, and any required pre-roll are frozen
only after the assembler listing and reference trace exist.  Changing the
boundary after observing a divergence creates a new slice ID; it does not revise
this result in place.

## Required implementations

1. Pinned yaYUL-compatible assembly of the unchanged source through a documented
   overlay, producing rope/listing/symbol artifacts.
2. Execution of the assembled source in a qualified external reference.
3. Execution of the same assembled source in the ApolloRS architecture model.
4. Faithful generated Rust or an explicitly provenance-bearing Rust
   reconstruction for the selected code, using the same semantic primitives.
5. A structured/idiomatic variant is optional for the first pass; if present it
   is a third comparison target and receives no inherited verification status.

## Fixture contract

The fixture must contain:

- source, assembler, emulator, ApolloRS, IR, and trace-schema revisions;
- complete registers, erasable memory, bank registers, interrupt state, channel
  and peripheral state, simulated time, scheduler state, and interpreter state;
- all fixed-memory image hashes and symbol/source maps;
- every external input with exact AGC cycle timestamp and source;
- engineering-unit metadata for selected guidance values without replacing raw
  words;
- fixture provenance explaining whether state was captured from a qualified
  mission run, constructed from documentation, or synthesized for a focused
  scenario;
- validation that no unspecified host clock, randomness, or UI input can affect
  the run.

No invented “Apollo 11 mission state” may be presented as historical.  A
synthetic state is acceptable when labeled and internally validated.

## Mandatory observables

At every applicable event, compare:

- fetched raw word, decoded instruction/order, source location, and PC/Z;
- A, L, Q and interrupt-save registers as raw representations;
- EBANK, FBANK, BBANK/SBANK and resolved physical addresses;
- all erasable writes and declared special-register side effects;
- channel reads/writes and modeled peripheral transitions;
- cycle count, simulated time, interrupt requests/acceptance/entry/return;
- Executive job/core-set/priority transitions and Waitlist task transitions;
- basic/interpreter boundaries, interpreter location/mode, and selected MPAC
  words;
- phase/restart/alarm state;
- selected P63 raw guidance state and DSKY/display outputs.

Derived decimal or engineering-unit values are reported only beside raw values
and explicit scale metadata.

## Acceptance criteria

The slice is done only when all of the following are true:

1. Historical source files match the Phase 0 SHA-256 manifest.
2. Assembly repeats bit-for-bit in two clean runs and records every overlay.
3. The external oracle passes the qualification suite for every instruction and
   runtime feature exercised by the slice.
4. Three repetitions of each implementation produce identical canonical trace
   digests.
5. Original source on ApolloRS and the external reference agree for every
   mandatory observable from the frozen start through terminal event.
6. Faithful/reconstructed Rust agrees with the qualified original execution for
   every mandatory observable through the same boundary.
7. There are zero unexplained divergences.  A documented divergence leaves the
   slice incomplete; it may still be published as a negative intermediate
   result.
8. Every executed event maps to original source and, where applicable, generated
   Rust provenance.  Mapping coverage and exclusions are reported.
9. Arithmetic, banking, interpreter, interrupt, scheduler, and I/O edge cases
   exercised by the trace have focused regression tests.
10. At least three deliberate semantic defect seeds—one arithmetic, one bank,
    and one timing/interrupt defect—are detected and correctly classified by the
    comparator.
11. A reviewer can rebuild and replay the evidence bundle from a clean checkout
    using documented commands.
12. The report states the exact instruction/event counts, modules and labels
    reached, observables, limitations, and highest equivalence level attained.

## Visual demonstration criteria

The research interface must allow a viewer to:

- view original and Rust source at the current event;
- step or run both executions against one canonical timeline;
- inspect raw/octal registers, banks, memory writes, interrupt state, scheduler
  state, interpreter state, guidance variables, and DSKY output;
- see matching state or the first divergence without hiding earlier context;
- inject at least one versioned fault and replay it deterministically;
- export the exact trace and divergence report used by the display.

The UI must be a projection of authoritative runtime/trace state.  Enabling it
must not change the canonical trace digest.

## Explicit non-claims

Completion does not establish whole-Luminary, whole-landing, flight-dynamics,
hardware, safety, or Apollo 11 mission equivalence.  It establishes only the
declared equivalence level for `VS-LM-P63-001`, its fixture, inputs, observables,
and checkpoint interval.

