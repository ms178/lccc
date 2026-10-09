/* The address of a member of a __seg_gs (per-CPU) object is a __seg_gs
 * pointer: `&cpu_fbatches.op` in linux-6.18.55 mm/swap.c passes it to a
 * `struct folio_batch __percpu *` parameter. Covered forms: `.`, `->` through a
 * __seg_gs pointer, `(*p).`, and a member of an element of a __seg_gs array.
 * GCC is the oracle; %gs.base is 0 for userspace so the values are identical. */
#include <stdio.h>

struct batch {
	int count;
	int items[3];
};

struct cpu_state {
	int stamp;
	struct batch op;
};

#define __percpu __seg_gs

static struct cpu_state __percpu state = {7, {1, {10, 20, 30}}};
static struct cpu_state __percpu states[2] = {{3, {2, {4, 5, 6}}}, {9, {0, {0, 0, 0}}}};
static struct cpu_state __percpu *cur = &state;

static int add_item(struct batch __percpu *b, int v)
{
	b->items[b->count] = v;
	b->count++;
	return b->count;
}

static int first_item(struct batch __percpu *b)
{
	return b->items[0];
}

int main(void)
{
	int r;

	printf("%d\n", add_item(&state.op, 40));		/* 2  */
	printf("%d\n", state.op.items[1]);			/* 40 */
	printf("%d\n", first_item(&cur->op));			/* 10 */
	printf("%d\n", first_item(&(*cur).op));			/* 10 */
	r = add_item(&states[1].op, 5);				/* 1  */
	printf("%d %d\n", r, states[1].op.items[0]);		/* 1 5 */
	printf("%d\n", first_item(&states[0].op));		/* 4  */
	return 0;
}
