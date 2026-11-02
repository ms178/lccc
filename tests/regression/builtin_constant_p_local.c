/* At optimization levels, a local initialized from a constant is a valid
 * __builtin_constant_p result even when the containing function is not an
 * inline candidate. */
#include <stdlib.h>

int main(void)
{
    int word_size = (int)sizeof(int);
#ifdef __OPTIMIZE__
    if (!__builtin_constant_p(word_size))
        abort();
#endif
    return 0;
}
