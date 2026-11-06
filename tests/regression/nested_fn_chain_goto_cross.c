/* GNU C nested functions on AArch64 and RV64, executed by
 * check_switch_cross_execute.py at every optimization level.  Direct calls
 * only: no trampoline, so the freestanding harness needs no executable
 * stack.
 *
 * - A nested function's parameters survive the static chain's save at its
 *   entry.  AArch64 -O0 staged the chain through x0, the first argument
 *   register, before the parameters were read (torture execute/921017-1.c
 *   indexed its parent's array with the chain pointer).
 * - A nested function that leaves through a nonlocal `goto` never runs its
 *   epilogue, so it must hold nothing in a callee-saved register: the
 *   parent's own caller keeps its values there (torture execute/pr24135.c
 *   at -O1 on AArch64 and RV64, where `x.y` homed its argument in x19/s1
 *   and `main` its loop counter).
 */
#define NOINLINE __attribute__((noinline))

static volatile int sink;
NOINLINE static void touch(int v) { sink = v; }

NOINLINE int vla_index(int n, int k)
{
    int a[n];
    int g(int i) { return a[i] + i; }
    for (int j = 0; j < n; j++)
        a[j] = 100 * j + 7;
    return g(k);
}

NOINLINE long nested_params(int n, long k, int m, short s)
{
    long base = n * 3L;
    long h(long i, int j, short t) { return base + i * j - t; }
    return h(k, m, s);
}

/* `y` keeps `t` across a call (a callee-saved home, were it allowed) and
 * then leaves through the goto on two of its cases. */
NOINLINE int leave_through_goto(int a, int b)
{
    __label__ out;
    void y(int c)
    {
        int t = c * 5 + a;
        touch(t);
        switch (c) {
        case 1:
            goto out;
        case 2:
            if (t != 10 + a)
                touch(-1);
            goto out;
        }
        touch(t + 1);
    }
    a = a + 2;
    y(b);
    return a + 100;
out:
    return a;
}

int main(void)
{
    int bad = 0;
    for (int n = 2; n < 9; n++)
        for (int k = 0; k < n; k++)
            bad += vla_index(n, k) != 100 * k + 7 + k;
    for (int n = -3; n < 4; n++)
        bad += nested_params(n, n * 1000L, n - 7, (short)(n * 3)) !=
               n * 3L + n * 1000L * (n - 7) - (short)(n * 3);
    /* Values that live across every call below sit in main's callee-saved
     * registers. */
    long acc = 0;
    int checks = 0;
    for (int j = 1; j <= 4; ++j)
        for (int i = 0; i <= 4; ++i) {
            int r = leave_through_goto(j, i);
            int want = (i == 1 || i == 2) ? j + 2 : j + 102;
            bad += r != want;
            acc += r * (i + 1);
            checks++;
        }
    long want_acc = 0;
    for (int j = 1; j <= 4; ++j)
        for (int i = 0; i <= 4; ++i)
            want_acc += ((i == 1 || i == 2) ? j + 2 : j + 102) * (i + 1);
    bad += acc != want_acc;
    bad += checks != 20;
    return bad != 0;
}
