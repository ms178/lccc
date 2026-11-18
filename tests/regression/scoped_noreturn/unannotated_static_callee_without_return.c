/* No attribute on the callee: it simply has no path that returns.
 *
 *   f calls the static `die`, whose only path ends in a call that cannot return.
 *   Control never comes back, so f's `return 42` is dead.  GCC finds this without
 *   being told (ipa-pure-const: "Function found to be noreturn"), and objtool
 *   infers the same of a local symbol that has no return instruction: a caller
 *   that keeps its epilogue is reported as `die() missing __noreturn`
 *   (Linux 6.18.63: io_worker_exit, __sysvec_reboot).
 *   h calls the static `maybe`, which returns on one of its paths: its
 *   `return 7` is live.
 *
 * As in same_signature_calls_in_both_scopes.c, the halves fail in opposite
 * directions: an inference that is too eager deletes h's live epilogue, one that
 * is missing keeps f's dead one.
 */
extern void stop(int) __attribute__((noreturn));
extern void note(int);

__attribute__((noinline)) static void die(int code)
{
    note(code);
    stop(code);
}

__attribute__((noinline)) static void maybe(int code)
{
    if (code)
        stop(code);
    note(code);
}

int f(int x)
{
    die(x);
    return 42;
}

int h(int x)
{
    maybe(x);
    return 7;
}
