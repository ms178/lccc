/* Runtime, self-checking contract for complex __seg_gs objects used as rvalues.
 *
 * A complex rvalue read from %gs must be copied exactly once into a generic
 * temporary. A second copy reads %gs at the temporary's stack address and
 * returns garbage. `__real__`/`__imag__` of a segment complex must also have
 * the component type at -O0, where a pointer type would read the component as
 * an address.
 *
 * GS_DISP and the mirror work as in seg_gs_audit.c: the GS base is set to
 * GS_DISP, and (&object + GS_DISP) is mapped as a mirror. A correct %gs access
 * reads the mirror. A generic access reads the object's own bytes, which hold
 * different values, so a missed segment fails the check.
 */
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>

#define ARCH_SET_GS 0x1001
#define GS_DISP ((uintptr_t)1 << 32)
#define MIR(g) ((void *)((char *)(g) + GS_DISP))

_Complex double __seg_gs cd_obj;
_Complex double __seg_gs cd2_obj;
_Complex float __seg_gs cf_obj;
extern _Complex double cd_g[] __asm__("cd_obj");
extern _Complex double cd2_g[] __asm__("cd2_obj");
extern _Complex float cf_g[] __asm__("cf_obj");

/* Generic objects with values that differ from the mirror, so a generic read
 * of a segment object cannot pass. */
static _Complex double cd_generic = 100.0 + 200.0 * 1.0i;

static int failures;

#define CHECK(cond, name)                                  \
    do {                                                   \
        if (cond) printf("ok   %s\n", name);               \
        else { printf("FAIL %s\n", name); failures++; }    \
    } while (0)

static void map_mirror(void *g, size_t n) {
    uintptr_t lo = (uintptr_t)g + GS_DISP;
    uintptr_t page = lo & ~(uintptr_t)4095;
    size_t len = ((lo + n - page) + 4095) & ~(size_t)4095;
    void *m = mmap((void *)page, len, PROT_READ | PROT_WRITE,
                   MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
    if (m != (void *)page && errno != EEXIST) {
        perror("mmap");
        _Exit(2);
    }
}

/* Complex value passed and returned by value: the argument of pass_d is the
 * comma's result, so it is a complex rvalue from a segment object. */
__attribute__((noinline)) static _Complex double pass_d(_Complex double z) { return z; }

__attribute__((noinline)) static double real_of_comma(void) {
    return __real__(0, cd_obj);
}

__attribute__((noinline)) static double arg_comma(void) {
    return __real__ pass_d((0, cd_obj));
}

__attribute__((noinline)) static double imag_of_arg(void) {
    return __imag__ pass_d(cd_obj);
}

__attribute__((noinline)) static double parts_sum(void) {
    return __real__ cd_obj + __imag__ cd_obj;
}

__attribute__((noinline)) static _Complex double assign_val(_Complex double z) {
    return (cd_obj = z);
}

__attribute__((noinline)) static _Complex double cond_pick(int k) {
    return k ? cd_obj : cd2_obj;
}

__attribute__((noinline)) static _Complex double comma_ret(void) {
    return (0, cd2_obj);
}

__attribute__((noinline)) static float cf_real_of_comma(void) {
    return __real__(cf_obj, cf_obj);
}

__attribute__((noinline)) static _Complex double sum_objects(void) {
    return cd_obj + cd2_obj;
}

__attribute__((noinline)) static _Complex double generic_pick(int k) {
    return k ? cd_generic : cd2_obj;
}

static int same(_Complex double z, double re, double im) {
    return __real__ z == re && __imag__ z == im;
}

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    map_mirror(cd_g, sizeof(_Complex double));
    map_mirror(cd2_g, sizeof(_Complex double));
    map_mirror(cf_g, sizeof(_Complex float));
    if (syscall(SYS_arch_prctl, ARCH_SET_GS, (void *)GS_DISP) != 0) {
        perror("arch_prctl");
        return 2;
    }

    /* Seed the mirrors through ordinary pointers. */
    volatile double *m1 = (volatile double *)MIR(cd_g);
    m1[0] = 3.0;
    m1[1] = 4.0;
    volatile double *m2 = (volatile double *)MIR(cd2_g);
    m2[0] = 7.0;
    m2[1] = 8.0;
    volatile float *mf = (volatile float *)MIR(cf_g);
    mf[0] = 1.5f;
    mf[1] = 2.5f;

    CHECK(real_of_comma() == 3.0, "comma_real_of_rvalue");
    CHECK(arg_comma() == 3.0, "comma_argument_real");
    CHECK(imag_of_arg() == 4.0, "argument_imag");
    CHECK(parts_sum() == 7.0, "direct_parts_sum");
    CHECK(same(sum_objects(), 10.0, 12.0), "sum_of_two_segment_objects");
    CHECK(same(comma_ret(), 7.0, 8.0), "comma_return_complex");
    CHECK(same(cond_pick(1), 3.0, 4.0), "conditional_arm_segment");
    CHECK(same(cond_pick(0), 7.0, 8.0), "conditional_other_arm_segment");
    CHECK(same(generic_pick(1), 100.0, 200.0), "conditional_generic_arm");
    CHECK(cf_real_of_comma() == 1.5f, "float_complex_comma_real");

    /* Assignment: the stored value is written through %gs, and the value of
     * the assignment expression is the same complex. */
    CHECK(same(assign_val(5.0 + 6.0 * 1.0i), 5.0, 6.0), "assignment_value");
    CHECK(m1[0] == 5.0 && m1[1] == 6.0, "assignment_stored_through_gs");

    if (failures == 0) {
        /* The generic copy must not have been touched. */
        CHECK(__real__ cd_generic == 100.0, "generic_object_untouched");
    }

    printf("%s (%d failure%s)\n", failures ? "FAILED" : "PASSED", failures,
           failures == 1 ? "" : "s");
    return failures ? 1 : 0;
}
