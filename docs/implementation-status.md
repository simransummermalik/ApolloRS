# Implementation status

This file is the concise source of truth for what ApolloRS can currently prove.

## Verified vertical slice

The tracked Luminary 099 rope is executed for 300,000 committed instructions
and 504,958 machine cycles with a P63-relevant subset of the Apollo 11 LM-5
erasable load. The runtime delivers the seven `V37E63E` key codes only after
the previous key has completed KEYRUPT1 and Pinball `CHARIN` processing.

Measured checkpoints:

| Checkpoint | Trace sequence | Instruction | Cycle | Physical/event location |
|---|---:|---:|---:|---|
| `MODREG=63` | 145,405 | 145,181 | 240,227 | F02:1314 |
| `P63LM` | 145,942 | 145,716 | 241,219 | F32:0776 |
| first `WHICH` write | 146,070 | 145,844 | 241,462 | E7:0055 |
| first `DVTHRUSH` write | 146,072 | 145,846 | 241,466 | E2:0251 |
| first `DVCNTR` write | 146,074 | 145,848 | 241,470 | E7:0115 |
| first `WCHPHASE` write | 146,076 | 145,850 | 241,474 | E2:0351 |
| first `FLPASS0` write | 146,078 | 145,852 | 241,478 | E7:0223 |

The five values are respectively octal `02076`, `00044`, `00004`, `77776`
(-1 in one's complement), and positive zero. They match the readable
`P63Initialization::luminary099()` model in source order.

The ApolloRS stream contains 300,468 events because interrupts are explicit
events in addition to committed instructions. Every one matches the pinned
yaAGC stream. The reference stream has 795,178 events, so the report is a
qualified common-prefix result, not a complete-stream result.

## Implemented machine surface

- basic and extended Block II decode across the complete 15-bit word domain;
- A/L/Q width rules, bank registers, fixed/erasable windows, bank-zero aliases,
  edit registers, channels, and special registers;
- instruction timing, scaler/timer effects, downlink timing, interrupt
  arbitration/entry, `RESUME`, and `EDRUPT` behavior required by the matched run;
- deterministic external channels, DSKY keys, IMU pulses, radar samples, and
  fault boundaries;
- trace events containing before/after registers, memory, I/O, interrupts,
  instruction/cycle counts, and physical memory descriptions.

## Source and build surface

Both source trees parse and round-trip without altering their historical bytes.
Include expansion, explicit compatibility overlays, source hashes, typed IR,
and graph artifacts are implemented. Focused native basic-instruction assembly
works and is tested.

Full native flight assembly is not implemented. The capability reports show
unsupported interpretive orders, `EBANK=`/`COUNT*`-family directives, expression
forms, and cascading location collisions. ApolloRS refuses to emit a native
flight rope under those conditions. Reference integration remains part of the
designed build path rather than a hidden fallback.

## Current non-claims

- no complete Apollo 11 mission-state replay;
- no dynamic lunar-module vehicle or sensor plant;
- no `P63SPOT` or `P63SPOT2` checkpoint in the current run;
- no complete high-level Rust rewrite of Luminary or Comanche;
- no whole-emulator formal proof or exhaustive independent instruction matrix;
- no fault-recovery equivalence claim.
