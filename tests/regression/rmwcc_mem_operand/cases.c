/* RMWcc memory-operand shapes.
 *
 * The defect this pins: an inline-asm memory operand printed as the EMPTY
 * string, so the assembler received a truncated instruction with a trailing
 * comma and no address (`lock  btsq  $0,`).  That is how Linux 6.18
 * mm/zsmalloc.c failed to build: folio_trylock -> folio_test_set_bit_locked ->
 * arch_test_and_set_bit -> GEN_BINARY_RMWcc(LOCK_PREFIX bts, *addr, c, "Ir",
 * nr) with `[var] "+m" (_var)`.  Standalone probes with a stack-slotted
 * address do NOT reproduce it: the operand has to arrive without a stack slot
 * (register home, Copy/Cast chain, or accumulator), which is what a big
 * function's register allocator produces.  The shapes below cover the address
 * provenance a real translation unit shows -- plain pointer, member, computed
 * (base + index), loop-carried phi, reloaded pointer-to-pointer -- and the gate
 * checks BOTH the emitted operand (no line may end in a comma) and the
 * run-time value against GCC.
 *
 * Every function returns a value derived from the condition-code output so the
 * compiler cannot delete the asm, and `main` prints the accumulated state so a
 * wrong *address* (not just a missing one) is caught by comparing both
 * compilers' output. */
#include <stdio.h>

struct s { unsigned long flags; };
struct big { unsigned long pad[4]; struct s in; };

int m1_plain(unsigned long *addr, long nr)
{
	int c;
	__asm__ volatile("btsq %2, %1" : "=@ccc" (c), "+m" (*addr) : "Ir" (nr));
	return c;
}

int m2_member(struct s *p, long nr)
{
	int c;
	__asm__ volatile("btsq %2, %1" : "=@ccc" (c), "+m" (p->flags) : "Ir" (nr));
	return c;
}

int m3_computed(unsigned long *base, unsigned long idx, long nr)
{
	int c;
	__asm__ volatile("btsq %2, %1"
			 : "=@ccc" (c), "+m" (*(unsigned long *)(base + idx)) : "Ir" (nr));
	return c;
}

int m4_phi(unsigned long *a, unsigned long *b, int n)
{
	unsigned long *p = a;
	int c, acc = 0;

	for (int i = 0; i < n; i++) {
		/* Constant bit number: the kernel's PG_* flags make the "I"
		 * alternative always applicable, which is the form 6.18 emits. */
		__asm__ volatile("btsq %2, %1"
				 : "=@ccc" (c), "+m" (*p) : "Ir" (1));
		acc += c;
		p = (i & 1) ? b : a;
	}
	return acc;
}

int m5_nested(struct big **pp, int n)
{
	int c, acc = 0;

	for (int i = 0; i < n; i++) {
		__asm__ volatile("btsq %2, %1"
				 : "=@ccc" (c), "+m" ((*pp)->in.flags) : "Ir" (2));
		acc += c;
		pp++;
	}
	return acc;
}

/* A read-only "m" input next to a read-write "m" output: both addresses must
 * be materialised, and the two scratch registers must not collide. */
int m6_two_mem(const unsigned long *src, unsigned long *dst, long nr)
{
	int c;
	__asm__ volatile("lock addq %2, %0" : "+m" (*dst) : "m" (*src), "r" (nr));
	__asm__ volatile("btsq %2, %1" : "=@ccc" (c), "+m" (*dst) : "Ir" (nr));
	return c;
}

/* Dynamic bit number: the "r" alternative of "Ir".  The register is 64-bit
 * because the value is `unsigned long` -- a 32-bit register here would make
 * the `q` suffix illegal, which is why the kernel only ever feeds constants. */
int m7_dynamic(unsigned long *addr, unsigned long nr)
{
	int c;
	__asm__ volatile("btsq %2, %1" : "=@ccc" (c), "+m" (*addr) : "Ir" (nr));
	return c;
}

/* Three distinct non-alloca memory operands referenced in the same template:
 * each address must receive its own scratch register (dynamic busy_regs). */
__attribute__((noinline))
void m8_three_mem_distinct(unsigned long *out0, unsigned long *inout1, const unsigned long *in2)
{
	unsigned long tmp;
	__asm__ volatile(
		"movq %3, %2\n\t"
		"movq %2, %0\n\t"
		"addq %2, %1"
		: "=m" (*out0), "+m" (*inout1), "=&r" (tmp)
		: "m" (*in2)
		: "memory");
}

/* Exhaust all 8 caller-saved GP scratch registers via clobbers so generic
 * register and memory-address scratch overflows into callee-saved registers
 * (%rbx/%r12..%r15), which the callee prologue/epilogue MUST preserve. */
__attribute__((noinline))
unsigned long m9_scratch_overflow_callee(unsigned long *p, unsigned long x)
{
	unsigned long out;
	__asm__ volatile(
		"leaq 7(%2), %0\n\t"
		"addq %0, %1"
		: "=&r" (out), "+m" (*p)
		: "r" (x)
		: "rcx", "rdx", "rsi", "rdi", "r8", "r9", "r10", "r11", "cc", "memory");
	return out;
}

__attribute__((noinline))
unsigned long m9_caller_live_callee_saved(unsigned long a, unsigned long b,
					  unsigned long c, unsigned long d,
					  unsigned long e)
{
	unsigned long slot = 100;
	unsigned long r1 = m9_scratch_overflow_callee(&slot, a + b);
	unsigned long r2 = m9_scratch_overflow_callee(&slot, c + d + e);
	return r1 + r2 + slot + a * 3 + b * 5 + c * 7 + d * 11 + e * 13;
}

/* Force "Q" constraint onto callee-saved %rbx by clobbering rax/rcx/rdx. */
__attribute__((noinline))
unsigned long m10_qreg_rbx_callee(unsigned long v)
{
	unsigned long q = v;
	__asm__ volatile(
		"xorb $0x5a, %b0\n\t"
		"xorb $0x33, %h0"
		: "+Q" (q)
		:
		: "rax", "rcx", "rdx", "cc");
	return q;
}

__attribute__((noinline))
unsigned long m10_caller_check(unsigned long x, unsigned long y, unsigned long z)
{
	unsigned long q = m10_qreg_rbx_callee(x);
	return q ^ (y * 17 + z * 29);
}

int main(void)
{
	static struct s sm = { 0 };
	static struct big bm[2];
	struct big *ptrs[2] = { &bm[0], &bm[1] };
	unsigned long x[4] = { 0 };
	unsigned long m8_a = 0, m8_b = 19, m8_c = 23;
	unsigned long m9, m10;
	int r = 0;

	r += m1_plain(&x[0], 0);
	r += m1_plain(&x[0], 0);	/* second set: carry must be 1 */
	r += m2_member(&sm, 1);
	r += m3_computed(x, 2, 2);
	r += m4_phi(&x[0], &x[1], 8);
	r += m5_nested(ptrs, 2);
	r += m6_two_mem(&x[0], &x[1], 3);
	r += m7_dynamic(&x[3], 40);
	m8_three_mem_distinct(&m8_a, &m8_b, &m8_c);
	m9 = m9_caller_live_callee_saved(11, 13, 17, 19, 23);
	m10 = m10_caller_check(0x12345678UL, 31, 37);
	printf("r=%d x0=%lu x1=%lu sm=%lu bm0=%lu m8=%lu/%lu m9=%lu m10=%lu\n",
	       r, x[0], x[1], sm.flags, bm[0].in.flags, m8_a, m8_b, m9, m10);
	return 0;
}
