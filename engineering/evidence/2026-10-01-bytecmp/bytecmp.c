/* Byte-compare (two-stream lockstep) loop shapes used by the bytecmp epic. */
#include <stddef.h>

/* Shape A: the canonical byte loop. */
const unsigned char *bc_scan(const unsigned char *p, const unsigned char *end,
                             const unsigned char *q)
{
    while (p < end && *p == *q) { p++; q++; }
    return p;
}

/* Shape A': signed bounds, as the frontend emits for `char *`. */
const char *bc_scan_signed(const char *p, const char *end, const char *q)
{
    while (p < end && *p == *q) { p++; q++; }
    return p;
}

/* Shape B: the LZ4/ZSTD match-extension shape (word loop + byte tail). */
size_t match_extend(const unsigned char *p, const unsigned char *q,
                    const unsigned char *limit)
{
    const unsigned char *start = p;
    while (p + 8 <= limit && *(const unsigned long *)p == *(const unsigned long *)q) {
        p += 8; q += 8;
    }
    while (p < limit && *p == *q) { p++; q++; }
    return (size_t)(p - start);
}
