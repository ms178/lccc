/* Kernel-style per-CPU access through the GS segment, end to end.
 *
 * WHY THIS TEST EXISTS
 * --------------------
 * Linux accesses per-CPU data as `%gs:offset`, where the offset is a
 * *computed* base (per_cpu_ptr's result) plus a constant struct displacement:
 *
 *     this_cpu_read(x)            -> %gs:x(%rip)
 *     per_cpu_ptr(&pcp, cpu)->f   -> GEP(per_cpu_ptr(...), +off) then load
 *
 * The second shape used to materialise `base + off` into a scratch register
 * first (`leaq 8(%rax), %rcx ; movq %gs:(%rcx), %rax`) where GCC and Clang fold
 * the displacement into the memory operand (`movq %gs:8(%rax), %rax`). The
 * codegen fold is gated by check_seg_const_offset_fold.sh; THIS test is the
 * behavioural half: it installs a real GS base with arch_prctl(ARCH_SET_GS)
 * and reads/writes through `struct S __seg_gs *` offsets, so a wrong
 * displacement (a fold that fires with a stale or wrong offset) cannot pass.
 *
 * Everything is arithmetic on a private 64-byte-aligned buffer, so the program
 * is deterministic and prints the same lines under lccc, GCC and Clang.
 */
#include <stdio.h>
#include <stdint.h>

struct S { long a; long b; long c[4]; };

/* The argument is an OFFSET into the GS segment, exactly like per_cpu_ptr(). */
static long read_b(uintptr_t off) { return ((struct S __seg_gs *)off)->b; }
static void write_c3(uintptr_t off, long v) { ((struct S __seg_gs *)off)->c[3] = v; }
static long read_c0(uintptr_t off) { return ((struct S __seg_gs *)off)->c[0]; }

/* Base RETURNED FROM A CALL: the pointer arrives in %rax and the fold must
 * still happen (`call base_at; movq %gs:8(%rax), %rax`), which is the shape
 * `per_cpu_ptr()` produces at -O2 when it is not inlined.  This is the half of
 * the contract the accumulator (no register home, no stack slot) exercises: the
 * displacement is folded into the operand at the call result, and the address
 * materialisation disappears. */
static struct S __seg_gs *base_at(uintptr_t off) { return (struct S __seg_gs *)off; }
static long read_b_call(uintptr_t off) { return base_at(off)->b; }
static void write_c3_call(uintptr_t off, long v) { base_at(off)->c[3] = v; }

/* INDEXED per-CPU access (`per_cpu(arr, cpu)[i]`): the address is `base` plus a
 * scaled index plus a constant, and GCC still folds all of it into the segment
 * operand (`movq %gs:16(%rax,%rdx,8), %rax`).  The index-scaled GEP is the shape
 * whose producer may now be dropped in favour of the fold, so it must be
 * exercised behaviourally as well as textually. */
static long read_ci(uintptr_t off, long i) { return ((struct S __seg_gs *)off)->c[i]; }

/* Scalar FP member: the load must stay in the SSE domain
 * (`movsd %gs:8(%rax), %xmm0`); a GPR round trip is both slower and a
 * different instruction, so the printed bits pin the value end to end. */
struct F { long a; double d; };
static double read_d(uintptr_t off) { return ((struct F __seg_gs *)off)->d; }

/* SYS_arch_prctl(ARCH_SET_GS, base), without a libc wrapper. */
static long set_gs_base(void *base) {
    register long code __asm__("rdi") = 0x1001; /* ARCH_SET_GS */
    register void *addr __asm__("rsi") = base;
    register long nr __asm__("rax") = 158;      /* SYS_arch_prctl */
    __asm__ volatile("syscall" : "+r"(nr) : "r"(code), "r"(addr) : "rcx", "r11", "memory");
    return nr;
}

#define N 8
static uintptr_t storage[N] __attribute__((aligned(64)));

int main(void) {
    if (set_gs_base(storage) != 0) {
        printf("arch_prctl(ARCH_SET_GS) failed\n");
        return 2;
    }
    for (int i = 0; i < N; i++) {
        storage[i] = 0x1111UL * (unsigned long)(i + 1);
    }

    /* Layout: a=0, b=8, c[0..3]=16,24,32,40, so a base at `off` addresses byte
     * off+8 for b and off+16+8*i for c[i].  storage[k] starts at byte 8*k and
     * initially holds 0x1111*(k+1):
     *   storage = 1111 2222 3333 4444 5555 6666 7777 8888
     * Every expectation below is a byte offset resolved against that table, so
     * a fold that fires with a wrong displacement changes a printed value. */
    printf("read_b(0)   = %ld\n", read_b(0)); /* 0+8   = 0x2222 */
    printf("read_b(24)  = %ld\n", read_b(24)); /* 24+8  = 0x5555 */
    printf("read_c0(8)  = %ld\n", read_c0(8)); /* 8+16  = 0x4444 (base 8!) */
    write_c3(8, 0x2B2B);                       /* 8+40 = byte 48 = storage[6] */
    printf("c3 raw      = %ld\n", (long)storage[6]); /* 0x2B2B */
    printf("read_b(0)   = %ld\n", read_b(0)); /* unchanged: byte 8 */

    /* Same offsets through a call-returned base (byte 8 and byte 32 are still
     * 0x2222 and 0x5555; the write above hit storage[6]). */
    printf("call_b(0)   = %ld\n", read_b_call(0));  /* 0+8  = 0x2222 */
    printf("call_b(24)  = %ld\n", read_b_call(24)); /* 24+8 = 0x5555 */
    write_c3_call(0, 0x3C3C);                       /* 0+40 = byte 40 = storage[5] */
    printf("call_c3raw  = %ld\n", (long)storage[5]); /* 0x3C3C */
    printf("call_b(24)  = %ld\n", read_b_call(24));  /* unchanged: byte 32 */

    /* Indexed: byte off + 16 + 8*i. */
    printf("read_ci(0,0)= %ld\n", read_ci(0, 0)); /* 16    = storage[2] = 0x3333 */
    printf("read_ci(0,1)= %ld\n", read_ci(0, 1)); /* 24    = storage[3] = 0x4444 */
    printf("read_ci(0,3)= %ld\n", read_ci(0, 3)); /* 40    = storage[5] = 0x3C3C */
    printf("read_ci(8,2)= %ld\n", read_ci(8, 2)); /* 8+32  = storage[5] = 0x3C3C */
    write_c3_call(8, 0x4D4D);                    /* 8+40 = byte 48 = storage[6] */
    printf("read_ci(0,3)= %ld\n", read_ci(0, 3)); /* 40 = storage[5], untouched */
    printf("c6 raw      = %ld\n", (long)storage[6]); /* 0x4D4D */

    /* FP member (d at byte 8, read through the SSE-domain fold).  The bit
     * patterns are pi and e, so the printed values pin both the loaded bits and
     * the destination XMM register the value lands in. */
    storage[1] = 0x400921FB54442D18UL; /* pi   */
    storage[3] = 0x4005BF0A8B145769UL; /* e    */
    printf("read_d(0)   = %.17g\n", read_d(0));  /* byte  8 -> storage[1] */
    printf("read_d(16)  = %.17g\n", read_d(16)); /* byte 24 -> storage[3] */
    return 0;
}
