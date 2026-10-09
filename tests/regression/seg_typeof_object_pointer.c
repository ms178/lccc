/* A segment qualifier written before `__typeof__(T *)` qualifies the object
 * itself, not the pointee (GCC: `__seg_gs __typeof__(struct device *) ci`
 * is a %gs-resident pointer object). The pointer value read from it is an
 * ordinary `struct device *`, so `device_unregister(per_cpu(ci, cpu))`
 * type-checks (linux-6.18.55 drivers/base/cacheinfo.c per_cpu_cache_dev()).
 * Also covers a two-level field whose qualifier sits before the base type,
 * `struct rt __seg_gs **pc`, which must match the parameter of the same
 * spelling (linux-6.18.55 include/net/ip_fib.h / net/ipv4/fib_semantics.c).
 * GCC is the oracle. Output checked against GCC:
 *   0 42 2
 */
#include <stdio.h>

struct device { int x; };
struct rt { int value; };

static int device_seen;

static void device_unregister(struct device *dev)
{
	device_seen = dev->x;
}

#define RELOC_HIDE(ptr, off) \
	({ unsigned long __ptr; __asm__("" : "=r"(__ptr) : "0"(ptr)); \
	   (__typeof__(ptr))(__ptr + (off)); })
#define PERCPU_PTR(p) ((__typeof__(*(p)) *)((unsigned long)(p)))
#define per_cpu_ptr(ptr, cpu) RELOC_HIDE(PERCPU_PTR(ptr), (cpu))
#define per_cpu(var, cpu) (*per_cpu_ptr(&(var), cpu))

static __seg_gs __typeof__(struct device *) ci_cache_dev;

struct rt_cache {
	struct rt __seg_gs **pcpu_rth;
};

static void rt_free(struct rt __seg_gs **rtp)
{
	(void)rtp;
}

static int read_cache(struct rt_cache *c)
{
	rt_free(c->pcpu_rth);
	return c->pcpu_rth != NULL;
}

int main(void)
{
	static struct device dev = { 42 };
	struct rt_cache c = { NULL };
	unsigned long off = 0;

	ci_cache_dev = &dev;
	device_unregister(per_cpu(ci_cache_dev, off));
	printf("%d %d %d\n", read_cache(&c), device_seen, (int)sizeof(struct rt_cache) / 8 + 1);
	return 0;
}
