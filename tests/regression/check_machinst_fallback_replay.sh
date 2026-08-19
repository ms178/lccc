#!/usr/bin/env bash
# Exercise the real MachInst fallback path. The emitter's endpoint assertion
# proves replay does not move the IR program-point cursor, and the runtime test
# proves that the default backend reproduced the buffered instructions.
#
# varargs_abi triggers the fallback on both the pre-rework and reworked
# allocators (an inline-asm-adjacent Lea keeps an unresolvable virtual
# register); kernel_asm_macro_semantics only triggered it under the old
# allocator, so it is not relied on here.
set -eu

CCC=${CCC:-./target/fastbuild/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp="${TMPDIR:-/tmp}/lccc-machinst-fallback.$$"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir -p "$tmp"

CCC_MI_DEBUG=1 "$CCC" -O2 "$dir/varargs_abi.c" \
    -o "$tmp/test" 2>"$tmp/compile.log"

grep -Eq '^\[MI-FALLBACK\] [1-9][0-9]* instructions -> default path$' \
    "$tmp/compile.log"
"$tmp/test" >"$tmp/run.log"
grep -Fxq 'OK varargs_abi' "$tmp/run.log"
