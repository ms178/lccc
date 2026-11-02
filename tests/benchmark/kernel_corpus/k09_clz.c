/* Count leading zeros with the zero guard. */
#include <stdint.h>

int lz(uint32_t x) { return x ? __builtin_clz(x) : 32; }
