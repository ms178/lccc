#!/usr/bin/env bash
# Wide (128-bit) condition zero-tests must fold BOTH halves of the value:
# zero-ness of a wide value is (lo | hi) == 0, so a single-half test
# (`testq` on the low half, or the historical `cmpl $0, slot` on the low 4
# bytes of a 16-byte slot) misclassifies `(__int128)1 << 64` and branches /
# selects the wrong arm.
#
# Shape contract:
#   * CondBranch on a wide condition: `movq slot, %rax; orq slot+8, %rax`
#     followed by ONE jcc (the GCC fold shape), never a lone `testq`/`cmpq`
#     of a single half.
#   * Select with a wide condition: the read-only rcx-form fold
#     (`movq slot, %rcx; orq slot+8, %rcx`) before the cmov, never `cmpl $0`
#     on the wide slot.
#   * The i128 parameter's high-half home store (`movq %rsi, slot+8`) must
#     SURVIVE the peephole: its only reader in the dead-store window is the
#     packed `movdqu` slot copy, which the dead-store scan now models as a
#     16-byte range access. A missing high-half store + a correct wide test
#     = the test reading stale frame bytes (worse than the old bug).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/w.c" <<'EOF'
int selif(__int128 c, int a, int b) { if (c) return a; return b; }
int sel(__int128 c, int a, int b) { return c ? a : b; }
EOF

"$CCC" -O2 -S "$td/w.c" -o "$td/w.s"

function_body() {
    local function=$1 file=$2
    awk -v wanted="$function" '
        $0 == wanted ":" { active=1 }
        active { print }
        active && $0 == ".size " wanted ", .-" wanted { exit }
    ' "$file"
}

# selif: branch context — the orq-merge fold, one control transfer.
body=$(function_body selif "$td/w.s")
grep -Eq '^[[:space:]]+movq[[:space:]]+-?[0-9]+\(%rsp\), %rax' <<<"$body" || {
    echo "FAIL: selif lost its low-half load" >&2; printf '%s\n' "$body" >&2; exit 1
}
grep -Eq '^[[:space:]]+orq[[:space:]]+-?[0-9]+\(%rsp\), %rax' <<<"$body" || {
    echo "FAIL: selif lost the high-half orq fold" >&2; printf '%s\n' "$body" >&2; exit 1
}
# The old bug shape: a bare low-half test without the fold.
if grep -Eq '^[[:space:]]+testq[[:space:]]+%rax, %rax' <<<"$body"; then
    echo "FAIL: selif still tests the low half alone" >&2; printf '%s\n' "$body" >&2; exit 1
fi
if grep -Eq '^[[:space:]]+cmpl[[:space:]]+\$0,' <<<"$body"; then
    echo "FAIL: selif still under-reads with cmpl" >&2; printf '%s\n' "$body" >&2; exit 1
fi

# sel: select context — read-only rcx fold before the cmov (flags must
# survive into the cmov, so no accumulator fold here).
body=$(function_body sel "$td/w.s")
grep -Eq '^[[:space:]]+movq[[:space:]]+-?[0-9]+\(%rsp\), %rcx' <<<"$body" || {
    echo "FAIL: sel lost its rcx-form low-half load" >&2; printf '%s\n' "$body" >&2; exit 1
}
grep -Eq '^[[:space:]]+orq[[:space:]]+-?[0-9]+\(%rsp\), %rcx' <<<"$body" || {
    echo "FAIL: sel lost the rcx-form orq fold" >&2; printf '%s\n' "$body" >&2; exit 1
}
if grep -Eq '^[[:space:]]+cmpl[[:space:]]+\$0,' <<<"$body"; then
    echo "FAIL: sel still under-reads with cmpl" >&2; printf '%s\n' "$body" >&2; exit 1
fi

# The i128 parameter's high-half home store must survive dead-store
# elimination (its reader is the packed movdqu slot copy).
cat >"$td/h.c" <<'EOF'
int loopmut(__int128 c, int a, int b) {
    int n = 0;
    for (int i = 0; i < 3; i++) { if (c) n += a; else n += b; c = c >> 1; }
    return n;
}
EOF
"$CCC" -O2 -S "$td/h.c" -o "$td/h.s"
body=$(function_body loopmut "$td/h.s")
grep -Eq '^[[:space:]]+movq[[:space:]]+%rsi, -?[0-9]+\(%rsp\)' <<<"$body" || {
    echo "FAIL: the i128 parameter's high-half home store was elided" >&2
    printf '%s\n' "$body" >&2; exit 1
}

# Dirty runtime proof: the high-half-only value must select the true arm.
cat >"$td/d.c" <<'DRIVER'
#include <stdio.h>
int sel(__int128 c, int a, int b) { return c ? a : b; }
int main(void) {
    __int128 hi = (__int128)1 << 64;
    if (sel(hi, 10, 20) != 10) return 1;
    if (sel(0, 10, 20) != 20) return 2;
    return 0;
}
DRIVER
"$CCC" -O2 "$td/d.c" -o "$td/d" && "$td/d"

echo "check_wide_cond_zero_test: OK"
