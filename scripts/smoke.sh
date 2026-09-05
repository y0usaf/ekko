#!/bin/sh
# Exercise the distributed executable outside the checkout with hostile init files.
set -eu
ekko_binary=$1
smoke_dir=$(mktemp -d)
trap 'rm -rf "$smoke_dir"' EXIT HUP INT TERM
export HOME=$smoke_dir/home
export XDG_CACHE_HOME=$HOME/cache
mkdir -p "$HOME"
printf '(error "user init must not load")\n' > "$HOME/.sbclrc"
printf '(error "user init must not load")\n' > "$HOME/.sbcl-init.lisp"
cd "$smoke_dir"
"$ekko_binary" --version > version.txt 2> error.txt
printf '0.1.0\n' > expected.txt
cmp version.txt expected.txt
test ! -s error.txt
"$ekko_binary" --help > help.txt 2> error.txt
grep -qF 'Usage: ekko' help.txt
test ! -s error.txt
for argument in --unknown --eval --load; do
  status=0
  "$ekko_binary" "$argument" > output.txt 2> error.txt || status=$?
  test "$status" -eq 2
  test ! -s output.txt
  test -s error.txt
done
status=0
"$ekko_binary" --version extra > output.txt 2> error.txt || status=$?
test "$status" -eq 2
test ! -s output.txt
test -s error.txt
