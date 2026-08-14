/* string library ops — the hot paths in gzip/expat/zlib
 * (strcmp/strlen/strcpy/strchr/memchr on edge cases). */
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

int main(void) {
    /* strlen edge cases */
    if (strlen("") != 0) return 1;
    if (strlen("a") != 1) return 2;
    if (strlen("hello world") != 11) return 3;
    char longstr[512];
    memset(longstr, 'x', 511); longstr[511] = 0;
    if (strlen(longstr) != 511) return 4;

    /* strcmp */
    if (strcmp("abc", "abc") != 0) return 5;
    if (strcmp("abc", "abd") >= 0) return 6;
    if (strcmp("abd", "abc") <= 0) return 7;
    if (strcmp("abc", "abcd") >= 0) return 8;   /* prefix shorter sorts first */
    if (strcmp("", "") != 0) return 9;

    /* strncmp */
    if (strncmp("abcdef", "abcxyz", 3) != 0) return 10;
    if (strncmp("abcdef", "abcxyz", 4) >= 0) return 11;

    /* strcpy / strncpy */
    char out[64];
    strcpy(out, "copy me");
    if (strcmp(out, "copy me") != 0) return 12;
    strncpy(out, "short", 20);
    if (out[5] != 0 || out[19] != 0) return 13;   /* zero-padded */
    if (strcmp(out, "short") != 0) return 14;

    /* strchr / strrchr / memchr */
    if (strchr("hello", 'l') != ("hello" + 2)) return 15;
    if (strrchr("hello", 'l') != ("hello" + 3)) return 16;
    if (strchr("hello", 'z') != NULL) return 17;
    const char *mem = "abcdefabcdef";
    if (memchr(mem, 'd', 12) != mem + 3) return 18;
    if (memchr(mem, 'z', 12) != NULL) return 19;
    if (memchr(mem, 'd', 3) != NULL) return 20;   /* not in first 3 */

    /* strstr */
    if (strstr("hello world", "world") != ("hello world" + 6)) return 21;
    if (strstr("hello world", "xyz") != NULL) return 22;
    if (strstr("aaaa", "aa") != ("aaaa")) return 23;

    /* atoi/strtol */
    if (atoi("42") != 42) return 24;
    if (atoi("-17") != -17) return 25;
    if (atoi("  123abc") != 123) return 26;
    if (strtol("0x1F", NULL, 16) != 31) return 27;
    if (strtol("101", NULL, 2) != 5) return 28;

    /* sprintf family */
    char fmt[128];
    sprintf(fmt, "%d-%s-%.1f", 7, "str", 2.5);
    if (strcmp(fmt, "7-str-2.5") != 0) return 29;
    int w = snprintf(fmt, 8, "%d", 123456789);
    if (w != 9) return 30;
    if (strcmp(fmt, "1234567") != 0) return 31;   /* truncated to 7 chars */

    /* memcmp */
    if (memcmp("abc", "abd", 3) >= 0) return 32;
    if (memcmp("abc", "abc", 3) != 0) return 33;

    printf("OK string_ops\n");
    return 0;
}
