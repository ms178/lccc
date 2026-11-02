#include <stdio.h>
#include <string.h>
#include <wchar.h>
#define P(e) printf("%s = %lld\n", #e, (long long)(e))
int main(void) {
    P('\0'); P('\a'); P('\b'); P('\f'); P('\n'); P('\r'); P('\t'); P('\v');
    P('\\'); P('\''); P('\"'); P('\?'); P('\101'); P('\x41'); P('\377');
    const char *s = "ab" "cd" "\x41\102" "\0tail";
    printf("%zu %d %d %d\n", sizeof("abcd"), s[0], s[4], s[5]);
    printf("%zu\n", strlen("a\0b"));
    printf("%zu %zu\n", sizeof(L"wide"), sizeof(u"u16"));
    printf("%zu %zu\n", sizeof(u8"u8"), sizeof(U"u32"));
    printf("%d %d\n", (int)("abc"[1]), (int)(*("xyz")));
    char arr[] = "hi"; printf("%zu %zu\n", sizeof arr, sizeof("hi"));
    printf("%zu\n", sizeof("a" "b" "c"));
    printf("%d\n", (int)L'x');
    printf("%d\n", (int)u'A');
    return 0;
}
