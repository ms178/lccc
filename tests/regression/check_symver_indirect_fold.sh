#!/usr/bin/env bash
# Asm-level oracle for versioned-symbol indirect-call folding (x86-64).
#
# lccc renames references bound by a top-level `.symver real, name@VER`
# directive to the versioned symbol (symver_ref_map). When such a reference
# is the VALUE of an indirect call target (a local function pointer whose
# initializer is the .symver'd function), the call-target resolver sees
# GlobalAddr("name@VER"). Folding it is NOT expressible:
#   - `call *name@VER+off(%rip)` is rejected by GAS 2.47;
#   - stripping to the base name re-binds the call to the DEFAULT version —
#     a silent .symver semantic change;
#   - the GOT/TLS/absolute guards key on full symbol names.
# The resolver must REJECT the fold: the call or sibling tail jump goes
# through the loaded pointer value (GOTPCREL load + branch *%r10), which is
# correct for every binding.
#
# Positive control: an unversioned local-pointer call DOES fold — to a
# direct call/jump, or (current behaviour) all the way to an inlined body
# with no transfer left. Either way it must not remain value-based indirect:
# that proves the resolver still folds the ordinary shapes.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
lccc=${CCC_X64:-$repo/target/fastbuild/lccc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cat > "$tmp/t.c" <<'EOF'
__asm__(".symver impl_v1, target_fn@VER1.0");
extern int impl_v1(int);
static int plain_fn(int x) { return x + 1; }
int versioned_local_ptr(int x) {
    int (*p)(int) = impl_v1;
    return p(x);            /* must NOT fold: GlobalAddr target_fn@VER1.0 */
}
int plain_local_ptr(int x) {
    int (*q)(int) = plain_fn;
    return q(x);            /* must fold: local symbol, direct call */
}
EOF

"$lccc" -O2 -S "$tmp/t.c" -o "$tmp/t.s"

ver_body=$(sed -n '/^versioned_local_ptr:/,/^\t\.size.*versioned_local_ptr/p' "$tmp/t.s")
plain_body=$(sed -n '/^plain_local_ptr:/,/^\t\.size.*plain_local_ptr/p' "$tmp/t.s")

fail=0
if grep -qE '(call|jmp)q? +\*?target_fn(@VER|@PLT)?\(' <<<"$ver_body" \
   || grep -qE '(call|jmp)q? +target_fn(@PLT|@VER)' <<<"$ver_body"; then
    echo "FAIL  versioned indirect call was folded (direct/memory-indirect target_fn form)"
    fail=$((fail+1))
fi
if ! grep -qE '(call|jmp)q? +\*%r10' <<<"$ver_body"; then
    echo "FAIL  versioned indirect call does not go through the loaded pointer (r10)"
    fail=$((fail+1))
fi
# Folded means: not value-based any more. Accept a direct `call plain_fn`
# or a fully inlined body (no branch to the callee at all); reject any
# surviving indirect transfer or GOT load of the callee.
if grep -qE '(call|jmp)q? +\*%' <<<"$plain_body" \
   || grep -q 'plain_fn@GOTPCREL' <<<"$plain_body"; then
    echo "FAIL  positive control: unversioned local-pointer call did not fold"
    fail=$((fail+1))
fi

if [[ $fail -gt 0 ]]; then
    echo "RESULT: FAIL"
    sed -n '/^versioned_local_ptr:/,/^\t\.size.*versioned_local_ptr/p' "$tmp/t.s"
    exit 1
fi
echo "ok    versioned indirect call unfolded (value-based, r10)"
echo "ok    unversioned local-pointer call folded (positive control)"
echo "RESULT: PASS"
