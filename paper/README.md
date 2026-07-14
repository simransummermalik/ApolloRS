# ApolloRS: bounded reconstruction and differential execution of Apollo 11 flight software

## Abstract

ApolloRS is an `unsafe`-free Rust system for preserving, executing, inspecting,
and reconstructing Apollo 11 Guidance Computer software. The system inventories
175 historical source files, loads independently validated Comanche 055 and
Luminary 099 rope images, models the Block II CPU and deterministic peripherals,
and emits provenance-bearing traces and reports. In the flagship experiment,
the real Luminary 099 rope processes `V37E63E`, selects program 63, reaches
`P63LM`, and begins landing-guidance updates. All 300,468 ApolloRS events match
a pinned yaAGC prefix on twelve architectural fields. Two bounded subsystems—a
typed Pinball V37 state machine and typed P63 initialization—also agree with
original-rope behavior. The experiment establishes a reproducible P63-entry
result, not a full powered-descent simulation.

## Method

The historical checkout is fixed at commit `247dd7d…` and verified with per-file
SHA-256 values. Luminary is assembled with pinned yaYUL; Comanche is imported
from a proofed octal listing only after Rust verifies all 36 bank checksums.
ApolloRS executes fetched rope words through a banked memory and Block II CPU
model. Runtime inputs are ordered at exact cycle boundaries, and observers read
rather than replace the historical Executive, Waitlist, Pinball, and guidance
state.

The P63 fixture combines a documented subset of the
[Apollo 11 LM-5 Luminary 99 pad load](https://www.ibiblio.org/apollo/Documents/Luminary99PadLoads.pdf)
with an explicit aligned-platform flag. Seven DSKY key codes are paced
by software acceptance. The trace records interrupts, register state, memory
accesses, channels, banks, cycles, and physical locations. A separate yaAGC
binary at commit `0b13e597…` receives the same fixture and writes an independent
architectural event stream.

## Results

| Measurement | Result |
|---|---:|
| Historical `.agc` files | 175 |
| Historical bytes / lines | 3,150,815 / 130,186 |
| Luminary rope bytes / nonzero words | 73,728 / 36,340 |
| Mission instructions / cycles | 300,000 / 504,958 |
| ApolloRS events | 300,468 |
| yaAGC reference events | 795,178 |
| Exact matched events | 300,468 |
| First divergence | none in ApolloRS stream |
| P63 `MODREG` selection cycle | 240,227 |
| `P63LM` entry cycle | 241,219 |

Every supplied key reached KEYRUPT1 and Pinball `CHARIN`. The typed V37 model
and rope both selected decimal program 63. At `P63LM`, the rope initialized
`WHICH`, `DVTHRUSH`, `DVCNTR`, `WCHPHASE`, and `FLPASS0` to the same raw values
and order as the typed Rust model. Subsequent writes included TPIP, LAND, TTF/8,
VGU, and RGU state.

The exact comparison covers event kind, normalized cycle, PC, instruction,
A/L/Q, EB/FB/BB, and interrupt vector/number. The result is a common-prefix
claim because the reference was intentionally run longer.

## Negative results and limitations

The native assembler does not encode the entire yaYUL/interpretive dialect and
rejects full flight assembly with explicit diagnostics. Whole-program generated
Rust remains mechanical and unverified. The P63 run does not reach P63SPOT or
P63SPOT2. More importantly, the official LM-5 pad-load book excludes
mission-time state vectors, and ApolloRS currently lacks a continuous
lunar-module plant. Saturated guidance values in the later run therefore cannot
be interpreted as a physical descent trajectory.

External implementations were inspected and are disclosed in
`docs/research/reference-and-originality.md`. No VirtualAGC or ragc source is a
Rust dependency or copied implementation. The independent reference patch is
kept under its applicable GPL boundary.

## Conclusion

ApolloRS demonstrates that historically significant AGC software can be run and
studied in idiomatic Rust without confusing emulation, generated dispatch, and
high-level reconstruction. Exact external traces provide the executable ground
truth; bounded typed models provide readability. The strongest current result
is real P63 selection and initialization with 300,468 matched yaAGC events. A
complete landing claim requires a qualified mission state and vehicle/sensor
model and is deliberately left open.
