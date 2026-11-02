/* Three register-carried parameters whose values must all survive until the call.
 *
 * Under -mregparm=3 the callee receives a, b, c in %eax, %edx, %ecx, and the call
 * to `sink` clobbers all three, so the prologue has to capture them first.  The
 * captures are the interesting part: `i686/codegen/prologue.rs` records that params
 * are processed in descending ABI-register order because "%ecx/%edx are themselves
 * allocatable (caller-saved phase), so a pre-store target could be a later param's
 * still-unread source".  The gate does not pin the *order* -- any order is correct
 * when the targets are disjoint from the sources -- it pins the property the order
 * exists to protect: no instruction may write one of %eax/%edx/%ecx before every
 * read of it has happened.  A capture that stages through a register it still owes
 * another param silently replaces one argument with another, and the miscompile is
 * visible only as a wrong value at -O2.
 */
extern int sink(int, int, int);

int f(int a, int b, int c)
{
    int t = sink(a, b, c);
    return t + a * 3 + b * 5 + c * 7;
}
