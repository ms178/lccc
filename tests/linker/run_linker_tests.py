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
import ctypes
import json
import os
import struct
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
CXX = os.environ.get("LINKTEST_CXX", "g++")

def compiler_for(fname):
    """Pick the driver for a fixture source; C++ needs g++, not gcc."""
    return CXX if os.path.splitext(fname)[1] in (".cpp", ".cc", ".cxx") else CC

class Case:
    def __init__(self, name, sources, link_inputs=None, ldflags=None,
                 run_args=None, expect_fail=False, expect_stdout=None,
                 expect_exit=0, compile_flags=None, setup=None,
                 oracle_only_flags=None, lccc_only_flags=None,
                 skip_oracles=False, run_env=None, tags=(),
                 expect_elf_type=None, known_defect=None,
                 expect_no_zero_size_syms=False, expect_no_symtab=False,
                 expect_dyn_tags=None, expect_prop_note_match=False,
                 expect_symtab_valid=False, expect_dyn_init_fini=False,
                 expect_dyn_relacount=None, expect_same_address=None,
                 expect_distinct_addresses=None,
                 expect_dyn_has=None, expect_dyn_missing=None,
                 expect_comment=None, expect_symbol_order=None):
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
        # Expected ELF e_type of the lccc output ("EXEC" / "DYN").  This is what
        # distinguishes a real position-independent image from a silent
        # fixed-base fallback: a PIE test that only checks stdout passes either
        # way, and losing ASLR is exactly the regression that must not hide.
        self.expect_elf_type = expect_elf_type
        # Set to a short description when the case documents a defect we have not
        # fixed yet.  A documented failure is reported as WARN so it stays
        # visible in the summary instead of being deleted or silently ignored;
        # if it starts passing, the flag is stale and should be removed.
        self.known_defect = known_defect
        # Assert no symbol has st_value == 0 with st_size != 0.  That is the
        # signature of a symbol from a section that was collected away.
        self.expect_no_zero_size_syms = expect_no_zero_size_syms
        # Assert the image carries no .symtab/.strtab at all (`-s`).
        self.expect_no_symtab = expect_no_symtab
        # Exact set of dynamic hash tags that must be present.  Checks the
        # section headers too, so a table that is tagged but never written (or
        # written over by the other table's writer) cannot pass.
        self.expect_dyn_tags = expect_dyn_tags
        # Compare the merged `.note.gnu.property` of lccc's output against
        # bfd's, entry for entry ({type: value} maps, so note-layout
        # choices cannot hide a merge disagreement).  bfd is the reference;
        # when it produced no output the case skips instead of guessing.
        self.expect_prop_note_match = expect_prop_note_match
        # Full `.symtab` shape check: sh_info == nlocals AND every LOCAL
        # precedes the first global.  Subsumes the count-only check that
        # `expect_no_zero_size_syms` runs as a side effect.
        self.expect_symtab_valid = expect_symtab_valid
        # DT_INIT/DT_FINI must be present AND equal the .init/.fini section
        # addresses (crti/crtn place _init/_fini at the section starts, so a
        # value check against the sections is the bfd behaviour, exactly).
        self.expect_dyn_init_fini = expect_dyn_init_fini
        # DT_RELACOUNT: True means present with a value equal to the leading
        # R_X86_64_RELATIVE run of .rela.dyn (re-parsed, not trusted);
        # False means the tag must be absent AND the run must be empty
        # (a GLOB_DAT-only .rela.dyn, as in a non-PIE link).  None skips.
        self.expect_dyn_relacount = expect_dyn_relacount
        # Groups of symbol names that must share one st_value (ICF alias
        # check): every name must be PRESENT in .symtab and all values in
        # a group must be equal.  stdout alone cannot tell "folded" from
        # "never folded" — or from "folded but the loser was dropped".
        self.expect_same_address = expect_same_address
        # Mirror image: groups of symbol names that must all have DIFFERENT
        # st_values (safe-ICF check).  Observes "not folded" without taking
        # any address in the test program itself — which would perturb the
        # very property under test.
        self.expect_distinct_addresses = expect_distinct_addresses
        # Dynamic tags (by name, see _DT_NUMBERS) that must be present in
        # or absent from .dynamic.  Presence-only — value checks live in
        # the dedicated params above.
        self.expect_dyn_has = expect_dyn_has
        self.expect_dyn_missing = expect_dyn_missing
        # Ordered .comment subsequence: every listed string must occur
        # EXACTLY ONCE in the merged .comment section, in the listed
        # relative order.  Extra entries (the toolchain's own ident from
        # the crt objects) are allowed — the check pins dedup and order,
        # not the toolchain string, so it stays robust across gcc builds.
        self.expect_comment = expect_comment
        # Symbol names in strictly ascending st_value order (--sort-common
        # check).  Every name must exist; equal addresses fail (a folded
        # alias is not an ordering).
        self.expect_symbol_order = expect_symbol_order

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
    # `main` deliberately does NOT reference anything in the archive.  It reads
    # a side effect the constructor produced through a variable main.c *does*
    # define, so lazy archive scanning can never pull the member in for us: the
    # only way this member gets into the link is --whole-archive itself.  (An
    # earlier version of this case had main.c declare `extern int
    # flag_from_ctor`, which made it pass with --whole-archive entirely
    # unimplemented.)
    {"main.c": """
        #include <stdio.h>
        int flag_from_ctor;   /* defined here, so the archive is not pulled
                                 in to satisfy an undefined symbol */
        int main(void){ printf("%d\\n", flag_from_ctor); return 0; }
     """,
     "wa.c": """
        extern int flag_from_ctor;
        __attribute__((constructor)) static void init(void){ flag_from_ctor = 42; }
     """},
    link_inputs=["main.o", "-Wl,--whole-archive", "libwa.a", "-Wl,--no-whole-archive"],
    expect_stdout="42\n",
    setup=_make_wa,
    tags=("archive",))

case("archive_lazy_loading_is_lazy",
    # The complement of whole_archive_exec: with the flag absent the same
    # archive member must stay out, or the flag would be indistinguishable from
    # doing nothing.
    {"main.c": """
        #include <stdio.h>
        int flag_from_ctor;
        int main(void){ printf("%d\\n", flag_from_ctor); return 0; }
     """,
     "wa.c": """
        extern int flag_from_ctor;
        __attribute__((constructor)) static void init(void){ flag_from_ctor = 42; }
     """},
    link_inputs=["main.o", "libwa.a"],
    expect_stdout="0\n",
    setup=_make_wa,
    tags=("archive",))

def _make_weakext(td):
    for f in ("n1.c", "w2.c"):
        r = sh([CC, "-c", f], cwd=td); assert r.returncode == 0
    r = sh(["ar", "rcs", "libweakext.a", "n1.o", "w2.o"], cwd=td); assert r.returncode == 0

case("archive_weak_ref_no_extract",
    # A weak-undefined reference must NOT pull an archive member (GNU ld
    # leaves it resolving to zero). The strong reference to `needed` in the
    # same archive proves extraction itself still works: expect "7 0".
    # Regression test: lccc used to pull w2.o for the weak ref, printing
    # "7 1" and dragging +37 KiB of libio into every static link.
    {"main.c": """
        #include <stdio.h>
        extern int needed(void);
        __attribute__((weak)) int wref(void);
        int main(void){ printf("%d %d\\n", needed(), wref ? 1 : 0); return 0; }
     """,
     "n1.c": "int needed(void){ return 7; }",
     "w2.c": "int wref(void){ return 99; }"},
    link_inputs=["main.o", "libweakext.a"],
    expect_stdout="7 0\n",
    setup=_make_weakext,
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

case("gc_sections_keeps_eh_frame",
    # Regression: --gc-sections used to collect `.eh_frame` outright (nothing
    # relocates *to* it), so a --gc-sections build silently lost all stack
    # unwinding: backtrace() returned a single frame where every oracle
    # returns the full depth.  `.eh_frame` is now a GC root, and the FDEs of
    # genuinely collected functions are pruned rather than left stale.
    {"a.c": """
        #include <stdio.h>
        #include <execinfo.h>
        static int l4(void){ void *bt[16]; return backtrace(bt, 16); }
        static int l3(void){ return l4(); }
        static int l2(void){ return l3(); }
        static int l1(void){ return l2(); }
        int dead_a(void){ return 1; }
        int dead_b(void){ return 2; }
        int main(void){ printf("%d\\n", l1() >= 4); return 0; }
     """},
    ldflags=["-Wl,--gc-sections", "-rdynamic"],
    compile_flags=["-O2", "-ffunction-sections", "-fdata-sections"],
    expect_stdout="1\n",
    tags=("sections", "gc", "unwind"))

case("gc_sections_comdat_winner_reached",
    # Regression: the sweep ran before COMDAT selection and followed a
    # reference to the referencing object's own copy of an inline function.
    # Here only main.o's copy is referenced from live code; COMDAT selection
    # keeps t1.o's (first in link order), which the sweep had collected --
    # so neither survived and main called into nothing (SIGSEGV).
    {"t1.cc": """
        inline int __attribute__((noinline)) shared_inline(int x) {
            static volatile int k = 7; return x * k + 1; }
        int dead_user(int x) { return shared_inline(x) + 5; }
     """,
     "m.cc": """
        #include <cstdio>
        inline int __attribute__((noinline)) shared_inline(int x) {
            static volatile int k = 7; return x * k + 1; }
        int main(int c, char **) { std::printf("%d\\n", shared_inline(c + 1)); return 0; }
     """},
    link_inputs=["t1.o", "m.o"],
    ldflags=["-Wl,--gc-sections"],
    compile_flags=["-O1", "-ffunction-sections", "-fdata-sections"],
    expect_stdout="15\n",
    tags=("sections", "gc", "comdat"))

case("gc_sections_weak_loses_to_strong",
    # The reference in a.o binds, at link time, to b.o's strong definition;
    # the sweep must keep THAT section, not a.o's weak copy it can see.
    {"a.c": """
        #include <stdio.h>
        __attribute__((weak, noinline)) int wfn(void) { return 1; }
        int main(void) { printf("%d\\n", wfn()); return 0; }
     """,
     "b.c": "int wfn(void) { return 42; }"},
    link_inputs=["a.o", "b.o"],
    ldflags=["-Wl,--gc-sections"],
    compile_flags=["-O1", "-ffunction-sections", "-fdata-sections"],
    expect_stdout="42\n",
    tags=("sections", "gc"))

case("gc_sections_cxx_personality_via_cie",
    # The personality routine's DW.ref pointer (a COMDAT data section) is
    # referenced only from the zPLR CIE, and the FDE pass used to follow the
    # FDE's relocations but not its CIE's: --gc-sections dropped it and the
    # first throw crashed in _Unwind_RaiseException.
    {"t.cc": """
        #include <stdexcept>
        void thrower(int x) { if (x) throw std::runtime_error("boom"); }
     """,
     "m.cc": """
        #include <cstdio>
        #include <stdexcept>
        void thrower(int);
        int main(int c, char **) {
            try { thrower(c); } catch (const std::exception &e) { std::printf("caught %s\\n", e.what()); }
            return 0;
        }
     """},
    link_inputs=["t.o", "m.o"],
    ldflags=["-Wl,--gc-sections", "-lstdc++"],
    compile_flags=["-O1", "-ffunction-sections", "-fdata-sections"],
    expect_stdout="caught boom\n",
    tags=("sections", "gc", "unwind"))

case("gc_sections_export_dynamic_dlsym",
    # Regression: with --export-dynamic an exported global is reachable only
    # from *outside* the image, so a reachability sweep collected it.  The
    # link succeeded and the binary ran; it just failed at the first dlsym.
    {"a.c": """
        #include <stdio.h>
        #include <dlfcn.h>
        int plugin_entry(int x){ return x * 7; }
        int main(void){
            void *h = dlopen(0, RTLD_NOW);
            int (*fn)(int) = (int (*)(int))dlsym(h, "plugin_entry");
            printf("%d\\n", fn ? fn(6) : -1);
            return 0;
        }
     """},
    ldflags=["-Wl,--gc-sections", "-rdynamic", "-ldl"],
    compile_flags=["-O2", "-ffunction-sections", "-fdata-sections"],
    expect_stdout="42\n",
    tags=("sections", "gc", "dynamic"))

case("build_id_note_emitted",
    # Debian's gcc passes --build-id on *every* link.  lccc-ld accepted the flag
    # but emitted no note, so no binary could be matched to its debuginfo.
    # The digest must be present, non-zero and reproducible; the assertion is a
    # dedicated test below (build_id_note_present_test), this case just proves
    # the flag does not break the link or the program.
    {"a.c": """
        #include <stdio.h>
        int main(void){ printf("ok\\n"); return 0; }
     """},
    ldflags=["-Wl,--build-id=sha1"],
    expect_stdout="ok\n",
    tags=("notes",))

case("defsym_two_argument_form",
    # GNU ld accepts both `--defsym=SYM=VAL` and `--defsym SYM=VAL`; only the
    # joined spelling was parsed, so the two-argument form died as an unknown
    # option (found via the differential oracle, census2).
    {"a.c": """
        #include <stdio.h>
        int main(void){ printf("ok\\n"); return 0; }
     """},
    ldflags=["-Wl,--defsym", "-Wl,aliased=main"],
    expect_stdout="ok\n",
    tags=("symbols",))

case("wrap_two_argument_form",
    # Same defect class as --defsym: `--wrap SYM` must work, not just
    # `--wrap=SYM`.  __wrap_bv intercepts, __real_bv reaches the original.
    {"a.c": """
        #include <stdio.h>
        extern int bv(void);
        int main(void){ printf("%d\\n", bv()); return 0; }
     """,
     "b.c": "int bv(void){ return 7; }",
     "w.c": """
        extern int __real_bv(void);
        int __wrap_bv(void){ return __real_bv() * 10; }
     """},
    ldflags=["-Wl,--wrap", "-Wl,bv"],
    expect_stdout="70\n",
    tags=("symbols",))

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

# PT_TLS p_vaddr must be a multiple of the segment's largest member
# alignment: the loader computes the thread-pointer offset from p_vaddr
# modulo p_align (glibc _dl_determine_tlsoffset), so an image whose TLS
# segment starts misaligned -- here .tdata (align 4) whose RW-segment offset
# is not 64-aligned, followed by an aligned(64) .tbss member -- hands out a
# misaligned `b`.  `tls_alignment` above passed by layout luck while the
# builtin linkers laid PT_TLS out at the .tdata start unaligned.
# The RW data in front of the TLS segment is varied so that the natural
# (unaligned) .tdata position lands at several different offsets mod 64.
for (_suffix, _flags), _pad in ((_l, _n) for _l in (("", []), ("_static", ["-static"]))
                                for _n in (1, 3, 6, 11)):
    case(f"tls_segment_alignment_after_tdata{_suffix}_pad{_pad}",
        {"a.c": """
            #include <stdio.h>
            #include <stdint.h>
            __thread int a = 1;
            __thread int a2 = 2;
            __thread char b[3] __attribute__((aligned(64)));
            int main(void){
                b[0] = 9;
                printf("%d %d %d %d\\n", a, a2, b[0], (int)((uintptr_t)b % 64));
                return 0;
            }
         """,
         "pad.c": f"int rw_pad[{_pad}] = {{1}};"},
        ldflags=_flags,
        expect_stdout="1 2 9 0\n",
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


# ── PIE (S06): -pie must produce a genuinely position-independent ET_DYN, not a
# fixed-base ET_EXEC.  These are the constructs that broke first when the image
# was rebased at 0; each one exercises a distinct class of self-referential
# address that has to be slid by the load base.

case("pie_basic_et_dyn",
    {"a.c": """
        #include <stdio.h>
        int main(void){ printf("pie-ok\\n"); return 0; }
     """},
    ldflags=["-pie"],
    expect_stdout="pie-ok\n",
    expect_elf_type="DYN",
    tags=("pie",))

case("pie_string_array",
    # The regression that made a competing -pie implementation segfault: string
    # merging rewrites these pointers to reference a synthetic pool symbol with
    # shndx 0, so they look undefined while being entirely local.  Without an
    # R_X86_64_RELATIVE per entry the array still holds link-time addresses and
    # the first "%s" dereference faults under a random load base.
    {"a.c": """
        #include <stdio.h>
        const char *msgs[] = {"alpha", "beta", "gamma", "delta"};
        int main(void){ printf("%s %s\\n", msgs[1], msgs[3]); return 0; }
     """},
    ldflags=["-pie"],
    expect_stdout="beta delta\n",
    expect_elf_type="DYN",
    tags=("pie", "strmerge"))

case("pie_jump_table",
    # Address-taken labels in a read-only jump table: .text-relative pointers
    # into the merged string/rodata sections plus .text itself.
    {"a.c": """
        #include <stdio.h>
        int main(void){
            static const void *jt[] = { &&L0, &&L1, &&L2 };
            int sum = 0;
            for (int i = 0; i < 3; i++) { goto *jt[i]; L0: sum += 1; goto E; L1: sum += 2; goto E; L2: sum += 4; E: ; }
            printf("%d\\n", sum);
            return 0;
        }
     """},
    ldflags=["-pie"],
    expect_stdout="7\n",
    expect_elf_type="DYN",
    tags=("pie",))

case("pie_function_pointer_array",
    {"a.c": """
        #include <stdio.h>
        static int f0(int x){ return x; }
        static int f1(int x){ return x * 2; }
        static int f2(int x){ return x * 3; }
        static int f3(int x){ return x * 4; }
        typedef int (*F)(int);
        static F table[] = { f0, f1, f2, f3 };
        int main(void){
            int s = 0;
            for (int i = 0; i < 4; i++) s += table[i](i + 1);
            printf("%d\\n", s);
            return 0;
        }
     """},
    ldflags=["-pie"],
    expect_stdout="30\n",   # f0(1)+f1(2)+f2(3)+f3(4) = 1+4+9+16
    expect_elf_type="DYN",
    tags=("pie",))

case("pie_main_via_got",
    # crt1.o reaches main through R_X86_64_REX_GOTPCRELX, so _start loads it out
    # of a GOT slot.  In an ET_EXEC the slot is just pre-filled; in a PIE it must
    # carry an R_X86_64_RELATIVE or the program jumps to the unslid address.
    # Covered implicitly by every PIE case, asserted explicitly here.
    {"a.c": """
        #include <stdio.h>
        int answer(void){ return 42; }
        int main(void){ printf("%d\\n", answer()); return 0; }
     """},
    ldflags=["-pie"],
    expect_stdout="42\n",
    expect_elf_type="DYN",
    tags=("pie", "got"))

case("no_pie_stays_et_exec",
    {"a.c": """
        #include <stdio.h>
        int main(void){ printf("exec-ok\\n"); return 0; }
     """},
    ldflags=["-no-pie"],
    expect_stdout="exec-ok\n",
    expect_elf_type="EXEC",
    tags=("pie",))

case("local_ifunc",
    # A *static* IFUNC.  This used to be a silent miscompile: local IFUNCs never
    # reached collect_ifunc_symbols (it walks `globals`, and a static symbol is
    # never promoted there), so they got no IPLT slot and no R_X86_64_IRELATIVE,
    # and the call site bound straight to the RESOLVER -- the caller received
    # the implementation's *address* as the call's return value and printed
    # something like 4199558 where 1 was meant.  Local IFUNCs now get slots
    # numbered after the global ones, keyed by (object, symbol) because local
    # names legitimately repeat across objects.
    {"a.c": """
        #include <stdio.h>
        static int impl(void){ return 1; }
        static int (*rsv(void))(void){ return impl; }
        static int fn(void) __attribute__((ifunc("rsv")));
        int main(void){ printf("%d\\n", fn()); return 0; }
     """},
    expect_stdout="1\n",
    tags=("ifunc",))


# ── v3 defect fixes ──────────────────────────────────────────────────────────

case("gc_sections_symtab_no_dead_locals",
    # --gc-sections used to leave every symbol from a collected section in
    # .symtab at address 0: the locals filter consulted section_map, which still
    # holds an entry for dead sections (layout assigns a slot before collection
    # decides they are unreachable).  On a 61-object -ffunction-sections link
    # that was 2341 of 2551 entries -- 61 KB against bfd's 6 KB.
    {"a.c": """
        #include <stdio.h>
        int used(int x){ return x + 1; }
        static int dead_a(int x){ return x * 3; }
        static int dead_b(int x){ return x * 5; }
        static int dead_c(int x){ return x * 7; }
        int main(void){ printf("%d\\n", used(1)); return 0; }
     """},
    compile_flags=["-O2", "-ffunction-sections", "-fdata-sections"],
    ldflags=["--gc-sections"],
    expect_stdout="2\n",
    # Assert the invariant directly: no symbol may claim a non-zero size at
    # address 0, which is what a collected-section local looks like.
    expect_no_zero_size_syms=True,
    expect_symtab_valid=True,
    tags=("gc", "symtab"))

case("symtab_sh_info_equals_locals",
    # sh_info must equal the number of STB_LOCAL entries (ELF: sh_info ==
    # nlocals).  It used to be snapshotted between the local and global loops,
    # so any global-loop entry carrying STB_LOCAL made it one short.
    {"a.c": """
        #include <stdio.h>
        static int helper(int x){ return x; }
        int global_fn(int x){ return helper(x); }
        int main(void){ printf("%d\\n", global_fn(9)); return 0; }
     """},
    expect_stdout="9\n",
    expect_symtab_valid=True,
    tags=("symtab",))

case("symtab_locals_before_globals_with_pools",
    # String merging creates synthetic pool symbols that live in BOTH the
    # object symbol lists and the resolved globals map; the globals-loop
    # copy used to be emitted a second time with its STB_LOCAL binding,
    # after sh_info — a hard ELF violation (readelf warns, debuggers
    # misclassify).  Two string-heavy objects guarantee pool formation.
    {"a.c": """
        #include <stdio.h>
        static int helper1(int x){ return x + 1; }
        int fn1(int x){ printf("fn1 sees %d\\n", helper1(x)); return helper1(x); }
     """,
     "b.c": """
        #include <stdio.h>
        int fn1(int);
        static int helper2(int x){ return x + 2; }
        int fn2(int x){ printf("fn2 sees %d\\n", helper2(x)); return helper2(x); }
        int main(void){ printf("%d\\n", fn1(1) + fn2(10)); return 0; }
     """},
    expect_stdout="fn1 sees 2\nfn2 sees 12\n14\n",
    expect_symtab_valid=True,
    tags=("symtab",))

case("strip_all_drops_symtab",
    # `-s` was parsed into a local that only reached the linker-script path, so
    # a "stripped" binary shipped with a full symbol table.  Worse than ignoring
    # the flag: the user believes they removed the symbols.
    {"a.c": """
        #include <stdio.h>
        int named_symbol(int x){ return x * 2; }
        int main(void){ printf("%d\\n", named_symbol(21)); return 0; }
     """},
    ldflags=["-Wl,-s"],
    expect_stdout="42\n",
    expect_no_symtab=True,
    tags=("strip",))

case("hash_style_gnu",
    {"a.c": """
        #include <stdio.h>
        int main(void){ printf("hs\\n"); return 0; }
     """},
    ldflags=["-Wl,--hash-style=gnu"],
    expect_stdout="hs\n",
    expect_dyn_tags=["GNU_HASH"],
    tags=("hash",))

case("hash_style_sysv",
    # SysV .hash.  With gnu_hash_size forced to 0 the two tables alias the same
    # offset, so an ungated GNU-hash writer silently clobbers the SysV one.
    {"a.c": """
        #include <stdio.h>
        int main(void){ printf("hs\\n"); return 0; }
     """},
    ldflags=["-Wl,--hash-style=sysv"],
    expect_stdout="hs\n",
    expect_dyn_tags=["HASH"],
    tags=("hash",))

case("hash_style_both",
    {"a.c": """
        #include <stdio.h>
        int main(void){ printf("hs\\n"); return 0; }
     """},
    ldflags=["-Wl,--hash-style=both"],
    expect_stdout="hs\n",
    expect_dyn_tags=["GNU_HASH", "HASH"],
    tags=("hash",))

case("dyn_init_fini_pie",
    # A PIE executable's .dynamic must tag .init/.fini (values equal to the
    # section addresses, as bfd emits) and count the leading RELATIVE run.
    {"a.c": """
        #include <stdio.h>
        int main(void){ printf("di\\n"); return 0; }
     """},
    ldflags=["-pie"],
    expect_stdout="di\n",
    expect_elf_type="DYN",
    expect_dyn_init_fini=True,
    expect_dyn_relacount=True,
    tags=("dynamic",))

case("dyn_init_fini_nopie",
    # Non-PIE: INIT/FINI stay, but with an empty RELATIVE run DT_RELACOUNT
    # must be omitted entirely (bfd parity: a non-PIE .dynamic has no
    # RELACOUNT tag).  The check also fails if .rela.dyn secretly opens
    # with RELATIVE entries while the tag is missing.
    {"a.c": """
        #include <stdio.h>
        int main(void){ printf("di\\n"); return 0; }
     """},
    ldflags=["-no-pie"],
    expect_stdout="di\n",
    expect_elf_type="EXEC",
    expect_dyn_init_fini=True,
    expect_dyn_relacount=False,
    tags=("dynamic",))

case("rpath_default_is_runpath",
    # A plain -rpath must land in DT_RUNPATH, not DT_RPATH: bfd (as every
    # major distro configures it), lld and mold all default to new dtags.
    {"a.c": """
        #include <stdio.h>
        int main(void){ printf("rp\\n"); return 0; }
     """},
    ldflags=["-Wl,-rpath,/opt/x"],
    expect_stdout="rp\n",
    expect_dyn_has=["RUNPATH"],
    expect_dyn_missing=["RPATH"],
    tags=("dynamic",))

case("rpath_disable_new_dtags",
    # --disable-new-dtags opts back into the historic DT_RPATH tag.
    {"a.c": """
        #include <stdio.h>
        int main(void){ printf("rp\\n"); return 0; }
     """},
    ldflags=["-Wl,-rpath,/opt/x", "-Wl,--disable-new-dtags"],
    expect_stdout="rp\n",
    expect_dyn_has=["RPATH"],
    expect_dyn_missing=["RUNPATH"],
    tags=("dynamic",))

case("rpath_enable_new_dtags",
    # Explicit --enable-new-dtags: DT_RUNPATH (documents the spelled-out
    # form of the default).
    {"a.c": """
        #include <stdio.h>
        int main(void){ printf("rp\\n"); return 0; }
     """},
    ldflags=["-Wl,-rpath,/opt/x", "-Wl,--enable-new-dtags"],
    expect_stdout="rp\n",
    expect_dyn_has=["RUNPATH"],
    expect_dyn_missing=["RPATH"],
    tags=("dynamic",))

case("comment_merged_deduplicated",
    # .comment inputs merge with dedup in link order, as in bfd: b.s
    # repeats a.s's string, so "comment-A" must appear exactly once and
    # before "comment-B".  Extra toolchain idents (from the crt objects)
    # are tolerated — only the relative order and uniqueness are pinned.
    {"main.c": """
        #include <stdio.h>
        int fa(void); int fb(void);
        int main(void){ printf("%d\\n", fa() + fb()); return 0; }
     """,
     "a.s": """
        .section .comment,"MS",@progbits,1
        .string "comment-A"
        .text
        .globl fa
        .type fa,@function
fa:
        movl $1, %eax
        ret
     """,
     "b.s": """
        .section .comment,"MS",@progbits,1
        .string "comment-A"
        .string "comment-B"
        .text
        .globl fb
        .type fb,@function
fb:
        movl $2, %eax
        ret
     """},
    compile_flags=["-O1", "-fno-ident"],
    link_inputs=["main.o", "a.o", "b.o"],
    expect_stdout="3\n",
    expect_comment=["comment-A", "comment-B"],
    tags=("sections",))

case("sort_common_largest_first",
    # --sort-common lays COMMONs out largest-first (bfd parity).  The
    # source declares them smallest-first, so ascending addresses prove
    # the sort ran.  Needs -fcommon: modern gcc defaults to -fno-common
    # (.bss instead of SHN_COMMON).
    {"a.c": """
        #include <stdio.h>
        char sc_c;
        short sc_s;
        long sc_l;
        int main(void){
            sc_c = 1; sc_s = 2; sc_l = 3;
            printf("%ld\\n", (long)sc_c + sc_s + sc_l);
            return 0;
        }
     """},
    compile_flags=["-O1", "-fcommon"],
    lccc_only_flags=["-Wl,--sort-common"],
    expect_stdout="6\n",
    expect_symbol_order=["sc_l", "sc_s", "sc_c"],
    tags=("common",))

case("local_ifunc_static_link",
    {"a.c": """
        #include <stdio.h>
        static int impl(void){ return 77; }
        static int (*rsv(void))(void){ return impl; }
        static int fn(void) __attribute__((ifunc("rsv")));
        int main(void){ printf("%d\\n", fn()); return 0; }
     """},
    ldflags=["-static"],
    expect_stdout="77\n",
    tags=("ifunc", "static"))


case("ifunc_resolver",
    {"a.c": """
        #include <stdio.h>
        static int impl_a(void){ return 1; }
        static int impl_b(void){ return 2; }
        static int (*resolve_pick(void))(void) { return impl_b; }
        int pick(void) __attribute__((ifunc("resolve_pick")));
        int main(void){ printf("%d\\n", pick()); return 0; }
     """},
    # Was a bare "runs with rc 0" check, which passed while the call bound to
    # the RESOLVER and printed the implementation's *address*.  Assert the value.
    expect_stdout="2\n",
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
    expect_stdout="33\n",
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

case("relro_data_rel_ro_dynamic",
    # .data.rel.ro must live INSIDE PT_GNU_RELRO: a write into it has to
    # SIGSEGV exactly like .init_array does. Regression test: lccc placed
    # the section in the generic RW loop, past the RELRO boundary, leaving
    # every vtable/const-pointer writable at runtime.
    {"a.c": """
        #include <stdio.h>
        #include <signal.h>
        #include <setjmp.h>
        static sigjmp_buf jb;
        static void segv(int s){ (void)s; siglongjmp(jb, 1); }
        static const int x = 5;
        static const int * const p = &x;
        int main(void){
            signal(SIGSEGV, segv);
            if (sigsetjmp(jb, 1) == 0) {
                *(const int **)&p = 0;
                printf("WRITABLE\\\\n");
                return 1;
            }
            printf("PROTECTED\\\\n");
            return 0;
        }
     """},
    compile_flags=["-O0"],
    expect_stdout="PROTECTED\\n",
    tags=("special", "relro"))

case("relro_static_init_array",
    # Static executables get PT_GNU_RELRO too, and static glibc enforces
    # it: writing .init_array must SIGSEGV. The .data counter increment
    # first proves writable data still works (RELRO must end before .data;
    # if it covered .data the increment would crash before any handler).
    {"a.c": """
        #include <stdio.h>
        #include <signal.h>
        #include <setjmp.h>
        static sigjmp_buf jb;
        static int data_counter;
        static void segv(int s){ (void)s; siglongjmp(jb, 1); }
        typedef void (*fp)(void);
        __attribute__((constructor)) static void c1(void){}
        extern fp __init_array_start[] __attribute__((weak));
        int main(void){
            data_counter += 1;
            if (data_counter != 1) { printf("DATA-BROKEN\\\\n"); return 2; }
            signal(SIGSEGV, segv);
            if (sigsetjmp(jb, 1) == 0) {
                __init_array_start[0] = (fp)main;
                printf("WRITABLE\\\\n");
                return 1;
            }
            printf("PROTECTED\\\\n");
            return 0;
        }
     """},
    ldflags=["-static"],
    compile_flags=["-O0"],
    expect_stdout="PROTECTED\\n",
    tags=("special", "relro", "static"))

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
 text PT_LOAD FLAGS(5) FILEHDR PHDRS;
 data PT_LOAD FLAGS(6);
}
SECTIONS
{
 /* Linux's vDSO starts at SIZEOF_HEADERS. Keep a normal userspace base here
    so the fixture remains executable while exercising the same expression. */
 . = 0x400000 + SIZEOF_HEADERS;
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


# ── Source shared by the MEMORY / SEGMENT_START / visibility script tests ──
# Runs real code so a mis-placed section shows up as a wrong exit status rather
# than merely a different section header.
REGION_TEST_C = r"""
__attribute__((used, section(".special"))) static int spec1 = 11;
__attribute__((used, section(".special"))) static int spec2 = 31;
extern int __special_start[], __special_end[];
int global_data = 5;
static long wr(int fd, const void *buf, unsigned long n){
    long r;
    __asm__ volatile("syscall" : "=a"(r)
                     : "a"(1L), "D"((long)fd), "S"(buf), "d"(n)
                     : "rcx", "r11", "memory");
    return r;
}
void my_start(void){
    int s = 0;
    for (int *p = __special_start; p < __special_end; p++) s += *p;
    char msg[2] = { (char)('0' + (s == 42) + (global_data == 5)), '\n' };
    wr(1, msg, 2);
    __asm__ volatile("syscall" :: "a"(60L), "D"(0L));
    __builtin_unreachable();
}
"""

# MEMORY regions drive placement; the image must still run. `org`/`len`
# abbreviations are exercised because real firmware scripts use them.
MEMORY_REGION_SCRIPT = r"""
ENTRY(my_start)
MEMORY {
  lowram (rwx) : ORIGIN = 0x400000, LENGTH = 16M
}
SECTIONS {
  .text : { *(.text*) } > lowram
  .rodata : { *(.rodata*) } > lowram
  .special : {
     __special_start = .;
     *(.special)
     __special_end = .;
  } > lowram
  .data : { *(.data*) } > lowram
  .bss : { *(.bss*) *(COMMON) } > lowram
  /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }
}
"""

# SEGMENT_START appears four times in ld's own default script, so any script
# derived from `ld --verbose` needs it. Without an override it must yield the
# declared default.
SEGMENT_START_SCRIPT = r"""
ENTRY(my_start)
SECTIONS {
  . = SEGMENT_START("text-segment", 0x400000) + SIZEOF_HEADERS;
  .text : { *(.text*) }
  .rodata : { *(.rodata*) }
  .special : { __special_start = .; *(.special) __special_end = .; }
  . = DATA_SEGMENT_ALIGN(0x1000, 0x1000);
  .data : { *(.data*) }
  .bss : { *(.bss*) *(COMMON) }
  . = DATA_SEGMENT_END(.);
  /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }
}
"""

# HIDDEN()/PROVIDE_HIDDEN() must define usable symbols. Visibility is checked
# separately in the ET_DYN test; here the point is that the script parses and
# the values are right.
HIDDEN_SYMBOL_SCRIPT = r"""
ENTRY(my_start)
SECTIONS {
  . = 0x400000 + SIZEOF_HEADERS;
  .text : { *(.text*) }
  .rodata : { *(.rodata*) }
  .special : {
     HIDDEN(__special_start = .);
     *(.special)
     PROVIDE_HIDDEN(__special_end = .);
  }
  .data : { *(.data*) }
  .bss : { *(.bss*) *(COMMON) }
  /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }
}
"""

SCRIPT_TESTS = [
    ("script_kernel_style", KERNEL_STYLE_SCRIPT, SCRIPT_TEST_C, "2\n"),
    ("script_memory_region_placement", MEMORY_REGION_SCRIPT, REGION_TEST_C, "2\n"),
    ("script_segment_start_and_data_segment", SEGMENT_START_SCRIPT, REGION_TEST_C, "2\n"),
    ("script_hidden_symbol_definitions", HIDDEN_SYMBOL_SCRIPT, REGION_TEST_C, "2\n"),
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


def _script_undefined_archive_test(args, oracles):
    """`-T` script links must honour `-u SYM` against archives.

    Linux's compressed vmlinux is linked as
        lccc-ld -pie -u efi_pe_entry -T vmlinux.lds ... libstub/lib.a startup/lib.a
    `efi_pe_entry` lives in an archive member nothing else references; that
    member then pulls `efi_is64` from efi-mixed.o, which is what supplies
    `efi32_stub_entry` / `efi64_stub_entry`. Those two labels must sit
    exactly 0x200 apart or header.S dies with
        "32-bit and 64-bit EFI entry points do not match".

    The userspace `-Wl,-u` path already had a test (`undefined_flag_pulls_archive`);
    the script-driven driver used to swallow `-u` into unused passthrough.
    """
    name = "script_undefined_pulls_archive"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "pe.c"), "w") as f:
            f.write("extern int efi_is64;\n"
                    "int efi_pe_entry(void){ return efi_is64; }\n")
        with open(os.path.join(td, "mixed.c"), "w") as f:
            f.write("int efi_is64 = 1;\n"
                    "void efi32_stub_entry(void){}\n"
                    "void efi64_stub_entry(void){}\n")
        with open(os.path.join(td, "head.c"), "w") as f:
            f.write("int startup_32(void){ return 0; }\n")
        with open(os.path.join(td, "t.lds"), "w") as f:
            f.write("ENTRY(startup_32)\n"
                    "SECTIONS {\n"
                    "  . = 0;\n"
                    "  .text : { *(.text*) }\n"
                    "  .data : { *(.data*) *(.rodata*) *(.bss*) *(COMMON) }\n"
                    "  /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }\n"
                    "}\n")
        for src in ("pe.c", "mixed.c", "head.c"):
            r = sh([CC, "-c", "-O1", "-ffreestanding", "-fno-pic",
                    "-fno-asynchronous-unwind-tables", "-fno-stack-protector",
                    src], cwd=td)
            if r.returncode != 0:
                return Result(name, "SKIP", r.stderr.decode()[:150])
        r = sh(["ar", "rcs", "libstub.a", "pe.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", "ar libstub failed")
        r = sh(["ar", "rcs", "libstartup.a", "mixed.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", "ar startup failed")
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        common = ["-pie", "--no-dynamic-linker", "-u", "efi_pe_entry",
                  "-T", "t.lds", "head.o", "libstub.a", "libstartup.a"]
        r = sh([lccc_ld] + common + ["-o", "out.lccc"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL",
                          f"lccc-ld -T -u failed: {r.stderr.decode()[:400]}")
        nm = sh(["nm", "out.lccc"], cwd=td).stdout.decode()
        for required in ("efi_pe_entry", "efi_is64",
                         "efi32_stub_entry", "efi64_stub_entry"):
            if not re.search(rf"\b{required}$", nm, re.M):
                return Result(name, "FAIL",
                              f"-u did not pull archive member defining {required}")
        # Negative control: without -u the EFI symbols must stay out.
        r = sh([lccc_ld, "-pie", "--no-dynamic-linker", "-T", "t.lds",
                "head.o", "libstub.a", "libstartup.a", "-o", "out.nou"], cwd=td)
        if r.returncode == 0:
            nm_nou = sh(["nm", "out.nou"], cwd=td).stdout.decode()
            if re.search(r"\befi_pe_entry$", nm_nou, re.M):
                return Result(name, "FAIL",
                              "archive member pulled without -u (over-broad extract)")
        r2 = sh(["ld"] + common + ["-o", "out.ld"], cwd=td)
        if r2.returncode == 0:
            nm2 = sh(["nm", "out.ld"], cwd=td).stdout.decode()
            for required in ("efi_pe_entry", "efi32_stub_entry"):
                if not re.search(rf"\b{required}$", nm2, re.M):
                    return Result(name, "SKIP",
                                  "GNU ld did not pull EFI symbols either")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _file_mode_libs_test(args, oracles):
    """`-l` in relocatable (`-r`) and script (`-T`) links, vs GNU ld.

    Both modes once ignored `-lNAME` outright (they read only positional
    files), so `ld -r a.o -L. -lfoo` produced an object still undefined in
    foo and `-T` links lost every `-l`.  Checked: the defined-symbol set of
    the output equals GNU ld's for plain, --whole-archive and script
    SEARCH_DIR resolution; a library that exists only as a shared object and
    a library that does not exist are errors in both linkers.
    """
    name = "file_mode_libs_r_and_T"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    srcs = {
        "head.c": "int lib_a(void); int lib_b(void);\n"
                  "int startup(void){ return lib_a() + lib_b(); }\n",
        "a.c": "int lib_a(void){ return 1; }\n",
        "unused.c": "int lib_unused(void){ return 3; }\n",
        "b.c": "int lib_b(void){ return 2; }\n",
        "so.c": "int lib_so(void){ return 4; }\n",
        "useso.c": "int lib_so(void); int user(void){ return lib_so(); }\n",
    }
    try:
        for fn, body in srcs.items():
            with open(os.path.join(td, fn), "w") as f:
                f.write(body)
        for d in ("sub", "soonly"):
            os.mkdir(os.path.join(td, d))
        with open(os.path.join(td, "t.lds"), "w") as f:
            f.write('SEARCH_DIR("sub")\nENTRY(startup)\nSECTIONS {\n  . = 0x400000;\n'
                    "  .text : { *(.text*) }\n  .data : { *(.data*) *(.rodata*) *(.bss*) }\n"
                    "  /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }\n}\n")
        cflags = ["-c", "-O1", "-ffreestanding", "-fno-pic", "-fno-asynchronous-unwind-tables",
                  "-fno-stack-protector"]
        steps = [[CC] + cflags + ["head.c", "a.c", "unused.c", "b.c", "useso.c"],
                 ["ar", "rcs", "liba.a", "a.o", "unused.o"],
                 ["ar", "rcs", "sub/libb.a", "b.o"],
                 [CC, "-shared", "-fPIC", "so.c", "-o", "soonly/libso.so"]]
        for cmd in steps:
            r = sh(cmd, cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"fixture: {' '.join(cmd)}: {r.stderr.decode()[:200]}")
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")

        def defined(path):
            out = sh(["nm", path], cwd=td).stdout.decode()
            return sorted(m.group(1) for m in re.finditer(r"^\S*\s+[TDBR] (\S+)$", out, re.M))

        cases = [
            ("-r", ["-r", "head.o", "-Lsub", "-L.", "-la", "-lb"]),
            ("-r --whole-archive", ["-r", "head.o", "-L.", "--whole-archive", "-la",
                                    "--no-whole-archive", "sub/libb.a"]),
            ("-T SEARCH_DIR", ["-T", "t.lds", "head.o", "-L.", "-la", "-lb"]),
        ]
        for i, (what, argv) in enumerate(cases):
            got = {}
            for tag, ld in (("lccc", lccc_ld), ("gnu", "ld")):
                out = f"o{i}.{tag}"
                r = sh([ld] + argv + ["-o", out], cwd=td)
                if r.returncode != 0:
                    return Result(name, "FAIL", f"{what}: {tag} failed: {r.stderr.decode()[:300]}")
                got[tag] = defined(out)
            if got["lccc"] != got["gnu"]:
                return Result(name, "FAIL", f"{what}: defined {got['lccc']} != GNU ld {got['gnu']}")
            if "lib_b" not in got["lccc"]:
                return Result(name, "FAIL", f"{what}: -lb not linked ({got['lccc']})")

        for what, argv, needle in (
            ("shared-only -r", ["-r", "useso.o", "-Lsoonly", "-lso"], "libso.so"),
            ("missing -r", ["-r", "head.o", "-lnope"], "-lnope"),
        ):
            if sh(["ld"] + argv + ["-o", "bad.gnu"], cwd=td).returncode == 0:
                return Result(name, "FAIL", f"fixture: GNU ld accepted {what}")
            r = sh([lccc_ld] + argv + ["-o", "bad.lccc"], cwd=td)
            if r.returncode == 0:
                return Result(name, "FAIL", f"lccc-ld accepted {what}")
            if needle.encode() not in r.stderr:
                return Result(name, "FAIL", f"{what}: diagnostic lacks {needle}: {r.stderr.decode()[:200]}")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _script_undefined_archive_test_i386(args, oracles):
    """ELF32 `-T` links must honour `-u SYM` against archives.

    Linux's real-mode `setup.elf` is `lccc-ld -m elf_i386 -T setup.ld -u …`.
    The x86-64 script path already seeded `-u`; the i386 loader used to
    pass an empty extra-undefined list into archive extraction.
    """
    name = "script_undefined_pulls_archive_i386"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "pe.c"), "w") as f:
            f.write("extern int efi_is64;\n"
                    "int efi_pe_entry(void){ return efi_is64; }\n")
        with open(os.path.join(td, "mixed.c"), "w") as f:
            f.write("int efi_is64 = 1;\n"
                    "void efi32_stub_entry(void){}\n")
        with open(os.path.join(td, "head.c"), "w") as f:
            f.write("int startup_32(void){ return 0; }\n")
        with open(os.path.join(td, "t.lds"), "w") as f:
            f.write("OUTPUT_FORMAT(\"elf32-i386\")\n"
                    "OUTPUT_ARCH(i386)\n"
                    "ENTRY(startup_32)\n"
                    "SECTIONS {\n"
                    "  . = 0;\n"
                    "  .text : { *(.text*) }\n"
                    "  .data : { *(.data*) *(.rodata*) *(.bss*) *(COMMON) }\n"
                    "  /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }\n"
                    "}\n")
        for src in ("pe.c", "mixed.c", "head.c"):
            r = sh([CC, "-m32", "-c", "-O1", "-ffreestanding", "-fno-pic",
                    "-fno-asynchronous-unwind-tables", "-fno-stack-protector",
                    src], cwd=td)
            if r.returncode != 0:
                return Result(name, "SKIP", r.stderr.decode()[:150])
        r = sh(["ar", "rcs", "libstub.a", "pe.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", "ar libstub failed")
        r = sh(["ar", "rcs", "libstartup.a", "mixed.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", "ar startup failed")
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        common = ["-m", "elf_i386", "-u", "efi_pe_entry",
                  "-T", "t.lds", "head.o", "libstub.a", "libstartup.a"]
        r = sh([lccc_ld] + common + ["-o", "out.lccc"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL",
                          f"lccc-ld -m elf_i386 -T -u failed: {r.stderr.decode()[:400]}")
        nm = sh(["nm", "out.lccc"], cwd=td).stdout.decode()
        for required in ("efi_pe_entry", "efi_is64", "efi32_stub_entry"):
            if not re.search(rf"\b{required}$", nm, re.M):
                return Result(name, "FAIL",
                              f"-u did not pull archive member defining {required}")
        r = sh([lccc_ld, "-m", "elf_i386", "-T", "t.lds",
                "head.o", "libstub.a", "libstartup.a", "-o", "out.nou"], cwd=td)
        if r.returncode == 0:
            nm_nou = sh(["nm", "out.nou"], cwd=td).stdout.decode()
            if re.search(r"\befi_pe_entry$", nm_nou, re.M):
                return Result(name, "FAIL",
                              "archive member pulled without -u (over-broad extract)")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)

VDSO_SCRIPT = r"""
PHDRS {
 text PT_LOAD FILEHDR PHDRS FLAGS(5);
 dynamic PT_DYNAMIC FLAGS(4);
 stack PT_GNU_STACK FLAGS(6);
}
SECTIONS {
 . = SIZEOF_HEADERS;
 .hash : { *(.hash) } :text
 .gnu.hash : { *(.gnu.hash) } :text
 .dynsym : { *(.dynsym) } :text
 .dynstr : { *(.dynstr) } :text
 .gnu.version : { *(.gnu.version) } :text
 .gnu.version_d : { *(.gnu.version_d) } :text
 .dynamic : { *(.dynamic) } :text :dynamic
 .text : { *(.text .text.*) } :text
 /DISCARD/ : { *(.comment) *(.note.*) *(.eh_frame) }
}
VERSION {
 LCCC_VDSO_1 { global: vdso_answer; local: *; };
}
"""

# The real linux-6.18 vDSO shape: a PHDRS clause that DECLARES `note PT_NOTE`
# while the SECTIONS clause places a real allocated `.note` output section.
# GNU ld emits exactly the four declared headers; lccc-ld once counted an
# extra auto PT_NOTE, skewing e_phnum, SIZEOF_HEADERS-based file offsets and
# the PT_LOAD filesz/memsz (see _vdso_note_phdr_test).
VDSO_NOTE_SCRIPT = r"""
PHDRS {
 text PT_LOAD FILEHDR PHDRS FLAGS(5);
 dynamic PT_DYNAMIC FLAGS(4);
 note PT_NOTE FLAGS(4);
 eh_frame_hdr 0x6474e550 FLAGS(4);
}
SECTIONS {
 . = SIZEOF_HEADERS;
 .hash : { *(.hash) } :text
 .gnu.hash : { *(.gnu.hash) } :text
 .dynsym : { *(.dynsym) } :text
 .dynstr : { *(.dynstr) } :text
 .gnu.version : { *(.gnu.version) } :text
 .gnu.version_d : { *(.gnu.version_d) } :text
 .dynamic : { *(.dynamic) } :text :dynamic
 .note : { *(.note.*) } :text :note
 .text : { *(.text .text.*) } :text
 /DISCARD/ : { *(.comment) *(.eh_frame) }
}
VERSION {
 LCCC_VDSO_1 { global: vdso_answer; local: *; };
}
"""


def _bstatic_positional_test(args, oracles):
    """-Bstatic / -Bdynamic are positional library-search modes (x86-64).

    `-Wl,-Bstatic -lfoo -Wl,-Bdynamic` must take libfoo.a while the program
    stays dynamically linked against libc -- lccc-ld once rewrote -Bstatic
    to -static and produced a fully static executable.  Compared against GNU
    ld: PT_INTERP presence, the DT_NEEDED set, the run output, and that a
    -Bstatic search never falls back to a directory's libbar.so.
    """
    name = "bstatic_positional_search"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        files = {
            "foo_a.c": "int foo(void){ return 1; }\n",
            "foo_so.c": "int foo(void){ return 2; }\n",
            "bar.c": "int bar(void){ return 3; }\n",
            "main.c": "#include <stdio.h>\nint foo(void);\n"
                      "int main(void){ printf(\"%d\\n\", foo()); return 0; }\n",
            "mbar.c": "int bar(void);\nint main(void){ return bar(); }\n",
        }
        for fn, body in files.items():
            with open(os.path.join(td, fn), "w") as f:
                f.write(body)
        os.mkdir(os.path.join(td, "lib"))
        os.mkdir(os.path.join(td, "solib"))
        os.mkdir(os.path.join(td, "shim"))
        os.symlink(os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld"),
                   os.path.join(td, "shim", "ld"))
        steps = [
            [CC, "-O1", "-c", "foo_a.c", "main.c", "mbar.c"],
            ["ar", "rcs", "lib/libfoo.a", "foo_a.o"],
            [CC, "-O1", "-fPIC", "-shared", "foo_so.c", "-o", "lib/libfoo.so"],
            [CC, "-O1", "-fPIC", "-shared", "bar.c", "-o", "solib/libbar.so"],
        ]
        for cmd in steps:
            r = sh(cmd, cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"fixture: {' '.join(cmd)}: {r.stderr.decode()[:200]}")

        def link(lccc, flags, out):
            shim = ["-B" + os.path.join(td, "shim")] if lccc else []
            return sh([CC, "-no-pie"] + shim + flags + ["-o", out], cwd=td)

        def shape(out):
            ph = sh(["readelf", "-lW", out], cwd=td).stdout.decode()
            d = sh(["readelf", "-dW", out], cwd=td).stdout.decode()
            return ("INTERP" in ph,
                    sorted(re.findall(r"NEEDED\)\s+Shared library: \[([^\]]+)\]", d)))

        flags = ["main.o", "-Llib", "-Wl,-Bstatic", "-lfoo", "-Wl,-Bdynamic"]
        for lccc, out in ((False, "ref"), (True, "new")):
            r = link(lccc, flags, out)
            if r.returncode != 0:
                who = "lccc-ld" if lccc else "GNU ld"
                return Result(name, "FAIL", f"{who} link failed: {r.stderr.decode()[:300]}")
        if shape("new") != shape("ref"):
            return Result(name, "FAIL", f"(PT_INTERP, DT_NEEDED) {shape('new')} != GNU ld {shape('ref')}")
        if not shape("new")[0]:
            return Result(name, "FAIL", "-Bstatic produced a static executable")
        run = sh([os.path.join(td, "new")], cwd=td)
        if run.returncode != 0 or run.stdout != b"1\n":
            return Result(name, "FAIL", f"run: rc={run.returncode} out={run.stdout!r} (libfoo.a gives 1)")

        # A -Bstatic search skips a directory holding only libbar.so.
        bad = ["mbar.o", "-Lsolib", "-Wl,-Bstatic", "-lbar", "-Wl,-Bdynamic"]
        if link(False, bad, "bad_ref").returncode == 0:
            return Result(name, "FAIL", "fixture: GNU ld resolved -Bstatic -lbar to a shared object")
        r = link(True, bad, "bad_new")
        if r.returncode == 0:
            return Result(name, "FAIL", "lccc-ld resolved -Bstatic -lbar to libbar.so")
        if b"-lbar" not in r.stderr:
            return Result(name, "FAIL", f"diagnostic does not name -lbar: {r.stderr.decode()[:200]}")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _as_needed_positional_test(args, oracles):
    """--as-needed / --no-as-needed are positional, and default is no-as-needed.

    Three cases, each compared against GNU ld's DT_NEEDED set:
      A  unused libm AFTER --as-needed        -> dropped
      B  unused libm under --no-as-needed     -> KEPT
      C  unused libm with no flag at all      -> KEPT (GNU default)

    B and C are the ones that matter: linking a library purely for the side
    effects of its ELF constructors is a real pattern, and silently dropping
    the entry changes program behaviour with no diagnostic.
    """
    name = "as_needed_positional_dt_needed"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "t.c"), "w") as f:
            f.write('#include <stdio.h>\nint main(void){ printf("x\\n"); return 0; }\n')
        r = sh([CC, "-c", "-O1", "t.c", "-o", "t.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:150])

        def gpf(n):
            return sh([CC, "-print-file-name=" + n], cwd=td).stdout.decode().strip()
        crt1, crti, cbeg = gpf("crt1.o"), gpf("crti.o"), gpf("crtbegin.o")
        cend, crtn = gpf("crtend.o"), gpf("crtn.o")
        libc_so = gpf("libc.so")
        if not os.path.isabs(crt1) or not os.path.isabs(libc_so):
            return Result(name, "SKIP", "cannot locate CRT objects")
        libdir = os.path.dirname(libc_so)
        base = ["-L" + libdir, "-dynamic-linker", "/lib64/ld-linux-x86-64.so.2",
                crt1, crti, cbeg, "t.o"]
        tail = [cend, crtn]
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")

        def needed(linker, flags, out):
            r = sh([linker] + base + flags + ["-lc"] + tail + ["-o", out], cwd=td)
            if r.returncode != 0:
                return None
            d = sh(["readelf", "-dW", out], cwd=td).stdout.decode()
            return sorted(re.findall(r"NEEDED\)\s+Shared library: \[([^\]]+)\]", d))

        cases = [
            ("A_as_needed",    ["--as-needed", "-lm", "--no-as-needed"]),
            ("B_no_as_needed", ["--no-as-needed", "-lm"]),
            ("C_default",      ["-lm"]),
        ]
        for label, flags in cases:
            got = needed(lccc_ld, flags, "o." + label)
            want = needed("ld", flags, "b." + label)
            if want is None:
                continue          # oracle cannot link here; skip this case
            if got is None:
                return Result(name, "FAIL", f"{label}: lccc-ld failed to link")
            if got != want:
                return Result(name, "FAIL",
                    f"{label}: lccc DT_NEEDED {got} != GNU ld {want}")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _script_as_needed_input_test(args, oracles):
    """A linker script whose inputs are AS_NEEDED must still be an input.

    The defect: script parsing went through the *filtered* accessor, which
    drops everything inside `AS_NEEDED ( ... )`.  A script whose body is only
    as-needed inputs — the shape distro `libgcc_s_asneeded.so` and `libc.so`
    use — then parsed to "no inputs at all", and the loader rejected the file
    as `not a valid ELF object or archive` instead of loading it.  Four
    subcases, each against GNU ld on the same files:

      all_as_needed   INPUT ( AS_NEEDED ( libfoo.so ) ) — referenced symbol:
                      the DSO must be loaded and named in DT_NEEDED
      unreferenced    same script, symbol not referenced: GNU drops the
                      DT_NEEDED entry, and so must we
      dash_l          GROUP ( AS_NEEDED ( -lfoo ) ) — the -l search path
      missing         a script naming a file that does not exist must name
                      the operand and the script, not silently drop it
      cycle           a script that names itself must end in a bounded
                      diagnostic, not stack exhaustion or a hang
    """
    name = "script_as_needed_input"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "foo.c"), "w") as f:
            f.write("int foo_value(void){ return 41; }\n")
        r = sh([CC, "-shared", "-fPIC", "-O1", "-Wl,-soname,libfoo.so",
                "foo.c", "-o", "libfoo.so"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:150])

        # Uses the library -> the as-needed DSO is needed.
        with open(os.path.join(td, "uses.c"), "w") as f:
            f.write("extern int foo_value(void);\n"
                    "int main(void){ return foo_value() == 41 ? 0 : 1; }\n")
        # Does not use it -> GNU ld drops the DT_NEEDED entry.
        with open(os.path.join(td, "plain.c"), "w") as f:
            f.write("int main(void){ return 0; }\n")
        for src, obj in (("uses.c", "uses.o"), ("plain.c", "plain.o")):
            r = sh([CC, "-c", "-O1", "-fno-pic", src, "-o", obj], cwd=td)
            if r.returncode != 0:
                return Result(name, "SKIP", r.stderr.decode()[:150])

        with open(os.path.join(td, "asneeded.so"), "w") as f:
            f.write("/* GNU ld script */\nINPUT ( AS_NEEDED ( libfoo.so ) )\n")
        with open(os.path.join(td, "asneeded_l.so"), "w") as f:
            f.write("/* GNU ld script */\nGROUP ( AS_NEEDED ( -lfoo ) )\n")
        with open(os.path.join(td, "missing.so"), "w") as f:
            f.write("/* GNU ld script */\nINPUT ( libdefinitelyabsent.so )\n")
        with open(os.path.join(td, "loop.so"), "w") as f:
            f.write("/* GNU ld script */\nINPUT ( loop.so )\n")

        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")

        def needed(linker, script, obj, out):
            r = sh([linker, "-L" + td, "-dynamic-linker",
                    "/lib64/ld-linux-x86-64.so.2", "-pie", obj, script,
                    "-o", out], cwd=td, timeout=60)
            if r.returncode != 0:
                return None
            d = sh(["readelf", "-dW", out], cwd=td).stdout.decode()
            return sorted(re.findall(r"NEEDED\)\s+Shared library: \[([^\]]+)\]", d))

        # 1+2: all-as-needed script, referenced and unreferenced.
        for label, obj in (("all_as_needed", "uses.o"), ("unreferenced", "plain.o")):
            got = needed(lccc_ld, "asneeded.so", obj, f"o.{label}")
            want = needed("ld", "asneeded.so", obj, f"b.{label}")
            if want is None:
                continue
            if got is None:
                return Result(name, "FAIL",
                              f"{label}: lccc-ld rejected an all-AS_NEEDED "
                              f"linker script")
            if got != want:
                return Result(name, "FAIL",
                              f"{label}: DT_NEEDED {got} != GNU ld {want}")

        # 3: the -l form inside AS_NEEDED.
        got = needed(lccc_ld, "asneeded_l.so", "uses.o", "o.dash_l")
        want = needed("ld", "asneeded_l.so", "uses.o", "b.dash_l")
        if want is not None:
            if got is None:
                return Result(name, "FAIL",
                              "dash_l: lccc-ld rejected GROUP ( AS_NEEDED ( -lfoo ) )")
            if got != want:
                return Result(name, "FAIL",
                              f"dash_l: DT_NEEDED {got} != GNU ld {want}")

        # 4: a missing input is reported, not dropped.
        r = sh([lccc_ld, "-L" + td, "plain.o", "missing.so", "-o", "o.missing"],
               cwd=td, timeout=60)
        if r.returncode == 0:
            return Result(name, "FAIL",
                          "missing: a script naming an absent file linked "
                          "instead of reporting it")
        err = r.stderr.decode()
        if "libdefinitelyabsent.so" not in err or "missing.so" not in err:
            return Result(name, "FAIL",
                          f"missing: diagnostic names neither operand nor "
                          f"script: {err[:200]!r}")

        # 5: a script that names another script (real toolchains nest one
        # level; the loader must walk it and keep DT_NEEDED correct).
        with open(os.path.join(td, "inner.so"), "w") as f:
            f.write("/* GNU ld script */\nINPUT ( libfoo.so )\n")
        with open(os.path.join(td, "outer.so"), "w") as f:
            f.write("/* GNU ld script */\nINPUT ( inner.so )\n")
        got = needed(lccc_ld, "outer.so", "uses.o", "o.nested")
        want = needed("ld", "outer.so", "uses.o", "b.nested")
        if want is not None:
            if got is None:
                return Result(name, "FAIL", "nested: lccc-ld rejected a nested script")
            if got != want:
                return Result(name, "FAIL",
                              f"nested: DT_NEEDED {got} != GNU ld {want}")

        # 6: a positional script in a relocatable (-r) link resolves its own
        # -l against -L.  The standalone loader used to receive an empty
        # search path, so the script's library could never be found.
        with open(os.path.join(td, "bar.c"), "w") as f:
            f.write("int bar_value(void){ return 7; }\n")
        with open(os.path.join(td, "needsbar.c"), "w") as f:
            f.write("extern int bar_value(void);\n"
                    "int use_bar(void){ return bar_value(); }\n")
        r = sh([CC, "-c", "-O1", "bar.c", "-o", "bar.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:150])
        sh(["ar", "rcs", "libbar.a", "bar.o"], cwd=td)
        r = sh([CC, "-c", "-O1", "needsbar.c", "-o", "needsbar.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:150])
        with open(os.path.join(td, "uses_bar.so"), "w") as f:
            f.write("/* GNU ld script */\nINPUT ( -lbar )\n")
        r = sh([lccc_ld, "-r", "-L" + td, "needsbar.o", "uses_bar.so",
                "-o", "o.r.o"], cwd=td, timeout=60)
        if r.returncode != 0:
            return Result(name, "FAIL",
                          f"r_link: a positional script's -l did not resolve "
                          f"against -L: {r.stderr.decode()[:200]}")
        syms = sh(["readelf", "-sW", "o.r.o"], cwd=td).stdout.decode()
        if "bar_value" not in syms:
            return Result(name, "FAIL",
                          "r_link: the archive member the script named is "
                          "missing from the relocatable output")
        # Without -L the same link must say which operand is missing rather
        # than produce an object with a silently unresolved library.
        r = sh([lccc_ld, "-r", "needsbar.o", "uses_bar.so", "-o", "o.r2.o"],
               cwd=td, timeout=60)
        if r.returncode == 0 or "-lbar" not in r.stderr.decode():
            return Result(name, "FAIL",
                          f"r_link: without -L expected a diagnostic naming "
                          f"-lbar, got rc={r.returncode} "
                          f"{r.stderr.decode()[:150]!r}")

        # 7: a self-referencing script terminates with a bounded diagnostic.
        r = sh([lccc_ld, "-L" + td, "plain.o", "loop.so", "-o", "o.loop"],
               cwd=td, timeout=60)
        if r.returncode == 0:
            return Result(name, "FAIL", "cycle: self-referencing script linked")
        if r.returncode < 0:
            return Result(name, "FAIL",
                          f"cycle: killed by signal {-r.returncode} "
                          f"(stack exhaustion?) instead of a diagnostic")
        if "nested more than" not in r.stderr.decode():
            return Result(name, "FAIL",
                          f"cycle: no nesting diagnostic: "
                          f"{r.stderr.decode()[:200]!r}")
        return Result(name, "PASS")
    except subprocess.TimeoutExpired:
        return Result(name, "FAIL", "cycle: link did not terminate")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _print_map_test(args, oracles):
    """--print-map and -M must write a map to stdout, not silently do nothing.

    Both spellings were accepted and ignored, so a build system asking for a
    map got a successful link and an empty file.
    """
    name = "print_map_to_stdout"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "t.c"), "w") as f:
            f.write("int data_sym = 7;\nint main(void){ return data_sym; }\n")
        r = sh([CC, "-c", "-O1", "t.c", "-o", "t.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:150])
        for flag in ("--print-map", "-M"):
            r = sh([args.lccc, "t.o", "-o", "out." + flag.strip("-"),
                    "-Wl," + flag], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL",
                    f"{flag}: link failed: {r.stderr.decode()[:200]}")
            out = r.stdout.decode()
            if not out.strip():
                return Result(name, "FAIL",
                    f"{flag} produced no map on stdout (silently ignored)")
            # GNU map files carry these section headings.
            for marker in ("Memory Configuration", "Linker script and memory map"):
                if marker not in out:
                    return Result(name, "FAIL",
                        f"{flag}: map missing '{marker}' heading")
            if "data_sym" not in out:
                return Result(name, "FAIL", f"{flag}: map does not mention data_sym")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _section_header_index_consistency_test(args, oracles):
    """Every symbol's st_shndx must name the section it actually lives in.

    The section-header order is produced by one walk, and .symtab/.strtab sit
    immediately after the output sections. That numbering used to be computed
    twice -- once to record each section's index, once to derive symtab's --
    and the two copies had to be edited in lockstep. A drift between them does
    not fail to link: it produces symbols pointing at the wrong section, which
    silently breaks debuggers, `perf`, and anything that resolves an address to
    a name.

    This checks the invariant end to end with readelf: for a handful of known
    symbols, the section named by st_shndx must be the one whose address range
    contains the symbol's value.
    """
    name = "section_header_index_consistency"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "t.c"), "w") as f:
            f.write(r"""
#include <stdio.h>
int g_data = 7;
const int g_ro = 9;
int g_bss;
__thread int g_tls = 3;
static int s_fn(void){ return g_data + g_ro; }
int main(void){ g_bss = s_fn() + g_tls; printf("%d\n", g_bss); return 0; }
""")
        r = sh([CC, "-c", "-O1", "t.c", "-o", "t.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:150])
        r = sh([args.lccc, "t.o", "-o", "out"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"link failed: {r.stderr.decode()[:250]}")

        secs = sh(["readelf", "-SW", "out"], cwd=td).stdout.decode()
        # index -> (name, addr, size, is_nobits, is_tls)
        table = {}
        for line in secs.splitlines():
            m = re.match(r"\s*\[\s*(\d+)\]\s+(\S+)\s+(\S+)\s+([0-9a-f]+)\s+"
                         r"([0-9a-f]+)\s+([0-9a-f]+)\s+(\S+)\s*(\S*)", line)
            if not m:
                continue
            idx, nm, typ, addr, off, size, es, flags = m.groups()
            table[int(idx)] = (nm, int(addr, 16), int(size, 16),
                               typ == "NOBITS", "T" in (flags or ""))

        syms = sh(["readelf", "-sW", "out"], cwd=td).stdout.decode()
        checked = 0
        for line in syms.splitlines():
            parts = line.split()
            if len(parts) < 8 or not parts[0].endswith(":"):
                continue
            value, styp, ndx, nm = parts[1], parts[3], parts[6], parts[7]
            if nm not in ("g_data", "g_ro", "g_bss", "main", "s_fn"):
                continue
            if not ndx.isdigit():
                continue
            i = int(ndx)
            if i not in table:
                return Result(name, "FAIL",
                    f"{nm}: st_shndx={i} names no section header")
            snm, saddr, ssize, _nobits, _tls = table[i]
            v = int(value, 16)
            if not (saddr <= v <= saddr + max(ssize, 1)):
                return Result(name, "FAIL",
                    f"{nm} at {v:#x} claims section [{i}] {snm} "
                    f"({saddr:#x}..{saddr + ssize:#x}): header index drifted")
            checked += 1
        if checked < 3:
            # A drift in the header numbering shows up here as symbols whose
            # st_shndx no longer names a section we can match. Treating that as
            # SKIP would let exactly the regression this test exists for pass
            # silently, so it is a failure.
            return Result(name, "FAIL",
                f"only {checked} of the 5 expected symbols resolved to a "
                f"section containing their value; header indices likely drifted")

        # .symtab's sh_link must name the real .strtab, which is the other half
        # of the numbering that used to be computed separately.
        link_ok = False
        for line in secs.splitlines():
            m = re.match(r"\s*\[\s*(\d+)\]\s+\.symtab\s", line)
            if m:
                det = sh(["readelf", "-SW", "--wide", "out"], cwd=td).stdout.decode()
                for l2 in det.splitlines():
                    if re.match(r"\s*\[\s*\d+\]\s+\.strtab\s", l2):
                        link_ok = True
        if not link_ok:
            return Result(name, "FAIL", "no .strtab alongside .symtab")

        code, out = run_bin(os.path.join(td, "out"), [], td)
        if code != 0:
            return Result(name, "FAIL", f"binary exited {code}")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _comdat_dedup_test(args, oracles):
    """COMDAT groups: the losing bodies must not be laid out.

    C++ emits one definition of every inline function and template
    instantiation in EVERY translation unit that uses it, marked as an
    interchangeable SHT_GROUP/GRP_COMDAT set. Symbol resolution alone picks a
    single winner, so the program runs correctly and `nm` agrees with GNU ld --
    but without group dedup the losing section BODIES are still emitted as dead
    bytes nothing can reach.

    Symbol counts therefore cannot detect this. The test searches the linked
    image for the exact byte pattern of a COMDAT function and requires exactly
    one occurrence, which is what GNU ld produces.
    """
    name = "comdat_group_dedup"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "h.hpp"), "w") as f:
            f.write("#pragma once\n#include <cstdio>\n"
                    "template <typename T> struct Widget {\n"
                    "  T v;\n  Widget(T x): v(x) {}\n"
                    "  T twice() const { return v + v; }\n"
                    "  void show() const { printf(\"%ld\\n\", (long)twice()); }\n"
                    "};\n"
                    "inline int shared_inline(int a){ return a * 3 + 1; }\n")
        for tu, body in (
            ("a", 'int use_a(){ Widget<long> w(21); w.show(); return shared_inline(1); }'),
            ("b", 'int use_b(){ Widget<long> w(10); w.show(); return shared_inline(2); }'),
            ("m", 'int use_a(); int use_b();\n'
                  'int main(){ Widget<long> w(1); w.show(); return use_a()+use_b(); }'),
        ):
            with open(os.path.join(td, tu + ".cpp"), "w") as f:
                f.write('#include "h.hpp"\n' + body + "\n")
        # -O0 -fno-inline keeps the COMDAT groups from being optimised away.
        objs = []
        for tu in ("a", "b", "m"):
            r = sh([CXX, "-O0", "-fno-inline", "-c", tu + ".cpp", "-o", tu + ".o"], cwd=td)
            if r.returncode != 0:
                return Result(name, "SKIP", r.stderr.decode()[:150])
            objs.append(tu + ".o")

        # Locate the exact bytes of Widget<long>::twice in one input object.
        secs = sh(["readelf", "-SW", "a.o"], cwd=td).stdout.decode()
        target = None
        for line in secs.splitlines():
            m = re.match(r"\s*\[\s*\d+\]\s+(\S+)\s+\S+\s+[0-9a-f]+\s+([0-9a-f]+)\s+([0-9a-f]+)", line)
            if m and m.group(1) == ".text._ZNK6WidgetIlE5twiceEv":
                off, size = int(m.group(2), 16), int(m.group(3), 16)
                with open(os.path.join(td, "a.o"), "rb") as fh:
                    target = fh.read()[off:off + size]
                break
        if not target or len(target) < 8:
            return Result(name, "SKIP", "could not locate the COMDAT body in a.o")

        r = sh([args.lccc] + objs + ["-o", "out.lccc", "-lstdc++", "-lm"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc link failed: {r.stderr.decode()[:300]}")
        with open(os.path.join(td, "out.lccc"), "rb") as fh:
            got = fh.read().count(target)
        if got != 1:
            return Result(name, "FAIL",
                f"COMDAT body appears {got} times in the image (want 1): "
                f"duplicate group members were laid out as dead bytes")

        # Behaviour must match GNU ld's link of the same objects.
        code, out = run_bin(os.path.join(td, "out.lccc"), [], td)
        r2 = sh([CXX] + objs + ["-o", "out.bfd"], cwd=td)
        if r2.returncode == 0:
            code2, out2 = run_bin(os.path.join(td, "out.bfd"), [], td)
            if (code, out) != (code2, out2):
                return Result(name, "FAIL",
                    f"lccc {(code, out)!r} != g++/bfd {(code2, out2)!r}")
            with open(os.path.join(td, "out.bfd"), "rb") as fh:
                want = fh.read().count(target)
            if want != got:
                return Result(name, "FAIL",
                    f"lccc keeps {got} copies, GNU ld keeps {want}")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _multi_version_node_test(args, oracles):
    """A version script with several nodes must export ALL of them.

    `LIBV_2.0 { global: new_fn; } LIBV_1.0;` also declares an inheritance edge:
    a LIBV_2.0 provider satisfies a LIBV_1.0 dependency. Only the first node
    used to be parsed, so new_fn vanished from .dynsym and the hierarchy was
    lost -- a library that looks versioned but cannot bind an older consumer.
    """
    name = "version_script_multiple_nodes"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "v.c"), "w") as f:
            f.write("int old_fn(void){ return 1; }\nint new_fn(void){ return 2; }\n"
                    "int hidden_helper(void){ return 3; }\n")
        with open(os.path.join(td, "v.map"), "w") as f:
            f.write("LIBV_1.0 { global: old_fn; local: *; };\n"
                    "LIBV_2.0 { global: new_fn; } LIBV_1.0;\n")
        r = sh([CC, "-c", "-O1", "-fPIC", "v.c", "-o", "v.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:150])
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        r = sh([lccc_ld, "-shared", "--version-script=v.map", "v.o",
                "-o", "v.so"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc-ld failed: {r.stderr.decode()[:300]}")
        dyn = sh(["readelf", "--dyn-syms", "-W", "v.so"], cwd=td).stdout.decode()
        if "old_fn@@LIBV_1.0" not in dyn:
            return Result(name, "FAIL", "old_fn@@LIBV_1.0 missing from .dynsym")
        if "new_fn@@LIBV_2.0" not in dyn:
            return Result(name, "FAIL",
                "new_fn@@LIBV_2.0 missing: only the first version node was parsed")
        if "hidden_helper" in dyn:
            return Result(name, "FAIL", "local: * did not hide hidden_helper")
        ver = sh(["readelf", "-VW", "v.so"], cwd=td).stdout.decode()
        if "Parent 1: LIBV_1.0" not in ver:
            return Result(name, "FAIL",
                "verdef lost the parent link; a LIBV_2.0 provider would not "
                "satisfy a LIBV_1.0 dependency")
        # The library must actually load and run.
        with open(os.path.join(td, "use.c"), "w") as f:
            f.write("#include <stdio.h>\nextern int old_fn(void), new_fn(void);\n"
                    "int main(void){ printf(\"%d %d\\n\", old_fn(), new_fn()); return 0; }\n")
        r = sh([CC, "-O1", "use.c", os.path.join(td, "v.so"),
                "-Wl,-rpath," + td, "-o", "use.bin"], cwd=td)
        if r.returncode == 0:
            code, out = run_bin(os.path.join(td, "use.bin"), [], td)
            if (code, out) != (0, "1 2\n"):
                return Result(name, "FAIL",
                    f"versioned .so did not run correctly: {(code, out)!r}")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _script_overlay_test(args, oracles):
    """OVERLAY: members share a VMA while their load images stay distinct.

    Checked by running the program and measuring the gap between
    __load_start_ovl1 and __load_start_ovl2. A layout that gives the members
    distinct VMAs, or that collapses their LMAs onto each other, both produce a
    file that looks fine to readelf and a runtime copy routine that clobbers
    the wrong bytes.
    """
    name = "script_overlay_shared_vma"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "t.c"), "w") as f:
            f.write(r"""
__attribute__((section(".ovl1"), used)) int o1v = 0x1111;
__attribute__((section(".ovl2"), used)) int o2v = 0x2222;
extern char __load_start_ovl1[], __load_stop_ovl1[], __load_start_ovl2[];
static long wr(int fd, const void *b, unsigned long n){
    long r; __asm__ volatile("syscall" : "=a"(r)
        : "a"(1L), "D"((long)fd), "S"(b), "d"(n) : "rcx","r11","memory");
    return r;
}
void my_start(void){
    long gap = __load_start_ovl2 - __load_start_ovl1;
    long sz  = __load_stop_ovl1  - __load_start_ovl1;
    int ok = (gap == 4) && (sz == 4);
    char m[2] = { (char)('0' + ok), '\n' };
    wr(1, m, 2);
    __asm__ volatile("syscall" :: "a"(60L), "D"(0L));
    __builtin_unreachable();
}
""")
        with open(os.path.join(td, "t.lds"), "w") as f:
            f.write("ENTRY(my_start)\n"
                    "SECTIONS {\n"
                    "  . = 0x400000 + SIZEOF_HEADERS;\n"
                    "  .text : { *(.text*) }\n"
                    "  OVERLAY 0x500000 : AT (0x480000) {\n"
                    "    .ovl1 { *(.ovl1) }\n"
                    "    .ovl2 { *(.ovl2) }\n"
                    "  }\n"
                    "  .data : { *(.data*) *(.rodata*) *(.bss*) }\n"
                    "  /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }\n"
                    "}\n")
        r = sh([CC, "-c", "-O1", "-ffreestanding", "-fno-pic",
                "-fno-asynchronous-unwind-tables", "-fno-stack-protector",
                "t.c", "-o", "t.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:150])
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        r = sh([lccc_ld, "-T", "t.lds", "t.o", "-o", "out.lccc"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc-ld failed: {r.stderr.decode()[:300]}")
        # Both members must report the same VMA.
        secs = sh(["readelf", "-SW", "out.lccc"], cwd=td).stdout.decode()
        addrs = {}
        for line in secs.splitlines():
            for nm in (".ovl1", ".ovl2"):
                if f" {nm} " in line:
                    parts = line.split()
                    addrs[nm] = parts[parts.index("PROGBITS") + 1]
        if len(addrs) == 2 and addrs[".ovl1"] != addrs[".ovl2"]:
            return Result(name, "FAIL",
                f"overlay members have different VMAs: {addrs}")
        code, out = run_bin(os.path.join(td, "out.lccc"), [], td)
        if (code, out) != (0, "1\n"):
            return Result(name, "FAIL",
                f"overlay load addresses wrong at runtime: {(code, out)!r}")
        r2 = sh(["ld", "-T", "t.lds", "t.o", "-o", "out.ld"], cwd=td)
        if r2.returncode == 0:
            code2, out2 = run_bin(os.path.join(td, "out.ld"), [], td)
            if (code, out) != (code2, out2):
                return Result(name, "FAIL",
                    f"lccc {(code, out)!r} != GNU ld {(code2, out2)!r}")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _script_tls_gd_ld_test(args, oracles):
    """General-Dynamic and Local-Dynamic TLS relaxation in the -T path.

    A linker-script link is static with a fixed TLS block, so __tls_get_addr
    can never be reached and GNU ld relaxes GD/LD sequences to Local-Exec.
    Rejecting them made any -fPIC object using the default TLS model
    unlinkable through a script.

    The subtle half is DTPOFF: after LD -> LE the sequence leaves the THREAD
    POINTER in %rax, not the module base, so DTPOFF must be TP-relative. Get
    that wrong and you emit `cmpl $0xb,0x4(%rax)` where GNU ld emits
    `cmpl $0xb,-0x4(%rax)` -- every Local-Dynamic access reads the wrong side
    of the thread pointer. The link succeeds and the values are garbage, so
    this test builds a real TCB and checks the values at runtime.
    """
    name = "script_tls_gd_ld_relaxation"
    src = r"""
__thread int gd_a = 11;
__thread int gd_b = 22;
static long sys_write(const void *b, unsigned long n){
  long r; __asm__ volatile("syscall":"=a"(r)
      :"a"(1L),"D"(1L),"S"(b),"d"(n):"rcx","r11","memory");
  return r;
}
static long set_fs(void *p){
  long r; __asm__ volatile("syscall":"=a"(r)
      :"a"(158L),"D"(0x1002L),"S"(p):"rcx","r11","memory");
  return r;
}
extern char __tdata_start[], __tdata_end[], __tls_size_sym[];
static char tls_area[512] __attribute__((aligned(64)));
void my_start(void){
  unsigned long tls_size = (unsigned long)__tls_size_sym;
  /* The ABI requires *(void**)tp == tp; the TLS block sits just below tp. */
  char *tp = tls_area + 256;
  *(void **)tp = tp;
  const char *s = __tdata_start;
  unsigned long init = (unsigned long)(__tdata_end - __tdata_start);
  for (unsigned long i = 0; i < tls_size; i++)
      (tp - tls_size)[i] = i < init ? s[i] : 0;
  set_fs(tp);
  int ok = (gd_a == 11) && (gd_b == 22);
  char m[2] = { (char)('0' + ok), '\n' };
  sys_write(m, 2);
  __asm__ volatile("syscall"::"a"(60L),"D"((long)(ok ? 0 : 1)));
  __builtin_unreachable();
}
"""
    script = ("ENTRY(my_start)\n"
              "SECTIONS {\n"
              "  . = 0x400000 + SIZEOF_HEADERS;\n"
              "  .text : { *(.text*) }\n"
              "  . = ALIGN(64);\n"
              "  __tdata_start = .;\n"
              "  .tdata : { *(.tdata*) }\n"
              "  __tdata_end = .;\n"
              "  .tbss : { *(.tbss*) }\n"
              "  __tls_size_sym = SIZEOF(.tdata) + SIZEOF(.tbss);\n"
              "  .data : { *(.data*) *(.rodata*) *(.bss*) }\n"
              "  /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }\n"
              "}\n")
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "t.c"), "w") as f:
            f.write(src)
        with open(os.path.join(td, "t.lds"), "w") as f:
            f.write(script)
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        for model, expect_reloc in (("global-dynamic", "TLSGD"),
                                    ("local-dynamic", "TLSLD")):
            obj = "t." + model + ".o"
            r = sh([CC, "-c", "-O1", "-fPIC", "-ftls-model=" + model,
                    "-ffreestanding", "-fno-stack-protector",
                    "t.c", "-o", obj], cwd=td)
            if r.returncode != 0:
                return Result(name, "SKIP", r.stderr.decode()[:150])
            rel = sh(["readelf", "-rW", obj], cwd=td).stdout.decode()
            if expect_reloc not in rel:
                # The compiler optimised the model away; nothing to test here.
                continue
            out_b = "out." + model
            r = sh([lccc_ld, "-T", "t.lds", obj, "-o", out_b], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL",
                    f"{model}: lccc-ld failed: {r.stderr.decode()[:300]}")
            code, out = run_bin(os.path.join(td, out_b), [], td)
            if (code, out) != (0, "1\n"):
                return Result(name, "FAIL",
                    f"{model}: relaxed TLS wrong at runtime: {(code, out)!r} "
                    f"(want (0, '1\\n'))")
            # GNU ld must agree.
            r2 = sh(["ld", "-T", "t.lds", obj, "-o", "ref." + model], cwd=td)
            if r2.returncode == 0:
                code2, out2 = run_bin(os.path.join(td, "ref." + model), [], td)
                if (code, out) != (code2, out2):
                    return Result(name, "FAIL",
                        f"{model}: lccc {(code, out)!r} != GNU ld {(code2, out2)!r}")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _script_tls_test(args, oracles):
    """Initial-Exec TLS through -T, verified by running the program.

    Two things must both be right. The TPOFF32 value is measured from the END
    of the TLS block (%fs:0 points past it on x86-64), so an off-by-block-size
    error still produces a plausible-looking negative offset. And a script that
    places .tdata/.tbss without declaring PT_TLS needs one synthesised, or the
    loader never allocates the thread block and every %fs access reads whatever
    precedes the TCB. Structural checks miss both; executing does not.
    """
    name = "script_tls_initial_exec"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "t.c"), "w") as f:
            f.write(r"""
__thread int tv1 = 5;
__thread int tv2 = 37;
static long wr(int fd, const void *b, unsigned long n){
    long r; __asm__ volatile("syscall" : "=a"(r)
        : "a"(1L), "D"((long)fd), "S"(b), "d"(n) : "rcx","r11","memory");
    return r;
}
void my_start(void){
    extern char __tls_end[];
    long r;
    /* arch_prctl(ARCH_SET_FS, __tls_end) */
    __asm__ volatile("syscall" : "=a"(r)
        : "a"(158L), "D"(0x1002L), "S"((long)__tls_end) : "rcx","r11","memory");
    int ok = (tv1 == 5) && (tv2 == 37);
    char m[2] = { (char)('0' + ok), '\n' };
    wr(1, m, 2);
    __asm__ volatile("syscall" :: "a"(60L), "D"(0L));
    __builtin_unreachable();
}
""")
        with open(os.path.join(td, "t.lds"), "w") as f:
            f.write("ENTRY(my_start)\n"
                    "SECTIONS {\n"
                    "  . = 0x400000 + SIZEOF_HEADERS;\n"
                    "  .text : { *(.text*) }\n"
                    "  . = ALIGN(8);\n"
                    "  .tdata : { *(.tdata*) }\n"
                    "  .tbss : { *(.tbss*) }\n"
                    "  __tls_end = .;\n"
                    "  .data : { *(.data*) }\n"
                    "  .bss : { *(.bss*) *(COMMON) }\n"
                    "  /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }\n"
                    "}\n")
        r = sh([CC, "-c", "-O1", "-fno-pic", "-ffreestanding",
                "-fno-asynchronous-unwind-tables", "-fno-stack-protector",
                "t.c", "-o", "t.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:150])
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        r = sh([lccc_ld, "-T", "t.lds", "t.o", "-o", "out.lccc"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc-ld failed: {r.stderr.decode()[:300]}")
        # PT_TLS must exist with memsz covering .tdata + .tbss.
        seg = sh(["readelf", "-lW", "out.lccc"], cwd=td).stdout.decode()
        if "TLS" not in seg:
            return Result(name, "FAIL",
                "no PT_TLS program header: the loader will not allocate the "
                "thread block even though the TPOFF values are correct")
        code, out = run_bin(os.path.join(td, "out.lccc"), [], td)
        if (code, out) != (0, "1\n"):
            return Result(name, "FAIL",
                f"TLS access wrong at runtime: got {(code, out)!r}, want (0, '1\\n')")
        r2 = sh(["ld", "-T", "t.lds", "t.o", "-o", "out.ld"], cwd=td)
        if r2.returncode == 0:
            code2, out2 = run_bin(os.path.join(td, "out.ld"), [], td)
            if (code, out) != (code2, out2):
                return Result(name, "FAIL",
                    f"lccc {(code, out)!r} != GNU ld {(code2, out2)!r}")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _script_hidden_visibility_test(args, oracles):
    """STV_HIDDEN symbols must never reach .dynsym, even under `global: *`.

    Visibility is an ABI decision made by the compiler; a version script's
    glob cannot override it. Getting this wrong exports every internal helper
    of a script-linked shared object (the vDSO included), which both bloats
    the dynamic symbol table and lets external code bind to symbols that were
    never part of the contract. GNU ld applies visibility first; so must we.
    """
    name = "script_hidden_not_exported"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "t.c"), "w") as f:
            f.write(
                '__attribute__((visibility("hidden"))) int hidden_fn(void){ return 1; }\n'
                '__attribute__((visibility("internal"))) int internal_fn(void){ return 2; }\n'
                '__attribute__((visibility("protected"))) int protected_fn(void){ return 3; }\n'
                'int public_fn(void){ return hidden_fn() + internal_fn() + protected_fn(); }\n')
        # `global: *` deliberately globs everything: the point of the test is
        # that STV_HIDDEN still wins over the glob.
        with open(os.path.join(td, "t.lds"), "w") as f:
            f.write(VDSO_SCRIPT.replace(
                "LCCC_VDSO_1 { global: vdso_answer; local: *; };",
                "LCCC_VDSO_1 { global: *; };"))
        r = sh([CC, "-c", "-O1", "-fPIC", "-fno-asynchronous-unwind-tables",
                "t.c", "-o", "t.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:150])
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        r = sh([lccc_ld, "-shared", "--hash-style=both", "-soname", "vis.so.1",
                "-T", "t.lds", "t.o", "-o", "out.so"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc-ld failed: {r.stderr.decode()[:300]}")
        dyn = sh(["readelf", "--dyn-syms", "-W", "out.so"], cwd=td).stdout.decode()
        for bad in ("hidden_fn", "internal_fn"):
            if bad in dyn:
                return Result(name, "FAIL",
                    f"{bad} is STV_HIDDEN/INTERNAL but appears in .dynsym; "
                    f"visibility must be applied before the version script glob")
        if "public_fn" not in dyn:
            return Result(name, "FAIL", "public_fn missing from .dynsym")
        # STV_PROTECTED is still exported (it only forbids preemption).
        if "protected_fn" not in dyn:
            return Result(name, "FAIL", "protected_fn must remain exported")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _script_pic_gotpcrel_test(args, oracles):
    """-fPIC objects must link through -T, with GOT references relaxed.

    A linker-script link has no GOT, so every `sym@GOTPCREL` must be rewritten
    into direct addressing. The dangerous failure is not an error but a wrong
    relaxation: `mov sym@GOTPCREL(%rip),%rax` LOADS the slot, so pointing it at
    the symbol yields the bytes stored there rather than its address. This test
    runs the program and compares a pointer and a value, which is the only way
    to tell a correct relaxation from a plausible one.
    """
    name = "script_pic_gotpcrel_relaxation"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "t.c"), "w") as f:
            f.write(r"""
int extdata = 1234;
__attribute__((noinline)) int *get_ptr(void){ return &extdata; }
__attribute__((noinline)) int  get_val(void){ return extdata; }
void my_start(void){
    int ok = (get_ptr() == &extdata) && (get_val() == 1234);
    __asm__ volatile("syscall" :: "a"(60L), "D"((long)(ok ? 0 : 1)));
    __builtin_unreachable();
}
""")
        with open(os.path.join(td, "t.lds"), "w") as f:
            f.write("ENTRY(my_start)\n"
                    "SECTIONS {\n"
                    "  . = 0x400000 + SIZEOF_HEADERS;\n"
                    "  .text : { *(.text*) }\n"
                    "  .rodata : { *(.rodata*) }\n"
                    "  .data : { *(.data*) }\n"
                    "  .bss : { *(.bss*) *(COMMON) }\n"
                    "  /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) "
                    "*(.got) *(.got.plt) }\n"
                    "}\n")
        # -fPIC is what makes the compiler emit GOTPCREL against extdata.
        r = sh([CC, "-c", "-O1", "-fPIC", "-fno-asynchronous-unwind-tables",
                "-fno-stack-protector", "t.c", "-o", "t.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:150])
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        r = sh([lccc_ld, "-T", "t.lds", "t.o", "-o", "out.lccc"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc-ld failed: {r.stderr.decode()[:300]}")
        code, _ = run_bin(os.path.join(td, "out.lccc"), [], td)
        if code != 0:
            return Result(name, "FAIL",
                f"relaxed binary exited {code}: the GOT reference was rewritten "
                f"to load the symbol's bytes instead of its address")
        # Cross-check against GNU ld where available.
        r2 = sh(["ld", "-T", "t.lds", "t.o", "-o", "out.ld"], cwd=td)
        if r2.returncode == 0:
            code2, _ = run_bin(os.path.join(td, "out.ld"), [], td)
            if code2 != code:
                return Result(name, "FAIL", f"lccc exit {code} != GNU ld {code2}")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _vdso_script_test(args, oracles):
    """Exercise linker-created dynamic/version metadata and FILEHDR PHDRS."""
    name = "script_vdso_dynamic"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "t.c"), "w") as f:
            f.write("int vdso_answer(void){return 42;}\n"
                    "int hidden_answer(void){return 7;}\n")
        with open(os.path.join(td, "t.lds"), "w") as f:
            f.write(VDSO_SCRIPT)
        r = sh([CC, "-c", "-O1", "-fPIC", "-fno-asynchronous-unwind-tables",
                "t.c", "-o", "t.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:150])
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        common = ["-shared", "--hash-style=both", "-Bsymbolic", "-soname",
                  "linux-vdso-test.so.1", "-z", "max-page-size=4096",
                  "-T", "t.lds", "t.o"]
        r = sh([lccc_ld] + common + ["-o", "out.lccc.so"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc-ld failed: {r.stderr.decode()[:300]}")

        headers = sh(["readelf", "-h", "-l", "-d", "-W", "out.lccc.so"],
                     cwd=td).stdout.decode()
        dynsyms = sh(["readelf", "--dyn-syms", "-W", "out.lccc.so"],
                     cwd=td).stdout.decode()
        required = ("DYN (Shared object file)", "SONAME", "HASH", "GNU_HASH",
                    "STRTAB", "SYMTAB", "VERSYM", "VERDEF", "VERDEFNUM")
        missing = [token for token in required if token not in headers]
        load_lines = [line.split() for line in headers.splitlines()
                      if re.match(r"^\s*LOAD\s", line)]
        if missing or not load_lines or load_lines[0][1:4] != ["0x000000", "0x0000000000000000",
                                                                "0x0000000000000000"]:
            return Result(name, "FAIL",
                          f"bad ELF structure; missing={missing}, LOAD={load_lines[:1]}")
        if "vdso_answer@@LCCC_VDSO_1" not in dynsyms or "hidden_answer" in dynsyms:
            return Result(name, "FAIL", "version-script exports are incorrect")

        # This also validates both hash tables through the system dynamic
        # loader rather than trusting readelf's structural display alone.
        lib = ctypes.CDLL(os.path.join(td, "out.lccc.so"))
        lib.vdso_answer.restype = ctypes.c_int
        if lib.vdso_answer() != 42:
            return Result(name, "FAIL", "dynamic lookup returned the wrong function")
        try:
            getattr(lib, "hidden_answer")
            return Result(name, "FAIL", "local:* symbol remained dynamically visible")
        except AttributeError:
            pass
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)

def _vdso_note_phdr_test(args, oracles):
    """Declared-PHDRS scripts emit exactly the declared headers (GNU parity).

    Regression test for the linux-6.18.52 vDSO blocker: the vDSO script
    declares `note PT_NOTE` in its PHDRS clause while also placing a real
    allocated `.note` section.  lccc-ld used to count an auto PT_NOTE for the
    section on top of the declared one, so e_phnum/file-offset seeding
    disagreed with the headers actually written.  The vDSO then got a phantom
    trailing NULL phdr and -- because SIZEOF_HEADERS seeded 4 headers while
    the file offsets seeded 5 -- the first section's file offset jumped a page
    (0x120 -> 0x1120), leaving the sole PT_LOAD with p_filesz > p_memsz.
    vdso2c rejects such an image ("cannot handle memsz != filesz") and the
    kernel build died at arch/x86/entry/vdso/vdso-image-64.c.

    GNU ld (measured on binutils 2.44) never auto-adds PT_NOTE, PT_GNU_PROPERTY
    or PT_TLS to a script that declares a PHDRS clause; auto-headers exist only
    on the implicit-segment path.  This test pins all of that: exact phdr
    count, filesz == memsz, and an alloc-section address/offset table identical
    to GNU ld's for the same link.
    """
    name = "script_vdso_declared_note_phdrs"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "t.c"), "w") as f:
            f.write("int vdso_answer(void){return 42;}\n"
                    "/* A real allocated note input section, like vdso-note.o. */\n"
                    "__attribute__((used, section(\".note.test\")))\n"
                    "static const struct { unsigned long namesz, descsz, type;\n"
                    "                      char name[8]; } my_note = {\n"
                    "    4, 0, 0x100, { 'T','E','S','T' } };\n")
        with open(os.path.join(td, "t.lds"), "w") as f:
            f.write(VDSO_NOTE_SCRIPT)
        r = sh([CC, "-c", "-O1", "-fPIC", "-fno-asynchronous-unwind-tables",
                "t.c", "-o", "t.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:150])
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        common = ["-shared", "--hash-style=both", "-Bsymbolic", "-soname",
                  "linux-vdso-test.so.1", "-z", "max-page-size=4096",
                  "-T", "t.lds", "t.o"]
        r = sh([lccc_ld] + common + ["-o", "out.lccc.so"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc-ld failed: {r.stderr.decode()[:300]}")

        phdrs = sh(["readelf", "-lW", "out.lccc.so"], cwd=td).stdout.decode()
        # exactly the four declared headers, no phantom NULL, no auto PT_NOTE
        types = [ln.split()[0] for ln in phdrs.splitlines()
                 if re.match(r"^\s*(LOAD|DYNAMIC|NOTE|TLS|GNU_EH_FRAME|GNU_STACK|"
                             r"GNU_RELRO|GNU_PROPERTY|NULL)\s", ln)]
        if types != ["LOAD", "DYNAMIC", "NOTE", "GNU_EH_FRAME"]:
            return Result(name, "FAIL", f"phdr set is {types}, expected the 4 declared")
        # vdso2c invariant: p_filesz must equal p_memsz on the single LOAD and
        # the segment must begin at offset 0 / vaddr 0 (FILEHDR PHDRS).
        load = [ln.split() for ln in phdrs.splitlines()
                if re.match(r"^\s*LOAD\s", ln)][0]
        if load[1] != "0x000000" or load[2] != "0x0000000000000000":
            return Result(name, "FAIL", f"LOAD does not start at 0: {load[:5]}")
        if load[4] != load[5]:
            return Result(name, "FAIL",
                          f"p_filesz {load[4]} != p_memsz {load[5]} (vdso2c rejects this)")
        # readelf itself flags filesz > memsz; assert it stayed silent.
        if "larger than its memory size" in phdrs:
            return Result(name, "FAIL", "readelf reports filesz > memsz")

        # Strongest check that is independent of synthetic dynamic-section
        # sizes (hash bucket counts and dynstr legitimately differ): every
        # allocated section must satisfy offset == vaddr in BOTH outputs.
        # This is exactly the invariant the bug broke -- the first section's
        # offset jumped a page ahead of its vaddr, which no self-consistency
        # check on phdrs alone would catch.
        r2 = sh(["ld"] + common + ["-o", "out.gnu.so"], cwd=td)
        if r2.returncode != 0:
            return Result(name, "SKIP", f"GNU ld oracle failed: {r2.stderr.decode()[:150]}")

        def alloc_table(path):
            out = sh(["readelf", "-SW", path], cwd=td).stdout.decode()
            rows = []
            for ln in out.splitlines():
                m = re.match(r"^\s*\[\s*\d+\]\s+(\S+)\s+(\S+)\s+([0-9a-f]+)\s+"
                             r"([0-9a-f]+)\s+([0-9a-f]+)", ln)
                if m and m.group(1) not in (".symtab", ".strtab", ".shstrtab"):
                    rows.append(m.groups())
            return rows

        for label, path in (("lccc", "out.lccc.so"), ("gnu", "out.gnu.so")):
            for row in alloc_table(path):
                if int(row[2], 16) != int(row[3], 16):
                    return Result(name, "FAIL",
                                  f"{label}: section {row[0]} vaddr {row[2]} "
                                  f"!= offset {row[3]} (page-skewed layout)")
        # and the .note section itself must be allocated and present
        note_row = [r for r in alloc_table("out.lccc.so") if r[0] == ".note"]
        if not note_row:
            return Result(name, "FAIL", "no allocated .note output section")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _elf32_script_test(args, oracles):
    """Linux setup.elf-shaped ELF32 link, including every narrow i386 REL.

    Compare the flat alloc image byte-for-byte with GNU BFD.  This catches both
    relocation formulas and output-class mistakes; merely asking readelf to
    parse the file would not notice a missing LONG signature or a bad addend.
    """
    name = "script_elf32_i386_relocations"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        asm = r'''
            .code32
            .section .text,"ax"
            .globl _start
        _start:
        low_target:
            call target
            .long target
            .word target
            .byte target
            ret
            .section .text16,"ax"
            .code16
            # These displacements are mathematically below -32768, but are
            # valid after the modulo-2^16 wrap specified for R_386_PC16.
            call low_target
            jmp low_target
            .code32
            .section .data,"aw"
            .globl target
        target:
            .long 0x12345678
        '''
        script = r'''
            OUTPUT_FORMAT("elf32-i386")
            OUTPUT_ARCH(i386)
            ENTRY(_start)
            SECTIONS {
              . = 0;
              .text : { *(.text) }
              .data : { *(.data) }
              . = 0x8200;
              .text16 : { *(.text16) }
              .signature : { BYTE(0x12) SHORT(0x3456) LONG(0x5a5aaa55) }
              _end = .;
              /DISCARD/ : { *(.note*) }
            }
        '''
        with open(os.path.join(td, "in.s"), "w") as f:
            f.write(textwrap.dedent(asm))
        with open(os.path.join(td, "setup.ld"), "w") as f:
            f.write(textwrap.dedent(script))
        r = sh(["as", "--32", "-o", "in.o", "in.s"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:200])

        rels = sh(["readelf", "-rW", "in.o"], cwd=td).stdout.decode()
        required_relocs = ("R_386_32", "R_386_PC32", "R_386_16",
                           "R_386_PC16", "R_386_8")
        if any(rel not in rels for rel in required_relocs):
            return Result(name, "SKIP", "assembler did not emit required i386 REL set")

        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        r = sh([lccc_ld, "-m", "elf_i386", "-T", "setup.ld",
                "-o", "out.lccc", "in.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc-ld failed: {r.stderr.decode()[:400]}")
        r = sh(["ld.bfd", "-m", "elf_i386", "-T", "setup.ld",
                "-o", "out.bfd", "in.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", f"bfd unavailable: {r.stderr.decode()[:200]}")

        for stem in ("lccc", "bfd"):
            r = sh(["objcopy", "-O", "binary", f"out.{stem}", f"out.{stem}.bin"], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"objcopy rejected {stem}: {r.stderr.decode()[:200]}")
        with open(os.path.join(td, "out.lccc.bin"), "rb") as f:
            lccc_bytes = f.read()
        with open(os.path.join(td, "out.bfd.bin"), "rb") as f:
            bfd_bytes = f.read()
        if lccc_bytes != bfd_bytes:
            return Result(name, "FAIL",
                          f"flat image differs from bfd ({len(lccc_bytes)} vs {len(bfd_bytes)} bytes)")
        if not lccc_bytes.endswith(bytes.fromhex("12563455aa5a5a")):
            return Result(name, "FAIL", "BYTE/SHORT/LONG signature bytes are absent")

        hdr = sh(["readelf", "-hW", "out.lccc"], cwd=td).stdout.decode()
        required_hdr = ("Class:                             ELF32",
                        "Machine:                           Intel 80386",
                        "Size of this header:               52 (bytes)",
                        "Size of program headers:           32 (bytes)",
                        "Size of section headers:           40 (bytes)")
        missing = [token for token in required_hdr if token not in hdr]
        if missing:
            return Result(name, "FAIL", f"bad ELF32 header; missing={missing}")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


def _elf32_script_gc_keep_test(args, oracles):
    """ELF32 script GC: ENTRY + KEEP roots, transitive REL reachability.

    This is the exact mechanism used by the Linux real-mode setup build:
    functions live in .text.<name>, while boot protocol payloads and registry
    tables have no incoming machine relocation and must be rooted by KEEP.
    """
    name = "script_elf32_gc_keep"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        asm = r'''
            .section .bootproto,"a",@progbits
            .globl bootproto
        bootproto:
            .long 0x13579bdf

            .section .text.start,"ax",@progbits
            .globl _start
        _start:
            call live_fn
            ret

            .section .text.live,"ax",@progbits
            .globl live_fn
        live_fn:
            movl $7, %eax
            ret

            .section .text.kept,"ax",@progbits
            .globl kept_fn
        kept_fn:
            movl $9, %eax
            ret

            .section .text.dead,"ax",@progbits
            .globl dead_fn
        dead_fn:
            movl $0xdead, %eax
            ret

            .section .registry,"a",@progbits
            .long kept_fn
        '''
        script = r'''
            OUTPUT_FORMAT("elf32-i386")
            OUTPUT_ARCH(i386)
            ENTRY(_start)
            SECTIONS {
              . = 0;
              .bootproto : { KEEP(*(.bootproto)) }
              .text : { *(.text.*) }
              .registry : { KEEP(*(.registry)) }
              _end = .;
              /DISCARD/ : { *(.note*) *(.comment) }
            }
        '''
        with open(os.path.join(td, "gc.s"), "w") as f:
            f.write(textwrap.dedent(asm))
        with open(os.path.join(td, "gc.ld"), "w") as f:
            f.write(textwrap.dedent(script))
        r = sh(["as", "--32", "-o", "gc.o", "gc.s"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:200])

        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        common = ["--gc-sections", "-m", "elf_i386", "-T", "gc.ld", "gc.o"]
        r = sh([lccc_ld] + common + ["-o", "out.lccc"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc-ld failed: {r.stderr.decode()[:400]}")
        r = sh(["ld.bfd"] + common + ["-o", "out.bfd"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", f"bfd unavailable: {r.stderr.decode()[:200]}")

        for stem in ("lccc", "bfd"):
            r = sh(["objcopy", "-O", "binary", f"out.{stem}", f"out.{stem}.bin"], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"objcopy rejected {stem}: {r.stderr.decode()[:200]}")
        with open(os.path.join(td, "out.lccc.bin"), "rb") as f:
            lccc_bytes = f.read()
        with open(os.path.join(td, "out.bfd.bin"), "rb") as f:
            bfd_bytes = f.read()
        if lccc_bytes != bfd_bytes:
            return Result(name, "FAIL",
                          f"flat image differs from bfd ({len(lccc_bytes)} vs {len(bfd_bytes)} bytes)")

        syms = sh(["nm", "out.lccc"], cwd=td).stdout.decode()
        for required in ("_start", "live_fn", "kept_fn", "bootproto"):
            if not re.search(rf"\b{required}$", syms, re.M):
                return Result(name, "FAIL", f"GC dropped required root {required}")
        if re.search(r"\bdead_fn$", syms, re.M):
            return Result(name, "FAIL", "unreachable function survived --gc-sections")

        # GNU options are positional: a later --no-gc-sections must restore the
        # original layout rather than leaving the earlier forwarded flag active.
        override = ["--gc-sections", "--no-gc-sections", "-m", "elf_i386",
                    "-T", "gc.ld", "gc.o"]
        r = sh([lccc_ld] + override + ["-o", "out.nogc.lccc"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc no-GC override failed: {r.stderr.decode()[:300]}")
        r = sh(["ld.bfd"] + override + ["-o", "out.nogc.bfd"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", f"bfd no-GC override failed: {r.stderr.decode()[:200]}")
        nogc_syms = sh(["nm", "out.nogc.lccc"], cwd=td).stdout.decode()
        if not re.search(r"\bdead_fn$", nogc_syms, re.M):
            return Result(name, "FAIL", "--no-gc-sections did not restore dead section")
        for stem in ("nogc.lccc", "nogc.bfd"):
            r = sh(["objcopy", "-O", "binary", f"out.{stem}", f"out.{stem}.bin"], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"objcopy rejected {stem}: {r.stderr.decode()[:200]}")
        with open(os.path.join(td, "out.nogc.lccc.bin"), "rb") as f:
            nogc_lccc = f.read()
        with open(os.path.join(td, "out.nogc.bfd.bin"), "rb") as f:
            nogc_bfd = f.read()
        if nogc_lccc != nogc_bfd:
            return Result(name, "FAIL", "--no-gc-sections image differs from bfd")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


def _shim_for(td, lccc_ld):
    """A -B dir whose `ld` is lccc-ld (gcc only switches linkers on that name)."""
    shim = os.path.join(td, "shim")
    os.makedirs(shim, exist_ok=True)
    link = os.path.join(shim, "ld")
    if not os.path.exists(link):
        os.symlink(os.path.abspath(lccc_ld), link)
    return shim


_PEQ_LIB = r"""
#include <stdio.h>
#include <stdlib.h>
void *lib_puts(void){ return (void*)&puts; }
void *lib_abs(void){ return (void*)&abs; }
int lib_var = 5;
int *lib_var_addr(void){ return &lib_var; }
int lib_call(int (*f)(const char *)){ return f("via-lib") >= 0; }
void lib_weakref(void){}
"""

_PEQ_MAIN = r"""
#include <stdio.h>
#include <stdlib.h>
#include <dlfcn.h>
void *lib_puts(void); void *lib_abs(void); int *lib_var_addr(void);
int lib_call(int (*f)(const char *));
extern void lib_weakref(void) __attribute__((weak));
extern int lib_var;
void *dtab[] = { (void*)puts, (void*)abs };
void * const rtab[] = { (void*)puts, (void*)abs };
int *pv = &lib_var;
extern __thread int tv;
int get_tv(void);
void extra(void);
__attribute__((noinline)) void *code_puts(void){ return (void*)&puts; }
int main(void){
  extra();
  puts("x");
  int r = abs(-3);
  void *lp = lib_puts();
  if (lib_weakref) lib_weakref();
  printf("code %d data %d rodata %d abs %d %d dlsym %d var %d %d rd %d call %d tv %d %d\n",
    code_puts()==lp, dtab[0]==lp, rtab[0]==lp, dtab[1]==lib_abs(), rtab[1]==lib_abs(),
    dlsym(RTLD_DEFAULT,"puts")==lp, pv==lib_var_addr(), &lib_var==lib_var_addr(), lib_var + r,
    lib_call((int (*)(const char *))dtab[0]), tv, get_tv());
  return 0;
}
"""

_PEQ_TLS = "__thread int tv = 77;\n"
_PEQ_TLSUSE = ('extern __thread int tv __attribute__((tls_model("initial-exec")));\n'
               "int get_tv(void){ return tv; }\n")
_PEQ_EXTRA_C = r"""
#include <stdio.h>
void *asm_puts(void); extern int pcrel_puts; void *lib_puts(void);
void extra(void){
  printf("asm-lea %d pcrel %d\n", asm_puts()==lib_puts(),
         (void*)((char*)&pcrel_puts + pcrel_puts)==lib_puts());
}
"""
# A non-branch R_X86_64_PC32 in code and a PC-relative word in read-only
# data, both against a shared-library function.
_PEQ_EXTRA_S = """\t.text
\t.globl asm_puts
asm_puts:
\tleaq puts(%rip), %rax
\tret
\t.section .rodata
\t.globl pcrel_puts
pcrel_puts:
\t.long puts - .
\t.section .note.GNU-stack,"",@progbits
"""
_PEQ_EXPECT = ("x\nvia-lib\ncode 1 data 1 rodata 1 abs 1 1 dlsym 1 var 1 1 rd 8 call 1 tv 77 77\n")


def _dso_pointer_equality_test(args, oracles):
    """`&f` of a shared-library function is ONE address in every module.

    The executable used to hand out its own PLT entry as the address of a
    DSO function (in code, in data, in its GOT slots) without telling ld.so,
    whose other modules then saw the library's definition: `&puts` in the
    program != `&puts` in a library.  The psABI's answer, implemented now, is
    the canonical PLT: a non-PIC address-of makes the PLT entry the address
    and publishes it as the undefined symbol's `.dynsym` value, and every GOT
    slot of a DSO symbol is left to GLOB_DAT.  Covered for -fPIE, -fno-pic
    and -fPIC objects: address-of in code, in `.data` and in read-only data,
    `dlsym`, a callback through the pointer, a copy-relocated variable whose
    address is also stored in data (its PIE slide must not depend on
    relocation order), an exe-defined TLS variable reached via IE (its GOT
    slot holds an offset, never slid), and a non-branch PC32 / `.long f - .`
    (asm) -- which GNU ld refuses in a PIE, so that part is compared against
    the expected output rather than an oracle.  The `.dynsym` bindings are
    checked too: an import is WEAK iff every reference is (glibc's `puts` is
    itself a weak definition, which must not leak into our references).
    """
    name = "dso_pointer_equality"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    files = {"lib.c": _PEQ_LIB, "main.c": _PEQ_MAIN, "tls.c": _PEQ_TLS,
             "tlsuse.c": _PEQ_TLSUSE, "extra.c": _PEQ_EXTRA_C, "asm.s": _PEQ_EXTRA_S,
             "noextra.c": "void extra(void){}\n"}
    try:
        for fn, body in files.items():
            with open(os.path.join(td, fn), "w") as f:
                f.write(body)
        r = sh([CC, "-shared", "-fPIC", "-O1", "lib.c", "-o", "libpq.so"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"fixture lib: {r.stderr.decode()[:200]}")
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        env = dict(os.environ, LD_LIBRARY_PATH=td)
        for mode, cf, lf in (("pie", "-fPIE", "-pie"), ("nopie", "-fno-pic", "-no-pie"),
                             ("pic", "-fPIC", "-pie")):
            for extra, oracle in ((["extra.c", "asm.s"], False), (["noextra.c"], True)):
                srcs = ["main.c", "tls.c", "tlsuse.c"] + extra
                want = _PEQ_EXPECT if oracle else "asm-lea 1 pcrel 1\n" + _PEQ_EXPECT
                linkers = [("lccc", ["-B" + shim])] + ([("gnu", [])] if oracle else [])
                for tag, bflag in linkers:
                    exe = f"t.{mode}.{int(oracle)}.{tag}"
                    r = sh([CC] + bflag + ["-O1", cf, lf] + srcs +
                           ["-L.", "-lpq", "-ldl", "-o", exe], cwd=td)
                    if r.returncode != 0:
                        return Result(name, "FAIL",
                                      f"{mode}/{tag} link: {r.stderr.decode()[:300]}")
                    run = sh([os.path.join(td, exe)], cwd=td, env=env)
                    got = run.stdout.decode()
                    if run.returncode != 0 or got != want:
                        return Result(name, "FAIL",
                                      f"{mode}/{tag}: rc={run.returncode} got {got!r} want {want!r}")
                if not oracle:
                    continue
                syms = sh(["readelf", "-W", "--dyn-syms", f"t.{mode}.1.lccc"],
                          cwd=td).stdout.decode()
                bind = {m.group(2): (m.group(1), int(m.group(0).split()[1], 16))
                        for m in re.finditer(r"^\s*\d+:\s+[0-9a-f]+\s+\d+\s+\w+\s+(\w+)\s+\w+\s+UND\s+([A-Za-z_]\w*)",
                                             syms, re.M)}
                if bind.get("puts", ("?",))[0] != "GLOBAL":
                    return Result(name, "FAIL", f"{mode}: puts import binding {bind.get('puts')}")
                if bind.get("lib_weakref", ("?",))[0] != "WEAK":
                    return Result(name, "FAIL",
                                  f"{mode}: weak reference imported as {bind.get('lib_weakref')}")
                # -fno-pic takes `&puts` with an absolute relocation: canonical PLT.
                if mode == "nopie" and not bind.get("puts", ("", 0))[1]:
                    return Result(name, "FAIL", "nopie: puts has no canonical-PLT st_value")
        return Result(name, "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


_DSE_LIB = r"""
int counter = 5;
void inc(void){ counter++; }
int get_counter(void){ return counter; }
static __thread int tie_local __attribute__((tls_model("initial-exec"))) = 7;
__thread int tie_glob __attribute__((tls_model("initial-exec"))) = 9;
int tls_sum(void){ tie_local++; tie_glob++; return tie_local + tie_glob; }
__attribute__((visibility("hidden"))) int hid_fn(void){ return 3; }
__attribute__((visibility("hidden"))) int hid_var = 11;
__attribute__((visibility("protected"))) int prot_fn(void){ return 4; }
int call_hid(void){ return hid_fn() + hid_var + prot_fn(); }
extern char __bss_start[], _end[];
__attribute__((section("dse_sec"), used)) int dse_item[2] = {1, 2};
extern int __start_dse_sec[], __stop_dse_sec[];
long anchors(void){ return (_end >= __bss_start) * 100 + (__stop_dse_sec - __start_dse_sec); }
int *dse_first(void){ return __start_dse_sec; }
"""

# Two translation units, each with its own `static __thread st`: their GOT
# slots/pairs must not be shared (they were keyed by name).  `tv`/`tv2` live
# in ANOTHER library, so their GD/TLSDESC accesses must name the symbol in
# DTPMOD64 (resolving to the defining module), not "this module".
_DSE_TLS1 = r"""
extern __thread int tv, tv2;
static __thread int st = 100;
int b1(void){ st++; tv++; return tv + tv2 + st; }
"""
_DSE_TLS2 = r"""
static __thread int st = 1000;
extern __thread int tv;
int b2(void){ st++; return st + tv; }
"""
_DSE_TLSDEF = "__thread int tv = 5;\n__thread int tv2 = 50;\n"

_DSE_MAIN = r"""
#include <stdio.h>
#include <dlfcn.h>
extern int counter; void inc(void); int get_counter(void); int tls_sum(void);
int call_hid(void); int prot_fn(void); long anchors(void); int *dse_first(void);
int b1(void), b2(void);
int main(void){
  inc();
  printf("counter %d %d\n", counter, get_counter());
  printf("tls %d\n", tls_sum());
  printf("hid %d %d\n", call_hid(), prot_fn());
  void *h = dlopen(NULL, RTLD_NOW);
  printf("exported %d %d %d\n", dlsym(h, "hid_fn") != 0, dlsym(h, "hid_var") != 0,
         dlsym(h, "__dso_handle") != 0);
  printf("anchors %ld %d\n", anchors(), dse_first()[1]);
  int x = b1(); int y = b2();
  printf("gd %d %d\n", x, y);
  return 0;
}
"""
_DSE_EXPECT = ("counter 6 6\ntls 18\nhid 18 4\nexported 0 0 0\nanchors 102 2\n"
               "gd 157 1007\n")

_DSE_CXX_LIB = r"""
#include <stdexcept>
extern "C" int thrower(int x){ if (x) throw std::runtime_error("boom"); return 0; }
"""
_DSE_CXX_MAIN = r"""
#include <stdexcept>
#include <cstdio>
extern "C" int thrower(int);
int main(){ try { thrower(1); } catch (const std::exception &e) {
  std::printf("caught %s\n", e.what()); return 0; } return 1; }
"""


def _dso_emit_semantics_test(args, oracles):
    """A shared library linked by lccc-ld behaves like one linked by GNU ld.

    Regression net for the x86-64 shared-object emitter, each line a bug it
    had: exported data reached through a GOT slot was bound with
    R_X86_64_RELATIVE, so an executable's COPY of it and the library's
    original diverged (`counter 5 6`); Initial-Exec TLS of the library's own
    variables was relaxed to Local-Exec offsets that only an executable has
    (`tls` read another module's block); HIDDEN symbols (`hid_fn`,
    `__dso_handle`, `DW.ref.__gxx_personality_v0`) were exported; every
    definition claimed section index 1, so GNU ld put the executable's copy
    of `counter` into write-protected `.data.rel.ro` (SIGSEGV on the first
    store, seen on i386); GD pairs of another library's TLS used DTPMOD64
    against symbol 0 ("this module") and static-TLS pairs of two TUs'
    `static __thread st` shared one slot; linker-provided anchors
    (`__start_SEC`, `_end`) were SHN_ABS and unrelocated; `.gnu.version_r`
    chained only its first provider (GNU ld refused to link against a C++
    library: "invalid needed version"); and no PT_GNU_EH_FRAME was emitted,
    so an exception thrown through the library hit std::terminate.  Each
    executable is linked by BOTH GNU ld and lccc-ld, in -fno-pic, PIE and
    (TLS dialects) gnu/gnu2 variants, against a fixed expected output.
    """
    name = "dso_emit_semantics"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    files = {"lib.c": _DSE_LIB, "tls1.c": _DSE_TLS1, "tls2.c": _DSE_TLS2,
             "tlsdef.c": _DSE_TLSDEF, "main.c": _DSE_MAIN,
             "cxxlib.cc": _DSE_CXX_LIB, "cxxmain.cc": _DSE_CXX_MAIN,
             "le.c": "__thread int le __attribute__((tls_model(\"local-exec\")));\n"
                     "int get_le(void){ return ++le; }\n"}
    try:
        for fn, body in files.items():
            with open(os.path.join(td, fn), "w") as f:
                f.write(body)
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        env = dict(os.environ, LD_LIBRARY_PATH=td)
        r = sh([CC, "-shared", "-fPIC", "-O1", "tlsdef.c", "-o", "libtv.so"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"fixture libtv: {r.stderr.decode()[:200]}")
        for dialect in ("gnu", "gnu2"):
            for opt in ("-O0", "-O2"):
                lib = "libdse.so"
                r = sh([CC, "-B" + shim, "-shared", "-fPIC", opt, f"-mtls-dialect={dialect}",
                        "lib.c", "tls1.c", "tls2.c", "-L.", "-ltv", "-o", lib], cwd=td)
                if r.returncode != 0:
                    return Result(name, "FAIL",
                                  f"{dialect}{opt} lib link: {r.stderr.decode()[:300]}")
                for mode, cf, lf in (("nopie", "-fno-pic", "-no-pie"), ("pie", "-fPIE", "-pie")):
                    for tag, bflag in (("gnu", []), ("lccc", ["-B" + shim])):
                        exe = f"m.{mode}.{tag}"
                        r = sh([CC] + bflag + ["-O1", cf, lf, "main.c", "-L.", "-ldse",
                                               "-ltv", "-ldl", "-o", exe], cwd=td)
                        if r.returncode != 0:
                            return Result(name, "FAIL", f"{dialect}{opt} {mode}/{tag} link: "
                                          f"{r.stderr.decode()[:300]}")
                        run = sh([os.path.join(td, exe)], cwd=td, env=env)
                        got = run.stdout.decode()
                        if run.returncode != 0 or got != _DSE_EXPECT:
                            return Result(name, "FAIL", f"{dialect}{opt} {mode}/{tag}: "
                                          f"rc={run.returncode} got {got!r}")
        # Static checks on the last library.
        syms = sh(["readelf", "-W", "--dyn-syms", "libdse.so"], cwd=td).stdout.decode()
        defined = {}
        for m in re.finditer(r"^\s*\d+:\s+[0-9a-f]+\s+\d+\s+\w+\s+\w+\s+(\w+)\s+(\w+)\s+(\S+)$",
                             syms, re.M):
            defined[m.group(3).split("@")[0]] = (m.group(1), m.group(2))
        for leaked in ("hid_fn", "hid_var", "__dso_handle", "__TMC_END__",
                       "_GLOBAL_OFFSET_TABLE_", "_DYNAMIC"):
            if leaked in defined:
                return Result(name, "FAIL", f"{leaked} exported from the library")
        if defined.get("prot_fn", ("",))[0] != "PROTECTED":
            return Result(name, "FAIL", f"prot_fn dynsym visibility {defined.get('prot_fn')}")
        shdrs = sh(["readelf", "-W", "-S", "libdse.so"], cwd=td).stdout.decode()
        flags = {int(m.group(1)): m.group(2) for m in re.finditer(
            r"^\s*\[\s*(\d+)\]\s+\S+\s+\w+\s+[0-9a-f]+\s+[0-9a-f]+\s+[0-9a-f]+\s+[0-9a-f]+\s+(\w*)",
            shdrs, re.M)}
        ndx = defined.get("counter", ("", ""))[1]
        if not ndx.isdigit() or "W" not in flags.get(int(ndx), ""):
            return Result(name, "FAIL", f"counter's st_shndx {ndx} is not a writable section")
        # `__stop_SEC` is one past SEC's end -- often the start of the next
        # section -- yet belongs to SEC, as does `__start_SEC` (bfd: PROTECTED).
        names = {int(m.group(1)): m.group(2) for m in re.finditer(
            r"^\s*\[\s*(\d+)\]\s+(\S+)", shdrs, re.M)}
        for anchor in ("__start_dse_sec", "__stop_dse_sec"):
            vis, andx = defined.get(anchor, ("", ""))
            if vis != "PROTECTED" or not andx.isdigit() or names.get(int(andx)) != "dse_sec":
                return Result(name, "FAIL", f"{anchor}: {vis} in section {andx} "
                              f"({names.get(int(andx)) if andx.isdigit() else '-'}), "
                              f"want PROTECTED in dse_sec")
        # Imports: the binding comes from THIS library's references (all
        # weak -> WEAK), the type from the references too (glibc's
        # definition of __cxa_finalize is a FUNC, but an import is NOTYPE
        # unless TLS), never from whatever the providing library says.
        imports = {m.group(3).split("@")[0]: (m.group(1), m.group(2)) for m in re.finditer(
            r"^ *\d+: +0+ +\d+ +(\w+) +(\w+) +\w+ +UND +(\S+)", syms, re.M)}
        for imp, want in (("__cxa_finalize", ("NOTYPE", "WEAK")),
                          ("_ITM_registerTMCloneTable", ("NOTYPE", "WEAK")),
                          ("tv", ("TLS", "GLOBAL"))):
            if imports.get(imp) != want:
                return Result(name, "FAIL", f"import {imp} is {imports.get(imp)}, want {want}")
        if "GNU_EH_FRAME" not in sh(["readelf", "-W", "-l", "libdse.so"], cwd=td).stdout.decode():
            return Result(name, "FAIL", "no PT_GNU_EH_FRAME")
        # Local-Exec TLS cannot be linked into a shared object.
        r = sh([CC, "-B" + shim, "-shared", "-fPIC", "-O1", "le.c", "-o", "le.so"], cwd=td)
        if r.returncode == 0 or b"R_X86_64_TPOFF32" not in r.stderr:
            return Result(name, "FAIL", "local-exec TLS in a shared object was not refused")
        # C++: exception through the library; GNU ld must accept its
        # version tables when linking the executable.
        cxx = shutil.which(CXX)
        if not cxx:
            return Result(name, "SKIP", f"C part passed; C++ part needs {CXX}")
        r = sh([cxx, "-B" + shim, "-shared", "-fPIC", "-O1", "cxxlib.cc", "-o", "libcx.so"],
               cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"c++ lib link: {r.stderr.decode()[:300]}")
        ver = sh(["readelf", "-V", "libcx.so"], cwd=td)
        if ver.stderr.strip():
            return Result(name, "FAIL", f"readelf -V: {ver.stderr.decode()[:200]}")
        # A strong reference makes a GLOBAL import (a WEAK one would let
        # the program run on with a null typeinfo).
        cxs = sh(["readelf", "-W", "--dyn-syms", "libcx.so"], cwd=td).stdout.decode()
        if not re.search(r"NOTYPE\s+GLOBAL\s+DEFAULT\s+UND\s+_ZTISt13runtime_error@", cxs):
            return Result(name, "FAIL", "_ZTISt13runtime_error not a GLOBAL NOTYPE import")
        r = sh([cxx, "-O1", "cxxmain.cc", "-L.", "-lcx", "-o", "cx"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"GNU ld vs lccc C++ DSO: {r.stderr.decode()[:300]}")
        run = sh([os.path.join(td, "cx")], cwd=td, env=env)
        if run.returncode != 0 or run.stdout.decode() != "caught boom\n":
            return Result(name, "FAIL", f"c++ exception through the DSO: rc={run.returncode} "
                          f"{run.stdout.decode()!r} {run.stderr.decode()[:200]!r}")
        return Result(name, "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


def _dso_emit_semantics_i386_test(args, oracles):
    """The i386 counterpart of `dso_emit_semantics`.

    The same library -- exported data behind a GOT slot, Initial-Exec TLS
    of its own variables, hidden/protected definitions, two TUs' `static
    __thread st`, another library's TLS through GD and TLSDESC, linker
    anchors -- linked by lccc-ld's ELF32 shared-object emitter, used by
    executables GNU ld and lccc-ld link (-fno-pic; GNU ld also -pie, which
    lccc-ld does not produce for i386).  The emitter used to write only
    the section headers GNU ld needs to accept the file, with every
    definition claiming index 1 -- `.dynamic`, inside PT_GNU_RELRO -- so
    GNU ld put an executable's COPY of `counter` into write-protected
    `.data.rel.ro`, and the first `inc()` faulted.  It now writes a header
    for every allocated section; `readelf` must read them without a
    warning, and `counter` must name a writable one.
    """
    name = "i386_dso_emit_semantics"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    files = {"lib.c": _DSE_LIB, "tls1.c": _DSE_TLS1, "tls2.c": _DSE_TLS2,
             "tlsdef.c": _DSE_TLSDEF, "main.c": _DSE_MAIN,
             "cxxlib.cc": _DSE_CXX_LIB, "cxxmain.cc": _DSE_CXX_MAIN}
    try:
        for fn, body in files.items():
            with open(os.path.join(td, fn), "w") as f:
                f.write(body)
        r = sh([CC, "-m32", "-shared", "-fPIC", "-O1", "tlsdef.c", "-o", "libtv.so"], cwd=td)
        if r.returncode != 0:
            status = "FAIL" if os.environ.get("LCCC_REQUIRE_I386") == "1" else "SKIP"
            return Result(name, status, f"no -m32 toolchain: {r.stderr.decode()[:150]}")
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        env = dict(os.environ, LD_LIBRARY_PATH=td)
        for dialect in ("gnu", "gnu2"):
            for opt in ("-O0", "-O2"):
                r = sh([CC, "-m32", "-B" + shim, "-shared", "-fPIC", opt,
                        f"-mtls-dialect={dialect}", "lib.c", "tls1.c", "tls2.c", "-L.", "-ltv",
                        "-o", "libdse.so"], cwd=td)
                if r.returncode != 0:
                    return Result(name, "FAIL",
                                  f"{dialect}{opt} lib link: {r.stderr.decode()[:300]}")
                for tag, flags in (("gnu", ["-fno-pic", "-no-pie"]),
                                   ("lccc", ["-B" + shim, "-fno-pic", "-no-pie"]),
                                   ("gnu-pie", ["-fPIE", "-pie"])):
                    exe = f"m.{tag}"
                    r = sh([CC, "-m32", "-O1"] + flags + ["main.c", "-L.", "-ldse", "-ltv",
                                                          "-ldl", "-o", exe], cwd=td)
                    if r.returncode != 0:
                        return Result(name, "FAIL", f"{dialect}{opt} {tag} link: "
                                      f"{r.stderr.decode()[:300]}")
                    run = sh([os.path.join(td, exe)], cwd=td, env=env)
                    got = run.stdout.decode()
                    if run.returncode != 0 or got != _DSE_EXPECT:
                        return Result(name, "FAIL", f"{dialect}{opt} {tag}: "
                                      f"rc={run.returncode} got {got!r}")
        hdrs = sh(["readelf", "-W", "-S", "-l", "--dyn-syms", "libdse.so"], cwd=td)
        if hdrs.returncode != 0 or hdrs.stderr.strip():
            return Result(name, "FAIL", f"readelf: {hdrs.stderr.decode()[:300]}")
        out = hdrs.stdout.decode()
        flags = {int(m.group(1)): m.group(2) for m in re.finditer(
            r"^\s*\[\s*(\d+)\]\s+\S+\s+\w+\s+[0-9a-f]+\s+[0-9a-f]+\s+[0-9a-f]+\s+[0-9a-f]+\s+(\w*)",
            out, re.M)}
        defined = {}
        for m in re.finditer(r"^\s*\d+:\s+[0-9a-f]+\s+\d+\s+\w+\s+\w+\s+(\w+)\s+(\w+)\s+(\S+)$",
                             out, re.M):
            defined[m.group(3).split("@")[0]] = (m.group(1), m.group(2))
        for leaked in ("hid_fn", "hid_var", "__dso_handle", "_GLOBAL_OFFSET_TABLE_", "_DYNAMIC"):
            if leaked in defined:
                return Result(name, "FAIL", f"{leaked} exported from the library")
        if defined.get("prot_fn", ("",))[0] != "PROTECTED":
            return Result(name, "FAIL", f"prot_fn dynsym visibility {defined.get('prot_fn')}")
        ndx = defined.get("counter", ("", ""))[1]
        if not ndx.isdigit() or "W" not in flags.get(int(ndx), ""):
            return Result(name, "FAIL", f"counter's st_shndx {ndx} is not a writable section")
        if "GNU_EH_FRAME" not in out:
            return Result(name, "FAIL", "no PT_GNU_EH_FRAME")
        cxx = shutil.which(CXX)
        if not cxx:
            return Result(name, "SKIP", f"C part passed; C++ part needs {CXX}")
        r = sh([cxx, "-m32", "-B" + shim, "-shared", "-fPIC", "-O1", "cxxlib.cc",
                "-o", "libcx.so"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"c++ lib link: {r.stderr.decode()[:300]}")
        r = sh([cxx, "-m32", "-O1", "cxxmain.cc", "-L.", "-lcx", "-o", "cx"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"GNU ld vs lccc C++ DSO: {r.stderr.decode()[:300]}")
        run = sh([os.path.join(td, "cx")], cwd=td, env=env)
        if run.returncode != 0 or run.stdout.decode() != "caught boom\n":
            return Result(name, "FAIL", f"c++ exception through the DSO: rc={run.returncode} "
                          f"{run.stdout.decode()!r} {run.stderr.decode()[:200]!r}")
        return Result(name, "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


_IF_LIB = """
static int impl_a(void) { return 11; }
static void *res_local(void) { return (void *)impl_a; }
static int loc_if(void) __attribute__((ifunc("res_local")));
static int impl_b(void) { return 22; }
static void *res_hidden(void) { return (void *)impl_b; }
__attribute__((visibility("hidden"))) int hid_if(void) __attribute__((ifunc("res_hidden")));
static int impl_c(void) { return 33; }
static void *res_def(void) { return (void *)impl_c; }
int def_if(void) __attribute__((ifunc("res_def")));
int (*const tbl[])(void) = { loc_if, hid_if, def_if };
int (*p_hid)(void) = hid_if;
int lib_test(void)
{
    int (*volatile f)(void) = hid_if;
    int (*volatile g)(void) = def_if;
    return loc_if() + hid_if() + def_if() + tbl[0]() + tbl[1]() + f() + g()
           + (f == p_hid) + (tbl[1] == hid_if) * 1000 + (tbl[2] == g) * 10000;
}
"""
_IF_MAIN = """
#include <stdio.h>
int lib_test(void);
int def_if(void);
extern int (*const tbl[])(void);
int main(void) { printf("%d %d %d\\n", lib_test(), def_if(), tbl[2] == def_if); return 0; }
"""


_EHP_WALK = r"""
#define _GNU_SOURCE
#include <link.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
/* Walk .eh_frame linearly, the way libgcc's __register_frame_info path and
   debuggers do, and compare the FDE count with .eh_frame_hdr's: a zero word
   in the middle of the section ends the walk early. */
static int eh_cb(struct dl_phdr_info *info, size_t size, void *data) {
    const char *want = data;
    (void)size;
    if (want[0] ? !strstr(info->dlpi_name, want) : info->dlpi_name[0] != 0)
        return 0;
    for (int i = 0; i < info->dlpi_phnum; i++) {
        if (info->dlpi_phdr[i].p_type != PT_GNU_EH_FRAME) continue;
        const unsigned char *hdr =
            (const unsigned char *)(info->dlpi_addr + info->dlpi_phdr[i].p_vaddr);
        if (hdr[0] != 1 || hdr[1] != 0x1b || hdr[2] != 0x03) { puts("hdr-enc?"); return 1; }
        int32_t rel; uint32_t count, fdes = 0, len, id;
        memcpy(&rel, hdr + 4, 4);
        memcpy(&count, hdr + 8, 4);
        const unsigned char *p = hdr + 4 + rel;
        for (;;) {
            memcpy(&len, p, 4);
            if (len == 0 || len == 0xffffffffu) break;
            memcpy(&id, p + 4, 4);
            fdes += id != 0;
            p += 4 + (size_t)len;
        }
        printf("%s %d\n", want[0] ? "lib" : "exe", count > 0 && fdes == count);
        return 1;
    }
    printf("%s no-PT_GNU_EH_FRAME\n", want[0] ? "lib" : "exe");
    return 1;
}
void eh_walk(const char *want) { dl_iterate_phdr(eh_cb, (void *)want); }
"""

_EHP_T1 = r"""
#include <stdexcept>
void thrower(int x) { if (x) throw std::runtime_error("boom"); }
int dead_t1(int x) { return x * 3; }
"""

_EHP_T2 = r"""
int helper_c(int x) { return x + 1; }      /* plain C++ CIE (no personality) */
int dead_t2(int x) { return x * 5; }
"""

_EHP_LIB = r"""
#include <stdexcept>
int lib_helper(int x) { return x * 2; }
void lib_throw(int x) { if (lib_helper(x)) throw std::logic_error("lib"); }
"""

_EHP_MAIN = r"""
#include <cstdio>
#include <stdexcept>
void thrower(int); int helper_c(int); void lib_throw(int);
extern "C" void eh_walk(const char *);
static int f(int x) {
    try { thrower(x); } catch (const std::exception &e) { std::printf("caught %s\n", e.what()); }
    try { lib_throw(helper_c(x)); } catch (const std::exception &e) { std::printf("caught %s\n", e.what()); }
    return 0;
}
int main(int c, char **) { f(c); eh_walk(""); eh_walk("libehp"); return 0; }
"""


def _eh_frame_packing_test(args, oracles):
    """`.eh_frame` of an executable and a DSO: one CIE of each kind, one
    terminator, and a linear walk that reaches every FDE.

    Each object brings its own CIEs, and every 8-aligned input section whose
    records sum to 4 mod 8 (crt1.o's) used to be followed by alignment zeros
    -- a zero-length record, i.e. a terminator, in mid-section.  Compacted
    (GC-pruned) inputs even got an explicit terminator appended.  Unwinding
    through .eh_frame_hdr's table still worked, which is why nothing failed;
    a linear walk (libgcc without the table, gdb, crash handlers) stopped at
    the first object.  Checked on both link paths (link_builtin with
    --gc-sections, link_shared) against GNU ld as the CIE-count oracle.
    """
    name = "eh_frame_packing"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        if shutil.which(CXX) is None:
            return Result(name, "SKIP", "no C++ compiler")
        for fn, body in (("walk.c", _EHP_WALK), ("t1.cc", _EHP_T1), ("t2.cc", _EHP_T2),
                         ("lib.cc", _EHP_LIB), ("main.cc", _EHP_MAIN)):
            with open(os.path.join(td, fn), "w") as f:
                f.write(body)
        cf = ["-O1", "-ffunction-sections"]
        for src in ("t1.cc", "t2.cc", "main.cc"):
            r = sh([CXX, *cf, "-c", src], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"{src}: {r.stderr.decode()[:300]}")
        r = sh([CC, *cf, "-c", "walk.c"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"walk.c: {r.stderr.decode()[:300]}")
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        env = dict(os.environ, LD_LIBRARY_PATH=td)
        objs = ["t1.o", "t2.o", "walk.o", "main.o"]

        def frames(path):
            out = sh(["readelf", "-W", "--debug-dump=frames", path], cwd=td).stdout.decode()
            recs = re.findall(r"^[0-9a-f]{8} (?:[0-9a-f]+ [0-9a-f]+ )?(CIE|FDE|ZERO)", out, re.M)
            return recs.count("CIE"), recs.count("FDE"), recs.count("ZERO")

        counts = {}
        for tag, b in (("lccc", ["-B" + shim]), ("bfd", [])):
            r = sh([CXX, *b, "-shared", "-fPIC", "-O1", "lib.cc", "-o", "libehp.so"], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"{tag} lib: {r.stderr.decode()[:300]}")
            exe = f"a.{tag}"
            r = sh([CXX, *b, "-Wl,--gc-sections", *objs, "-L.", "-lehp", "-o", exe], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"{tag} exe: {r.stderr.decode()[:300]}")
            r = sh([os.path.join(td, exe)], cwd=td, env=env)
            got = r.stdout.decode()
            want = "caught boom\ncaught lib\nexe 1\nlib 1\n"
            if r.returncode != 0 or got != want:
                return Result(name, "FAIL", f"{tag}: rc={r.returncode} got {got!r}, want {want!r}")
            counts[tag] = {"exe": frames(os.path.join(td, exe)),
                           "lib": frames(os.path.join(td, "libehp.so"))}
            if tag == "lccc":
                os.rename(os.path.join(td, "libehp.so"), os.path.join(td, "libehp.lccc.so"))
        for kind in ("exe", "lib"):
            (lc, lf, lz), (bc, _bf, _bz) = counts["lccc"][kind], counts["bfd"][kind]
            if lz != 1:
                return Result(name, "FAIL", f"{kind}: {lz} ZERO terminators, want exactly 1")
            if lf == 0 or lc > bc:
                return Result(name, "FAIL",
                              f"{kind}: {lc} CIEs / {lf} FDEs; GNU ld keeps {bc} CIEs")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


def _so_bound_ifunc_test(args, oracles):
    """IFUNCs bound inside a shared library (static, hidden, -Bsymbolic).

    Their symbol value is the resolver, so every reference -- call, `lea`,
    GOT slot, pointer in `.data.rel.ro` -- must go through an IPLT entry
    whose `.got.plt` slot an R_X86_64_IRELATIVE fills; lccc-ld used to
    refuse the link.  The entry is the function's one address, so
    `f == p_hid` (a `lea` against a pointer initialised in data) holds:
    11155.  (GNU ld gives 11154 -- it resolves the data pointer to the
    implementation and the `lea` to its PLT entry, so C's pointer equality
    fails there; the test therefore pins lccc's output, not bfd's.)
    """
    name = "so_bound_ifunc"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        for fn, body in (("lib.c", _IF_LIB), ("main.c", _IF_MAIN)):
            with open(os.path.join(td, fn), "w") as f:
                f.write(body)
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        env = dict(os.environ, LD_LIBRARY_PATH=td)
        for opt in ("-O0", "-O2"):
            for bs in ([], ["-Wl,-Bsymbolic"]):
                r = sh([CC, "-B" + shim, "-shared", "-fPIC", opt] + bs +
                       ["lib.c", "-o", "libif.so"], cwd=td)
                if r.returncode != 0:
                    return Result(name, "FAIL", f"{opt}{bs} lib: {r.stderr.decode()[:300]}")
                rel = sh(["readelf", "-W", "-r", "libif.so"], cwd=td).stdout.decode()
                want_irel = 3 if bs else 2
                if rel.count("R_X86_64_IRELATIVE") != want_irel:
                    return Result(name, "FAIL", f"{opt}{bs}: want {want_irel} IRELATIVE: {rel}")
                for tag, bflag in (("gnu", []), ("lccc", ["-B" + shim])):
                    r = sh([CC] + bflag + ["-O1", "main.c", "-L.", "-lif", "-o", "m"], cwd=td)
                    if r.returncode != 0:
                        return Result(name, "FAIL", f"{opt}{bs} {tag} exe: "
                                      f"{r.stderr.decode()[:300]}")
                    run = sh([os.path.join(td, "m")], cwd=td, env=env)
                    if run.returncode != 0 or run.stdout.decode() != "11155 33 1\n":
                        return Result(name, "FAIL", f"{opt}{bs} {tag}: rc={run.returncode} "
                                      f"{run.stdout.decode()!r} {run.stderr.decode()[:200]!r}")
        return Result(name, "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


_LM_EXT = "int ext_var = 100;\nint ext_fn(int x) { return 2 * x; }\n"
_LM_LIB = """
extern int ext_var;
extern int ext_fn(int);
int gvar = 5;
static int svar = 7;
static int lbig[20000] = {9};   /* > -mlarge-data-threshold: .ldata */
static int sfn(int x) { return x + svar + lbig[0]; }
int lib_fn(int x) { return ext_fn(x) + sfn(gvar) + ext_var; }
int *addr_g(void) { return &gvar; }
"""
_LM_MAIN = """
#include <stdio.h>
extern int gvar;
extern int lib_fn(int);
extern int *addr_g(void);
extern int asm_probe(void), asm_probe_x(void);
static int bigarr[20000];       /* .lbss */
int big_data[20000] = {1, 2, 3};
static int (*fp)(int) = lib_fn;
int main(void)
{
    bigarr[19999] = 3;
    printf("%d %d %d %d %d %d %d\\n", lib_fn(1), gvar, addr_g() == &gvar, fp(2),
           bigarr[19999] + big_data[2], asm_probe(), asm_probe_x());
    return 0;
}
"""
# Every large-model PIC relocation a compiler or hand-written code emits:
# GOTPC64 (GOT base), GOT64 (slot offset), GOTOFF64 (local data), PLTOFF64
# (call through the PLT), GOTPC32 (the medium-model base), GOTPCREL64 (a
# slot's PC-relative distance; gas takes it only as data) and GOTPLT64.
# 5 + 7 + ext_fn(3) + 1 + 5 + ext_fn(4) = 32.
_LM_ASM = """\t.text
\t.globl asm_probe
\t.type asm_probe,@function
asm_probe:
\tpushq %r15
\tpushq %rbx
\tsubq $8, %rsp
.Lb:\tleaq .Lb(%rip), %r15
\tmovabsq $_GLOBAL_OFFSET_TABLE_-.Lb, %r11
\taddq %r11, %r15
\tmovabsq $gvar@GOT, %rax
\tmovq (%r15,%rax), %rax
\tmovl (%rax), %ebx
\tmovabsq $svar@GOTOFF, %rax
\taddl (%r15,%rax), %ebx
\tmovabsq $ext_fn@PLTOFF, %rax
\taddq %r15, %rax
\tmovl $3, %edi
\tcall *%rax
\taddl %eax, %ebx
\tleaq _GLOBAL_OFFSET_TABLE_(%rip), %rdx
\txorl %eax, %eax
\tcmpq %rdx, %r15
\tsete %al
\taddl %eax, %ebx
\tleaq gpq(%rip), %rdx
\tmovq (%rdx), %rax
\taddq %rdx, %rax
\tmovq (%rax), %rax
\taddl (%rax), %ebx
\tmovabsq $ext_fn@GOTPLT, %rax
\tmovq (%r15,%rax), %rax
\tmovl $4, %edi
\tcall *%rax
\taddl %ebx, %eax
\taddq $8, %rsp
\tpopq %rbx
\tpopq %r15
\tret
\t.data
\t.p2align 3
gpq:\t.quad gvar@GOTPCREL
svar:\t.long 7
\t.section .note.GNU-stack,"",@progbits
"""
_LM_EXPECT = "123 5 1 125 6 32 32\n"


def _code_model_large_medium_test(args, oracles):
    """-mcmodel=large and -mcmodel=medium code links like GNU ld links it.

    The x86-64 emitters used to reject or mis-resolve the large-model PIC
    relocations (R_X86_64_GOTPC64, GOT64, GOTOFF64, PLTOFF64, GOTPCREL64,
    GOTPLT64) and medium's GOTPC32.  A library and an executable (-fno-pic
    and PIE) are built from the same model, each linked by GNU ld AND by
    lccc-ld in every combination, plus a hand-written probe using every
    large-model relocation in both the library and the executable.
    """
    name = "code_model_large_medium"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        files = {"ext.c": _LM_EXT, "lm.c": _LM_LIB, "main.c": _LM_MAIN,
                 "probe.s": _LM_ASM,
                 "probe_x.s": _LM_ASM.replace("asm_probe", "asm_probe_x")}
        for fn, body in files.items():
            with open(os.path.join(td, fn), "w") as f:
                f.write(body)
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        env = dict(os.environ, LD_LIBRARY_PATH=td)
        r = sh([CC, "-shared", "-fPIC", "-O1", "ext.c", "-o", "libext.so"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"fixture libext: {r.stderr.decode()[:200]}")
        for s_ in ("probe.s", "probe_x.s"):
            r = sh([CC, "-c", s_], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"assemble {s_}: {r.stderr.decode()[:200]}")
        linkers = (("gnu", []), ("lccc", ["-B" + shim]))
        for model in ("large", "medium"):
            for ltag, lflag in linkers:
                lib = f"liblm_{model}_{ltag}.so"
                r = sh([CC] + lflag + ["-shared", "-fPIC", "-O1", f"-mcmodel={model}",
                                       "lm.c", "probe.o", "-L.", "-lext", "-o", lib], cwd=td)
                if r.returncode != 0:
                    return Result(name, "FAIL", f"{model} lib/{ltag}: {r.stderr.decode()[:300]}")
                for mode, cf, lf in (("nopie", "-fno-pic", "-no-pie"), ("pie", "-fPIE", "-pie")):
                    for etag, eflag in linkers:
                        exe = f"m.{model}.{ltag}.{mode}.{etag}"
                        r = sh([CC] + eflag + ["-O1", f"-mcmodel={model}", cf, lf, "main.c",
                                               "probe_x.o", lib, "-L.", "-lext", "-o", exe],
                               cwd=td)
                        if r.returncode != 0:
                            return Result(name, "FAIL", f"{exe} link: {r.stderr.decode()[:300]}")
                        run = sh([os.path.join(td, exe)], cwd=td, env=env)
                        got = run.stdout.decode()
                        if run.returncode != 0 or got != _LM_EXPECT:
                            return Result(name, "FAIL", f"{exe}: rc={run.returncode} got {got!r} "
                                          f"{run.stderr.decode()[:200]!r}")
        return Result(name, "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


_XD_LIB = """
extern int hook(void);                 /* the program's callback */
extern char abs_marker[];              /* an absolute symbol of the program */
int dflt(void) { return 1; }           /* the program replaces it */
int call_hook(void) { return hook(); }
int call_dflt(void) { return dflt(); }
unsigned long get_abs(void) { return (unsigned long)abs_marker; }
"""
_XD_MAIN = """
#include <stdio.h>
extern int call_hook(void), call_dflt(void);
extern unsigned long get_abs(void);
__asm__(".globl abs_marker\\n.set abs_marker, 0x1234");
__attribute__((visibility("protected"))) int hook(void) { return 40; }
int dflt(void) { return 2; }
int unrelated(void) { return 3; }
int main(void)
{
    printf("%d %d %lx\\n", call_hook(), call_dflt(), get_abs());
    return 0;
}
"""


def _exe_exports_dso_names_test(args, oracles):
    """An executable exports exactly the definitions its libraries name.

    GNU ld (and lld) put a program's definition in `.dynsym` when a linked
    shared library defines or references the same name: the library's
    callback into the program (`hook`) resolves only then, and its own
    interposable call (`dflt`) binds to the program's replacement -- the
    `malloc` pattern.  lccc-ld exported nothing without --export-dynamic,
    so `hook` was an unresolved symbol at load time.  `unrelated` must stay
    unexported, and --gc-sections must keep `hook` (a root by the same rule).
    The exported entries must also carry what ld.so reads: `abs_marker` is
    SHN_ABS (it once claimed section 1, so in a PIE the library saw it
    displaced by the load base), and `hook` keeps STV_PROTECTED.
    """
    name = "exe_exports_dso_names"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        for fn, body in (("lib.c", _XD_LIB), ("main.c", _XD_MAIN)):
            with open(os.path.join(td, fn), "w") as f:
                f.write(body)
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        env = dict(os.environ, LD_LIBRARY_PATH=td)
        r = sh([CC, "-shared", "-fPIC", "-O1", "lib.c", "-o", "libxd.so"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"fixture: {r.stderr.decode()[:200]}")
        for mode, cf, lf in (("nopie", "-fno-pic", "-no-pie"), ("pie", "-fPIE", "-pie")):
            for gc in ([], ["-ffunction-sections", "-Wl,--gc-sections"]):
                for tag, bflag in (("gnu", []), ("lccc", ["-B" + shim])):
                    exe = f"m.{mode}.{tag}{'.gc' if gc else ''}"
                    r = sh([CC] + bflag + ["-O1", cf, lf] + gc + ["main.c", "-L.", "-lxd",
                                                                  "-o", exe], cwd=td)
                    if r.returncode != 0:
                        return Result(name, "FAIL", f"{exe} link: {r.stderr.decode()[:300]}")
                    run = sh([os.path.join(td, exe)], cwd=td, env=env)
                    if run.returncode != 0 or run.stdout.decode() != "40 2 1234\n":
                        return Result(name, "FAIL", f"{exe}: rc={run.returncode} "
                                      f"{run.stdout.decode()!r} {run.stderr.decode()[:200]!r}")
                    dyn = sh(["readelf", "-W", "--dyn-syms", exe], cwd=td).stdout.decode()
                    ents = {f[7].split("@")[0]: (f[5], f[6]) for f in
                            (ln.split() for ln in dyn.splitlines()
                             if re.match(r"\s*\d+:", ln) and " UND " not in ln)
                            if len(f) >= 8}
                    if not {"hook", "dflt", "abs_marker"} <= set(ents) or "unrelated" in ents:
                        return Result(name, "FAIL", f"{exe} exports {sorted(ents)}")
                    if ents["abs_marker"][1] != "ABS" or ents["hook"][0] != "PROTECTED" \
                            or not ents["dflt"][1].isdigit():
                        return Result(name, "FAIL", f"{exe}: abs_marker {ents['abs_marker']}, "
                                      f"hook {ents['hook']}, dflt {ents['dflt']}")
        return Result(name, "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


_PEQ_EXTRA_S32 = """\t.text
\t.globl asm_puts
asm_puts:
\tmovl $puts, %eax
\tret
\t.section .rodata
\t.globl pcrel_puts
pcrel_puts:
\t.long puts - .
\t.section .note.GNU-stack,"",@progbits
"""


def _dso_pointer_equality_i386_test(args, oracles):
    """The i386 counterpart of `dso_pointer_equality` (ET_EXEC only).

    Same program, from -fno-pic and -fPIC objects.  Besides the canonical
    PLT (`movl $puts`, `R_386_32` in data, `.long puts - .`), this pins the
    GOT side: a `-fPIC` `&puts` is a GOT32X load, and it used to read the
    function's lazily-bound `.got.plt` slot -- `PLT+6` before the first call,
    the library's address after it.  It now has its own GLOB_DAT slot.
    """
    name = "i386_dso_pointer_equality"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    files = {"lib.c": _PEQ_LIB, "main.c": _PEQ_MAIN, "tls.c": _PEQ_TLS,
             "tlsuse.c": _PEQ_TLSUSE, "extra.c": _PEQ_EXTRA_C, "asm.s": _PEQ_EXTRA_S32}
    try:
        for fn, body in files.items():
            with open(os.path.join(td, fn), "w") as f:
                f.write(body)
        r = sh([CC, "-m32", "-shared", "-fPIC", "-O1", "lib.c", "-o", "libpq.so"], cwd=td)
        if r.returncode != 0:
            # CI installs multilib and sets LCCC_REQUIRE_I386=1: there an
            # unusable -m32 toolchain is a broken runner, not a reason to skip.
            status = "FAIL" if os.environ.get("LCCC_REQUIRE_I386") == "1" else "SKIP"
            return Result(name, status, f"no -m32 toolchain: {r.stderr.decode()[:150]}")
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        env = dict(os.environ, LD_LIBRARY_PATH=td)
        want = "asm-lea 1 pcrel 1\n" + _PEQ_EXPECT
        for cf in ("-fno-pic", "-fPIC"):
            for tag, bflag in (("lccc", ["-B" + shim]), ("gnu", [])):
                exe = f"t{cf}.{tag}"
                r = sh([CC, "-m32"] + bflag + ["-O1", cf, "-no-pie", "main.c", "tls.c",
                       "tlsuse.c", "extra.c", "asm.s", "-L.", "-lpq", "-ldl", "-o", exe], cwd=td)
                if r.returncode != 0:
                    return Result(name, "FAIL", f"{cf}/{tag} link: {r.stderr.decode()[:300]}")
                run = sh([os.path.join(td, exe)], cwd=td, env=env)
                got = run.stdout.decode()
                if run.returncode != 0 or got != want:
                    return Result(name, "FAIL",
                                  f"{cf}/{tag}: rc={run.returncode} got {got!r} want {want!r}")
            syms = sh(["readelf", "-W", "-D", "-s", f"t{cf}.lccc"], cwd=td).stdout.decode()
            und = {m.group(3): (m.group(2), int(m.group(1), 16))
                   for m in re.finditer(r"^\s*\d+:\s+([0-9a-f]+)\s+\d+\s+\w+\s+(\w+)\s+\w+\s+UND\s+([A-Za-z_]\w*)",
                                        syms, re.M)}
            if und.get("puts", ("?", 0)) [0] != "GLOBAL" or not und["puts"][1]:
                return Result(name, "FAIL", f"{cf}: puts import {und.get('puts')} (want GLOBAL, canonical)")
            if und.get("lib_weakref", ("?",))[0] != "WEAK":
                return Result(name, "FAIL", f"{cf}: weak reference imported as {und.get('lib_weakref')}")
        return Result(name, "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


_GOTX_ASM = r"""	.text
	.globl	target
	.type	target, @function
target:	movl	$42, %eax
	ret
	.globl	f_mov, f_mov32, f_movr9, f_call, f_jmp, f_high, f_dso, f_test, f_add, f_cmp
f_mov:	movq	var@GOTPCREL(%rip), %rax
	movq	(%rax), %rax
	ret
f_mov32: movl	var@GOTPCREL(%rip), %eax
	leaq	var(%rip), %rcx
	cmpl	%ecx, %eax
	sete	%al
	movzbl	%al, %eax
	ret
f_movr9: movq	var@GOTPCREL(%rip), %r9
	movq	(%r9), %rax
	ret
f_call:	subq	$8, %rsp
	call	*target@GOTPCREL(%rip)
	addq	$8, %rsp
	ret
f_jmp:	jmp	*target@GOTPCREL(%rip)
f_high:	movl	var@GOTPCREL+4(%rip), %eax
	leaq	var(%rip), %rcx
	shrq	$32, %rcx
	cmpl	%ecx, %eax
	sete	%al
	movzbl	%al, %eax
	ret
f_dso:	movq	puts@GOTPCREL(%rip), %rax
	testq	%rax, %rax
	setne	%al
	movzbl	%al, %eax
	ret
f_test:	movq	$-1, %rcx
	xorl	%eax, %eax
	testq	%rcx, var@GOTPCREL(%rip)
	setne	%al
	ret
f_add:	xorl	%eax, %eax
	addq	var@GOTPCREL(%rip), %rax
	leaq	var(%rip), %rcx
	cmpq	%rcx, %rax
	sete	%al
	movzbl	%al, %eax
	ret
f_cmp:	leaq	var(%rip), %rcx
	xorl	%eax, %eax
	cmpq	var@GOTPCREL(%rip), %rcx
	sete	%al
	ret
	.data
	.globl	var
var:	.quad	7
	.section .note.GNU-stack,"",@progbits
"""
_GOTX_MAIN = r"""#include <stdio.h>
long f_mov(void), f_movr9(void);
int f_mov32(void), f_call(void), f_jmp(void), f_high(void), f_dso(void), f_test(void),
    f_add(void), f_cmp(void);
int main(void){
  printf("%ld %ld %d %d %d %d %d %d %d %d\n", f_mov(), f_movr9(), f_mov32(), f_call(), f_jmp(),
         f_high(), f_dso(), f_test(), f_add(), f_cmp());
  return 0;
}
"""


def _gotpcrelx_relax_test(args, oracles):
    """GOTPCRELX relaxation matches GNU ld instruction for instruction.

    Every relaxable form (mov -> mov-imm without PIC / lea with it, a REX.R
    destination, the 32-bit mov, call -> addr32 call, jmp -> jmp+nop,
    test/add/cmp -> immediate forms without PIC) and every form that must
    stay GOT-indirect (a high-half load with addend 0, a DSO symbol) is
    linked by both linkers as PIE and non-PIE.  Checked: identical runtime
    output, identical per-function instruction shapes (mnemonics and
    operands with addresses masked), and no more GLOB_DAT relocations than
    GNU ld.
    """
    name = "gotpcrelx_relax_vs_bfd"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "f.s"), "w") as f:
            f.write(_GOTX_ASM)
        with open(os.path.join(td, "m.c"), "w") as f:
            f.write(_GOTX_MAIN)
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        want = "7 7 1 42 42 1 1 1 1 1\n"

        def shape(exe):
            out = sh(["objdump", "-d", "--no-show-raw-insn", exe], cwd=td).stdout.decode()
            lines, fn = [], None
            for ln in out.splitlines():
                m = re.match(r"^[0-9a-f]+ <(f_\w+)>:$", ln)
                if m:
                    fn = m.group(1)
                    continue
                if not ln.strip():
                    fn = None
                    continue
                if fn and re.match(r"^\s+[0-9a-f]+:", ln):
                    insn = re.sub(r"^\s+[0-9a-f]+:\s*", "", ln)
                    insn = re.sub(r"0x[0-9a-f]+|\b[0-9a-f]{4,}\b|<[^>]*>|#.*", "X", insn)
                    lines.append(f"{fn}: {' '.join(insn.split())}")
            return lines

        for mode in ("-pie", "-no-pie"):
            shapes, glob = {}, {}
            for tag, bflag in (("lccc", ["-B" + shim]), ("gnu", [])):
                exe = f"t{mode}.{tag}"
                r = sh([CC] + bflag + ["-O1", mode, "f.s", "m.c", "-o", exe], cwd=td)
                if r.returncode != 0:
                    return Result(name, "FAIL", f"{mode}/{tag} link: {r.stderr.decode()[:300]}")
                run = sh([os.path.join(td, exe)], cwd=td)
                if run.returncode != 0 or run.stdout.decode() != want:
                    return Result(name, "FAIL",
                                  f"{mode}/{tag}: got {run.stdout.decode()!r} want {want!r}")
                shapes[tag] = shape(exe)
                relocs = sh(["readelf", "-W", "-r", exe], cwd=td).stdout.decode()
                glob[tag] = relocs.count("R_X86_64_GLOB_DAT")
            if shapes["lccc"] != shapes["gnu"]:
                diff = [f"{a} | {b}" for a, b in zip(shapes["lccc"], shapes["gnu"]) if a != b]
                return Result(name, "FAIL", f"{mode}: instruction shapes differ: {diff[:4]}")
            if not shapes["lccc"]:
                return Result(name, "FAIL", f"{mode}: no f_* functions disassembled")
            if glob["lccc"] > glob["gnu"]:
                return Result(name, "FAIL", f"{mode}: GLOB_DAT {glob['lccc']} > GNU ld {glob['gnu']}")
        return Result(name, "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


_GOTEDGE_ASM = r"""	.text
	.globl	g_rexb, g_lcall, g_lcmp, g_ladd, g_got64, g_sec, g_code4
# REX.B set on a RIP-relative load into %rax (REX.B selects nothing there).
g_rexb:	.byte	0x49, 0x8b, 0x05
	.reloc	., R_X86_64_REX_GOTPCRELX, gvar-4
	.long	0
	movq	(%rax), %rax
	ret
# REX-prefixed indirect call through a LOCAL function's slot.
g_lcall: subq	$8, %rsp
	.byte	0x40, 0xff, 0x15
	.reloc	., R_X86_64_REX_GOTPCRELX, lfunc-4
	.long	0
	addq	$8, %rsp
	ret
lfunc:	movl	$55, %eax
	ret
# cmp against a local's slot: an immediate without PIC, a real slot with it.
g_lcmp:	leaq	lvar(%rip), %rcx
	xorl	%eax, %eax
	cmpq	lvar@GOTPCREL(%rip), %rcx
	sete	%al
	ret
# Plain GOTPCREL (no relaxation promise) on an add of a local.
g_ladd:	xorl	%eax, %eax
	.byte	0x48, 0x03, 0x05
	.reloc	., R_X86_64_GOTPCREL, lvar-4
	.long	0
	leaq	lvar(%rip), %rcx
	cmpq	%rcx, %rax
	sete	%al
	movzbl	%al, %eax
	ret
# Large-model GOT64 slot offset of a local.
g_got64: leaq	_GLOBAL_OFFSET_TABLE_(%rip), %rcx
	movabsq	$lvar@GOT, %rax
	movq	(%rcx,%rax), %rax
	leaq	lvar(%rip), %rcx
	cmpq	%rcx, %rax
	sete	%al
	movzbl	%al, %eax
	ret
# A section symbol through the GOT (empty st_name).
g_sec:	leaq	.rodata.edge(%rip), %rcx
	xorl	%eax, %eax
	cmpq	.rodata.edge@GOTPCREL(%rip), %rcx
	sete	%al
	ret
# REX2 call: no prefix-preserving rewrite exists (never executed: APX).
g_code4: .byte	0xd5, 0x00, 0xff, 0x15
	.reloc	., R_X86_64_CODE_4_GOTPCRELX, lfunc-4
	.long	0
	ret
	.byte	0x4c
# REX_GOTPCRELX at offset 2 of its section: no REX byte in the section;
# the 0x4c above belongs to the previous one and must not be touched.
	.section .text.edge2,"ax",@progbits
	.globl	g_start2
g_start2: .byte	0x8b, 0x05
	.reloc	., R_X86_64_REX_GOTPCRELX, gvar-4
	.long	0
	leaq	gvar_l(%rip), %rcx
	cmpl	%ecx, %eax
	sete	%al
	movzbl	%al, %eax
	ret
	.data
	.globl	gvar
	.p2align 3
gvar:
gvar_l:	.quad	7
lvar:	.quad	9
	.section .rodata.edge,"a",@progbits
	.quad	5
	.section .note.GNU-stack,"",@progbits
"""
_GOTEDGE_MAIN = r"""#include <stdio.h>
long g_rexb(void);
int g_lcall(void), g_lcmp(void), g_ladd(void), g_got64(void), g_sec(void), g_start2(void);
int main(int argc, char **argv){
  (void)argv;
  /* GNU ld 2.47 relaxes g_rexb into a move to %r8 (see the test), so the
     oracle run passes an argument and skips the call. */
  printf("%ld %d %d %d %d %d %d\n", argc > 1 ? 7 : g_rexb(), g_lcall(), g_lcmp(), g_ladd(),
         g_got64(), g_sec(), g_start2());
  return 0;
}
"""


def _got64_spelling_asm():
    """One `checkN` per pad size, so a single object carries all three.

    `checkN` reads `lvarN@GOT` through `_GLOBAL_OFFSET_TABLE_` and answers
    whether the loaded value equals `lvarN`'s own address.  The pad BEFORE
    each `lvarN` is what makes its section offset non-zero -- and a non-zero
    offset is exactly what makes the assembler's spelling choice observable
    (`R_X86_64_GOT64 .data + 8` on binutils <= 2.43 versus
    `R_X86_64_GOT64 lvar + 0` on >= 2.44).

    The result is a COMPARISON rather than an inspection because
    `R_X86_64_GOT64` stores the relocated address in the slot and the slot's
    plain offset in the field: folding the addend into the field reads eight
    bytes past the slot, which is a value that can look plausible until it is
    compared against the thing it was supposed to be.
    """
    parts = ["\t.text"]
    for n in (0, 1, 2):
        parts += [
            f"\t.globl\tcheck{n}",
            f"\t.type\tcheck{n}, @function",
            f"check{n}:",
            "\tleaq\t_GLOBAL_OFFSET_TABLE_(%rip), %rcx",
            f"\tmovabsq\t$lvar{n}@GOT, %rax",
            "\tmovq\t(%rcx,%rax), %rax",
            f"\tleaq\tlvar{n}(%rip), %rcx",
            "\tcmpq\t%rcx, %rax",
            "\tsete\t%al",
            "\tmovzbl\t%al, %eax",
            "\tret",
            f"\t.size\tcheck{n}, .-check{n}",
        ]
    parts += ["\t.data", "\t.p2align 3"]
    for n in (0, 1, 2):
        parts += [f"lvar{n}:", f"\t.quad\t{0x1111 + n * 0x111}", "\t.p2align 3"]
        if n < 2:
            parts += ["\t.zero\t8"]
    parts += ['\t.section .note.GNU-stack,"",@progbits', ""]
    return "\n".join(parts)


# Every check must answer 1: the GOT slot holds the address it names.
GOT64_WANT = "1 1 1\n"

_GOT64_SPELLING_MAIN = r"""#include <stdio.h>
/* extern, NOT static: the definitions are GLOBAL symbols in the fixture.  A
   `static` declaration here declares a different, local, undefined function,
   so every call binds to 0 and every check answers 0 -- which makes the whole
   test pass while testing nothing.  The expected value is asserted, so that
   cannot go unnoticed. */
int check0(void), check1(void), check2(void);
int main(void){ printf("%d %d %d\n", check0(), check1(), check2()); return 0; }
"""


def _elf_vread(d, va, n):
    """`n` bytes at virtual address `va` of an ELF64 image (via PT_LOAD)."""
    for typ, _fl, off, pva, fsz, _msz, _al in _elf_bytes_phdrs(d) or []:
        if typ == _PT_LOAD and pva <= va and va + n <= pva + fsz:
            return d[off + va - pva: off + va - pva + n]
    return None


def _nm_addrs(path, td):
    """Symbol -> address from `nm` (locals included; first definition wins)."""
    out = {}
    for ln in sh(["nm", path], cwd=td).stdout.decode().splitlines():
        parts = ln.split()
        if len(parts) == 3:
            out.setdefault(parts[2], int(parts[0], 16))
    return out


def _reloc_sites_by_function(obj, td, rtype):
    """Function -> offsets (within the function) of its `rtype` relocations.

    Locates instructions through the relocation table of the *object*
    instead of assuming a fixed distance from the function's entry: a
    compiler defaulting to `-fcf-protection` (Ubuntu's gcc) opens every
    function with a 4-byte `endbr64`, and `-pg`, `-fpatchable-function-entry`
    or a different prologue shift the body just the same.  A function spans
    [st_value, st_value + st_size); an assembly label without `.size` ends
    at the next symbol of its section (or the section end).
    """
    syms = []  # (shndx, value, size, name)
    for ln in sh(["readelf", "-W", "-s", obj], cwd=td).stdout.decode().splitlines():
        f = ln.split()
        if len(f) == 8 and f[0].endswith(":") and f[6].isdigit() and f[3] in ("FUNC", "NOTYPE"):
            syms.append((int(f[6]), int(f[1], 16), int(f[2], 0), f[7]))
    shdr = {}  # section name -> (index, size)
    for ln in sh(["readelf", "-W", "-S", obj], cwd=td).stdout.decode().splitlines():
        m = re.match(r" *\[ *(\d+)\] +(\S+) +\S+ +[0-9a-f]+ +[0-9a-f]+ +([0-9a-f]+)", ln)
        if m:
            shdr[m.group(2)] = (int(m.group(1)), int(m.group(3), 16))
    out = {}
    target = None
    for ln in sh(["readelf", "-W", "-r", obj], cwd=td).stdout.decode().splitlines():
        m = re.match(r"Relocation section '\.rela(\S+)'", ln)
        if m:
            target = shdr.get(m.group(1))
            continue
        f = ln.split()
        if target is None or len(f) < 3 or f[2] != rtype:
            continue
        ndx, sec_size = target
        off = int(f[0], 16)
        starts = sorted((v, sz, n) for (sx, v, sz, n) in syms if sx == ndx)
        for i, (v, sz, n) in enumerate(starts):
            end = v + sz if sz else (starts[i + 1][0] if i + 1 < len(starts) else sec_size)
            if v <= off < end:
                out.setdefault(n, []).append(off - v)
    return out


def _relative_addends(path, td):
    """r_offset -> r_addend of every R_X86_64_RELATIVE."""
    out = {}
    for ln in sh(["readelf", "-W", "-r", path], cwd=td).stdout.decode().splitlines():
        f = ln.split()
        if len(f) >= 4 and f[2] == "R_X86_64_RELATIVE":
            out[int(f[0], 16)] = int(f[3], 16)
    return out


def _gotpcrel_edges_test(args, oracles):
    """GOT-indirect references relaxation must NOT rewrite get a real slot.

    Regressions for three defects of the GOTPCRELX relaxer and the slot
    planner, each asserted on exact bytes of the lccc-ld output (and on the
    runtime result, cross-checked against GNU ld):

    * `49 8b 05` (REX.B set, loads %rax) relaxed to `49 c7 c0` -- a move
      into %r8 -- because the old REX.B survived next to the moved REX.R.
      Must become `48 c7 c0` without PIC.  GNU ld 2.44 and 2.47 have the
      same bug (measured: `49 c7 c0`, SIGSEGV), so the GNU runs skip that
      call and it is checked on lccc-ld's bytes and runtime alone.
    * A REX2 `call *` (`d5 xx ff 15`) became `d5 xx 67 e8`, leaving REX2 in
      front of a prefix (`(bad)`; GNU ld 2.47 emits the same bytes); and a REX_GOTPCRELX at offset 2 of its section had
      its "REX byte" -- the previous section's last byte -- rewritten.  Both
      must stay GOT-indirect with the surrounding bytes intact.
    * A LOCAL symbol never got a slot in an executable, so every reference
      the relaxer declines (REX `call *`, PIE `cmp`, plain GOTPCREL `add`,
      large-model `@GOT`, a section symbol) was patched into a load of the
      symbol's own bytes or refused.  Each must read a slot holding the
      address (a RELATIVE in a PIE), in executables and shared objects.
    """
    name = "gotpcrel_edges"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "e.s"), "w") as f:
            f.write(_GOTEDGE_ASM)
        with open(os.path.join(td, "m.c"), "w") as f:
            f.write(_GOTEDGE_MAIN)
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        want = "7 55 1 1 1 1 1\n"
        if sh(["as", "e.s", "-o", "e.o"], cwd=td).returncode != 0:
            return Result(name, "FAIL", "fixture does not assemble")
        errs = []

        def slot_target(d, relas, ins_va, disp_off, pie, what, expect):
            """The rel32 at ins_va+disp_off must address a GOT slot whose
            value (non-PIE) or RELATIVE addend (PIE) is `expect`."""
            disp = int.from_bytes(_elf_vread(d, ins_va + disp_off, 4), "little", signed=True)
            slot = ins_va + disp_off + 4 + disp
            if pie:
                got = relas.get(slot)
            else:
                raw = _elf_vread(d, slot, 8)
                got = int.from_bytes(raw, "little") if raw else None
            if got != expect:
                errs.append(f"{what}: slot {slot:#x} holds {got!r}, want {expect:#x}")

        for mode in ("-no-pie", "-pie", "-shared"):
            for tag, bflag in (("lccc", ["-B" + shim]), ("gnu", [])):
                exe = f"t{mode}.{tag}"
                if mode == "-shared":
                    lib = f"libe.{tag}.so"
                    r = sh([CC] + bflag + ["-shared", "e.o", "-o", lib], cwd=td)
                    if r.returncode != 0:
                        return Result(name, "FAIL", f"{mode}/{tag}: {r.stderr.decode()[:300]}")
                    r = sh([CC, "m.c", lib, "-Wl,-rpath," + td, "-o", exe], cwd=td)
                else:
                    r = sh([CC] + bflag + [mode, "e.o", "m.c", "-o", exe], cwd=td)
                if r.returncode != 0:
                    return Result(name, "FAIL", f"{mode}/{tag} link: {r.stderr.decode()[:300]}")
                run = sh([os.path.join(td, exe)] + ([] if tag == "lccc" else ["skip-rexb"]),
                         cwd=td)
                if run.returncode != 0 or run.stdout.decode() != want:
                    return Result(name, "FAIL",
                                  f"{mode}/{tag}: got {run.stdout.decode()!r} rc {run.returncode}")
            img = f"libe.lccc.so" if mode == "-shared" else f"t{mode}.lccc"
            pic = mode != "-no-pie"
            path = os.path.join(td, img)
            with open(path, "rb") as f:
                d = f.read()
            syms = _nm_addrs(path, td)
            relas = _relative_addends(path, td)
            ins = lambda s, n: _elf_vread(d, syms[s], n)
            gvar, lvar, lfunc = syms["gvar"], syms["lvar"], syms["lfunc"]
            # g_rexb: gvar is preemptible in a .so (GLOB_DAT slot).
            if mode == "-no-pie":
                if ins("g_rexb", 7) != bytes([0x48, 0xc7, 0xc0]) + gvar.to_bytes(4, "little"):
                    errs.append(f"{mode} g_rexb: {ins('g_rexb', 7).hex()} want 48c7c0+&gvar")
            elif mode == "-pie":
                if ins("g_rexb", 3) != bytes([0x49, 0x8d, 0x05]):
                    errs.append(f"{mode} g_rexb: {ins('g_rexb', 3).hex()} want 498d05 (lea)")
            # g_lcall: REX call never rewritten; local slot = &lfunc.
            lc = syms["g_lcall"] + 4
            if _elf_vread(d, lc, 3) != bytes([0x40, 0xff, 0x15]):
                errs.append(f"{mode} g_lcall: {_elf_vread(d, lc, 3).hex()} want 40ff15")
            slot_target(d, relas, lc, 3, pic, f"{mode} g_lcall", lfunc)
            # g_lcmp: `cmp $lvar, %rcx` without PIC, else a slot.
            cm = syms["g_lcmp"] + 9
            if mode == "-no-pie":
                if _elf_vread(d, cm, 7) != bytes([0x48, 0x81, 0xf9]) + lvar.to_bytes(4, "little"):
                    errs.append(f"{mode} g_lcmp: {_elf_vread(d, cm, 7).hex()} want 4881f9+&lvar")
            else:
                if _elf_vread(d, cm, 3) != bytes([0x48, 0x3b, 0x0d]):
                    errs.append(f"{mode} g_lcmp: {_elf_vread(d, cm, 3).hex()} want 483b0d")
                slot_target(d, relas, cm, 3, pic, f"{mode} g_lcmp", lvar)
            # g_ladd: plain GOTPCREL keeps its slot in every mode.
            la = syms["g_ladd"] + 2
            if _elf_vread(d, la, 3) != bytes([0x48, 0x03, 0x05]):
                errs.append(f"{mode} g_ladd: {_elf_vread(d, la, 3).hex()} want 480305")
            slot_target(d, relas, la, 3, pic, f"{mode} g_ladd", lvar)
            # g_code4: REX2 call untouched, slot = &lfunc.
            c4 = syms["g_code4"]
            if _elf_vread(d, c4, 4) != bytes([0xd5, 0x00, 0xff, 0x15]):
                errs.append(f"{mode} g_code4: {_elf_vread(d, c4, 4).hex()} want d500ff15")
            slot_target(d, relas, c4, 4, pic, f"{mode} g_code4", lfunc)
            # g_start2: unrelaxed, previous section's 0x4c intact.
            st = syms["g_start2"]
            if _elf_vread(d, st - 1, 3) != bytes([0x4c, 0x8b, 0x05]):
                errs.append(f"{mode} g_start2: {_elf_vread(d, st - 1, 3).hex()} want 4c8b05")
        if errs:
            return Result(name, "FAIL", "; ".join(errs[:6]))
        return Result(name, "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


def _assembler_variants():
    """Every assembler this run should cross-check, as `(label, argv)`.

    A relocation's SPELLING is a property of the ASSEMBLER, not of the linker.
    The measured example: for a LOCAL `$lvar@GOT` with no symbol-table entry
    of its own, binutils <= 2.43 emits `R_X86_64_GOT64 .data + 8` (the section
    symbol with the offset folded into the addend) while >= 2.44 emits
    `R_X86_64_GOT64 lvar + 0`.  Both name one address, but a linker that
    handles only one of them is silently wrong on exactly the toolchain that
    produced the other -- and a suite that assembles every fixture with a
    single pinned `as` cannot see the difference at all.  The GOT64 defect
    this branch repaired was invisible in CI for precisely that reason.

    So the same fixture is assembled by every assembler on this list and each
    one is required to agree with GNU ld.  `LCCC_ASSEMBLERS` is a
    colon-separated list of extra assembler binaries (typically one older
    build); the default `as` from PATH is always included.  A variant that
    cannot assemble a given fixture is SKIPPED for it, never failed: an old
    `as` has no REX2/APX, and that is a property of the tool, not of the
    linker.
    """
    out = [("as", ["as"])]
    for spec in (os.environ.get("LCCC_ASSEMBLERS") or "").split(":"):
        spec = spec.strip()
        if not spec:
            continue
        if not (os.path.isabs(spec) and os.access(spec, os.X_OK)):
            # A configured assembler that is not there is a broken runner.
            out.append((spec, [spec]))
            continue
        out.append((os.path.basename(os.path.dirname(spec)) + "/" +
                    os.path.basename(spec), [spec]))
    return out


def _as_version(argv):
    """`GNU assembler (GNU Binutils) 2.42` -> `2.42`; '' when unknown."""
    r = sh(argv + ["--version"])
    if r.returncode != 0:
        return ""
    m = re.search(r"(\d+\.\d+(?:\.\d+)?)\s*$", r.stdout.decode(errors="replace").splitlines()[0])
    return m.group(1) if m else ""


def _got64_spelling_matrix_test(args, oracles):
    """One fixture, every assembler, each required to agree with GNU ld.

    The fixture is the shape whose spelling is version-sensitive: a local
    `$lvar@GOT` read through `_GLOBAL_OFFSET_TABLE_`, where the answer is
    compared against `lvar`'s own address at runtime.  `R_X86_64_GOT64` stores
    the relocated ADDRESS in the slot and the slot's plain offset in the
    relocation field, so an implementation that folds the addend into the
    field reads eight bytes past the slot -- a value that can look plausible
    until it is compared, which is why the check is a comparison and not an
    inspection.

    Three offsets are covered, because the offset is what selects the
    spelling: 0 (the address IS the section base, so `+ 0` either way) and
    two non-zero offsets (where <= 2.43 and >= 2.44 genuinely differ).  Each
    assembler is reported with the relocations it actually emitted, so a
    future binutils that changes the spelling again shows up in the output
    instead of silently narrowing what is covered.
    """
    name = "got64_spelling_matrix"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        for pad in (0, 1, 2):
            with open(os.path.join(td, f"t{pad}.s"), "w") as f:
                f.write(_got64_spelling_asm())
        with open(os.path.join(td, "m.c"), "w") as f:
            f.write(_GOT64_SPELLING_MAIN)
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        variants = _assembler_variants()
        ran, spells, skipped, oracles_differ = 0, [], [], []
        for label, argv in variants:
            for pad in (0, 1, 2):
                obj = f"t{pad}.{label}.o"
                r = sh(argv + [f"t{pad}.s", "-o", obj], cwd=td)
                if r.returncode != 0:
                    skipped.append(f"{label}@{pad}:assemble")
                    continue
                spells.append("%s/%s: %s" % (label, pad, _got64_spellings(obj, td)))
                for mode in ("-no-pie", "-pie"):
                    outs = {}
                    for tag, bflag in (("lccc", ["-B" + shim]), ("gnu", [])):
                        exe = f"a{pad}{mode}.{label}.{tag}"
                        lr = sh([CC] + bflag + [mode, obj, "m.c", "-o", exe], cwd=td)
                        if lr.returncode != 0:
                            return Result(name, "FAIL",
                                          f"{label}@{pad} {mode}/{tag} link: "
                                          f"{lr.stderr.decode()[:300]}")
                        run = sh([os.path.join(td, exe)], cwd=td)
                        outs[tag] = (run.returncode, run.stdout.decode())
                    # THE SPECIFICATION IS THE ASSERTION, NOT THE ORACLE.
                    #
                    # Each `checkN` must answer 1: the GOT slot holds the
                    # address it names.  That is checkable without a reference,
                    # and it is asserted directly, because two linkers that are
                    # wrong in the same way agree perfectly -- and a fixture
                    # that fails to bind (a `static` declaration against global
                    # asm symbols) makes every check answer 0, so "lccc == GNU"
                    # is not evidence and neither is "lccc == GNU == 0 0 0".
                    if outs["lccc"] != (0, GOT64_WANT):
                        return Result(name, "FAIL",
                                      f"{label}@{pad} {mode}: lccc-ld gives "
                                      f"{outs['lccc']}, want {GOT64_WANT} "
                                      f"(spelling: {spells[-1]})")
                    # GNU ld is reported, not required.  On binutils <= 2.43
                    # it answers `1 0 0` here: it keys the local GOT slot on
                    # the SECTION symbol, so three locals at .data+0, +0x10 and
                    # +0x20 collapse onto one slot and the field then points
                    # past it.  lccc-ld is deliberately more correct than the
                    # reference on this shape; see `LocalSlots` in
                    # src/backend/x86/linker/types.rs.
                    if outs["gnu"] != outs["lccc"]:
                        oracles_differ.append(
                            f"{label}@{pad} {mode}: GNU ld {outs['gnu']!r}, "
                            f"lccc-ld {outs['lccc']!r}")
                ran += 1
        if ran == 0:
            return Result(name, "SKIP",
                          "no assembler could build the fixture: %s" % ", ".join(skipped))
        detail = ("%d fixture(s) x %d assembler(s); relocations seen: %s"
                  % (ran, len(variants), "; ".join(spells[:3])))
        if oracles_differ:
            detail += " | ORACLE DISAGREES (lccc-ld follows the psABI): " + \
                      "; ".join(oracles_differ[:3])
        return Result(name, "PASS", detail)
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


def _got64_spellings(obj, td):
    """The GOT-family relocations `obj` actually carries, compactly.

    `readelf -rW` prints `Offset Info Type Value Name+Addend`; only the last
    two columns are interesting here, and only for the GOT family -- the
    `R_X86_64_64` rows for `_GLOBAL_OFFSET_TABLE_` are an artefact of reading
    the GOT base, not of the relocation under test.
    """
    d = sh(["readelf", "-rW", obj], cwd=td).stdout.decode(errors="replace")
    out = []
    for line in d.splitlines():
        if "R_X86_64_" not in line:
            continue
        rtype = re.search(r"(R_X86_64_\w+)", line).group(1)
        if not rtype.startswith("R_X86_64_GOT"):
            continue
        tail = line.split(rtype, 1)[1].split(None, 1)
        out.append(rtype.replace("R_X86_64_", "") + ":" +
                   (tail[1].strip() if len(tail) > 1 else ""))
    return ",".join(out) or "(none)"



# `objabs` lives in its own object: GAS refuses `@GOTPCREL` on a symbol it
# already knows to be absolute.
_ABSSYM_DEF = r"""	.globl	objabs
	.set	objabs, 0x777
	.section .note.GNU-stack,"",@progbits
"""
_ABSSYM_ASM = r"""	.text
	.globl	a_obj, a_def, a_rel, a_expr, a_anchor4, a_ptr
a_obj:	movq	objabs@GOTPCREL(%rip), %rax
	ret
a_def:	movq	defabs@GOTPCREL(%rip), %rax
	ret
a_rel:	movq	defrel@GOTPCREL(%rip), %rax
	ret
a_expr:	movq	defexpr@GOTPCREL(%rip), %rax
	ret
a_anchor4: leaq	anchor_l+4(%rip), %rax
	ret
a_ptr:	leaq	ptrs_l(%rip), %rax
	movq	(%rax,%rdi,8), %rax
	ret
	.data
	.globl	anchor, anchor_end
	.p2align 3
anchor:
anchor_l: .quad	1, 2
anchor_end:
ptrs_l:	.quad	objabs, defabs, defrel, defexpr
	.section .note.GNU-stack,"",@progbits
"""
_ABSSYM_MAIN = r"""#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
long a_obj(void), a_def(void), a_rel(void), a_expr(void), a_anchor4(void), a_ptr(long);
int main(void){
  long o = (long)dlsym(RTLD_DEFAULT, "objabs"), d = (long)dlsym(RTLD_DEFAULT, "defabs");
  long x = (long)dlsym(RTLD_DEFAULT, "defexpr"), r = (long)dlsym(RTLD_DEFAULT, "defrel");
  long a4 = a_anchor4();
  printf("%lx %lx %d %ld | %lx %lx %d %ld | %lx %lx %ld %d\n",
         a_obj(), a_def(), a_rel() == a4, a_expr(),
         a_ptr(0), a_ptr(1), a_ptr(2) == a4, a_ptr(3), o, d, x, r == a4);
  return 0;
}
"""


def _absolute_symbols_pic_test(args, oracles):
    """Absolute symbols never slide; linker-created addresses always do.

    Every symbol the linker creates -- `__ehdr_start`, `_end`, `__start_X`,
    any `--defsym` -- used to be stored as SHN_ABS, and the PIE/.so code
    could not tell a CONSTANT (`--defsym defabs=0x1234`, an object's
    `.set objabs, 0x777`, `--defsym defexpr=anchor_end-anchor`) from an
    ADDRESS (`--defsym defrel=anchor+4`).  A PIE slid the constants
    (RELATIVE on their GOT slots and `.quad`s, `.dynsym` claiming a real
    section so ld.so added the base on lookup); a shared object froze the
    address (no RELATIVE, exported SHN_ABS).  Checked: the runtime values in
    a PIE and a .so, exact `.dynsym` st_shndx, and no RELATIVE carrying a
    constant.

    GNU ld is no oracle here: 2.44 and 2.47 both relax `defabs@GOTPCREL` to
    a RIP-relative `lea` in a PIE (the constant slides), export `defrel` as
    SHN_ABS (it does not), and leave `.quad defrel` unrelocated (measured:
    `777 <base+1234> 1 <base+16> | 777 1234 0 16 | 777 1234 16 0`).
    """
    name = "absolute_symbols_pic"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "a.s"), "w") as f:
            f.write(_ABSSYM_ASM)
        with open(os.path.join(td, "abs.s"), "w") as f:
            f.write(_ABSSYM_DEF)
        with open(os.path.join(td, "m.c"), "w") as f:
            f.write(_ABSSYM_MAIN)
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        defs = "-Wl,--defsym=defabs=0x1234,--defsym=defrel=anchor+4,--defsym=defexpr=anchor_end-anchor"
        want = "777 1234 1 16 | 777 1234 1 16 | 777 1234 16 1\n"
        errs = []
        for mode in ("-pie", "-shared"):
            for tag, bflag in (("lccc", ["-B" + shim]),):
                exe = f"t{mode}.{tag}"
                if mode == "-shared":
                    img = f"liba.{tag}.so"
                    r = sh([CC] + bflag + ["-shared", "a.s", "abs.s", defs, "-o", img], cwd=td)
                    if r.returncode != 0:
                        return Result(name, "FAIL", f"{mode}/{tag}: {r.stderr.decode()[:300]}")
                    r = sh([CC, "m.c", img, "-Wl,-rpath," + td, "-ldl", "-o", exe], cwd=td)
                else:
                    img = exe
                    r = sh([CC] + bflag + ["-pie", "-rdynamic", "a.s", "abs.s", "m.c", defs, "-ldl",
                                           "-o", exe], cwd=td)
                if r.returncode != 0:
                    return Result(name, "FAIL", f"{mode}/{tag} link: {r.stderr.decode()[:300]}")
                run = sh([os.path.join(td, exe)], cwd=td)
                if run.returncode != 0 or run.stdout.decode() != want:
                    errs.append(f"{mode}/{tag}: got {run.stdout.decode()!r} want {want!r}")
                    continue
                dyn = sh(["readelf", "-W", "--dyn-syms", img], cwd=td).stdout.decode()
                ndx = {}
                for ln in dyn.splitlines():
                    f = ln.split()
                    if len(f) >= 8 and f[0].endswith(":"):
                        ndx[f[7].split("@")[0]] = f[6]
                for sym, absolute in (("objabs", True), ("defabs", True), ("defexpr", True),
                                      ("defrel", False), ("anchor", False)):
                    got = ndx.get(sym)
                    if got is None or (got == "ABS") != absolute:
                        errs.append(f"{mode}: .dynsym {sym} st_shndx {got!r}, "
                                    f"want {'ABS' if absolute else 'a section'}")
                bad = {hex(a) for a in _relative_addends(os.path.join(td, img), td).values()
                       if a in (0x777, 0x1234, 16)}
                if bad:
                    errs.append(f"{mode}: RELATIVE with a constant addend {sorted(bad)}")
        if errs:
            return Result(name, "FAIL", "; ".join(errs[:6]))
        return Result(name, "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


_IELE_C = r"""#include <stdio.h>
static __thread int a __attribute__((tls_model("initial-exec"))) = 11;
static __thread int b __attribute__((tls_model("initial-exec"))) = 22;
__attribute__((noinline)) int *pb(void) { return &b; }
__attribute__((noinline)) int *pa(void) { return &a; }
int ie_r12(void), ie_add(void);
int main(void) { printf("%d %d %d %d\n", *pa(), *pb(), ie_r12(), ie_add()); return 0; }
"""
_IELE_ASM = r"""	.text
	.globl	ie_r12, ie_add
ie_r12:	pushq	%r12
	movq	tv@gottpoff(%rip), %r12
	movl	%fs:(%r12), %eax
	popq	%r12
	ret
ie_add:	movq	%fs:0, %rax
	addq	tv@gottpoff(%rip), %rax
	movl	(%rax), %eax
	ret
	.section .tdata,"awT",@progbits
	.p2align 2
tv:	.long	33
	.section .note.GNU-stack,"",@progbits
"""


def _ie_to_le_local_test(args, oracles):
    """Initial-Exec -> Local-Exec of LOCAL TLS symbols in an executable.

    A local symbol has no GOT slot in an executable, so every GOTTPOFF
    against one is rewritten to an immediate.  The immediate used to be
    `tpoff + addend`, and the addend of a RIP-relative GOTTPOFF is the -4 of
    the pc-relative field -- so `&b` pointed 4 bytes low (at `a`).  Also
    checked: the destination register survives the move from ModRM.reg to
    ModRM.rm (REX.R -> REX.B; %r12 must not become %rsp), `addq` forms, and
    the exact immediates against the TLS layout, in dynamic and static
    executables, next to GNU ld's runtime output.
    """
    name = "ie_to_le_local"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "t.c"), "w") as f:
            f.write(_IELE_C)
        with open(os.path.join(td, "ie.s"), "w") as f:
            f.write(_IELE_ASM)
        r = sh([CC, "-O1", "-fPIC", "-c", "t.c", "ie.s"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"compile: {r.stderr.decode()[:300]}")
        ie_sites = {}
        for obj in ("t.o", "ie.o"):
            ie_sites.update(_reloc_sites_by_function(obj, td, "R_X86_64_GOTTPOFF"))
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        want = "11 22 33 33\n"
        for mode in ([], ["-static"]):
            for tag, bflag in (("lccc", ["-B" + shim]), ("gnu", [])):
                exe = f"t{''.join(mode)}.{tag}"
                r = sh([CC] + bflag + mode + ["t.o", "ie.o", "-o", exe], cwd=td)
                if r.returncode != 0:
                    return Result(name, "FAIL", f"{mode}/{tag} link: {r.stderr.decode()[:300]}")
                run = sh([os.path.join(td, exe)], cwd=td)
                if run.returncode != 0 or run.stdout.decode() != want:
                    return Result(name, "FAIL",
                                  f"{mode}/{tag}: got {run.stdout.decode()!r} rc {run.returncode}")
            path = os.path.join(td, f"t{''.join(mode)}.lccc")
            with open(path, "rb") as f:
                d = f.read()
            tls = [p for p in _elf_bytes_phdrs(d) if p[0] == 7]  # PT_TLS
            if len(tls) != 1:
                return Result(name, "FAIL", f"{mode}: {len(tls)} PT_TLS headers")
            _t, _fl, _off, tva, _fsz, tmsz, tal = tls[0]
            block = (tmsz + tal - 1) // tal * tal
            syms = _nm_addrs(path, td)

            def tpoff(sym):
                return (syms[sym] - tva - block) & 0xFFFFFFFF

            for fn, prefix, sym in (("pb", [0x48, 0xc7, 0xc0], "b"),
                                    ("pa", [0x48, 0xc7, 0xc0], "a"),
                                    ("ie_r12", [0x49, 0xc7, 0xc4], "tv"),
                                    ("ie_add", [0x48, 0x81, 0xc0], "tv")):
                # The rewritten instruction is REX + opcode + ModRM ahead of
                # the 4-byte field the GOTTPOFF relocation addressed.
                if len(ie_sites.get(fn, [])) != 1:
                    return Result(name, "FAIL", f"{fn}: GOTTPOFF sites {ie_sites.get(fn)} "
                                  "in the object, want exactly one")
                got = _elf_vread(d, syms[fn] + ie_sites[fn][0] - 3, 7)
                exp = bytes(prefix) + tpoff(sym).to_bytes(4, "little")
                if got != exp:
                    return Result(name, "FAIL", f"{mode} {fn}: {got.hex()} want {exp.hex()}")
        return Result(name, "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


_IEF_ASM = r"""	.text
	.globl	ie_movl, ie_gadd, apx
# movl ly@gottpoff(%rip), %ecx -- no REX.W, so no prefix byte the IE->LE
# rewrite may touch (GNU as refuses to assemble it; hand-encoded).
ie_movl:	.byte	0x8b, 0x0d
	.reloc	., R_X86_64_GOTTPOFF, ly-4
	.long	0
	movslq	%ecx, %rcx
	movl	%fs:(%rcx), %eax
	ret
# addq of the executable's own GLOBAL TLS symbol.
ie_gadd:	movq	%fs:0, %rax
	addq	gx@gottpoff(%rip), %rax
	movl	(%rax), %eax
	ret
# APX forms (encoded, never executed).
apx:	addq	ly@gottpoff(%rip), %rax, %r17
	addq	%r20, ly@gottpoff(%rip), %r9
	addq	gx@gottpoff(%rip), %r21, %r22
	{nf} addq	ly@gottpoff(%rip), %r18
	{nf} addq	ly@gottpoff(%rip), %r18, %r11
	movrs	ly@gottpoff(%rip), %r19
	movq	gx@gottpoff(%rip), %r25
	movq	ly@gottpoff(%rip), %r31
	addq	gx@gottpoff(%rip), %r16
	addq	ly@gottpoff(%rip), %r8
	ret
	.section .tdata,"awT",@progbits
	.p2align 2
ly:	.long	44
	.globl	gx
	.type	gx,@object
	.size	gx,4
gx:	.long	55
	.section .note.GNU-stack,"",@progbits
"""

# `apx` after the IE -> LE rewrite, one (bytes before the imm32, symbol)
# pair per instruction: GNU ld 2.47's output for the same object, except
# the last -- GNU writes `lea disp32(%r8), %r8` (4d 8d 80) where lccc-ld
# keeps `add $imm32, %r8`: same length and result (flags aside), and one
# form for every register (GNU itself must use `add` for %rsp).
_IEF_APX_WANT = (
    ("62f4f41081c0", "ly"),    # add $tpoff, %rax, %r17   (EVEX: 03 -> 81)
    ("62fcb41881c4", "ly"),    # add $tpoff, %r20, %r9    (01 + ND: same sum)
    ("62fccc1081c5", "gx"),    # add $tpoff, %r21, %r22
    ("62fcfc0c81c2", "ly"),    # {nf} add $tpoff, %r18
    ("62fca41c81c2", "ly"),    # {nf} add $tpoff, %r18, %r11
    ("2e2ed518c7c3", "ly"),    # movrs -> cs cs mov $tpoff, %r19 (REX2)
    ("d519c7c1", "gx"),        # mov $tpoff, %r25          (REX2 R -> B)
    ("d519c7c7", "ly"),        # mov $tpoff, %r31
    ("d51881c0", "gx"),        # add $tpoff, %r16
    ("4981c0", "ly"),          # add $tpoff, %r8
)


def _ie_to_le_forms_test(args, oracles):
    """Initial-Exec in an executable: every form rewritten or slotted.

    The planner and the relocation pass now share one per-reference
    decision (`exec_ie_target_local` + `elf::gottpoff_ie_to_le`).  Before,
    an executable's own GLOBAL TLS symbol always took a GOT slot and a load
    (no IE -> LE at all), while a LOCAL one never got a slot, so a
    reference the rewrite does not cover failed the link: APX EVEX forms
    (`R_X86_64_CODE_6_GOTTPOFF`, which GNU ld rewrites) and a non-REX.W
    `movl`, whose preceding byte belongs to another instruction.  Checked:
    runtime values; the exact rewritten bytes of the REX.W, REX2 and EVEX
    forms (GNU ld 2.47's encodings: `81 /0` with R moved to B, `movrs` to a
    REX2 `mov` behind `cs` prefixes); the non-REX.W reference reading a
    link-time slot that holds the TP offset and carries no dynamic
    relocation -- in PDE, PIE and static executables.
    """
    name = "ie_to_le_forms"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "ief.s"), "w") as f:
            f.write(_IEF_ASM)
        with open(os.path.join(td, "m.c"), "w") as f:
            f.write("#include <stdio.h>\nint ie_movl(void), ie_gadd(void);\n"
                    "int main(void) { printf(\"%d %d\\n\", ie_movl(), ie_gadd()); return 0; }\n")
        r = sh([CC, "-c", "ief.s"], cwd=td)
        if r.returncode != 0:
            err = r.stderr.decode()[:300]
            if "movrs" in err or "no such instruction" in err:
                return Result(name, "SKIP", f"assembler without APX/MOVRS: {err}")
            return Result(name, "FAIL", f"assemble: {err}")
        r = sh([CC, "-O1", "-c", "m.c"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"compile: {r.stderr.decode()[:300]}")
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        for mode in (["-no-pie"], ["-pie"], ["-static"]):
            exe = "t" + "".join(mode)
            r = sh([CC, "-B" + shim] + mode + ["m.o", "ief.o", "-o", exe], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"{mode} link: {r.stderr.decode()[:300]}")
            run = sh([os.path.join(td, exe)], cwd=td)
            if run.returncode != 0 or run.stdout.decode() != "44 55\n":
                return Result(name, "FAIL", f"{mode}: got {run.stdout.decode()!r} "
                              f"rc {run.returncode}")
            path = os.path.join(td, exe)
            with open(path, "rb") as f:
                d = f.read()
            tls = [p for p in _elf_bytes_phdrs(d) if p[0] == 7]  # PT_TLS
            if len(tls) != 1:
                return Result(name, "FAIL", f"{mode}: {len(tls)} PT_TLS headers")
            _t, _fl, _off, tva, _fsz, tmsz, tal = tls[0]
            block = (tmsz + tal - 1) // tal * tal
            syms = _nm_addrs(path, td)

            def tpoff(sym):
                return syms[sym] - tva - block

            def imm(sym):
                return (tpoff(sym) & 0xFFFFFFFF).to_bytes(4, "little")

            # addq gx@gottpoff(%rip), %rax  ->  addq $tpoff, %rax
            got = _elf_vread(d, syms["ie_gadd"] + 9, 7)
            if got != bytes.fromhex("4881c0") + imm("gx"):
                return Result(name, "FAIL", f"{mode} ie_gadd: {got.hex()}: the executable's "
                              "own TLS must be rewritten to an immediate, not loaded")
            want = b"".join(bytes.fromhex(pre) + imm(sym) for pre, sym in _IEF_APX_WANT)
            got = _elf_vread(d, syms["apx"], len(want) + 1)
            if got != want + b"\xc3":
                return Result(name, "FAIL", f"{mode} apx:\n  got  {got.hex()}\n"
                              f"  want {want.hex()}c3")
            # movl: unchanged instruction reading a slot that holds tpoff(ly).
            ins = _elf_vread(d, syms["ie_movl"], 6)
            if ins[:2] != b"\x8b\x0d":
                return Result(name, "FAIL", f"{mode} ie_movl rewritten: {ins.hex()}")
            slot = syms["ie_movl"] + 6 + struct.unpack("<i", ins[2:])[0]
            val = _elf_vread(d, slot, 8)
            if val is None or struct.unpack("<q", val)[0] != tpoff("ly"):
                return Result(name, "FAIL", f"{mode} slot 0x{slot:x} holds "
                              f"{val.hex() if val else None}, want tpoff {tpoff('ly')}")
            rel = sh(["readelf", "-rW", path], cwd=td).stdout.decode()
            if any(ln.split()[0].lstrip("0") == f"{slot:x}" for ln in rel.splitlines()
                   if ln[:1] in "0123456789abcdef" and ln.split()):
                return Result(name, "FAIL", f"{mode}: dynamic relocation against the TP-offset "
                              f"slot 0x{slot:x}:\n{rel[-600:]}")
        # A -T link has no GOT: the same rewrites (it used to patch only the
        # opcode -- `%r12` became `%rsp`, `addq` and REX2/EVEX forms failed or
        # were garbled), and a clear error for the unconvertible `movl`.
        # Its synthesised PT_TLS used to be overwritten by the PT_NOTE after
        # it whenever the input carried a note (GNU as 2.47's property note).
        with open(os.path.join(td, "s.ld"), "w") as f:
            f.write("ENTRY(apx)\nSECTIONS {\n  . = 0x400000;\n  .text : { *(.text) }\n"
                    "  . = ALIGN(0x1000);\n  .tdata : { *(.tdata) }\n  .tbss : { *(.tbss) }\n}\n")
        a = _IEF_ASM
        nomovl = a[:a.index("# movl ly")] + a[a.index("# addq of"):]
        with open(os.path.join(td, "ief2.s"), "w") as f:
            f.write(nomovl.replace("ie_movl, ", ""))
        note = "\t.section .note.x,\"a\",@note\n\t.p2align 2\n\t.long 4, 4, 1\n\t.ascii \"LCCC\"\n\t.long 0\n"
        with open(os.path.join(td, "note.s"), "w") as f:
            f.write(note)
        r = sh([CC, "-c", "ief2.s", "note.s"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"assemble ief2.s: {r.stderr.decode()[:300]}")
        r = sh([lccc_ld, "-T", "s.ld", "ief.o", "-o", "full.img"], cwd=td)
        if r.returncode == 0 or b"a -T link has no GOT" not in r.stderr:
            return Result(name, "FAIL", f"-T with an unconvertible GOTTPOFF: rc {r.returncode} "
                          f"{r.stderr[:300]!r}")
        r = sh([lccc_ld, "-T", "s.ld", "ief2.o", "note.o", "-o", "s.img"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"-T link: {r.stderr.decode()[:300]}")
        path = os.path.join(td, "s.img")
        with open(path, "rb") as f:
            d = f.read()
        phdrs = _elf_bytes_phdrs(d)
        types = [p[0] for p in phdrs]
        if types.count(7) != 1 or 0 in types or 4 not in types:
            return Result(name, "FAIL", f"-T: program header types {types}: want one PT_TLS, "
                          "the PT_NOTE, no PT_NULL")
        _t, _fl, _off, tva, _fsz, tmsz, tal = next(p for p in phdrs if p[0] == 7)
        block = (tmsz + tal - 1) // tal * tal
        syms = _nm_addrs(path, td)
        imm = {s_: ((syms[s_] - tva - block) & 0xFFFFFFFF).to_bytes(4, "little")
               for s_ in ("ly", "gx")}
        want = b"".join(bytes.fromhex(pre) + imm[sym] for pre, sym in _IEF_APX_WANT)
        got = _elf_vread(d, syms["apx"], len(want) + 1)
        if got != want + b"\xc3":
            return Result(name, "FAIL", f"-T apx:\n  got  {got.hex() if got else None}\n"
                          f"  want {want.hex()}c3")
        got = _elf_vread(d, syms["ie_gadd"] + 9, 7)
        if got != bytes.fromhex("4881c0") + imm["gx"]:
            return Result(name, "FAIL", f"-T ie_gadd: {got.hex() if got else None}")
        return Result(name, "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


_MOVRS_ASM = r"""	.text
	.globl	mvr, mvd, mvf
# Executed: both become plain moves when the target is this executable's.
mvr:	movrs	ly@gottpoff(%rip), %rax
	movl	%fs:(%rax), %eax
	ret
mvd:	movrs	dat@GOTPCREL(%rip), %rax
	movq	(%rax), %rax
	ret
# Encoded only (a library's symbol keeps its MOVRS load).
mvf:	movrs	gx@gottpoff(%rip), %r12
	movrs	dat@GOTPCREL(%rip), %r13
	movrs	ext@GOTPCREL(%rip), %rcx
	addq	dat@GOTPCREL(%rip), %r12
	ret
	.data
	.globl	dat
	.type	dat,@object
	.size	dat,8
dat:	.quad	1
	.section .tdata,"awT",@progbits
	.p2align 2
ly:	.long	44
	.globl	gx
	.type	gx,@object
	.size	gx,4
gx:	.long	55
	.section .note.GNU-stack,"",@progbits
"""


def _movrs_relocations_test(args, oracles):
    """MOVRS relocations (`R_X86_64_CODE_5_*`, GNU as / ld 2.47).

    `movrs sym@gottpoff(%rip)` / `movrs sym@GOTPCREL(%rip)` with a REX
    prefix (`REX 0f 38 8b`) carry R_X86_64_CODE_5_GOTTPOFF /
    CODE_5_GOTPCRELX, which lccc-ld refused as unknown relocation types.
    Now: IE -> LE and GOTPCRELX relaxation with GNU ld 2.47's encodings
    (`cs cs REX' mov $imm, %reg` / `cs cs REX lea sym(%rip), %reg`, the
    `0f 38` escape turned into ignored prefixes), the slot otherwise -- a
    shared library's symbol, or any TLS reference in a shared object.
    Exact bytes in PDE / PIE / static executables, runtime values of the
    rewritten forms, the shared-object slots and their dynamic
    relocations.  Also the -T path's GOTPCRELX rewrite, which moved
    ModRM.reg without REX.R (`addq dat@GOTPCREL(%rip), %r12` became
    `add $dat, %rsp`).
    """
    name = "movrs_relocations"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "mv.s"), "w") as f:
            f.write(_MOVRS_ASM)
        with open(os.path.join(td, "m.c"), "w") as f:
            f.write("#include <stdio.h>\nint mvr(void); long mvd(void);\n"
                    "int main(void) { printf(\"%d %ld\\n\", mvr(), mvd()); return 0; }\n")
        with open(os.path.join(td, "ext.c"), "w") as f:
            f.write("int ext = 3;\n")
        r = sh([CC, "-c", "mv.s"], cwd=td)
        if r.returncode != 0:
            err = r.stderr.decode()[:300]
            if "movrs" in err:
                return Result(name, "SKIP", f"assembler without MOVRS: {err}")
            return Result(name, "FAIL", f"assemble: {err}")
        for argv in (["-O1", "-c", "m.c"], ["-O1", "-c", "ext.c"],
                     ["-fPIC", "-shared", "ext.c", "-o", "libext.so"]):
            r = sh([CC] + argv, cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"{argv}: {r.stderr.decode()[:300]}")
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        for mode in (["-no-pie"], ["-pie"], ["-static"]):
            exe = "t" + "".join(mode)
            lib = ["ext.o"] if mode == ["-static"] else ["./libext.so"]
            r = sh([CC, "-B" + shim] + mode + ["m.o", "mv.o"] + lib + ["-o", exe], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"{mode} link: {r.stderr.decode()[:300]}")
            env = dict(os.environ, LD_LIBRARY_PATH=td)
            run = subprocess.run([os.path.join(td, exe)], cwd=td, env=env,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)
            if run.returncode != 0 or run.stdout.decode() != "44 1\n":
                return Result(name, "FAIL", f"{mode}: got {run.stdout.decode()!r} "
                              f"rc {run.returncode}")
            path = os.path.join(td, exe)
            with open(path, "rb") as f:
                d = f.read()
            tls = [p for p in _elf_bytes_phdrs(d) if p[0] == 7]
            _t, _fl, _off, tva, _fsz, tmsz, tal = tls[0]
            block = (tmsz + tal - 1) // tal * tal
            syms = _nm_addrs(path, td)

            def le32(v):
                return (v & 0xFFFFFFFF).to_bytes(4, "little")

            def tp(sym):
                return le32(syms[sym] - tva - block)

            pie = mode == ["-pie"]
            mvf = syms["mvf"]

            def dat_load(pre_imm, pre_lea, at):
                # at: address of the instruction; the field ends 9 bytes in.
                if pie:
                    return bytes.fromhex(pre_lea) + le32(syms["dat"] - (at + 9))
                return bytes.fromhex(pre_imm) + le32(syms["dat"])

            checks = [
                ("mvr", syms["mvr"], bytes.fromhex("2e2e48c7c0") + tp("ly")),
                ("mvd", syms["mvd"], dat_load("2e2e48c7c0", "2e2e488d05", syms["mvd"])),
                ("mvf/gx", mvf, bytes.fromhex("2e2e49c7c4") + tp("gx")),
                ("mvf/dat", mvf + 9, dat_load("2e2e49c7c5", "2e2e4c8d2d", mvf + 9)),
            ]
            if mode == ["-static"]:
                checks.append(("mvf/ext", mvf + 18, bytes.fromhex("2e2e48c7c1") + le32(syms["ext"])))
                checks.append(("mvf/add", mvf + 27, bytes.fromhex("4981c4") + le32(syms["dat"])))
            else:
                # A library's symbol: the MOVRS load through its slot stays.
                checks.append(("mvf/ext", mvf + 18, None))
                if not pie:
                    checks.append(("mvf/add", mvf + 27, bytes.fromhex("4981c4") + le32(syms["dat"])))
            for label, at, want in checks:
                got = _elf_vread(d, at, 9 if want is None else len(want))
                if want is None:
                    if got[:5] != bytes.fromhex("480f388b0d"):
                        return Result(name, "FAIL", f"{mode} {label}: {got.hex()}: a dynamic "
                                      "symbol's MOVRS must keep loading its slot")
                    continue
                if got != want:
                    return Result(name, "FAIL", f"{mode} {label}: {got.hex()} want {want.hex()}")
        # Shared object: every TLS reference keeps its slot (TPOFF64); a
        # preemptible symbol keeps its GOTPCREL slot (GLOB_DAT).
        r = sh([CC, "-B" + shim, "-shared", "mv.o", "-o", "libmv.so"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"-shared link: {r.stderr.decode()[:300]}")
        rel = sh(["readelf", "-rW", "libmv.so"], cwd=td).stdout.decode()
        for want in ("R_X86_64_TPOFF64", "R_X86_64_GLOB_DAT"):
            if want not in rel:
                return Result(name, "FAIL", f"-shared: no {want}:\n{rel[-800:]}")
        with open(os.path.join(td, "libmv.so"), "rb") as f:
            d = f.read()
        syms = _nm_addrs(os.path.join(td, "libmv.so"), td)
        got = _elf_vread(d, syms["mvr"], 5)
        if got != bytes.fromhex("480f388b05"):
            return Result(name, "FAIL", f"-shared mvr rewritten: {got.hex()}")
        # -T: GOTPCRELX rewrites with the prefix handled.
        with open(os.path.join(td, "s.ld"), "w") as f:
            f.write("ENTRY(mvf)\nSECTIONS {\n  . = 0x400000;\n  .text : { *(.text) }\n"
                    "  .data : { *(.data) }\n  . = ALIGN(0x1000);\n  .tdata : { *(.tdata) }\n}\n")
        r = sh([lccc_ld, "-T", "s.ld", "mv.o", "ext.o", "-o", "s.img"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"-T link: {r.stderr.decode()[:300]}")
        with open(os.path.join(td, "s.img"), "rb") as f:
            d = f.read()
        syms = _nm_addrs(os.path.join(td, "s.img"), td)
        dat = (syms["dat"] & 0xFFFFFFFF).to_bytes(4, "little")
        for label, at, want in (("mvf/dat", syms["mvf"] + 9, bytes.fromhex("2e2e49c7c5") + dat),
                                ("mvf/add", syms["mvf"] + 27, bytes.fromhex("4981c4") + dat)):
            got = _elf_vread(d, at, len(want))
            if got != want:
                return Result(name, "FAIL", f"-T {label}: {got.hex() if got else None} "
                              f"want {want.hex()}")
        return Result(name, "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


# Plain R_X86_64_GOTPCREL (assembled with -mrelax-relocations=no): every
# relaxable shape, a library symbol, an absolute symbol and a MOVRS load.
_PLAIN_GOTPCREL_ASM = """\
	.text
	.globl	pgd, pgr, pgc, pgx, pga, pgm
	.type	pgd, @function
pgd:	mov	dat@GOTPCREL(%rip), %rax
	mov	(%rax), %eax
	ret
	.size	pgd, .-pgd
	.type	pgr, @function
pgr:	mov	dat@GOTPCREL(%rip), %r11
	mov	(%r11), %eax
	ret
	.size	pgr, .-pgr
	.type	pgc, @function
pgc:	push	%rbx
	call	*fn@GOTPCREL(%rip)
	pop	%rbx
	jmp	*fn@GOTPCREL(%rip)
	.size	pgc, .-pgc
	.type	pgx, @function
pgx:	mov	ext@GOTPCREL(%rip), %rax
	mov	(%rax), %eax
	ret
	.size	pgx, .-pgx
	.type	pga, @function
pga:	mov	absv@GOTPCREL(%rip), %rax
	ret
	.size	pga, .-pga
	.type	pgm, @function
pgm:	movrs	dat@GOTPCREL(%rip), %eax
	ret
	.size	pgm, .-pgm
	.type	fn, @function
fn:	mov	$5, %eax
	ret
	.size	fn, .-fn
	.globl	absv
	.set	absv, 0x1234
	.data
	.type	dat, @object
dat:	.long	44
	.size	dat, 4
"""

_PLAIN_GOTPCREL_SCRIPT_ASM = """\
	.text
	.globl	_start
_start:	mov	dat@GOTPCREL(%rip), %rax
	call	*fn@GOTPCREL(%rip)
	ret
fn:	ret
	.data
dat:	.long	1
"""


def _plain_gotpcrel_test(args, oracles):
    """Plain R_X86_64_GOTPCREL (old assemblers, `-mrelax-relocations=no`,
    NASM's `wrt ..gotpcrel`) is relaxed where no prefix byte has to change,
    for a recognizable legacy opcode only.

    lccc-ld used to give every such reference a GOT slot in executables and
    shared objects (a load per access for a locally bound symbol); now `mov`
    -> `lea`, `call *` -> `addr32 call`, `jmp *` -> `jmp; nop`, exact bytes
    and runtime values in PDE / PIE / static / shared links.  A library
    symbol, an absolute symbol (whose immediate `mov` would need the REX
    byte edited) and `movrs` (`0f 38 8b`: GNU ld 2.47 turns it into the
    undefined `0f 38 8d`) keep their slot, whose contents are checked.  The
    -T path decided these as GOTPCRELX, corrupting `movrs` the same way: it
    is refused now (a -T link has no GOT).
    """
    name = "plain_gotpcrel_relaxation"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        for fname, text in (("pg.s", _PLAIN_GOTPCREL_ASM),
                            ("sc.s", _PLAIN_GOTPCREL_SCRIPT_ASM),
                            ("m.c", "#include <stdio.h>\n"
                                    "int pgd(void), pgr(void), pgc(void), pgx(void);\n"
                                    "long pga(void);\nint main(void) {\n"
                                    "  printf(\"%d %d %d %d %#lx\\n\", pgd(), pgr(), pgc(),"
                                    " pgx(), pga());\n  return 0;\n}\n"),
                            ("ext.c", "int ext = 3;\n")):
            with open(os.path.join(td, fname), "w") as f:
                f.write(text)
        r = sh([CC, "-Wa,-mrelax-relocations=no", "-c", "pg.s"], cwd=td)
        if r.returncode != 0:
            err = r.stderr.decode()[:300]
            if "movrs" in err:
                return Result(name, "SKIP", f"assembler without MOVRS: {err}")
            return Result(name, "FAIL", f"assemble: {err}")
        rel = sh(["readelf", "-rW", "pg.o"], cwd=td).stdout.decode()
        if rel.count("R_X86_64_GOTPCREL ") != 7:
            return Result(name, "FAIL", f"assembler did not emit plain GOTPCREL:\n{rel[-600:]}")
        for argv in (["-Wa,-mrelax-relocations=no", "-c", "sc.s"], ["-O1", "-c", "m.c"],
                     ["-O1", "-c", "ext.c"], ["-fPIC", "-shared", "ext.c", "-o", "libext.so"]):
            r = sh([CC] + argv, cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"{argv}: {r.stderr.decode()[:300]}")
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        env = dict(os.environ, LD_LIBRARY_PATH=td)

        def le32(v):
            return (v & 0xFFFFFFFF).to_bytes(4, "little")

        for mode in ("-no-pie", "-pie", "-static", "-shared"):
            if mode == "-shared":
                r = sh([CC, "-B" + shim, "-shared", "pg.o", "./libext.so", "-o", "libpg.so"],
                       cwd=td)
                img = "libpg.so"
                if r.returncode == 0:
                    r = sh([CC, "-B" + shim, "m.o", "./libpg.so", "-o", "t-shared"], cwd=td)
            else:
                lib = ["ext.o"] if mode == "-static" else ["./libext.so"]
                img = "t" + mode
                r = sh([CC, "-B" + shim, mode, "m.o", "pg.o"] + lib + ["-o", img], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"{mode} link: {r.stderr.decode()[:300]}")
            run = subprocess.run([os.path.join(td, "t" + mode)], cwd=td, env=env,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)
            if run.returncode != 0 or run.stdout.decode() != "44 44 5 3 0x1234\n":
                return Result(name, "FAIL", f"{mode}: got {run.stdout.decode()!r} "
                              f"rc {run.returncode} {run.stderr.decode()[:200]!r}")
            path = os.path.join(td, img)
            with open(path, "rb") as f:
                d = f.read()
            syms = _nm_addrs(path, td)
            dat, fn = syms["dat"], syms["fn"]
            pgc = syms["pgc"]
            want = [
                ("pgd", syms["pgd"], bytes.fromhex("488d05") + le32(dat - (syms["pgd"] + 7))),
                ("pgr", syms["pgr"], bytes.fromhex("4c8d1d") + le32(dat - (syms["pgr"] + 7))),
                ("pgc/call", pgc + 1, bytes.fromhex("67e8") + le32(fn - (pgc + 7))),
                ("pgc/jmp", pgc + 8, bytes.fromhex("e9") + le32(fn - (pgc + 13)) + b"\x90"),
            ]
            if mode == "-static":
                want.append(("pgx", syms["pgx"],
                             bytes.fromhex("488d05") + le32(syms["ext"] - (syms["pgx"] + 7))))
            dynrel = sh(["readelf", "-rW", img], cwd=td).stdout.decode()

            def slot_of(label, at, head, fill):
                # The load must stay, through a slot that yields `fill`.
                got = _elf_vread(d, at, len(head) + 4)
                if got is None or got[:len(head)] != head:
                    return f"{label}: {got.hex() if got else None}, want the {head.hex()} load kept"
                slot = at + len(head) + 4 + int.from_bytes(got[len(head):], "little", signed=True)
                rows = [ln.split() for ln in dynrel.splitlines()
                        if ln.split() and ln.split()[0].lstrip("0") == f"{slot:x}"]
                if fill is None:
                    return None if rows else f"{label}: slot {slot:#x} has no dynamic relocation"
                if mode in ("-pie", "-shared") and fill != 0x1234:
                    ok = any(rw[2] == "R_X86_64_RELATIVE" and int(rw[3], 16) == fill
                             for rw in rows)
                    return None if ok else f"{label}: slot {slot:#x} rows {rows}, want RELATIVE"
                val = _elf_vread(d, slot, 8)
                if rows or val is None or int.from_bytes(val, "little") != fill:
                    return f"{label}: slot {slot:#x} = {val.hex() if val else None} rows {rows}"
                return None

            problems = [f"{label}: {(_elf_vread(d, at, len(w)) or b'').hex()} want {w.hex()}"
                        for label, at, w in want if _elf_vread(d, at, len(w)) != w]
            if mode != "-static":
                problems.append(slot_of("pgx", syms["pgx"], bytes.fromhex("488b05"), None))
            problems.append(slot_of("pga", syms["pga"], bytes.fromhex("488b05"),
                                    None if mode == "-shared" else 0x1234))
            problems.append(slot_of("pgm", syms["pgm"], bytes.fromhex("0f388b05"), dat))
            problems = [p for p in problems if p]
            if problems:
                return Result(name, "FAIL", f"{mode}: " + "; ".join(problems)[:600])
        # -T: no GOT, so the relaxable forms are rewritten and MOVRS refused.
        with open(os.path.join(td, "s.ld"), "w") as f:
            f.write("SECTIONS {\n  . = 0x400000;\n  .text : { *(.text) }\n"
                    "  .data : { *(.data) }\n}\n")
        r = sh([lccc_ld, "-T", "s.ld", "sc.o", "-o", "s.img"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"-T link: {r.stderr.decode()[:300]}")
        with open(os.path.join(td, "s.img"), "rb") as f:
            d = f.read()
        syms = _nm_addrs(os.path.join(td, "s.img"), td)
        st = syms["_start"]
        want = (bytes.fromhex("488d05") + le32(syms["dat"] - (st + 7))
                + bytes.fromhex("67e8") + le32(syms["fn"] - (st + 13)))
        got = _elf_vread(d, st, len(want))
        if got != want:
            return Result(name, "FAIL", f"-T: {got.hex() if got else None} want {want.hex()}")
        r = sh([lccc_ld, "-T", "s.ld", "pg.o", "ext.o", "-o", "m.img"], cwd=td)
        err = r.stderr.decode()
        field = _nm_addrs(os.path.join(td, "pg.o"), td)["pgm"] + 4
        refusal = (f"R_X86_64_GOTPCREL against 'dat' in pg.o at offset {field:#x} uses an "
                   "instruction form this linker cannot relax")
        if r.returncode == 0 or refusal not in err:
            with open(os.path.join(td, "m.img"), "rb") as f:
                d = f.read()
            pgm = _nm_addrs(os.path.join(td, "m.img"), td).get("pgm", 0)
            got = _elf_vread(d, pgm, 4)
            return Result(name, "FAIL", f"-T movrs: rc={r.returncode} bytes "
                          f"{got.hex() if got else None} {err[:300]!r}; want the refusal")
        return Result(name, "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


_TEXTREL_RO_ASM = """\
    .section .rodata,"a"
    .p2align 3
    .globl tbl
    .type tbl,@object
    .size tbl,8
tbl: .quad val
    .data
val: .long 5
    .section .note.GNU-stack,"",@progbits
"""

_TEXTREL_TEXT_ASM = """\
    .text
    .globl getval
    .type getval,@function
getval:
    movabs $val2, %rax
    movl (%rax), %eax
    ret
    .size getval, .-getval
    .data
val2: .long 7
    .section .note.GNU-stack,"",@progbits
"""


def _elf64_dynamic(d):
    """(tag, value) pairs of an ELF64 image's PT_DYNAMIC, up to DT_NULL."""
    out = []
    for typ, _fl, off, _va, fsz, _msz, _al in _elf_bytes_phdrs(d) or []:
        if typ == 2:  # PT_DYNAMIC
            for p in range(off, off + fsz, 16):
                tag, val = struct.unpack_from("<qQ", d, p)
                if tag == 0:
                    break
                out.append((tag, val))
    return out


def _text_relocations_test(args, oracles):
    """Dynamic relocations into read-only storage (text relocations).

    A PIE whose `.rodata` holds an absolute pointer (`.quad sym` from
    hand-written assembly or non-PIC objects) used to keep that section in
    the read-only segment with no DT_TEXTREL: ld.so faulted writing the
    RELATIVE before `main`.  lccc-ld now places such a section at the head
    of PT_GNU_RELRO (as its shared-object emitter already did): written
    while still writable, then protected -- no text relocation, so `-z
    text` links it.  Code cannot move (`movabs $sym` in `.text`, in a PIE or
    a shared object): that is a real text relocation, which used to be
    written into a read-only mapping just the same, and now follows GNU
    ld's policy on both linkers -- the default warns about the first site
    and that DT_TEXTREL is created and emits DT_TEXTREL + DF_TEXTREL,
    `-z text` refuses the link with GNU's message, `-z notext` is silent.
    """
    name = "text_relocations"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        for fn, body in (("ro.s", _TEXTREL_RO_ASM), ("txt.s", _TEXTREL_TEXT_ASM),
                         ("m_ro.c", "#include <stdio.h>\nextern int *const tbl;\n"
                                    "int main(void){ printf(\"%d\\n\", *tbl); return 0; }\n"),
                         ("m_all.c", "#include <stdio.h>\nextern int *const tbl;\n"
                                     "int getval(void);\nint main(void){ printf(\"%d %d\\n\","
                                     " *tbl, getval()); return 0; }\n"),
                         ("s.c", "int getval(void);\nint call(void){ return getval(); }\n")):
            with open(os.path.join(td, fn), "w") as f:
                f.write(body)
        r = sh([CC, "-c", "ro.s", "txt.s"], cwd=td)
        r2 = sh([CC, "-O1", "-fPIE", "-c", "m_ro.c", "m_all.c"], cwd=td)
        r3 = sh([CC, "-O1", "-fPIC", "-c", "s.c"], cwd=td)
        for rr in (r, r2, r3):
            if rr.returncode != 0:
                return Result(name, "FAIL", f"compile: {rr.stderr.decode()[:300]}")
        lccc_ld = os.path.join(os.path.dirname(os.path.abspath(args.lccc)), "lccc-ld")
        shim = _shim_for(td, lccc_ld)
        linkers = (("lccc", ["-B" + shim]), ("gnu", []))

        def link(tag, argv, out):
            bflag = dict(linkers)[tag]
            return sh([CC] + bflag + argv + ["-o", out], cwd=td)

        def image(out):
            with open(os.path.join(td, out), "rb") as f:
                return f.read()

        def textrel_tags(d):
            dyn = _elf64_dynamic(d)
            has_tag = any(t == 22 for t, _ in dyn)  # DT_TEXTREL
            flags = next((v for t, v in dyn if t == 30), 0)  # DT_FLAGS
            return has_tag, bool(flags & 4)  # DF_TEXTREL

        def run(exe, want, libdir=None):
            env = dict(os.environ)
            if libdir:
                env["LD_LIBRARY_PATH"] = libdir
            rr = subprocess.run([os.path.join(td, exe)], cwd=td, env=env,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)
            if rr.returncode != 0 or rr.stdout.decode() != want:
                return f"{exe}: rc {rr.returncode} out {rr.stdout.decode()!r} want {want!r}"
            return None

        # 1. PIE, read-only data only: relro-moved, no text relocation.
        r = link("lccc", ["-pie", "m_ro.o", "ro.o", "-Wl,-z,text"], "ro.pie")
        if r.returncode != 0 or r.stderr:
            return Result(name, "FAIL", f"ro PIE -z text: rc {r.returncode} "
                          f"stderr {r.stderr.decode()[:300]!r}")
        d = image("ro.pie")
        if textrel_tags(d) != (False, False):
            return Result(name, "FAIL", "ro PIE: DT_TEXTREL/DF_TEXTREL without code relocations")
        tbl = _nm_addrs(os.path.join(td, "ro.pie"), td)["tbl"]
        relro = [p for p in _elf_bytes_phdrs(d) if p[0] == 0x6474E552]
        if len(relro) != 1 or not relro[0][3] <= tbl < relro[0][3] + relro[0][5]:
            return Result(name, "FAIL", f"ro PIE: tbl 0x{tbl:x} not inside PT_GNU_RELRO {relro}")
        err = run("ro.pie", "5\n")
        if err:
            return Result(name, "FAIL", err)
        # 2. Code relocations in a PIE and in a shared object: GNU's policy on
        #    both linkers.
        variants = (("PIE", ["-pie", "m_all.o", "ro.o", "txt.o"], "all.pie"),
                    ("shared object", ["-shared", "txt.o", "s.o"], "libtr.so"))
        for kind, argv, out in variants:
            for tag, _b in linkers:
                label = f"{kind}/{tag}"
                r = link(tag, argv, out + "." + tag)
                msg = r.stderr.decode(errors="replace")
                if r.returncode != 0:
                    return Result(name, "FAIL", f"{label}: rc {r.returncode}: {msg[:300]}")
                site = ("txt.o: warning: relocation in read-only section `.text'" if tag == "lccc"
                        else "warning: relocation in read-only section")
                if f"creating DT_TEXTREL in a {kind}" not in msg or site not in msg:
                    return Result(name, "FAIL", f"{label}: default link must warn like GNU ld: "
                                  f"{msg[:300]!r}")
                if textrel_tags(image(out + "." + tag)) != (True, True):
                    return Result(name, "FAIL", f"{label}: DT_TEXTREL + DF_TEXTREL missing")
                r = link(tag, argv + ["-Wl,-z,text"], out + ".ztext")
                if r.returncode == 0 or b"read-only segment has dynamic relocations" not in r.stderr:
                    return Result(name, "FAIL", f"{label} -z text: rc {r.returncode} "
                                  f"{r.stderr[:300]!r}")
                r = link(tag, argv + ["-Wl,-z,notext"], out + ".notext")
                if r.returncode != 0 or b"TEXTREL" in r.stderr or b"read-only" in r.stderr:
                    return Result(name, "FAIL", f"{label} -z notext: rc {r.returncode} "
                                  f"{r.stderr[:300]!r}")
                if textrel_tags(image(out + ".notext")) != (True, True):
                    return Result(name, "FAIL", f"{label} -z notext: tags missing")
        err = run("all.pie.lccc", "5 7\n")
        if err:
            return Result(name, "FAIL", err)
        # The library, used by an executable (and the read-only data one:
        # relro-moved, so `-z text` accepts it).
        os.replace(os.path.join(td, "libtr.so.lccc"), os.path.join(td, "libtr.so"))
        r = link("lccc", ["-shared", "ro.o", "-Wl,-z,text"], "libro.so")
        if r.returncode != 0 or textrel_tags(image("libro.so")) != (False, False):
            return Result(name, "FAIL", f"ro .so -z text: rc {r.returncode} {r.stderr[:300]!r}")
        r = link("lccc", ["-pie", "m_all.o", "./libtr.so", "./libro.so"], "use.pie")
        if r.returncode != 0:
            return Result(name, "FAIL", f"link against the libraries: {r.stderr[:300]!r}")
        err = run("use.pie", "5 7\n", libdir=td)
        if err:
            return Result(name, "FAIL", err)
        return Result(name, "PASS")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


def _build_id_note_test(args, oracles):
    """--build-id must emit a content-derived .note.gnu.build-id.

    Regression for a defect the differential oracle found: Debian's gcc passes
    --build-id on *every* link, lccc-ld accepted the flag, and the output had no
    note at all -- so no binary could be matched to its debuginfo.  Also checks
    the digest is reproducible, content-sensitive, and that --build-id=none
    suppresses it.
    """
    name = "build_id_note"
    lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    if not os.path.exists(lccc_ld):
        return Result(name, "SKIP", "no lccc-ld")
    with tempfile.TemporaryDirectory() as td:
        with open(os.path.join(td, "a.c"), "w") as f:
            f.write('#include <stdio.h>\nint main(void){ printf("ok\\n"); return 0; }\n')
        with open(os.path.join(td, "b.c"), "w") as f:
            f.write('#include <stdio.h>\nint main(void){ printf("other\\n"); return 0; }\n')
        r = sh([CC, "-c", "-O1", "a.c"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", "fixture compile failed")
        sh([CC, "-c", "-O1", "b.c"], cwd=td)
        shim = _shim_for(td, lccc_ld)

        def digest(path):
            if not os.path.exists(path):
                return None
            # -SW, not -S: plain -S wraps each section header over two lines,
            # which splits long section names.
            o = sh(["readelf", "-SW", path]).stdout.decode(errors="replace")
            if ".note.gnu.build-id" not in o:
                return None
            n = sh(["readelf", "-n", path]).stdout.decode(errors="replace")
            for ln in n.splitlines():
                ln = ln.strip()
                if ln.startswith("Build ID:"):
                    return ln.split(":", 1)[1].strip()
            return None

        def link(obj, out, extra):
            return sh([CC, "-B" + shim, obj, "-o", out] + extra, cwd=td)

        out = os.path.join(td, "a.out")
        r = link("a.o", out, ["-Wl,--build-id=sha1"])
        if r.returncode != 0:
            return Result(name, "FAIL",
                          "link failed: " + r.stderr.decode(errors="replace")[-300:])
        got = digest(out)
        if got is None:
            return Result(name, "FAIL", "no .note.gnu.build-id section")
        if set(got) <= {"0"}:
            return Result(name, "FAIL", "degenerate digest %r" % got)

        out2 = os.path.join(td, "a2.out")
        link("a.o", out2, ["-Wl,--build-id=sha1"])
        if digest(out2) != got:
            return Result(name, "FAIL",
                          "not reproducible: %s vs %s" % (got, digest(out2)))

        out3 = os.path.join(td, "b.out")
        link("b.o", out3, ["-Wl,--build-id=sha1"])
        if digest(out3) == got:
            return Result(name, "FAIL", "digest does not depend on the inputs")

        out4 = os.path.join(td, "c.out")
        link("a.o", out4, ["-Wl,--build-id=none"])
        if digest(out4) is not None:
            return Result(name, "FAIL", "--build-id=none still emitted a note")

        code, txt = run_bin(out, [], td)
        if code != 0 or txt != "ok\n":
            return Result(name, "FAIL", "binary broken: %s" % ((code, txt),))
        return Result(name, "PASS", "digest %s" % got)


_PT_LOAD = 1
_PT_NOTE = 4
_PT_GNU_RELRO = 0x6474E552
_PT_GNU_PROPERTY = 0x6474E553
_PF_W = 2  # ELF p_flags: X=1, W=2, R=4
_SHT_NOTE = 7


def _elf_bytes_phdrs(d):
    """Parse all program headers from ELF bytes (class-agnostic)."""
    if d[:4] != b"\x7fELF" or d[4] != 2 or d[5] != 1:
        return None  # ELF64 LSB only (all fixtures here are)
    e_phoff, = struct.unpack_from("<Q", d, 0x20)
    e_phentsize, e_phnum = struct.unpack_from("<HH", d, 0x36)
    out = []
    for i in range(e_phnum):
        p = e_phoff + i * e_phentsize
        typ, flags = struct.unpack_from("<II", d, p)
        off, va, pa, fsz, msz, al = struct.unpack_from("<QQQQQQ", d, p + 8)
        out.append((typ, flags, off, va, fsz, msz, al))
    return out


def _elf_bytes_shdrs(d):
    """Parse all section headers; returns list of (name, type, flags, addr, off, size, align)."""
    e_shoff, = struct.unpack_from("<Q", d, 0x28)
    e_shentsize, e_shnum, e_shstrndx = struct.unpack_from("<HHH", d, 0x3A)
    hdrs = []
    for k in range(e_shnum):
        b = e_shoff + k * e_shentsize
        n, t, fl, a, off, sz, lk, inf, al, es = struct.unpack_from("<IIQQQQIIQQ", d, b)
        hdrs.append([n, t, fl, a, off, sz, lk, inf, al])
    if e_shstrndx < e_shnum:
        str_off = hdrs[e_shstrndx][4]
        for h in hdrs:
            end = d.index(b"\0", str_off + h[0])
            h[0] = d[str_off + h[0]:end].decode("ascii", errors="replace")
    return hdrs


def _relro_nobits_pad_test(args, oracles):
    """RELRO page pad must cost address space, not file space (two RW LOADs).

    Regression for a measured 1.8 KiB/binary defect: the RELRO boundary used
    to advance the file offset by up to one page of zeros so the virtual
    address reached a page boundary.  lld/mold instead give the RELRO window
    its own PT_LOAD whose memsz covers a NOBITS pad, then continue the
    writable tail at the SAME dense file offset on a fresh page.  After the
    fix lccc's hello PIE is 6.2 KiB (lld 6.0, mold 8.4, bfd 15.8).

    Asserts the full structural invariant, via a direct ELF parse (readelf
    text layout is not a stable API):
      * e_phnum matches the actual header table (no latent PT_NULL padding),
      * GNU_RELRO: memsz >= filesz, memsz - filesz < one page, and
        vaddr+memsz is page-aligned,
      * two writable PT_LOADs: LOAD_W1 covers [relro.start, relro.fsz),
        LOAD_W2 starts at the same dense file offset, and both obey the
        gABI congruence rule off % page == vaddr % page,
      * LOAD_W2's page does not intersect the RELRO mprotect range,
      * the binary still runs.
    """
    name = "relro_nobits_pad"
    lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    if not os.path.exists(lccc_ld):
        return Result(name, "SKIP", "no lccc-ld")
    page = 0x1000
    with tempfile.TemporaryDirectory() as td:
        # Writable .data AND .bss force post-RELRO content in every binding mode.
        with open(os.path.join(td, "a.c"), "w") as f:
            f.write('#include <stdio.h>\n'
                    'int writable_global = 7; char bss_buf[64];\n'
                    'int main(void){ bss_buf[0]=1; '
                    'printf("%d\\n", writable_global + bss_buf[0]); return 0; }\n')
        r = sh([CC, "-c", "-O1", "a.c"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", "fixture compile failed")
        shim = _shim_for(td, lccc_ld)
        out = os.path.join(td, "a.out")
        r = sh([CC, "-B" + shim, "a.o", "-o", out, "-Wl,--build-id=sha1"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL",
                          "link failed: " + r.stderr.decode(errors="replace")[-300:])
        d = open(out, "rb").read()
        phdrs = _elf_bytes_phdrs(d)
        if phdrs is None:
            return Result(name, "FAIL", "not an ELF64 LSB file")
        relro = [p for p in phdrs if p[0] == _PT_GNU_RELRO]
        if len(relro) != 1:
            return Result(name, "FAIL", "expected 1 GNU_RELRO, got %d" % len(relro))
        _, _, r_off, r_va, r_fsz, r_msz, _ = relro[0]
        if not (0 <= r_msz - r_fsz < page):
            return Result(name, "FAIL",
                          "relro memsz-filesz %#x not a sub-page NOBITS pad"
                          % (r_msz - r_fsz))
        if (r_va + r_msz) % page != 0:
            return Result(name, "FAIL", "relro end %#x not page-aligned"
                          % (r_va + r_msz))
        wloads = [p for p in phdrs if p[0] == _PT_LOAD and (p[1] & _PF_W)]
        if len(wloads) < 2:
            return Result(name, "FAIL",
                          "expected split RW loads, got %d writable LOAD(s)"
                          % len(wloads))
        wloads.sort(key=lambda p: p[2])
        w1, w2 = wloads[0], wloads[1]
        if w1[2] != r_off or w1[3] != r_va or w1[4] != r_fsz or w1[5] != r_msz:
            return Result(name, "FAIL", "load W1 %r does not mirror GNU_RELRO %r"
                          % (w1, relro[0]))
        if w2[2] != w1[2] + w1[4]:
            return Result(name, "FAIL",
                          "writable tail not file-dense: w2.off=%#x want %#x"
                          % (w2[2], w1[2] + w1[4]))
        for p in phdrs:
            if p[0] != _PT_LOAD:
                continue
            if p[6] and p[6] > 1 and (p[2] % p[6]) != (p[3] % p[6]):
                return Result(name, "FAIL",
                              "gABI congruence violated: off=%#x va=%#x align=%#x"
                              % (p[2], p[3], p[6]))
        # The mprotect range must not cover the writable tail's own mapping.
        relro_end_page = (r_va + r_msz) // page
        w2_first_page = w2[3] // page
        if w2_first_page < relro_end_page:
            return Result(name, "FAIL",
                          "writable tail starts inside relro mprotect range")
        code, txt = run_bin(out, [], td)
        if code != 0 or txt != "8\n":
            return Result(name, "FAIL", "binary broken: %s" % ((code, txt),))
        return Result(name, "PASS",
                      "relro fsz=%#x msz=%#x w2 off=%#x va=%#x"
                      % (r_fsz, r_msz, w2[2], w2[3]))


def _note_run_merge_test(args, oracles):
    """Contiguous allocated note sections share ONE PT_NOTE (lld-style).

    A PT_NOTE phdr per note section wastes a 56-byte header per extra note.
    Validates: the number of PT_NOTE phdrs equals the number of maximal
    file-contiguous runs of allocated SHT_NOTE sections (comparing against
    an independently computed expectation, not a hardcoded 1); every note
    section is covered by exactly one PT_NOTE; GNU_PROPERTY aliases
    .note.gnu.property exactly; build-id stays readable through the merge.
    """
    name = "note_run_merge"
    lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    if not os.path.exists(lccc_ld):
        return Result(name, "SKIP", "no lccc-ld")
    with tempfile.TemporaryDirectory() as td:
        with open(os.path.join(td, "a.c"), "w") as f:
            f.write('#include <stdio.h>\nint main(void){ printf("ok\\n"); return 0; }\n')
        r = sh([CC, "-c", "-O1", "a.c"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", "fixture compile failed")
        shim = _shim_for(td, lccc_ld)
        out = os.path.join(td, "a.out")
        r = sh([CC, "-B" + shim, "a.o", "-o", out, "-Wl,--build-id=sha1"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL",
                          "link failed: " + r.stderr.decode(errors="replace")[-300:])
        d = open(out, "rb").read()
        phdrs, shdrs = _elf_bytes_phdrs(d), _elf_bytes_shdrs(d)
        if phdrs is None or shdrs is None:
            return Result(name, "FAIL", "not an ELF64 LSB file")
        SHF_ALLOC = 2
        notes = sorted(
            ([h[3], h[4], h[5], h[6]] for h in shdrs
             if h[1] == _SHT_NOTE and (h[2] & SHF_ALLOC) and h[5] > 0),
            key=lambda h: h[1])
        if len(notes) < 2:
            return Result(name, "SKIP", "fixture has <2 note sections")
        # Independent expectation: maximal runs allowing <= 8-byte alignment
        # gaps (section alignment inside a span is legal for one PT_NOTE).
        runs = 1
        prev_end = notes[0][1] + notes[0][2]
        for _, off, sz, _al in notes[1:]:
            if off - prev_end > 8:
                runs += 1
            prev_end = max(prev_end, off + sz)
        ptn = [p for p in phdrs if p[0] == _PT_NOTE]
        if len(ptn) != runs:
            return Result(name, "FAIL",
                          "%d PT_NOTE phdrs for %d contiguous run(s) of %d notes"
                          % (len(ptn), runs, len(notes)))
        covered = []
        for _, _, off, _, fsz, msz, _ in ptn:
            if fsz != msz:
                return Result(name, "FAIL", "PT_NOTE fsz %#x != msz %#x"
                              % (fsz, msz))
            covered.append((off, off + fsz))
        covered.sort()
        for _, off, sz, _ in notes:
            if not any(lo <= off and off + sz <= hi for lo, hi in covered):
                return Result(name, "FAIL", "note section at %#x not covered"
                              % off)
        props = [p for p in phdrs if p[0] == _PT_GNU_PROPERTY]
        prop_secs = [h for h in shdrs if h[0] == ".note.gnu.property"]
        if prop_secs:
            want = prop_secs[0]
            if len(props) != 1 or props[0][2] != want[4] or props[0][4] != want[5]:
                return Result(name, "FAIL", "GNU_PROPERTY does not alias "
                              ".note.gnu.property exactly: %r vs %r"
                              % (props, want))
        n = sh(["readelf", "-n", out]).stdout.decode(errors="replace")
        if "Build ID:" not in n:
            return Result(name, "FAIL", "merged notes broke build-id parsing")
        code, txt = run_bin(out, [], td)
        if code != 0 or txt != "ok\n":
            return Result(name, "FAIL", "binary broken: %s" % ((code, txt),))
        return Result(name, "PASS", "%d notes in %d PT_NOTE run(s)"
                      % (len(notes), runs))


def _gnu_hash_sizing_test(args, oracles):
    """differential .gnu.hash: oracle-measured sizing + glibc-walk correctness.

    Locks in three measured facts (see linker_common/hash.rs GnuHashParams):
      * bloom words = next_pow2(ceil(n/4)), a power of two (glibc derives
        the word mask as bloom_size-1; the old single-word version saturated
        beyond ~30 exports: measured FPR ~1.0 at 20k symbols),
      * bloom_shift = log2(bloom_size * class_bits),
      * nbuckets = max(1, n/4) (lld parity).

    Then proves the table is *usable*, not just plausible: an independent
    re-implementation of glibc's dl-lookup.c two-bit bloom walk resolves
    every exported symbol through the emitted table, and a consumer
    executable resolves one through the real ld.so at runtime.
    """
    name = "gnu_hash_sizing"
    lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    if not os.path.exists(lccc_ld):
        return Result(name, "SKIP", "no lccc-ld")
    N = 600
    with tempfile.TemporaryDirectory() as td:
        src = os.path.join(td, "big.c")
        with open(src, "w") as f:
            for k in range(N):
                f.write("int h%d(void){return %d;}\n" % (k, k))
        r = sh([CC, "-c", "-O1", "-fPIC", src, "-o", os.path.join(td, "big.o")])
        if r.returncode != 0:
            return Result(name, "SKIP", "fixture compile failed")
        so = os.path.join(td, "big.so")
        r = sh([lccc_ld, "-shared", "-o", so, os.path.join(td, "big.o")])
        if r.returncode != 0:
            return Result(name, "FAIL", "lccc-ld -shared failed: "
                          + r.stderr.decode(errors="replace")[-200:])
        d = open(so, "rb").read()
        shdrs = _elf_bytes_shdrs(d)
        gh = None
        dynsym = dynstr = None
        sec_data = {}
        for h in shdrs:
            o, s = h[4], h[5]
            if h[0] == ".gnu.hash":
                gh = struct.unpack_from("<IIII", d, o)
                sec_data["gh"] = d[o:o + s]
            elif h[0] == ".dynsym":
                sec_data["dynsym"] = d[o:o + s]
            elif h[0] == ".dynstr":
                sec_data["dynstr"] = d[o:o + s]
        if gh is None or "dynsym" not in sec_data:
            return Result(name, "FAIL", "no .gnu.hash / .dynsym in output")
        nbuckets, symoffset, bloom_size, bloom_shift = gh
        # Ground truth: measured lld sizing for n=600 is (150, 256, shift
        # lg2(256*64)=14). Assert the rule, not the echo of our own value.
        exp_bloom = max(1, 1 << ((N + 3) // 4 - 1).bit_length())
        if (bloom_size & (bloom_size - 1)) != 0 or bloom_size != exp_bloom:
            return Result(name, "FAIL", "bloom_size=%d, want power of 2 = %d"
                          % (bloom_size, exp_bloom))
        if bloom_shift != (bloom_size * 64).bit_length() - 1:
            return Result(name, "FAIL", "bloom_shift=%d, want %d"
                          % (bloom_shift, (bloom_size * 64).bit_length() - 1))
        if nbuckets != max(1, N // 4):
            return Result(name, "FAIL", "nbuckets=%d, want %d"
                          % (nbuckets, N // 4))
        # Independent glibc-walk resolution of EVERY exported symbol.
        def gnu_hash(nm):
            h = 5381
            for c in nm.encode():
                h = (h * 33 + c) & 0xFFFFFFFF
            return h
        ghb, syms, strs = sec_data["gh"], sec_data["dynsym"], sec_data["dynstr"]
        bloom_off = 16
        buckets_off = bloom_off + bloom_size * 8
        chains_off = buckets_off + nbuckets * 4
        nsym = len(syms) // 24
        def lookup(want):
            h = gnu_hash(want)
            w_idx = (h // 64) & (bloom_size - 1)
            w = struct.unpack_from("<Q", ghb, bloom_off + w_idx * 8)[0]
            if not (w & (1 << (h % 64)) and
                    w & (1 << ((h >> bloom_shift) % 64))):
                return None
            idx = struct.unpack_from("<I", ghb, buckets_off
                                     + (h % nbuckets) * 4)[0]
            while idx >= symoffset:
                no = struct.unpack_from("<I", syms, idx * 24)[0]
                nm = strs[no:strs.index(b"\0", no)].decode()
                chain = struct.unpack_from("<I", ghb,
                                           chains_off + (idx - symoffset) * 4)[0]
                if nm == want:
                    return idx
                if chain & 1:
                    return None
                idx += 1
            return None
        bad = ["h%d" % k for k in range(N) if lookup("h%d" % k) is None]
        if bad:
            return Result(name, "FAIL",
                          "%d symbols unresolvable through .gnu.hash, e.g. %s"
                          % (len(bad), bad[:3]))
        # Absent symbols must mostly be rejected by the two-bit bloom probe
        # (pure filter check; a chain walk can never "find" an absent name).
        def bloom_pass(want):
            h = gnu_hash(want)
            w = struct.unpack_from(
                "<Q", ghb, bloom_off + ((h // 64) & (bloom_size - 1)) * 8)[0]
            return bool(w & (1 << (h % 64)) and
                        w & (1 << ((h >> bloom_shift) % 64)))
        misses = sum(1 for k in range(N // 2) if bloom_pass("absent_%d" % k))
        if misses > N // 20:
            return Result(name, "FAIL",
                          "bloom passed %d/%d absent symbols (FPR too high)"
                          % (misses, N // 2))
        # End-to-end: resolve one through the real dynamic loader.
        with open(os.path.join(td, "use.c"), "w") as f:
            f.write('extern int h%d(void);\n'
                    '#include <stdio.h>\n'
                    'int main(void){printf("%%d\\n", h%d());return 0;}\n'
                    % (N - 1, N - 1))
        if sh([CC, "-c", "-O1", os.path.join(td, "use.c"),
               "-o", os.path.join(td, "use.o")]).returncode != 0:
            return Result(name, "SKIP", "consumer compile failed")
        exe = os.path.join(td, "use")
        shim = _shim_for(td, lccc_ld)
        r = sh([CC, "-B" + shim, os.path.join(td, "use.o"), so, "-o", exe])
        if r.returncode != 0:
            return Result(name, "FAIL", "consumer link failed: "
                          + r.stderr.decode(errors="replace")[-200:])
        code, txt = run_bin(exe, [], td, env={"LD_LIBRARY_PATH": td})
        if code != 0 or txt != "%d\n" % (N - 1):
            return Result(name, "FAIL", "runtime lookup broken: %s"
                          % ((code, txt),))
        return Result(name, "PASS", "n=%d buckets=%d bloom=%d shift=%d"
                      % (N, nbuckets, bloom_size, bloom_shift))


def _crossarch_gnu_hash_relro_test(args, oracles):
    """cross-arch .gnu.hash sizing + RELRO page-boundary invariants.

    The .gnu.hash sizing port and the RELRO page-rightsizing were ported to
    every backend; this test compiles a 48-export shared object with each
    arch driver's *own* linker (i686=aarch32-class ELF32, arm=AArch64 ELF64
    @64K pages, riscv=RISC-V ELF64) and verifies, without section headers
    (the i686 emitter writes none), purely from phdrs+DT entries:

      * `.gnu.hash`: pow2 bloom words = next_pow2(ceil(n/4)), shift =
        log2(words*class_bits), buckets = n/4 — the measured lld rule, at
        the arch's class width; all 48 exports resolvable through an
        independent glibc-walk; absent-name FPR bounded.
      * RELRO (arm/riscv; i686 has no RELRO by design): PT_GNU_RELRO spans
        [page, next page) — start page-aligned, memsz reaching the page
        edge, so glibc's round-down mprotect actually covers the region
        (pre-fix riscv exes shipped sub-page memsz = zero protection, and
        aarch64 had no PT_GNU_RELRO at all).
    """
    name = "crossarch_gnu_hash_relro"
    bindir = os.path.dirname(args.lccc)
    drivers = [
        ("lccc-i686", 32, 0x1000, False),
        ("lccc-arm", 64, 0x10000, True),
        ("lccc-riscv", 64, 0x1000, True),
    ]
    seen = 0
    m32_why = ""
    with tempfile.TemporaryDirectory() as td:
        N = 48
        src = os.path.join(td, "f.c")
        with open(src, "w") as f:
            for k in range(N):
                f.write("int v%d(void){return %d;}\n" % (k, k * 3))
        for drv, bits, page, want_relro in drivers:
            path = os.path.join(bindir, drv)
            if not os.path.exists(path):
                continue
            if bits == 32:
                # Same rule as the C++ probe, for the same reason: ask the
                # REFERENCE toolchain before blaming the driver.  `lccc-i686
                # -shared` on a host without 32-bit libgcc fails inside
                # libgcc discovery, which says nothing about `.gnu.hash`
                # sizing or RELRO — the two things this test is about.
                #
                # Skip THIS driver, not the whole test: arm and riscv do not
                # need the multilib, and a host that has those two but not the
                # i386 toolchain should still get the coverage it can get.
                # The `seen` tally below decides whether anything survived.
                pr = sh([CC, "-m32", "-shared", "-fPIC", "-O1", src,
                         "-o", os.path.join(td, ".m32probe.so")], timeout=120)
                if pr.returncode != 0:
                    m32_why = pr.stderr.decode(errors="replace")[-200:]
                    continue
            so = os.path.join(td, drv + ".so")
            r = sh([path, "-O1", "-shared", "-fPIC", src, "-o", so],
                   timeout=120)
            if r.returncode != 0:
                return Result(name, "FAIL", "%s -shared failed: %s"
                              % (drv, r.stderr.decode(errors="replace")[-200:]))
            d = open(so, "rb").read()
            if d[:4] != b"\x7fELF" or (d[4] == 1) != (bits == 32):
                return Result(name, "FAIL", "%s: bad ELF class" % drv)
            # phdr walk (works without section headers), normalised to
            # (type, offset, vaddr, filesz, memsz); 64-bit phdr has TWO
            # 32-bit leading fields (type, flags) — never 8 qwords.
            if bits == 32:
                phoff = struct.unpack_from("<I", d, 28)[0]
                phnum = struct.unpack_from("<H", d, 44)[0]
                ph = [struct.unpack_from("<8I", d, phoff + i * 32)
                      for i in range(phnum)]
                ph = [(p[0], p[1], p[2], p[4], p[5]) for p in ph]
            else:
                phoff = struct.unpack_from("<Q", d, 32)[0]
                phnum = struct.unpack_from("<H", d, 56)[0]
                ph = [struct.unpack_from("<II6Q", d, phoff + i * 56)
                      for i in range(phnum)]
                ph = [(p[0], p[2], p[3], p[5], p[6]) for p in ph]
            loads = [p for p in ph if p[0] == 1]
            def v2o(va):
                for _, off, v, fsz, _ in loads:
                    if v <= va < v + fsz:
                        return off + (va - v)
                return None
            dyn = None
            for ptype, off, _, fsz, _ in ph:
                if ptype == 2:  # PT_DYNAMIC
                    step = 8 if bits == 32 else 16
                    dyn = [struct.unpack_from(
                        "<iI" if bits == 32 else "<qQ", d, off + i * step)
                        for i in range(fsz // step)]
            if dyn is None:
                return Result(name, "FAIL", "%s: no PT_DYNAMIC" % drv)
            ents = dict((t, v) for t, v in dyn)
            gh_v = ents.get(0x6FFFFEF5)
            sym_v, str_v = ents.get(6), ents.get(5)
            syment = ents.get(11)
            if gh_v is None or sym_v is None or str_v is None:
                return Result(name, "FAIL", "%s: DT_GNU_HASH/SYMTAB missing"
                              % drv)
            gho = v2o(gh_v)
            nb, so_off, bw, bsh = struct.unpack_from("<4I", d, gho)
            exp_bw = max(1, 1 << ((N + 3) // 4 - 1).bit_length())
            if (bw & (bw - 1)) or bw != exp_bw:
                return Result(name, "FAIL", "%s: bloom_words=%d want %d"
                              % (drv, bw, exp_bw))
            if bsh != (bw * bits).bit_length() - 1:
                return Result(name, "FAIL", "%s: bloom_shift=%d" % (drv, bsh))
            if nb != N // 4:
                return Result(name, "FAIL", "%s: nbuckets=%d want %d"
                              % (drv, nb, N // 4))
            kw = 4 if bits == 32 else 8
            bloom_o = gho + 16
            buckets_o = bloom_o + bw * kw
            chains_o = buckets_o + nb * 4
            syms_o, strs_o = v2o(sym_v), v2o(str_v)
            def gnu_hash(nm):
                h = 5381
                for c in nm.encode():
                    h = (h * 33 + c) & 0xFFFFFFFF
                return h
            def lookup(want):
                h = gnu_hash(want)
                wv = struct.unpack_from(
                    "<I" if bits == 32 else "<Q", d,
                    bloom_o + ((h // bits) & (bw - 1)) * kw)[0]
                if not (wv >> (h % bits)) & 1 or \
                   not (wv >> ((h >> bsh) % bits)) & 1:
                    return None
                idx = struct.unpack_from("<I", d,
                                         buckets_o + (h % nb) * 4)[0]
                while idx >= so_off:
                    no = struct.unpack_from("<I", d, syms_o + idx * syment)[0]
                    nm = d[strs_o + no:d.index(b"\0", strs_o + no)].decode()
                    chain = struct.unpack_from(
                        "<I", d, chains_o + (idx - so_off) * 4)[0]
                    if nm == want:
                        return idx
                    if chain & 1:
                        return None
                    idx += 1
                return None
            bad = [k for k in range(N) if lookup("v%d" % k) is None]
            if bad:
                return Result(name, "FAIL",
                              "%s: %d symbols unresolvable, e.g. v%d"
                              % (drv, len(bad), bad[0]))
            def bloom_pass(want):
                h = gnu_hash(want)
                wv = struct.unpack_from(
                    "<I" if bits == 32 else "<Q", d,
                    bloom_o + ((h // bits) & (bw - 1)) * kw)[0]
                return bool((wv >> (h % bits)) & 1 and
                            (wv >> ((h >> bsh) % bits)) & 1)
            misses = sum(1 for k in range(3 * N)
                         if bloom_pass("absent_%d" % k))
            if misses > N:
                return Result(name, "FAIL",
                              "%s: absent-name FPR too high (%d/%d)"
                              % (drv, misses, 3 * N))
            if want_relro:
                relro = [p for p in ph if p[0] == 0x6474E552]
                if not relro:
                    return Result(name, "FAIL", "%s: no PT_GNU_RELRO" % drv)
                va, len_ = relro[0][2], relro[0][4]
                if va % page != 0 or (va + len_) % page != 0 or len_ < page:
                    return Result(name, "FAIL",
                                  "%s: RELRO [%#x,+%#x) not page-spanning "
                                  "(sub-page memsz => glibc protects nothing)"
                                  % (drv, va, len_))
            seen += 1
    if seen < 2:
        # A skipped i386 driver is the informative case: the reference
        # toolchain cannot build for i386 here, so the driver was never
        # asked.  `LCCC_REQUIRE_I386=1` (ci.yml and ci_local.sh both set it)
        # means a runner that promised the multilib and did not deliver it is
        # broken, not untested.
        if m32_why:
            status = "FAIL" if os.environ.get("LCCC_REQUIRE_I386") == "1" else "SKIP"
            return Result(name, status,
                          "no -m32 toolchain (%d other drivers verified): %s"
                          % (seen, m32_why))
        return Result(name, "SKIP", "arch drivers missing (built %d)" % seen)
    return Result(name, "PASS", "%d arch drivers verified (@48 exports each)"
                  % seen)


def _static_pie_refusal_test(args, oracles):
    """-static-pie must be refused with a diagnostic, not linked into a SIGSEGV.

    lccc-ld has no position-independent static emitter: it used to write the
    image anyway and the program died in the CRT self-relocation.  A linker that
    cannot honour a request has to say so at link time.
    """
    name = "static_pie_refused"
    lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    if not os.path.exists(lccc_ld):
        return Result(name, "SKIP", "no lccc-ld")
    with tempfile.TemporaryDirectory() as td:
        with open(os.path.join(td, "a.c"), "w") as f:
            f.write('#include <stdio.h>\nint main(void){ printf("ok\\n"); return 0; }\n')
        r = sh([CC, "-c", "-O1", "a.c"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", "fixture compile failed")
        shim = _shim_for(td, lccc_ld)
        out = os.path.join(td, "a.out")
        r = sh([CC, "-B" + shim, "-static-pie", "a.o", "-o", out], cwd=td)
        err = r.stderr.decode(errors="replace")
        if r.returncode == 0:
            # If it links it must also run: the old behaviour linked and then
            # SIGSEGVed, which is strictly worse than refusing.
            code, _ = run_bin(out, [], td)
            if code != 0:
                return Result(name, "FAIL",
                              "linked a -static-pie image that fails at runtime "
                              "(rc=%s); it must refuse at link time" % code)
            return Result(name, "PASS", "supported and runs")
        if "static-pie" not in err and "position-independent" not in err:
            return Result(name, "FAIL",
                          "refused without an explanatory diagnostic")
        return Result(name, "PASS", "refused with a diagnostic")


def _dyn_init_fini_shared_test(args, oracles):
    """A lccc-linked .so must tag .init/.fini and count its RELATIVE run.

    Same contract as the `dyn_init_fini_*` cases, but through the shared
    writer (`-shared`), whose .dynamic is sized and laid out independently:
    DT_INIT/DT_FINI must equal the .init/.fini section addresses, and
    DT_RELACOUNT must equal the leading R_X86_64_RELATIVE run of .rela.dyn
    (re-parsed, not trusted).  A consumer linked against the library by the
    system linker must also run, proving the resized .dynamic did not
    corrupt the image.
    """
    name = "dyn_init_fini_shared"
    lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    if not os.path.exists(lccc_ld):
        return Result(name, "SKIP", "no lccc-ld")
    with tempfile.TemporaryDirectory() as td:
        with open(os.path.join(td, "impl.c"), "w") as f:
            f.write("int impl_var = 30;\n"
                    "int impl_fn(int x){ return x * impl_var; }\n")
        with open(os.path.join(td, "main.c"), "w") as f:
            f.write("#include <stdio.h>\n"
                    "extern int impl_fn(int);\n"
                    "int main(void){ printf(\"%d\\n\", impl_fn(5)); return 0; }\n")
        r = sh([CC, "-c", "-fpic", "-O1", "impl.c"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", "fixture compile failed")
        # Link through the system driver with a -B shim (not the bare lccc
        # driver): only the full driver link pulls in crti/crtn, which are
        # what give the .so its .init/.fini content.
        shim = _shim_for(td, lccc_ld)
        out = os.path.join(td, "libimpl.so")
        r = sh([CC, "-B" + shim, "-shared", "impl.o", "-o", out], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL",
                          "lccc -shared failed: %s" % r.stderr.decode(errors="replace")[:200])
        tags, addrs = dyn_tag_map(out)
        for tagname, tag, sec in (("INIT", _DT_INIT, ".init"),
                                  ("FINI", _DT_FINI, ".fini")):
            if tag not in tags:
                return Result(name, "FAIL", f"DT_{tagname} missing from .dynamic")
            if sec not in addrs:
                return Result(name, "FAIL",
                              f"DT_{tagname} present but {sec} section missing")
            if tags[tag] != addrs[sec]:
                return Result(name, "FAIL",
                              f"DT_{tagname} is {tags[tag]:#x}, "
                              f"{sec} section is at {addrs[sec]:#x}")
        run = rela_dyn_relative_run(out)
        if _DT_RELACOUNT not in tags:
            return Result(name, "FAIL", "DT_RELACOUNT missing from .dynamic")
        if run is None:
            return Result(name, "FAIL",
                          "DT_RELACOUNT present but no .rela.dyn to count")
        if tags[_DT_RELACOUNT] != run:
            return Result(name, "FAIL",
                          f"DT_RELACOUNT is {tags[_DT_RELACOUNT]}, but the "
                          f"leading RELATIVE run of .rela.dyn is {run}")
        # The system linker consumes the library: it must link and run.
        r = sh([CC, "-c", "-O1", "main.c"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", "consumer compile failed")
        r = sh([CC, "main.o", out, "-Wl,-rpath,$ORIGIN", "-o", "main"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL",
                          "consumer link against lccc .so failed: %s"
                          % r.stderr.decode(errors="replace")[:200])
        code, prog_out = run_bin(os.path.join(td, "main"), [], td)
        if code != 0 or prog_out != "150\n":
            return Result(name, "FAIL",
                          "consumer misbehaves: %r" % ((code, prog_out),))
        return Result(name, "PASS")


def _emit_relocs_warns_test(args, oracles):
    """--emit-relocs on a non-script link must warn, never drop silently.

    Relocation retention is only implemented for `-T` script links.  The
    built-in/shared emitters cannot honour the flag, so they must say so:
    lccc-ld used to swallow it (consumed into a bool that only the script
    path reads) and link a `-q` image with no `.rela.*` and no diagnostic.
    Both spellings (`--emit-relocs`, `-q`) are exercised.  Written to pass
    either way once retention lands: if `.rela.text` appears the image must
    run; otherwise the warning must be present.
    """
    name = "emit_relocs_warns_when_unimplemented"
    lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    if not os.path.exists(lccc_ld):
        return Result(name, "SKIP", "no lccc-ld")
    with tempfile.TemporaryDirectory() as td:
        with open(os.path.join(td, "a.c"), "w") as f:
            f.write('#include <stdio.h>\nint main(void){ printf("ok\\n"); return 0; }\n')
        r = sh([CC, "-c", "-O1", "a.c"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", "fixture compile failed")
        shim = _shim_for(td, lccc_ld)
        for spelling in ("--emit-relocs", "-q"):
            out = os.path.join(td, "a.out")
            r = sh([CC, "-B" + shim, "-Wl," + spelling, "a.o", "-o", out], cwd=td)
            err = r.stderr.decode(errors="replace")
            if r.returncode != 0:
                return Result(name, "FAIL",
                              "%s: link failed: %s" % (spelling, err[:200]))
            code, prog_out = run_bin(out, [], td)
            if code != 0 or prog_out != "ok\n":
                return Result(name, "FAIL",
                              "%s: linked image misbehaves: %r" % (spelling, (code, prog_out)))
            has_rela = b".rela.text" in open(out, "rb").read()
            if has_rela:
                continue  # retention implemented: image runs, nothing to warn
            if "--emit-relocs ignored" not in err:
                return Result(name, "FAIL",
                              "%s: no .rela.text retained and no warning on stderr"
                              % spelling)
        return Result(name, "PASS")



def _gc_eh_frame_invariant_test(args, oracles):
    """`.eh_frame` must survive --gc-sections with exactly the live FDE set.

    The runtime `gc_sections_keeps_eh_frame` case proves unwinding works; this
    checks the *structure*, because the two failure modes are different: a
    dropped `.eh_frame` breaks `backtrace()`, while a stale FDE left behind
    hands the unwinder CFI for a function whose address range was recycled by
    live code after compaction -- wrong unwinding rather than none, and only
    on the paths that happen to hit it.

    Invariants on the lccc image (bfd as cross-check):
      * every STT_FUNC symbol in an executable section is covered by an FDE;
      * no FDE describes a region that is not live code (PLT/.init/.fini
        excepted: those FDEs are linker-synthesised by design).
    """
    name = "gc_eh_frame_invariant"
    td = tempfile.mkdtemp(prefix="lccc-gceh-")
    try:
        src = r"""
            #include <stdio.h>
            #include <execinfo.h>
            static int l4(void){ void *bt[16]; return backtrace(bt, 16); }
            static int l3(void){ return l4(); }
            static int l2(void){ return l3(); }
            int live_a(void){ return l2(); }
            int live_b(void){ return live_a() + 1; }
            int dead_1(void){ return 111; }
            int dead_2(void){ return 222; }
            int dead_3(void){ return 333; }
            int dead_4(void){ return 444; }
            int main(void){ printf("%d\n", live_b()); return 0; }
        """
        with open(os.path.join(td, "a.c"), "w") as f:
            f.write(src)
        cf = ["-O0", "-ffunction-sections", "-fdata-sections"]
        r = sh([CC, *cf, "-c", "a.c", "-o", "a.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", f"compile failed: {r.stderr.decode()[:200]}")

        # lccc-ld is driven directly with the same flag surface gcc would use,
        # so the test does not depend on a -B shim being installed.
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        crt = [sh([CC, "-print-file-name=" + n], cwd=td).stdout.decode().strip()
               for n in ("crt1.o", "crti.o", "crtn.o")]
        r = sh([lccc_ld, "-o", "a.lccc", crt[0], crt[1], "a.o", crt[2],
                "--dynamic-linker", "/lib64/ld-linux-x86-64.so.2",
                "-L/usr/lib/x86_64-linux-gnu", "-lc",
                "--gc-sections", "--export-dynamic"], cwd=td)
        if r.returncode != 0 or not os.path.exists(os.path.join(td, "a.lccc")):
            return Result(name, "FAIL", f"lccc-ld failed: {r.stderr.decode()[:400]}")

        images = {"lccc": "a.lccc"}
        r = sh([CC, *cf, "-Wl,--gc-sections", "-rdynamic", "-o", "a.bfd", "a.o"], cwd=td)
        if r.returncode == 0:
            images["bfd"] = "a.bfd"

        sys.path.insert(0, HERE)
        import check_gc_eh_frame as inv  # noqa: PLC0415 - sibling module

        failed = []
        for tag, img in images.items():
            path = os.path.join(td, img)
            problems = inv.check(path)
            if not inv.fde_ranges(path):
                problems.append("no FDEs at all: unwinding is dead")
            if problems:
                failed.append(f"{tag}: " + "; ".join(problems))
        if failed:
            return Result(name, "FAIL", " | ".join(failed))

        # GC must still collect: the link is not allowed to "pass" by simply
        # keeping every FDE.  `--export-dynamic` exports the dead_* functions
        # and so legitimately roots them, so compare against a link of the
        # very same object without --gc-sections: the collected link must
        # carry strictly fewer FDEs while still covering all live code.
        # Neither link may use --export-dynamic here: it legitimately roots
        # every global, so nothing would be collected and the comparison
        # would prove nothing.
        base = ["--dynamic-linker", "/lib64/ld-linux-x86-64.so.2",
                "-L/usr/lib/x86_64-linux-gnu", "-lc"]
        r = sh([lccc_ld, "-o", "a.gc", crt[0], crt[1], "a.o", crt[2],
                *base, "--gc-sections"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc-ld GC link failed: {r.stderr.decode()[:300]}")
        r = sh([lccc_ld, "-o", "a.nogc", crt[0], crt[1], "a.o", crt[2], *base], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc-ld no-GC failed: {r.stderr.decode()[:300]}")
        n_gc = len(inv.fde_ranges(os.path.join(td, "a.gc")))
        n_nogc = len(inv.fde_ranges(os.path.join(td, "a.nogc")))
        if not (0 < n_gc < n_nogc):
            return Result(
                name, "FAIL",
                f"--gc-sections must prune FDEs but keep unwinding: "
                f"{n_gc} with GC vs {n_nogc} without",
            )
        for tag in ("gc", "nogc"):
            problems = inv.check(os.path.join(td, f"a.{tag}"))
            if problems:
                return Result(name, "FAIL", f"a.{tag}: " + "; ".join(problems))
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


def _elf32_vdso_test(args, oracles):
    """ELF32 ET_DYN metadata, i386 PIC relocations and multi-node versions."""
    name = "script_elf32_vdso_multiversion"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        source = """
            __attribute__((visibility("hidden"))) int hidden_data = 40;
            int old_api(void) { return hidden_data + 1; }
            int new_api(void) { return hidden_data + 2; }
        """
        script = r'''
            OUTPUT_FORMAT("elf32-i386")
            OUTPUT_ARCH(i386)
            PHDRS {
              text PT_LOAD FLAGS(5) FILEHDR PHDRS;
              dynamic PT_DYNAMIC FLAGS(4);
            }
            SECTIONS {
              . = SIZEOF_HEADERS;
              .hash : { *(.hash) } :text
              .gnu.hash : { *(.gnu.hash) }
              .dynsym : { *(.dynsym) }
              .dynstr : { *(.dynstr) }
              .gnu.version : { *(.gnu.version) }
              .gnu.version_d : { *(.gnu.version_d) }
              .dynamic : { *(.dynamic) } :text :dynamic
              .text : { *(.text*) } :text
              .data : { *(.data*) } :text
              /DISCARD/ : { *(.note*) *(.eh_frame*) *(.comment) }
            }
            VERSION {
              OLD_1 { global: old_api; local: *; };
              NEW_2 { global: new_api; } OLD_1;
            }
        '''
        with open(os.path.join(td, "v.c"), "w") as f:
            f.write(textwrap.dedent(source))
        with open(os.path.join(td, "v.lds"), "w") as f:
            f.write(textwrap.dedent(script))
        r = sh([CC, "-m32", "-fPIC", "-O1", "-fno-asynchronous-unwind-tables",
                "-fno-stack-protector", "-c", "v.c", "-o", "v.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:200])
        input_relocs = sh(["readelf", "-rW", "v.o"], cwd=td).stdout.decode()
        if "R_386_GOTPC" not in input_relocs or "R_386_GOTOFF" not in input_relocs:
            return Result(name, "SKIP", "compiler did not emit i386 PIC GOT relocations")

        common = ["-m", "elf_i386", "-shared", "-Bsymbolic", "-soname",
                  "test-vdso32.so.1", "-T", "v.lds", "v.o"]
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        r = sh([lccc_ld] + common + ["-o", "out.lccc.so"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"lccc-ld failed: {r.stderr.decode()[:400]}")
        r = sh(["ld.bfd"] + common + ["-o", "out.bfd.so"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", f"bfd rejected oracle fixture: {r.stderr.decode()[:200]}")

        dynsyms = sh(["readelf", "--dyn-syms", "-W", "out.lccc.so"], cwd=td).stdout.decode()
        versions = sh(["readelf", "-V", "-W", "out.lccc.so"], cwd=td).stdout.decode()
        dynamic = sh(["readelf", "-d", "-W", "out.lccc.so"], cwd=td).stdout.decode()
        remaining = sh(["readelf", "-r", "-W", "out.lccc.so"], cwd=td).stdout.decode()
        required = ("old_api@@OLD_1", "new_api@@NEW_2")
        if any(token not in dynsyms for token in required) or "hidden_data" in dynsyms:
            return Result(name, "FAIL", "ELF32 dynsym version/visibility assignment is wrong")
        if "Parent 1: OLD_1" not in versions or "REV: 1" not in versions.upper():
            return Result(name, "FAIL", "multi-node verdef inheritance is absent")
        for token in ("SONAME", "HASH", "GNU_HASH", "SYMENT"):
            if token not in dynamic:
                return Result(name, "FAIL", f"ELF32 .dynamic lacks {token}")
        if re.search(r" R_386_", remaining):
            return Result(name, "FAIL", "vDSO retained a dynamic relocation")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


def _script_nonalloc_section_test(args, oracles):
    """Non-ALLOC PROGBITS must be written and have relocations applied."""
    name = "script_nonalloc_debug_payload"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        asm = r'''
            .globl _start
            .section .text,"ax"
        _start:
            ret
            .section .discard.addressable,"a",@progbits
        discarded_local:
            .long 0xdeadbeef
            .section .debug_test,"",@progbits
            .quad _start
            .quad discarded_local
            .ascii "DWARF"
        '''
        script = r'''
            ENTRY(_start)
            SECTIONS {
              . = 0x400000;
              .text : {
                *(.text)
                inside_text_end = .;
                inside_constant = 5;
                inside_absolute = ABSOLUTE(.);
              }
              relative_text_end = .;
              absolute_text_size = relative_text_end - ADDR(.text);
              forced_absolute_end = ABSOLUTE(.);
              .debug_test 0 : { *(.debug_test) }
              /DISCARD/ : { *(.note*) *(.discard.addressable) }
            }
        '''
        with open(os.path.join(td, "d.s"), "w") as f:
            f.write(textwrap.dedent(asm))
        with open(os.path.join(td, "d.lds"), "w") as f:
            f.write(textwrap.dedent(script))
        r = sh([CC, "-c", "d.s", "-o", "d.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "SKIP", r.stderr.decode()[:200])
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
        for linker, output in ((lccc_ld, "out.lccc"), ("ld.bfd", "out.bfd")):
            r = sh([linker, "-T", "d.lds", "d.o", "-o", output], cwd=td)
            if r.returncode != 0:
                status = "FAIL" if linker == lccc_ld else "SKIP"
                return Result(name, status, r.stderr.decode()[:300])
            r = sh(["objcopy", "--dump-section", f".debug_test={output}.debug",
                    output], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL", f"objcopy rejected {output}: {r.stderr.decode()[:200]}")
        lccc_data = open(os.path.join(td, "out.lccc.debug"), "rb").read()
        bfd_data = open(os.path.join(td, "out.bfd.debug"), "rb").read()
        expected = (0x400000).to_bytes(8, "little") + bytes(8) + b"DWARF"
        if lccc_data != bfd_data or lccc_data != expected:
            return Result(name, "FAIL", "non-ALLOC bytes/relocation differ from bfd")
        readelf = sh(["readelf", "-SW", "out.lccc"], cwd=td)
        if readelf.returncode != 0 or b"past end of file" in readelf.stderr:
            return Result(name, "FAIL", "section header extends beyond the output file")
        names = {
            "inside_text_end", "inside_constant", "inside_absolute",
            "relative_text_end", "absolute_text_size", "forced_absolute_end",
        }
        def symbol_semantics(output):
            result = {}
            lines = sh(["readelf", "-sW", output], cwd=td).stdout.decode().splitlines()
            for line in lines:
                fields = line.split()
                if len(fields) >= 8 and fields[7] in names:
                    result[fields[7]] = (fields[1], fields[6])
            return result
        lccc_syms = symbol_semantics("out.lccc")
        bfd_syms = symbol_semantics("out.bfd")
        if lccc_syms != bfd_syms:
            return Result(name, "FAIL",
                          f"script symbol value/section semantics differ: "
                          f"lccc={lccc_syms}, bfd={bfd_syms}")
        for relative in ("inside_text_end", "inside_constant", "relative_text_end"):
            if lccc_syms.get(relative, (None, "ABS"))[1] == "ABS":
                return Result(name, "FAIL", f"{relative} lost its output-section home")
        for absolute in ("inside_absolute", "absolute_text_size", "forced_absolute_end"):
            if lccc_syms.get(absolute, (None, None))[1] != "ABS":
                return Result(name, "FAIL", f"{absolute} is not SHN_ABS")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        if not args.keep:
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


def _whole_archive_r_test(args, oracles):
    """Standalone `lccc-ld -r --whole-archive` — the kernel's vmlinux.o link.

    linux 6.18 links vmlinux.o with
      $(LD) -r -o vmlinux.o --whole-archive vmlinux.a --no-whole-archive \
          --start-group --end-group
    where vmlinux.a is a THIN archive (Makefile.vmlinux_a).  lccc-ld once
    rerouted positional archives under --whole-archive into a passthrough list
    that only the userspace mode reads, so the relocatable mode lost the
    archive entirely: the kernel build died with `lccc-ld: error: no input
    files`, and a lone `-r --whole-archive x.a -o y.o` silently produced an
    EMPTY relocatable object.  Both archive flavours (regular and thin) are
    exercised here, and the merged object must keep the unreferenced members
    (that is the entire point of --whole-archive) and final-link + run with
    identical output to GNU ld's merge.
    """
    name = "reloc_whole_archive_thin_and_regular"
    td = tempfile.mkdtemp(prefix=f"lnk.{name}.")
    try:
        with open(os.path.join(td, "u.c"), "w") as f:
            f.write("int used(void){return 7;}\n")
        with open(os.path.join(td, "u2.c"), "w") as f:
            f.write("int used2(void){return 35;}\n")
        # keepme() is referenced by NOBODY: lazy archive scanning can never
        # pull it in, so its presence in the merged object proves the
        # --whole-archive force-load (the same discipline as
        # whole_archive_exec, applied to the relocatable mode).
        with open(os.path.join(td, "k.c"), "w") as f:
            f.write("int keepme(void){return 1;}\n")
        with open(os.path.join(td, "m2.c"), "w") as f:
            f.write("#include <stdio.h>\n"
                    "extern int used(void);\n"
                    "extern int used2(void);\n"
                    "int main(void){ printf(\"%d\\n\", used() + used2()); return 0; }\n")
        for c in ("u.c", "u2.c", "k.c", "m2.c"):
            r = sh([CC, "-c", "-O1", c, "-o", c[:-2] + ".o"], cwd=td)
            if r.returncode != 0:
                return Result(name, "SKIP", r.stderr.decode()[:200])
        sh(["ar", "crD", "--thin", "thin.a", "u.o", "u2.o", "k.o"], cwd=td)
        sh(["ar", "crD", "reg.a", "u.o", "u2.o", "k.o"], cwd=td)
        lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")

        def defined_syms(path):
            out = sh(["readelf", "-sW", path], cwd=td).stdout.decode()
            names = set()
            for ln in out.splitlines():
                parts = ln.split()
                if len(parts) >= 8 and parts[3] == "FUNC" and parts[4] != "UND":
                    names.add(parts[7])
            return names

        for arch in ("thin.a", "reg.a"):
            # exact kernel flag shape
            r = sh([lccc_ld, "-r", "-o", f"m.{arch}.lccc.o", "--whole-archive",
                    arch, "--no-whole-archive", "--start-group", "--end-group"], cwd=td)
            if r.returncode != 0:
                return Result(name, "FAIL",
                              f"lccc-ld -r --whole-archive {arch} failed: {r.stderr.decode()[:300]}")
            mine = defined_syms(f"m.{arch}.lccc.o")
            for required in ("used", "used2", "keepme"):
                if required not in mine:
                    return Result(name, "FAIL",
                                  f"{arch}: whole-archive member lost ({required} missing)")
            # GNU ld parity on the identical link: same defined symbol set
            r = sh(["ld", "-r", "-o", f"m.{arch}.ld.o", "--whole-archive",
                    arch, "--no-whole-archive"], cwd=td)
            if r.returncode != 0:
                return Result(name, "SKIP", f"GNU ld -r {arch} failed")
            gnu = defined_syms(f"m.{arch}.ld.o")
            if mine != gnu:
                return Result(name, "FAIL",
                              f"{arch}: defined symbol set differs from GNU ld: "
                              f"lccc-only={sorted(mine-gnu)} gnu-only={sorted(gnu-mine)}")
        # End-to-end: merge the thin archive via -r --whole-archive (kernel
        # shape), final-link the merged object with gcc, run it.
        r = sh([lccc_ld, "-r", "-o", "e2e.lccc.o", "--whole-archive", "thin.a",
                "--no-whole-archive"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"e2e -r failed: {r.stderr.decode()[:300]}")
        rr = sh([CC, "e2e.lccc.o", "m2.o", "-o", "fin.e2e"], cwd=td)
        if rr.returncode != 0:
            return Result(name, "FAIL", f"e2e final link failed: {rr.stderr.decode()[:300]}")
        code, out = run_bin(os.path.join(td, "fin.e2e"), [], td)
        if code != 0 or out != "42\n":
            return Result(name, "FAIL", f"e2e run got {(code, out)!r}")
        return Result(name, "PASS")
    except Exception as e:
        return Result(name, "FAIL", f"harness exception: {e!r}")
    finally:
        shutil.rmtree(td, ignore_errors=True)


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
        if not bfd:
            return Result(name, "SKIP", "ld.bfd (the --defsym reference) not found")
        r = sh([ld, "-shared", "--defsym=alias_sym=keep_me", "-o", "d.so", "lib.o"], cwd=td)
        if r.returncode != 0:
            return Result(name, "FAIL", f"--defsym link failed: {r.stderr.decode()[:200]}")
        got = sh(["readelf", "--dyn-syms", "-W", "d.so"], cwd=td).stdout.decode(errors="replace")
        n_lccc = got.count("alias_sym")
        rb = sh([bfd, "-shared", "--defsym=alias_sym=keep_me", "-o", "db.so", "lib.o"], cwd=td)
        if rb.returncode != 0:
            return Result(name, "SKIP", f"ld.bfd refused the reference link: "
                                        f"{rb.stderr.decode()[:200]}")
        ref = sh(["readelf", "--dyn-syms", "-W", "db.so"], cwd=td).stdout.decode(errors="replace")
        n_bfd = ref.count("alias_sym")
        if n_lccc == 0 or n_lccc != n_bfd:
            return Result(name, "FAIL", f"--defsym alias count {n_lccc} != bfd {n_bfd}")
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
    if shutil.which(CXX) is None:
        return Result(name, "SKIP", f"no {CXX}")
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

# GCC driver options lccc-ld must accept without an "ignoring" diagnostic.
_DRIVER_SILENT_FLAGS = ("--push-state", "--pop-state", "--eh-frame-hdr",
                        "--no-warn-execstack", "-plugin-opt=whatever")


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
        for flag in _DRIVER_SILENT_FLAGS:
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
        relocs_tool = os.environ.get("LCCC_RELOCS_TOOL") or os.path.join(
            os.environ.get("LCCC_ORACLE_PREFIX") or os.path.expanduser("~/tools"),
            "bin", "relocs")
        if not os.path.exists(relocs_tool):
            out.append(Result("emit_relocs_kernel_relocs_tool", "SKIP",
                              f"{relocs_tool} missing: run tests/linker/setup_kernel_tools.sh"))
        elif not bfd:
            out.append(Result("emit_relocs_kernel_relocs_tool", "SKIP",
                              "no GNU ld for the reference image"))
        else:
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
            else:
                out.append(Result("export_dynamic_matches_gnu_ld", "SKIP",
                                  f"GNU ld reference link failed: {r.stderr.decode()[:200]}"))
        else:
            out.append(Result("export_dynamic_matches_gnu_ld", "SKIP", "ld.bfd not found"))
        return out
    except Exception as e:
        return [Result("export_dynamic", "FAIL", f"harness exception: {e!r}")]
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


# ============================================================================
# IDENTICAL CODE FOLDING (--icf)
# ============================================================================
#
# The dangerous failure mode of ICF is silent: two functions with identical
# instruction bytes that differ only in their relocation targets. On x86-64 a
# call is "e8 <rel32>" with the displacement supplied by a relocation, so
#     int wrap_a(void){ return alpha(); }
#     int wrap_b(void){ return beta();  }
# compile to the same six bytes. A byte-only comparison folds them and wrap_b()
# starts returning alpha()'s value, with no diagnostic anywhere. These cases
# run the program and check the values, which is the only way to catch it.

case("icf_never_folds_different_call_targets",
    {"a.c": """
#include <stdio.h>
__attribute__((noinline)) int alpha(void){ return 11; }
__attribute__((noinline)) int beta (void){ return 22; }
__attribute__((noinline)) int wrap_a(void){ return alpha(); }
__attribute__((noinline)) int wrap_b(void){ return beta();  }
int main(void){ printf("%d %d\\n", wrap_a(), wrap_b()); return 0; }
"""},
    compile_flags=["-O1", "-ffunction-sections"],
    lccc_only_flags=["-Wl,--icf=all"],
    expect_stdout="11 22\n",
    tags=("icf",))

# Genuinely identical functions -- same bytes, same relocation targets -- must
# fold onto one address. Verified through the symbol table, not just size.
case("icf_folds_truly_identical_functions",
    {"a.c": """
#include <stdio.h>
__attribute__((noinline)) int leaf(void){ return 5; }
__attribute__((noinline)) int dup_one(void){ return leaf() * 3 + 1; }
__attribute__((noinline)) int dup_two(void){ return leaf() * 3 + 1; }
int main(void){ printf("%d %d\\n", dup_one(), dup_two()); return 0; }
"""},
    compile_flags=["-O1", "-ffunction-sections"],
    lccc_only_flags=["-Wl,--icf=all"],
    expect_stdout="16 16\n",
    expect_same_address=[["dup_one", "dup_two"]],
    tags=("icf",))

# Differing addends are differing code even when the bytes match.
case("icf_respects_reloc_addends",
    {"a.c": """
#include <stdio.h>
int tbl[4] = {10, 20, 30, 40};
__attribute__((noinline)) int at0(void){ return tbl[0]; }
__attribute__((noinline)) int at2(void){ return tbl[2]; }
int main(void){ printf("%d %d\\n", at0(), at2()); return 0; }
"""},
    compile_flags=["-O1", "-ffunction-sections"],
    lccc_only_flags=["-Wl,--icf=all"],
    expect_stdout="10 30\n",
    tags=("icf",))

# C requires distinct functions to have distinct addresses. Under --icf=safe a
# function whose address escapes must not be folded. The pointers are volatile
# so the compiler cannot fold the comparison at compile time.
case("icf_safe_preserves_function_pointer_identity",
    {"a.c": """
#include <stdio.h>
__attribute__((noinline)) int dup_x(int v){ return v ^ 0x5a5a; }
__attribute__((noinline)) int dup_y(int v){ return v ^ 0x5a5a; }
int (*volatile px)(int) = dup_x;
int (*volatile py)(int) = dup_y;
int main(void){ printf("same=%d val=%d\\n", px == py, px(1)); return 0; }
"""},
    compile_flags=["-O1", "-ffunction-sections"],
    lccc_only_flags=["-Wl,--icf=safe"],
    expect_stdout="same=0 val=23131\n",
    tags=("icf",))

# Folding must not disturb C++ exceptions: the unwinder looks up the landing
# pad by return address, so a fold that loses .eh_frame coverage crashes here.
case("icf_folded_code_still_unwinds",
    {"a.cpp": """
#include <cstdio>
__attribute__((noinline)) int thrower(int k){ if (k) throw 7; return 0; }
__attribute__((noinline)) int guard_a(int k){ try { return thrower(k); } catch (int e) { return e + 1; } }
__attribute__((noinline)) int guard_b(int k){ try { return thrower(k); } catch (int e) { return e + 1; } }
int main(){ printf("%d %d\\n", guard_a(1), guard_b(1)); return 0; }
"""},
    compile_flags=["-O1", "-ffunction-sections"],
    ldflags=["-lstdc++", "-lm"],
    lccc_only_flags=["-Wl,--icf=all"],
    expect_stdout="8 8\n",
    tags=("icf",))

# --icf= with an unrecognised value must disable folding, not fail the link
# (gold and lld both accept --icf=none this way).
case("icf_none_is_accepted_and_disables",
    {"a.c": """
#include <stdio.h>
__attribute__((noinline)) int d1(void){ return 3; }
__attribute__((noinline)) int d2(void){ return 3; }
int main(void){ printf("%d\\n", d1() + d2()); return 0; }
"""},
    compile_flags=["-O1", "-ffunction-sections"],
    lccc_only_flags=["-Wl,--icf=none"],
    expect_stdout="6\n",
    tags=("icf",))

# Two objects, byte-identical getters over same-indexed locals with DIFFERENT
# bytes: folding swaps the tables (the unqualified-(shndx,value) trap that a
# single-pass ICF cannot see). Must hold under safe AND all — local-target
# identity is correctness, not safety mode.
case("icf_cross_object_locals_safe",
    {"a.c": 'static const int table_a[4] = {11, 22, 33, 44};\n'
            'const int *get_a(void) { return table_a; }\n',
     "b.c": 'static const int table_b[4] = {55, 66, 77, 88};\n'
            'const int *get_b(void) { return table_b; }\n',
     "main.c": """
#include <stdio.h>
const int *get_a(void);
const int *get_b(void);
int main(void){
    const int *a = get_a(), *b = get_b();
    printf("%d %d %d %d / %d %d %d %d\\n", a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]);
    return 0;
}
"""},
    compile_flags=["-O2", "-ffunction-sections", "-fdata-sections"],
    lccc_only_flags=["-Wl,--icf=safe"],
    expect_stdout="11 22 33 44 / 55 66 77 88\n",
    tags=("icf",)),

case("icf_cross_object_locals_all",
    {"a.c": 'static const int table_a[4] = {11, 22, 33, 44};\n'
            'const int *get_a(void) { return table_a; }\n',
     "b.c": 'static const int table_b[4] = {55, 66, 77, 88};\n'
            'const int *get_b(void) { return table_b; }\n',
     "main.c": """
#include <stdio.h>
const int *get_a(void);
const int *get_b(void);
int main(void){
    const int *a = get_a(), *b = get_b();
    printf("%d %d %d %d / %d %d %d %d\\n", a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]);
    return 0;
}
"""},
    compile_flags=["-O2", "-ffunction-sections", "-fdata-sections"],
    lccc_only_flags=["-Wl,--icf=all"],
    expect_stdout="11 22 33 44 / 55 66 77 88\n",
    tags=("icf",)),

case("icf_transitive_cross_object_fold",
    # Positive cross-object proof for iterative ICF: f1->g1 and f2->g2 fold
    # only because the leaves are proven equivalent first (same global
    # callee). Single-pass comparison folds these for the wrong reason or
    # not at all; iteration folds them soundly — in safe mode, nothing taken.
    # NOTE: `leaf` lives in a third object on purpose. A same-TU leaf lets
    # gcc skip the call realignment (`call; ret`, 6 bytes) while an imported
    # one forces `sub/add` — the chains would differ before the linker ever
    # sees them and the test would assert a fold that must not happen.
    {"a.c": """
int leaf(void);
__attribute__((noinline)) static int g1(void){ return leaf(); }
__attribute__((noinline)) int f1(void){ return g1(); }
""",
     "b.c": """
int leaf(void);
__attribute__((noinline)) static int g2(void){ return leaf(); }
__attribute__((noinline)) int f2(void){ return g2(); }
""",
     "leaf.c": "__attribute__((noinline)) int leaf(void){ return 42; }\n",
     "main.c": """
#include <stdio.h>
int f1(void); int f2(void);
int main(void){ printf("%d %d\\n", f1(), f2()); return 0; }
"""},
    compile_flags=["-O1", "-ffunction-sections"],
    lccc_only_flags=["-Wl,--icf=safe"],
    expect_stdout="42 42\n",
    expect_same_address=[["f1", "f2"]],
    tags=("icf",)),

case("icf_hot_unlikely_stay_split",
    # PGO partitioning is a linker-level contract (merge keeps .text.hot /
    # .text.unlikely as separate outputs for I-cache locality). Byte-identical
    # twins across the split must NOT fold — not even under `all`: same
    # output section is the deep invariant, and folding hot onto unlikely
    # would silently relocate hot code into the cold tail.
    {"a.c": '__attribute__((section(".text.hot")))\n'
            'int fa(void) { return 42; }\n',
     "b.c": '__attribute__((section(".text.unlikely")))\n'
            'int fb(void) { return 42; }\n',
     "main.c": """
#include <stdio.h>
int fa(void); int fb(void);
int main(void){ printf("%d\\n", fa() + fb()); return 0; }
"""},
    compile_flags=["-O2"],
    lccc_only_flags=["-Wl,--icf=all"],
    expect_stdout="84\n",
    expect_distinct_addresses=[["fa", "fb"]],
    tags=("icf",)),

case("icf_local_ifunc_resolvers_survive",
    # Twin resolvers (byte-identical, like the twin impls) must NEVER fold —
    # not even under `all`. The IPLT slot allocator skips ifuncs in dead
    # sections, so a folded resolver loses its slot while its ifunc still
    # binds through it: the caller would run the resolver bytes as the
    # target and print an address sum instead of 2.
    {"a.c": """
#include <stdio.h>
static int impl1(void){ return 1; }
static int impl2(void){ return 1; }
static int (*rsv1(void))(void){ return impl1; }
static int (*rsv2(void))(void){ return impl2; }
static int fn1(void) __attribute__((ifunc("rsv1")));
static int fn2(void) __attribute__((ifunc("rsv2")));
int main(void){ printf("%d\\n", fn1() + fn2()); return 0; }
"""},
    compile_flags=["-O2", "-ffunction-sections"],
    lccc_only_flags=["-Wl,--icf=all"],
    expect_stdout="2\n",
    tags=("icf", "ifunc")),

case("icf_shifted_indices_fold",
    # The discriminating iteration proof, live: f1 and f2 are byte-identical
    # but reference their (also identical) callees through DIFFERENT section
    # indices (6 vs 7 — the pad section shifts b.s). Index comparison cannot
    # fold these; equivalence classes prove the callees identical first and
    # fold both pairs. Assembly fixtures pin the layout exactly (gcc orders
    # same-shaped TUs identically, which would hide the divergence).
    {"a.s": """
    .section .text.f1,"ax"
    .globl f1
    .type f1,@function
f1:
    call g1
    ret
    .size f1,.-f1
    .section .text.g1,"ax"
    .type g1,@function
g1:
    call leaf
    ret
    .size g1,.-g1
""",
     "b.s": """
    .section .text.pad,"ax"
    .type pad,@function
pad:
    mov $7, %eax
    ret
    .size pad,.-pad
    .section .text.f2,"ax"
    .globl f2
    .type f2,@function
f2:
    call g2
    ret
    .size f2,.-f2
    .section .text.g2,"ax"
    .type g2,@function
g2:
    call leaf
    ret
    .size g2,.-g2
""",
     "leaf.s": """
    .text
    .globl leaf
    .type leaf,@function
leaf:
    mov $42, %eax
    ret
    .size leaf,.-leaf
""",
     "main.c": """
#include <stdio.h>
int f1(void); int f2(void);
int main(void){ printf("%d %d\\n", f1(), f2()); return 0; }
"""},
    link_inputs=["main.o", "a.o", "b.o", "leaf.o"],
    lccc_only_flags=["-Wl,--icf=safe"],
    expect_stdout="42 42\n",
    expect_same_address=[["f1", "f2"]],
    tags=("icf",)),

case("icf_weak_strong_address_identity",
    # Weak dup in A (byte-distinct), strong dup + identical twin in B, table
    # observing dup's address: first-def-wins marked only the shadowed weak
    # twin and folded the surviving strong address onto its twin. Safe mode
    # must keep all three addresses distinct (observed via .symtab, so the
    # test itself takes no address and cannot perturb the property).
    {"a.c": "__attribute__((weak)) int dup(void){ return 1; }\n",
     "b.c": """
int dup(void){ return 0x10 + 0x20; }
int twin(void){ return 0x10 + 0x20; }
int (*tab[])(void) = { dup };
""",
     "main.c": """
#include <stdio.h>
int dup(void); int twin(void);
int main(void){ printf("%d\\n", dup() + twin()); return 0; }
"""},
    compile_flags=["-O1", "-ffunction-sections"],
    lccc_only_flags=["-Wl,--icf=safe"],
    expect_stdout="96\n",
    expect_distinct_addresses=[["dup", "twin"]],
    tags=("icf",)),

case("icf_lea_address_taken_safe",
    # PC-relative lea of f (no absolute relocation anywhere; the volatile
    # pins the materialization against devirtualization): the opcode check
    # must see the address escape and keep f and g distinct. An
    # absolute-only safe mode folds them.
    {"a.c": """
#include <stdio.h>
__attribute__((noinline)) int f(void){ return 7; }
__attribute__((noinline)) int g(void){ return 7; }
int main(void){
    int (*volatile p)(void) = f;
    printf("%d\\n", p() + g());
    return 0;
}
"""},
    compile_flags=["-O1", "-ffunction-sections"],
    lccc_only_flags=["-Wl,--icf=safe"],
    expect_stdout="14\n",
    expect_distinct_addresses=[["f", "g"]],
    tags=("icf",)),

# ============================================================================
# 13. GNU PROPERTY NOTE (CET / ISA) MERGE — differential against bfd
# ============================================================================
# The `.s` fixtures carry hand-crafted `.note.gnu.property` notes (gcc emits
# none for plain objects); the linker's merged output note must agree with
# bfd's entry for entry.  Merge rules verified against binutils 2.47
# (`_bfd_x86_elf_merge_gnu_properties`) and ld 2.44 behaviour.

def _prop_note_asm(sym, entries, *more_notes, entry=False):
    """Assembly defining `sym` plus a `.note.gnu.property` section.

    `entries` (and each of `more_notes`) is one NT_GNU_PROPERTY_TYPE_0 note:
    [(type, value), ...] for 4-byte bitmasks, or (type, value, datasz) for
    the generic types with another size (datasz 8: STACK_SIZE, 0: marker).
    Entries use the 64-bit layout (data padded to 8); every note starts
    8-aligned.  Extra notes land in the SAME section, the shape gas 2.47
    produces itself when it appends its x86 used-note to a hand-written
    one.  `entry` adds a raw exit(0) `_start` for -nostdlib links.
    Directives stay indented and the label sits at column 0, exactly
    like COMDAT_ASM above (the harness dedents fixtures, so the label must
    carry no indentation of its own).
    """
    lines = [
        '    .section .note.gnu.property,"a"',
        "    .align 8",
    ]
    for note in (entries,) + more_notes:
        body = []
        for ent in note:
            t, v, sz = ent if len(ent) == 3 else (ent[0], ent[1], 4)
            body += [f"    .long {t:#x}", f"    .long {sz}"]
            if sz == 4:
                body += [f"    .long {v:#x}"]
            elif sz == 8:
                body += [f"    .quad {v:#x}"]
            body += ["    .balign 8"]
        lines += [
            "    .long 4",
            "    .long 2f - 1f",
            "    .long 5",
            '    .asciz "GNU"',
            "1:",
        ] + body + ["2:"]
    lines += [
        "    .text",
        f"    .globl {sym}",
        f"{sym}:",
        "    .long 1",
    ]
    if entry:
        lines += ["    .globl _start", "_start:", "    movl $60, %eax",
                  "    xorl %edi, %edi", "    syscall"]
    return "\n".join(lines) + "\n"

_PROP_MAIN_C = """
#include <stdio.h>
int main(void){ printf("prop-ok\\\\n"); return 0; }
"""

case("prop_note_and_or_merge",
    {"a.s": _prop_note_asm("note_a", [(0xC0000002, 0x3), (0xC0008002, 0x4)]),
     "b.s": _prop_note_asm("note_b", [(0xC0000002, 0x1)]),
     "main.c": _PROP_MAIN_C},
    # AND class intersects (3 & 1 = IBT only); OR class keeps
    # ISA_1_NEEDED from whichever input has it.
    expect_prop_note_match=True,
    tags=("propnote",))

case("prop_note_or_survives_veto",
    {"a.s": _prop_note_asm("note_a", [(0xC0000002, 0x3), (0xC0008002, 0x4)]),
     "main.c": _PROP_MAIN_C},
    # main.o carries no note: the AND type is vetoed (dropped) while the
    # OR type survives — the shape every CET-less TU gives the merge.
    expect_prop_note_match=True,
    tags=("propnote",))

case("prop_note_z_ibt_rescues_veto",
    {"a.s": _prop_note_asm("note_a", [(0xC0000002, 0x3)]),
     "main.c": _PROP_MAIN_C},
    ldflags=["-Wl,-z,ibt"],
    # `-z ibt` recreates the vetoed AND type with exactly the IBT bit.
    expect_prop_note_match=True,
    tags=("propnote",))

case("prop_note_z_ibt_joined_form",
    {"a.s": _prop_note_asm("note_a", [(0xC0000002, 0x3)]),
     "main.c": _PROP_MAIN_C},
    ldflags=["-Wl,-zibt"],
    # Joined `-z<keyword>` inside the group: identical to the split form.
    expect_prop_note_match=True,
    tags=("propnote",))

case("prop_note_isa_level_created",
    {"a.c": "int helper(void){ return 41; }",
     "main.c": """
#include <stdio.h>
extern int helper(void);
int main(void){ printf("prop-ok %d\\\\n", helper()); return 0; }
"""},
    ldflags=["-Wl,-z,x86-64-v3"],
    # No input carries a note; `-z x86-64-v3` creates ISA_1_NEEDED=V3.
    expect_prop_note_match=True,
    tags=("propnote",))

case("prop_note_z_lam_u48_sets_both_bits",
    {"a.c": "int helper(void){ return 41; }",
     "main.c": """
#include <stdio.h>
extern int helper(void);
int main(void){ printf("prop-ok %d\\\\n", helper()); return 0; }
"""},
    ldflags=["-Wl,-z,lam-u48"],
    # No input carries a note; `-z lam-u48` creates FEATURE_1_AND with BOTH
    # the LAM_U48 and LAM_U57 bits (0xC), not just U48 — the grouping GNU ld
    # applies (measured on ld 2.44: `-z lam-u48` reports LAM_U48,LAM_U57).
    expect_prop_note_match=True,
    tags=("propnote",))

def _prop_note_strings_asm(sym, text, entry):
    """`_prop_note_asm` plus a local SHF_MERGE|SHF_STRINGS string that the
    object's own code references (a merge section holding a global symbol
    is not pooled); `entry` also makes the code a raw `_start` (the link
    is -nostdlib)."""
    body = _prop_note_asm(sym, [(0xC0000002, 0x3)])
    lines = [
        '    .section .rodata.str1.1,"aMS",@progbits,1',
        ".Lstr:",
        f'    .asciz "{text}"',
        "    .text",
    ]
    if entry:
        lines += ["    .globl _start", "_start:"]
    else:
        lines += [f"    .globl {sym}_use", f"{sym}_use:"]
    lines += ["    leaq .Lstr(%rip), %rsi"]
    lines += (["    movl $60, %eax", "    xorl %edi, %edi", "    syscall"]
              if entry else ["    ret"])
    return body + "\n".join(lines) + "\n"

case("prop_note_synthetic_objects_do_not_veto",
    {"a.s": _prop_note_strings_asm("note_a", "shared-tail", True),
     "b.s": _prop_note_strings_asm("note_b", "tail", False)},
    ldflags=["-nostdlib", "-static", "-Wl,--build-id"],
    # Every real input carries IBT|SHSTK, so the output keeps both.  The
    # string pool the linker synthesizes for the SHF_MERGE sections (and
    # the build-id note object) has no property note and must not take
    # part in the AND merge: the opt-in exclusion set used to miss the
    # string pool, which vetoed FEATURE_1_AND on every link that merged a
    # string — invisible on Debian, whose crti.o carries no note anyway,
    # and a 6-test CI failure on Ubuntu, whose crt files and gcc default
    # (-fcf-protection) carry the note everywhere.
    expect_prop_note_match=True,
    tags=("propnote",))

case("prop_note_multi_note_section",
    {"a.s": _prop_note_asm("note_a",
                           [(0xC0000002, 0x3), (0xC0008002, 0x1)],
                           [(0xC0008002, 0x4), (0xC0010002, 0x0),
                            (0xB0008000, 0x2), (0x1, 0x1000, 8)],
                           entry=True),
     "b.s": _prop_note_asm("note_b",
                           [(0xC0000002, 0x1), (0xC0010002, 0x0),
                            (0xC0028000, 0x7), (0x2, 0, 0), (0x1, 0x4000, 8)])},
    ldflags=["-nostdlib", "-static"],
    # Every note of an input's section counts, not just the first: a.o's
    # second note carries ISA_1_NEEDED=4 (ORed with the first note's 1
    # inside the object), ISA_1_USED=0 and a generic GNU_PROPERTY_1_NEEDED
    # bit.  The all-zero OR-AND ISA_1_USED present in every input survives
    # (bfd removes empty AND/OR bitmasks only); the generic OR type, the
    # zero-sized NO_COPY_ON_PROTECTED marker and STACK_SIZE (maximum)
    # survive from whichever input has them; the unknown 0xc0028000 is
    # dropped at parse time instead of vetoing or surviving.  lccc used to
    # read only the first note, so gas 2.47's appended used-note made every
    # hand-noted object veto FEATURE_2_USED / ISA_1_USED.
    expect_prop_note_match=True,
    tags=("propnote",))

case("prop_note_invalid_isa_rejected",
    {"main.c": _PROP_MAIN_C},
    ldflags=["-Wl,-z,x86-64-v9"],
    # GNU fatals with `invalid x86-64 ISA level`; so must we.
    expect_fail=True,
    tags=("propnote",))

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
        r = sh([compiler_for(fname), "-c", fname, "-o", obj] + flags, cwd=td)
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

# field kind -> (assembly directive, symbol, in-range value, out-of-range value)
# (relocation, directive, symbol, a value that fits, one that does not).
# `.long sym` is always R_X86_64_32 to GAS, so R_X86_64_32S needs an explicit
# `.reloc`; its bad value 0x80000000 fits an R_X86_64_32 and is exactly the
# one the signed field must refuse.  The good value of R_X86_64_32 is the
# largest one only the unsigned field holds.
_FIELD_CASES = [
    ("R_X86_64_32", ".long {sym}", "far32", 0xFFFF_FFFF, 0x1_0000_0000),
    ("R_X86_64_32S", ".reloc ., R_X86_64_32S, {sym}\n        .long 0", "far32s",
     0x7FFF_FFFF, 0x8000_0000),
    ("R_X86_64_16", ".word {sym}", "far16", 0x1000, 0x1_0000),
    ("R_X86_64_8", ".byte {sym}", "far8", 0x40, 0x100),
]


def _reloc_range_fixture(td, directive, sym):
    """Assemble one object whose single relocation is `directive sym`.

    Returns the object path, or None if the toolchain refused (reported as SKIP
    by the caller rather than as a failure: a missing assembler feature is not a
    linker defect).
    """
    src = os.path.join(td, "f.s")
    # `directive` is a template over `{sym}`: `.long sym - .` is the
    # PC-relative spelling; a plain `.long sym` is R_X86_64_32 whatever the
    # section, so the signed field is requested with `.reloc`.
    with open(src, "w") as f:
        f.write(
            "        .section .data.rel.ro,\"aw\"\n"
            "        .globl slot\n"
            f"slot:   {directive.format(sym=sym)}\n"
            "        .text\n"
            "        .globl probe\n"
            "probe:  ret\n"
        )
    obj = os.path.join(td, "f.o")
    r = sh([CC, "-c", src, "-o", obj], cwd=td)
    if r.returncode != 0 or not os.path.exists(obj):
        return None
    return obj


_SCRIPT_TOKEN_RE = re.compile(rb"linker script|\bt\.ld\b", re.I)


def _oracle_rejected_the_script(oerr: bytes) -> bool:
    """Did this oracle fail on the FIXTURE rather than on the relocation?

    The minimal `-T` script this suite hands the oracles is deliberately
    tiny, and a linker that cannot parse it never reaches the relocation at
    all: the question was never put to it, so it has no opinion.  That is a
    missing FEATURE, not a missing opinion, and the two must not share a
    class -- see `reloc_oracle_agreement`, whose quorum is taken over the
    oracles that could have answered.

    Without this split, merely *installing* a linker without this script
    syntax turns a conformance test red while proving nothing about lccc,
    and the fix ("just accept the narrower quorum") is to delete the test.
    Scoped to `label == "script"` deliberately: on the other paths there is
    no script, so a mention of one would be a red herring.
    """
    return bool(_SCRIPT_TOKEN_RE.search(oerr))


def reloc_oracle_verdict(oerr: bytes, orc: int, kind: str, label: str) -> str:
    """How one oracle linker's answer counts for an out-of-range relocation.

    This is the cross-check's whole vocabulary, so it lives at module scope
    (and is unit-tested by `test_reloc_oracle_verdict.py`) instead of being
    buried as a closure inside the test body: a classification that decides
    PASS/FAIL must be checkable without building a fixture and running three
    linkers.

    * ``accepted``     -- it linked the input.  A real disagreement: lccc
                          refuses what the ecosystem accepts.
    * ``refused``      -- it refused AND named a range failure for this
                          relocation type.  Agreement.
    * ``silent``       -- it refused without naming the type or a range.
                          Counted as a disagreement, so an unrecognised
                          refusal can never be mistaken for conformity.
    * ``incapable``  -- it could not process the INPUT SHAPE: a linker
                          without this script syntax rejects the fixture
                          before reaching any relocation.  A missing
                          FEATURE, so it is excluded from the quorum --
                          which is then taken over the oracles that could
                          have answered -- and the result says so.
    * ``inapplicable`` -- it REACHED the relocation and its answer is that
                          this relocation type does not apply here.  It
                          could have disagreed, so it still counts against
                          the quorum.
    * ``errored``      -- it DIED rather than answered: an exit status
                          outside {0, 1}, i.e. a crash, an abort, or the OOM
                          killer.  Never excluded, and always a
                          DISAGREEMENT.  A crashed oracle is a MISSING
                          result, not a neutral one -- and `inapplicable` is
                          inferred from the absence of a substring in
                          stderr, so without this class a segfaulting mold is
                          indistinguishable from a mold that legitimately
                          skipped the relocation, and a red test turns
                          green.

    `errored` is classified FIRST, before the `shared` shortcut: that
    shortcut accepts any refusal on the shared path (GNU ld words its
    R_X86_64_32 refusal its own way), and a segfault is not a refusal.

    The WORDING is deliberately not part of the contract: bfd says
    "relocation truncated to fit", mold and lld say "out of range", and the
    psABI mandates neither.  Demanding bfd's spelling from every oracle
    meant that merely installing mold turned a conforming lccc into "lccc is
    inventing a restriction", with no diagnostics printed.
    """
    if orc not in (0, 1):
        return "errored"
    if orc == 0:
        return "accepted"
    if label == "shared":
        # GNU ld refuses every R_X86_64_32 in a shared object with wording
        # of its own, so any refusal counts on this path.
        return "refused"
    if f"R_X86_64_{kind}".encode() not in oerr:
        if label == "script" and _oracle_rejected_the_script(oerr):
            return "incapable"
        return "inapplicable"
    if b"truncated" in oerr or b"out of range" in oerr:
        return "refused"
    return "silent"


def reloc_oracle_agreement(agree, notes, oracles):
    """Fold per-oracle verdicts into a single `(status, detail)` for the test.

    Lives at module scope and is unit-tested (`test_reloc_oracle_verdict.py`)
    because this is where the cross-check can silently weaken: excluding an
    oracle from the agreement set is a decision about how much evidence a
    PASS rests on, and a rule that only ever runs against three real linkers
    on a well-stocked host is a rule nobody ever sees fail.

    Three ways to fail, all fail-closed:

    * **Applicability floor.** `inapplicable` shrinks the agreement set, so
      it needs a floor.  With the normal three-oracle set, two oracles that
      fail to reach the relocation would otherwise leave ONE linker deciding
      the whole test -- and a "cross-check" resting on a single opinion is
      not a cross-check.  The floor is `min(2, len(oracles))`, not a flat 2,
      so a host with a single linker installed still gets the (weaker, but
      real) conformance check instead of a spurious failure.
    * **The reference must have an opinion.** `oracles[0]` is always bfd,
      the psABI reference.  If the reference never reached the relocation the
      fixture did not exercise what the test claims, and no amount of
      agreement from the others repairs that.
    * **Any disagreement fails.** `errored` is never excluded, so a crashed
      oracle lands here rather than being dropped as "no opinion".

    A PASS that rested on fewer oracles than were configured says so in the
    detail: a cross-check that quietly narrowed is exactly how a gate rots.
    """
    # An empty oracle set must FAIL, never PASS. With `oracles == []` the
    # floor below degenerates to `min(2, 0) == 0`, `reference` is `None`, and
    # `all([])` is `True` -- every guard is satisfied and the function returns
    # PASS having consulted nobody. A gate whose entire purpose is to refuse
    # verdicts that rest on too little evidence must not be able to certify
    # a verdict resting on none. Unreachable from today's only call site
    # (which always seeds bfd), but this is module scope, it is unit-tested,
    # and a future caller that filters the oracle list would inherit a
    # silent fail-open.
    if not oracles:
        return ("FAIL", "no oracles configured to cross-check against")
    # `incapable` is a missing FEATURE, proved by a structural probe, not a
    # missing opinion: such an oracle could never have reached the relocation
    # on this input shape whatever the fixture said, so it is excluded from
    # the opinion set -- and the floor is taken over the oracles that COULD
    # have an opinion.  That keeps the cross-check meaningful instead of
    # failing a test because one installed linker lacks a script feature,
    # while still refusing to let the set collapse to a single opinion.
    # `inapplicable` is deliberately NOT treated the same way: that oracle
    # ran and reached the relocation, it simply had nothing to say here, so
    # it still counts against the quorum.
    incapable = [n for n, (v, _, _) in notes.items() if v == "incapable"]
    inapplicable = [n for n, (v, _, _) in notes.items() if v == "inapplicable"]
    applicable = [n for n, (v, _, _) in notes.items()
                  if v not in ("inapplicable", "incapable")]
    n_capable = len(oracles) - len(incapable)
    floor = min(2, n_capable)
    reference = oracles[0][0] if oracles else None
    if len(applicable) < floor:
        return ("FAIL",
                "only %d of %d oracles could express an opinion, need %d: %s"
                % (len(applicable), n_capable, floor, notes))
    if reference is not None and reference not in applicable:
        return ("FAIL",
                "the reference oracle %r never reached the relocation, so the "
                "fixture was not exercised: %s" % (reference, notes))
    if not all(ok for _, ok in agree):
        return ("FAIL",
                "oracles disagree with the refusal: %s; diagnostics: %s"
                % (agree, notes))
    detail = ""
    if len(applicable) < len(oracles):
        parts = []
        for label, names in (("inapplicable", inapplicable),
                             ("not counted (incapable)", incapable)):
            if names:
                parts.append("%s: %s" % (
                    label, ", ".join("%s(%s)" % (n, notes[n][0])
                                     for n in sorted(names))))
        detail = ("agreed by %d of %d oracles; %s"
                  % (len(applicable), len(oracles), "; ".join(parts)))
    return ("PASS", detail)


def _kinds_in(obj):
    out = sh(["readelf", "-rW", obj]).stdout.decode()
    return {m for m in re.findall(r"R_X86_64_\w+", out)}


def _reloc_field_range_tests(args, oracles):
    """Diagnose a relocation value that does not fit its field, on every path."""
    results = []
    lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    if not os.path.exists(lccc_ld):
        return [Result("reloc_field_range", "SKIP", "lccc-ld not built")]

    # ── 1. a link-time-constant field that overflows, on all three paths ──
    # Plain (emit_exec) and -T (emit_script) links resolve a PC32 against a
    # far absolute symbol statically, so the overflow is theirs to report.
    # A -shared link does not: the image moves and the symbol does not, so
    # GNU ld hands a PC32 in writable storage to ld.so as a dynamic
    # R_X86_64_PC32 (checked below).  Its link-time-constant counterpart is
    # an R_X86_64_32 against a HIDDEN absolute symbol -- the one narrow
    # absolute field a shared object may hold (ld.so cannot rebind it) --
    # so that is the fixture the shared path must diagnose.  GNU ld refuses
    # every R_X86_64_32 in a shared object, so its refusal is required but
    # not its wording.  Before the fix only the first two paths diagnosed.
    td = tempfile.mkdtemp(prefix="lnk.reloc_pc32.")
    try:
        with open(os.path.join(td, "p.s"), "w") as f:
            f.write(
                "        .section .data.rel.ro,\"aw\"\n"
                "        .globl slot\n"
                "slot:   .long farpc - .\n"      # R_X86_64_PC32
                "        .text\n"
                "        .globl probe\n"
                "probe:  ret\n"
            )
        with open(os.path.join(td, "q.s"), "w") as f:
            f.write(
                "        .section .data.rel.ro,\"aw\"\n"
                "        .globl slot\n"
                "        .hidden farpc\n"
                "slot:   .long farpc\n"          # R_X86_64_32
            )
        with open(os.path.join(td, "x.s"), "w") as f:
            f.write(
                "        .section .data.rel.ro,\"aw\"\n"
                "        .globl slot\n"
                "slot:   .long farpc\n"          # R_X86_64_32, exported target
            )
        if sh([CC, "-c", "p.s", "-o", "p.o"], cwd=td).returncode != 0 \
                or sh([CC, "-c", "q.s", "-o", "q.o"], cwd=td).returncode != 0 \
                or sh([CC, "-c", "x.s", "-o", "x.o"], cwd=td).returncode != 0:
            results.append(Result("reloc_pc32_fixture", "SKIP", "assembler refused"))
        elif "R_X86_64_PC32" not in _kinds_in(os.path.join(td, "p.o")):
            results.append(Result("reloc_pc32_fixture", "SKIP",
                                  "fixture did not produce R_X86_64_PC32"))
        else:
            with open(os.path.join(td, "t.ld"), "w") as f:
                f.write("ENTRY(probe)\nSECTIONS {\n  . = 0x400000;\n"
                        "  .text : { *(.text) }\n"
                        "  .data.rel.ro : { *(.data.rel.ro) }\n}\n")
            far = "0x7fff00000000"     # outside +/- 2 GiB of any image address
            paths = [
                ("exec", "PC32", [lccc_ld, "--defsym", f"farpc={far}", "p.o",
                                  "-o", "o.exe", "--no-dynamic-linker"]),
                ("script", "PC32", [lccc_ld, "-T", "t.ld", "--defsym", f"farpc={far}",
                                    "p.o", "-o", "o.script", "--no-dynamic-linker"]),
                ("shared", "32", [lccc_ld, "-shared", "--defsym", f"farpc={far}",
                                  "q.o", "-o", "o.so"]),
            ]
            for label, kind, cmd in paths:
                name = f"reloc_{kind.lower()}_out_of_range_diagnosed_on_{label}_path"
                r = sh(cmd, cwd=td)
                if r.returncode == 0:
                    results.append(Result(
                        name, "FAIL",
                        f"{label} path accepted an out-of-range R_X86_64_{kind} and "
                        f"emitted {cmd[cmd.index('-o') + 1]}: the value would be "
                        f"silently truncated"))
                    continue
                err = r.stderr.decode()
                if f"R_X86_64_{kind} " not in err or "truncated" not in err:
                    results.append(Result(
                        name, "FAIL",
                        f"{label} path failed but did not name the type and the "
                        f"truncation: {err[:200]!r}"))
                    continue
                # The oracle must refuse the same input, or this is lccc
                # inventing a restriction rather than conforming.
                # (Linker options go through -Wl: the driver itself
                # rejects `--defsym`, which once made every oracle "agree"
                # vacuously.)  The oracle must also name the truncation.
                agree = []
                # Oracle answers are classified by `reloc_oracle_verdict`
                # (module scope, unit-tested by
                # `test_reloc_oracle_verdict.py`): the classification
                # decides PASS/FAIL, so it is a named function rather
                # than a closure buried in this loop.
                def _oracle_verdict(oerr: bytes, orc: int, kind: str) -> str:
                    return reloc_oracle_verdict(oerr, orc, kind, label)

                notes = {}
                defsym = f"-Wl,--defsym,farpc={far}"
                for oname, ocmd in oracles:
                    if label == "script":
                        o = sh(ocmd + ["-nostdlib", "-Wl,-T,t.ld", defsym, "p.o",
                                       "-o", f"o.{oname}", "-Wl,--no-dynamic-linker"], cwd=td)
                    elif label == "shared":
                        o = sh(ocmd + ["-nostdlib", "-shared", defsym, "q.o",
                                       "-o", f"o.{oname}.so"], cwd=td)
                    else:
                        o = sh(ocmd + ["-nostdlib", "-static", defsym, "-Wl,-e,probe", "p.o",
                                       "-o", f"o.{oname}"], cwd=td)
                    verdict = _oracle_verdict(o.stderr, o.returncode, kind)
                    notes[oname] = (verdict, o.returncode, o.stderr.decode()[:120])
                    # `inapplicable` AND `incapable` are both "no opinion":
                    # the first ran and had nothing to say about this
                    # relocation, the second never reached one.  Either way
                    # there is nothing to agree or disagree WITH, so neither
                    # may enter `agree`.  Letting `incapable` through here
                    # appended `(name, False)` and turned "this linker
                    # cannot parse the fixture" into a DISAGREEMENT --
                    # the precise inversion the class exists to prevent.
                    if verdict not in ("inapplicable", "incapable"):
                        agree.append((oname, verdict == "refused"))
                status, detail = reloc_oracle_agreement(agree, notes, oracles)
                results.append(Result(name, status, detail))
            # An R_X86_64_32 against an EXPORTED absolute symbol is refused
            # whatever its value: ld.so may bind the name elsewhere.
            name = "reloc_32_preemptible_abs_in_shared_refused"
            r = sh([lccc_ld, "-shared", "--defsym", "farpc=0x1000", "x.o", "-o", "x.so"],
                   cwd=td)
            err = r.stderr.decode()
            results.append(
                Result(name, "PASS") if r.returncode != 0
                and "R_X86_64_32 against 'farpc' can not be used when making a shared object" in err
                else Result(name, "FAIL", f"rc={r.returncode} {err[:200]!r}"))
            # The shared PC32 itself: a dynamic R_X86_64_PC32 against
            # `farpc` (exported, so named by index), exactly like GNU ld.
            # Only GNU ld is the reference here: the psABI leaves dynamic
            # PC32 optional, glibc implements it, and lld refuses the link.
            name = "reloc_pc32_abs_in_shared_is_dynamic"
            r = sh([lccc_ld, "-shared", "--defsym", f"farpc={far}", "p.o", "-o", "d.so"],
                   cwd=td)
            dyn = sh(["readelf", "-W", "-r", "d.so"], cwd=td).stdout.decode() \
                if r.returncode == 0 else ""
            want = re.compile(r"R_X86_64_PC32\s+0*7fff00000000\s+farpc \+ 0")
            if r.returncode != 0:
                results.append(Result(name, "FAIL", f"refused: {r.stderr.decode()[:200]!r}"))
            elif not want.search(dyn):
                results.append(Result(name, "FAIL", f"no dynamic PC32 against farpc: {dyn!r}"))
            else:
                bad = []
                for oname, ocmd in oracles[:1]:
                    o = sh(ocmd + ["-nostdlib", "-shared", f"-Wl,--defsym,farpc={far}", "p.o",
                                   "-o", f"d.{oname}.so"], cwd=td)
                    odyn = sh(["readelf", "-W", "-r", f"d.{oname}.so"], cwd=td).stdout.decode()
                    if o.returncode != 0 or not want.search(odyn):
                        bad.append(oname)
                results.append(Result(name, "FAIL", f"oracles differ: {bad}") if bad
                               else Result(name, "PASS"))
    except Exception as e:
        results.append(Result("reloc_pc32_out_of_range", "FAIL", f"harness: {e!r}"))
    finally:
        shutil.rmtree(td, ignore_errors=True)

    # ── 2. every absolute field width, both verdicts ───────────────────────
    for kind, directive, sym, good, bad in _FIELD_CASES:
        td = tempfile.mkdtemp(prefix=f"lnk.reloc_{sym}.")
        try:
            obj = _reloc_range_fixture(td, directive, sym)
            if obj is None:
                results.append(Result(f"reloc_{sym}_fixture", "SKIP",
                                      "assembler refused the fixture"))
                continue
            kinds = _kinds_in(obj)
            if kinds != {kind}:
                results.append(Result(f"reloc_{sym}_fixture", "FAIL",
                                      f"fixture produced {sorted(kinds)}, not {kind}"))
                continue
            # out of range: must be diagnosed, and the oracle must agree
            name = f"reloc_out_of_range_diagnosed_{kind.lower()}"
            r = sh([lccc_ld, "--defsym", f"{sym}={bad:#x}", os.path.basename(obj),
                    "-o", "bad.exe", "--no-dynamic-linker"], cwd=td)
            if r.returncode == 0:
                results.append(Result(
                    name, "FAIL",
                    f"{sorted(kinds)} value {bad:#x} was accepted and truncated"))
            else:
                err = r.stderr.decode()
                if "truncated" not in err and "out of range" not in err:
                    results.append(Result(name, "FAIL",
                                          f"refused without a range diagnostic: "
                                          f"{err[:200]!r}"))
                else:
                    o = sh([CC, "-fuse-ld=bfd", "--defsym", f"{sym}={bad:#x}",
                            os.path.basename(obj), "-o", "bad.bfd",
                            "-nostdlib", "--no-dynamic-linker"], cwd=td)
                    if o.returncode == 0:
                        results.append(Result(
                            name, "FAIL",
                            f"bfd accepted {sorted(kinds)}={bad:#x} but lccc "
                            f"refused: lccc is stricter than the ecosystem"))
                    else:
                        results.append(Result(name, "PASS"))

            # in range: must link, and the field must hold the value
            name = f"reloc_in_range_accepted_{kind.lower()}"
            r = sh([lccc_ld, "--defsym", f"{sym}={good:#x}", os.path.basename(obj),
                    "-o", "good.exe", "--no-dynamic-linker"], cwd=td)
            if r.returncode != 0:
                results.append(Result(
                    name, "FAIL",
                    f"{sorted(kinds)}={good:#x} fits its field but was refused: "
                    f"{r.stderr.decode()[:200]!r}"))
            else:
                dump = sh(["readelf", "-x", ".data.rel.ro", "good.exe"],
                          cwd=td).stdout.decode()
                want = "".join(f"{(good >> (8 * i)) & 0xff:02x}" for i in range(4))
                width = {"R_X86_64_8": 1, "R_X86_64_16": 2}.get(kind, 4)
                want = want[:width * 2]
                if want not in dump.replace(" ", ""):
                    results.append(Result(
                        name, "FAIL",
                        f"linked but the field does not hold {good:#x} "
                        f"(expected {want} in .data.rel.ro)"))
                else:
                    results.append(Result(name, "PASS"))
        except Exception as e:
            results.append(Result(f"reloc_{sym}", "FAIL", f"harness: {e!r}"))
        finally:
            shutil.rmtree(td, ignore_errors=True)

    # ── 3. a 64-bit field must still accept values above 4 GiB ─────────────
    # The regression risk of adding range checks is rejecting what the ABI
    # allows. This runs the binary, so it also proves the value survived.
    td = tempfile.mkdtemp(prefix="lnk.reloc_64.")
    try:
        with open(os.path.join(td, "w.s"), "w") as f:
            f.write("        .section .data.rel.ro,\"aw\"\n"
                    "        .globl slot64\n"
                    "slot64: .quad far64\n")
        with open(os.path.join(td, "m.c"), "w") as f:
            f.write("#include <stdio.h>\n#include <stdint.h>\n"
                    "extern uint64_t slot64;\n"
                    "int main(void){ printf(\"%llx\\n\", "
                    "(unsigned long long)slot64); return 0; }\n")
        ok = (sh([CC, "-c", "w.s", "-o", "w.o"], cwd=td).returncode == 0
              and sh([CC, "-c", "-O1", "m.c", "-o", "m.o"], cwd=td).returncode == 0)
        if not ok:
            results.append(Result("reloc_64_accepts_above_4g", "SKIP",
                                  "assembler refused the fixture"))
        else:
            big = "0x100000000"        # 4 GiB: legal in a 64-bit field
            # args.lccc is the *driver*: linker flags go through `-Wl,`
            # (bare `--defsym` is a linker spelling; even GNU gcc rejects it
            # at driver level).  The linked value is what we actually test.
            r = sh([args.lccc, f"-Wl,--defsym,far64={big}", "m.o", "w.o",
                    "-o", "w.bin", "-Wl,--no-dynamic-linker"] , cwd=td)
            if r.returncode != 0:
                # fall back to lccc-ld with the C runtime pieces it needs
                r = sh([CC, "-fuse-ld=bfd", "-c", "m.c", "-o", "m2.o"], cwd=td)
                results.append(Result(
                    "reloc_64_accepts_above_4g", "SKIP",
                    f"driver link unavailable: {r.stderr.decode()[:120]!r}"))
            else:
                code, out = run_bin(os.path.join(td, "w.bin"), [], td)
                # The fixture prints with %llx, so compare in hex.
                want_hex = f"{int(big, 16):x}"
                if (code, out.strip()) != (0, want_hex):
                    results.append(Result(
                        "reloc_64_accepts_above_4g", "FAIL",
                        f"64-bit field lost its value: {(code, out)!r} "
                        f"(expected {want_hex!r})"))
                else:
                    results.append(Result("reloc_64_accepts_above_4g", "PASS"))
    except Exception as e:
        results.append(Result("reloc_64_accepts_above_4g", "FAIL", f"harness: {e!r}"))
    finally:
        shutil.rmtree(td, ignore_errors=True)

    return results

def _elf64_sections(d):
    """Section headers of a little-endian ELF64 image, with names resolved."""
    shoff = int.from_bytes(d[0x28:0x30], "little")
    shentsize = int.from_bytes(d[0x3A:0x3C], "little")
    shnum = int.from_bytes(d[0x3C:0x3E], "little")
    shstrndx = int.from_bytes(d[0x3E:0x40], "little")
    secs = []
    for i in range(shnum):
        o = shoff + i * shentsize
        secs.append(dict(
            nameoff=int.from_bytes(d[o:o + 4], "little"),
            off=int.from_bytes(d[o + 0x18:o + 0x20], "little"),
            size=int.from_bytes(d[o + 0x20:o + 0x28], "little"),
            entsize=int.from_bytes(d[o + 0x38:o + 0x40], "little")))
    stro = secs[shstrndx]["off"]
    for sec in secs:
        end = d.index(b"\0", stro + sec["nameoff"])
        sec["name"] = bytes(d[stro + sec["nameoff"]:end]).decode()
    return secs


def _reloc_offset_tests(args, oracles):
    """r_offset comes from the input object, so it decides where we write.

    Two failures were measured before this was validated, both with exit status 0:

      * r_offset = 2^64 - 16 made the u64 sum wrap; the unchecked writer no-oped
        and the image kept the unrelocated instruction -- `lea` loaded the address
        of the next instruction instead of the symbol's;
      * r_offset = section size wrote four bytes into whatever the layout had put
        after that section.

    GNU ld refuses both (bfd_reloc_outofrange, reported as "error 4"), and a sweep
    of r_offset across the boundary established that its rule is not
    `offset < size` but `offset + field width <= size`: with a 9-byte .text and a
    4-byte field, 5 is accepted and 6 is refused. So this sweeps two field widths
    and requires the accept boundary to move with the width -- a check that
    ignored the width would pass one sweep and fail the other.
    """
    results = []
    lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    if not os.path.exists(lccc_ld):
        return [Result("reloc_offset", "SKIP", "lccc-ld not built")]

    src = textwrap.dedent("""
        \t.text
        \t.globl _start
        _start:
        \tleaq\ttgt(%rip), %rax
        \tret
        \t.globl tgt
        tgt:
        \tret
        \t.section .data.rel.ro,"aw"
        \t.quad\ttgt
    """).lstrip()

    for sec, width in ((".text", 4), (".data.rel.ro", 8)):
        name = f"reloc_offset_{sec.strip('.')}_w{width}"
        td = tempfile.mkdtemp()
        try:
            spath = os.path.join(td, "f.s")
            with open(spath, "w") as f:
                f.write(src)
            opath = os.path.join(td, "f.o")
            r = sh([CC, "-c", spath, "-o", opath], cwd=td)
            if r.returncode != 0:
                results.append(Result(name, "SKIP", f"assembler: {r.stderr.decode()[:100]!r}"))
                continue
            base = bytearray(open(opath, "rb").read())
            secs = _elf64_sections(base)
            rela = next(s for s in secs if s["name"] == ".rela" + sec)
            target = next(s for s in secs if s["name"] == sec)
            size = target["size"]

            diverge = []
            last_accept = None
            for off in range(0, size + 4):
                v = bytearray(base)
                v[rela["off"]:rela["off"] + 8] = off.to_bytes(8, "little")
                obj = os.path.join(td, f"s{off}.o")
                open(obj, "wb").write(bytes(v))
                g = sh(["ld", obj, "-o", os.path.join(td, "g.elf"), "-nostdlib"], cwd=td)
                l = sh([lccc_ld, "-nostdlib", obj, "-o", os.path.join(td, "l.elf")], cwd=td)
                gv, lv = g.returncode == 0, l.returncode == 0
                if gv != lv:
                    diverge.append((off, "ld accepts/lccc refuses" if gv else
                                    "ld refuses/lccc ACCEPTS",
                                    l.stderr.decode()[:110]))
                if gv:
                    last_accept = off

            # The boundary itself is the assertion: the highest offset GNU ld
            # accepts must be exactly size - width, which is where a
            # width-unaware check would go wrong.
            want = size - width
            if diverge:
                results.append(Result(name, "FAIL",
                    f"{len(diverge)} divergence(s) vs GNU ld over offsets 0..{size+3}: "
                    + "; ".join(f"0x{o:x} {why} ({msg!r})" for o, why, msg in diverge[:3])))
            elif last_accept != want:
                results.append(Result(name, "FAIL",
                    f"GNU ld's last accepted offset was 0x{last_accept:x}, expected "
                    f"0x{want:x} (size 0x{size:x} - width {width})"))
            else:
                results.append(Result(name, "PASS",
                    f"{size+4} offsets swept, boundary 0x{want:x} matches GNU ld"))

            # The value that wrapped: not merely refused, but refused for the
            # offset reason rather than something incidental.
            v = bytearray(base)
            v[rela["off"]:rela["off"] + 8] = (2**64 - 16).to_bytes(8, "little")
            obj = os.path.join(td, "wrap.o")
            open(obj, "wb").write(bytes(v))
            g = sh(["ld", obj, "-o", os.path.join(td, "g2.elf"), "-nostdlib"], cwd=td)
            l = sh([lccc_ld, "-nostdlib", obj, "-o", os.path.join(td, "l2.elf")], cwd=td)
            wname = f"reloc_offset_wrap_{sec.strip('.')}"
            if g.returncode == 0:
                results.append(Result(wname, "SKIP", "GNU ld accepted a wrapping r_offset"))
            elif l.returncode == 0:
                results.append(Result(wname, "FAIL",
                    "lccc linked an object whose r_offset wraps the u64 sum; the image "
                    "is missing a patch or patched at the wrong place"))
            elif "offset" not in l.stderr.decode() and "outside" not in l.stderr.decode():
                results.append(Result(wname, "FAIL",
                    f"refused, but not for an offset reason: {l.stderr.decode()[:120]!r}"))
            else:
                results.append(Result(wname, "PASS"))
        except Exception as e:
            results.append(Result(name, "FAIL", f"harness: {e!r}"))
        finally:
            shutil.rmtree(td, ignore_errors=True)
    return results


def _sym_value(binary, name):
    """Value of the first symtab entry called `name`, or None."""
    out = sh(["readelf", "-sW", binary]).stdout.decode()
    for line in out.splitlines():
        m = re.match(r"\s*\d+: ([0-9a-f]+)\s+\d+\s+\S+\s+\S+\s+\S+\s+\S+\s+" +
                     re.escape(name) + r"\s*$", line)
        if m:
            return int(m.group(1), 16)
    return None


def _defsym_layout_test(args, oracles):
    """`--defsym` expressions must see FINAL addresses, not section-relative ones.

    Regression: the expression was evaluated before layout, when every
    section-resident symbol still carried its in-object offset.  With
    `start_val` at offset 0 of its section, `--defsym X=start_val+8` produced
    the absolute value 8 -- a plausible low address, wrong by the entire image
    base.  GNU ld evaluates the same expression against layout addresses.

    The check is differential on the DELTA between the defsym'd symbol and its
    referent (layout bases legitimately differ between linkers, deltas do not),
    plus a direct byte-level check that a relocation REFERENCING the defsym'd
    symbol is patched with the final value.
    """
    results = []
    lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    if not os.path.exists(lccc_ld):
        return [Result("defsym_layout_expr", "SKIP", "lccc-ld not built")]

    td = tempfile.mkdtemp(prefix="lnk.defsym_layout.")
    try:
        with open(os.path.join(td, "d.c"), "w") as f:
            f.write("int start_val = 0;\nint other = 1;\n")
        with open(os.path.join(td, "ref.s"), "w") as f:
            f.write(
                "        .section .data\n"
                "        .globl slot\n"
                "slot:   .long X\n"     # R_X86_64_32 against the defsym'd symbol
            )
        if (sh([CC, "-c", "d.c", "-o", "d.o"], cwd=td).returncode != 0 or
                sh([CC, "-c", "ref.s", "-o", "ref.o"], cwd=td).returncode != 0):
            return [Result("defsym_layout_expr", "SKIP", "fixture compile failed")]

        bfd = "ld"
        # (name, defsym expr, referent, expected delta or None).
        #
        # The delta (sym - referent) is the assertion because the two images
        # are linked at different bases; it is base-invariant for correct
        # post-layout evaluation and COLLAPSES to `constant - base` under the
        # old pre-layout evaluation, which cannot be confused with the right
        # answer.  A hardcoded delta is only safe where it follows from the
        # expression itself (see `want`); elsewhere lccc must agree with
        # GNU ld, which evaluates these expressions against final values.
        #
        # `_end` is deliberately NOT in this table: GNU ld evaluates it at a
        # mid-layout stage, so its E is _not_ final `_end` - 4 (measured:
        # delta 8188).  lccc's semantics are the documented one -- expressions
        # see the final value -- and are checked as an internal invariant
        # below.
        expr_cases = [
            ("defsym_layout_plus",  "X=start_val+8",           "start_val", 8),
            ("defsym_layout_arith", "H=(start_val+other*2)/3", "start_val", None),
        ]
        for name, expr, referent, want in expr_cases:
            sym = expr.split("=")[0]
            # Only d.o: the delta is an identity in the referent's own value,
            # and ref.o would drag in an undefined X for every expr that does
            # not define it.  ref.o is exercised by the reloc case below.
            l = sh([lccc_ld, "--defsym", expr, "d.o",
                    "-o", "l.elf", "--no-dynamic-linker"], cwd=td)
            g = sh([bfd, "--defsym", expr, "d.o",
                    "-o", "g.elf", "--no-dynamic-linker"], cwd=td)
            if g.returncode != 0:
                results.append(Result(name, "SKIP",
                    f"GNU ld refused the fixture: {g.stderr.decode()[:120]}"))
                continue
            if l.returncode != 0:
                results.append(Result(name, "FAIL",
                    f"lccc-ld refused but GNU ld accepted: {l.stderr.decode()[:160]}"))
                continue
            lv, lv_ref = _sym_value(os.path.join(td, "l.elf"), sym), \
                         _sym_value(os.path.join(td, "l.elf"), referent)
            gv, gv_ref = _sym_value(os.path.join(td, "g.elf"), sym), \
                         _sym_value(os.path.join(td, "g.elf"), referent)
            if None in (lv, lv_ref, gv, gv_ref):
                results.append(Result(name, "FAIL",
                    f"symbol '{sym}' or '{referent}' missing from a symtab"))
                continue
            ldelta, gdelta = lv - lv_ref, gv - gv_ref
            if want is not None and gdelta != want:
                results.append(Result(name, "SKIP",
                    f"oracle sanity: GNU delta {gdelta} != {want}"))
            elif ldelta != gdelta:
                results.append(Result(name, "FAIL",
                    f"lccc delta {ldelta} != GNU delta {gdelta} for {expr}: "
                    f"lccc {sym}={lv:#x} {referent}={lv_ref:#x} (pre-layout "
                    f"evaluation would store {sym}={want if want is not None else 'a section-relative constant'}, "
                    f"collapsing the delta to constant - base)"))
            else:
                results.append(Result(name, "PASS",
                    f"delta {ldelta} matches GNU ld"))

        # lccc's documented `_end` semantics: the expression sees the FINAL
        # value, so E - _end is exactly -4 in lccc's own image.
        name = "defsym_layout_end_final"
        l = sh([lccc_ld, "--defsym", "E=_end-4", "d.o",
                "-o", "le.elf", "--no-dynamic-linker"], cwd=td)
        if l.returncode != 0:
            results.append(Result(name, "FAIL",
                f"lccc-ld refused: {l.stderr.decode()[:160]}"))
        else:
            ev, env_ = _sym_value(os.path.join(td, "le.elf"), "E"), \
                       _sym_value(os.path.join(td, "le.elf"), "_end")
            if None in (ev, env_) or ev - env_ != -4:
                results.append(Result(name, "FAIL",
                    f"E={ev and ev:#x} _end={env_ and env_:#x}, want delta -4"))
            else:
                results.append(Result(name, "PASS", f"E = final _end - 4 ({ev:#x})"))

        # A relocation that REFERENCES the defsym'd symbol must be patched
        # with the final value, not the pre-layout placeholder.
        name = "defsym_layout_reloc_reference"
        l = sh([lccc_ld, "--defsym", "X=start_val+8", "d.o", "ref.o",
                "-o", "lr.elf", "--no-dynamic-linker"], cwd=td)
        g = sh([bfd, "--defsym", "X=start_val+8", "d.o", "ref.o",
                "-o", "gr.elf", "--no-dynamic-linker"], cwd=td)
        if l.returncode != 0 or g.returncode != 0:
            results.append(Result(name, "FAIL",
                "link failed: " + (l.stderr.decode()[:120] if l.returncode else
                                    g.stderr.decode()[:120])))
        else:
            lx, gx = _sym_value(os.path.join(td, "lr.elf"), "X"), \
                     _sym_value(os.path.join(td, "gr.elf"), "X")
            # The slot's file position is where the symbol says it is:
            # (slot_vma - .data_vma) into the section dump.  Computing it from
            # the symbols (rather than assuming a layout) keeps the test valid
            # if the compiler moves `start_val` (e.g. a zero init landing in
            # .bss) and reorders the .data contents.
            def slot_bytes(binary):
                hdr = sh(["readelf", "-SW", binary]).stdout.decode()
                dm = re.search(r"\.data\s+PROGBITS\s+([0-9a-f]+)\s+([0-9a-f]+)", hdr)
                sv = _sym_value(binary, "slot")
                if not dm or sv is None:
                    return None
                off = sv - int(dm.group(1), 16)
                out = sh(["readelf", "-x", ".data", binary]).stdout.decode()
                words = []
                for line in out.splitlines():
                    m = re.match(r"\s*0x[0-9a-f]+\s+(.*)$", line)
                    if m:
                        words += re.findall(r"[0-9a-f]{8}", m.group(1))
                b = bytes.fromhex("".join(words)) if words else b""
                if off + 4 <= len(b):
                    return struct.unpack("<I", b[off:off + 4])[0]
                return None
            # The slot holds an ABSOLUTE R_X86_64_32, so each image's bytes
            # must equal that image's OWN X (the bases differ between
            # linkers; comparing across them would be a false failure).
            lb, gb = slot_bytes(os.path.join(td, "lr.elf")), slot_bytes(os.path.join(td, "gr.elf"))
            if lb is None or gb is None:
                results.append(Result(name, "FAIL", "could not read .data slot bytes"))
            elif lb != lx or gb != gx:
                results.append(Result(name, "FAIL",
                    f"slot bytes lccc {lb:#x} != lccc X {lx:#x}, or "
                    f"gnu {gb:#x} != gnu X {gx:#x}"))
            else:
                results.append(Result(name, "PASS",
                    f"relocation against defsym'd symbol patched with the "
                    f"final value (lccc {lx:#x} / gnu {gx:#x})"))

        # Same invariant on the shared-object path.
        name = "defsym_layout_shared"
        l = sh([lccc_ld, "-shared", "--defsym", "X=start_val+8", "d.o",
                "-o", "ls.so"], cwd=td)
        g = sh([bfd, "-shared", "--defsym", "X=start_val+8", "d.o",
                "-o", "gs.so"], cwd=td)
        if l.returncode != 0 or g.returncode != 0:
            results.append(Result(name, "FAIL", "shared link failed"))
        else:
            lx, lref = _sym_value(os.path.join(td, "ls.so"), "X"), \
                       _sym_value(os.path.join(td, "ls.so"), "start_val")
            gx, gref = _sym_value(os.path.join(td, "gs.so"), "X"), \
                       _sym_value(os.path.join(td, "gs.so"), "start_val")
            if None in (lx, lref, gx, gref):
                results.append(Result(name, "FAIL", "symbol missing from shared symtab"))
            elif (lx - lref) != (gx - gref):
                results.append(Result(name, "FAIL",
                    f"shared delta {lx - lref} != GNU delta {gx - gref}"))
            else:
                results.append(Result(name, "PASS"))
    finally:
        shutil.rmtree(td, ignore_errors=True)
    return results


_DEFSYM_ORDER_MAIN = r"""
#include <stdio.h>
extern char anchor[], a[], b[], c[], selfy[], selfy_orig[];
int main(void)
{
    printf("%ld %ld %lu %ld\n", (long)(a - anchor), (long)(b - anchor),
           (unsigned long)c, (long)(selfy - selfy_orig));
    return 0;
}
"""

_DEFSYM_ORDER_ASM = """\
        .text
        .globl anchor
        .type anchor, @function
anchor: nop
        nop
        nop
        nop
        nop
        nop
        nop
        nop
        .size anchor, 8
        .data
        .globl selfy, selfy_orig
        .type selfy, @object
selfy_orig:
selfy:  .quad 0
        .size selfy, 8
"""


def _write(td, name, text):
    with open(os.path.join(td, name), "w") as f:
        f.write(text)


def _defsym_order_test(args, oracles):
    """`--defsym` order semantics: forward references, redefinitions, self
    references and cycles, on every lccc-ld path, against GNU ld.

    lccc-ld applied `--defsym`s strictly left to right against the input
    symbols: `a=b b=anchor+4` failed with "undefined symbol `b'" (GNU ld
    accepts it and gives a = b = anchor+4), the reversed spelling left `a`
    at 0, and `selfy=selfy+1` over an input `selfy` yielded 1 instead of
    the input's address plus one.  The shared `DefsymPlan` binds each
    reference as GNU ld's multi-pass assignment evaluation does.

    Checked exactly:
      * symbol values relative to `anchor` in both orders, equal to GNU
        ld's, for x86-64 -pie (the builtin path), -shared and -T (the
        script path, which used to refuse any expression over a placed
        symbol);
      * i386, by running a -no-pie program linked by each: identical
        output, `4 4 4 1`;
      * an x86-64 PIE run (main built -fPIC, so it reads the symbols
        through the GOT): `a` and `b` are ADDRESSES and slide with the load
        base (R_X86_64_RELATIVE), `c = b - anchor` is the absolute 4.  GNU
        ld makes every --defsym SHN_ABS and its PIE leaves the pointers
        unslid (documented divergence; lld agrees with lccc);
      * -fPIE code reaches `c` with R_X86_64_PC32, which no PIE can
        satisfy for an absolute symbol (the image moves, 4 does not): an
        error naming the symbol.  GNU ld links it and the program reads
        base+4 (documented divergence; lld also refuses);
      * `a=b b=a` is an error naming the cycle (GNU ld silently defines
        both as 0 -- documented divergence).
    """
    names = ("defsym_forward_reference_pie", "defsym_forward_reference_shared",
             "defsym_forward_reference_script", "defsym_forward_reference_i386",
             "defsym_address_slides_in_pie", "defsym_pc32_absolute_pie_refused",
             "defsym_cycle_diagnosed")
    lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    if not os.path.exists(lccc_ld):
        return [Result(n, "SKIP", "lccc-ld not built") for n in names]
    orders = (
        ["a=b", "b=anchor+4", "c=b-anchor", "selfy=selfy+1"],
        ["c=b-anchor", "selfy=selfy+1", "b=anchor+4", "a=b"],
    )
    want = {"a": 4, "b": 4, "c": 4, "selfy": 1}
    results = []
    td = tempfile.mkdtemp(prefix="lnk.defsym_order.")
    try:
        _write(td, "d.s", _DEFSYM_ORDER_ASM)
        _write(td, "d32.s", _DEFSYM_ORDER_ASM.replace(".quad", ".long").replace(
            ".size selfy, 8", ".size selfy, 4"))
        _write(td, "m.c", _DEFSYM_ORDER_MAIN)
        _write(td, "t.ld", "SECTIONS { . = 0x400000; .text : { *(.text) } "
                           ". = 0x600000; .data : { *(.data) } }\n")
        for cmd in ([CC, "-c", "d.s", "-o", "d.o"], [CC, "-m32", "-c", "d32.s", "-o", "d32.o"],
                    [CC, "-O1", "-fPIC", "-c", "m.c", "-o", "m.o"],
                    [CC, "-O1", "-fPIE", "-c", "m.c", "-o", "mpie.o"],
                    [CC, "-m32", "-O1", "-fno-pic", "-c", "m.c", "-o", "m32.o"]):
            r = sh(cmd, cwd=td)
            if r.returncode != 0:
                return [Result(n, "FAIL", f"fixture: {' '.join(cmd)}: {r.stderr.decode()[:200]}")
                        for n in names]

        def values(path):
            syms = {}
            out = sh(["readelf", "-sW", path]).stdout.decode()
            for line in out.splitlines():
                f = line.split()
                if len(f) >= 8 and f[0][:-1].isdigit() and f[0].endswith(":"):
                    syms.setdefault(f[7], int(f[1], 16))
            return syms

        def rel(path):
            """{a, b, c, selfy} relative to anchor/selfy_orig, as `want`."""
            v = values(path)
            if not all(k in v for k in ("anchor", "a", "b", "c", "selfy", "selfy_orig")):
                return None
            return {"a": v["a"] - v["anchor"], "b": v["b"] - v["anchor"], "c": v["c"],
                    "selfy": v["selfy"] - v["selfy_orig"]}

        variants = (
            ("defsym_forward_reference_pie", ["-pie", "-e", "anchor"]),
            ("defsym_forward_reference_shared", ["-shared"]),
            ("defsym_forward_reference_script", ["-T", "t.ld", "-e", "anchor"]),
        )
        for name, mode in variants:
            problems = []
            for order in orders:
                defs = [x for d in order for x in ("--defsym", d)]
                got = {}
                for who, ld in (("lccc", lccc_ld), ("GNU", "ld")):
                    out = f"{name}.{who}.{orders.index(order)}"
                    r = sh([ld] + mode + defs + ["d.o", "-o", out], cwd=td)
                    if r.returncode != 0:
                        problems.append(f"{who} [{' '.join(order)}]: rc={r.returncode} "
                                        f"{r.stderr.decode()[:160]}")
                        continue
                    got[who] = rel(os.path.join(td, out))
                if "lccc" in got and "GNU" in got:
                    if got["GNU"] != want:
                        problems.append(f"oracle drift: GNU gives {got['GNU']}, expected {want}")
                    elif got["lccc"] != got["GNU"]:
                        problems.append(f"[{' '.join(order)}] lccc {got['lccc']} != GNU {got['GNU']}")
            results.append(Result(name, "FAIL" if problems else "PASS",
                                  "; ".join(problems)[:600]))

        shim = _shim_for(td, lccc_ld)

        def run(name, link, want_out):
            """Link with each command in `link` (label -> argv) per order,
            run, and require `want_out`."""
            problems = []
            for i, order in enumerate(orders):
                wl = ",".join(f"--defsym={d}" for d in order)
                for who, argv in link.items():
                    exe = f"{name}.{who}.{i}"
                    r = sh(argv + [f"-Wl,{wl}", "-o", exe], cwd=td)
                    if r.returncode != 0:
                        problems.append(f"{who} link [{' '.join(order)}]: "
                                        f"{r.stderr.decode()[:200]}")
                        continue
                    rr = sh([os.path.join(td, exe)], cwd=td)
                    if rr.returncode != 0 or rr.stdout != want_out:
                        problems.append(f"{who} run [{' '.join(order)}]: rc={rr.returncode} "
                                        f"out={rr.stdout!r}, want {want_out!r}")
            results.append(Result(name, "FAIL" if problems else "PASS",
                                  "; ".join(problems)[:600]))

        run("defsym_forward_reference_i386",
            {"lccc": [CC, "-m32", f"-B{shim}", "-no-pie", "m32.o", "d32.o"],
             "GNU": [CC, "-m32", "-no-pie", "m32.o", "d32.o"]},
            b"4 4 4 1\n")
        # The addresses slide, the absolute does not (lccc only; see above).
        run("defsym_address_slides_in_pie",
            {"lccc": [CC, f"-B{shim}", "-pie", "m.o", "d.o"]}, b"4 4 4 1\n")

        # PC32 against an absolute symbol in a PIE is refused, naming it.
        name = "defsym_pc32_absolute_pie_refused"
        r = sh([CC, f"-B{shim}", "-pie", "mpie.o", "d.o",
                "-Wl,--defsym=b=anchor+4,--defsym=a=b,--defsym=c=b-anchor,"
                "--defsym=selfy=selfy+1", "-o", "pc32"], cwd=td)
        msg = "relocation R_X86_64_PC32 against absolute symbol `c'"
        if r.returncode == 0 or msg not in r.stderr.decode():
            results.append(Result(name, "FAIL",
                                  f"rc={r.returncode} stderr={r.stderr.decode()[:200]!r}, "
                                  f"want an error containing {msg!r}"))
        else:
            results.append(Result(name, "PASS"))

        # A cycle is diagnosed, naming it.
        name = "defsym_cycle_diagnosed"
        r = sh([lccc_ld, "-pie", "-e", "anchor", "--defsym", "a=b", "--defsym", "b=a",
                "d.o", "-o", "cyc"], cwd=td)
        msg = "--defsym:1: circular reference: a -> b -> a"
        if r.returncode == 0 or msg not in r.stderr.decode():
            results.append(Result(name, "FAIL",
                                  f"rc={r.returncode} stderr={r.stderr.decode()[:200]!r}, "
                                  f"want an error containing {msg!r}"))
        else:
            results.append(Result(name, "PASS"))
    finally:
        shutil.rmtree(td, ignore_errors=True)
    return results


_PLTOFF_LIB = "int dvar = 42;\nint fn(void) { return 7; }\n"

_PLTOFF_ASM = """\
        .text
        .globl get_pltoff, get_pc32, call_fnoff
get_pltoff:
        leaq _GLOBAL_OFFSET_TABLE_(%rip), %rcx
        movabsq $dvar@PLTOFF, %rax
        addq %rcx, %rax
        ret
get_pc32:
        leaq dvar(%rip), %rax
        ret
call_fnoff:
        leaq _GLOBAL_OFFSET_TABLE_(%rip), %rcx
        movabsq $fn@PLTOFF, %rax
        addq %rcx, %rax
        jmp *%rax
        .section .note.GNU-stack,"",@progbits
"""

_PLTOFF_MAIN = r"""
#include <stdio.h>
extern int dvar;
void *get_pltoff(void), *get_pc32(void);
int call_fnoff(void);
int main(void)
{
    printf("%d %d %d %d\n", get_pc32() == (void *)&dvar,
           get_pltoff() == (void *)&dvar, dvar, call_fnoff());
    return 0;
}
"""

_DSO_HANDLE_MAIN = r"""
#include <stdio.h>
extern void *__dso_handle;
int main(void) { printf("%d\n", __dso_handle == (void *)&__dso_handle); return 0; }
"""


def _reference_kind_test(args, oracles):
    """References whose meaning depends on what the target is.

    * `pltoff_library_variable_exec`: `v@PLTOFF` (large-model GOT-relative)
      of a shared-library VARIABLE used to give the variable a PLT entry
      and a JUMP_SLOT; that entry also captured every direct (PC32)
      reference, so the program read the stub's code bytes.  GNU ld 2.44
      and 2.47 do the same (measured: `1 1 <garbage> 7`).  Now the variable
      is copy-relocated like any direct reference: `1 1 42 7`, for -no-pie
      and for -pie (main built -fPIE); no JUMP_SLOT names it.
    * `pltoff_library_variable_shared`: in a shared object there is no copy
      and no link-time address: refused, naming the variable.
    * `pie_absolute32_refused`: non-PIC code (`R_X86_64_32S` to a variable,
      `R_X86_64_32` to a string section) in a PIE was written with
      link-time addresses and crashed; now GNU ld's exact error.  An
      absolute symbol in the same field still links and runs.
    * `driver_pie_startfiles`: `lccc -pie` linked crt1.o/crtbegin.o (non-PIC,
      the latter now refused above); gcc's spec takes Scrt1.o/crtbeginS.o/
      crtendS.o.  crtbeginS.o's `__dso_handle` points at itself (crtbegin.o's
      is 0), so the program prints 1.
    * `as_needed_dropped_library_exports_nothing`: an `--as-needed` library
      nothing needed is not in the link, so its undefined references must
      not export the executable's definitions to `.dynsym` (GNU ld unloads
      it); a library that is linked still does.
    """
    names = ("pltoff_library_variable_exec", "pltoff_library_variable_shared",
             "pie_absolute32_refused", "driver_pie_startfiles",
             "as_needed_dropped_library_exports_nothing")
    lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    if not os.path.exists(lccc_ld):
        return [Result(n, "SKIP", "lccc-ld not built") for n in names]
    results = []
    td = tempfile.mkdtemp(prefix="lnk.refkind.")
    try:
        _write(td, "lib.c", _PLTOFF_LIB)
        _write(td, "h.s", _PLTOFF_ASM)
        _write(td, "m.c", _PLTOFF_MAIN)
        _write(td, "abs.s", "\t.text\n\t.globl get_abs\nget_abs:\tmovl $kabs, %eax\n\tret\n"
                            "\t.globl kabs\n\t.set kabs, 0x1234\n"
                            "\t.section .note.GNU-stack,\"\",@progbits\n")
        _write(td, "am.c", '#include <stdio.h>\nint get_abs(void);\n'
                           'int main(void){printf("%x\\n", get_abs());return 0;}\n')
        _write(td, "x.c", "extern int foo(void);\nint libx_use(void){return foo();}\n")
        _write(td, "fm.c", "int foo(void){return 3;}\nint main(void){return foo()-3;}\n")
        _write(td, "dh.c", _DSO_HANDLE_MAIN)
        for cmd in ([CC, "-shared", "-fPIC", "lib.c", "-o", "libd.so"],
                    [CC, "-c", "h.s", "-o", "h.o"],
                    [CC, "-O1", "-fno-pic", "-c", "m.c", "-o", "m.o"],
                    [CC, "-O1", "-fPIE", "-c", "m.c", "-o", "mpie.o"],
                    [CC, "-c", "abs.s", "-o", "abs.o"],
                    [CC, "-O1", "-fPIE", "-c", "am.c", "-o", "am.o"],
                    [CC, "-shared", "-fPIC", "x.c", "-o", "libx.so"],
                    [CC, "-O1", "-fPIE", "-c", "fm.c", "-o", "fm.o"]):
            r = sh(cmd, cwd=td)
            if r.returncode != 0:
                return [Result(n, "FAIL", f"fixture: {' '.join(cmd)}: {r.stderr.decode()[:200]}")
                        for n in names]
        shim = _shim_for(td, lccc_ld)
        rpath = f"-Wl,-rpath,{td}"

        # ── PLTOFF64 of a library variable, executable ──
        problems = []
        for mode, main in (("-no-pie", "m.o"), ("-pie", "mpie.o")):
            exe = f"pltoff{mode}"
            r = sh([CC, f"-B{shim}", mode, main, "h.o", "-L.", "-ld", rpath, "-o", exe], cwd=td)
            if r.returncode != 0:
                problems.append(f"{mode} link: {r.stderr.decode()[:200]}")
                continue
            rr = sh([os.path.join(td, exe)], cwd=td)
            if rr.returncode != 0 or rr.stdout != b"1 1 42 7\n":
                problems.append(f"{mode}: rc={rr.returncode} out={rr.stdout!r}, want b'1 1 42 7\\n'")
            rel = sh(["readelf", "-rW", exe], cwd=td).stdout.decode()
            slots = [l for l in rel.splitlines() if "JUMP_SLOT" in l and " dvar" in l]
            if slots:
                problems.append(f"{mode}: JUMP_SLOT for the variable: {slots[0].strip()}")
            if not any("R_X86_64_COPY" in l and " dvar" in l for l in rel.splitlines()):
                problems.append(f"{mode}: no R_X86_64_COPY for dvar")
        results.append(Result("pltoff_library_variable_exec", "FAIL" if problems else "PASS",
                              "; ".join(problems)[:600]))

        # ── ... and in a shared object ──
        _write(td, "q.s", "\t.text\n\t.globl g2\ng2:\tmovabsq $dvar@PLTOFF, %rax\n\tret\n")
        sh([CC, "-c", "q.s", "-o", "q.o"], cwd=td)
        r = sh([lccc_ld, "-shared", "q.o", "-L.", "-ld", "-o", "q.so"], cwd=td)
        msg = "relocation R_X86_64_PLTOFF64 against shared-library variable 'dvar'"
        ok = r.returncode != 0 and msg in r.stderr.decode()
        results.append(Result("pltoff_library_variable_shared", "PASS" if ok else "FAIL",
                              "" if ok else f"rc={r.returncode} stderr={r.stderr.decode()[:200]!r}, "
                                            f"want an error containing {msg!r}"))

        # ── Narrow absolute fields in a PIE ──
        problems = []
        want = ("relocation R_X86_64_32S against symbol `dvar' can not be used when making "
                "a PIE object; recompile with -fPIE")
        r = sh([CC, f"-B{shim}", "-pie", "m.o", "h.o", "-L.", "-ld", "-o", "bad"], cwd=td)
        if r.returncode == 0 or want not in r.stderr.decode():
            problems.append(f"variable: rc={r.returncode} stderr={r.stderr.decode()[:200]!r}")
        gnu = sh([CC, "-pie", "m.o", "h.o", "-L.", "-ld", "-o", "bad.gnu"], cwd=td)
        if want not in gnu.stderr.decode():
            problems.append(f"oracle drift: GNU ld said {gnu.stderr.decode()[:200]!r}")
        _write(td, "s.s", "\t.text\n\t.globl get_str\nget_str:\tmovl $.LC0, %eax\n\tret\n"
                          "\t.section .rodata.str1.1,\"aMS\",@progbits,1\n.LC0:\t.string \"x\"\n")
        sh([CC, "-c", "s.s", "-o", "s.o"], cwd=td)
        want_s = ("relocation R_X86_64_32 against `.rodata.str1.1' can not be used when "
                  "making a PIE object; recompile with -fPIE")
        r = sh([lccc_ld, "-pie", "-e", "get_str", "s.o", "-o", "bad2"], cwd=td)
        if r.returncode == 0 or want_s not in r.stderr.decode():
            problems.append(f"section: rc={r.returncode} stderr={r.stderr.decode()[:200]!r}")
        gnu = sh(["ld", "-pie", "-e", "get_str", "s.o", "-o", "bad2.gnu"], cwd=td)
        if want_s not in gnu.stderr.decode():
            problems.append(f"oracle drift: GNU ld said {gnu.stderr.decode()[:200]!r}")
        r = sh([CC, f"-B{shim}", "-pie", "am.o", "abs.o", "-o", "absok"], cwd=td)
        rr = sh([os.path.join(td, "absok")], cwd=td) if r.returncode == 0 else None
        if rr is None or rr.stdout != b"1234\n":
            problems.append(f"absolute: link rc={r.returncode} {r.stderr.decode()[:160]!r} "
                            f"out={rr and rr.stdout!r}")
        results.append(Result("pie_absolute32_refused", "FAIL" if problems else "PASS",
                              "; ".join(problems)[:600]))

        # ── The driver's PIE startfiles ──
        r = sh([args.lccc, "-O1", "-pie", "dh.c", "-o", "dh"], cwd=td)
        rr = sh([os.path.join(td, "dh")], cwd=td) if r.returncode == 0 else None
        hdr = sh(["readelf", "-hW", "dh"], cwd=td).stdout.decode() if rr else ""
        if rr is None or rr.stdout != b"1\n" or "DYN" not in hdr:
            results.append(Result("driver_pie_startfiles", "FAIL",
                                  f"link rc={r.returncode} {r.stderr.decode()[:200]!r} "
                                  f"out={rr and rr.stdout!r} (want b'1\\n', ET_DYN)"))
        else:
            results.append(Result("driver_pie_startfiles", "PASS"))

        # ── A dropped --as-needed library exports nothing ──
        problems = []
        for label, flags, want_needed in (("as-needed", ["-Wl,--as-needed", "-lx",
                                                         "-Wl,--no-as-needed"], 0),
                                          ("linked", ["-Wl,--no-as-needed", "-lx"], 1)):
            for who, b in (("lccc", [f"-B{shim}"]), ("GNU", [])):
                exe = f"an.{label}.{who}"
                r = sh([CC] + b + ["-pie", "fm.o", "-L."] + flags + [rpath, "-o", exe], cwd=td)
                if r.returncode != 0:
                    problems.append(f"{who} {label}: {r.stderr.decode()[:160]}")
                    continue
                dyn = sh(["readelf", "-dW", exe], cwd=td).stdout.decode()
                syms = sh(["readelf", "--dyn-syms", "-W", exe], cwd=td).stdout.decode()
                needed = int("libx.so" in dyn)
                exported = sum(1 for l in syms.splitlines() if l.split()[-1:] == ["foo"])
                if (needed, exported) != (want_needed, want_needed):
                    problems.append(f"{who} {label}: DT_NEEDED libx={needed} foo exported="
                                    f"{exported}, want {want_needed}/{want_needed}")
        results.append(Result("as_needed_dropped_library_exports_nothing",
                              "FAIL" if problems else "PASS", "; ".join(problems)[:600]))
    finally:
        shutil.rmtree(td, ignore_errors=True)
    return results


def _standalone_ld_flags_test(args, oracles):
    """Behaviour of `lccc-ld` invoked as `ld` (by gcc, or by hand), for
    x86-64 and i386, against GNU ld:

    * `-lc` finds libc on GNU ld's built-in search path (lccc-ld had none:
      "cannot find -lc" unless a driver passed -L), and `-nostdlib` drops
      that path, as GNU ld's does;
    * bare `--disable-new-dtags` / `--enable-new-dtags` pick DT_RPATH /
      DT_RUNPATH for `-rpath` (only the `-Wl,` spelling used to reach the
      parser: both were dropped as unknown options, so a GCC-driven link
      always got DT_RUNPATH).
    """
    names = ("lccc_ld_default_search_path_x86_64", "lccc_ld_default_search_path_i386",
             "lccc_ld_new_dtags_x86_64", "lccc_ld_new_dtags_i386")
    lccc_ld = os.path.join(os.path.dirname(args.lccc), "lccc-ld")
    if not os.path.exists(lccc_ld):
        return [Result(n, "SKIP", "lccc-ld not built") for n in names]
    results = []
    td = tempfile.mkdtemp(prefix="lnk.searchdir.")
    try:
        _write(td, "x.c", "#include <stdio.h>\nint hello(void){return puts(\"hi\");}\n")
        for name, cflags, mflags in ((names[0], [], []),
                                     (names[1], ["-m32"], ["-m", "elf_i386"])):
            r = sh([CC] + cflags + ["-fPIC", "-c", "x.c", "-o", "x.o"], cwd=td)
            if r.returncode != 0:
                results.append(Result(name, "FAIL", f"fixture: {r.stderr.decode()[:200]}"))
                continue
            problems = []
            for who, ld in (("lccc", lccc_ld), ("GNU", "ld")):
                r = sh([ld] + mflags + ["-shared", "x.o", "-lc", "-o", f"x.{who}.so"], cwd=td)
                dyn = sh(["readelf", "-dW", f"x.{who}.so"], cwd=td).stdout.decode()
                if r.returncode != 0 or "[libc.so.6]" not in dyn:
                    problems.append(f"{who}: rc={r.returncode} {r.stderr.decode()[:160]!r}, "
                                    "want DT_NEEDED libc.so.6")
                r = sh([ld] + mflags + ["-nostdlib", "-shared", "x.o", "-lc", "-o", "n.so"],
                       cwd=td)
                if r.returncode == 0 or "cannot find -lc" not in r.stderr.decode():
                    problems.append(f"{who} -nostdlib: rc={r.returncode} "
                                    f"{r.stderr.decode()[:160]!r}, want \"cannot find -lc\"")
            results.append(Result(name, "FAIL" if problems else "PASS",
                                  "; ".join(problems)[:600]))
            dname = name.replace("default_search_path", "new_dtags")
            problems = []
            for flag, tag in (("--disable-new-dtags", "(RPATH)"),
                              ("--enable-new-dtags", "(RUNPATH)")):
                for who, ld in (("lccc", lccc_ld), ("GNU", "ld")):
                    out = f"d.{who}.so"
                    r = sh([ld] + mflags + ["-shared", "x.o", "-rpath", "/opt/lccc-x", flag,
                                            "-o", out], cwd=td)
                    dyn = sh(["readelf", "-dW", out], cwd=td).stdout.decode()
                    got = [l.split()[1] for l in dyn.splitlines() if "/opt/lccc-x" in l]
                    if r.returncode != 0 or got != [tag]:
                        problems.append(f"{who} {flag}: rc={r.returncode} tags={got}, want "
                                        f"[{tag!r}] {r.stderr.decode()[:120]!r}")
            results.append(Result(dname, "FAIL" if problems else "PASS",
                                  "; ".join(problems)[:600]))
    finally:
        shutil.rmtree(td, ignore_errors=True)
    return results


class _Entry:
    """One registered test (or group of tests sharing a fixture).

    `names` are the result names the runner reports and the only thing
    `--filter` matches and `--list` prints; `aux` are extra names it may report
    instead when its fixture cannot be built (group-level SKIP/FAIL).  main()
    turns any other reported name into a failure, so the registry cannot drift
    from the runners.
    """

    def __init__(self, names, runner, tags=(), aux=()):
        self.names = tuple(names)
        self.runner = runner
        self.tags = tuple(tags)
        self.aux = tuple(aux)

    def selected(self, args):
        if args.tag and args.tag not in self.tags:
            return False
        return not args.filter or any(args.filter in n for n in self.names)

    def declares(self, name):
        return name in self.names or name in self.aux


def _registry(args, oracles):
    """Every test, in execution order."""
    import i386_userspace  # noqa: PLC0415 - sibling module (ELF32 userspace via gcc -m32)

    def one(name, fn, *tags, aux=()):
        return _Entry((name,), lambda: fn(args, oracles), tags, aux)

    def group(names, fn, *tags, aux=()):
        return _Entry(names, lambda: fn(args, oracles), tags, aux)

    reg = [_Entry((c.name,), (lambda c=c: run_case(c, args, oracles)), c.tags)
           for c in CASES]
    reg += [_Entry((name,), (lambda t=(name, script, csrc, expect):
                             _script_test(*t)(args, oracles)), ("script",))
            for (name, script, csrc, expect) in SCRIPT_TESTS]
    for entry in REL_TESTS:
        reg.append(_Entry((entry[0],), (lambda e=entry: _rel_test(
            e[0], e[1], e[2], compile_flags=e[3] if len(e) > 3 else None)(args, oracles)),
            ("rel",)))
    reg += [
        one("reloc_whole_archive_thin_and_regular", _whole_archive_r_test, "rel"),
        one("file_mode_libs_r_and_T", _file_mode_libs_test, "rel"),
    ]
    for tname, runner in (("dso_pointer_equality", _dso_pointer_equality_test),
                          ("i386_dso_pointer_equality", _dso_pointer_equality_i386_test),
                          ("gotpcrelx_relax_vs_bfd", _gotpcrelx_relax_test),
                          ("dso_emit_semantics", _dso_emit_semantics_test),
                          ("i386_dso_emit_semantics", _dso_emit_semantics_i386_test),
                          ("code_model_large_medium", _code_model_large_medium_test),
                          ("exe_exports_dso_names", _exe_exports_dso_names_test),
                          ("so_bound_ifunc", _so_bound_ifunc_test),
                          ("eh_frame_packing", _eh_frame_packing_test),
                          ("gotpcrel_edges", _gotpcrel_edges_test),
                          ("got64_spelling_matrix", _got64_spelling_matrix_test),
                          ("absolute_symbols_pic", _absolute_symbols_pic_test),
                          ("ie_to_le_local", _ie_to_le_local_test),
                          ("ie_to_le_forms", _ie_to_le_forms_test),
                          ("movrs_relocations", _movrs_relocations_test),
                          ("plain_gotpcrel_relaxation", _plain_gotpcrel_test),
                          ("text_relocations", _text_relocations_test)):
        reg.append(one(tname, runner, "dynamic"))
    reg += [
        one("cxx_exceptions_unwind", _cxx_eh_test, "ehframe"),
        one("cxx_exceptions_local_typeinfo", _cxx_eh_local_type_test, "ehframe"),
        one("script_pie_base0", _pie_script_test, "script"),
        one("script_undefined_pulls_archive", _script_undefined_archive_test, "script"),
        one("script_undefined_pulls_archive_i386", _script_undefined_archive_test_i386,
            "script"),
        _Entry(tuple(n for n, _ in i386_userspace.CASES),
               lambda: i386_userspace.run_all(args, CC, Result), ("shared",)),
        one("script_vdso_dynamic", _vdso_script_test, "script"),
        one("script_vdso_declared_note_phdrs", _vdso_note_phdr_test, "script"),
        one("script_elf32_i386_relocations", _elf32_script_test, "script", "kernel"),
        one("script_elf32_gc_keep", _elf32_script_gc_keep_test, "script", "kernel"),
        one("script_elf32_vdso_multiversion", _elf32_vdso_test, "script", "kernel"),
        one("gc_eh_frame_invariant", _gc_eh_frame_invariant_test, "gc", "sections"),
        one("build_id_note", _build_id_note_test, "notes"),
        one("relro_nobits_pad", _relro_nobits_pad_test, "layout"),
        one("note_run_merge", _note_run_merge_test, "notes"),
        one("gnu_hash_sizing", _gnu_hash_sizing_test, "dynamic"),
        one("crossarch_gnu_hash_relro", _crossarch_gnu_hash_relro_test, "dynamic"),
        one("static_pie_refused", _static_pie_refusal_test, "driver"),
        one("emit_relocs_warns_when_unimplemented", _emit_relocs_warns_test, "kernel"),
        one("dyn_init_fini_shared", _dyn_init_fini_shared_test, "dynamic"),
        one("script_hidden_not_exported", _script_hidden_visibility_test, "script"),
        one("script_pic_gotpcrel_relaxation", _script_pic_gotpcrel_test, "script"),
        one("script_tls_initial_exec", _script_tls_test, "script"),
        one("script_tls_gd_ld_relaxation", _script_tls_gd_ld_test, "script"),
        one("script_overlay_shared_vma", _script_overlay_test, "script"),
        one("as_needed_positional_dt_needed", _as_needed_positional_test, "driver"),
        one("script_as_needed_input", _script_as_needed_input_test, "script", "driver"),
        one("bstatic_positional_search", _bstatic_positional_test, "driver"),
        one("print_map_to_stdout", _print_map_test, "map"),
        one("version_script_multiple_nodes", _multi_version_node_test, "exports"),
        one("comdat_group_dedup", _comdat_dedup_test, "sections"),
        one("section_header_index_consistency", _section_header_index_consistency_test,
            "layout"),
        one("script_nonalloc_debug_payload", _script_nonalloc_section_test,
            "script", "kernel"),
        _Entry(("so_default_interposable",),
               lambda: _interpose_test(args, oracles, "so_default_interposable", [], "101\n"),
               ("shared",)),
        _Entry(("so_bsymbolic_binds_local",),
               lambda: _interpose_test(args, oracles, "so_bsymbolic_binds_local",
                                       ["-Wl,-Bsymbolic"], "43\n"),
               ("shared",)),
        one("so_z_defs_rejects_undefined", _zdefs_test, "shared"),
        one("so_shared_flag_parity", _so_shared_flags_test, "shared"),
        group(("export_dynamic_exports_globals", "export_dynamic_dlopen_callback",
               "export_dynamic_version_script", "export_dynamic_matches_gnu_ld"),
              _export_dynamic_test, "exports", aux=("export_dynamic",)),
        group(("emit_relocs_sections_present", "emit_relocs_header_wiring",
               "emit_relocs_matches_gnu_ld", "emit_relocs_does_not_perturb_image",
               "emit_relocs_kernel_relocs_tool"),
              _emit_relocs_test, "kernel", aux=("emit_relocs",)),
        group(("exclude_libs_control", "exclude_libs_hides_symbols",
               "exclude_libs_lib_still_works", "exclude_libs_no_null_jumpslot",
               "exclude_libs_ALL"),
              _exclude_libs_test, "exports", aux=("exclude_libs",)),
        group(("driver_option_noise",)
              + tuple(f"driver_flag_silent[{f}]" for f in _DRIVER_SILENT_FLAGS)
              + ("lto_bytecode_refused",),
              _driver_option_noise_test, "driver"),
        group(("segment_congruence", "segment_no_page_padding", "relro_ends_on_page_boundary",
               "shared_lib_segment_congruence", "shared_lib_no_page_padding",
               "shared_lib_still_loads", "output_not_larger_than_bfd"),
              _segment_packing_test, "layout", aux=("segment_packing",)),
        one("map_file_matches_binary", _map_file_test, "map"),
        one("map_file_two_arg_spelling", _map_file_spellings_test, "map"),
        group(tuple(n for n, _ in MALFORMED_CASES), _robustness_tests, "robustness",
              aux=("robustness",)),
        group(tuple(f"reloc_pc32_out_of_range_diagnosed_on_{label}_path"
                    for label in ("exec", "script"))
              + ("reloc_32_out_of_range_diagnosed_on_shared_path",
                 "reloc_32_preemptible_abs_in_shared_refused",
                 "reloc_pc32_abs_in_shared_is_dynamic")
              + tuple(f"reloc_{v}_{k.lower()}"
                      for k, *_ in _FIELD_CASES
                      for v in ("out_of_range_diagnosed", "in_range_accepted"))
              + ("reloc_64_accepts_above_4g",),
              _reloc_field_range_tests, "reloc",
              aux=("reloc_field_range", "reloc_pc32_fixture", "reloc_pc32_out_of_range")
              + tuple(f"reloc_{sym}{sfx}" for _, _, sym, *_ in _FIELD_CASES
                      for sfx in ("", "_fixture"))),
        group(("reloc_offset_text_w4", "reloc_offset_wrap_text",
               "reloc_offset_data.rel.ro_w8", "reloc_offset_wrap_data.rel.ro"),
              _reloc_offset_tests, "reloc", aux=("reloc_offset",)),
        group(("defsym_layout_plus", "defsym_layout_arith", "defsym_layout_end_final",
               "defsym_layout_reloc_reference", "defsym_layout_shared"),
              _defsym_layout_test, "defsym", aux=("defsym_layout_expr",)),
        group(("lccc_ld_default_search_path_x86_64", "lccc_ld_default_search_path_i386",
               "lccc_ld_new_dtags_x86_64", "lccc_ld_new_dtags_i386"),
              _standalone_ld_flags_test, "dynamic", "i386"),
        group(("pltoff_library_variable_exec", "pltoff_library_variable_shared",
               "pie_absolute32_refused", "driver_pie_startfiles",
               "as_needed_dropped_library_exports_nothing"),
              _reference_kind_test, "dynamic"),
        group(("defsym_forward_reference_pie", "defsym_forward_reference_shared",
               "defsym_forward_reference_script", "defsym_forward_reference_i386",
               "defsym_address_slides_in_pie", "defsym_pc32_absolute_pie_refused",
               "defsym_cycle_diagnosed"),
              _defsym_order_test, "defsym"),
    ]
    return reg


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc", default=DEFAULT_LCCC)
    ap.add_argument("--filter", default="")
    ap.add_argument("--tag", default="")
    ap.add_argument("-v", "--verbose", action="store_true")
    ap.add_argument("--keep", action="store_true", help="keep temp dirs")
    ap.add_argument("--list", action="store_true",
                    help="print the selected test names (and tags) without running them")
    ap.add_argument("--json", metavar="FILE", help="also write the results as JSON")
    ap.add_argument("--strict", action="store_true",
                    help="fail on SKIP and WARN results too (CI mode)")
    args = ap.parse_args()
    # Many cases execute the driver from a temporary working directory. Keep a
    # caller-supplied relative path anchored to the invocation directory.
    args.lccc = os.path.abspath(os.path.expanduser(args.lccc))

    have_mold = shutil.which("mold") is not None
    have_wild = shutil.which("wild") is not None
    # `lld` must be probed FUNCTIONALLY, not just present: the suite's whole
    # point on the script-path fixture is a second opinion, and `ld.lld` being
    # on PATH does not mean this gcc accepts `-fuse-ld=lld` (a cross toolchain
    # will happily ship the binary and reject the driver flag). Probing with
    # the driver is the only way to know the oracle can actually be used.
    have_lld = (
        (shutil.which("ld.lld") is not None or shutil.which("lld") is not None)
        and sh([CC, "-fuse-ld=lld", "-Wl,--version"]).returncode == 0
    )

    oracles = [("bfd", [CC, "-fuse-ld=bfd"])]
    # lld is the oracle that actually implements linker scripts. Without it the
    # script-path cross-check runs on bfd alone, `incapable` correctly excludes
    # mold, and `floor = min(2, 1)` quietly certifies a single opinion -- which
    # is the outcome `.github/workflows/ci.yml` installs lld specifically to
    # prevent. A rebase once dropped this registration while leaving the
    # install step and its rationale comment in place, so the workflow claimed
    # a two-oracle cross-check that never ran.
    if have_lld:
        oracles.append(("lld", [CC, "-fuse-ld=lld"]))
    if have_mold:
        oracles.append(("mold", [CC, "-fuse-ld=mold"]))
    if have_wild:
        wildpath = shutil.which("wild")
        oracles.append(("wild", [CC, f"-B{os.path.dirname(_wild_shim(wildpath))}"]))

    registry = _registry(args, oracles)
    if args.list:
        for e in registry:
            if e.selected(args):
                for n in e.names:
                    if not args.filter or args.filter in n:
                        print(f"{n}\t{','.join(e.tags)}")
        return
    results = []
    seen = set()
    for e in registry:
        if not e.selected(args):
            continue
        out = e.runner()
        out = out if isinstance(out, list) else [out]
        # A declared test its runner silently did not report (an optional
        # tool was missing, a reference link failed) did not run: that is a
        # SKIP, which --strict refuses, unless a group-level result stands in.
        reported = {r.name for r in out}
        if not any(n in reported for n in e.aux):
            out += [Result(n, "SKIP", "not reported by its runner (did not run)")
                    for n in e.names if n not in reported]
        for r in out:
            # Every result must be one the registry declares: that is what
            # makes `--filter`/`--list` exact.  A misnamed or duplicated one
            # is a harness defect, reported as a failure, never dropped.
            if not e.declares(r.name):
                r = Result(r.name, "FAIL",
                           f"undeclared result name (declare it in _registry): "
                           f"{r.status} {r.detail}")
            elif r.name in seen:
                r = Result(r.name, "FAIL", f"duplicate result name: {r.status} {r.detail}")
            seen.add(r.name)
            if not args.filter or args.filter in r.name:
                results.append(r)

    npass = sum(1 for r in results if r.status == "PASS")
    nfail = sum(1 for r in results if r.status == "FAIL")
    nwarn = sum(1 for r in results if r.status == "WARN")
    nskip = sum(1 for r in results if r.status == "SKIP")
    print()
    for r in results:
        if r.status != "PASS" or args.verbose:
            print(f"[{r.status}] {r.name}" + (f"\n    {r.detail}" if r.detail else ""))
    # Print the oracle names that were ACTUALLY used. The previous hardcoded
    # "bfd{mold}{wild}" string could not drift when the list did, so a lost
    # registration was invisible in the one line a reviewer would look at --
    # which is exactly how the missing lld entry survived a rebase.
    print(f"\n== linker tests: {npass} pass, {nfail} fail, {nwarn} warn, {nskip} skip "
          f"(oracles: {' '.join(n for n, _ in oracles)}) ==")
    if args.json:
        with open(args.json, "w") as f:
            json.dump([{"name": r.name, "status": r.status, "detail": r.detail}
                       for r in results], f, indent=1)
    if args.filter and not results:
        print(f"error: --filter {args.filter!r} selected no test", file=sys.stderr)
        sys.exit(1)
    # --strict: a SKIP is a test that did not run (a missing compiler, a
    # fixture the toolchain refused) and a WARN a known defect; in CI, where
    # the job installs every tool, both are failures to be fixed, not parked.
    if args.strict and (nskip or nwarn):
        print(f"error: --strict: {nskip} skipped and {nwarn} warned test(s)", file=sys.stderr)
        sys.exit(1)
    sys.exit(1 if nfail else 0)

_WILD_SHIM_DIR = None
def _wild_shim(wildpath):
    """gcc has no -fuse-ld=wild; create a shim dir with ld -> wild."""
    global _WILD_SHIM_DIR
    if _WILD_SHIM_DIR is None:
        _WILD_SHIM_DIR = tempfile.mkdtemp(prefix="wildshim.")
        os.symlink(wildpath, os.path.join(_WILD_SHIM_DIR, "ld"))
    return os.path.join(_WILD_SHIM_DIR, "ld")

def symtab_problems(path):
    """Inspect .symtab: presence, entry count, sh_info correctness, zero-size ghosts."""
    out = {"present": False, "nsyms": 0, "zero_size": 0, "sh_info_bad": None,
           "order_bad": None}
    try:
        with open(path, "rb") as fh:
            d = fh.read()
    except OSError:
        return out
    if len(d) < 64 or d[:4] != b"\x7fELF":
        return out
    e_shoff, = struct.unpack_from("<Q", d, 0x28)
    e_shentsize, e_shnum, e_shstrndx = struct.unpack_from("<HHH", d, 0x3A)
    secs = []
    for i in range(e_shnum):
        b = e_shoff + i * e_shentsize
        if b + e_shentsize > len(d):
            return out
        n, t, fl, a, off, sz, lk, inf, al, es = struct.unpack_from("<IIQQQQIIQQ", d, b)
        secs.append((n, t, off, sz, lk, inf))
    for n, t, off, sz, lk, inf in secs:
        if t != 2:  # SHT_SYMTAB
            continue
        out["present"] = True
        out["nsyms"] = sz // 24
        nlocal = 0
        maxlocal = -1
        firstglob = None
        for k in range(sz // 24):
            nm, info, other, shndx, val, size = struct.unpack_from("<IBBHQQ", d, off + k * 24)
            if val == 0 and size != 0:
                out["zero_size"] += 1
            if (info >> 4) == 0:
                nlocal += 1
                maxlocal = k
            elif firstglob is None:
                firstglob = k
        if inf != nlocal:
            out["sh_info_bad"] = (f".symtab sh_info is {inf} but there are "
                                  f"{nlocal} STB_LOCAL entries (ELF requires sh_info == nlocals)")
        # sh_info == nlocals is necessary but NOT sufficient: every LOCAL
        # must also precede the first global.  A STB_LOCAL entry appended by
        # the globals loop (e.g. a synthetic pool symbol living in both the
        # object symbol lists and the resolved map) passes the count check
        # and still violates the spec — readelf warns, debuggers misread.
        if firstglob is not None and maxlocal > firstglob:
            out["order_bad"] = (f".symtab not partitioned: STB_LOCAL entry at index "
                                f"{maxlocal} follows the first global at {firstglob} "
                                f"(ELF requires all locals first)")
    return out


def dyn_hash_tags(path):
    """Sorted list of hash tags in .dynamic, plus a check that each has a section."""
    tags = []
    try:
        with open(path, "rb") as fh:
            d = fh.read()
    except OSError:
        return tags
    if len(d) < 64 or d[:4] != b"\x7fELF":
        return tags
    e_shoff, = struct.unpack_from("<Q", d, 0x28)
    e_shentsize, e_shnum, e_shstrndx = struct.unpack_from("<HHH", d, 0x3A)
    secs = []
    for i in range(e_shnum):
        b = e_shoff + i * e_shentsize
        if b + e_shentsize > len(d):
            return tags
        n, t, fl, a, off, sz, lk, inf, al, es = struct.unpack_from("<IIQQQQIIQQ", d, b)
        secs.append((n, t, off, sz))
    shstr = secs[e_shstrndx] if e_shstrndx < len(secs) else None
    def nm(x):
        if shstr is None or x >= shstr[3]:
            return ""
        e = d.index(b"\0", shstr[2] + x)
        return d[shstr[2] + x:e].decode("latin1", "replace")
    names = {nm(s[0]) for s in secs}
    dyn = next((s for s in secs if s[1] == 6), None)  # SHT_DYNAMIC
    if dyn is None:
        return tags
    for k in range(dyn[3] // 16):
        tag, val = struct.unpack_from("<qQ", d, dyn[2] + k * 16)
        if tag == 0x6FFFFEF5:  # DT_GNU_HASH
            if ".gnu.hash" in names:
                tags.append("GNU_HASH")
        elif tag == 4:  # DT_HASH
            if ".hash" in names:
                tags.append("HASH")
    return sorted(tags)


# Dynamic-tag numbers (ELF64, x86-64 System V ABI).
_DT_INIT = 12
_DT_FINI = 13
_DT_RELACOUNT = 0x6FFFFFF9
_DT_RPATH = 15
_DT_RUNPATH = 29
_DT_NUMBERS = {
    "INIT": _DT_INIT,
    "FINI": _DT_FINI,
    "RELACOUNT": _DT_RELACOUNT,
    "RPATH": _DT_RPATH,
    "RUNPATH": _DT_RUNPATH,
}
_R_X86_64_RELATIVE = 8


def _elf_sections(d):
    """[(name_off, type, addr, off, size, link)], shstrndx — or None.

    64-bit LE only: every link in this harness is an x86-64 driver link.
    """
    if len(d) < 64 or d[:4] != b"\x7fELF" or d[4] != 2 or d[5] != 1:
        return None
    e_shoff, = struct.unpack_from("<Q", d, 0x28)
    e_shentsize, e_shnum, e_shstrndx = struct.unpack_from("<HHH", d, 0x3A)
    secs = []
    for i in range(e_shnum):
        b = e_shoff + i * e_shentsize
        if b + e_shentsize > len(d):
            return None
        n, t, fl, a, off, sz, lk, inf, al, es = struct.unpack_from("<IIQQQQIIQQ", d, b)
        secs.append((n, t, a, off, sz, lk))
    return secs, e_shstrndx


def _sec_name(d, shstr, x):
    if shstr is None or x >= shstr[4]:
        return ""
    e = d.index(b"\0", shstr[3] + x)
    return d[shstr[3] + x:e].decode("latin1", "replace")


def dyn_tag_map(path):
    """({tag: value} from SHT_DYNAMIC, {section-name: address}).

    Either dict is empty when the image cannot be parsed.
    """
    tags, addrs = {}, {}
    try:
        with open(path, "rb") as fh:
            d = fh.read()
    except OSError:
        return tags, addrs
    parsed = _elf_sections(d)
    if parsed is None:
        return tags, addrs
    secs, e_shstrndx = parsed
    shstr = secs[e_shstrndx] if e_shstrndx < len(secs) else None
    for n, t, a, off, sz, _lk in secs:
        addrs[_sec_name(d, shstr, n)] = a
    dyn = next((s for s in secs if s[1] == 6), None)  # SHT_DYNAMIC
    if dyn is None:
        return tags, addrs
    for k in range(dyn[4] // 16):
        tag, val = struct.unpack_from("<qQ", d, dyn[3] + k * 16)
        tags.setdefault(tag, val)
    return tags, addrs


def rela_dyn_relative_run(path):
    """Count of leading R_X86_64_RELATIVE entries in .rela.dyn, or None.

    None means the section is absent (or the image unparseable) — distinct
    from 0, so callers can tell "no .rela.dyn" from "GLOB_DAT-only".
    """
    try:
        with open(path, "rb") as fh:
            d = fh.read()
    except OSError:
        return None
    parsed = _elf_sections(d)
    if parsed is None:
        return None
    secs, e_shstrndx = parsed
    shstr = secs[e_shstrndx] if e_shstrndx < len(secs) else None
    rela = next((s for s in secs
                 if s[1] == 4 and _sec_name(d, shstr, s[0]) == ".rela.dyn"), None)
    if rela is None:
        return None
    run = 0
    for k in range(rela[4] // 24):
        r_info, = struct.unpack_from("<Q", d, rela[3] + k * 24 + 8)
        if r_info & 0xFFFFFFFF != _R_X86_64_RELATIVE:
            break
        run += 1
    return run


def sym_values(path):
    """{symbol name: st_value} from SHT_SYMTAB, or None if unparseable.

    First definition wins: a folded-away duplicate that survives as an
    alias keeps its name with the representative's address (that is the
    property the ICF alias check asserts), while a dropped symbol is
    simply absent.
    """
    try:
        with open(path, "rb") as fh:
            d = fh.read()
    except OSError:
        return None
    parsed = _elf_sections(d)
    if parsed is None:
        return None
    secs, e_shstrndx = parsed
    shstr = secs[e_shstrndx] if e_shstrndx < len(secs) else None
    symtab = next((s for s in secs
                   if s[1] == 2 and _sec_name(d, shstr, s[0]) == ".symtab"), None)
    if symtab is None or symtab[5] >= len(secs):
        return None
    strtab = secs[symtab[5]]
    vals = {}
    for k in range(symtab[4] // 24):
        st_name, _, _, _, st_value, _ = struct.unpack_from(
            "<IBBHQQ", d, symtab[3] + k * 24)
        if st_name == 0 or st_name >= strtab[4]:
            continue
        e = d.index(b"\0", strtab[3] + st_name)
        name = d[strtab[3] + st_name:e].decode("latin1", "replace")
        vals.setdefault(name, st_value)
    return vals


def comment_strings(path):
    """Ordered .comment strings (NUL-split, empties dropped), or None.

    None means no .comment section at all — distinct from an empty one.
    """
    try:
        with open(path, "rb") as fh:
            d = fh.read()
    except OSError:
        return None
    parsed = _elf_sections(d)
    if parsed is None:
        return None
    secs, e_shstrndx = parsed
    shstr = secs[e_shstrndx] if e_shstrndx < len(secs) else None
    for s in secs:
        if s[1] == 1 and _sec_name(d, shstr, s[0]) == ".comment":
            return [p.decode("latin1", "replace")
                    for p in d[s[3]:s[3] + s[4]].split(b"\0") if p]
    return None


def prop_note_map(path):
    """{property-type: value} from the NT_GNU_PROPERTY_TYPE_0 note, or None.

    Parsed from the section bytes directly (64-bit LE only — every link in
    this harness is an x86-64 driver link): each entry is type(4) +
    datasz(4) + data, padded to the 8-alignment, i.e. 16 bytes per x86
    bitmask.  A 4-byte value maps to an int, any other size to
    (datasz, value).  Every note of the section is read; more than one
    property note in an output is malformed (the loader reads one), so it
    is returned as a list of maps, which never equals GNU's single map.
    None means absent or unparseable.
    """
    try:
        with open(path, "rb") as fh:
            d = fh.read()
    except OSError:
        return None
    try:
        if len(d) < 64 or d[:4] != b"\x7fELF" or d[4] != 2 or d[5] != 1:
            return None
        e_shoff, = struct.unpack_from("<Q", d, 0x28)
        e_shentsize, e_shnum, e_shstrndx = struct.unpack_from("<HHH", d, 0x3A)
        secs = []
        for i in range(e_shnum):
            b = e_shoff + i * e_shentsize
            if b + e_shentsize > len(d):
                return None
            n, t, fl, a, off, sz = struct.unpack_from("<IIQQQQ", d, b)[:6]
            secs.append((n, off, sz))
        if e_shstrndx >= len(secs):
            return None
        _, soff, ssz = secs[e_shstrndx]

        def nm(x):
            if x >= ssz:
                return ""
            e = d.index(b"\0", soff + x)
            return d[soff + x:e].decode("latin1", "replace")

        note = next((s for s in secs if nm(s[0]) == ".note.gnu.property"), None)
        if note is None or note[2] < 16:
            return None
        _, off, sz = note
        maps = []
        p, end_sec = off, off + sz
        while p + 12 <= end_sec:
            namesz, descsz, ntype = struct.unpack_from("<III", d, p)
            desc = p + ((12 + namesz + 7) & ~7)
            name = d[p + 12:p + 12 + namesz]
            nxt = desc + ((descsz + 7) & ~7)
            if desc + descsz > end_sec:
                return None
            if ntype == 5 and name == b"GNU\0":
                out = {}
                o, end = desc, desc + descsz
                while o + 8 <= end:
                    t, datasz = struct.unpack_from("<II", d, o)
                    if o + 8 + datasz > end:
                        return None
                    v = int.from_bytes(d[o + 8:o + 8 + datasz], "little")
                    out[t] = v if datasz == 4 else (datasz, v)
                    o += 8 + ((datasz + 7) & ~7)
                maps.append(out)
            p = nxt
        if not maps:
            return None
        return maps[0] if len(maps) == 1 else maps
    except (struct.error, ValueError, IndexError):
        return None


def elf_e_type(path):
    """Return the ELF e_type of `path` as "EXEC"/"DYN"/"REL"/..., or None.

    Parsed from the header directly rather than from `readelf` text: readelf's
    section-table columns shift when a name is long enough to wrap.
    """
    try:
        with open(path, "rb") as fh:
            ident = fh.read(20)
    except OSError:
        return None
    if len(ident) < 20 or ident[:4] != b"\x7fELF":
        return None
    is64 = ident[4] == 2
    little = ident[5] == 1
    fmt = ("<" if little else ">") + ("H" if is64 else "H")
    etype = struct.unpack(fmt, ident[16:18])[0]
    return {0: "NONE", 1: "REL", 2: "EXEC", 3: "DYN", 4: "CORE"}.get(etype, f"?{etype}")


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

        # --- symbol table shape ---
        if c.expect_no_symtab or c.expect_no_zero_size_syms or c.expect_symtab_valid:
            bad = symtab_problems(lccc_out)
            if c.expect_no_symtab and bad.get("present"):
                return Result(c.name, "FAIL",
                    f"-s was requested but .symtab is still present ({bad['nsyms']} symbols)")
            if c.expect_no_zero_size_syms and bad.get("zero_size"):
                return Result(c.name, "FAIL",
                    f"{bad['zero_size']} symbols have st_value==0 with st_size!=0 "
                    f"(symbols from collected sections, out of {bad['nsyms']})")
            if bad.get("sh_info_bad"):
                return Result(c.name, "FAIL", bad["sh_info_bad"])
            if c.expect_symtab_valid and bad.get("order_bad"):
                return Result(c.name, "FAIL", bad["order_bad"])
        if c.expect_dyn_tags is not None:
            got = dyn_hash_tags(lccc_out)
            want = sorted(c.expect_dyn_tags)
            if got != want:
                return Result(c.name, "FAIL",
                    f"dynamic hash tags are {got}, expected {want}")
        if c.expect_dyn_init_fini:
            tags, addrs = dyn_tag_map(lccc_out)
            for tagname, tag, sec in (("INIT", _DT_INIT, ".init"),
                                      ("FINI", _DT_FINI, ".fini")):
                if tag not in tags:
                    return Result(c.name, "FAIL",
                        f"DT_{tagname} missing from .dynamic")
                if sec not in addrs:
                    return Result(c.name, "FAIL",
                        f"DT_{tagname} present but {sec} section missing")
                if tags[tag] != addrs[sec]:
                    return Result(c.name, "FAIL",
                        f"DT_{tagname} is {tags[tag]:#x}, "
                        f"{sec} section is at {addrs[sec]:#x}")
        if c.expect_dyn_relacount is not None:
            tags, _ = dyn_tag_map(lccc_out)
            run = rela_dyn_relative_run(lccc_out)
            if c.expect_dyn_relacount:
                if _DT_RELACOUNT not in tags:
                    return Result(c.name, "FAIL",
                        "DT_RELACOUNT missing from .dynamic")
                if run is None:
                    return Result(c.name, "FAIL",
                        "DT_RELACOUNT present but no .rela.dyn to count")
                if tags[_DT_RELACOUNT] != run:
                    return Result(c.name, "FAIL",
                        f"DT_RELACOUNT is {tags[_DT_RELACOUNT]}, but the "
                        f"leading RELATIVE run of .rela.dyn is {run}")
            else:
                if _DT_RELACOUNT in tags:
                    return Result(c.name, "FAIL",
                        "DT_RELACOUNT must be omitted when the RELATIVE run "
                        f"is empty (value {tags[_DT_RELACOUNT]})")
                if run not in (None, 0):
                    return Result(c.name, "FAIL",
                        f"no DT_RELACOUNT but .rela.dyn opens with {run} "
                        "RELATIVE entries")
        if c.expect_same_address is not None:
            vals = sym_values(lccc_out)
            if vals is None:
                return Result(c.name, "FAIL",
                    "cannot parse .symtab for the alias check")
            for group in c.expect_same_address:
                missing = [n for n in group if n not in vals]
                if missing:
                    return Result(c.name, "FAIL",
                        f"symbols missing from .symtab: {missing} "
                        "(a folded-away duplicate must survive as an alias)")
                addrs = {vals[n] for n in group}
                if len(addrs) != 1:
                    return Result(c.name, "FAIL",
                        f"symbols not folded onto one address: "
                        + ", ".join(f"{n}={vals[n]:#x}" for n in group))
        if c.expect_distinct_addresses is not None:
            vals = sym_values(lccc_out)
            if vals is None:
                return Result(c.name, "FAIL",
                    "cannot parse .symtab for the distinct-address check")
            for group in c.expect_distinct_addresses:
                missing = [n for n in group if n not in vals]
                if missing:
                    return Result(c.name, "FAIL",
                        f"symbols missing from .symtab: {missing}")
                addrs = [vals[n] for n in group]
                if len(set(addrs)) != len(addrs):
                    return Result(c.name, "FAIL",
                        f"symbols wrongly folded onto one address: "
                        + ", ".join(f"{n}={vals[n]:#x}" for n in group))
        if c.expect_dyn_has is not None or c.expect_dyn_missing is not None:
            tags, _ = dyn_tag_map(lccc_out)
            for name in (c.expect_dyn_has or []):
                if _DT_NUMBERS[name] not in tags:
                    return Result(c.name, "FAIL",
                        f"DT_{name} missing from .dynamic")
            for name in (c.expect_dyn_missing or []):
                if _DT_NUMBERS[name] in tags:
                    return Result(c.name, "FAIL",
                        f"DT_{name} must not be in .dynamic")
        if c.expect_comment is not None:
            got = comment_strings(lccc_out)
            if got is None:
                return Result(c.name, "FAIL",
                    "no .comment section in output")
            for want in c.expect_comment:
                if got.count(want) != 1:
                    return Result(c.name, "FAIL",
                        f".comment contains {got.count(want)}x {want!r}, "
                        f"want exactly once: {got}")
            idx = [got.index(w) for w in c.expect_comment]
            if idx != sorted(idx):
                return Result(c.name, "FAIL",
                    f".comment order wrong: {got} "
                    f"(want {c.expect_comment} in order)")
        if c.expect_symbol_order is not None:
            vals = sym_values(lccc_out)
            if vals is None:
                return Result(c.name, "FAIL",
                    "cannot parse .symtab for the order check")
            missing = [n for n in c.expect_symbol_order if n not in vals]
            if missing:
                return Result(c.name, "FAIL",
                    f"symbols missing from .symtab: {missing}")
            addrs = [vals[n] for n in c.expect_symbol_order]
            if any(b <= a for a, b in zip(addrs, addrs[1:])):
                return Result(c.name, "FAIL",
                    "symbols not in ascending address order: "
                    + ", ".join(f"{n}={vals[n]:#x}"
                                for n in c.expect_symbol_order))

        # --- image shape ---
        if c.expect_elf_type is not None:
            got = elf_e_type(lccc_out)
            if got != c.expect_elf_type:
                return Result(c.name, "FAIL",
                    f"e_type is {got!r}, expected {c.expect_elf_type!r} "
                    f"(a fixed-base ET_EXEC here means -pie was silently downgraded)")

        # --- GNU property note (CET/ISA) merge vs bfd ---
        if c.expect_prop_note_match:
            bfd_out = next((o for n, o in oracle_outs if n == "bfd"), None)
            if bfd_out is None:
                return Result(c.name, "SKIP",
                    "bfd oracle produced no output to compare notes against")
            lm = prop_note_map(lccc_out)
            bm = prop_note_map(bfd_out)
            if lm is None or bm is None:
                return Result(c.name, "FAIL",
                    f"property note unreadable: lccc={lm!r} bfd={bm!r}")
            if lm != bm:
                return Result(c.name, "FAIL",
                    f"property note mismatch: lccc={lm!r} bfd={bm!r}")

        # --- run & compare ---
        code, out = run_bin(lccc_out, c.run_args, td, c.run_env)
        if c.expect_stdout is not None:
            if out != c.expect_stdout or code != c.expect_exit:
                detail = (f"lccc output {(code, out)!r} != expected "
                          f"{(c.expect_exit, c.expect_stdout)!r}")
                if c.known_defect:
                    return Result(c.name, "WARN", f"KNOWN DEFECT ({c.known_defect}): {detail}")
                return Result(c.name, "FAIL", detail)
            if c.known_defect:
                return Result(c.name, "WARN",
                    f"expected defect now passes -- remove known_defect: {c.known_defect}")
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
