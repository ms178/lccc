// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
# 1 "Sema/address-packed.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/address-packed.c" 2


# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdint.h" 1
# 56 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdint.h"
# 1 "/usr/include/stdint.h" 1 3 4
# 25 "/usr/include/stdint.h" 3 4
# 1 "/usr/include/features.h" 1 3 4
# 345 "/usr/include/features.h" 3 4
# 1 "/usr/include/stdc-predef.h" 1 3 4
# 346 "/usr/include/features.h" 2 3 4
# 375 "/usr/include/features.h" 3 4
# 1 "/usr/include/sys/cdefs.h" 1 3 4
# 392 "/usr/include/sys/cdefs.h" 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 393 "/usr/include/sys/cdefs.h" 2 3 4
# 376 "/usr/include/features.h" 2 3 4
# 399 "/usr/include/features.h" 3 4
# 1 "/usr/include/gnu/stubs.h" 1 3 4
# 10 "/usr/include/gnu/stubs.h" 3 4
# 1 "/usr/include/gnu/stubs-64.h" 1 3 4
# 11 "/usr/include/gnu/stubs.h" 2 3 4
# 400 "/usr/include/features.h" 2 3 4
# 26 "/usr/include/stdint.h" 2 3 4
# 1 "/usr/include/bits/wchar.h" 1 3 4
# 22 "/usr/include/bits/wchar.h" 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 23 "/usr/include/bits/wchar.h" 2 3 4
# 27 "/usr/include/stdint.h" 2 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 28 "/usr/include/stdint.h" 2 3 4








typedef signed char int8_t;
typedef short int int16_t;
typedef int int32_t;

typedef long int int64_t;







typedef unsigned char uint8_t;
typedef unsigned short int uint16_t;

typedef unsigned int uint32_t;



typedef unsigned long int uint64_t;
# 65 "/usr/include/stdint.h" 3 4
typedef signed char int_least8_t;
typedef short int int_least16_t;
typedef int int_least32_t;

typedef long int int_least64_t;






typedef unsigned char uint_least8_t;
typedef unsigned short int uint_least16_t;
typedef unsigned int uint_least32_t;

typedef unsigned long int uint_least64_t;
# 90 "/usr/include/stdint.h" 3 4
typedef signed char int_fast8_t;

typedef long int int_fast16_t;
typedef long int int_fast32_t;
typedef long int int_fast64_t;
# 103 "/usr/include/stdint.h" 3 4
typedef unsigned char uint_fast8_t;

typedef unsigned long int uint_fast16_t;
typedef unsigned long int uint_fast32_t;
typedef unsigned long int uint_fast64_t;
# 119 "/usr/include/stdint.h" 3 4
typedef long int intptr_t;


typedef unsigned long int uintptr_t;
# 134 "/usr/include/stdint.h" 3 4
typedef long int intmax_t;
typedef unsigned long int uintmax_t;
# 57 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdint.h" 2
# 4 "Sema/address-packed.c" 2

extern void f1(int *);
extern void f2(char *);

struct Ok {
  char c;
  int x;
};

struct __attribute__((packed)) Arguable {
  char c0;
  int x;
  char c1;
};

union __attribute__((packed)) UnionArguable {
  char c;
  int x;
};

typedef struct Arguable ArguableT;

struct Arguable *get_arguable(void);

void to_void(void *);
void to_intptr(intptr_t);

void g0(void) {
  {
    struct Ok ok;
    f1(&ok.x);
    f2(&ok.c);
  }
  {
    struct Arguable arguable;
    f2(&arguable.c0);
    f1(&arguable.x);
    f2(&arguable.c1);

    f1((int *)(void *)&arguable.x);
    to_void(&arguable.x);
    void *p = &arguable.x;
    to_void(p);
    to_intptr((intptr_t)p);
  }
  {
    union UnionArguable arguable;
    f2(&arguable.c);
    f1(&arguable.x);

    f1((int *)(void *)&arguable.x);
    to_void(&arguable.x);
    to_intptr((intptr_t)&arguable.x);
  }
  {
    ArguableT arguable;
    f2(&arguable.c0);
    f1(&arguable.x);
    f2(&arguable.c1);

    f1((int *)(void *)&arguable.x);
    to_void(&arguable.x);
    to_intptr((intptr_t)&arguable.x);
  }
  {
    struct Arguable *arguable = get_arguable();
    f2(&arguable->c0);
    f1(&arguable->x);
    f2(&arguable->c1);

    f1((int *)(void *)&arguable->x);
    to_void(&arguable->c1);
    to_intptr((intptr_t)&arguable->c1);
  }
  {
    ArguableT *arguable = get_arguable();
    f2(&(arguable->c0));
    f1(&(arguable->x));
    f2(&(arguable->c1));

    f1((int *)(void *)&(arguable->x));
    to_void(&(arguable->c1));
    to_intptr((intptr_t)&(arguable->c1));
  }
}

struct S1 {
  char c;
  int i __attribute__((packed));
};

int *g1(struct S1 *s1) {
  return &s1->i;
}

struct S2_i {
  int i;
};
struct __attribute__((packed)) S2 {
  char c;
  struct S2_i inner;
};

int *g2(struct S2 *s2) {
  return &s2->inner.i;
}

struct S2_a {
  char c;
  struct S2_i inner __attribute__((packed));
};

int *g2_a(struct S2_a *s2_a) {
  return &s2_a->inner.i;
}

struct __attribute__((packed)) S3 {
  char c;
  struct {
    int i;
  } inner;
};

int *g3(struct S3 *s3) {
  return &s3->inner.i;
}

struct S4 {
  char c;
  struct __attribute__((packed)) {
    int i;
  } inner;
};

int *g4(struct S4 *s4) {
  return &s4->inner.i;
}

struct S5 {
  char c;
  struct {
    char c1;
    int i __attribute__((packed));
  } inner;
};

int *g5(struct S5 *s5) {
  return &s5->inner.i;
}

struct __attribute__((packed, aligned(2))) AlignedTo2 {
  int x;
};

char *g6(struct AlignedTo2 *s) {
  return (char *)&s->x;
}

struct __attribute__((packed, aligned(2))) AlignedTo2Bis {
  int x;
};

struct AlignedTo2Bis* g7(struct AlignedTo2 *s)
{
    return (struct AlignedTo2Bis*)&s->x;
}

typedef struct {
  char c;
  int x;
} __attribute__((packed)) TypedefStructArguable;

typedef union {
  char c;
  int x;
} __attribute((packed)) TypedefUnionArguable;

typedef TypedefStructArguable TypedefStructArguableTheSecond;

int *typedef1(TypedefStructArguable *s) {
    return &s->x;
}

int *typedef2(TypedefStructArguableTheSecond *s) {
    return &s->x;
}

int *typedef3(TypedefUnionArguable *s) {
    return &s->x;
}

struct S6 {
  union {
    char c;
    int x;
  } __attribute__((packed));
};

int *anonymousInnerUnion(struct S6 *s) {
  return &s->x;
}

struct S6a {
    int a;
    int _;
    int c;
    char __;
    int d;
} __attribute__((packed, aligned(16))) s6;

void g8(void)
{
    f1(&s6.a);
    f1(&s6.c);
    f1(&s6.d);
}

struct __attribute__((packed, aligned(1))) MisalignedContainee { double d; };
struct __attribute__((aligned(8))) AlignedContainer { struct MisalignedContainee b; };

struct AlignedContainer *p;
double* g9(void) {
  return &p->b.d;
}

union OneUnion
{
    uint32_t a;
    uint32_t b:1;
};

struct __attribute__((packed)) S7 {
    uint8_t length;
    uint8_t stuff;
    uint8_t padding[2];
    union OneUnion one_union;
};

union AnotherUnion {
    long data;
    struct S7 s;
} *au;

union OneUnion* get_OneUnion(void)
{
    return &au->s.one_union;
}

struct __attribute__((packed)) S8 {
    uint8_t data1;
    uint8_t data2;
 uint16_t wider_data;
};






uint32_t get_wider_data(struct S8 *s)
{
    return ((uint16_t) ((((const uint8_t *)(&s->wider_data))[0] ) | (((const uint8_t *)(&s->wider_data))[1] << 8)));
}

struct S9 {
  uint32_t x;
  uint8_t y[2];
  uint16_t z;
} __attribute__((__packed__));

typedef struct S9 __attribute__((__aligned__(16))) aligned_S9;

void g10(void) {
  struct S9 x;
  struct S9 __attribute__((__aligned__(8))) y;
  aligned_S9 z;

  uint32_t *p32;
  p32 = &x.x;
  p32 = &y.x;
  p32 = &z.x;
}

typedef struct {
  uint32_t msgh_bits;
  uint32_t msgh_size;
  int32_t msgh_voucher_port;
  int32_t msgh_id;
} S10Header;

typedef struct {
  uint32_t t;
  uint64_t m;
  uint32_t p;
  union {
    struct {
      uint32_t a;
      double z;
    } __attribute__((aligned(8), packed)) a;
    struct {
      uint32_t b;
      double z;
      uint32_t a;
    } __attribute__((aligned(8), packed)) b;
  };
} __attribute__((aligned(8), packed)) S10Data;

typedef struct {
  S10Header hdr;
  uint32_t size;
  uint8_t count;
  S10Data data[] __attribute__((aligned(8)));
} __attribute__((aligned(8), packed)) S10;

void g11(S10Header *hdr);
void g12(S10 *s) {
  g11(&s->hdr);
}

struct S11 {
  uint32_t x;
} __attribute__((__packed__));

void g13(void) {
  struct S11 __attribute__((__aligned__(4))) a[4];
  uint32_t *p32;
  p32 = &a[0].x;
}

struct Invalid0 {
  void *x;
  struct fwd f;
} __attribute__((packed));

void *g14(struct Invalid0 *ivl) {
  return &(ivl->x);
}

void to_void_with_expr(void *ptr, int expr);

void g15(void) {
  struct Arguable arguable;
  to_void_with_expr(&arguable.x, 3);
  to_void_with_expr(&arguable.x, ({3;}));
}
