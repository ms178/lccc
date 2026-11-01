#!/usr/bin/env bash
# The codegen mode and the link mode are ONE decision.
#
# A FULL invocation that final-links ET_EXEC produces a position-FIXED image.
# Position-independent codegen there buys nothing — the image cannot be
# relocated — and costs instructions, `leaq`s and callee-saved registers: every
# symbol+index access needs a `leaq sym(%rip), %reg` base because SIB has no
# RIP-relative form, and every such base that lives across a loop enters the
# save set. The census over the 86-program benchmark corpus measured the price
# at -257 instructions, -172 `leaq` and -54 `push`/`pop` per default build.
#
# The same argument already decided the absolute switch-table form
# (`fixed_exec_link`); this gate pins that symbol addressing follows it, and
# that everything which can still move keeps the position-independent path.
#
# Contracts:
#   1. default full link: the symbol+index access is the absolute SIB form
#      (`0x404040(,%rax,4)`) and no symbol base is materialized with `lea
#      sym(%rip)`, and the program prints what gcc prints;
#   2. `-pie` link: the image is ET_DYN, the RIP-relative symbol base is back
#      (an ET_DYN cannot hold the R_X86_64_32S the absolute form needs), the
#      absolute SIB form is gone, and the program prints the same thing;
#   3. an EXPLICIT `-fpie` is honoured even though the link is position-fixed —
#      the user asked for position-independent code, not for an ET_DYN;
#   4. compile-only output never depends on the link mode: `-S` with and
#      without `-no-pie` is byte-identical and keeps the position-independent
#      shape, and `-c` output links into a working `-pie` image, because a
#      later link may be a PIE;
#   5. `-fPIC` stays fully position independent, GOT indirection included;
#   6. every arm prints gcc's own output, so a mode that changes behaviour is
#      a failure even if its shape is right.
#
# The shapes are read from the linked image because that is the only place the
# codegen mode of a FULL invocation is observable: `-S`/`-c` are deliberately
# mode-independent (contract 4). AT&T syntax makes the two SIB arms
# distinguishable without objdump's symbol annotations: `(,%rax,4)` has no base
# register, `(%rcx,%rax,4)` has one.
#
# Usage: bash tests/regression/check_default_codegen_mode.sh
#        CCC=/path/to/lccc GCC_ORACLE=/usr/bin/gcc bash tests/regression/check_default_codegen_mode.sh
set -eu

CCC=${CCC:-target/fastbuild/lccc}
GCC_ORACLE=${GCC_ORACLE:-gcc}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if [ ! -x "$CCC" ]; then
    echo "FAIL: lccc not found at $CCC" >&2
    exit 1
fi
if ! command -v "$GCC_ORACLE" >/dev/null 2>&1; then
    echo "SKIP: no reference compiler '$GCC_ORACLE' on PATH" >&2
    exit 0
fi

work=$TMP/work
mkdir -p "$work"

src=$work/mode.c
cat >"$src" <<'EOF'
#include <stdio.h>
/* Static: a local symbol is what the symbol+index SIB form addresses, and what
   a position-independent image may address directly (a preemptible global
   would go through the GOT and the shape would say nothing about the mode).
   `ext_var` is preemptible on purpose: it is the -fPIC GOT contract. */
static int table[4096] = {0};
static unsigned char window[8192] = {0};
int ext_var = 5;

int pick(int i) { return table[i] + table[i + 1]; }

unsigned char *window_addr(void) { return window; }

int walk(int cur, int len) {
    int s = 0;
    for (int i = 0; i < len; i++) s += window[cur + i];
    return s;
}

int read_ext(void) { return ext_var; }

int main(void) {
    for (int i = 0; i < 4096; i++) table[i] = i & 7;
    for (int i = 0; i < 8192; i++) window[i] = (unsigned char)(i & 3);
    printf("pick=%d walk=%d addr=%d ext=%d\n", pick(3) + pick(100), walk(16, 32),
           (int)((window_addr() - window) + window[7]), read_ext());
    return 0;
}
EOF

note() { printf '  %-8s %s\n' "$1" "$2"; }
fail=0
ok() { note ok "$1"; }
bad() {
    note FAIL "$1"
    fail=1
}

# Absolute symbol+index: a symbol displacement with an index register and NO
# base register — `0x404040(,%rax,4)` in a disassembly, `table(, %rax, 4)` in
# assembly text. Only position-fixed codegen can form it (SIB has no
# RIP-relative slot), so its presence/absence is the mode.
abs_sib() { grep -Ec '\(,[[:space:]]*%r[a-z0-9]+,?[[:space:]]*[1248]?\)' "$1" 2>/dev/null || true; }
# The position-independent alternative: base+index `(%rcx,%rax,4)`.
base_sib() { grep -Ec '\(%r[a-z0-9]+,[[:space:]]*%r[a-z0-9]+,?[[:space:]]*[1248]?\)' "$1" 2>/dev/null || true; }

# ---- oracle -------------------------------------------------------------
"$GCC_ORACLE" -O2 -o "$work/ref" "$src" 2>/dev/null || bad "gcc oracle failed to build"
want=$("$work/ref")

# ---- 1: default full link is position-fixed codegen ---------------------
"$CCC" -O2 -o "$work/def" "$src" || bad "default build failed"
if got=$("$work/def"); then
    [ "$got" = "$want" ] && ok "default build prints gcc's output" \
        || bad "default build printed '$got', gcc printed '$want'"
else
    bad "default build did not run"
fi
objdump -d "$work/def" >"$work/def.dis" 2>/dev/null || bad "objdump failed on the default build"
d_abs=$(abs_sib "$work/def.dis")
d_base=$(base_sib "$work/def.dis")
[ "$d_abs" -ge 1 ] && ok "default: absolute symbol+index SIB present ($d_abs)" \
    || bad "default: no absolute symbol+index SIB — codegen still pays for PIE"
# The symbol+index accesses themselves must not be rebuilt through a base
# register: at least one of them is the absolute form.
[ "$d_base" -ge 1 ] && ok "default: register-base SIBs still used where they fit ($d_base)" \
    || bad "default: no base+index SIB at all — the default build did not emit the fixture's accesses"

# ---- 2: a `-pie` link keeps position independence -----------------------
if "$CCC" -O2 -pie -o "$work/pie" "$src" 2>"$work/pie.err"; then
    type=$(readelf -h "$work/pie" | awk '/Type:/{print $2}')
    [ "$type" = "DYN" ] && ok "-pie links ET_DYN" || bad "-pie linked $type, not DYN"
    got=$("$work/pie") || bad "-pie build did not run"
    [ "${got:-}" = "$want" ] && ok "-pie build prints gcc's output" \
        || bad "-pie build printed '${got:-<crash>}', gcc printed '$want'"
    objdump -d "$work/pie" >"$work/pie.dis" 2>/dev/null || true
    p_rip=$(base_sib "$work/pie.dis")
    p_abs=$(abs_sib "$work/pie.dis")
    [ "$p_rip" -ge 1 ] && ok "-pie: index accesses go through a base register ($p_rip)" \
        || bad "-pie: no base+index SIB — the symbol+index access was not rebuilt for ET_DYN"
    [ "$p_abs" -eq 0 ] && ok "-pie: no absolute symbol+index SIB" \
        || bad "-pie: $p_abs absolute SIB form(s) in an ET_DYN (R_X86_64_32S is unresolvable there)"
else
    bad "-pie link failed: $(tail -1 "$work/pie.err")"
fi

# ---- 3: an explicit -fpie is honoured in a position-fixed link ----------
"$CCC" -O2 -fpie -o "$work/explicit" "$src" || bad "-fpie build failed"
got=$("$work/explicit") || bad "-fpie build did not run"
[ "${got:-}" = "$want" ] && ok "-fpie build prints gcc's output" \
    || bad "-fpie build printed '${got:-<crash>}'"
objdump -d "$work/explicit" >"$work/explicit.dis" 2>/dev/null || true
e_abs=$(abs_sib "$work/explicit.dis")
e_rip=$(base_sib "$work/explicit.dis")
[ "$e_rip" -ge 1 ] && ok "-fpie: index accesses go through a base register ($e_rip)" \
    || bad "-fpie: no base+index SIB — the explicit request was overridden by the link mode"
[ "$e_abs" -eq 0 ] && ok "-fpie: no absolute symbol+index SIB" \
    || bad "-fpie: $e_abs absolute SIB form(s) despite an explicit -fpie"

# ---- 4: compile-only output is independent of the link mode -------------
"$CCC" -O2 -S -o "$work/a.s" "$src" || bad "-S failed"
"$CCC" -O2 -no-pie -S -o "$work/b.s" "$src" || bad "-S -no-pie failed"
cmp -s "$work/a.s" "$work/b.s" \
    && ok "-S output is byte-identical with and without -no-pie" \
    || bad "-S output changed with the link mode"
s_abs=$(abs_sib "$work/a.s")
s_base=$(base_sib "$work/a.s")
[ "$s_abs" -eq 0 ] && ok "-S keeps the position-independent shape (no absolute SIB)" \
    || bad "-S dropped position independence: $s_abs absolute symbol+index SIB form(s)"
[ "$s_base" -ge 1 ] && ok "-S index accesses go through a base register ($s_base)" \
    || bad "-S lost the symbol+index access entirely"

"$CCC" -O2 -c -o "$work/a.o" "$src" || bad "-c failed"
objdump -d "$work/a.o" >"$work/a.dis" 2>/dev/null || true
o_abs=$(abs_sib "$work/a.dis")
o_base=$(base_sib "$work/a.dis")
[ "$o_abs" -eq 0 ] && ok "-c object is position independent (no absolute SIB)" \
    || bad "-c object is position dependent ($o_abs absolute SIB) — a later -pie link would fail"
[ "$o_base" -ge 1 ] && ok "-c object uses register bases ($o_base)" \
    || bad "-c object lost the symbol+index access"
if "$CCC" -pie -o "$work/twostep" "$work/a.o" 2>"$work/two.err"; then
    got=$("$work/twostep") || bad "two-step -c then -pie did not run"
    [ "${got:-}" = "$want" ] && ok "-c then -pie links and prints gcc's output" \
        || bad "-c then -pie printed '${got:-<crash>}'"
else
    bad "two-step -c then -pie link failed: $(tail -1 "$work/two.err")"
fi

# ---- 5: -fPIC stays fully position independent -------------------------
"$CCC" -O2 -fPIC -S -o "$work/pic.s" "$src" || bad "-fPIC -S failed"
c_abs=$(abs_sib "$work/pic.s")
c_base=$(base_sib "$work/pic.s")
[ "$c_abs" -eq 0 ] && ok "-fPIC stays position independent (no absolute SIB)" \
    || bad "-fPIC emitted $c_abs absolute symbol+index SIB form(s)"
[ "$c_base" -ge 1 ] && ok "-fPIC keeps the register-base form ($c_base)" \
    || bad "-fPIC lost the symbol+index access"
if grep -Eq 'ext_var@GOTPCREL\(' "$work/pic.s"; then
    ok "-fPIC reaches preemptible data through the GOT"
else
    bad "-fPIC did not use the GOT for a preemptible global — the interposition rule was dropped"
fi

if [ "$fail" -ne 0 ]; then
    echo "FAIL: default codegen mode" >&2
    exit 1
fi
echo "PASS: default codegen mode (a position-fixed ET_EXEC pays no PIE price; -pie, -fpie, -c, -S and -fPIC stay position independent)"
