/* Executed observable for the builtins whose value is a property of the file:
   the include level of the file each token appears in, and that a file's
   `__TIMESTAMP__` is the 24-character ctime rendering of its own mtime. The
   timestamp itself is not asserted as a literal -- the case cannot control its
   own mtime -- it is printed so that the *oracles* have to agree with lccc on
   the same file and zone, which is the property that matters. */
#include <stdio.h>
#include <string.h>

int main_level = __INCLUDE_LEVEL__;
#include "pp_builtins_scope.h"

int main(void)
{
    printf("levels %d %d %d\n", main_level, header_level, alias_level);
    printf("timestamp length %d\n", (int)strlen(timestamp_here));
    printf("timestamp decade %c%c%c%c\n", timestamp_here[20], timestamp_here[21],
           timestamp_here[22], timestamp_here[23]);
    printf("timestamp shape %d\n", timestamp_here[3] == ' ' && timestamp_here[7] == ' '
                                      && timestamp_here[10] == ' ' && timestamp_here[19] == ' ');
    return 0;
}
