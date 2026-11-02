#!/usr/bin/env bash
# `-mno-sse` / `-mgeneral-regs-only` contract on x86-64.
#
# The kernel compiles with `-mno-sse -mno-mmx -mno-sse2 -mno-avx -mno-80387`:
# the xmm file is architecturally unavailable (CR4.OSFXSR clear → #UD).
# lccc's x86-64 FP lowering has no x87 path, so a floating-point VALUE that
# survives optimisation in such a TU can only become SSE code.  Emitting it
# silently was a boot-time #UD; the driver now refuses the TU with a
# diagnostic naming the function and the offending instruction (GCC refuses
# the same TUs under -mno-sse -mno-80387: "SSE register return with SSE
# disabled").  Integer-only TUs — including 16-byte struct copies, memset,
# and every idiom the kernel relies on — must still compile to zero SIMD
# references, and the i686 backend (which lowers FP through x87) is
# unaffected.
set -euo pipefail
repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail=0

KERNEL=(-O2 -std=gnu18 -fno-strict-aliasing -mno-sse -mno-mmx -mno-sse2 -mno-3dnow
        -mno-avx -mno-sse4a -ffreestanding -fno-stack-protector
        -fno-asynchronous-unwind-tables -mno-red-zone -mcmodel=kernel
        -march=x86-64 -mtune=generic -fno-jump-tables -mno-80387)

cat > "$tmp/fp.c" <<'C'
int g(int x) { double d = x; return (int)(d * 1.5); }
C
cat > "$tmp/int.c" <<'C'
struct s { unsigned long a, b; };
struct big { unsigned long w[8]; };
unsigned long dot(const struct s *p, int n) {
    unsigned long t = 0;
    for (int i = 0; i < n; i++) t += p[i].a * p[i].b;
    return t;
}
void cp(struct s *d, const struct s *s) { *d = *s; }
void cpb(struct big *d, const struct big *s) { *d = *s; }
void z(struct big *d) { __builtin_memset(d, 0, sizeof *d); }
int cmp(const struct big *a, const struct big *b) { return __builtin_memcmp(a, b, sizeof *a); }
unsigned __int128 mul128(unsigned long a, unsigned long b) { return (unsigned __int128)a * b; }
C
cat > "$tmp/dead.c" <<'C'
/* FP that folds away at -O2 must not trip the diagnostic: the check is on
 * the final text, not on the source. */
int k(void) { return (int)(2.5 * 4.0); }
C

# 1. FP value under the kernel flag set: refused, with a precise message.
if "$ccc" "${KERNEL[@]}" -c "$tmp/fp.c" -o "$tmp/fp.o" 2>"$tmp/fp.err"; then
    echo "FAIL: -mno-sse TU with a live double compiled silently" >&2; fail=1
elif ! grep -q "in function 'g'.*requires SSE.*offending instruction" "$tmp/fp.err"; then
    echo "FAIL: diagnostic text: $(cat "$tmp/fp.err")" >&2; fail=1
fi
# 1b. Same under -mgeneral-regs-only alone.
if "$ccc" -O2 -mgeneral-regs-only -c "$tmp/fp.c" -o "$tmp/fp.o" 2>/dev/null; then
    echo "FAIL: -mgeneral-regs-only TU with a live double compiled silently" >&2; fail=1
fi

# 2. Integer-only kernel TU: compiles, zero SIMD references.
if ! "$ccc" "${KERNEL[@]}" -S "$tmp/int.c" -o "$tmp/int.s" 2>"$tmp/int.err"; then
    echo "FAIL: integer kernel TU refused: $(cat "$tmp/int.err")" >&2; fail=1
elif grep -qE '%[xyz]mm|%mm[0-7]' "$tmp/int.s"; then
    echo "FAIL: SIMD register in integer kernel TU:" >&2
    grep -nE '%[xyz]mm|%mm[0-7]' "$tmp/int.s" | head -5 >&2; fail=1
fi

# 3. Folded FP: accepted.
if ! "$ccc" "${KERNEL[@]}" -c "$tmp/dead.c" -o "$tmp/dead.o" 2>"$tmp/dead.err"; then
    echo "FAIL: constant-folded FP refused: $(cat "$tmp/dead.err")" >&2; fail=1
fi

# 4. Without -mno-sse the same FP TU compiles (the gate is flag-driven).
if ! "$ccc" -O2 -c "$tmp/fp.c" -o "$tmp/fp.o" 2>/dev/null; then
    echo "FAIL: FP TU refused without -mno-sse" >&2; fail=1
fi

# 5. i686 lowers FP through x87 and is not subject to the gate.
if ! "$ccc" -m32 -O2 -mno-sse -S "$tmp/fp.c" -o "$tmp/fp32.s" 2>"$tmp/fp32.err"; then
    echo "FAIL: i686 -mno-sse FP TU refused: $(cat "$tmp/fp32.err")" >&2; fail=1
elif grep -q '%xmm' "$tmp/fp32.s"; then
    echo "FAIL: i686 -mno-sse emitted xmm" >&2; fail=1
fi

if [[ $fail -eq 0 ]]; then echo "PASS: -mno-sse FP diagnostic contract"; fi
exit $fail
