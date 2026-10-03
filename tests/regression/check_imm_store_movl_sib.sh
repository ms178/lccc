#!/usr/bin/env bash
# IS-24 + IS-15 structural checks: immediate stores through register-homed
# pointers, and I32 SIB loads without a 64-bit consumer emit `movl`.
#
# 1. `s->a = 1; s->a = 2; s->b = 3;` on a pointer PARAMETER: the field
#    stores must be immediate-form `movl $imm, off(%reg)` — the
#    `movq $imm, %rax; movl %eax, (reg)` accumulator round-trip is what
#    IS-24 removed for SlotAddr::Reg bases.
# 2. `window[i]` loaded into an I32 with no 64-bit consumer must be
#    `movl (%rcx,%rdi,4)` (or any GPR), not `movslq` (IS-15).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/i.c" <<'EOF'
struct S { int a, b; };
void g(struct S *s) { s->a = 1; s->a = 2; s->b = 3; }
extern int window[65536];
int load(int i) { return window[i]; }
EOF

"$CCC" -O2 -S "$td/i.c" -o "$td/i.s"
g_body=$(sed -n '/^g:/,/^\.size[[:space:]]*g/p' "$td/i.s")

# IS-24: every store in g() must be an immediate store to memory.
n_stores=$(grep -cE 'mov[a-z]*[[:space:]]+\$[0-9]+,[[:space:]]*[0-9]*\(%' <<<"$g_body" || true)
n_rax=$(grep -cE 'movq[[:space:]]+\$[0-9]+,[[:space:]]*%rax' <<<"$g_body" || true)
if [ "$n_stores" -lt 2 ] || [ "$n_rax" -gt 0 ]; then
    echo "g() should use immediate stores through the register-homed pointer; got:" >&2
    echo "$g_body" >&2
    exit 1
fi

# IS-15: the indexed I32 load has no 64-bit consumer -> movl, not movslq.
load_body=$(sed -n '/^load:/,/^\.size[[:space:]]*load/p' "$td/i.s")
if grep -qE 'movslq[[:space:]]+\(' <<<"$load_body"; then
    echo "load() should emit movl for the I32 SIB load (no 64-bit consumer); got:" >&2
    echo "$load_body" >&2
    exit 1
fi
if ! grep -qE 'movl[[:space:]]+\(%[a-z0-9]+,[[:space:]]*%[a-z0-9]+,[[:space:]]*4\)' <<<"$load_body"; then
    echo "load() should emit a scale-4 SIB movl load; got:" >&2
    echo "$load_body" >&2
    exit 1
fi
echo "OK imm_store_and_movl_sib"
