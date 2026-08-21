#!/usr/bin/env bash
# A single-operand unary RMW (`notl %eax`, `negl %eax`, `incl %eax`, ...) must
# be classified as WRITING its register.  Before this fix, classify_line gave
# such lines dest_reg = REG_NONE (no comma -> parse_dest_reg fails), so
# propagate_reg_copies let a `movl %edx,%eax` alias SURVIVE `notl %eax` and
# rewrote the result's consumer back to the pre-not register: `v0 = ~v0`
# silently returned the OLD v0.  The miscompile fired at every -O level
# because the peephole (not codegen) introduced it — LCCC_NO_PEEPHOLE hid it.
# Found by tests/fuzz/slot_rmw_differential.py (seed 2).
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cat >"$td/test.c" <<'EOF'
int probe_not(int v) { int v0 = v; v0 = ~v0; return v0 & 255; }
int probe_neg(int v) { int v0 = v; v0 = -v0; return v0 & 255; }
int probe_chain(int v) {
    int v0 = v;
    v0 = ~v0;      /* unary not, result must survive the peephole */
    v0 = -v0;      /* unary neg directly on the previous result   */
    v0 += 1;
    return v0 & 255;
}
int main(void) {
    int rc = 0;
    rc |= probe_not(12345)  != ((~12345) & 255);
    rc |= probe_not(-1)     != 0;
    rc |= probe_neg(12345)  != ((-12345) & 255);
    rc |= probe_neg(0)      != 0;
    rc |= probe_chain(777)  != ((-~777 + 1) & 255);
    return rc;
}
EOF
have_i386_loader=0
for loader in /lib/ld-linux.so.2 /lib32/ld-linux.so.2 /usr/lib32/ld-linux.so.2; do
    [ -x "$loader" ] && have_i386_loader=1
done
for opt in O0 O1 O2 Os; do
    "$CCC" -m32 -fno-pic -$opt "$td/test.c" -o "$td/test_$opt" 2>"$td/err_$opt.txt" || {
        echo "compile failed at -$opt"; cat "$td/err_$opt.txt"; exit 1;
    }
    if [ "$have_i386_loader" = 1 ]; then
        "$td/test_$opt" || { echo "runtime mismatch at -$opt (unary RMW result lost)"; exit 1; }
    fi
done
if [ "$have_i386_loader" = 0 ]; then
    echo "SKIP runtime: all modes compiled; host has no i386 ELF interpreter"
fi
