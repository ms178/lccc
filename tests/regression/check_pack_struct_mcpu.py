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
Run: python3 tests/regression/check_pack_struct_mcpu.py \\
        --lccc target/fastbuild/lccc [--reference gcc]
"""

import argparse
from pathlib import Path
import subprocess
import sys
import tempfile

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

# Array member under -fpack-struct=2: { char c; int arr[3]; char d; }
# pack(2): c@0, arr@2 (align 2, size 12), d@14, sizeof=16, align=2
ARRAY_SOURCE = r'''
#include <stdint.h>
struct Arr { char c; int arr[3]; char d; };
_Static_assert(sizeof(struct Arr) == 16, "pack(2) array sizeof");
_Static_assert(_Alignof(struct Arr) == 2, "pack(2) array align");
_Static_assert(__builtin_offsetof(struct Arr, c) == 0, "pack(2) array c off");
_Static_assert(__builtin_offsetof(struct Arr, arr) == 2, "pack(2) array arr off");
_Static_assert(__builtin_offsetof(struct Arr, d) == 14, "pack(2) array d off");
int main(void) { return 0; }
'''

# __attribute__((aligned(8))) member under -fpack-struct=2: capped at 2
ALIGNED_MEMBER_SOURCE = r'''
#include <stdint.h>
struct Am { char c; int i __attribute__((aligned(8))); char d; };
_Static_assert(sizeof(struct Am) == 8, "pack(2) aligned member capped");
_Static_assert(__builtin_offsetof(struct Am, i) == 2, "pack(2) aligned member off");
int main(void) { return 0; }
'''

# #pragma pack(0) restores the command-line baseline (not natural alignment
# when -fpack-struct=<n> is set). Without a command-line cap, pack(0)
# restores natural alignment — all vendors agree on this.
PRAGMA_ZERO_RESTORE_SOURCE = r'''
struct Base { char c; int i; };          /* natural alignment: sizeof 8 */
#pragma pack(2)
struct Mid { char c; int i; };           /* pack(2): sizeof 6 */
#pragma pack(0)
struct After { char c; int i; };         /* pack(0) restores natural: sizeof 8 */
_Static_assert(sizeof(struct Base) == 8, "natural alignment");
_Static_assert(sizeof(struct Mid) == 6, "pack(2) applies");
_Static_assert(sizeof(struct After) == 8, "pack(0) restores natural");
int main(void) { return 0; }
'''

# #pragma pack(push, 0) with no command-line cap: push current (natural),
# reset to natural — all vendors agree.
PRAGMA_PUSH_ZERO_SOURCE = r'''
struct First { char c; int i; };         /* natural alignment: sizeof 8 */
#pragma pack(push, 0)
struct Second { char c; int i; };        /* push(0) keeps natural: sizeof 8 */
_Static_assert(sizeof(struct First) == 8, "natural alignment");
_Static_assert(sizeof(struct Second) == 8, "push(0) keeps natural");
int main(void) { return 0; }
'''

# Bare -fpack-struct: #pragma pack(4) is inert, struct packed to 5 bytes
BARE_PRAGMA_SOURCE = r'''
#pragma pack(4)
struct S { char c; int i; };
_Static_assert(sizeof(struct S) == 5, "pragma inline");
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

        # ---- CAP_CASES: layout table against every compiler ----
        for label, cc in compilers:
            for flags, s_size, s_align, s_ioff, s_loff, a_ioff in CAP_CASES:
                src = (CAP_TEMPLATE.replace("@S_SIZE@", str(s_size))
                       .replace("@S_ALIGN@", str(s_align))
                       .replace("@S_IOFF@", str(s_ioff))
                       .replace("@S_LOFF@", str(s_loff))
                       .replace("@A_IOFF@", str(a_ioff)))
                if flags:
                    suffix = flags[0].split("=", 1)[1] if "=" in flags[0] else flags[0]
                else:
                    suffix = "none"
                tag = f"cap-{label}-{suffix}"
                rc, out, err, _ = compile_and_run(tag, src, flags, cc)
                check(f"{label} cap {flags or 'default'}", rc == 0, err[:300])

            rc, out, err, _ = compile_and_run(f"bare-{label}", BARE_SOURCE,
                                              ["-fpack-struct"], cc)
            check(f"{label} bare -fpack-struct layouts", rc == 0, err[:300])

            rc, out, err, _ = compile_and_run(f"cap-pragma-{label}",
                                              PRAGMA_AFTER_CAP_SOURCE,
                                              ["-fpack-struct=2"], cc)
            check(f"{label} pragma overrides/restores cap", rc == 0, err[:300])

            # Array member under pack(2)
            rc, out, err, _ = compile_and_run(f"array-{label}", ARRAY_SOURCE,
                                              ["-fpack-struct=2"], cc)
            check(f"{label} array under -fpack-struct=2", rc == 0, err[:300])

            # Aligned member under pack(2): capped at 2
            rc, out, err, _ = compile_and_run(f"aligned-member-{label}",
                                              ALIGNED_MEMBER_SOURCE,
                                              ["-fpack-struct=2"], cc)
            check(f"{label} aligned member capped by pack(2)", rc == 0, err[:300])

            # #pragma pack(0) restores natural alignment (no command-line cap)
            rc, out, err, _ = compile_and_run(f"pragma-zero-{label}",
                                              PRAGMA_ZERO_RESTORE_SOURCE,
                                              [], cc)
            check(f"{label} #pragma pack(0) restores natural", rc == 0, err[:300])

            # #pragma pack(push, 0) keeps natural alignment (no command-line cap)
            rc, out, err, _ = compile_and_run(f"pragma-push-zero-{label}",
                                              PRAGMA_PUSH_ZERO_SOURCE,
                                              [], cc)
            check(f"{label} #pragma pack(push,0) keeps natural", rc == 0, err[:300])

            for flags, expect in RUNTIME_EXPECT.items():
                tag = f"run-{label}-{'_'.join(flags) or 'none'}".replace("=", "")
                rc, out, err, _ = compile_and_run(tag, RUNTIME_SOURCE,
                                                  list(flags), cc)
                check(f"{label} runtime layout {flags or 'default'}",
                      rc == 0 and out == expect,
                      f"rc={rc} out={out!r} err={err[:200]}")

        # ---- lccc-only diagnostics ----
        # Bare form makes `#pragma pack` inert with GCC's warning text.
        src = tmp / "bare-pragma.c"
        src.write_text(BARE_PRAGMA_SOURCE)
        rc, out, err = run([args.lccc, "-fpack-struct", str(src), "-o", str(tmp / "bp")])
        check("lccc bare: #pragma pack ignored with warning",
              rc == 0 and "has no effect with '-fpack-struct'" in err,
              f"rc={rc} err={err[:300]}")

        # Invalid caps are hard errors with GCC's wording.
        trivial = tmp / "trivial.c"
        trivial.write_text("struct S { char c; int i; };\nint main(void){return 0;}\n")
        for bad in ("0", "3", "32", "bogus"):
            rc, out, err = run([args.lccc, f"-fpack-struct={bad}", "-c",
                                str(trivial), "-o", str(tmp / "t.o")])
            check(f"lccc -fpack-struct={bad} refused",
                  rc != 0 and "small power of two" in err,
                  f"rc={rc} err={err[:200]}")
        if args.reference:
            for bad in ("0", "3", "32"):
                rc, out, err = run([args.reference, f"-fpack-struct={bad}", "-c",
                                    str(trivial), "-o", str(tmp / "t.o")])
                check(f"reference -fpack-struct={bad} refused", rc != 0,
                      f"rc={rc}")

        # Out-of-range power-of-two values (17, 64, 128).
        for bad in ("17", "64", "128"):
            rc, out, err = run([args.lccc, f"-fpack-struct={bad}", "-c",
                                str(trivial), "-o", str(tmp / f"bad_{bad}.o")])
            check(f"lccc -fpack-struct={bad} refused (>16)",
                  rc != 0 and "small power of two" in err,
                  f"rc={rc} err={err[:200]}")

        # ---- -mcpu= alias parity ----
        # lccc emits a deprecation warning for -mcpu=; GCC accepts it
        # silently (no warning). Only check the warning on lccc.
        for label, cc in compilers:
            rc, out, err = run([cc, "-mcpu=core2", "-c", str(trivial),
                                "-o", str(tmp / "m.o")])
            check(f"{label} -mcpu=core2 accepted", rc == 0, err[:200])
            if label == "lccc":
                check(f"{label} -mcpu=core2 warns deprecated",
                      "deprecated" in err, err[:200])
            rc, out, err = run([cc, "-mcpu=bogus123", "-c", str(trivial),
                                "-o", str(tmp / "m.o")])
            check(f"{label} -mcpu=bogus123 refused", rc != 0, f"rc={rc}")

        # -mcpu= assembly identical to -mtune= (never selects an ISA).
        asm = {}
        for tag, flag in (("mcpu", "-mcpu=core2"), ("mtune", "-mtune=core2")):
            out_s = tmp / f"{tag}.s"
            rc, out, err = run([args.lccc, flag, "-S", str(trivial),
                                "-o", str(out_s)])
            asm[tag] = out_s.read_text() if rc == 0 else ""
        check("lccc -mcpu assembly identical to -mtune",
              bool(asm["mcpu"]) and asm["mcpu"] == asm["mtune"])

        # -mcpu= with i686-only tune refused on x86-64.
        rc, out, err = run([args.lccc, "-mcpu=pentium4", "-c",
                            str(trivial), "-o", str(tmp / "m3.o")])
        check("lccc -mcpu=pentium4 refused on x86-64 (i686-only tune)",
              rc != 0 and ("pentium4" in err or "unsupported" in err),
              f"rc={rc} err={err[:200]}")

        # -mcpu= error message names -mcpu=, not -mtune=.
        rc, out, err = run([args.lccc, "-mcpu=bogus123", "-c",
                            str(trivial), "-o", str(tmp / "m4.o")])
        check("lccc -mcpu bogus error names -mcpu= not -mtune=",
              rc != 0 and "-mcpu=bogus123" in err and "-mtune=bogus123" not in err,
              f"rc={rc} err={err[:200]}")

        # -mtune= error message names -mtune=, not -mcpu=.
        rc, out, err = run([args.lccc, "-mtune=bogus123", "-c",
                            str(trivial), "-o", str(tmp / "m5.o")])
        check("lccc -mtune bogus error names -mtune= not -mcpu=",
              rc != 0 and "-mtune=bogus123" in err and "-mcpu=bogus123" not in err,
              f"rc={rc} err={err[:200]}")

        # -mcpu=native accepted (CPUID-based tune resolution).
        # Skipped in CI containers where CPUID may be virtualized/restricted;
        # covered by the unit test in src/driver/cli.rs instead.

        # ---- -fsyntax-only parity ----
        # Simple template without layout-dependent static asserts: we only
        # need both compilation modes to agree (both succeed or both fail).
        SYNTAX_TEMPLATE = r'''
struct S { char c; int i; char d; long long l; };
int main(void) { return 0; }
'''
        for flags in [[], ["-fpack-struct"], ["-fpack-struct=2"],
                      ["-fpack-struct=4"], ["-fpack-struct=16"]]:
            src_syntax = tmp / f"syntax_{'_'.join(flags) or 'default'}.c"
            src_syntax.write_text(SYNTAX_TEMPLATE)
            rc_full, _, _ = run([args.lccc, *flags, str(src_syntax),
                                 "-o", str(tmp / "syntax_full.o")])
            rc_syntax, _, _ = run([args.lccc, *flags, "-fsyntax-only",
                                   str(src_syntax)])
            check(f"lccc -fsyntax-only agrees {' '.join(flags) or 'default'}",
                  rc_full == 0 and rc_syntax == 0,
                  f"full rc={rc_full} syntax rc={rc_syntax}")

        # -fpack-struct=16 (max valid cap) compiles and matches default
        # layout (16 >= natural alignment of all members, so no change).
        src16 = tmp / "pack16.c"
        src16.write_text(CAP_TEMPLATE.replace("@S_SIZE@", "24")
                         .replace("@S_ALIGN@", "8")
                         .replace("@S_IOFF@", "4")
                         .replace("@S_LOFF@", "16")
                         .replace("@A_IOFF@", "8"))
        rc, _, _ = run([args.lccc, "-fpack-struct=16", str(src16),
                        "-o", str(tmp / "pack16.o")])
        check("lccc -fpack-struct=16 compiles (max valid cap)", rc == 0)

        # #pragma pack(push/pop) with -fpack-struct=2 cap baseline:
        # push(1)/push(2) set explicit values; pop() restores the previous
        # stack entry; final pop() restores the command-line cap (2).
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
_Static_assert(sizeof(struct P2) == 6, "P2 pack 2");
_Static_assert(sizeof(struct P3) == 5, "P3 pop restores previous (1)");
_Static_assert(sizeof(struct P4) == 6, "P4 pop restores cap (2)");
int main(void) { return 0; }
''')
        rc, _, err = run([args.lccc, "-fpack-struct=2", str(src_pushpop),
                          "-o", str(tmp / "pushpop.o")])
        check("lccc #pragma pack push/pop with cap baseline", rc == 0,
              f"rc={rc} err={err[:300]}")

        # #pragma pack(push) with bare -fpack-struct: all inert, stack untouched.
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

        # -mcpu= on reference compiler rejects bogus too.
        if args.reference:
            rc, _, err = run([args.reference, "-mcpu=bogus123", "-c",
                              str(trivial), "-o", str(tmp / "ref_m.o")])
            check(f"reference -mcpu=bogus123 refused", rc != 0,
                  f"rc={rc} err={err[:200]}")

    print(f"\n{'FAIL' if failures else 'PASS'}: {len(failures)} failure(s)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
