#!/usr/bin/env bash
# ARX lane-vectorizer: lane frames are chosen for latency, not op count.
#
# A ChaCha-style round loop is a serial recurrence (the benchmark even
# chains blocks through the feed-forward), so its speed is the critical
# path per double round, not its instruction count.  Both lane
# vectorizers (vec_arx: state array in memory; arx_vectorize: state in 16
# scalars) used to lane-rotate the diagonal group's b/c/d roles.  b is
# produced by `b = rotl(b, 7)` and consumed by `a += b` at once, so its two
# pshufd per double round sat on the critical path: 30 cycles where 28 are
# possible (llvm-mca agrees on znver4/5, raptorlake, sapphirerapids).
# Rotating a/c/d instead (b anchored) changes no instruction count.
#
# Checks, for both source forms and three ISA levels:
#   1. the loop really is lane-vectorized (pshufd present, so the latency
#      check below is not vacuous);
#   2. the recurrence bound (scripts/loop_latency.py) is at most the
#      anchored optimum: v3 28, SSE2 baseline 32, AVX-512VL (vprold) 24;
#   3. the programs compute RFC 7539 ChaCha20 (known-answer vector) and
#      the benchmark checksum equals the GCC build's (executed only where
#      the host supports the ISA).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC_BIN:-gcc}
repo=$(cd "$(dirname "$0")/../.." && pwd)
lat="python3 $repo/scripts/loop_latency.py"
tmp=${TMPDIR:-/tmp}/lccc-arx-frame.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
fail=0

cp "$repo/tests/benchmark/programs/chacha20_block.c" "$tmp/array.c"
cat >"$tmp/locals.c" <<'C'
typedef unsigned int u32;
#define R(v, n) (((v) << (n)) | ((v) >> (32 - (n))))
#define QR(a, b, c, d) \
  a += b; d ^= a; d = R(d, 16); c += d; b ^= c; b = R(b, 12); \
  a += b; d ^= a; d = R(d, 8);  c += d; b ^= c; b = R(b, 7);
__attribute__((noinline)) void core(u32 out[16], const u32 in[16])
{
  u32 x0 = in[0], x1 = in[1], x2 = in[2], x3 = in[3], x4 = in[4], x5 = in[5];
  u32 x6 = in[6], x7 = in[7], x8 = in[8], x9 = in[9], x10 = in[10];
  u32 x11 = in[11], x12 = in[12], x13 = in[13], x14 = in[14], x15 = in[15];
  for (int i = 0; i < 10; i++) {
    QR(x0, x4, x8, x12) QR(x1, x5, x9, x13) QR(x2, x6, x10, x14) QR(x3, x7, x11, x15)
    QR(x0, x5, x10, x15) QR(x1, x6, x11, x12) QR(x2, x7, x8, x13) QR(x3, x4, x9, x14)
  }
  out[0] = x0 + in[0]; out[1] = x1 + in[1]; out[2] = x2 + in[2]; out[3] = x3 + in[3];
  out[4] = x4 + in[4]; out[5] = x5 + in[5]; out[6] = x6 + in[6]; out[7] = x7 + in[7];
  out[8] = x8 + in[8]; out[9] = x9 + in[9]; out[10] = x10 + in[10];
  out[11] = x11 + in[11]; out[12] = x12 + in[12]; out[13] = x13 + in[13];
  out[14] = x14 + in[14]; out[15] = x15 + in[15];
}
int main(void)
{
  static const u32 t[16] = {
    0x61707865, 0x3320646e, 0x79622d32, 0x6b206574, 0x03020100, 0x07060504,
    0x0b0a0908, 0x0f0e0d0c, 0x13121110, 0x17161514, 0x1b1a1918, 0x1f1e1d1c,
    0x00000001, 0x09000000, 0x4a000000, 0x00000000};
  u32 o[16];
  core(o, t);
  return !(o[0] == 0xe4e7f110 && o[1] == 0x15593bd1 && o[14] == 0xe883d0cb &&
           o[15] == 0x4e3c50a2);
}
C

host_has() { grep -qw "$1" /proc/cpuinfo; }

# check FORM FUNC FLAGS MAX-BOUND run|skip
check() {
  local form=$1 func=$2 flags=$3 max=$4 run=$5
  local tag="$form ${flags:-(default v3)}"
  # shellcheck disable=SC2086
  "$CCC" -O2 $flags -DBLOCK_COUNT=4096U -DPASSES=2U -S "$tmp/$form.c" -o "$tmp/$form.s"
  if ! awk "/^$func:/,/^\\.size $func/" "$tmp/$form.s" | grep -q pshufd; then
    echo "FAIL [$tag]: $func is not lane-vectorized (no pshufd)"
    fail=1
    return
  fi
  local got
  got=$($lat "$tmp/$form.s" --function "$func" --json |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["cycles_per_iteration"])')
  if python3 -c "import sys; sys.exit(0 if $got <= $max else 1)"; then
    echo "ok   [$tag]: recurrence $got cycles/double-round (<= $max)"
  else
    echo "FAIL [$tag]: recurrence $got cycles/double-round, anchored optimum is $max"
    fail=1
  fi
  [[ $run == run ]] || return 0
  # shellcheck disable=SC2086
  "$CCC" -O2 $flags -DBLOCK_COUNT=4096U -DPASSES=2U "$tmp/$form.c" -o "$tmp/$form.bin"
  if [[ $form == array ]]; then
    "$GCC" -O2 -DBLOCK_COUNT=4096U -DPASSES=2U "$tmp/array.c" -o "$tmp/array.gcc"
    local want have
    want=$("$tmp/array.gcc")
    if ! have=$("$tmp/$form.bin"); then
      echo "FAIL [$tag]: RFC 7539 known-answer check failed"
      fail=1
    elif [[ $want != "$have" ]]; then
      echo "FAIL [$tag]: checksum $have, gcc says $want"
      fail=1
    fi
  elif ! "$tmp/$form.bin"; then
    echo "FAIL [$tag]: RFC 7539 known-answer vector mismatch"
    fail=1
  fi
}

for form in array:chacha20_core locals:core; do
  f=${form%%:*}
  fn=${form#*:}
  check "$f" "$fn" "" 28 "$(host_has avx2 && echo run || echo skip)"
  check "$f" "$fn" "-march=x86-64" 32 run
  check "$f" "$fn" "-march=x86-64-v4" 24 "$(host_has avx512vl && echo run || echo skip)"
done

if [[ $fail -ne 0 ]]; then
  exit 1
fi
echo "ARX lane-frame latency: PASS"
