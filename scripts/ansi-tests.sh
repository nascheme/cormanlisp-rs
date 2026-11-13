#!/bin/sh
# Run the upstream HyperSpec example tests (test/ansi-chapter-*.lisp) and
# report pass/fail counts per chapter.
#
# usage: scripts/ansi-tests.sh [chapter...]
#   UPSTREAM  upstream Corman Lisp checkout (default /home/nas/pkg/cormanlisp)
#   KEEP      directory to keep the per-chapter output in (default: temporary)
set -e
here=$(cd "$(dirname "$0")/.." && pwd)
upstream=${UPSTREAM:-/home/nas/pkg/cormanlisp}
chapters=${*:-2 3 4 5 6 7 8}
work=${KEEP:-$(mktemp -d)}
mkdir -p "$work"

cargo build -q --manifest-path "$here/Cargo.toml"
lisp="$here/target/debug/cormanlisp"
"$lisp" --boot "$here/lisp/boot.lisp" --save "$work/test.img" > /dev/null

# The harness from ansi-examples.lisp, without its LOAD forms.
sed '/^(load "test/d' "$upstream/test/ansi-examples.lisp" > "$work/harness.lisp"

total_pass=0
total_fail=0
total_err=0
cd "$upstream"
for c in $chapters; do
    out="$work/chapter-$c.out"
    printf '(load "%s")\n(load "test/ansi-chapter-%s.lisp")\n' "$work/harness.lisp" "$c" |
        timeout 300 "$lisp" --image "$work/test.img" > "$out" 2>&1 || true
    pass=$(grep -c '^PASSED' "$out" || true)
    fail=$(grep -c '^FAILED' "$out" || true)
    err=$(grep -c 'An error occurred' "$out" || true)
    echo "chapter $c: $pass passed, $fail failed, $err errors"
    total_pass=$((total_pass + pass))
    total_fail=$((total_fail + fail))
    total_err=$((total_err + err))
done
echo "total: $total_pass passed, $total_fail failed, $total_err errors"
[ -n "$KEEP" ] || rm -rf "$work"
