#include <stdio.h>

#if !__has_attribute(externally_visible)
#error "missing __has_attribute(externally_visible)"
#endif

static int init_order = 0;

__attribute__((constructor))
static void wp_ctor(void) {
    init_order += 10;
}

int internal_helper(int x) {
    return x * 4 + init_order;
}

__attribute__((externally_visible))
int exported_api(int x) {
    return internal_helper(x) + 3;
}

__attribute__((used))
int retained_symbol = 17;

int unused_external_fn(int x) {
    return x * 999;
}

int main(void) {
    printf("%d\n", exported_api(5) + retained_symbol);
    return 0;
}
