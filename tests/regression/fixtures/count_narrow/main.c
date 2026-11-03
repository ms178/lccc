/* The hosted half: the same checksum, run natively by the host backend. */
#include "runtime.c"
int main(void) { return count_narrow_checksum() != 4177401467814966825ull; }
