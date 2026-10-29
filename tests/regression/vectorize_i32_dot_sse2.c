/* The XMM2-quarantine subject for the integer-reduction-register gate.
 *
 * The SSE2 lowering of the 4-lane I32 product stages through a third XMM
 * register (`%xmm2`, see `regalloc_helpers::clobbers_xmm2`), so the allocator
 * must keep every live accumulator OUT of `%xmm2` for this function.  That is
 * the property `check_integer_reduction_vecreg_codegen.sh` pins here.
 *
 * It used to be pinned on an I64 dot loop, which no longer vectorizes: a 64x64
 * low-half product needs an emulation (see the gate's cost note), and the
 * emulation is what used to occupy `%xmm2`.  An I32 dot at SSE2 exercises the
 * same quarantine with a shape that is still a win.
 */
int dot_i32(const int *a, const int *b, int n) {
    int s = 0;
    for (int i = 0; i < n; i++)
        s += a[i] * b[i];
    return s;
}

int main(void) {
    int a[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    int b[8] = {1, 1, 1, 1, 1, 1, 1, 1};
    return dot_i32(a, b, 8) == 36 ? 0 : 1;
}
