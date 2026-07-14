# Verification matrix

Status values: `complete`, `planned`, `blocked`, or `not started`.  “Complete”
means the named evidence exists; it does not confer broader equivalence.

| Layer / claim | Inputs and partitions | Independent oracle or basis | Required observables | Method | Acceptance evidence | Status |
|---|---|---|---|---|---|---|
| Historical source integrity | All 175 `.agc` files | Upstream commit and SHA-256 | Path, bytes, digest, dirty state | Deterministic inventory/check mode | Manifest repeats for pinned clean checkout | complete |
| Textual include inventory | Both `MAIN.agc` files | Literal source tokens and filesystem | Order, source line, exact resolution, unresolved token | Lexical parser plus machine graph | 173 include edges retained; unresolved edges explicit | complete |
| Reproducible assembly | Comanche 055 and Luminary 099, explicit overlays | Pinned yaYUL and documented reference checksums | emitted words, bank layout, symbols, warnings, tool/source hashes | Clean builds repeated in pinned environment | Byte-identical bundles across two clean runs; checksum comparison reported | not started |
| `AgcWord` raw representation | All `2^15` patterns | Mathematical one's-complement definition independent of implementation | raw bits, sign, zero class, checked integer | Exhaustive round-trip/table tests | No mismatch; both zeros preserved | not started |
| Unary word operations | All raw patterns | Independent bit-level oracle | complement, sign, magnitude, octal view | Exhaustive tests | No mismatch for every pattern | not started |
| Addition/subtraction | Zero classes, extrema, carry/overflow partitions, randomized pairs, scheduled pair sweep | Independent reference function and external vectors | raw result, carry, overflow state | Examples, properties, partitioned exhaustive/nightly sweep | All mandatory partitions and declared sweep shards pass | not started |
| Double-precision arithmetic | Sign/zero/word-boundary/rounding partitions | Independent DP specification and reference traces | high/low raw words, sign, overflow, scale | Properties, focused exhaustive domains, differential vectors | No mismatch in declared domain | not started |
| Fixed-point scale safety | Every supported scale pair and forbidden combination | Type rules plus hand-derived vectors | raw words, scale metadata, checked conversion | Compile-fail tests and numerical properties | Invalid scale mixing fails to compile; vectors agree | not started |
| Logical/physical memory map | Every address-region boundary and all bank identifiers | Memory-map specification and assembler symbols | logical address, bank registers, physical target | Exhaustive mapping by finite address/bank domain where feasible | All valid mappings agree; invalid states rejected | not started |
| Central/edit registers | Per-register read/write values including zeros and extrema | Architecture semantics and external one-step traces | stored value, read value, side effects | Table-driven unit and differential tests | Every special register has edge fixtures | not started |
| Instruction decoding | Every instruction word under basic/extracode context | Cited decode table and external disassembly | raw word, context, decoded op/address | Exhaustive decode over finite word/context domain | No unsupported or ambiguous decoded state in declared ISA | not started |
| Instruction state transition | Every instruction, representative addressing modes, numerical edge states | Qualified external emulator plus semantic vectors | PC, A/L/Q, banks, writes, channels, cycles | One-step differential tests | Exact agreement for every declared state partition | not started |
| Cycle accounting | Every instruction and unprogrammed sequence; boundary arrivals | Architecture timing references and external traces | cycle delta, phase, accepted event | Unit and timing-boundary differential tests | Exact cycle agreement in declared timing model | not started |
| Interrupt arbitration | Each source alone, pairwise/simultaneous sources, inhibit/enable windows | Explicit priority spec and external traces | pending set, accepted source, save state, handler PC, cycle | Scenario matrix and deterministic replay | Correct order/state for every matrix row | not started |
| Interrupt return | Basic/extracode interruption points, bank states, deferred events | External traces and formal save/restore invariants | save registers, banks, resumed word/context | Focused differential tests | Exact pre-entry restoration except specified effects | not started |
| Channel/I/O semantics | Every implemented channel operation and relevant bit partition | Peripheral contract and external traces | operation, channel, prior/result bits, cycle | Unit, property, and scenario tests | No undeclared bit changes; event order agrees | not started |
| Determinism | Repeated runs of every golden fixture | Prior run's canonical digest | complete canonical event stream | Run at least three times with controlled environment | Identical digest and terminal state | not started |
| Trace serializer/comparator | Constructed adversarial event streams | Hand-authored expected alignments | signed zeros, banks, cycles, event order, first mismatch | Schema round-trip, mutation, and golden tests | No semantic distinction lost; defect class/location correct | not started |
| Original code load/step | Minimal assembled snippets, then full rope images | Assembler listing/symbol artifacts | loaded words, initial state, PC and step trace | Loader tests and checkpoint execution | Loaded image digest matches bundle; trace maps to source | not started |
| Executive | Priority/core-set/sleep/wake/idle/overload scenarios | Original Executive execution in qualified reference | job/core-set state, priority, dispatch, cycles | Differential scenario traces and invariants | Exact event/state agreement through named checkpoints | not started |
| Waitlist | Equal deadlines, wrap, signed-zero delta, long calls, timer interrupt | Original Waitlist execution | queue/table state, time words, dispatch order, cycles | Differential scenario traces | Exact state and dispatch order for all declared cases | not started |
| Interpretive decode | Every supported packed order form and addressing mode | Assembler/listing and external interpreter traces | raw order pair, address word, mode, next location | Exhaustive finite decode plus focused blocks | Decode and boundary events agree | not started |
| Interpretive execution | Scalar/vector/DP/TP modes, MPAC stack, overflow and branch partitions | Original interpretive routines in reference execution | mode, MPAC raw words, location, banks, basic/interpreter transitions | Differential blocks and properties | Exact raw-state trace through named blocks | not started |
| Restart/recovery | Fresh start, software/hardware restart, phase groups, interrupted jobs/tasks | Original code in qualified runtime | preserved/cleared words, phase tables, alarms, resumed jobs/tasks | Deterministic fault scenarios | All declared invariants and event traces agree or divergence is reported | not started |
| Typed AST / assembler semantics | Entire pinned source corpus | Pinned assembler listing/symbol/emitted words | tokens, symbols, banks, pseudo-ops, diagnostics, provenance | Differential assembly/front-end tests | No silent guesses; supported corpus emits identical words/symbols | not started |
| Bank-aware IR | Selected modules then full supported corpus | AST, assembler graph, manual audits | labels, addresses, banks, data/control edges, comments, source lines | Schema checks, graph coverage, audits | Every node/edge has provenance; unresolved edges retained | not started |
| Faithful Rust generation | Focused routines then flagship slice | Original assembled execution | complete declared machine/event trace | Differential execution | Exact trace agreement through named checkpoint | not started |
| Structured Rust generation | Individually justified transformations | Faithful Rust and original execution | same declared events plus outputs | Transformation tests and three-way differential traces | No mismatch through declared boundary | not started |
| Flagship P63 slice | Versioned initial state and complete input stream | Qualified original execution plus faithful mode | instructions, registers, banks, writes, channels, interrupts, scheduler/interpreter events, outputs | Three-way differential run and audit | All criteria in vertical-slice DoD pass | not started |
| Mission numerical analysis | Flagship checkpoints and selected guidance variables | Raw fixed-point reference states | raw words, scale, engineering-unit rendering, error metric | Post-process matched traces only | Raw agreement reported before derived error; method reproducible | not started |
| Fault injection | Versioned plausible/adversarial fault catalog | Nominal paired runs and declared recovery invariants | detection, alarm, invariant failure, recovery, divergence | Deterministic paired scenario matrix | Every run classified; no missing/hidden divergence | not started |
| Regression preservation | Every confirmed semantic defect | Minimal failing fixture from original divergence | first mismatch and corrected terminal state | Mandatory CI suite | Original failure reproduced before fix and passes after fix | not started |
| Performance | Tracing-off and declared tracing modes on pinned host/toolchain | Repeated benchmark protocol | simulated cycles/s, wall time, memory, trace bytes | Warmed repeated benchmark with raw samples | Statistics and environment published; no correctness inference | not started |
| Reproducible research release | Clean checkout and declared evidence target | Artifact manifest | source/tool hashes, commands, tests, traces, reports | Independent clean-room rebuild | Manifest and declared deterministic outputs match | not started |

## Matrix rules

1. An oracle cannot call the implementation being tested to compute expected
   values.
2. “No crash,” compilation, and plausible decimal output are never acceptance
   evidence for semantic rows.
3. Any mismatch blocks only the affected claim, is preserved in the divergence
   catalog, and becomes a regression case after diagnosis.
4. If the external reference cannot expose a required observable, ApolloRS must
   add another evidence source or narrow the claim explicitly.
5. Golden artifacts include source, tool, fixture, schema, and configuration
   digests.  An unexplained digest change invalidates the golden result.

