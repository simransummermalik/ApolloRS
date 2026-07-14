# ADR-0003: Raw-exact representations and strong semantic types

- Status: Accepted
- Date: 2026-07-13

## Context

Rust primitive integers do not encode AGC one's-complement signed zeros,
end-around carry, register overflow state, banked address spaces, channel
addresses, or fixed-point scale.  Reusing one integer type for these concepts
permits valid Rust code that is invalid AGC behavior.

## Decision

Represent authoritative machine state with raw-exact, invariant-bearing types:

- `AgcWord` for a 15-bit raw word with distinguishable signed zeros;
- distinct accumulator/register representation if overflow extension requires it;
- `AgcDoubleWord` for the documented AGC double-precision convention;
- `AgcAddress`, `FixedAddress`, `ErasableAddress`, `FixedBank`, `ErasableBank`,
  and `ChannelAddress` rather than interchangeable integers;
- `AgcFixed<const FRACTION_BITS: usize>` or an equivalent scale-bearing type;
- checked, explicit conversions to host integers or presentation floats.

Raw/octal values are canonical in state and traces.  Decimal and engineering
units are derived views with scale metadata.

## Alternatives considered

- **Use `i16`/`i32` and normalize around operations.** Rejected because negative
  zero and invalid intermediate states are easily lost.
- **Use bitfields everywhere without domain types.** Raw-exact but rejected as
  insufficient protection against mixing addresses, banks, channels, and scales.
- **Use arbitrary-precision integers or rationals as state.** Useful in an
  independent oracle, but they do not directly represent hardware raw states.
- **Use runtime-only scale tags.** May be needed for dynamically parsed source,
  but compile-time scales are preferred where reconstruction evidence fixes the
  scale.

## Consequences

- APIs are more verbose and conversions require intent.
- Exhaustive tests are practical for many finite raw domains.
- Translation and traces can retain distinctions that modern numeric types hide.
- Some operations require explicit result/status types instead of operator
  overloading.

## Risks introduced

- A type can give false confidence if its operation semantics are wrong.
- Const-generic scales may not cover source values whose scale is recovered only
  at runtime.
- Excessive type fragmentation could make instruction semantics unreadable.

## Validation

- Exhaust all raw conversion, classification, and unary finite domains.
- Add compile-fail tests for cross-address-space and incompatible-scale use.
- Compare arithmetic with an independently written mathematical oracle and
  external instruction traces.
- Review generated APIs for any unchecked primitive escape hatch.

