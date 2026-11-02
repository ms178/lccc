/* C11 6.2.1p4: the nested `die` is its own entity and returns, so `g` can fall
 * out of its body even though the outer `die` never does.  GCC warns; a compiler
 * that resolves the call by bare name does not. */
extern void die(void) __attribute__((noreturn));
int g(void) { auto void die(void) { return; } die(); }
