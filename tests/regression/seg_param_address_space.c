/* Named address space (__seg_gs) carried by PARAMETERS and by OBJECTS.
 *
 * Kernel provenance: linux-6.18.55 kernel/softirq.c passes `&tasklet_vec`
 * (a DEFINE_PER_CPU object, i.e. a __seg_gs object) to a parameter declared
 * `struct tasklet_head __percpu *headp`, and include/linux/srcu.h passes a
 * `struct srcu_ctr __percpu *` guard member to `srcu_read_unlock_fast()`.
 *
 * Two defects this pins down:
 *   1. The qualifier of a parameter was parsed and discarded, so `*p` in
 *      `int read_param(int __seg_gs *p)` compiled to a plain `movl (%rdi)`
 *      instead of `movl %gs:(%rdi)` -- a silent miscompile of every per-CPU
 *      access made through a parameter (check_seg_param_deref.sh asserts the
 *      `%gs:` prefix; this test asserts the values).
 *   2. `&obj` for a __seg_gs object was typed as a plain pointer, so passing it
 *      to a __seg_gs parameter was rejected as an address-space mismatch.
 *
 * %gs.base is 0 for a normal Linux process, so a __seg_gs access through an
 * ordinary address reads the same memory as a plain access and the outputs
 * are comparable to GCC's, which accepts the same spellings natively. */
#include <stdio.h>

static int __seg_gs obj = 41;
static int __seg_gs arr[4] = {1, 2, 3, 4};

struct holder {
	int __seg_gs *p;
};

static int read_param(int __seg_gs *p)
{
	return *p;
}

static void write_param(int __seg_gs *p, int v)
{
	*p = v;
}

static int sum_second(int a, int __seg_gs *p)
{
	return a + *p;
}

/* Member argument: the field's own declared space must match the parameter. */
static int get_member(struct holder *h)
{
	return read_param(h->p);
}

/* Prototype first, definition later: the space must survive both spellings. */
static int proto_only(int __seg_gs *p);

static int proto_only(int __seg_gs *p)
{
	return p[1];
}

/* Pointer-to-pointer: the qualifier belongs to the innermost level only, so
 * `*pp` is an ordinary pointer and `**pp` reads the %gs space. */
static int read_pp(int __seg_gs **pp)
{
	return **pp;
}

int main(void)
{
	int __seg_gs *q = &arr[2];
	int __seg_gs *pp_target = q;
	struct holder h = {&arr[0]};
	int r = 0;

	write_param(&obj, 42);
	printf("%d\n", read_param(&obj));		/* 42 */
	printf("%d\n", sum_second(5, &obj));		/* 47 */
	printf("%d\n", get_member(&h));			/* 1  */
	printf("%d\n", proto_only(arr));		/* 2  */
	printf("%d\n", proto_only(&arr[1]));		/* 3  */
	printf("%d\n", read_pp(&pp_target));		/* 3  */
	write_param(q, 30);
	printf("%d %d\n", arr[2], read_param(q));	/* 30 30 */
	r += read_param(&obj) + sum_second(0, arr);	/* 42 + 1 */
	printf("%d\n", r);				/* 43 */
	return 0;
}
