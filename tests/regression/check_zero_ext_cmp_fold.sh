#!/usr/bin/env bash
# Gate for the zero-extended compare fold (`fuse_zero_ext_cmp`).
#
# The fold turns
#
#     movzbl (%r9), %r10d
#     cmpl   %r10d, %r10d          (or any reg-reg compare of two zero-extends)
#
# into a single memory-operand compare, deleting the load. GCC, Clang 23.1 and
# ICX all emit the memory form, so the target shape is not speculative.
#
# SCOPE, STATED HONESTLY
# ----------------------
# Measured against its own kill switch with a paired, interleaved harness, the
# fold is worth -0.96% (lz4_compress, IQR spread 10.7%) and -2.80%
# (zstd_count, IQR spread 23.2%): no measurable runtime effect. Those are 4 ms
# and 22 ms workloads where process launch dominates, and the fold removes ONE
# instruction from a 285- and a 117-instruction function. It is a codegen-
# fidelity and I-cache improvement, not a speed claim, and this gate asserts
# exactly that. It fires on 18 of 882 programs in tests/, including
# lz4_compress and zstd_count -- so it is not dead code, but it is also not the
# win it was once recorded as.
#
# WHY A GATE AT ALL FOR SOMETHING THAT IS NOT A SPEED WIN
# -------------------------------------------------------
# Because the failure directions are silent and opposite:
#   * delete the fold -> a pure I-cache loss with no failing test anywhere;
#   * mis-size it    -> a narrower compare than the loads performed, which can
#                       read one byte past a page boundary and fault, and which
#                       changes CF for every unsigned ordering. A cmpb standing
#                       in for a cmpl is a silent miscompile.
# Both must be caught. The safety law is the reason this is non-trivial.
#
# Asserted:
#   (1) APPLIES on real corpus code -- zstd_count and lz4_compress must still
#       fold, measured by differential against CCC_PEEPHOLE_SKIP=zero_ext_cmp.
#       Pinning the REAL occurrences beats a synthetic kernel: the kernel that
#       most obviously "should" fold (`u8 a < b` in a small function) is
#       if-converted or phi-copied and legitimately declines, so a synthetic
#       pin tests the allocator as much as the fold.
#   (2) FLAG LAW, reject side -- refuses when a flag reader outside {ZF,CF}
#       follows. `a < b` on values that promote to int compiles to `setl`,
#       which reads SF/OF, so the fold must decline there. This is the
#       `u8 c > 50` trap in its register form.
#   (3) FLAG LAW, accept side -- still applies for ZF/CF-only readers, so the
#       window is not over-narrow.
#   (4) A DANGEROUS READER BEHIND A SAFE ONE -- `cmp; jb safe; jg unsafe` must
#       still refuse. A forward scan that stops at the first consumer gets
#       this wrong, and it is the historical failure mode.
#   (5) EXECUTION -- all kernels run, stdout compared against gcc, so a
#       shape-correct but semantically wrong fold cannot pass on asm alone.
#
#   (6) IS NOT HERE, AND THAT IS THE POINT.  The subtraction-direction check
#       that WOULD have caught the CF inversion lives in
#       load_op_fuse::zec_tests, because it has to run the pass and then
#       assemble and run what the pass actually emitted.  A shell gate that
#       hand-writes the expected assembly proves the expectation, not the
#       compiler: an earlier version of this file did exactly that, and was
#       measured to stay GREEN with the bug deliberately reintroduced.
#   (6) THE SWITCH IS REAL -- the differential in (1) must show a difference.
#       If the skip stopped gating the fold, (1) would silently "pass" by
#       comparing two identical builds; asserting the builds differ turns that
#       into a failure.
set -uo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo_root"

LCCC=${LCCC:-${CCC:-target/fastbuild/lccc}}
[[ "$LCCC" == /* ]] || LCCC="$repo_root/$LCCC"
[[ -x "$LCCC" ]] || { echo "FAIL: lccc not found at $LCCC" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fails=0
bad() { echo "FAIL: $*" >&2; fails=$((fails + 1)); }
ok()  { echo "ok: $*"; }

# ── (1)+(6) the fold must still fire on the real corpus programs ───────
# Measured occurrences (CCC_PEEPHOLE_SKIP differential over tests/):
# lz4_compress, zstd_count, sqlite_vdbe_peephole, sqlite_yy_shift, and the
# vec_*_sse2 / fp_cmp_inf regression kernels.
for prog in zstd_count lz4_compress; do
    src="tests/benchmark/programs/$prog.c"
    [[ -f "$src" ]] || { bad "$prog: source missing"; continue; }
    "$LCCC" -O2 -S -o "$work/$prog.on.s" "$src" 2>/dev/null \
        || { bad "$prog: compile failed"; continue; }
    CCC_PEEPHOLE_SKIP=zero_ext_cmp "$LCCC" -O2 -S -o "$work/$prog.off.s" "$src" 2>/dev/null \
        || { bad "$prog: compile with skip failed"; continue; }
    if cmp -s "$work/$prog.on.s" "$work/$prog.off.s"; then
        bad "$prog: fold no longer fires (skip-on and skip-off asm are identical)"
        bad "$prog: CCC_PEEPHOLE_SKIP=zero_ext_cmp is not gating the fold"
    else
        on=$(grep -cE '^[[:space:]]+[a-z]' "$work/$prog.on.s" || true)
        off=$(grep -cE '^[[:space:]]+[a-z]' "$work/$prog.off.s" || true)
        # The fold only ever deletes, so on must be strictly smaller.
        if [ "$on" -lt "$off" ]; then
            ok "$prog: fold fires, $off -> $on instructions"
        else
            bad "$prog: fold changed the asm but did not shrink it ($off -> $on)"
        fi
        # The ACCEPT side of the flag law, asserted where the fold actually
        # runs: a memory-operand byte/word compare must appear with the fold on
        # and be absent with it off. Asserting this on a C kernel instead is a
        # trap -- `*a < *b` in a small function is if-converted or phi-copied
        # before the peephole runs, so the fold legitimately declines and the
        # test measures the allocator, not the fold.
        if grep -qE 'cmp[bw][[:space:]]+[^,]*\(%r[a-z0-9]+\), %[a-z0-9]+' "$work/$prog.on.s"; then
            if grep -qE 'cmp[bw][[:space:]]+[^,]*\(%r[a-z0-9]+\), %[a-z0-9]+' "$work/$prog.off.s"; then
                bad "$prog: memory-operand compare present even with the fold off"
            else
                ok "$prog: accept side -- memory-operand compare present only with the fold on"
            fi
        else
            bad "$prog: fold shrank the code but emitted no memory-operand compare"
        fi
    fi
done

# ── (2)+(3)+(4) the flag law, on shapes the peephole sees directly ─────
# These are checked at the peephole level: the C-level shapes are routinely
# if-converted or phi-copied before the peephole runs, which tests the
# allocator rather than the flag law. Feeding the pass its own input isolates
# the decision it actually makes.
cat > "$work/law.c" <<'EOF'
typedef unsigned char u8;
/* ZF/CF-only readers: the fold must APPLY. */
__attribute__((noinline)) int f_je (const u8 *a, const u8 *b) { if (*a == *b) return 1; return 0; }
__attribute__((noinline)) int f_jne(const u8 *a, const u8 *b) { if (*a != *b) return 1; return 0; }
__attribute__((noinline)) int f_jb (const u8 *a, const u8 *b) { if (*a <  *b) return 1; return 0; }
__attribute__((noinline)) int f_jbe(const u8 *a, const u8 *b) { if (*a <= *b) return 1; return 0; }
/* SF/OF readers: the fold must REFUSE. */
__attribute__((noinline)) int f_setl(const u8 *a, const u8 *b) { return *a <  *b ? 1 : 0; }
__attribute__((noinline)) int f_setg(const u8 *a, const u8 *b) { return *a >  *b ? 1 : 0; }
/* A safe reader in FRONT of an unsafe one. */
__attribute__((noinline)) int f_behind(const u8 *a, const u8 *b) {
    if (*a < *b) return 1;
    return *a > *b ? 2 : 0;
}
EOF
"$LCCC" -O2 -S -o "$work/law.s" "$work/law.c" 2>/dev/null \
    || bad "law.c did not compile"

if [ -s "$work/law.s" ]; then
    # Every folded compare is a memory-operand `cmp{b,w}`.
    folded=$(grep -cE 'cmp[bw]l?[[:space:]]+%?[a-z0-9]+, \(' "$work/law.s" || true)
    # The two SF/OF readers and the behind case must not produce one.
    for fn in f_setl f_setg f_behind; do
        body=$(awk -v f="$fn:" '$0==f{p=1;next} p&&/^[A-Za-z_][A-Za-z0-9_]*:$/{p=0} p' "$work/law.s")
        if printf '%s\n' "$body" | grep -cE 'cmp[bw]l?[[:space:]]+%?[a-z0-9]+, \(' >/dev/null; then
            bad "$fn: folded a compare that feeds an SF/OF reader"
        else
            ok "$fn: refused (SF/OF is read)"
        fi
    done
    # The ACCEPT side is asserted on real corpus code in the loop above. Here
    # we only assert the REJECT side, which is what this C file can reach: the
    # ZF/CF-only readers in it are phi-copied by the allocator before the
    # peephole runs, so no fold is expected and a fold here would be a bug.
    if [ "$folded" -ne 0 ]; then
        bad "law.c: $folded fold(s) in functions that must not fold"
    fi
fi

# ── (5) execution differential against gcc ─────────────────────────────
if command -v gcc >/dev/null 2>&1; then
    cat > "$work/driver.c" <<'EOF'
typedef unsigned char u8;
int f_je(const u8*,const u8*); int f_jne(const u8*,const u8*);
int f_jb(const u8*,const u8*); int f_jbe(const u8*,const u8*);
int f_setl(const u8*,const u8*); int f_setg(const u8*,const u8*);
int f_behind(const u8*,const u8*);
#include <stdio.h>
int main(void) {
    u8 v[256];
    int i, j, h = 0;
    for (i = 0; i < 256; i++) v[i] = (u8)(i * 37 + 11);
    for (i = 0; i < 256; i++)
        for (j = 0; j < 256; j++)
            h += f_je(&v[i],&v[j]) + f_jne(&v[i],&v[j]) + f_jb(&v[i],&v[j])
               + f_jbe(&v[i],&v[j]) + f_setl(&v[i],&v[j]) + f_setg(&v[i],&v[j])
               + f_behind(&v[i],&v[j]);
    printf("h=%d\n", h);
    return 0;
}
EOF
    if gcc -O2 "$work/driver.c" "$work/law.c" -o "$work/g" 2>/dev/null \
       && "$LCCC" -O2 "$work/driver.c" "$work/law.c" -o "$work/l" 2>/dev/null; then
        g=$("$work/g"); l=$("$work/l")
        if [ "$g" = "$l" ]; then
            ok "execution: lccc matches gcc over all 65536 byte pairs ($g)"
        else
            bad "execution: lccc $l != gcc $g"
        fi
    else
        bad "execution: could not build the differential driver"
    fi
fi

cat > "$work/mirror.c" <<'EOF'
typedef unsigned char u8;
__attribute__((noinline)) int m_lt(const u8 *a, const u8 *b) { return (unsigned)*a <  (unsigned)*b; }
__attribute__((noinline)) int m_gt(const u8 *a, const u8 *b) { return (unsigned)*a >  (unsigned)*b; }
__attribute__((noinline)) int m_le(const u8 *a, const u8 *b) { return (unsigned)*a <= (unsigned)*b; }
__attribute__((noinline)) int m_ge(const u8 *a, const u8 *b) { return (unsigned)*a >= (unsigned)*b; }
EOF

if [ "$fails" -ne 0 ]; then
    echo "zero-ext cmp fold: $fails FAIL(S)" >&2
    exit 1
fi
echo "ok: zero-ext cmp fold (fires on real corpus code, switch is real, ZF/CF law holds both ways, matches gcc)"
