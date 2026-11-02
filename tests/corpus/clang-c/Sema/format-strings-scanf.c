// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c11: --c11
# 1 "Sema/format-strings-scanf.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 411 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/format-strings-scanf.c" 2





# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 1
# 47 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg_header_macro.h" 1
# 48 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2



# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg___gnuc_va_list.h" 1
# 12 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg___gnuc_va_list.h"
typedef __builtin_va_list __gnuc_va_list;
# 52 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg_va_list.h" 1
# 12 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg_va_list.h"
typedef __builtin_va_list va_list;
# 57 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg_va_arg.h" 1
# 62 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg___va_copy.h" 1
# 67 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg_va_copy.h" 1
# 72 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2
# 7 "Sema/format-strings-scanf.c" 2
typedef long unsigned int size_t;







typedef __typeof__(_Generic((long unsigned int)0, unsigned long long int : (long long int)0, unsigned long int : (long int)0, unsigned int : (int)0, unsigned short : (short)0, unsigned char : (signed char)0)) ssize_t;

typedef long int ptrdiff_t;
# 26 "Sema/format-strings-scanf.c"
typedef struct _FILE FILE;
typedef int wchar_t;

int fscanf(FILE * restrict, const char * restrict, ...) ;
int scanf(const char * restrict, ...) ;
int sscanf(const char * restrict, const char * restrict, ...) ;
int my_scanf(const char * restrict, ...) __attribute__((__format__(__scanf__, 1, 2)));
int my_gnu_scanf(const char * restrict, ...) __attribute__((__format__(gnu_scanf, 1, 2)));

int vscanf(const char * restrict, va_list);
int vfscanf(FILE * restrict, const char * restrict, va_list);
int vsscanf(const char * restrict, const char * restrict, va_list);

void test(const char *s, int *i) {
  scanf(s, i);

  scanf("%0d", i);
  scanf("%00d", i);
  scanf("%d%[asdfasdfd", i, s);
  scanf("%B", i);

  unsigned short s_x;
  scanf ("%" "hu" "\n", &s_x);
  scanf("%hb", &s_x);
  scanf("%y", i);
  scanf("%%");
  scanf("%%%1$d", i);
  scanf("%1$d%%", i);
  scanf("%d", i, i);
  scanf("%*d", i);
  scanf("%*d", i);
  scanf("%*d%1$d", i);

  scanf("%s", (char*)0);
  scanf("%s", (volatile char*)0);
  scanf("%s", (signed char*)0);
  scanf("%s", (unsigned char*)0);
  scanf("%hhu", (signed char*)0);
  scanf("%hhb", (signed char*)0);
}

void bad_length_modifiers(char *s, void *p, wchar_t *ws, long double *ld) {
  scanf("%hhs", "foo");
  scanf("%1$zp", &p);
  scanf("%ls", ws);
  scanf("%#.2Lf", ld);
}

void missing_argument_with_length_modifier() {
  char buf[30];
  scanf("%s:%900s", buf);
}



void pr9751(void) {
  int *i;
  char str[100];
  const char kFormat1[] = "%00d";
  scanf(kFormat1, i);
  scanf("%00d", i);
  const char kFormat2[] = "%[";
  scanf(kFormat2, str);
  scanf("%[", str);
  const char kFormat3[] = "%hu";
  scanf(kFormat3, &i);
  const char kFormat4[] = "%lp";
  scanf(kFormat4, &i);
}

void test_variants(int *i, const char *s, ...) {
  FILE *f = 0;
  char buf[100];

  fscanf(f, "%ld", i);
  sscanf(buf, "%ld", i);
  my_scanf("%ld", i);
  my_gnu_scanf("%ld", i);

  va_list ap;
  __builtin_va_start(ap, s);

  vscanf("%[abc", ap);
  vfscanf(f, "%[abc", ap);
  vsscanf(buf, "%[abc", ap);
}

void test_scanlist(int *ip, char *sp, wchar_t *ls) {
  scanf("%[abc]", ip);
  scanf("%h[abc]", sp);
  scanf("%l[xyx]", ls);
  scanf("%ll[xyx]", ls);


  scanf("%[]% ]", sp);
  scanf("%[^]% ]", sp);
  scanf("%[a^]% ]", sp);
}

void test_alloc_extension(char **sp, wchar_t **lsp, float *fp) {



  scanf("%as", sp);
  scanf("%aS", lsp);
  scanf("%a[bcd]", sp);



  scanf("%ms", sp);
  scanf("%mS", lsp);
  scanf("%mc", sp);
  scanf("%mC", lsp);
  scanf("%m[abc]", sp);
  scanf("%md", sp);


  scanf("%ms", fp);
  scanf("%mS", fp);
  scanf("%mc", fp);
  scanf("%mC", fp);
  scanf("%m[abc]", fp);
}

void test_quad(int *x, long long *llx) {
  scanf("%qd", x);
  scanf("%qd", llx);
}

void test_writeback(int *x) {
  scanf("%n", (void*)0);
  scanf("%n %c", x, x);

  scanf("%hhn", (signed char*)0);
  scanf("%hhn", (char*)0);
  scanf("%hhn", (unsigned char*)0);
  scanf("%hhn", (int*)0);

  scanf("%hn", (short*)0);
  scanf("%hn", (unsigned short*)0);
  scanf("%hn", (int*)0);

  scanf("%n", (int*)0);
  scanf("%n", (unsigned int*)0);
  scanf("%n", (char*)0);

  scanf("%ln", (long*)0);
  scanf("%ln", (unsigned long*)0);
  scanf("%ln", (int*)0);

  scanf("%lln", (long long*)0);
  scanf("%lln", (unsigned long long*)0);
  scanf("%lln", (int*)0);

  scanf("%qn", (long long*)0);
  scanf("%qn", (unsigned long long*)0);
  scanf("%qn", (int*)0);

}

void test_qualifiers(const int *cip, volatile int* vip,
                     const char *ccp, volatile char* vcp,
                     const volatile int *cvip) {
  scanf("%d", cip);
  scanf("%n", cip);
  scanf("%s", ccp);
  scanf("%d", cvip);

  scanf("%d", vip);
  scanf("%n", vip);
  scanf("%c", vcp);

  typedef int* ip_t;
  typedef const int* cip_t;
  scanf("%d", (ip_t)0);
  scanf("%d", (cip_t)0);
}

void test_size_types(void) {
  size_t s = 0;
  scanf("%zu", &s);
  scanf("%zb", &s);

  double d1 = 0.;
  scanf("%zu", &d1);

  ssize_t ss = 0;
  scanf("%zd", &s);

  double d2 = 0.;
  scanf("%zd", &d2);

  ssize_t sn = 0;
  scanf("%zn", &sn);

  double d3 = 0.;
  scanf("%zn", &d3);
}

void test_ptrdiff_t_types(void) {
  __typeof__(_Generic((long int)0, long long int : (unsigned long long int)0, long int : (unsigned long int)0, int : (unsigned int)0, short : (unsigned short)0, signed char : (unsigned char)0)) p1 = 0;
  scanf("%tu", &p1);
  scanf("%tb", &p1);

  double d1 = 0.;
  scanf("%tu", &d1);

  ptrdiff_t p2 = 0;
  scanf("%td", &p2);

  double d2 = 0.;
  scanf("%td", &d2);

  ptrdiff_t p3 = 0;
  scanf("%tn", &p3);

  double d3 = 0.;
  scanf("%tn", &d3);
}

void check_conditional_literal(char *s, int *i) {
  scanf(0 ? "%s" : "%d", i);
  scanf(1 ? "%s" : "%d", i);
  scanf(0 ? "%d %d" : "%d", i);
  scanf(1 ? "%d %d" : "%d", i);
  scanf(0 ? "%d %d" : "%d", i, s);
  scanf(1 ? "%d %s" : "%d", i, s);
  scanf(i ? "%d %s" : "%d", i, s);
  scanf(i ? "%d" : "%d", i, s);
  scanf(i ? "%s" : "%d", s);
}

void test_promotion(void) {



  int i;
  signed char sc;
  unsigned char uc;
  short ss;
  unsigned short us;


  scanf("%hhd", &i);
  scanf("%hd", &i);
  scanf("%d", &i);

  scanf("%hhd", &sc);
  scanf("%hhd", &uc);
  scanf("%hd", &sc);
  scanf("%hd", &uc);
  scanf("%d", &sc);
  scanf("%d", &uc);

  scanf("%hhd", &ss);
  scanf("%hhd", &us);
  scanf("%hd", &ss);
  scanf("%hd", &us);
  scanf("%d", &ss);
  scanf("%d", &us);


  scanf("%ld", &i);
  scanf("%lld", &i);
  scanf("%ld", &sc);
  scanf("%lld", &sc);
  scanf("%ld", &uc);
  scanf("%lld", &uc);
  scanf("%llx", &i);


  scanf("%hf",
  &sc);


  scanf("%s", i);


  char c;
  void *vp;
  scanf("%hhd", &c);
  scanf("%hhd", vp);
}
