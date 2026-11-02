/* `__INCLUDE_LEVEL__` is the depth of the file the token appears in, not of the
   macro that mentions it: the alias below is defined in the header at level 1
   and expanded in the main file, where it must still read 0. Both oracles
   agree; a frontend that stored the value in the macro body would print 1. */
#include "171_include_level.h"

int main_level = __INCLUDE_LEVEL__;
int alias_here = LEVEL_ALIAS;
