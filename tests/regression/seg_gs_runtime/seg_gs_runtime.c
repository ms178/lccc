/* Runtime, self-checking contract for x86-64 named address space __seg_gs.
 *
 * The GS base register is pointed at `area`, so every access through a
 * __seg_gs object or pointer is a real %gs-relative access. Each check
 * compares the observed value with an expected value computed through
 * ordinary (generic) memory, so the program does not depend on any other
 * compiler to decide pass or fail.
 *
 * Covered:
 *   D2  indexed / pointer-arithmetic member access keeps %gs
 *   D4  whole-struct and vector copies to and from a segment object
 *       (initializer, assignment, argument, return), including a 320-byte
 *       object that takes the counted-loop copy path
 *   D7  typedef base with a pointer declarator keeps the pointer itself
 *       in ordinary memory (only the pointee is %gs)
 *   D6  bitfield loads, stores, compound assignment and ++/-- through a
 *       segment pointer, including a field that straddles a 64-bit unit
 */
#include <stdio.h>
#include <sys/syscall.h>
#include <unistd.h>

#define ARCH_SET_GS 0x1001
#define AREA 8192

struct pair { long a, b; };          /* 16 B: packed-register/SSE class */
struct tiny { int x, y; };           /* 8 B: single-register class */
struct trio { long v[3]; };          /* 24 B: memory class */
struct big  { long v[40]; };         /* 320 B: counted-loop copy path */
struct ent  { long v[2]; long f; };  /* 24 B: indexed member access */
struct bf   { unsigned a:3; unsigned b:13; int c:7; unsigned long d:40;
              unsigned long e:30; };  /* e spans bits 63..92: crosses a unit */
typedef struct pair PT;
typedef long L;
typedef long v2_t __attribute__((vector_size(16)));

static unsigned char area[AREA] __attribute__((aligned(64)));
static int failures;

#define CHECK(cond, name) do { \
    if (cond) printf("ok   %s\n", name); \
    else { printf("FAIL %s\n", name); failures++; } \
} while (0)

/* Segment-relative pointers: the value is an offset from the GS base. */
static struct pair __seg_gs *SP = (struct pair __seg_gs *)0;
static struct pair __seg_gs *SP1 = (struct pair __seg_gs *)16;
static struct tiny __seg_gs *ST = (struct tiny __seg_gs *)64;
static struct trio __seg_gs *STR = (struct trio __seg_gs *)128;
static struct big __seg_gs *SB = (struct big __seg_gs *)512;
static v2_t __seg_gs *SV = (v2_t __seg_gs *)1024;
static struct ent __seg_gs *GP = (struct ent __seg_gs *)2048;
static PT __seg_gs *TB = (PT __seg_gs *)2400;   /* D7: typedef pointer */
static L __seg_gs *TA = (L __seg_gs *)2432;     /* D7: typedef pointer */
static struct bf __seg_gs *SBF = (struct bf __seg_gs *)3072;  /* D6 */

__attribute__((noinline)) static long take_pair(struct pair p) { return p.a * 1000 + p.b; }
__attribute__((noinline)) static struct pair mk_pair(long a, long b) { struct pair p = { a, b }; return p; }
__attribute__((noinline)) static struct pair ret_seg_pair(void) { return *SP; }
__attribute__((noinline)) static struct tiny ret_seg_tiny(void) { return *ST; }
__attribute__((noinline)) static struct big mk_big(long s) { struct big b; for (int i = 0; i < 40; i++) b.v[i] = s + i; return b; }

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    if (syscall(SYS_arch_prctl, ARCH_SET_GS, area) != 0) { perror("arch_prctl"); return 2; }
    struct pair *gen_pair = (struct pair *)area;
    struct tiny *gen_tiny = (struct tiny *)(area + 64);
    struct trio *gen_trio = (struct trio *)(area + 128);
    struct big *gen_big = (struct big *)(area + 512);
    v2_t *gen_v = (v2_t *)(area + 1024);
    struct ent *gen_ent = (struct ent *)(area + 2048);
    struct pair *gen_td = (struct pair *)(area + 2400);
    long *gen_ta = (long *)(area + 2432);
    struct bf *gen_bf = (struct bf *)(area + 3072);

    /* D4: destination write, 16 B struct */
    { struct pair t = mk_pair(11, 22); *SP = t; CHECK(gen_pair->a == 11 && gen_pair->b == 22, "struct_copy_to_seg_16"); }
    /* D4: source read by initializer, 16 B */
    { gen_pair->a = 5; gen_pair->b = 6; struct pair t = *SP; CHECK(t.a == 5 && t.b == 6, "struct_init_from_seg_16"); }
    /* D4: struct assignment seg -> seg+16 */
    { *SP1 = *SP; CHECK(gen_pair[1].a == 5 && gen_pair[1].b == 6, "struct_assign_seg_to_seg"); }
    /* D4: by-value argument from segment */
    { long r = take_pair(*SP); CHECK(r == 5006, "struct_arg_from_seg"); }
    /* D4: return of a segment struct, and return into a segment object */
    { struct pair r = ret_seg_pair(); CHECK(r.a == 5 && r.b == 6, "struct_ret_from_seg"); }
    { *SP = mk_pair(7, 8); CHECK(gen_pair->a == 7 && gen_pair->b == 8, "struct_ret_to_seg"); }
    /* D4: 8 B struct, single-register path */
    { gen_tiny->x = 3; gen_tiny->y = 4; struct tiny t = *ST; CHECK(t.x == 3 && t.y == 4, "tiny_init_from_seg");
      struct tiny u = ret_seg_tiny(); CHECK(u.x == 3 && u.y == 4, "tiny_ret_from_seg");
      t.x = 9; t.y = 10; *ST = t; CHECK(gen_tiny->x == 9 && gen_tiny->y == 10, "tiny_copy_to_seg"); }
    /* D4: 24 B memory-class struct */
    { struct trio t = { { 1, 2, 3 } }; *STR = t; CHECK(gen_trio->v[0] == 1 && gen_trio->v[2] == 3, "trio_copy_to_seg");
      struct trio u = *STR; CHECK(u.v[1] == 2, "trio_init_from_seg"); }
    /* D4: 320 B struct, counted-loop copy path (> 256 bytes) */
    { struct big b = mk_big(100); *SB = b; int ok = 1;
      for (int i = 0; i < 40; i++) ok &= gen_big->v[i] == 100 + i;
      CHECK(ok, "big_copy_to_seg_loop");
      struct big c = *SB; ok = 1;
      for (int i = 0; i < 40; i++) ok &= c.v[i] == 100 + i;
      CHECK(ok, "big_init_from_seg_loop"); }
    /* D4: vector object in a segment */
    { v2_t t = { 7, 9 }; *SV = t; CHECK(gen_v[0][0] == 7 && gen_v[0][1] == 9, "vec_copy_to_seg");
      v2_t u = *SV; CHECK(u[0] == 7 && u[1] == 9, "vec_init_from_seg"); }
    /* D2: indexed and pointer-arithmetic member access keeps %gs */
    { GP[1].v[0] = 5; CHECK(gen_ent[1].v[0] == 5, "d2_indexed_store_member_array");
      (GP + 1)[0].f = 13; CHECK(gen_ent[1].f == 13, "d2_ptr_arith_store_member");
      gen_ent[1].f = 21; CHECK(GP[1].f == 21, "d2_indexed_load_member");
      CHECK((GP + 1)[0].f == 21, "d2_ptr_arith_load_member");
      GP[1].v[1] += 3; CHECK(gen_ent[1].v[1] == 3, "d2_compound_indexed_member"); }
    /* D7: typedef base with a pointer declarator */
    { gen_td->b = 42; CHECK(TB->b == 42, "d7_typedef_struct_ptr_load");
      gen_ta[0] = 77; CHECK(*TA == 77, "d7_typedef_scalar_ptr_load"); }

    /* D6: bitfields through a segment pointer. Every expected value is read
       back through the generic view of the same bytes (gen_bf). */
    { SBF->a = 5; SBF->b = 0x1abc; SBF->c = -9; SBF->d = 0xabcdef1234UL; SBF->e = 0x2badcafeUL;
      CHECK(gen_bf->a == 5 && gen_bf->b == 0x1abc && gen_bf->c == -9
            && gen_bf->d == 0xabcdef1234UL && gen_bf->e == 0x2badcafeUL, "d6_bitfield_store_seg");
      CHECK(SBF->a == 5 && SBF->b == 0x1abc && SBF->c == -9
            && SBF->d == 0xabcdef1234UL && SBF->e == 0x2badcafeUL, "d6_bitfield_load_seg");
      gen_bf->e = 0x0badf00dUL; CHECK(SBF->e == 0x0badf00dUL && SBF->d == 0xabcdef1234UL, "d6_straddle_unit_load");
      SBF->e = 0x2badcafeUL; CHECK(gen_bf->d == 0xabcdef1234UL && gen_bf->c == -9, "d6_straddle_unit_store_neighbours");
      SBF->b += 7; CHECK(gen_bf->b == 0x1ac3 && gen_bf->a == 5, "d6_compound_add_seg");
      SBF->c--; CHECK(gen_bf->c == -10 && gen_bf->b == 0x1ac3, "d6_postdec_seg");
      unsigned long old = SBF->e++; CHECK(old == 0x2badcafeUL && gen_bf->e == 0x2badcaffUL, "d6_postinc_straddle_seg");
      unsigned long nw = ++SBF->d; CHECK(nw == 0xabcdef1235UL && gen_bf->d == 0xabcdef1235UL, "d6_preinc_seg");
      SBF->a = 2; SBF->a |= 1; CHECK(gen_bf->a == 3, "d6_bitor_seg");
      CHECK((SBF)->b == 0x1ac3 && (*SBF).c == -10, "d6_paren_and_deref_load");
      /* Generic object: the initializer path must still store at the right bits. */
      struct bf g = { 1, 2, -3, 4, 5 };
      CHECK(g.a == 1 && g.b == 2 && g.c == -3 && g.d == 4 && g.e == 5, "d6_generic_init_control");
    }

    printf("%s (%d failure%s)\n", failures ? "FAILED" : "PASSED", failures, failures == 1 ? "" : "s");
    return failures ? 1 : 0;
}
