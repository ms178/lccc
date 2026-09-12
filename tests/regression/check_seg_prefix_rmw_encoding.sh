#!/usr/bin/env bash
# Assembler segment-prefix gate for RMW memory encodings (kernel 6.18.50
# preempt_count boot killer):
#
# The kernel's per-CPU unary ops lower to `asm("incl %[var]" : [var] "+m"
# (__my_cpu_var(__preempt_count)))`; the template text correctly carries
# `%gs:` (frontend AS walk), but the integrated assembler's ENCODER arms
# silently dropped the override on exactly the unary/RMW class:
#
#   * encode_inc_dec      (inc/dec with size suffix)
#   * encode_unary_rm     (suffixless inc/dec/neg/not, mul/div/idiv)
#   * encode_xchg         (this_cpu_xchg)
#   * encode_xadd         (this_cpu_add_return / lock xadd)
#
# 1,125 unprefixed `incl/decl __preempt_count(%rip)` sites vs 87 correct
# binary-`addl` sites in the 6.18.50 vmlinux: every preempt_disable()/
# preempt_enable() pair incremented the STATIC percpu image while every
# read stayed %gs-correct — the per-CPU counter never moved, and the boot
# died at PID 1 (finish_task_switch "corrupted preempt_count: .../0x1",
# workqueue "leaked atomic" BUGs, page-fault cascade).
#
# This gate compiles the exact shapes through the INTEGRATED ASSEMBLER
# (-c, not -S: the .s text path always printed the prefix — the encoder
# is the defect) and byte-checks the 0x65 override in the objects.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-seg-prefix-rmw.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat >"$tmp/t.c" <<'C'
extern __seg_gs int pvar;
void inc_gs(void)   { asm volatile("incl %[var]"   : [var] "+m"(pvar)); }
void dec_gs(void)   { asm volatile("decl %[var]"   : [var] "+m"(pvar)); }
void not_gs(void)   { asm volatile("notl %[var]"   : [var] "+m"(pvar)); }
void neg_gs(void)   { asm volatile("negl %[var]"   : [var] "+m"(pvar)); }
void add_gs(void)   { asm volatile("addl $7, %[var]" : [var] "+m"(pvar)); }
void xchg_gs(int n) { asm volatile("xchgl %[var], %[nv]"
                                 : [var] "+m"(pvar), [nv] "+r"(n) :: "memory"); }
void xadd_gs(int n) { asm volatile("lock xaddl %[nv], %[var]"
                                 : [nv] "+r"(n), [var] "+m"(pvar) :: "memory"); }
void incq_gs(void)  { asm volatile("incq %[var]"   : [var] "+m"(pvar)); }
C

"$CCC" -O2 -c -o "$tmp/t.o" "$tmp/t.c"

# Every RMW on the per-CPU object must carry the 0x65 (%gs) override byte.
check() { # check <fn> <count-of-0x65-expected>
    local fn=$1 want=$2
    local got
    got=$(objdump -d --disassemble="$fn" "$tmp/t.o" 2>/dev/null \
          | grep -cE '^\s+[0-9a-f]+:\s+65' || true)
    if [[ "$got" -lt "$want" ]]; then
        echo "FAIL: $fn: %gs override missing (found $got instruction(s) with 0x65, want >= $want)" >&2
        objdump -d --disassemble="$fn" "$tmp/t.o" >&2 || true
        exit 1
    fi
}

check inc_gs 1
check dec_gs 1
check not_gs 1
check neg_gs 1
check add_gs 1
check xchg_gs 1
check xadd_gs 1
check incq_gs 1

# Negative control: no override on an ordinary (non-AS) variable.
cat >"$tmp/n.c" <<'C'
int plain;
void inc_plain(void) { asm volatile("incl %[var]" : [var] "+m"(plain)); }
C
"$CCC" -O2 -c -o "$tmp/n.o" "$tmp/n.c"
if objdump -d --disassemble=inc_plain "$tmp/n.o" 2>/dev/null | grep -qE '^\s+[0-9a-f]+:\s+65'; then
    echo "FAIL: %gs override sprayed onto a plain variable" >&2
    exit 1
fi

echo "PASS: RMW encoder segment-prefix (incl/decl/not/neg/addl/xchg/xadd)"
