#!/usr/bin/env bash
# Explicit-cast diagnostics between named address spaces, and the abstract
# declarator paths (sizeof, unnamed parameters, casts) that carry them.
#
# GCC 14.2 accepts every cast below and warns with "cast to ... address space
# pointer from disjoint ...". The rule is applied to the outermost pointer
# only, so `long *__seg_gs *` -> `long *__seg_gs *` is silent.
#
#   line 1  generic -> __seg_gs          warns
#   line 2  __seg_gs -> generic          warns
#   line 3  __seg_fs -> __seg_gs         warns
#   line 4  __seg_gs -> __seg_gs         silent
#   line 5  generic -> `long *__seg_gs *` warns
#   line 6  void * -> void *             silent
#
# The runtime half checks sizeof and the unnamed-parameter and cast reads
# against the values GCC produces.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-seg-cast.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

src="$tmp/casts.c"
cat > "$src" <<'C'
void a(void *p) { long __seg_gs *x = (long __seg_gs *)p; (void)x; }
void b(long __seg_gs *p) { long *y = (long *)p; (void)y; }
void c(long __seg_fs *p) { long __seg_gs *z = (long __seg_gs *)p; (void)z; }
void d(long *__seg_gs *p) { long *__seg_gs *w = (long *__seg_gs *)p; (void)w; }
void e(long *p) { long *__seg_gs *w = (long *__seg_gs *)p; (void)w; }
void f(void *p) { void *q = (void *)p; (void)q; }
C

fail=0
"$CCC" -O0 -c -o "$tmp/casts.o" "$src" 2> "$tmp/casts.err" || { echo "FAIL: casts.c rejected"; cat "$tmp/casts.err"; exit 1; }
warned=$(grep -c "warning: cast to" "$tmp/casts.err" || true)
lines=$( (grep "warning: cast to" "$tmp/casts.err" || true) | sed -E 's/^[^:]*:([0-9]+):.*/\1/' | tr '\n' ' ')
if [ "$warned" -ne 4 ] || [ "$lines" != "1 2 3 5 " ]; then
    echo "FAIL: expected 4 cast warnings on lines 1 2 3 5, got $warned on: $lines"
    cat "$tmp/casts.err"
    fail=1
fi
grep -q "cast to '__seg_gs' address space pointer from disjoint generic address space pointer" "$tmp/casts.err" \
    || { echo "FAIL: wording for generic -> __seg_gs"; fail=1; }
grep -q "cast to generic address space pointer from disjoint '__seg_gs' address space pointer" "$tmp/casts.err" \
    || { echo "FAIL: wording for __seg_gs -> generic"; fail=1; }
grep -q "cast to '__seg_gs' address space pointer from disjoint '__seg_fs' address space pointer" "$tmp/casts.err" \
    || { echo "FAIL: wording for __seg_fs -> __seg_gs"; fail=1; }

run="$tmp/run.c"
cat > "$run" <<'C'
#include <stdio.h>
long __seg_gs *gs_obj;
long *__seg_gs *gs_pp;
long sz1(void) { return sizeof(long *__seg_gs *); }
long sz2(void) { return sizeof(long __seg_gs *); }
long rd_unnamed(long *__seg_gs *);
long rd_unnamed(long *__seg_gs *q) { return **q; }
long cast_read(void *p) { return **(long *__seg_gs *)p; }
int main(void) {
    printf("%ld %ld\n", sz1(), sz2());
    return 0;
}
C
"$CCC" -O0 -o "$tmp/run" "$run" 2> "$tmp/run.err" || { echo "FAIL: run.c rejected"; cat "$tmp/run.err"; exit 1; }
got=$("$tmp/run")
if [ "$got" != "8 8" ]; then
    echo "FAIL: sizeof of pointer-to-segment-pointer printed '$got', expected '8 8'"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "check_seg_cast_warning: PASSED (4 cast warnings on lines 1 2 3 5; 0 on 4 6; sizeof 8 8)"
