# Reproducing the exact yaAGC comparison

The independent reference is VirtualAGC commit
`0b13e5976dbc3c6c76aeab35195135261d7999ff`. The patch beside this document
adds a twelve-column TSV logger and the same bounded P63 fixture used by
ApolloRS. The patch modifies GPL-licensed VirtualAGC code and must be handled
under VirtualAGC's license.

## Build the reference

```sh
git clone https://github.com/virtualagc/virtualagc.git /tmp/apollors-virtualagc
git -C /tmp/apollors-virtualagc checkout 0b13e5976dbc3c6c76aeab35195135261d7999ff
git -C /tmp/apollors-virtualagc apply \
  "$PWD/docs/validation/yaagc-exact-trace.patch"
make -C /tmp/apollors-virtualagc/yaAGC cc=cc yaAGC
```

Create a debugger command file containing:

```text
step 3032400
info registers
quit
```

Run the reference from its `yaAGC` directory with the tracked Luminary rope:

```sh
cd /tmp/apollors-virtualagc/yaAGC
./yaAGC --no-resume \
  --command=/tmp/apollors-yaagc-command.txt \
  /path/to/ApolloRS/artifacts/generated/luminary099-reference.bin
```

The instrumentation writes `/tmp/yaagc-exact-trace.tsv`. Hostname/network
warnings from optional yaAGC peripheral discovery do not affect this local run.

## Produce the ApolloRS stream

```sh
cargo run --release -p apollors-cli -- --repository . mission \
  --rope artifacts/generated/luminary099-reference.bin \
  --instructions 300000 \
  --output artifacts/generated/luminary099-p63-run.json \
  --trace /tmp/apollors-p63-trace.jsonl
```

## Compare

```sh
cargo run --release -p apollors-cli -- --repository . validate-reference \
  --apollors /tmp/apollors-p63-trace.jsonl \
  --reference /tmp/yaagc-exact-trace.tsv \
  --allow-prefix \
  --output artifacts/generated/luminary099-p63-vs-yaagc.json
```

The current report matches 300,468 ApolloRS events against the first 300,468
events of a 795,178-event yaAGC stream. `--allow-prefix` changes only stream
completion policy; any field mismatch still fails at the first divergent event.

## TSV columns and timing

Columns are cycle, event kind (`I` or `R`), PC, instruction, A, L, Q, EB, FB,
BB, interrupt vector, and interrupt number. yaAGC logs interrupt acceptance on
the first of a two-MCT sequence; ApolloRS logs the completed entry event. The
Rust comparator therefore normalizes a yaAGC interrupt cycle by +1 and records
that rule in the output report.

The comparison does not inspect every memory cell or peripheral. The mission
artifact separately proves KEYRUPT/CHARIN, Pinball selection, P63 entry, typed
P63 initialization, and later landing-equation writes from ApolloRS's complete
trace schema.
