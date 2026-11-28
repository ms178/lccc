/* `builtin_library_arguments.c` with every library function declared by its
 * header: a builtin's prototype is its own, and the declarations of the
 * functions it calls change nothing about the call. */
#include <complex.h>
#include <ctype.h>
#include <inttypes.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <wctype.h>
#include "builtin_library_arguments.c"
