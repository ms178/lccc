#!/usr/bin/env bash
# Assembly contract for a segment qualifier written after an earlier `*`.
#
#   long __seg_gs *__seg_fs *q;   q is ordinary memory; *q is a %fs pointer;
#                                 **q is read through %gs after the %fs load.
#   long *__seg_gs *p;            *p is read through %gs; p is ordinary memory.
#   long *__seg_gs p2;            the object p2 itself is in %gs (last star).
#
# The %fs and %gs forms are checked by sequence, at -O0 -O1 -O2, against the
# instructions GCC 14.2 emits. The GS-only runtime test
# (seg_gs_runtime/seg_gs_pointee_chain.c) cannot exercise %fs, because glibc
# uses the FS base for TLS.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
if [ "$(uname -m)" != "x86_64" ]; then
    echo "SKIP: segment assembly contract needs x86-64"
    exit 0
fi
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-seg-chain.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
src="$tmp/chain.c"
cat > "$src" <<'C'
long __seg_gs *__seg_fs *q;
long *__seg_gs *p;
long *__seg_gs p2;
long fs_then_gs(void) { return **q; }
long gs_pointee(void) { return *(*p); }
long gs_object(void) { return *p2; }
C
fail=0
# check <opt> <function> <regex...>: the regexes must match the function body
# IN ORDER. The %fs load of the outer pointer must come before the %gs access
# that uses the loaded value; an unordered match would accept the reverse.
check() {
    local opt=$1 fn=$2; shift 2
    "$CCC" "$opt" -S -o "$tmp/out$opt.s" "$src"
    local body
    body=$(awk -v f="$fn:" '$0==f{p=1;next} p&&/^[^ \t#.]/{exit} p' "$tmp/out$opt.s" | grep -v '^\s*[.#]')
    local want line prev=0
    for want in "$@"; do
        line=$(PAT="$want" PREV="$prev" awk \
            'NR > ENVIRON["PREV"] && $0 ~ ENVIRON["PAT"] { print NR; exit }' <<<"$body")
        if [ -z "$line" ]; then
            echo "FAIL: lccc $opt $fn lacks /$want/ after line $prev (in order)" >&2
            echo "$body" >&2
            fail=1
            return
        fi
        prev=$line
    done
    echo "PASS: lccc $opt $fn (sequence in order)"
}
for opt in -O0 -O1 -O2; do
    # *q is a %fs pointer: its load is %fs-prefixed, then the %gs load follows.
    check "$opt" fs_then_gs 'movq[[:space:]]+q\(%rip\)' '%fs:\(' '%gs:\('
    check "$opt" gs_pointee 'movq[[:space:]]+p\(%rip\)' '%gs:\('
    check "$opt" gs_object '%gs:p2\(%rip\)'
done
if [ "$fail" -ne 0 ]; then
    echo "check_seg_pointee_chain_asm: FAILED" >&2
    exit 1
fi
echo "check_seg_pointee_chain_asm: PASSED"
