/* Fork-derived regression: IrConst::narrowed_to U32 zero-extension.
   unsigned int ui = UINT_MAX; ui /= 5000.0; must be 858993. */
#include <stdio.h>
#include <limits.h>
int main(void){ unsigned int ui = UINT_MAX; ui /= 5000.0;
  printf("%u\n", ui); return ui==858993?0:1; }
