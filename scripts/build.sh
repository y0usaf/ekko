#!/bin/sh
set -eu
export EKKO_SOURCE_DIR="${EKKO_SOURCE_DIR:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}"
export EKKO_OUTPUT="${EKKO_OUTPUT:-$EKKO_SOURCE_DIR/ekko}"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
home=$(sbcl --noinform --no-userinit --no-sysinit --non-interactive \
  --eval '(write-string (sb-ext:native-namestring (sb-int:sbcl-homedir-pathname)))')
cc -O2 -Wall -Wextra -Werror -c "$EKKO_SOURCE_DIR/src/platform.c" -o "$work/platform.o"
cc -o "$work/ekko-runtime" "$home/sbcl.o" "$work/platform.o" \
  $(sed -n 's/^LINKFLAGS=//p' "$home/sbcl.mk") $(sed -n 's/^LIBS=//p' "$home/sbcl.mk") -lutil -lz
lisp() {
  SBCL_HOME=$home "$work/ekko-runtime" --core "$home/sbcl.core" --noinform \
    --no-userinit --no-sysinit --non-interactive "$@"
}
lisp --eval '(require "asdf")' \
  --eval '(let ((asdf:*central-registry* (list (truename (uiop:getenv "EKKO_SOURCE_DIR"))))) (asdf:compile-system (or (uiop:getenv "EKKO_BUILD_SYSTEM") "ekko")))'
lisp --load "$EKKO_SOURCE_DIR/scripts/build.lisp"
