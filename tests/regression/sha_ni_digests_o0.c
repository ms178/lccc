/* Same SHA-NI digest program as sha_ni_digests.c, built at -O0.  At -O0 the
 * immediate of _mm_sha1rnds4_epu32 must still reach the builtin as a literal,
 * which the header macro guarantees; this file pins that path. */
#include "sha_ni_digests.c"
