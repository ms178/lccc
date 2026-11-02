/* A small but complete workload: a deterministic image kernel.  It mixes
 * the things a real program mixes -- 2-D indexing with a stride, early
 * exits, saturating arithmetic, and a reduction -- so the measured code
 * quality is a property of the whole compiler rather than of one pass. */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

#define W 64
#define H 48

typedef struct { uint8_t r, g, b, a; } Rgba;

static inline uint8_t sat_add(uint8_t a, int v) {
    int r = (int)a + v;
    return (uint8_t)(r < 0 ? 0 : r > 255 ? 255 : r);
}
static inline uint8_t sat_mix(uint8_t a, uint8_t b, unsigned t) {   /* t in 0..256 */
    return (uint8_t)(((int)a * (256 - (int)t) + (int)b * (int)t) >> 8);
}

/* Separable-ish blur: horizontal pass then vertical, in place on a scratch. */
static void blur(const Rgba *src, Rgba *dst, Rgba *tmp, int radius) {
    for (int y = 0; y < H; y++) {
        for (int x = 0; x < W; x++) {
            int r = 0, g = 0, b = 0, n = 0;
            for (int k = -radius; k <= radius; k++) {
                int xx = x + k;
                if (xx < 0 || xx >= W) continue;
                const Rgba *p = &src[y * W + xx];
                r += p->r; g += p->g; b += p->b; n++;
            }
            Rgba *o = &tmp[y * W + x];
            o->r = (uint8_t)(r / n); o->g = (uint8_t)(g / n);
            o->b = (uint8_t)(b / n); o->a = src[y * W + x].a;
        }
    }
    for (int y = 0; y < H; y++) {
        for (int x = 0; x < W; x++) {
            int r = 0, g = 0, b = 0, n = 0;
            for (int k = -radius; k <= radius; k++) {
                int yy = y + k;
                if (yy < 0 || yy >= H) continue;
                const Rgba *p = &tmp[yy * W + x];
                r += p->r; g += p->g; b += p->b; n++;
            }
            Rgba *o = &dst[y * W + x];
            o->r = (uint8_t)(r / n); o->g = (uint8_t)(g / n);
            o->b = (uint8_t)(b / n); o->a = tmp[y * W + x].a;
        }
    }
}

static void adjust(Rgba *p, int n, unsigned bias) {
    for (int i = 0; i < n; i++) {
        p[i].r = sat_mix(p[i].r, 255, bias);
        p[i].g = sat_mix(p[i].g, 128, bias);
        p[i].b = sat_add(p[i].b, (int)bias / 8 - 16);
    }
}

int main(void) {
    /* Automatic, not file-scope `static`: a file-scope array needs
     * `.comm`/`.local`, and Compiler Explorer's asm view strips every
     * section directive, so the oracle's code cannot then be re-assembled
     * locally.  The work performed is identical; only the storage
     * duration differs. */
    Rgba src[W * H], dst[W * H], tmp[W * H];
    uint64_t h = 1469598103934665603ull;
    for (int y = 0; y < H; y++)
        for (int x = 0; x < W; x++) {
            Rgba *p = &src[y * W + x];
            p->r = (uint8_t)(x * 4 + y); p->g = (uint8_t)(x ^ y);
            p->b = (uint8_t)(x * y); p->a = 255;
        }

    blur(src, dst, tmp, 3);
    adjust(dst, W * H, 96);
    blur(dst, src, tmp, 1);

    for (int i = 0; i < W * H; i++) {
        h ^= src[i].r; h *= 1099511628211ull;
        h ^= src[i].g; h *= 1099511628211ull;
        h ^= src[i].b; h *= 1099511628211ull;
        h ^= src[i].a; h *= 1099511628211ull;
    }
    printf("pixmap %016llx\n", (unsigned long long)h);
    return 0;
}
