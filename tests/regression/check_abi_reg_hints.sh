#!/usr/bin/env bash
# RA-26: integer-only leaf parameters should remain in their incoming ABI
# registers when those registers are legal for the current allocation wave.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-abi-hints.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cat >"$tmp/t.c" <<'C'
__attribute__((noinline))
long abi_hint(long a,long b,long c,long d,long e,long f)
{
    return ((((((((a*(e+11))+(f+9))|(e+9))&(a+8))-(a+14))
             ^(c+16))*(a+4))&(c+14));
}
static long ref(long a,long b,long c,long d,long e,long f)
{
    return ((((((((a*(e+11))+(f+9))|(e+9))&(a+8))-(a+14))
             ^(c+16))*(a+4))&(c+14));
}
int main(void)
{
    for (long a=-5;a<7;a++) for (long c=-3;c<5;c++)
        if (abi_hint(a,2,c,4,5,6) != ref(a,2,c,4,5,6)) return 1;
    return 0;
}
C
"$CCC" -O2 -S "$tmp/t.c" -o "$tmp/t.s"
"$CCC" -O2 "$tmp/t.c" -o "$tmp/t"
"$tmp/t"
body=$(sed -n '/^abi_hint:/,/^\.size abi_hint/p' "$tmp/t.s")
# The pre-hint allocator emitted three entry shuffles for c/e/f. ABI hints
# retain them in rdx/r8/r9 and reduce this kernel by nine encoded bytes.
if grep -Eq 'movq %rdx, %rsi|movq %r8, %rdx|movq %r9, %r8' <<<"$body"; then
    echo "ABI parameter shuffles survived:" >&2
    echo "$body" >&2
    exit 1
fi
