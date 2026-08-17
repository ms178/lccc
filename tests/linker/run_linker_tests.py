#!/usr/bin/env python3
"""LCCC linker regression suite with mold/wild as differential oracles.

Strategy
========
The generated code is held constant: every fixture is compiled to .o with the
SAME system compiler (gcc).  Only the *linker* varies:

    lccc builtin linker   (system under test)
    mold                  (oracle 1, if installed)
    wild                  (oracle 2, if installed)
    GNU ld / bfd          (oracle 3, always present)

Each produced executable is run; stdout + exit code must agree across all
linkers that succeeded.  A test therefore fails when:
  * lccc's linker errors out while the oracles link fine  -> missing feature
  * lccc's binary crashes or produces different output    -> miscompiled link
  * lccc's linker accepts something all oracles reject    -> missing diagnostic
    (reported as a warning, not a failure, unless marked expect_fail)

Usage:
    tests/linker/run_linker_tests.py [--lccc PATH] [--filter SUBSTR] [-v]
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
import textwrap

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
DEFAULT_LCCC = os.environ.get(
    "LCCC_BIN", os.path.join(REPO, "target", "release", "lccc-x86"))

CC = os.environ.get("LINKTEST_CC", "gcc")

class Case:
    def __init__(self, name, sources, link_inputs=None, ldflags=None,
                 run_args=None, expect_fail=False, expect_stdout=None,
                 expect_exit=0, compile_flags=None, setup=None,
                 oracle_only_flags=None, lccc_only_flags=None,
                 skip_oracles=False, run_env=None, tags=()):
        self.name = name
        self.sources = sources              # dict fname -> contents (.c or .s)
        self.link_inputs = link_inputs      # ordered link inputs; default: all objects
        self.ldflags = ldflags or []
        self.run_args = run_args or []
        self.expect_fail = expect_fail      # link must FAIL (diagnostic test)
        self.expect_stdout = expect_stdout  # None -> compare against oracles
        self.expect_exit = expect_exit
        self.compile_flags = compile_flags or ["-O1"]
        self.setup = setup                  # callable(tmpdir) for archives etc.
        self.oracle_only_flags = oracle_only_flags or []
        self.lccc_only_flags = lccc_only_flags or []
        self.skip_oracles = skip_oracles
        self.run_env = run_env or {}
        self.tags = tags

CASES = []
def case(*a, **kw):
    CASES.append(Case(*a, **kw))

def sh(cmd, cwd=None, timeout=60, env=None):
    e = dict(os.environ)
    if env:
        e.update(env)
    return subprocess.run(cmd, cwd=cwd, timeout=timeout, env=e,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE)

# ============================================================================
# 1. SYMBOL RESOLUTION
# ============================================================================

case("weak_strong_override",
    {"a.c": """
        #include <stdio.h>
        __attribute__((weak)) int val(void){ return 1; }
        int main(void){ printf("%d\\n", val()); return 0; }
     """,
     "b.c": "int val(void){ return 2; }"},
    tags=("symbols",))

case("weak_no_override",
    {"a.c": """
        #include <stdio.h>
        __attribute__((weak)) int val(void){ return 1; }
        int main(void){ printf("%d\\n", val()); return 0; }
     """},
    tags=("symbols",))

case("weak_undef_null_check",
    {"a.c": """
        #include <stdio.h>
        extern int optional_fn(void) __attribute__((weak));
        int main(void){
            if (&optional_fn) printf("present %d\\n", optional_fn());
            else printf("absent\\n");
            return 0;
        }
     """},
    tags=("symbols",))

case("weak_undef_data",
    {"a.c": """
        #include <stdio.h>
        extern int optional_var __attribute__((weak));
        int main(void){
            printf("%s\\n", &optional_var ? "present" : "absent");
            return 0;
        }
     """},
    tags=("symbols",))

case("common_symbols_merge",
    {"a.c": """
        #include <stdio.h>
        int shared_common;           /* tentative definition */
        int main(void){ extern void bump(void); bump(); bump();
                        printf("%d\\n", shared_common); return 0; }
     """,
     "b.c": "int shared_common; void bump(void){ shared_common++; }"},
    compile_flags=["-O1", "-fcommon"],
    tags=("symbols",))

case("common_vs_strong_def",
    {"a.c": """
        #include <stdio.h>
        int x;                        /* common */
        int main(void){ printf("%d\\n", x); return 0; }
     """,
     "b.c": "int x = 77;"},          # strong definition must win
    compile_flags=["-O1", "-fcommon"],
    tags=("symbols",))

case("common_alignment",
    {"a.c": """
        #include <stdio.h>
        #include <stdint.h>
        __attribute__((aligned(64))) char big_buf[100];
        int main(void){ printf("%d\\n", (int)((uintptr_t)big_buf & 63)); return 0; }
     """,
     "b.c": "__attribute__((aligned(64))) char big_buf[100];"},
    compile_flags=["-O1", "-fcommon"],
    tags=("symbols", "layout"))

case("strong_duplicate_rejected",
    {"a.c": "int f(void){return 1;} int main(void){return f();}",
     "b.c": "int f(void){return 2;}"},
    expect_fail=True,
    tags=("symbols", "diagnostics"))

case("undefined_symbol_rejected",
    {"a.c": "extern int nowhere(void); int main(void){ return nowhere(); }"},
    expect_fail=True,
    tags=("symbols", "diagnostics"))

case("alias_attribute",
    {"a.c": """
        #include <stdio.h>
        int real_impl(int x){ return x + 5; }
        int aliased(int) __attribute__((alias("real_impl")));
        int main(void){ printf("%d\\n", aliased(10)); return 0; }
     """},
    tags=("symbols",))

case("hidden_visibility",
    {"a.c": """
        #include <stdio.h>
        extern int hv(void);
        int main(void){ printf("%d\\n", hv()); return 0; }
     """,
     "b.c": '__attribute__((visibility("hidden"))) int hv(void){ return 9; }'},
    tags=("symbols", "visibility"))

case("protected_visibility",
    {"a.c": """
        #include <stdio.h>
        extern int pv(void);
        int main(void){ printf("%d\\n", pv()); return 0; }
     """,
     "b.c": '__attribute__((visibility("protected"))) int pv(void){ return 8; }'},
    tags=("symbols", "visibility"))

case("local_symbol_shadowing",
    {"a.c": """
        #include <stdio.h>
        static int helper(void){ return 1; }
        int a_val(void){ return helper(); }
        int main(void){ extern int b_val(void);
                        printf("%d %d\\n", a_val(), b_val()); return 0; }
     """,
     "b.c": "static int helper(void){ return 2; } int b_val(void){ return helper(); }"},
    tags=("symbols",))

# ============================================================================
# 2. ARCHIVES
# ============================================================================

def _make_archive(td):
    r = sh(["ar", "rcs", "libdep.a", "dep1.o", "dep2.o"], cwd=td)
    assert r.returncode == 0, r.stderr.decode()

case("archive_selective_extract",
    {"main.c": """
        #include <stdio.h>
        extern int used(void);
        int main(void){ printf("%d\\n", used()); return 0; }
     """,
     "dep1.c": "int used(void){ return 11; }",
     "dep2.c": 'extern int nowhere_at_all(void);\n'
               'int unused_member(void){ return nowhere_at_all(); }'},
    link_inputs=["main.o", "libdep.a"],
    setup=lambda td: (sh([CC, "-c", "dep1.c", "dep2.c", "-o", "/dev/null"], cwd=td),
                      sh([CC, "-c", "dep1.c"], cwd=td),
                      sh([CC, "-c", "dep2.c"], cwd=td),
                      _make_archive(td)),
    tags=("archive",))

def _make_circ(td):
    for f in ("c1.c", "c2.c"):
        r = sh([CC, "-c", f], cwd=td); assert r.returncode == 0
    r = sh(["ar", "rcs", "lib1.a", "c1.o"], cwd=td); assert r.returncode == 0
    r = sh(["ar", "rcs", "lib2.a", "c2.o"], cwd=td); assert r.returncode == 0

case("archive_circular_groups",
    {"main.c": """
        #include <stdio.h>
        extern int f1(int);
        int main(void){ printf("%d\\n", f1(3)); return 0; }
     """,
     "c1.c": "extern int f2(int); int f1(int x){ return x>0 ? f2(x-1)+1 : 0; }",
     "c2.c": "extern int f1(int); int f2(int x){ return x>0 ? f1(x-1)+10 : 0; }"},
    link_inputs=["main.o", "-Wl,--start-group", "lib1.a", "lib2.a", "-Wl,--end-group"],
    setup=_make_circ,
    tags=("archive",))

def _make_wa(td):
    r = sh([CC, "-c", "wa.c"], cwd=td); assert r.returncode == 0
    r = sh(["ar", "rcs", "libwa.a", "wa.o"], cwd=td); assert r.returncode == 0

case("whole_archive_exec",
    {"main.c": """
        #include <stdio.h>
        /* nothing references the archive member, but --whole-archive must pull
           in its constructor */
        extern int flag_from_ctor;
        int main(void){ printf("%d\\n", flag_from_ctor); return 0; }
     """,
     "wa.c": """
        int flag_from_ctor;
        __attribute__((constructor)) static void init(void){ flag_from_ctor = 42; }
     """},
    link_inputs=["main.o", "-Wl,--whole-archive", "libwa.a", "-Wl,--no-whole-archive"],
    setup=_make_wa,
    tags=("archive",))

def _make_thin(td):
    r = sh([CC, "-c", "t1.c"], cwd=td); assert r.returncode == 0
    r = sh(["ar", "rcsT", "libthin.a", "t1.o"], cwd=td); assert r.returncode == 0

case("thin_archive",
    {"main.c": """
        #include <stdio.h>
        extern int tfn(void);
        int main(void){ printf("%d\\n", tfn()); return 0; }
     """,
     "t1.c": "int tfn(void){ return 21; }"},
    link_inputs=["main.o", "libthin.a"],
    setup=_make_thin,
    tags=("archive",))

case("archive_strong_over_weak_obj",
    # A weak def in an object + strong def in archive: GNU ld keeps the weak
    # object def (archive member not pulled since symbol already defined).
    {"main.c": """
        #include <stdio.h>
        __attribute__((weak)) int wsv(void){ return 1; }
        int main(void){ printf("%d\\n", wsv()); return 0; }
     """,
     "s.c": "int wsv(void){ return 2; }"},
    link_inputs=["main.o", "libs.a"],
    setup=lambda td: (sh([CC, "-c", "s.c"], cwd=td),
                      sh(["ar", "rcs", "libs.a", "s.o"], cwd=td)),
    tags=("archive", "symbols"))

# ============================================================================
# 3. SECTIONS / LAYOUT
# ============================================================================

case("init_fini_arrays",
    {"a.c": """
        #include <stdio.h>
        __attribute__((constructor)) static void c1(void){ printf("ctor\\n"); }
        __attribute__((destructor))  static void d1(void){ printf("dtor\\n"); }
        int main(void){ printf("main\\n"); return 0; }
     """},
    tags=("sections",))

case("ctor_priority_order",
    {"a.c": """
        #include <stdio.h>
        __attribute__((constructor(200))) static void c2(void){ printf("2"); }
        __attribute__((constructor(101))) static void c1(void){ printf("1"); }
        __attribute__((constructor(300))) static void c3(void){ printf("3"); }
        int main(void){ printf("\\n"); return 0; }
     """},
    tags=("sections",))

case("start_stop_section_symbols",
    {"a.c": """
        #include <stdio.h>
        extern int __start_mydata[];
        extern int __stop_mydata[];
        __attribute__((used, section("mydata"))) static int e1 = 10;
        __attribute__((used, section("mydata"))) static int e2 = 20;
        __attribute__((used, section("mydata"))) static int e3 = 12;
        int main(void){
            int s = 0;
            for (int *p = __start_mydata; p < __stop_mydata; p++) s += *p;
            printf("%d\\n", s);
            return 0;
        }
     """},
    tags=("sections",))

case("gc_sections_basic",
    {"a.c": """
        #include <stdio.h>
        int unused_fn(void){ return 123; }
        int main(void){ printf("ok\\n"); return 0; }
     """},
    ldflags=["-Wl,--gc-sections"],
    compile_flags=["-O1", "-ffunction-sections", "-fdata-sections"],
    tags=("sections", "gc"))

case("gc_sections_keeps_used",
    {"a.c": """
        #include <stdio.h>
        extern int kept(void);
        int main(void){ printf("%d\\n", kept()); return 0; }
     """,
     "b.c": "int kept(void){ return 5; } int dropped(void){ return 6; }"},
    ldflags=["-Wl,--gc-sections"],
    compile_flags=["-O1", "-ffunction-sections", "-fdata-sections"],
    tags=("sections", "gc"))

case("gc_sections_keep_start_stop",
    {"a.c": """
        #include <stdio.h>
        extern int __start_regs[], __stop_regs[];
        __attribute__((used, retain, section("regs"))) static int r1 = 7;
        int main(void){
            printf("%d\\n", (int)(__stop_regs - __start_regs));
            return 0;
        }
     """},
    ldflags=["-Wl,--gc-sections"],
    compile_flags=["-O1", "-ffunction-sections", "-fdata-sections"],
    tags=("sections", "gc"))

case("bss_zeroed",
    {"a.c": """
        #include <stdio.h>
        static char big[1 << 20];
        int main(void){
            unsigned s = 0;
            for (unsigned i = 0; i < sizeof big; i++) s += big[i];
            printf("%u\\n", s);
            return 0;
        }
     """},
    tags=("sections", "layout"))

case("large_alignment_data",
    {"a.c": """
        #include <stdio.h>
        #include <stdint.h>
        __attribute__((aligned(4096))) static int page_aligned = 3;
        __attribute__((aligned(256)))  static int a256 = 4;
        int main(void){
            printf("%d %d %d %d\\n", page_aligned, a256,
                   (int)((uintptr_t)&page_aligned & 4095),
                   (int)((uintptr_t)&a256 & 255));
            return 0;
        }
     """},
    tags=("layout",))

case("rodata_merge_strings",
    {"a.c": """
        #include <stdio.h>
        #include <string.h>
        const char *s1 = "shared-string";
        int main(void){ extern const char *s2;
            printf("%d %s\\n", (int)strlen(s1) + (int)strlen(s2), s2);
            return 0; }
     """,
     "b.c": 'const char *s2 = "shared-string";'},
    tags=("sections",))

case("strmerge_dedup_identity",
    # SHF_MERGE dedup: identical strings across objects must compare
    # pointer-EQUAL after dedup in GNU ld/mold; and must never corrupt
    # interior-pointer arithmetic. lccc's pool remap keeps intra-entry deltas.
    {"a.c": """
        #include <stdio.h>
        #include <string.h>
        const char *pa = "dedup-me-please";
        const char *tail_a = "dedup-me-please" + 6;   /* interior pointer */
        int main(void){
            extern const char *pb, *tail_b;
            printf("%d %d %s %s\\n",
                   strcmp(pa, pb) == 0,
                   strcmp(tail_a, tail_b) == 0,
                   tail_a, pb);
            return 0;
        }
     """,
     "b.c": 'const char *pb = "dedup-me-please";\n'
            'const char *tail_b = "dedup-me-please" + 6;'},
    compile_flags=["-O2"],
    tags=("sections", "strmerge"))

case("strmerge_fp_constants",
    # .rodata.cst8/.cst16 dedup: FP constants must survive pooling.
    {"a.c": """
        #include <stdio.h>
        double da(void){ return 3.14159265358979; }
        float  fa(void){ return 2.71828f; }
        int main(void){
            extern double db(void); extern float fb(void);
            printf("%d %d\\n", da() == db(), fa() == fb());
            return 0;
        }
     """,
     "b.c": "double db(void){ return 3.14159265358979; }\n"
            "float  fb(void){ return 2.71828f; }"},
    compile_flags=["-O2"],
    tags=("sections", "strmerge"))

case("strmerge_wide_and_narrow",
    # Mixed .rodata.str1.1 / .rodata.str1.8 (from -O2 aligned string ops):
    # alignment classes must not be cross-polluted.
    {"a.c": """
        #include <stdio.h>
        #include <string.h>
        int main(void){
            extern const char *get_msg(void);
            char buf[64];
            strcpy(buf, "format %s %d here");
            printf(buf, get_msg(), (int)strlen(get_msg()));
            printf("\\n");
            return 0;
        }
     """,
     "b.c": 'const char *get_msg(void){ return "format %s %d here"; }'},
    compile_flags=["-O2"],
    tags=("sections", "strmerge"))

case("tentative_array",
    {"a.c": """
        #include <stdio.h>
        int arr[100];
        int main(void){ arr[42] = 7; printf("%d %d\\n", arr[42], arr[0]); return 0; }
     """},
    tags=("sections",))

# ============================================================================
# 4. RELOCATIONS / TLS / IFUNC
# ============================================================================

case("pc32_cross_object",
    {"a.c": """
        #include <stdio.h>
        extern int far_fn(int);
        int main(void){ printf("%d\\n", far_fn(4)); return 0; }
     """,
     "b.c": "int far_fn(int x){ return x * 3; }"},
    compile_flags=["-O2", "-fno-pic", "-fno-pie"],
    ldflags=["-no-pie"],
    tags=("reloc",))

case("abs64_data_reloc",
    {"a.c": """
        #include <stdio.h>
        int target = 55;
        int *ptr_to_target = &target;      /* R_X86_64_64 in .data */
        int main(void){ printf("%d\\n", *ptr_to_target); return 0; }
     """},
    tags=("reloc",))

case("got_data_access_pic",
    {"a.c": """
        #include <stdio.h>
        extern int gvar;
        int main(void){ printf("%d\\n", gvar); return 0; }
     """,
     "b.c": "int gvar = 66;"},
    compile_flags=["-O1", "-fpic"],
    tags=("reloc", "got"))

case("tls_local_exec",
    {"a.c": """
        #include <stdio.h>
        static __thread int tls_a = 5;
        static __thread int tls_b;
        int main(void){ tls_b = 7; printf("%d\\n", tls_a + tls_b); return 0; }
     """},
    tags=("tls",))

case("tls_cross_object",
    {"a.c": """
        #include <stdio.h>
        extern __thread int shared_tls;
        int main(void){ shared_tls = 3; printf("%d\\n", shared_tls + 1); return 0; }
     """,
     "b.c": "__thread int shared_tls = 100;"},
    tags=("tls",))

case("tls_initial_values",
    {"a.c": """
        #include <stdio.h>
        #include <pthread.h>
        __thread int tval = 41;
        static void *th(void *p){ (void)p; tval++; return (void*)(long)tval; }
        int main(void){
            pthread_t t; void *r;
            tval = 9;
            pthread_create(&t, 0, th, 0);
            pthread_join(t, &r);
            printf("%d %d\\n", tval, (int)(long)r);
            return 0;
        }
     """},
    ldflags=["-lpthread"],
    tags=("tls",))

case("tls_alignment",
    {"a.c": """
        #include <stdio.h>
        #include <stdint.h>
        __attribute__((aligned(64))) static __thread char tbuf[64];
        static __thread int tsmall = 2;
        int main(void){
            printf("%d %d\\n", (int)((uintptr_t)tbuf & 63), tsmall);
            return 0;
        }
     """},
    tags=("tls", "layout"))

case("tls_general_dynamic",
    {"a.c": """
        #include <stdio.h>
        extern __thread int xtls;
        int main(void){ xtls = 5; printf("%d\\n", xtls); return 0; }
     """,
     "b.c": "__thread int xtls = 1;"},
    compile_flags=["-O1", "-fpic"],
    tags=("tls", "gd"))

case("tls_local_dynamic",
    {"a.c": """
        #include <stdio.h>
        static __thread int a = 3, b = 4;
        int get(void){ return a + b; }
        int main(void){ a = 10; printf("%d\\n", get()); return 0; }
     """},
    compile_flags=["-O1", "-fpic", "-ftls-model=local-dynamic"],
    tags=("tls", "ld"))

case("tls_gd_across_shared_lib",
    {"main.c": """
        #include <stdio.h>
        extern int get_lib_tls(void);
        extern void set_lib_tls(int);
        extern __thread int lib_tls;
        int main(void){
            printf("%d\\n", get_lib_tls());
            set_lib_tls(7);
            printf("%d %d\\n", get_lib_tls(), lib_tls);
            return 0;
        }
     """,
     "impl.c": """
        __thread int lib_tls = 42;
        int get_lib_tls(void){ return lib_tls; }
        void set_lib_tls(int v){ lib_tls = v; }
     """},
    link_inputs=["main.o", "libimpl.so"],
    ldflags=["-Wl,-rpath,$ORIGIN"],
    setup="LCCC_SO",
    tags=("tls", "gd", "shared"))

case("tls_ld_dlopen_lccc_so",
    {"main.c": """
        #include <stdio.h>
        #include <dlfcn.h>
        int main(void){
            void *h = dlopen("./libplug2.so", RTLD_NOW);
            if (!h){ printf("fail %s\\n", dlerror()); return 1; }
            int (*f)(void) = (int(*)(void))dlsym(h, "plug_bump");
            int a = f(); int b = f(); int c = f();
            printf("%d %d %d\\n", a, b, c);
            return 0;
        }
     """,
     "plug2.c": "static __thread int counter;\nint plug_bump(void){ return ++counter; }"},
    link_inputs=["main.o"],
    ldflags=["-ldl"],
    setup="LCCC_SO_PLUG2",
    tags=("tls", "ld", "shared"))

case("tls_mixed_models",
    {"a.c": """
        #include <stdio.h>
        __thread int ie_var = 1;                  /* IE via GOTTPOFF */
        extern __thread int gd_var;               /* GD via TLSGD */
        static __thread int le_var = 3;           /* LE via TPOFF32 */
        int main(void){
            printf("%d\\n", ie_var + gd_var + le_var);
            return 0;
        }
     """,
     "b.c": "__thread int gd_var = 2;"},
    compile_flags=["-O1", "-fpic"],
    tags=("tls",))


case("tls_many_gd_sites",
    {"a.c": """
        #include <stdio.h>
        #define TLS(n) \\
            static __thread int t##n = n; \\
            int get_##n(void){ return t##n; }
        TLS(0) TLS(1) TLS(2) TLS(3) TLS(4) TLS(5) TLS(6) TLS(7)
        TLS(8) TLS(9) TLS(10) TLS(11) TLS(12) TLS(13) TLS(14) TLS(15)
        int main(void){
            int s = 0;
            s += get_0()+get_1()+get_2()+get_3()+get_4()+get_5()+get_6()+get_7();
            s += get_8()+get_9()+get_10()+get_11()+get_12()+get_13()+get_14()+get_15();
            printf("%d\\n", s);
            return 0;
        }
     """},
    compile_flags=["-O1", "-ftls-model=global-dynamic"],
    tags=("tls", "stress"))

case("tls_consumed_skip_correctness",
    {"a.c": """
        #include <stdio.h>
        static __thread long x = 42;
        static __thread long y = 7;
        int main(void){
            printf("%ld\\n", x + y);
            return 0;
        }
     """},
    compile_flags=["-O1", "-ftls-model=global-dynamic"],
    tags=("tls",))

case("icf_identical_leaf_functions",
    {"a.c": """
        #include <stdio.h>
        int leaf_a(int x){ return x * 3 + 1; }
        int main(void){
            extern int leaf_b(int);
            printf("%d\\n", leaf_a(7) + leaf_b(7));
            return 0;
        }
     """,
     "b.c": "int leaf_b(int x){ return x * 3 + 1; }"},
    tags=("icf", "sections"))

case("parallel_reloc_smoke",
    {"a.c": """
        #include <stdio.h>
        extern int f0(void), f1(void), f2(void), f3(void), f4(void);
        extern int f5(void), f6(void), f7(void), f8(void), f9(void);
        int main(void){
            int s = f0()+f1()+f2()+f3()+f4()+f5()+f6()+f7()+f8()+f9();
            printf("%d\\n", s);
            return 0;
        }
     """,
     "f0.c": "int f0(void){ return 1; }",
     "f1.c": "int f1(void){ return 2; }",
     "f2.c": "int f2(void){ return 3; }",
     "f3.c": "int f3(void){ return 4; }",
     "f4.c": "int f4(void){ return 5; }",
     "f5.c": "int f5(void){ return 6; }",
     "f6.c": "int f6(void){ return 7; }",
     "f7.c": "int f7(void){ return 8; }",
     "f8.c": "int f8(void){ return 9; }",
     "f9.c": "int f9(void){ return 10; }"},
    tags=("stress", "reloc"))

case("large_got_pressure",
    {"a.c": """
        #include <stdio.h>
        #include <errno.h>
        int main(void){
            volatile int *p1 = &errno;
            volatile void *p2 = stdin;
            volatile void *p3 = stdout;
            volatile void *p4 = stderr;
            printf("%d %d %d %d\\n", p1 != 0, p2 != 0, p3 != 0, p4 != 0);
            return 0;
        }
     """},
    tags=("got", "dynamic"))


case("ifunc_resolver",
    {"a.c": """
        #include <stdio.h>
        static int impl_a(void){ return 1; }
        static int impl_b(void){ return 2; }
        static int (*resolve_pick(void))(void) { return impl_b; }
        int pick(void) __attribute__((ifunc("resolve_pick")));
        int main(void){ printf("%d\\n", pick()); return 0; }
     """},
    tags=("ifunc",))

case("ifunc_static_link",
    {"a.c": """
        #include <stdio.h>
        static int impl(void){ return 33; }
        static int (*rsv(void))(void) { return impl; }
        int f(void) __attribute__((ifunc("rsv")));
        int main(void){ printf("%d\\n", f()); return 0; }
     """},
    ldflags=["-static"],
    tags=("ifunc", "static"))

case("copy_reloc_libc_data",
    {"a.c": """
        #include <stdio.h>
        extern char **environ;
        int main(void){ printf("%s\\n", environ ? "have-environ" : "null"); return 0; }
     """},
    compile_flags=["-O1", "-fno-pic", "-fno-pie"],
    ldflags=["-no-pie"],
    tags=("reloc", "dynamic"))

case("gotpcrelx_relaxation",
    # GCC emits R_X86_64_REX_GOTPCRELX for extern data under -fpie;
    # linkers may relax mov->lea when the symbol binds locally.
    {"a.c": """
        #include <stdio.h>
        extern int rx_val;
        extern int *rx_addr(void);
        int main(void){ printf("%d %d\\n", rx_val, *rx_addr()); return 0; }
     """,
     "b.c": "int rx_val = 12; int *rx_addr(void){ return &rx_val; }"},
    compile_flags=["-O2", "-fpie"],
    tags=("reloc", "got", "relax"))

# ============================================================================
# 5. DYNAMIC LINKING
# ============================================================================

case("plt_libc_calls",
    {"a.c": """
        #include <stdio.h>
        #include <string.h>
        #include <stdlib.h>
        int main(void){
            char buf[64];
            snprintf(buf, sizeof buf, "%d-%s", atoi("7"), "x");
            printf("%s %zu\\n", buf, strlen(buf));
            return 0;
        }
     """},
    tags=("dynamic",))

case("libm_link",
    {"a.c": """
        #include <stdio.h>
        #include <math.h>
        int main(void){ printf("%.3f %.3f\\n", sqrt(2.0), pow(2.0, 10.0)); return 0; }
     """},
    ldflags=["-lm"],
    tags=("dynamic",))

case("export_dynamic_dladdr",
    {"a.c": """
        #define _GNU_SOURCE
        #include <stdio.h>
        #include <dlfcn.h>
        int exported_marker(void){ return 1; }
        int main(void){
            Dl_info info;
            if (dladdr((void*)&exported_marker, &info) && info.dli_sname)
                printf("%s\\n", info.dli_sname);
            else
                printf("no-symbol\\n");
            return 0;
        }
     """},
    ldflags=["-rdynamic", "-ldl"],
    tags=("dynamic",))

case("dlopen_shared_lib",
    {"main.c": """
        #include <stdio.h>
        #include <dlfcn.h>
        int main(void){
            void *h = dlopen("./libplug.so", RTLD_NOW);
            if (!h) { printf("dlopen-failed %s\\n", dlerror()); return 1; }
            int (*fn)(int) = (int(*)(int))dlsym(h, "plug_fn");
            if (!fn) { printf("dlsym-failed\\n"); return 1; }
            printf("%d\\n", fn(20));
            dlclose(h);
            return 0;
        }
     """,
     "plug.c": "int plug_fn(int x){ return x + 2; }"},
    link_inputs=["main.o"],
    ldflags=["-ldl"],
    setup=lambda td: (sh([CC, "-c", "-fpic", "plug.c"], cwd=td),
                      sh([CC, "-shared", "plug.o", "-o", "libplug.so"], cwd=td)),
    tags=("dynamic",))

def _mklib_lccc_so(td, lccc):
    """Build shared library with LCCC's own linker."""
    r = sh([CC, "-c", "-fpic", "impl.c"], cwd=td)
    assert r.returncode == 0, r.stderr.decode()
    r = sh([lccc, "-shared", "impl.o", "-o", "libimpl.so"], cwd=td)
    return r

case("link_against_lccc_so",
    {"main.c": """
        #include <stdio.h>
        extern int impl_fn(int);
        extern int impl_var;
        int main(void){ printf("%d %d\\n", impl_fn(5), impl_var); return 0; }
     """,
     "impl.c": "int impl_var = 30;\nint impl_fn(int x){ return x * impl_var; }"},
    link_inputs=["main.o", "libimpl.so"],
    ldflags=["-Wl,-rpath,$ORIGIN"],
    setup="LCCC_SO",   # special: needs lccc path
    tags=("dynamic", "shared"))

case("soname_and_needed",
    {"main.c": """
        #include <stdio.h>
        extern int sn_fn(void);
        int main(void){ printf("%d\\n", sn_fn()); return 0; }
     """,
     "impl.c": "int sn_fn(void){ return 88; }"},
    link_inputs=["main.o", "libsn.so.1"],
    ldflags=["-Wl,-rpath,$ORIGIN"],
    setup=lambda td: (sh([CC, "-c", "-fpic", "impl.c"], cwd=td),
                      sh([CC, "-shared", "-Wl,-soname,libsn.so.1", "impl.o",
                          "-o", "libsn.so.1"], cwd=td)),
    tags=("dynamic", "shared"))

case("preinit_array",
    {"a.c": """
        #include <stdio.h>
        static void pre(void){ printf("pre\\n"); }
        __attribute__((used, section(".preinit_array")))
        static void (*pre_ptr)(void) = pre;
        int main(void){ printf("main\\n"); return 0; }
     """},
    tags=("dynamic", "sections"))

# ============================================================================
# 6. STATIC LINKING
# ============================================================================

case("static_hello",
    {"a.c": """
        #include <stdio.h>
        int main(void){ printf("static-ok\\n"); return 0; }
     """},
    ldflags=["-static"],
    tags=("static",))

case("static_tls_pthread",
    {"a.c": """
        #include <stdio.h>
        #include <pthread.h>
        __thread int stv = 4;
        static void *th(void *p){ (void)p; return (void*)(long)(stv + 1); }
        int main(void){
            pthread_t t; void *r;
            pthread_create(&t, 0, th, 0);
            pthread_join(t, &r);
            printf("%d\\n", (int)(long)r);
            return 0;
        }
     """},
    ldflags=["-static", "-lpthread"],
    tags=("static", "tls"))

case("static_malloc_heavy",
    {"a.c": """
        #include <stdio.h>
        #include <stdlib.h>
        #include <string.h>
        int main(void){
            unsigned s = 0;
            for (int i = 1; i < 200; i++) {
                char *p = malloc(i * 13);
                memset(p, i, i * 13);
                s += (unsigned char)p[i - 1];
                free(p);
            }
            printf("%u\\n", s);
            return 0;
        }
     """},
    ldflags=["-static"],
    tags=("static",))

# ============================================================================
# 7. ENTRY / NOSTDLIB / SPECIAL LINK MODES
# ============================================================================

case("nostdlib_custom_start",
    {"a.c": """
        long write_sys(int fd, const void *buf, unsigned long n){
            long r;
            __asm__ volatile("syscall" : "=a"(r)
                             : "a"(1L), "D"((long)fd), "S"(buf), "d"(n)
                             : "rcx", "r11", "memory");
            return r;
        }
        void _start(void){
            write_sys(1, "bare\\n", 5);
            __asm__ volatile("syscall" :: "a"(60L), "D"(0L));
            __builtin_unreachable();
        }
     """},
    ldflags=["-nostdlib", "-static"],
    tags=("special",))

case("custom_entry_flag",
    {"a.c": """
        long wr(int fd, const void *buf, unsigned long n){
            long r;
            __asm__ volatile("syscall" : "=a"(r)
                             : "a"(1L), "D"((long)fd), "S"(buf), "d"(n)
                             : "rcx", "r11", "memory");
            return r;
        }
        void my_entry(void){
            wr(1, "entry\\n", 6);
            __asm__ volatile("syscall" :: "a"(60L), "D"(0L));
            __builtin_unreachable();
        }
     """},
    ldflags=["-nostdlib", "-static", "-Wl,-e,my_entry"],
    tags=("special",))

case("defsym_alias",
    {"a.c": """
        #include <stdio.h>
        extern int defsym_target(void);
        int real_target(void){ return 61; }
        int main(void){ printf("%d\\n", defsym_target()); return 0; }
     """},
    ldflags=["-Wl,--defsym=defsym_target=real_target"],
    tags=("special",))

case("wrap_symbol",
    {"a.c": """
        #include <stdio.h>
        extern int compute(int);
        int main(void){ printf("%d\\n", compute(10)); return 0; }
     """,
     "b.c": "int compute(int x){ return x * 2; }",
     "w.c": """
        extern int __real_compute(int);
        int __wrap_compute(int x){ return __real_compute(x) + 100; }
     """},
    ldflags=["-Wl,--wrap=compute"],
    tags=("special", "wrap"))

case("z_now_relro",
    {"a.c": """
        #include <stdio.h>
        int main(void){ printf("relro-ok\\n"); return 0; }
     """},
    ldflags=["-Wl,-z,now", "-Wl,-z,relro"],
    tags=("special",))

case("relro_write_protection",
    # PT_GNU_RELRO must actually protect .init_array after startup: a write
    # into the RELRO page has to SIGSEGV. Compared against the bfd/mold/wild
    # linked references, which enforce the same.
    {"a.c": """
        #include <stdio.h>
        #include <signal.h>
        #include <setjmp.h>
        static sigjmp_buf jb;
        static void segv(int s){ (void)s; siglongjmp(jb, 1); }
        typedef void (*fp)(void);
        __attribute__((constructor)) static void c1(void){}
        extern fp __init_array_start[] __attribute__((weak));
        int main(void){
            signal(SIGSEGV, segv);
            if (sigsetjmp(jb, 1) == 0) {
                __init_array_start[0] = (fp)main;
                printf("WRITABLE\\n");
                return 1;
            }
            printf("PROTECTED\\n");
            return 0;
        }
     """},
    compile_flags=["-O0"],
    ldflags=["-Wl,-z,now"],
    tags=("special", "relro"))

case("z_now_dynamic_flags",
    # -z now must emit DT_FLAGS=BIND_NOW and DT_FLAGS_1=NOW (checked by
    # readelf below via expect_readelf_dynamic), and the binary must run.
    {"a.c": '#include <stdio.h>\nint main(void){ printf("now\\n"); return 0; }'},
    ldflags=["-Wl,-z,now"],
    tags=("special", "relro"))

case("z_noexecstack",
    {"a.c": '#include <stdio.h>\nint main(void){ printf("nx\\n"); return 0; }'},
    ldflags=["-Wl,-z,noexecstack"],
    tags=("special",))

case("as_needed_flag",
    {"a.c": '#include <stdio.h>\nint main(void){ printf("an\\n"); return 0; }'},
    ldflags=["-Wl,--as-needed", "-lm", "-Wl,--no-as-needed"],
    tags=("special",))

case("build_id_flag_accepted",
    {"a.c": '#include <stdio.h>\nint main(void){ printf("bid\\n"); return 0; }'},
    ldflags=["-Wl,--build-id"],
    tags=("special",))

case("strip_all",
    {"a.c": '#include <stdio.h>\nint main(void){ printf("stripped\\n"); return 0; }'},
    ldflags=["-Wl,-s"],
    tags=("special",))

case("undefined_flag_pulls_archive",
    {"main.c": """
        #include <stdio.h>
        int main(void){ extern int pulled_flag; printf("%d\\n", pulled_flag); return 0; }
     """,
     "u.c": """
        int pulled_flag;
        __attribute__((constructor)) static void ic(void){ pulled_flag = 4; }
        int force_me(void){ return 0; }
     """},
    link_inputs=["main.o", "libu.a"],
    ldflags=["-Wl,-u,force_me"],
    setup=lambda td: (sh([CC, "-c", "u.c"], cwd=td),
                      sh(["ar", "rcs", "libu.a", "u.o"], cwd=td)),
    expect_fail=False,
    tags=("special",),
    # pulled_flag only defined in archive member force_me lives in; without -u
    # nothing references the member so link would fail on pulled_flag.
    )

# ============================================================================
# 8. C++-STYLE INPUTS (COMDAT groups, .eh_frame) — compiled from C w/ asm
# ============================================================================

COMDAT_ASM = r"""
    .section .text.dupfn,"axG",@progbits,dupfn,comdat
    .globl dupfn
    .weak dupfn
    .type dupfn,@function
dupfn:
    movl ${val}, %eax
    ret
    .size dupfn, .-dupfn
"""

case("comdat_dedup",
    {"main.c": """
        #include <stdio.h>
        extern int dupfn(void);
        int main(void){ printf("%d\\n", dupfn()); return 0; }
     """,
     "g1.s": COMDAT_ASM.format(val=7),
     "g2.s": COMDAT_ASM.format(val=7)},
    tags=("comdat",))

case("eh_frame_present",
    {"a.c": """
        #include <stdio.h>
        /* force .eh_frame with -fasynchronous-unwind-tables (default on x86-64) */
        int deep(int n){ return n <= 0 ? 0 : deep(n - 1) + 1; }
        int main(void){ printf("%d\\n", deep(10)); return 0; }
     """},
    compile_flags=["-O0", "-fasynchronous-unwind-tables"],
    tags=("ehframe",))

case("eh_frame_hdr_backtrace",
    # PT_GNU_EH_FRAME + .eh_frame_hdr binary search table: backtrace()
    # depends on the header to walk frames without a linear .eh_frame scan.
    {"a.c": """
        #include <stdio.h>
        #include <execinfo.h>
        __attribute__((noinline)) int level3(void){
            void *frames[16];
            int n = backtrace(frames, 16);
            printf("%s\\n", n > 3 ? "deep-stack" : "shallow");
            return n;
        }
        __attribute__((noinline)) int level2(void){ return level3() + 1; }
        __attribute__((noinline)) int level1(void){ return level2() + 1; }
        int main(void){ level1(); return 0; }
     """},
    compile_flags=["-O0", "-fasynchronous-unwind-tables", "-fno-omit-frame-pointer"],
    tags=("ehframe",))

# ============================================================================
# 9. SCALE / STRESS
# ============================================================================

def _many_objects_sources():
    srcs = {}
    calls, protos = [], []
    for i in range(60):
        srcs[f"m{i}.c"] = f"int fn_{i}(int x){{ return x + {i}; }}\n"
        protos.append(f"extern int fn_{i}(int);")
        calls.append(f"s += fn_{i}(i);")
    srcs["main.c"] = ("#include <stdio.h>\n" + "\n".join(protos) +
        "\nint main(void){ int s = 0; for (int i = 0; i < 3; i++) { " +
        " ".join(calls) + " } printf(\"%d\\n\", s); return 0; }\n")
    return srcs

case("many_objects_60", _many_objects_sources(), tags=("stress",))

case("mixed_pic_nopic",
    {"a.c": """
        #include <stdio.h>
        extern int mixed(void);
        int main(void){ printf("%d\\n", mixed()); return 0; }
     """,
     "b.c": "int mixed(void){ return 3; }"},
    compile_flags=["-O1"],  # a.o gets default; b.o overridden in setup
    setup=lambda td: sh([CC, "-c", "-fno-pic", "b.c"], cwd=td),
    tags=("stress",))

case("large_rodata",
    {"a.c": """
        #include <stdio.h>
        const unsigned char table[65536] = {1, 2, 3, [65535] = 9};
        int main(void){
            unsigned s = 0;
            for (int i = 0; i < 65536; i++) s += table[i];
            printf("%u\\n", s);
            return 0;
        }
     """},
    tags=("stress",))

# ============================================================================
# 10. LINKER SCRIPT (-T) — exercised via the lccc-ld driver
# ============================================================================

KERNEL_STYLE_SCRIPT = r"""
ENTRY(my_start)
PHDRS {
 text PT_LOAD FLAGS(5);
 data PT_LOAD FLAGS(6);
}
SECTIONS
{
 . = 0x400000;
 _stext = .;
 .text : {
  *(.text .text.*)
  . = ALIGN(16);
  __special_start = .;
  KEEP(*(.special))
  __special_end = .;
 } :text = 0x90909090
 _etext = .;
 . = ALIGN(0x1000);
 .rodata : { *(.rodata .rodata.*) } :data
 .data : { _sdata = .; *(.data .data.*) _edata = .; }
 .bss : { __bss_start = .; *(.bss .bss.*) *(COMMON) __bss_stop = .; }
 _end = .;
 /DISCARD/ : { *(.comment) *(.note.*) *(.eh_frame) }
}
ASSERT(_end - 0x400000 < 0x100000, "image too big")
"""

SCRIPT_TEST_C = r"""
__attribute__((used, section(".special"))) static int spec1 = 11;
__attribute__((used, section(".special"))) static int spec2 = 31;
extern int __special_start[], __special_end[];
static int sum_special(void){
    int s = 0;
    for (int *p = __special_start; p < __special_end; p++) s += *p;
    return s;
}
int global_data = 5;
static long wr(int fd, const void *buf, unsigned long n){
    long r;
    __asm__ volatile("syscall" : "=a"(r)
                     : "a"(1L), "D"((long)fd), "S"(buf), "d"(n)
                     : "rcx", "r11", "memory");
    return r;
}
void my_start(void){
    char msg[2] = { (char)('0' + (sum_special() == 42) + (global_data == 5)), '\n' };
    wr(1, msg, 2);
    __asm__ volatile("syscall" :: "a"(60L), "D"(0L));
    __builtin_unreachable();
}
"""

def _script_test(name, script, csrc, expect_stdout, cflags=None):
    def runner(args, oracles):
        td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
        try:
            with open(os.path.join(td, "t.c"), "w") as f:
                f.write(csrc)
            with open(os.path.join(td, "t.lds"), "w") as f:
                f.write(script)
            r = sh([CC, "-c", "t.c", "-o", "t.o"] + (cflags or ["-O1", "-fno-pic",
                    "-fno-asynchronous-unwind-tables", "-fno-stack-protector"]), cwd=td)
            if r.returncode != 0:
                return Result(name, "SKIP", r.stderr.decode()[:200])
            lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
            r = sh([lccc_ld, "-T", "t.lds", "t.o", "-o", "out.lccc"], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"lccc-ld failed: {r.stderr.decode()[:300]}")
            code, out = run_bin(os.path.join(td, "out.lccc"), [], td)
            # oracle: GNU ld with the same script
            r2 = sh(["ld", "-T", "t.lds", "t.o", "-o", "out.ld"], cwd=td)
            if r2.returncode == 0:
                code2, out2 = run_bin(os.path.join(td, "out.ld"), [], td)
                if (code, out) != (code2, out2):
                    return Result(name, "FAIL",
                        f"lccc {(code, out)!r} != GNU ld {(code2, out2)!r}")
            if out != expect_stdout or code != 0:
                return Result(name, "FAIL", f"got {(code, out)!r}")
            return Result(name, "PASS")
        except Exception as e:
            return Result(name, "FAIL", f"harness exception: {e!r}")
        finally:
            shutil.rmtree(td, ignore_errors=True)
    return runner

SCRIPT_TESTS = [
    ("script_kernel_style", KERNEL_STYLE_SCRIPT, SCRIPT_TEST_C, "2\n"),
]

# PIE script link (kernel-decompressor style): base-0 ET_DYN with all
# dynamic sections discarded and only PC-relative relocations. Verified
# structurally (GNU ld's output for this pattern cannot be executed as a
# userspace binary either - the stub relocates itself).
PIE_SCRIPT = r"""
ENTRY(my_start)
SECTIONS
{
 . = 0;
 .head.text : { _head = .; *(.head.text) _ehead = .; }
 .text : { _text = .; *(.text .text.*) _etext = .; }
 .rodata : { _rodata = .; *(.rodata .rodata.*) _erodata = .; }
 .data : ALIGN(0x1000) { _data = .; *(.data .data.*) _edata = .; }
 .bss : { _bss = .; *(.bss .bss.*) *(COMMON) . = ALIGN(8); _ebss = .; }
 /DISCARD/ : { *(.dynamic) *(.dynsym) *(.dynstr) *(.hash) *(.gnu.hash)
               *(.note.*) *(.comment) *(.eh_frame) }
}
"""

PIE_TEST_C = r"""
__attribute__((section(".head.text"), used))
void my_start(void){ }
int a_var = 5;
const int r_var = 7;
int compute(int x){ return x + a_var + r_var; }
"""

def _pie_script_test(args, oracles):
    name = "script_pie_base0"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "t.c"), "w") as f:
            f.write(PIE_TEST_C)
        with open(os.path.join(td, "t.lds"), "w") as f:
            f.write(PIE_SCRIPT)
        r = sh([CC, "-c", "-O1", "-fno-pic", "-fno-asynchronous-unwind-tables",
                "-fno-stack-protector", "t.c"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:150])
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        r = sh([lccc_ld, "-pie", "--no-dynamic-linker", "-T", "t.lds",
                "t.o", "-o", "out.lccc"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc-ld -pie failed: {r.stderr.decode()[:300]}")
        r2 = sh(["ld", "-pie", "--no-dynamic-linker", "-T", "t.lds",
                 "t.o", "-o", "out.ld"], cwd=td)
        if r2.returncode != 0:
            return Result(name, "SKIP", "GNU ld -pie failed")
        # structural comparison: e_type, entry, key symbol addresses
        def props(binp):
            rh = sh(["readelf", "-h", binp], cwd=td).stdout.decode()
            etype = [l for l in rh.splitlines() if "Type:" in l][0].split()[1]
            nm_out = sh(["nm", binp], cwd=td).stdout.decode()
            syms = {}
            for l in nm_out.splitlines():
                parts = l.split()
                if len(parts) == 3 and parts[2] in (
                    "_head", "_text", "_etext", "_rodata", "_data", "my_start"):
                    syms[parts[2]] = parts[0]
            return etype, syms
        et_a, sy_a = props("out.lccc")
        et_b, sy_b = props("out.ld")
        if et_a != "DYN":
            return Result(name, "FAIL", f"expected ET_DYN, got {et_a}")
        if et_a != et_b or sy_a != sy_b:
            return Result(name, "FAIL",
                f"structure mismatch: lccc ({et_a},{sy_a}) vs ld ({et_b},{sy_b})")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)

# ============================================================================
# 11. RELOCATABLE LINKING (ld -r) — differential against GNU ld -r
# ============================================================================

def _rel_test(name, sources, expect_stdout, asm=None, compile_flags=None):
    """lccc-ld -r vs GNU ld -r: merge objects, final-link both merged objects
    with gcc AND with lccc, run all four, all outputs must agree."""
    def runner(args, oracles):
        td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
        try:
            objs = []
            for fname, content in sources.items():
                with open(os.path.join(td, fname), "w") as f:
                    f.write(textwrap.dedent(content))
                obj = os.path.splitext(fname)[0] + ".o"
                r = sh([CC, "-c", fname, "-o", obj] + (compile_flags or ["-O1"]), cwd=td)
                if r.returncode != 0:
                    return Result(name, "SKIP", r.stderr.decode()[:200])
                objs.append(obj)
            lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
            r = sh([lccc_ld, "-r"] + objs + ["-o", "m.lccc.o"], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"lccc-ld -r failed: {r.stderr.decode()[:300]}")
            r = sh(["ld", "-r"] + objs + ["-o", "m.ld.o"], cwd=td)
            if r.returncode != 0:
                return Result(name, "SKIP", "GNU ld -r failed")
            outs = {}
            for tag, merged, linker in [
                ("lccc-r+gcc", "m.lccc.o", [CC]),
                ("ld-r+gcc", "m.ld.o", [CC]),
                ("lccc-r+lccc", "m.lccc.o", [args.lccc]),
            ]:
                rr = sh(linker + [merged, "-o", f"fin.{tag}"], cwd=td)
                if rr.returncode != 0:
                    return Result(name, "FAIL",
                        f"final link ({tag}) failed: {rr.stderr.decode()[:300]}")
                code, out = run_bin(os.path.join(td, f"fin.{tag}"), [], td)
                outs[tag] = (code, out)
            vals = set(outs.values())
            if len(vals) != 1:
                return Result(name, "FAIL", f"outputs disagree: {outs!r}")
            code, out = vals.pop()
            if expect_stdout is not None and (out != expect_stdout or code != 0):
                return Result(name, "FAIL", f"got {(code, out)!r}")
            return Result(name, "PASS")
        except Exception as e:
            return Result(name, "FAIL", f"harness exception: {e!r}")
        finally:
            shutil.rmtree(td, ignore_errors=True)
    return runner

# ============================================================================
# 12. C++ EXCEPTIONS — unwinding through lccc-linked binaries
# ============================================================================

# A *locally defined* exception class. This is deliberately not
# std::runtime_error: the typeinfo for a library type lives in libstdc++, so
# throwing one never makes the linker emit a typeinfo object of its own. A
# user-defined class forces the compiler to emit `_ZTI1E` into
# .data.rel.ro, whose first word is an absolute 64-bit reference to
# `_ZTVN10__cxxabiv117__class_type_infoE` (+0x10) in libstdc++.
#
# In an ET_EXEC link that address is unknowable until ld.so maps the library,
# so it must become a dynamic R_X86_64_64. lccc used to write only the addend
# (0x10) and emit no dynamic relocation, so __gxx_personality_v0 dereferenced
# 0x10 while matching the LSDA type table and the program died with SIGSEGV --
# *after* printing correct output, which is exactly the kind of failure a
# stdout-only comparison misses. Hence the explicit exit-code check below.
CXX_EH_LOCAL_TYPE_SRC = r"""
#include <cstdio>
struct E { int v; };
struct F { double d; };
__attribute__((noinline)) static void deep3(int x){ if (x > 2) throw E{x * 7}; }
__attribute__((noinline)) static void deep2(int x){ deep3(x); }
__attribute__((noinline)) static void deep1(int x){ deep2(x); }
int main(){
    int caught = 0, sum = 0;
    for (int i = 0; i < 6; i++) {
        try { deep1(i); }
        catch (const E &e) { caught++; sum += e.v; }
    }
    // A second, unrelated local type: exercises more than one typeinfo object
    // and therefore more than one absolute dynamic relocation.
    try { throw F{1.5}; } catch (const F &f) { printf("f=%.1f\n", f.d); }
    printf("caught=%d sum=%d\n", caught, sum);
    return 0;
}
"""

CXX_EH_SRC = r"""
#include <cstdio>
#include <stdexcept>
#include <string>
struct Probe {
    const char *name;
    explicit Probe(const char *n) : name(n) { printf("ctor %s\n", name); }
    ~Probe() { printf("dtor %s\n", name); }
};
static int depth3(int x){
    Probe p("d3");
    if (x > 2) throw std::runtime_error("boom-" + std::to_string(x));
    return x;
}
static int depth2(int x){ Probe p("d2"); return depth3(x + 1); }
static int depth1(int x){ Probe p("d1"); return depth2(x + 1); }
int main(){
    try { depth1(1); }
    catch (const std::exception &e) { printf("caught: %s\n", e.what()); }
    try { throw 42; } catch (int v) { printf("int: %d\n", v); }
    printf("done\n");
    return 0;
}
"""

INTERPOSE_LIB = r"""
int get_answer(void){ return 42; }
int call_get(void){ return get_answer() + 1; }
"""
INTERPOSE_MAIN = r"""
#include <stdio.h>
int get_answer(void){ return 100; }   /* interposer in the executable */
extern int call_get(void);
int main(void){ printf("%d\n", call_get()); return 0; }
"""

def _interpose_test(args, oracles, name, extra_so_flags, expect):
    """Shared-library symbol interposition semantics vs GNU reference."""
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "lib.c"), "w") as f:
            f.write(INTERPOSE_LIB)
        with open(os.path.join(td, "main.c"), "w") as f:
            f.write(INTERPOSE_MAIN)
        r = sh([CC, "-c", "-fpic", "-O1", "lib.c"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", "fixture compile failed")
        r = sh([args.lccc, "-shared", "lib.o"] + extra_so_flags
               + ["-o", "libt.so"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc -shared failed: {r.stderr.decode()[:200]}")
        r = sh([CC, "main.c", "./libt.so", "-Wl,-rpath,$ORIGIN", "-o", "m"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"main link failed: {r.stderr.decode()[:200]}")
        code, out = run_bin(os.path.join(td, "m"), [], td)
        # GNU reference
        r2 = sh([CC, "-shared", "lib.o"] + extra_so_flags + ["-o", "libr.so"], cwd=td)
        ref = None
        if r2.returncode == 0:
            r3 = sh([CC, "main.c", "./libr.so", "-Wl,-rpath,$ORIGIN", "-o", "mr"], cwd=td)
            if r3.returncode == 0:
                _, ref = run_bin(os.path.join(td, "mr"), [], td)
        if out != expect or code != 0:
            return Result(name, "FAIL", f"got {(code, out)!r}, want {expect!r} (ref={ref!r})")
        if ref is not None and out != ref:
            return Result(name, "FAIL", f"mismatch vs GNU ref: {out!r} != {ref!r}")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)

def _zdefs_test(args, oracles):
    """-z defs: undefined symbol in a .so must fail the link."""
    name = "so_z_defs_rejects_undefined"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "u.c"), "w") as f:
            f.write("extern int nowhere(void);\nint f(void){ return nowhere(); }\n")
        sh([CC, "-c", "-fpic", "u.c"], cwd=td)
        r1 = sh([args.lccc, "-shared", "u.o", "-o", "a.so"], cwd=td)
        r2 = sh([args.lccc, "-shared", "u.o", "-Wl,-z,defs", "-o", "b.so"], cwd=td)
        if r1.returncode != 0:
            return Result(name, "FAIL", "default .so link should tolerate undefined")
        if r2.returncode == 0:
            return Result(name, "FAIL", "-z defs should reject undefined symbol")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)

def _so_shared_flags_test(args, oracles):
    """Flags that worked for executables must also work for shared libraries.

    `link_shared` used to carry its own argument parser, a near-copy of
    `linker_common::parse_linker_args`. Whichever copy was forgotten failed
    *silently*: measured before the parsers were merged, 21 flags known to
    args.rs were dropped on the .so path. Two had user-visible consequences and
    are pinned here, both against ld.bfd:

      -Map=FILE   wrote a map for executables, nothing at all for .so
      --defsym    bfd emitted the alias symbol, lccc emitted none

    A single-parser regression would silently drop these again, so this test
    is the guard for the whole class, not just the two instances.
    """
    name = "so_shared_flag_parity"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "lib.c"), "w") as f:
            f.write("int keep_me(void){return 42;}\nint other(void){return 7;}\n")
        r = sh([CC, "-fPIC", "-c", "lib.c"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", "compile failed")

        ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")

        # --- -Map on a shared library ---
        r = sh([ld, "-shared", "-Map=m.map", "-o", "a.so", "lib.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"-Map link failed: {r.stderr.decode()[:200]}")
        mp = os.path.join(td, "m.map")
        if not os.path.exists(mp) or os.path.getsize(mp) == 0:
            return Result(name, "FAIL", "-Map wrote no map file for a shared library")

        # The map must agree with the ELF actually emitted, not merely exist.
        elf = sh(["readelf", "-SW", "a.so"], cwd=td).stdout.decode(errors="replace")
        secs = {}
        for line in elf.splitlines():
            m = re.match(r"\s*\[\s*\d+\]\s+(\S+)\s+\S+\s+([0-9a-f]+)", line)
            if m:
                secs[m.group(1)] = int(m.group(2), 16)
        checked = 0
        for line in open(mp):
            m = re.match(r"^(\.[\w.]+)\s+0x([0-9a-f]+)\s+\d+", line)
            if m and m.group(1) in secs:
                if secs[m.group(1)] != int(m.group(2), 16):
                    return Result(name, "FAIL",
                        f"map address for {m.group(1)} disagrees with the ELF")
                checked += 1
        if checked == 0:
            return Result(name, "FAIL", "map contained no verifiable section rows")

        # --- --defsym on a shared library, compared against bfd ---
        bfd = shutil.which("ld.bfd")
        r = sh([ld, "-shared", "--defsym=alias_sym=keep_me", "-o", "d.so", "lib.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"--defsym link failed: {r.stderr.decode()[:200]}")
        got = sh(["readelf", "--dyn-syms", "-W", "d.so"], cwd=td).stdout.decode(errors="replace")
        n_lccc = got.count("alias_sym")
        if bfd:
            sh([bfd, "-shared", "--defsym=alias_sym=keep_me", "-o", "db.so", "lib.o"], cwd=td)
            ref = sh(["readelf", "--dyn-syms", "-W", "db.so"], cwd=td).stdout.decode(errors="replace")
            n_bfd = ref.count("alias_sym")
            if n_lccc != n_bfd:
                return Result(name, "FAIL",
                    f"--defsym alias count {n_lccc} != bfd {n_bfd}")
        elif n_lccc == 0:
            return Result(name, "FAIL", "--defsym produced no alias symbol")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _cxx_eh_local_type_test(args, oracles):
    """Throw a locally-defined class type across several frames.

    Guards the absolute-dynamic-relocation path (see CXX_EH_LOCAL_TYPE_SRC).
    Checks the *exit code* as well as stdout, because the historical failure
    printed the right answer and then segfaulted while unwinding.
    """
    name = "cxx_exceptions_local_typeinfo"
    with tempfile.TemporaryDirectory() as td:
        with open(os.path.join(td, "lt.cpp"), "w") as f:
            f.write(CXX_EH_LOCAL_TYPE_SRC)
        r = sh(["g++", "-c", "-O1", "lt.cpp"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", "g++ compile failed")
        r = sh([args.lccc, "lt.o", "-lstdc++", "-o", "lt.lccc"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc link failed: {r.stderr.decode()[:300]}")
        code, out = run_bin(os.path.join(td, "lt.lccc"), [], td)
        r2 = sh(["g++", "lt.o", "-lstdc++", "-o", "lt.ref"], cwd=td)
        if r2.returncode != 0:
            return Result(name, "SKIP", "reference link failed")
        code2, out2 = run_bin(os.path.join(td, "lt.ref"), [], td)
        if (code, out) != (code2, out2):
            return Result(name, "FAIL",
                f"lccc {(code, out)!r} != g++ {(code2, out2)!r}")
        # Explicit: a SIGSEGV during unwinding shows up only in the exit code.
        if code != 0:
            return Result(name, "FAIL",
                f"non-zero exit {code} (unwinder crashed?) with output {out!r}")
        if "caught=3 sum=84" not in out:
            return Result(name, "FAIL", f"unexpected output {out!r}")

        # The emitted image must carry a dynamic R_X86_64_64 for the class
        # type_info vtable; without it the typeinfo slot keeps a bare addend.
        rr = sh(["readelf", "-rW", "lt.lccc"], cwd=td)
        rel = rr.stdout.decode(errors="replace")
        if "R_X86_64_64" not in rel or "__cxxabiv1" not in rel:
            return Result(name, "FAIL",
                "no dynamic R_X86_64_64 against a __cxxabiv1 type_info vtable; "
                "the typeinfo slot would hold only the addend")
        return Result(name, "PASS")


def _cxx_eh_test(args, oracles):
    name = "cxx_exceptions_unwind"
    if shutil.which("g++") is None:
        return Result(name, "SKIP", "no g++")
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "ex.cpp"), "w") as f:
            f.write(CXX_EH_SRC)
        r = sh(["g++", "-c", "-O1", "ex.cpp"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", "g++ compile failed")
        r = sh([args.lccc, "ex.o", "-lstdc++", "-o", "ex.lccc"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc link failed: {r.stderr.decode()[:300]}")
        code, out = run_bin(os.path.join(td, "ex.lccc"), [], td)
        r2 = sh(["g++", "ex.o", "-o", "ex.ref"], cwd=td)
        code2, out2 = run_bin(os.path.join(td, "ex.ref"), [], td)
        if (code, out) != (code2, out2):
            return Result(name, "FAIL",
                f"lccc {(code, out)!r} != g++ {(code2, out2)!r}")
        if "caught: boom-3" not in out or code != 0:
            return Result(name, "FAIL", f"unexpected output {(code, out)!r}")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)

REL_TESTS = [
    ("rel_basic_merge",
     {"a.c": """
        #include <stdio.h>
        extern int bval; extern int bfn(int);
        static int helper(int x){ return x * 3; }
        int main(void){ printf("%d\\n", helper(2) + bfn(4) + bval); return 0; }
      """,
      "b.c": "int bval = 50;\nstatic int bh = 5;\nint bfn(int x){ return x + bh; }"},
     "65\n"),
    ("rel_local_name_collision",
     {"a.c": """
        #include <stdio.h>
        static int counter = 10;   /* same local name in both objects */
        int a_get(void){ return counter; }
        int main(void){ extern int b_get(void);
                        printf("%d %d\\n", a_get(), b_get()); return 0; }
      """,
      "b.c": "static int counter = 20;\nint b_get(void){ return counter; }"},
     "10 20\n"),
    ("rel_common_symbols",
     {"a.c": """
        #include <stdio.h>
        int shared;   /* tentative */
        int main(void){ extern void bump(void); bump();
                        printf("%d\\n", shared); return 0; }
      """,
      "b.c": "int shared;\nvoid bump(void){ shared += 7; }"},
     "7\n", ["-O1", "-fcommon"]),
    ("rel_ctor_preserved",
     {"a.c": """
        #include <stdio.h>
        int flag;
        __attribute__((constructor)) static void init(void){ flag = 9; }
        int main(void){ printf("%d\\n", flag); return 0; }
      """,
      "b.c": "int other(void){ return 1; }"},
     "9\n"),
    ("rel_data_relocs",
     {"a.c": """
        #include <stdio.h>
        extern int t1(void), t2(void);
        int (*table[2])(void) = { t1, t2 };   /* R_X86_64_64 in .data */
        int main(void){ printf("%d\\n", table[0]() + table[1]()); return 0; }
      """,
      "b.c": "int t1(void){ return 30; }\nint t2(void){ return 12; }"},
     "42\n"),
]

# ============================================================================
# ROBUSTNESS: malformed / hostile ELF inputs
# ============================================================================
#
# A linker is routinely handed corrupt objects: interrupted builds, truncated
# artefacts on a full disk, bad NFS writes, or deliberately hostile input in a
# distro build service.  The contract is:
#
#     reject with a clear diagnostic and a normal error exit  --  never
#     panic, never abort in the allocator, never read out of bounds, never
#     loop forever.
#
# Every case below is a *surgical* mutation of one field in a real object file
# (not a random smash), so the test states precisely which invariant is under
# test.  Cases marked `found_by_fuzzing` are regressions for defects an actual
# mutation-fuzzing campaign found in this linker.
#
# Note on oracles: bfd and mold silently *accept* several of these (mold even
# segfaults on the truncated-section case, exit 139), so they cannot serve as
# a pass/fail oracle here.  The invariant is lccc-internal and absolute:
# a controlled rejection, i.e. exit code 1 with a diagnostic, or a successful
# link -- but never a crash, an abort, or a hang.

ROBUSTNESS_SRC = r"""
static char msg[16] = "hi\n";
static int table[8] = {1,2,3,4,5,6,7,8};
int helper(int x) { return x * 3 + table[x & 7]; }
static long wr(long fd, const void *b, long n) {
    long r; __asm__ volatile("syscall" : "=a"(r) : "a"(1),"D"(fd),"S"(b),"d"(n)
                             : "rcx","r11","memory"); return r;
}
void _start(void) {
    wr(1, msg, 3);
    long code = helper(5);
    __asm__ volatile("syscall" :: "a"(60), "D"(code));
    __builtin_unreachable();
}
"""

def _u16(b, off):  return int.from_bytes(b[off:off+2], "little")
def _u32(b, off):  return int.from_bytes(b[off:off+4], "little")
def _u64(b, off):  return int.from_bytes(b[off:off+8], "little")
def _pu16(b, off, v): b[off:off+2] = (v & 0xffff).to_bytes(2, "little")
def _pu32(b, off, v): b[off:off+4] = (v & 0xffffffff).to_bytes(4, "little")
def _pu64(b, off, v): b[off:off+8] = (v & 0xffffffffffffffff).to_bytes(8, "little")

def _shdr(b, i):
    """Return the file offset of section header `i`."""
    return _u64(b, 40) + i * _u16(b, 58)

def _find_section(b, name):
    """Return index of the section whose name is `name`, or None."""
    shoff, shentsize, shnum = _u64(b, 40), _u16(b, 58), _u16(b, 60)
    shstrndx = _u16(b, 62)
    stro = _u64(b, shoff + shstrndx * shentsize + 24)
    for i in range(shnum):
        nameo = stro + _u32(b, shoff + i * shentsize)
        end = b.index(b"\0", nameo)
        if b[nameo:end].decode() == name:
            return i
    return None

# --- the mutations -----------------------------------------------------------
# Each entry: (case name, mutator(bytearray) -> None, note)

def _m_addralign_not_pow2(b):
    """sh_addralign = 0xffffffffff violates the ELF gABI (must be 0 or 2^n).

    The layout engine aligns the section address up to that value, demanding a
    ~1 TiB output buffer; the process then died with
    'memory allocation of 1099511627796 bytes failed' (SIGABRT), which
    catch_unwind cannot intercept.  wild rejects this input; bfd and mold
    silently accept it.  found_by_fuzzing
    """
    i = _find_section(b, ".data.msg") or 1
    _pu64(b, _shdr(b, i) + 48, 0xffffffffff)

def _m_section_offset_wraps(b):
    """sh_offset near u64::MAX makes the naive check `off + size <= len` wrap.

    The guard then passes and the slice panics with
    'range start index 18446744073692774483 out of range'.  found_by_fuzzing
    """
    i = _find_section(b, ".text._start") or 1
    _pu64(b, _shdr(b, i) + 24, (1 << 64) - 0x2000)
    _pu64(b, _shdr(b, i) + 32, 0x4000)

def _m_section_size_huge(b):
    """sh_size far beyond the file: must be a bounds error, not a huge read."""
    i = _find_section(b, ".text._start") or 1
    _pu64(b, _shdr(b, i) + 32, 0xffff_ffff_0000)

def _m_shoff_beyond_eof(b):
    """e_shoff points past the end of the file."""
    _pu64(b, 40, len(b) + 0x100000)

def _m_shentsize_zero(b):
    """e_shentsize = 0 makes every section header alias header 0."""
    _pu16(b, 58, 0)

def _m_shnum_huge(b):
    """e_shnum claims 65535 sections in a file that has ~18."""
    _pu16(b, 60, 0xffff)

def _m_shstrndx_oob(b):
    """e_shstrndx indexes a section that does not exist."""
    _pu16(b, 62, 0xfffe)

def _m_symtab_link_oob(b):
    """.symtab sh_link points at a non-existent string table."""
    i = _find_section(b, ".symtab")
    if i is not None:
        _pu32(b, _shdr(b, i) + 40, 0xfffe)

def _m_sym_name_oob(b):
    """A symbol's st_name indexes far outside .strtab."""
    i = _find_section(b, ".symtab")
    if i is not None:
        symoff = _u64(b, _shdr(b, i) + 24)
        _pu32(b, symoff + 24 * 2, 0xffff_fff0)   # symbol #2

def _m_sym_shndx_oob(b):
    """A symbol claims membership in a section index that does not exist."""
    i = _find_section(b, ".symtab")
    if i is not None:
        symoff = _u64(b, _shdr(b, i) + 24)
        _pu16(b, symoff + 24 * 2 + 6, 0xfff0)

def _m_reloc_sym_idx_oob(b):
    """r_info symbol index points past the end of .symtab."""
    i = _find_section(b, ".rela.text._start")
    if i is not None:
        ro = _u64(b, _shdr(b, i) + 24)
        _pu32(b, ro + 8 + 4, 0xffff)     # high half of r_info = sym index

def _m_reloc_offset_oob(b):
    """r_offset lies outside the section the relocation applies to."""
    i = _find_section(b, ".rela.text._start")
    if i is not None:
        ro = _u64(b, _shdr(b, i) + 24)
        _pu64(b, ro, 0xffff_ffff_0000)

def _m_reloc_type_unknown(b):
    """An x86-64 relocation type the linker does not implement."""
    i = _find_section(b, ".rela.text._start")
    if i is not None:
        ro = _u64(b, _shdr(b, i) + 24)
        _pu32(b, ro + 8, 250)

def _m_truncated_file(b):
    """The file ends in the middle of the section header table."""
    del b[len(b) // 2:]

def _m_rela_info_target_oob(b):
    """A SHT_RELA section's sh_info names a section that does not exist."""
    i = _find_section(b, ".rela.text._start")
    if i is not None:
        _pu32(b, _shdr(b, i) + 44, 0xfffe)

MALFORMED_CASES = [
    ("malformed_addralign_not_power_of_two", _m_addralign_not_pow2),
    ("malformed_section_offset_wraps_u64",   _m_section_offset_wraps),
    ("malformed_section_size_huge",          _m_section_size_huge),
    ("malformed_shoff_beyond_eof",           _m_shoff_beyond_eof),
    ("malformed_shentsize_zero",             _m_shentsize_zero),
    ("malformed_shnum_huge",                 _m_shnum_huge),
    ("malformed_shstrndx_out_of_range",      _m_shstrndx_oob),
    ("malformed_symtab_link_out_of_range",   _m_symtab_link_oob),
    ("malformed_symbol_name_out_of_range",   _m_sym_name_oob),
    ("malformed_symbol_shndx_out_of_range",  _m_sym_shndx_oob),
    ("malformed_reloc_sym_idx_out_of_range", _m_reloc_sym_idx_oob),
    ("malformed_reloc_offset_out_of_range",  _m_reloc_offset_oob),
    ("malformed_reloc_type_unknown",         _m_reloc_type_unknown),
    ("malformed_rela_info_target_oob",       _m_rela_info_target_oob),
    ("malformed_truncated_file",             _m_truncated_file),
]

def _robustness_tests(args, _oracles):
    """Link each mutated object with lccc-ld and assert controlled behaviour."""
    out = []
    td = tempfile.mkdtemp(prefix="lnk.robust.")
    ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    try:
        if not os.path.exists(ld):
            return [Result("robustness", "SKIP", f"{ld} not built")]
        with open(os.path.join(td, "r.c"), "w") as f:
            f.write(ROBUSTNESS_SRC)
        r = sh([CC, "-c", "-O1", "-ffunction-sections", "-fdata-sections",
                "r.c", "-o", "r.o"], cwd=td)
        if r.returncode != 0:
            return [Result("robustness", "SKIP",
                           f"fixture compile failed: {r.stderr.decode()[:200]}")]
        pristine = bytearray(open(os.path.join(td, "r.o"), "rb").read())

        # Sanity: the pristine fixture must link and run, otherwise the
        # mutations would be testing a failure that is not theirs.
        base = os.path.join(td, "base.out")
        r = sh([ld, "-o", base, "r.o"], cwd=td)
        if r.returncode != 0:
            return [Result("robustness", "SKIP",
                           f"pristine fixture does not link: {r.stderr.decode()[:200]}")]

        for name, mutate in MALFORMED_CASES:
            b = bytearray(pristine)
            try:
                mutate(b)
            except Exception as e:
                out.append(Result(name, "SKIP", f"mutator failed: {e!r}"))
                continue
            obj = os.path.join(td, f"{name}.o")
            with open(obj, "wb") as f:
                f.write(bytes(b))
            outp = os.path.join(td, f"{name}.out")
            try:
                r = sh([ld, "-o", outp, obj], cwd=td, timeout=25)
            except subprocess.TimeoutExpired:
                out.append(Result(name, "FAIL", "linker hung (>25s) on malformed input"))
                continue

            rc = r.returncode
            err = (r.stderr.decode(errors="replace") +
                   r.stdout.decode(errors="replace"))

            if rc < 0:
                out.append(Result(name, "FAIL",
                    f"killed by signal {-rc} (must be a clean diagnostic)"))
                continue
            if rc == 101:
                out.append(Result(name, "FAIL", f"rust panic (exit 101): {err[:300]}"))
                continue
            for marker in ("panicked at", "memory allocation of",
                           "index out of bounds", "unreachable",
                           "capacity overflow", "attempt to subtract with overflow"):
                if marker in err:
                    out.append(Result(name, "FAIL",
                        f"internal failure leaked ({marker!r}): {err[:300]}"))
                    break
            else:
                if rc == 0:
                    # Accepting the input is legal (bfd does for several of
                    # these) provided the result is a well-formed file.
                    if not os.path.exists(outp) or os.path.getsize(outp) == 0:
                        out.append(Result(name, "FAIL",
                            "reported success but produced no output"))
                    else:
                        out.append(Result(name, "PASS"))
                elif rc == 1:
                    if err.strip():
                        out.append(Result(name, "PASS"))
                    else:
                        out.append(Result(name, "FAIL",
                            "exit 1 with no diagnostic message"))
                else:
                    out.append(Result(name, "FAIL",
                        f"unexpected exit code {rc}: {err[:300]}"))
        return out
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)

# ============================================================================
# -Map= : link map file
# ============================================================================
#
# The value of a map file is that it is *authoritative*: every address it
# prints must be the address the linker actually emitted.  A map that is
# merely plausible is worse than none, because it silently misleads size
# accounting and post-mortem debugging.
#
# The test therefore does not compare text against GNU ld (whose layout
# differs legitimately).  It checks the two properties that matter:
#   1. every symbol address in the map equals that symbol's st_value in the
#      produced ELF (ground truth read back with readelf);
#   2. the structural GNU format is present, so existing scrapers parse it.

def _map_file_test(args, _oracles):
    td = tempfile.mkdtemp(prefix="lnk.mapfile.")
    ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    try:
        if not os.path.exists(ld):
            return Result("map_file_matches_binary", "SKIP", "lccc-ld not built")
        if not shutil.which("readelf"):
            return Result("map_file_matches_binary", "SKIP", "readelf not available")

        with open(os.path.join(td, "m.c"), "w") as f:
            f.write(ROBUSTNESS_SRC)
        with open(os.path.join(td, "extra.c"), "w") as f:
            f.write("int extra_a = 1; int extra_b = 2;\n"
                    "int extra_fn(int x){ return x + extra_a + extra_b; }\n")
        r = sh([CC, "-c", "-O1", "-ffunction-sections", "-fdata-sections",
                "m.c", "extra.c"], cwd=td)
        if r.returncode != 0:
            return Result("map_file_matches_binary", "SKIP",
                          f"fixture compile failed: {r.stderr.decode()[:200]}")

        out = os.path.join(td, "a.out")
        mp = os.path.join(td, "a.map")
        r = sh([ld, "-Map=" + mp, "-o", out, "m.o", "extra.o"], cwd=td)
        if r.returncode != 0:
            return Result("map_file_matches_binary", "FAIL",
                          f"link failed: {r.stderr.decode()[:300]}")
        if not os.path.exists(mp):
            return Result("map_file_matches_binary", "FAIL",
                          "-Map= produced no file")

        text = open(mp).read()
        for needed in ("Memory Configuration", "Linker script and memory map"):
            if needed not in text:
                return Result("map_file_matches_binary", "FAIL",
                              f"map lacks GNU section {needed!r}")

        # map: lines of the form "        0xADDR        name"
        map_syms = {}
        for line in text.splitlines():
            m = re.match(r"\s+0x([0-9a-f]{16})\s+(\S+)\s*$", line)
            if m:
                map_syms[m.group(2)] = int(m.group(1), 16)
        if not map_syms:
            return Result("map_file_matches_binary", "FAIL",
                          "map contains no symbol lines")

        # ground truth from the emitted binary
        rr = sh(["readelf", "-sW", out], cwd=td)
        elf_syms = {}
        for line in rr.stdout.decode(errors="replace").splitlines():
            parts = line.split()
            if len(parts) >= 8 and re.match(r"^\d+:$", parts[0]):
                try:
                    elf_syms[parts[7]] = int(parts[1], 16)
                except ValueError:
                    pass

        common = set(map_syms) & set(elf_syms)
        if not common:
            return Result("map_file_matches_binary", "FAIL",
                          "no symbols shared between map and binary")
        bad = [(k, hex(elf_syms[k]), hex(map_syms[k]))
               for k in common if elf_syms[k] != map_syms[k]]
        if bad:
            return Result("map_file_matches_binary", "FAIL",
                          f"{len(bad)}/{len(common)} map addresses disagree "
                          f"with the binary, e.g. {bad[:3]}")

        # The binary must also still work.
        code, sout = run_bin(out, [], td)
        if sout != "hi\n":
            return Result("map_file_matches_binary", "FAIL",
                          f"binary output {sout!r} != 'hi\\n'")
        return Result("map_file_matches_binary", "PASS")
    except Exception as e:
        return Result("map_file_matches_binary", "FAIL", f"harness exception: {e!r}")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


def _map_file_spellings_test(args, _oracles):
    """`-Map FILE` (two-arg) must behave exactly like `-Map=FILE`."""
    td = tempfile.mkdtemp(prefix="lnk.mapspell.")
    ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    try:
        if not os.path.exists(ld):
            return Result("map_file_two_arg_spelling", "SKIP", "lccc-ld not built")
        with open(os.path.join(td, "m.c"), "w") as f:
            f.write(ROBUSTNESS_SRC)
        r = sh([CC, "-c", "-O1", "m.c"], cwd=td)
        if r.returncode != 0:
            return Result("map_file_two_arg_spelling", "SKIP", "compile failed")
        r = sh([ld, "-Map", "two.map", "-o", "a.out", "m.o"], cwd=td)
        if r.returncode != 0:
            return Result("map_file_two_arg_spelling", "FAIL",
                          f"link failed: {r.stderr.decode()[:200]}")
        p = os.path.join(td, "two.map")
        if not os.path.exists(p) or os.path.getsize(p) == 0:
            return Result("map_file_two_arg_spelling", "FAIL",
                          "-Map FILE produced no map")
        return Result("map_file_two_arg_spelling", "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


# ============================================================================
# SEGMENT PACKING: congruence + no wasted pages
# ============================================================================
#
# A PT_LOAD segment's file offset does NOT have to be page-aligned; the ELF
# gABI only requires p_offset === p_vaddr (mod p_align), because mmap maps
# p_offset rounded down to a page onto p_vaddr rounded down to a page.
#
# Rounding the *file offset* up to a page at every segment boundary wastes up
# to one page per segment.  It cost this linker 12 288 bytes on a small
# zlib-ng binary (20 640 vs 8 352 after the fix; bfd 16 400, mold 9 808,
# wild 6 773).
#
# These tests lock in both halves of the property:
#   1. congruence holds for every PT_LOAD (else the loader maps garbage);
#   2. inter-segment file padding stays small (else the regression is back);
#   3. RELRO still ends on a page boundary in *address* space, so ld.so's
#      mprotect cannot spill into the following page.

def _parse_phdrs(binary):
    r = sh(["readelf", "-lW", binary])
    out = r.stdout.decode(errors="replace")
    seg = []
    for m in re.finditer(
            r"^\s+(LOAD|GNU_RELRO|DYNAMIC|PHDR|INTERP|TLS)\s+"
            r"0x([0-9a-f]+)\s+0x([0-9a-f]+)\s+0x[0-9a-f]+\s+"
            r"0x([0-9a-f]+)\s+0x([0-9a-f]+)\s+(\S+)\s+0x([0-9a-f]+)",
            out, re.M):
        t, off, va, fsz, msz, flags, align = m.groups()
        seg.append({"type": t, "off": int(off, 16), "vaddr": int(va, 16),
                    "filesz": int(fsz, 16), "memsz": int(msz, 16),
                    "flags": flags.strip(), "align": int(align, 16)})
    return seg


def _segment_packing_test(args, _oracles):
    td = tempfile.mkdtemp(prefix="lnk.segpack.")
    ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    out = []
    try:
        if not shutil.which("readelf"):
            return [Result("segment_packing", "SKIP", "readelf not available")]

        # A program with distinct RX / R / RW content, linked dynamically so
        # RELRO and .dynamic are present.
        with open(os.path.join(td, "s.c"), "w") as f:
            f.write("""
                #include <stdio.h>
                const char rodata_blob[4096] = "ro";
                char rw_blob[4096] = {1};
                static char bss_blob[8192];
                int helper(int x){ return x + rw_blob[0] + rodata_blob[0]; }
                int main(void){
                    bss_blob[0] = 3;
                    printf("%d\\n", helper(1) + bss_blob[0]);
                    return 0;
                }
            """)
        r = sh([CC, "-c", "-O1", "s.c"], cwd=td)
        if r.returncode != 0:
            return [Result("segment_packing", "SKIP", "fixture compile failed")]

        shim = os.path.join(td, "shim")
        os.makedirs(shim, exist_ok=True)
        os.symlink(os.path.abspath(ld), os.path.join(shim, "ld"))
        binp = os.path.join(td, "a.out")
        r = sh([CC, "-B" + shim, "s.o", "-o", binp,
                "-Wl,-z,relro", "-Wl,-z,now"], cwd=td)
        if r.returncode != 0 or not os.path.exists(binp):
            return [Result("segment_packing", "FAIL",
                           f"link failed: {r.stderr.decode()[:300]}")]

        # It must still run.
        code, sout = run_bin(binp, [], td)
        if code != 0:
            return [Result("segment_packing", "FAIL",
                           f"binary exited {code}: {sout!r}")]

        segs = _parse_phdrs(binp)
        loads = [s for s in segs if s["type"] == "LOAD"]
        if not loads:
            return [Result("segment_packing", "FAIL", "no PT_LOAD segments")]

        # (1) congruence
        bad = [s for s in loads
               if s["align"] > 1 and (s["off"] % s["align"]) != (s["vaddr"] % s["align"])]
        out.append(Result("segment_congruence",
                          "FAIL" if bad else "PASS",
                          "" if not bad else
                          f"p_offset !== p_vaddr (mod align) for {bad}"))

        # (2) no page-sized holes between consecutive LOADs
        loads_sorted = sorted(loads, key=lambda s: s["off"])
        worst = 0
        for a, b in zip(loads_sorted, loads_sorted[1:]):
            gap = b["off"] - (a["off"] + a["filesz"])
            worst = max(worst, gap)
        # Allow modest alignment padding, but never a whole page per segment.
        out.append(Result("segment_no_page_padding",
                          "PASS" if worst < 4096 else "FAIL",
                          f"largest inter-LOAD file gap = {worst} bytes"
                          + ("" if worst < 4096 else " (page padding regressed)")))

        # (3) RELRO ends on a page boundary in ADDRESS space
        relro = [s for s in segs if s["type"] == "GNU_RELRO"]
        if relro:
            r0 = relro[0]
            end = r0["vaddr"] + r0["memsz"]
            out.append(Result("relro_ends_on_page_boundary",
                              "PASS" if end % 4096 == 0 else "FAIL",
                              f"RELRO end vaddr = 0x{end:x}"))
        else:
            out.append(Result("relro_ends_on_page_boundary", "SKIP",
                              "no PT_GNU_RELRO"))

        # (4b) the same invariants for a SHARED LIBRARY, which goes through
        # emit_shared.rs — an independent layout implementation that had the
        # identical page-padding defect (19 568 B -> 7 280 B once fixed).
        with open(os.path.join(td, "lib.c"), "w") as f:
            f.write("int gv = 1; static int t[64] = {1};\n"
                    "int f1(int x){ return x + gv + t[x & 63]; }\n"
                    "int f2(int x){ return f1(x) * 2; }\n")
        rl = sh([CC, "-c", "-O2", "-fPIC", "lib.c", "-o", "lib.o"], cwd=td)
        if rl.returncode == 0:
            so = os.path.join(td, "liblx.so")
            rl = sh([CC, "-shared", "-B" + shim, "lib.o", "-o", so,
                     "-Wl,-z,relro"], cwd=td)
            if rl.returncode == 0 and os.path.exists(so):
                sosegs = _parse_phdrs(so)
                soloads = [x for x in sosegs if x["type"] == "LOAD"]
                sobad = [x for x in soloads
                         if x["align"] > 1
                         and (x["off"] % x["align"]) != (x["vaddr"] % x["align"])]
                out.append(Result("shared_lib_segment_congruence",
                                  "FAIL" if sobad else "PASS",
                                  "" if not sobad else f"non-congruent: {sobad}"))
                ss = sorted(soloads, key=lambda x: x["off"])
                sworst = 0
                for a, b in zip(ss, ss[1:]):
                    sworst = max(sworst, b["off"] - (a["off"] + a["filesz"]))
                out.append(Result("shared_lib_no_page_padding",
                                  "PASS" if sworst < 4096 else "FAIL",
                                  f"largest inter-LOAD gap = {sworst} bytes"))
                # It must still be loadable and correct.
                with open(os.path.join(td, "use.c"), "w") as f:
                    f.write("#include <stdio.h>\nextern int f2(int);\n"
                            "int main(void){ printf(\"%d\\n\", f2(5)); return 0; }\n")
                ru = sh([CC, "use.c", so, "-Wl,-rpath," + td, "-o", "useso"], cwd=td)
                if ru.returncode == 0:
                    code, sout = run_bin(os.path.join(td, "useso"), [], td)
                    out.append(Result("shared_lib_still_loads",
                                      "PASS" if sout == "12\n" else "FAIL",
                                      f"output {sout!r} (expected '12')"))

        # (4) not larger than GNU ld on the same input
        bfd_bin = os.path.join(td, "a.bfd")
        rb = sh([CC, "-fuse-ld=bfd", "s.o", "-o", bfd_bin,
                 "-Wl,-z,relro", "-Wl,-z,now"], cwd=td)
        if rb.returncode == 0 and os.path.exists(bfd_bin):
            ls, bs = os.path.getsize(binp), os.path.getsize(bfd_bin)
            out.append(Result("output_not_larger_than_bfd",
                              "PASS" if ls <= bs * 1.05 else "FAIL",
                              f"lccc {ls} B vs bfd {bs} B"))
        return out
    except Exception as e:
        return [Result("segment_packing", "FAIL", f"harness exception: {e!r}")]
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


# ============================================================================
# DRIVER OPTION HANDLING: silence on benign flags, precision on dangerous ones
# ============================================================================
#
# `gcc -fuse-ld=<linker>` passes a fixed set of driver artefacts on every
# single link: the LTO plugin triplet (-plugin, -plugin-opt=...) and
# --push-state/--pop-state around --as-needed groups.  lccc-ld used to print
# "warning: ignoring unknown option" for each of them -- twelve lines of noise
# per invocation -- which trains users to ignore lccc's output entirely and
# buries the diagnostics that do matter.
#
# The two halves of correct behaviour are tested separately, because they pull
# in opposite directions:
#
#   * benign driver flags  -> accept SILENTLY (bfd and mold do)
#   * LTO bytecode input   -> REFUSE LOUDLY with an actionable message
#
# The second is the interesting one.  bfd/mold/wild "succeed" on LTO input
# only because they load the plugin that turns IR back into machine code.
# lccc has no plugin support, so accepting the file would mean emitting a
# binary with code silently missing.  Refusing is the correct answer, and the
# message must name the file and the fix.

def _driver_option_noise_test(args, _oracles):
    td = tempfile.mkdtemp(prefix="lnk.optnoise.")
    ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    out = []
    try:
        if not os.path.exists(ld):
            return [Result("driver_option_noise", "SKIP", "lccc-ld not built")]
        with open(os.path.join(td, "n.c"), "w") as f:
            f.write(ROBUSTNESS_SRC)
        r = sh([CC, "-c", "-O1", "n.c"], cwd=td)
        if r.returncode != 0:
            return [Result("driver_option_noise", "SKIP", "compile failed")]

        shim = os.path.join(td, "shim")
        os.makedirs(shim, exist_ok=True)
        os.symlink(os.path.abspath(ld), os.path.join(shim, "ld"))

        # --- half 1: a normal gcc-driven link must be silent -----------------
        r = sh([CC, "-B" + shim, "-nostdlib", "-static", "n.o",
                "-o", os.path.join(td, "a.out")], cwd=td)
        noise = [ln for ln in r.stderr.decode(errors="replace").splitlines()
                 if "ignoring unknown option" in ln]
        if r.returncode != 0:
            out.append(Result("driver_option_noise", "FAIL",
                              f"link failed: {r.stderr.decode()[:250]}"))
        elif noise:
            out.append(Result("driver_option_noise", "FAIL",
                              f"{len(noise)} spurious warning(s), e.g. {noise[0]!r}"))
        else:
            out.append(Result("driver_option_noise", "PASS"))

        # Explicitly check the individual flags too, so a future refactor that
        # drops one of them from the allow-list is caught by name.
        for flag in ("--push-state", "--pop-state", "--eh-frame-hdr",
                     "--no-warn-execstack", "-plugin-opt=whatever"):
            rr = sh([ld, flag, "-o", os.path.join(td, "b.out"), "n.o"], cwd=td)
            msg = rr.stderr.decode(errors="replace")
            if "ignoring unknown option" in msg:
                out.append(Result(f"driver_flag_silent[{flag}]", "FAIL",
                                  "still warns"))
            else:
                out.append(Result(f"driver_flag_silent[{flag}]", "PASS"))

        # --- half 2: LTO bytecode must be refused with a useful message ------
        rl = sh([CC, "-c", "-O1", "-flto", "n.c", "-o", "nlto.o"], cwd=td)
        if rl.returncode != 0:
            out.append(Result("lto_bytecode_refused", "SKIP",
                              "compiler cannot produce -flto objects"))
            return out
        rr = sh([ld, "-plugin", "/nonexistent/liblto_plugin.so",
                 "-o", os.path.join(td, "c.out"), "nlto.o"], cwd=td)
        msg = (rr.stderr.decode(errors="replace") +
               rr.stdout.decode(errors="replace"))
        if rr.returncode == 0:
            out.append(Result("lto_bytecode_refused", "FAIL",
                              "linked LTO bytecode without a plugin — the "
                              "output would be missing code"))
        elif "LTO" not in msg or "nlto.o" not in msg:
            out.append(Result("lto_bytecode_refused", "FAIL",
                              f"rejected, but message is not actionable: {msg[:200]!r}"))
        else:
            out.append(Result("lto_bytecode_refused", "PASS"))
        return out
    except Exception as e:
        return [Result("driver_option_noise", "FAIL", f"harness exception: {e!r}")]
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


# ============================================================================
# --exclude-libs
# ============================================================================
#
# `--exclude-libs=ARCHIVE` links an archive's members in but keeps their
# symbols out of .dynsym.  This is how a shared library statically absorbs a
# helper archive (OpenSSL's libcrypto.a inside a plugin .so is the canonical
# case) without leaking that archive's whole symbol table into its ABI, where
# it would collide with a different copy loaded elsewhere in the process.
#
# The test checks three things, and the third is the one that actually bit:
#
#   1. without the flag the helper symbols ARE exported (control);
#   2. with the flag they are NOT, while the real API still is;
#   3. the library still LOADS AND RUNS.
#
# (3) matters because the first implementation passed (1) and (2) and still
# produced a broken .so: the excluded symbol kept its PLT entry and JUMP_SLOT
# relocation, but the .dynsym entry that relocation referenced was gone, so it
# degenerated to symbol index 0 and the loader died with
# `symbol lookup error: ...: undefined symbol: ` (empty name).  Checking only
# the symbol table would have shipped that.

def _exclude_libs_test(args, _oracles):
    td = tempfile.mkdtemp(prefix="lnk.exclibs.")
    ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    out = []
    try:
        if not os.path.exists(ld):
            return [Result("exclude_libs", "SKIP", "lccc-ld not built")]
        if not shutil.which("readelf") or not shutil.which("ar"):
            return [Result("exclude_libs", "SKIP", "readelf/ar not available")]

        with open(os.path.join(td, "h.c"), "w") as f:
            f.write("int helper_internal(int x){ return x * 7; }\n"
                    "int helper_other(int x){ return x + 1; }\n")
        with open(os.path.join(td, "api.c"), "w") as f:
            f.write("extern int helper_internal(int);\n"
                    "int public_api(int x){ return helper_internal(x) + 1; }\n")
        with open(os.path.join(td, "u.c"), "w") as f:
            f.write("#include <stdio.h>\nextern int public_api(int);\n"
                    "int main(void){ printf(\"%d\\n\", public_api(6)); return 0; }\n")
        r = sh([CC, "-c", "-O2", "-fPIC", "h.c", "api.c"], cwd=td)
        if r.returncode != 0:
            return [Result("exclude_libs", "SKIP", "fixture compile failed")]
        r = sh(["ar", "rcs", "libhelper.a", "h.o"], cwd=td)
        if r.returncode != 0:
            return [Result("exclude_libs", "SKIP", "ar failed")]

        shim = os.path.join(td, "shim")
        os.makedirs(shim, exist_ok=True)
        os.symlink(os.path.abspath(ld), os.path.join(shim, "ld"))

        def build(name, extra):
            so = os.path.join(td, name)
            rr = sh([CC, "-shared", "-B" + shim, "api.o", "libhelper.a"]
                    + extra + ["-o", so], cwd=td)
            return (rr, so)

        def dyn_names(so):
            rr = sh(["readelf", "--dyn-syms", "-W", so], cwd=td)
            names = set()
            for line in rr.stdout.decode(errors="replace").splitlines():
                parts = line.split()
                if len(parts) >= 8 and re.match(r"^\d+:$", parts[0]):
                    names.add(parts[7].split("@")[0])
            return names

        # (1) control: helper symbols exported without the flag
        rr, so_without = build("lib_without.so", [])
        if rr.returncode != 0:
            return [Result("exclude_libs", "FAIL",
                           f"control link failed: {rr.stderr.decode()[:250]}")]
        n_without = dyn_names(so_without)
        if "helper_internal" not in n_without:
            out.append(Result("exclude_libs_control", "SKIP",
                              "helper not exported even without the flag"))
        else:
            out.append(Result("exclude_libs_control", "PASS"))

        # (2) with the flag: helpers hidden, API still exported
        rr, so_with = build("lib_with.so", ["-Wl,--exclude-libs=libhelper.a"])
        if rr.returncode != 0:
            return out + [Result("exclude_libs_hides_symbols", "FAIL",
                                 f"link failed: {rr.stderr.decode()[:250]}")]
        n_with = dyn_names(so_with)
        leaked = {"helper_internal", "helper_other"} & n_with
        if leaked:
            out.append(Result("exclude_libs_hides_symbols", "FAIL",
                              f"still exported: {sorted(leaked)}"))
        elif "public_api" not in n_with:
            out.append(Result("exclude_libs_hides_symbols", "FAIL",
                              "public_api was hidden too — over-broad exclusion"))
        else:
            out.append(Result("exclude_libs_hides_symbols", "PASS"))

        # (3) the .so must still load and produce the right answer
        rr = sh([CC, "u.c", so_with, "-Wl,-rpath," + td, "-o", "useit"], cwd=td)
        if rr.returncode != 0:
            out.append(Result("exclude_libs_lib_still_works", "FAIL",
                              f"consumer link failed: {rr.stderr.decode()[:250]}"))
        else:
            code, sout = run_bin(os.path.join(td, "useit"), [], td)
            if sout != "43\n":
                out.append(Result("exclude_libs_lib_still_works", "FAIL",
                                  f"got {sout!r} (exit {code}), expected '43' — "
                                  "excluded symbols likely left a dangling "
                                  "PLT/JUMP_SLOT at symbol index 0"))
            else:
                out.append(Result("exclude_libs_lib_still_works", "PASS"))

        # (3b) no relocation may reference the NULL symbol (index 0).
        #
        # readelf -r prints "<offset> <info> <type> <value> <name> + <addend>".
        # The symbol index is the HIGH 32 bits of r_info, not the value column:
        # an undefined-but-named symbol such as __cxa_finalize legitimately has
        # value 0, so keying on the value column produces a false positive.
        rr = sh(["readelf", "-rW", so_with], cwd=td)
        dangling = []
        for ln in rr.stdout.decode(errors="replace").splitlines():
            m = re.match(r"^[0-9a-f]{16}\s+([0-9a-f]{16})\s+(\S+)", ln)
            if not m:
                continue
            sym_idx = int(m.group(1), 16) >> 32
            if sym_idx == 0 and "JUMP_SLOT" in m.group(2):
                dangling.append(ln)
        out.append(Result("exclude_libs_no_null_jumpslot",
                          "FAIL" if dangling else "PASS",
                          "" if not dangling else
                          f"JUMP_SLOT against symbol index 0: {dangling[0][:110]}"))

        # (4) ALL keyword
        rr, so_all = build("lib_all.so", ["-Wl,--exclude-libs=ALL"])
        if rr.returncode == 0:
            n_all = dyn_names(so_all)
            leaked = {"helper_internal", "helper_other"} & n_all
            out.append(Result("exclude_libs_ALL",
                              "FAIL" if leaked else "PASS",
                              "" if not leaked else f"still exported: {sorted(leaked)}"))
        else:
            out.append(Result("exclude_libs_ALL", "FAIL",
                              f"link failed: {rr.stderr.decode()[:200]}"))
        return out
    except Exception as e:
        return [Result("exclude_libs", "FAIL", f"harness exception: {e!r}")]
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


# ============================================================================
# --emit-relocs  (Linux kernel: CONFIG_RELOCATABLE / KASLR)
# ============================================================================
#
# `--emit-relocs` keeps a record of every relocation the linker applied, as
# .rela.<section> sections in the *linked* image.  The Linux kernel's
# arch/x86/tools/relocs pass reads them out of vmlinux and turns them into the
# table the boot code walks to slide the kernel to a random base
# (CONFIG_RELOCATABLE, CONFIG_RANDOMIZE_BASE).
#
# Before this was implemented lccc-ld *accepted the flag and ignored it*.  The
# link reported success and produced an image with zero .rela sections, so the
# kernel build would complete and then fail to boot -- the worst possible
# failure mode, and the reason this ranks above cosmetic feature gaps.
#
# What is checked, in increasing order of strength:
#   1. the sections exist at all, with correct sh_link/sh_info wiring;
#   2. the *set* of relocations equals GNU ld's, compared SECTION-RELATIVE
#      (absolute addresses legitimately differ: bfd and lccc are free to lay
#      the image out differently, and they do);
#   3. type, target symbol and addend agree entry-for-entry;
#   4. the flag does not perturb the image otherwise -- the same link without
#      --emit-relocs must produce identical section contents.

EMIT_RELOCS_SRC = r"""
int gvar = 42;
int *gptr = &gvar;                 /* R_X86_64_64 against data      */
int helper(int x){ return x + gvar; }
int (*fptr)(int) = helper;         /* R_X86_64_64 against a function */
static const int table[4] = {1,2,3,4};
const int *tptr = table;           /* R_X86_64_64 against a local    */
int use(int i){ return table[i & 3] + helper(i); }
void _start(void){ }
"""

EMIT_RELOCS_LDS = """
ENTRY(_start)
SECTIONS {
  . = 0xffffffff81000000;
  .text : { *(.text .text.*) }
  .rodata : { *(.rodata .rodata.*) }
  .data : { *(.data .data.*) }
  .bss  : { *(.bss) *(COMMON) }
}
"""


def _read_sections(binary, td):
    """name -> (index, addr) for every section header."""
    r = sh(["readelf", "-SW", binary], cwd=td)
    out = {}
    for m in re.finditer(
            r"\[\s*(\d+)\]\s+(\S+)\s+(\S+)\s+([0-9a-f]+)\s+([0-9a-f]+)\s+([0-9a-f]+)",
            r.stdout.decode(errors="replace")):
        idx, name, _t, addr, _off, _size = m.groups()
        out[name] = (int(idx), int(addr, 16))
    return out


def _read_relocs_relative(binary, td):
    """{(section, offset_within_section): (type, symbol, addend)}.

    Section-relative so two linkers that chose different base addresses can
    still be compared.  Section-symbol references print with an empty name in
    readelf, which is normalised away.
    """
    secs = _read_sections(binary, td)
    r = sh(["readelf", "-rW", binary], cwd=td)
    cur, out = None, {}
    for line in r.stdout.decode(errors="replace").splitlines():
        m = re.match(r"Relocation section '(\S+)'", line)
        if m:
            cur = m.group(1).replace(".rela", "", 1)
            continue
        m = re.match(r"^([0-9a-f]{16})\s+([0-9a-f]{16})\s+(\S+)"
                     r"(?:\s+[0-9a-f]{16})?\s*(.*)$", line)
        if m and cur is not None:
            off, _info, rtype, rest = m.groups()
            base = secs.get(cur, (0, 0))[1]
            rest = (rest or "").strip()
            # Drop a leading section-symbol name so ".text + 9" and "+ 9"
            # compare equal; the addend is what carries the information.
            rest = re.sub(r"^\.[\w.]+", "", rest).replace(" ", "")
            out[(cur, int(off, 16) - base)] = (rtype, rest)
    return out


def _emit_relocs_test(args, _oracles):
    td = tempfile.mkdtemp(prefix="lnk.emitrelocs.")
    ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    out = []
    try:
        if not os.path.exists(ld):
            return [Result("emit_relocs", "SKIP", "lccc-ld not built")]
        if not shutil.which("readelf"):
            return [Result("emit_relocs", "SKIP", "readelf not available")]

        with open(os.path.join(td, "k.c"), "w") as f:
            f.write(EMIT_RELOCS_SRC)
        with open(os.path.join(td, "k.lds"), "w") as f:
            f.write(EMIT_RELOCS_LDS)
        r = sh([CC, "-c", "-O1", "-fno-pic", "-fno-stack-protector",
                "k.c", "-o", "k.o"], cwd=td)
        if r.returncode != 0:
            return [Result("emit_relocs", "SKIP",
                           f"fixture compile failed: {r.stderr.decode()[:200]}")]

        # lccc, with and without the flag
        r = sh([ld, "--emit-relocs", "-T", "k.lds", "k.o", "-o", "k.lccc"], cwd=td)
        if r.returncode != 0:
            return [Result("emit_relocs", "FAIL",
                           f"lccc link failed: {r.stderr.decode()[:300]}")]
        r = sh([ld, "-T", "k.lds", "k.o", "-o", "k.plain"], cwd=td)
        if r.returncode != 0:
            return [Result("emit_relocs", "FAIL",
                           f"lccc plain link failed: {r.stderr.decode()[:300]}")]

        lccc_rel = _read_relocs_relative(os.path.join(td, "k.lccc"), td)

        # (1) the sections must exist and be non-empty
        if not lccc_rel:
            return [Result("emit_relocs_sections_present", "FAIL",
                           "--emit-relocs produced no .rela sections at all "
                           "(a KASLR kernel built this way would not boot)")]
        out.append(Result("emit_relocs_sections_present", "PASS",
                          f"{len(lccc_rel)} relocations retained"))

        # (1b) sh_link must point at .symtab and sh_info at the target section
        rs = sh(["readelf", "-SW", os.path.join(td, "k.lccc")], cwd=td)
        text = rs.stdout.decode(errors="replace")
        secs = _read_sections(os.path.join(td, "k.lccc"), td)
        symtab_idx = secs.get(".symtab", (None, 0))[0]
        bad_link = []
        # readelf -SW column layout:
        #   [Nr] Name Type Address Off Size ES Flg Lk Inf Al
        # `Flg` is EMPTY for SHT_RELA sections, so a positional regex that
        # assumes it is present silently reads Lk/Inf one column early and
        # reports a false mismatch. Anchor on the trailing fields instead:
        # the last three whitespace-separated tokens are always Lk, Inf, Al.
        for line in text.splitlines():
            m = re.match(r"\s*\[\s*(\d+)\]\s+(\.rela\S*)\s+RELA\s+(.*)$", line)
            if not m:
                continue
            _i, name, tail = m.groups()
            fields = tail.split()
            if len(fields) < 3:
                continue
            link, info = fields[-3], fields[-2]
            target = name.replace(".rela", "", 1)
            if symtab_idx is not None and int(link) != symtab_idx:
                bad_link.append(f"{name}: sh_link={link} != .symtab({symtab_idx})")
            want_info = secs.get(target, (None, 0))[0]
            if want_info is not None and int(info) != want_info:
                bad_link.append(f"{name}: sh_info={info} != {target}({want_info})")
        out.append(Result("emit_relocs_header_wiring",
                          "FAIL" if bad_link else "PASS",
                          "; ".join(bad_link[:3])))

        # (2)+(3) differential against GNU ld
        bfd = shutil.which("ld.bfd") or shutil.which("ld")
        if bfd:
            rb = sh([bfd, "--emit-relocs", "-T", "k.lds", "k.o", "-o", "k.bfd"], cwd=td)
            if rb.returncode == 0:
                bfd_rel = _read_relocs_relative(os.path.join(td, "k.bfd"), td)
                keys = set(bfd_rel) | set(lccc_rel)
                disagree = [(k, bfd_rel.get(k), lccc_rel.get(k))
                            for k in sorted(keys)
                            if bfd_rel.get(k) != lccc_rel.get(k)]
                if disagree:
                    out.append(Result("emit_relocs_matches_gnu_ld", "FAIL",
                        f"{len(disagree)}/{len(keys)} differ, e.g. "
                        f"{disagree[0]}"))
                else:
                    out.append(Result("emit_relocs_matches_gnu_ld", "PASS",
                        f"all {len(keys)} relocations agree with GNU ld"))
            else:
                out.append(Result("emit_relocs_matches_gnu_ld", "SKIP",
                                  "GNU ld rejected the script"))

        # (3b) STRONGEST CHECK: run the *actual* Linux kernel relocs tool.
        #
        # arch/x86/tools/relocs is the real consumer of --emit-relocs. If it
        # accepts lccc's image and derives the same relocation set it derives
        # from GNU ld's, the KASLR boot path will work. Nothing short of this
        # proves the feature; the tool is fetched by
        # tests/linker/setup_kernel_tools.sh and skipped if unavailable.
        relocs_tool = os.environ.get("LCCC_RELOCS_TOOL",
                                     "/home/user/tools/bin/relocs")
        if os.path.exists(relocs_tool) and bfd:
            def reloc_targets(binary):
                """Relocation targets as SECTION-RELATIVE strings.

                Absolute addresses legitimately differ between linkers (bfd
                pads sections differently), so comparing raw addresses would
                report a false mismatch. What must agree is *which bytes* need
                relocating.
                """
                secs = _read_sections(binary, td)
                rr = sh([relocs_tool, "--text", binary], cwd=td)
                if rr.returncode != 0:
                    return None
                vals = [int(x, 16) for x in re.findall(
                    r"\.long (0x[0-9a-f]+)", rr.stdout.decode(errors="replace"))]
                out_t = []
                for v in vals:
                    if not v:
                        continue
                    full = 0xffffffff00000000 | v
                    hit = None
                    for n, (_i, a) in secs.items():
                        # size is not in _read_sections; use the next section
                        # start implicitly by picking the closest lower base.
                        if a and a <= full and (hit is None or a > hit[1]):
                            hit = (n, a)
                    out_t.append(f"{hit[0]}+0x{full - hit[1]:x}" if hit else hex(v))
                return out_t

            t_lccc = reloc_targets(os.path.join(td, "k.lccc"))
            t_bfd = reloc_targets(os.path.join(td, "k.bfd"))
            if t_lccc is None:
                out.append(Result("emit_relocs_kernel_relocs_tool", "FAIL",
                                  "kernel relocs tool REJECTED lccc's image"))
            elif t_bfd is None:
                out.append(Result("emit_relocs_kernel_relocs_tool", "SKIP",
                                  "relocs tool rejected the bfd reference too"))
            elif t_lccc != t_bfd:
                out.append(Result("emit_relocs_kernel_relocs_tool", "FAIL",
                                  f"relocation set differs from GNU ld's:\n"
                                  f"      bfd  = {t_bfd}\n      lccc = {t_lccc}"))
            else:
                out.append(Result("emit_relocs_kernel_relocs_tool", "PASS",
                                  f"kernel relocs tool derives an identical "
                                  f"set ({len(t_lccc)} entries) from lccc and GNU ld"))

        # (4) the flag must not change the image itself
        def alloc_contents(binary):
            got = {}
            for name in (".text", ".rodata", ".data"):
                rr = sh(["objcopy", "-O", "binary", "--only-section", name,
                         binary, f"{binary}{name}.bin"], cwd=td)
                p = os.path.join(td, f"{binary}{name}.bin")
                if rr.returncode == 0 and os.path.exists(p):
                    got[name] = open(p, "rb").read()
            return got
        if shutil.which("objcopy"):
            a = alloc_contents(os.path.join(td, "k.lccc"))
            b = alloc_contents(os.path.join(td, "k.plain"))
            changed = [n for n in a if a.get(n) != b.get(n)]
            out.append(Result("emit_relocs_does_not_perturb_image",
                              "FAIL" if changed else "PASS",
                              f"sections differ: {changed}" if changed else ""))
        return out
    except Exception as e:
        return [Result("emit_relocs", "FAIL", f"harness exception: {e!r}")]
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


# ============================================================================
# -rdynamic / --export-dynamic on EXECUTABLES  (+ --version-script)
# ============================================================================
#
# `gcc -rdynamic` must put the executable's own global symbols into .dynsym so
# that a dlopen'd plugin can call back into the host, and so backtrace_symbols
# can name frames.  This is how every plugin host works (and how the kernel's
# own userspace tooling, perf/bpftool style, resolves callbacks).
#
# The bug this pins down: gcc's driver spells the flag with a SINGLE dash
# (`gcc -rdynamic` -> `collect2 ... -export-dynamic`), while lccc-ld only
# matched the double-dash `--export-dynamic`.  The flag therefore fell through
# to the unknown-option arm and was dropped, so `gcc -rdynamic` produced an
# executable that exported nothing.
#
# It survived because `lccc-ld --export-dynamic ...` invoked DIRECTLY worked
# perfectly — so any test that drove the linker directly passed.  The test
# below deliberately goes through `gcc`, which is how real builds invoke it,
# and finishes with an end-to-end dlopen round trip rather than a symbol-table
# inspection.

RDYNAMIC_HOST = r"""
#include <stdio.h>
#include <dlfcn.h>
int host_callback(int x){ return x * 10; }
int main(void){
    void *h = dlopen("./plug.so", RTLD_NOW);
    if(!h){ printf("dlopen failed: %s\n", dlerror()); return 1; }
    int (*run)(int) = (int(*)(int))dlsym(h, "plug_run");
    if(!run){ printf("dlsym failed\n"); return 1; }
    printf("%d\n", run(4));
    return 0;
}
"""

RDYNAMIC_PLUG = r"""
extern int host_callback(int);
int plug_run(int x){ return host_callback(x) + 1; }
"""

RDYNAMIC_LIB = r"""
#include <stdio.h>
int exported_api(int x){ return x + 1; }
int internal_helper(int x){ return x * 2; }
int main(void){ printf("%d\n", exported_api(1) + internal_helper(2)); return 0; }
"""


def _dynsym_names(binary, td):
    r = sh(["readelf", "--dyn-syms", "-W", binary], cwd=td)
    names = set()
    for line in r.stdout.decode(errors="replace").splitlines():
        p = line.split()
        if len(p) >= 8 and re.match(r"^\d+:$", p[0]):
            names.add(p[7].split("@")[0])
    return names


def _export_dynamic_test(args, _oracles):
    td = tempfile.mkdtemp(prefix="lnk.rdynamic.")
    ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    out = []
    try:
        if not os.path.exists(ld):
            return [Result("export_dynamic", "SKIP", "lccc-ld not built")]
        if not shutil.which("readelf"):
            return [Result("export_dynamic", "SKIP", "readelf not available")]

        shim = os.path.join(td, "shim")
        os.makedirs(shim, exist_ok=True)
        os.symlink(os.path.abspath(ld), os.path.join(shim, "ld"))

        for fn, src in (("host.c", RDYNAMIC_HOST), ("plug.c", RDYNAMIC_PLUG),
                        ("lib.c", RDYNAMIC_LIB)):
            with open(os.path.join(td, fn), "w") as f:
                f.write(src)
        with open(os.path.join(td, "v.map"), "w") as f:
            f.write("{ global: exported_api; main; local: *; };\n")

        r = sh([CC, "-shared", "-fPIC", "plug.c", "-o", "plug.so"], cwd=td)
        if r.returncode != 0:
            return [Result("export_dynamic", "SKIP", "plugin build failed")]

        # --- 1. symbol-table check, driven through gcc (not lccc-ld directly)
        r = sh([CC, "-Bshim", "-rdynamic", "lib.c", "-o", "app"], cwd=td)
        if r.returncode != 0:
            return [Result("export_dynamic_exports_globals", "FAIL",
                           f"link failed: {r.stderr.decode()[:250]}")]
        names = _dynsym_names(os.path.join(td, "app"), td)
        missing = {"exported_api", "main"} - names
        if missing:
            out.append(Result("export_dynamic_exports_globals", "FAIL",
                f"gcc -rdynamic did not export {sorted(missing)}; "
                f"gcc spells the flag '-export-dynamic' (single dash) — "
                f"is that spelling handled?"))
        else:
            out.append(Result("export_dynamic_exports_globals", "PASS",
                              f"{len(names)} dynamic symbols"))

        # --- 2. end-to-end: dlopen'd plugin calls back into the host
        r = sh([CC, "-Bshim", "-rdynamic", "host.c", "-ldl", "-o", "host"], cwd=td)
        if r.returncode != 0:
            out.append(Result("export_dynamic_dlopen_callback", "FAIL",
                              f"host link failed: {r.stderr.decode()[:250]}"))
        else:
            code, sout = run_bin(os.path.join(td, "host"), [], td)
            if sout.strip() != "41":
                out.append(Result("export_dynamic_dlopen_callback", "FAIL",
                    f"plugin could not call back into the host: "
                    f"got {sout!r} (exit {code}), expected '41'"))
            else:
                out.append(Result("export_dynamic_dlopen_callback", "PASS"))

        # --- 3. --version-script must still narrow the export set
        r = sh([CC, "-Bshim", "-rdynamic", "lib.c",
                "-Wl,--version-script=v.map", "-o", "app_vs"], cwd=td)
        if r.returncode != 0:
            out.append(Result("export_dynamic_version_script", "FAIL",
                              f"link failed: {r.stderr.decode()[:250]}"))
        else:
            vnames = _dynsym_names(os.path.join(td, "app_vs"), td)
            code, sout = run_bin(os.path.join(td, "app_vs"), [], td)
            if "internal_helper" in vnames:
                out.append(Result("export_dynamic_version_script", "FAIL",
                    "version script 'local: *' did not hide internal_helper"))
            elif "exported_api" not in vnames:
                out.append(Result("export_dynamic_version_script", "FAIL",
                    "version script hid exported_api, which is in 'global:'"))
            elif sout.strip() != "6":
                out.append(Result("export_dynamic_version_script", "FAIL",
                    f"binary misbehaved: {sout!r}"))
            else:
                out.append(Result("export_dynamic_version_script", "PASS"))

        # --- 4. differential: same export decisions as GNU ld
        bfd = shutil.which("ld.bfd")
        if bfd:
            r = sh([CC, "-fuse-ld=bfd", "-rdynamic", "lib.c", "-o", "app_bfd"], cwd=td)
            if r.returncode == 0:
                bnames = _dynsym_names(os.path.join(td, "app_bfd"), td)
                # Compare only the user's own symbols; CRT/linker-defined
                # symbols legitimately differ between linkers.
                user = {"exported_api", "internal_helper", "main"}
                if (bnames & user) != (names & user):
                    out.append(Result("export_dynamic_matches_gnu_ld", "FAIL",
                        f"bfd exports {sorted(bnames & user)}, "
                        f"lccc exports {sorted(names & user)}"))
                else:
                    out.append(Result("export_dynamic_matches_gnu_ld", "PASS",
                        f"both export {sorted(bnames & user)}"))
        return out
    except Exception as e:
        return [Result("export_dynamic", "FAIL", f"harness exception: {e!r}")]
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


# ============================================================================
# Runner
# ============================================================================

class Result:
    def __init__(self, name, status, detail=""):
        self.name, self.status, self.detail = name, status, detail

def compile_sources(td, c, flags):
    objs = []
    for fname, content in c.sources.items():
        path = os.path.join(td, fname)
        with open(path, "w") as f:
            f.write(textwrap.dedent(content))
        obj = os.path.splitext(fname)[0] + ".o"
        r = sh([CC, "-c", fname, "-o", obj] + flags, cwd=td)
        if r.returncode != 0:
            return None, f"fixture compile failed: {r.stderr.decode()}"
        objs.append(obj)
    return objs, None

def expand_ldflags(flags, td):
    return [f.replace("$ORIGIN", "'$ORIGIN'") if False else f for f in flags]

def link_with(linker_cmd, inputs, out, ldflags, td):
    cmd = list(linker_cmd) + inputs + ["-o", out] + ldflags
    return sh(cmd, cwd=td)

def run_bin(path, args, td, env=None):
    e = {"LC_ALL": "C"}
    if env:
        e.update(env)
    try:
        r = sh([path] + args, cwd=td, timeout=30, env=e)
        return r.returncode, r.stdout.decode(errors="replace")
    except subprocess.TimeoutExpired:
        return None, "<timeout>"

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc", default=DEFAULT_LCCC)
    ap.add_argument("--filter", default="")
    ap.add_argument("--tag", default="")
    ap.add_argument("-v", "--verbose", action="store_true")
    ap.add_argument("--keep", action="store_true", help="keep temp dirs")
    args = ap.parse_args()

    have_mold = shutil.which("mold") is not None
    have_wild = shutil.which("wild") is not None

    oracles = [("bfd", [CC, "-fuse-ld=bfd"])]
    if have_mold:
        oracles.append(("mold", [CC, "-fuse-ld=mold"]))
    if have_wild:
        wildpath = shutil.which("wild")
        oracles.append(("wild", [CC, f"-B{os.path.dirname(_wild_shim(wildpath))}"]))

    results = []
    for c in CASES:
        if args.filter and args.filter not in c.name:
            continue
        if args.tag and args.tag not in c.tags:
            continue
        results.append(run_case(c, args, oracles))

    for (name, script, csrc, expect) in SCRIPT_TESTS:
        if args.filter and args.filter not in name:
            continue
        if args.tag and args.tag != "script":
            if args.tag:
                continue
        results.append(_script_test(name, script, csrc, expect)(args, oracles))

    for entry in REL_TESTS:
        name, sources, expect = entry[0], entry[1], entry[2]
        cflags = entry[3] if len(entry) > 3 else None
        if args.filter and args.filter not in name:
            continue
        if args.tag and args.tag != "rel":
            if args.tag:
                continue
        results.append(_rel_test(name, sources, expect,
                                 compile_flags=cflags)(args, oracles))

    if (not args.filter or "cxx" in args.filter) and (not args.tag or args.tag == "ehframe"):
        results.append(_cxx_eh_test(args, oracles))
        results.append(_cxx_eh_local_type_test(args, oracles))

    if (not args.filter or "pie" in args.filter) and (not args.tag or args.tag == "script"):
        results.append(_pie_script_test(args, oracles))

    if (not args.filter or "so_" in args.filter) and not args.tag:
        results.append(_interpose_test(args, oracles,
            "so_default_interposable", [], "101\n"))
        results.append(_interpose_test(args, oracles,
            "so_bsymbolic_binds_local", ["-Wl,-Bsymbolic"], "43\n"))
        results.append(_zdefs_test(args, oracles))
        results.append(_so_shared_flags_test(args, oracles))

    if not args.tag or args.tag == "exports":
        if not args.filter or "export_dynamic" in args.filter or "rdynamic" in args.filter:
            results.extend(_export_dynamic_test(args, oracles))

    if not args.tag or args.tag == "kernel":
        if not args.filter or "emit_relocs" in args.filter or "kernel" in args.filter:
            results.extend(_emit_relocs_test(args, oracles))

    if not args.tag or args.tag == "exports":
        if not args.filter or "exclude" in args.filter:
            results.extend(_exclude_libs_test(args, oracles))

    if not args.tag or args.tag == "driver":
        if not args.filter or "driver" in args.filter or "lto" in args.filter:
            results.extend(_driver_option_noise_test(args, oracles))

    if not args.tag or args.tag == "layout":
        if not args.filter or "segment" in args.filter or "relro" in args.filter \
                or "shared_lib" in args.filter \
                or "output_not" in args.filter:
            results.extend(_segment_packing_test(args, oracles))

    if not args.tag or args.tag == "map":
        if not args.filter or "map" in args.filter:
            results.append(_map_file_test(args, oracles))
            results.append(_map_file_spellings_test(args, oracles))

    if not args.tag or args.tag == "robustness":
        if not args.filter or "malformed" in args.filter or "robust" in args.filter:
            results.extend(_robustness_tests(args, oracles))

    npass = sum(1 for r in results if r.status == "PASS")
    nfail = sum(1 for r in results if r.status == "FAIL")
    nwarn = sum(1 for r in results if r.status == "WARN")
    nskip = sum(1 for r in results if r.status == "SKIP")
    print()
    for r in results:
        if r.status != "PASS" or args.verbose:
            print(f"[{r.status}] {r.name}" + (f"\n    {r.detail}" if r.detail else ""))
    print(f"\n== linker tests: {npass} pass, {nfail} fail, {nwarn} warn, {nskip} skip "
          f"(oracles: bfd{' mold' if have_mold else ''}{' wild' if have_wild else ''}) ==")
    sys.exit(1 if nfail else 0)

_WILD_SHIM_DIR = None
def _wild_shim(wildpath):
    """gcc has no -fuse-ld=wild; create a shim dir with ld -> wild."""
    global _WILD_SHIM_DIR
    if _WILD_SHIM_DIR is None:
        _WILD_SHIM_DIR = tempfile.mkdtemp(prefix="wildshim.")
        os.symlink(wildpath, os.path.join(_WILD_SHIM_DIR, "ld"))
    return os.path.join(_WILD_SHIM_DIR, "ld")

def run_case(c, args, oracles):
    td = tempfile.mkdtemp(prefix=f"lnk.{c.name}.")
    try:
        objs, err = compile_sources(td, c, c.compile_flags)
        if err:
            return Result(c.name, "SKIP", err)

        if c.setup == "LCCC_SO":
            r = _mklib_lccc_so(td, args.lccc)
            if r.returncode != 0:
                return Result(c.name, "FAIL",
                              f"lccc -shared failed: {r.stderr.decode()[:400]}")
        elif c.setup == "LCCC_SO_PLUG2":
            r = sh([CC, "-c", "-fpic", "-O1", "plug2.c"], cwd=td)
            if r.returncode == 0:
                r = sh([args.lccc, "-shared", "plug2.o", "-o", "libplug2.so"], cwd=td)
            if r.returncode != 0:
                return Result(c.name, "FAIL",
                              f"lccc -shared failed: {r.stderr.decode()[:400]}")
        elif callable(c.setup):
            c.setup(td)

        inputs = c.link_inputs if c.link_inputs else objs
        # drop archive-only fixture objects from default input list
        outputs = {}

        # --- lccc link ---
        lccc_out = os.path.join(td, "out.lccc")
        r = link_with([args.lccc], inputs, "out.lccc",
                      c.ldflags + c.lccc_only_flags, td)
        lccc_link_ok = (r.returncode == 0 and os.path.exists(lccc_out))
        lccc_link_err = (r.stderr.decode(errors="replace") +
                         r.stdout.decode(errors="replace"))[:500]

        # --- oracle links ---
        oracle_outs = []
        if not c.skip_oracles:
            for oname, ocmd in oracles:
                oout = os.path.join(td, f"out.{oname}")
                orr = link_with(ocmd, inputs, f"out.{oname}",
                                c.ldflags + c.oracle_only_flags, td)
                if orr.returncode == 0 and os.path.exists(oout):
                    oracle_outs.append((oname, oout))

        if c.expect_fail:
            if lccc_link_ok:
                return Result(c.name, "FAIL", "link unexpectedly succeeded "
                              "(expected diagnostic)")
            return Result(c.name, "PASS")

        if not lccc_link_ok:
            if oracle_outs:
                return Result(c.name, "FAIL",
                    f"lccc link failed but {oracle_outs[0][0]} succeeded: {lccc_link_err}")
            return Result(c.name, "SKIP", f"all linkers failed: {lccc_link_err}")

        # --- run & compare ---
        code, out = run_bin(lccc_out, c.run_args, td, c.run_env)
        if c.expect_stdout is not None:
            if out != c.expect_stdout or code != c.expect_exit:
                return Result(c.name, "FAIL",
                    f"lccc output {(code, out)!r} != expected {(c.expect_exit, c.expect_stdout)!r}")
            return Result(c.name, "PASS")

        mismatches = []
        for oname, oout in oracle_outs:
            ocode, oo = run_bin(oout, c.run_args, td, c.run_env)
            if (code, out) != (ocode, oo):
                mismatches.append(f"{oname}: {(ocode, oo)!r}")
        if mismatches:
            return Result(c.name, "FAIL",
                f"lccc {(code, out)!r} != " + "; ".join(mismatches))
        if not oracle_outs and code != 0:
            return Result(c.name, "FAIL", f"binary exited {code}: {out!r}")
        return Result(c.name, "PASS")
    except Exception as e:
        return Result(c.name, "FAIL", f"harness exception: {e!r}")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)

if __name__ == "__main__":
    main()
