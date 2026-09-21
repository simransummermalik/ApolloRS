# P63 multiclass fault matrix

The fault matrix replaces a single illustrative rope mutation with a declared,
versioned set of paired experiments. The declaration is
`experiments/p63-fault-matrix.json`; the measured report is
`artifacts/generated/luminary099-p63-fault-matrix.json`.

Every case starts from an identical pre-execution controller. The 180,000
instruction baseline is run once, semantic anchors are resolved from its trace,
and each faulted arm runs to the same instruction horizon. No result is used to
choose a later injection point within the same matrix.

## Current result

| Outcome class | Cases |
|---|---:|
| masked through the horizon | 2 |
| diverged, then recovered final registers | 2 |
| persistent state divergence without acceptance regression | 3 |
| mission evidence changed | 1 |
| mission acceptance degraded | 3 |
| total | 11 |

Nine cases diverged from the baseline trace. Three reduced mission acceptance,
and two reconverged to the same final architectural register snapshot.

| Case | Injection boundary | First visible effect | Outcome |
|---|---:|---:|---|
| drop reset DOWNRUPT | 0 | +354 instructions | mission timing/evidence changed |
| force first-key channel low | 23,994 | +45 | mission degraded; program 63 not selected |
| flip MODREG before its store | 145,180 | none | masked by the rope write |
| suppress superbank near P63 | 145,516 | immediate | mission degraded; F32 entry absent |
| flip first P63LM rope word | 145,715 | immediate | mission degraded; initialization absent |
| flip A at P63 entry | 145,716 | immediate | recovered by horizon |
| flip DVCNTR after write | 145,849 | none | masked through horizon |
| flip LAND.X.HI after write | 157,564 | +14,946 | persistent latent divergence |
| jump TIME1 by +100 | 145,766 | +327 | recovered by horizon |
| inject three IMU pulses | 145,736 | +613 | persistent state divergence |
| inject radar sample/interrupt | 176,492 | immediate | persistent state divergence |

“Masked” means no trace-visible effect through the declared horizon; it does
not prove that an unobserved memory bit was physically restored. “Recovered”
means the trace diverged and the final A/L/Q/Z/EB/FB/BB snapshot reconverged
without changing the tracked mission-acceptance fields. It does not imply that
every erasable word or future execution is identical.

## Why the injection anchors matter

Cases may name an exact instruction, a trace-backed mission anchor, or the first
write to a named guidance variable. The report records the declaration, the
baseline evidence used to resolve it, and the exact resulting instruction.
This keeps experiments tied to software meaning even if unrelated timing
changes shift raw instruction counts.

The fault engine now supports logical and physical erasable mutations, physical
rope mutations, central-register corruption, bounded stuck channels, interrupt
drops, timer jumps, IMU pulses, and radar samples. Applied faults retain exact
instruction/cycle boundaries and resulting raw words.

## Engineering lessons exposed by the matrix

- A passing final-state check can hide transiently wrong execution. The
  accumulator and timer cases diverged before reconverging, so trace evidence
  and final-state evidence answer different questions.
- Redundant writes can mask corruption. The pre-store MODREG mutation and the
  post-write DVCNTR mutation produced no trace-visible effect by the horizon.
- Fault latency is a first-class metric. The LAND.X perturbation remained
  invisible for 14,946 instructions before affecting execution.
- Control state can dominate data-path resilience. Disturbing channel 7 around
  the bank transfer prevented the physical P63 entry path even though the rope
  bytes were unchanged.
- Rust memory safety does not establish mission resilience. Modern
  implementations still need explicit timing, fault activation, observability,
  recovery horizons, and acceptance criteria.

## Reproduce

```sh
cargo run --release -p apollors-cli -- --repository . fault-matrix \
  --rope artifacts/generated/luminary099-reference.bin \
  --format yayul \
  --spec experiments/p63-fault-matrix.json \
  --output /tmp/luminary099-p63-fault-matrix.json
```

These are deterministic adversarial software-sensitivity experiments. The
matrix does not estimate component reliability, radiation rates, or Apollo
mission risk, and the P63 fixture still lacks a coupled vehicle and sensor
plant.
