/* SIB base==index + loop-wrap soundness (zlib-ng deflateHeaders `put_byte`
 * shape): base pointer and index both come from the same struct, the store
 * `s->buf[s->pos++]` folded into a SIB operand whose in-place index
 * extension must not clobber the base, and the index stays live across the
 * loop back edge (RA folded-index loop-wrap protection). Differential vs
 * the GCC oracle on the transformed-buffer checksum. */
#include <stdio.h>
#include <stdlib.h>

struct outbuf {
    unsigned char *buf;
    unsigned pos;
    unsigned char *lit;
    unsigned lpos;
};

__attribute__((noinline)) static void put_bytes(struct outbuf *s,
                                                const unsigned char *p,
                                                unsigned n) {
    for (unsigned i = 0; i < n; i++) {
        s->buf[s->pos++] = (unsigned char)(p[i] ^ 0x5A);
        s->lit[s->lpos++] = (unsigned char)(p[i] + i);
    }
}

int main(void) {
    struct outbuf s;
    s.buf = malloc(512);
    s.lit = malloc(512);
    s.pos = 0;
    s.lpos = 0;
    unsigned char in[257];
    for (unsigned i = 0; i < 257; i++)
        in[i] = (unsigned char)(i * 31 + 7);
    put_bytes(&s, in, 257);
    unsigned long h = 1469598103934665603ULL;
    for (unsigned i = 0; i < s.pos; i++) {
        h ^= s.buf[i];
        h *= 1099511628211ULL;
    }
    for (unsigned i = 0; i < s.lpos; i++) {
        h ^= s.lit[i];
        h *= 1099511628211ULL;
    }
    printf("pos=%u lpos=%u h=%016llx\n", s.pos, s.lpos, h);
    free(s.buf);
    free(s.lit);
    return 0;
}
