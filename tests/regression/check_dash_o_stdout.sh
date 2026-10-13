#!/usr/bin/env bash
# AUD-DRV-1 gate: `-o -` is the standard output for every TEXT mode.
#
# `gcc -S -o - x.c` writes assembly to stdout; before this fix lccc created a
# FILE literally named `-` in the working directory and wrote nothing to
# stdout, so `lccc -S -o - x.c | ...` pipelines (and anything that streams
# preprocessed text) silently got an empty stream plus a stray file.
#
# Pinned here:
#   * -S / -E / -M / -MM / -dM with `-o -` write to stdout (through a PIPE,
#     which is how consumers use it);
#   * `-MF -` writes the make rule to stdout (GCC);
#   * object/executable modes reject `-o -` with a diagnostic instead of
#     producing a binary in a file named `-` (GCC refuses too).
set -euo pipefail

CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cd "$td"
printf 'int f(void){return 42;}\n' > t.c

fail=0
note() { printf 'FAIL: %s\n' "$1" >&2; fail=1; }

# 1. -S to stdout, consumed through a pipe.
got=$("$CCC" -S -o - t.c | grep -c 'f:')
[ "$got" = 1 ] || note "-S -o - did not stream assembly to stdout (matches: ${got:-0})"
[ -e ./- ] && note "-S -o - created a file named '-'"

# 2. -E to stdout.
"$CCC" -E -o - t.c | grep -c 'return 42' >/dev/null || note "-E -o - did not stream preprocessed text"
[ -e ./- ] && note "-E -o - created a file named '-'"

# 3. -M to stdout (dependency mode).
"$CCC" -M -o - t.c | grep -c '^[^:]*: ' >/dev/null || note "-M -o - did not stream a make rule"
[ -e ./- ] && note "-M -o - created a file named '-'"

# 4. -dM to stdout.
"$CCC" -dM -E -o - t.c | grep -c '^#define __STDC__' >/dev/null || note "-dM -E -o - did not stream macro definitions"

# 5. -MF - sends the dependency rule to stdout.
"$CCC" -c t.c -o /dev/null -MMD -MF - | grep -c 't.c' >/dev/null || note "-MF - did not stream the dependency rule"

# 6. Object/executable modes must refuse, not create a file named '-'.
out=$("$CCC" -c -o - t.c 2>&1) && note "-c -o - was accepted"
grep -c 'cannot write an object file or executable to stdout' <<<"$out" >/dev/null ||
    note "-c -o - rejection lacks the explanatory diagnostic: $out"
[ -e ./- ] && note "-c -o - created a file named '-'"

# 7. Negative control: a real file path still works (the change must not have
#    turned every -o into stdout).
"$CCC" -S -o real.s t.c
[ -s real.s ] || note "-S -o real.s did not write the assembly file"

[ "$fail" = 0 ] || { echo 'dash-o-stdout: FAIL'; exit 1; }
echo 'dash-o-stdout: PASS'
