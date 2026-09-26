#!/bin/sh
# Bootstrap: tangle ekko.org into the working tree.
#
# ekko.org is the source of truth for every file it carries a :tangle target
# for.  This script is deliberately NOT tangled -- it is the thing that does
# the tangling, so a block for it would be circular.  Run it from anywhere.
set -eu
cd "$(dirname "$0")/.."

if [ -n "${EMACS:-}" ]; then
  emacs_cmd=$EMACS
elif command -v emacs >/dev/null 2>&1; then
  emacs_cmd=emacs
else
  emacs_cmd="nix run nixpkgs#emacs-nox --"
fi

# shellcheck disable=SC2086
exec $emacs_cmd --batch \
  --eval '(progn (require (quote org)) (org-babel-tangle-file "ekko.org"))'
