/* A file-scope object's value is a definite negative __builtin_constant_p
 * answer even in an integer-constant-expression context. This must let
 * __builtin_choose_expr initialize static storage, while parameter queries
 * remain eligible for later inlining/propagation. */
#include <stdlib.h>

int dynamic_global;
int selected = __builtin_choose_expr(
    !__builtin_constant_p(dynamic_global), 37, 99);

static int query_parameter(int value)
{
    return __builtin_choose_expr(
        !__builtin_constant_p(value), 11, dynamic_global++);
}

int main(void)
{
    if (selected != 37 || dynamic_global != 0)
        abort();
    if (query_parameter(4) != 11 || dynamic_global != 0)
        abort();
    return 0;
}
