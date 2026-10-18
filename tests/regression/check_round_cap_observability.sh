#!/usr/bin/env bash
# T5 of engineering/FOLLOWUP-2026-10-18-vellvm-vir-transplant.md: a bounded
# fixpoint loop that stops because its round budget ran out (while the last
# round still changed the IR) must be observable, not silent.
#
# Two checks, both fail closed:
#   1. Static: every `round_cap::Site` variant is wired to exactly one
#      `note_cap_hit(round_cap::Site::<V>)` call in src/passes, and every
#      call names a declared variant. A new site without a call, or a call
#      whose counter was deleted, fails here.
#   2. Behavioral witnesses: `cross_pr_redteam.c` exhausts backedge_pre's
#      3-round budget and `fortify_open_va_pack.c` exhausts the ordinary
#      inline budget, both at -O2 (2 654-file corpus sweep, see the
#      follow-up). Under CCC_TIME_PASSES=1 the compiler must print the
#      matching `round_cap <site>` line; with the variable unset it must
#      print nothing. A cap-hit edit that drops the report, or a cap change
#      that removes the witness, fails here and must be re-measured rather
#      than silently relaxed.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"
CCC=${CCC:-target/fastbuild/lccc}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-roundcap.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

fail=0
die() { echo "FAIL: $*" >&2; fail=1; }

# 1. Static wiring ---------------------------------------------------------
decl=$(sed -n '/^pub(crate) enum Site {/,/^}/p' src/passes/round_cap.rs \
    | sed -n 's/^    \([A-Za-z]*\),$/\1/p')
[ -n "$decl" ] || die "could not parse the Site enum in src/passes/round_cap.rs"
calls=$(grep -rhoE 'round_cap::note_cap_hit\(round_cap::Site::[A-Za-z]+\)' src/passes \
    | sed -E 's/.*Site::([A-Za-z]+)\)/\1/' | sort)
for v in $decl; do
    n=$(grep -c "^$v\$" <<<"$calls" || true)
    [ "$n" -eq 1 ] || die "Site::$v has $n note_cap_hit call(s), want exactly 1"
done
while read -r c; do
    [ -n "$c" ] || continue
    grep -qx "$c" <<<"$decl" || die "note_cap_hit names undeclared Site::$c"
done <<<"$calls"
echo "static: $(wc -w <<<"$decl" | tr -d ' ') sites wired"

# 2. Behavioral witnesses ---------------------------------------------------
# Each pair is "<source> <expected report prefix>", measured on the 2 654-file
# corpus at -O2 (see the follow-up document). The report must appear under
# CCC_TIME_PASSES=1 and must not appear without it.
witness() {
    local src=$1 want=$2
    CCC_TIME_PASSES=1 "$CCC" -O2 -c "$src" -o "$tmp/on.o" 2>"$tmp/on.err" \
        || { die "lccc failed on $src with CCC_TIME_PASSES=1"; return; }
    grep -q "^\[PASS\] round_cap $want: [0-9][0-9]* hit(s)\$" "$tmp/on.err" \
        || { die "no 'round_cap $want' report for $src under CCC_TIME_PASSES=1"; return; }
    "$CCC" -O2 -c "$src" -o "$tmp/off.o" 2>"$tmp/off.err" \
        || { die "lccc failed on $src without CCC_TIME_PASSES"; return; }
    grep -q 'round_cap' "$tmp/off.err" && die "round_cap output leaked without CCC_TIME_PASSES for $src"
    echo "witness: $want on $src"
}
witness tests/regression/cross_pr_redteam.c backedge_pre
witness tests/regression/fortify_open_va_pack.c inline_ordinary_rounds

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "OK check_round_cap_observability"
