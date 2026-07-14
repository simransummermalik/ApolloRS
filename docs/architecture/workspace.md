# ApolloRS architecture

This is the implemented architecture, not a proposed scaffold. The workspace
contains 24 Rust crates and forbids unsafe code throughout.

## Execution path

```text
historical .agc sources ──► loss-preserving parser / overlays / typed IR
          │                                      │
          │                                      ├──► graphs and Rust generation
          │                                      └──► native capability diagnostics
          ▼
pinned yaYUL or validated binsource ──► rope loader ──► banked AGC memory
                                                         │
                                                         ▼
                                                   Block II CPU
                                                         │
                   DSKY / IMU / radar / faults ──► deterministic runtime
                                                         │
                                                         ▼
                                    trace / mission evidence / yaAGC comparator
```

Reference assembly is an explicit boundary. `agc-assembler` can assemble
focused basic-instruction units, but it does not pretend to support the full
flight-source dialect. Unsupported interpretive orders and directives become
diagnostics. Full Luminary execution uses a clean pinned yaYUL build; Comanche
uses a Rust-parsed, 36-bank checksum-validated VirtualAGC binsource because the
tracked transcription is not cleanly reassembled by the current reference
toolchain.

## Crate responsibilities

| Layer | Crates | Responsibility |
|---|---|---|
| Exact values | `agc-word`, `agc-fixed` | 15-bit one's-complement words, signed zeros, double words, scaled integer values |
| Machine | `agc-isa`, `agc-memory`, `agc-cpu` | complete decode domain, bank/register/channel semantics, instruction/interrupt transitions and MCT timing |
| Runtime | `agc-runtime`, `agc-faults`, `agc-dsky`, `agc-mission` | deterministic events, physical-input models, DSKY relays/keys, fault audit, synchronized mission evidence |
| Historical source | `agc-source`, `agc-ast`, `agc-parser`, `agc-overlay`, `agc-ir`, `agc-symbols` | immutable source access, exact syntax, explicit compatibility edits, typed records and definitions |
| Build/recovery | `agc-assembler`, `agc-loader`, `agc-xref`, `agc-transpiler` | native focused assembly, strict external assembly, rope validation, graphs, provenance-preserving Rust generation |
| Research evidence | `agc-trace`, `agc-validation`, `agc-reports`, `apollors-cli` | canonical events, divergence classification, yaAGC adapter, provenance envelopes and operator commands |
| Bounded models | `agc-interpreter` plus typed models in `agc-dsky` and `agc-mission` | exact integer experimentation and readable subsystem reconstructions; never hidden replacement of rope execution |

## Semantic ownership

- `agc-word` owns signed zero and end-around-carry behavior.
- `agc-memory` owns central-register widths, editing registers, EB/FB/BB,
  fixed-fixed regions, bank-zero aliases, and channel state.
- `agc-cpu` fetches the current word every step, performs instruction state
  transitions, advances timers/scalers, and emits one canonical trace event.
- `agc-runtime` applies external events only at deterministic cycle boundaries.
- UI, observers, reports, and typed reconstructions consume execution state;
  they do not schedule Apollo software work or alter CPU semantics.

## Validation structure

ApolloRS uses three distinct comparators:

1. Unit/property checks for finite arithmetic, decode, and mapping domains.
2. ApolloRS trace comparison for deterministic replay and regression tests.
3. A separately compiled yaAGC stream for independent architectural events.

The P63 mission additionally derives acceptance evidence from the trace:
KEYRUPT entry, `CHARIN`, `MODREG=63`, the physical `P63LM` fetch, and exact
erasable writes. Typed Pinball and P63 initialization models are fed only facts
accepted or produced by the original rope and are compared with rope state.

## Artifact policy

JSON research outputs use the `agc-reports::Envelope` schema. Raw JSONL traces,
rope binaries, DOT files, and generated Rust receive adjacent provenance
sidecars. Every record includes historical and reference revisions, SHA-256
inputs, the generation command, time, and known limitations. Large traces stay
outside Git; compact reports retain their hashes.
