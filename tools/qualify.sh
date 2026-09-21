#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

if [ -n "${APOLLORS_QUALIFY_OUTPUT:-}" ]; then
    OUTPUT=$APOLLORS_QUALIFY_OUTPUT
    if [ -e "$OUTPUT" ]; then
        echo "refusing to overwrite qualification output: $OUTPUT" >&2
        exit 1
    fi
    mkdir -p "$OUTPUT"
else
    OUTPUT=$(mktemp -d "${TMPDIR:-/tmp}/apollors-qualification.XXXXXX")
fi

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build --workspace --release

cargo run --quiet --release -p apollors-cli -- --repository . verify-source \
    --manifest artifacts/generated/source-manifest.json

if [ -n "${APOLLORS_YAAGC:-}" ]; then
    cargo run --quiet --release -p apollors-cli -- --repository . conformance \
        --output-dir "$OUTPUT/conformance" \
        --yaagc "$APOLLORS_YAAGC"
else
    cargo run --quiet --release -p apollors-cli -- --repository . conformance \
        --output-dir "$OUTPUT/conformance"
fi

cargo run --quiet --release -p apollors-cli -- --repository . mission \
    --rope artifacts/generated/luminary099-reference.bin \
    --format yayul \
    --instructions 300000 \
    --output "$OUTPUT/luminary099-p63-run.json" \
    --trace "$OUTPUT/luminary099-p63-trace.jsonl"

cargo run --quiet --release -p apollors-cli -- --repository . coverage \
    --trace "$OUTPUT/luminary099-p63-trace.jsonl" \
    --output "$OUTPUT/luminary099-p63-coverage.json"

cargo run --quiet --release -p apollors-cli -- --repository . fault-matrix \
    --rope artifacts/generated/luminary099-reference.bin \
    --format yayul \
    --spec experiments/p63-fault-matrix.json \
    --output "$OUTPUT/luminary099-p63-fault-matrix.json"

for artifact in \
    "$OUTPUT/conformance/report.json" \
    "$OUTPUT/conformance/manifest.json" \
    "$OUTPUT/conformance/apollors-trace.meta.json" \
    "$OUTPUT/luminary099-p63-run.json" \
    "$OUTPUT/luminary099-p63-coverage.json" \
    "$OUTPUT/luminary099-p63-fault-matrix.json"
do
    cargo run --quiet --release -p apollors-cli -- --repository . \
        validate-artifact --artifact "$artifact"
done

echo "ApolloRS qualification passed; artifacts: $OUTPUT"
