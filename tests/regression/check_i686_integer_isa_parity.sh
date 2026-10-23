#!/usr/bin/env bash
# Integer-ISA parity for the legacy F3-prefixed counting forms, x86-64 and
# i686, under EXPLICIT flags only (-march profiles or -m<feature>), with the
# -mno-* veto and the macro contract.
#
# Feature ownership follows GCC exactly:
#   TZCNT (F3 0F BC)  <- BMI1  (-mbmi)    ; without it: BSF (+ zero fixup)
#   LZCNT (F3 0F BD)  <- ABM   (-mlzcnt)  ; without it: BSR (+ zero fixup)
#   POPCNT            <- -mpopcnt         ; without it: bit loop
# The F3 forms never fault on CPUs lacking the feature: they execute as
# BSF/BSR and return wrong results (Ctz(0)/Clz(x) garbage). So TZCNT must not
# follow LZCNT: LZCNT-only CPUs (AMD K10, Bobcat, Jaguar) exist.
#
# __BMI__ stays undefined on i686 by design: the i686 backend consumes BMI1
# only for TZCNT (no ANDN/BLSR lowering, and the middle end's ANDN-dependent
# choices are x86-64-locked), so the macro would promise more than codegen
# serves. __LZCNT__/__POPCNT__ mirror codegen exactly.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC_ORACLE=${GCC_ORACLE:-$(command -v gcc-16 || true)}
tmp=${TMPDIR:-/tmp}/lccc-i686isa.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
fail() { echo "FAIL: $* (${BASH_SOURCE[0]##*/}:${BASH_LINENO[0]})" >&2; exit 1; }
cat >"$tmp/t.c" <<'C'
typedef unsigned int u32;
u32 ctz(u32 x) { return (u32)__builtin_ctz(x); }
u32 clz(u32 x) { return (u32)__builtin_clz(x); }
u32 pop(u32 x) { return (u32)__builtin_popcount(x); }
int main(void) { return (int)(ctz(8u) + clz(1u) + pop(7u)); }
C
# has <file.s> <regex>
has() { grep -qE "$2" "$1"; }
asm() { "$CCC" -O2 "$@" -S "$tmp/t.c" -o "$tmp/out.s"; }

# 1. All three granted on i686: every form is used and computes correctly.
asm -m32 -mbmi -mlzcnt -mpopcnt
has "$tmp/out.s" 'tzcntl' || fail "-m32 -mbmi: no tzcntl"
has "$tmp/out.s" 'lzcntl' || fail "-m32 -mlzcnt: no lzcntl"
has "$tmp/out.s" 'popcntl' || fail "-m32 -mpopcnt: no popcntl"
"$CCC" -O2 -m32 -mbmi -mlzcnt -mpopcnt "$tmp/t.c" -o "$tmp/on"
# Some developer hosts can *build* an i686 ELF but cannot run it (no 32-bit
# interpreter). Assembly, veto and macro checks continue; CI with an
# interpreter still requires the exit code.
interp=$(readelf -l "$tmp/on" | sed -n 's/.*Requesting program interpreter: \([^]]*\)].*/\1/p')
if [[ -n "$interp" && ! -e "$interp" ]]; then
    echo "SKIP-RUN: i686 binary requires missing $interp; assembly/macro checks continue"
else
    set +e
    "$tmp/on"
    st=$?
    set -e
    # ctz(8) + clz(1) + pop(7) == 3 + 31 + 3
    [ "$st" -eq 37 ] || fail "runtime result $st != 37"
fi

# 2. Ownership: LZCNT alone never grants TZCNT, BMI1 alone never grants LZCNT.
for m in "-m32" ""; do
    asm $m -mlzcnt
    has "$tmp/out.s" 'tzcnt' && fail "$m -mlzcnt emitted tzcnt (TZCNT is BMI1)"
    has "$tmp/out.s" 'lzcnt' || fail "$m -mlzcnt: no lzcnt"
    asm $m -mbmi
    has "$tmp/out.s" 'tzcnt' || fail "$m -mbmi: no tzcnt"
    has "$tmp/out.s" 'lzcnt' && fail "$m -mbmi emitted lzcnt (LZCNT is ABM)"
done

# 3. Baselines: broad-compatibility contract, no post-baseline integer form.
for m in "-m32" ""; do
    asm $m
    has "$tmp/out.s" 'tzcnt|lzcnt|popcnt' && fail "baseline ${m:-x86-64} emitted a post-baseline form"
done

# 4. -mno-* vetoes win over explicit grants and -march profiles, and each
#    veto removes only its own form.
asm -m32 -mbmi -mno-bmi
has "$tmp/out.s" 'tzcnt' && fail "-m32 -mbmi -mno-bmi emitted tzcnt"
asm -m32 -mlzcnt -mno-lzcnt
has "$tmp/out.s" 'lzcnt' && fail "-m32 -mlzcnt -mno-lzcnt emitted lzcnt"
asm -m32 -mpopcnt -mno-popcnt
has "$tmp/out.s" 'popcnt' && fail "-m32 -mpopcnt -mno-popcnt emitted popcnt"
asm -march=x86-64-v3
has "$tmp/out.s" 'tzcnt' && has "$tmp/out.s" 'lzcnt' && has "$tmp/out.s" 'popcnt' \
    || fail "-march=x86-64-v3 lacks a counting form"
asm -march=x86-64-v3 -mno-lzcnt
has "$tmp/out.s" 'lzcnt' && fail "v3 -mno-lzcnt emitted lzcnt"
has "$tmp/out.s" 'tzcnt' || fail "v3 -mno-lzcnt lost tzcnt (BMI1 is independent)"
asm -march=x86-64-v3 -mno-bmi
has "$tmp/out.s" 'tzcnt' && fail "v3 -mno-bmi emitted tzcnt"
has "$tmp/out.s" 'lzcnt' || fail "v3 -mno-bmi lost lzcnt (ABM is independent)"

# 5. Macro contract mirrors codegen.
printf '#ifdef __LZCNT__\nint lz_m;\n#endif\n#ifdef __POPCNT__\nint pc_m;\n#endif\n#ifdef __BMI__\nint bmi_m;\n#endif\n' >"$tmp/m.c"
"$CCC" -O2 -m32 -mbmi -mlzcnt -mpopcnt -S "$tmp/m.c" -o "$tmp/m1.s"
has "$tmp/m1.s" 'lz_m' && has "$tmp/m1.s" 'pc_m' || fail "i686 __LZCNT__/__POPCNT__ missing"
has "$tmp/m1.s" 'bmi_m' && fail "i686 __BMI__ defined without ANDN/BLSR codegen"
"$CCC" -O2 -m32 -S "$tmp/m.c" -o "$tmp/m2.s"
has "$tmp/m2.s" 'lz_m|pc_m|bmi_m' && fail "baseline -m32 defines an integer-ISA macro"
"$CCC" -O2 -mbmi -S "$tmp/m.c" -o "$tmp/m3.s"
has "$tmp/m3.s" 'bmi_m' || fail "x86-64 -mbmi lacks __BMI__"

# 6. GCC oracle: the Ctz/Clz instruction CLASS per flag set must match GCC
#    (GCC's `rep bsf` is the BSF class: F3 0F BC executes as BSF without BMI1).
if [[ -n "$GCC_ORACLE" ]]; then
    class() { # <asm> -> "tz|bsf lz|bsr"
        local tz=bsf lz=bsr
        grep -qE '^\s*tzcnt' "$1" && tz=tz
        grep -qE '^\s*lzcnt' "$1" && lz=lz
        echo "$tz $lz"
    }
    checked=0
    for flags in "-m32 -mlzcnt" "-m32 -mbmi" "-m32 -mbmi -mlzcnt" "-mlzcnt" "-mbmi" \
                 "-march=x86-64-v3 -mno-bmi" "-march=x86-64-v3 -mno-lzcnt" "-march=x86-64-v3"; do
        # A host GCC without -m32 support cannot judge the i686 rows.
        "$GCC_ORACLE" -O2 $flags -S "$tmp/t.c" -o "$tmp/g.s" 2>/dev/null || continue
        asm $flags
        [ "$(class "$tmp/g.s")" = "$(class "$tmp/out.s")" ] \
            || fail "oracle: '$flags' GCC=$(class "$tmp/g.s") LCCC=$(class "$tmp/out.s")"
        checked=$((checked + 1))
    done
    echo "oracle: $checked flag sets match $("$GCC_ORACLE" -dumpfullversion)"
else
    echo "SKIP-ORACLE: no gcc-16 (set GCC_ORACLE) for the instruction-class cross-check"
fi

echo "i686 integer-ISA parity gate: PASS"
