typedef __seg_gs int *gp_t;
int *q;
gp_t gq;
struct A { int x; struct { int y; int *p; }; } a1 = { 1, 2, 0 };
struct A a2 = { .p = 0 };
struct S { gp_t p; int n; } s1 = { 0, 3 };
struct S s2 = { .n = 1, .p = 0 };
struct T { int n; struct S inner; } t1 = { 1, { 0, 2 } };
struct T t2 = { 1, 0, 2 };
gp_t arr[2] = { 0, [1] = 0 };
struct U { struct { gp_t p; }; int z; } u1 = { 0, 5 };
struct U u2 = { .p = 0, .z = 6 };
union V { gp_t p; int i; } v1 = { 0 };
struct W { char name[4]; gp_t p; } w1 = { "abc", 0 };
struct W w2 = { .name = "abc", .p = 0 };
struct Bf { int a : 3; int : 2; gp_t p; } bf1 = { 1, 0 };
void f(void) {
    struct S l = { gq, 3 };
    struct T lt = { 1, gq, 2 };
    struct U lu = { gq, 5 };
    gp_t la[2] = { [0] = gq, gq };
    struct S ls = (struct S){ gq, 1 };
    struct S l2 = { .p = gq, .n = 2 };
    struct T l3 = { .inner.p = gq, 7 };
    struct U lu2 = { .z = 1, gq };
    (void)l; (void)lt; (void)lu; (void)la; (void)ls; (void)l2; (void)l3; (void)lu2;
}
