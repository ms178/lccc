/*
 * Workload-derived kernel: glibc 2.44 TLS access patterns (THREAD_SELF /
 * THREAD_GETMEM / THREAD_SETMEM shapes from sysdeps/x86_64/nptl/tls.h).
 *
 * Hot glibc paths (errno, malloc arenas, stack guard, pthread self) are
 * segment-relative loads/stores at constant %fs offsets, mixed with
 * ordinary loads through the pd pointer fetched from %fs:16. Three distinct
 * miscompile classes were found and fixed in exactly this pattern mix:
 *   1. post-phi const-copy removal orphaning Load %fs:(const) (S02),
 *   2. emit_seg_store operand conflation (ptr home clobbered by value),
 *   3. peephole lea-fold deleting a LEA whose dest is the store SOURCE
 *      (GVN CSEs &pd->member for both address and value).
 * This benchmark keeps all three shapes hot AND measures the cost of the
 * segment-override addressing the backend chooses.
 *
 * Real TLS (via __thread) is used so the binary runs anywhere; the compiler
 * lowers the accesses through the same segment-register machinery glibc's
 * macros hand-write.
 */
#include <stdio.h>

#define PASSES 200000U
#define LANES 64

static __thread unsigned long tls_slots[LANES + 2];

static __thread unsigned long *tls_indirect;

__attribute__((noinline)) static unsigned long
tls_pass (unsigned p)
{
  unsigned long acc = 0;
  /* Address-of-TLS taken and STORED (pointer-typed TLS slot): the &slot
     value is both a stored value and a load base later — the GVN-CSE +
     lea-fold shape from __tls_init_tp — but the address itself never
     enters the arithmetic, so output is layout-independent. */
  tls_indirect = &tls_slots[1];
  tls_slots[1] = p;
  for (int i = 2; i <= LANES; i++)
    {
      tls_slots[i] = tls_slots[i - 1] + p;
      acc += tls_slots[i] ^ *(volatile unsigned long *) &tls_slots[(i & 7) + 1];
    }
  /* Read back through the pointer-typed TLS slot. */
  return acc + *tls_indirect;
}

int
main (void)
{
  unsigned long total = 0;
  for (unsigned p = 0; p < PASSES; p++)
    total += tls_pass (p | 1);
  printf ("tls-seg-access: %lu\n", total);
  return 0;
}
