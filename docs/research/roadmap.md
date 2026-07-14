# Gated milestone roadmap

The roadmap is evidence-gated rather than date-gated.  A phase may overlap with
documentation or tooling from later phases, but no later semantic claim is
available until its gate passes.

## Phase 0 — Forensics and research foundation

**Deliverables**

- immutable source baseline and SHA-256 manifest;
- complete lexical repository inventory and textual include graph;
- reproducible assembly plan and explicit source-overlay policy;
- assembler-backed symbols, banks, declarations, and output manifests;
- reviewed subsystem and entry-point maps;
- research plan, semantic risk register, verification matrix, workspace design,
  initial ADRs, assumptions, and first-slice definition of done.

**Gate P0**

- both programs have deterministic assembly bundles or a precisely documented
  blocker;
- source bytes remain unchanged;
- all include discrepancies are explicit;
- every Phase 0 artifact has scope and evidence level;
- the first vertical-slice boundary and oracle qualification plan are reviewed.

Production emulator/transpiler code begins only after P0.  Small forensic and
artifact-generation tools are allowed during Phase 0.

## Phase 1 — Exact numerical semantics

**Deliverables**

- `AgcWord`, `AgcDoubleWord`, checked raw/integer conversions, octal formatting;
- explicit positive/negative zero and end-around-carry operations;
- typed AGC fixed-point values with scale-preserving operations;
- typed address and bank identifiers that cannot be interchanged accidentally;
- formal/pseudocode arithmetic semantics and independent expected-value model;
- exhaustive unary/raw tests, boundary suites, properties, and scheduled
  partitioned exhaustive binary tests.

**Gate P1**

- all 15-bit raw patterns round-trip;
- signed-zero and documented arithmetic boundaries pass;
- no authoritative state relies on host signed integers or floating point;
- mutation/defect seeds demonstrate that the tests detect representative
  arithmetic errors.

## Phase 2 — Memory, ISA, and deterministic CPU

**Deliverables**

- central registers, erasable banks, fixed banks, fixed-fixed regions, edit
  registers, and logical-to-physical address mapping;
- complete basic and extracode decode representation;
- deterministic single-step CPU and cycle accounting;
- interrupt-save state and channel interfaces;
- instruction semantics documents with source citations and trace examples.

**Gate P2**

- address-map tests exhaust logical boundary classes;
- every implemented instruction passes normal and edge partitions;
- no unsupported instruction is hidden by a no-op or placeholder;
- one-step differential tests agree with a qualified external execution path
  for the declared instruction set.

## Phase 3 — Runtime, original assembly loading, and trace baseline

**Deliverables**

- loader for rope/listing/symbol artifacts from the pinned assembly pipeline;
- clocks, unprogrammed sequences, interrupt arbitration, and I/O channel model;
- versioned trace/event schema, deterministic replay, and source mapping;
- first execution of original assembled AGC code in ApolloRS;
- repeatable register, memory, bank, interrupt, channel, and cycle traces.

**Gate P3**

- identical runs produce identical canonical trace digests;
- original code reaches predeclared checkpoints without unsupported semantics;
- source/assembled-address mappings resolve all events in the trace prefix;
- external and ApolloRS traces have a documented alignment method.

## Phase 4 — Differential verification harness

**Deliverables**

- external-oracle adapter and qualification corpus;
- initial-state and timestamped-input fixture schemas;
- streaming comparator and first-divergence report;
- mismatch classes for decode, arithmetic, memory bank, timing, interrupt, I/O,
  interpreter, scheduler/restart, and unknown divergences;
- regression minimizer and permanent fixture workflow.

**Gate P4**

- deliberately injected defects land in the expected mismatch classes;
- comparator adversarial tests preserve signed zero, banks, and event order;
- at least one interrupt-bearing original-code trace prefix agrees for every
  declared observable;
- all unexplained differences remain visible and block the corresponding claim.

## Phase 5 — AGC source front end and typed IR

**Deliverables**

- loss-aware lexer/concrete syntax tree and typed AST;
- assembler-differential symbol and pseudo-op semantics;
- bank-aware CFG/data references with unresolved-edge representation;
- interpretive-block representation;
- provenance for comments, labels, source lines, scales, declarations, and
  assembled addresses;
- versioned IR schema and inspection tools.

**Gate P5**

- parser never silently guesses on the pinned corpus;
- symbols and emitted words agree with assembler artifacts for supported input;
- every IR node maps to source, and every generated instruction maps to IR;
- graph coverage and unresolved edges are measured and reported.

## Phase 6 — Faithful and structured Rust generation

**Deliverables**

- faithful generator preserving instruction boundaries and labels;
- structured transformations with preconditions and provenance;
- generated-code metadata naming source module, label, mode, status, and
  assumptions;
- trace hooks shared with original-code execution without shared semantics;
- focused source-to-generated-code audit process.

**Gate P6**

- faithful generated execution agrees through selected trace checkpoints;
- each structured transform has a trace-preservation regression test;
- unsupported control flow blocks structured generation rather than being
  guessed;
- compilation alone is never reported as verification.

## Phase 7 — Flagship Luminary landing slice

**Deliverables**

- bounded P63 fixture and checkpoint contract;
- original, faithful generated, and selected structured executions;
- bit/event trace comparison and numerical-error report;
- source/Rust split-screen debugging view;
- complete evidence bundle and limitation statement.

**Gate P7**

All criteria in `docs/validation/vertical-slice-dod.md` pass.  The claim is
limited to the named fixture, observables, and checkpoint interval.

## Phase 8 — DSKY research interface and mission visualization

**Deliverables**

- DSKY, verb/noun, program, register, bank, interrupt, scheduler, and guidance
  views;
- synchronized original/Rust stepping with source provenance;
- divergence highlighting and trace export;
- deterministic headless mode so visualization is not required for validation.

**Gate P8**

- UI state is a read-only projection of canonical traces/runtime state;
- visual and exported values agree with raw events;
- disabling the UI does not change semantic trace digests.

## Phase 9 — Fault injection, resilience, and generalization

**Deliverables**

- versioned fault models for timing, interrupts, memory, channels, overload,
  sensors, restart, and arithmetic boundaries;
- nominal/fault paired runs and recovery invariant reports;
- reconstruction defect-seeding study;
- comparison of historical, faithful, and structured implementations;
- reproducible performance and artifact-size benchmarks;
- a second historical-software pilot or explicit generalization analysis.

**Gate P9**

- every result is generated from raw versioned runs;
- impossible/adversarial faults are distinguished from hardware-plausible ones;
- negative and inconclusive results are retained;
- paper claims match measured scope and known divergences.

## Cross-phase release rule

Each research release must include source/tool versions, artifact manifests,
test and trace summaries, known divergences, limitations, and the exact highest
equivalence level reached for each named subsystem.  “Runs,” “compiles,” and
“looks numerically close” are never substitutes for a passed gate.

