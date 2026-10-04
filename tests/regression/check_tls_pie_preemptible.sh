#!/usr/bin/env bash
# Preemptible TLS in PIE executables keeps the dynamic model (S24, fixes
# §5.1 of docs/PR663_CI_REPAIR_AND_FOLLOWUP.md -- with full support rather
# than the proposed refuse-to-link gate): a PIE that exports a default-
# visibility global TLS definition must not relax GD/GOTTPOFF references
# to it into Local-Exec. Its TP offset is ld.so's to compute, so GD
# relaxes to IE and GOTTPOFF loads from a slot with an R_X86_64_TPOFF64
# dynamic relocation, exactly like a shared library's TLS.
#
# Hidden, protected and unexported definitions are never interposable and
# still relax to LE, as does every TLS reference in a non-PIE; those are
# the controls below. An honest scope note: the executable's own reads
# resolve to its own definition either way (it is first in lookup order),
# so the LE-vs-IE difference is structural, not behavioral -- the gate
# pins the structure (`R_X86_64_TPOFF64` present/absent in `.rela.dyn`)
# plus runtime correctness. The pre-fix linker emits no TPOFF64 here.
set -euo pipefail
CCC=${CCC:-target/fastbuild/lccc}
LD=${LD:-$(dirname "$CCC")/lccc-ld}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-tlspie.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/shim"
ln -s "$(cd "$(dirname "$LD")" && pwd)/$(basename "$LD")" "$tmp/shim/ld"
# Compiler-generated GD against a global `__thread` (default visibility).
cat >"$tmp/tls.c" <<'C'
__thread int tv2 = 42;
int get(void){ return tv2; }
C
# Hand-written GOTTPOFF reader of a second global `__thread`.
cat >"$tmp/ie.s" <<'S'
	.text
	.globl get3
	.type get3, @function
get3:
	movq tv3@gottpoff(%rip), %rax
	movl %fs:(%rax), %eax
	ret
	.size get3, .-get3
S
cat >"$tmp/tls3.c" <<'C'
__thread int tv3 = 43;
C
cat >"$tmp/main.c" <<'C'
#include <stdio.h>
extern int get(void), get3(void);
int main(void){ printf("%d %d\n", get(), get3()); return 0; }
C
# Hidden-TLS control: same shape, hidden visibility -- always LE.
cat >"$tmp/hid.c" <<'C'
__attribute__((visibility("hidden"))) __thread int tvh = 44;
int geth(void){ return tvh; }
C
gcc -fPIC -O1 -c "$tmp/tls.c" -o "$tmp/tls.o"
gcc -fPIC -O1 -c "$tmp/tls3.c" -o "$tmp/tls3.o"
as "$tmp/ie.s" -o "$tmp/ie.o"
gcc -fPIE -c "$tmp/main.c" -o "$tmp/main.o"
gcc -fPIC -O1 -c "$tmp/hid.c" -o "$tmp/hid.o"
grep -q "R_X86_64_TLSGD.*tv2" <(readelf -rW "$tmp/tls.o") || { echo "fixture lost its TLSGD"; exit 1; }
grep -q "R_X86_64_GOTTPOFF.*tv3" <(readelf -rW "$tmp/ie.o") || { echo "fixture lost its GOTTPOFF"; exit 1; }
tpoff_have() { readelf -rW "$1" | grep -c "R_X86_64_TPOFF64.*$2 + 0"; }
# 1. PIE + -rdynamic: both TLS models stay dynamic, program correct.
gcc -B "$tmp/shim" "$tmp/tls.o" "$tmp/tls3.o" "$tmp/ie.o" "$tmp/main.o" \
  -o "$tmp/a-pie-rdyn" -pie -rdynamic
[ "$(tpoff_have "$tmp/a-pie-rdyn" tv2)" = "1" ] || { echo "pie+rdynamic: tv2 lost its TPOFF64"; exit 1; }
[ "$(tpoff_have "$tmp/a-pie-rdyn" tv3)" = "1" ] || { echo "pie+rdynamic: tv3 lost its TPOFF64"; exit 1; }
[ "$("$tmp/a-pie-rdyn")" = "42 43" ] || { echo "pie+rdynamic: wrong output"; exit 1; }
# 2. PIE without -rdynamic: unexported, not preemptible -- LE, still correct.
gcc -B "$tmp/shim" "$tmp/tls.o" "$tmp/tls3.o" "$tmp/ie.o" "$tmp/main.o" \
  -o "$tmp/a-pie" -pie
[ "$(tpoff_have "$tmp/a-pie" tv2)" = "0" ] || { echo "pie: tv2 unexpectedly dynamic"; exit 1; }
[ "$(tpoff_have "$tmp/a-pie" tv3)" = "0" ] || { echo "pie: tv3 unexpectedly dynamic"; exit 1; }
[ "$("$tmp/a-pie")" = "42 43" ] || { echo "pie: wrong output"; exit 1; }
# 3. Hidden TLS under -rdynamic: never interposable -- LE, still correct.
cat >"$tmp/mainh.c" <<'C'
#include <stdio.h>
extern int geth(void);
int main(void){ printf("%d\n", geth()); return 0; }
C
gcc -fPIE -c "$tmp/mainh.c" -o "$tmp/mainh.o"
gcc -B "$tmp/shim" "$tmp/hid.o" "$tmp/mainh.o" -o "$tmp/a-hid" -pie -rdynamic
[ "$(tpoff_have "$tmp/a-hid" tvh)" = "0" ] || { echo "hidden: tvh unexpectedly dynamic"; exit 1; }
[ "$("$tmp/a-hid")" = "44" ] || { echo "hidden: wrong output"; exit 1; }
# 4. Non-PIE + -rdynamic: static TLS model -- LE, still correct.
gcc -B "$tmp/shim" "$tmp/tls.o" "$tmp/tls3.o" "$tmp/ie.o" "$tmp/main.o" \
  -o "$tmp/a-nopie" -no-pie -rdynamic
[ "$(tpoff_have "$tmp/a-nopie" tv2)" = "0" ] || { echo "nopie: tv2 unexpectedly dynamic"; exit 1; }
[ "$("$tmp/a-nopie")" = "42 43" ] || { echo "nopie: wrong output"; exit 1; }
echo "tls-pie-preemptible: TPOFF64 kept iff preemptible, all run correctly"
