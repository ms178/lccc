#!/usr/bin/env bash
# ISA contract of the kernel boot path — self-test for scripts/kernel_boot_isa.sh.
#
# WHY THIS EXISTS
# ---------------
# arch/x86/boot/compressed/Makefile and drivers/firmware/efi/libstub/Makefile
# REPLACE KBUILD_CFLAGS, which discards the `-march=` that arch/x86/Makefile
# derives from CONFIG_GENERIC_CPU/X86_64_VERSION.  GCC/Clang/ICC default to the
# x86-64 baseline when `-march` is absent; LCCC's documented default ISA is the
# integer-v3 profile, so the same Makefile silently built a BMI2 decompressor:
# `shrxq` in kernel_ident_mapping_init is #UD on QEMU's `qemu64`, and a #UD in
# early 64-bit startup is a triple fault with no serial output at all.
#
# The fix has two halves and BOTH are tested here, because either half alone
# can rot silently:
#   1. `patch`  — the two flag-wiping Makefiles name the x86-64 baseline ISA,
#                 idempotently, and refuse to guess when the file is refactored.
#   2. `scan`   — the BUILT boot/decompressor objects contain no BMI/ABM, VEX or
#                 SSE4.2-class instruction.  That is the measured half.
#
# WHAT IS CHECKED
# ---------------
#   A. pristine fixtures -> patch rewrites exactly the x86-64 line, leaves the
#      32-bit line and every other byte untouched, and is idempotent.
#   B. a reverted line -> `check` FAILS (the gate can fail, it is not vacuous),
#      and a second `patch` repairs it.
#   C. refactored fixtures (32-bit reference line gone; x86-64 line gone) ->
#      `patch` FAILS LOUDLY and does not modify the file.
#   D. `scan` on a real object with BMI2 (`-mbmi2`) and on a real object with
#      VEX (`-mavx2 -mno-bmi2`, so the hit cannot come from the integer set)
#      FAILS and names the instruction; on a baseline object it PASSES; a
#      directory argument is honored; a missing path and an empty directory
#      both fail.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
REPO=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
SCRIPT=$REPO/scripts/kernel_boot_isa.sh
tmp=${TMPDIR:-/tmp}/lccc-boot-isa.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cd "$REPO"

fail=0
[[ -f $SCRIPT ]] || { echo "FAIL: $SCRIPT missing" >&2; exit 2; }
if ! command -v "$CCC" >/dev/null 2>&1; then
    echo "FAIL: compiler '$CCC' not found" >&2
    exit 2
fi

# --- fixtures ----------------------------------------------------------------
K=$tmp/k
mkdir -p "$K/arch/x86/boot/compressed" "$K/drivers/firmware/efi/libstub"

cat > "$K/arch/x86/boot/compressed/Makefile" <<'EOF'
# SPDX-License-Identifier: GPL-2.0
KBUILD_CFLAGS := -m$(BITS) -O2 $(CLANG_FLAGS)
KBUILD_CFLAGS += -std=gnu18
cflags-$(CONFIG_X86_32) := -march=i386
cflags-$(CONFIG_X86_64) := -mcmodel=small -mno-red-zone
KBUILD_CFLAGS += $(cflags-y)
EOF
# The EFI-stub file separates its assignments with tabs, exactly as upstream.
{
    printf 'cflags-y\t\t\t:= $(KBUILD_CFLAGS)\n'
    printf '\n'
    printf 'cflags-$(CONFIG_X86_32)\t\t:= -march=i386\n'
    printf 'cflags-$(CONFIG_X86_64)\t\t:= -mcmodel=small\n'
} > "$K/drivers/firmware/efi/libstub/Makefile"

cp "$K/arch/x86/boot/compressed/Makefile" "$tmp/compressed.pristine"
cp "$K/drivers/firmware/efi/libstub/Makefile" "$tmp/efi.pristine"

# --- A. patch ----------------------------------------------------------------
echo "== patch (pristine fixture tree) =="
if ! bash "$SCRIPT" patch "$K" > "$tmp/patch1.log" 2>&1; then
    echo "FAIL: patch exited non-zero on a pristine tree:" >&2
    sed 's/^/  /' "$tmp/patch1.log" >&2
    fail=1
fi
grep -c -- '-march=x86-64' "$K/arch/x86/boot/compressed/Makefile" >/dev/null || {
    echo "FAIL: decompressor line was not made explicit" >&2
    fail=1
}
grep -c -- '-march=x86-64 -mcmodel=small$' "$K/drivers/firmware/efi/libstub/Makefile" >/dev/null || {
    echo "FAIL: EFI stub line was not made explicit" >&2
    fail=1
}
# The 32-bit branch and every unrelated byte must survive the rewrite.
if ! cmp -s <(sed 's/-march=x86-64 -mcmodel=small -mno-red-zone/-mcmodel=small -mno-red-zone/' \
                "$K/arch/x86/boot/compressed/Makefile") "$tmp/compressed.pristine"; then
    echo "FAIL: patch changed more than the x86-64 line (decompressor)" >&2
    diff -u "$tmp/compressed.pristine" "$K/arch/x86/boot/compressed/Makefile" >&2 || true
    fail=1
fi
if ! cmp -s <(sed 's/\t\t:= -march=x86-64 -mcmodel=small$/\t\t:= -mcmodel=small/' \
                "$K/drivers/firmware/efi/libstub/Makefile") "$tmp/efi.pristine"; then
    echo "FAIL: patch changed more than the x86-64 line (EFI stub)" >&2
    diff -u "$tmp/efi.pristine" "$K/drivers/firmware/efi/libstub/Makefile" >&2 || true
    fail=1
fi

echo "== patch is idempotent =="
cp "$K/arch/x86/boot/compressed/Makefile" "$tmp/compressed.patched"
cp "$K/drivers/firmware/efi/libstub/Makefile" "$tmp/efi.patched"
if ! bash "$SCRIPT" patch "$K" > "$tmp/patch2.log" 2>&1; then
    echo "FAIL: second patch run exited non-zero" >&2
    sed 's/^/  /' "$tmp/patch2.log" >&2
    fail=1
fi
cmp -s "$tmp/compressed.patched" "$K/arch/x86/boot/compressed/Makefile" || {
    echo "FAIL: second patch run changed the decompressor file" >&2
    fail=1
}
cmp -s "$tmp/efi.patched" "$K/drivers/firmware/efi/libstub/Makefile" || {
    echo "FAIL: second patch run changed the EFI stub file" >&2
    fail=1
}
grep -c 'already explicit' "$tmp/patch2.log" >/dev/null || {
    echo "FAIL: second patch run did not report the fixture as already explicit" >&2
    fail=1
}

echo "== check passes on a patched tree =="
if ! bash "$SCRIPT" check "$K" > "$tmp/check1.log" 2>&1; then
    echo "FAIL: check exited non-zero on a patched tree" >&2
    sed 's/^/  /' "$tmp/check1.log" >&2
    fail=1
fi

# --- B. a reverted line must fail the check ---------------------------------
echo "== NEGATIVE CONTROL: check fails on a reverted tree =="
cp "$tmp/compressed.pristine" "$K/arch/x86/boot/compressed/Makefile"
if bash "$SCRIPT" check "$K" > "$tmp/check2.log" 2>&1; then
    echo "FAIL: check passed with the decompressor line reverted (vacuous gate)" >&2
    fail=1
fi
bash "$SCRIPT" patch "$K" > /dev/null 2>&1 || { echo "FAIL: repair patch failed" >&2; fail=1; }
bash "$SCRIPT" check "$K" > /dev/null 2>&1 || { echo "FAIL: check failed after repair" >&2; fail=1; }

# --- C. refactored fixtures must fail LOUDLY ---------------------------------
echo "== refactor: the 32-bit reference line is gone =="
mkdir -p "$tmp/refactor1/arch/x86/boot/compressed" "$tmp/refactor1/drivers/firmware/efi/libstub"
sed 's/^cflags-\$(CONFIG_X86_32) := -march=i386$/# x86-32 flags moved elsewhere/' \
    "$tmp/compressed.pristine" > "$tmp/refactor1/arch/x86/boot/compressed/Makefile"
cp "$tmp/efi.patched" "$tmp/refactor1/drivers/firmware/efi/libstub/Makefile"
cp "$tmp/refactor1/arch/x86/boot/compressed/Makefile" "$tmp/refactor1.before"
if bash "$SCRIPT" patch "$tmp/refactor1" > "$tmp/refactor1.log" 2>&1; then
    echo "FAIL: patch accepted a tree whose 32-bit reference line was removed" >&2
    fail=1
fi
cmp -s "$tmp/refactor1.before" "$tmp/refactor1/arch/x86/boot/compressed/Makefile" || {
    echo "FAIL: patch modified a file it could not reason about" >&2
    fail=1
}

echo "== refactor: neither pristine nor patched x86-64 line =="
mkdir -p "$tmp/refactor2/arch/x86/boot/compressed" "$tmp/refactor2/drivers/firmware/efi/libstub"
sed 's/^cflags-\$(CONFIG_X86_64) := .*/cflags-$(CONFIG_X86_64) := -mcmodel=large/' \
    "$tmp/compressed.pristine" > "$tmp/refactor2/arch/x86/boot/compressed/Makefile"
cp "$tmp/efi.patched" "$tmp/refactor2/drivers/firmware/efi/libstub/Makefile"
cp "$tmp/refactor2/arch/x86/boot/compressed/Makefile" "$tmp/refactor2.before"
if bash "$SCRIPT" patch "$tmp/refactor2" > "$tmp/refactor2.log" 2>&1; then
    echo "FAIL: patch guessed at an unknown x86-64 line" >&2
    fail=1
fi
cmp -s "$tmp/refactor2.before" "$tmp/refactor2/arch/x86/boot/compressed/Makefile" || {
    echo "FAIL: patch modified a file whose x86-64 line it did not recognize" >&2
    fail=1
}

# --- D. scan: real objects, both directions ----------------------------------
echo "== scan: baseline object passes, BMI/ABM/VEX objects fail =="
# Probe bodies are the ISA gate's proven BMI/BMI2 triggers (check_isa_gate_
# escape.sh), plus a float add for the VEX control: `x & ~y` -> andn,
# variable shifts -> shlx/shrx/sarx, bit masks -> bzhi, a float op -> VEX.
cat > "$tmp/probe.c" <<'EOF'
#include <stdint.h>
uint32_t andn_fused(uint32_t x, uint32_t y) { return x & ~y; }
uint64_t andn_fused64(uint64_t x, uint64_t y) { return x & ~y; }
uint32_t shv(uint32_t a, uint32_t n) { return a >> n; }
uint64_t shvl(uint64_t a, uint64_t n) { return a << n; }
uint32_t mask_bits(uint32_t a, uint32_t n) { return a & ((1u << n) - 1); }
uint64_t mulhi(uint64_t a, uint64_t b) { return (uint64_t)(((unsigned __int128)a * b) >> 64); }
float vadd(float *restrict a, float *restrict b, int i) { return a[i] + b[i]; }
EOF
"$CCC" -O2 -march=x86-64 -c "$tmp/probe.c" -o "$tmp/base.o"
if ! bash "$SCRIPT" scan "$tmp/base.o" > "$tmp/scan_base.log" 2>&1; then
    echo "FAIL: scan rejected a baseline object" >&2
    sed 's/^/  /' "$tmp/scan_base.log" >&2
    fail=1
fi
# The project's documented default ISA profile: BMI1+BMI2 granted, so the
# integer probes must produce the class and a baseline-claiming boot path
# must never see it.
"$CCC" -O2 -march=x86-64-v3 -c "$tmp/probe.c" -o "$tmp/bmi.o"
if bash "$SCRIPT" scan "$tmp/bmi.o" > "$tmp/scan_bmi.log" 2>&1; then
    echo "FAIL: scan accepted an object with BMI/ABM instructions (vacuous scan)" >&2
    fail=1
fi
grep -cE '\b(andn[ql]?|rorx[ql]|sh[lr]x[ql]|sarx[ql]|mulx[ql]|bzhi[ql]|blsr[ql]?|tzcnt[ql]?|lzcnt[ql]?)\b' "$tmp/scan_bmi.log" >/dev/null || {
    echo "FAIL: scan failed but named no BMI/ABM mnemonic" >&2
    sed 's/^/  /' "$tmp/scan_bmi.log" >&2
    fail=1
}
"$CCC" -O2 -mavx2 -mno-bmi -mno-bmi2 -c "$tmp/probe.c" -o "$tmp/vex.o"
if bash "$SCRIPT" scan "$tmp/vex.o" > "$tmp/scan_vex.log" 2>&1; then
    echo "FAIL: scan accepted an object with VEX instructions" >&2
    fail=1
fi
grep -cE '(^|[[:space:]])v[a-z][a-z0-9]+[[:space:]]' "$tmp/scan_vex.log" >/dev/null || {
    echo "FAIL: scan failed but named no VEX mnemonic" >&2
    sed 's/^/  /' "$tmp/scan_vex.log" >&2
    fail=1
}
echo "== scan: directory argument, empty directory, missing path =="
mkdir -p "$tmp/objdir"
cp "$tmp/base.o" "$tmp/objdir/a.o"
if ! bash "$SCRIPT" scan "$tmp/objdir" > "$tmp/scan_dir.log" 2>&1; then
    echo "FAIL: scan rejected a clean object directory" >&2
    sed 's/^/  /' "$tmp/scan_dir.log" >&2
    fail=1
fi
cp "$tmp/bmi.o" "$tmp/objdir/b.o"
if bash "$SCRIPT" scan "$tmp/objdir" > /dev/null 2>&1; then
    echo "FAIL: scan accepted a directory containing a BMI/ABM object" >&2
    fail=1
fi
mkdir -p "$tmp/emptydir"
if bash "$SCRIPT" scan "$tmp/emptydir" > /dev/null 2>&1; then
    echo "FAIL: scan accepted an empty directory (nothing was audited)" >&2
    fail=1
fi
if bash "$SCRIPT" scan "$tmp/does-not-exist" > /dev/null 2>&1; then
    echo "FAIL: scan accepted a missing path" >&2
    fail=1
fi

echo "== scan: ZSTD DYNAMIC_BMI2 target-attributed clone vs baseline leak =="
cat > "$tmp/zstd_dyn.c" <<'EOF'
#include <stdint.h>
__attribute__((__target__("lzcnt,bmi,bmi2")))
uint32_t FSE_readNCount_body_bmi2(uint32_t a, uint32_t n) {
    return (a << n) ^ (uint32_t)__builtin_clz(a | 1u);
}
uint32_t FSE_readNCount_body_default(uint32_t a, uint32_t n) {
    return a + n;
}
EOF
"$CCC" -O2 -march=x86-64 -c "$tmp/zstd_dyn.c" -o "$tmp/zstd_dyn_ok.o"
# Anti-vacuity control: the symbol allowlist earns its keep only if the
# target-attributed clone ACTUALLY emits an lzcnt/bmi/bmi2 mnemonic. If
# per-function target handling regressed to baseline codegen the object
# would contain zero allowed instructions and the scan would still pass,
# proving nothing. Pin the signal before asking the scanner to accept it.
bmi2_mnemonics=$(objdump -d --no-show-raw-insn "$tmp/zstd_dyn_ok.o" \
    | awk '
        /^[0-9a-fA-F]+[[:space:]]+<[^>]+>:/ {
            s = $0
            sub(/^[^<]*</, "", s)
            sub(/>:.*$/, "", s)
            in_bmi2 = (s ~ /^FSE_readNCount_body_bmi2(\.[A-Za-z0-9_.]+)?$/)
            next
        }
        in_bmi2 { sub(/#.*/, ""); print }' \
    | grep -cE '(^|[[:space:]])(rorx[ql]?|andn[ql]?|shrx[ql]?|shlx[ql]?|sarx[ql]?|mulx[ql]?|bzhi[ql]?|bextr[ql]?|blsi[ql]?|blsmsk[ql]?|blsr[ql]?|pdep[ql]?|pext[ql]?|tzcnt[ql]?|lzcnt[ql]?)([[:space:]]|$)' || true)
if [ "$bmi2_mnemonics" -lt 1 ]; then
    echo "FAIL: FSE_readNCount_body_bmi2 contains no DYNAMIC_BMI2 mnemonic (per-function target attribute regressed?)" >&2
    objdump -d --no-show-raw-insn "$tmp/zstd_dyn_ok.o" >&2
    fail=1
fi
if ! bash "$SCRIPT" scan "$tmp/zstd_dyn_ok.o" > "$tmp/scan_zstd_ok.log" 2>&1; then
    echo "FAIL: scan rejected ZSTD DYNAMIC_BMI2 target-attributed clone" >&2
    sed 's/^/  /' "$tmp/scan_zstd_ok.log" >&2
    fail=1
fi
if grep -qi 'warning:' "$tmp/scan_zstd_ok.log"; then
    echo "FAIL: successful ISA scan produced a warning" >&2
    cat "$tmp/scan_zstd_ok.log" >&2
    exit 1
fi
# A non-dot suffix must never acquire the exemption. GNU awk -v used
# to consume the regex's backslash, turning the literal dot into a wildcard.
sed 's/FSE_readNCount_body_bmi2/FSE_readNCount_body_bmi2Xsuffix/g' \
    "$tmp/zstd_dyn.c" > "$tmp/zstd_bad_suffix.c"
"$CCC" -O2 -march=x86-64 -c "$tmp/zstd_bad_suffix.c" -o "$tmp/zstd_bad_suffix.o"
if bash "$SCRIPT" scan "$tmp/zstd_bad_suffix.o" > /dev/null 2>&1; then
    echo "FAIL: scan granted an ISA exemption to a non-dot clone suffix" >&2
    exit 1
fi
cat > "$tmp/zstd_leak.c" <<'EOF'
#include <stdint.h>
__attribute__((__target__("lzcnt,bmi,bmi2")))
uint32_t FSE_readNCount_body_bmi2(uint32_t a, uint32_t n) { return a & ~n; }
uint32_t FSE_readNCount_body_default(uint32_t a, uint32_t n) { return a & ~n; }
EOF
"$CCC" -O2 -march=x86-64-v3 -c "$tmp/zstd_leak.c" -o "$tmp/zstd_leak.o"
if bash "$SCRIPT" scan "$tmp/zstd_leak.o" > /dev/null 2>&1; then
    echo "FAIL: scan accepted BMI2 leaked into FSE_readNCount_body_default" >&2
    fail=1
fi

if [ "$fail" = 0 ]; then
    echo "PASS: boot-path ISA contract (patch, idempotence, refactor refusal, artifact scan)"
fi
exit "$fail"
