# P63 entry vertical slice

The P63 vertical slice is complete for program selection, entry, and initial
landing-guidance activity. It is not a complete-descent slice.

## Inputs

- unchanged Luminary 099 historical source at commit `247dd7d…`;
- reference rope SHA-256 `bf873988…` built with pinned yaYUL;
- 52-word P63-relevant subset of the
  [Luminary 99 Apollo 11 LM-5 pad-load book](https://www.ibiblio.org/apollo/Documents/Luminary99PadLoads.pdf),
  recorded word-by-word in the mission artifact;
- explicit `REFSMFLG` aligned-flight precondition;
- software-paced DSKY sequence `V37E63E`;
- 300,000 committed-instruction budget.

The official pad-load book excludes mission-time computed state vectors. That
restriction is carried into artifact limitations and prevents a complete
trajectory claim.

## Acceptance gates

- every key has a host request, KEYRUPT1 event, and historical `CHARIN` fetch;
- typed Pinball V37 reconstruction completes with program 63;
- rope `MODREG` contains decimal 63;
- the trace reaches `P63LM` at F32:0776;
- typed P63 initialization and original-rope writes agree in source order and
  raw value;
- later trace writes demonstrate landing-equation state activity;
- the complete ApolloRS event stream matches the pinned yaAGC prefix for all
  declared external fields.

All gates pass in the current artifacts.

## Deliberate exclusions

- mission-time LM state vector and navigation history;
- coupled thrust, IMU, radar, and vehicle dynamics;
- `P63SPOT`, `P63SPOT2`, ignition, throttle profile, touchdown, or abort
  checkpoints;
- whole-program high-level Rust equivalence.

A future complete-descent slice must supply a primary-source mission state or a
qualified plant/replay trace, add those checkpoints, and compare raw state
before reporting engineering-unit trajectory error.
