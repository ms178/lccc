/* loop_memset trip guards at -Os, where the unextended start offset of a
 * negative int init once sent memset ~4 GiB past the buffer. */
#include "loop_memset_trip_guard.c"
