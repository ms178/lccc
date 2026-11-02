// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
# 1 "Sema/format-string-matches.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/format-string-matches.c" 2



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
# 5 "Sema/format-string-matches.c" 2

__attribute__((format_matches(printf, -1, "%s")))
int test_out_of_bounds(void);

__attribute__((format_matches(printf, 0, "%s")))
int test_out_of_bounds(void);

__attribute__((format_matches(printf, 1, "%s")))
int test_out_of_bounds(void);

__attribute__((format_matches(printf, 1, "%s")))
int test_out_of_bounds_int(int x);




__attribute__((format(printf, 1, 2)))
int printf(const char *fmt, ...);

__attribute__((format(printf, 1, 0)))
int vprintf(const char *fmt, va_list);

__attribute__((format_matches(printf, 1, "%s %1.5s")))
void format_str_str0(const char *fmt) {
    printf(fmt, "hello", "world");
}

__attribute__((format_matches(printf, 1, "%s" "%1.5s")))
void format_str_str1(const char *fmt) {
    printf(fmt, "hello", "world");
}

__attribute__((format_matches(printf, 1, ("%s" "%1.5s") + 5)))
void format_str_str2(const char *fmt);

__attribute__((format_matches(printf, 1, "%s %g")))
void format_str_double_warn(const char *fmt) {
    printf(fmt, "hello", "world");
}

__attribute__((format_matches(printf, 1, "%s %g")))
void vformat(const char *fmt, ...) {
    va_list ap;
    __builtin_va_start(ap, fmt);
    vprintf(fmt, ap);
    __builtin_va_end(ap);
}




void cvt_percent(const char *c) __attribute__((format_matches(printf, 1, "%%")));
void cvt_at(const char *c) __attribute__((format_matches(NSString, 1, "%@")));


void cvt_c(const char *c) __attribute__((format_matches(printf, 1, "%c")));
void cvt_u(const char *c) __attribute__((format_matches(printf, 1, "%u")));
void cvt_hhi(const char *c) __attribute__((format_matches(printf, 1, "%hhi")));
void cvt_i(const char *c) __attribute__((format_matches(printf, 1, "%i")));
void cvt_p(const char *c) __attribute__((format_matches(printf, 1, "%p")));
void cvt_s(const char *c) __attribute__((format_matches(printf, 1, "%s")));
void cvt_n(const char *c) __attribute__((format_matches(printf, 1, "%n")));

void test_compatibility(void) {
    cvt_c("%i");
    const char *const fmt_i = "%i";
    cvt_c(fmt_i);

    cvt_i("%c");
    cvt_c("%u");
    cvt_u("%c");

    const char *const fmt_c = "%c";
    cvt_u(fmt_c);

    cvt_i("%hi");
    cvt_i("%hhi");
    cvt_i("%lli");
    cvt_i("%p");
    cvt_hhi("%hhi");
    cvt_hhi("%hi");
    cvt_hhi("%i");
    cvt_hhi("%li");
    cvt_n("%s");
    cvt_s("%hhn");

    cvt_p("%@");
    cvt_at("%p");

    cvt_percent("hello");
    cvt_percent("%c");

    const char *const too_many = "%c";
    cvt_percent(too_many);
}

void test_too_few_args(void) {
    cvt_at("a");
    cvt_at("%@ %@");

    const char *const too_few = "a";
    cvt_at(too_few);
}

void cvt_several(const char *c) __attribute__((format_matches(printf, 1, "%f %i %s")));

void test_moving_args_around(void) {
    cvt_several("%1g %-d %1.5s");

    cvt_several("%3$s %1$g %2$i");

    cvt_several("%f %*s");
}

void cvt_freebsd_D(const char *c) __attribute__((format_matches(freebsd_kprintf, 1, "%D")));

void test_freebsd_specifiers(void) {
    cvt_freebsd_D("%D");
    cvt_freebsd_D("%b");
    cvt_freebsd_D("%s %i");
}


void takes_printf_string(const char *fmt) __attribute__((format_matches(printf, 1, "%s")));
__attribute__((format_matches(freebsd_kprintf, 1, "%s")))
void takes_freebsd_kprintf_string(const char *fmt) {
    takes_printf_string(fmt);

    const char *const fmt2 = fmt;
    takes_printf_string(fmt2);
}

__attribute__((format_matches(printf, 1, "%s")))
__attribute__((format_matches(os_log, 2, "%i")))
void test_recv_multiple_format_strings(const char *fmt1, const char *fmt2);

__attribute__((format_matches(printf, 1, "%s")))
__attribute__((format_matches(os_log, 2, "%i")))
void test_multiple_format_strings(const char *fmt1, const char *fmt2) {
    test_recv_multiple_format_strings("%s", "%i");
    test_recv_multiple_format_strings("%s", "%s");
    test_recv_multiple_format_strings("%i", "%i");

    test_recv_multiple_format_strings(fmt1, fmt2);
    test_recv_multiple_format_strings("%.5s", fmt2);
    test_recv_multiple_format_strings(fmt1, "%04d");

    test_recv_multiple_format_strings("%s", fmt1);
    test_recv_multiple_format_strings(fmt2, "%d");

    test_recv_multiple_format_strings(fmt2, fmt1);


}

__attribute__((format_matches(os_log, 1, "%{public}s")))
void call_oslog_public(const char *fmt);

__attribute__((format_matches(os_log, 1, "%{sensitive}s")))
void call_oslog_sensitive(const char *fmt);

__attribute__((format_matches(os_log, 1, "%{private}s")))
void call_oslog_private(const char *fmt);

void test_oslog(void) {
    call_oslog_public("%{public}s");
    call_oslog_public("%{private}s");
    call_oslog_public("%{sensitive}s");

    call_oslog_sensitive("%{public}s");
    call_oslog_sensitive("%{private}s");
    call_oslog_sensitive("%{sensitive}s");

    call_oslog_private("%{public}s");
    call_oslog_private("%{private}s");
    call_oslog_private("%{sensitive}s");



    call_oslog_public("%{private}i");
}


void accept_value(const char *f) __attribute__((format_matches(freebsd_kprintf, 1, "%s%i%i")));


void accept_indirect_field_width(const char *f) __attribute__((format_matches(freebsd_kprintf, 1, "%s%*i")));


void accept_indirect_field_precision(const char *f) __attribute__((format_matches(freebsd_kprintf, 1, "%s%.*i")));


void accept_aux_value(const char *f) __attribute__((format_matches(freebsd_kprintf, 1, "%D%i")));



void accept_value(const char *f) {
    accept_indirect_field_width(f);
    accept_indirect_field_precision(f);
    accept_aux_value(f);
}

void accept_indirect_field_width(const char *f) {
    accept_value(f);
    accept_indirect_field_precision(f);
    accept_aux_value(f);
}

void accept_indirect_field_precision(const char *f) {
    accept_value(f);
    accept_indirect_field_width(f);
    accept_aux_value(f);
}

void accept_aux_value(const char *f) {
    accept_value(f);
    accept_indirect_field_width(f);
    accept_indirect_field_precision(f);
}


__attribute__((format_matches(printf, 1, "%i")))
__attribute__((format_matches(printf, 1, "%d")))
void test_merge_self(const char *f);

__attribute__((format_matches(printf, 1, "%i")))
__attribute__((format_matches(printf, 1, "%s")))
void test_merge_self_warn(const char *f);

__attribute__((format_matches(printf, 1, "%i")))
void test_merge_redecl(const char *f);

__attribute__((format_matches(printf, 1, "%d")))
void test_merge_redecl(const char *f);




__attribute__((format_matches(printf, 1, "%i")))
void test_merge_redecl_warn(const char *f);

__attribute__((format_matches(printf, 1, "%s")))
void test_merge_redecl_warn(const char *f);




__attribute__((format_matches(printf, 1, "%1$s %1$d")))


void test_positional_incompatible(const char *f);

void call_positional_incompatible(void) {

    test_positional_incompatible("%d %d %d %d %d");
}

void test_many_i(void) {
    cvt_i("%1$d %1$i");
    cvt_i("%1$d %1$s");
}

__attribute__((format_matches(printf, 1, "%*d %*d")))
void accept_modifiers(const char *f);

void test_modifiers(void) {
    accept_modifiers("%2$*1$d %4$*3$d");
    accept_modifiers("%2$*3$d %4$*3$d");
}
