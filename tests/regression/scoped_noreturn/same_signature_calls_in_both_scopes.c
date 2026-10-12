/* C11 6.2.1p4 and 6.7.6.3p6, in the shape that a type comparison cannot see:
 * the nested declaration has the *same signature* as the file-scope one, and both
 * entities are called -- each in the scope where it is the binding.
 *
 *   f calls the file-scope `die`, which never returns: its `return 42` is dead.
 *   h calls a nested `die` with an identical prototype, which does return: its
 *   `return 7` is live.
 *
 * The two halves fail in opposite directions, which is why both are here.  A
 * compiler that resolves the call by bare name gives `h` the outer entity's
 * noreturn and deletes h's epilogue -- a miscompile of a program that plainly
 * returns 7.  A compiler that lets the nested (returning) declaration overwrite
 * the file-scope entity's facts keeps f's dead epilogue alive and, worse, can
 * drop f's noreturn for every *other* caller.  Neither shows up in a
 * -Wreturn-type count, so the gate asserts the emitted control flow as well.
 */
extern void die(void) __attribute__((noreturn));

int f(void)
{
    die();
    return 42;
}

int h(void)
{
    auto void die(void)
    {
        return;
    }
    die();
    return 7;
}
