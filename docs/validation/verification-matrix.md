# Verification matrix

Status meanings:

- `verified` — the named bounded claim has current evidence.
- `implemented` — functionality and local tests exist, but the broader claim is
  not independently qualified.
- `unsupported` — ApolloRS rejects or explicitly excludes the claim.

| Claim | Evidence boundary | Status |
|---|---|---|
| Historical source integrity | 175 `.agc` files; exact path, byte count, line count, SHA-256; pinned commit `247dd7d…` | verified |
| One's-complement word representation | all 32,768 raw words round-trip; both zeros retained; complement and integer-domain properties | verified |
| Block II decode domain | every 15-bit word decoded in basic and extracode contexts | verified |
| Bank/register/edit behavior | focused boundary and alias tests, including L overflow correction and channel 7 masks | implemented |
| Luminary 099 rope | clean pinned yaYUL build, 73,728 bytes, zero fatal/unresolved symbols, SHA `bf873988…` | verified |
| Comanche 055 rope import | 36 banks × 1,024 words, every bank checksum accepted, SHA `2ba31de9…` | verified |
| Native full-flight assembly | current diagnostic reports retain unsupported directive/interpretive families and collisions | unsupported |
| Original Luminary execution | real rope loaded and executed through timers, interrupts, channels, Pinball, and P63 entry | verified |
| yaAGC architectural agreement | 300,468-event ApolloRS stream is an exact prefix match on 12 fields; reference has 795,178 events | verified |
| DSKY V37E63E path | all seven keys have request, KEYRUPT1, and `CHARIN` milestones; `MODREG=63` observed | verified |
| Typed Pinball reconstruction | typed `V37ProgramChange` accepts only rope-accepted keys and yields the same program 63 | verified |
| P63 entry | physical `P63LM` fetch at F32:0776 after `MODREG=63` | verified |
| Typed P63 initialization | five source-ordered writes match typed Rust values for `WHICH`, `DVTHRUSH`, `DVCNTR`, `WCHPHASE`, `FLPASS0` | verified |
| Landing-equation activity | trace-backed writes include TPIP, LAND, TTF/8, VGU, and RGU words | verified |
| Complete powered descent / landing | no mission-time state vector or coupled vehicle/sensor plant; P63SPOT/P63SPOT2 not reached | unsupported |
| Deterministic fault injection | audited bit flip at P63LM physical F32:0776; paired baseline/faulted traces first diverge at event 145,942 | verified |
| Restart/recovery equivalence under faults | P63 entry fault does not recover by the 180,000-instruction horizon; no general recovery-equivalence claim | unsupported |
| Whole-program mechanical Rust generation | standalone, compile-checkable source/word dispatch with provenance | implemented |
| Whole-program idiomatic Rust rewrite | no such claim; only bounded typed subsystems are reconstructed | unsupported |
| Artifact provenance | envelope validation plus raw-file sidecars and content hashes | verified |

## Exact P63 acceptance

The mission is accepted only when all of these are true:

1. Every requested DSKY key is accepted through KEYRUPT1 and the historical
   Pinball `CHARIN` routine.
2. The typed V37 state machine and rope `MODREG` both select decimal program 63.
3. The trace reaches `P63LM` at logical address `2776`, physical fixed bank 32,
   offset `0776`.
4. The five P63 initialization writes occur in source order with exact raw
   values and match `P63Initialization::luminary099()`.

The current report also lists later landing-guidance writes, but those are not
promoted into a complete-descent claim.

## Exact-reference boundary

The yaAGC adapter compares event kind, normalized MCT cycle, PC, instruction,
A, L, Q, EB, FB, BB, interrupt vector, and interrupt number. yaAGC records
interrupt acceptance on the first of its two MCTs; ApolloRS records completion,
so reference interrupt cycles are normalized by +1. Memory access lists,
peripheral internals, and every erasable word are not part of this external
comparison. The mission report supplies separate trace-backed evidence for the
crew and guidance milestones.
