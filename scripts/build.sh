#!/bin/sh
set -eu
export EKKO_SOURCE_DIR="${EKKO_SOURCE_DIR:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}"
export EKKO_OUTPUT="${EKKO_OUTPUT:-$EKKO_SOURCE_DIR/ekko}"
if [ -z "${EKKO_PLATFORM_LIBRARY:-}" ]; then
  export EKKO_PLATFORM_LIBRARY="$EKKO_SOURCE_DIR/libekko-platform.so"
  cc -O2 -Wall -Wextra -Werror -fPIC -shared "$EKKO_SOURCE_DIR/src/platform.c" -o "$EKKO_PLATFORM_LIBRARY" -lutil -lz
fi
sbcl --no-userinit --no-sysinit --non-interactive --eval '(require "asdf")' \
  --eval '(let ((asdf:*central-registry* (list (truename (uiop:getenv "EKKO_SOURCE_DIR"))))) (asdf:compile-system (or (uiop:getenv "EKKO_BUILD_SYSTEM") "ekko")))'
exec sbcl --no-userinit --no-sysinit --non-interactive \
  --load "$EKKO_SOURCE_DIR/scripts/build.lisp"
