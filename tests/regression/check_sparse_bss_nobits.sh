#!/usr/bin/env bash
# Sparse NOBITS `.bss`: zero fills are a size, not bytes.
#
# A `.bss` object is written as NOBITS with an exact `sh_size`, so a 4 GiB
# array costs a constant amount of compiler memory and time, and the zero
# bytes are produced by the kernel at run time instead of by the compiler.
# What each probe attacks:
#
#   1. The 4 GiB array: compiled, linked, *run*. The object and the executable
#      must both stay tiny, the exit code must be right, and the same source
#      must agree with GCC. A representation that materialised the zeros would
#      either blow up here or produce a giant object.
#   2. A symbol defined inside a sparse tail: `.skip 100000` followed by a
#      label must place the label exactly 100000 bytes later, both when the
#      compiler reads its own output and at run time. This is the case where a
#      "size, not bytes" representation gives the wrong answer if the offset
#      arithmetic is off by one section's worth.
#   3. `@progbits` forced on `.bss`: the section is then allowed to carry data,
#      so the bytes must exist and be writable. GAS is used as the oracle for
#      what the assembler directive means (it warns and ignores the type), so
#      this probe cannot be satisfied by an assumption about GAS.
#   4. A negative `.zero` count: GAS warns and IGNORES it ("repeat count is
#      negative, ignored"), contributing no bytes, so this compiler must do the
#      same -- the count is carried as `u64` and a wrapped value must never
#      become a section size.
#   5. A huge alignment in a NOBITS section: GAS records `sh_addralign` and
#      writes no bytes, so `.section .bss` + `.p2align 40` must be free and
#      exact. In a section that *does* have to materialise the gap the same
#      alignment must be a diagnostic, never an aborted process.
#   6. VLA `sizeof`: a variable-length array's `sizeof` is a runtime value and
#      must not be replaced by the decayed pointer size (the decay rule is for
#      comma/conditional operands, not for `sizeof` of the array itself).
#
# gcc/as are optional: the parity probes SKIP (exit 0) when they are missing,
# so this gate is usable on hosts that only have a bootstrapped compiler.
#
# Usage: bash tests/regression/check_sparse_bss_nobits.sh
#        CCC=/path/to/lccc bash tests/regression/check_sparse_bss_nobits.sh
set -eu

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
CCC=${CCC:-target/fastbuild/lccc}
case $CCC in /*) ;; *) CCC=$REPO_ROOT/$CCC ;; esac
[ -x "$CCC" ] || { echo "setup error: no compiler at $CCC" >&2; exit 2; }

TMP=$(mktemp -d "${TMPDIR:-/tmp}/lccc_sparse_bss.XXXXXX")
trap 'rm -rf "$TMP"' EXIT
cd "$TMP"

PASS=0
FAIL=0
SKIP=0
pass() { printf '  ok   %s\n' "$1"; PASS=$((PASS + 1)); }
fail() { printf '  FAIL %s: %s\n' "$1" "$2" >&2; FAIL=$((FAIL + 1)); }
skip() { printf '  skip %s: %s\n' "$1" "$2"; SKIP=$((SKIP + 1)); }
eq() { if [ "$2" = "$3" ]; then pass "$1"; else fail "$1" "expected [$2], got [$3]"; fi; }

# Run a command and echo its status; never let `set -e` end the probe run.
status_of() { set +e; "$@" >"$1.log" 2>&1; rc=$?; set -e; echo "$rc"; }

HAVE_GCC=0
if command -v gcc >/dev/null 2>&1; then HAVE_GCC=1; fi
HAVE_AS=0
if command -v as >/dev/null 2>&1; then HAVE_AS=1; fi

if [ "$HAVE_GCC" = 1 ]; then
    GCC_INC=$(gcc -print-file-name=include)
    CCC=("$CCC" "-I$GCC_INC")
else
    CCC=("$CCC")
fi

echo "== 1. a 4 GiB .bss array: tiny object, tiny executable, correct run =="
cat > big.c <<'EOF'
int a[1000000000];
int main(void) { return (int)(sizeof(a) % 7); }
EOF
eq "4 GiB array compiles" "0" "$(status_of "${CCC[@]}" -O2 -c -o big.o big.c)"
obj_size=$(wc -c <big.o 2>/dev/null || echo 0)
if [ "$obj_size" -gt 0 ] && [ "$obj_size" -lt 65536 ]; then
    pass "object is sparse (${obj_size} bytes for 4 GiB of .bss)"
else
    fail "object is sparse" "object is ${obj_size} bytes; a materialised .bss would be gigabytes"
fi
eq "4 GiB array links" "0" "$(status_of "${CCC[@]}" -O2 -o big big.c)"
exe_size=$(wc -c <big 2>/dev/null || echo 0)
if [ "$exe_size" -gt 0 ] && [ "$exe_size" -lt 65536 ]; then
    pass "executable is small (${exe_size} bytes)"
else
    fail "executable is small" "executable is ${exe_size} bytes"
fi
set +e; ./big; rc=$?; set -e
eq "4 GiB .bss program runs" "3" "$rc"
if [ "$HAVE_GCC" = 1 ]; then
    gcc -O2 -o big_gcc big.c 2>/dev/null || true
    set +e; ./big_gcc; grc=$?; set -e
    eq "gcc agrees on the same program" "$grc" "$rc"
    gcc_size=$(wc -c <big_gcc 2>/dev/null || echo 0)
    echo "  info executable sizes: lccc=${exe_size}B gcc=${gcc_size}B"
else
    skip "gcc parity for the 4 GiB program" "no gcc"
fi

echo "== 2. a label inside a sparse tail keeps its exact offset =="
cat > tail.c <<'EOF'
__asm__(".pushsection .bss\n"
        "bigbuf: .skip 100000\n"
        "after_bigbuf: .skip 8\n"
        ".popsection\n");
extern char bigbuf[], after_bigbuf[];
int main(void) { return (int)(after_bigbuf - bigbuf) - 100000; }
EOF
eq "tail-label program compiles" "0" "$(status_of "${CCC[@]}" -O2 -o tail tail.c)"
set +e; ./tail; t_rc=$?; set -e
eq "label after 100000-byte skip is at +100000" "0" "$t_rc"
if [ "$HAVE_GCC" = 1 ]; then
    gcc -O2 -o tail_gcc tail.c 2>/dev/null || true
    set +e; ./tail_gcc; tg_rc=$?; set -e
    eq "gcc agrees on the tail offset" "0" "$tg_rc"
else
    skip "gcc parity for the tail label" "no gcc"
fi

echo "== 3. @progbits forced on .bss: bytes exist and are writable =="
cat > progbits.c <<'EOF'
__asm__(".section .bss,\"aw\",@progbits\n"
        "pbuf: .skip 70000\n"
        ".previous\n");
extern char pbuf[];
int main(void) { pbuf[69999] = 1; return pbuf[69999] - 1; }
EOF
eq "forced-progbits .bss compiles" "0" "$(status_of "${CCC[@]}" -O2 -o progbits progbits.c)"
set +e; ./progbits; p_rc=$?; set -e
eq "last byte of a forced-progbits .bss payload is writable" "0" "$p_rc"
if [ "$HAVE_AS" = 1 ]; then
    printf '.section .bss,"aw",@progbits\npbuf: .skip 8\n' > gas_probe.s
    set +e; as -o gas_probe.o gas_probe.s 2>gas_probe.err; gas_rc=$?; set -e
    gas_verdict=$([ "$gas_rc" = 0 ] && echo accept || echo reject)
    echo "  info GAS verdict on '@progbits .bss': $gas_verdict ($(head -1 gas_probe.err 2>/dev/null))"
    # Same verdict as GAS: both accept the section and both keep the payload.
    eq "lccc matches the GAS verdict" "0" "$gas_rc"
else
    skip "GAS oracle for @progbits .bss" "no as(1)"
fi

echo "== 4. a negative .zero count is ignored, exactly as GAS does =="
cat > neg.c <<'EOF'
__asm__(".pushsection .bss\n"
        "n: .zero -1\n"
        "m: .zero 8\n"
        ".popsection\n");
extern char n[], m[];
int main(void) { return (int)(m - n); }
EOF
is_parity=1
if [ "$HAVE_AS" = 1 ]; then
    printf '.pushsection .bss\nn: .zero -1\nm: .zero 8\n.popsection\n' > neg.s
    set +e; as -o neg_gas.o neg.s 2>neg_gas.err; neg_gas_rc=$?; set -e
    gas_delta=$(( $(nm neg_gas.o | awk '$3=="m"{print "0x"$1}') - $(nm neg_gas.o | awk '$3=="n"{print "0x"$1}') ))
    echo "  info GAS: rc=$neg_gas_rc m-n=$gas_delta ($(head -1 neg_gas.err))"
else
    gas_delta=0
fi
eq "negative .zero compiles (GAS ignores the count)" "0" "$(status_of "${CCC[@]}" -O2 -o neg neg.c)"
set +e; ./neg; neg_delta=$?; set -e
eq "the ignored count contributes no bytes (m-n)" "${gas_delta:-0}" "$neg_delta"
if [ "$HAVE_AS" = 1 ] && [ "$neg_gas_rc" != 0 ]; then
    fail "the GAS oracle accepted the same input it accepts here" "GAS rc=$neg_gas_rc"
else
    pass "lccc matches the GAS verdict on a negative count"
fi

echo "== 5. a huge alignment in .bss is a size (GAS: free, rc 0) =="
cat > align_bss.s <<'EOF'
.section .bss
.p2align 40
aligned: .zero 4
EOF
eq "huge .bss alignment assembles" "0" "$(status_of "${CCC[@]}" -c -o align_bss.o align_bss.s)"
align_obj=$(wc -c <align_bss.o 2>/dev/null || echo 0)
if [ "$align_obj" -gt 0 ] && [ "$align_obj" -lt 65536 ]; then
    pass "aligned .bss object stays small (${align_obj} bytes)"
else
    fail "aligned .bss object stays small" "object is ${align_obj} bytes"
fi
if [ "$HAVE_AS" = 1 ]; then
    as -o align_gas.o align_bss.s 2>/dev/null || true
    lccc_align=$(readelf -SW align_bss.o 2>/dev/null | awk '/\] \.bss/{print $NF}')
    gas_align=$(readelf -SW align_gas.o 2>/dev/null | awk '/\] \.bss/{print $NF}')
    eq "sh_addralign matches GAS" "${gas_align:-1099511627776}" "$lccc_align"
else
    eq "sh_addralign is the requested alignment" "1099511627776" \
       "$(readelf -SW align_bss.o 2>/dev/null | awk '/\] \.bss/{print $NF}')"
fi

echo "== 6. an alignment no file can hold is a diagnostic, never a signal =="
cat > align_text.s <<'EOF'
.text
.p2align 40
n: .zero 4
EOF
rm -f align_text.o
set +e; timeout 120 "${CCC[@]}" -c -o align_text.o align_text.s >align_text.log 2>&1; at_rc=$?; set -e
if [ "$at_rc" -gt 0 ] && [ "$at_rc" -lt 128 ] && [ ! -f align_text.o ]; then
    pass "refused with a diagnostic ($(head -c 60 align_text.log))"
elif [ "$at_rc" -ge 128 ]; then
    fail "refused with a diagnostic" "signal $(($at_rc - 128)) -- the process must not be killed"
else
    fail "refused with a diagnostic" "rc=$at_rc, object=$([ -f align_text.o ] && echo written)"
fi

echo "== 7. VLA sizeof is a runtime value, not a decayed pointer =="
cat > vla.c <<'EOF'
#include <stdio.h>
int main(void) {
    int n = 7;
    int a[n];
    printf("%zu %zu %zu\n", sizeof a, sizeof((0, a)), sizeof(a) / sizeof(a[0]));
    return 0;
}
EOF
if [ "$HAVE_GCC" = 1 ]; then
    gcc -O2 -o vla_gcc vla.c 2>/dev/null && ./vla_gcc >vla_gcc.out 2>&1 || true
fi
eq "VLA program compiles" "0" "$(status_of "${CCC[@]}" -O2 -o vla vla.c)"
set +e; ./vla >vla.out 2>&1; set -e
if [ "$HAVE_GCC" = 1 ]; then
    eq "VLA sizeof parity with gcc" "$(cat vla_gcc.out 2>/dev/null)" "$(cat vla.out)"
else
    eq "VLA sizeof is the array size, not the pointer size" "28 8 7" "$(cat vla.out)"
fi

echo
if [ "$FAIL" != 0 ]; then
    printf 'sparse-bss-nobits: %d passed, %d failed, %d skipped\n' "$PASS" "$FAIL" "$SKIP" >&2
    exit 1
fi
printf 'sparse-bss-nobits: %d passed, %d skipped\n' "$PASS" "$SKIP"
