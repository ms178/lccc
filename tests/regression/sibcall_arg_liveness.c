/* Sibling-call argument liveness: the argument setup of a tail call must
 * survive peephole cleanup.
 *
 * WHAT THIS PINS
 * A tail call publishes a call marker (`# LCCC_CALL_ARGS n`) that the liveness
 * reader consults to decide how many argument registers the call keeps alive.
 * Two defects made that read wrong, and their combination was a silent
 * miscompile rather than a crash:
 *
 *   1. the emitter published the marker twice for a sibling call -- once in
 *      place and once again after the tail `jmp`, so an orphan block carried a
 *      second marker reading 0 arguments;
 *   2. the reader scanned a fixed window *after* the call and took the LAST
 *      marker it found, so the orphan's 0 downgraded the real 1.
 *
 * With `arg_reads == 0` the argument register became dead, dead-pure-write
 * elimination deleted the `movq %rbx, %rdi` that loaded it, and the value that
 * fed it then lost its own reader and was deleted too. The kernel instance was
 * `addrconf_verify_work` passing `&rtnl_mutex`: the callee ran with a garbage
 * pointer.
 *
 * Every case below therefore checks the VALUE THE CALLEE OBSERVES, not the
 * assembly text. A deleted argument setup shows up as a wrong number, so this
 * is a runtime oracle and needs no disassembly to be meaningful. The shapes are
 * chosen to stress exactly the paths that broke: a pointer argument computed
 * from a register that has other live uses, several arguments, an argument that
 * is a field of an aggregate, and a tail call reached only through a branch.
 *
 * The stdout of this test must be byte-identical to gcc-16's; see
 * tests/regression/run_regression.py for the protocol.
 */

#include <stdio.h>

struct lock_like {
    long owner;
    long depth;
    long pad[6];
};

static struct lock_like g_lock;
static struct lock_like g_other;

/* Observed arguments, recorded so a deleted setup cannot hide behind a
 * coincidentally-equal return value. */
static long seen_ptr;
static long seen_val;
static long seen_third;

__attribute__((noinline)) static long take_ptr_val(struct lock_like *m, long v)
{
    seen_ptr = (m == &g_lock) ? 1 : (m == &g_other) ? 2 : -1;
    seen_val = v;
    return (m ? m->owner : -999) + v;
}

__attribute__((noinline)) static long take_three(struct lock_like *m, long a, long b)
{
    seen_ptr = (m == &g_lock) ? 1 : (m == &g_other) ? 2 : -1;
    seen_val = a;
    seen_third = b;
    return (m ? m->depth : -999) + a * 2 + b * 3;
}

__attribute__((noinline)) static long take_field(const long *p)
{
    seen_val = *p;
    return *p + 7;
}

/* Case 1 -- the kernel shape. A tail call whose first argument is the address
 * of a static aggregate and whose second is derived from a register that also
 * has other live uses further down, so deleting the argument setup cascades. */
long case_ptr_and_derived(long k)
{
    long scaled = k * 8;          /* keeps the base register live afterwards */
    long r = take_ptr_val(&g_lock, scaled - 2192);
    return r + (scaled & 3);
}

/* Case 2 -- the same, with the *other* aggregate, so a hardcoded pointer
 * cannot pass by accident. */
long case_second_aggregate(long k)
{
    return take_ptr_val(&g_other, k + 41);
}

/* Case 3 -- three arguments: the marker's count matters for more than one
 * register, so a downgrade to 0 or 1 is observable in different registers. */
long case_three_args(long k)
{
    return take_three(&g_lock, k, k + 1);
}

/* Case 4 -- a tail call whose argument is the address of a field inside a
 * static aggregate, i.e. an address that needs real materialisation. */
long case_field_address(void)
{
    return take_field(&g_lock.pad[3]);
}

/* Case 5 -- a genuine tail call in a void-ish position: the callee's result is
 * returned directly, which is what makes the emitter choose the `jmp` form
 * rather than `call` + `ret`. */
long case_pure_tail(long k)
{
    return take_ptr_val(&g_lock, k);
}

/* Case 6 -- the tail call is reached only through a conditional branch, so the
 * marker sits after a label and the reader's window crosses control flow. */
long case_tail_after_branch(long k)
{
    if (k & 1)
        return take_ptr_val(&g_lock, k * 3);
    return take_ptr_val(&g_other, k * 5);
}

/* Case 7 -- a loop whose last statement is a tail call. The marker is inside a
 * backward-branch region, the layout the orphan block used to land in. */
long case_tail_in_loop(long n)
{
    long acc = 0;
    for (long i = 0; i < n; i++)
        acc += take_ptr_val(&g_lock, i);
    return acc;
}

/* Case 8 -- a chain of tail calls, so a marker published for one call must not
 * be read as belonging to the next. */
long case_chained_tails(long k)
{
    long a = take_ptr_val(&g_lock, k);
    long b = take_ptr_val(&g_other, a);
    return take_three(&g_lock, a, b);
}

int main(void)
{
    g_lock.owner = 1000;
    g_lock.depth = 7;
    g_lock.pad[3] = 55;
    g_other.owner = 2000;
    g_other.depth = 9;

    long fails = 0;

    /* Each check re-reads what the callee actually observed. `seen_ptr` is the
     * discriminator: a deleted or corrupted argument pointer yields -1. */
    seen_ptr = seen_val = seen_third = 0;
    long r = case_ptr_and_derived(300);
    printf("case_ptr_and_derived: ret=%ld ptr=%ld val=%ld\n", r, seen_ptr, seen_val);
    if (seen_ptr != 1 || seen_val != 300 * 8 - 2192 || r != 1000 + 300 * 8 - 2192 + 0)
        fails++;

    seen_ptr = seen_val = 0;
    r = case_second_aggregate(1);
    printf("case_second_aggregate: ret=%ld ptr=%ld val=%ld\n", r, seen_ptr, seen_val);
    if (seen_ptr != 2 || seen_val != 42 || r != 2000 + 42)
        fails++;

    seen_ptr = seen_val = seen_third = 0;
    r = case_three_args(5);
    printf("case_three_args: ret=%ld ptr=%ld a=%ld b=%ld\n", r, seen_ptr, seen_val, seen_third);
    if (seen_ptr != 1 || seen_val != 5 || seen_third != 6 || r != 7 + 10 + 18)
        fails++;

    seen_val = 0;
    r = case_field_address();
    printf("case_field_address: ret=%ld val=%ld\n", r, seen_val);
    if (seen_val != 55 || r != 62)
        fails++;

    seen_ptr = seen_val = 0;
    r = case_pure_tail(11);
    printf("case_pure_tail: ret=%ld ptr=%ld val=%ld\n", r, seen_ptr, seen_val);
    if (seen_ptr != 1 || seen_val != 11 || r != 1011)
        fails++;

    seen_ptr = seen_val = 0;
    r = case_tail_after_branch(3);
    printf("case_tail_after_branch(odd): ret=%ld ptr=%ld val=%ld\n", r, seen_ptr, seen_val);
    if (seen_ptr != 1 || seen_val != 9 || r != 1009)
        fails++;
    seen_ptr = seen_val = 0;
    r = case_tail_after_branch(4);
    printf("case_tail_after_branch(even): ret=%ld ptr=%ld val=%ld\n", r, seen_ptr, seen_val);
    if (seen_ptr != 2 || seen_val != 20 || r != 2020)
        fails++;

    seen_ptr = seen_val = 0;
    r = case_tail_in_loop(4);
    printf("case_tail_in_loop: ret=%ld last_ptr=%ld last_val=%ld\n", r, seen_ptr, seen_val);
    if (r != (1000 + 0) + (1000 + 1) + (1000 + 2) + (1000 + 3) || seen_val != 3)
        fails++;

    seen_ptr = seen_val = seen_third = 0;
    r = case_chained_tails(2);
    printf("case_chained_tails: ret=%ld ptr=%ld a=%ld b=%ld\n", r, seen_ptr, seen_val, seen_third);
    /* a = 1000+2 = 1002 ; b = 2000+1002 = 3002 ; ret = 7 + 2*1002 + 3*3002 */
    if (seen_val != 1002 || seen_third != 3002 || r != 7 + 2004 + 9006)
        fails++;

    printf("fails=%ld\n", fails);
    return fails != 0;
}
