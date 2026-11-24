/* -fwrapv: a signed 32-bit index wraps from INT_MAX to INT_MIN, and the
 * store behind it lands where the loop's "invariant" load reads.
 *
 * Without -fwrapv the overflow is undefined and `a[i + 1]` may be modelled
 * as `a + i + 1` over the integers. With it the wrap is defined, and the
 * model let LICM take the load of `a[INT_MIN]` out of a loop that stores
 * to it on its third trip: the function below ran 200 trips instead of 3.
 * See lin_form_unsigned_index_wrap.c for the unsigned form of the defect.
 *
 * `a` points into the middle of a 4 GiB + 1 page anonymous mapping, so
 * that both a[INT_MIN] and a[INT_MAX] are elements of it.
 */
#include <limits.h>
#include <stdio.h>
#include <sys/mman.h>

#define NOINLINE __attribute__((noinline))

NOINLINE unsigned long bound_in_the_array(unsigned char *a) {
    unsigned long trips = 0;
    int i = INT_MAX - 2;
    while (trips < a[INT_MIN]) {
        a[i + 1] = 0; /* INT_MAX - 1, INT_MAX, INT_MIN */
        i++;
        trips++;
    }
    return trips;
}

NOINLINE unsigned long bound_in_the_array_do_while(unsigned char *a) {
    unsigned long trips = 0;
    int i = INT_MAX - 2;
    do {
        a[i + 1] = 0;
        i++;
        trips++;
    } while (trips < a[INT_MIN]);
    return trips;
}

NOINLINE void accumulator_in_the_array(unsigned char *a, unsigned long n) {
    int i = INT_MAX - 2;
    for (unsigned long t = 0; t < n; t++) {
        a[INT_MIN] += 3;
        a[i + 1] = 100;
        i++;
    }
}

/* Control: the index stays far from the wrap and the load is invariant. */
NOINLINE unsigned long far_from_the_wrap(unsigned char *a) {
    unsigned long trips = 0;
    int i = 1;
    while (trips < a[0]) {
        a[i + 1] = 0;
        i++;
        trips++;
    }
    return trips;
}

static int failures;

static void expect(const char *what, unsigned long got, unsigned long want) {
    printf("%s: %lu\n", what, got);
    if (got != want) {
        printf("FAIL %s: want %lu\n", what, want);
        failures++;
    }
}

int main(void) {
    unsigned char *m, *a;
    if (sizeof(void *) < 8) {
        printf("no 4 GiB object in this address space: nothing to test\n");
        return 0;
    }
    m = mmap(0, ((unsigned long long)1 << 32) + 4096, PROT_READ | PROT_WRITE,
             MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (m == MAP_FAILED) {
        printf("no 4 GiB mapping on this host: nothing to test\n");
        return 0;
    }
    a = m + ((unsigned long long)1 << 31);

    a[INT_MIN] = 200;
    expect("bound_in_the_array", bound_in_the_array(a), 3);
    a[INT_MIN] = 200;
    expect("bound_in_the_array_do_while", bound_in_the_array_do_while(a), 3);

    a[INT_MIN] = 1;
    accumulator_in_the_array(a, 5);
    expect("accumulator_in_the_array", a[INT_MIN], 106);

    a[0] = 7;
    expect("far_from_the_wrap", far_from_the_wrap(a), 7);

    if (failures) {
        printf("%d failures\n", failures);
        return 1;
    }
    printf("ok\n");
    return 0;
}
