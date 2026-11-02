/* curl/OpenSSL-style base64 decode (scalar table path): 4-char quanta ->
 * 3 bytes through a 256-entry classification table, values bit-packed with
 * shifts/ors.  The workload behind MIME/PEM parsing everywhere.  Exercises
 * table loads, byte extraction/packing chains, and the scalar-vs-SIMD gap
 * that vectorised base64 (aklomp) exploits -- this kernel is the scalar
 * baseline such a vectoriser must beat, and a tripwire if byte shuffles ever
 * regress.  Input is canonical encoder output built at setup. */
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include "bench.h"

#define RAW 6144
#define ENC ((RAW / 3) * 4)
static unsigned char enc[ENC];
static unsigned char dec[RAW];
static signed char tab[256];

static const char alphabet[] =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

void bench_setup(void) {
    uint32_t s = 0x0BADF00Du;
    for (int i = 0; i < 256; i++) tab[i] = -1;
    for (int i = 0; i < 64; i++) tab[(unsigned char)alphabet[i]] = (signed char)i;
    for (int i = 0; i < RAW; i += 3) {
        s = s * 1103515245u + 12345u;
        uint32_t v = s >> 8;
        int o = (i / 3) * 4;
        enc[o + 0] = (unsigned char)alphabet[(v >> 18) & 63u];
        enc[o + 1] = (unsigned char)alphabet[(v >> 12) & 63u];
        enc[o + 2] = (unsigned char)alphabet[(v >> 6) & 63u];
        enc[o + 3] = (unsigned char)alphabet[v & 63u];
    }
}

static uint64_t base64_decode(const unsigned char *in, size_t len, unsigned char *out) {
    uint64_t acc = 0;
    size_t o = 0;
    for (size_t i = 0; i + 4 <= len; i += 4) {
        uint32_t a = (uint32_t)tab[in[i]], b = (uint32_t)tab[in[i + 1]];
        uint32_t c = (uint32_t)tab[in[i + 2]], d = (uint32_t)tab[in[i + 3]];
        uint32_t v = (a << 18) | (b << 12) | (c << 6) | d;
        out[o++] = (unsigned char)(v >> 16);
        out[o++] = (unsigned char)(v >> 8);
        out[o++] = (unsigned char)v;
        acc += v;
    }
    return acc + (uint64_t)o;
}

int bench_verify(void) {
    /* RFC 4648 section 10 vectors, decode direction (whole quanta only:
     * this decoder is the no-padding scalar path).  "Zm9vYmFy" must yield
     * the bytes "foobar"; a transposed alphabet or packing shift is
     * deterministic, so cross-compiler checksum agreement alone would
     * never notice it. */
    unsigned char out[16];
    (void)base64_decode((const unsigned char *)"Zm9vYmFy", 8, out);
    if (memcmp(out, "foobar", 6) != 0) return 1;
    (void)base64_decode((const unsigned char *)"TWFueQ==", 8, out);
    /* '=' is not in the alphabet table (value -1 as uint32): only the
     * first two output bytes "Ma" are well-defined for this kernel. */
    if (out[0] != 'M' || out[1] != 'a') return 2;
    return 0;
}

unsigned long long bench_run(void) { return base64_decode(enc, ENC, dec); }
