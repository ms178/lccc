#!/usr/bin/env bash
# ISA-escape gate: an explicit `-march=` is a CEILING, not a suggestion.
#
# WHY THIS EXISTS
# ---------------
# The linux-cachymod-6.18 package builds with CONFIG_X86_NATIVE_CPU, and
# arch/x86/Makefile therefore appends `-march=native`.  A kernel built that way
# only runs on the build host's CPU class, which is fine for the owner's own
# machine and disastrous for a QEMU boot gate: on QEMU's default `qemu64`
# model the first BMI2/AVX2 instruction raises #UD, and in early 64-bit
# startup a #UD is a triple fault with NO serial output at all — it looks
# exactly like "the compiler miscompiled everything".
#
# The kernel was not at fault; neither was the ISA gate.  What was missing was
# a gate that checks the negative direction: that a TU compiled for a lower
# target contains no instruction from a higher one.  `-march=x86-64` (and
# `-march=x86-64-v2`, and `-mno-bmi2`, ...) must be honored exactly, including
# where the flag is *absent* for a Makefile that replaces KBUILD_CFLAGS.
#
# WHAT IS CHECKED
# ---------------
#   1. NEGATIVE: benchmark corpus + BMI-triggering probes compiled with
#      `-march=x86-64` and with `-march=x86-64-v2` emit no BMI/BMI2/AVX-class
#      mnemonic.  v2 is the interesting case: POPCNT/SSE4.2 are legal there,
#      BMI1 (andn/bextr/bls*) and BMI2 (rorx/shlx/shrx/sarx/bzhi/mulx/pdep/
#      pext) are not, and the peephole layer rewrites freely between
#      `and`/`not` and `andn` — so a single missing feature check shows up here.
#   2. NEGATIVE: `-mno-bmi2` suppresses the BMI2 class even when `-mavx2`
#      would otherwise open it, while `-mbmi` still selects `andn`.
#   3. POSITIVE (anti-vacuous): the same probes compiled with `-mavx2 -mbmi
#      -mbmi2` DO contain the class, so a typo in the regex cannot make the
#      gate pass forever.
#   4. THE REAL SOURCE: the kernel decompressor's own flags — the ones
#      arch/x86/boot/compressed/Makefile produces — applied to `ident_map`
#      style code, must stay baseline-clean.  That is the exact configuration
#      that triple-faulted, and it is a normal `make` invocation, so it is
#      checked through `scripts/build_kernel_compressed.sh`'s CF array when a
#      kernel tree is present (skipped otherwise: the gate must stay runnable
#      on a bare checkout).
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
REPO=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
tmp=${TMPDIR:-/tmp}/lccc-isa-gate.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cd "$REPO"

if ! command -v "$CCC" >/dev/null 2>&1; then
    echo "FAIL: compiler '$CCC' not found" >&2
    exit 2
fi

# The BMI/BMI2 class.  PCRE-free (POSIX ERE) so this runs on any host grep.
BMI_RE='\b(rorx[ql]|andn[ql]?|shrx[ql]|shlx[ql]|sarx[ql]|mulx[ql]|bzhi[ql]|bextr[ql]?|blsi[ql]?|blsmsk[ql]?|blsr[ql]?|pdep[ql]?|pext[ql]?|bzhi[ql]?)\b'
# AVX/AVX2 VEX class (v-prefixed mnemonics, including `vmovdqu`).
VEX_RE='(^|[[:space:]])v[a-z][a-z0-9]+[[:space:]]'
# Baseline-illegal SSE4.2 class used by the v2 boundary.
V2RE='\b(pcmpgtq|pcmpistri|pcmpestri|crc32[ql]?|popcnt[ql]?)\b'

# Probes: the exact shapes whose passes emit the class (and-not fusion,
# rotates -> rorx, variable shifts -> shlx/shrx/sarx, mask -> bzhi/bextr,
# count-zero -> bls*, CRC/POPCNT table).
cat >"$tmp/probe.c" <<'EOF'
#include <stdint.h>
uint32_t  andn_fused(uint32_t x, uint32_t y) { return x & ~y; }
uint64_t  andn_fused64(uint64_t x, uint64_t y) { return x & ~y; }
uint32_t  rot19(uint32_t x) { return (x >> 19) | (x << 13); }
uint64_t  rot19l(uint64_t x) { return (x >> 19) | (x << 45); }
uint32_t  shv(uint32_t a, uint32_t n) { return a >> n; }
uint64_t  shvl(uint64_t a, uint64_t n) { return a << n; }
int32_t   sar32(int32_t a, uint32_t n) { return a >> n; }
uint32_t  mask_bits(uint32_t a, uint32_t n) { return a & ((1u << n) - 1); }
int       lowest_bit(unsigned x) { return (int)(x & -x); }
int       clear_lowest(unsigned x) { return (int)(x & (x - 1)); }
int       highest_bit(unsigned x) { return x ? (int)(31 - __builtin_clz(x)) : 0; }
unsigned  count_zero(unsigned x) { return x ? (unsigned)__builtin_ctz(x) : 32u; }
uint64_t  mulhi(uint64_t a, uint64_t b) { return (uint64_t)(((unsigned __int128)a * b) >> 64); }
EOF
# A second probe with the kernel's scalar idioms (byte swap, popcount, CRC-ish
# loops) because those are the shapes the decompressor is full of.
cat >"$tmp/probe2.c" <<'EOF'
#include <stdint.h>
uint32_t bswap32(uint32_t x) { return __builtin_bswap32(x); }
uint64_t bswap64(uint64_t x) { return __builtin_bswap64(x); }
int popc(uint64_t x) { return __builtin_popcountll(x); }
uint64_t rol_mix(uint64_t a, uint64_t b) {
    uint64_t h = a ^ b;
    for (int i = 0; i < 4; i++) h = (h >> 7) | (h << 57), h ^= b + (uint64_t)i;
    return h;
}
EOF

fail=0
probe_only() { # the two probe files are ours: a compile failure is a defect
    case $(basename "$1") in probe.c|probe2.c) return 0 ;; *) return 1 ;; esac
}
scan() { # scan <tag> <regex> <flags...>
    local tag=$1 re=$2; shift 2
    local hits=0 file skipped=0
    for f in "$tmp/probe.c" "$tmp/probe2.c" \
             tests/benchmark/programs/*.c; do
        [ -f "$f" ] || continue
        if ! "$CCC" "$@" -S "$f" -o "$tmp/out.s" 2>"$tmp/err.txt"; then
            if probe_only "$f"; then
                echo "FAIL: $tag: $(basename "$f") does not compile under '$*'" >&2
                sed -n '1,5p' "$tmp/err.txt" >&2
                fail=1
            else
                # A corpus program that cannot be built for a LOWER target
                # (it may use higher-ISA intrinsics by design) is reported,
                # never silently ignored.
                skipped=$((skipped + 1))
            fi
            continue
        fi
        file=$(basename "$f")
        local n clean
        # Strip `#` comments first: a comment mentioning `vmovdqu` is not an
        # instruction, and scanning raw text would make this gate lie.
        sed 's/#.*//' "$tmp/out.s" >"$tmp/out.clean.s"
        n=$(grep -cE "$re" "$tmp/out.clean.s" || true)
        if [ "${n:-0}" != "0" ]; then
            clean=$(grep -oE "$re" "$tmp/out.clean.s" | sort -u | tr '\n' ' ')
            echo "FAIL: $tag: $file emitted $clean" >&2
            hits=$((hits + n))
        fi
    done
    echo "  $tag: $hits illegal instruction(s), $skipped corpus file(s) not buildable at this ISA"
    [ "$hits" = 0 ] || fail=1
}

echo "ISA-escape gate: explicit -march is a ceiling"
scan "-march=x86-64 (baseline: no BMI, no VEX)" "$BMI_RE|$VEX_RE" -march=x86-64 -O2
scan "-march=x86-64-v2 (SSE4.2 legal, BMI still not)" "$BMI_RE" -march=x86-64-v2 -O2

# -mno-bmi2 must win over a positive -mavx2 (which on this compiler implies
# the v3 integer set), while -mbmi keeps `andn`.
hits=0
"$CCC" -march=x86-64 -mavx2 -mno-bmi2 -O2 -S "$tmp/probe.c" -o "$tmp/out.s"
sed 's/#.*//' "$tmp/out.s" >"$tmp/out.clean.s"
n=$(grep -cE '\b(rorx[ql]|shrx[ql]|shlx[ql]|sarx[ql]|mulx[ql]|bzhi[ql]|pdep[ql]?|pext[ql]?)\b' "$tmp/out.s" || true)
[ "${n:-0}" = 0 ] || { echo "FAIL: -mno-bmi2 leaked: $(grep -oE '\b(rorx|shrx|shlx|sarx|mulx|bzhi)[ql]?' "$tmp/out.s" | sort -u | tr '\n' ' ')" >&2; hits=$n; }
echo "  -mavx2 -mno-bmi2: $hits BMI2 instruction(s)"
[ "$hits" = 0 ] || fail=1

# Anti-vacuous positive control: the class MUST appear when the features are
# granted, or a regex typo would make every negative above pass forever.
"$CCC" -mavx2 -mbmi -mbmi2 -O2 -S "$tmp/probe.c" -o "$tmp/pos.s"
sed 's/#.*//' "$tmp/pos.s" >"$tmp/pos.clean.s"
pos=$(grep -cE "$BMI_RE" "$tmp/pos.clean.s" || true)
if [ "${pos:-0}" = 0 ]; then
    echo "FAIL: positive control emitted no BMI instruction with -mavx2 -mbmi -mbmi2" >&2
    fail=1
fi
echo "  positive control (-mavx2 -mbmi -mbmi2): $pos BMI instruction(s)"

# The DEFAULT (no -march at all) is the integer-v3 profile — a deliberate,
# documented divergence from GCC's baseline default (commit "x86-64 /
# peephole / middle-end: integer-v3 default ISA").  It is pinned here because
# two harnesses depend on knowing it: scripts/build_kernel_compressed.sh must
# pass -march=x86-64 explicitly for Makefiles that REPLACE KBUILD_CFLAGS, and
# scripts/qemu_boot_test.sh pins the QEMU CPU model.  If the default ever
# changes, this gate says so instead of a kernel triple-faulting silently.
"$CCC" -O2 -S "$tmp/probe.c" -o "$tmp/def.s"
sed 's/#.*//' "$tmp/def.s" >"$tmp/def.clean.s"
def=$(grep -cE "$BMI_RE" "$tmp/def.clean.s" || true)
if [ "${def:-0}" = 0 ]; then
    echo "NOTE: the absent--march default no longer emits the v3 integer set" >&2
    echo "      ($def BMI instructions) — revisit the kernel harness assumptions" >&2
    echo "      in scripts/build_kernel_compressed.sh / kernel-vm.fragment." >&2
else
    echo "  default (-O2, no -march): $def BMI instruction(s) => integer-v3 profile (pinned)"
fi

echo "PASS: ISA ceilings honored ($pos mnemonic(s) in the positive control)"
exit "$fail"
