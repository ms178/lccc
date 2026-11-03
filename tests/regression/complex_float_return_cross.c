/* A `__complex__ float` returned through a relay call.
 *
 * RISC-V moves the complex result between the FP return registers and the
 * integer registers through frame slots; the relay's imaginary part takes
 * the path `sd t0, -72(s0)` then `flw fa1, -72(s0)`.  The global dead-store
 * census of the RV64 peephole counted only integer loads as readers, so it
 * deleted the `sd` at -O2, -O3 and -Os and the caller received stale stack
 * contents as the imaginary part (gcc.c-torture execute/pr38969.c).  Every
 * FP load width is a reader now, and a frame access the census cannot size
 * keeps every store.
 *
 * Keep this file minimal: unrelated frame traffic elsewhere in the unit can
 * supply an integer load of the same slot and hide the defect.
 *
 * Self-checking: exit status 0 is success; abort() otherwise. */
extern void abort(void);

__attribute__((noinline)) _Complex float pass_cf(_Complex float x) { return x; }

__attribute__((noinline)) _Complex float relay_cf(_Complex float x) { return pass_cf(x); }

int main(void)
{
    _Complex float a, b;
    __real__ a = 9;
    __imag__ a = 42;
    b = relay_cf(a);
    if (a != b)
        abort();
    return 0;
}
