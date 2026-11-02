/* Volatile loads must not be hoisted out of a loop by LICM.
 *
 * WHY THIS FILE EXISTS (and why the runtime alone cannot catch the bug).
 *
 * C11 5.1.2.3: a volatile access is part of the program's observable
 * behaviour. Hoisting `sum_volatile`'s load into a preheader turns N
 * accesses into 1, which is a miscompilation -- but it is a miscompilation
 * that produces the SAME ANSWER, because the global happens to hold the
 * same value on every iteration. No amount of stdout comparison detects it.
 * Any differential worth running here therefore has to be structural: the
 * question is not "does it compute the right number" but "does the load
 * still sit inside the loop".
 *
 * So this file carries three contracts, and the structural one is the
 * detector:
 *
 *   1. RUNTIME    correct results with LICM on and off (guards against a
 *                 miscompilation, which is a different failure).
 *   2. STRUCTURAL in sum_volatile's assembly the load of g_volatile must be
 *                 inside the loop; in sum_plain's assembly the load of
 *                 g_plain must be OUTSIDE it (hoisted).
 *   3. CONTROL    contract 2's second half is the negative control: a
 *                 compiler that hoisted nothing at all would also leave the
 *                 volatile load in place, so the test must prove LICM is
 *                 actually running and actually hoisting the plain load.
 *                 Without it, contracts 1 and 2 pass vacuously.
 */
#include <stdio.h>

volatile int g_volatile;
int g_plain;

/* N observable reads of a volatile global. */
int sum_volatile(int n)
{
    int s = 0;
    for (int i = 0; i < n; i++)
        s += g_volatile;
    return s;
}

/* Identical shape, non-volatile. LICM is expected to hoist this one. */
int sum_plain(int n)
{
    int s = 0;
    for (int i = 0; i < n; i++)
        s += g_plain;
    return s;
}

/* A volatile read guarded by a condition, to pin the second half of the
 * hoist decision (must-execute) as well as the first (volatile). */
int sum_volatile_cond(int n, int flag)
{
    int s = 0;
    for (int i = 0; i < n; i++)
        if (flag)
            s += g_volatile;
    return s;
}

/* The SHAPE THAT ACTUALLY REACHES THE BUG.
 *
 * The rotated `for` loops above are refused for a different reason: their
 * guard block is part of the natural loop, so the body block does not
 * dominate every loop block and LICM's must-execute test rejects the load
 * before volatility is ever consulted. A do-while has no guard block, so
 * the load's block does dominate the whole loop and volatility is the ONLY
 * thing standing between that load and the preheader. That is the shape the
 * structural contracts below are written against. */
int sum_volatile_dowhile(int n)
{
    int s = 0, i = 0;
    do { s += g_volatile; i++; } while (i < n);
    return s;
}

int sum_plain_dowhile(int n)
{
    int s = 0, i = 0;
    do { s += g_plain; i++; } while (i < n);
    return s;
}

int main(void)
{
    g_volatile = 3;
    g_plain = 3;
    if (sum_volatile(8) != 24) return 1;
    if (sum_plain(8) != 24) return 2;
    if (sum_volatile_cond(8, 1) != 24) return 3;
    if (sum_volatile_cond(8, 0) != 0) return 4;

    g_volatile = 5;
    g_plain = 5;
    if (sum_volatile(4) != 20) return 5;
    if (sum_plain(4) != 20) return 6;

    /* The do-while shapes (n >= 1, so the loops really do run). */
    if (sum_volatile_dowhile(4) != 20) return 7;
    if (sum_plain_dowhile(4) != 20) return 8;
    if (sum_volatile_dowhile(1) != 5) return 9;
    if (sum_plain_dowhile(1) != 5) return 10;

    printf("volatile_licm: ok\n");
    return 0;
}
