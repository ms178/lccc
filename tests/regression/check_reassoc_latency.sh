#!/usr/bin/env bash
# Latency-driven reassociation (src/passes/reassoc_latency.rs, "reassoc_lat").
#
# SHA-256's round computes t1 = h + Σ1(e) + Ch(e,f,g) + K[i] + W[i] and
# e' = d + t1.  Built in source order, four serial adds follow Σ1 on the
# loop-carried e -> e' chain although h + K[i] + W[i] does not depend on
# this round's e (the loads are addressed by the induction variable).  The
# pass recombines associative trees by operand availability; the round
# loop's recurrence falls from 8 to 5 cycles at the default x86-64-v3
# target (Σ1: 3, + t1: 1, d +: 1 -- the floor), 8 to 6 at baseline x86-64
# (no andn: Ch is 3 deep, like Σ1).  GCC 16.2, Clang 23.1 and ICX all
# leave 7 (scripts/loop_latency.py --loads on their -O2 output).
#
# Checks:
#   1. bound: the round loop's recurrence bound is at most the optimum at
#      both ISA levels, AND the same compiler with the pass disabled
#      exceeds it (negative control in the same run: the gate cannot pass
#      vacuously because the loop picker or the model drifted);
#   2. SHA-256 known-answer vector (the program exits 2 on mismatch) and
#      the benchmark checksum;
#   3. differential: loops with signed/unsigned 32/64-bit Add/Xor/And/Or
#      trees, loop-carried and pointer-chasing leaves, wrapping unsigned
#      sums and a multi-use interior node print the same as GCC's build;
#   4. the rewritten IR passes the structural verifier (CCC_VERIFY_IR=abort
#      armed for the differential builds), and the pass really fires on the
#      differential program.  Idempotence is a unit test.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC_BIN:-gcc}
repo=$(cd "$(dirname "$0")/../.." && pwd)
lat="python3 $repo/scripts/loop_latency.py --loads"
tmp=${TMPDIR:-/tmp}/lccc-reassoc-lat.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
fail=0

sha="$repo/tests/benchmark/programs/sha256_transform.c"
bound_of() { $lat "$1" --function sha256_transform | sed -n 's/.*bound \([0-9.]*\) .*/\1/p'; }

for cfg in ":v3:5" "-march=x86-64:x86-64:6"; do
  flag=${cfg%%:*}
  rest=${cfg#*:}
  name=${rest%%:*}
  want=${rest#*:}
  $CCC -O2 $flag -S -o "$tmp/on.s" "$sha"
  CCC_DISABLE_PASSES=reassoc_lat $CCC -O2 $flag -S -o "$tmp/off.s" "$sha"
  on=$(bound_of "$tmp/on.s")
  off=$(bound_of "$tmp/off.s")
  if awk -v a="$on" -v w="$want" 'BEGIN{exit !(a <= w)}'; then
    echo "PASS: $name round recurrence $on cycles (<= $want)"
  else
    echo "FAIL: $name round recurrence $on cycles, want <= $want"
    fail=1
  fi
  if awk -v a="$off" -v w="$want" 'BEGIN{exit !(a > w)}'; then
    echo "PASS: $name negative control: pass disabled gives $off (> $want)"
  else
    echo "FAIL: $name negative control gives $off: the check is not sensitive"
    fail=1
  fi
  $CCC -O2 $flag -o "$tmp/sha" "$sha"
  if got=$("$tmp/sha") && [[ $got == 054db5f638d89d8b ]]; then
    echo "PASS: $name SHA-256 known answer + checksum $got"
  else
    echo "FAIL: $name SHA-256 run: '${got:-}' (exit $?)"
    fail=1
  fi
done

cat >"$tmp/diff.c" <<'C'
#include <stdio.h>
#include <stdint.h>
struct node { struct node *next; int32_t v; };
static struct node pool[64];
__attribute__((noinline)) static int32_t s32(const int32_t *a, int n) {
  int32_t x = 1, y = 2;
  for (int i = 0; i < n; i++) {        /* signed tree, carried leaves */
    int32_t t = y + (x ^ (x >> 3)) + a[i & 31] + a[(i + 5) & 31] + 7;
    y = x; x = t & 0x0fffffff;         /* no signed overflow */
  }
  return x ^ y;
}
__attribute__((noinline)) static uint64_t u64(const uint64_t *a, int n) {
  uint64_t x = 0x9e3779b97f4a7c15u, y = 3;
  for (int i = 0; i < n; i++) {        /* wrapping unsigned sums */
    uint64_t r = (x << 13) | (x >> 51);
    x = y + r + a[i & 31] + (x ^ a[(i * 7) & 31]) + 0x1234;
    y ^= x;
  }
  return x + y;
}
__attribute__((noinline)) static uint32_t logic(const uint32_t *a, int n) {
  uint32_t p = ~0u, q = 0, s = 5;
  for (int i = 0; i < n; i++) {        /* Xor / And / Or trees */
    s = s ^ (s << 5) ^ a[i & 31] ^ a[(i + 1) & 31] ^ (uint32_t)i;
    p = p & (s | 1u) & a[i & 31] & ~a[(i + 3) & 31] & 0xffff00ffu;
    q = q | (s >> 7) | (a[i & 31] & 3u) | p;
  }
  return p ^ q ^ s;
}
__attribute__((noinline)) static int32_t chase(int n) {
  int32_t acc = 0;
  struct node *c = &pool[0];
  for (int i = 0; i < n; i++) {        /* pointer-chasing leaf */
    acc = (acc + c->v + i + (acc >> 2)) & 0x3fffffff;
    c = c->next;
  }
  return acc;
}
__attribute__((noinline)) static uint32_t multiuse(const uint32_t *a, int n) {
  uint32_t x = 1, m = 0;
  for (int i = 0; i < n; i++) {        /* interior node with two uses */
    uint32_t t = x + a[i & 31];
    x = t + a[(i + 2) & 31] + (x >> 1);
    m ^= t;
  }
  return x ^ m;
}
int main(void) {
  int32_t a[32]; uint64_t b[32]; uint32_t c[32];
  for (int i = 0; i < 32; i++) {
    a[i] = (int32_t)(i * 2654435761u) >> 8;
    b[i] = 0xda942042e4dd58b5u * (uint64_t)(i + 1);
    c[i] = 0x85ebca6bu * (uint32_t)(i + 7);
  }
  for (int i = 0; i < 64; i++) {
    pool[i].next = &pool[(i * 17 + 5) & 63];
    pool[i].v = i * 31 - 500;
  }
  printf("%d %llu %u %d %u\n", s32(a, 1000), (unsigned long long)u64(b, 1000),
         logic(c, 1000), chase(1000), multiuse(c, 1000));
  return 0;
}
C
$GCC -O1 -o "$tmp/ref" "$tmp/diff.c"
want=$("$tmp/ref")
for flag in "" "-march=x86-64" "-O3"; do
  CCC_VERIFY_IR=abort $CCC -O2 $flag -o "$tmp/diff" "$tmp/diff.c"
  got=$("$tmp/diff")
  if [[ $got == "$want" ]]; then
    echo "PASS: differential ${flag:-v3}: $got"
  else
    echo "FAIL: differential ${flag:-v3}: got '$got', gcc '$want'"
    fail=1
  fi
done

# The pass fires on the differential program (otherwise check 3 tests
# nothing): its -O2 IR changes when the pass is disabled.
$CCC -O2 -S -o "$tmp/d_on.s" "$tmp/diff.c"
CCC_DISABLE_PASSES=reassoc_lat $CCC -O2 -S -o "$tmp/d_off.s" "$tmp/diff.c"
if cmp -s "$tmp/d_on.s" "$tmp/d_off.s"; then
  echo "FAIL: reassoc_lat did not change the differential program"
  fail=1
else
  echo "PASS: reassoc_lat rewrites trees in the differential program"
fi
exit $fail
