/* CPU tuning model — the same block copies at the Generic row (no -mtune):
 * the vector loop path must be taken (Generic has no ERMS), with 16-byte
 * SSE2 moves at the baseline -march.  Also exercises a copy whose size is
 * not a multiple of the loop chunk so the remainder ladder runs. */
#include <stdio.h>
#include <string.h>

struct s65 { unsigned char b[65]; };
struct s130 { unsigned char b[130]; };
struct s4097 { unsigned char b[4097]; };

__attribute__((noinline)) void c65(struct s65 *d, const struct s65 *s) { *d = *s; }
__attribute__((noinline)) void c130(struct s130 *d, const struct s130 *s) { *d = *s; }
__attribute__((noinline)) void c4097(struct s4097 *d, const struct s4097 *s) { *d = *s; }

static struct s65 a65, b65;
static struct s130 a130, b130;
static struct s4097 a4097, b4097;

int main(void) {
    unsigned sum = 0;
    for (unsigned i = 0; i < sizeof a65; i++) a65.b[i] = (unsigned char)(i + 1);
    for (unsigned i = 0; i < sizeof a130; i++) a130.b[i] = (unsigned char)(i * 3 + 1);
    for (unsigned i = 0; i < sizeof a4097; i++) a4097.b[i] = (unsigned char)(i * 7 + 1);
    c65(&b65, &a65);
    c130(&b130, &a130);
    c4097(&b4097, &a4097);
    if (memcmp(&a65, &b65, sizeof a65)) { puts("FAIL 65"); return 1; }
    if (memcmp(&a130, &b130, sizeof a130)) { puts("FAIL 130"); return 1; }
    if (memcmp(&a4097, &b4097, sizeof a4097)) { puts("FAIL 4097"); return 1; }
    for (unsigned i = 0; i < sizeof b4097; i++) sum = sum * 31 + b4097.b[i];
    printf("ok %u %u %u\n", b65.b[64], b130.b[129], sum);
    return 0;
}
