#!/usr/bin/env bash
# Guard the hot scalar CRC recurrence against type-erased 64-bit register copies.
#
# The recurrence is
#   c = table[(c ^ *p++) & 0xff] ^ (c >> 8)
# where both XOR and AND operands are 32-bit values.  A sound peephole must turn
# `movq %r8, %rcx` / `movq %rax, %r8` for known-zero-extended values into
# 32-bit `movl` copies (shorter encoding, no REX.W / 64-bit dependency).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cat >"$td/crc.c" <<'EOF'
typedef unsigned int u32;
typedef unsigned char u8;
extern const u32 crc32table[256];
u32 crc32_update(u32 crc, const u8 *buf, unsigned len) {
    const u8 *end = buf + len;
    while (buf != end)
        crc = crc32table[(crc ^ *buf++) & 0xff] ^ (crc >> 8);
    return crc;
}
EOF
"$CCC" -O3 -march=x86-64-v3 -fno-pic -no-pie -S "$td/crc.c" -o "$td/crc.s"
body=$(sed -n '/^crc32_update:/,/^\.size[[:space:]]*crc32_update/p' "$td/crc.s")
# Inspect the hot loop specifically.  ABI/pointer setup may legitimately use
# 64-bit copies before .LBB1; a type-erased 64-bit copy of the zero-extended
# table index or XOR result inside the loop is the defect under test.
loop=$(sed -n '/^\.LBB1:/,/^\.LBB3:/p' <<<"$body")
if grep -E '^[[:space:]]+movq[[:space:]]+%r[a-z0-9]+,[[:space:]]*%r(cx|8|11|ax)' <<<"$loop"; then
    echo "64-bit zero-extended-value copy remains in the hot CRC recurrence" >&2
    echo "$loop" >&2
    exit 1
fi
# The compact recurrence should remain well under the old 28-instruction form.
# This is a canary, not a tight performance promise; it prevents future RA or
# peephole regressions from silently doubling the loop body.
count=$(grep -Ec '^[[:space:]]+[a-z][a-z0-9.]+[[:space:]]' <<<"$loop" || true)
if [ "$count" -gt 18 ]; then
    echo "crc32_update expanded to $count instructions" >&2
    echo "$body" >&2
    exit 1
fi
