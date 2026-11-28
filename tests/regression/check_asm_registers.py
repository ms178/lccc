#!/usr/bin/env python3
"""Assembler statements and the registers the compiler keeps for itself.

A statement names registers: in its operands (`"b"`, a register variable),
in its clobber list, and through the symbols its memory operands print. The
compiler keeps registers too -- the GOT pointer in %ebx on i686, the frame
pointer, the homes of live values -- and where the two meet it has to give
way. Five defects were one family, and each crashed or silently computed
something else:

* i686 `-fPIC`/`-fPIE`: a statement that takes %ebx (`cpuid`'s `"=b"`, a
  `"b"` input, a clobber) destroyed the GOT pointer for the rest of the
  function (`fixtures/asm_registers/got_pointer.c`, `got_pointer_goto.c`);
* a clobber spelled `"bx"`, `"bl"`, `"si"`, `"sil"`, `"rbx"` or any name of
  %ebp was not understood on i686, and on x86-64 `"rbp"` made the function
  address its frame through the register the statement destroys
  (`clobber_spellings.c`);
* with a frame pointer the function cannot do without, GCC refuses such a
  statement ("bp cannot be used in 'asm' here"); lccc compiled it
  (`frame_pointer_refused.c`);
* a memory operand that names a global printed an absolute or PC-relative
  address in position-independent code: a text relocation on i686, an
  object no linker makes a shared library of on x86-64
  (`symbol_operands.c`, `shared_library.c`);
* a memory operand that names a thread-local object printed the address of
  the image threads are initialised from (`symbol_operands.c`).

What is checked, against GCC 16.2:

1. **Programs.** Every fixture, for x86-64 and i686, position dependent,
   `-fPIC` and `-fPIE -pie`, at -O0 to -Os: the program lccc builds prints
   what GCC's prints and exits as it exits.
2. **Shared library.** lccc's objects of `shared_library.c` are linked into
   a shared object by GNU ld with `-z text` (a text relocation is an
   error), and a program that shares a global with the library sees one
   object.
3. **Refusals.** Where GCC refuses a statement that takes the frame
   pointer register, lccc refuses it in the same words at the same line;
   where GCC compiles it, lccc does.
4. **Clobber names.** Every name GCC takes in a clobber list lccc takes;
   a string that names nothing ("r14l", "EBX", "st(0)") is refused in GCC's
   words at GCC's position, where lccc used to take it and protect
   nothing. (The sub-register names of r8-r15 and a few status registers
   are Clang's; lccc keeps taking them.)
5. **Negative control.** A program built to print something else is
   reported.

i686 needs a 32-bit C library to link and run against. Without it, or
without GCC 16.2, the gate fails, unless `--allow-missing-tools` is given,
in which case it says what it did not check.
"""

from __future__ import annotations

import argparse
import itertools
import re
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
FIXTURES = HERE / "fixtures" / "asm_registers"

LEVELS = ("-O0", "-O1", "-O2", "-O3", "-Os")
POSITION = (("-fno-pic", "-no-pie"), ("-fPIC",), ("-fPIE", "-pie"))
TLS_MODELS = (("-fPIC", "-ftls-model=initial-exec"), ("-fPIC", "-ftls-model=local-dynamic"))

# (fixture, targets, extra options, position modes, levels)
PROGRAMS = (
    ("got_pointer.c", ("-m32",), (), POSITION, LEVELS),
    ("got_pointer_goto.c", ("-m32",), (), POSITION, LEVELS),
    ("symbol_operands.c", ("-m32", "-m64"), (), POSITION, LEVELS),
    ("symbol_operands.c", ("-m32", "-m64"), (), TLS_MODELS, ("-O0", "-O2")),
    # GCC takes the frame pointer's names only where the frame pointer is
    # omitted, which -O0 never does and 32-bit -Os not by default.
    ("clobber_spellings.c", ("-m32", "-m64"), ("-fomit-frame-pointer",), POSITION,
     ("-O1", "-O2", "-O3", "-Os")),
)

# (name, options, the function of frame_pointer_refused.c, whether GCC refuses it)
REFUSALS = (
    ("frame pointer omitted", ("-O2",), "omitted", False),
    ("variable-length array", ("-O2",), "kept_for_the_array", True),
    ("-O0", ("-O0",), "kept_by_request", True),
    ("-fno-omit-frame-pointer", ("-O2", "-fno-omit-frame-pointer"), "kept_by_request", True),
    # The same function is fine where nothing asks for the frame pointer.
    ("frame pointer omitted", ("-O2",), "kept_by_request", False),
)
REFUSAL = "bp cannot be used in 'asm' here"

# Clobber names GCC 16.2 takes for both targets (the frame pointer's are in
# clobber_spellings.c, where the frame pointer is omitted).
CLOBBERS_TAKEN = (
    "eax ax al ah rax ebx bx bl bh rbx ecx cx cl ch rcx edx dx dl dh rdx esi si sil rsi "
    "edi di dil rdi %eax %bl %si #eax st st(1) st(7) %st %st(1) mm0 mm7 %mm0 xmm0 xmm7 %xmm3 "
    "flags fpsr cc memory redzone %cc %memory %redzone %flags %fpsr 0 %0").split()
# Strings that name nothing, for GCC and for Clang.
CLOBBERS_UNKNOWN = (
    "r8l r14l r32 st(0) st(8) xmm32 mm8 k8 eflags rflags fpsw mxcsr rip eip ip cs ds es fs "
    "gs ss RBX EBX Rax ST foo %%rax %% {rax} xmm r e").split() + ["", "ax ", " ax", "% eax",
                                                                  "st( 1)"]
# Names GCC refuses and Clang takes: lccc takes them.
CLOBBERS_CLANG = "r8d r8w r8b r15d r15w r15b %r8d tmm0 fpcr dirflag %dirflag cr0 dr0 bnd0".split()


def run(command: list[str], cwd: Path | None = None, timeout: int = 120):
    return subprocess.run(command, cwd=cwd, capture_output=True, text=True, timeout=timeout)


def build_and_run(compiler: str, options: list[str], source: Path, exe: Path) -> str:
    """What the program prints and how it exits, or why it was not built."""
    built = run([compiler, *options, "-o", str(exe), str(source)])
    if built.returncode != 0:
        first = (built.stderr.strip().splitlines() or ["no message"])[0]
        return f"not built: {first}"
    try:
        ran = run([str(exe)], timeout=60)
    except subprocess.TimeoutExpired:
        return "ran for more than 60 seconds"
    return f"{ran.stdout}exit status {ran.returncode}"


def program_cases():
    for fixture, targets, extra, modes, levels in PROGRAMS:
        for target, mode, level in itertools.product(targets, modes, levels):
            yield fixture, [target, level, *extra, *mode]


def check_programs(lccc: str, gcc: str, work: Path, targets: set[str], jobs: int):
    cases = [case for case in program_cases() if case[1][0] in targets]

    def one(numbered):
        index, (fixture, options) = numbered
        source = FIXTURES / fixture
        expected = build_and_run(gcc, options, source, work / f"g{index}")
        if expected.startswith("not built"):
            return f"{fixture} {' '.join(options)}: GCC: {expected}"
        got = build_and_run(lccc, options, source, work / f"l{index}")
        if got == expected:
            return None
        differing = [f"`{g}` for `{e}`" for e, g in
                     itertools.zip_longest(expected.splitlines(), got.splitlines(), fillvalue="")
                     if e != g]
        return f"{fixture} {' '.join(options)}: {differing[0] if differing else 'differs'}"

    with ThreadPoolExecutor(max_workers=jobs) as pool:
        findings = [finding for finding in pool.map(one, enumerate(cases)) if finding]
    return len(cases), findings


def check_shared_library(lccc: str, gcc: str, work: Path, targets: set[str]):
    findings = []
    checked = 0
    for target in sorted(targets):
        tag = target.lstrip("-")
        outputs = {}
        for name, compiler in (("gcc", gcc), ("lccc", lccc)):
            obj = work / f"shared-{tag}-{name}.o"
            lib = work / f"libshared-{tag}-{name}.so"
            exe = work / f"shared-{tag}-{name}"
            compiled = run([compiler, target, "-O2", "-fPIC", "-c", "-o", str(obj),
                            str(FIXTURES / "shared_library.c")])
            if compiled.returncode != 0:
                findings.append(f"shared library {target}: {name} does not compile it: "
                                f"{(compiled.stderr.strip().splitlines() or [''])[0]}")
                break
            # GNU ld, told that a relocation in the text is an error.
            linked = run([gcc, target, "-shared", "-Wl,-z,text", "-o", str(lib), str(obj)])
            if linked.returncode != 0:
                findings.append(f"shared library {target}: GNU ld refuses {name}'s object: "
                                f"{(linked.stderr.strip().splitlines() or [''])[0]}")
                break
            program = run([gcc, target, "-O2", "-fno-pie", "-no-pie", "-o", str(exe),
                           str(FIXTURES / "shared_main.c"), str(lib),
                           f"-Wl,-rpath,{work}"])
            if program.returncode != 0:
                findings.append(f"shared library {target}: the program does not link against "
                                f"{name}'s library: "
                                f"{(program.stderr.strip().splitlines() or [''])[0]}")
                break
            ran = run([str(exe)])
            outputs[name] = f"{ran.stdout}exit status {ran.returncode}"
        else:
            checked += 1
            if outputs["lccc"] != outputs["gcc"]:
                findings.append(f"shared library {target}: with lccc's library the program "
                                f"prints {outputs['lccc']!r}, with GCC's {outputs['gcc']!r}")
    return checked, findings


def refusal_lines(compiler: str, options: list[str], source: Path):
    """The lines the compiler refuses the statement at, and whether it compiled."""
    result = run([compiler, *options, "-c", "-o", "/dev/null", str(source)])
    lines = [int(match.group(1)) for match in
             re.finditer(r":(\d+):\d+: error: " + re.escape(REFUSAL), result.stderr)]
    return result.returncode == 0, lines, result.stderr


def check_refusals(lccc: str, gcc: str, work: Path, targets: set[str]):
    # One function per compilation: GCC stops at the first it refuses.
    template = (FIXTURES / "frame_pointer_refused.c").read_text(encoding="utf-8")
    findings = []
    checked = 0
    for target in sorted(targets):
        for name, options, only, refused in REFUSALS:
            source = work / f"refusal-{target.lstrip('-')}-{only}.c"
            source.write_text(select_function(template, only), encoding="utf-8")
            options = [target, *options]
            gcc_ok, gcc_lines, gcc_err = refusal_lines(gcc, options, source)
            lccc_ok, lccc_lines, lccc_err = refusal_lines(lccc, options, source)
            checked += 1
            where = f"{target} {name} ({only})"
            if gcc_ok == refused:
                findings.append(f"refusal {where}: GCC was expected to "
                                f"{'refuse' if refused else 'compile'} it: {gcc_err.strip()[:200]}")
            elif lccc_ok != gcc_ok:
                findings.append(f"refusal {where}: GCC {'compiles' if gcc_ok else 'refuses'} it, "
                                f"lccc {'compiles' if lccc_ok else 'refuses'} it: "
                                f"{lccc_err.strip()[:200]}")
            elif lccc_lines != gcc_lines:
                findings.append(f"refusal {where}: GCC reports it at line {gcc_lines}, "
                                f"lccc at {lccc_lines}")
    return checked, findings


def select_function(template: str, only: str) -> str:
    """The fixture with every function but `only` (and `sink`) removed."""
    functions = re.findall(r"^int (\w+)\(", template, re.MULTILINE)
    assert only in functions, (only, functions)
    kept = []
    skipping = False
    for line in template.splitlines():
        start = re.match(r"int (\w+)\(.*\{$", line)
        if start and start.group(1) != only:
            skipping = True
        if not skipping:
            kept.append(line)
        if skipping and line == "}":
            skipping = False
            # Keep the line count: the refusal is reported by line.
            kept.append("")
        elif skipping:
            kept.append("")
    return "\n".join(kept) + "\n"


def diagnostics(compiler: str, options: list[str], source: Path):
    result = run([compiler, *options, "-c", "-o", "/dev/null", str(source)])
    errors = re.findall(r":(\d+:\d+): error: (.*)", result.stderr)
    return result.returncode == 0, errors


def check_clobber_names(lccc: str, gcc: str, work: Path, targets: set[str]):
    findings = []
    checked = 0

    def statement(name: str) -> str:
        return f'__asm__ volatile("" : : : "{name}");'

    for target in sorted(targets):
        taken = work / f"clobbers-taken{target}.c"
        taken.write_text("".join(f"void f{i}(void) {{ {statement(name)} }}\n"
                                 for i, name in enumerate(CLOBBERS_TAKEN)), encoding="utf-8")
        gcc_ok, gcc_errors = diagnostics(gcc, [target, "-O2"], taken)
        lccc_ok, lccc_errors = diagnostics(lccc, [target, "-O2"], taken)
        checked += len(CLOBBERS_TAKEN)
        if not gcc_ok:
            findings.append(f"clobber names {target}: GCC refuses a name it was measured to "
                            f"take: {gcc_errors[:2]}")
        elif not lccc_ok:
            findings.append(f"clobber names {target}: lccc refuses a name GCC takes: "
                            f"{lccc_errors[:2]}")
        for name in CLOBBERS_UNKNOWN + CLOBBERS_CLANG:
            source = work / "clobber-one.c"
            source.write_text(f"void f(void) {{ {statement(name)} }}\n", encoding="utf-8")
            gcc_ok, gcc_errors = diagnostics(gcc, [target, "-O2"], source)
            lccc_ok, lccc_errors = diagnostics(lccc, [target, "-O2"], source)
            checked += 1
            if gcc_ok:
                findings.append(f"clobber names {target}: GCC takes {name!r}, which it was "
                                f"measured to refuse")
            elif name in CLOBBERS_CLANG:
                if not lccc_ok:
                    findings.append(f"clobber names {target}: lccc refuses Clang's {name!r}: "
                                    f"{lccc_errors[:1]}")
            elif lccc_ok:
                findings.append(f"clobber names {target}: GCC refuses {name!r} "
                                f"({gcc_errors[0][1]}), lccc takes it")
            elif lccc_errors[:1] != gcc_errors[:1]:
                findings.append(f"clobber names {target}: {name!r}: GCC says {gcc_errors[:1]}, "
                                f"lccc {lccc_errors[:1]}")
    return checked, findings


def check_control(lccc: str, gcc: str, work: Path) -> list[str]:
    source = FIXTURES / "control.c"
    expected = build_and_run(gcc, ["-O2"], source, work / "control-g")
    planted = build_and_run(lccc, ["-O2", "-DPLANTED"], source, work / "control-l")
    honest = build_and_run(lccc, ["-O2"], source, work / "control-h")
    findings = []
    if planted == expected:
        findings.append("negative control: the planted difference was not seen")
    if honest != expected:
        findings.append(f"negative control: without the plant the programs differ: "
                        f"{honest!r} for {expected!r}")
    return findings


def self_check() -> list[str]:
    """`select_function` keeps one function and every line number."""
    template = (FIXTURES / "frame_pointer_refused.c").read_text(encoding="utf-8")
    problems = []
    for only in ("kept_for_the_array", "kept_by_request", "omitted"):
        selected = select_function(template, only)
        if len(selected.splitlines()) != len(template.splitlines()):
            problems.append(f"select_function({only}) changes the line count")
        names = re.findall(r"^int (\w+)\(.*\{$", selected, re.MULTILINE)
        if names != [only]:
            problems.append(f"select_function({only}) keeps {names}")
    return problems


def links_and_runs(gcc: str, target: str, work: Path) -> bool:
    source = work / f"probe{target}.c"
    source.write_text("int main(void) { return 0; }\n", encoding="utf-8")
    exe = work / f"probe{target}"
    built = run([gcc, target, "-o", str(exe), str(source)])
    return built.returncode == 0 and run([str(exe)]).returncode == 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lccc", default=str(ROOT / "target/fastbuild/lccc"))
    parser.add_argument("--gcc", default=None)
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--allow-missing-tools", action="store_true",
                        help="say what was not checked instead of failing")
    args = parser.parse_args()

    problems = self_check()
    if problems:
        print("\n".join(f"asm-registers: {p}" for p in problems), file=sys.stderr)
        return 1

    gcc = args.gcc or shutil.which("gcc-16")
    version = run([gcc, "-dumpfullversion"]).stdout.strip() if gcc else ""
    missing = []
    if not version.startswith("16.2"):
        missing.append(f"GCC 16.2 (found {version or 'none'})")
    with tempfile.TemporaryDirectory(prefix="lccc-asm-registers-") as tmp:
        work = Path(tmp)
        targets = set()
        if not missing:
            for target in ("-m64", "-m32"):
                if links_and_runs(gcc, target, work):
                    targets.add(target)
                else:
                    missing.append(f"a C library GCC links and runs {target} programs against")
        if missing:
            message = "asm-registers: missing " + "; ".join(missing)
            if not args.allow_missing_tools or not targets:
                print(message, file=sys.stderr)
                return 0 if args.allow_missing_tools else 1
            print(message + " (not checked)")

        cases, findings = check_programs(args.lccc, gcc, work, targets, max(1, args.jobs))
        libraries, more = check_shared_library(args.lccc, gcc, work, targets)
        findings += more
        refusals, more = check_refusals(args.lccc, gcc, work, targets)
        findings += more
        names, more = check_clobber_names(args.lccc, gcc, work, targets)
        findings += more
        findings += check_control(args.lccc, gcc, work)

    for finding in findings:
        print(f"asm-registers: {finding}", file=sys.stderr)
    if findings:
        print(f"asm-registers: FAIL ({len(findings)} findings)", file=sys.stderr)
        return 1
    print(f"asm-registers: {cases} programs equal GCC {version}'s "
          f"({', '.join(sorted(targets))}), {libraries} shared libraries linked with -z text, "
          f"{refusals} refusals agree, {names} clobber names judged as GCC judges them, "
          f"negative control reported, PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
