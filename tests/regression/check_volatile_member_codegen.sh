#!/usr/bin/env bash
# A member declared `volatile` is a volatile object at every access: `s.f`,
# `p->f`, `p[i].f`, nested `p->in.f`, and members of anonymous unions. The
# memory-mapped register block idiom (`struct regs { volatile u32 ctrl; }`
# through a pointer) depends on it. lccc used to drop every such access at -O2
# (no store, no load), so a register write could vanish and a status read could
# be folded. The same holds for `*(volatile T *)addr` casts (the kernel MMIO idiom)
# and for a volatile load whose value is unused (both reads must stay).
# Non-volatile loads are not compared: lccc may fold them, as it does. A
# member's qualifier must also not leak into the next member or into the
# enclosing declaration.
#
# The check counts, per function at -O2, the stores with an immediate source
# (`mov $imm, mem`) and the memory-operand references (loads and stores). lccc
# must match the oracle on every probe, and each probe's GCC count is pinned so a
# fixture drift is reported as drift, not as a lccc bug.
set -euo pipefail
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle check_volatile_member_codegen || exit 1

CCC=${CCC:-target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_volatile_member_codegen: FAIL: compiler not found at $CCC" >&2; exit 1; }
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-vm.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
gcc_oracle_positive_control "$CCC" "$tmp" || exit 1
gcc_oracle_positive_control "$GCC_ORACLE_BIN" "$tmp" || exit 1

src=$tmp/vm.c
cat > "$src" <<'C'
struct regs { volatile unsigned ctrl; unsigned data; };
struct regs gregs;
void m_ptr_store(struct regs *r) { r->ctrl = 1; r->ctrl = 2; }
void m_obj_store(void) { struct regs r; r.ctrl = 1; r.ctrl = 2; (void)r; }
void m_global_store(void) { gregs.ctrl = 1; gregs.ctrl = 2; }
void m_arr_store(struct regs *r) { r[1].ctrl = 1; r[1].ctrl = 2; }
struct inner { volatile int v; };
struct outer { int pad; struct inner in; };
void m_nested_store(struct outer *o) { o->in.v = 1; o->in.v = 2; }
struct anon_u { union { volatile int av; int plain; }; int z; };
void m_anon_store(struct anon_u *a) { a->av = 1; a->av = 2; }
struct next_leak { volatile int a; int b; };
void m_next_member_store(void) { struct next_leak s; s.b = 1; s.b = 2; (void)s; }
void m_plain_field_store(struct regs *r) { r->data = 1; r->data = 2; }
unsigned l_vol_read(volatile struct regs *r) { unsigned a = r->ctrl; unsigned b = r->ctrl; return a + b; }
void c_cast_store(unsigned long a) { *(volatile unsigned *)a = 1; *(volatile unsigned *)a = 2; }
void c_arr_cast_store(unsigned long a) { ((volatile unsigned *)a)[1] = 1; ((volatile unsigned *)a)[1] = 2; }
void c_cast_read(unsigned long a) { unsigned x = *(volatile unsigned *)a; unsigned y = *(volatile unsigned *)a; (void)(x + y); }
void c_cast_stmt_read(unsigned long a) { *(volatile unsigned *)a; *(volatile unsigned *)a; }
void v_unused_member_read(volatile struct regs *r) { unsigned a = r->ctrl; unsigned b = r->ctrl; (void)(a + b); }
C

# Store count: a mov with an immediate source to a memory operand.
count_stores() {
    awk -v want="$2" '
        /^[A-Za-z_][A-Za-z0-9_]*:/ { fn = $0; sub(/:.*/, "", fn) }
        /^[ \t]+mov[lq]?[ \t]+\$/ && fn == want && /\(/ { n++ }
        END { print n + 0 }' "$1"
}
# Memory-operand references: every instruction that reads or writes memory.
count_mem_refs() {
    awk -v want="$2" '
        /^[A-Za-z_][A-Za-z0-9_]*:/ { fn = $0; sub(/:.*/, "", fn) }
        /^[ \t]+[a-z]+[ \t]/ && fn == want && /\(%/ { n++ }
        END { print n + 0 }' "$1"
}

fail=0
"$GCC_ORACLE_BIN" -O2 -S -o "$tmp/gcc.s" "$src"
"$CCC" -O2 -S "$src" -o "$tmp/lccc.s"

# function:metric:pinned-GCC-count. Store probes count immediate stores; load
# probes count memory-operand references.
for probe in m_ptr_store:store:2 m_obj_store:store:2 m_global_store:store:2 \
    m_arr_store:store:2 m_nested_store:store:2 m_anon_store:store:2 \
    m_next_member_store:store:0 m_plain_field_store:store:1 \
    l_vol_read:mem:2 c_cast_store:store:2 c_arr_cast_store:store:2 c_cast_read:mem:2 \
    c_cast_stmt_read:mem:2 v_unused_member_read:mem:2; do
    rest=${probe#*:}
    fn=${probe%%:*}
    metric=${rest%%:*}
    want=${rest##*:}
    if [[ $metric == store ]]; then
        g=$(count_stores "$tmp/gcc.s" "$fn")
        l=$(count_stores "$tmp/lccc.s" "$fn")
    else
        g=$(count_mem_refs "$tmp/gcc.s" "$fn")
        l=$(count_mem_refs "$tmp/lccc.s" "$fn")
    fi
    if [[ $g != "$want" ]]; then
        echo "FAIL $fn: GCC $GCC_ORACLE_MAJOR $metric count is $g, the probe pins $want (fixture drift)" >&2
        fail=$((fail + 1))
    elif [[ $l != "$g" ]]; then
        echo "FAIL $fn: lccc $metric count is $l, GCC $GCC_ORACLE_MAJOR keeps $g" >&2
        sed -n "/^$fn:/,/\.size $fn,/p" "$tmp/lccc.s" >&2
        fail=$((fail + 1))
    fi
done
if (( fail > 0 )); then
    echo "check_volatile_member_codegen: FAILED ($fail)" >&2
    exit 1
fi
echo "check_volatile_member_codegen: PASSED (15 functions, store and memory-reference counts match GCC $GCC_ORACLE_MAJOR)"
