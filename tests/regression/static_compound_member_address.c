/* Non-kernel static-address/relocation fixture, reduced from the shape used
 * by i915 sysfs attributes. No Linux headers or target kernel code. */
#include <stdio.h>
struct leaf { char padding[3]; int value; };
struct outer { long prefix; struct leaf attr; long tail; };
struct with_array { int prefix; struct leaf entries[3]; };
struct with_union { long prefix; union { int value; long other; } u; };

#define LITERAL(v) (&((struct outer[]){ {.attr = { .value = (v) }} })[0].attr.value)
#define PAIR(v) LITERAL(v), LITERAL(v)
static int *addresses[] = {
    &((struct outer[]) {
        { .prefix = 11, .attr = { .value = 13 } },
        { .prefix = 17, .attr = { .value = 19 } }
    })[1].attr.value,
    &((struct outer[2][3]) { [1][2] = { .attr = { .value = 23 } } })[1][2].attr.value,
    &((struct outer) { .attr = { .value = 29 } }).attr.value,
    &((struct with_array) { .entries = { [2] = { .value = 31 } } }).entries[2].value,
    &((struct with_union[]) { [1] = { .u = { .value = 37 } } })[1].u.value,
    &((struct { long prefix; struct leaf attr; }) { .attr = { .value = 41 } }).attr.value,
    LITERAL(43), LITERAL(47), PAIR(53),
};
static const int expected[] = {19, 23, 29, 31, 37, 41, 43, 47, 53, 53};
int main(void) {
    if (addresses[8] == addresses[9]) return 3;
    for (unsigned i = 0; i < sizeof(expected)/sizeof(expected[0]); ++i) {
        if (*addresses[i] != expected[i]) return 1;
        *addresses[i] += (int)i;
        if (*addresses[i] != expected[i] + (int)i) return 2;
        printf("%d\n", *addresses[i]);
    }
    return 0;
}
