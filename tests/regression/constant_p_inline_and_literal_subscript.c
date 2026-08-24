/* External-linkage inline bodies are valid inlining candidates. This enables
 * __builtin_constant_p(parameter) specialization; a constant-indexed string
 * literal element is itself a compiler constant. */
#include <stdlib.h>

inline int parameter_is_constant(int value)
{
    return __builtin_constant_p(value);
}

static int specialized(void)
{
    return parameter_is_constant(1234);
}

int main(void)
{
    if (!specialized())
        abort();
    if (!__builtin_constant_p("lccc"[2]))
        abort();
    return 0;
}
