#!/bin/sh
# Fail (non-zero) unless the working tree is exactly what ekko.org tangles to.
#
# Copies the generated file set into a temp dir, tangles ekko.org there, and
# compares every generated file against the working tree.  Same file set as the
# flake's checks.<system>.tangle derivation.  Nothing in the tree is modified.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

tar -C "$root" -cf - ekko.org ekko.asd flake.nix src examples scripts nix \
  | tar -C "$tmp" -xf -

list=$( { echo ekko.asd; echo flake.nix; ( cd "$tmp" && find src examples scripts nix -type f ); } | sort )

"$tmp/scripts/tangle.sh" >/dev/null

status=0
for rel in $list; do
  if [ ! -f "$tmp/$rel" ]; then
    echo "MISSING  $rel (ekko.org did not tangle it)"
    status=1
  elif ! cmp -s "$tmp/$rel" "$root/$rel"; then
    echo "CHANGED  $rel (working tree differs from ekko.org)"
    status=1
  fi
done

if [ "$status" -ne 0 ]; then
  echo "ekko.org is out of date; run scripts/tangle.sh" >&2
  exit 1
fi
echo "ok: every generated file matches ekko.org"
