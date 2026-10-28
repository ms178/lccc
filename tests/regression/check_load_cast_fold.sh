#!/usr/bin/env bash
# W2 Load->Cast fold: a single-use I32 load feeding an adjacent widening
# SIGN-extending cast must fold into ONE `movslq mem, reg` - not materialize
# the load canonically and re-extend (`movslq mem, %rax; movslq %eax, %rdx`).
#
# Regression class (PR #842 synthesis): the needs-sext convention stopped
# marking widening-cast SOURCES (the cast extends itself), but the fold
# admission in prologue.rs still gated I32 loads on that very marking - so
# the fold silently died for every I32 load + sext-cast pair (+50% body
# instructions on the rbtree checksum shape) while all outputs stayed
# correct. The fix makes the firing fold emit the TYPE-based extending
# opcode, so the admission no longer needs the convention.
#
# Contracts:
#   1. structural: h() contains exactly 2 memory-form movslq and no
#      reg->reg movslq;
#   2. the fold planner reports both candidates (debug channel);
#   3. unsigned control: u32->u64 keeps the plain movl zero-extend path
#      (no movslq anywhere in u());
#   4. runtime: folded loads of NEGATIVE fields produce the exact
#      precomputed values (sign-extension actually happened).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
CCC=$(readlink -f "$CCC")
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/f.c" <<'EOF'
struct node { int pad[6]; int key; int val; };
struct unode { int pad[6]; unsigned key; unsigned val; };
unsigned long long h(struct node *n) {
    return (unsigned long long)n->val ^ ((unsigned long long)n->key << 16);
}
unsigned long long u(struct unode *n) {
    return (unsigned long long)n->val ^ ((unsigned long long)n->key << 16);
}
int main(void) {
    struct node a = {{0}, -2, -3};
    struct node b = {{0}, 7, 9};
    struct unode c = {{0}, 0x80000001u, 0x90000003u};
    if (h(&a) != 0x1FFFDULL) return 1;
    if (h(&b) != 0x70009ULL) return 2;
    if (u(&c) != (0x90000003ULL ^ (0x80000001ULL << 16))) return 3;
    return 0;
}
EOF

CCC_DEBUG_LOAD_CAST_FOLD=1 "$CCC" -O2 -S "$td/f.c" -o "$td/f.s" 2> "$td/f.dbg"

h_body=$(sed -n '/^h:/,/^\.size[[:space:]]*h/p' "$td/f.s")
u_body=$(sed -n '/^u:/,/^\.size[[:space:]]*u/p' "$td/f.s")

# 1. Exactly two folded sign-extending memory loads, zero re-extensions.
n_mem=$(grep -cE 'movslq[[:space:]]+[-0-9]*\(%' <<<"$h_body" || true)
n_rr=$(grep -cE 'movslq[[:space:]]+%e' <<<"$h_body" || true)
if [ "$n_mem" -ne 2 ] || [ "$n_rr" -ne 0 ]; then
    echo "h() must fold both I32 loads into their sext casts (want 2 memory-form movslq, 0 reg->reg); got $n_mem/$n_rr:" >&2
    echo "$h_body" >&2
    exit 1
fi

# 2. The planner saw both candidates for h.
if ! grep -qE 'fn=h folds=2' "$td/f.dbg"; then
    echo "fold planner did not report 2 candidates for h():" >&2
    cat "$td/f.dbg" >&2
    exit 1
fi

# 3. Unsigned loads keep the movl zero-extend form; a movslq here would
#    sign-extend an unsigned field.
if grep -qE 'movslq' <<<"$u_body"; then
    echo "u() must not sign-extend unsigned loads:" >&2
    echo "$u_body" >&2
    exit 1
fi

# 4. Runtime proof that the folded movslq really sign-extended.
"$CCC" -O2 "$td/f.c" -o "$td/f.bin"
"$td/f.bin" || { rc=$?; echo "runtime check failed with exit $rc" >&2; exit 1; }

# Same registered cast-fold gate, separate focused harness: the shift+byte
# sign-extension fold has its own skip control and GCC runtime comparison.
CCC="$CCC" GCC="${GCC:-gcc-16}" bash "$(dirname "$0")/check_shift_signext_fold.sh"

echo "OK load_cast_fold (2 folded sext loads, debug=2, unsigned control, runtime)"
