#!/usr/bin/env bash
# Structural guard for the -Os nested-loop inlining policy.
#
# The zlib-ng Adler-32 corpus has one cold validation call and one hot call in
# an outer loop.  Inlining the cold call enables constant folding; inlining the
# remaining large loop body into the outer loop creates severe register spills.
# Small tail-loop helpers should still fold into the standalone checksum body.
# Conversely, the bounded hash-table helpers must inline into their hot outer
# loops: outlining them is smaller, but reproducibly slows pointer chasing.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
src="$dir/../benchmark/programs/zlib_ng_adler32.c"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-os-nested-loop.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

"$CCC" -Os -S "$src" -o "$tmp/test.s"

grep -Eq '^zlib_ng_adler32_c:' "$tmp/test.s"
calls=$(grep -Ec '^[[:space:]]*call[q]?[[:space:]]+zlib_ng_adler32_c([[:space:]]|$)' "$tmp/test.s" || true)
if [ "$calls" -ne 1 ]; then
    echo "expected one outlined hot zlib_ng_adler32_c call at -Os, found $calls" >&2
    exit 1
fi

tail_calls=$(grep -Ec '^[[:space:]]*call[q]?[[:space:]]+zlib_ng_adler32_len_(16|64)([[:space:]]|$)' "$tmp/test.s" || true)
if [ "$tail_calls" -ne 0 ]; then
    echo "expected Adler-32 tail helpers to inline at -Os, found $tail_calls calls" >&2
    exit 1
fi

hash_src="$dir/../benchmark/programs/hash_table.c"
"$CCC" -Os -S "$hash_src" -o "$tmp/hash.s"

hash_calls=$(grep -Ec '^[[:space:]]*call[q]?[[:space:]]+(hash|insert|lookup)([[:space:]]|$)' "$tmp/hash.s" || true)
if [ "$hash_calls" -ne 0 ]; then
    echo "expected bounded hash-table loop helpers to inline at -Os, found $hash_calls calls" >&2
    exit 1
fi

# Cost nested-loop candidates after the tiny/static-inline descendants that the
# fixed-point inliner will expand.  The raw loop_kernel snapshot is only 43 IR
# instructions, but its eight mandatory descendants raise the cloned body well
# above the 64-instruction cap.  A raw-only estimate used to clone one copy into
# outer_loop before the second inliner invocation observed the expanded cost.
transitive_src="$dir/os_nested_loop_transitive_inline.c"
"$CCC" -m32 -Os -DSTRUCTURAL_ONLY -S "$transitive_src" -o "$tmp/transitive.s"

grep -Eq '^loop_kernel:' "$tmp/transitive.s"
transitive_calls=$(grep -Ec '^[[:space:]]*call[lq]?[[:space:]]+loop_kernel([[:space:]]|$)' "$tmp/transitive.s" || true)
if [ "$transitive_calls" -ne 2 ]; then
    echo "expected 2 outlined transitive-cost loop_kernel calls at -Os, found $transitive_calls" >&2
    exit 1
fi
