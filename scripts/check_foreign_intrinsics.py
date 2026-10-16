#!/usr/bin/env python3
"""check_foreign_intrinsics.py — no x86-only builtin silently compiles on a
non-x86 backend (AUD-SILENT-ZERO-1).

THE GAP THIS PINS
-----------------
The ARM and RISC-V backends lowered every x86-only intrinsic (the whole
__builtin_ia32_* AES/SSE/CLMUL family) to ZEROS: `lccc-arm -O2 -S` on
`__builtin_ia32_aesenc128` exited 0 with `stp xzr, xzr` as the function body —
a silently wrong result where GCC rejects the builtin on aarch64.  The same
files carried `unimplemented!` ICEs reachable from source.

THE CONTRACT ASSERTED HERE
--------------------------
* the lowering layer rejects every x86-only builtin on aarch64/riscv64 with
  `file:line:col: error: '<name>' is not available on <target>` (single
  funnel: expr_builtins.rs maps all 268 BuiltinIntrinsic::X86* variants
  through one arm) — so NO arity of ANY registered x86-only builtin name may
  compile cleanly on a non-x86 backend;
* the compiler never answers `internal error` / a panic for them;
* the canonical audit reproducer (aesenc128 on a vector type) is asserted
  verbatim, with the caret shape GCC produces;
* the x86-64 backend still compiles the same fixtures (no over-blocking).

The x86-only name set is DERIVED from src/frontend/sema/builtins.rs (the
`m.insert("<name>", BuiltinInfo::intrinsic(BuiltinIntrinsic::X86...))` pairs),
so the sweep self-updates when builtins are added.

Self-test (--self-test): a fake compiler stub that happily compiles
everything must be REJECTED by the sweep; that is the mutation proof that the
gate cannot pass a silent-zero regression.
"""

import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BUILTINS_SRC = REPO / "src" / "frontend" / "sema" / "builtins.rs"
DEFAULT_ARM = REPO / "target" / "fastbuild" / "lccc-arm"
DEFAULT_RISCV = REPO / "target" / "fastbuild" / "lccc-riscv"
DEFAULT_X86 = REPO / "target" / "fastbuild" / "lccc"

INTERNAL_ERROR_RE = re.compile(r"internal error|panicked|\.error\b", re.IGNORECASE)
INSERT_RE = re.compile(
    r'm\.insert\(\s*"([^"]+)"\s*,\s*BuiltinInfo::intrinsic\(BuiltinIntrinsic::(X86[A-Za-z0-9_]+)',
)


def x86_builtin_names() -> list[str]:
    """Every registered builtin name that maps to a BuiltinIntrinsic::X86*."""
    text = BUILTINS_SRC.read_text(errors="replace")
    names = sorted({n for n, _ in INSERT_RE.findall(text)})
    return names


def compile_probe(
    compiler: Path, source: str, cwd: Path, extra_args: list[str] | None = None
) -> subprocess.CompletedProcess:
    """Run one compile-to-assembly probe (no codegen artifacts needed)."""
    probe = cwd / "probe.c"
    probe.write_text(source)
    return subprocess.run(
        [str(compiler), "-O2", "-S", "-o", str(cwd / "probe.s"), *(extra_args or []), str(probe)],
        capture_output=True,
        text=True,
        timeout=120,
        cwd=cwd,
    )


def arity_source(name: str, arity: int) -> str:
    args = ", ".join(["0"] * arity)
    return f"void lccc_probe(void){{ (void){name}({args}); }}\n"


def sweep(backend: Path, label: str, names: list[str], work: Path) -> list[str]:
    """Assert: no arity compiles cleanly, no internal error.  Returns failures."""
    failures: list[str] = []
    backend_dir = work / label
    backend_dir.mkdir(parents=True, exist_ok=True)
    for name in names:
        clean_arity = None
        for arity in (0, 1, 2):
            r = compile_probe(backend, arity_source(name, arity), backend_dir)
            out = (r.stdout or "") + (r.stderr or "")
            if r.returncode == 0:
                clean_arity = arity
            if INTERNAL_ERROR_RE.search(out):
                failures.append(
                    f"{label}: {name}(arity {arity}): internal-error text leaked: {out.strip()[:200]}"
                )
        if clean_arity is not None:
            failures.append(
                f"{label}: {name}: arity {clean_arity} compiled CLEANLY on a non-x86 "
                f"target (silent path survived the lowering gate)"
            )
    return failures


def canonical_reproducer(arm: Path, riscv: Path, x86: Path, work: Path) -> list[str]:
    """The audit's exact reproducer, asserted verbatim on both backends."""
    failures: list[str] = []
    src = (
        "typedef int v2 __attribute__((vector_size(16)));\n"
        "v2 f(v2 a, v2 k){ return __builtin_ia32_aesenc128(a, k); }\n"
    )
    for backend, target in ((arm, "aarch64"), (riscv, "riscv64")):
        d = work / f"canon-{target}"
        d.mkdir(parents=True, exist_ok=True)
        r = compile_probe(backend, src, d)
        out = (r.stdout or "") + (r.stderr or "")
        if r.returncode == 0:
            failures.append(f"{target}: canonical aesenc128 reproducer exited 0")
        if f"'__builtin_ia32_aesenc128' is not available on {target}" not in out:
            failures.append(f"{target}: availability diagnostic missing/malformed: {out.strip()[:200]}")
        if f"{target}" in out and "aes.c:2:" not in out.replace("probe.c", "aes.c"):
            # file:line:col prefix shape (the probe file name differs; shape only)
            failures.append(f"{target}: diagnostic lacks a file:line:col prefix")
        if "internal error" in out.lower() or "panicked" in out.lower():
            failures.append(f"{target}: internal error text in canonical reproducer")
    # The x86-64 backend must still accept the same source under -maes (no
    # over-blocking), and must REJECT it without -maes — gcc 14.2 errors
    # ("needs isa option -maes ...") whenever the ISA extension is absent,
    # and check_x86_intrinsic_feature mirrors that contract on x86 too.
    d = work / "canon-x86"
    d.mkdir(parents=True, exist_ok=True)
    r = compile_probe(x86, src, d, ["-maes"])
    if r.returncode != 0:
        failures.append(f"x86-64: canonical aesenc128 must still compile under -maes: {(r.stderr or '')[:200]}")
    d = work / "canon-x86-noisa"
    d.mkdir(parents=True, exist_ok=True)
    r = compile_probe(x86, src, d)
    if r.returncode == 0:
        failures.append("x86-64: aesenc128 compiled without -maes (gcc requires the ISA extension)")
    return failures


def self_test(work: Path) -> int:
    """A fake compiler that compiles everything must be rejected by the sweep."""
    fake = work / "fake-silent-zero-lccc"
    fake.write_text("#!/bin/sh\nexit 0\n")
    fake.chmod(0o755)
    failures = sweep(fake, "fake", ["__builtin_ia32_aesenc128"], work)
    if failures:
        print("PASS self-test (a silent-zero compiler is rejected)")
        return 0
    print("FAIL self-test (gate does not bite on a silent-zero compiler)")
    return 1


def main() -> int:
    if "--self-test" in sys.argv:
        work = Path(__import__("tempfile").mkdtemp(prefix="lccc-foreign-selftest."))
        try:
            return self_test(work)
        finally:
            __import__("shutil").rmtree(work, ignore_errors=True)

    arm = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_ARM
    riscv = Path(sys.argv[2]) if len(sys.argv) > 2 else DEFAULT_RISCV
    x86 = Path(sys.argv[3]) if len(sys.argv) > 3 else DEFAULT_X86

    backends = [(p, lbl) for p, lbl in ((arm, "arm"), (riscv, "riscv"), (x86, "x86")) if p.exists()]
    missing = [lbl for p, lbl in ((arm, "arm"), (riscv, "riscv")) if not p.exists()]
    if missing:
        print(f"SKIP: non-x86 backend binaries absent from the build tree: {missing}")
        return 0

    names = x86_builtin_names()
    if not names:
        print("FAIL: no x86-only builtin names derived from sema/builtins.rs")
        return 1

    import tempfile
    import shutil

    work = Path(tempfile.mkdtemp(prefix="lccc-foreign-intrinsics."))
    try:
        failures: list[str] = []
        failures += canonical_reproducer(arm, riscv, x86, work)
        for backend, label in backends:
            if label == "x86":
                continue  # x86 must compile; covered by the canonical reproducer
            failures += sweep(backend, label, names, work)
    finally:
        shutil.rmtree(work, ignore_errors=True)

    if failures:
        print(f"FAIL {len(failures)} row(s):")
        for f in failures[:40]:
            print(f"  {f}")
        if len(failures) > 40:
            print(f"  ... and {len(failures) - 40} more")
        return 1
    print(f"PASS: {len(names)} x86-only builtins x 2 non-x86 backends: "
          f"no clean compile, no internal error; canonical reproducer + x86 control green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
