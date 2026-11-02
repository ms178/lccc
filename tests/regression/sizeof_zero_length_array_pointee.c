/*
 * sizeof through pointers to zero-length arrays (GNU C `[0]` types).
 *
 * GCC keeps the true size of zero-length arrays everywhere:
 *   char (*p)[0];          sizeof(*p)   == 0
 *   typedef char T0[0];    sizeof(*q)   == 0   for T0 *q
 *   char x[0][4];          sizeof(*x)   == 0,  sizeof(x[0]) == 0
 * while the GNU extensions sizeof(*void_ptr) == 1 and
 * sizeof(*func_ptr) == 1 must keep working.
 *
 * This is load-bearing kernel code: include/linux/kfifo.h encodes
 * "record fifo vs plain fifo" as sizeof of the `char (*rectype)[recsize]`
 * member of __STRUCT_KFIFO_COMMON, where recsize == 0 for plain kfifo.
 * A compiler that clamps that sizeof to 1 makes every plain kfifo take
 * the __kfifo_in_r/__kfifo_out_r record path and corrupts the byte
 * stream (observed as mangled serial console output).
 *
 * GCC-oracle differential: exit code and stdout must match gcc exactly.
 */
#include <stdio.h>

typedef char T0[0];
typedef char T1[1];
typedef char T2[2];
typedef int  I0[0];

struct holder {
    char (*rectype)[0];   /* kfifo idiom, recsize 0 */
    char (*rec1)[1];      /* recsize 1 */
    char (*rec2)[2];      /* recsize 2 */
};

static struct holder h;
static char (*p0)[0];
static char (*p3)[3];
static char (*pd)[];      /* incomplete pointee: only sizeof(pd) is valid C */
static T0 *q0;
static I0 *qi;
static char x00[0][0];
static char x04[0][4];
static char x40[4][0];
static char a0[0];

/* pointer-to-function and void* keep the GNU sizeof == 1 extension */
static int fn(int a) { return a + 1; }
static int (*fp)(int) = fn;

int main(void) {
    printf("sizeof(*p0)      = %zu (gcc 0)\n", sizeof(*p0));
    printf("sizeof(p0[0])    = %zu (gcc 0)\n", sizeof(p0[0]));
    printf("sizeof(p0)       = %zu (gcc 8)\n", sizeof(p0));
    printf("sizeof(*p3)      = %zu (gcc 3)\n", sizeof(*p3));
    printf("sizeof(p3[0])    = %zu (gcc 3)\n", sizeof(p3[0]));
    printf("sizeof(pd)       = %zu (gcc 8)\n", sizeof(pd));

    printf("sizeof(T0)       = %zu (gcc 0)\n", sizeof(T0));
    printf("sizeof(*q0)      = %zu (gcc 0)\n", sizeof(*q0));
    printf("sizeof(q0[0])    = %zu (gcc 0)\n", sizeof(q0[0]));
    printf("sizeof(*qi)      = %zu (gcc 0)\n", sizeof(*qi));
    printf("sizeof(qi[0])    = %zu (gcc 0)\n", sizeof(qi[0]));

    /* struct-field spellings: the exact kfifo recsize encoding */
    printf("sizeof(*h.rectype) = %zu (gcc 0)\n", sizeof(*h.rectype));
    printf("sizeof(*h.rec1)    = %zu (gcc 1)\n", sizeof(*h.rec1));
    printf("sizeof(*h.rec2)    = %zu (gcc 2)\n", sizeof(*h.rec2));
    printf("sizeof(h.rectype[0]) = %zu (gcc 0)\n", sizeof(h.rectype[0]));

    /* nested zero dimensions */
    printf("sizeof(*x00)     = %zu (gcc 0)\n", sizeof(*x00));
    printf("sizeof(x00[0])   = %zu (gcc 0)\n", sizeof(x00[0]));
    printf("sizeof(*x04)     = %zu (gcc 0)\n", sizeof(*x04));
    printf("sizeof(x04[0])   = %zu (gcc 0)\n", sizeof(x04[0]));
    printf("sizeof(*x40)     = %zu (gcc 0)\n", sizeof(*x40));
    printf("sizeof(x40[0])   = %zu (gcc 0)\n", sizeof(x40[0]));
    printf("sizeof(x40[0][0])= %zu (gcc 1)\n", sizeof(x40[0][0]));

    printf("sizeof(a0)       = %zu (gcc 0)\n", sizeof(a0));

    /* cast spellings */
    printf("cast  = %zu (gcc 0)\n", sizeof(*(char (*)[0])0));
    printf("cast1 = %zu (gcc 1)\n", sizeof(*(char (*)[1])0));

    /* GNU extensions that must stay 1 */
    printf("void* = %zu (gcc 1)\n", sizeof(*(void *)0));
    printf("func* = %zu (gcc 1)\n", sizeof(*fp));

    /* typeof spelling */
    printf("typeof = %zu (gcc 0)\n", sizeof(__typeof__(*p0)));

    return 0;
}
