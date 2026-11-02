#include <stdio.h>
enum E1 { A1, B1, C1 = 5, D1, E1v = -1 };
enum E2 { X2 = 0x7fffffff };
enum E3 { Y3 = 1u << 31 };
enum class_like { Z3 = sizeof(int) * 3 };
#define P(e) printf("%-16s %lld\n", #e, (long long)(e))
int main(void) {
    P(A1); P(B1); P(C1); P(D1); P(E1v); P(X2); P(Y3); P(Z3);
    P(sizeof(enum E1)); P(sizeof(enum E2)); P(sizeof(enum E3));
    struct B { unsigned a : 3; signed b : 3; unsigned c : 1; } b = { 7, -1, 1 };
    P(b.a); P(b.b); P(b.c); P(sizeof b);
    struct B2 { int x : 31; int y : 1; } b2 = { -1, -1 };
    P(b2.x); P(b2.y);
    struct { unsigned f : 32; } w = { 0xffffffffu };
    P(w.f);
    unsigned u = 0xffffffffu; u >>= 1; P(u);
    return 0;
}
