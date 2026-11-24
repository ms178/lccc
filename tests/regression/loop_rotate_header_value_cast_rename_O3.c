/* loop_rotate_header_value_readers.c at -O3, with both knobs of the
 * companion (see the .flags and the .env): the defect was the same at -O2
 * and -O3, and -O3 puts the vectorizer and the unroller behind the
 * rotation.
 */
#include "loop_rotate_header_value_readers.c"
