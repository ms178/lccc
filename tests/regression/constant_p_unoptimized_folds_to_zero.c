/* At -O0 `__builtin_constant_p` of anything not constant in the source is 0,
 * and the code it guards folds away with the `if (0)`.
 *
 * GCC 16.2 expands the query to 0 when not optimizing (its resolver pass only
 * runs at -O1 and up).  LCCC deferred the query to that resolver at every
 * level, so at -O0 the guarded `link_error()` call stayed in the object.  It
 * did not matter while -O0 dropped unreferenced static functions; once -O0
 * emitted them, as GCC does, gcc.c-torture execute/20030330-1.c stopped
 * linking on every target.  This is that test plus the reachable form, at
 * -O0 (see the .flags file): a reference to `link_error` is a link error.
 */
extern void link_error(void);

static void usb_hub_port_wait_reset(unsigned int delay)
{
    int delay_time;
    for (delay_time = 0; delay_time < 500; delay_time += delay) {
        if (__builtin_constant_p(delay))
            link_error();
    }
}

__attribute__((noinline)) int count_steps(unsigned step)
{
    int n = 0;
    for (unsigned i = 0; i < 100; i += step) {
        if (__builtin_constant_p(step))
            link_error();
        n++;
    }
    return n;
}

int main(void)
{
    /* Literal constants are still constants at -O0. */
    if (!__builtin_constant_p(42) || !__builtin_constant_p("abc"))
        return 1;
    return count_steps(10) == 10 ? 0 : 2;
}
