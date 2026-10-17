#include <stdio.h>
#include <string.h>

static size_t through_parameter(const char *text)
{
    return strlen(text);
}

int main(void)
{
    static const char text[] = "ab\0cd";
    size_t direct_builtin = __builtin_strlen("ab\0cd");
    size_t direct_library = strlen("ab\0cd");
    size_t parameter_library = through_parameter("ab\0cd");

    if (sizeof(text) != 6 || direct_builtin != 2 || direct_library != 2 ||
        parameter_library != 2)
        return 1;
    printf("%zu %zu %zu %zu\n", sizeof(text), direct_builtin,
           direct_library, parameter_library);
    return 0;
}
