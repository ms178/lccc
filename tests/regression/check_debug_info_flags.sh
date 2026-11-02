#!/usr/bin/env bash
# Debug-info honesty law: `-g` must never be silently dropped, and the
# `-g` family must follow GCC flag semantics exactly.
#
#   1. -g0/-ggdb0 explicitly DISABLE debug info (byte-identical to -g-less).
#   2. -g on a TU with surviving source spans emits a .file record.
#   3. -g describes every translation unit that defines code, even when
#      optimization folds the body into nothing (the file table is seeded
#      with the unit's own file rather than built from surviving spans),
#      and a TU that only declares symbols stays as quiet as GCC 16.2.
#   4. -gsplit-dwarf downgrades to non-split with a warning; -gno-* never
#      turns debug info on.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-dbgflags.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

printf 'int f(void){return 1;}\n' > "$tmp/spanless.c"
printf 'int g(int x){ int y = x + 3; while (y > 0) y--; return y; }\n' > "$tmp/spans.c"

# Pin 1: -g0 and -ggdb0 disable debug info entirely (GCC semantics).
"$CCC" -c -o "$tmp/plain.o" "$tmp/spans.c"
"$CCC" -g0 -c -o "$tmp/g0.o" "$tmp/spans.c"
"$CCC" -ggdb0 -c -o "$tmp/ggdb0.o" "$tmp/spans.c"
cmp "$tmp/plain.o" "$tmp/g0.o"
cmp "$tmp/plain.o" "$tmp/ggdb0.o"
if objdump -h "$tmp/g0.o" | grep -c '\.debug' >/dev/null; then
    echo "FAIL: -g0 produced debug sections" >&2; exit 1
fi

# Pin 2: -g emits a .file record when spans survive lowering.
"$CCC" -g -S -o "$tmp/g.s" "$tmp/spans.c"
grep -q '^\.file ' "$tmp/g.s" || { echo "FAIL: -g produced no .file record" >&2; exit 1; }

# Pin 3: -g describes a TU whose whole body folds away, at every level.
printf 'extern int ext(int);\n' > "$tmp/decls.c"
for opt in -O0 -O1 -O2; do
    "$CCC" -g $opt -c -o "$tmp/spanless$opt.o" "$tmp/spanless.c"
    # `grep -c ... >/dev/null` rather than `grep -q`: the consumer must read
    # every line, or objdump takes SIGPIPE and pipefail reports 141 on a
    # section that is present (scripts/check_pipefail_sigpipe.py).
    objdump -h "$tmp/spanless$opt.o" | grep -c '\.debug_info' >/dev/null || {
        echo "FAIL: $opt -g dropped .debug_info for a defining TU" >&2; exit 1
    }
    objdump -h "$tmp/spanless$opt.o" | grep -c '\.debug_line' >/dev/null || {
        echo "FAIL: $opt -g dropped .debug_line for a defining TU" >&2; exit 1
    }
done
# Silence is only honest where there is nothing to describe.
"$CCC" -g -c -o "$tmp/decls.o" "$tmp/decls.c" 2> "$tmp/decls.txt"
if [ -s "$tmp/decls.txt" ]; then
    echo "FAIL: -g reported on a declaration-only TU: $(cat "$tmp/decls.txt")" >&2
    exit 1
fi
if objdump -h "$tmp/decls.o" | grep -c '\.debug_info' >/dev/null; then
    echo "FAIL: declaration-only TU grew .debug_info" >&2
    exit 1
fi

# Pin 4: -gsplit-dwarf downgrades loudly; -gno-* never enables debug.
set +e
"$CCC" -gsplit-dwarf -S -o "$tmp/split.s" "$tmp/spanless.c" 2> "$tmp/split.txt"
st=$?
set -e
[ $st -eq 0 ] || { echo "FAIL: -gsplit-dwarf errored" >&2; exit 1; }
grep -q "gsplit-dwarf is not supported" "$tmp/split.txt" || {
    echo "FAIL: -gsplit-dwarf downgraded silently" >&2; exit 1
}
"$CCC" -gno-column-info -S -o "$tmp/gno.s" "$tmp/spans.c"
! grep -q '^\.file ' "$tmp/gno.s" || {
    echo "FAIL: -gno-* enabled debug info" >&2; exit 1
}

echo "debug-info flags gate: PASS"
