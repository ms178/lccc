/* C11 6.2.7p4 / 6.2.2p4: a redeclaration of an identifier with linkage in a
 * scope where a prior declaration WITH LINKAGE is visible takes the
 * composite type, so `extern T a[];` after `T a[N];` is complete (sizeof is
 * N * sizeof(T)).  lccc gave the incomplete declaration its own type, so
 * sizeof yielded 0 and gcc.c-torture/compile/20001018-1.c failed with
 * "size of array is negative".  A prior declaration WITHOUT linkage (a
 * local that hides the file-scope object) does not contribute: the inner
 * `extern` then names the file-scope object with its own incomplete type. */
#include <stdio.h>

int g[10];
extern int g[];                     /* file scope: stays int[10] */
static char s[7];                   /* internal linkage */
int sz_g = sizeof g;

int block_extern(void) { extern int g[]; return sizeof g; }
int internal_linkage(void) { extern char s[]; return sizeof s; }
int later_definition(void) { extern int m[][3]; return sizeof m[0]; }
int m[2][3];

/* Block-scope chain: the outer block extern is complete, the inner one
 * inherits it through the linked outer declaration (20001018-1). */
int nested_block_chain(void)
{
    extern char i[10];
    {
        extern char i[];
        char x[sizeof(i) == 10 ? 1 : -1];
        return (int)sizeof(i) + (int)sizeof(x);
    }
}
char i[10];

/* A no-linkage local hides the file-scope array: the inner extern refers to
 * the FILE-SCOPE object (not the local), with no composite from the local. */
int shadowed[10];
int local_shadow(void)
{
    int shadowed[3] = {7, 7, 7};
    (void)shadowed;
    {
        extern int shadowed[];
        return shadowed[1];
    }
}

int main(void)
{
    shadowed[1] = 42;
    printf("%d %d %d %d %d %d\n", sz_g, block_extern(), internal_linkage(),
           later_definition(), nested_block_chain(), local_shadow());
    return 0;
}
