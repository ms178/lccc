/* Memory disambiguation must use TARGET access widths.  The same-block DSE,
 * store-to-load forwarding and aggregate-copy forwarding passes sized every
 * `Ptr` access as 8 bytes, so on i686 a 4-byte pointer store "fully covered"
 * an earlier 8-byte store at the same offset: the zero-fill of a local
 * aggregate whose first pointer member was then stored lost the zeroing of
 * the following 4 bytes (struct F f2 below: `f2[1].p[1]` read stack
 * garbage), and a pointer store was forwarded to an overlapping 8-byte load
 * as if it had written all of it.  The x87 long double (10 to 16 bytes
 * written) is modeled as a range.  Every function dirties the stack first so
 * a lost store cannot read back as a lucky zero. */
#include <stdio.h>
#include <string.h>

struct F { const char *p[3]; int n; };
struct P2 { void *a; void *b; };
struct PK { void *p; int k; };

__attribute__((noinline)) static void dirty(void)
{
    volatile unsigned char junk[512];
    memset((unsigned char *)junk, 0xa5, sizeof junk);
}

__attribute__((noinline)) static int f_array_of_struct(void)
{
    struct F f2[2] = { [1].p = { [2] = "z" }, [0].n = 4, [1].p[0] = "y" };
    return (f2[0].n == 4) + 2 * (f2[1].p[1] == 0) + 4 * (f2[1].n == 0) +
           8 * (f2[0].p[0] == 0 && f2[0].p[1] == 0 && f2[0].p[2] == 0) +
           16 * (f2[1].p[0][0] == 'y' && f2[1].p[2][0] == 'z');
}

__attribute__((noinline)) static int f_first_member(void *v)
{
    struct P2 s = { .a = v };
    return (s.a == v) + 2 * (s.b == 0);
}

__attribute__((noinline)) static int f_two_members(void *v)
{
    struct PK s = { v };
    return (s.p == v) + 2 * (s.k == 0);
}

__attribute__((noinline)) static unsigned long long f_forward(void *v)
{
    unsigned char buf[8];
    unsigned long long x;
    memset(buf, 0xff, sizeof buf);
    memcpy(buf, &v, sizeof v);   /* overwrites the low sizeof(void *) bytes */
    memcpy(&x, buf, sizeof x);
    return x;
}

int main(void)
{
    int t;
    dirty(); t = f_array_of_struct(); printf("array_of_struct %d\n", t);
    dirty(); t = f_first_member(&t); printf("first_member %d\n", t);
    dirty(); t = f_two_members(&t); printf("two_members %d\n", t);
    dirty(); printf("forward %llx\n", f_forward((void *)0x1234));
    return 0;
}
