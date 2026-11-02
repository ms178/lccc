/* One translation unit: kernels, driver and entry together, so inlining and
 * interprocedural constant propagation reach every kernel with the driver's
 * concrete arguments (the separate-object build keeps them opaque). */
#include "browser_bits.c"
#include "browser_driver.c"
#include "browser_check.c"
