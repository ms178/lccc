#!/usr/bin/env bash
# RA-26/AB-09: call-free CFG leaves keep leading ParamRefs in ABI/caller homes
# and do not allocate a duplicate empty local frame.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}; t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
cat >"$t/t.c" <<'C'
__attribute__((noinline)) int leaf(const int *p,int n){if(n>0)return p[0]+n;return p[1]-n;}
int main(void){int x[2]={3,4};if(leaf(x,1)!=4)return 1;if(leaf(x,0)!=4)return 2;return 0;}
C
"$CCC" -O2 "$t/t.c" -o "$t/t"; "$t/t"
"$CCC" -O2 -S "$t/t.c" -o "$t/on.s"
CCC_NO_LEAF_PARAM_GPR=1 CCC_NO_EMPTY_LOCAL_FRAME_ELISION=1 "$CCC" -O2 -S "$t/t.c" -o "$t/off.s"
on=$(sed -n '/^leaf:/,/^\.size leaf/p' "$t/on.s")
off=$(sed -n '/^leaf:/,/^\.size leaf/p' "$t/off.s")
! grep -Eq '(pushq|popq|(subq|addq).*%rsp)' <<<"$on" || { echo "FAIL: negative assertion at ${BASH_SOURCE[0]##*/}:$LINENO" >&2; exit 1; }
! grep -Eq 'movq[[:space:]]+%r(di|si),' <<<"$on" || { echo "FAIL: negative assertion at ${BASH_SOURCE[0]##*/}:$LINENO" >&2; exit 1; }
# A frameless leaf keeps the CIE's entry CFA (rsp+8): zero directives (GCC's
# form) or only `.cfi_def_cfa_offset 8`; any other offset means a frame.
awk '/\.cfi_def_cfa_offset/ && !/\.cfi_def_cfa_offset 8$/ {
        print "FAIL: frameless leaf moved its CFA: " $0 > "/dev/stderr"; bad = 1
    } END { exit bad }' <<<"$on"
grep -q 'pushq' <<<"$off"
grep -Eq 'subq.*%rsp' <<<"$off"
