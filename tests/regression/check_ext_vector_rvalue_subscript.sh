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

# --- 2. Statement-expression value category (GNU vectors) ----------------
# GCC, which defines statement expressions, gives `({ v; })` its tail's value
# category, so `({ v; })[0] = x` writes into a GNU `vector_size` v -- pinned
# at -O0/-O1/-O2. (Clang treats the statement expression as an rvalue; for
# Clang's own ext vectors lccc follows Clang, see section 3.)
cat >"$work/self_forms.c" <<'EOF'
typedef double g1 __attribute__((vector_size(8)));
int main(void) {
    g1 v = {13.0};
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
        echo "FAIL: stmt-expr GNU vector subscript store at $opt (rc=$rc)" >&2
        exit 1
    }
done

# --- 3. Oracle-diagnosed rvalue lane forms --------------------------------
# A lane or selection of a vector rvalue is not an lvalue: the materialized
# temporary dies at the end of the full expression, so a store must be
# DIAGNOSED, never silently discarded. Each form carries Clang 23.1.3's
# first diagnostic (ext vectors are Clang's extension): a store says
# "expression is not assignable", `&` "cannot take the address of an
# rvalue", and the reversed `0[v]` is no subscript at all (commutativity is
# pointer + integer only; GCC rejects it for GNU vectors too).
diag_header='typedef double d1 __attribute__((ext_vector_type(1)));
typedef float f4 __attribute__((ext_vector_type(4)));
typedef _Bool bb17 __attribute__((ext_vector_type(17)));
struct S { f4 v; };
d1 f(void){return (d1){3.0};}
f4 f4f(void){f4 r={}; return r;}
bb17 fb(void){bb17 r={}; return r;}
struct S sf(void){struct S s={}; return s;}
d1 v={3.0}, w={5.0}; f4 a;
int c;'
assignable='expression is not assignable'
rvalue_addr='cannot take the address of an rvalue of type'
reversed='subscripted value is'
rejects=(
    "f()[0] = 9.0;|$assignable"
    "f()[0] += 2.0;|$assignable"
    "f()[0]++;|$assignable"
    "double *p = &f()[0]; (void)p;|$rvalue_addr 'double'"
    "fb()[3] = 1;|$assignable"
    "(c?v:w)[0] = 9.0;|$assignable"
    "(0,v)[0] = 9.0;|$assignable"
    "((d1)v)[0] = 9.0;|$assignable"
    "({ v; })[0] = 9.0;|$assignable"
    "f().x = 9.0;|$assignable"
    "f4f().yx = a.xy;|$assignable"
    "f4f().x += 1;|$assignable"
    "++f4f().w;|$assignable"
    "sf().v.z = 1;|$assignable"
    "({ a; }).x = 1;|$assignable"
    "float *p = &f4f().y; (void)p;|$rvalue_addr 'float'"
    "float *p = &sf().v[2]; (void)p;|$rvalue_addr 'float'"
    "double d = 0[v]; (void)d;|$reversed"
    "double d = 0[f()]; (void)d;|$reversed"
)
n=0
for entry in "${rejects[@]}"; do
    n=$((n + 1))
    body=${entry%%|*}
    want=${entry#*|}
    printf '%s\nint main(void){ %s return 0; }\n' "$diag_header" "$body" \
        >"$work/reject-$n.c"
    if out=$("$ccc" -O2 "$work/reject-$n.c" -o "$work/reject-$n" 2>&1); then
        echo "FAIL: accepted invalid rvalue-lane form: $body" >&2
        exit 1
    fi
    grep -m1 'error:' <<<"$out" | grep -qF "$want" || {
        echo "FAIL: wrong diagnostic for '$body' (want '$want'): $(grep -m1 'error:' <<<"$out")" >&2
        exit 1
    }
done

echo "PASS: vector rvalue subscript (${#compilers[@]} compiler(s), separate TUs, -O0/-O1/-O2, m64/m32; GNU stmt-expr write-through; ${n} oracle-rejected rvalue-lane forms)"
