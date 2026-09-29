/* Strings and pointers: the library shapes a compiler most often has to
 * recognise and inline (strlen/memcpy/memcmp/strcpy), plus pointer walks
 * that defeat naive strength reduction.  Results are order-sensitive on
 * purpose: an inlined vs library-call difference shows up immediately. */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

static uint64_t hash(const char *s, uint64_t h) {
    while (*s) h = (h ^ (unsigned char)*s++) * 1099511628211ull;
    return h;
}
static size_t count_byte(const char *s, char c) {
    size_t n = 0;
    while (*s) n += (*s++ == c);
    return n;
}
static int str_cmp_words(const char *a, const char *b) {
    while (*a && *a == *b) { a++; b++; }
    return (int)((unsigned char)*a) - (int)((unsigned char)*b);
}
static void rev(char *s) {
    size_t n = strlen(s);
    for (size_t i = 0; i < n / 2; i++) {
        char t = s[i]; s[i] = s[n - 1 - i]; s[n - 1 - i] = t;
    }
}
static int memcmp_scan(const unsigned char *a, const unsigned char *b, size_t n) {
    for (size_t i = 0; i < n; i++)
        if (a[i] != b[i]) return (int)a[i] - (int)b[i];
    return 0;
}

int main(void) {
    /* Automatic: see the note in pixmap.c about Compiler Explorer's asm
     * view and section directives. */
    char buf[512], other[512];
    uint64_t h = 0;
    for (int i = 0; i < 64; i++) {
        int n = 0;
        for (int k = 0; k < 200 && n < 480; k++)
            n += snprintf(buf + n, sizeof buf - (size_t)n, "%d:%s;", k, "abcdefghijklmnop");
        rev(buf);
        h ^= hash(buf, 1469598103934665603ull);
        h += count_byte(buf, ':') * 1000003ull;
        h ^= strlen(buf) * 31ull;
        memcpy(other, buf, sizeof other);
        other[sizeof other - 1] = 0;
        h += (uint64_t)(int64_t)str_cmp_words(buf, other);
    }
    printf("strings %016llx\n", (unsigned long long)h);

    unsigned char a[256], b[256];
    for (int i = 0; i < 256; i++) { a[i] = (unsigned char)(i * 3); b[i] = (unsigned char)(i * 3); }
    b[7] ^= 0xFF; b[200] ^= 0x01;
    printf("strings memcmp %d %d\n", memcmp_scan(a, b, 256), memcmp(a, b, 256));
    return 0;
}
