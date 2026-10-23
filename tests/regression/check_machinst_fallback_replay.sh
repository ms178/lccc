#!/usr/bin/env bash
# Exercise the real MachInst fallback path. The emitter's endpoint assertion
# proves replay does not move the IR program-point cursor, and the runtime test
# proves that the default backend reproduced the buffered instructions.
#
# Trigger shapes age out as the allocator improves: varargs_abi triggered
# the fallback on both the pre-rework and reworked allocators (an
# inline-asm-adjacent Lea kept an unresolvable virtual register), but the
# current allocator resolves that web, so the fallback leg uses
# agg_split_copy_out (a 3-instruction window with an unresolvable vreg at
# the time of writing). varargs_abi remains the runtime leg. If the debug
# grep below ever goes empty again, find a fresh trigger with
# `CCC_MI_DEBUG=1 lccc -O2 -S` over tests/regression/*.c rather than
# weakening the assertion — the fallback path must stay exercised.
set -eu

CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp="${TMPDIR:-/tmp}/lccc-machinst-fallback.$$"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir -p "$tmp"

CCC_MI_DEBUG=1 "$CCC" -O2 -S "$dir/agg_split_copy_out.c" \
    -o "$tmp/trigger.s" 2>"$tmp/trigger.log"
grep -Eq '^\[MI-FALLBACK\] [1-9][0-9]* instructions -> default path$' \
    "$tmp/trigger.log"

CCC_MI_DEBUG=1 "$CCC" -O2 "$dir/varargs_abi.c" \
    -o "$tmp/test" 2>"$tmp/compile.log"
"$tmp/test" >"$tmp/run.log"
grep -Fxq 'OK varargs_abi' "$tmp/run.log"
