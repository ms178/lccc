#!/usr/bin/env bash
# P0-04: shared linear-form alias analysis proves a[0] disjoint from the
# marching store a[i+1], allowing LICM to hoist exactly one load.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-licm-alias.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cat >"$tmp/t.c" <<'C'
__attribute__((noinline))
int disjoint(int *a, int n)
{
    int sum = 0;
    for (int i=0; i<n; ++i) {
        sum += a[0];
        a[i+1] = i;
    }
    return sum;
}
int main(void) { int a[20] = {3}; return disjoint(a,10) != 30; }
C
"$CCC" -O2 -S "$tmp/t.c" -o "$tmp/t.s"
"$CCC" -O2 "$tmp/t.c" -o "$tmp/t"
"$tmp/t"
body=$(sed -n '/^disjoint:/,/^\.size disjoint/p' "$tmp/t.s")
loads=$(grep -Ec 'movl \(%r[a-z0-9]+\),' <<<"$body" || true)
[[ $loads -eq 1 ]] || {
    echo "expected one hoisted invariant load, found $loads" >&2
    echo "$body" >&2
    exit 1
}
