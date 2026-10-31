/* GNU static initializers can select using a string literal's non-null
 * array address. Linux nf_conntrack_sip.c uses this for string lengths. */
#include <stdio.h>

struct sip_header { const char *name; unsigned long len; const char *selected; };
static struct sip_header headers[] = {
    { "f", "f" ? sizeof("f") - 1 : 0, "f" ? "yes" : "no" },
    { 0, (void *)0 ? 99 : 0, "" ? "empty" : "wrong" },
    { "sip:", "sip:" ? sizeof("sip:") - 1 : 0, "sip:" ? "right" : "wrong" }
};
static int scalar = "x" ? 37 : 81;
static int nested = "a" ? ("b" ? 19 : 20) : 21;
static int local(void) {
    static unsigned long len = "local" ? sizeof("local") - 1 : 0;
    static const char *value = "local" ? "ok" : "bad";
    return len == 5 && value[0] == 'o';
}
int main(void) {
    int ok = headers[0].len == 1 && headers[1].len == 0 && headers[2].len == 4 &&
        headers[0].selected[0] == 'y' && headers[1].selected[0] == 'e' &&
        headers[2].selected[0] == 'r' && scalar == 37 && nested == 19 && local();
    printf("%d\n", ok);
    return !ok;
}
