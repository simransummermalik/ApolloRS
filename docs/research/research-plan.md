# ApolloRS research plan

Status: Phase 0 proposal.  Results sections remain intentionally empty until
measurements are produced from versioned artifacts.

## Central question

How can Apollo 11 Guidance Computer software be reconstructed in modern Rust
while preserving the observable semantics of the original architecture,
including one's-complement arithmetic, banked memory, fixed-point scales,
interrupts, timing, channels, scheduling, interpretive execution, and restart
behavior?

## Research questions

**RQ1 — Semantic fidelity.** Which AGC behaviors must be represented explicitly
to reproduce instruction and mission-routine traces, and which modern
abstractions preserve those behaviors without hiding historical state?

**RQ2 — Verification.** At what granularity can an independent Rust model be
compared with an established AGC execution environment, and which observables
localize divergence most effectively?

**RQ3 — Translation.** Can a bank-aware, provenance-preserving IR support both
instruction-faithful and structured Rust generation without conflating
readability with equivalence?

**RQ4 — Type-system value.** Which reconstruction errors are prevented or made
reviewable by distinct word, address-space, bank, channel, and fixed-point scale
types?

**RQ5 — Recovery behavior.** Under controlled faults, which invariants and
restart mechanisms are preserved by the historical execution and bounded Rust
reconstructions, and where do behaviors diverge?

**RQ6 — Generalization.** Which parts of the method—provenance, executable
semantics, typed IR, trace comparison, and fault injection—transfer to other
historical embedded software?

## Hypotheses

| ID | Hypothesis | Falsifying evidence |
|---|---|---|
| H1 | Explicit signed-zero and end-around-carry semantics eliminate a class of divergences that appears when AGC words are modeled as ordinary signed integers. | An ordinary-integer model matches the complete arithmetic and instruction corpus under the same observables without hidden correction logic. |
| H2 | Strong address-space and scale types detect reconstruction defects before execution that would otherwise become bank or numerical divergences. | Defect-seeding experiments show no material difference in detection time or escaped defects. |
| H3 | Instruction/event traces localize faults more reliably than final-state comparison alone. | Controlled defects are localized equally well using only final state under a predeclared metric. |
| H4 | A typed, bank-aware IR can retain source provenance while enabling structured Rust for a bounded landing slice. | Required transformations lose source/bank provenance or cannot be validated against faithful execution. |
| H5 | A bounded Luminary landing slice can achieve bit- and event-level agreement for declared observables under a reproducible initial-state fixture. | Any unresolved divergence remains at the declared endpoint, or the fixture cannot be independently reproduced. |
| H6 | Restart/fault experiments reveal behaviorally meaningful differences not exposed by nominal execution. | The declared fault suite produces no additional invariant, recovery, or divergence information. |

Hypotheses may be rejected.  Rejection is a reportable result, not a reason to
weaken acceptance criteria after observing data.

## Systems under comparison

1. **Historical source execution:** binaries assembled from the pinned
   Comanche 055 or Luminary 099 source through a pinned yaYUL-compatible tool.
2. **External reference execution:** a pinned Virtual AGC/yaAGC-family emulator
   is the initial oracle candidate, subject to baseline qualification and trace
   extraction.  Its version and patches are not yet selected.
3. **ApolloRS semantic model:** the Rust AGC word, memory, ISA, CPU, runtime,
   channel, and interpretive execution model.
4. **Faithful generated Rust:** IR-generated state transitions retaining labels
   and instruction boundaries.
5. **Structured or idiomatic Rust:** selected manually or mechanically
   structured routines, compared only within named boundaries.

No implementation is assumed infallible.  External reference results will be
triangulated with architecture documentation, assembler listings, focused
test vectors, and—where available—another independently implemented emulator.
If references disagree, the disagreement becomes an unresolved research item,
not a majority vote.

## Evaluation methodology

### 1. Corpus and provenance

- Pin every source, tool, document, scenario, and generated artifact by version
  or cryptographic digest.
- Keep historical source bytes read-only.
- Record all compatibility overlays separately.
- Emit source-file, source-line, assembled-address, and generated-code mappings.

### 2. Exact numerical semantics

- Exhaust raw-word conversions and unary operations over the 15-bit state
  space.
- Test positive and negative zero as distinct raw states.
- Use boundary partitions and property-based generation for binary arithmetic,
  double precision, shifts, multiplication, division, and scaling.
- Run partitioned exhaustive binary sweeps outside normal CI where feasible.
- Derive expected values independently from the implementation under test.

### 3. Instruction and architecture semantics

- Define each instruction as a state transition over registers, memory, banks,
  channels, interrupt state, and cycle count.
- Exercise normal, signed-zero, overflow, boundary-address, and bank-transition
  cases.
- Test central/edit-register behavior and unprogrammed sequences explicitly.
- Require a cited interpretation source and at least one trace fixture for every
  supported instruction.

### 4. Differential execution

Given identical initial state and timestamped input events, compare:

- instruction identity and program counter;
- A, L, Q, Z and interrupt-save registers;
- EBANK, FBANK, BBANK and resolved physical addresses;
- erasable writes and selected rope reads;
- channel reads/writes and peripheral state transitions;
- interrupt request, acceptance, masking, entry, and return;
- cycle count and simulated time;
- Executive job and Waitlist task transitions;
- interpreter boundaries, mode state, and selected MPAC values;
- alarm/restart state and declared guidance outputs.

The comparator classifies the first unexplained mismatch as decode, arithmetic,
memory-bank, timing, interrupt, I/O, interpreter, scheduler/restart, or unknown.
Every confirmed defect receives a minimal permanent regression fixture.

### 5. Vertical-slice experiment

The first flagship target is a bounded Luminary 099 P63 landing path defined in
`docs/validation/vertical-slice-dod.md`.  The experiment will use a recorded
initial-state and input fixture, execute historical source and Rust paths, align
their traces, report all selected observables, and visualize agreement or the
first divergence.

### 6. Defect-seeding and fault injection

Separate two experiment families:

- **Reconstruction defect seeds:** wrong sign/zero handling, bank selection,
  scale, instruction decode, cycle count, or scheduler order.  Measure whether
  type checking, unit tests, or differential traces detect each seed and at what
  stage.
- **Mission/runtime faults:** delayed inputs, repeated/missed interrupts,
  corrupted words, stuck channel bits, overload, invalid sensor values, and
  restart triggers.  Measure alarms, invariant failures, recovery, final state,
  and cross-implementation divergence.

Seeds and injected faults must be versioned, deterministic, and disabled in
baseline runs.

## Observable equivalence criteria

An equivalence statement is valid only when it names:

1. source and implementation revisions;
2. assembly and execution tools;
3. initial state and complete input stream;
4. start and end checkpoints;
5. observables and comparison normalization;
6. matched event/instruction count;
7. excluded state and justification;
8. all known divergences;
9. reproducible evidence bundle.

Equivalence levels are intentionally bounded:

| Level | Required agreement |
|---|---|
| E0 — build | Artifacts compile/assemble; no behavioral claim. |
| E1 — arithmetic | Named numerical operations agree over declared test domains. |
| E2 — instruction | One-step transitions agree for named instructions and state partitions. |
| E3 — trace prefix | All declared events agree from a named initial state through a named checkpoint. |
| E4 — scenario | E3 plus declared mission outputs, timing, interrupts, and terminal state for a complete scenario. |
| E5 — subsystem envelope | Multiple scenarios and fault cases cover a declared subsystem input envelope. |

Passing a higher level for one slice says nothing about untested modules.

## Measures

- arithmetic state-space coverage and failed cases;
- instruction/state-partition coverage;
- assembled words and checksum agreement;
- matched instructions and events before first divergence;
- divergence class and localization distance;
- source modules, labels, and addresses exercised;
- interrupt, bank-switch, channel, scheduler, and interpreter-boundary coverage;
- defects caught by compiler, unit tests, trace comparison, or manual audit;
- wall time, simulated cycles, memory use, and trace volume;
- nominal and fault-scenario recovery outcomes.

Coverage percentages will not be published without a stated denominator.

## Reproducibility plan

- Pin Rust, assembler, reference emulator, OS/container image, and dependencies.
- Provide one command to build each evidence bundle from a clean checkout.
- Store small fixtures and metadata in the repository; store large rope images,
  traces, and visual assets under a content-addressed artifact policy.
- Include command lines, environment metadata, source/tool digests, schemas,
  seeds, and checksums in each bundle.
- Separate deterministic semantic tests from platform performance benchmarks.
- Run a clean-room reproduction before a release or paper artifact is tagged.
- Preserve raw results and derive tables/figures by checked scripts.

## Limitations and threats to validity

- The GitHub corpus is a transcription/adaptation, not the physical rope or
  original development environment.
- yaYUL syntax and behavior differ from historical YUL/GAP; compatibility work
  can affect provenance and assembly.
- External emulators may share documentation interpretations or defects.
- Complete spacecraft hardware, sensor dynamics, and mission environmental
  state are outside the initial model.
- A vertical slice cannot establish whole-program or whole-mission equivalence.
- Trace instrumentation can omit semantically relevant state unless the schema
  is audited.
- Property tests establish sampled properties unless their domain is exhausted.
- Manual routine reconstruction introduces researcher judgment and confirmation
  bias; faithful execution remains the comparison baseline.
- Performance results depend on host, compiler, tracing level, and artifact
  configuration and must be reported separately from correctness.

## Expected contributions

- An executable, typed specification of central AGC semantics.
- A provenance-preserving route from source through assembly, execution, IR,
  generated Rust, and evidence.
- A differential trace vocabulary for banked, interrupt-driven historical
  systems.
- Empirical evidence about where Rust types help—and do not help—software
  recovery.
- A bounded, auditable Apollo landing reconstruction and debugging instrument.
- A reusable methodology and negative-result catalog for digital preservation
  of embedded software.

## Result-integrity rule

Results, citations, checksums, benchmarks, coverage, and equivalence percentages
remain `TBD` until generated and reviewed.  A failed experiment or unresolved
divergence is recorded as such; it is never converted into a success by changing
the observable set after the run.

