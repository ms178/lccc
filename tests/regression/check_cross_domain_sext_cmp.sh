#!/usr/bin/env bash
# Emission + semantics gate for the cross-domain comparison rewrite
# (simplify's `cross_domain_sign_test`, under the `cmprange` knob).
#
#   fire-shape   `(unsigned)(signed char)x < 300` — the unsigned reading of a
#                sign extension is two halves, and 300 sits between them, so
#                the answer is the sign bit.  Must be a sign test, not a
#                compare against 300, on every backend.
#   keep-shape   a constant INSIDE a half (`< 5` on a byte, `< 300` on a short)
#                or one step past the halves' boundary (`>= 0xffffff81`,
#                `>= 2^64 - 2^31 + 1`): the answer depends on the value, so the
#                compare against the constant must survive untouched.
#   width-shape  `(unsigned long long)(int)x >= 2^64 - 2^31` — the same rule at
#                a 64-bit compare width, where the halves are 2^31 apart.
#
# Every shape is measured on every backend the tree can emit for: the fold must
# never make a function longer (this is what caught the same rule costing two
# instructions on AArch64 before the narrow-compare immediate path existed),
# and the values are then checked by executing cross_domain_sext_cmp.c, whose
# model is arithmetic rather than the rewritten expression.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
bindir=$(dirname "$ccc")
src=$repo/tests/regression/cross_domain_sext_cmp_asm.c
values=$repo/tests/regression/cross_domain_sext_cmp.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail=0
bad() {
    echo "FAIL: $*" >&2
    fail=1
}
body() { # <function> <file>
    sed -n "/^$1:/,/^\.size/p" "$2"
}
count() { # <function> <file>
    body "$1" "$2" | grep -cE '^\s+[a-z]' || true
}
check_present() { # <desc> <file> <egrep pattern>
    if ! grep -qE "$3" "$2"; then
        echo "FAIL: $1 -- required pattern '$3' not found:" >&2
        cat "$2" >&2
        fail=1
    fi
}
check_absent() { # <desc> <file> <egrep pattern>
    if grep -qE "$3" "$2"; then
        echo "FAIL: $1 -- forbidden pattern '$3' matched:" >&2
        grep -nE "$3" "$2" >&2
        fail=1
    fi
}

# ── 1. x86-64: the sign test, and no compare against the wide constant ──────
"$ccc" -O2 -S "$src" -o "$tmp/on.s"
body fire_lt "$tmp/on.s" > "$tmp/fire_lt.s"
check_present "fire_lt: sign test on the byte" "$tmp/fire_lt.s" 'testb +%[a-z0-9]+, %[a-z0-9]+'
check_present "fire_lt: non-negative answer" "$tmp/fire_lt.s" 'setge|setns'
check_absent "fire_lt: 300 is gone" "$tmp/fire_lt.s" '\$300'
body fire_ge "$tmp/on.s" > "$tmp/fire_ge.s"
check_present "fire_ge: the sign bit is the answer" "$tmp/fire_ge.s" 'shrl +\$31|setns|setl'
check_absent "fire_ge: 300 is gone" "$tmp/fire_ge.s" '\$300'
body fire64_ge "$tmp/on.s" > "$tmp/fire64_ge.s"
check_absent "fire64_ge: no 64-bit compare is left" "$tmp/fire64_ge.s" 'cmpq'

# ── 2. The controls: inside a half, or one step outside the boundary ───────
for f in keep_in_half keep16_in_half keep_gap keep64_gap keep64_high; do
    body "$f" "$tmp/on.s" > "$tmp/$f.s"
    check_present "$f: the compare survives" "$tmp/$f.s" 'cmp[bwlq]?|cmpl|cmpq'
done
body keep_in_half "$tmp/on.s" > "$tmp/keep_in_half.s"
check_present "keep_in_half: still compares with 5" "$tmp/keep_in_half.s" 'cmpl? +\$5,'
body keep_gap "$tmp/on.s" > "$tmp/keep_gap.s"
check_present "keep_gap: still compares with the boundary constant" "$tmp/keep_gap.s" 'cmpl? +\$-127,'

# ── 3. Attribution: the knob is what fires ────────────────────────────────
CCC_SIMPLIFY_SKIP=cmprange "$ccc" -O2 -S "$src" -o "$tmp/off.s"
body fire_lt "$tmp/off.s" > "$tmp/fire_lt_off.s"
check_present "knob-off: the compare against 300 is back" "$tmp/fire_lt_off.s" '\$300'
if [ "$(count fire_lt "$tmp/on.s")" -ge "$(count fire_lt "$tmp/off.s")" ]; then
    bad "fire_lt did not get shorter with the rule on ($(count fire_lt "$tmp/off.s") -> $(count fire_lt "$tmp/on.s"))"
fi

# ── 4. Every backend: never longer, and the fire shapes lose the constant ──
labels=$(grep -oE '^[a-z_][a-zA-Z0-9_]*:' "$tmp/on.s" | tr -d ':' | sort -u)
for t in "" i686 arm riscv; do
    name=${t:-x86-64}
    driver="$ccc"
    [ -z "$t" ] || driver="$bindir/lccc-$t"
    [ -x "$driver" ] || continue
    "$driver" -O2 -S "$src" -o "$tmp/$name-on.s" 2>/dev/null || { bad "$name: emission failed"; continue; }
    CCC_SIMPLIFY_SKIP=cmprange "$driver" -O2 -S "$src" -o "$tmp/$name-off.s" 2>/dev/null \
        || { bad "$name: knob-off emission failed"; continue; }
    for label in $labels; do
        on=$(count "$label" "$tmp/$name-on.s")
        off=$(count "$label" "$tmp/$name-off.s")
        if [ "$on" -gt "$off" ]; then
            bad "$name: $label got longer ($off -> $on)"
            body "$label" "$tmp/$name-on.s" >&2
        fi
    done
    for label in fire_lt fire_le fire_ge fire_gt fire16_ge fire64_ge fire64_gt; do
        body "$label" "$tmp/$name-on.s" > "$tmp/$name-$label.s"
        # The constant the rule proved irrelevant must not appear anywhere in
        # the function, on any backend.
        case $label in
            fire_lt|fire_le|fire_ge|fire_gt) pattern='\b300\b' ;;
            fire16_ge) pattern='\b70000\b' ;;
            *) pattern='184467440715620679(6[78])' ;;
        esac
        if grep -qE "$pattern" "$tmp/$name-$label.s"; then
            bad "$name: $label still materializes the constant"
            cat "$tmp/$name-$label.s" >&2
        fi
    done
done

# ── 5. The values ─────────────────────────────────────────────────────────
"$ccc" -O2 "$values" -o "$tmp/values"
out=$("$tmp/values")
case $out in
    "cross_domain_sext_cmp: failures=0 hash="*) ;;
    *) bad "cross_domain_sext_cmp: $out" ;;
esac
# -O0 and -O3 must agree with -O2: the rewrite is a simplification, not a
# different answer.
for opt in -O0 -O1 -O3 -Os; do
    "$ccc" "$opt" "$values" -o "$tmp/values$opt"
    out=$("$tmp/values$opt")
    case $out in
        "cross_domain_sext_cmp: failures=0 hash="*) ;;
        *) bad "cross_domain_sext_cmp at $opt: $out" ;;
    esac
done

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "check_cross_domain_sext_cmp: OK (the sign-extension's two unsigned halves are separated by the constant, on every backend; values agree with an arithmetic model at -O0/-O1/-O2/-O3/-Os)"
