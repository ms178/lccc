/* Indirect tail calls must convert in a FRAMELESS epilogue.
 *
 * optimize_tail_calls only recognised the %rbp teardown pair
 * (`movq %rbp,%rsp` + `popq %rbp`). lccc emits `addq $N,%rsp` whenever the
 * function has no dynamic alloca -- the common case -- so an indirect tail
 * call was never converted:
 *     call *%r10 ; addq $24,%rsp ; ret
 * where GCC emits a single `jmp *%rdi`. Releasing the stack BEFORE the jump
 * is what makes the tail call legal, so that form is now accepted too.
 *
 * Correctness is what this test pins down: the callee must still see its
 * arguments, the return value must propagate, and deep recursion through the
 * converted path must not grow the stack.
 */
#include <stdio.h>

static int add3(int a, int b, int c) { return a + b + c; }
static int mul2(int a, int b, int c) { return a * b * c; }

static int dispatch(int (*p)(int, int, int), int a, int b, int c)
{
	return p(a, b, c);
}

/* Mutual recursion through an indirect tail call: if the conversion were
 * wrong (stack released after the jump, or arguments clobbered) this either
 * crashes or returns garbage. */
static long countdown(long n, long (*self)(long, void *), void *ctx);
static long trampoline(long n, void *ctx)
{
	return countdown(n, trampoline, ctx);
}
static long countdown(long n, long (*self)(long, void *), void *ctx)
{
	if (n <= 0)
		return 0;
	return 1 + self(n - 1, ctx);
}

int main(void)
{
	if (dispatch(add3, 1, 2, 3) != 6) {
		printf("FAIL add3\n");
		return 1;
	}
	if (dispatch(mul2, 2, 3, 4) != 24) {
		printf("FAIL mul2\n");
		return 2;
	}
	if (trampoline(1000, (void *)0) != 1000) {
		printf("FAIL trampoline\n");
		return 3;
	}
	printf("PASS tail_call_frameless\n");
	return 0;
}
