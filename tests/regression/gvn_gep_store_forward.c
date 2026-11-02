#include <stdint.h>
struct pair { uint64_t a, b; };
__attribute__((noinline)) static uint64_t set_read(struct pair *p, uint64_t x) {
    p->a = x;
    return p->a;
}
int main(void) {
    struct pair p = { 1, 2 };
    uint64_t x = set_read(&p, UINT64_C(0x123456789abcdef0));
    return !(x == UINT64_C(0x123456789abcdef0) && p.a == x && p.b == 2);
}
