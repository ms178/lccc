// BB-SLP correctness battery: copy chains of every supported lane family,
// ALU trees, hazards that MUST stay scalar, gather cost gates, extracts,
// and strict-FP lane exactness.
//
// Part of tests/regression/check_bb_slp_codegen.sh (runtime + codegen).
#include <stdio.h>
#include <stdint.h>

typedef struct { double x, y; } TwoD;
typedef struct { float a, b, c, d; } FourF;
typedef struct { uint32_t a, b, c, d; } FourU32;
typedef struct { uint16_t a, b, c, d, e, f, g, h; } EightU16;
typedef struct { uint8_t a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p; } SixteenU8;
typedef struct { uint64_t a, b, c, d; } FourU64;

TwoD g2; FourF g4f; FourU32 g4u; EightU16 g8h; SixteenU8 g16b; FourU64 g4q;
TwoD s2 = {1.5, 2.5}; FourF s4f = {1,2,3,4}; FourU32 s4u = {10,20,30,40};
EightU16 s8h = {1,2,3,4,5,6,7,8};
SixteenU8 s16b = {1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16};
FourU64 s4q = {0x1111, 0x2222, 0x3333, 0x4444};

// ── copy chains (the epic) ─────────────────────────────────────────────
void copy_2f64(TwoD *o, const TwoD *i) { *o = *i; __asm__ volatile("" :: "r"(o), "r"(i)); }
void copy_4f32(FourF *o, const FourF *i) { o->a=i->a; o->b=i->b; o->c=i->c; o->d=i->d; }
void copy_4u32(FourU32 *o, const FourU32 *i) { o->a=i->a; o->b=i->b; o->c=i->c; o->d=i->d; }
void copy_8u16(EightU16 *o, const EightU16 *i) { o->a=i->a; o->b=i->b; o->c=i->c; o->d=i->d; o->e=i->e; o->f=i->f; o->g=i->g; o->h=i->h; }
void copy_16u8(SixteenU8 *o, const SixteenU8 *i) { o->a=i->a; o->b=i->b; o->c=i->c; o->d=i->d; o->e=i->e; o->f=i->f; o->g=i->g; o->h=i->h; o->i=i->i; o->j=i->j; o->k=i->k; o->l=i->l; o->m=i->m; o->n=i->n; o->o=i->o; o->p=i->p; }
void copy_4u64(FourU64 *o, const FourU64 *i) { o->a=i->a; o->b=i->b; o->c=i->c; o->d=i->d; }

// ── ALU trees ──────────────────────────────────────────────────────────
void alu_add_f64(TwoD *o, const TwoD *a, const TwoD *b) { o->x=a->x+b->x; o->y=a->y+b->y; }
void alu_sub_f64(TwoD *o, const TwoD *a, const TwoD *b) { o->x=a->x-b->x; o->y=a->y-b->y; }
void alu_xor_u64(FourU64 *o, const FourU64 *a, const FourU64 *b) { o->a=a->a^b->a; o->b=a->b^b->b; o->c=a->c^b->c; o->d=a->d^b->d; }
void alu_and_u64(FourU64 *o, const FourU64 *a, const FourU64 *b) { o->a=a->a&b->a; o->b=a->b&b->b; o->c=a->c&b->c; o->d=a->d&b->d; }
void alu_or_u32(FourU32 *o, const FourU32 *a, const FourU32 *b) { o->a=a->a|b->a; o->b=a->b|b->b; o->c=a->c|b->c; o->d=a->d|b->d; }
void alu_sub_u32(FourU32 *o, const FourU32 *a, const FourU32 *b) { o->a=a->a-b->a; o->b=a->b-b->b; o->c=a->c-b->c; o->d=a->d-b->d; }
void alu_mul_u16(EightU16 *o, const EightU16 *a, const EightU16 *b) { o->a=a->a*b->a; o->b=a->b*b->b; o->c=a->c*b->c; o->d=a->d*b->d; o->e=a->e*b->e; o->f=a->f*b->f; o->g=a->g*b->g; o->h=a->h*b->h; }
void alu_add_u8(SixteenU8 *o, const SixteenU8 *a, const SixteenU8 *b) { o->a=a->a+b->a; o->b=a->b+b->b; o->c=a->c+b->c; o->d=a->d+b->d; o->e=a->e+b->e; o->f=a->f+b->f; o->g=a->g+b->g; o->h=a->h+b->h; o->i=a->i+b->i; o->j=a->j+b->j; o->k=a->k+b->k; o->l=a->l+b->l; o->m=a->m+b->m; o->n=a->n+b->n; o->o=a->o+b->o; o->p=a->p+b->p; }

// ── nested tree (two ALU levels) ───────────────────────────────────────
void alu_madd_u64(FourU64 *o, const FourU64 *a, const FourU64 *b, const FourU64 *c) {
    o->a=a->a+b->a+c->a; o->b=a->b+b->b+c->b; o->c=a->c+b->c+c->c; o->d=a->d+b->d+c->d;
}

// ── hazards: these MUST NOT be SLP-vectorized (semantics) ─────────────
uint64_t hazard_interleaved_read(FourU64 *o, const FourU64 *a) {
    o->a=a->a; o->b=a->b;
    uint64_t mid = o->a + a->a;   // read between the stores
    o->c=a->c; o->d=a->d;
    return mid;
}
uint64_t hazard_load_after_store(FourU64 *o, const FourU64 *a) {
    o->a=a->a;                    // store to o lane 0
    uint64_t v = a->b;            // load positioned between the stores
    o->b=a->b; o->c=a->c; o->d=a->d;
    return v;
}
volatile uint64_t vol_sink;
void hazard_volatile(const FourU64 *a) {
    vol_sink = a->a; vol_sink = a->b; vol_sink = a->c; vol_sink = a->d;
}

// ── extract path: external uses of loaded lanes AFTER the stores ───────
uint64_t copy_extract(FourU64 *o, const FourU64 *a) {
    uint64_t s = 0;
    o->a=a->a; o->b=a->b; o->c=a->c; o->d=a->d;
    s += a->a; s += a->b; s += a->c; s += a->d;   // external uses (post-store loads may CSE to lanes)
    return s;
}

// ── gather: mixed scalar sources (VecPack path, if profitable) ─────────
uint64_t g1, g2_, g3_, g4_;
void gather_globals(FourU64 *o) {
    o->a=g1; o->b=g2_; o->c=g3_; o->d=g4_;
}

// ── partially-overlapping lane runs (2 of 4) ──────────────────────────
void copy_run2(FourU64 *o, const FourU64 *i) { o->a=i->a; o->b=i->b; o->c=i->c; o->d=i->d; o->a=i->a; }

// ── strict FP: lane-parallel exactness must hold under -ffast-math too ─
double fp_strict(TwoD *o, const TwoD *a, const TwoD *b) {
    o->x = a->x * b->x - a->x;  // mul then sub, per lane
    o->y = a->y * b->y - a->y;
    return o->x + o->y;
}

int main(void) {
    int fails = 0;
    #define CHECK(cond, name) do { if (!(cond)) { printf("FAIL %s\n", name); fails++; } } while (0)

    TwoD a2={10.0,20.0}, b2={1.0,2.0}, r2;
    FourU64 a4={1,2,3,4}, b4={5,6,7,8}, r4;
    FourU32 a4u={100,200,300,400}, b4u={1,2,3,4}, r4u;
    EightU16 a8h={100,200,300,400,500,600,700,800}, b8h={1,2,3,4,5,6,7,8}, r8h;
    SixteenU8 a16b={1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16}, b16b={1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1}, r16b;
    FourF a4f={1.5f,2.5f,3.5f,4.5f}, r4f;

    g2=s2; CHECK(g2.x==1.5 && g2.y==2.5, "copy 2xf64");
    g4f=s4f; CHECK(g4f.a==1 && g4f.d==4, "copy 4xf32");
    g4u=s4u; CHECK(g4u.a==10 && g4u.d==40, "copy 4xu32");
    g8h=s8h; CHECK(g8h.a==1 && g8h.h==8, "copy 8xu16");
    g16b=s16b; CHECK(g16b.a==1 && g16b.p==16, "copy 16xu8");
    g4q=s4q; CHECK(g4q.a==0x1111 && g4q.d==0x4444, "copy 4xu64");

    alu_add_f64(&r2,&a2,&b2); CHECK(r2.x==11.0&&r2.y==22.0, "add f64");
    alu_sub_f64(&r2,&a2,&b2); CHECK(r2.x==9.0&&r2.y==18.0, "sub f64");
    alu_sub_f64(&r2,&b2,&a2); CHECK(r2.x==-9.0&&r2.y==-18.0, "sub order");
    alu_xor_u64(&r4,&a4,&b4); CHECK(r4.a==4&&r4.b==4&&r4.c==4&&r4.d==12, "xor u64");
    alu_and_u64(&r4,&a4,&b4); CHECK(r4.a==1&&r4.b==2&&r4.c==3&&r4.d==0, "and u64");
    alu_or_u32(&r4u,&a4u,&b4u); CHECK(r4u.a==101&&r4u.d==404, "or u32");
    alu_sub_u32(&r4u,&a4u,&b4u); CHECK(r4u.a==99&&r4u.d==396, "sub u32");
    alu_sub_u32(&r4u,&b4u,&a4u); CHECK(r4u.a==(uint32_t)-99&&r4u.d==(uint32_t)-396, "sub u32 order");
    alu_mul_u16(&r8h,&a8h,&b8h); CHECK(r8h.a==100&&r8h.h==6400, "mul u16");
    alu_add_u8(&r16b,&a16b,&b16b); CHECK(r16b.a==2&&r16b.p==17, "add u8");
    alu_madd_u64(&r4,&a4,&b4,&b4); CHECK(r4.a==11&&r4.d==20, "madd u64");

    FourU64 h1; CHECK(hazard_interleaved_read(&h1,&a4)==2, "hazard read");
    FourU64 h2; CHECK(hazard_load_after_store(&h2,&a4)==2, "hazard load");
    hazard_volatile(&a4);

    FourU64 ce; CHECK(copy_extract(&ce,&a4)==10, "copy extract");

    g1=11; g2_=22; g3_=33; g4_=44;
    FourU64 gg; gather_globals(&gg); CHECK(gg.a==11&&gg.d==44, "gather globals");

    FourU64 r2c; copy_run2(&r2c,&a4); CHECK(r2c.a==1&&r2c.d==4, "copy run2");

    TwoD fs; double fsv = fp_strict(&fs,&a2,&b2);
    CHECK(fs.x==10.0*1.0-10.0 && fs.y==20.0*2.0-20.0, "fp strict lanes");

    printf(fails ? "bb_slp_basic FAILURES: %d\n" : "bb_slp_basic: all pass (%d fails)\n", fails);
    return fails != 0;
}
