/* The corpus compiles root-level C files standalone. The dedicated checker
 * separately compiles driver.c and helpers.c to retain opaque call effects. */
#include "llvm_guarded_folds/driver.c"
#include "llvm_guarded_folds/helpers.c"
