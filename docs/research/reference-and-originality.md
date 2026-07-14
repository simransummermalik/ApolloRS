# References, licenses, and originality

ApolloRS separates historical input, inspected reference implementations,
external validation tooling, and original project code.

## Historical flight source

- Project: [`chrislgarry/Apollo-11`](https://github.com/chrislgarry/Apollo-11)
- ApolloRS commit: `247dd7d0d1b0e7f9f270750ec08983e0a72e73e1`
- Local location: `historical/Apollo-11`
- License marker: Public Domain Mark 1.0 in the upstream `LICENSE.md`
- Use: immutable historical source, comments, labels, and flight-program input

ApolloRS verifies all 175 `.agc` files against a SHA-256 manifest. Compatibility
changes are represented outside this checkout in `overlays/`.

## VirtualAGC / yaAGC / yaYUL

- Project: [`virtualagc/virtualagc`](https://github.com/virtualagc/virtualagc)
  (the local clone remote is
  `rburkey2005/virtualagc`)
- Inspected and executed commit:
  `0b13e5976dbc3c6c76aeab35195135261d7999ff`
- License in `COPYING`: GNU GPL version 2
- Inspected areas: `yaAGC/agc_engine.c`, register/memory macros, instruction
  transitions, interrupt entry and timing, and yaYUL output/diagnostics
- Use: external behavioral oracle, strict reference assembler, proofed Comanche
  binsource, and semantic cross-check while diagnosing divergences

No VirtualAGC C source is compiled into an ApolloRS crate. No C function was
copied into the Rust workspace. ApolloRS uses its own types, module boundaries,
error model, event schema, and state-transition code. Behavior learned from
manuals and reference execution is re-expressed independently and tested by
trace comparison.

`docs/validation/yaagc-exact-trace.patch` is intentionally separate reference
instrumentation. When applied to VirtualAGC it is governed by VirtualAGC's GPL
terms; it is not part of the dual-licensed Rust crates.

## ragc

- Project: [`felipevb/ragc`](https://github.com/felipevb/ragc)
- Inspected commit: `fed2ba8277f577c8c55ff1ba456417273c385a53`
- License: MIT OR Apache-2.0
- Inspected areas: CPU organization, unprogrammed-sequence representation,
  central-register widths, memory/peripheral decomposition, and test layout
- Use: architectural comparison and a warning source for likely AGC edge cases

ApolloRS does not depend on ragc and does not copy or adapt ragc source. In
particular, ApolloRS's event model, two-MCT interrupt representation, bank-zero
alias handling, source/assembler pipeline, mission evidence, and yaAGC trace
adapter are original to this project.

## ApolloRS licensing

New Rust code is declared `MIT OR Apache-2.0` in the workspace manifest. This
does not relicense the historical checkout, external reference software, or the
GPL reference patch. Generated artifacts retain their input provenance and
should be distributed with the corresponding envelope or sidecar.

## Adaptation statement

The implementation process included reading external source and using exact
execution traces. That is disclosed because it materially informed debugging.
The project does not claim clean-room development. It does claim independent
Rust expression: no external implementation is a linked dependency, and no
substantial source passage was translated or pasted into ApolloRS. Where a
reference behavior was uncertain, ApolloRS narrowed its claim and retained the
first-divergence evidence instead of inheriting a reference result silently.
