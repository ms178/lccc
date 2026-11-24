/* `atomic_operators.c` on i686, where a 64-bit atomic object is wider than
 * a register: its load, store and update are `lock cmpxchg8b` sequences,
 * and an update split into two 32-bit halves is torn as well as lost. */
#include "atomic_operators.c"
