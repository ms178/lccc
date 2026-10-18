/* T9 (vellvm/vir transplant follow-up): partially initialised aggregates read
 * after SROA-style scalarisation.  The C analogue of Vellvm's known
 * insertvalue-into-undef failure: a struct whose unwritten members are
 * indeterminate is scalarised, and only the written members may be observed.
 * Every read below is of a member the program has stored to (C11 6.2.6.1,
 * 6.7.9), so the program is free of undefined behaviour; the reference
 * compiler's sanitizer run is the proof, and the differential run compares
 * stdout against GCC.  Indeterminate members are never read.  Aggregates are
 * copied whole, which is defined for a struct of unsigned-char-like storage. */
#include <stdio.h>
#include <string.h>

struct P3 { int a; int b; int c; };
struct Mixed { char tag; double w; short s; int n; };
struct Pad { char c; long l; char d; };
union U { int i; float f; unsigned char b[4]; };

__attribute__((noinline)) static int sum3(struct P3 p) { return p.a * 3 + p.c; }
__attribute__((noinline)) static struct P3 make_ac(int x) {
    struct P3 p;
    p.a = x;
    p.c = x * 2;
    return p; /* b never written: returned, never read by any caller */
}
__attribute__((noinline)) static double mixed_w(struct Mixed m) { return m.w * 2.0 + m.tag; }

static volatile int sink;

int main(void) {
    /* 1. Only a and c are written; the copy carries an indeterminate b. */
    struct P3 s;
    s.a = 5;
    s.c = 9;
    struct P3 t = s;
    sink = t.a + t.c;
    printf("sroa-copy %d %d %d\n", t.a, t.c, sum3(t));

    /* 2. Designated initialiser leaves b zero; partial write afterwards. */
    struct P3 d = { .a = 11 };
    d.c = -4;
    printf("designated %d %d\n", d.a, d.c);

    /* 3. Struct returned from a callee that wrote two of three members. */
    for (int i = 0; i < 4; i++) {
        struct P3 r = make_ac(i + 1);
        printf("ret %d %d\n", r.a, r.c);
    }

    /* 4. Mixed-type aggregate, only some members stored, passed by value. */
    struct Mixed m;
    m.tag = 'A';
    m.w = 1.5;
    m.n = 42;
    printf("mixed %.2f %d\n", mixed_w(m), m.n);

    /* 5. Union written through one member and read through another that
     *    overlaps only the written bytes (defined: C11 6.5.2.3). */
    union U u;
    u.i = 0x01020304;
    printf("union bytes %u %u %u %u\n", u.b[0], u.b[1], u.b[2], u.b[3]);
    u.f = 1.0f;
    printf("union float bits %08x\n", (unsigned)u.i);

    /* 6. Partially written array of structs; every element's written fields read back. */
    struct P3 arr[6];
    for (int i = 0; i < 6; i++) {
        arr[i].a = i * i;
        arr[i].b = -i;
        arr[i].c = i + 100;
    }
    long acc = 0;
    for (int i = 0; i < 6; i++) acc += arr[i].a * 7 + arr[i].b + arr[i].c;
    printf("array acc %ld\n", acc);

    /* 7. Padding is never read; only the members of struct Pad are. */
    struct Pad pd;
    pd.c = 'x';
    pd.l = 1234567890123L;
    pd.d = 'y';
    struct Pad pc;
    memcpy(&pc, &pd, sizeof pd);
    printf("pad %c %ld %c\n", pc.c, pc.l, pc.d);

    /* 8. Conditionally initialised aggregate: the branch decides which
     *    members are stored; only the stored member is read on each path. */
    for (int k = 0; k < 3; k++) {
        struct P3 q;
        if (k == 0) {
            q.a = 7;
            printf("cond0 %d\n", q.a);
        } else if (k == 1) {
            q.c = 8;
            printf("cond1 %d\n", q.c);
        } else {
            q.b = 9;
            q.a = q.b + 1;
            printf("cond2 %d %d\n", q.a, q.b);
        }
    }
    return 0;
}
