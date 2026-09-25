#!/usr/bin/env bash
# Debug-info honesty law: `-g` must never be silently dropped, and the
# `-g` family must follow GCC flag semantics exactly.
#
#   1. -g0/-ggdb0 explicitly DISABLE debug info (byte-identical to -g-less).
#   2. -g on a TU with surviving source spans emits a .file record.
#   3. -g on a TU whose spans were all erased WARNS instead of failing
#      later downstream (the kernel's pahole/BTF step is the motivating
#      consumer: without this warning the failure surfaced as an unrelated
#      BTFIDS link error three link stages away from the cause).
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
if objdump -h "$tmp/g0.o" | grep -q '\.debug'; then
    echo "FAIL: -g0 produced debug sections" >&2; exit 1
fi

# Pin 2: -g emits a .file record when spans survive lowering.
"$CCC" -g -S -o "$tmp/g.s" "$tmp/spans.c"
grep -q '^\.file ' "$tmp/g.s" || { echo "FAIL: -g produced no .file record" >&2; exit 1; }

# Pin 3: -g on a fully span-erased TU warns, never silently drops.
set +e
"$CCC" -g -S -o "$tmp/none.s" "$tmp/spanless.c" 2> "$tmp/warn.txt"
st=$?
set -e
[ $st -eq 0 ] || { echo "FAIL: spanless -g compile errored" >&2; exit 1; }
grep -q "no debug information was generated" "$tmp/warn.txt" || {
    echo "FAIL: -g silently produced no debug info" >&2; exit 1
}

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
