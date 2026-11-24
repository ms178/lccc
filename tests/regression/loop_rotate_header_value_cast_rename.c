/* The same program as loop_rotate_header_value_readers.c, compiled with
 * value numbering through casts as well (see the .env): then the body of a
 * loop over a narrow counter reads the header's widening of the counter,
 * and rotation left it the guard's (`unsigned char i; ... a[i + 1]` read
 * a[1] on every iteration; tests/regression/vectorize_narrow_iv_reduction
 * failed under CCC_CAST_RENAME=1 for that reason and no other).
 */
#include "loop_rotate_header_value_readers.c"
