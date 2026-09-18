/* Regression: inline-asm TIED operands ("0" matching "=r") whose input value
 * is register-homed WITHOUT a stack slot silently skipped the operand load.
 * The asm emitter's load_input_to_reg only handled slot-resident values, so
 * the tied scratch register kept whatever it held — kernel 6.18.52
 * __get_file_rcu (fs/file.c): `file_reloaded = *f; OPTIMIZER_HIDE_VAR(...)`
 * compared a leftover "=@ccs" boolean chain (rcx) against the file pointer
 * (r12) and hung userspace boot in a retry loop. The fix materialises
 * no-slot inputs through the canonical value_to_reg chain (register homes,
 * width-consistent typed loads, Copy/Cast chains, GlobalAddr remats, and the
 * accumulator).
 *
 * This is a userspace lookalike of __get_file_rcu: same tied hide-var, same
 * atomic "+m" + "=@ccs" sign-overflow idiom, same reload-and-compare shape. */
#include <stdio.h>

#define OPTIMIZER_HIDE_VAR(var) __asm__ ("" : "=r" (var) : "0" (var))

struct fobj {
    long f_ref;
    char pad[16];
};

/* file_ref_get() lookalike: atomically bump the refcount, detect overflow
 * through the sign flag ("=@ccs"), normalise through a _Bool the same way
 * the kernel's file_ref_get does. */
static int ref_get(struct fobj *file)
{
    unsigned char not_neg;
    __asm__ volatile("lock addq $1, %1"
                     : "=@ccns"(not_neg), "+m"(file->f_ref)
                     :
                     : "memory");
    return not_neg != 0;
}

static long put_count;

/* __get_file_rcu lookalike: returns the object pointer, 0 for NULL, or
 * (long)-11 (-EAGAIN). The hide-var tie on the freshly RELOADED pointer is
 * the heart of the regression. */
static struct fobj *get_rcu(struct fobj **f)
{
    struct fobj *file, *file_reloaded, *file_reloaded_cmp;

    file = *f;
    if (!file)
        return (void *)0;

    if (!ref_get(file))
        return (void *)-11L;

    file_reloaded = *f;
    file_reloaded_cmp = file_reloaded;
    OPTIMIZER_HIDE_VAR(file_reloaded_cmp);

    if (file == file_reloaded_cmp)
        return file_reloaded;

    put_count++;
    return (void *)-11L;
}

int main(void)
{
    struct fobj obj;
    struct fobj *p, *np, *r;
    long lr;

    obj.f_ref = 1;
    p = &obj;
    r = get_rcu(&p);
    lr = (long)r;
    if (lr != (long)&obj) {
        printf("FAIL: live path returned %ld, expected object (%ld)\n",
               lr, (long)&obj);
        return 1;
    }
    if (obj.f_ref != 2) {
        printf("FAIL: refcount %ld, expected 2\n", obj.f_ref);
        return 2;
    }
    if (put_count != 0) {
        printf("FAIL: put_count %ld on matching pointers\n", put_count);
        return 3;
    }

    np = (void *)0;
    r = get_rcu(&np);
    if (r != (void *)0) {
        printf("FAIL: null path returned %ld\n", (long)r);
        return 4;
    }
    if (obj.f_ref != 2) {
        printf("FAIL: null path bumped refcount to %ld\n", obj.f_ref);
        return 5;
    }

    /* Overflow path: refcount would go negative -> -EAGAIN, no object. */
    obj.f_ref = -2;
    p = &obj;
    r = get_rcu(&p);
    if ((long)r != -11L) {
        printf("FAIL: overflow path returned %ld, expected -11\n", (long)r);
        return 6;
    }

    printf("OK hide-var tied operand\n");
    return 0;
}
