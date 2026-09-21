#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
DEST=${1:-/tmp/apollors-virtualagc-conformance}
REVISION=0b13e5976dbc3c6c76aeab35195135261d7999ff
PATCH="$ROOT/docs/validation/yaagc-conformance-trace.patch"

if [ -e "$DEST" ]; then
    echo "refusing to overwrite existing yaAGC checkout: $DEST" >&2
    exit 1
fi

git clone https://github.com/virtualagc/virtualagc.git "$DEST" >&2
git -C "$DEST" checkout "$REVISION" >&2
git -C "$DEST" apply "$PATCH" >&2
make -C "$DEST/yaAGC" cc=cc yaAGC >&2

echo "$DEST/yaAGC/yaAGC"
