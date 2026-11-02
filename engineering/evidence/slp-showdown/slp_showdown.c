// BB-SLP oracle showdown: the classic basic-block SLP pattern zoo.
// Every function is straight-line (no loops) so ONLY SLP can vectorize.
#include <stdint.h>

typedef struct { uint64_t a, b, c, d; } Q4;
typedef struct { double x, y; } D2;
typedef struct { uint32_t a, b, c, d; } W4;
typedef struct { uint16_t a, b, c, d, e, f, g, h; } H8;
typedef struct { uint8_t p[16]; } B16;

void copy_q4(Q4 *restrict d, const Q4 *restrict s) {
    d->a = s->a; d->b = s->b; d->c = s->c; d->d = s->d;
}
void copy_d2(D2 *restrict d, const D2 *restrict s) {
    d->x = s->x; d->y = s->y;
}
void copy_w4(W4 *restrict d, const W4 *restrict s) {
    d->a = s->a; d->b = s->b; d->c = s->c; d->d = s->d;
}
void copy_b16(B16 *restrict d, const B16 *restrict s) {
    for (int i = 0; i < 16; i++) d->p[i] = s->p[i];
}
void xor_q4(Q4 *restrict d, const Q4 *restrict a, const Q4 *restrict b) {
    d->a = a->a ^ b->a; d->b = a->b ^ b->b;
    d->c = a->c ^ b->c; d->d = a->d ^ b->d;
}
void add_q4(Q4 *restrict d, const Q4 *restrict a, const Q4 *restrict b) {
    d->a = a->a + b->a; d->b = a->b + b->b;
    d->c = a->c + b->c; d->d = a->d + b->d;
}
void sub_d2(D2 *restrict d, const D2 *restrict a, const D2 *restrict b) {
    d->x = a->x - b->x; d->y = a->y - b->y;
}
void mul_h8(H8 *restrict d, const H8 *restrict a, const H8 *restrict b) {
    d->a = a->a * b->a; d->b = a->b * b->b;
    d->c = a->c * b->c; d->d = a->d * b->d;
    d->e = a->e * b->e; d->f = a->f * b->f;
    d->g = a->g * b->g; d->h = a->h * b->h;
}
void add_b16(B16 *restrict d, const B16 *restrict a, const B16 *restrict b) {
    for (int i = 0; i < 16; i++) d->p[i] = a->p[i] + b->p[i];
}
void ksub_w4(W4 *restrict d, const W4 *restrict a) {
    d->a = a->a - 1; d->b = a->b - 1; d->c = a->c - 1; d->d = a->d - 1;
}
void madd_q4(Q4 *restrict d, const Q4 *restrict a, const Q4 *restrict b,
             const Q4 *restrict c) {
    d->a = a->a + b->a + c->a; d->b = a->b + b->b + c->b;
    d->c = a->c + b->c + c->c; d->d = a->d + b->d + c->d;
}
void mixed(D2 *restrict dd, const D2 *restrict ds,
           W4 *restrict wd, const W4 *restrict ws) {
    dd->x = ds->x; dd->y = ds->y;
    wd->a = ws->a; wd->b = ws->b;
}
