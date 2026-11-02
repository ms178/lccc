#!/usr/bin/env bash
# `LCCC_IFCONV_ARM_BUDGET` is an A/B lever, so its CONTRACT is the only thing
# keeping measurements comparable: `=1` must really tighten the loop-aware
# budget, `off`/`full` must both disable it, and an unparsable value must be
# ignored rather than aborting a build.  Without a gate, a refactor can move
# the accessor and silently change all four answer sets, and every A/B number
# taken afterwards is a number about a different compiler.
#
# It also pins WHERE the switch is resolved.  A pipeline-wide switch belongs at
# the pipeline boundary (`src/passes/mod.rs`), which the env-read ratchet in
# `tests/regression/check_env_test_hygiene.sh` deliberately excludes; reading it
# from the pass body (`src/passes/if_convert.rs`) put the ratchet one over its
# budget the moment the switch landed.  Both halves are checked, because moving
# the read back into the pass would satisfy neither.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo"

ccc=${CCC:-target/fastbuild/lccc}
[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# A loop-resident diamond whose arms the default budget accepts and the
# strictest budget refuses: measured, not assumed -- the `=1` row below asserts
# the difference, so a kernel that stopped exercising the switch fails here
# instead of passing vacuously.
cat >"$tmp/kernel.c" <<'EOF'
int clamp_step(int v, int lo, int hi) {
    int r = v;
    if (r < lo) r = lo;
    if (r > hi) r = hi;
    return r;
}
long mix(const int *p, long n, long acc) {
    for (long i = 0; i < n; i++) {
        int v = p[i];
        int a = v & 7;
        int b = (v >> 3) & 7;
        int s = a + b;
        int d = a - b;
        acc += (s > d) ? s : d;
        acc ^= (v > 0) ? (long)v : -(long)v;
        acc += clamp_step(v, -100, 100);
    }
    return acc;
}
EOF

emit() { # emit <label> [VALUE]
    local label=$1 value=${2-}
    if [ -n "$value" ]; then
        LCCC_IFCONV_ARM_BUDGET="$value" "$ccc" -O2 -S "$tmp/kernel.c" -o "$tmp/$label.s"
    else
        env -u LCCC_IFCONV_ARM_BUDGET "$ccc" -O2 -S "$tmp/kernel.c" -o "$tmp/$label.s"
    fi
}

emit default
emit off off
emit full full
emit one 1
emit zero 0
emit junk banana

same() { cmp -s "$tmp/$1.s" "$tmp/$2.s"; }

# `off` and `full` are two spellings of "disable the loop-aware restriction".
same off full || fail "LCCC_IFCONV_ARM_BUDGET=off and =full disagree"

# An unparsable value and the documented-as-default `0` must be inert: the
# build succeeds (it did, or `set -e` would already have ended the run) and the
# code is byte-identical to the default configuration.
same junk default || fail "an unparsable LCCC_IFCONV_ARM_BUDGET changed the code"
same zero default || fail "LCCC_IFCONV_ARM_BUDGET=0 must mean 'default', not 'never convert'"

# The numeric value must actually reach the budget: `=1` is the strictest arm
# budget, so it may only ever refuse conversions relative to the default.
if same one default; then
    fail "LCCC_IFCONV_ARM_BUDGET=1 produced the default code: the switch is not applied"
fi
if [ "$(grep -c cmov "$tmp/one.s")" -gt "$(grep -c cmov "$tmp/default.s")" ]; then
    fail "LCCC_IFCONV_ARM_BUDGET=1 converted MORE diamonds than the default budget"
fi

# Where the switch is resolved: at the pipeline boundary, not in the pass.
# `if_convert.rs` legitimately reads its own trace switches; the BUDGET switch
# is pipeline configuration, and a read of it there is what pushed the ratchet
# over budget when it landed.
grep -q 'fn ifconv_arm_budget_override' src/passes/mod.rs ||
    fail "the accessor must live in src/passes/mod.rs (pipeline configuration)"
if grep -n 'LCCC_IFCONV_ARM_BUDGET' src/passes/if_convert.rs >&2; then
    fail "if_convert.rs reads LCCC_IFCONV_ARM_BUDGET directly: the accessor" \
         "belongs in src/passes/mod.rs, which the env-read ratchet excludes"
fi

echo "PASS: LCCC_IFCONV_ARM_BUDGET pins its four answer sets and its home"
