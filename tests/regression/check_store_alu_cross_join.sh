#!/usr/bin/env bash
# ============================================================================
# check_store_alu_cross_join.sh
#
# Pin the soundness of the phase-2 peephole pass `store_alu_fold`
# (memory_fold::fold_store_alu_memop) at CFG merge blocks.
#
# The pass replaces an ALU frame-slot memory operand with the register that
# the immediately-preceding `mov %reg, slot` stored, then deletes the store
# when liveness proves it dead.  Its forward scan documented labels (CFG
# rejoin points) as hard barriers — but the scan loop's no-operand line arm
# did `j += 1; continue` WITHOUT running the barrier test.  A block label
# ` .LBB34:` contains no whitespace, so it sailed straight through:
#
#     .LBB32:                      # n != 0 arm
#       ... compute rotl in %r12
#       movq %r12, 120(%rsp)
#       jmp .LBB34
#     .LBB33:                     # n == 0 arm
#       movq %rbp, 120(%rsp)      # identity value
#     .LBB34:                      # CFG join
#       xorq 120(%rsp), %r11      # became: xorq %rbp, %r11  <-- WRONG
#
# The join reads the slot to merge the TWO different arm values; forwarding
# the last linear store substituted the identity arm unconditionally,
# discarding the rotated arm.  Found by the differential fuzz engine at
# -O1 (seeds 20260912/20260939/20260960/20260990); pre-fix binaries print
# cd6d9925a34d293a where gcc and the fixed compiler print d4c1e9cb5328b2ec.
#
# Contract pinned:
#   1. The original fuzzer program matches gcc at -O0/-O1/-O2/-Os (the bug
#      was -O1-only, but all levels are pinned to stop a regression moving).
#   2. Small variable-rotate ternary diamonds in a loop match gcc, with and
#      without register pressure added.
#   3. The pass stays ACTIVE: default -O1 assembly must still differ from a
#      CCC_PEEPHOLE_SKIP=store_alu_fold build on the pressure case (the fix
#      narrows the fold to straight-line windows; it must not kill it).
#   4. The skip-switch build is itself correct (legacy escape hatch).
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_store_alu_cross_join: lccc not found at $CCC" >&2; exit 1; }
CC=${CC:-gcc}
command -v $CC >/dev/null || { echo "check_store_alu_cross_join: $CC not found" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fail=0

# ---- Case 1: the exact differential-fuzz repro (seed 20260912) ------------
cat > "$work/diamond_soup.c" <<'EOF'
#include <stdint.h>
#include <stdio.h>
#include <inttypes.h>

static uint64_t global_a;
static uint32_t global_b;
static volatile uint32_t observed;

struct pair { uint32_t lo; uint32_t hi; };
struct bits { unsigned a:3; unsigned b:5; unsigned c:8; };

static uint64_t rotl64(uint64_t x, unsigned n) {
  n &= 63u; return n ? ((x << n) | (x >> ((64u - n) & 63u))) : x;
}
static uint64_t mix(uint64_t x, uint64_t y, unsigned n) {
  x ^= rotl64(y + UINT64_C(0x9e3779b97f4a7c15), n);
  x *= UINT64_C(0xbf58476d1ce4e5b9);
  x ^= x >> 29; return x;
}
static struct pair step_pair(struct pair p, uint32_t x) {
  p.lo = (p.lo + x) ^ (p.hi >> 3);
  p.hi = (p.hi * UINT32_C(1664525)) + UINT32_C(1013904223) + p.lo;
  return p;
}
static uint64_t postdec_path(uint32_t n, uint64_t seed) {
  uint64_t sum = seed;
  do { sum = mix(sum, (uint64_t)n + UINT64_C(17), n); } while (n-- != 0);
  return sum;
}
static uint64_t wide_path(uint64_t a, uint64_t b) {
  __int128 x = ((__int128)(uint64_t)a << 32) | (uint32_t)b;
  x = x * 3 + 7;
  return (uint64_t)x ^ (uint64_t)(x >> 64);
}

int main(void) {
  uint64_t acc = UINT64_C(0xaef9c97a0fe8f576);
  uint64_t salt = UINT64_C(0x36fe6b9d6e6d7582);
  uint32_t a[16];
  struct pair p = { UINT32_C(1), UINT32_C(2) };
  struct bits bf = { 0, 0, 0 };
  (void)global_a; (void)global_b; (void)bf;
  for (unsigned i = 0; i < 16; ++i) a[i] = (uint32_t)(acc >> (i & 31u)) ^ (i * UINT32_C(0x45d9f3b));
  acc = mix(acc + UINT64_C(0xf6eca5f99f23daa7), salt ^ UINT64_C(0x92c33820b360f560), 10u);
  a[1] ^= (uint32_t)(acc >> 8u); acc += a[1];
  if ((acc ^ UINT64_C(0xb0edbe70cf3badd6)) & UINT64_C(1)) acc ^= rotl64(salt, 5u); else acc += UINT64_C(0x896e8da7fd61a658);
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0xfd79dbce))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0x1152aede))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  a[4] ^= (uint32_t)(acc >> 13u); acc += a[4];
  a[14] ^= (uint32_t)(acc >> 13u); acc += a[14];
  if ((acc ^ UINT64_C(0x0db26c5050d7287e)) & UINT64_C(1)) acc ^= rotl64(salt, 54u); else acc += UINT64_C(0x5389943609c8779e);
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0x11d365ae))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  a[2] ^= (uint32_t)(acc >> 24u); acc += a[2];
  a[6] ^= (uint32_t)(acc >> 53u); acc += a[6];
  observed = (uint32_t)(acc ^ UINT32_C(0x0b1fc704));
  { volatile uint32_t local_v = observed; local_v ^= (uint32_t)salt; observed = local_v; acc ^= observed; }
  acc ^= postdec_path(8u, salt);
  acc ^= wide_path(acc, salt);
  for (unsigned j = 0; j < 16; ++j) {
    unsigned k = (unsigned)((acc + j) & 15u);
    a[k] = (uint32_t)mix(a[k], acc ^ j, j);
    acc ^= ((uint64_t)a[k] << ((j & 7u) * 8u));
  }
  printf("%016" PRIx64 " %08" PRIx32 "\n", acc, observed);
  return 0;
}
EOF

# ---- Case 2: focused adversarial diamonds -------------------------------
cat > "$work/rot_diamonds.c" <<'EOF'
#include <stdint.h>
#include <stdio.h>
#include <inttypes.h>

/* Every rotate count 0..63 exercised; n==0 must take the identity arm
   (x86 shift masks the count but the C ternary still mergges two arms). */
static uint64_t rotl64(uint64_t x, unsigned n) {
  n &= 63u;
  return n ? ((x << n) | (x >> ((64u - n) & 63u))) : x;
}
static uint64_t rotr64(uint64_t x, unsigned n) {
  n &= 63u;
  return n ? ((x >> n) | (x << ((64u - n) & 63u))) : x;
}

/* live across the diamond: register pressure so the merge value spills */
static uint64_t sink[32];
static uint64_t pressure(uint64_t x, uint64_t y, unsigned n) {
  uint64_t a = x ^ 0x123456789abcdef0ULL;
  uint64_t b = y + 0x0fedcba987654321ULL;
  uint64_t c = a * 31 + b;
  uint64_t d = rotl64(a ^ b, n) ^ rotr64(c, (n + 7) & 63u);
  sink[0] = a; sink[1] = b; sink[2] = c;
  sink[3] = c + a; sink[4] = b ^ d; sink[5] = a + d;
  sink[6] = a * 7; sink[7] = b * 13;
  uint64_t e = rotl64(y, n);
  uint64_t f = n & 1 ? rotr64(x, n) : rotl64(x, 63 & (unsigned)(n + 1));
  sink[8] = e + f; sink[9] = e ^ f; sink[10] = e * 3 + f;
  return d ^ e ^ f;
}

int main(void) {
  uint64_t acc = 0xdeadbeefcafef00dULL;
  for (unsigned n = 0; n < 64; ++n) {
    acc ^= pressure(acc + n * 0x9e3779b97f4a7c15ULL,
                    acc ^ (uint64_t)(0x9e3779b97f4a7c15ULL + n), n);
  }
  uint64_t chk = 0;
  for (int i = 0; i < 11; ++i) chk += sink[i];
  printf("%016" PRIx64 " %016" PRIx64 "\n", acc, chk);
  return 0;
}
EOF

run_diff() {
  local src=$1 opt=$2 extra=${3:-}
  $CC -O2 -w "$src" -o "$work/ref"
  env $extra "$CCC" "$opt" "$src" -o "$work/t" 2>"$work/err"
  "$work/ref" > "$work/r1"
  "$work/t"   > "$work/r2"
  if ! cmp -s "$work/r1" "$work/r2"; then
    echo "FAIL: $(basename "$src") $opt $extra" >&2
    diff "$work/r1" "$work/r2" >&2 || true
    return 1
  fi
}

for opt in -O0 -O1 -O2 -O3 -Os; do
  run_diff "$work/diamond_soup.c" "$opt" || fail=1
  run_diff "$work/rot_diamonds.c" "$opt" || fail=1
done
# Escape-hatch build must also be correct.
run_diff "$work/diamond_soup.c" -O1 "CCC_PEEPHOLE_SKIP=store_alu_fold" || fail=1
run_diff "$work/rot_diamonds.c" -O1 "CCC_PEEPHOLE_SKIP=store_alu_fold" || fail=1

# ---- Property 3: the pass is still active (straight-line folds survive) --
# The label-barrier fix must not disable the pass wholesale; a straight-line
# store->ALU window in a high-pressure straight-line kernel still folds.
sha="$here/../benchmark/programs/sha256_transform.c"
[[ -f $sha ]] || sha=$here/../../tests/benchmark/programs/sha256_transform.c
[[ -f $sha ]] || { echo "check_store_alu_cross_join: sha256_transform.c not found" >&2; exit 1; }
"$CCC" -O1 -S "$sha" -o "$work/on.s" 2>/dev/null
env CCC_PEEPHOLE_SKIP=store_alu_fold "$CCC" -O1 -S "$sha" -o "$work/off.s" 2>/dev/null
if cmp -s "$work/on.s" "$work/off.s"; then
  echo "FAIL: store_alu_fold made no edits on the straight-line pressure" >&2
  echo "kernel; the label barrier fix must not disable the pass wholesale." >&2
  fail=1
fi

if (( fail == 0 )); then
  echo "PASS check_store_alu_cross_join"
fi
exit $fail
