/* expat/glibc-style UTF-8 well-formedness scan: the byte-classification
 * core of XmlUtf8Convert / mbrtowc fast paths.  Mostly-ASCII input with
 * periodic 2/3/4-byte sequences: a data-dependent branch ladder over
 * lead-byte ranges plus continuation-shape checks.  Exercises branch
 * prediction friendliness, range-test lowering ((c-0xC2) <= 0x1D style),
 * and short-vector scan opportunities on the ASCII runs. Returns a
 * checksum over code points so a miscompiled arm cannot win by scanning
 * less.
 *
 * Precision about what this is NOT: a strict RFC 3629 validator.  It
 * checks lead-byte ranges and continuation shapes, but not the second-
 * byte subranges that reject overlong forms (E0: A0-BF, F0: 90-BF),
 * UTF-16 surrogates (ED: 80-9F) or values above U+10FFFF (F4: 80-8F).
 * That is the same coarse shape as the first-pass scanners in real XML
 * pipelines (fine classification happens later, on demand), and the
 * generated input contains none of those cases -- but do not cite this
 * kernel as "UTF-8 validation" without the qualifier. */
#include <stddef.h>
#include <stdint.h>
#include "bench.h"

#define N 8192
static unsigned char buf[N];

void bench_setup(void) {
    size_t i = 0;
    uint32_t s = 0x12345678u;
    while (i + 4 < N) {
        s = s * 1664525u + 1013904223u;
        uint32_t r = s >> 24;
        if (r < 200) { buf[i++] = (unsigned char)('a' + (r % 26u)); }
        else if (r < 225) { buf[i++] = 0xC3; buf[i++] = 0xA9; }                    /* é */
        else if (r < 243) { buf[i++] = 0xE2; buf[i++] = 0x82; buf[i++] = 0xAC; }   /* € */
        else { buf[i++] = 0xF0; buf[i++] = 0x9F; buf[i++] = 0x98; buf[i++] = 0x80; } /* 😀 */
    }
    while (i < N) buf[i++] = 'x';
}

static uint64_t utf8_scan(const unsigned char *b, size_t len) {
    uint64_t acc = 0, bad = 0;
    size_t i = 0;
    while (i < len) {
        uint32_t c = b[i];
        if (c < 0x80u) { acc += c; i += 1; continue; }
        if (c >= 0xC2u && c <= 0xDFu && i + 1 < len && (b[i + 1] & 0xC0u) == 0x80u) {
            acc += ((c & 0x1Fu) << 6) | (b[i + 1] & 0x3Fu); i += 2; continue;
        }
        if (c >= 0xE0u && c <= 0xEFu && i + 2 < len
                && (b[i + 1] & 0xC0u) == 0x80u && (b[i + 2] & 0xC0u) == 0x80u) {
            acc += ((c & 0x0Fu) << 12) | ((b[i + 1] & 0x3Fu) << 6) | (b[i + 2] & 0x3Fu);
            i += 3; continue;
        }
        if (c >= 0xF0u && c <= 0xF4u && i + 3 < len
                && (b[i + 1] & 0xC0u) == 0x80u && (b[i + 2] & 0xC0u) == 0x80u
                && (b[i + 3] & 0xC0u) == 0x80u) {
            acc += ((c & 0x07u) << 18) | ((b[i + 1] & 0x3Fu) << 12)
                 | ((b[i + 2] & 0x3Fu) << 6) | (b[i + 3] & 0x3Fu);
            i += 4; continue;
        }
        bad++; i += 1;
    }
    return acc * 131u + bad;
}

int bench_verify(void) {
    /* Code-point accumulation over one sequence of each length, plus the
     * bad-byte accounting: "a" + U+00E9 + U+20AC + U+1F600 sums to
     * 97 + 233 + 8364 + 128512 = 137206, folded as acc*131 + bad. */
    static const unsigned char seq[] = {
        'a', 0xC3, 0xA9, 0xE2, 0x82, 0xAC, 0xF0, 0x9F, 0x98, 0x80 };
    if (utf8_scan(seq, sizeof(seq)) != 137206u * 131u) return 1;
    /* Lone continuation byte and a truncated 3-byte lead are bad bytes,
     * never silently accepted sequences. */
    static const unsigned char bad1[] = { 0x80 };
    static const unsigned char bad2[] = { 'x', 0xE2, 0x82 };
    if (utf8_scan(bad1, sizeof(bad1)) != 1u) return 2;
    if (utf8_scan(bad2, sizeof(bad2)) != (uint64_t)'x' * 131u + 2u) return 3;
    return 0;
}

unsigned long long bench_run(void) { return utf8_scan(buf, N); }
