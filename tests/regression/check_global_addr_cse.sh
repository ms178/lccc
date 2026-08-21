#!/usr/bin/env bash
# GlobalAddr CSE: RIP-foldable loads stay RIP-relative; must-materialize
# duplicates of the same symbol collapse to one lea.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-gaddr.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat >"$tmp/fold.c" <<'C'
/* static: local symbol, so x86-64 can fold to window(%rip) without GOT. */
static int window[256];
__attribute__((noinline)) int scan(int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += window[i];
    return s;
}
int main(void) { window[0] = 0; return scan(0); }
C

cat >"$tmp/mat.c" <<'C'
static int perm[16], perm1[16], count[16];
__attribute__((noinline)) int use3(int *a, int *b, int *c) {
    return a[0] + b[0] + c[0];
}
__attribute__((noinline)) int fannkuch_like(int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        s += use3(perm, perm1, count);
        s += use3(perm, perm1, count);
        s += use3(perm, perm1, count);
    }
    return s;
}
int main(void) { perm[0] = perm1[0] = count[0] = 1; return fannkuch_like(0) != 0; }
C

"$CCC" -O2 -march=x86-64-v3 -S "$tmp/fold.c" -o "$tmp/fold.s"
"$CCC" -O2 -march=x86-64-v3 -S "$tmp/mat.c" -o "$tmp/mat.s"

# Foldable class: the scan loop must use RIP-relative (or SIB-symbol) window
# accesses, not a dedicated lea of window into a GPR that is then reused.
fold_body=$(sed -n '/^scan:/,/^\.size scan/p' "$tmp/fold.s")
if ! grep -E -q 'window(\(%rip\)|,)' <<<"$fold_body"; then
    echo "foldable window[] lost RIP/SIB symbol addressing" >&2
    echo "$fold_body" >&2
    exit 1
fi

# Must-materialize class: three identical &perm / &perm1 / &count sites per
# iteration must not emit three independent leas of each symbol.
mat_body=$(sed -n '/^fannkuch_like:/,/^\.size fannkuch_like/p' "$tmp/mat.s")
perm_leas=$(grep -c 'leaq *perm(%rip)' <<<"$mat_body" || true)
# A well-behaved CSE leaves at most one lea of each symbol in the loop
# body (plus possibly a reload after the call clobber). Three or more
# independent per-site leas is the pre-CSE frontend dump.
if [ "$perm_leas" -ge 6 ]; then
    echo "GlobalAddr CSE did not collapse perm leas (count=$perm_leas)" >&2
    echo "$mat_body" >&2
    exit 1
fi

# Kill switch must restore the duplicate materializations.
CCC_NO_GADDR_CSE=1 "$CCC" -O2 -march=x86-64-v3 -S "$tmp/mat.c" -o "$tmp/mat.off.s"
off_body=$(sed -n '/^fannkuch_like:/,/^\.size fannkuch_like/p' "$tmp/mat.off.s")
off_leas=$(grep -c 'leaq *perm(%rip)' <<<"$off_body" || true)
if [ "$off_leas" -lt "$perm_leas" ]; then
    echo "kill switch produced fewer perm leas ($off_leas < $perm_leas)" >&2
    exit 1
fi
