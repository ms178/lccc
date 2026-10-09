/* A segment qualifier written after a `*` qualifies the pointee of the NEXT
 * (outer) pointer level: `struct item * __percpu *cells` is a pointer to a
 * %gs-resident `struct item *` (kernel kernel/events/hw_breakpoint.c:
 * `struct perf_event * __percpu *cpu_events`, passed to
 * unregister_wide_hw_breakpoint()).
 *
 * Before the fix the after-star qualifier was discarded, so the parameter had
 * no space, the element `*cells` was typed as a plain pointer, and passing
 * `cpu_events` / dereferencing `*cells` produced "incompatible pointer type"
 * diagnostics against __seg_gs-typed declarations.
 *
 * GCC accepts the same spellings and is the oracle. %gs.base is 0 for a
 * userspace process, so the %gs accesses hit the same memory as plain ones. */
#include <stdio.h>

#define __percpu __seg_gs

struct item {
	int v;
};

static struct item a = {1}, b = {2}, c = {4};

/* Each element is a pointer that lives in %gs space. */
static struct item *__percpu table[3] = {&a, &b, &c};

/* A single %gs-resident pointer. */
static struct item *__percpu slot = &b;

static int sum_cells(struct item *__percpu *cells, int n)
{
	int s = 0;
	for (int i = 0; i < n; i++)
		s += cells[i]->v;
	return s;
}

static int read_slot(struct item *__percpu *p)
{
	struct item *it = *p;	/* loads the %gs-resident pointer */
	return it->v;
}

static void set_slot(struct item *__percpu *p, struct item *it)
{
	*p = it;
}

int main(void)
{
	struct item *__percpu *cells = table;
	struct item *local = 0;
	int r;

	printf("%d\n", sum_cells(table, 3));		/* 7 */
	printf("%d\n", read_slot(&slot));		/* 2 */
	set_slot(&slot, &c);
	printf("%d\n", read_slot(&slot));		/* 4 */
	printf("%d\n", cells[1]->v);			/* 2 */
	local = *cells;					/* a, from the %gs pointer */
	printf("%d\n", local->v);			/* 1 */
	r = sum_cells(cells + 1, 2);			/* b + c */
	printf("%d\n", r);				/* 6 */
	return 0;
}
