#!/usr/bin/env bash
# Emission + semantics gate for a 32-bit compare against a constant whose top
# bit is set.
#
# `cmp r32, imm32` takes the constant's 32 bits verbatim: `4294967295` IS
# `$-1`, `2654435761` IS `$-1640531535`. LCCC staged such constants through a
# register instead (`movl $4294967295, %ecx; cmpl %ecx, %edi`), one instruction
# more than GCC and Clang spend, on every compare against a hash constant with
# the top bit set -- the golden-ratio constant is the one everybody writes.
#
# The control is the 64-bit compare, whose only immediate is a SIGN-EXTENDED
# imm32: the same constant there must still stage, so a fix that "just allowed
# the value" would break 64-bit compares and fail here. The 64-bit-pattern
# constant (`0x123456789ab`) is the second control: no immediate fits, at any
# width.
#
# Every probe is also EXECUTED: an encoding change that silently compared the
# wrong bits would pass an emission check.
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
[[ -x $ccc ]] || { echo "check_cmp_imm32_pattern: no lccc at $ccc" >&2; exit 1; }
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-cmpimm32.XXXXXX")
trap 'rm -rf "$work"' EXIT

fail=0
check_eq() { # <desc> <actual> <expected>
    if [[ $2 != "$3" ]]; then
        echo "FAIL: $1 -- got '$2', want '$3'" >&2
        fail=1
    fi
}
# <desc> <file> <egrep-pattern that must NOT appear>
check_absent() {
    if grep -qE -- "$3" "$2"; then
        echo "FAIL: $1 -- forbidden pattern '$3' matched:" >&2
        grep -nE -- "$3" "$2" | sed -n '1,3p' >&2
        fail=1
    fi
}
# <desc> <file> <egrep-pattern that must appear>
check_present() {
    if ! grep -qE -- "$3" "$2"; then
        echo "FAIL: $1 -- required pattern '$3' not found" >&2
        fail=1
    fi
}
body() { # <function> <asm file> -> that function's body
    sed -n "/^$1:/,/^\.size/p" "$2"
}

cat > "$work/imm32.c" <<'EOF'
#include <stdint.h>
#define NOINLINE __attribute__((noinline))
NOINLINE int eq_max(unsigned x) { return x == 4294967295u; }
NOINLINE int ne_max(unsigned x) { return x != 4294967295u; }
NOINLINE int ge_max(unsigned x) { return x >= 4294967295u; }
NOINLINE int lt_high(unsigned x) { return x < 2147483648u; }
NOINLINE int eq_hash(unsigned x) { return x == 2654435761u; }
NOINLINE int lt_hash(unsigned x) { return x < 2654435761u; }
NOINLINE int branch(unsigned x) {
    if (x == 3000000000u) return 7;
    if (x < 2654435761u) return 3;
    return 9;
}
/* 64-bit controls: imm32 sign-extends, so these must still stage. */
NOINLINE int eq64_wide(unsigned long long x) { return x == 4294967295ull; }
NOINLINE int eq64_pattern(unsigned long long x) { return x == 0x123456789abull; }
NOINLINE int branch64(unsigned long long x) { return x == 0x123456789abull ? 1 : 0; }
int main(void) {
    static const unsigned U[] = {
        0u, 1u, 2147483647u, 2147483648u, 2654435760u, 2654435761u,
        3000000000u, 4294967294u, 4294967295u,
    };
    static const unsigned long long Q[] = {
        0ull, 4294967295ull, 0x123456789aaull, 0x123456789abull, 0x123456789acull,
    };
    unsigned long long h = 0;
    for (unsigned i = 0; i < sizeof U / sizeof *U; ++i) {
        unsigned x = U[i];
        h = h * 131 + (unsigned)eq_max(x) + (unsigned)ne_max(x) + (unsigned)ge_max(x);
        h = h * 131 + (unsigned)lt_high(x) + (unsigned)eq_hash(x) + (unsigned)lt_hash(x);
        h = h * 131 + (unsigned)branch(x);
    }
    for (unsigned i = 0; i < sizeof Q / sizeof *Q; ++i) {
        unsigned long long x = Q[i];
        h = h * 131 + (unsigned)eq64_wide(x) + (unsigned)eq64_pattern(x) + (unsigned)branch64(x);
    }
    /* 32-bit: h contributes its low 32 bits only, so keep the check narrow. */
    return h == 0 ? 1 : 0;
}
EOF

"$ccc" -O2 -S "$work/imm32.c" -o "$work/imm32.s"

# The value forms: one compare against the sign-extended spelling of the same
# 32 bits, and no register staging of the constant.  (`-O0` is excluded by
# construction: unoptimized code keeps every constant in a stack slot, so the
# immediate there is a store, not a compare operand.)
body eq_max "$work/imm32.s" > "$work/eq_max.s"
check_absent "eq_max: no register shuttle" "$work/eq_max.s" 'mov[lq]? +\$[0-9]{9,},'
check_present "eq_max: immediate is the imm32 pattern" "$work/eq_max.s" 'cmpl +\$-1, %e[a-z]+'
body ne_max "$work/imm32.s" > "$work/ne_max.s"
check_absent "ne_max: no register shuttle" "$work/ne_max.s" 'mov[lq]? +\$[0-9]{9,},'
check_present "ne_max: immediate is the imm32 pattern" "$work/ne_max.s" 'cmpl +\$-1, %e[a-z]+'
body ge_max "$work/imm32.s" > "$work/ge_max.s"
check_present "ge_max: immediate is the imm32 pattern" "$work/ge_max.s" 'cmpl +\$-1, %e[a-z]+'
body lt_high "$work/imm32.s" > "$work/lt_high.s"
check_present "lt_high: 0x80000000 is imm32" "$work/lt_high.s" 'cmpl +\$-2147483648, %e[a-z]+'
body eq_hash "$work/imm32.s" > "$work/eq_hash.s"
check_present "eq_hash: the golden-ratio constant is imm32" "$work/eq_hash.s" \
    'cmpl +\$-1640531535, %e[a-z]+'
body lt_hash "$work/imm32.s" > "$work/lt_hash.s"
check_present "lt_hash: the golden-ratio constant is imm32" "$work/lt_hash.s" \
    'cmpl +\$-1640531535, %e[a-z]+'
# The branch form is a different lowering; it must not stage either.  Its
# immediate is spelled as the raw u32 here (3000000000), which the encoder takes
# verbatim; what must not appear is the register.
body branch "$work/imm32.s" > "$work/branch.s"
check_absent "branch: no register shuttle" "$work/branch.s" 'mov[lq]? +\$(3000000000|2654435761|4294967295),'
check_present "branch: first constant is an immediate compare" "$work/branch.s" \
    'cmpl +\$(3000000000|-1294967296), %e[a-z]+'
check_present "branch: second constant is an immediate compare" "$work/branch.s" \
    'cmpl +\$(2654435761|-1640531535), %e[a-z]+'

# The controls.  A 64-bit compare has only a SIGN-EXTENDED imm32, so the same
# number must not become `cmpq $-1` (which would compare against
# 0xffffffffffffffff): it stages, and LCCC stages it with a 32-bit `movl`,
# which zero-extends -- five bytes, not ten.
body eq64_wide "$work/imm32.s" > "$work/eq64_wide.s"
check_present "eq64_wide: stages rather than building -1" "$work/eq64_wide.s" \
    'mov[lq]? +\$4294967295, %e?[a-z]+'
check_present "eq64_wide: compares in registers" "$work/eq64_wide.s" 'cmpq +%r[a-z]+, %r[a-z]+'
check_absent "eq64_wide: never compares against sign-extended -1" "$work/eq64_wide.s" 'cmpq +\$-1,'
body eq64_pattern "$work/imm32.s" > "$work/eq64_pattern.s"
check_present "eq64_pattern: 64-bit pattern stages" "$work/eq64_pattern.s" 'movabsq +\$1250999896491,'
body branch64 "$work/imm32.s" > "$work/branch64.s"
check_present "branch64: 64-bit pattern stages" "$work/branch64.s" 'movabsq +\$1250999896491,'

# Semantics: execute every probe, then re-check the shape at three more levels
# (the emission path differs by level; the encoding rule does not).
"$ccc" -O2 "$work/imm32.c" -o "$work/imm32.bin" && "$work/imm32.bin"
check_eq "imm32 compare: execution" "$?" "0"
for opt in -O1 -O3 -Os; do
    "$ccc" "$opt" -S "$work/imm32.c" -o "$work/imm32$opt.s"
    body eq_max "$work/imm32$opt.s" > "$work/eq_max$opt.s"
    check_present "eq_max at $opt: imm32 pattern" "$work/eq_max$opt.s" 'cmpl +\$-1, %e[a-z]+'
    body eq_hash "$work/imm32$opt.s" > "$work/eq_hash$opt.s"
    check_present "eq_hash at $opt: imm32 pattern" "$work/eq_hash$opt.s" \
        'cmpl +\$-1640531535, %e[a-z]+'
done

if [[ $fail -ne 0 ]]; then
    echo "check_cmp_imm32_pattern: FAILED" >&2
    exit 1
fi
echo "check_cmp_imm32_pattern: OK (32-bit constants with the top bit set encode as imm32; 64-bit controls still stage)"
