#!/usr/bin/env bash
# Regression: IPA-RA must not trust the clobber summary of a definition that
# another TU can replace at link time (#pragma weak, a weak redeclaration
# after the definition, top-level asm ".weak"). Before the fix the weak
# bodies were summarized and use_* printed 55 instead of 79 once override.c
# supplied strong leaves that clobber %r8-%r11. The static control proves the
# test exercises IPA-RA at all (its caller must carry an LCCC_IPA_KEEP mask).
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
[ -x "$ccc" ] || { echo "FAIL: $ccc not built" >&2; exit 1; }
d=$root/tests/regression/ipa_ra_interposition
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
"$ccc" -O2 -S "$d/caller.c" -o "$tmp/caller.s"
keep=$(awk '/^use_static:/,/\.size use_static/' "$tmp/caller.s")
grep -c 'LCCC_IPA_KEEP' <<<"$keep" >/dev/null \
  || { echo "FAIL ipa_ra_interposition: control use_static has no IPA keep mask"; exit 1; }
for mode in "" -fPIE -fPIC; do
  for opt in -O1 -O2; do
    for f in caller override main; do
      "$ccc" $opt $mode -c "$d/$f.c" -o "$tmp/$f.o"
    done
    link=""; [ "$mode" = -fPIE ] && link=-pie
    [ "$mode" = -fPIC ] && link=-pie
    "$ccc" $link "$tmp/main.o" "$tmp/caller.o" "$tmp/override.o" -o "$tmp/t"
    out=$("$tmp/t")
    [ "$out" = "79 79 79 79" ] \
      || { echo "FAIL ipa_ra_interposition ($opt ${mode:-no-pic}): $out"; exit 1; }
  done
done
echo "PASS check_ipa_ra_interposition"
