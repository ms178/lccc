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
import shutil
import subprocess
import sys
import tempfile
import textwrap

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
DEFAULT_LCCC = os.path.join(REPO, "target", "release", "lccc-x86")

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
