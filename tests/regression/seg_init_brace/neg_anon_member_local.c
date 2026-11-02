typedef __seg_gs int *gp_t;
int *q;
struct A1 { int a; struct { int y; gp_t p; }; };
struct A2 { int x; struct { int y; gp_t p; }; } g2 = { .p = q };
struct A3 { int x; struct { int y; gp_t p; }; } g3 = { 1, 2, q };
struct A4 { int x; struct { int y; gp_t p; }; } g4 = { 1, 2, (gp_t)0 };
struct B1 { gp_t p; } g5 = { q };
union U1 { gp_t p; int i; } g6 = { q };
gp_t g7[2] = { [1] = q };
struct A5 { int x; struct { int y; gp_t p; }; } g8 = { .y = 1, .p = q };
void f(void) {
    struct A1 l1 = { 1, 2, q };
    struct B1 l2 = { q };
    gp_t l3[2] = { [1] = q };
    static struct A3 s3 = { 1, 2, q };
    (void)l1; (void)l2; (void)l3; (void)s3;
}
