#!/usr/bin/env python3
"""`-fpack-struct[=<n>]` and x86 `-mcpu=` contracts (DA-07).

Every layout expectation below was measured against GCC 14.2 and GCC 16.2
on x86-64 (the two disagree on nothing here):

  * bare `-fpack-struct` behaves like `__attribute__((packed))` on every
    record: an explicitly aligned member keeps its alignment (offset 8 in
    fixture A), bitfields become a contiguous bit stream, union alignment
    drops to 1, and `#pragma pack` is inert with a `-Wpragmas` warning;
  * `-fpack-struct=<n>` behaves like a `#pragma pack(<n>)` baseline: it
    caps even explicitly aligned members, later pragmas override it, and
    `#pragma pack()` / `#pragma pack(0)` restore it (not natural layout);
  * a value that is not a small power of two (1..16) is a hard error:
    "structure alignment must be a small power of two, not <n>";
  * `-mcpu=<name>` on x86 is the deprecated `-mtune=` alias: accepted for
    names in the tune vocabulary with a deprecation warning, refused for
    bogus names, and it never selects an ISA (assembly identical to the
    `-mtune=` spelling).

The fixture uses only char/int/long long through stdint-width types whose
x86-64 layout is ABI-fixed, so the static asserts are exact, not sampled.
Run: python3 tests/regression/check_pack_struct_mcpu.py \
        --lccc target/fastbuild/lccc [--reference gcc]
"""

import argparse
from pathlib import Path
import subprocess
import sys
import tempfile

# struct S { char c; int i; char d; long long l; }
# struct A { char c; int i __attribute__((aligned(8))); } __attribute__((aligned(16)));
CAP_TEMPLATE = r'''
#include <stdint.h>
struct S { char c; int i; char d; long long l; };
struct A { char c; int i __attribute__((aligned(8))); } __attribute__((aligned(16)));
#define CK(t, sz, al, memb, off) \
  _Static_assert(sizeof(struct t) == (sz) && _Alignof(struct t) == (al) \
      && __builtin_offsetof(struct t, memb) == (off), #t)
CK(S, @S_SIZE@, @S_ALIGN@, i, @S_IOFF@);
_Static_assert(__builtin_offsetof(struct S, l) == @S_LOFF@, "S.l");
CK(A, 16, 16, i, @A_IOFF@);
int main(void) { return 0; }
'''

# (flags, S size, S align, S.i, S.l, A.i) measured on GCC 14.2/16.2.
CAP_CASES = [
    ([], 24, 8, 4, 16, 8),
    (["-fpack-struct=1"], 14, 1, 1, 6, 1),
    (["-fpack-struct=2"], 16, 2, 2, 8, 2),
    (["-fpack-struct=4"], 20, 4, 4, 12, 4),
    (["-fpack-struct=8"], 24, 8, 4, 16, 8),
    (["-fpack-struct=16"], 24, 8, 4, 16, 8),
]

BARE_SOURCE = r'''
#include <stdint.h>
struct S { char c; int i; char d; long long l; };
struct A { char c; int i __attribute__((aligned(8))); } __attribute__((aligned(16)));
struct B { unsigned a:3; unsigned b:5; unsigned c:20; char d; };
union U { char c; long long l; };
struct I { char c; int i; };
struct O { char c; struct I inner; char d; };
_Static_assert(sizeof(struct S) == 14 && _Alignof(struct S) == 1
    && __builtin_offsetof(struct S, i) == 1
    && __builtin_offsetof(struct S, l) == 6, "bare S");
_Static_assert(sizeof(struct A) == 16 && __builtin_offsetof(struct A, i) == 8,
    "explicit member alignment escapes bare -fpack-struct");
_Static_assert(sizeof(struct B) == 5 && __builtin_offsetof(struct B, d) == 4,
    "bare bitfields are a contiguous bit stream");
_Static_assert(sizeof(union U) == 8 && _Alignof(union U) == 1, "bare union");
_Static_assert(sizeof(struct O) == 7 && __builtin_offsetof(struct O, d) == 6,
    "bare nested records");
int main(void) { return 0; }
'''

PRAGMA_AFTER_CAP_SOURCE = r'''
struct S { char c; int i; };
#pragma pack(4)
struct T { char c; int i; };
#pragma pack()
struct U { char c; int i; };
#pragma pack(push, 1)
struct V { char c; int i; };
#pragma pack(pop)
struct W { char c; int i; };
_Static_assert(sizeof(struct S) == 6, "cap 2 applies before any pragma");
_Static_assert(sizeof(struct T) == 8, "pragma pack(4) overrides the cap");
_Static_assert(sizeof(struct U) == 6, "bare reset restores the cap, not natural");
_Static_assert(sizeof(struct V) == 5, "push 1");
_Static_assert(sizeof(struct W) == 6, "pop restores the cap");
int main(void) { return 0; }
'''

RUNTIME_SOURCE = r'''
#include <stdio.h>
struct S { char c; int i; char d; long long l; };
int main(void) {
    printf("%zu %zu %zu %zu\n", sizeof(struct S), _Alignof(struct S),
           __builtin_offsetof(struct S, i), __builtin_offsetof(struct S, l));
    return 0;
}
'''
RUNTIME_EXPECT = {
    (): "24 8 4 16\n",
    ("-fpack-struct",): "14 1 1 6\n",
    ("-fpack-struct=2",): "16 2 2 8\n",
}


def run(cmd, timeout=30):
    r = subprocess.run(cmd, text=True, capture_output=True, timeout=timeout)
    return r.returncode, r.stdout, r.stderr


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc", required=True, help="lccc driver to test")
    ap.add_argument("--reference", help="reference compiler (GCC) for parity legs")
    args = ap.parse_args()
    compilers = [("lccc", args.lccc)] + (
        [("reference", args.reference)] if args.reference else [])
    failures = []

    def check(name, ok, detail=""):
        print(f"{'PASS' if ok else 'FAIL'}: {name}" + ("" if ok else f" -- {detail}"))
        if not ok:
            failures.append(name)

    with tempfile.TemporaryDirectory(prefix="lccc-pack-struct-") as tmp:
        tmp = Path(tmp)

        def compile_and_run(tag, source, flags, compiler, expect_ok=True):
            src = tmp / f"{tag}.c"
            src.write_text(source)
            exe = tmp / f"{tag}.exe"
            rc, out, err = run([compiler, *flags, str(src), "-o", str(exe)])
            if not expect_ok:
                return rc, out, err, None
            if rc != 0:
                return rc, out, err, None
            rc2, out2, err2 = run([str(exe)])
            return rc2, out2, err + err2, exe

        for label, cc in compilers:
            for flags, s_size, s_align, s_ioff, s_loff, a_ioff in CAP_CASES:
                src = (CAP_TEMPLATE.replace("@S_SIZE@", str(s_size))
                       .replace("@S_ALIGN@", str(s_align))
                       .replace("@S_IOFF@", str(s_ioff))
                       .replace("@S_LOFF@", str(s_loff))
                       .replace("@A_IOFF@", str(a_ioff)))
                def _tag_suffix(flags: list[str]) -> str:
                    if not flags:
                        return "none"
                    if "=" in flags[0]:
                        return flags[0].split("=", 1)[1]
                    return flags[0]
                tag = f"cap-{label}-{_tag_suffix(flags)}"
                rc, out, err, _ = compile_and_run(tag, src, flags, cc)
                check(f"{label} cap {flags or 'default'}", rc == 0, err[:300])

            rc, out, err, _ = compile_and_run(f"bare-{label}", BARE_SOURCE,
                                              ["-fpack-struct"], cc)
            check(f"{label} bare -fpack-struct layouts", rc == 0, err[:300])

            rc, out, err, _ = compile_and_run(f"cap-pragma-{label}",
                                              PRAGMA_AFTER_CAP_SOURCE,
                                              ["-fpack-struct=2"], cc)
            check(f"{label} pragma overrides/restores cap", rc == 0, err[:300])

            for flags, expect in RUNTIME_EXPECT.items():
                tag = f"run-{label}-{'_'.join(flags) or 'none'}".replace("=", "")
                rc, out, err, _ = compile_and_run(tag, RUNTIME_SOURCE,
                                                  list(flags), cc)
                check(f"{label} runtime layout {flags or 'default'}",
                      rc == 0 and out == expect, f"rc={rc} out={out!r} err={err[:200]}")

        # lccc-only diagnostics: bare form makes `#pragma pack` inert, with
        # GCC's warning text; invalid caps are hard errors with GCC's words.
        src = tmp / "bare-pragma.c"
        src.write_text("#pragma pack(4)\nstruct S { char c; int i; };\n"
                       "_Static_assert(sizeof(struct S) == 5, \"pragma inert\");\n"
                       "int main(void) { return 0; }\n")
        rc, out, err = run([args.lccc, "-fpack-struct", str(src), "-o", str(tmp / "bp")])
        check("lccc bare: #pragma pack ignored with warning",
              rc == 0 and "has no effect with '-fpack-struct'" in err,
              f"rc={rc} err={err[:300]}")

        trivial = tmp / "trivial.c"
        trivial.write_text("struct S { char c; int i; };\nint main(void){return 0;}\n")
        for bad in ("0", "3", "32", "bogus"):
            rc, out, err = run([args.lccc, f"-fpack-struct={bad}", "-c",
                                str(trivial), "-o", str(tmp / "t.o")])
            check(f"lccc -fpack-struct={bad} refused",
                  rc != 0 and "small power of two" in err, f"rc={rc} err={err[:200]}")
        if args.reference:
            for bad in ("0", "3", "32"):
                rc, out, err = run([args.reference, f"-fpack-struct={bad}", "-c",
                                    str(trivial), "-o", str(tmp / "t.o")])
                check(f"reference -fpack-struct={bad} refused", rc != 0,
                      f"rc={rc}")

        # -mcpu= alias parity: accepted with a deprecation warning, bogus
        # refused, and codegen identical to the -mtune= spelling.
        for label, cc in compilers:
            rc, out, err = run([cc, "-mcpu=core2", "-c", str(trivial),
                                "-o", str(tmp / "m.o")])
            check(f"{label} -mcpu=core2 accepted", rc == 0, err[:200])
            check(f"{label} -mcpu=core2 warns deprecated",
                  "deprecated" in err, err[:200])
            rc, out, err = run([cc, "-mcpu=bogus123", "-c", str(trivial),
                                "-o", str(tmp / "m.o")])
            check(f"{label} -mcpu=bogus123 refused", rc != 0, f"rc={rc}")
        asm = {}
        for tag, flag in (("mcpu", "-mcpu=core2"), ("mtune", "-mtune=core2")):
            out_s = tmp / f"{tag}.s"
            rc, out, err = run([args.lccc, flag, "-S", str(trivial),
                                "-o", str(out_s)])
            asm[tag] = out_s.read_text() if rc == 0 else ""
        check("lccc -mcpu assembly identical to -mtune",
              bool(asm["mcpu"]) and asm["mcpu"] == asm["mtune"])

        # -mcpu= with a tune valid only for i686 must be refused on x86-64.
        rc, out, err = run([args.lccc, "-mcpu=pentium4", "-c",
                            str(trivial), "-o", str(tmp / "m3.o")])
        check("lccc -mcpu=pentium4 refused on x86-64 (i686-only tune)",
              rc != 0 and ("pentium4" in err or "unsupported" in err),
              f"rc={rc} err={err[:200]}")

        # -mcpu= error message must name -mcpu=, not -mtune=.
        rc, out, err = run([args.lccc, "-mcpu=bogus123", "-c",
                            str(trivial), "-o", str(tmp / "m4.o")])
        check("lccc -mcpu bogus error names -mcpu= not -mtune=",
              rc != 0 and "-mcpu=bogus123" in err and "-mtune=bogus123" not in err,
              f"rc={rc} err={err[:200]}")

        # -mtune= error message must name -mtune=, not -mcpu=.
        rc, out, err = run([args.lccc, "-mtune=bogus123", "-c",
                            str(trivial), "-o", str(tmp / "m5.o")])
        check("lccc -mtune bogus error names -mtune= not -mcpu=",
              rc != 0 and "-mtune=bogus123" in err and "-mcpu=bogus123" not in err,
              f"rc={rc} err={err[:200]}")

        # -mcpu=native is accepted (CPUID-based tune resolution).
        rc, out, err = run([args.lccc, "-mcpu=native", "-c",
                            str(trivial), "-o", str(tmp / "m6.o")])
        check("lccc -mcpu=native accepted", rc == 0,
              f"rc={rc} err={err[:200]}")

        # -fsyntax-only must agree with full compilation for pack-struct.
        for flags in [[], ["-fpack-struct"], ["-fpack-struct=2"],
                      ["-fpack-struct=4"]]:
            src_syntax = tmp / f"syntax_{'_'.join(flags) or 'default'}.c"
            src_syntax.write_text(CAP_TEMPLATE.replace("@S_SIZE@", "24")
                                  .replace("@S_ALIGN@", "8")
                                  .replace("@S_IOFF@", "4")
                                  .replace("@S_LOFF@", "16")
                                  .replace("@A_IOFF@", "8"))
            rc_full, _, _ = run([args.lccc, *flags, str(src_syntax),
                                 "-o", str(tmp / "syntax_full.o")])
            rc_syntax, _, _ = run([args.lccc, *flags, "-fsyntax-only",
                                   str(src_syntax)])
            check(f"lccc -fsyntax-only agrees {' '.join(flags) or 'default'}",
                  rc_full == 0 and rc_syntax == 0,
                  f"full rc={rc_full} syntax rc={rc_syntax}")

        # -fpack-struct=16 (max valid cap) behaves like default (natural alignment).
        src16 = tmp / "pack16.c"
        src16.write_text(CAP_TEMPLATE.replace("@S_SIZE@", "24")
                         .replace("@S_ALIGN@", "8")
                         .replace("@S_IOFF@", "4")
                         .replace("@S_LOFF@", "16")
                         .replace("@A_IOFF@", "16"))
        rc, _, _ = run([args.lccc, "-fpack-struct=16", str(src16),
                        "-o", str(tmp / "pack16.o")])
        check("lccc -fpack-struct=16 compiles (max valid cap)", rc == 0)

        # #pragma pack(push) / pack(pop) with -fpack-struct=2 cap.
        src_pushpop = tmp / "pushpop.c"
        src_pushpop.write_text(r'''
#include <stdint.h>
#pragma pack(push, 1)
struct P1 { char c; int i; };
#pragma pack(push, 2)
struct P2 { char c; int i; };
#pragma pack(pop)
struct P3 { char c; int i; };
#pragma pack(pop)
struct P4 { char c; int i; };
_Static_assert(sizeof(struct P1) == 5, "P1 pack 1");
_Static_assert(sizeof(struct P2) == 6, "P2 pack 2 (cap overrides)");
_Static_assert(sizeof(struct P3) == 6, "P3 pop restores cap");
_Static_assert(sizeof(struct P4) == 6, "P4 pop restores cap again");
int main(void) { return 0; }
''')
        rc, _, err = run([args.lccc, "-fpack-struct=2", str(src_pushpop),
                          "-o", str(tmp / "pushpop.o")])
        check("lccc #pragma pack push/pop with cap baseline", rc == 0,
              f"rc={rc} err={err[:300]}")

        # #pragma pack(push) with -fpack-struct (bare): all inert, stack untouched.
        src_bare_pushpop = tmp / "bare_pushpop.c"
        src_bare_pushpop.write_text(r'''
#pragma pack(push, 1)
struct X { char c; int i; };
#pragma pack(pop)
struct Y { char c; int i; };
_Static_assert(sizeof(struct X) == 5, "bare: push 1 inert");
_Static_assert(sizeof(struct Y) == 5, "bare: pop inert");
int main(void) { return 0; }
''')
        rc, out, err = run([args.lccc, "-fpack-struct", str(src_bare_pushpop),
                            "-o", str(tmp / "bare_pushpop.o")])
        check("lccc bare -fpack-struct: pragma push/pop all inert",
              rc == 0 and "has no effect with '-fpack-struct'" in err,
              f"rc={rc} err={err[:300]}")

        # Invalid -fpack-struct values at boundary: 17 (power of 2 but > 16).
        for bad in ("17", "64", "128"):
            rc, out, err = run([args.lccc, f"-fpack-struct={bad}", "-c",
                                str(trivial), "-o", str(tmp / f"bad_{bad}.o")])
            check(f"lccc -fpack-struct={bad} refused (>16)",
                  rc != 0 and "small power of two" in err,
                  f"rc={rc} err={err[:200]}")

        # -mcpu= on the reference compiler (if provided) also rejects bogus.
        if args.reference:
            rc, _, err = run([args.reference, "-mcpu=bogus123", "-c",
                              str(trivial), "-o", str(tmp / "ref_m.o")])
            check(f"reference -mcpu=bogus123 refused", rc != 0,
                  f"rc={rc} err={err[:200]}")

    print(f"\n{ 'FAIL' if failures else 'PASS' }: {len(failures)} failure(s)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
