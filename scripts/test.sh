#!/bin/sh
set -eu
export EKKO_SOURCE_DIR="${EKKO_SOURCE_DIR:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}"
exec sbcl --no-userinit --no-sysinit --non-interactive \
  --load "$EKKO_SOURCE_DIR/scripts/test.lisp"
