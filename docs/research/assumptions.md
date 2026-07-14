# Assumptions requiring confirmation

These are not blockers for documentation work.  Each has an interim default so
forensics can continue without silently fixing a consequential project choice.

| ID | Assumption/question | Interim default | Impact if changed |
|---|---|---|---|
| A-001 | Is `/Users/summermalik/Desktop/Apollo11` the ApolloRS project root? | Yes. | Relocate generated documents/artifacts and update paths. |
| A-002 | Is upstream commit `247dd7d0d1b0e7f9f270750ec08983e0a72e73e1` the initial historical baseline? | Yes; never auto-update. | Re-run hash, source, assembly, and trace baseline review. |
| A-003 | Must original `.agc` files remain byte-for-byte untouched? | Yes, including apparent spelling defects. | A different preservation model would supersede ADR-0001. |
| A-004 | Should both Comanche 055 and Luminary 099 remain in project scope? | Yes; Luminary receives the first execution slice. | Inventory and long-term architecture scope would narrow. |
| A-005 | Is the initial flagship the Luminary P63 path described in `VS-LM-P63-001`? | Yes, with boundary frozen after the first qualified trace. | Create a new vertical-slice definition and revise coverage priorities. |
| A-006 | May compatibility aliases resolve the two Luminary include-name discrepancies outside the historical tree? | Yes, only through a versioned overlay that records exact source tokens. | Reproducible assembly may remain blocked or require another source revision. |
| A-007 | Is a pinned Virtual AGC/yaAGC + yaYUL toolchain acceptable as the initial external reference? | Candidate only; qualify before use in a claim. | Build a different oracle adapter and qualification suite. |
| A-008 | Which AGC architecture manuals/listings/scans are authoritative and locally redistributable? | Record citations/links only until rights and archival policy are confirmed. | Citation set, evidence hierarchy, and artifact packaging change. |
| A-009 | Which mission-state source should seed the P63 fixture? | Prefer a captured, reproducible qualified run; otherwise use an explicitly synthetic focused fixture. | Determines historical relevance and the claims allowed for the demo. |
| A-010 | How cycle-accurate must the first runtime be? | Instruction cycles, unprogrammed sequences, interrupt acceptance boundaries, and channel-event timing required; analog hardware detail is deferred. | Runtime architecture and flagship observables may expand or contract. |
| A-011 | Which Rust release/MSRV and host platforms are required? | Pin a current stable toolchain when Phase 1 starts; support a deterministic headless Unix-like build first. | Workspace metadata, CI matrix, and dependency choices change. |
| A-012 | Is WebAssembly/browser execution required for the first demo? | No; preserve a portable semantic core and defer UI technology. | DSKY adapter/API and dependency constraints may need earlier design. |
| A-013 | Which license should cover new ApolloRS code and documentation? | Undecided; do not add a project license by inference. | Distribution, dependency policy, and contribution terms remain incomplete. |
| A-014 | May large generated rope images and traces be committed? | Commit small metadata/goldens; use content-addressed external storage for large artifacts once policy is chosen. | Repository size and release process change. |
| A-015 | Is `unsafe` ever acceptable for performance? | No; `#![forbid(unsafe_code)]` until a specific accepted ADR proves necessity. | A later exception needs safety and semantic-equivalence evidence. |
| A-016 | Should benchmarks target correctness-first debugability or maximum emulation speed? | Correctness and semantic clarity first; measure optimized performance only after validation. | Crate API and optimization schedule could change. |
| A-017 | Who performs the independent source/trace audit required for paper release? | Role not yet assigned. | Release gate remains incomplete until a reviewer and protocol are named. |
| A-018 | Are the filename-based subsystem classifications acceptable as provisional metadata? | Yes, explicitly marked candidate/lexical. | Regenerate inventory taxonomy without changing source facts. |

Confirmed answers should become ADRs, project policy, or updated slice contracts.
Do not delete the original assumption; mark its resolution and link the decision.

