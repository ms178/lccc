/* MINMAX-1 corpus entry: the REFUSED shapes, executed.
 *
 * Contract 3 of `check_minmax_reduction.sh` is that every shape the min/max
 * reduction transform declines to model — unsigned compares, 16-bit elements,
 * float elements, a `continue` guard, a second accumulator in the same loop —
 * stays scalar AND stays correct.  Refusing is load-bearing there: modelling a
 * multi-accumulator loop with a single-accumulator pattern produced `sum == 0`
 * for every n below the vector width.
 *
 * A shape census that is only ever asserted on assembly could drift into
 * "refused, and also wrong" without anything noticing, so the driver is run
 * for its output too.  The driver lives in `minmax_shapes/` (the gate compiles
 * it against `minmax_shapes.c` as two objects); this wrapper makes it a
 * complete TU so the ordinary corpus runs it at -O2 on every invocation.
 * Deterministic: fixed xorshift seed, one checksum line on stdout.
 */
#include "minmax_shapes/minmax_shapes.c"
#include "minmax_shapes/minmax_refused_main.c"
