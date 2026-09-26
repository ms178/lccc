#!/usr/bin/env bash
# Unwinding through lccc-compiled code (x86-64 and i686).
#
# The integrated assembler used to drop every .cfi_* directive, so lccc
# objects had no .eh_frame at all: backtrace() stopped at the first lccc
# frame, and pthread_exit()/pthread_cancel() forced unwinding ended there
# too -- the cleanup handlers of every caller above it silently never ran.
# The x86-64 prologue additionally never described its callee-saved
# pushes, so even a present FDE would have restored the callee's values
# into the caller's %rbx/%r12-%r15.
#
# Checks:
#   1. `-c` output carries an .eh_frame with one FDE per function;
#   2. backtrace() sees the full recursion through lccc frames;
#   3. pthread_exit() from an lccc-compiled callee runs the cleanup
#      handler of a GCC-compiled caller, and that handler reads values
#      the caller keeps in callee-saved registers the lccc frame reused
#      (wrong/missing register save rules corrupt them);
#   4. the unwind tables pass the same test when lccc links (lccc-ld);
#   5. lccc-ld's .eh_frame_hdr indexes only live code, and every FDE keeps
#      its own CIE after --gc-sections compacts .eh_frame.  The pruning used
#      to re-aim an FDE at the nearest preceding CIE, handing a plain C
#      frame the personality/LSDA augmentation of a neighbour with a
#      cleanup (zPLR instead of zR); the i686 linker kept the FDEs of
#      discarded COMDAT __x86.get_pc_thunk.* copies as [0, 4) entries;
#   6. the CFA stays exact past calls whose callee pops stack bytes itself
#      (i386 SysV struct return: `ret $4`; fastcall: `ret $N`).  Nothing in
#      the call's text shows the pop, and the CFI used to stay N bytes too
#      high for the rest of the caller, so backtrace() from below such a
#      caller read a stack argument as the return address and stopped.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC_BIN:-gcc}
tmp=${TMPDIR:-/tmp}/lccc-eh-frame.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat >"$tmp/bt.c" <<'C'
#include <execinfo.h>
#include <stdio.h>
__attribute__((noinline)) int depth(int n, long a, long b)
{
    if (n == 0) {
        void *buf[64];
        return backtrace(buf, 64);
    }
    long x = a * 3 + n, y = b ^ n;
    int r = depth(n - 1, y, x);
    __asm__ volatile("" ::"r"(x), "r"(y));
    return r;
}
int main(void) { return depth(10, 1, 2) >= 12 ? 0 : 1; }
C

cat >"$tmp/caller.c" <<'C'
#include <pthread.h>
#include <stdio.h>
long churn(long a);
static volatile long seen = -1;
struct ctx { long a, b; };
static void cl(struct ctx **p) { seen = (*p)->a * 1000 + (*p)->b; }
__attribute__((noinline)) long F(long k)
{
    struct ctx c = {k * 100 + 1, k * 200 + 3};
    struct ctx *pc __attribute__((cleanup(cl))) = &c;
    long r = churn(k);
    return r + pc->a;
}
static void *th(void *a) { F((long)a); return 0; }
int main(void)
{
    pthread_t t;
    pthread_create(&t, 0, th, (void *)5);
    pthread_join(t, 0);
    if (seen != 502003) {
        printf("cleanup saw %ld, want 502003\n", seen);
        return 1;
    }
    return 0;
}
C

cat >"$tmp/callee.c" <<'C'
#include <pthread.h>
__attribute__((noinline)) long opaque(long x) { __asm__ volatile("" : "+r"(x)); return x; }
long churn(long a)
{
    long x1 = opaque(a * 3), x2 = opaque(a * 5), x3 = opaque(a * 7);
    long x4 = opaque(a * 11), x5 = opaque(a * 13), x6 = opaque(a * 17);
    if (a)
        pthread_exit(0);
    return x1 + x2 + x3 + x4 + x5 + x6;
}
C

cat >"$tmp/gc.c" <<'C'
#include <stdio.h>
void cl(int *p) { printf("cleanup %d\n", *p); }
__attribute__((noinline)) int f_unused(int x) { return x * 7 + printf("u"); }
__attribute__((noinline)) int f_cleanup(int x)
{
    int v __attribute__((cleanup(cl))) = x;
    return printf("c%d\n", v);
}
__attribute__((noinline)) int f_used(int x) { return x * 3 + printf("s\n"); }
int main(void) { return f_cleanup(1) + f_used(2) > 100; }
C

cat >"$tmp/pop.c" <<'C'
#include <execinfo.h>
#include <stdio.h>
struct T { int a, b, c; };
__attribute__((noinline)) struct T make(int x)
{
    struct T t = {x, x * 2, x * 3};
    __asm__ volatile("" ::"r"(&t) : "memory");
    return t;
}
#ifdef __i386__
__attribute__((noinline, fastcall))
#else
__attribute__((noinline))
#endif
int fc(int a, int b, int c, int d, int e) { return a + b * c - d * e; }
__attribute__((noinline)) int walk(void)
{
    void *buf[64];
    return backtrace(buf, 64);
}
__attribute__((noinline)) int mid(int n)
{
    struct T t = make(n);
    int k = fc(n, t.a, t.b, t.c, n + 1);
    int r = walk();
    __asm__ volatile("" ::"r"(t.a), "r"(k));
    return r * 1000 + ((t.b + k) & 1);
}
int main(int argc, char **argv)
{
    (void)argv;
    printf("%d\n", mid(argc + 4) / 1000);
    return 0;
}
C

# Walk PT_GNU_EH_FRAME: print "<bad-entry count> <augmentation per FDE>".
cat >"$tmp/ehhdr.py" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read()
is64 = d[4] == 2
if is64:
    phoff, = struct.unpack_from("<Q", d, 0x20)
    phentsize, phnum = struct.unpack_from("<HH", d, 0x36)
else:
    phoff, = struct.unpack_from("<I", d, 0x1C)
    phentsize, phnum = struct.unpack_from("<HH", d, 0x2A)
loads, eh = [], None
for i in range(phnum):
    o = phoff + i * phentsize
    if is64:
        t, fl, off, va, _, fs, _, _ = struct.unpack_from("<IIQQQQQQ", d, o)
    else:
        t, off, va, _, fs, _, fl, _ = struct.unpack_from("<IIIIIIII", d, o)
    if t == 1:
        loads.append((va, off, fs, fl))
    elif t == 0x6474E550:
        eh = (va, off)
def v2o(v):
    return next(off + v - va for va, off, fs, _ in loads if va <= v < va + fs)
hva, hoff = eh
assert d[hoff + 1] == 0x1B and d[hoff + 2:hoff + 4] == b"\x03\x3b"
cnt, = struct.unpack_from("<I", d, hoff + 8)
text = [(va, va + fs) for va, _, fs, fl in loads if fl & 1]
bad, prev, augs = 0, None, []
for i in range(cnt):
    ip, fde = struct.unpack_from("<ii", d, hoff + 12 + 8 * i)
    ip += hva
    fo = v2o(fde + hva)
    cie_ptr, pcb = struct.unpack_from("<Ii", d, fo + 4)
    if pcb + fde + hva + 8 != ip or not any(a <= ip < b for a, b in text) or (prev is not None and ip <= prev):
        bad += 1
    prev = ip
    c = fo + 4 - cie_ptr
    augs.append(d[c + 9:d.index(b"\0", c + 9)].decode())
print(bad, " ".join(augs))
PY

for m in "" -m32; do
    if [[ -n "$m" ]] && ! echo 'int main(void){return 0;}' | "$GCC" -m32 -x c - -o "$tmp/probe" 2>/dev/null; then
        echo "SKIP -m32: no 32-bit toolchain"
        continue
    fi
    tag=${m:-x86-64}
    "$CCC" $m -O2 -c "$tmp/callee.c" -o "$tmp/callee.o"
    # One FDE per defined function (opaque, churn, plus the i686 PIC thunk
    # when it is emitted).
    fdes=$(readelf --debug-dump=frames "$tmp/callee.o" | grep -c ' FDE ' || true)
    funcs=$(readelf -sW "$tmp/callee.o" | awk '$4 == "FUNC" && $7 != "UND"' | wc -l)
    if [[ "$funcs" -lt 2 || "$fdes" != "$funcs" ]]; then
        echo "FAIL ($tag): $fdes FDEs for $funcs functions in callee.o" >&2
        exit 1
    fi
    "$CCC" $m -O2 "$tmp/bt.c" -o "$tmp/bt"
    "$tmp/bt" || { echo "FAIL ($tag): backtrace stopped inside lccc frames" >&2; exit 1; }
    "$GCC" $m -O2 -fexceptions -c "$tmp/caller.c" -o "$tmp/caller.o"
    "$GCC" $m "$tmp/caller.o" "$tmp/callee.o" -o "$tmp/mix-gld" -lpthread
    "$tmp/mix-gld" || { echo "FAIL ($tag): forced unwind through lccc frame (GNU ld)" >&2; exit 1; }
    "$CCC" $m "$tmp/caller.o" "$tmp/callee.o" -o "$tmp/mix-lld" -lpthread
    "$tmp/mix-lld" || { echo "FAIL ($tag): forced unwind through lccc frame (lccc link)" >&2; exit 1; }

    "$GCC" $m -O2 -fexceptions -ffunction-sections -c "$tmp/gc.c" -o "$tmp/gc.o"
    "$GCC" $m -Wl,--gc-sections "$tmp/gc.o" -o "$tmp/gc-gld"
    "$CCC" $m -Wl,--gc-sections "$tmp/gc.o" -o "$tmp/gc-lld"
    "$tmp/gc-lld" >/dev/null || { echo "FAIL ($tag): gc-sections link does not run" >&2; exit 1; }
    read -r bad augs < <(python3 "$tmp/ehhdr.py" "$tmp/gc-lld")
    read -r _ ref_augs < <(python3 "$tmp/ehhdr.py" "$tmp/gc-gld")
    want=$(tr ' ' '\n' <<<"$ref_augs" | grep -c zPLR || true)
    got=$(tr ' ' '\n' <<<"$augs" | grep -c zPLR || true)
    if [[ "$bad" != 0 || "$got" != "$want" ]]; then
        echo "FAIL ($tag): .eh_frame_hdr after --gc-sections: bad=$bad zPLR FDEs=$got (GNU ld: $want) [$augs]" >&2
        exit 1
    fi
    "$GCC" $m -O2 "$tmp/pop.c" -o "$tmp/pop-gcc"
    "$CCC" $m -O2 "$tmp/pop.c" -o "$tmp/pop"
    want=$("$tmp/pop-gcc")
    got=$("$tmp/pop")
    if [[ "$got" != "$want" ]]; then
        echo "FAIL ($tag): backtrace past callee-popping calls saw $got frames, GCC build $want" >&2
        exit 1
    fi
    for f in "$tmp/mix-lld" "$tmp/bt" "$tmp/pop"; do
        read -r bad _ < <(python3 "$tmp/ehhdr.py" "$f")
        [[ "$bad" == 0 ]] || { echo "FAIL ($tag): $bad bogus .eh_frame_hdr entries in $(basename "$f")" >&2; exit 1; }
    done
done
echo "PASS: unwinding through lccc frames"
