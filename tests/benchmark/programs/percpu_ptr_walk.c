/*
 * Pointer-base per-CPU walk — the shape Linux's per_cpu_ptr() produces at -O2.
 *
 * The corpus's tls_seg_access covers glibc's CONSTANT-offset TLS accesses
 * (`movq %fs:sym+N, %r`), where the base is implied by the segment register.
 * It does NOT cover the kernel's computed per-CPU base, where every struct
 * field displacement is a constant but the base arrives in a register from
 * per_cpu_ptr()/this_cpu_ptr() and the field then feeds an ALU op:
 *
 *     acc += p->hits;                 movq %gs:8(%rsi), %rax
 *                                     addq %rax, %r8          <- not fused
 *     acc += p->hits;                 addq %gs:8(%rsi), %r8   <- fused
 *
 * The second form is what gcc emits for a segment-qualified load feeding a
 * two-operand ALU instruction (the memory operand keeps the override and the
 * scratch register disappears); this benchmark keeps that shape hot, together
 * with the marching base (`p = p->next`), so a fold that left a stale base
 * register alive would be visible here too.
 *
 * The chain is linked by per-CPU OFFSETS, exactly like the kernel's per-CPU
 * lists: offset 0 is a valid address (GS base + 0), so the terminator is
 * `CHAIN * sizeof (struct node)` — one past the array — never NULL.
 *
 * The GS base is installed with arch_prctl(ARCH_SET_GS) on private storage, so
 * the program requires Linux x86-64 GS support; accesses are segment-relative; the
 * output is pure arithmetic over that storage.
 */
#include <stdio.h>
#include <stdint.h>

#ifndef PASSES
#define PASSES 300000U
#endif
#ifndef CHAIN
#define CHAIN 16
#endif
#define END_OFF ((unsigned long)(CHAIN * sizeof (struct node)))

struct node
{
  unsigned long tag;    /* +0  */
  unsigned long hits;   /* +8  */
  unsigned long miss;   /* +16 */
  unsigned long next;   /* +24: byte offset of the next node in the GS area */
};

/* SYS_arch_prctl(ARCH_SET_GS, base), no libc wrapper. */
static long
set_gs_base (void *base)
{
  register long code __asm__ ("rdi") = 0x1001; /* ARCH_SET_GS */
  register void *addr __asm__ ("rsi") = base;
  register long nr __asm__ ("rax") = 158;      /* SYS_arch_prctl */
  __asm__ volatile ("syscall" : "+r" (nr) : "r" (code), "r" (addr)
                    : "rcx", "r11", "memory");
  return nr;
}

static struct node nodes[CHAIN] __attribute__((aligned(64)));
static volatile unsigned long seed = 1;

__attribute__((noinline)) static unsigned long
walk (unsigned long start_off, unsigned passes)
{
  unsigned long acc = 0;
  for (unsigned i = 0; i < passes; i++)
    {
      unsigned long off = start_off;
      /* Three field accesses per node: two accumulator fed ALU ops (the
         fusible shape) and one keeping the recurrence layout-independent. */
      while (off != END_OFF)
        {
          struct node __seg_gs *p = (struct node __seg_gs *)(uintptr_t)off;
          acc += p->hits;
          acc ^= p->miss;
          acc += p->tag & 7;
          off = p->next;
        }
    }
  return acc;
}

int
main (void)
{
  if (set_gs_base (nodes) != 0)
    {
      printf ("percpu-ptr-walk: no-gs\n");
      /* 77 denotes an unavailable workload. Benchmark drivers treat every
         nonzero exit as unmeasurable/failure, never a successful timing. */
      return 77;
    }
  for (unsigned i = 0; i < CHAIN; i++)
    {
      nodes[i].tag = i + seed;
      nodes[i].hits = (i * 2654435761UL) & 0xffff;
      nodes[i].miss = (i * 40503UL) & 0xff;
      nodes[i].next = (i + 1 < CHAIN) ? (unsigned long)((i + 1) * sizeof (nodes[0]))
                                      : END_OFF;
    }
  printf ("percpu-ptr-walk: %lu\n", walk (0, PASSES));
  return 0;
}
