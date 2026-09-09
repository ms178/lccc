/*
 * SSE2-baseline twin of vec_arx_scalar_spelling.c: the ARX transform runs
 * at the plain SSE2 baseline too (the packed add/xor/rotate core needs no
 * SSSE3); only the pshufb whole-byte-rotate fast path is absent, so every
 * rotate is the pslld/psrld/por triple.  Known-answer results must be
 * bit-identical at both ISA levels.
 */
#include "vec_arx_scalar_spelling.c"
