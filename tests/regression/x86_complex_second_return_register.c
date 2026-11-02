/* gcc.c-torture/execute/20050121-1.c and 20070614-1.c reduced.
 * The second FP half of _Complex float/double returns lives in %xmm1 even when
 * the imaginary SSA value was register-allocated and has no stack slot.
 */
extern void abort(void);
__attribute__((pure)) _Complex double cd(int x) {
    _Complex double r;
    __real__ r = x + 1;
    __imag__ r = x - 1;
    return r;
}
__attribute__((pure)) _Complex float cf(int x) {
    _Complex float r;
    __real__ r = x + 1;
    __imag__ r = x - 1;
    return r;
}
int main(void) {
    if (__real__ cd(5) != 6.0 || __imag__ cd(5) != 4.0) abort();
    if (__real__ cf(5) != 6.0f || __imag__ cf(5) != 4.0f) abort();
    return 0;
}
