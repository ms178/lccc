/* Regression: aggregate copy forwarding must not delete the materializing
 * memcpy of a compound literal when the destination's address feeds a MIXED
 * phi (kernel 6.18 static_call_inline.c __static_call_update shape):
 *
 *   first = (struct static_call_mod){ ... };
 *   for (sm = &first; sm; sm = sm->next) use(sm->sites);
 *
 * The marching phi merges &first with a LOADED pointer, so every loop load
 * through sm is unattributable; forwarding the copy left `first`
 * uninitialized and the walk read garbage (boot died in
 * __static_call_update+0xff dereferencing site=1). The pass must fail
 * closed on mixed-point phis.
 */
#include <stdio.h>
#include <stdint.h>

struct smod { struct smod *next; void *mod; int *sites; };
struct skey { void (*func)(void); unsigned long type; };

struct skey K;

int __attribute__((noinline)) upd(struct skey *key)
{
	struct smod first;
	struct smod *sm;

	if (key->func == (void (*)(void))0x1000)
		goto done;

	first = (struct smod){
		.next = (key->type & 1) ? 0 : (struct smod *)key->type,
		.mod = 0,
		.sites = (key->type & 1) ? (int *)(key->type & ~1UL) : 0,
	};

	int n = 0;
	for (sm = &first; sm; sm = sm->next)
		if (sm->sites)
			n += (int)(uintptr_t)sm->sites;
	return n;
done:
	return -1;
}

int main(void)
{
	K.type = 1;
	int a = upd(&K);            /* sites NULL: n stays 0 */
	if (a != 0) {
		printf("FAIL: untagged key walk returned %d\n", a);
		return 1;
	}
	static struct smod node = { 0, 0, (int *)0 };
	struct skey K2 = { 0, (unsigned long)&node | 1 };
	int b = upd(&K2);           /* sites = &node: n = (int)&node */
	if (b != (int)(uintptr_t)&node) {
		printf("FAIL: tagged key walk returned %d\n", b);
		return 1;
	}
	printf("PASS\n");
	return 0;
}
