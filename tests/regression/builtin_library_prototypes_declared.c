/* `builtin_library_prototypes.c` with every library function declared by its
 * header (glibc's <stdio.h> gives `sscanf` another symbol and, with
 * _FORTIFY_SOURCE, routes `memcpy` through `__builtin___memcpy_chk`). */
#define _GNU_SOURCE
#define _FORTIFY_SOURCE 2
#include <complex.h>
#include <ctype.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <wctype.h>
#include "builtin_library_prototypes.c"
