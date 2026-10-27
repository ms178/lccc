/* gcc.c-torture pr39240.c, reduced to the x86-64 store-forwarding defect.
 * identity deliberately leaves unspecified upper bits in its integer return
 * register.  Converting produce's unsigned-int result to unsigned long must
 * retain the dword zero-extension even when phase 1 removes an explicit zext
 * and phase 2 forwards the subsequent same-register dword spill reload. */
extern void abort(void);

__attribute__((noinline)) static int identity(int x) { return x; }
__attribute__((noinline)) unsigned int produce(int x) { return identity(x + 6); }
volatile unsigned long expected = (unsigned int)-4;

int main(void) {
    if (produce(-10) != expected)
        abort();
    return 0;
}
