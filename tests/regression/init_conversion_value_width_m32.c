/* init_conversion_value_width on i686 (-m32): the widened I64 source type
 * made the i686 backend reach its scalar cast emitter with a 64-bit cast
 * (internal error) instead of converting the int value. */
#include "init_conversion_value_width.c"
