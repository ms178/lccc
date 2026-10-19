#!/usr/bin/env bash
# Separate-TU vector rvalue indexing, both cross-compiler directions.
# --self-only is an explicit local smoke test, NOT cross-ABI validation.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
[[ -x "$ccc" ]] || { echo "FAIL: compiler not built: $ccc" >&2; exit 1; }
compilers=("$ccc")
case ${1:-} in
    --self-only) ;;
    '')
        . "$root/tests/regression/lib/clang_oracle.sh"
        require_clang_oracle check_ext_vector_rvalue_subscript
        compilers+=("$CLANG_ORACLE_BIN") ;;
    *) echo "usage: $0 [--self-only]" >&2; exit 1 ;;
esac
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
src=$root/tests/regression/ext_vector_rvalue_subscript.c
# A freestanding i386 entry point avoids any multilib/libc prerequisite.
# Align the call frame, preserve main's status, and exit via the Linux ABI.
cat >"$work/start.s" <<'ASM'
.globl _start
_start:
    andl $-16, %esp
    call main
    movl %eax, %ebx
    movl $1, %eax
    int $0x80
.section .note.GNU-stack,"",@progbits
ASM
"$ccc" -m32 -c "$work/start.s" -o "$work/start.o"
for bits in 64 32; do
    link_flags=(-m"$bits")
    if [[ $bits == 32 ]]; then
        link_flags+=(-nostdlib -static "$work/start.o")
    fi
    for opt in -O0 -O1 -O2; do
        for p in "${!compilers[@]}"; do
            "${compilers[p]}" -m"$bits" "$opt" -DVECTOR_PROVIDER -c "$src" -o "$work/provider$p.o"
            "${compilers[p]}" -m"$bits" "$opt" -DVECTOR_CALLER -c "$src" -o "$work/caller$p.o"
        done
        for p in "${!compilers[@]}"; do
            for c in "${!compilers[@]}"; do
                "$ccc" "${link_flags[@]}" "$work/provider$p.o" "$work/caller$c.o" -o "$work/test"
                rc=0
                "$work/test" || rc=$?
                if [[ $rc != 0 ]]; then
                    detail="rvalue subscript m$bits $opt provider=$p caller=$c status=$rc (shape=$(((rc-1)/4+1)), consumer=$(((rc-1)%4+1)))"
                    echo "FAIL: $detail" >&2
                    if [[ ${GITHUB_ACTIONS:-} == true ]]; then
                        echo "::error title=Vector rvalue cross-ABI::$detail"
                    fi
                    exit 1
                fi
            done
        done
    done
done

# --- 2. Reversed form and statement-expression value category -------------
# `0[f()]` reads lane 0 of the call result (the integer is the index, not
# the base); it segfaulted even after the materialization landed because
# the subscript normalizer only knew pointers/arrays. Statement
# expressions keep their tail's value category in both oracles, so
# `({ v; })[0] = x` writes into v -- pinned at -O0/-O1/-O2.
cat >"$work/self_forms.c" <<'EOF'
typedef double d1 __attribute__((ext_vector_type(1)));
d1 f(void) { return (d1){3.0}; }
int main(void) {
    if ((int)0[f()] != 3)
        return 10;
    d1 v = {13.0};
    ({ v; })[0] = 17.0;
    if ((int)v[0] != 17)
        return 11;
    return 0;
}
EOF
for opt in -O0 -O1 -O2; do
    "$ccc" "$opt" "$work/self_forms.c" -o "$work/self_forms"
    "$work/self_forms" || {
        rc=$?
        echo "FAIL: reversed-form/stmt-expr rvalue subscript at $opt (rc=$rc)" >&2
        exit 1
    }
done

# --- 3. Oracle-diagnosed rvalue lane stores --------------------------------
# A lane of a vector rvalue is not an lvalue: the materialized temporary
# dies at the end of the full expression, so a store must be DIAGNOSED,
# never silently discarded. GCC says "lvalue required as left operand of
# assignment", Clang "expression is not assignable"; lccc pins the second
# spelling for all eight forms.
diag_header='typedef double d1 __attribute__((ext_vector_type(1)));
typedef _Bool bb17 __attribute__((ext_vector_type(17)));
d1 f(void){return (d1){3.0};}
bb17 fb(void){bb17 r={}; return r;}
d1 v={3.0}, w={5.0};
int c;'
rejects=(
    'f()[0] = 9.0;'
    'f()[0] += 2.0;'
    'f()[0]++;'
    'double *p = &f()[0]; (void)p;'
    'fb()[3] = 1;'
    '(c?v:w)[0] = 9.0;'
    '(0,v)[0] = 9.0;'
    '((d1)v)[0] = 9.0;'
)
n=0
for body in "${rejects[@]}"; do
    n=$((n + 1))
    printf '%s\nint main(void){ %s return 0; }\n' "$diag_header" "$body" \
        >"$work/reject-$n.c"
    if out=$("$ccc" -O2 "$work/reject-$n.c" -o "$work/reject-$n" 2>&1); then
        echo "FAIL: accepted invalid rvalue-lane store: $body" >&2
        exit 1
    fi
    grep -q 'not assignable' <<<"$out" || {
        echo "FAIL: wrong diagnostic for '$body': $(head -1 <<<"$out")" >&2
        exit 1
    }
done

echo "PASS: vector rvalue subscript (${#compilers[@]} compiler(s), separate TUs, -O0/-O1/-O2, m64/m32; reversed form; stmt-expr write-through; ${n} oracle-rejected store forms)"
