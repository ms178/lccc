/* gcc.c-torture/execute/20030218-1.c reduced.
 * Peephole relay folding must keep signed sub-32-bit values sign-extended to
 * the full 64-bit destination when the C type is long.
 */
extern void abort(void);
short *sink;
long sx(short *p) {
    long v = *p;
    sink = p + 1;
    return v;
}
int main(void) {
    short a = (short)0xff00;
    if (sx(&a) != (long)(short)0xff00) abort();
    return 0;
}
