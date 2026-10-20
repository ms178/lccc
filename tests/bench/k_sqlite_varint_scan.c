/* SQLite record scan: sqlite3GetVarint32-style header walking.
 * A synthetic page of records, each header a chain of 7-bit varints (length
 * prefix + serial types); the kernel walks every record, decoding varints
 * and accumulating payload lengths.  The varint loop is SQLite's hottest
 * small loop shape: unpredictable continuation bit, short trip counts,
 * byte loads feeding a shift-or accumulation chain.  Exercises loop peeling
 * (the 1-byte fast path dominates), branch layout and byte-load folding. */
#include <stddef.h>
#include <stdint.h>
#include "bench.h"

#define N 16384
static unsigned char page[N];

static size_t put_varint(unsigned char *p, uint32_t v) {
    unsigned char tmp[5];
    int n = 0;
    do { tmp[n++] = (unsigned char)(v & 0x7fu); v >>= 7; } while (v);
    size_t out = 0;
    for (int i = n - 1; i >= 0; i--)
        p[out++] = (unsigned char)(tmp[i] | (i ? 0x80u : 0u));
    return out;
}

void bench_setup(void) {
    size_t i = 0;
    uint32_t s = 0xABCD1234u;
    while (i + 64 < N) {
        s = s * 22695477u + 1u;
        uint32_t ncols = 1u + ((s >> 8) % 6u);
        size_t hdr_at = i;
        i += 1; /* placeholder for header length */
        uint32_t payload = 0;
        for (uint32_t cix = 0; cix < ncols; cix++) {
            s = s * 22695477u + 1u;
            uint32_t serial = (s >> 11) % 128u;         /* 1-2 byte varints, payload <= 57 B */
            if ((s & 0x3ffu) == 0) serial += 200u;      /* rare 2-byte varint, longer blob */
            i += put_varint(page + i, serial);
            payload += (serial >= 13u && (serial & 1u)) ? (serial - 13u) / 2u
                       : (serial >= 12u) ? (serial - 12u) / 2u : serial % 9u;
        }
        page[hdr_at] = (unsigned char)(i - hdr_at);
        for (uint32_t k = 0; k < payload && i < N; k++) page[i++] = (unsigned char)k;
    }
    while (i < N) page[i++] = 0;
}

/* Bounded decode: bytes at index >= avail read as 0 (terminator), so a
 * varint starting within the final 4 bytes of the page can never read out
 * of bounds.  The 1-byte fast path (the overwhelmingly common case, and
 * the shape this kernel exists to measure) is untouched; the bound checks
 * live only on the already-slow continuation paths.  The unbounded
 * original was safe for this setup's data by a construction margin
 * (records start >= 64 bytes before the end and the tail is zero-filled),
 * but "safe by margin" rots the first time somebody edits setup. */
static uint32_t get_varint32(const unsigned char *p, size_t avail, uint32_t *out) {
    uint32_t v = p[0];
    if (!(v & 0x80u)) { *out = v; return 1; }
    uint32_t b1 = avail > 1 ? p[1] : 0u;
    v = (v & 0x7fu) << 7; v |= b1 & 0x7fu;
    if (!(b1 & 0x80u)) { *out = v; return 2; }
    uint32_t n = 2;
    for (;;) {
        uint32_t bn = (size_t)n < avail ? p[n] : 0u;
        v = (v << 7) | (bn & 0x7fu);
        n++;
        if (!(bn & 0x80u) || n >= 5) break;
    }
    *out = v;
    return n;
}

static uint64_t scan_records(const unsigned char *p, size_t len) {
    uint64_t acc = 0;
    size_t i = 0;
    while (i < len && p[i]) {
        uint32_t hdr_len = p[i];
        if (hdr_len < 2 || i + hdr_len > len) break;
        size_t j = i + 1, hdr_end = i + hdr_len;
        uint32_t payload = 0;
        while (j < hdr_end) {
            uint32_t serial;
            j += get_varint32(p + j, len - j, &serial);
            acc += serial;
            payload += (serial >= 13u && (serial & 1u)) ? (serial - 13u) / 2u
                       : (serial >= 12u) ? (serial - 12u) / 2u : serial % 9u;
        }
        i = hdr_end + payload;
    }
    return acc;
}

int bench_verify(void) {
    /* Encode/decode round-trip over the varint width boundaries (the
     * 7-bit group edges 127/128, 16383/16384, 2^21, 2^28, UINT32_MAX):
     * setup and scan share put/get, so a symmetric transcription bug in
     * both would cancel out in the bench checksum but not here if the
     * byte count is also asserted. */
    static const uint32_t vals[] = {0u, 1u, 127u, 128u, 300u, 16383u, 16384u,
                                    2097151u, 2097152u, 268435455u, 268435456u,
                                    4294967295u};
    unsigned char tmp[8];
    for (size_t i = 0; i < sizeof(vals) / sizeof(vals[0]); i++) {
        size_t wrote = put_varint(tmp, vals[i]);
        uint32_t back = 0;
        uint32_t read = get_varint32(tmp, sizeof(tmp), &back);
        if (back != vals[i] || (size_t)read != wrote) return 1;
    }
    /* Truncated tail: a continuation byte with nothing after it must
     * terminate (bounded read), not walk off the buffer. */
    tmp[0] = 0x80u;
    uint32_t back = 0;
    if (get_varint32(tmp, 1, &back) != 2u || back != 0u) return 2;
    return 0;
}

unsigned long long bench_run(void) { return scan_records(page, N); }
