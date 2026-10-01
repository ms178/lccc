// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
# 1 "Sema/attr-format.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/attr-format.c" 2


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
# 4 "Sema/attr-format.c" 2

void a(const char *a, ...) __attribute__((format(printf, 1, 2)));
void b(const char *a, ...) __attribute__((format(printf, 1, 1)));
void c(const char *a, ...) __attribute__((format(printf, 0, 2)));
void d(const char *a, int c) __attribute__((format(printf, 1, 2)));
void e(char *str, int c, ...) __attribute__((format(printf, 2, 3)));
void f(int a, const char *b, ...) __attribute__((format(printf, 2, 1)));
void g(int a, const char *b, ...) __attribute__((format(printf, 2, 2)));
void h(int a, const char *b, ...) __attribute__((format(printf, 2, 3)));
void i(const char *a, int b, ...) __attribute__((format(printf, 1, 2)));

typedef const char *xpto;
void j(xpto c, va_list list) __attribute__((format(printf, 1, 0)));
void k(xpto c) __attribute__((format(printf, 1, 0)));

void y(char *str) __attribute__((format(strftime, 1, 0)));
void z(char *str, int c, ...) __attribute__((format(strftime, 1, 2)));

int (*f_ptr)(char*,...) __attribute__((format(printf, 1,2)));
int (*f2_ptr)(double,...) __attribute__((format(printf, 1, 2)));

struct _mystruct {
  int (*printf)(const char *format, ...) __attribute__((__format__(printf, 1, 2)));
  int (*printf2)(double format, ...) __attribute__((__format__(printf, 1, 2)));
};

typedef int (*f3_ptr)(char*,...) __attribute__((format(printf,1,0)));

int rdar6623513(void *, const char*, const char*, ...)
  __attribute__ ((format (printf, 3, 0)));

int rdar6623513_aux(int len, const char* s) {
  rdar6623513(0, "hello", "%.*s", len, s);
}


void a2(const char *a, ...) __attribute__((format(printf0, 1, 2)));
void b2(const char *a, ...) __attribute__((format(printf0, 1, 1)));
void c2(const char *a, ...) __attribute__((format(printf0, 0, 2)));
void d2(const char *a, int c) __attribute__((format(printf0, 1, 2)));
void e2(char *str, int c, ...) __attribute__((format(printf0, 2, 3)));



void null(int i, const char *a, ...) __attribute__((__format__(__printf0__, 2, 0)));
void null(int i, const char *a, ...) {
  if (a)
    (void)0 ;
}

void callnull(void){
  null(0, 0);
  null(0, (char*)0);
  null(0, (void*)0);
  null(0, (int*)0);
}


void a3(const char *a, ...) __attribute__((format(freebsd_kprintf, 1, 2)));
void b3(const char *a, ...) __attribute__((format(freebsd_kprintf, 1, 1)));
void c3(const char *a, ...) __attribute__((format(freebsd_kprintf, 0, 2)));
void d3(const char *a, int c) __attribute__((format(freebsd_kprintf, 1, 2)));
void e3(char *str, int c, ...) __attribute__((format(freebsd_kprintf, 2, 3)));


int xx_vprintf(const char *, va_list);

const char *foo(const char *format) __attribute__((format_arg(1)));

void __attribute__((format(printf, 1, 0)))
foo2(const char *fmt, va_list va) {
  xx_vprintf(foo(fmt), va);
}


extern void gcc_format(const char *, ...)
    __attribute__((__format__(__gcc_diag__, 1, 2)));
extern void gcc_cformat(const char *, ...)
    __attribute__((__format__(__gcc_cdiag__, 1, 2)));
extern void gcc_cxxformat(const char *, ...)
    __attribute__((__format__(__gcc_cxxdiag__, 1, 2)));
extern void gcc_tformat(const char *, ...)
    __attribute__((__format__(__gcc_tdiag__, 1, 2)));

const char *foo3(const char *format) __attribute__((format_arg("foo")));

void call_nonvariadic(void) {
  d3("%i", 123);
  d3("%d", 123);
  d3("%s", 123);
}

__attribute__((format(printf, 1, 2)))
void forward_fixed(const char *fmt, _Bool b, char i, short j, int k, float l, double m) {
  forward_fixed(fmt, b, i, j, k, l, m);
  a(fmt, b, i, j, k, l, m);
}



void a2(const char *a, ...) __attribute__((format(syslog, 1, 2)));
void b2(const char *a, ...) __attribute__((format(syslog, 1, 1)));
void c2(const char *a, ...) __attribute__((format(syslog, 0, 2)));
void d2(const char *a, int c) __attribute__((format(syslog, 1, 2)));
void e2(char *str, int c, ...) __attribute__((format(syslog, 2, 3)));



void a2(const char *a, ...) __attribute__((format(gnu_printf, 1, 2)));
void b2(const char *a, ...) __attribute__((format(gnu_printf, 1, 1)));
void c2(const char *a, ...) __attribute__((format(gnu_printf, 0, 2)));
void d2(const char *a, int c) __attribute__((format(gnu_printf, 1, 2)));
void e2(char *str, int c, ...) __attribute__((format(gnu_printf, 2, 3)));
