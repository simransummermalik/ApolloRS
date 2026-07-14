# Semantic risk register

Status: initial Phase 0 register.  Scores are planning judgments and must be
revisited after assembler-backed forensics and the first differential traces.

## Scale

- **Severity (S):** 5 corrupts broad execution or invalidates equivalence; 1 has
  localized presentation impact.
- **Likelihood (L):** 5 is expected without a dedicated control; 1 is unlikely.
- **Testability (T):** 5 is readily isolated/exhausted; 1 requires scarce
  mission context or weak observability.
- **Priority:** P0 blocks trusted architecture work; P1 blocks the flagship;
  P2 must be controlled before broader claims.

Testability is not subtracted from importance.  A high-severity risk with low
testability receives stronger instrumentation and narrower claims.

## Ranked risks

| ID | Semantic risk and likely failure | S | L | T | Priority | Required control/evidence |
|---|---|---:|---:|---:|---|---|
| SR-001 | Modeling 15-bit one's-complement values as host signed integers loses representation semantics. | 5 | 5 | 5 | P0 | Raw-bit canonical type; exhaustive conversion/complement tests; independent arithmetic oracle. |
| SR-002 | Collapsing positive and negative zero changes branches, signs, stores, and scheduler/time comparisons. | 5 | 5 | 5 | P0 | Distinct zero states in APIs/traces; zero-focused tests for every arithmetic and branch operation. |
| SR-003 | Missing or repeated end-around carry yields incorrect sums near carry boundaries. | 5 | 4 | 5 | P0 | Bit-level addition specification; boundary partitions; partitioned exhaustive pair sweep. |
| SR-004 | Treating the accumulator like an ordinary 15-bit word loses overflow-extension and overflow-correction behavior. | 5 | 4 | 4 | P0 | Separate register representation where required; instruction traces for overflow and correction cases. |
| SR-005 | Double-precision sign, signed-zero, word order, or carry propagation is reconstructed incorrectly. | 5 | 4 | 4 | P0 | `AgcDoubleWord` invariants; edge vectors; cross-check interpretive and basic DP routines. |
| SR-006 | Multiply/divide special cases, quotient limits, or signed-zero results differ from hardware semantics. | 5 | 4 | 4 | P0 | Per-instruction formal transition; architecture-derived vectors; external one-step differential tests. |
| SR-007 | Fixed-point binary scale is omitted, guessed, or mixed across variables. | 5 | 5 | 3 | P0 | Scale-bearing types/IR annotations; source provenance; dimensional review; known-vector tests. |
| SR-008 | Host floating point is used inside authoritative state and changes rounding or exceptional behavior. | 5 | 4 | 4 | P0 | Ban host floats from semantic state; explicit conversion only at visualization boundaries; lint/review. |
| SR-009 | Basic/extracode decode, `EXTEND` lifetime, or instruction aliases are misinterpreted. | 5 | 4 | 5 | P0 | Table-driven decoder from cited semantics; exhaustive instruction-word decode; one-step traces. |
| SR-010 | Indexing, relative labels, or instruction-modification behavior is normalized into ordinary addressing. | 5 | 4 | 3 | P0 | Preserve raw effective instruction/address; tests with indexed instructions and modified words; trace both forms. |
| SR-011 | Fixed bank selection through FBANK/BBANK/SBANK resolves the wrong physical rope word. | 5 | 5 | 4 | P0 | Typed logical/physical addresses; bank-transition tests at boundaries; resolved address in every memory trace. |
| SR-012 | Erasable bank selection, superbank state, or EBANK restoration is wrong. | 5 | 4 | 4 | P0 | Explicit bank registers and save/restore state; cross-bank read/write and interrupt-entry tests. |
| SR-013 | Fixed-fixed and unswitched address ranges are incorrectly mapped or treated as banked. | 5 | 3 | 5 | P0 | Exhaustive logical-to-physical map tests; assembler symbol/address comparison. |
| SR-014 | Central, special, or edit registers behave like ordinary erasable memory. | 5 | 4 | 4 | P0 | Register-specific access semantics; read/write/edit test vectors; no raw slice bypass. |
| SR-015 | Aliased addresses or register/memory views diverge after one access path writes state. | 5 | 3 | 4 | P0 | Single authoritative storage mapping; alias-coherence properties; write provenance in traces. |
| SR-016 | Self-modifying or state-dependent instruction behavior is erased by early decoding/caching. | 5 | 3 | 3 | P0 | Decode from current fetched word; invalidate assumptions on writes; regression fixtures from source occurrences. |
| SR-017 | Pseudo-ops, constants, erasable allocation, or yaYUL dialect are parsed differently from the reference assembler. | 5 | 5 | 3 | P0 | Pinned assembler; listing/symbol differential; parser rejects ambiguity; compatibility overlay manifest. |
| SR-018 | Source transcription or include-name discrepancies are silently “fixed,” breaking historical provenance. | 4 | 4 | 5 | P0 | Immutable source hashes; unresolved-token artifacts; explicit alias overlay with before/after assembly hashes. |
| SR-019 | Instruction cycle counts or timing phases are simplified, changing interrupt boundaries and I/O timing. | 5 | 5 | 2 | P0 | Cited cycle model; cycle in every trace event; boundary scenarios with interrupts arriving within instruction windows. |
| SR-020 | Counter increments and other unprogrammed sequences are treated as normal instructions or omitted. | 5 | 4 | 2 | P0 | Explicit runtime events and priority rules; targeted counter/overflow traces against external execution. |
| SR-021 | Interrupt priority, request latching, masking, inhibit windows, or simultaneous arrival order is wrong. | 5 | 5 | 3 | P0 | Deterministic arbitration specification; pairwise/simultaneous request matrix; reference traces. |
| SR-022 | Interrupt save registers, bank state, `RESUME`, or return to an interrupted extracode is wrong. | 5 | 4 | 3 | P0 | Entry/return state snapshots; nested/deferred interrupt tests; source-specific handler traces. |
| SR-023 | I/O channel reads/writes use ordinary memory semantics or overwrite bits that hardware preserves. | 5 | 5 | 3 | P0 | Typed channel addresses; operation-specific read/modify/write semantics; channel event traces. |
| SR-024 | Peripheral timing or discrete polarity is guessed, producing plausible but incorrect mission inputs. | 5 | 4 | 2 | P1 | Versioned peripheral contracts; cited polarity/timing; raw input event fixtures; explicit unknown states. |
| SR-025 | Executive priority, core-set allocation, job sleep/wake, or idle behavior is reconstructed as a conventional thread scheduler. | 5 | 5 | 2 | P1 | Execute original Executive first; trace jobs/core sets/priority; scheduler invariants and overload cases. |
| SR-026 | Waitlist ordering, timer wrap, signed-zero delta time, or `LONGCALL` handling changes task dispatch. | 5 | 4 | 3 | P1 | Original Waitlist execution; exact time-word semantics; equal-deadline/wrap/negative-zero fixtures. |
| SR-027 | Interpreter order boundaries, packed opcode pairs, implicit address words, or mode transitions are decoded incorrectly. | 5 | 5 | 2 | P1 | Separate interpreter trace layer; table/listing cross-check; focused blocks before mission execution. |
| SR-028 | MPAC layout, pushdown behavior, vector/scalar modes, or interpretive overflow differs from the original. | 5 | 5 | 2 | P1 | Typed MPAC views; mode-transition invariants; source test blocks and external traces. |
| SR-029 | Basic/interpreter transitions hide implicit A, Q, bank, or location state. | 5 | 4 | 3 | P1 | Boundary snapshots on `INTPRET`/exit; no implicit host stack state; cross-layer trace assertions. |
| SR-030 | Alarm and abort paths are merged or treated as logging, changing control flow and restart state. | 5 | 4 | 3 | P1 | Distinct alarm events and state transitions; source-derived alarm scenarios; recovery checkpoints. |
| SR-031 | Phase tables, restart groups, preserved erasable state, or fresh-start/restart distinction is mistranslated. | 5 | 5 | 1 | P1 | Execute original restart code; trace phase tables and preserved words; fault injection at named checkpoints. |
| SR-032 | Mission initial state is incomplete or internally inconsistent, making trace agreement meaningless. | 5 | 5 | 2 | P1 | Capture fixture from a qualified run; schema validation and invariants; include full provenance and inputs. |
| SR-033 | Sensor/DSKY/uplink input timestamps are applied at host-event boundaries rather than AGC cycle boundaries. | 5 | 4 | 3 | P1 | Cycle-stamped input log; deterministic replay; tests one cycle before/at/after acceptance boundaries. |
| SR-034 | Final-output-only validation hides compensating divergences in registers, banks, memory, or timing. | 5 | 5 | 5 | P0 | Mandatory first-divergence trace comparison; terminal state is only one observable group. |
| SR-035 | Both compared implementations share code or expected-value logic, creating correlated false agreement. | 5 | 4 | 2 | P0 | Independent oracle adapters and test oracles; architecture vectors; mutation/defect-seeding experiments. |
| SR-036 | External emulator behavior is accepted as truth despite version, configuration, or model defects. | 5 | 3 | 2 | P0 | Pin/configure oracle; qualification suite; triangulate disputed behavior; catalog oracle limitations. |
| SR-037 | Trace normalization erases meaningful distinctions such as signed zero, physical bank, event order, or cycle phase. | 5 | 4 | 4 | P0 | Canonical raw trace schema; normalization audit; adversarial comparator tests. |
| SR-038 | Trace collection or UI callbacks mutate execution order/state or introduce nondeterminism. | 4 | 3 | 4 | P1 | One-way observer interface; repeat-run digest equality; no wall-clock input in semantic core. |
| SR-039 | Structured control-flow recovery invents functions/loops across computed transfers or restart entry points. | 5 | 4 | 2 | P1 | Bank-aware CFG with unresolved edges; faithful mode baseline; manual evidence for every restructuring. |
| SR-040 | Generated Rust drops labels, comments, source lines, scale notes, or banking provenance. | 4 | 4 | 5 | P1 | Required provenance metadata; round-trip/source-map tests; generated-code audit. |
| SR-041 | Guidance outputs appear numerically close while raw fixed-point state or event timing diverges. | 5 | 4 | 4 | P1 | Require bit-level internal checkpoints before numerical-error summaries; report both raw and engineering units. |
| SR-042 | Optimizations reorder reads/writes or collapse state before semantics are established. | 5 | 3 | 4 | P1 | No optimization phase before faithful validation; IR transformation proofs/tests and trace preservation. |
| SR-043 | Fault injection creates impossible hardware states and produces misleading resilience conclusions. | 4 | 4 | 2 | P2 | Fault-model taxonomy; distinguish physical, interface, and adversarial faults; cite assumptions. |
| SR-044 | Artifact/tool drift prevents reproduction or changes a previous equivalence result. | 5 | 4 | 4 | P0 | Lockfiles, tool/source hashes, trace schemas, clean rebuilds, and evidence-bundle manifests. |

## Cross-cutting controls

1. No unchecked conversion may discard raw AGC representation, address space, or
   fixed-point scale.
2. Every runtime transition has one authoritative implementation; the test
   oracle must not call it to compute expected state.
3. Every trace stores raw values first and derived display forms second.
4. Every unsupported or ambiguous construct is an explicit error or unresolved
   record.
5. Every equivalence report includes its tested boundary and known exclusions.
6. Every discovered semantic divergence becomes a minimized regression fixture
   after root-cause classification.

## Review cadence

- Review P0 risks before a crate crosses its implementation gate.
- Review P1 risks before the flagship trace baseline is frozen.
- Re-score the register after the first successful original-program execution,
  first interpreter block, first interrupt-bearing trace, and first fault run.
- Preserve score history in version control; do not delete retired risks.  Mark
  them controlled and link the evidence that justifies the change.

