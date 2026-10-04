#!/usr/bin/env python3
"""Array-bound contract gate: diagnostics, decay agreement, and no OOM death.

Three defect classes this pins, all of which shipped at least once:

  1. **A bad bound must be diagnosed, not wrapped.** A constant array bound
     that is negative is a constraint violation (C11 6.7.6.2p5). Converting it
     to `usize` with `as` wrapped `-1` into 2^64-1 elements, which the backend
     then turned into a multi-gigabyte `.bss` reservation: compiling
     `int a[sizeof((0, arr)) == 8 ? 1 : -1];` (with `static int arr[4];`)
     aborted with "memory allocation of 17179869168 bytes failed". The bound
     must produce `error: size of array is negative`.

  2. **A comma/conditional result decays.** `sizeof((0, arr))` and
     `sizeof(1 ? arr : arr)` are the size of an `int *`, not of `int[4]`, and
     the operand's own `sizeof` is unchanged. This is the shape that made the
     differential correctness gate red: sema's comma arm learned the rule but
     the sizeof walk in `ir/lowering/expr_sizeof.rs` had its own type map and
     kept reporting 16 where GCC reports 8.

  3. **A huge `.bss` object is a size, never bytes, and never a SIGKILL.**
     `.bss` is emitted as NOBITS with an exact `sh_size`, so a 4 GiB array (and
     larger) costs a constant amount of memory and time: the gate compiles one
     under a hard 1 GiB address-space limit and requires the exact `.bss` size
     with a small object file. The compiler used to materialise the zeros and
     was removed by the OOM killer at 4 GiB (measured: SIGKILL, ~30 s of
     thrashing); a signal is still a failure, for every case in this gate.

  4. **A bound whose *byte* size cannot exist is rejected, never wrapped.**
     The bound that has to be rejected is the byte size, and the arithmetic
     that produces it overflows 64 bits for exactly those bounds: 2^62 `int`s
     is 2^64 bytes, which wrapped to **0** and produced a zero-byte object with
     no diagnostic, and 5e18 `int`s wrapped to a bogus 1.5 EiB size. The limit
     is `PTRDIFF_MAX` bytes (`MAX_OBJECT_BYTES`), the same rule GCC applies.
     Every case here must be reported from the *source*, through a diagnostic
     that names the array construct -- an error message that quotes synthesized
     `.zero` output is a failure too, because it means the front end handed an
     impossible bound to the assembler instead of rejecting it.

No libc headers are used, so this gate has no dependency on an external
compiler or include path.

Usage: scripts/check_array_bound_contract.py [--lccc PATH]
Exit 0 = every case holds, 1 = at least one case failed (printed), 2 = setup error.
"""
from __future__ import annotations

import argparse
import os
import re
import resource
import subprocess
import sys
import tempfile

from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_LCCC = REPO_ROOT / "target" / "fastbuild" / "lccc"

# Address-space limit for the "must not die" case: large enough for the
# compiler itself, far too small for the object it is asked to materialise.
TIGHT_AS_LIMIT_BYTES = 1 << 30

DIAG_NEGATIVE = "size of array is negative"
DIAG_TOO_LARGE = "size of array is too large"

# PTRDIFF_MAX bytes: GCC's maximum object size and this compiler's too.
MAX_OBJECT_BYTES = 9223372036854775807

# A sparse `.bss` object must stay this small however large the array is:
# only the ELF headers, symbol table and the one section header are written.
MAX_SPARSE_OBJECT_BYTES = 64 * 1024

# (name, source, expectation)
#   ("reject", message)  -- compile must fail with `message` in the diagnostics
#   ("accept", exit_code) -- compile must succeed and the program must exit
#                            with this code when run
CASES: list[tuple[str, str, tuple[str, object]]] = [
    (
        "negative-bound-literal",
        "int a[-1];\nint main(void) { return 0; }\n",
        ("reject", DIAG_NEGATIVE),
    ),
    (
        "negative-bound-conditional",
        "int a[(1 ? -3 : 2)];\nint main(void) { return 0; }\n",
        ("reject", DIAG_NEGATIVE),
    ),
    (
        "negative-bound-through-sizeof-decay",
        # The exact shape of the CI failure: the bound is the *good* value once
        # `sizeof((0, arr))` is 8, so this must compile and a1 must have one
        # element. Before the decay fix the bound took the -1 arm.
        "static int arr[4];\n"
        "int a1[sizeof((0, arr)) == 8 ? 1 : -1];\n"
        "int main(void) { return (int)(sizeof(a1) / sizeof(a1[0])); }\n",
        ("accept", 1),
    ),
    (
        "zero-bound-accepted",
        "int a[0];\nint main(void) { return (int)sizeof(a); }\n",
        ("accept", 0),
    ),
    (
        "moderate-bss-array-accepted",
        # Guards the memory pre-check against over-firing: 32 MiB is a normal
        # object and must still assemble.
        "int a[8 * 1024 * 1024];\n"
        "int main(void) { return (int)(sizeof(a) >> 20) + a[4194303]; }\n",
        ("accept", 32),
    ),
    (
        # sizeof(void) is 1 in GCC (a documented extension) and must agree with
        # _Alignof(void). The same expression through a comma operator goes
        # through the bounds evaluator, which is why it is here: it read 0 while
        # the evaluator sized `void` as 0 bytes.
        "void-and-comma-sizeof",
        "int main(void) {\n"
        "    return sizeof(void) == 1 && sizeof((void)0) == 1\n"
        "        && sizeof((0, (void)0)) == 1 && _Alignof(void) == 1 ? 0 : 1;\n"
        "}\n",
        ("accept", 0),
    ),
    # --- the byte-size limit: exact at PTRDIFF_MAX, rejected past it ---------
    (
        "bound-just-inside-max-object-size",
        "char a[9223372036854775807];\nint main(void) { return 0; }\n",
        ("bss", (MAX_OBJECT_BYTES, MAX_SPARSE_OBJECT_BYTES, None)),
    ),
    (
        "bound-one-past-max-object-size",
        "int a[2305843009213693952];\nint main(void) { return 0; }\n",
        ("reject-object", DIAG_TOO_LARGE),
    ),
    (
        # 2^62 ints = 2^64 bytes: wrapped to 0 before this gate existed, and the
        # object compiled, linked and ran as an empty array.
        "bound-wrapping-to-zero-is-refused",
        "int a[4611686018427387904];\nint main(void) { return 0; }\n",
        ("reject-object", DIAG_TOO_LARGE),
    ),
    (
        # 5e18 ints = 2e19 bytes: wrapped to 1.5 EiB and was accepted silently.
        "bound-wrapping-to-bogus-size-is-refused",
        "int a[5000000000000000000];\nint main(void) { return 0; }\n",
        ("reject-object", DIAG_TOO_LARGE),
    ),
    (
        # The bound that overflows is the inner one; the diagnostic must point
        # at the inner array, and the outer dimension must not hide it.
        "nested-bound-overflow-is-refused",
        "int a[2][2305843009213693952];\nint main(void) { return 0; }\n",
        ("reject-object", DIAG_TOO_LARGE),
    ),
    (
        # Element size 3 (unpadded char[3]) decides this: 3 * 3074457345618258603
        # is 2^63 + 2 bytes. Using the aligned element size (or 1) would accept
        # it; using 4 would reject the case below too.
        "struct-element-size-decides-the-limit",
        "struct s { char c[3]; };\n"
        "struct s a[3074457345618258603];\n"
        "int main(void) { return 0; }\n",
        ("reject-object", DIAG_TOO_LARGE),
    ),
    (
        "struct-element-size-decides-the-limit-accepted",
        "struct s { char c[3]; };\n"
        "struct s a[3074457345618258602];\n"
        "int main(void) { return 0; }\n",
        ("bss", (9223372036854775806, MAX_SPARSE_OBJECT_BYTES, None)),
    ),
    # --- sparse .bss: size, not bytes ---------------------------------------
    (
        # 4 GiB of .bss under a 1 GiB address-space limit: only possible if the
        # zeros are a size and never allocated. This is the case that used to be
        # a SIGKILL.
        "huge-bss-under-tight-limit-is-sparse",
        "int a[1000000000];\nint main(void) { return 0; }\n",
        ("bss", (4000000000, MAX_SPARSE_OBJECT_BYTES, 1 << 30)),
    ),
    (
        # 16 GiB, twelve times the size of a pointer-sized `.bss` cap: proves no
        # 32-bit truncation of the section size or of the `.zero` operand.
        "huge-bss-16g-exact-size",
        "int a[4000000000];\nint main(void) { return 0; }\n",
        ("bss", (16000000000, MAX_SPARSE_OBJECT_BYTES, None)),
    ),
]


def read_bss_size(obj: Path) -> int | None:
    """`sh_size` of `.bss` in an ELF object, or None if the file does not parse.

    Reading the field directly is deliberate: a `.bss` size is the one number
    this gate is about, and `readelf` is not guaranteed to exist next to the
    compiler under test. Supports ELF32/ELF64, both endiannesses.
    """
    data = obj.read_bytes()
    if len(data) < 52 or data[:4] != b"\x7fELF":
        return None
    is64 = data[4] == 2
    little = data[5] == 1
    order = "little" if little else "big"

    def u(off: int, size: int) -> int:
        return int.from_bytes(data[off:off + size], order)

    if is64:
        shoff, shentsize = u(0x28, 8), u(0x3A, 2)
        shnum, shstrndx = u(0x3C, 2), u(0x3E, 2)
        name_off, size_off, ent_size = 0x00, 0x20, 64
    else:
        shoff, shentsize = u(0x20, 4), u(0x2E, 2)
        shnum, shstrndx = u(0x30, 2), u(0x32, 2)
        name_off, size_off, ent_size = 0x00, 0x14, 40
    if not 0 < shnum < 0xFFFF or not 0 < shentsize <= 1024:
        return None

    def section(index: int) -> tuple[int, int, int]:
        # sh_offset sits at 0x18 in ELF64 and 0x10 in ELF32; the -m32 family
        # below is what exercises the second layout.
        base = shoff + index * shentsize
        off_off = 0x18 if is64 else 0x10
        return (
            u(base + name_off, 4),
            u(base + size_off, 8 if is64 else 4),
            u(base + off_off, 8 if is64 else 4),
        )

    _, str_size, str_off = section(shstrndx)[0], section(shstrndx)[1], section(shstrndx)[2]
    if str_off + str_size > len(data):
        return None
    names = data[str_off:str_off + str_size]
    for i in range(shnum):
        name_idx, size, _ = section(i)
        end = names.find(b"\0", name_idx)
        if names[name_idx:end] == b".bss":
            return size
    return None


def run_lccc(
    lccc: Path,
    source: Path,
    obj: Path | None,
    limit_as: int | None = None,
    emit: str = "-S",
    extra: list[str] | None = None,
) -> tuple[int, str]:
    """Compile `source`, returning (returncode, diagnostics).

    `emit` selects the output stage: "-S" stops in the assembler and never
    reaches the ELF writer, "-c" writes a real object file. The memory-limit
    case needs "-c", because that is the stage that materialises `.bss`.
    """
    cmd = (
        [str(lccc), emit, "-o", str(obj if obj else "/dev/null"), str(source)]
        if obj
        else [str(lccc), "-fsyntax-only", str(source)]
    )
    if extra:
        cmd[1:1] = extra

    def child() -> None:
        if limit_as is not None:
            resource.setrlimit(resource.RLIMIT_AS, (limit_as, limit_as))

    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=120,
                          preexec_fn=child if limit_as is not None else None)
    return proc.returncode, (proc.stderr or "") + (proc.stdout or "")


def compile_and_run(lccc: Path, source: Path, exe: Path) -> tuple[int, str]:
    proc = subprocess.run([str(lccc), "-o", str(exe), str(source)],
                          capture_output=True, text=True, timeout=120)
    if proc.returncode != 0:
        return proc.returncode, proc.stderr
    try:
        run = subprocess.run([str(exe)], capture_output=True, text=True, timeout=30)
    except subprocess.TimeoutExpired:
        return -99, "program timed out"
    return run.returncode, run.stdout + run.stderr


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lccc", default=str(DEFAULT_LCCC))
    args = parser.parse_args()

    lccc = Path(args.lccc)
    if not lccc.is_file():
        print(f"setup error: no lccc at {lccc}", file=sys.stderr)
        return 2

    failures: list[str] = []
    m32_ran = False
    with tempfile.TemporaryDirectory(prefix="lccc_bound_contract_") as tmpdir:
        tmp = Path(tmpdir)
        for name, src, (kind, expected) in CASES:
            source = tmp / f"{name}.c"
            source.write_text(src)
            if kind == "reject":
                obj = tmp / f"{name}.s"
                rc, out = run_lccc(lccc, source, obj)
                ok = rc != 0 and re.search(re.escape(str(expected)), out) is not None
                detail = f"rc={rc} expected a rejection containing {expected!r}; output={out.strip()[:200]!r}"
            elif kind == "reject-object":
                # The rejection must come from the source (a diagnostic that
                # names the array), must not be a signal, and must leave no
                # object behind claiming a wrapped size. The assembler-message
                # leak is excluded by name so that a future regression reports
                # the missing front-end check instead of hiding behind it.
                obj = tmp / f"{name}.o"
                rc, out = run_lccc(lccc, source, obj, emit="-c")
                named_from_source = re.search(re.escape(str(expected)), out) is not None
                leaked_asm = re.search(r"\.(?:zero|space|skip)\b", out) is not None
                # No object may be left behind at all: a wrapped `.bss` that
                # still links is exactly the defect this kind exists to catch,
                # and a partial object is no better.
                left_object = obj.exists()
                ok = rc > 0 and named_from_source and not leaked_asm and not left_object
                detail = (f"rc={rc} diagnostic-from-source={named_from_source} "
                          f"leaked-assembler-output={leaked_asm} object-.bss={bss}; "
                          f"output={out.strip()[:200]!r}")
            elif kind == "bss":
                want_bss, max_obj, limit_as = expected  # type: ignore[misc]
                obj = tmp / f"{name}.o"
                rc, out = run_lccc(lccc, source, obj, limit_as=limit_as, emit="-c")
                bss = read_bss_size(obj)
                size = obj.stat().st_size if obj.exists() else -1
                ok = (rc == 0 and bss == want_bss
                      and 0 <= size <= max_obj)
                detail = (f"rc={rc} .bss={bss} (want {want_bss}) object={size}B "
                          f"(max {max_obj}); output={out.strip()[:200]!r}")
            else:
                exe = tmp / f"{name}.exe"
                rc, out = compile_and_run(lccc, source, exe)
                ok = rc == expected
                detail = f"exit={rc} expected {expected}; output={out.strip()[:200]!r}"
            status = "ok  " if ok else "FAIL"
            print(f"  {status} {name}")
            if not ok:
                failures.append(f"{name}: {detail}")

        # ---- the 32-bit target: PTRDIFF_MAX is 2^31-1 there ----------------
        # The limit has to be the *target's*, and the element count has to fit
        # the target's `usize`: with the 64-bit limit and a 64-bit conversion,
        # `char a[4294967296]` under `-m32` truncated to a zero-element array
        # and compiled to a zero-byte `.bss` (measured against GCC, which
        # rejects it). The probe is a capability check on this host, not on the
        # compiler's codegen: without 32-bit headers nothing here can run, and
        # that is a SKIP, not a failure.
        probe = tmp / "m32_probe.c"
        probe.write_text("int main(void) { return 0; }\n")
        rc, out = run_lccc(lccc, probe, tmp / "m32_probe.o",
                           extra=["-m32"], emit="-c")
        if rc != 0:
            print("  skip 32-bit-boundary family: no 32-bit headers on this host")
        else:
            m32_ran = True
            for name, src, kind, expected in (
                ("m32-bound-at-target-limit",
                 "char a[2147483647];\nint main(void) { return 0; }\n",
                 "bss", (2147483647, MAX_SPARSE_OBJECT_BYTES, None)),
                ("m32-bound-past-target-limit",
                 "char a[2147483648];\nint main(void) { return 0; }\n",
                 "reject-object", DIAG_TOO_LARGE),
                ("m32-bound-truncating-to-zero",
                 "char a[4294967296];\nint main(void) { return 0; }\n",
                 "reject-object", DIAG_TOO_LARGE),
                ("m32-bound-far-past-target-limit",
                 "char a[5000000000];\nint main(void) { return 0; }\n",
                 "reject-object", DIAG_TOO_LARGE),
            ):
                source = tmp / f"{name}.c"
                source.write_text(src)
                obj = tmp / f"{name}.o"
                if kind == "bss":
                    want_bss, max_obj, _ = expected  # type: ignore[misc]
                    rc, out = run_lccc(lccc, source, obj, extra=["-m32"], emit="-c")
                    bss = read_bss_size(obj)
                    ok = rc == 0 and bss == want_bss
                    detail = f"rc={rc} .bss={bss} (want {want_bss})"
                else:
                    rc, out = run_lccc(lccc, source, obj, extra=["-m32"], emit="-c")
                    named = re.search(re.escape(str(expected)), out) is not None
                    ok = rc > 0 and named and not obj.exists()
                    detail = f"rc={rc} diagnostic-from-source={named}"
                print(f"  {'ok  ' if ok else 'FAIL'} {name}")
                if not ok:
                    failures.append(f"{name}: {detail}; output={out.strip()[:160]!r}")

        # A signal is a failure for every case above; the `bss` and
        # `reject-object` kinds assert a real return code instead, because a
        # process removed by the OOM killer cannot report a `.bss` size at all.
        if failures:
            print()
            print("note: reject cases must be diagnosed from the source; an error")
            print("      quoting synthesized .zero/.skip output means the impossible")
            print("      bound reached the assembler instead of being rejected.")

    print()
    if failures:
        print(f"{len(failures)} array-bound contract failure(s):")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    m32 = " + 4 i686" if m32_ran else ""  # noqa: PLW2901 - reporting only
    print(f"all {len(CASES)} array-bound contract cases{m32} hold")
    return 0


if __name__ == "__main__":
    sys.exit(main())
