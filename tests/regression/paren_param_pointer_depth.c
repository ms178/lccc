/* Parenthesized parameter declarators: every parenthesized star is a pointer
 * level. `int (*a[2])` is an array of pointers and decays to `int **`, which
 * the declarator path dropped to `int *`. `int (**fpp)(int)` is a pointer to a
 * function pointer, and `int (**pp)[3]` a pointer to a pointer to an array.
 * Self-checking; GCC is the oracle for the printed values. */
#include <stdio.h>

static int add1(int x) { return x + 1; }
static int sum_ptrs(int (*a[2])) { return *a[0] + *a[1]; }
static int call_via(int (**fpp)(int), int v) { return (**fpp)(v); }
static int first_row(int (**pp)[3]) { return (*pp)[0][0]; }

int main(void)
{
	int x = 3, y = 4;
	int *p[2] = { &x, &y };
	int (*fp)(int) = add1;
	int row[1][3] = { { 9, 8, 7 } };
	int (*rp)[3] = row;

	if (sum_ptrs(p) != 7)
		return 1;
	if (call_via(&fp, 41) != 42)
		return 2;
	if (first_row(&rp) != 9)
		return 3;
	printf("%d %d %d\n", sum_ptrs(p), call_via(&fp, 41), first_row(&rp));
	return 0;
}
