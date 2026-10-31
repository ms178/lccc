/* Hosted fixture for check_riscv_relax_bfd.py: linked against the real crt
 * objects, glibc and libgcc(_eh), so relaxation sees ifunc calls (static
 * memcpy/strlen go through IPLT stubs), hundreds of archive members, merged
 * CIEs and a working unwinder.  The program unwinds its own stack through
 * the linked `.eh_frame`, so a broken frame table fails the run. */
#include <stdio.h>
#include <string.h>
#include <unwind.h>

static int depth;

static _Unwind_Reason_Code count_frame(struct _Unwind_Context *ctx, void *arg)
{
    (void)ctx;
    (void)arg;
    depth++;
    return _URC_NO_REASON;
}

__attribute__((noinline)) int leaf(int x)
{
    _Unwind_Backtrace(count_frame, 0);
    return x + depth;
}

__attribute__((noinline)) int mid(int x)
{
    return leaf(x * 2) + 1;
}

int main(int argc, char **argv)
{
    char buf[64];
    int r = mid(argc);
    memcpy(buf, argv[0], strlen(argv[0]) < sizeof buf ? strlen(argv[0]) + 1 : 1);
    printf("depth=%d r=%d\n", depth, r);
    return depth >= 3 && buf[0] != 0 ? 0 : 1;
}
