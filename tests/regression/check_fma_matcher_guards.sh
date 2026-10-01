#!/usr/bin/env bash
# ============================================================================
# check_fma_matcher_guards.sh — the matmul arm's legality proof, and the two
# miscompiles it was letting through.
#
# The PR #716 review filed four high-risk correctness gaps.  Two of them were
# reproduced with the compiler before anything was changed, and both are
# RUNTIME claims, so this gate is built the way they were found: build the
# shape with lccc and with `gcc -O0`, run both, compare stdout.  `gcc -O0`
# cannot vectorize, contract or reassociate, so it is the reference for what
# the source program means.
#
# Contracts:
#
#   1. THE INVERTED-POLARITY LOOP IS NOT VECTORIZED AS IF IT WERE CANONICAL.
#      `while (1) { if (j != limit) break; body; ++j; }` branches out on the
#      TRUE edge, so the loop's continuation is the fall-through.  The
#      transform implements exactly one orientation (rewrite the comparison in
#      place, keep looping while it holds).  Before the fix, `limit = 0`
#      printed 0.000000 where the scalar loop prints 10623.257143, and at
#      `limit = 1` the two outputs were exactly swapped.  All four limits here
#      must now match the oracle.
#
#   2. AN EFFECT THE BLACKLIST DID NOT NAME IS REFUSED.  The legality check
#      was a blacklist (calls, atomics, volatiles) and `asm volatile` rode
#      along on one lane of each vector group: the counter totalled 16384 in a
#      64^3 kernel where the scalar loop runs 262144.  It must now total
#      262144.  This is the contract that motivates an ALLOWLIST: what is not
#      proven per-element safe is refused by construction.
#
#   3. UNMODELED LOOP-CARRIED STATE IS REFUSED.  A scalar the loop maintains
#      for after the loop (`extra += 1`) escapes as one update per group.
#      Refused now.
#
#   (Both IR verifiers run for every contract in this file: CCC_VERIFY_IR=abort
#   and CCC_VALIDATE_SSA=1.  They check structure -- duplicate labels and
#   definitions, uses no path defines -- not types; the width fix is correct by
#   construction, and a gate can only add that the rewritten loops stay valid.)
#
#   4. A `long` INDEX KEEPS ITS WIDTH AND ITS REACH.  The reminder this
#      transform inserts was built with hardcoded I32 values (phi, compare,
#      arithmetic) over the pattern's IV regardless of the IV's type; it now
#      carries the IV width.  The runtime-bound `long` matmul must (a) still be
#      vectorized -- asserted on the emitted `vfmadd` count, so "fix" by
#      disabling the arm fails here -- and (b) print the oracle's number.
#
#   5. THE REACH CONTROL.  The canonical `for (j = 0; j <= N-1; j++)` matmul
#      must still match the packed arm.  Without this, every contract above
#      could be satisfied by turning the transform off.
#
#   6. THE REDUCTION ARM'S EXIT POLARITY (a memory-safety contract).  The
#      reduction arm took its vector bound from the exit comparison's limit
#      while IGNORING the operator, then re-emitted the loop as the canonical
#      exclusive test -- so `i <= n-1` ran one full vector group past the end
#      of the array whenever the trip count was not a multiple of the width.
#      The pre-fix compiler SEGFAULTED on the guard-page probe below (n = 63,
#      array placed against a PROT_NONE page, so an over-read faults instead of
#      quietly reading a neighbour).  A non-canonical comparison is refused
#      now; the canonical `<` / `!=` shapes stay vectorized and must read only
#      the array they were given.  The guard-page probe is deliberate: an
#      over-read that stays inside a big array is invisible to a plain run.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc}
[[ -x $CCC ]] || { echo "check_fma_matcher_guards: lccc not found at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "check_fma_matcher_guards: no gcc oracle" >&2; exit 1; }

flags="-O2 -march=x86-64-v3"
# Every compile in this gate also runs the IR verifiers.  `CCC_VERIFY_IR=abort`
# panics INSIDE the pass loop, naming the pass that produced the bad IR, when a
# rewritten function has a duplicate label, a duplicate definition, or a use of
# a value no executed path defines; `CCC_VALIDATE_SSA=1` adds the unique-def and
# watermark check.  They are STRUCTURAL, not type checks: the long-index width
# fix is correct by construction (every remainder value and type field carries
# the IV's type), so what a gate can add on top of the oracle is that the loops
# this transform rewrites stay structurally valid IR.
export CCC_VERIFY_IR=abort
export CCC_VALIDATE_SSA=1
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-matcher.XXXXXX")
trap 'rm -rf "$work"' EXIT
fail=0
note() { printf '  %s\n' "$*"; }
bad() { printf '  FAIL: %s\n' "$*" >&2; fail=1; }

# Compare a source against the gcc -O0 oracle, both built and RUN.
# $1 name, $2 expected stdout (empty = must equal the oracle's), remaining args
# are extra compiler flags.
run_case() {
    local name=$1 src=$2 want=${3:-}
    local ref got
    "$GCC" -O0 -w "$src" -o "$work/$name.ref" || { bad "$name: gcc oracle failed to build"; return; }
    "$CCC" $flags -w "$src" -o "$work/$name.got" || { bad "$name: lccc failed to build"; return; }
    ref=$(timeout 300 "$work/$name.ref") || bad "$name: oracle exited non-zero"
    got=$(timeout 300 "$work/$name.got") || bad "$name: lccc exited non-zero"
    if [[ -n $want ]]; then
        [[ "$got" == "$want" ]] || bad "$name: lccc printed '$got', want '$want'"
    elif [[ "$ref" != "$got" ]]; then
        bad "$name: lccc='$got' oracle='$ref'"
    fi
    echo "$got"
}

prelude() {
    cat > "$1" <<EOF
#include <stdio.h>
#define N 64
static double A[N][N], B[N][N], C[N][N];
static void init(void){ for (int i=0;i<N;i++) for (int j=0;j<N;j++){ A[i][j]=(double)((i*7+j*3)%13)/7.0; B[i][j]=(double)((i*5+j)%11)/5.0; C[i][j]=0.0; } }
static double fin(void){ double h=0; for(int i=0;i<N;i++) for(int j=0;j<N;j++) h+=C[i][j]*((i*3+j)%5+1); return h; }
EOF
}

# ── contract 1: inverted-polarity break loops ───────────────────────────────
note "contract 1: inverted-polarity break loops match the oracle at every trip count"
for lim in 0 1 3 17; do
    prelude "$work/p2_$lim.c"
    cat >> "$work/p2_$lim.c" <<EOF
int lim = $lim;
void mm(void){ for(int i=0;i<N;i++) for(int k=0;k<N;k++){ int j=0; while(1){ if (j != lim) break; C[i][j]+=A[i][k]*B[k][j]; ++j; } } }
int main(void){ init(); mm(); printf("%.6f\n", fin()); return 0; }
EOF
    got=$(run_case "p2_$lim" "$work/p2_$lim.c" "")
    note "  lim=$lim -> $got"
done

# ── contract 2: inline asm effects are refused ──────────────────────────────
note "contract 2: an inline-asm side effect in the body keeps its count"
prelude "$work/asm.c"
cat >> "$work/asm.c" <<'EOF'
volatile int asm_runs = 0;
void mm(void){ for(int i=0;i<N;i++) for(int k=0;k<N;k++) for(int j=0;j<N;j++){
    C[i][j]+=A[i][k]*B[k][j];
    __asm__ __volatile__("addl $1, asm_runs(%%rip)" ::: "memory");
} }
int main(void){ init(); mm(); printf("%.6f %d\n", fin(), asm_runs); return 0; }
EOF
# The oracle comparison AND the exact count: 64^3 = 262144 is what the scalar
# loop runs, and 16384 is what the vectorized form used to run.
asm_out=$(run_case "asm" "$work/asm.c" "")
if [[ "$asm_out" != *" 262144" ]]; then
    bad "inline asm: '$asm_out' (want the scalar count 262144)"
fi
note "  asm counter -> ${asm_out##* }"

# ── contract 3: unmodeled loop-carried state is refused ─────────────────────
note "contract 3: a loop-carried scalar read after the loop"
prelude "$work/carry.c"
cat >> "$work/carry.c" <<'EOF'
int extra_total = 0;
void mm(void){ int extra=0; for(int i=0;i<N;i++) for(int k=0;k<N;k++) for(int j=0;j<64;j++){ C[i][j]+=A[i][k]*B[k][j]; extra+=1; } extra_total=extra; }
int main(void){ init(); mm(); printf("%.6f %d\n", fin(), extra_total); return 0; }
EOF
carry_out=$(run_case "carry" "$work/carry.c" "")
if [[ "$carry_out" != *" 262144" ]]; then
    bad "loop-carried scalar: '$carry_out' (want the scalar count 262144)"
fi
note "  carried counter -> ${carry_out##* }"

# ── contract 4: `long` index width, with reach ──────────────────────────────
note "contract 4: long-indexed runtime-bound matmul -- correct AND still vectorized"
prelude "$work/long.c"
cat >> "$work/long.c" <<'EOF'
long lim = 61;
void mm(void){ for(int i=0;i<N;i++) for(int k=0;k<N;k++) for(long j=0;j<=lim;j++) C[i][j]+=A[i][k]*B[k][j]; }
int main(void){ init(); mm(); printf("%.6f\n", fin()); return 0; }
EOF
run_case "long" "$work/long.c" "" >/dev/null
# The width defect lived in the IR while the answer stayed right, so the
# runtime comparison cannot be the whole contract: the verifiers (exported for
# the whole gate above) audit the rewritten function too -- a failure aborts the
# compile, so the exit status below IS the assertion.
ssa_out=$("$CCC" $flags -w -S -o "$work/long_ssa.s" "$work/long.c" 2>&1) \
    || bad "long index: the IR verifier rejected the rewritten loop: $ssa_out"
if grep -qE '\[ir-verify\]|SSA VIOLATION|WATERMARK VIOLATION' <<<"$ssa_out"; then
    bad "long index: verifier complaint over the remainder loop: $ssa_out"
fi
note "  long index: IR verifiers clean (CCC_VERIFY_IR=abort + CCC_VALIDATE_SSA=1)"
"$CCC" $flags -w -S -o "$work/long.s" "$work/long.c"
packed=$(grep -cE 'vf(n)?m(add|sub)(add|sub)?[0-9]*[a-z]*[[:space:]]+.*%[yz]mm' "$work/long.s" || true)
[[ $packed -gt 0 ]] || bad "long index: no packed FMA left -- the width fix must not cost the vectorization"
note "  long IV: $packed packed FMAs, result matches the oracle"

# ── contract 5: the reach control ───────────────────────────────────────────
note "contract 5: the canonical inclusive-bound matmul still matches the packed arm"
prelude "$work/canon.c"
cat >> "$work/canon.c" <<'EOF'
void mm(void){ for(int i=0;i<N;i++) for(int k=0;k<N;k++) for(int j=0;j<=N-1;j++) C[i][j]+=A[i][k]*B[k][j]; }
int main(void){ init(); mm(); printf("%.6f\n", fin()); return 0; }
EOF
run_case "canon" "$work/canon.c" "" >/dev/null
"$CCC" $flags -w -S -o "$work/canon.s" "$work/canon.c"
packed=$(grep -cE 'vfmadd.*%[yz]mm' "$work/canon.s" || true)
[[ $packed -eq 4 ]] || bad "canonical matmul: $packed packed FMAs, want the four-lane body"
note "  canonical matmul: $packed packed FMAs (reach preserved)"

# ── contract 6: the reduction exit polarity (memory safety) ─────────────────
note "contract 6: reduction exit polarity -- inclusive bound refused, no over-read"
cat > "$work/red_gp.c" <<'EOF'
#include <stdio.h>
#include <sys/mman.h>
static int src[64];
static long sum_lt(int *a,int n){ long s=0; for(int i=0;i<n;i++) s+=a[i]; return s; }
static long sum_le(int *a,int n){ long s=0; for(int i=0;i<=n-1;i++) s+=a[i]; return s; }
static long sum_ne(int *a,int n){ long s=0; for(int i=0;i!=n;i++) s+=a[i]; return s; }
int main(void){ long pg=4096; int n=63;
  for(int i=0;i<64;i++) src[i]=i;
  char *base=mmap(0,pg*2,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);
  if(base==(char*)-1){ perror("mmap"); return 2; }
  if(mprotect(base+pg,pg,PROT_NONE)!=0){ perror("mprotect"); return 2; }
  int *a=(int*)(base+pg-n*4);        /* exactly 63 ints, ends at the guard page */
  for(int i=0;i<n;i++) a[i]=src[i];
  printf("%ld %ld %ld\n", sum_lt(a,n), sum_le(a,n), sum_ne(a,n));
  return 0; }
EOF
run_case "red_guardpage" "$work/red_gp.c" "1953 1953 1953" >/dev/null
# Capture instead of piping into `grep -q`: under `pipefail` the compiler's
# SIGPIPE on the closed pipe would fail the check even when the message matched.
vec_msgs=$(LCCC_DEBUG_VECTORIZE=1 "$CCC" $flags -w -S -o "$work/red_gp.dbg.s" "$work/red_gp.c" 2>&1 || true)
if grep -q "exit comparison is not the canonical exclusive IV test" <<<"$vec_msgs"; then
    note "  inclusive exit comparison: refused (was the over-read)"
else
    bad "inclusive reduction: refusal not reported -- the guard is gone"
fi
"$CCC" $flags -w -S -o "$work/red_gp.s" "$work/red_gp.c"
packed=$(grep -cE 'vpadd|padd' "$work/red_gp.s" || true)
[[ $packed -gt 0 ]] || bad "reduction arm: no packed add left -- contract 6 must not cost the reach"
note "  canonical < / != reductions: $packed packed adds (reach intact)"

if [[ $fail -ne 0 ]]; then
    echo "check_fma_matcher_guards: FAILED" >&2
    exit 1
fi
echo "check_fma_matcher_guards: PASS (inverted polarity + asm + carried state + inclusive exit refused; long-IV width kept, reach intact)"
