/* A function-like macro parameter named like an encoding prefix must not be
 * substituted when that spelling is part of a character/string literal token. */
#include <stdlib.h>
#include <wchar.h>

#define WIDE_CHAR(L) (L'Q' + (L))
#define WIDE_STRING(L) L"ok"

int main(void)
{
    if (WIDE_CHAR(0) != L'Q')
        abort();
    if (sizeof(WIDE_STRING(7)) != 3 * sizeof(wchar_t))
        abort();
    return 0;
}
