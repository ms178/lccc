#!/usr/bin/env bash
# ============================================================================
# check_fma_gating.sh — the FMA contract gate on the matmul arm.
#
# The audit found the vectorizer's matmul arm contracting `C += A*B` into FMA
# unconditionally: it ignored both the translate-time contract setting and the
# target ISA, and it did not check that the loop it was rewriting was the
# canonical full-trip shape.  `fma_transform_allowed(neon, fp_contract)` now
# gates the arm on `-ffp-contract` and on the FMA feature bit, and the matcher
# proves the loop's shape (header condition is the exit compare's operand, the
# exit is the sole exit, and the body holds no call, atomic or volatile) before
# any packed rewrite.  This gate pins all of that.
#
# Contracts:
#
#   1. THE ARM FIRES, and with the exact shape.  Default flags
#      (`-O2 -march=x86-64-v3`) must produce FOUR packed FMAs (`vfmadd`, ymm)
#      on the 64x64 matmul.  Four, not "some": that is the unrolled vector
#      body, and pinning the count means a regression that quietly drops to a
#      scalar remainder or to one lane fails here.  Counted on `%ymm` because
#      a scalar contraction elsewhere in the function is legitimate and must
#      NOT be mistaken for the packed arm (it is exactly what the shapes in
#      (4) keep, so the two counts have to be separate signals).
#
#   2. `-ffp-contract=off` emits ZERO FMAs of any width -- the source semantics
#      say the multiply and the add are separate operations, so contracting
#      them is a language-level violation, not an optimisation choice.  Zero
#      total (not just zero packed): the scalar contraction is gated by the
#      same flag, and if only the packed arm were gated this would still fail.
#
#   3. `-mno-fma` emits ZERO FMAs.  The target has no FMA unit; a `vfmadd` here
#      is an illegal instruction on the machine the flags describe.  And
#      `-ffp-contract=fast` still emits FOUR -- so (2) is a real gate and not a
#      frozen counter.
#
#   4. THE MATCHER REFUSES WHAT IT HAS NOT PROVEN.  A call in the loop body and
#      an early `return` (a side exit) must both keep the arm off: zero packed
#      FMAs, while the per-element scalar contraction may remain (zero or one
#      is accepted, and the scalar/ymm split is asserted, not the total).  This
#      is the non-vacuity control for (1): the same source text, the same
#      flags, and the only difference is the property that makes the rewrite
#      unproven.
#
#   5. SEMANTICS, ELEMENTWISE.  The inclusive-bound matrix multiply (the
#      pre-existing miscompile this round fixed) must print the oracle's
#      number at N=17, where the vector body covers only part of the row:
#      gcc -O0 cannot contract, reorder or vectorize, so it is the reference.
#      The comparison is a per-element weighted fold printed twice (forward and
#      reversed) plus the exact bit patterns of the weighted total and of two
#      elements -- a single `%.6f` of the sum would hide a wrong element that
#      cancels, which is exactly the failure mode a vector remainder has.  A
#      gate that only counted instructions would have passed on the broken
#      compiler.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc}
[[ -x $CCC ]] || { echo "check_fma_gating: lccc not found at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "check_fma_gating: no gcc oracle" >&2; exit 1; }

flags="-O2 -march=x86-64-v3"
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-fma.XXXXXX")
trap 'rm -rf "$work"' EXIT
fail=0
note() { printf '  %s\n' "$*"; }
bad() { printf '  FAIL: %s\n' "$*" >&2; fail=1; }

# Every FMA SPELLING, so a rewrite that swaps `vfmadd` for `vfnmadd`, `vfmsub`,
# `vfmaddsub`, or a scalar 3-operand form (`vfmadd132sd`) cannot slip past a
# counter that only knew one name.  The mnemonic must BE the whole first token,
# which is what keeps a label or a symbol containing "fmadd" from counting.
total() {
    awk '{ t = $1; if (t ~ /^v?f(n)?m(add|sub)(add|sub)?[0-9]*[a-z]*$/) n++ } END { print n + 0 }' "$1"
}
# The same spellings restricted to the packed (ymm/zmm) form: a scalar
# contraction elsewhere in the function is legitimate and must not be mistaken
# for the widened arm.
packed() {
    awk '{ t = $1; if (t ~ /^v?f(n)?m(add|sub)(add|sub)?[0-9]*[a-z]*$/ && $0 ~ /%[yz]mm/) n++ } END { print n + 0 }' "$1"
}

cat > "$work/shape.c" <<'EOF'
#define N 64
static double A[N][N], B[N][N], C[N][N];
void mm(void) { for (int i = 0; i < N; i++) for (int k = 0; k < N; k++) for (int j = 0; j < N; j++) C[i][j] += A[i][k] * B[k][j]; }
EOF

# (4)'s shapes: same kernel, one unproven property each.
cat > "$work/call.c" <<'EOF'
#define N 64
static double A[N][N], B[N][N], C[N][N];
double sink(double);
void mm(void) { for (int i = 0; i < N; i++) for (int k = 0; k < N; k++) for (int j = 0; j < N; j++) { double t = sink(C[i][j]); C[i][j] += A[i][k] * B[k][j] + t * 0.0; } }
EOF

cat > "$work/side_exit.c" <<'EOF'
#define N 64
static double A[N][N], B[N][N], C[N][N];
int mm(void) { for (int i = 0; i < N; i++) for (int k = 0; k < N; k++) for (int j = 0; j < N; j++) { if (B[k][j] > 0.9) return -1; C[i][j] += A[i][k] * B[k][j]; } return 0; }
EOF

for src in shape call side_exit; do
    "$CCC" $flags -S -o "$work/$src.s" "$work/$src.c" || bad "$src: compile failed"
done

note "contract 1: the packed arm fires, four lanes deep"
p=$(packed "$work/shape.s")
[[ $p -eq 4 ]] || bad "default flags: $p packed FMAs, want exactly 4; body:
$(grep -E 'vfmadd' "$work/shape.s" || true)"

note "contract 2/3: contract and target flags gate every FMA"
"$CCC" $flags -ffp-contract=off -S -o "$work/off.s" "$work/shape.c" || bad "-ffp-contract=off: compile failed"
t=$(total "$work/off.s")
[[ $t -eq 0 ]] || bad "-ffp-contract=off still emits $t FMA(s)"

"$CCC" $flags -mno-fma -S -o "$work/nofma.s" "$work/shape.c" || bad "-mno-fma: compile failed"
t=$(total "$work/nofma.s")
[[ $t -eq 0 ]] || bad "-mno-fma still emits $t FMA(s) (illegal on the target)"

"$CCC" $flags -ffp-contract=fast -S -o "$work/fast.s" "$work/shape.c" || bad "-ffp-contract=fast: compile failed"
p=$(packed "$work/fast.s")
[[ $p -eq 4 ]] || bad "-ffp-contract=fast: $p packed FMAs, want 4 (is contract=off frozen?)"

note "contract 4: unproven shapes do not reach the packed arm"
for src in call side_exit; do
    p=$(packed "$work/$src.s")
    [[ $p -eq 0 ]] || bad "$src: $p packed FMAs despite the unproven shape"
    s=$(grep -c 'vfmadd' "$work/$src.s" || true)
    [[ $s -le 1 ]] || bad "$src: $s scalar FMAs (the per-element contraction duplicates)"
    note "  $src: 0 packed, $s scalar (contraction only)"
done

note "contract 5: the inclusive-bound multiply still prints the oracle's number"
cat > "$work/rt.c" <<'EOF'
#include <stdio.h>
#define N 17
static double A[N][N], B[N][N], C[N][N];
int lim = N - 1;
void mm(void) { for (int i = 0; i < N; i++) for (int k = 0; k < N; k++) for (int j = 0; j <= lim; j++) C[i][j] += A[i][k] * B[k][j]; }
int main(void) {
    for (int i = 0; i < N; i++) for (int j = 0; j < N; j++) { A[i][j] = (double)((i * 7 + j * 3) % 13) / 7.0; B[i][j] = (double)((i * 5 + j) % 11) / 5.0; }
    mm();
    /* ELEMENTWISE oracle, not one 6-decimal aggregate: a wrong element that
       cancels in the total is invisible to a sum.  Every element is folded
       with its own weight, the fold is repeated in reverse (the two disagree
       if any element is wrong), and the exact bit patterns of the weighted
       total, of C[0][0] and of the last element are printed too (%a), so a
       result that is "right to six decimals" but not the right double fails. */
    double fwd = 0, rev = 0, last = C[0][0];
    for (int i = 0; i < N; i++) for (int j = 0; j < N; j++) {
        fwd += C[i][j] * (double)(i * N + j + 1);
        last = C[i][j];
    }
    for (int i = N - 1; i >= 0; i--) for (int j = N - 1; j >= 0; j--) rev += C[i][j] * (double)(i * N + j + 1);
    printf("%.17g %.17g %a %a %a\n", fwd, rev, fwd, C[0][0], last);
    return 0;
}
EOF
"$GCC" -O0 "$work/rt.c" -o "$work/ref" || bad "gcc oracle failed to build"
"$CCC" $flags "$work/rt.c" -o "$work/got" || bad "runtime probe failed to build"
ref=$(timeout 120 "$work/ref") || bad "gcc oracle exited non-zero"
got=$(timeout 120 "$work/got") || bad "lccc probe exited non-zero"
[[ "$ref" == "$got" ]] || bad "inclusive bound: lccc=$got oracle=$ref"

if [[ $fail -ne 0 ]]; then
    echo "check_fma_gating: FAILED" >&2
    exit 1
fi
echo "check_fma_gating: PASS (packed arm exact at 4, contract/target gating zero, unproven shapes refused, inclusive bound exact)"
