# Block II semantic conformance against yaAGC

The synthetic conformance rope complements the historical Luminary mission
run. Luminary P63 supplies realistic integration depth; the conformance rope
deliberately reaches instruction and boundary classes that one mission path
does not.

## Current measured result

| Measure | Result |
|---|---:|
| ApolloRS instructions | 250 |
| ApolloRS events | 252 |
| ApolloRS machine cycles | 461 |
| Canonical mnemonics observed | 38 / 38 |
| Basic/extracode forms observed | 39 / 39 |
| Specification-derived final-state assertions | 46 / 46 |
| ApolloRS events matched by pinned yaAGC | 252 / 252 |
| yaAGC events available | 337 |
| First divergence | none |

`INDEX` is required in both basic and extended context, which is why the form
count is one larger than the mnemonic count. `EDRUPT` is represented by its
vector-zero interrupt transition rather than an ordinary instruction event.

## Cases

| Case | Deliberate boundary surface |
|---|---|
| interrupt control | reset DOWNRUPT, `RESUME`, protected `RELINT`/`INHINT` |
| one's-complement arithmetic | both zeros, both overflow directions, `AUG`, `DIM`, `SU`, `MSU`, end-around carry |
| branch boundaries | four `CCS` classes; taken and untaken `BZF`/`BZMF` |
| exchange and index | A/L/Q exchanges, double exchange, basic and extended `INDEX` |
| double precision | `DCA`, `DCS`, `DAS`, negative `MP`, quotient/remainder `DV` |
| channel logic | `READ`, `WRITE`, `RAND`, `WAND`, `ROR`, `WOR`, `RXOR` |
| bank selection | switched erasable isolation, EB/FB/BB synchronization, FBANK 4, superbank 40 |
| software interrupt | unmaskable `EDRUPT`, vector zero, instruction fetch from A |

The generated rope is self-bounded by separate success and failure PCs. The
local runner stops only at one of those breakpoints, verifies every required
mnemonic/context form, and samples exact erasable, channel, and register
obligations. The yaAGC comparison then checks the architectural stream rather
than trusting those local assertions as an independent oracle.

## Reproduce

Build the pinned external executable in a new temporary directory:

```sh
sh tools/build-yaagc-conformance.sh /tmp/apollors-virtualagc
```

Run both implementations over the exact generated rope bytes:

```sh
cargo run --release -p apollors-cli -- --repository . conformance \
  --output-dir /tmp/apollors-conformance \
  --yaagc /tmp/apollors-virtualagc/yaAGC/yaAGC
```

The build script checks out VirtualAGC commit
`0b13e5976dbc3c6c76aeab35195135261d7999ff`, applies
`yaagc-conformance-trace.patch`, and builds only the external `yaAGC` target.
The reference checkout is not linked into any Rust crate.

The instrumentation path is selected through `APOLLORS_YAAGC_TRACE`. yaAGC's
loop uses an internal value one past its hardware interrupt range after an
`EDRUPT` vector-zero fallback; the patch exports that software-interrupt
sentinel as number zero. Hardware interrupt numbers and vectors are unchanged.

## Exact comparison boundary

The comparator checks event kind, normalized cycle, PC, raw instruction,
A/L/Q, EB/FB/BB, and interrupt number/vector. yaAGC runs slightly beyond the
ApolloRS success breakpoint; qualification requires every ApolloRS event to
match in order. The longer yaAGC suffix is explicitly reported and is not
treated as compared.

Memory-access lists and complete peripheral internals are outside the external
TSV. The 46 local assertions make final erasable/channel obligations visible,
but they remain specification-derived tests rather than a second
implementation.
