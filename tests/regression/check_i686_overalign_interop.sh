#!/usr/bin/env bash
# Four-direction interop oracle for the i686 (cdecl -m32) over-aligned
# struct stack-argument ABI: lccc and system GCC are caller AND callee, so
# every combination must agree on the 4-granular argument layout that GCC
# 14.2 -m32 uses (GCC never aligns stack args beyond PARM_BOUNDARY; the
# `aligned` attribute only rounds the TYPE SIZE). Shapes covered:
#   - aligned(32)/24-byte struct at natural offset 12 (== 4 mod 8) after a
#     3-int prefix — the divergence case for any >4 alignment cap;
#   - aligned(16)/12-byte struct at the same natural offset;
#   - the variadic side: over-aligned struct named params (va_start's
#     overflow offset) followed by two int varargs;
#   - anchor-parity sweep: 0..7 leading int args so the struct lands on
#     every natural-offset phase mod 32.
# Runs each linked binary under $RUNNER (qemu-i386) when the host cannot
# execute ELF32 natively.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
lccc=${CCC_I686:-$repo/target/fastbuild/lccc}
cc=${CC:-gcc}
runner=${LCCC_I686_RUNNER:-}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if [[ -z "$runner" ]]; then
    # Native ELF32 probe: a tiny gcc -m32 binary exiting 42.
    cat > "$tmp/probe.c" <<'PEOF'
void _start(void) {
    __asm__ volatile ("int $0x80" : : "a"(1), "b"(42) : "memory");
    __builtin_unreachable();
}
PEOF
    if "$cc" -m32 -O2 -fno-pic -nostdlib -static -Wl,-e,_start \
            "$tmp/probe.c" -o "$tmp/probe.bin" 2>/dev/null; then
        "$tmp/probe.bin" >/dev/null 2>&1 && rc=$? || rc=$?
        if [[ $rc -ne 42 ]]; then
            for cand in qemu-i386 qemu-i386-static; do
                if command -v "$cand" >/dev/null 2>&1; then
                    "$cand" "$tmp/probe.bin" >/dev/null 2>&1 && rc2=$? || rc2=$?
                    if [[ $rc2 -eq 42 ]]; then runner="$cand"; break; fi
                fi
            done
        fi
    fi
fi

run_bin() {  # run_bin <binary> [expected-exit]
    local bin=$1 want=${2:-0}
    if [[ -n "$runner" ]]; then
        set +e; "$runner" "$bin" >/dev/null 2>&1; local rc=$?; set -e
    else
        set +e; "$bin" >/dev/null 2>&1; local rc=$?; set -e
    fi
    [[ $rc -eq $want ]] || { echo "  RUN FAIL: $bin exited $rc (want $want)"; return 1; }
}

gen_callee() {  # gen_callee <align> <prefix-ints> -> callee .c on stdout
    local align=$1 nints=$2
    local params="" callargs=""
    for ((i=0; i<nints; i++)); do params+="int a$i, "; done
    cat <<CEOF
typedef struct { __attribute__((aligned($align))) char p[$((align - 8))]; } A;
int callee($params A x, ...) {
    unsigned *w = (unsigned *)x.p;
    for (int i = 0; i < $(($align / 4 - 2)); i++)
        if (w[i] != 0x70000u + (unsigned)i) return 0;
    __builtin_va_list ap;
    __builtin_va_start(ap, x);
    unsigned v1 = __builtin_va_arg(ap, unsigned);
    unsigned v2 = __builtin_va_arg(ap, unsigned);
    __builtin_va_end(ap);
    return v1 == 0x5a5a5a5au && v2 == 0xc3c3c3c3u;
}
CEOF
}

decl_callee() {  # decl_callee <align> <prefix-ints> -> callee prototype on stdout
    local align=$1 nints=$2
    local params=""
    for ((i=0; i<nints; i++)); do params+="int a$i, "; done
    echo "int callee($params A x, ...);"
}

gen_caller() {  # gen_caller <align> <prefix-ints> -> caller .c on stdout
    local align=$1 nints=$2
    local args=""
    for ((i=0; i<nints; i++)); do args+="$i, "; done
    args+="s, 0x5a5a5a5au, 0xc3c3c3c3u"
    cat <<CEOF
typedef struct { __attribute__((aligned($align))) char p[$((align - 8))]; } A;
$(decl_callee "$align" "$nints")
void _start(void) {
    A s;
    unsigned *w = (unsigned *)s.p;
    for (int i = 0; i < $(($align / 4 - 2)); i++)
        w[i] = 0x70000u + (unsigned)i;
    int r = callee($args);
    __asm__ volatile ("int \$0x80" : : "a"(1), "b"(r ? 42 : 1) : "memory");
    __builtin_unreachable();
}
CEOF
}

fail=0
run_matrix() {  # run_matrix <align> <nints> <label>
    local align=$1 nints=$2 label=$3 ct gt
    for ct in l g; do
        for gt in l g; do
            gen_callee "$align" "$nints" > "$tmp/callee.c"
            gen_caller "$align" "$nints" > "$tmp/caller.c"
            if [[ "$ct" == l ]]; then
                "$lccc" -m32 -O2 -fno-pic -nostdlib -c "$tmp/callee.c" -o "$tmp/callee_$ct.o"
            else
                "$cc" -m32 -O2 -fno-pic -nostdlib -c "$tmp/callee.c" -o "$tmp/callee_$ct.o"
            fi
            if [[ "$gt" == l ]]; then
                "$lccc" -m32 -O2 -fno-pic -nostdlib -c "$tmp/caller.c" -o "$tmp/caller_$gt.o"
            else
                "$cc" -m32 -O2 -fno-pic -nostdlib -c "$tmp/caller.c" -o "$tmp/caller_$gt.o"
            fi
            local bin="$tmp/mix_${ct}${gt}.bin"
            if [[ "$gt" == l ]]; then
                "$lccc" -m32 -nostdlib -static -Wl,-e,_start \
                    "$tmp/caller_$gt.o" "$tmp/callee_$ct.o" -o "$bin"
            else
                "$cc" -m32 -nostdlib -static -Wl,-e,_start \
                    "$tmp/caller_$gt.o" "$tmp/callee_$ct.o" -o "$bin"
            fi
            if ! run_bin "$bin" 42; then
                echo "FAIL  $label callee=$ct caller=$gt"
                fail=$((fail+1))
            fi
        done
    done
    echo "ok    $label (4 combos)"
}

echo "i686 over-aligned struct-arg interop (runner: ${runner:-native})"
run_matrix 32 3 "A32 @ natural offset 12 (4 mod 8)"
run_matrix 16 3 "A16 @ natural offset 12 (4 mod 8)"
run_matrix 32 1 "A32 @ natural offset 8"
run_matrix 32 5 "A32 @ natural offset 24 (8 mod 32)"
run_matrix 32 7 "A32 @ natural offset 32 (0 mod 32)"

if [[ $fail -gt 0 ]]; then
    echo "RESULT: $fail combo(s) FAILED"
    exit 1
fi
echo "RESULT: PASS (20 combos)"
