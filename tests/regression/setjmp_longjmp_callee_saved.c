/* Regression (torture execute/pr84521.c): __builtin_longjmp abandoned the
 * epilogue restores, so a value homed in a callee-saved register inside the
 * longjmp-carrying function permanently corrupted the setjmp caller's
 * register (main's `q` lived in %rbx; broken_longjmp spilled its buf
 * parameter there and jumped away without restoring).
 *
 * Functions containing __builtin_longjmp must stay out of the callee-saved
 * pool (same rule as GNU non-local goto).
 */
extern void abort(void);

void broken_longjmp(void *p) { __builtin_longjmp(p, 1); }

volatile int x = 256;
void *volatile p = (void *)&x;

void test(void) {
    void *buf[5];
    void *volatile q = p;

    if (!__builtin_setjmp(buf))
        broken_longjmp(buf);

    /* Fails if stack pointer corrupted.  */
    if (p != q)
        abort();
}

int main(void) {
    /* Kept in a callee-saved register across test() at -O2: must survive
       the longjmp round-trip intact.  */
    void *volatile q = p;
    test();
    /* Fails if a callee-saved register was corrupted.  */
    if (p != q)
        abort();

    return 0;
}
