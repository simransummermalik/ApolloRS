# ApolloRS

ApolloRS is an executable Rust research system for the Apollo 11 Block II
Guidance Computer and its flown software. It preserves the historical
Comanche 055 and Luminary 099 sources byte-for-byte, executes real rope images,
models the DSKY and deterministic hardware inputs, records architectural
traces, and compares those traces with a pinned yaAGC reference.

The flagship result is a real Luminary 099 P63 entry slice. ApolloRS sends
`V37E63E` through KEYRUPT and Pinball, observes program 63 in `MODREG`, reaches
the historical `P63LM` rope location, and records the first landing-guidance
writes. All 300,468 ApolloRS instruction/interrupt events in that run match the
pinned yaAGC trace on event kind, cycle, PC, instruction, A/L/Q, EB/FB/BB, and
interrupt identity.

This is not a claim that ApolloRS has replayed the complete powered descent.
The official LM-5 pad-load document excludes mission-time state vectors, and
the current scenario has no continuous vehicle, IMU, or landing-radar plant.
It proves P63 selection, entry, and initial equation activity under a documented
entry fixture. It does not reach `P63SPOT` or `P63SPOT2` in the bounded run.

## What is implemented

- A 24-crate, `unsafe`-free Rust workspace for one's-complement arithmetic,
  banked memory, Block II instructions, interrupts, timers, channels, runtime
  events, DSKY, faults, traces, validation, reports, source parsing, overlays,
  assembly integration, cross references, and Rust generation.
- Exact source inventory for 175 `.agc` files, 3,150,815 bytes, and 130,186
  physical lines at historical commit
  `247dd7d0d1b0e7f9f270750ec08983e0a72e73e1`.
- Luminary 099 reference rope built by pinned yaYUL:
  `bf87398818b99446e300aa319c3e177e42131277f7e83822e8fa0db8ba3008b1`.
- Comanche 055 rope imported from the independently proofed VirtualAGC
  binsource only after Rust validates all 36 bank checksums:
  `2ba31de9291cd10fb351a64d261bae8514a1cb75b4651bfa6a135dfa821a2d79`.
- Two readable typed reconstructions that are checked against original-rope
  execution: the Pinball `V37E nnE` state machine and the five-word `P63LM`
  landing-guidance initialization.
- Provenance envelopes and sidecars containing source/tool revisions, hashes,
  generation commands, timestamps, and limitations.

## Honest boundaries

- ApolloRS executes the original rope; it is not a complete high-level rewrite
  of every Apollo routine.
- The native parser is corpus-wide and loss-preserving, but the native assembler
  does not yet encode the complete yaYUL directive and interpretive dialect.
  Luminary therefore uses strict external yaYUL integration; Comanche uses the
  checksum-validated reference binsource. The native-assembly gap is quantified
  in `artifacts/generated/*-native-assembly-status.json`.
- Whole-program transpilation currently emits provenance-preserving instruction
  dispatch. It is compile-checked but explicitly marked unverified. Readable
  reconstruction is performed only for bounded, separately tested subsystems.
- Exact yaAGC agreement is bounded to the recorded streams and compared fields.
  A matched common prefix is not a whole-emulator proof.

The current claim matrix is in
[`docs/validation/verification-matrix.md`](docs/validation/verification-matrix.md).

## Build and test

Rust 1.97.0 is pinned in `rust-toolchain.toml`.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

Verify that the historical checkout still matches the committed manifest:

```sh
cargo run -p apollors-cli -- --repository . verify-source \
  --manifest artifacts/generated/source-manifest.json
```

Regenerate source and graph forensics with Rust-native tooling:

```sh
cargo run -p apollors-cli -- --repository . forensics \
  --output artifacts/generated
```

Run the P63 entry scenario:

```sh
cargo run --release -p apollors-cli -- --repository . mission \
  --rope artifacts/generated/luminary099-reference.bin \
  --instructions 300000 \
  --output artifacts/generated/luminary099-p63-run.json \
  --trace /tmp/apollors-p63-trace.jsonl
```

The independent trace procedure and exact VirtualAGC patch are documented in
[`docs/validation/yaagc-reference.md`](docs/validation/yaagc-reference.md).

## Evidence map

- `artifacts/generated/source-manifest.json` — byte-level historical inventory.
- `artifacts/generated/luminary099-reference-build.json` — yaYUL build record.
- `artifacts/generated/comanche055-reference-build.json` — binsource bank checks.
- `artifacts/generated/luminary099-p63-run.json` — P63 entry evidence and frames.
- `artifacts/generated/luminary099-p63-vs-yaagc.json` — exact matched-prefix report.
- `docs/research/reference-and-originality.md` — inspected projects, commits,
  licenses, adaptation boundary, and original ApolloRS work.
- `paper/README.md` — concise manuscript with measured results and limitations.

## Evidence vocabulary

ApolloRS keeps four claims separate:

1. Historical emulation: original AGC words executing on the Rust machine.
2. Mechanical translation: source/IR transformed into Rust without a
   readability or equivalence claim.
3. Idiomatic reconstruction: a bounded routine expressed as normal typed Rust.
4. Behaviorally verified equivalence: a named initial state, input stream,
   oracle, observable set, and finite interval agree.

Only the fourth is called equivalent, and only within its recorded boundary.
