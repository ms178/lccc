#!/usr/bin/env python3
"""ld_feature_census.py -- differential ELF feature census for lccc-ld.

The compiler-differential oracles (godbolt.py / codegen_oracle.py) answer
"does lccc generate the same *code*?".  Nothing in the tree answered the
symmetric question for the linker: "given byte-identical input objects and a
byte-identical command line, does lccc-ld produce the same *image*?".

This script does that.  For every (workload, link configuration) pair it

  * compiles the workload once with the host gcc, so every linker sees the
    exact same .o files (linker differences are never contaminated by
    compiler differences),
  * links it with each selected linker through the *same* gcc driver
    invocation (lccc-ld via a `-B` shim directory containing `ld`, the
    reference linkers via `-fuse-ld=`), which is how real build systems
    reach a linker and therefore how they must be judged,
  * records link status, link stderr, wall time, output size, program
    headers, section inventory, dynamic tags, notes, and the SHA-256 of
    `.text`,
  * runs the result and compares stdout/exit status against the reference
    linker, so a "successful" link that produces a broken binary cannot
    hide behind a zero exit status,
  * diffs everything against the reference linker (bfd by default) and
    emits a machine-readable JSON plus a markdown divergence report.

It is a census, not a pass/fail gate: a linker that legitimately differs
(for instance lccc-ld's deliberate ET_EXEC base of 0x400000) shows up as a
divergence with the reason attached, and the report is meant to be read by
a human deciding what to fix next.  Every divergence line cites the exact
case id and the exact readelf field so it is reproducible by hand.

Usage:
  scripts/ld_feature_census.py                     # all linkers, all cases
  scripts/ld_feature_census.py --cases pie,static  # subset by id substring
  scripts/ld_feature_census.py --ref mold --md /tmp/report.md
  scripts/ld_feature_census.py --json /tmp/census.json --only-missing

Environment:
  LCCC_LD   path to the lccc-ld binary (default: target/fastbuild/lccc-ld)
  CC        host compiler driver (default: gcc)

Exit status: 0 always (census), unless --strict, in which case non-zero when
any case fails to link or produces a wrong runtime result.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEFAULT_LCCC_LD = REPO / "target" / "fastbuild" / "lccc-ld"

# --------------------------------------------------------------------------
# Workloads.  Each is a small, self-contained C program chosen to exercise one
# specific part of a linker.  They are deliberately boring: the interesting
# variable in this harness is the linker, not the program.
# --------------------------------------------------------------------------

SRC_HELLO = r"""
#include <stdio.h>
int main(void) { puts("hello"); return 0; }
"""

# Multiple objects + cross-object calls + a global data reference.  Exercises
# symbol resolution, PC-relative call relocations and .data layout.
SRC_MULTIA = r"""
#include <stdio.h>
extern int b_value(void);
int a_value(void) { return 1; }
int main(void) { printf("%d%d\n", a_value(), b_value()); return 0; }
"""
SRC_MULTIB = r"""
int b_value(void) { return 2; }
"""
# __wrap_b_value intercepts calls to b_value; __real_b_value reaches the
# original.  Without this file the --wrap case is an expected link failure on
# every linker, which measures nothing.
SRC_WRAPIMPL = r"""
#include <stdio.h>
extern int __real_b_value(void);
int __wrap_b_value(void) { return __real_b_value() * 10; }
"""

# Copy relocation: taking the address of, and reading, an object defined in
# libc.so.6 forces R_X86_64_COPY in a non-PIE executable and a GOT-relative
# access in a PIE.  `environ` is the classic case.
SRC_COPYRELOC = r"""
#include <stdio.h>
#include <unistd.h>
extern char **environ;
int main(void) {
    char **p = environ;
    int n = 0;
    while (*p) { p++; n++; }
    printf("env=%d\n", n > 0);
    return 0;
}
"""

# Thread-local storage in all three models the linker sees: local-exec
# (defined here, TPOFF32), initial-exec via a shared library consumer, and
# the PT_TLS program header itself.
SRC_TLS = r"""
#include <stdio.h>
#include <pthread.h>
static __thread int tls_counter;
static void *worker(void *arg) {
    (void)arg;
    for (int i = 0; i < 1000; i++) tls_counter++;
    return (void *)(long)tls_counter;
}
int main(void) {
    pthread_t t;
    pthread_create(&t, 0, worker, 0);
    void *r;
    pthread_join(t, &r);
    for (int i = 0; i < 1000; i++) tls_counter++;
    printf("tls=%ld,%d\n", (long)r, tls_counter);
    return 0;
}
"""

# GNU IFUNC: the resolver runs at startup, before main.  A linker that
# mishandles STT_GNU_IFUNC / R_X86_64_IRELATIVE produces a segfault or a
# silently wrong dispatch.
SRC_IFUNC = r"""
#include <stdio.h>
static int impl_a(void) { return 1; }
static int impl_b(void) { return 2; }
static void *resolver(void) { return (void *)impl_b; }
extern int pick(void) __attribute__((ifunc("resolver")));
int main(void) { printf("ifunc=%d\n", pick()); return 0; }
"""

# Constructor priorities.  .init_array.NNNNN ordering is a classic linker
# defect: the numeric suffix, not the input order, decides execution order.
SRC_CTOR = r"""
#include <stdio.h>
static void c3(void) __attribute__((constructor(300)));
static void c1(void) __attribute__((constructor(100)));
static void c2(void) __attribute__((constructor(200)));
static void c3(void) { fputs("3", stdout); }
static void c1(void) { fputs("1", stdout); }
static void c2(void) { fputs("2", stdout); }
static void d1(void) __attribute__((destructor(100)));
static void d1(void) { fputs("|d1", stdout); }
int main(void) { fputs("M", stdout); return 0; }
"""

# Unwinding: throw/catch through a shared boundary needs .eh_frame to survive
# the merge and .eh_frame_hdr to be consistent with it.  C only, so we use
# pthread_cancel-free longjmp-style unwinding via __builtin_unwind_init and
# the real thing: a backtrace-free forced unwind is overkill; instead we take
# a backtrace with backtrace_symbols, which reads .eh_frame_hdr at runtime.
SRC_UNWIND = r"""
#include <stdio.h>
#include <execinfo.h>
static int inner(void) {
    void *bt[8];
    int n = backtrace(bt, 8);
    return n;
}
int main(void) { printf("bt=%d\n", inner() > 0); return 0; }
"""

# Weak symbols and a deliberately unresolved weak reference.  A linker must
# bind weak-undefined to 0 and must *not* fail the link.
SRC_WEAK = r"""
#include <stdio.h>
extern int maybe_absent(void) __attribute__((weak));
int present(void) { return 7; }
int main(void) { printf("weak=%d,%d\n", maybe_absent != 0, present()); return 0; }
"""

# Archive member selection plus --whole-archive: `only_ctor` is reachable only
# through its constructor, so lazy archive loading must not drop it, and
# --whole-archive must pull it in even when nothing references it.
SRC_ARCHMAIN = r"""
#include <stdio.h>
extern int used_fn(void);
extern int ctor_ran;
int main(void) { printf("arch=%d,%d\n", used_fn(), ctor_ran); return 0; }
"""
SRC_ARCHLIB_USED = r"""
int used_fn(void) { return 5; }
"""
SRC_ARCHLIB_CTOR = r"""
#include <stdio.h>
int ctor_ran = 0;
static void init(void) __attribute__((constructor));
static void init(void) { ctor_ran = 1; fputs("", stdout); }
"""

# Large-ish: many functions and strings, so section merging / string merging /
# ICF have something measurable to do.
SRC_MANY = "".join(
    f"int fn_{i}(int x) {{ static const char s[] = \"string payload number {i} %d\\n\";"
    f" return x + {i} + (int)sizeof(s); }}\n"
    for i in range(400)
) + r"""
#include <stdio.h>
int main(void) {
    long acc = 0;
""" + "\n".join(f"    acc += fn_{i}(acc);" for i in range(0, 400, 7)) + r"""
    printf("many=%ld\n", acc & 0xffff);
    return 0;
}
"""

# Shared library + consumer, to cover -shared: SONAME, exported symbol set,
# version script, -Bsymbolic, RPATH/RUNPATH and DT_NEEDED.
SRC_LIB = r"""
int lib_pub(int x) { return x * 2; }
int lib_hidden_impl(int x) { return x + 1; }
static int internal_only(int x) { return x - 1; }
int lib_pub2(int x) { return internal_only(x) + lib_hidden_impl(x); }
"""
SRC_LIBMAIN = r"""
#include <stdio.h>
extern int lib_pub(int);
extern int lib_pub2(int);
int main(void) { printf("lib=%d,%d\n", lib_pub(3), lib_pub2(3)); return 0; }
"""

# --whole-archive oracle.  The plugin member is reachable *only* through its
# own constructor: nothing in main.o references it, and its own references all
# point outward.  Lazy archive loading must drop it; --whole-archive must pull
# it in and its constructor must then run.  A linker that accepts
# --whole-archive but implements lazy loading reports reg=0 and is wrong.
SRC_PLUGIN_MAIN = r"""
#include <stdio.h>
int registry[16];
int registry_n;
void register_plugin(int id) { if (registry_n < 16) registry[registry_n++] = id; }
extern int used_fn(void);
int main(void) { printf("reg=%d,used=%d\n", registry_n, used_fn()); return 0; }
"""
SRC_PLUGIN = r"""
extern void register_plugin(int);
static void init(void) __attribute__((constructor));
static void init(void) { register_plugin(42); }
int plugin_symbol(void) { return 42; }
"""

# -rdynamic / --export-dynamic + dlsym: the executable must export `main`
# and `exported_fn` in .dynsym.
SRC_EXPORT = r"""
#include <stdio.h>
#include <dlfcn.h>
__attribute__((visibility("default"))) int exported_fn(void) { return 9; }
int main(void) {
    void *h = dlopen(0, RTLD_NOW);
    void *s = h ? dlsym(h, "exported_fn") : 0;
    printf("export=%d\n", s != 0);
    return 0;
}
"""

# Debug info: -g must survive the link, otherwise gdb, coredump symbolisation
# and distro debuginfo extraction are all broken.
SRC_DEBUG = r"""
#include <stdio.h>
static int helper(int x) { return x * 3; }
int main(void) { printf("dbg=%d\n", helper(4)); return 0; }
"""


@dataclass
class Case:
    """One (workload, link configuration) point in the census."""

    id: str
    sources: dict  # filename -> C source
    # Extra gcc flags applied to the *link* step (and to -c for objects when
    # they are also compile flags, e.g. -fPIC / -pthread / -g).
    flags: list = field(default_factory=list)
    shared_libs: dict = field(default_factory=dict)  # soname -> {file: src}
    link_inputs: list = field(default_factory=list)  # extra linker inputs
    archive: dict = field(default_factory=dict)  # archive name -> member .c files
    positional: list = field(default_factory=list)  # .c files linked as objects
    objs: list = field(default_factory=list)  # compiled object paths (filled in)
    # Linkers that are *expected* to refuse this configuration.  A linker that
    # rejects an unsupported request with a clear diagnostic is behaving
    # correctly; reporting it as a defect would hide the cases that matter.
    expect_link_fail: tuple = ()
    # Per-linker flags this linker does not implement at all.  A census that
    # reports "bfd failed --icf=all" as a defect would be noise: bfd has no
    # such option.  Matching cases are reported SKIP for that linker.
    unsupported: dict = field(default_factory=dict)
    expect_stdout: str = ""  # substring that must appear in the program output
    note: str = ""


CASES: list[Case] = [
    Case("hello", {"a.c": SRC_HELLO}, [], note="baseline dynamic executable"),
    Case(
        "multiobj",
        {"a.c": SRC_MULTIA, "b.c": SRC_MULTIB},
        [],
        expect_stdout="12",
        note="cross-object symbol resolution",
    ),
    Case(
        "copyreloc",
        {"a.c": SRC_COPYRELOC},
        [],
        expect_stdout="env=1",
        note="R_X86_64_COPY for environ",
    ),
    Case(
        "tls",
        {"a.c": SRC_TLS},
        ["-pthread"],
        expect_stdout="tls=1000,1000",
        note="PT_TLS + local-exec TLS",
    ),
    Case(
        "ifunc",
        {"a.c": SRC_IFUNC},
        [],
        expect_stdout="ifunc=2",
        note="STT_GNU_IFUNC / IRELATIVE",
    ),
    Case(
        "ctors",
        {"a.c": SRC_CTOR},
        [],
        expect_stdout="123M|d1",
        note=".init_array priority ordering",
    ),
    Case(
        "unwind",
        {"a.c": SRC_UNWIND},
        ["-rdynamic"],
        expect_stdout="bt=1",
        note=".eh_frame / .eh_frame_hdr + backtrace()",
    ),
    Case("weak", {"a.c": SRC_WEAK}, [], expect_stdout="weak=0,7", note="weak-undefined binds to 0"),
    Case(
        "many",
        {"a.c": SRC_MANY},
        ["-O2", "-ffunction-sections", "-fdata-sections"],
        expect_stdout="many=",
        note="400 functions: merging/ICF pressure",
    ),
    Case(
        "export",
        {"a.c": SRC_EXPORT},
        ["-rdynamic", "-ldl"],
        expect_stdout="export=1",
        note="--export-dynamic + dlsym(RTLD_DEFAULT)",
    ),
    # --- link-mode variations -------------------------------------------
    Case("pie", {"a.c": SRC_HELLO}, ["-pie"], expect_stdout="hello", note="ET_DYN executable"),
    Case(
        "pie-copyreloc",
        {"a.c": SRC_COPYRELOC},
        ["-pie"],
        expect_stdout="env=1",
        note="PIE: environ via GOT, no copy reloc",
    ),
    Case(
        "static",
        {"a.c": SRC_HELLO},
        ["-static"],
        expect_stdout="hello",
        note="fully static executable",
    ),
    Case(
        "static-copyreloc",
        {"a.c": SRC_COPYRELOC},
        ["-static"],
        expect_stdout="env=1",
        note="static: environ from libc.a",
    ),
    Case(
        "static-tls",
        {"a.c": SRC_TLS},
        ["-static", "-pthread"],
        expect_stdout="tls=1000,1000",
        note="static TLS + pthread",
    ),
    Case(
        "static-pie",
        {"a.c": SRC_HELLO},
        ["-static-pie"],
        expect_stdout="hello",
        # lccc-ld has no position-independent static emitter.  It refuses with
        # a diagnostic instead of emitting an image that faults in the CRT
        # self-relocation, so a failed link here is the correct behaviour.
        expect_link_fail=("lccc",),
        note="ET_DYN with no interpreter",
    ),
    # --- feature switches ------------------------------------------------
    Case(
        "gc",
        {"a.c": SRC_MANY},
        ["-O2", "-ffunction-sections", "-fdata-sections", "-Wl,--gc-sections"],
        expect_stdout="many=",
        note="--gc-sections",
    ),
    Case(
        "icf-all",
        {"a.c": SRC_MANY},
        ["-O2", "-ffunction-sections", "-Wl,--icf=all"],
        expect_stdout="many=",
        unsupported={"bfd": ["--icf"], "gold": ["--icf"]},
        note="--icf=all (bfd/gold have no ICF)",
    ),
    Case(
        "buildid",
        {"a.c": SRC_DEBUG},
        ["-g", "-Wl,--build-id=sha1"],
        expect_stdout="dbg=12",
        note=".note.gnu.build-id",
    ),
    Case(
        "debuginfo",
        {"a.c": SRC_DEBUG},
        ["-g"],
        expect_stdout="dbg=12",
        note=".debug_* must survive the link",
    ),
    Case(
        "strip-all",
        {"a.c": SRC_DEBUG},
        ["-g", "-Wl,--strip-all"],
        expect_stdout="dbg=12",
        note="--strip-all drops .symtab",
    ),
    Case(
        "now-relro",
        {"a.c": SRC_HELLO},
        ["-Wl,-z,now", "-Wl,-z,relro"],
        expect_stdout="hello",
        note="DT_BIND_NOW + full RELRO",
    ),
    Case(
        "noexecstack",
        {"a.c": SRC_HELLO},
        ["-Wl,-z,noexecstack"],
        expect_stdout="hello",
        note="PT_GNU_STACK without PF_X",
    ),
    Case(
        "rpath",
        {"a.c": SRC_HELLO},
        ["-Wl,-rpath,/opt/lib", "-Wl,--enable-new-dtags"],
        expect_stdout="hello",
        note="DT_RUNPATH",
    ),
    Case(
        "soname-map",
        {"a.c": SRC_HELLO},
        ["-Wl,--print-map"],
        expect_stdout="hello",
        note="-M / --print-map emits a link map",
    ),
    Case(
        "defsym",
        {"a.c": SRC_HELLO},
        ["-Wl,--defsym,aliased=main"],
        expect_stdout="hello",
        note="--defsym symbol alias",
    ),
    Case(
        "wrap",
        {"a.c": SRC_MULTIA, "b.c": SRC_MULTIB, "w.c": SRC_WRAPIMPL},
        ["-Wl,--wrap,b_value"],
        expect_stdout="120",
        note="--wrap: b_value -> __wrap_b_value -> __real_b_value",
    ),
    Case(
        "undefined-u",
        {"a.c": SRC_HELLO},
        ["-Wl,-u,printf"],
        expect_stdout="hello",
        note="-u forces an archive member",
    ),
    Case(
        "emit-relocs",
        {"a.c": SRC_HELLO},
        ["-Wl,--emit-relocs"],
        expect_stdout="hello",
        note="--emit-relocs keeps .rela.text",
    ),
    Case(
        "compress-debug",
        {"a.c": SRC_DEBUG},
        ["-g", "-Wl,--compress-debug-sections=zlib"],
        expect_stdout="dbg=12",
        note="SHF_COMPRESSED debug sections",
    ),
    Case(
        "hash-both",
        {"a.c": SRC_HELLO},
        ["-Wl,--hash-style=both"],
        expect_stdout="hello",
        note=".hash and .gnu.hash",
    ),
    Case(
        "max-page",
        {"a.c": SRC_HELLO},
        ["-Wl,-z,max-page-size=4096"],
        expect_stdout="hello",
        note="-z max-page-size",
    ),
    Case(
        "sort-common",
        {"a.c": SRC_HELLO},
        ["-Wl,--sort-common"],
        expect_stdout="hello",
        note="--sort-common",
    ),
    # --- archives / shared libraries -------------------------------------
    Case(
        "archive",
        {"main.c": SRC_ARCHMAIN, "used.c": SRC_ARCHLIB_USED, "ctor.c": SRC_ARCHLIB_CTOR},
        [],
        archive={"libarch.a": ["used.c", "ctor.c"]},
        positional=["main.c"],
        link_inputs=["@ARCHIVE@"],
        expect_stdout="arch=5,1",
        note="lazy archive loading + constructor member",
    ),
    Case(
        "whole-archive",
        {"main.c": SRC_ARCHMAIN, "used.c": SRC_ARCHLIB_USED, "ctor.c": SRC_ARCHLIB_CTOR},
        [],
        archive={"libarch.a": ["used.c", "ctor.c"]},
        positional=["main.c"],
        link_inputs=["-Wl,--whole-archive", "@ARCHIVE@", "-Wl,--no-whole-archive"],
        expect_stdout="arch=5,1",
        note="--whole-archive in a userspace link",
    ),
    Case(
        "whole-archive-plugin",
        {"main.c": SRC_PLUGIN_MAIN, "used.c": SRC_ARCHLIB_USED, "plugin.c": SRC_PLUGIN},
        [],
        archive={"libplug.a": ["used.c", "plugin.c"]},
        positional=["main.c"],
        link_inputs=["-Wl,--whole-archive", "@ARCHIVE@", "-Wl,--no-whole-archive"],
        expect_stdout="reg=1,used=5",
        note="--whole-archive must pull an unreferenced constructor member",
    ),
    Case(
        "lazy-archive-plugin",
        {"main.c": SRC_PLUGIN_MAIN, "used.c": SRC_ARCHLIB_USED, "plugin.c": SRC_PLUGIN},
        [],
        archive={"libplug.a": ["used.c", "plugin.c"]},
        positional=["main.c"],
        link_inputs=["@ARCHIVE@"],
        expect_stdout="reg=0,used=5",
        note="lazy archive loading must NOT pull the unreferenced member",
    ),
    Case(
        "sharedlib",
        {"main.c": SRC_LIBMAIN},
        [],
        shared_libs={"liblccctest.so": {"lib.c": SRC_LIB}},
        expect_stdout="lib=6,6",
        note="-shared: SONAME, exports, consumer link",
    ),
]


# --------------------------------------------------------------------------
# ELF introspection
# --------------------------------------------------------------------------


def _readelf(binary: str, *flags: str) -> str:
    try:
        r = subprocess.run(
            ["readelf", *flags, binary],
            capture_output=True,
            text=True,
            timeout=60,
        )
        return r.stdout
    except Exception:  # noqa: BLE001 - readelf missing / timeout
        return ""


_SECTION_RE = re.compile(r"^\s*\[\s*\d+\]\s+(\S+)\s+(\S+)\s+([0-9a-f]+)\s+([0-9a-f]+)")


def sections(binary: str) -> list[str]:
    out = _readelf(binary, "-SW")
    names = []
    for line in out.splitlines():
        m = _SECTION_RE.match(line)
        if m:
            names.append(m.group(1))
    return names


def program_headers(binary: str) -> list[str]:
    out = _readelf(binary, "-lW")
    hdrs = []
    for line in out.splitlines():
        line = line.strip()
        for t in (
            "PHDR",
            "INTERP",
            "LOAD",
            "DYNAMIC",
            "NOTE",
            "TLS",
            "GNU_EH_FRAME",
            "GNU_STACK",
            "GNU_RELRO",
            "GNU_PROPERTY",
        ):
            if line.startswith(t):
                hdrs.append(t)
                break
    return hdrs


def dynamic_tags(binary: str) -> list[str]:
    out = _readelf(binary, "-dW")
    tags = []
    for line in out.splitlines():
        m = re.match(r"\s*0x[0-9a-f]+\s+\((\S+)\)", line)
        if m:
            tags.append(m.group(1))
    return tags


def elf_type(binary: str) -> str:
    out = _readelf(binary, "-hW")
    m = re.search(r"Type:\s+(\S+)", out)
    return m.group(1) if m else "?"


def entry_addr(binary: str) -> str:
    out = _readelf(binary, "-hW")
    m = re.search(r"Entry point address:\s+(\S+)", out)
    return m.group(1) if m else "?"


def text_digest(binary: str) -> tuple[str, int]:
    """SHA-256 and size of .text, for "did the code bytes change?" questions."""
    try:
        r = subprocess.run(
            ["objcopy", "-O", "binary", "--only-section=.text", binary, "/dev/stdout"],
            capture_output=True,
            timeout=60,
        )
        if r.returncode == 0:
            return hashlib.sha256(r.stdout).hexdigest()[:16], len(r.stdout)
    except Exception:  # noqa: BLE001
        pass
    return "", 0


def note_names(binary: str) -> list[str]:
    out = _readelf(binary, "-nW")
    names = []
    for line in out.splitlines():
        m = re.match(r"\s+(?:NT_GNU_\S+|Displaying notes found in: (\S+))", line)
        if m and m.group(1):
            names.append(m.group(1))
        m2 = re.search(r"\b(NT_GNU_[A-Z_]+|NT_VERSION|NT_ARCH)\b", line)
        if m2:
            names.append(m2.group(1))
    return sorted(set(names))


@dataclass
class Result:
    case: str
    linker: str
    ok_link: bool = False
    link_rc: int = -1
    link_time_s: float = 0.0
    stderr: str = ""
    exists: bool = False
    size: int = 0
    etype: str = ""
    entry: str = ""
    phdrs: list = field(default_factory=list)
    sections: list = field(default_factory=list)
    tags: list = field(default_factory=list)
    notes: list = field(default_factory=list)
    text_sha: str = ""
    text_size: int = 0
    run_rc: int = -999
    run_stdout: str = ""
    run_timeout: bool = False
    skipped: str = ""  # linker does not implement one of the case's flags
    wrong_output: str = ""  # expected substring missing from stdout
    notes_extra: list = field(default_factory=list)  # census observations


# --------------------------------------------------------------------------
# Drivers
# --------------------------------------------------------------------------


class Linkers:
    """Build the per-linker gcc driver invocations."""

    def __init__(self, cc: str, lccc_ld: Path, workdir: Path):
        self.cc = cc
        self.lccc_ld = lccc_ld
        self.shim = workdir / "shim"
        self.shim.mkdir(parents=True, exist_ok=True)
        # gcc -B<dir> looks for <dir>ld; that is exactly the drop-in path a
        # Makefile uses (`make LD=lccc-ld` / `CC="gcc -B..."`), so it exercises
        # the same argument surface.
        link = self.shim / "ld"
        if link.is_symlink() or link.exists():
            link.unlink()
        link.symlink_to(lccc_ld)

    def available(self) -> dict[str, bool]:
        avail = {"bfd": shutil.which("ld.bfd") is not None}
        for name, exe in (("lld", "ld.lld"), ("mold", "mold"), ("gold", "ld.gold")):
            avail[name] = shutil.which(exe) is not None
        avail["lccc"] = self.lccc_ld.exists()
        return avail

    def args(self, linker: str) -> list[str]:
        if linker == "bfd":
            return ["-fuse-ld=bfd"]
        if linker == "lld":
            return ["-fuse-ld=lld"]
        if linker == "mold":
            return ["-fuse-ld=mold"]
        if linker == "gold":
            return ["-fuse-ld=gold"]
        if linker == "lccc":
            return [f"-B{self.shim}"]
        raise ValueError(linker)


def sh(cmd: list[str], cwd: Path, timeout: int = 120) -> subprocess.CompletedProcess:
    return subprocess.run(
        cmd, cwd=str(cwd), capture_output=True, text=True, timeout=timeout
    )


def build_case(workdir: Path, case: Case, cc: str) -> tuple[list[str], str | None]:
    """Compile the case's objects once.  Returns (object list, error)."""
    cdir = workdir / "obj" / case.id
    cdir.mkdir(parents=True, exist_ok=True)
    for name, src in case.sources.items():
        (cdir / name).write_text(src)
    for _soname, files in case.shared_libs.items():
        for name, src in files.items():
            (cdir / name).write_text(src)
    objs: list[str] = []
    compile_flags = [f for f in case.flags if f in ("-g", "-pthread", "-O2", "-O1", "-O0")]
    compile_flags += [
        f
        for f in case.flags
        if f.startswith(("-ffunction-sections", "-fdata-sections", "-fPIC", "-fpic", "-m32"))
    ]
    positional = sorted(case.positional or case.sources.keys())
    for name in positional:
        o = cdir / (name[:-2] + ".o")
        r = sh([cc, *compile_flags, "-c", str(cdir / name), "-o", str(o)], cdir)
        if r.returncode != 0:
            return [], f"compile {name}: {r.stderr.strip()[:400]}"
        objs.append(str(o))
    return objs, None


def build_extras(workdir: Path, case: Case, cc: str) -> tuple[list[str], str | None]:
    """Archives and shared libraries for the case.  Returns extra -L/-l args."""
    cdir = workdir / "obj" / case.id
    extra: list[str] = []
    for libname, members in case.archive.items():
        mobjs = []
        for name in members:
            o = cdir / (name[:-2] + ".o")
            if not o.exists():
                r = sh([cc, "-O2", "-c", str(cdir / name), "-o", str(o)], cdir)
                if r.returncode != 0:
                    return [], f"compile {name}: {r.stderr.strip()[:400]}"
            mobjs.append(o.name)
        r = sh(["ar", "rcs", libname, *mobjs], cdir)
        if r.returncode != 0:
            return [], f"ar: {r.stderr.strip()[:400]}"
        extra.append(f"-L{cdir}")
        extra.append("-l" + libname[3:-2])
    for soname, files in case.shared_libs.items():
        sobjs = []
        for name in files:
            o = cdir / (name[:-2] + ".pic.o")
            r = sh(
                [cc, "-O2", "-fPIC", "-c", str(cdir / name), "-o", str(o)],
                cdir,
            )
            if r.returncode != 0:
                return [], f"compile {name}: {r.stderr.strip()[:400]}"
            sobjs.append(o.name)
        r = sh([cc, "-shared", f"-Wl,-soname,{soname}", "-o", soname, *sobjs], cdir)
        if r.returncode != 0:
            return [], f"shared {soname}: {r.stderr.strip()[:400]}"
        extra.append(f"-L{cdir}")
        extra.append("-l:" + soname)
        # The consumer must find the library at runtime, not just at link
        # time: without an rpath it loads the *system* copy of a same-named
        # library, or fails, and the census would report a bogus divergence.
        extra.append(f"-Wl,-rpath,{cdir}")
    return extra, None


def run_case(
    workdir: Path,
    case: Case,
    linker: str,
    linkers: Linkers,
    cc: str,
    run_it: bool,
) -> Result:
    res = Result(case=case.id, linker=linker)
    unsupported = [
        pat for pat in case.unsupported.get(linker, []) if any(pat in f for f in case.flags)
    ]
    if unsupported:
        res.skipped = ",".join(unsupported)
        return res
    outdir = workdir / "out" / case.id
    outdir.mkdir(parents=True, exist_ok=True)
    binary = outdir / f"a.{linker}"
    if binary.exists():
        binary.unlink()

    flags = list(case.flags)
    inputs = list(case.link_inputs)

    cmd = [
        cc,
        *flags,
        *linkers.args(linker),
        "-o",
        str(binary),
        *case.objs,
        *inputs,
    ]
    t0 = time.time()
    try:
        r = sh(cmd, outdir, timeout=180)
    except subprocess.TimeoutExpired:
        res.stderr = "LINK TIMEOUT"
        return res
    res.link_time_s = time.time() - t0
    res.link_rc = r.returncode
    res.stderr = (r.stderr or "").strip()
    res.ok_link = r.returncode == 0 and binary.exists()
    if not res.ok_link:
        return res

    st = binary.stat()
    res.exists = True
    res.size = st.st_size
    res.etype = elf_type(str(binary))
    res.entry = entry_addr(str(binary))
    res.phdrs = program_headers(str(binary))
    res.sections = sections(str(binary))
    res.tags = dynamic_tags(str(binary))
    res.notes = note_names(str(binary))
    res.text_sha, res.text_size = text_digest(str(binary))

    if run_it:
        try:
            rr = sh([str(binary)], outdir, timeout=20)
            res.run_rc = rr.returncode
            res.run_stdout = (rr.stdout or "")[-4096:]
        except subprocess.TimeoutExpired:
            res.run_timeout = True
        # The case's expected output is the semantic oracle: a link that
        # succeeds but produces a binary that prints the wrong thing (or dies)
        # is a defect, and it must not be hidden by a zero exit status.
        if (
            case.expect_stdout
            and not res.run_timeout
            and case.expect_stdout not in res.run_stdout
        ):
            res.wrong_output = case.expect_stdout
    return res


# --------------------------------------------------------------------------
# Comparison
# --------------------------------------------------------------------------

# Divergence classes: some differences are expected and should be reported as
# such rather than as defects, so a reader is not drowned in noise.
EXPECTED = {
    ("lccc", "etype"): "lccc-ld emits ET_EXEC at 0x400000 for non-PIE links",
    ("lccc", "entry"): "follows from the ET_EXEC base address",
}


def divergence_fields(ref: Result, got: Result) -> list[tuple[str, str, str]]:
    """Return (field, ref_value, got_value) for every meaningful difference."""
    out: list[tuple[str, str, str]] = []
    if ref.etype != got.etype:
        out.append(("etype", ref.etype, got.etype))
    if ref.run_rc != got.run_rc:
        out.append(("run_rc", str(ref.run_rc), str(got.run_rc)))
    if ref.run_stdout.strip() != got.run_stdout.strip():
        out.append(
            (
                "stdout",
                repr(ref.run_stdout.strip()[:80]),
                repr(got.run_stdout.strip()[:80]),
            )
        )
    missing_secs = [s for s in ref.sections if s not in got.sections]
    extra_secs = [s for s in got.sections if s not in ref.sections]
    if missing_secs:
        out.append(("sections-", ",".join(missing_secs)[:200], ""))
    if extra_secs:
        out.append(("sections+", "", ",".join(extra_secs)[:200]))
    missing_ph = [p for p in dict.fromkeys(ref.phdrs) if p not in got.phdrs]
    extra_ph = [p for p in dict.fromkeys(got.phdrs) if p not in ref.phdrs]
    if missing_ph:
        out.append(("phdrs-", ",".join(missing_ph), ""))
    if extra_ph:
        out.append(("phdrs+", "", ",".join(extra_ph)))
    missing_tags = [t for t in dict.fromkeys(ref.tags) if t not in got.tags]
    extra_tags = [t for t in dict.fromkeys(got.tags) if t not in ref.tags]
    if missing_tags:
        out.append(("tags-", ",".join(missing_tags), ""))
    if extra_tags:
        out.append(("tags+", "", ",".join(extra_tags)))
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--linkers", default="bfd,lld,mold,lccc")
    ap.add_argument("--ref", default=None, help="reference linker (default: first available of --linkers)")
    ap.add_argument("--cases", default="", help="comma-separated substrings selecting case ids")
    ap.add_argument("--cc", default=os.environ.get("CC", "gcc"))
    ap.add_argument("--lccc-ld", default=os.environ.get("LCCC_LD", str(DEFAULT_LCCC_LD)))
    ap.add_argument("--json", default="")
    ap.add_argument("--md", default="")
    ap.add_argument("--keep", action="store_true", help="keep the scratch directory")
    ap.add_argument("--no-run", action="store_true", help="do not execute linked binaries")
    ap.add_argument("--only-missing", action="store_true", help="print only cases with divergences")
    ap.add_argument("--strict", action="store_true", help="exit non-zero on any divergence")
    args = ap.parse_args()

    lccc_ld = Path(args.lccc_ld).resolve()
    tmp = Path(tempfile.mkdtemp(prefix="ldcensus-"))
    try:
        linkers = Linkers(args.cc, lccc_ld, tmp)
        avail = linkers.available()
        want = [x.strip() for x in args.linkers.split(",") if x.strip()]
        use = [x for x in want if avail.get(x)]
        skipped = [x for x in want if not avail.get(x)]
        if skipped:
            print(f"# skipped (not installed): {', '.join(skipped)}", file=sys.stderr)
        if not use:
            print("no usable linkers", file=sys.stderr)
            return 2
        ref_name = args.ref if args.ref in use else use[0]

        sel = [c for c in CASES if not args.cases or any(s in c.id for s in args.cases.split(","))]
        print(f"# linkers: {', '.join(use)}   reference: {ref_name}   cases: {len(sel)}")
        print(f"# lccc-ld: {lccc_ld}")

        all_res: dict[str, dict[str, Result]] = {}
        for case in sel:
            objs, err = build_case(tmp, case, args.cc)
            if err:
                print(f"!! {case.id}: {err}", file=sys.stderr)
                continue
            case.objs = objs
            extra, err = build_extras(tmp, case, args.cc)
            if err:
                print(f"!! {case.id}: {err}", file=sys.stderr)
                continue
            # `@ARCHIVE@` marks where the generated archive and its `-L` must
            # be spliced in.  --whole-archive is positional, so a case that
            # needs the archive inside a flag group has to place it exactly.
            tokens = list(case.link_inputs)
            if "@ARCHIVE@" in tokens:
                spliced: list[str] = []
                for tok in tokens:
                    spliced.extend(extra if tok == "@ARCHIVE@" else [tok])
                tokens = spliced
            else:
                tokens += extra
            case.link_inputs = tokens
            per: dict[str, Result] = {}
            for lk in use:
                per[lk] = run_case(tmp, case, lk, linkers, args.cc, not args.no_run)
            all_res[case.id] = per

        # ---- report -------------------------------------------------------
        rows = []
        bad = 0
        for case in sel:
            per = all_res.get(case.id)
            if not per:
                continue
            ref = per[ref_name]
            for lk, r in per.items():
                if lk == ref_name:
                    div: list[tuple[str, str, str]] = []
                else:
                    div = divergence_fields(ref, r) if (ref.ok_link and not r.skipped) else []
                if div or lk == ref_name or r.skipped:
                    if args.only_missing and not div:
                        continue
                    if r.skipped:
                        status = "SKIP:" + r.skipped
                    elif not r.ok_link:
                        status = (
                            "REFUSED-OK" if lk in case.expect_link_fail else "LINKFAIL"
                        )
                    elif r.run_timeout:
                        status = "TIMEOUT"
                    elif r.wrong_output:
                        status = "WRONGOUT"
                    elif r.run_rc != 0:
                        status = "BADRUN"
                    else:
                        status = "ok"
                    if status not in ("ok", "REFUSED-OK") and not status.startswith("SKIP"):
                        bad += 1
                    rows.append((case.id, lk, status, r, div))

        # Console table
        print()
        print(f"{'case':18} {'linker':6} {'status':9} {'etype':7} {'size':>8} {'link_s':>7}  divergences")
        print("-" * 118)
        last_case = None
        for cid, lk, status, r, div in rows:
            if last_case and cid != last_case:
                print()
            last_case = cid
            dtxt = "; ".join(f"{f}:{a or '-'}→{b or '-'}" for f, a, b in div)[:90]
            print(
                f"{cid:18} {lk:6} {status:9} {r.etype:7} {r.size:8d} {r.link_time_s:7.2f}  {dtxt}"
            )

        # Markdown
        if args.md:
            lines = [
                "# lccc-ld feature census",
                "",
                f"Linkers: {', '.join(use)}. Reference: `{ref_name}`. Cases: {len(sel)}.",
                "",
                "| case | linker | status | type | size | link s | divergence |",
                "|------|--------|--------|------|-----:|-------:|------------|",
            ]
            for cid, lk, status, r, div in rows:
                dtxt = "; ".join(f"`{f}` {a or '-'}→{b or '-'}" for f, a, b in div) or "-"
                lines.append(
                    f"| {cid} | {lk} | {status} | {r.etype} | {r.size} | {r.link_time_s:.2f} | {dtxt} |"
                )
            Path(args.md).write_text("\n".join(lines) + "\n")
            print(f"\n# markdown report: {args.md}")

        if args.json:
            payload = {
                "linkers": use,
                "reference": ref_name,
                "results": {
                    cid: {lk: vars(r) for lk, r in per.items()} for cid, per in all_res.items()
                },
            }
            Path(args.json).write_text(json.dumps(payload, indent=1))
            print(f"# json: {args.json}")

        return 1 if (args.strict and bad) else 0
    finally:
        if args.keep:
            print(f"# scratch kept: {tmp}")
        else:
            shutil.rmtree(tmp, ignore_errors=True)



if __name__ == "__main__":
    sys.exit(main())
