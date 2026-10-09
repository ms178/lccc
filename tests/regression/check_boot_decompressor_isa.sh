#!/usr/bin/env bash
# Boot-decompressor ISA gate (scripts/build_kernel_compressed.sh).
#
# The compressed-kernel decompressor runs before the kernel probes the CPU, so
# it must not use BMI/BMI2/LZCNT/POPCNT/MOVBE. lccc's default ISA is
# x86-64-v3, which would emit e.g. SHRX into misc.o. The build script denies
# those per translation unit through BOOT_NO_ISA; this gate reads that list
# from the script (so it cannot drift) and checks that the denial holds:
#   * control: with -march=x86-64-v3 the snippet does lower to andnq, the same
#     construct check_andn_fusion.sh relies on, so the negatives are meaningful;
#   * the denial holds against an explicit -march=x86-64-v3 (sticky denial);
#   * the denial holds under the default ISA, the configuration the build uses.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
root=$(cd "$(dirname "$0")/../.." && pwd)
tmp=${TMPDIR:-/tmp}/lccc-boot-isa.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

deny_line=$(sed -n 's/^BOOT_NO_ISA=(\(.*\))$/\1/p' "$root/scripts/build_kernel_compressed.sh")
read -r -a deny <<<"$deny_line"
if [ "${#deny[@]}" -lt 5 ]; then
    echo "FAIL: BOOT_NO_ISA not found in scripts/build_kernel_compressed.sh" >&2
    exit 1
fi

cat >"$tmp/t.c" <<'C'
__attribute__((noinline)) unsigned long andn_op(unsigned long a, unsigned long b) { return a & ~b; }
__attribute__((noinline)) unsigned long shr_var(unsigned long a, unsigned int s) { return a >> s; }
__attribute__((noinline)) unsigned int ctz_op(unsigned long a) { return a ? __builtin_ctzl(a) : 64; }
__attribute__((noinline)) unsigned int pop_op(unsigned long a) { return __builtin_popcountl(a); }
C

bmi_re='^\s*(andn|shrx|sarx|shlx|bzhi|rorx|pdep|pext|mulx|blsr|blsi|blsmsk|bextr|tzcnt|lzcnt|popcnt|movbe)[lqwb]?\s'
fail=0

"$CCC" -O2 -march=x86-64-v3 -S "$tmp/t.c" -o "$tmp/control.s"
if ! grep -Eq '^\s*andnq\s' "$tmp/control.s"; then
    echo "FAIL: control -march=x86-64-v3 does not emit andnq; gate is vacuous" >&2
    fail=1
fi

"$CCC" -O2 -march=x86-64-v3 -mno-sse "${deny[@]}" -S "$tmp/t.c" -o "$tmp/v3_denied.s"
if grep -Eq "$bmi_re" "$tmp/v3_denied.s"; then
    echo "FAIL: BOOT_NO_ISA does not hold against -march=x86-64-v3:" >&2
    grep -E "$bmi_re" "$tmp/v3_denied.s" >&2
    fail=1
fi

"$CCC" -O2 -mno-sse "${deny[@]}" -S "$tmp/t.c" -o "$tmp/default_denied.s"
if grep -Eq "$bmi_re" "$tmp/default_denied.s"; then
    echo "FAIL: BOOT_NO_ISA does not hold under the default ISA:" >&2
    grep -E "$bmi_re" "$tmp/default_denied.s" >&2
    fail=1
fi

exit "$fail"
