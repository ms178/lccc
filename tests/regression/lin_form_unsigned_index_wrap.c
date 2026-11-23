/* An unsigned 32-bit index wraps, and the store behind it lands where the
 * loop's "invariant" load reads.
 *
 * With `unsigned i` at UINT_MAX, `a[i + 1]` is `a[0]`: unsigned arithmetic
 * is modular by definition (C11 6.2.5p9), and an object of more than 4 GiB
 * makes both ends of that wrap valid elements of one array. The compiler's
 * linear address forms used to model `zext(i + 1)` as `i + 1` over the
 * integers, so the store "marched away" from `a[0]` for ever, LICM took the
 * load of `a[0]` out of the loop, and the memory promoter kept `a[0]` in a
 * register across a store to it. The first function below then ran 200
 * trips instead of 3.
 *
 * The array is a 4 GiB + 1 page anonymous mapping of which three pages are
 * touched. Where such a mapping cannot be had (a 32-bit address space) the
 * test says so and passes: there `unsigned` is as wide as an address and
 * the wrap is the address's own.
 */
#include <stdio.h>
#include <sys/mman.h>

#define NOINLINE __attribute__((noinline))

/* The loop's bound is a[0]; the store of the third trip writes it. */
NOINLINE unsigned long bound_in_the_array(unsigned char *a) {
    unsigned long trips = 0;
    unsigned i = 0xFFFFFFFDu;
    while (trips < a[0]) {
        a[(unsigned)(i + 1)] = 0;
        i++;
        trips++;
    }
    return trips;
}

/* The same, tested at the bottom: the addresses are computed in the block
 * that is the loop's header. */
NOINLINE unsigned long bound_in_the_array_do_while(unsigned char *a) {
    unsigned long trips = 0;
    unsigned i = 0xFFFFFFFDu;
    do {
        a[(unsigned)(i + 1)] = 0;
        i++;
        trips++;
    } while (trips < a[0]);
    return trips;
}

/* The index runs downwards into the wrap: 1 - 3, 2 - 3, 3 - 3. */
NOINLINE unsigned long bound_below_the_index(unsigned char *a) {
    unsigned long trips = 0;
    unsigned i = 1;
    while (trips < a[0]) {
        a[(unsigned)(i - 3)] = 0;
        i++;
        trips++;
    }
    return trips;
}

/* An accumulator in a[0] next to the wrapping store: the third trip's
 * store replaces the sum, and the trips behind it add to what it wrote. */
NOINLINE void accumulator_in_the_array(unsigned char *a, unsigned long n) {
    unsigned i = 0xFFFFFFFDu;
    for (unsigned long t = 0; t < n; t++) {
        a[0] += 3;
        a[(unsigned)(i + 1)] = 100;
        i++;
    }
}

/* The counter starts where the caller says. */
NOINLINE unsigned long bound_in_the_array_from(unsigned char *a, unsigned i) {
    unsigned long trips = 0;
    while (trips < a[0]) {
        a[(unsigned)(i + 1)] = 0;
        i++;
        trips++;
    }
    return trips;
}

/* Control: a counter as wide as the address never comes back to a[0], the
 * load is invariant, and hoisting it is right. */
NOINLINE unsigned long wide_counter(unsigned char *a) {
    unsigned long trips = 0;
    unsigned long i = 1;
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
    unsigned char *a;
    if (sizeof(void *) < 8) {
        printf("no 4 GiB object in this address space: nothing to test\n");
        return 0;
    }
    a = mmap(0, ((unsigned long long)1 << 32) + 4096, PROT_READ | PROT_WRITE,
             MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (a == MAP_FAILED) {
        printf("no 4 GiB mapping on this host: nothing to test\n");
        return 0;
    }

    a[0] = 200;
    expect("bound_in_the_array", bound_in_the_array(a), 3);
    a[0] = 200;
    expect("bound_in_the_array_do_while", bound_in_the_array_do_while(a), 3);
    a[0] = 200;
    expect("bound_below_the_index", bound_below_the_index(a), 3);
    a[0] = 200;
    expect("bound_in_the_array_from", bound_in_the_array_from(a, 0xFFFFFFFDu), 3);
    a[0] = 200;
    expect("bound_in_the_array_from 0", bound_in_the_array_from(a, 0), 200);

    a[0] = 1;
    accumulator_in_the_array(a, 5);
    /* 1 + 3 + 3 + 3, replaced by 100, + 3 + 3. */
    expect("accumulator_in_the_array", a[0], 106);

    a[0] = 7;
    expect("wide_counter", wide_counter(a), 7);

    if (failures) {
        printf("%d failures\n", failures);
        return 1;
    }
    printf("ok\n");
    return 0;
}
