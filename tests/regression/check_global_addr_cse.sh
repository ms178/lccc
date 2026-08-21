#!/usr/bin/env bash
# GlobalAddr CSE must actually run *and* merge, before the first GVN.
#
# Assembly lea-counts are a weak proxy: RA can still emit one lea per symbol
# even when the IR kept nine GlobalAddr values, and pre-inline GVN already
# collapses same-block GlobalAddrs into Copies. The hard gate is the pass's
# own merge counter. A dead `mod global_addr_cse` (declared, never called)
# produces no [GADDR_CSE] line and must fail here. A late-only wiring after
# post-structural-inline also fails: GVN has already rewritten the duplicates.
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

cat >"$tmp/mix.c" <<'C'
/* Same symbol used as a RIP-foldable load AND a call argument. CSE must
 * not mix the classes: the scan of window[] must stay window(%rip). */
static int window[256];
__attribute__((noinline)) int sink(int *p) { return p[0]; }
__attribute__((noinline)) int mix(int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        s += window[i];
        s += sink(window);
    }
    return s;
}
int main(void) { window[0] = 0; return mix(0); }
C

max_merged() {
    # The pass may run twice (pre-GVN + post-structural). Take the max.
    sed -n 's/.*\[GADDR_CSE\] fn=fannkuch_like merged=\([0-9][0-9]*\).*/\1/p' "$1" \
        | sort -n | tail -1
}

# --- 1. The pass must run, hoist each symbol into entry, and delete the
# 9 loop-body GlobalAddrs (3 symbols × 3 call sites). Same-block-only CSE
# without the entry hoist would report 6.
CCC_DEBUG_GADDR_CSE=1 "$CCC" -O2 -march=x86-64-v3 -S "$tmp/mat.c" \
    -o "$tmp/mat.s" >"$tmp/on.log" 2>&1
merged=$(max_merged "$tmp/on.log")
if [ -z "${merged:-}" ]; then
    echo "global_addr_cse did not run (no [GADDR_CSE] line for fannkuch_like)" >&2
    echo "--- compiler log ---" >&2
    cat "$tmp/on.log" >&2
    exit 1
fi
if [ "$merged" -lt 9 ]; then
    echo "global_addr_cse under-merged fannkuch_like (merged=$merged, expected >=9)" >&2
    echo "hint: late-only wiring after GVN → 0; same-block CSE without entry hoist → 6" >&2
    cat "$tmp/on.log" >&2
    exit 1
fi

# --- 2. Foldable class: RIP/SIB window addressing must survive CSE.
"$CCC" -O2 -march=x86-64-v3 -S "$tmp/fold.c" -o "$tmp/fold.s"
fold_body=$(sed -n '/^scan:/,/^\.size scan/p' "$tmp/fold.s")
if ! grep -E -q 'window(\(%rip\)|,)' <<<"$fold_body"; then
    echo "foldable window[] lost RIP/SIB symbol addressing" >&2
    echo "$fold_body" >&2
    exit 1
fi

# --- 3. Mixed class: RIP-foldable loads of window[] must survive a
# must-materialize use of the same symbol in the same function.
"$CCC" -O2 -march=x86-64-v3 -S "$tmp/mix.c" -o "$tmp/mix.s"
mix_body=$(sed -n '/^mix:/,/^\.size mix/p' "$tmp/mix.s")
if ! grep -E -q 'window(\(%rip\)|,)' <<<"$mix_body"; then
    echo "mixed-class window[] lost RIP/SIB symbol addressing" >&2
    echo "$mix_body" >&2
    exit 1
fi

# --- 4. Must-materialize class: at most one lea of perm in the hot function.
mat_body=$(sed -n '/^fannkuch_like:/,/^\.size fannkuch_like/p' "$tmp/mat.s")
perm_leas=$(grep -c 'leaq *perm(%rip)' <<<"$mat_body" || true)
if [ "$perm_leas" -ge 6 ]; then
    echo "GlobalAddr CSE did not collapse perm leas (count=$perm_leas)" >&2
    echo "$mat_body" >&2
    exit 1
fi

# --- 5. Both kill switches must prevent the substitution merge.
# (GVN may still CSE same-class GlobalAddrs via Copies; the counter is the
# contract for *this* pass.)
for kill in 'CCC_NO_GADDR_CSE=1' 'CCC_DISABLE_PASSES=gaddrcse'; do
    # shellcheck disable=SC2086
    env $kill CCC_DEBUG_GADDR_CSE=1 "$CCC" -O2 -march=x86-64-v3 -S "$tmp/mat.c" \
        -o "$tmp/mat.off.s" >"$tmp/off.log" 2>&1
    off_merged=$(max_merged "$tmp/off.log")
    if [ -n "${off_merged:-}" ] && [ "$off_merged" -gt 0 ]; then
        echo "kill switch $kill still merged $off_merged GlobalAddrs" >&2
        cat "$tmp/off.log" >&2
        exit 1
    fi
done
