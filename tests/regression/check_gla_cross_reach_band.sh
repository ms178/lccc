#!/usr/bin/env bash
# RA-GLA-03: the Speed-tier reach band is derived per target from the buyable
# callee-saved GPR set and validated by measured fire-site frontiers:
#
#   aarch64 = 10 (x19-x28)   riscv64 = 6 (deliberately below the 11 s-GPRs)
#   x86-64  = 6 (rbx,rbp,r12-r15)        i686 = 2 (esi/edi under PIC)
#
# This is an *assembly/plan* pin: no qemu execution is required.
#
#  1. AArch64 default planning must use reach=10, which is what removes the
#     const slot home from the N=17 matmul prologue (frame 128 -> 112; the
#     hot FP loops are untouched). Forcing CCC_GLA_REACH=6 must restore the
#     wider frame, proving the number comes from the target derivation rather
#     than an x86 literal leaking across backends.
#
#  2. RISC-V default planning must stay at reach=6: on double_reduction the
#     three global bases are `covers_reachable=false` at the default and zero
#     edits are applied; forcing reach=11 admits them, and the register-
#     allocator cascade then remats the LCG constants *inside* the hot loop
#     (304 insns vs GCC's 74 — moving away from the oracle). This pin fails
#     loudly if a future "ABI says 11" change reintroduces that regression.
#
#  3. The x86-64 host default remains reach=6.
set -euo pipefail
repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
arm_cc=$repo/target/fastbuild/lccc-arm
rv_cc=$repo/target/fastbuild/lccc-riscv
x64_cc=${CCC:-$repo/target/fastbuild/lccc}
matmul=$repo/tests/regression/vectorize_matmul_n17.c
dred=$repo/tests/benchmark/programs/double_reduction.c
nbody=$repo/tests/benchmark/programs/nbody.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Cross binaries are only present after the full fastbuild; hosts without the
# cross toolchain still run the x86-64 leg. Missing pieces skip, never fail.
arm_inc=""
if command -v aarch64-linux-gnu-gcc >/dev/null 2>&1; then
  arm_inc=$(aarch64-linux-gnu-gcc -print-file-name=include)
fi
fail=0
# Hermetic: an ambient A/B override must not change the derived defaults this
# test pins (the Rust unit tests skip under CCC_GLA_REACH for the same reason).
unset CCC_GLA_REACH

# matmul.c has no system includes, so the aarch64 leg needs only the cross
# binary; the cross gcc (header directory) is passed when it exists.
arm_inc_flag=()
[[ -n $arm_inc ]] && arm_inc_flag=(-I"$arm_inc")
# The body of one function: its lines from `name:` to the next label.
fn_body() { awk -v f="$2" '$0 == f":" {p=1; next} p && /^[A-Za-z_][A-Za-z0-9_.]*:/ {exit} p' "$1"; }
if [[ -x $arm_cc ]]; then
  # (1) AArch64: reach=10 by default, and the trace must say so.
  env -u CCC_GLA_REACH CCC_DEBUG_SPLIT=1 CCC_RA_GLOBAL_LOCATION=1 CCC_GLA_TRACE=1 \
    "$arm_cc" "${arm_inc_flag[@]}" -O2 -S "$matmul" -o /dev/null \
    >"$tmp/arm-trace.txt" 2>&1
  grep -q "reach=10" "$tmp/arm-trace.txt" || {
    echo "FAIL: aarch64 default Speed reach band is not 10"; fail=1; }

  env -u CCC_GLA_REACH "$arm_cc" "${arm_inc_flag[@]}" -O2 -S "$matmul" -o "$tmp/mm10.s"
  env CCC_GLA_REACH=6 "$arm_cc" "${arm_inc_flag[@]}" -O2 -S "$matmul" -o "$tmp/mm6.s"
  # Current invariant on this tree (measured): matmul's prologue frame is 96
  # bytes at both bands, and the band does not change matmul's body. An earlier
  # pin expected 128 -> 112 from the band; that no longer holds, so the pin is
  # the measured state. A deliberate change must update this check.
  for mm in mm10 mm6; do
    frame=$(fn_body "$tmp/$mm.s" matmul | grep -oE 'stp[[:space:]]+x29, x30, \[sp, #-[0-9]+\]!' | grep -oE '#-[0-9]+' || true)
    [[ $frame == "#-96" ]] || {
      echo "FAIL: aarch64 matmul frame ($mm) is ${frame:-none}, pinned #-96"; fail=1; }
  done
  fn_body "$tmp/mm10.s" matmul >"$tmp/mm10.body"
  fn_body "$tmp/mm6.s" matmul >"$tmp/mm6.body"
  [[ -s $tmp/mm10.body ]] || { echo "FAIL: aarch64 matmul body not found"; fail=1; }
  cmp -s "$tmp/mm10.body" "$tmp/mm6.body" || {
    echo "FAIL: aarch64 reach band changed matmul's code (pinned: identical)"; fail=1; }
  [[ $fail -eq 0 ]] && echo "ok: aarch64 matmul frame 96 at bands 10 and 6, body identical"
else
  echo "SKIP: aarch64 cross compiler unavailable"
fi

# double_reduction.c uses only uint32_t, int64_t and printf. lccc's own include
# directory has no stddef.h, so the riscv leg compiles a copy whose three
# declarations are written out. Only declarations differ; the code and the
# trace lines the checks read are the same. The leg therefore needs no cross
# headers, and a missing cross gcc cannot turn it into a silent skip.
dred_nohdr="$tmp/double_reduction_nohdr.c"
sed -e 's|^#include <stdint.h>$|typedef unsigned int uint32_t; typedef long int64_t;|' \
    -e 's|^#include <stdio.h>$|int printf(const char *, ...);|' "$dred" >"$dred_nohdr"
if grep -q '^#include' "$dred_nohdr"; then
  echo "FAIL: double_reduction copy still has #include lines"; fail=1
fi
if [[ -x $rv_cc ]]; then
  # (2) RISC-V: reach=6 by default; the three bases are unreachable and the
  # function takes zero edits.
  env -u CCC_GLA_REACH CCC_DEBUG_SPLIT=1 CCC_RA_GLOBAL_LOCATION=1 CCC_GLA_TRACE=1 \
    "$rv_cc" -O2 -S "$dred_nohdr" -o "$tmp/dr6.s" \
    >"$tmp/rv6-trace.txt" 2>&1
  grep -q "reach=6" "$tmp/rv6-trace.txt" || {
    echo "FAIL: riscv64 default Speed reach band is not 6"; fail=1; }
  if grep -q "covers_reachable=true b0..b2" "$tmp/rv6-trace.txt"; then
    echo "FAIL: riscv64 double_reduction bases reachable at band 6 (hot-loop cascade; RA-GLA-03)"; fail=1
  fi
  if grep -q "\[GLA\] main applied: [1-9]" "$tmp/rv6-trace.txt"; then
    echo "FAIL: riscv64 double_reduction applied GLA edits at band 6"; fail=1
  fi

  # Forcing 11 reopens the documented cliff: three weighted-1 global bases
  # become reachable and are applied, and three large li pairs move into the
  # LCG loop.
  env CCC_DEBUG_SPLIT=1 CCC_RA_GLOBAL_LOCATION=1 CCC_GLA_REACH=11 CCC_GLA_TRACE=1 \
    "$rv_cc" -O2 -S "$dred_nohdr" -o "$tmp/dr11.s" \
    >"$tmp/rv11-trace.txt" 2>&1
  grep -q "applied: 3 values (3 remat" "$tmp/rv11-trace.txt" || {
    echo "FAIL: forced riscv64 band 11 no longer admits the 3 bases (RA-GLA-03 corpus changed - re-triage)"; fail=1; }
  echo "ok: riscv64 reach band 6 (band-11 hot-loop cliff pinned)"
else
  echo "SKIP: riscv64 cross compiler unavailable"
fi

# (3) x86-64 host derivation remains 6.
if [[ -x $x64_cc ]]; then
  env -u CCC_GLA_REACH CCC_DEBUG_SPLIT=1 CCC_RA_GLOBAL_LOCATION=1 CCC_GLA_TRACE=1 \
    "$x64_cc" -O2 -S "$nbody" -o /dev/null >"$tmp/x64-trace.txt" 2>&1
  grep -q "reach=6" "$tmp/x64-trace.txt" || {
    echo "FAIL: x86-64 default Speed reach band is not 6"; fail=1; }
  echo "ok: x86-64 reach band 6"
fi

if [[ $fail -ne 0 ]]; then exit 1; fi
echo "cross reach-band pins pass"
