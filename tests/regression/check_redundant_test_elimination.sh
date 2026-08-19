#!/usr/bin/env bash
# `andl`/`orl` set ZF/SF/PF from the result and clear CF/OF — the exact flag
# state of `testl %R,%R` over that result.  A test immediately after such a
# logical op is pure overhead and must be eliminated (boot corpus: 29 sites,
# e.g. cpucheck's `andl $536870912,%eax; testl %eax,%eax; je`).  Regression
# guard: the pattern must vanish AND the sign flag must still be consumable
# (SF comes from the AND itself now, not from a test).
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cat >"$td/test.c" <<'EOF'
static unsigned word;
int has_high_bit(void) { return (word & 0x20000000u) != 0; }
int is_negative_masked(int x) { return (x & 0x80000000) != 0; }
EOF
"$CCC" -m32 -Os -fno-pic -S "$td/test.c" -o "$td/test.s"

body=$(sed -n '/^has_high_bit:/,/^\.size has_high_bit/p' "$td/test.s")
grep -q 'andl \$536870912' <<<"$body" || { echo "missing andl site"; echo "--- $body"; exit 1; }
if grep -q 'testl' <<<"$body"; then
    echo "redundant testl survived the logical op"
    echo "--- $body"
    exit 1
fi

body2=$(sed -n '/^is_negative_masked:/,/^\.size is_negative_masked/p' "$td/test.s")
if grep -q 'testl' <<<"$body2"; then
    echo "redundant testl survived on the sign-flag consumer"
    echo "--- $body2"
    exit 1
fi

# Runtime semantics (flag consumers: ZF via setcc/jcc, SF via sign compare).
cat >"$td/run.c" <<'EOF'
static unsigned word;
static int has_high_bit(void) { return (word & 0x20000000u) != 0; }
static int is_negative_masked(int x) { return (x & 0x80000000) != 0; }
int main(void)
{
    int rc = 0;
    word = 0;                 rc |= has_high_bit() != 0;
    word = 0x20000000u;       rc |= has_high_bit() != 1;
    word = 0x1fffffffu;       rc |= has_high_bit() != 0;
    rc |= is_negative_masked(0) != 0;
    rc |= is_negative_masked(-1) != 1;
    rc |= is_negative_masked(0x7fffffff) != 0;
    return rc;
}
EOF
"$CCC" -m32 -O2 -fno-pic "$td/run.c" -o "$td/run"
"$td/run"
