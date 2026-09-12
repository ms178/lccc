#!/usr/bin/env bash
# Segment-override full-matrix gate: the integrated assembler's %gs/%fs/%es/%cs/%ss
# encodings must be BYTE-IDENTICAL to GNU as across the whole memory-operand
# instruction matrix.
#
# WHY THIS EXISTS
# ---------------
# The encoder historically emitted the segment override per-arm, and every
# family that forgot the call silently DROPPED the override: shifts, bt/bts/
# btr/btc, the regular cmpxchg (this_cpu_cmpxchg — the kernel's hottest
# per-CPU op), clflush, every SSE load/store, every VEX/AVX memory form, and
# the f2/f3-mandatory-prefixed scalars (addsd et al. emitted `f2 65` where
# GAS emits `65 f2`). The kernel consequence was catastrophic and silent:
# 6.18.50 boot died at PID 1 with "corrupted preempt_count" because 1,125
# incl/decl sites updated the STATIC percpu image while every %gs read saw
# the real per-CPU counter.
#
# Emission now lives at ONE choke point — the operand scan in
# InstructionEncoder::encode (x86-64) and the post-body splice (i686, which
# must keep the x87 FWAIT byte ahead of it) — so an arm can no longer forget.
# This gate is the contract that keeps it that way: the full matrix of
# segment-qualified memory forms × sizes × prefix combinations is assembled
# by BOTH GNU as and lccc, and the .text sections must match BYTE FOR BYTE
# (objdump text + hex). Any new encoder path that diverges — wrong order,
# dropped prefix, extra prefix — fails here.
#
# The matrix deliberately includes: every ALU size (b/w/l/q), RMW unary
# family, shifts (imm/cl/1-op), bt family, cmpxchg/8b/16b, xchg/xadd,
# movzx/movsx, lea (incl. the 16-bit leaw + 0x66), push/pop mem, lock
# combinations (segment BEFORE lock), clflush/opt, prefetch, fxsave/rstor,
# the x87 memory family, MMX, SSE loads/stores/arith/cvt (66/f2/f3
# mandatory prefixes), VEX loads/stores/arith/cvt/broadcast, all six
# segment names (DS dropped like GAS in 64-bit), SIB forms, negative and
# positive displacements, 32-bit addressing (0x67 + REX), and — in the
# code32 matrix — the i686 default-segment drop rule (SS default for
# ebp/esp-based forms) and `lock` ordering.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
REPO=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
AS=${AS:-as}
OBJDUMP=${OBJDUMP:-objdump}
tmp=${TMPDIR:-/tmp}/lccc-seg-prefix-matrix.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cd "$REPO"

for tool in "$CCC" "$AS" "$OBJDUMP"; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "FAIL: required tool '$tool' not found" >&2
        exit 2
    }
done

fail=0

# compare_matrix <matrix.s> <extra-as-flags> <objdump-mode>
compare_matrix() {
    local matrix=$1 gas_flags=$2 objdump_flags=$3
    local base=${matrix%.s}
    local base_name
    base_name=$(basename "$base")

    if ! $AS $gas_flags -o "$tmp/$base_name.gas.o" "$matrix" 2>"$tmp/$base_name.gas.err"; then
        echo "FAIL: $base_name: GNU as (oracle) rejected the matrix — oracle is broken, fix the corpus:" >&2
        cat "$tmp/$base_name.gas.err" >&2
        exit 2
    fi
    # GAS warnings are acceptable (e.g. "segment override on `lea' is
    # ineffectual" — GAS still emits the byte and so must we); only errors
    # reject the corpus.
    if grep -q "Error" "$tmp/$base_name.gas.err"; then
        echo "FAIL: $base_name: GNU as reported errors" >&2
        exit 2
    fi

    if ! "$CCC" -c "$matrix" -o "$tmp/$base_name.lccc.o" 2>"$tmp/$base_name.lccc.err"; then
        echo "FAIL: $base_name: lccc rejected the matrix that GNU as accepts:" >&2
        head -20 "$tmp/$base_name.lccc.err" >&2
        fail=1
        return
    fi

    $OBJDUMP -d $objdump_flags "$tmp/$base_name.gas.o" \
        | awk '/^[ \t]+[0-9a-f]+:/{print}' > "$tmp/$base_name.gas.txt"
    $OBJDUMP -d $objdump_flags "$tmp/$base_name.lccc.o" \
        | awk '/^[ \t]+[0-9a-f]+:/{print}' > "$tmp/$base_name.lccc.txt"

    if ! diff -u "$tmp/$base_name.gas.txt" "$tmp/$base_name.lccc.txt" \
        > "$tmp/$base_name.diff"; then
        echo "FAIL: $base_name: byte/text divergence from GNU as (first 30 lines):" >&2
        head -30 "$tmp/$base_name.diff" >&2
        fail=1
        return
    fi
    local n
    n=$(wc -l < "$tmp/$base_name.gas.txt")
    echo "PASS: $base_name — $n instructions byte-identical to GNU as"
}

compare_matrix tests/regression/seg_prefix_matrix.s "" ""
compare_matrix tests/regression/seg_prefix_matrix_code32.s "--32" "-m i386"

# ── Whitespace-insensitive directive matching (the .ifb\t regression) ─────
# A tab-separated `.ifb\t<arg>` inside a macro must behave exactly like the
# space form: the historical enumeration `starts_with(".ifb ") ||
# starts_with(".ifb\t")` lost its tab variant once and silently emitted BOTH
# branches of every conditional. directive_arg() closes the class; this
# checks the whole whitespace family end-to-end.
cat >"$tmp/ws.s" <<'EOF'
.macro optarg arg2=""
.ifb	\arg2
    movl $1, %eax
.else
    movl $2, %eax
.endif
.endm
.text
.globl _start
_start:
    optarg 42
    optarg
EOF
if ! "$CCC" -c "$tmp/ws.s" -o "$tmp/ws.o" 2>"$tmp/ws.err"; then
    echo "FAIL: tab-separated .ifb macro rejected:" >&2
    cat "$tmp/ws.err" >&2
    fail=1
else
    if ! $AS -o "$tmp/ws.gas.o" "$tmp/ws.s" 2>/dev/null; then
        echo "FAIL: oracle rejected the ws corpus" >&2
        exit 2
    fi
    $OBJDUMP -d "$tmp/ws.gas.o" | awk '/^[ \t]+[0-9a-f]+:/{print}' > "$tmp/ws.gas.txt"
    $OBJDUMP -d "$tmp/ws.o" | awk '/^[ \t]+[0-9a-f]+:/{print}' > "$tmp/ws.txt"
    if ! diff -q "$tmp/ws.gas.txt" "$tmp/ws.txt" >/dev/null; then
        echo "FAIL: tab-separated .ifb macro diverges from GNU as:" >&2
        diff -u "$tmp/ws.gas.txt" "$tmp/ws.txt" >&2
        fail=1
    fi
fi

# ── Stray conditional terminators are a hard error (GAS parity) ────────────
# A conditional-family line that no arm recognized used to fall through and
# SILENTLY EMIT EVERY BRANCH — a wrong-code failure mode with no diagnostic.
# GAS errors (".else" without matching ".if"); now so do we.
cat >"$tmp/stray.s" <<'EOF'
.text
.globl _start
_start:
    movl $1, %eax
.else
    movl $2, %eax
.endif
EOF
if "$CCC" -c "$tmp/stray.s" -o "$tmp/stray.o" 2>/dev/null; then
    echo "FAIL: stray .else/.endif silently assembled (must error like GAS)" >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "FAIL: segment-prefix matrix gate" >&2
    exit 1
fi
echo "PASS: segment-prefix full matrix (64-bit + code32 + whitespace + stray-error)"
