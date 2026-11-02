/* Vectorizer-bailout orphan ICE (found via godbolt oracle kernels).
 *
 * `gd[i] = gd[i] * k + c` over a GLOBAL array made the vectorizer hoist
 * VecBroadcastF64x4 setup into the preheader, bail out of the loop (twice:
 * one orphan per attempt), and leave the loop scalar. The orphaned
 * broadcasts were not DCE-eligible -- IntrinsicOp::is_pure() missed the
 * modern Vec* families, so DCE rooted them as side-effecting -- and the
 * x86 intrinsic emitter ICEd storing a vector value that never received a
 * register or slot home:
 *
 *   x86 codegen: value 36 has no register, stack slot, Copy, or
 *   GlobalAddr definition
 *
 * The runtime check keeps the scalar loop honest as well.
 */
double gd[64];
volatile double sink;

void scale_add(double k, double c) {
    for (int i = 0; i < 64; i++)
        gd[i] = gd[i] * k + c;
}

double sum_scaled(void) {
    double s = 0.0;
    for (int i = 0; i < 64; i++)
        s += gd[i];
    return s;
}

int main(void) {
    for (int i = 0; i < 64; i++)
        gd[i] = (double)i;
    scale_add(2.0, 0.5);
    sink = sum_scaled();
    /* sum(i*2 + 0.5, i=0..63) = 2*(63*64/2) + 32 = 4032 + 32 = 4064 */
    return sink == 4064.0 ? 0 : 1;
}
