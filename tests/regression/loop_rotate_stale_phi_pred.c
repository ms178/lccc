/* Regression: loop_rotate labelled the rotated body's init phi incoming with
 * the ORIGINAL PREHEADER instead of the newly created guard block.
 *
 * After rotation the body is reachable only via guard -> body, so naming the
 * preheader produced malformed IR: the phi claimed a predecessor that is not
 * a predecessor. Phi elimination resolved the stale label to the preheader's
 * block index and emitted the init copy there, which happened to dominate the
 * body, so the bug stayed latent for months. Any pass that trusts the phi's
 * predecessor list instead (SCCP prunes operands on provably-dead edges) drops
 * the induction variable's initialisation entirely: the loop then indexes an
 * array with an uninitialised register -> wild store -> SIGSEGV.
 *
 * Shape requirements to reproduce:
 *   - a counted loop whose IV feeds an array subscript (so a lost init is a
 *     wild store, not merely a wrong number),
 *   - the loop must be rotatable (simple guard-at-top, pure backedge),
 *   - a second loop consuming the array, so the result is observable.
 *
 * Needs CCC_LOOP_ROTATE=1 (see the .env sidecar).
 * Expected output: 4950 174300 24750
 */
#include <stdio.h>

static int arr[100];

int main(void) {
    int i;
    long sum = 0, wsum = 0, esum = 0;

    /* Rotated loop with a stale init-phi label: arr[i] is the wild store. */
    for (i = 0; i < 100; i++) {
        arr[i] = (i * 7) % 100;
    }

    for (i = 0; i < 100; i++) {
        sum += arr[i];
    }

    /* A second rotatable loop whose IV init is a non-zero constant, so the
     * lost-initialisation failure mode is a wrong value rather than only a
     * fault, catching the case where the register happens to be zero. */
    for (i = 3; i < 100; i++) {
        wsum += (long) arr[i] * i;
    }

    /* IV init from a runtime value (not a constant), exercising the
     * dominance argument for `pre_op` rather than the constant fast path. */
    int start = (int) (sum % 5);
    for (i = start; i < 100; i += 2) {
        esum += arr[i];
    }

    printf("%ld %ld %ld\n", sum, wsum, esum);
    return 0;
}
