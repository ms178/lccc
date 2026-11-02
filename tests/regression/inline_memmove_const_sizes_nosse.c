/* The exhaustive constant-size memcpy/memmove matrix built with
 * `-mno-sse -O3 -march=native`, i.e. every expansion forced onto the
 * scalar-GPR arm: CR4.OSFXSR=0 in early boot makes one xmm/ymm instruction
 * #UD, so the 17..32-byte memmove window takes the `rep movsb` fallback
 * here and everything at or below 16 bytes is built from %rax/%rcx pieces.
 *
 * The backwards `rep movsb` path is pinned only by this file: it must move
 * BOTH pointers onto the last byte of their range before copying, or the
 * dst > src case reads from one byte past the source end.
 *
 * Same source, different code path.  GCC is the differential oracle.
 */
#include "inline_memmove_const_sizes.c"
