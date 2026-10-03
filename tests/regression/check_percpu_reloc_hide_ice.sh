#!/usr/bin/env bash
# Kernel tracing_iter_reset ICE (session 2026-08-29):
#
#   per_cpu_ptr(iter->array_buffer->data, cpu)->skipped_entries = 0
#
# lowers as RELOC_HIDE empty-asm (stack-homed ptr) + indexed-sym load of
# `__per_cpu_offset` (register-homed) + Add + const GEP + store.
# MachInst selected the Add as `Lea { base: Vreg(asm_out), index: Phys }`.
# The Vreg is unresolvable (the slot holds the pointer VALUE, not an
# address to SIB-index), so flush_machinst replayed with empty fold maps
# and operand_to_rax panicked on a homeless value.
#
# Gate: this kernel-shaped TU must compile, and the store must go through
# `base + per_cpu_offset[cpu] + 72`.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-percpu-reloc-hide.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cat >"$tmp/t.c" <<'C'
extern unsigned long __per_cpu_offset[];

struct percpu_data {
    char pad[72];
    unsigned long skipped_entries;
};
struct array_buffer {
    void *buffer;
    struct percpu_data *data;
};
struct trace_iterator {
    void *pad0;
    void *pad1;
    struct array_buffer *array_buffer;
};

#define RELOC_HIDE(ptr, off)					\
  ({ unsigned long __ptr;					\
     __asm__ ("" : "=r"(__ptr) : "0"(ptr));		\
     (typeof(ptr)) (__ptr + (off)); })

void tracing_iter_reset(struct trace_iterator *iter, int cpu)
{
    struct percpu_data *p = RELOC_HIDE(iter->array_buffer->data, __per_cpu_offset[cpu]);
    p->skipped_entries = 0;
}
C
# Kernel-like flags that hit the ICE in the full TU (mcmodel=kernel
# indexed-sym load of __per_cpu_offset into a GPR, frame pointer on).
"$CCC" -O2 -mcmodel=kernel -fno-pic -fno-PIE -mno-sse -mno-red-zone \
    -fno-omit-frame-pointer -c "$tmp/t.c" -o "$tmp/t.o"
"$CCC" -O2 -mcmodel=kernel -fno-pic -fno-PIE -mno-sse -mno-red-zone \
    -fno-omit-frame-pointer -S "$tmp/t.c" -o "$tmp/t.s"
# Indexed load of the per-cpu offset table, then a store at +72 (0x48).
grep -q '__per_cpu_offset' "$tmp/t.s"
grep -Eq 'movq[[:space:]]+\$0,[[:space:]]*72\(|movq[[:space:]]+\$0,[[:space:]]*\(|leaq[[:space:]]+72\(' "$tmp/t.s"
