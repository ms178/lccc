#include <limits.h>
#include <stdint.h>
#include <stdio.h>

#define NOINLINE __attribute__((noinline))

/* Three table values force Packed rather than TwoVal. The default also occurs
 * in slots 1 and 3; the final result select must still reject every other
 * out-of-range index, including shift counts that wrap onto a valid slot. */
NOINLINE int dense_with_slot(int x) {
    switch (x) {
    case -2: return -10;
    case -1: return -11;
    case 0: return -12;
    case 1: return -11;
    default: return -11;
    }
}

/* Same packed table, but no slot produces the default. This makes a leaked
 * out-of-range table value especially visible at wrapped shift counts. */
NOINLINE int dense_without_slot(int x) {
    switch (x) {
    case -2: return -10;
    case -1: return -11;
    case 0: return -12;
    case 1: return -11;
    default: return -9;
    }
}

/* The missing -2 arm initializes its table slot to the default. Three
 * explicit cases are required for switch_lut to consider the switch. */
NOINLINE int hole_with_slot(int x) {
    switch (x) {
    case -3: return -10;
    case -1: return -12;
    case 0: return -13;
    default: return -11;
    }
}

static int dense_ref(int x, int default_value) {
    static const int values[4] = {-10, -11, -12, -11};
    if (x >= -2 && x <= 1)
        return values[x + 2];
    return default_value;
}

static int hole_ref(int x) {
    static const int values[4] = {-10, -11, -12, -13};
    if (x >= -3 && x <= 0)
        return values[x + 3];
    return -11;
}

static int check_one(int x) {
    if (dense_with_slot(x) != dense_ref(x, -11))
        return 1;
    if (dense_without_slot(x) != dense_ref(x, -9))
        return 2;
    if (hole_with_slot(x) != hole_ref(x))
        return 3;
    return 0;
}

int main(void) {
    /* Exhaust the complete signed-16-bit input domain, including every table
     * boundary and a large default-only population. */
    for (int x = INT16_MIN; x <= INT16_MAX; ++x) {
        int rc = check_one(x);
        if (rc)
            return rc;
    }
    const int edges[] = {INT_MIN, INT_MIN + 1, -1000000, 1000000,
                         INT_MAX - 1, INT_MAX};
    for (unsigned i = 0; i < sizeof(edges) / sizeof(edges[0]); ++i) {
        int rc = check_one(edges[i]);
        if (rc)
            return 10 + rc;
    }
    puts("switch-lut-default-slot ok");
    return 0;
}
