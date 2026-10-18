/**
 * Body-less `enum tag` references must carry the definition's variant values.
 *
 * `EnumType::compatible_ctype` derives an enum's compatible integer type
 * (C11 6.7.2.2p4, GCC's choice) from the presence of a negative enumerator.
 * The sema resolver used to populate that variant list only for *packed*
 * bodies, so every body-less `enum tag` reference -- a local variable, a
 * struct field, a parameter -- resolved to a variants-less EnumType whose
 * compatible type was `unsigned int`.  A pointer to such an enum was then
 * rejected against `int *` parameters.
 *
 * Real-world shape (drivers/md/md.c, linux v6.18):
 *
 *   include/uapi/linux/raid/md_u.h:106:  #define LEVEL_LINEAR (-1)
 *   drivers/md/md.h:32:  enum md_submodule_id { ID_LINEAR = LEVEL_LINEAR, ID_RAID0 = 0, ... };
 *   md.c: kstrtoint(buf, 10, &submodule->id)   -- `int *` parameter
 *
 * lccc rejected that call ("incompatible pointer type ... have
 * 'enum md_submodule_id *' but expected 'int *'") while GCC 16 accepts it;
 * the enum has a negative enumerator, so its compatible type is `int`, and
 * pointers to compatible types interchange.  Compatibility is symmetric, so
 * the reverse direction (`int *` value for an `enum tag *` parameter) must
 * keep working too, and an all-non-negative enum must stay on the `unsigned
 * int` side.  Compiled and executed: exit 0 is the pass condition.
 */
#include <stdio.h>

#define LEVEL_LINEAR (-1)

enum md_submodule_id {
	ID_LINEAR	= LEVEL_LINEAR,
	ID_RAID0	= 0,
	ID_RAID1	= 1,
	ID_RAID4	= 4,
	ID_RAID5	= 5,
	ID_RAID6	= 6,
	ID_RAID10	= 10,
	ID_CLUSTER,
	ID_BITMAP,
	ID_LLBITMAP,
	ID_BITMAP_NONE,
};

/* All non-negative: GCC makes this compatible with `unsigned int`, not `int`. */
enum md_level {
	LEVEL_RAID0	= 0,
	LEVEL_RAID1	= 1,
	LEVEL_RAID5	= 5,
};

struct md_submodule_head {
	enum md_submodule_id	id;
	const char		*name;
};

/* Kernel-shaped callee: `int *` out-parameter. */
static int kstrtoint(const char *s, unsigned int base, int *res)
{
	long v = 0;
	int neg = 0;

	if (*s == '-') {
		neg = 1;
		s++;
	}
	while (*s >= '0' && *s <= '9')
		v = v * (long)base + (*s++ - '0');
	*res = neg ? -(int)v : (int)v;
	return 0;
}

/* The unsigned side: `unsigned int *` out-parameter. */
static int bump_level(unsigned int *level)
{
	*level = *level + 1;
	return 0;
}

/* Both directions of the compatible-pointer interchange. */
static int note_level(enum md_submodule_id *out, int *level)
{
	*out = (enum md_submodule_id)*level;
	return 0;
}

static enum md_submodule_id *pick_head(struct md_submodule_head *h)
{
	return &h->id;
}

int main(void)
{
	struct md_submodule_head h;
	enum md_submodule_id local;
	enum md_submodule_id *p;
	enum md_level lvl;
	int v = 0;
	unsigned int u = 0;

	/* The reference through a local variable is the case that used to fail:
	 * `&local` is `enum md_submodule_id *` and must interoperate with the
	 * `int *` parameter because ID_LINEAR < 0. */
	h.id = ID_BITMAP;
	if (kstrtoint("-42", 10, &local) != 0)
		return 1;
	if ((int)local != -42)
		return 2;
	if (kstrtoint("10", 10, &h.id) != 0)
		return 3;
	if ((int)h.id != 10)
		return 4;

	/* Reverse direction: an `int *` lvalue handed to an enum out-parameter. */
	v = 13;
	if (note_level(&local, &v) != 0)
		return 5;
	if ((int)local != 13)
		return 6;

	/* Through a struct field and a pointer, the md.c call shape. */
	p = pick_head(&h);
	if (p != &h.id || *p != ID_RAID10)
		return 7;

	/* The compatible types themselves, both spellings. */
	_Static_assert(__builtin_types_compatible_p(enum md_submodule_id, int),
		       "enum with a negative enumerator is compatible with int");
	{
		enum md_submodule_id again;
		(void)again;
		_Static_assert(__builtin_types_compatible_p(typeof(again), int),
			       "a body-less reference keeps the compatible type");
	}

	/* The unsigned side is untouched: `unsigned int *` accepts the
	 * all-non-negative enum's pointer, and bumping through it works. */
	u = 9;
	if (bump_level(&u) != 0 || u != 10)
		return 8;
	if (bump_level(&lvl) != 0)
		return 9;

	printf("ok %d %d %u\n", (int)local, (int)h.id, u);
	return 0;
}
